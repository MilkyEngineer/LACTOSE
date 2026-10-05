//! Stackless tasks, timers, worker classes, `block_on` and shutdown.

#![cfg(not(feature = "loom"))]

mod common;

use core::future::pending;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
    let outer = executor.spawn(WorkerClass::FrameCritical, async move { inner.await + 1 });
    assert_eq!(executor.block_on(outer), 21);
    // `block_on` drives a plain future too, and tasks spawned from inside it.
    let spawner = executor.clone();
    let value = executor.block_on(async move {
        let task = spawner.spawn(WorkerClass::FrameCritical, async { "spawned inside" });
        yield_now().await;
        task.await
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
        tasks.into_iter().map(|task| executor.block_on(task)).collect::<Vec<_>>()
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
    executor.block_on(executor.run_graph(graph(vec![node], &[])));
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
        gate.await;
        first.await;
        second.await;
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
    executor.block_on(blocker);
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
    assert!(executor.block_on(task).iter().all(|&tag| tag == AllocTag::Net));
    // The tag does not leak to the next task the worker polls.
    assert_eq!(executor.block_on(untagged), AllocTag::Untagged);
    executor.shutdown();
}

#[test]
fn sleep_until_resolves_at_its_deadline() {
    let executor = executor(2, 1);
    let delay = Duration::from_millis(20);

    // From the thread inside `block_on`, with every worker asleep.
    let start = Instant::now();
    executor.block_on(executor.sleep_until(start + delay));
    assert!(start.elapsed() >= delay);
    // A deadline in the past resolves at once.
    executor.block_on(executor.sleep_until(start));

    // From tasks, with deadlines registered out of order; a later timer does not hold an
    // earlier one back, and a dropped timer does not fire.
    let order = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();
    let tasks: Vec<_> = [3u32, 1, 2]
        .into_iter()
        .map(|rank| {
            let (sleeper, order) = (executor.clone(), order.clone());
            executor.spawn(WorkerClass::FrameCritical, async move {
                sleeper.sleep_until(start + delay * rank).await;
                order.lock().unwrap().push(rank);
                start.elapsed()
            })
        })
        .collect();
    let abandoned = {
        let sleeper = executor.clone();
        executor.spawn(WorkerClass::FrameCritical, async move {
            sleeper.sleep_until(start + delay / 2).await;
            unreachable!("this task is cancelled before its deadline");
        })
    };
    drop(abandoned);
    for (task, rank) in tasks.into_iter().zip([3u32, 1, 2]) {
        assert!(executor.block_on(task) >= delay * rank);
    }
    assert_eq!(*order.lock().unwrap(), [1, 2, 3]);
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
        sleeper.sleep_until(start + delay).await;
        start.elapsed()
    });
    let busy = Node::new("tasks::busy").run(move |_, _| common::spin_for(delay * 2)).build();
    executor.block_on(executor.run_graph(graph(vec![busy], &[])));
    assert!(executor.block_on(timer) >= delay);
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
            sleeper.sleep_until(Instant::now() + Duration::from_secs(3600)).await;
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
