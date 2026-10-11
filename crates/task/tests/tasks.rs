//! Stackless tasks, timers, worker classes, `block_on` and shutdown.

#![cfg(not(feature = "loom"))]

mod common;

use core::future::pending;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Node, executor, graph, sized, wait_for};
use lactose_core::{AllocTag, ErrorKind, Result, ThreadClass};
use lactose_task::{
    CoreTopology, Executor, Platform, StdPlatform, ThreadHandle, ThreadSpec, WorkerClass, yield_now,
};

fn thread_name() -> String {
    std::thread::current().name().unwrap_or_default().to_owned()
}

#[test]
fn tasks_return_their_output_and_chain() {
    let executor = executor(2, 1);
    let inner = executor.spawn(WorkerClass::Background, async { 20 });
    let outer = executor.spawn(WorkerClass::FrameCritical, async move { inner.await.unwrap() + 1 });
    assert_eq!(executor.block_on(outer).unwrap(), 21);
    // `block_on` drives a plain future too, and tasks spawned from inside it.
    let spawner = executor.clone();
    let value = executor.block_on(async move {
        let task = spawner.spawn(WorkerClass::FrameCritical, async { "spawned inside" });
        yield_now().await;
        task.await.unwrap()
    });
    assert_eq!(value, "spawned inside");
    executor.shutdown();
}

#[test]
fn each_class_runs_on_its_own_threads() {
    let executor = executor(2, 2);
    let count = sized(64, 6);
    let spawn = |class| {
        let tasks: Vec<_> = (0..count)
            .map(|_| {
                executor.spawn(class, async {
                    yield_now().await;
                    thread_name()
                })
            })
            .collect();
        tasks.into_iter().map(|task| executor.block_on(task).unwrap()).collect::<Vec<_>>()
    };
    // The frame-critical workers are idle while the background tasks queue up, and still
    // never take one.
    let background = spawn(WorkerClass::Background);
    assert!(background.iter().all(|name| name.starts_with("whisk-background-")), "{background:?}");
    let frame = spawn(WorkerClass::FrameCritical);
    assert!(frame.iter().all(|name| name.starts_with("whisk-frame-")), "{frame:?}");

    // Graph work runs on frame-critical workers only.
    let names = Arc::new(Mutex::new(Vec::new()));
    let node = {
        let names = names.clone();
        Node::new("tasks::class")
            .units(sized(64, 6))
            .run(move |_, _| names.lock().unwrap().push(thread_name()))
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    assert!(names.lock().unwrap().iter().all(|name| name.starts_with("whisk-frame-")));
    executor.shutdown();
}

#[test]
fn yield_now_reschedules_behind_other_ready_tasks() {
    let executor = executor(1, 1);
    let order = Arc::new(Mutex::new(Vec::new()));
    let queued = Arc::new(AtomicBool::new(false));
    // Holds the only background worker until both tasks below are queued.
    let gate = {
        let queued = queued.clone();
        executor.spawn(WorkerClass::Background, async move { wait_for(&queued) })
    };
    let first = {
        let order = order.clone();
        executor.spawn(WorkerClass::Background, async move {
            order.lock().unwrap().push("first, before yield");
            yield_now().await;
            order.lock().unwrap().push("first, after yield");
        })
    };
    let second = {
        let order = order.clone();
        executor.spawn(WorkerClass::Background, async move {
            order.lock().unwrap().push("second");
        })
    };
    queued.store(true, Ordering::Release);
    executor.block_on(async {
        gate.await.unwrap();
        first.await.unwrap();
        second.await.unwrap();
    });
    assert_eq!(*order.lock().unwrap(), ["first, before yield", "second", "first, after yield"]);
    executor.shutdown();
}

#[test]
fn dropping_a_task_cancels_it_and_detaching_does_not() {
    let executor = executor(1, 1);

    // Suspended on something that never completes: the drop releases what it owns.
    let token = Arc::new(());
    let started = Arc::new(AtomicBool::new(false));
    let suspended = {
        let (token, started) = (token.clone(), started.clone());
        executor.spawn(WorkerClass::Background, async move {
            let _held = token;
            started.store(true, Ordering::Release);
            pending::<()>().await;
        })
    };
    wait_for(&started);
    drop(suspended);
    let deadline = Instant::now() + Duration::from_secs(60);
    while Arc::strong_count(&token) != 1 {
        assert!(Instant::now() < deadline, "the cancelled task kept its future");
        std::thread::yield_now();
    }

    // Still queued behind a task that holds the only worker: it never runs at all.
    let release = Arc::new(AtomicBool::new(false));
    let holding = Arc::new(AtomicBool::new(false));
    let blocker = {
        let (release, holding) = (release.clone(), holding.clone());
        executor.spawn(WorkerClass::Background, async move {
            holding.store(true, Ordering::Release);
            wait_for(&release);
        })
    };
    wait_for(&holding);
    let ran = Arc::new(AtomicBool::new(false));
    let queued = {
        let ran = ran.clone();
        executor.spawn(WorkerClass::Background, async move { ran.store(true, Ordering::Release) })
    };
    drop(queued);

    // Detached: runs to completion with nobody holding its handle.
    let detached_ran = Arc::new(AtomicBool::new(false));
    {
        let detached_ran = detached_ran.clone();
        executor
            .spawn(WorkerClass::Background, async move {
                yield_now().await;
                detached_ran.store(true, Ordering::Release);
            })
            .detach();
    }
    release.store(true, Ordering::Release);
    executor.block_on(blocker).unwrap();
    wait_for(&detached_ran);
    assert!(!ran.load(Ordering::Acquire), "a task cancelled while queued must not run");
    executor.shutdown();
}

#[test]
fn a_task_keeps_its_spawners_allocation_tag_on_every_poll() {
    let executor = executor(2, 2);
    let task = {
        let _net = AllocTag::Net.scope();
        executor.spawn(WorkerClass::Background, async {
            let mut tags = vec![AllocTag::current()];
            for _ in 0..sized(8, 2) {
                yield_now().await;
                tags.push(AllocTag::current());
            }
            tags
        })
    };
    let untagged = executor.spawn(WorkerClass::Background, async { AllocTag::current() });
    assert!(executor.block_on(task).unwrap().iter().all(|&tag| tag == AllocTag::Net));
    // The tag does not leak to the next task the worker polls.
    assert_eq!(executor.block_on(untagged).unwrap(), AllocTag::Untagged);
    executor.shutdown();
}

#[test]
fn sleep_until_resolves_at_its_deadline() {
    let executor = executor(2, 1);
    let delay = Duration::from_millis(20);

    // From the thread inside `block_on`, with every worker asleep.
    let start = Instant::now();
    executor.block_on(executor.sleep_until(start + delay)).unwrap();
    assert!(start.elapsed() >= delay);
    // A deadline in the past resolves at once.
    executor.block_on(executor.sleep_until(start)).unwrap();

    // From tasks, with deadlines registered out of order: none resolves early, and a
    // dropped timer does not fire. The order the tasks finish in is not asserted: a task
    // first polled after its deadline resolves at that poll, and woken tasks run on either
    // worker. `due_timers_wake_in_deadline_order` covers the order the executor keeps.
    let start = Instant::now();
    let tasks: Vec<_> = [3u32, 1, 2]
        .into_iter()
        .map(|rank| {
            let sleeper = executor.clone();
            executor.spawn(WorkerClass::FrameCritical, async move {
                sleeper.sleep_until(start + delay * rank).await.unwrap();
                start.elapsed()
            })
        })
        .collect();
    let abandoned = {
        let sleeper = executor.clone();
        executor.spawn(WorkerClass::FrameCritical, async move {
            let _ = sleeper.sleep_until(start + Duration::from_secs(3600)).await;
            unreachable!("this task is cancelled before its deadline");
        })
    };
    drop(abandoned);
    for (task, rank) in tasks.into_iter().zip([3u32, 1, 2]) {
        assert!(executor.block_on(task).unwrap() >= delay * rank);
    }
    executor.shutdown();
}

/// The keeper parks only until the earliest deadline, so a timer registered after a later
/// one wakes the keeper and fires at its own deadline, not the later one's. Both timers are
/// registered from this thread, which wakes no worker by running anything, so with the far
/// timer an hour out, holding the near one back fails the wait.
#[test]
fn a_later_timer_does_not_hold_an_earlier_one_back() {
    struct Flag(AtomicBool);
    impl std::task::Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::Release);
        }
    }

    let executor = executor(2, 1);
    let mut far = core::pin::pin!(executor.sleep_until(Instant::now() + Duration::from_secs(3600)));
    assert!(far.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending());
    // Lets a worker lie down as the far timer's keeper. If none has when the near timer
    // registers, the near one is the earliest anyway: the test proves less, never fails.
    std::thread::sleep(Duration::from_millis(10));

    let fired = Arc::new(Flag(AtomicBool::new(false)));
    let waker = Waker::from(fired.clone());
    let start = Instant::now();
    let delay = Duration::from_millis(20);
    let mut near = core::pin::pin!(executor.sleep_until(start + delay));
    if near.as_mut().poll(&mut Context::from_waker(&waker)).is_pending() {
        wait_for(&fired.0);
    }
    assert!(near.as_mut().poll(&mut Context::from_waker(&waker)).is_ready());
    assert!(start.elapsed() >= delay);
    executor.shutdown();
}

