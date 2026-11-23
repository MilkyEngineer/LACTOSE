//! The width of [`Executor::run_job`]: what only the pool's own counts can show. Every
//! assertion is about an order or a count; every wait is bounded by a thread of the test's
//! own, so a regression fails with a message instead of hanging.

use core::ops::Range;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{Executor, GraphBuilder, GraphNode, JobContext, WorkerClass};

/// How long a wait may last that only a regression runs out.
const LIMIT: Duration = Duration::from_secs(if cfg!(miri) { 600 } else { 60 });

fn executor(frame: u32) -> Executor {
    Executor::builder().frame_workers(frame).background_workers(1).build().unwrap()
}

/// Waits until `done` holds, failing the test with `what` after [`LIMIT`].
fn wait_until(what: &str, done: impl Fn() -> bool) {
    let begun = Instant::now();
    while !done() {
        assert!(begun.elapsed() < LIMIT, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Runs `f` on a helper thread, failing the test with `what` if it does not return within
/// [`LIMIT`].
fn within<R: Send + 'static>(what: &str, f: impl FnOnce() -> R + Send + 'static) -> R {
    let (done, wait) = mpsc::channel();
    let runner = std::thread::spawn(move || {
        let _ = done.send(f());
    });
    match wait.recv_timeout(LIMIT) {
        Ok(result) => {
            runner.join().unwrap();
            result
        }
        Err(RecvTimeoutError::Disconnected) => {
            std::panic::resume_unwind(runner.join().expect_err("`f` returned without sending"))
        }
        Err(RecvTimeoutError::Timeout) => panic!("{what} did not finish in {LIMIT:?}"),
    }
}

struct Nothing;

impl GraphNode for Nothing {
    fn name(&self) -> &'static str {
        "executor::job_tests::nothing"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, _ctx: &mut JobContext<'_>) {}
}

/// A task's `sleep_until` resolves and a graph run started meanwhile completes: both need a
/// frame-critical worker that is in no job of `run_job` and no child of one.
fn a_free_worker_serves_a_timer_and_a_graph(executor: &Executor) {
    let sleeper = executor.clone();
    let task = executor.spawn(WorkerClass::FrameCritical, async move {
        sleeper.sleep_until(Instant::now() + Duration::from_millis(2)).await
    });
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(Nothing));
    let run = executor.run_graph(builder.build().unwrap());
    let executor = executor.clone();
    within("a timer and a graph beside a full width", move || {
        executor.block_on(task).unwrap().unwrap();
        executor.block_on(run).unwrap();
    });
}

/// Children held until released: how many are in one at once, how many there were, and
/// the slots they ran under.
#[derive(Default)]
struct Held {
    release: AtomicBool,
    running: AtomicU32,
    most: AtomicU32,
    ran: AtomicU32,
    slots: Mutex<Vec<u32>>,
}

impl Held {
    fn hold(&self, slot: u32) {
        self.slots.lock().unwrap().push(slot);
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.running.fetch_sub(1, Ordering::SeqCst);
        self.ran.fetch_add(1, Ordering::SeqCst);
    }
}

/// Releases the held children when the test ends, so that a failing assertion fails the
/// test instead of leaving the executor's drop waiting for them.
struct Release(Arc<Held>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.release.store(true, Ordering::SeqCst);
    }
}

/// A job whose scope forks `frame_worker_count()` children and one more, each held where it
/// begins. They fill the width and no more: the job's worker holds one, every other place
/// one each, and the last frame-critical worker fires the timer and runs the graph while
/// they are held. Given a width of every worker, that last worker would hold a child too.
/// Returns the job's slot and the slots its children ran under.
fn children_beyond_the_width(workers: u32) -> (u32, Vec<u32>) {
    let executor = executor(workers);
    let width = executor.run_job_width();
    assert_eq!(width, workers - 1);
    let held = Arc::new(Held::default());
    let _release = Release(held.clone());
    let run = {
        let held = held.clone();
        executor.run_job(move |ctx| {
            ctx.scope(|scope| {
                for _ in 0..=workers {
                    scope.fork(|child| held.hold(child.slot()));
                }
            });
            ctx.slot()
        })
    };
    wait_until("the width is full", || held.running.load(Ordering::SeqCst) >= width);
    a_free_worker_serves_a_timer_and_a_graph(&executor);
    assert_eq!(held.running.load(Ordering::SeqCst), width, "a child began beyond the width");
    assert_eq!(executor.owner.shared.wide.width.occupied(), width as usize);

    held.release.store(true, Ordering::SeqCst);
    let blocking = executor.clone();
    let slot = within("the job", move || blocking.block_on(run)).unwrap();
    assert_eq!(held.ran.load(Ordering::SeqCst), workers + 1);
    assert_eq!(held.most.load(Ordering::SeqCst), width);
    assert_eq!(executor.owner.shared.wide.width.occupied(), 0);
    executor.shutdown();
    let slots = core::mem::take(&mut *held.slots.lock().unwrap());
    (slot, slots)
}

