//! A task woken while `block_in_place` holds the gate, as a render task is when its present
//! call returns during a physics step. It is polled only by a worker that holds a run permit
//! within the gate's limit: on a worker the reservation leaves awake if there is one, and
//! otherwise not before the gate opens; and its wake is not lost. The task is woken from a
//! thread that is no worker, by a job beside the block and by its timer; the wake, and the
//! gate's opening, are landed between a worker's look round and its park; and a worker that
//! takes the task over the limit puts it back and goes to the gate.
//!
//! Every assertion is about an order or a count. The orders are made by the pool's own
//! counts and by two hooks only tests compile: one a worker calls just before it parks
//! (`Gate::before_park`) and one it calls holding a permit before it looks for work
//! (`Pool::before_find`). Every wait is bounded, so a regression fails with a message
//! instead of hanging; a worker never asserts, since a panic there aborts.

use core::ops::Range;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::gate::Look;
use crate::pool::{Pool, with_local};
use crate::{Executor, GraphBuilder, GraphNode, JobContext, JobRun, Task, WorkerClass, lock};

/// How long a wait may last that only a regression runs out.
const LIMIT: Duration = Duration::from_secs(if cfg!(miri) { 600 } else { 60 });

/// Rounds per test: each round wakes one task at a closed gate.
const ROUNDS: u32 = if cfg!(miri) { 2 } else { 10 };

fn executor(frame: u32) -> Executor {
    Executor::builder().frame_workers(frame).background_workers(1).build().unwrap()
}

/// Waits on the test thread until `done` holds, failing the test with `what` after [`LIMIT`].
fn wait_until(what: &str, done: impl Fn() -> bool) {
    let begun = Instant::now();
    while !done() {
        assert!(begun.elapsed() < LIMIT, "timed out waiting until {what}");
        std::thread::yield_now();
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

/// What a worker or a hook found wrong, for the test thread to assert on.
#[derive(Default)]
struct Failures(Mutex<Vec<String>>);

impl Failures {
    /// Notes `what` unless `holds`. Returns `holds`.
    fn check(&self, holds: bool, what: &str) -> bool {
        if !holds {
            lock(&self.0).push(what.to_owned());
        }
        holds
    }

    /// Waits until `done` holds, or notes that it never did within [`LIMIT`]. Returns
    /// whether it holds.
    fn wait(&self, what: &str, done: impl Fn() -> bool) -> bool {
        let begun = Instant::now();
        while !done() {
            if begun.elapsed() >= LIMIT {
                return self.check(false, &format!("timed out waiting until {what}"));
            }
            std::thread::yield_now();
        }
        true
    }

    fn assert_none(&self) {
        let failures = lock(&self.0);
        assert!(failures.is_empty(), "{}", failures.join("; "));
    }
}

/// Where a task's second poll ran: the worker, the permits out then, and the gate's limit,
/// `None` while it is open.
type Seen = (usize, (usize, Option<usize>));

/// A frame-critical task that parks on its first poll, leaving its waker here, and finishes
/// at its second, noting where that poll ran.
#[derive(Default)]
struct Woken {
    waker: Mutex<Option<Waker>>,
    polls: AtomicU32,
    second: Mutex<Option<Seen>>,
}

impl Woken {
    /// Spawns the task and waits until it has parked.
    fn spawn(executor: &Executor) -> (Arc<Self>, Task<()>) {
        let woken = Arc::new(Self::default());
        let task = executor.spawn(WorkerClass::FrameCritical, Parks(woken.clone()));
        wait_until("the task parks", || woken.polls() == 1);
        (woken, task)
    }

    fn polls(&self) -> u32 {
        self.polls.load(Ordering::SeqCst)
    }

    fn take_waker(&self) -> Waker {
        lock(&self.waker).take().expect("the task parked once")
    }

    /// Wakes the task on the calling thread.
    fn wake(&self) {
        self.take_waker().wake();
    }

    /// Wakes the task from a thread that is no worker, as the present thread does, and
    /// returns once the wake has queued it and claimed a sleeper, if there was one.
    fn wake_from_another_thread(&self) {
        let waker = self.take_waker();
        std::thread::spawn(move || waker.wake()).join().unwrap();
    }

    fn second(&self) -> Seen {
        lock(&self.second).expect("the task's second poll ran on a worker")
    }
}

struct Parks(Arc<Woken>);

impl Future for Parks {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let woken = &self.0;
        if woken.polls() == 0 {
            *lock(&woken.waker) = Some(cx.waker().clone());
            woken.polls.store(1, Ordering::SeqCst);
            return Poll::Pending;
        }
        *lock(&woken.second) =
            with_local(|local| local.map(|local| (local.index, local.pool().permits())));
        woken.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(())
    }
}

/// A graph node whose one unit is a closure.
struct Body(Box<dyn Fn(&mut JobContext<'_>) + Send + Sync>);

impl GraphNode for Body {
    fn name(&self) -> &'static str {
        "executor::gate_tests::body"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, ctx: &mut JobContext<'_>) {
        (self.0)(ctx);
    }
}

/// Runs `body` as a graph's one unit on a frame-critical worker, and returns once the run
/// has completed.
fn run_on_a_worker(
    executor: &Executor,
    body: impl Fn(&mut JobContext<'_>) + Send + Sync + 'static,
) {
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(Body(Box::new(body))));
    let graph = builder.build().unwrap();
    let blocking = executor.clone();
    within("the blocking node", move || blocking.block_on(blocking.run_graph(graph))).unwrap();
}

/// Waits for the task to resolve.
fn finish(executor: &Executor, task: Task<()>) {
    let blocking = executor.clone();
    within("the task", move || blocking.block_on(task)).unwrap();
}

/// Clears the pool's hooks when a round ends, whether it passed or not.
struct Unhook<'a>(&'a Pool);

impl Drop for Unhook<'_> {
    fn drop(&mut self) {
        self.0.set_before_park(None);
        *lock(&self.0.before_find) = None;
    }
}