/// A worker that fires timers wakes the due ones in deadline order, ties in registration
/// order. The only frame-critical worker, the only thread that fires timers here, is held
/// in a job until every deadline has passed, so it finds them all due at once and the
/// order of the wake-ups is the timer store's alone. The only timing is that every
/// deadline falls after all of them are registered; an attempt where one does not proves
/// nothing and is repeated with a longer delay.
#[test]
fn due_timers_wake_in_deadline_order() {
    struct Record {
        name: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }
    impl std::task::Wake for Record {
        fn wake(self: Arc<Self>) {
            self.log.lock().unwrap().push(self.name);
        }
    }

    let executor = executor(1, 1);
    for delay in [20, 200, 2000].map(Duration::from_millis) {
        let (started, release) =
            (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (flag, hold) = (started.clone(), release.clone());
        let busy = Node::new("tasks::hold")
            .run(move |_, _| {
                flag.store(true, Ordering::Release);
                while !hold.load(Ordering::Acquire) {
                    core::hint::spin_loop();
                }
            })
            .build();
        let run = executor.run_graph(graph(vec![busy], &[]));
        wait_for(&started);

        let log = Arc::new(Mutex::new(Vec::new()));
        let start = Instant::now();
        // Registered out of deadline order, with a tie at the earliest deadline.
        let mut timers: Vec<_> = [("third", 3u32), ("first", 1), ("second", 2), ("tied", 1)]
            .into_iter()
            .map(|(name, rank)| {
                let waker = Waker::from(Arc::new(Record { name, log: log.clone() }));
                (Box::pin(executor.sleep_until(start + delay * rank)), waker)
            })
            .collect();
        let registered = timers.iter_mut().all(|(sleep, waker)| {
            sleep.as_mut().poll(&mut Context::from_waker(waker)).is_pending()
        });
        let in_time = registered && Instant::now() < start + delay;
        if in_time {
            std::thread::sleep((start + delay * 3).saturating_duration_since(Instant::now()));
        }
        release.store(true, Ordering::Release);
        executor.block_on(run).unwrap();
        if !in_time {
            // A deadline passed during registration. Dropping the timers removes them.
            drop(timers);
            continue;
        }
        let limit = Instant::now() + Duration::from_secs(60);
        while log.lock().unwrap().len() < timers.len() {
            assert!(Instant::now() < limit, "timed out waiting for the timers to fire");
            std::thread::yield_now();
        }
        assert_eq!(*log.lock().unwrap(), ["first", "tied", "second", "third"]);
        executor.shutdown();
        return;
    }
    // Under Miri the simulated clock can pass the first deadline during registration.
    if !cfg!(miri) {
        panic!("every attempt saw a deadline pass before the timers were registered");
    }
    executor.shutdown();
}

#[test]
fn timers_fire_while_every_worker_is_busy_and_then_idle() {
    let executor = executor(1, 1);
    // The only frame-critical worker runs a job across the deadline; the timer fires when
    // the worker next looks for work.
    let start = Instant::now();
    let delay = Duration::from_millis(10);
    let sleeper = executor.clone();
    let timer = executor.spawn(WorkerClass::Background, async move {
        sleeper.sleep_until(start + delay).await.unwrap();
        start.elapsed()
    });
    let busy = Node::new("tasks::busy").run(move |_, _| common::spin_for(delay * 2)).build();
    executor.block_on(executor.run_graph(graph(vec![busy], &[]))).unwrap();
    assert!(executor.block_on(timer).unwrap() >= delay);
    executor.shutdown();
}

/// The worker that fires a timer keeps the task it wakes as its own next job only while
/// nothing waits in the queue. Here three tasks were queued before the timers fired, so
/// both timer tasks queue behind them, in deadline order: a timer never lets a task cut in
/// ahead of work that was waiting, whose wake-up may already have been spent on the very
/// worker that fires it. (With nothing queued, the in-crate test
/// `a_timer_task_runs_on_the_worker_that_fires_it_and_wakes_nobody` covers the keep.)
///
/// One frame-critical worker makes the order exact. The only timing is that the deadline
/// falls after the timers are registered and the busy task has started; an attempt where
/// it does not proves nothing and is repeated with a longer margin.
#[test]
fn a_task_woken_by_a_timer_never_runs_ahead_of_queued_tasks() {
    let executor = executor(1, 1);
    for margin in [20, 200, 2000].map(Duration::from_millis) {
        if let Some(order) = order_after_timers_fire(&executor, margin) {
            assert_eq!(order, ["queued 0", "queued 1", "queued 2", "first timer", "second timer"]);
            executor.shutdown();
            return;
        }
    }
    // Under Miri the simulated clock can pass the deadline before the tasks are polled.
    if !cfg!(miri) {
        panic!("every attempt saw the deadline pass before the busy task started");
    }
    executor.shutdown();
}

/// From one task, so they are queued together behind it: two timer tasks with the same
/// deadline, then a busy task that queues three more and spins past that deadline. Returns
/// the order the five ran in, or `None` if the deadline came before both timers were
/// registered and the busy task had started.
fn order_after_timers_fire(executor: &Executor, margin: Duration) -> Option<Vec<String>> {
    let order = Arc::new(Mutex::new(Vec::new()));
    let registered = Arc::new(AtomicUsize::new(0));
    let spawner = executor.clone();
    let log = order.clone();
    let setup = executor.spawn(WorkerClass::FrameCritical, async move {
        let deadline = Instant::now() + margin;
        let timers = ["first timer", "second timer"].map(|name| {
            let (sleep, log, registered) =
                (spawner.sleep_until(deadline), log.clone(), registered.clone());
            spawner.spawn(WorkerClass::FrameCritical, async move {
                let mut sleep = core::pin::pin!(sleep);
                let mut counted = false;
                core::future::poll_fn(|cx| {
                    let poll = sleep.as_mut().poll(cx);
                    if poll.is_pending() && !core::mem::replace(&mut counted, true) {
                        registered.fetch_add(1, Ordering::SeqCst);
                    }
                    poll
                })
                .await
                .unwrap();
                log.lock().unwrap().push(name.to_owned());
            })
        });
        let queuer = spawner.clone();
        let busy = spawner.spawn(WorkerClass::FrameCritical, async move {
            let started_in_time = Instant::now() < deadline;
            let queued: Vec<_> = (0..3)
                .map(|index| {
                    let log = log.clone();
                    queuer.spawn(WorkerClass::FrameCritical, async move {
                        log.lock().unwrap().push(format!("queued {index}"));
                    })
                })
                .collect();
            while Instant::now() < deadline {
                core::hint::spin_loop();
            }
            (started_in_time && registered.load(Ordering::SeqCst) == 2, queued)
        });
        (timers, busy)
    });
    let (timers, busy) = executor.block_on(setup).unwrap();
    let (valid, queued) = executor.block_on(busy).unwrap();
    for task in timers.into_iter().chain(queued) {
        executor.block_on(task).unwrap();
    }
    valid.then(|| order.lock().unwrap().clone())
}

/// A task whose every poll registers a short timer and works past it before returning (a
/// short sleep raced against work, in a loop) finds its timer due each time its worker next
/// looks for work. That must not keep it ahead of the queue on every pass: with one
/// frame-critical worker, a task spawned meanwhile runs before the loop's next iteration
/// ends, not when the loop does.
///
/// Counted in iterations, not time. The loop stops once the other task has run, or gives up
/// a hundred iterations after it was queued.
#[test]
#[cfg_attr(miri, ignore = "a timer has to fall due during each poll, on the real clock")]
fn a_task_whose_timer_is_always_due_does_not_starve_the_queue() {
    let executor = executor(1, 1);
    let started = Arc::new(AtomicBool::new(false));
    let queued = Arc::new(AtomicBool::new(false));
    let ran = Arc::new(AtomicBool::new(false));
    let looper = {
        let (sleeper, started, queued, ran) =
            (executor.clone(), started.clone(), queued.clone(), ran.clone());
        executor.spawn(WorkerClass::FrameCritical, async move {
            started.store(true, Ordering::SeqCst);
            // Iterations that began after the other task was queued and gave the worker
            // back; one whose deadline had passed before its first poll never suspends.
            let mut after = 0u32;
            let mut iterations = 0u32;
            while !ran.load(Ordering::SeqCst) && after <= 100 && iterations < 1_000_000 {
                iterations += 1;
                let seen = queued.load(Ordering::SeqCst);
                let deadline = Instant::now() + Duration::from_micros(20);
                let mut sleep = core::pin::pin!(sleeper.sleep_until(deadline));
                let mut suspended = false;
                core::future::poll_fn(|cx| {
                    let poll = sleep.as_mut().poll(cx);
                    if poll.is_pending() {
                        suspended = true;
                        common::spin_for(Duration::from_micros(30));
                    }
                    poll
                })
                .await
                .unwrap();
                after += u32::from(seen && suspended);
            }
            after
        })
    };
    wait_for(&started);
    let other = {
        let ran = ran.clone();
        executor.spawn(WorkerClass::FrameCritical, async move { ran.store(true, Ordering::SeqCst) })
    };
    queued.store(true, Ordering::SeqCst);
    let after = executor.block_on(looper).unwrap();
    executor.block_on(other).unwrap();
    // The first iteration that sees the task queued suspends into a firing that finds it
    // there, so the task runs before the loop resumes.
    assert!(after <= 1, "the queued task waited {after} iterations of the loop");
    executor.shutdown();
}

#[test]
fn shutdown_with_work_outstanding_finishes_running_jobs_and_drops_the_rest() {
    let executor = executor(sized(3, 2), 1);
    let token = Arc::new(());
    let in_scope = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let children = Arc::new(AtomicU32::new(0));
    let scope_closed = Arc::new(AtomicBool::new(false));

    // A job that is inside a scope when shutdown starts: it and its children finish.
    let running = {
        let (in_scope, release, children, scope_closed) =
            (in_scope.clone(), release.clone(), children.clone(), scope_closed.clone());
        Node::new("tasks::running")
            .run(move |_, ctx| {
                ctx.scope(|scope| {
                    let children = &children;
                    scope.fork_range(0..sized(64, 6), move |range, _| {
                        std::thread::yield_now();
                        children.fetch_add(range.len() as u32, Ordering::Relaxed);
                    });
                    in_scope.store(true, Ordering::Release);
                    wait_for(&release);
                });
                scope_closed.store(true, Ordering::Release);
            })
            .build()
    };
    // Work that is still queued behind it: dropped, never run.
    let never = {
        let token = token.clone();
        Node::new("tasks::never")
            .units(sized(1000, 10))
            .run(move |_, _| {
                let _held = &token;
                unreachable!("released only after a node that outlives the executor");
            })
            .build()
    };
    let run = executor.run_graph(graph(vec![running, never], &[(0, 1)]));

    // Tasks in every state: suspended on a far timer, suspended forever, and queued behind
    // a busy worker.
    let far = {
        let (sleeper, token) = (executor.clone(), token.clone());
        executor.spawn(WorkerClass::Background, async move {
            let _held = token;
            let _ = sleeper.sleep_until(Instant::now() + Duration::from_secs(3600)).await;
        })
    };
    let forever = {
        let token = token.clone();
        executor.spawn(WorkerClass::FrameCritical, async move {
            let _held = token;
            pending::<()>().await;
        })
    };
    let blocker = {
        let release = release.clone();
        executor.spawn(WorkerClass::Background, async move { wait_for(&release) })
    };
    let queued: Vec<_> = (0..sized(32, 4))
        .map(|_| {
            let token = token.clone();
            executor.spawn(WorkerClass::Background, async move { drop(token) })
        })
        .collect();

    wait_for(&in_scope);
    let stopper = {
        let executor = executor.clone();
        std::thread::spawn(move || executor.shutdown())
    };
    // Once shutdown has begun, a spawned task is cancelled inside `spawn`: its future, and
    // the probe it holds, are gone when `spawn` returns. Before that it queues behind the
    // blocker.
    let begun = Instant::now();
    loop {
        let probe = Arc::new(());
        let held = probe.clone();
        let task = executor.spawn(WorkerClass::Background, async move { drop(held) });
        if Arc::strong_count(&probe) == 1 {
            break;
        }
        drop(task);
        assert!(begun.elapsed() < Duration::from_secs(60), "shutdown never began");
        std::thread::yield_now();
    }
    // Shutdown waits for the running job, which waits for this.
    std::thread::sleep(Duration::from_millis(sized(20, 1).into()));
    assert!(!stopper.is_finished());
    release.store(true, Ordering::Release);
    stopper.join().unwrap();

    assert!(scope_closed.load(Ordering::Acquire), "the running job was cut short");
    assert_eq!(children.load(Ordering::Relaxed), sized(64, 6));
    // Every future the executor still held is gone; the graph lives in its run future.
    drop(run);
    assert_eq!(Arc::strong_count(&token), 1, "shutdown left work alive");

    // The handle stays valid and inert: new work is cancelled at once.
    let late = {
        let token = token.clone();
        executor.spawn(WorkerClass::FrameCritical, async move { drop(token) })
    };
    assert_eq!(Arc::strong_count(&token), 1);
    drop((far, forever, blocker, queued, late));
    executor.shutdown();
}

#[test]
fn zero_workers_of_a_class_is_an_error() {
    let error = |frame, background| {
        Executor::builder()
            .frame_workers(frame)
            .background_workers(background)
            .build()
            .err()
            .map(|error| error.kind())
    };
    assert_eq!(error(0, 1), Some(ErrorKind::Unsupported));
    assert_eq!(error(1, 0), Some(ErrorKind::Unsupported));
}

/// A platform that reports a fixed topology and counts the threads it creates.
struct CountingPlatform {
    topology: CoreTopology,
    specs: Mutex<Vec<ThreadSpec>>,
    fail_after: Option<usize>,
}

impl Platform for CountingPlatform {
    fn topology(&self) -> CoreTopology {
        self.topology
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        let mut specs = self.specs.lock().unwrap();
        if self.fail_after == Some(specs.len()) {
            return Err(lactose_core::Error::new(ErrorKind::ResourceExhausted, "no more threads"));
        }
        specs.push(spec.clone());
        drop(specs);
        // Miri cannot apply a scheduling class; `StdPlatform` knows.
        StdPlatform.spawn_thread(spec, body)
    }
}

#[test]
fn worker_counts_follow_the_platform_topology() {
    let build = |performance, efficiency| {
        let platform = Arc::new(CountingPlatform {
            topology: CoreTopology { performance, efficiency },
            specs: Mutex::new(Vec::new()),
            fail_after: None,
        });
        let executor = Executor::builder().platform(platform.clone()).build().unwrap();
        assert_eq!(executor.frame_worker_count(), performance);
        // Threads other crates own go through the same platform.
        let dedicated = ThreadSpec {
            name: "milkman".to_owned(),
            class: ThreadClass::Network,
            index_in_class: 0,
        };
        executor.platform().spawn_thread(dedicated, Box::new(|| {})).unwrap().join();
        executor.shutdown();
        let specs = platform.specs.lock().unwrap();
        let count = |class| specs.iter().filter(|spec| spec.class == class).count() as u32;
        assert_eq!(count(ThreadClass::Network), 1);
        let indices: Vec<u32> = specs
            .iter()
            .filter(|spec| spec.class == ThreadClass::FrameCritical)
            .map(|spec| spec.index_in_class)
            .collect();
        assert_eq!(indices, (0..performance).collect::<Vec<_>>());
        (count(ThreadClass::FrameCritical), count(ThreadClass::Background))
    };
    // Hybrid: one background worker per efficiency core.
    assert_eq!(build(3, 2), (3, 2));
    // Homogeneous: two low-priority background threads.
    assert_eq!(build(2, 0), (2, 2));
}

#[test]
fn a_failed_thread_creation_fails_the_build_and_stops_the_started_workers() {
    let platform = Arc::new(CountingPlatform {
        topology: CoreTopology { performance: 2, efficiency: 1 },
        specs: Mutex::new(Vec::new()),
        fail_after: Some(2),
    });
    let error = Executor::builder().platform(platform.clone()).build().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::ResourceExhausted);
    // `build` joined the two workers it had started before returning.
    assert_eq!(platform.specs.lock().unwrap().len(), 2);
}