#[test]
#[cfg_attr(miri, ignore = "holds workers for real time")]
fn children_beyond_the_width_leave_a_worker_free() {
    children_beyond_the_width(4);
}

/// The same on two frame-critical workers, where the width is one: every child runs on the
/// job's own worker while the other fires the timer and runs the graph.
#[test]
#[cfg_attr(miri, ignore = "holds workers for real time")]
fn a_width_of_one_runs_the_children_on_the_jobs_worker() {
    let (slot, slots) = children_beyond_the_width(2);
    assert_eq!(slots.len(), 3);
    assert!(slots.iter().all(|&child| child == slot), "children ran under {slots:?}, not {slot}");
}

/// A width of one with a range long enough to be split many times: a half never goes to
/// the other worker.
#[test]
fn a_width_of_one_never_splits_a_range_to_another_worker() {
    let executor = executor(2);
    let units = if cfg!(miri) { 8 } else { 512 };
    let run = executor.run_job(move |ctx| {
        let own = ctx.slot();
        let (elsewhere, ran) = (AtomicU32::new(0), AtomicU32::new(0));
        for _ in 0..2 {
            ctx.scope(|scope| {
                let (elsewhere, ran) = (&elsewhere, &ran);
                scope.fork_range(0..units, move |range, child| {
                    if child.slot() != own {
                        elsewhere.fetch_add(1, Ordering::Relaxed);
                    }
                    // Long enough per unit that a thief would have time to take a half.
                    let begun = Instant::now();
                    while begun.elapsed() < Duration::from_micros(range.len() as u64 * 5) {
                        core::hint::spin_loop();
                    }
                    ran.fetch_add(range.len() as u32, Ordering::Relaxed);
                });
            });
        }
        (elsewhere.into_inner(), ran.into_inner())
    });
    let blocking = executor.clone();
    let (elsewhere, ran) = within("the job", move || blocking.block_on(run)).unwrap();
    assert_eq!(elsewhere, 0, "a part of the range ran on another worker");
    assert_eq!(ran, 2 * units);
    executor.shutdown();
}

/// What the parked-opener test records.
#[derive(Default)]
struct Parked {
    held: Arc<Held>,
    /// The scope's body saw every child begin on another worker before it returned.
    all_began_elsewhere: AtomicBool,
    opener: AtomicU32,
    second_began: AtomicBool,
    /// Set by the second job if it began before the first one's children were released.
    second_began_early: AtomicBool,
}

/// The opener parked on its latch, on four frame-critical workers: the scope forks one
/// child fewer than the width, and each has begun on another worker, where it is held,
/// before the scope's body returns. The opener finds none left to run and parks. A second
/// `run_job` job started once it has parked does not begin before the children are
/// released, and the free worker fires a timer and runs a graph meanwhile.
#[test]
#[cfg_attr(miri, ignore = "holds workers for real time")]
fn an_opener_parks_beside_a_full_width_and_the_next_job_waits() {
    const WORKERS: u32 = 4;
    let executor = executor(WORKERS);
    let width = executor.run_job_width();
    let parked = Arc::new(Parked::default());
    let _release = Release(parked.held.clone());
    let first = {
        let parked = parked.clone();
        executor.run_job(move |ctx| {
            let own = ctx.slot();
            parked.opener.store(own, Ordering::SeqCst);
            let held = &*parked.held;
            ctx.scope(|scope| {
                for _ in 0..width - 1 {
                    scope.fork(|child| held.hold(child.slot()));
                }
                // The body waits, so that no child is left for the opener to run.
                wait_until("the children began", || {
                    held.running.load(Ordering::SeqCst) == width - 1
                });
                let elsewhere = held.slots.lock().unwrap().iter().all(|&slot| slot != own);
                parked.all_began_elsewhere.store(elsewhere, Ordering::SeqCst);
            });
        })
    };
    let frame = executor.owner.shared.frame();
    wait_until("the opener is parked", || frame.parked_openers() == 1);
    assert!(parked.all_began_elsewhere.load(Ordering::SeqCst));
    // The parked opener keeps its job's place: one given up while it sleeps would let a
    // worker without a place into the job, and no worker would be left for the timers.
    let occupied = executor.owner.shared.wide.width.occupied();
    assert_eq!(occupied, width as usize, "the parked opener gave its job's place up");
    let second = {
        let parked = parked.clone();
        executor.run_job(move |_| {
            // A panic in a job aborts; the test thread asserts instead.
            let early = !parked.held.release.load(Ordering::SeqCst);
            parked.second_began_early.store(early, Ordering::SeqCst);
            parked.second_began.store(true, Ordering::SeqCst);
        })
    };
    a_free_worker_serves_a_timer_and_a_graph(&executor);
    assert_eq!(frame.parked_openers(), 1);
    assert!(!parked.second_began.load(Ordering::SeqCst), "a job began beside another");

    parked.held.release.store(true, Ordering::SeqCst);
    let blocking = executor.clone();
    within("both jobs", move || {
        blocking.block_on(first).unwrap();
        blocking.block_on(second).unwrap();
    });
    assert!(parked.second_began.load(Ordering::SeqCst));
    assert!(!parked.second_began_early.load(Ordering::SeqCst), "a job began beside another");
    assert_eq!(parked.held.ran.load(Ordering::SeqCst), width - 1);
    assert_eq!(parked.held.most.load(Ordering::SeqCst), width - 1);
    executor.shutdown();
}