/// With no permit reserved, a task woken from a thread that is no worker is not polled
/// while the gate is closed: the wake claims the other worker, which finds no permit and
/// sleeps again, the task still queued. Once the gate opens the other worker polls it, while
/// the blocking worker stays in its job, so only the opening can have woken it.
#[test]
fn a_task_woken_from_another_thread_at_a_closed_gate_waits_for_it() {
    let executor = executor(2);
    let shared = executor.owner.shared.clone();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let blocker = Arc::new(AtomicUsize::new(usize::MAX));
        {
            let (shared, woken, failures, blocker) =
                (shared.clone(), woken.clone(), failures.clone(), blocker.clone());
            run_on_a_worker(&executor, move |ctx| {
                blocker.store(ctx.slot() as usize, Ordering::SeqCst);
                let frame = shared.frame();
                ctx.block_in_place(0, || {
                    if !failures.wait("the other worker sleeps", || frame.sleepers() == 1) {
                        return;
                    }
                    woken.wake_from_another_thread();
                    // The wake claimed the other worker; it finds no permit and sleeps again.
                    if failures.wait("the other worker sleeps again", || frame.sleepers() == 1) {
                        failures.check(woken.polls() == 1, "a task was polled at a closed gate");
                        failures
                            .check(frame.work_queued(), "a task left the queue at a closed gate");
                    }
                });
                failures.wait("the task is polled once the gate opened", || woken.polls() == 2);
            });
        }
        failures.assert_none();
        finish(&executor, task);
        let (worker, (_, limit)) = woken.second();
        assert_ne!(worker, blocker.load(Ordering::SeqCst), "the blocking worker polled the task");
        assert_eq!(limit, None, "the task was polled before the gate opened");
    }
    executor.shutdown();
}

/// With one permit reserved and both other workers asleep, a task woken from a thread that
/// is no worker is polled while the gate is closed, by the worker the wake claimed, under
/// the one permit out.
#[test]
fn a_task_woken_from_another_thread_runs_within_the_reservation() {
    let executor = executor(3);
    let shared = executor.owner.shared.clone();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let blocker = Arc::new(AtomicUsize::new(usize::MAX));
        {
            let (shared, woken, failures, blocker) =
                (shared.clone(), woken.clone(), failures.clone(), blocker.clone());
            run_on_a_worker(&executor, move |ctx| {
                blocker.store(ctx.slot() as usize, Ordering::SeqCst);
                let frame = shared.frame();
                ctx.block_in_place(1, || {
                    if !failures.wait("both other workers sleep", || frame.sleepers() == 2) {
                        return;
                    }
                    woken.wake_from_another_thread();
                    failures.wait("the task is polled at the closed gate", || woken.polls() == 2);
                });
            });
        }
        failures.assert_none();
        finish(&executor, task);
        let (worker, permits) = woken.second();
        assert_ne!(worker, blocker.load(Ordering::SeqCst), "the blocking worker polled the task");
        assert_eq!(permits, (1, Some(1)), "the task was polled without the reservation's permit");
    }
    executor.shutdown();
}