/// Whatever was waiting on work that shutdown cut off gets an answer. Before, a thread in
/// `block_on` on a cancelled task parked for ever.
#[test]
fn futures_cut_off_by_shutdown_resolve_to_cancelled() {
    let executor = executor(1, 1);
    let release = Arc::new(AtomicBool::new(false));
    let running = Arc::new(AtomicBool::new(false));

    // A graph run that cannot finish: its second node waits behind one that is still
    // running when shutdown begins.
    let first = {
        let (release, running) = (release.clone(), running.clone());
        Node::new("tasks::held")
            .run(move |_, _| {
                running.store(true, Ordering::Release);
                wait_for(&release);
            })
            .build()
    };
    let second = Node::new("tasks::cut").build();
    let run = executor.run_graph(graph(vec![first, second], &[(0, 1)]));
    let task = executor.spawn(WorkerClass::Background, pending::<u32>());
    let sleep = executor.sleep_until(Instant::now() + Duration::from_secs(3600));

    // Three threads blocked on the three kinds of future, as `main` or a tool would be.
    let waiters = {
        let (a, b, c) = (executor.clone(), executor.clone(), executor.clone());
        [
            std::thread::spawn(move || a.block_on(run).err().map(|error| error.kind())),
            std::thread::spawn(move || b.block_on(task).err().map(|error| error.kind())),
            std::thread::spawn(move || c.block_on(sleep).err().map(|error| error.kind())),
        ]
    };
    wait_for(&running);
    let stopper = {
        let executor = executor.clone();
        std::thread::spawn(move || executor.shutdown())
    };
    // Let shutdown begin before the running job is released, so the second node is cut.
    // Until it has begun, a trivial task completes; from then on it is cancelled at once.
    let begun = Instant::now();
    while executor.block_on(executor.spawn(WorkerClass::Background, async {})).is_ok() {
        assert!(begun.elapsed() < Duration::from_secs(60), "shutdown never began");
        std::thread::yield_now();
    }
    release.store(true, Ordering::Release);
    stopper.join().unwrap();
    for waiter in waiters {
        assert_eq!(waiter.join().unwrap(), Some(ErrorKind::Cancelled));
    }

    // What is started after shutdown is cancelled at once, not left pending.
    let late_run = executor.run_graph(graph(vec![Node::new("tasks::late").build()], &[]));
    let late_run = executor.block_on(late_run).err().map(|error| error.kind());
    assert_eq!(late_run, Some(ErrorKind::Cancelled));
    let late_task = executor.spawn(WorkerClass::FrameCritical, async { 1 });
    assert_eq!(executor.block_on(late_task).unwrap_err().kind(), ErrorKind::Cancelled);
    let late_sleep = executor.sleep_until(Instant::now() + Duration::from_secs(3600));
    assert_eq!(executor.block_on(late_sleep).unwrap_err().kind(), ErrorKind::Cancelled);
}