/// One frame-critical worker: the width is one, and that worker runs the job, its children
/// and then a timer and a graph.
#[test]
fn one_worker_runs_a_job_and_its_children_itself() {
    let executor = executor(1);
    assert_eq!(executor.run_job_width(), 1);
    let run = executor.run_job(|ctx| {
        let own = ctx.slot();
        let mut slots = [u32::MAX; 4];
        ctx.scope(|scope| {
            for slot in &mut slots {
                scope.fork(move |child| *slot = child.slot());
            }
        });
        slots.iter().all(|&slot| slot == own)
    });
    let blocking = executor.clone();
    assert!(within("the job", move || blocking.block_on(run)).unwrap());
    a_free_worker_serves_a_timer_and_a_graph(&executor);
    executor.shutdown();
}

/// The job flag alone: on four frame-critical workers a first job with no children holds
/// one place of three, so the count leaves room, and a second job queued beside it has not
/// begun once every other frame-critical worker is asleep again. Queueing it wakes nobody,
/// since no worker without a place may take it; without the flag one would wake and run it.
#[test]
#[cfg_attr(miri, ignore = "holds workers for real time")]
fn a_second_job_waits_with_room_in_the_width() {
    let executor = executor(4);
    let held = Arc::new(Held::default());
    let _release = Release(held.clone());
    let first = {
        let held = held.clone();
        executor.run_job(move |ctx| held.hold(ctx.slot()))
    };
    wait_until("the first job began", || held.running.load(Ordering::SeqCst) == 1);
    let frame = executor.owner.shared.frame();
    wait_until("the other workers sleep", || frame.sleepers() == 3);
    assert_eq!(executor.owner.shared.wide.width.occupied(), 1);
    let began = Arc::new(AtomicBool::new(false));
    let second = {
        let began = began.clone();
        executor.run_job(move |_| began.store(true, Ordering::SeqCst))
    };
    wait_until("the other workers sleep again", || frame.sleepers() == 3);
    assert!(!began.load(Ordering::SeqCst), "a job began beside another");
    held.release.store(true, Ordering::SeqCst);
    let blocking = executor.clone();
    within("both jobs", move || {
        blocking.block_on(first).unwrap();
        blocking.block_on(second).unwrap();
    });
    assert!(began.load(Ordering::SeqCst));
    executor.shutdown();
}

/// A graph job that holds the gate with no permit reserved and queues a `run_job` job
/// inside its block.
struct Gated {
    executor: Executor,
    started: Arc<AtomicBool>,
    /// Whether the job had begun, and whether it was still queued, once the other worker
    /// had been woken for it and was asleep again; and the job's future.
    seen: Mutex<Option<(bool, bool, crate::JobRun<()>)>>,
}

impl GraphNode for Gated {
    fn name(&self) -> &'static str {
        "executor::job_tests::gated"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, ctx: &mut JobContext<'_>) {
        ctx.block_in_place(0, || {
            let shared = &self.executor.owner.shared;
            let frame = shared.frame();
            wait_until("the other worker sleeps", || frame.sleepers() == 1);
            let started = self.started.clone();
            let run = self.executor.run_job(move |_| started.store(true, Ordering::SeqCst));
            // Queueing the job claimed the sleeper to wake it; it finds no permit and
            // announces itself again.
            wait_until("the other worker sleeps again", || frame.sleepers() == 1);
            let began = self.started.load(Ordering::SeqCst);
            let queued = crate::width::Queued::jobs(&shared.wide);
            *self.seen.lock().unwrap() = Some((began, queued, run));
        });
    }
}