/// With one permit reserved and held by a job of `run_job` beside the block, a task the job
/// wakes is not polled by the third worker: the wake claims it, and it finds no permit and
/// sleeps again. The job's worker polls the task once the job has returned, while the gate
/// is still closed.
#[test]
fn a_task_woken_by_a_job_beside_the_block_runs_on_the_jobs_worker() {
    let executor = executor(3);
    let shared = executor.owner.shared.clone();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let job_slot = Arc::new(AtomicUsize::new(usize::MAX));
        let job: Arc<Mutex<Option<JobRun<()>>>> = Arc::default();
        {
            let (executor_, shared, woken, failures, job_slot, job) = (
                executor.clone(),
                shared.clone(),
                woken.clone(),
                failures.clone(),
                job_slot.clone(),
                job.clone(),
            );
            run_on_a_worker(&executor, move |ctx| {
                let frame = shared.frame();
                ctx.block_in_place(1, || {
                    if !failures.wait("both other workers sleep", || frame.sleepers() == 2) {
                        return;
                    }
                    let run = {
                        let (shared, woken, failures, job_slot) =
                            (shared.clone(), woken.clone(), failures.clone(), job_slot.clone());
                        executor_.run_job(move |ctx| {
                            job_slot.store(ctx.slot() as usize, Ordering::SeqCst);
                            woken.wake();
                            // The wake claimed the third worker, which finds the permit held
                            // by this one and sleeps again.
                            let frame = shared.frame();
                            if failures
                                .wait("the third worker sleeps again", || frame.sleepers() == 1)
                            {
                                failures.check(
                                    woken.polls() == 1,
                                    "a worker without a permit polled the task beside the job",
                                );
                            }
                        })
                    };
                    failures.wait("the task is polled at the closed gate", || woken.polls() == 2);
                    *lock(&job) = Some(run);
                });
            });
        }
        failures.assert_none();
        finish(&executor, task);
        let run = lock(&job).take().unwrap();
        let blocking = executor.clone();
        within("the job", move || blocking.block_on(run)).unwrap();
        assert_eq!(
            woken.second(),
            (job_slot.load(Ordering::SeqCst), (1, Some(1))),
            "the task was not polled by the job's worker under the reservation's permit"
        );
    }
    executor.shutdown();
}