/// A platform that counts the threads it has running.
struct LiveThreads {
    live: Arc<AtomicUsize>,
}

impl Platform for LiveThreads {
    fn topology(&self) -> CoreTopology {
        CoreTopology { performance: 2, efficiency: 1 }
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        let live = self.live.clone();
        live.fetch_add(1, Ordering::AcqRel);
        StdPlatform.spawn_thread(
            spec,
            Box::new(move || {
                body();
                live.fetch_sub(1, Ordering::AcqRel);
            }),
        )
    }
}

#[test]
fn dropping_the_last_handle_shuts_the_pool_down() {
    let live = Arc::new(AtomicUsize::new(0));
    let platform = Arc::new(LiveThreads { live: live.clone() });
    let executor = Executor::builder().platform(platform).build().unwrap();
    assert_eq!(live.load(Ordering::Acquire), 3);

    // A suspended task that owns something, and a clone of the handle.
    let token = Arc::new(());
    let started = Arc::new(AtomicBool::new(false));
    {
        let (token, started) = (token.clone(), started.clone());
        executor
            .spawn(WorkerClass::Background, async move {
                let _held = token;
                started.store(true, Ordering::Release);
                pending::<()>().await;
            })
            .detach();
    }
    wait_for(&started);
    let clone = executor.clone();
    drop(executor);
    assert_eq!(live.load(Ordering::Acquire), 3, "a handle is still alive");
    assert_eq!(clone.frame_worker_count(), 2);

    // The last handle, dropped off the pool's threads: the workers are joined and the
    // task's future is dropped, as `shutdown` would have done.
    drop(clone);
    assert_eq!(live.load(Ordering::Acquire), 0, "the workers outlived every handle");
    assert_eq!(Arc::strong_count(&token), 1);
}