/// A job queued while a `block_in_place` keeps the other worker parked does not start
/// while the gate is closed: that worker is woken for it, finds no permit and sleeps again,
/// the job still queued. A worker that takes one at a closed gate puts it back unstarted,
/// which `pool::tests` shows by handing it one.
#[test]
fn a_job_waits_while_the_gate_holds_its_workers() {
    let executor = executor(2);
    let rounds = if cfg!(miri) { 2 } else { 20 };
    for _ in 0..rounds {
        let node = Arc::new(Gated {
            executor: executor.clone(),
            started: Arc::new(AtomicBool::new(false)),
            seen: Mutex::new(None),
        });
        let mut builder = GraphBuilder::new();
        builder.add_node(node.clone());
        let blocking = executor.clone();
        let graph = builder.build().unwrap();
        within("the gated graph", move || blocking.block_on(blocking.run_graph(graph))).unwrap();
        let (began, queued, run) = node.seen.lock().unwrap().take().unwrap();
        assert!(!began, "a job began at a closed gate");
        assert!(queued, "a job left the queue at a closed gate");
        let blocking = executor.clone();
        within("the job after the gate opened", move || blocking.block_on(run)).unwrap();
        assert!(node.started.load(Ordering::SeqCst));
    }
    executor.shutdown();
}

/// No unit of the range has run yet.
const NOBODY: u32 = u32::MAX;

/// A range in a job's scope reaches a second worker by the split rule: the first unit to
/// run waits until a unit has run on another worker. A range that is never split runs all
/// of its units on one worker, and the wait runs out.
#[test]
fn a_range_in_a_job_splits_to_a_second_worker() {
    let executor = executor(3);
    assert_eq!(executor.run_job_width(), 2);
    let (rounds, units) = if cfg!(miri) { (2, 8) } else { (20, 64) };
    for round in 0..rounds {
        let run = executor.run_job(move |ctx| {
            let first = AtomicU32::new(NOBODY);
            let crossed = AtomicBool::new(false);
            ctx.scope(|scope| {
                let (first, crossed) = (&first, &crossed);
                scope.fork_range(0..units, move |_range, child| {
                    let slot = child.slot();
                    match first.compare_exchange(NOBODY, slot, Ordering::SeqCst, Ordering::SeqCst) {
                        Ok(_) => {
                            let begun = Instant::now();
                            while !crossed.load(Ordering::SeqCst) && begun.elapsed() < LIMIT {
                                std::thread::yield_now();
                            }
                        }
                        Err(owner) if owner != slot => crossed.store(true, Ordering::SeqCst),
                        Err(_) => {}
                    }
                });
            });
            crossed.into_inner()
        });
        let blocking = executor.clone();
        let crossed = within("the job", move || blocking.block_on(run)).unwrap();
        assert!(crossed, "round {round}: the range ran on one worker");
    }
    executor.shutdown();
}

/// An opener that parked takes the halves a holder queues after it parked. Units on the
/// opener take no time and units elsewhere 1 ms each, so the opener runs out of work and
/// parks while a holder still runs its part; each half that holder splits off then reaches
/// the opener only through its look after the wake. With that look the holder runs about as
/// many units as the opener takes milliseconds to wake; without it, the opener stays parked
/// and the holder runs at least the half it took.
#[test]
#[cfg_attr(miri, ignore = "times units in real time")]
fn an_opener_takes_a_half_queued_after_it_parked() {
    const UNITS: u32 = 256;
    let executor = executor(3);
    assert_eq!(executor.run_job_width(), 2);
    let mut most = 0;
    for _ in 0..20 {
        let run = executor.run_job(move |ctx| {
            let own = ctx.slot();
            let elsewhere = AtomicU32::new(0);
            ctx.scope(|scope| {
                let elsewhere = &elsewhere;
                scope.fork_range(0..UNITS, move |range, child| {
                    if child.slot() != own {
                        elsewhere.fetch_add(range.len() as u32, Ordering::SeqCst);
                        let begun = Instant::now();
                        while begun.elapsed() < Duration::from_millis(1) {
                            core::hint::spin_loop();
                        }
                    }
                });
            });
            elsewhere.into_inner()
        });
        let blocking = executor.clone();
        most = most.max(within("the job", move || blocking.block_on(run)).unwrap());
    }
    assert!(most < UNITS / 4, "{most} of {UNITS} units ran off the opener: it stayed parked");
    executor.shutdown();
}