/// With no permit reserved, a job of `run_job` that began before the block finishes beside
/// it, over the limit, and wakes a task while the third worker is between its look round
/// and its park. Neither the job's worker nor the third polls the task while the gate is
/// closed; once it opens one of them does, while the blocking worker stays in its job.
#[test]
fn a_task_woken_by_a_job_begun_before_the_block_waits_for_the_gate() {
    let executor = executor(3);
    let shared = executor.owner.shared.clone();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let job_slot = Arc::new(AtomicUsize::new(usize::MAX));
        let (started, blocked, in_window, woke) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let run = {
            let (woken, failures, job_slot, started, blocked, in_window, woke) = (
                woken.clone(),
                failures.clone(),
                job_slot.clone(),
                started.clone(),
                blocked.clone(),
                in_window.clone(),
                woke.clone(),
            );
            executor.run_job(move |ctx| {
                job_slot.store(ctx.slot() as usize, Ordering::SeqCst);
                started.store(true, Ordering::SeqCst);
                if failures.wait("the block begins", || blocked.load(Ordering::SeqCst))
                    && failures.wait("the third worker is between its look and its park", || {
                        in_window.load(Ordering::SeqCst)
                    })
                {
                    woken.wake();
                }
                woke.store(true, Ordering::SeqCst);
            })
        };
        let _unhook = Unhook(shared.frame());
        {
            let (shared, woken, failures, job_slot, started, blocked, in_window, woke) = (
                shared.clone(),
                woken.clone(),
                failures.clone(),
                job_slot.clone(),
                started.clone(),
                blocked,
                in_window,
                woke,
            );
            run_on_a_worker(&executor, move |ctx| {
                let frame = shared.frame();
                if !failures.wait("the job begins", || started.load(Ordering::SeqCst)) {
                    blocked.store(true, Ordering::SeqCst);
                    return;
                }
                let third = 3 - ctx.slot() as usize - job_slot.load(Ordering::SeqCst);
                ctx.block_in_place(0, || {
                    blocked.store(true, Ordering::SeqCst);
                    let hook = {
                        let (failures, in_window, woke) =
                            (failures.clone(), in_window.clone(), woke.clone());
                        Arc::new(move |worker: usize, _: Look| {
                            if worker == third && !in_window.swap(true, Ordering::SeqCst) {
                                failures
                                    .wait("the job's wake lands", || woke.load(Ordering::SeqCst));
                            }
                        })
                    };
                    frame.set_before_park(Some(hook));
                    // Brings the third worker round its idle loop to the hook.
                    frame.wake(third);
                    if failures
                        .wait("the job has woken the task and both other workers sleep", || {
                            woke.load(Ordering::SeqCst) && frame.sleepers() == 2
                        })
                    {
                        failures.check(woken.polls() == 1, "a task was polled at a closed gate");
                        failures
                            .check(frame.work_queued(), "a task left the queue at a closed gate");
                    }
                });
                failures.wait("the task is polled once the gate opened", || woken.polls() == 2);
            });
        }
        failures.assert_none();
        finish(&executor, task);
        let blocking = executor.clone();
        within("the job", move || blocking.block_on(run)).unwrap();
        let (_, (_, limit)) = woken.second();
        assert_eq!(limit, None, "the task was polled before the gate opened");
    }
    executor.shutdown();
}

/// A task woken from a thread that is no worker while the other worker is between its look
/// round and its park: the wake claims the worker, whose park then returns at once. With one
/// permit reserved it polls the task at the closed gate; with none it sleeps again, the task
/// queued, and polls it once the gate opens, the blocking worker staying in its job.
fn a_wake_between_the_look_and_the_park(reserved: u32) {
    let executor = executor(2);
    let shared = executor.owner.shared.clone();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let other = Arc::new(AtomicUsize::new(usize::MAX));
        let _unhook = Unhook(shared.frame());
        {
            let (shared, woken, failures, other) =
                (shared.clone(), woken.clone(), failures.clone(), other.clone());
            run_on_a_worker(&executor, move |ctx| {
                let frame = shared.frame();
                let worker = 1 - ctx.slot() as usize;
                other.store(worker, Ordering::SeqCst);
                let (fired, woke) =
                    (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
                ctx.block_in_place(reserved, || {
                    let hook = {
                        let (woken, fired, woke) = (woken.clone(), fired.clone(), woke.clone());
                        Arc::new(move |parking: usize, _: Look| {
                            if parking == worker && !fired.swap(true, Ordering::SeqCst) {
                                woken.wake_from_another_thread();
                                woke.store(true, Ordering::SeqCst);
                            }
                        })
                    };
                    frame.set_before_park(Some(hook));
                    frame.wake(worker);
                    if reserved > 0 {
                        failures
                            .wait("the task is polled at the closed gate", || woken.polls() == 2);
                    } else if failures.wait("the other worker sleeps after the wake", || {
                        woke.load(Ordering::SeqCst) && frame.sleepers() == 1
                    }) {
                        failures.check(woken.polls() == 1, "a task was polled at a closed gate");
                        failures
                            .check(frame.work_queued(), "a task left the queue at a closed gate");
                    }
                });
                failures.wait("the task is polled", || woken.polls() == 2);
            });
        }
        failures.assert_none();
        finish(&executor, task);
        // At the closed gate the poller's permit is the one out; once it opens the blocking
        // worker, still in its job, has taken its own back.
        let expected = if reserved > 0 { (1, Some(1)) } else { (2, None) };
        assert_eq!(woken.second(), (other.load(Ordering::SeqCst), expected));
    }
    executor.shutdown();
}