#[test]
fn a_handle_held_by_a_task_keeps_the_pool_until_the_task_ends() {
    let live = Arc::new(AtomicUsize::new(0));
    let platform = Arc::new(LiveThreads { live: live.clone() });
    let executor = Executor::builder().platform(platform).build().unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    {
        let (handle, release, done) = (executor.clone(), release.clone(), done.clone());
        executor
            .spawn(WorkerClass::FrameCritical, async move {
                while !release.load(Ordering::Acquire) {
                    yield_now().await;
                }
                // Still usable: the pool is alive because this task holds a handle.
                let inner = handle.spawn(WorkerClass::Background, async { 5 });
                assert_eq!(inner.await.unwrap(), 5);
                done.store(true, Ordering::Release);
                // The last handle goes here, on a worker thread, which cannot join itself:
                // the workers are told to exit and do so on their own.
            })
            .detach();
    }
    drop(executor);
    assert_eq!(live.load(Ordering::Acquire), 3);
    release.store(true, Ordering::Release);
    wait_for(&done);
    let start = Instant::now();
    while live.load(Ordering::Acquire) != 0 {
        assert!(start.elapsed() < Duration::from_secs(60), "the workers never exited");
        std::thread::yield_now();
    }
}

/// Notes the allocation tag current when it is dropped.
struct TagAtDrop(Arc<Mutex<Option<AllocTag>>>);

