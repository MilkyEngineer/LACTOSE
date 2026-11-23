//! Jobs of `Executor::run_job`: where they run, what the future says, shutdown, the gate's
//! reservation, who drops the value, a job queued as the workers go idle, a child that
//! crosses to another worker, and the figures.

#![cfg(not(feature = "loom"))]

use crate::common;

use core::cell::Cell;
use core::pin::pin;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{LIMIT, Node, executor, graph, must_finish, wait_for};
use lactose_core::ErrorKind;
use lactose_task::{Executor, JobRun, WorkerClass};

thread_local! {
    /// Set on a thread while it is inside a call of `run_job`.
    static IN_CALL: Cell<bool> = const { Cell::new(false) };
}

fn thread_name() -> String {
    std::thread::current().name().unwrap_or_default().to_owned()
}

/// Where a job ran: its thread's name, and whether that thread was inside the call that
/// queued it.
type Ran = Arc<Mutex<Vec<(String, bool)>>>;

fn call_run_job(executor: &Executor, ran: &Ran) -> JobRun<()> {
    let ran = ran.clone();
    IN_CALL.with(|in_call| in_call.set(true));
    let run = executor.run_job(move |_| {
        ran.lock().unwrap().push((thread_name(), IN_CALL.with(Cell::get)));
    });
    IN_CALL.with(|in_call| in_call.set(false));
    run
}

/// From a thread that is no worker, from a background task and from a graph job on a
/// frame-critical worker: the job runs on a frame-critical worker, and never inside the
/// call that queued it, even when the caller is one.
#[test]
fn a_job_runs_on_a_frame_worker_and_never_inside_the_call() {
    let executor = executor(2, 1);
    let ran = Ran::default();
    let rounds = if cfg!(miri) { 2 } else { 50 };
    for _ in 0..rounds {
        executor.block_on(call_run_job(&executor, &ran)).unwrap();

        let (spawner, background_ran) = (executor.clone(), ran.clone());
        let task = executor.spawn(WorkerClass::Background, async move {
            call_run_job(&spawner, &background_ran).await
        });
        executor.block_on(task).unwrap().unwrap();

        let queued = Arc::new(Mutex::new(None));
        let node = {
            let (caller, ran, queued) = (executor.clone(), ran.clone(), queued.clone());
            Node::new("jobs::caller")
                .run(move |_, _| *queued.lock().unwrap() = Some(call_run_job(&caller, &ran)))
                .build()
        };
        executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
        let run = queued.lock().unwrap().take().expect("the node queued a job");
        executor.block_on(run).unwrap();
    }
    let ran = ran.lock().unwrap();
    assert_eq!(ran.len(), 3 * rounds);
    for (name, in_call) in ran.iter() {
        assert!(name.starts_with("whisk-frame-"), "a job ran on {name}");
        assert!(!in_call, "a job ran inside the call that queued it, on {name}");
    }
    drop(ran);
    executor.shutdown();
}

/// Counts its drops, and names the thread of the last one.
struct Tracked {
    dropped: Arc<AtomicU32>,
    on: Arc<Mutex<String>>,
}