#[test]
fn a_wake_between_the_look_and_the_park_is_taken_within_the_reservation() {
    a_wake_between_the_look_and_the_park(1);
}

#[test]
fn a_wake_between_the_look_and_the_park_is_kept_for_the_gate() {
    a_wake_between_the_look_and_the_park(0);
}

/// The gate opens while the other worker, which has seen the woken task and found no
/// permit, is between its look round and its park: the opening claims it, so its park
/// returns at once and it polls the task, the blocking worker staying in its job.
#[test]
fn the_gate_opening_between_the_look_and_the_park_wakes_the_worker() {
    let executor = executor(2);
    let shared = executor.owner.shared.clone();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let other = Arc::new(AtomicUsize::new(usize::MAX));
        let _unhook = Unhook(shared.frame());
        {
            let (shared, woken, failures, other) =
                (shared.clone(), woken.clone(), failures.clone(), other.clone());
            run_on_a_worker(&executor, move |ctx| {
                let frame = shared.frame();
                let worker = 1 - ctx.slot() as usize;
                other.store(worker, Ordering::SeqCst);
                let (in_window, opened) =
                    (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
                ctx.block_in_place(0, || {
                    if !failures.wait("the other worker sleeps", || frame.sleepers() == 1) {
                        return;
                    }
                    let hook = {
                        let (failures, in_window, opened) =
                            (failures.clone(), in_window.clone(), opened.clone());
                        Arc::new(move |parking: usize, seen: Look| {
                            if parking == worker
                                && seen == Look::Work
                                && !in_window.swap(true, Ordering::SeqCst)
                            {
                                failures.wait("the gate opens", || opened.load(Ordering::SeqCst));
                            }
                        })
                    };
                    frame.set_before_park(Some(hook));
                    woken.wake_from_another_thread();
                    failures
                        .wait("the other worker has seen the task and is about to park", || {
                            in_window.load(Ordering::SeqCst)
                        });
                });
                opened.store(true, Ordering::SeqCst);
                failures.wait("the task is polled once the gate opened", || woken.polls() == 2);
            });
        }
        failures.assert_none();
        finish(&executor, task);
        // The blocking worker, still in its job, has taken its permit back.
        assert_eq!(woken.second(), (other.load(Ordering::SeqCst), (2, None)));
    }
    executor.shutdown();
}

/// With no permit reserved, a task whose timer falls due during the block is not polled
/// while the gate is closed: the other worker, the timer's keeper, fires it and keeps the
/// task on its own deque, then looks round, sees it and parks without a permit. Once the gate
/// opens that worker polls it, the blocking worker staying in its job.
#[test]
fn a_task_its_timer_wakes_at_a_closed_gate_waits_for_it() {
    let executor = executor(2);
    let shared = executor.owner.shared.clone();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let other = Arc::new(AtomicUsize::new(usize::MAX));
        let _unhook = Unhook(shared.frame());
        {
            let (executor_, shared, woken, failures, other) =
                (executor.clone(), shared.clone(), woken.clone(), failures.clone(), other.clone());
            run_on_a_worker(&executor, move |ctx| {
                let frame = shared.frame();
                let worker = 1 - ctx.slot() as usize;
                other.store(worker, Ordering::SeqCst);
                let saw = Arc::new(AtomicBool::new(false));
                ctx.block_in_place(0, || {
                    if !failures.wait("the other worker sleeps", || frame.sleepers() == 1) {
                        return;
                    }
                    let hook = {
                        let saw = saw.clone();
                        Arc::new(move |parking: usize, seen: Look| {
                            if parking == worker && seen == Look::Work {
                                saw.store(true, Ordering::SeqCst);
                            }
                        })
                    };
                    frame.set_before_park(Some(hook));
                    let waker = woken.take_waker();
                    // A deadline that has passed by the first poll resolves without a timer;
                    // the next one is later.
                    let mut ahead = Duration::from_millis(2);
                    let _timer = loop {
                        let mut sleep = Box::pin(executor_.sleep_until(Instant::now() + ahead));
                        if sleep.as_mut().poll(&mut Context::from_waker(&waker)).is_pending() {
                            break sleep;
                        }
                        ahead *= 2;
                    };
                    if failures.wait("the keeper fires the timer and parks beside the task", || {
                        saw.load(Ordering::SeqCst)
                    }) {
                        failures.check(woken.polls() == 1, "a task was polled at a closed gate");
                    }
                });
                failures.wait("the task is polled once the gate opened", || woken.polls() == 2);
            });
        }
        failures.assert_none();
        finish(&executor, task);
        // The blocking worker, still in its job, has taken its permit back.
        assert_eq!(woken.second(), (other.load(Ordering::SeqCst), (2, None)));
    }
    executor.shutdown();
}