impl Drop for TagAtDrop {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = Some(AllocTag::current());
    }
}

/// A future that parks its waker where the test can reach it and never completes.
struct ParkWaker {
    slot: Arc<Mutex<Option<Waker>>>,
    _dropped: TagAtDrop,
}

impl Future for ParkWaker {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        *self.slot.lock().unwrap() = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// A detached task nobody can wake any more is gone: its future is dropped, under the tag
/// it was spawned with, when its last waker is.
#[test]
fn a_detached_task_whose_last_waker_is_dropped_is_dropped_under_its_tag() {
    let executor = executor(1, 1);
    let slot = Arc::new(Mutex::new(None));
    let dropped_under = Arc::new(Mutex::new(None));
    {
        let _asset = AllocTag::Asset.scope();
        let future = ParkWaker { slot: slot.clone(), _dropped: TagAtDrop(dropped_under.clone()) };
        executor.spawn(WorkerClass::Background, future).detach();
    }
    let start = Instant::now();
    let waker = loop {
        if let Some(waker) = slot.lock().unwrap().take() {
            break waker;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "the task never ran");
        std::thread::yield_now();
    };
    // The worker may still be on its way out of the poll, holding the task for a moment.
    drop(waker);
    let start = Instant::now();
    while dropped_under.lock().unwrap().is_none() {
        assert!(start.elapsed() < Duration::from_secs(60), "the orphaned future was kept");
        std::thread::yield_now();
    }
    assert_eq!(*dropped_under.lock().unwrap(), Some(AllocTag::Asset));
    executor.shutdown();
}

/// A future that hands its waker to another thread and completes after three polls.
struct Ping {
    slot: Arc<Mutex<Option<Waker>>>,
    left: u32,
    dropped: Arc<AtomicUsize>,
}

impl Future for Ping {
    type Output = u32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
        if self.left == 0 {
            return Poll::Ready(7);
        }
        self.left -= 1;
        *self.slot.lock().unwrap() = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for Ping {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::AcqRel);
    }
}