impl Tracked {
    fn new() -> (Self, Arc<AtomicU32>, Arc<Mutex<String>>) {
        let (dropped, on) = (Arc::new(AtomicU32::new(0)), Arc::new(Mutex::new(String::new())));
        (Self { dropped: dropped.clone(), on: on.clone() }, dropped, on)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        *self.on.lock().unwrap() = thread_name();
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

/// Holds the only frame-critical worker in a graph job until released, so that what is
/// queued behind it cannot start.
fn hold_the_worker(executor: &Executor, release: &Arc<AtomicBool>) -> lactose_task::GraphRun {
    let (entered, release) = (Arc::new(AtomicBool::new(false)), release.clone());
    let node = {
        let entered = entered.clone();
        Node::new("jobs::hold")
            .run(move |_, _| {
                entered.store(true, Ordering::Release);
                wait_for(&release);
            })
            .build()
    };
    let run = executor.run_graph(graph(vec![node], &[]));
    wait_for(&entered);
    run
}

fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
    pin!(future).poll(&mut Context::from_waker(Waker::noop()))
}

/// Shutdown begins while the job is queued: the job never starts, its closure is dropped
/// on the thread that shuts the executor down, and the future says `Cancelled`.
#[test]
fn shutdown_before_the_job_starts_cancels_it_and_drops_the_closure() {
    let executor = executor(1, 1);
    let release = Arc::new(AtomicBool::new(false));
    let holding = hold_the_worker(&executor, &release);
    let (tracked, dropped, dropped_on) = Tracked::new();
    let started = Arc::new(AtomicBool::new(false));
    let run = {
        let started = started.clone();
        executor.run_job(move |_| {
            let _tracked = &tracked;
            started.store(true, Ordering::SeqCst);
        })
    };
    let stopper = std::thread::Builder::new()
        .name("jobs-stopper".to_owned())
        .spawn(move || executor.shutdown())
        .unwrap();
    // The sweep at shutdown drops the queued job while the worker is still held.
    let begun = std::time::Instant::now();
    while dropped.load(Ordering::SeqCst) == 0 {
        assert!(begun.elapsed() < LIMIT, "shutdown did not drop the queued job");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(*dropped_on.lock().unwrap(), "jobs-stopper");
    release.store(true, Ordering::Release);
    stopper.join().unwrap();

    assert!(!started.load(Ordering::SeqCst));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    match poll_once(run) {
        Poll::Ready(Err(error)) => assert_eq!(error.kind(), ErrorKind::Cancelled),
        Poll::Ready(Ok(())) => panic!("a job that never started resolved to Ok"),
        Poll::Pending => panic!("a cancelled job's future is pending"),
    }
    drop(holding);
}

/// Shutdown begins while the job runs: it finishes, and its future hands the value back. The
/// job is released only once shutdown has begun, which a job queued on a clone of the
/// handle shows by resolving to `Cancelled` at once.
#[test]
fn a_started_job_hands_its_value_back_through_shutdown() {
    let executor = executor(2, 1);
    let inert = executor.clone();
    let (started, release) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let run = {
        let (started, release) = (started.clone(), release.clone());
        executor.run_job(move |ctx| {
            started.store(true, Ordering::Release);
            wait_for(&release);
            // Its scopes still run after shutdown has begun.
            let sum = AtomicU32::new(0);
            ctx.scope(|scope| {
                scope.fork_range(0..64, |units, _| {
                    sum.fetch_add(units.len() as u32, Ordering::Relaxed);
                });
            });
            sum.into_inner()
        })
    };
    wait_for(&started);
    let stopper = std::thread::spawn(move || executor.shutdown());
    let begun = Instant::now();
    while !matches!(poll_once(inert.run_job(|_| ())), Poll::Ready(Err(_))) {
        assert!(begun.elapsed() < LIMIT, "shutdown never began");
        std::thread::sleep(Duration::from_millis(1));
    }
    release.store(true, Ordering::Release);
    stopper.join().unwrap();
    match poll_once(run) {
        Poll::Ready(Ok(sum)) => assert_eq!(sum, 64),
        Poll::Ready(Err(error)) => panic!("a started job resolved to {error}"),
        Poll::Pending => panic!("a finished job's future is pending"),
    }
}

/// While a `block_in_place` holds the gate, a job starts on a worker its reservation leaves
/// awake: on three frame-critical workers, a graph job holds the gate with one worker
/// reserved and queues a `run_job` job inside its block, which waits until the job has
/// begun. Whether a job waits at a closed gate with no reservation is the executor's unit
/// test `a_job_waits_while_the_gate_holds_its_workers`.
#[test]
fn a_job_starts_within_the_reservation() {
    let executor = executor(3, 1);
    let rounds = if cfg!(miri) { 2 } else { 20 };
    for _ in 0..rounds {
        let started = Arc::new(AtomicBool::new(false));
        let inside = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(Mutex::new(None));
        let node = {
            let (caller, queued, started, inside) =
                (executor.clone(), queued.clone(), started.clone(), inside.clone());
            Node::new("jobs::reservation")
                .run(move |_, ctx| {
                    ctx.block_in_place(1, || {
                        let starts = started.clone();
                        let run = caller.run_job(move |_| starts.store(true, Ordering::SeqCst));
                        let begun = Instant::now();
                        while !started.load(Ordering::SeqCst) && begun.elapsed() < LIMIT {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        // A panic here would abort; the test thread asserts instead.
                        inside.store(started.load(Ordering::SeqCst), Ordering::SeqCst);
                        *queued.lock().unwrap() = Some(run);
                    });
                })
                .build()
        };
        let blocking = executor.clone();
        must_finish("the reserving graph", LIMIT * 2, move || {
            blocking.block_on(blocking.run_graph(graph(vec![node], &[]))).unwrap();
        });
        assert!(inside.load(Ordering::SeqCst), "the job did not start within the reservation");
        let run = queued.lock().unwrap().take().unwrap();
        executor.block_on(run).unwrap();
    }
    executor.shutdown();
}

/// A job queued while the one frame-critical worker is on its way to sleep is taken. The
/// caller spins rather than parks, so it queues each job within microseconds of the last
/// one's end, and the gaps sweep the window between the worker's last look for work, after
/// its scan, and its announcement: only the look before it sleeps sees a job queued there,
/// since a wake finds nobody to claim.
#[test]
#[cfg_attr(miri, ignore = "sweeps a window of real time")]
fn a_job_queued_as_the_workers_go_idle_is_taken() {
    let executor = executor(1, 1);
    for job in 0..5_000u32 {
        let taken = Arc::new(AtomicBool::new(false));
        let run = {
            let taken = taken.clone();
            executor.run_job(move |_| taken.store(true, Ordering::Release))
        };
        let queued = Instant::now();
        while !taken.load(Ordering::Acquire) {
            assert!(queued.elapsed() < LIMIT, "job {job} was never taken: a lost wake-up");
            core::hint::spin_loop();
        }
        drop(run);
        // 40 to 80 µs in steps of 10 ns, about the scan of 50 µs.
        let gap = Duration::from_nanos(40_000 + u64::from(job) * 7_919 % 4_000 * 10);
        let ended = Instant::now();
        while ended.elapsed() < gap {
            core::hint::spin_loop();
        }
    }
    executor.shutdown();
}

/// A child of a job's scope runs on another worker than its opener, through the pool's
/// queue for such work, small enough for Miri. The opener writes the input before it forks
/// and reads what the children wrote after the scope without atomics, so Miri checks that
/// the entry's publication and the latch carry them. The opener's first child waits, a
/// bounded while, until another worker has run one.
#[test]
fn a_child_crosses_to_another_worker() {
    let executor = executor(3, 1);
    assert_eq!(executor.run_job_width(), 2);
    let rounds = if cfg!(miri) { 3 } else { 200 };
    let mut crossed = 0;
    for round in 0..rounds {
        let run = executor.run_job(move |ctx| {
            let own = ctx.slot();
            let input: Vec<u64> = (0..4).map(|i| i * 3 + round).collect();
            let mut out = [0u64; 4];
            let mut rows = [0u64; 6];
            let elsewhere = AtomicU32::new(0);
            ctx.scope(|scope| {
                let (input, elsewhere) = (&input, &elsewhere);
                for (i, out) in out.iter_mut().enumerate() {
                    scope.fork(move |child| {
                        if child.slot() != own {
                            elsewhere.fetch_add(1, Ordering::Relaxed);
                        } else if i == 0 {
                            for _ in 0..2_000 {
                                if elsewhere.load(Ordering::Relaxed) > 0 {
                                    break;
                                }
                                std::thread::yield_now();
                            }
                        }
                        *out = input[i] * 2;
                    });
                }
                for (i, row) in rows.iter_mut().enumerate() {
                    scope.fork(move |child| {
                        if child.slot() != own {
                            elsewhere.fetch_add(1, Ordering::Relaxed);
                        }
                        *row = input[i % 4] + 1;
                    });
                }
                scope.fork_range(0..6, move |range, child| {
                    if child.slot() != own {
                        elsewhere.fetch_add(1, Ordering::Relaxed);
                    }
                    for unit in range {
                        assert!(input[unit as usize % 4] >= round);
                    }
                });
            });
            for (i, out) in out.iter().enumerate() {
                assert_eq!(*out, input[i] * 2);
            }
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(*row, input[i % 4] + 1);
            }
            elsewhere.into_inner()
        });
        crossed += executor.block_on(run).unwrap();
    }
    assert!(crossed > 0, "no child ran on another worker than its opener");
    executor.shutdown();
}

/// The future dropped while the job runs: the worker drops the value when the closure
/// returns.
#[test]
fn a_value_nobody_waits_for_is_dropped_by_the_worker() {
    let executor = executor(2, 1);
    let release = Arc::new(AtomicBool::new(false));
    let (tracked, dropped, dropped_on) = Tracked::new();
    let run = {
        let release = release.clone();
        executor.run_job(move |_| {
            wait_for(&release);
            tracked
        })
    };
    drop(run);
    release.store(true, Ordering::Release);
    let begun = std::time::Instant::now();
    while dropped.load(Ordering::SeqCst) == 0 {
        assert!(begun.elapsed() < LIMIT, "the value was never dropped");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(dropped_on.lock().unwrap().starts_with("whisk-frame-"));
    executor.shutdown();
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

/// The future dropped after the job finished: the dropping thread drops the value.
#[test]
fn a_finished_jobs_value_is_dropped_with_its_future() {
    let executor = executor(2, 1);
    let (tracked, dropped, dropped_on) = Tracked::new();
    let first = executor.run_job(move |_| tracked);
    // Jobs run one after the other, and a job's place is given up only after its value is
    // handed over: once the second begins, the first has finished.
    let second_began = Arc::new(AtomicBool::new(false));
    let second = {
        let second_began = second_began.clone();
        executor.run_job(move |_| second_began.store(true, Ordering::Release))
    };
    wait_for(&second_began);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(first);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(*dropped_on.lock().unwrap(), thread_name());
    executor.block_on(second).unwrap();
    executor.shutdown();
}

#[test]
#[should_panic(expected = "a job run is not polled after it resolved")]
fn a_job_run_polled_after_it_resolved_panics() {
    let executor = executor(1, 1);
    let mut run = pin!(executor.run_job(|_| 3));
    let mut context = Context::from_waker(Waker::noop());
    let begun = std::time::Instant::now();
    while run.as_mut().poll(&mut context).is_pending() {
        assert!(begun.elapsed() < LIMIT);
        std::thread::yield_now();
    }
    let _ = run.as_mut().poll(&mut context);
}

/// A fan-out of a figure: what one job's scope forks.
#[derive(Clone, Copy)]
enum FanOut {
    /// `count` children of `each`.
    Children { count: u32, each: Duration },
    /// One range of `units` units of `each`.
    Range { units: u32, each: Duration },
}

impl FanOut {
    fn run(self, ctx: &mut lactose_task::JobContext<'_>, began: &AtomicBool) {
        ctx.scope(|scope| match self {
            FanOut::Children { count, each } => {
                for _ in 0..count {
                    scope.fork(move |_| {
                        began.store(true, Ordering::Release);
                        common::spin_for(each);
                    });
                }
            }
            FanOut::Range { units, each } => {
                scope.fork_range(0..units, move |range, _| {
                    began.store(true, Ordering::Release);
                    for _ in range {
                        common::spin_for(each);
                    }
                });
            }
        });
    }
}

/// The lateness of a frame-critical task's `sleep_until`, set 2 ms ahead once the fan-out
/// has begun, `samples` times: the fan-out inside a job of `run_job` when `in_job`, else
/// inside a graph job, which no width bounds.
fn lateness(executor: &Executor, fan_out: FanOut, in_job: bool, samples: usize) -> Vec<f64> {
    let mut late = Vec::with_capacity(samples);
    for _ in 0..samples {
        let began = Arc::new(AtomicBool::new(false));
        let work: Box<dyn FnOnce() + Send> = if in_job {
            let began = began.clone();
            let run = executor.run_job(move |ctx| fan_out.run(ctx, &began));
            let blocking = executor.clone();
            Box::new(move || blocking.block_on(run).unwrap())
        } else {
            let began = began.clone();
            let node =
                Node::new("jobs::fan_out").run(move |_, ctx| fan_out.run(ctx, &began)).build();
            let run = executor.run_graph(graph(vec![node], &[]));
            let blocking = executor.clone();
            Box::new(move || drop(blocking.block_on(run).unwrap()))
        };
        wait_for(&began);
        let sleeper = executor.clone();
        let task = executor.spawn(WorkerClass::FrameCritical, async move {
            let deadline = std::time::Instant::now() + Duration::from_millis(2);
            sleeper.sleep_until(deadline).await.unwrap();
            std::time::Instant::now().saturating_duration_since(deadline)
        });
        late.push(executor.block_on(task).unwrap().as_secs_f64() * 1e3);
        work();
    }
    late.sort_by(f64::total_cmp);
    late
}

/// Figures with no threshold: how late a timed wake set inside a fan-out that fills the
/// width is, in the three shapes of D162's review, beside the same fan-out in a graph job.
/// `cargo test -p lactose-task --release --test integration -- --ignored --nocapture
/// jobs::timed_wake_inside_a_fan_out`, on a quiet machine.
#[test]
#[ignore = "figures, on a quiet machine"]
fn timed_wake_inside_a_fan_out() {
    let executor = Executor::builder().build().unwrap();
    let samples = 200;
    let shapes = [
        ("4 children of 10 ms", FanOut::Children { count: 4, each: Duration::from_millis(10) }),
        ("2000 units of 20 us", FanOut::Range { units: 2000, each: Duration::from_micros(20) }),
        ("40 children of 1 ms", FanOut::Children { count: 40, each: Duration::from_millis(1) }),
    ];
    println!(
        "{} frame-critical workers, width {}, {samples} samples a row, lateness in ms",
        executor.frame_worker_count(),
        executor.run_job_width()
    );
    for (name, fan_out) in shapes {
        for (in_job, under) in [(true, "run_job"), (false, "a graph job")] {
            let late = lateness(&executor, fan_out, in_job, samples);
            let at = |q: f64| late[((late.len() - 1) as f64 * q).round() as usize];
            println!(
                "{name}, in {under}: P50 {:.3}, P99 {:.3}, max {:.3}",
                at(0.5),
                at(0.99),
                late[late.len() - 1]
            );
        }
    }
    executor.shutdown();
}

/// How long the fan-out itself takes, from its scope's start to its end, `samples` times:
/// inside a job of `run_job` when `in_job`, else inside a graph job.
fn fan_out_time(executor: &Executor, fan_out: FanOut, in_job: bool, samples: usize) -> Vec<f64> {
    let mut took = Vec::with_capacity(samples);
    let timed = move |ctx: &mut lactose_task::JobContext<'_>| {
        let begun = Instant::now();
        fan_out.run(ctx, &AtomicBool::new(false));
        begun.elapsed()
    };
    for _ in 0..samples {
        let time = if in_job {
            executor.block_on(executor.run_job(timed)).unwrap()
        } else {
            let cell = Arc::new(Mutex::new(Duration::ZERO));
            let out = cell.clone();
            let node = Node::new("jobs::fan_out")
                .run(move |_, ctx| *out.lock().unwrap() = timed(ctx))
                .build();
            drop(executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap());
            *cell.lock().unwrap()
        };
        took.push(time.as_secs_f64() * 1e3);
    }
    took.sort_by(f64::total_cmp);
    took
}

/// Figures with no threshold: how long a fan-out takes inside a job of `run_job`, whose
/// width leaves one frame-critical worker out, beside the same fan-out in a graph job, in
/// the shapes of D162's review. Where `timed_wake_inside_a_fan_out` times a timer, this
/// times the fan-out's throughput under the width.
/// `cargo test -p lactose-task --release --test integration -- --ignored --nocapture
/// jobs::fan_out_time_under_the_width`, on a quiet machine.
#[test]
#[ignore = "figures, on a quiet machine"]
fn fan_out_time_under_the_width() {
    let executor = Executor::builder().build().unwrap();
    let samples = 100;
    let shapes = [
        ("4 children of 10 ms", FanOut::Children { count: 4, each: Duration::from_millis(10) }),
        ("3 children of 10 ms", FanOut::Children { count: 3, each: Duration::from_millis(10) }),
        ("2000 units of 20 us", FanOut::Range { units: 2000, each: Duration::from_micros(20) }),
        ("40 children of 1 ms", FanOut::Children { count: 40, each: Duration::from_millis(1) }),
    ];
    println!(
        "{} frame-critical workers, width {}, {samples} samples a row, fan-out time in ms",
        executor.frame_worker_count(),
        executor.run_job_width()
    );
    for (name, fan_out) in shapes {
        let job = fan_out_time(&executor, fan_out, true, samples);
        let graph = fan_out_time(&executor, fan_out, false, samples);
        let at = |took: &[f64], q: f64| took[((took.len() - 1) as f64 * q).round() as usize];
        println!(
            "{name}: run_job P50 {:.3}, P99 {:.3}; a graph job P50 {:.3}, P99 {:.3}; \
             ratio of the P50s {:.3}",
            at(&job, 0.5),
            at(&job, 0.99),
            at(&graph, 0.5),
            at(&graph, 0.99),
            at(&job, 0.5) / at(&graph, 0.5)
        );
    }
    executor.shutdown();
}