/// The other worker holds a permit and has passed its check of the gate with the gate open,
/// when the gate closes and a task is woken from a thread that is no worker; then it looks
/// for work. With no permit reserved it is over the limit: it puts the task back unpolled,
/// once, gives its permit up and sleeps, and polls the task once the gate opens. With one
/// permit reserved it is within the limit and polls the task at the closed gate.
fn a_task_taken_as_the_gate_closes(reserved: u32) {
    let executor = executor(2);
    let shared = executor.owner.shared.clone();
    let frame = shared.frame();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let other = Arc::new(AtomicUsize::new(usize::MAX));
        let put_back_before = frame.tasks_put_back.load(Ordering::SeqCst);
        let _unhook = Unhook(frame);
        {
            let (shared, woken, failures, other) =
                (shared.clone(), woken.clone(), failures.clone(), other.clone());
            run_on_a_worker(&executor, move |ctx| {
                let frame = shared.frame();
                let worker = 1 - ctx.slot() as usize;
                other.store(worker, Ordering::SeqCst);
                // Asleep before the hook is set, so that the wake below brings it to the hook:
                // awake, it might reach the hook first and never sleep. Its park lasts until a
                // wake claims it.
                if !failures.wait("the other worker sleeps", || frame.sleepers() == 1) {
                    return;
                }
                let (at_find, go) =
                    (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
                let hook = {
                    let (failures, at_find, go) = (failures.clone(), at_find.clone(), go.clone());
                    Arc::new(move |looking: usize| {
                        if looking == worker && !at_find.swap(true, Ordering::SeqCst) {
                            failures.wait("the gate closes and the task is woken", || {
                                go.load(Ordering::SeqCst)
                            });
                        }
                    })
                };
                *lock(&frame.before_find) = Some(hook);
                frame.wake(worker);
                if !failures
                    .wait("the other worker holds a permit and is about to look for work", || {
                        at_find.load(Ordering::SeqCst)
                    })
                {
                    go.store(true, Ordering::SeqCst);
                    return;
                }
                let put_back = || frame.tasks_put_back.load(Ordering::SeqCst) - put_back_before;
                ctx.block_in_place(reserved, || {
                    woken.wake_from_another_thread();
                    go.store(true, Ordering::SeqCst);
                    if reserved > 0 {
                        failures
                            .wait("the task is polled at the closed gate", || woken.polls() == 2);
                    } else if failures.wait(
                        "the other worker puts the task back and sleeps",
                        || {
                            put_back() == 1 && frame.sleepers() == 1
                                || woken.polls() == 2
                                || put_back() > 1
                        },
                    ) {
                        failures
                            .check(woken.polls() == 1, "a worker over the limit polled the task");
                        failures
                            .check(put_back() == 1, "a worker over the limit took the task again");
                    }
                });
                failures.wait("the task is polled", || woken.polls() == 2);
            });
        }
        failures.assert_none();
        finish(&executor, task);
        // At the closed gate the poller's permit is the one out; once it opens the blocking
        // worker, still in its job, has taken its own back.
        let expected = if reserved > 0 { (1, Some(1)) } else { (2, None) };
        assert_eq!(woken.second(), (other.load(Ordering::SeqCst), expected));
        let put_back = frame.tasks_put_back.load(Ordering::SeqCst) - put_back_before;
        assert_eq!(put_back, u32::from(reserved == 0), "tasks put back");
    }
    executor.shutdown();
}

#[test]
fn a_task_taken_over_the_limit_is_put_back_once_and_waits_for_the_gate() {
    a_task_taken_as_the_gate_closes(0);
}