/// Wakes, cancellation, detaching and completion racing from several threads: every future
/// is dropped exactly once and nothing is polled after it is gone (Miri checks the latter).
#[test]
fn wake_cancel_and_complete_race() {
    let rounds = sized(3000, 20) as usize;
    let executor = executor(2, 2);
    let dropped = Arc::new(AtomicUsize::new(0));
    for round in 0..rounds {
        let slot = Arc::new(Mutex::new(None::<Waker>));
        let class =
            if round % 2 == 0 { WorkerClass::Background } else { WorkerClass::FrameCritical };
        let ping = Ping { slot: slot.clone(), left: 3, dropped: dropped.clone() };
        let task = executor.spawn(class, ping);
        let waker_thread = {
            let slot = slot.clone();
            std::thread::spawn(move || {
                for _ in 0..6 {
                    if let Some(waker) = slot.lock().unwrap().take() {
                        waker.wake_by_ref();
                        waker.wake();
                    }
                    std::thread::yield_now();
                }
            })
        };
        match round % 3 {
            0 => drop(task),
            1 => task.detach(),
            _ => {
                std::thread::yield_now();
                drop(task);
            }
        }
        waker_thread.join().unwrap();
        // A stale waker, after the task may have completed or been cancelled.
        if let Some(waker) = slot.lock().unwrap().take() {
            waker.wake();
        }
    }
    executor.shutdown();
    assert_eq!(dropped.load(Ordering::Acquire), rounds, "every future is dropped exactly once");
}

/// A node whose one unit is inside a scope, with a child still running, when its run is
/// cancelled. Returns the node, the flag it sets on entry, the flag that lets its child
/// finish, the flag it sets when the unit has returned from the scope, and its finalize
/// count.
#[allow(clippy::type_complexity, reason = "a one-off bundle of test flags")]
fn node_caught_mid_scope() -> (
    Arc<dyn lactose_task::GraphNode>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    Arc<AtomicU32>,
) {
    let running = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let returned = Arc::new(AtomicBool::new(false));
    let finalized = Arc::new(AtomicU32::new(0));
    let node = {
        let (running, release, returned, finalized) =
            (running.clone(), release.clone(), returned.clone(), finalized.clone());
        Node::new("tasks::mid-scope")
            .run(move |_, ctx| {
                let (running, release) = (&running, &release);
                ctx.scope(|scope| {
                    scope.fork(move |_| {
                        running.store(true, Ordering::Release);
                        wait_for(release);
                    });
                });
                returned.store(true, Ordering::Release);
            })
            .finalize(move |_| {
                finalized.fetch_add(1, Ordering::AcqRel);
            })
            .build()
    };
    (node, running, release, returned, finalized)
}

/// `Cancelled` means quiet. A run that is cancelled while one of its jobs is inside a scope
/// does not resolve until that job, children included, has returned; and nothing of the run
/// starts afterwards, not even the finalize of the node that was caught.
#[test]
fn a_cancelled_run_resolves_only_when_its_jobs_have_returned() {
    let executor = executor(2, 1);
    let (node, running, release, returned, finalized) = node_caught_mid_scope();
    let never = Arc::new(AtomicU32::new(0));
    let dependent = {
        let never = never.clone();
        Node::new("tasks::after-cancel")
            .run(move |_, _| {
                never.fetch_add(1, Ordering::AcqRel);
            })
            .build()
    };
    let run = executor.run_graph(graph(vec![node, dependent], &[(0, 1)]));
    wait_for(&running);

    let resolved = Arc::new(AtomicBool::new(false));
    let waiter = {
        let (handle, resolved, returned) = (executor.clone(), resolved.clone(), returned.clone());
        std::thread::spawn(move || {
            let result = handle.block_on(run);
            // Read at the moment of resolution: the job must be out of its scope by now.
            let quiet = returned.load(Ordering::Acquire);
            resolved.store(true, Ordering::Release);
            (result.err().map(|error| error.kind()), quiet)
        })
    };
    let stopper = {
        let handle = executor.clone();
        std::thread::spawn(move || handle.shutdown())
    };
    // Shutdown has begun once a new task is refused. The run must still be unresolved
    // then, however long the machine takes: its job has not been let go.
    let begun = Instant::now();
    while executor.block_on(executor.spawn(WorkerClass::Background, async {})).is_ok() {
        assert!(begun.elapsed() < Duration::from_secs(60), "shutdown never began");
        std::thread::yield_now();
    }
    for _ in 0..sized(200, 5) {
        assert!(!resolved.load(Ordering::Acquire), "resolved while a job was in its scope");
        std::thread::yield_now();
    }
    release.store(true, Ordering::Release);
    let (kind, quiet) = waiter.join().unwrap();
    stopper.join().unwrap();
    assert_eq!(kind, Some(ErrorKind::Cancelled));
    assert!(quiet, "the run resolved before its job had returned");
    assert_eq!(finalized.load(Ordering::Acquire), 0, "a finalize started after cancellation");
    assert_eq!(never.load(Ordering::Acquire), 0, "a dependent ran after cancellation");
}

/// The same when nobody joins: the last handle is dropped on one of the executor's own
/// workers, which only tells the others to stop.
#[test]
fn a_run_cancelled_from_a_worker_is_quiet_when_it_resolves() {
    let executor = executor(2, 1);
    let (node, running, release, returned, finalized) = node_caught_mid_scope();
    let mut run = core::pin::pin!(executor.run_graph(graph(vec![node], &[])));
    wait_for(&running);

    // The only handle moves into a task, which drops it on a background worker.
    let dropped = Arc::new(AtomicBool::new(false));
    let go = Arc::new(AtomicBool::new(false));
    {
        let (dropped, go) = (dropped.clone(), go.clone());
        let spawner = executor.clone();
        spawner
            .spawn(WorkerClass::Background, async move {
                while !go.load(Ordering::Acquire) {
                    yield_now().await;
                }
                drop(executor);
                dropped.store(true, Ordering::Release);
            })
            .detach();
    }
    // Only now, with the test's own handle gone, is the task's handle the last one.
    go.store(true, Ordering::Release);
    let mut context = Context::from_waker(Waker::noop());
    // Until the task has dropped the last handle the run is simply pending; after that it
    // is cancelled but its job is still in its scope, so it stays pending.
    while !dropped.load(Ordering::Acquire) {
        assert!(run.as_mut().poll(&mut context).is_pending());
        std::thread::yield_now();
    }
    for _ in 0..sized(200, 5) {
        assert!(run.as_mut().poll(&mut context).is_pending(), "resolved over a running job");
        std::thread::yield_now();
    }
    release.store(true, Ordering::Release);
    let start = Instant::now();
    let result = loop {
        if let Poll::Ready(result) = run.as_mut().poll(&mut context) {
            break (result.err().map(|error| error.kind()), returned.load(Ordering::Acquire));
        }
        assert!(start.elapsed() < Duration::from_secs(60), "the run never resolved");
        std::thread::yield_now();
    };
    assert_eq!(result, (Some(ErrorKind::Cancelled), true));
    assert_eq!(finalized.load(Ordering::Acquire), 0);
}

/// Dropping an executor's last handle on a worker of a different executor is an ordinary
/// drop: the threads are joined. Only a worker of the executor itself cannot wait for it.
#[test]
fn another_executors_worker_joins_the_pool_it_drops() {
    let live = Arc::new(AtomicUsize::new(0));
    let platform = Arc::new(LiveThreads { live: live.clone() });
    let inner = Executor::builder().platform(platform).build().unwrap();
    assert_eq!(live.load(Ordering::Acquire), 3);
    let outer = executor(1, 1);

    let live_after_drop = {
        let live = live.clone();
        outer.block_on(outer.spawn(WorkerClass::FrameCritical, async move {
            assert_eq!(inner.frame_worker_count(), 2);
            drop(inner);
            // Joined: nothing of the inner pool is running by the time the drop returns.
            live.load(Ordering::Acquire)
        }))
    };
    assert_eq!(live_after_drop.unwrap(), 0);
    outer.shutdown();
}

/// `shutdown` called on one of the executor's own workers cannot join the thread it is on.
/// It does the rest and returns; the workers exit by themselves.
#[test]
fn shutdown_on_an_own_worker_returns_without_joining_itself() {
    let live = Arc::new(AtomicUsize::new(0));
    let platform = Arc::new(LiveThreads { live: live.clone() });
    let executor = Executor::builder().platform(platform).build().unwrap();
    let returned = Arc::new(AtomicBool::new(false));
    let pending_task = executor.spawn(WorkerClass::Background, pending::<()>());
    {
        let (handle, returned) = (executor.clone(), returned.clone());
        executor
            .spawn(WorkerClass::FrameCritical, async move {
                handle.shutdown();
                returned.store(true, Ordering::Release);
            })
            .detach();
    }
    wait_for(&returned);
    // Everything else about shutdown happened: waiting futures are answered...
    assert_eq!(executor.block_on(pending_task).unwrap_err().kind(), ErrorKind::Cancelled);
    // ...and the threads go away on their own.
    let start = Instant::now();
    while live.load(Ordering::Acquire) != 0 {
        assert!(start.elapsed() < Duration::from_secs(60), "the workers never exited");
        std::thread::yield_now();
    }
}