#[test]
fn a_task_taken_within_the_reservation_as_the_gate_closes_is_polled() {
    a_task_taken_as_the_gate_closes(1);
}

/// With one permit reserved and held by a job of `run_job` that began before the block,
/// the third worker passed its check of the gate with the gate open and takes a task woken
/// as it closes: it is over the limit, puts the task back once and sleeps. The job's worker,
/// the one the reservation leaves awake, polls the task once the job returns, while the gate
/// is still closed.
#[test]
fn a_task_put_back_over_the_limit_runs_on_the_reservations_worker() {
    let executor = executor(3);
    let shared = executor.owner.shared.clone();
    let frame = shared.frame();
    for _ in 0..ROUNDS {
        let (woken, task) = Woken::spawn(&executor);
        let failures = Arc::new(Failures::default());
        let job_slot = Arc::new(AtomicUsize::new(usize::MAX));
        let (started, release) =
            (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let put_back_before = frame.tasks_put_back.load(Ordering::SeqCst);
        let _unhook = Unhook(frame);
        let run = {
            let (failures, job_slot, started, release) =
                (failures.clone(), job_slot.clone(), started.clone(), release.clone());
            executor.run_job(move |ctx| {
                job_slot.store(ctx.slot() as usize, Ordering::SeqCst);
                started.store(true, Ordering::SeqCst);
                failures.wait("the job is released", || release.load(Ordering::SeqCst));
            })
        };
        {
            let (shared, woken, failures, job_slot, started, release) = (
                shared.clone(),
                woken.clone(),
                failures.clone(),
                job_slot.clone(),
                started,
                release.clone(),
            );
            run_on_a_worker(&executor, move |ctx| {
                let frame = shared.frame();
                if !failures.wait("the job begins", || started.load(Ordering::SeqCst)) {
                    release.store(true, Ordering::SeqCst);
                    return;
                }
                let third = 3 - ctx.slot() as usize - job_slot.load(Ordering::SeqCst);
                // As in `a_task_taken_as_the_gate_closes`: asleep before the hook is set.
                if !failures.wait("the third worker sleeps", || frame.sleepers() == 1) {
                    release.store(true, Ordering::SeqCst);
                    return;
                }
                let (at_find, go) =
                    (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
                let hook = {
                    let (failures, at_find, go) = (failures.clone(), at_find.clone(), go.clone());
                    Arc::new(move |looking: usize| {
                        if looking == third && !at_find.swap(true, Ordering::SeqCst) {
                            failures.wait("the gate closes and the task is woken", || {
                                go.load(Ordering::SeqCst)
                            });
                        }
                    })
                };
                *lock(&frame.before_find) = Some(hook);
                frame.wake(third);
                if !failures
                    .wait("the third worker holds a permit and is about to look for work", || {
                        at_find.load(Ordering::SeqCst)
                    })
                {
                    go.store(true, Ordering::SeqCst);
                    release.store(true, Ordering::SeqCst);
                    return;
                }
                let put_back = || frame.tasks_put_back.load(Ordering::SeqCst) - put_back_before;
                ctx.block_in_place(1, || {
                    woken.wake_from_another_thread();
                    go.store(true, Ordering::SeqCst);
                    if failures.wait("the third worker puts the task back and sleeps", || {
                        put_back() == 1 && frame.sleepers() == 1
                            || woken.polls() == 2
                            || put_back() > 1
                    }) {
                        failures
                            .check(woken.polls() == 1, "a worker over the limit polled the task");
                        failures
                            .check(put_back() == 1, "a worker over the limit took the task again");
                    }
                    release.store(true, Ordering::SeqCst);
                    failures.wait("the task is polled at the closed gate", || woken.polls() == 2);
                });
            });
        }
        release.store(true, Ordering::SeqCst);
        failures.assert_none();
        finish(&executor, task);
        let blocking = executor.clone();
        within("the job", move || blocking.block_on(run)).unwrap();
        assert_eq!(
            woken.second(),
            (job_slot.load(Ordering::SeqCst), (1, Some(1))),
            "the task was not polled by the reservation's worker at the closed gate"
        );
        assert_eq!(frame.tasks_put_back.load(Ordering::SeqCst) - put_back_before, 1);
    }
    executor.shutdown();
}
