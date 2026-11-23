//! loom models of the scope latch, the sleep-and-wake protocol, the permit gate, run
//! quiescence, the shutdown flag, and the completion slot and the width of `Executor::run_job`.
//!
//! Run with `cargo test -p lactose-task --features loom --release`. Every model calls the
//! functions the executor ships: `gate::wait_for_children` is the opener's wait,
//! `Sleep::announce_entry` is what a worker does after queueing work, `Sleep::wake_worker`
//! is how it wakes a worker, `Timers::fire_due`
//! is how a deadline reaches its task, `Quiesce` is what holds a run's future back,
//! `ShutdownFlag::queue` is how work is queued against shutdown's sweep, `JobSlot` is where a
//! job's value waits for its future, and `Width` is what bounds the workers such jobs occupy.
//! What a model supplies is only what loom does not model: a deque is a counter, a worker is a
//! loom thread, and a scope's children are a number.
//!
//! Four call sites of `run_job`'s are copies in the models, because loom cannot run the types
//! they live in (`WorkerLocal`, `Shared`), and each is held by a native test instead:
//! - `idle`'s look (`pool.rs`) counts the two queues only while `Width::admits` says so; the
//!   model's `Pool::work` calls `Width::admits` itself
//!   (`jobs::a_job_queued_as_the_workers_go_idle_is_taken` in `tests/jobs.rs`);
//! - `WideChildren::look` (`job.rs`) asks whether the children's queue holds any; the model's
//!   `ModelOpener::look` asks its own queue
//!   (`executor::job_tests::an_opener_takes_a_half_queued_after_it_parked`; the split rule
//!   that queues those halves in `a_range_in_a_job_splits_to_a_second_worker`);
//! - `WideChildren::park` keeps the job's place while the opener sleeps; the model's opener
//!   parks on its latch and never gives a place up
//!   (`executor::job_tests::an_opener_parks_beside_a_full_width_and_the_next_job_waits`);
//! - `JobCell::run` (`job_run.rs`) hands `JobSlot::take` the shutdown flag; the model hands it
//!   `ShutdownFlag::is_set` itself
//!   (`pool::tests::a_job_taken_after_shutdown_began_never_starts`).
//!
//! A lost wake-up shows as a deadlock, which loom reports. The models are exhaustive
//! except where a preemption bound is named; `LOOM_MAX_PREEMPTIONS` overrides the bounds.

use loom::sync::Arc;
use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use loom::thread::{self, Thread};

use crate::gate::{self, Gate, Opener, Wake};
use crate::latch::Latch;
use crate::quiesce::{Entry, Quiesce};
use crate::sleep::Sleep;

/// The preemption bound of the three-thread models, which are too large to explore
/// exhaustively.
const THREE_THREAD_BOUND: usize = 5;

/// How long the keeper model's registrar spins for a watch before it calls the hand-off
/// lost: far above the two spins a passing execution needs.
const REGISTRAR_SPINS: usize = 150;

/// Checks `model` with at most `bound` preemptions per execution unless the environment
/// sets its own bound.
fn bounded(bound: usize, model: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    if builder.preemption_bound.is_none() {
        builder.preemption_bound = Some(bound);
    }
    builder.check(model);
}

/// The pool as the gate sees it: no idle sleepers, and workers that are loom threads.
struct Workers(Vec<Thread>);

impl Wake for Workers {
    fn idle_one(&self) {}

    fn unpark(&self, worker: usize) {
        self.0[worker].unpark();
    }
}

/// An opener whose deque holds `children` single-unit children of its own scope.
struct ModelOpener<'a> {
    gate: &'a Gate,
    latch: &'a Latch,
    children: usize,
    shutdown: &'a AtomicBool,
}

impl Opener for ModelOpener<'_> {
    fn run_children(&mut self) -> bool {
        while !self.gate.over_limit() || self.shutting_down() {
            if self.children == 0 {
                return false;
            }
            self.children -= 1;
            // The opener's own decrement needs no unpark.
            let _last = self.latch.count_down(1);
        }
        self.children != 0
    }

    fn shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    fn spin(&mut self, latch: &Latch) -> bool {
        latch.is_set()
    }

    /// Only this opener pushes onto its deque.
    fn look(&mut self) -> bool {
        false
    }
}

/// A gate with `holders` permits out and, if `reserved` is given, closed by a blocked
/// worker that has already given its own permit up.
fn gate_with(workers: usize, holders: usize, reserved: Option<usize>) -> Gate {
    let gate = Gate::new(workers);
    for _ in 0..holders {
        gate.retake();
    }
    if let Some(reserved) = reserved {
        gate.retake();
        gate.close(reserved, &Workers(Vec::new()));
    }
    gate
}

/// A scope with two children stolen by other workers. The opener must not get past its
/// wait before both have run, and neither child may touch the latch after the decrement
/// that releases the opener (`Latch::retire` ends the modelled stack frame).
#[test]
fn latch_releases_the_opener_after_the_last_child() {
    loom::model(|| {
        let latch = Arc::new(Latch::new());
        let ran = Arc::new(AtomicUsize::new(0));
        let gate = gate_with(1, 1, None);
        let shutdown = AtomicBool::new(false);
        let opener = thread::current();

        latch.add(2);
        let thieves: Vec<_> = (0..2)
            .map(|_| {
                let (latch, ran, opener) = (latch.clone(), ran.clone(), opener.clone());
                thread::spawn(move || {
                    ran.fetch_add(1, Ordering::Relaxed);
                    if latch.count_down(1) {
                        opener.unpark();
                    }
                })
            })
            .collect();

        let mut waiting =
            ModelOpener { gate: &gate, latch: &latch, children: 0, shutdown: &shutdown };
        gate::wait_for_children(&latch, &gate, 0, &Workers(vec![opener]), &mut waiting);
        // Relaxed on purpose: the latch's own orderings must carry the children's writes.
        assert_eq!(ran.load(Ordering::Relaxed), 2);
        latch.retire();
        for thief in thieves {
            thief.join().unwrap();
        }
    });
}

/// The opener runs one child itself while a thief runs a range of two units, and a stale
/// park token is already pending when it starts to wait.
#[test]
fn latch_tolerates_inline_children_and_stale_tokens() {
    loom::model(|| {
        let latch = Arc::new(Latch::new());
        let ran = Arc::new(AtomicUsize::new(0));
        let gate = gate_with(1, 1, None);
        let shutdown = AtomicBool::new(false);
        let opener = thread::current();
        // What an unrelated wake-up of this worker leaves behind.
        opener.unpark();

        latch.add(3);
        let thief = {
            let (latch, ran, opener) = (latch.clone(), ran.clone(), opener.clone());
            thread::spawn(move || {
                ran.fetch_add(2, Ordering::Relaxed);
                if latch.count_down(2) {
                    opener.unpark();
                }
            })
        };
        let mut waiting =
            ModelOpener { gate: &gate, latch: &latch, children: 1, shutdown: &shutdown };
        gate::wait_for_children(&latch, &gate, 0, &Workers(vec![opener]), &mut waiting);
        assert_eq!(ran.load(Ordering::Relaxed), 2);
        assert_eq!(waiting.children, 0);
        latch.retire();
        thief.join().unwrap();
    });
}

/// The second review's finding: every worker but the blocked one is an opener with
/// children on its own deque when a `block_in_place(1)` closes the gate. Both are over the
/// limit and both give way. One of them must end up with the reserved permit and run its
/// children, and when its scope is done the other must get the permit in turn; nothing
/// opens the gate in this model, so parking beside runnable children is a deadlock.
#[test]
fn gate_hands_the_reserved_permit_to_an_opener_with_children() {
    loom::model(|| {
        let gate = Arc::new(gate_with(2, 2, Some(1)));
        let shutdown = Arc::new(AtomicBool::new(false));
        let first = thread::current();

        let other = {
            let (gate, shutdown, first) = (gate.clone(), shutdown.clone(), first.clone());
            thread::spawn(move || {
                let latch = Latch::new();
                latch.add(1);
                let wake = Workers(vec![first, thread::current()]);
                let mut waiting =
                    ModelOpener { gate: &gate, latch: &latch, children: 1, shutdown: &shutdown };
                gate::wait_for_children(&latch, &gate, 1, &wake, &mut waiting);
                assert!(latch.is_set());
                // The job goes on and ends; the worker would now go idle.
                gate.release(&wake);
            })
        };
        let latch = Latch::new();
        latch.add(1);
        let wake = Workers(vec![first, other.thread().clone()]);
        let mut waiting =
            ModelOpener { gate: &gate, latch: &latch, children: 1, shutdown: &shutdown };
        gate::wait_for_children(&latch, &gate, 0, &wake, &mut waiting);
        assert!(latch.is_set());
        gate.release(&wake);
        other.join().unwrap();
    });
}

/// An opener stopped by a gate that reserves nothing runs its children once the gate
/// opens, and holds a permit again when its wait returns.
#[test]
fn gate_opening_resumes_a_waiting_opener() {
    loom::model(|| {
        let gate = Arc::new(gate_with(1, 1, Some(0)));
        let shutdown = AtomicBool::new(false);
        let opener = thread::current();

        let unparked = Arc::new(AtomicBool::new(false));
        let blocked = {
            let (gate, opener, unparked) = (gate.clone(), opener.clone(), unparked.clone());
            thread::spawn(move || {
                let wake = Workers(vec![opener]);
                gate.open(0, &wake);
                // The pool unparks every worker after opening.
                wake.unpark(0);
                unparked.store(true, Ordering::SeqCst);
            })
        };
        let latch = Latch::new();
        latch.add(1);
        let mut waiting =
            ModelOpener { gate: &gate, latch: &latch, children: 1, shutdown: &shutdown };
        gate::wait_for_children(&latch, &gate, 0, &Workers(vec![opener]), &mut waiting);
        assert!(latch.is_set());
        // loom cannot have a thread unparked while it is blocked in `join`, so wait for the
        // last unpark before joining.
        while !unparked.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        blocked.join().unwrap();
        // The opener and the worker that was blocked both hold permits again.
        assert!(!gate.over_limit());
        assert!(!gate.release_if_over() && gate.try_acquire());
    });
}

/// At shutdown nobody else will run what is on the opener's deque, and the gate stops
/// applying: a gated opener takes its permit back and finishes its scope.
#[test]
fn shutdown_lets_a_gated_opener_finish() {
    loom::model(|| {
        let gate = gate_with(1, 1, Some(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let opener = thread::current();

        let stopper = {
            let (shutdown, opener) = (shutdown.clone(), opener.clone());
            thread::spawn(move || {
                shutdown.store(true, Ordering::SeqCst);
                opener.unpark();
            })
        };
        let latch = Latch::new();
        latch.add(1);
        let mut waiting =
            ModelOpener { gate: &gate, latch: &latch, children: 1, shutdown: &shutdown };
        gate::wait_for_children(&latch, &gate, 0, &Workers(vec![opener]), &mut waiting);
        assert!(latch.is_set());
        // One permit out, the opener's, against a limit of none.
        assert!(gate.over_limit() && gate.release_if_over() && !gate.release_if_over());
        stopper.join().unwrap();
    });
}

/// What an idle worker does, with the deque reduced to a count of entries: take work if
/// there is any, otherwise announce, re-check and park. Returns once it has taken one
/// entry.
fn take_one_entry(sleep: &Sleep, queue: &AtomicUsize, worker: usize) {
    loop {
        let take = |entries: usize| entries.checked_sub(1);
        // The orderings are the weakest the real deque uses to publish an entry.
        if queue.fetch_update(Ordering::AcqRel, Ordering::Acquire, take).is_ok() {
            return;
        }
        sleep.announce(worker);
        if queue.load(Ordering::Acquire) != 0 {
            sleep.cancel(worker);
        } else {
            sleep.park(worker, None);
        }
    }
}

/// What a worker does after it makes an entry visible. `kept` says it will take one of the
/// entries it is releasing itself.
fn push_one_entry(sleep: &Sleep, queue: &AtomicUsize, workers: &[Thread], kept: bool) {
    queue.fetch_add(1, Ordering::Release);
    if let Some(worker) = sleep.announce_entry(kept) {
        sleep.wake_worker(worker, || workers[worker].unpark());
    }
}

/// One worker going to sleep while another thread pushes: the entry is always taken.
#[test]
fn sleep_never_misses_a_push() {
    loom::model(|| {
        let sleep = Arc::new(Sleep::new(1));
        let queue = Arc::new(AtomicUsize::new(0));
        let workers = [thread::current()];

        let pusher = {
            let (sleep, queue) = (sleep.clone(), queue.clone());
            thread::spawn(move || push_one_entry(&sleep, &queue, &workers, false))
        };
        take_one_entry(&sleep, &queue, 0);
        pusher.join().unwrap();
    });
}

/// Two workers going to sleep and two pushes from one thread: both entries are taken, each
/// by a different worker, so neither notification is spent on a worker that then leaves the
/// other entry stranded.
#[test]
fn sleep_wakes_a_worker_for_every_push() {
    bounded(THREE_THREAD_BOUND, || {
        let sleep = Arc::new(Sleep::new(2));
        let queue = Arc::new(AtomicUsize::new(0));

        let other = {
            let (sleep, queue) = (sleep.clone(), queue.clone());
            thread::spawn(move || take_one_entry(&sleep, &queue, 1))
        };
        let workers = [thread::current(), other.thread().clone()];
        let pusher = {
            let (sleep, queue) = (sleep.clone(), queue.clone());
            thread::spawn(move || {
                push_one_entry(&sleep, &queue, &workers, false);
                push_one_entry(&sleep, &queue, &workers, false);
            })
        };
        take_one_entry(&sleep, &queue, 0);
        other.join().unwrap();
        pusher.join().unwrap();
    });
}

/// A worker that finalizes a node releases two dependents and keeps one for itself: the
/// first entry it queues wakes nobody. The other worker, which may be asleep or on its way
/// there, must still end up with the second entry.
#[test]
fn a_kept_entry_wakes_nobody_and_strands_nothing() {
    loom::model(|| {
        let sleep = Arc::new(Sleep::new(2));
        let queue = Arc::new(AtomicUsize::new(0));

        let other = {
            let (sleep, queue) = (sleep.clone(), queue.clone());
            thread::spawn(move || take_one_entry(&sleep, &queue, 1))
        };
        let workers = [thread::current(), other.thread().clone()];
        push_one_entry(&sleep, &queue, &workers, true);
        push_one_entry(&sleep, &queue, &workers, false);
        // The releasing worker's own next job is one of the two, without going to sleep.
        let took = queue.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
        if took.is_err() {
            // The other worker was quicker to the first; the second is the releaser's.
            take_one_entry(&sleep, &queue, 0);
        }
        other.join().unwrap();
        assert_eq!(queue.load(Ordering::Acquire), 0);
    });
}

/// A condition one particular worker waits for (its turn in a replay, an earlier timer) and
/// the targeted and broadcast notifications that announce it.
#[test]
fn sleep_delivers_targeted_and_broadcast_wakes() {
    loom::model(|| {
        let sleep = Arc::new(Sleep::new(2));
        let flag = Arc::new(AtomicUsize::new(0));

        let wait_for = |sleep: &Sleep, flag: &AtomicUsize, worker: usize, value: usize| {
            while flag.load(Ordering::Acquire) < value {
                sleep.announce(worker);
                if flag.load(Ordering::Acquire) >= value {
                    sleep.cancel(worker);
                } else {
                    sleep.park(worker, None);
                }
            }
        };

        let main = thread::current();
        let waker = {
            let (sleep, flag) = (sleep.clone(), flag.clone());
            thread::spawn(move || {
                flag.store(1, Ordering::Release);
                if sleep.notify(0) {
                    sleep.wake_worker(0, || main.unpark());
                }
                flag.store(2, Ordering::Release);
                sleep.notify_all(|worker| {
                    assert_eq!(worker, 0);
                    sleep.wake_worker(worker, || main.unpark());
                });
            })
        };
        wait_for(&sleep, &flag, 0, 1);
        wait_for(&sleep, &flag, 0, 2);
        waker.join().unwrap();
    });
}

/// A run with two jobs, one of which finishes the run, and a cancellation that may land
/// anywhere. Whenever the future is allowed to resolve, no job is executing; a job that
/// was admitted never sees the run already resolved; and the run does resolve.
#[test]
fn a_run_resolves_only_when_no_job_of_it_is_executing() {
    bounded(THREE_THREAD_BOUND, || {
        let jobs = Arc::new(Quiesce::new());
        let executing = Arc::new(AtomicUsize::new(0));
        let resolved = Arc::new(AtomicUsize::new(0));

        let job = |finishes: bool| {
            let (jobs, executing, resolved) = (jobs.clone(), executing.clone(), resolved.clone());
            move || {
                let resolve = || {
                    assert_eq!(executing.load(Ordering::SeqCst), 0, "resolved over a running job");
                    resolved.fetch_add(1, Ordering::SeqCst);
                };
                match jobs.enter() {
                    Entry::Entered => {
                        executing.fetch_add(1, Ordering::SeqCst);
                        assert_eq!(
                            resolved.load(Ordering::SeqCst),
                            0,
                            "a job ran after resolution"
                        );
                        if finishes {
                            // The last finalize: the run closes with this job inside.
                            assert!(!jobs.close(), "quiet with a job registered");
                        }
                        executing.fetch_sub(1, Ordering::SeqCst);
                        if jobs.exit() {
                            resolve();
                        }
                    }
                    Entry::Refused { quiet } => {
                        if quiet {
                            resolve();
                        }
                    }
                }
            }
        };
        let workers = [thread::spawn(job(false)), thread::spawn(job(true))];
        // Shutdown, on the calling thread.
        if jobs.close() {
            assert_eq!(executing.load(Ordering::SeqCst), 0, "resolved over a running job");
            resolved.fetch_add(1, Ordering::SeqCst);
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(resolved.load(Ordering::SeqCst) >= 1, "the run never resolved");
        assert!(!jobs.close(), "closing twice must not report quiet again");
    });
}

/// The pool as an idle worker's release sees it: `idle_one` really claims a sleeper.
struct PoolWake {
    sleep: Arc<Sleep>,
    threads: Vec<Thread>,
}

impl Wake for PoolWake {
    fn idle_one(&self) {
        if let Some(worker) = self.sleep.notify_one() {
            self.sleep.wake_worker(worker, || self.threads[worker].unpark());
        }
    }

    fn unpark(&self, worker: usize) {
        self.threads[worker].unpark();
    }
}

/// The worker loop reduced to its permit and sleep steps, over a deque that is a count of
/// entries. Returns when the one entry has been taken, by this worker or another.
fn work(
    gate: &Gate,
    sleep: &Sleep,
    queue: &AtomicUsize,
    taken: &AtomicBool,
    me: usize,
    wake: &PoolWake,
    mut permit: bool,
) {
    while !taken.load(Ordering::SeqCst) {
        if !permit {
            permit = gate.try_acquire();
        }
        if permit {
            let take = |entries: usize| entries.checked_sub(1);
            if queue.fetch_update(Ordering::AcqRel, Ordering::Acquire, take).is_ok() {
                // Stands for shutdown, and wakes as `Shared::shut_down` does: lets the other
                // worker's model thread end.
                taken.store(true, Ordering::SeqCst);
                sleep.notify_all(|_| {});
                (0..wake.threads.len())
                    .for_each(|worker| sleep.wake_worker(worker, || wake.unpark(worker)));
                break;
            }
            gate.release_idle(wake);
            permit = false;
        }
        gate::idle_park(gate, sleep, me, || {
            let seen = if taken.load(Ordering::SeqCst) {
                gate::Look::Stop
            } else if queue.load(Ordering::Acquire) != 0 {
                gate::Look::Work
            } else {
                gate::Look::Nothing
            };
            (seen, None)
        });
    }
}

/// Two idle workers under a closed gate that reserves one permit, and one entry that is
/// queued while they are going to sleep. A worker that scans and finds nothing gives its
/// permit up without waking the other (`Gate::release_idle`), then sleeps through
/// `gate::idle_park`. Whatever the interleaving, the entry is taken: by the releasing
/// worker's own re-check if it was queued before, by the pusher's wake-up if after.
#[test]
fn an_idle_release_wakes_nobody_and_loses_no_work() {
    // Four preemptions: this model takes 24 s at four and close to three minutes at five.
    bounded(THREE_THREAD_BOUND - 1, || {
        let gate = Arc::new(gate_with(2, 1, Some(1)));
        let sleep = Arc::new(Sleep::new(2));
        let queue = Arc::new(AtomicUsize::new(0));
        let taken = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicUsize::new(0));
        let first = thread::current();

        let other = {
            let (gate, sleep, queue, taken, ended, first) = (
                gate.clone(),
                sleep.clone(),
                queue.clone(),
                taken.clone(),
                ended.clone(),
                first.clone(),
            );
            thread::spawn(move || {
                let wake =
                    PoolWake { sleep: sleep.clone(), threads: vec![first, thread::current()] };
                work(&gate, &sleep, &queue, &taken, 1, &wake, false);
                ended.fetch_add(1, Ordering::SeqCst);
            })
        };
        let threads = vec![first, other.thread().clone()];
        let pusher = {
            let (sleep, queue, ended, threads) =
                (sleep.clone(), queue.clone(), ended.clone(), threads.clone());
            thread::spawn(move || {
                push_one_entry(&sleep, &queue, &threads, false);
                ended.fetch_add(1, Ordering::SeqCst);
            })
        };
        let wake = PoolWake { sleep: sleep.clone(), threads };
        // This worker starts with the one permit the gate allows.
        work(&gate, &sleep, &queue, &taken, 0, &wake, true);
        assert_eq!(queue.load(Ordering::Acquire), 0);
        // loom cannot have a thread unparked while it is blocked in `join`.
        while ended.load(Ordering::SeqCst) != 2 {
            thread::yield_now();
        }
        other.join().unwrap();
        pusher.join().unwrap();
    });
}

/// During shutdown an opener whose remaining child is running on another worker has
/// nothing to do but wait for it. It parks; it does not go round its loop giving its permit
/// up and taking it back until the child is done, which in this model would never end.
#[test]
fn shutdown_parks_an_opener_with_nothing_on_its_deque() {
    loom::model(|| {
        let gate = gate_with(1, 1, None);
        let shutdown = AtomicBool::new(true);
        let latch = Arc::new(Latch::new());
        let opener = thread::current();

        latch.add(1);
        let thief = {
            let (latch, opener) = (latch.clone(), opener.clone());
            thread::spawn(move || {
                if latch.count_down(1) {
                    opener.unpark();
                }
            })
        };
        let mut waiting =
            ModelOpener { gate: &gate, latch: &latch, children: 0, shutdown: &shutdown };
        gate::wait_for_children(&latch, &gate, 0, &Workers(vec![opener]), &mut waiting);
        assert!(latch.is_set());
        latch.retire();
        thief.join().unwrap();
    });
}

/// The keeper hand-off: one worker parks as the timer keeper, watching a deadline, while
/// another thread registers an earlier one. The registration must reach the keeper, which
/// gives the watch up and takes it again for the earlier deadline. The wait is the shipped
/// idle sequence (`gate::idle_park` over `Timers::keep` and `Timers::release`) with the
/// stepped timed park. Under loom a step waits on a flag that only `Sleep::wake_worker`
/// sets, so the keeper can only be woken by the hand-off itself. The registrar wakes the
/// keeper as `Pool::wake` does, whether or not its claim succeeds. A lost hand-off, or a
/// hand-off that only unparks, leaves the registrar spinning for a watch that never comes;
/// the spin is bounded, so that fails with the state it found, not loom's branch limit.
#[test]
fn an_earlier_timer_reaches_the_parked_keeper() {
    use core::task::Waker;
    use std::time::{Duration, Instant};

    use crate::time::{Rearm, Timers};

    loom::model(|| {
        let gate = Arc::new(Gate::new(1));
        let sleep = Arc::new(Sleep::with_timed_parks(1));
        let timers = Arc::new(Timers::new());
        // Far enough that neither is due during the model.
        let later = Instant::now() + Duration::from_secs(3600);
        let earlier = later - Duration::from_secs(60);
        let mut later_id = None;
        let first = timers.register(later, &mut later_id, Waker::noop());
        assert!(matches!(first, Rearm::AnySleeper));
        // What the keeper last parked for: 2 the later deadline, 1 the earlier, 0 none.
        let watching = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let keeper = thread::current();

        let registrar = {
            let (sleep, timers, watching, stop, keeper) =
                (sleep.clone(), timers.clone(), watching.clone(), stop.clone(), keeper.clone());
            thread::spawn(move || {
                // A passing execution spins at most twice per wait.
                let wait_for_watch = |want: usize| {
                    let mut spins = 0;
                    while watching.load(Ordering::SeqCst) != want {
                        spins += 1;
                        if spins > REGISTRAR_SPINS {
                            let (bit, flag) = sleep.worker_state(0);
                            panic!(
                                "lost hand-off: the keeper never came to watch {want} in \
                                 {REGISTRAR_SPINS} spins; keeper bit set {bit}, flag {flag:?}, \
                                 watching {}",
                                watching.load(Ordering::SeqCst),
                            );
                        }
                        thread::yield_now();
                    }
                };
                // Once the keeper watches the later deadline, register the earlier one.
                wait_for_watch(2);
                let mut earlier_id = None;
                match timers.register(earlier, &mut earlier_id, Waker::noop()) {
                    Rearm::Keeper(worker) => {
                        let _ = sleep.notify(worker);
                        sleep.wake_worker(worker, || keeper.unpark());
                    }
                    Rearm::AnySleeper => {
                        if let Some(worker) = sleep.notify_one() {
                            assert_eq!(worker, 0);
                            sleep.wake_worker(worker, || keeper.unpark());
                        }
                    }
                    Rearm::Nobody => unreachable!("the earlier deadline is new"),
                }
                // The keeper must come to watch the earlier deadline on its own.
                wait_for_watch(1);
                // Stops the keeper as `Shared::shut_down` does.
                stop.store(true, Ordering::SeqCst);
                sleep.notify_all(|_| {});
                sleep.wake_worker(0, || keeper.unpark());
            })
        };

        // The idle loop of a frame-critical worker with no work, reduced to its timer duty.
        while !stop.load(Ordering::SeqCst) {
            gate::idle_park(&gate, &sleep, 0, || {
                if stop.load(Ordering::SeqCst) {
                    return (gate::Look::Stop, None);
                }
                timers.fire_due();
                let deadline = timers.keep(0);
                let watched = match deadline {
                    Some(deadline) if deadline == earlier => 1,
                    Some(_) => 2,
                    None => 0,
                };
                watching.store(watched, Ordering::SeqCst);
                (gate::Look::Nothing, deadline)
            });
            timers.fire_due();
            if timers.release(0) {
                // Deadlines remain: hand the watch to another sleeper, of which this model
                // has none, so the loop takes it again.
                let _nobody = sleep.notify_one();
            }
        }
        registrar.join().unwrap();
        assert_eq!(watching.load(Ordering::SeqCst), 1);
    });
}

/// A task woken by a timer the way `Shared::submit_task` queues one that the worker firing
/// the timer keeps: it lands on that worker's deque and wakes nobody.
struct KeptTask {
    sleep: Arc<Sleep>,
    queue: Arc<AtomicUsize>,
    threads: Vec<Thread>,
}

impl std::task::Wake for KeptTask {
    fn wake(self: std::sync::Arc<Self>) {
        push_one_entry(&self.sleep, &self.queue, &self.threads, true);
    }
}

/// The timer hand-over at its least favourable. A worker comes out of its park as the
/// keeper and fires the due timer (`Timers::fire_due`); nothing else is queued, so the
/// woken task is kept for it and nobody is woken (`Sleep::announce_entry`). But a
/// `block_in_place(1)` has closed the
/// gate, and the one permit is with a second worker that scanned and found nothing. The
/// keeper goes on as the worker loop does after its park, taking a permit or sleeping
/// through `gate::idle_park`, and the second worker gives its permit up through
/// `Gate::release_idle`, which wakes nobody either. The task must be taken all the same:
/// by the keeper if its re-check sees the free permit, by the second worker if its
/// re-check sees the task. Parking both beside it is a deadlock here.
#[test]
fn a_task_kept_by_the_worker_that_fired_its_timer_is_taken() {
    use core::task::Waker;
    use std::time::Instant;

    use crate::time::Timers;

    loom::model(|| {
        let gate = Arc::new(gate_with(2, 1, Some(1)));
        let sleep = Arc::new(Sleep::new(2));
        let queue = Arc::new(AtomicUsize::new(0));
        let taken = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let keeper = thread::current();

        let other = {
            let (gate, sleep, queue, taken, ended, keeper) = (
                gate.clone(),
                sleep.clone(),
                queue.clone(),
                taken.clone(),
                ended.clone(),
                keeper.clone(),
            );
            thread::spawn(move || {
                let wake =
                    PoolWake { sleep: sleep.clone(), threads: vec![keeper, thread::current()] };
                // Holds the permit the gate allows.
                work(&gate, &sleep, &queue, &taken, 1, &wake, true);
                ended.store(true, Ordering::SeqCst);
            })
        };
        let threads = vec![keeper, other.thread().clone()];
        let timers = Timers::new();
        let task = std::sync::Arc::new(KeptTask {
            sleep: sleep.clone(),
            queue: queue.clone(),
            threads: threads.clone(),
        });
        let mut id = None;
        let _ = timers.register(Instant::now(), &mut id, &Waker::from(task));
        // Due at once. Had it not fired, nothing would be queued and both workers would
        // park for good, which fails the model as a deadlock.
        timers.fire_due();

        let wake = PoolWake { sleep: sleep.clone(), threads };
        work(&gate, &sleep, &queue, &taken, 0, &wake, false);
        assert_eq!(queue.load(Ordering::Acquire), 0);
        // loom cannot have a thread unparked while it is blocked in `join`.
        while !ended.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        other.join().unwrap();
    });
}

// The jobs of `Executor::run_job`: their completion slot (`job_run.rs`) and the width
// (`width.rs`), the opener's wait included.

#[expect(
    clippy::arc_with_non_send_sync,
    reason = "the models share loom objects that are not `Sync`; loom's threads ask no bound"
)]
mod jobs {
    use core::task::{Context, Poll, Waker};
    // Not loom's `Arc`: a model that fails by deadlock unwinds with its threads' handles
    // still live, and dropping a loom `Arc` then panics in loom's runtime, which aborts the
    // test binary instead of failing the one test.
    use std::sync::{Arc, OnceLock};

    use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use loom::thread::{self, Thread};

    use super::bounded;
    use crate::gate::{self, Gate, Look, Opener, Wake};
    use crate::job_run::JobSlot;
    use crate::latch::Latch;
    use crate::shutdown::{self, ShutdownFlag};
    use crate::sleep::Sleep;
    use crate::width::{Place, Queued, Weakened, Width};

    /// A job's value, which counts its drops.
    struct Value(Arc<AtomicUsize>);

    impl Drop for Value {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Which side of a mutant leaves the value it is handed undropped.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Forget {
        Nobody,
        Worker,
        Future,
    }

    /// The worker hands the value over while the future is polled once and dropped. The
    /// value is dropped exactly once: by the future if it took it, otherwise by whichever of
    /// the worker and the dropper comes last.
    fn the_value_against_a_dropped_future(forget: Forget) {
        loom::model(move || {
            let drops = Arc::new(AtomicUsize::new(0));
            let slot = Arc::new(JobSlot::<(), Value>::new(()));
            let worker = {
                let (slot, drops) = (slot.clone(), drops.clone());
                thread::spawn(move || {
                    slot.take(false, drop).expect("no shutdown in this model");
                    let left = slot.finish(Value(drops));
                    if forget == Forget::Worker {
                        core::mem::forget(left);
                    }
                })
            };
            match slot.poll(&mut Context::from_waker(Waker::noop())) {
                // The future took it; a resolved `JobRun` holds nothing more.
                Poll::Ready(value) => drop(value.expect("the job ran")),
                Poll::Pending => {
                    let left = slot.abandon();
                    if forget == Forget::Future {
                        core::mem::forget(left);
                    }
                }
            }
            worker.join().unwrap();
            let dropped = drops.load(Ordering::SeqCst);
            assert_eq!(dropped, 1, "the value was dropped {dropped} times");
        });
    }

    #[test]
    fn a_job_value_is_dropped_once_by_whoever_comes_last() {
        the_value_against_a_dropped_future(Forget::Nobody);
    }

    #[test]
    #[should_panic(expected = "the value was dropped 0 times")]
    fn weakened_a_worker_that_keeps_the_value_of_a_dropped_future() {
        the_value_against_a_dropped_future(Forget::Worker);
    }

    #[test]
    #[should_panic(expected = "the value was dropped 0 times")]
    fn weakened_a_dropper_that_leaves_the_value_in_the_slot() {
        the_value_against_a_dropped_future(Forget::Future);
    }

    /// What happened to the model's closure.
    #[derive(Default)]
    struct Record {
        ran: AtomicUsize,
        dropped_unrun: AtomicUsize,
        /// Written by the child of the closure's scope.
        child_wrote: AtomicUsize,
    }

    /// The closure: it opens a scope whose one child runs on a thread of its own, and
    /// returns once the child has counted down.
    struct Closure {
        record: Arc<Record>,
        ran: bool,
    }

    impl Closure {
        fn run(mut self) -> u32 {
            self.ran = true;
            self.record.ran.fetch_add(1, Ordering::SeqCst);
            let latch = Arc::new(Latch::new());
            latch.add(1);
            let opener = thread::current();
            let _child = {
                let (latch, record) = (latch.clone(), self.record.clone());
                thread::spawn(move || {
                    // Relaxed on purpose: the latch and the slot must carry it to the future.
                    record.child_wrote.store(1, Ordering::Relaxed);
                    if latch.count_down(1) {
                        opener.unpark();
                    }
                })
            };
            while !latch.is_set() {
                latch.park();
            }
            7
        }
    }

    impl Drop for Closure {
        fn drop(&mut self) {
            if !self.ran {
                self.record.dropped_unrun.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// Wakes the model's main thread, which polls the future.
    struct MainWake(Thread);

    impl std::task::Wake for MainWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    /// How a mutant of the shutdown sweep goes wrong.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Sweep {
        /// As `Shared::discard_queued` does: the entry's drop cancels the job.
        Cancels,
        /// Drops the entry and forgets the job.
        Forgets,
        /// Resolves the future before it drops the closure.
        ResolvesFirst,
    }

    /// One queued job, a worker that takes it and shutdown beginning. The worker takes the
    /// entry and starts the closure unless it reads the shutdown flag; shutdown sets the flag
    /// and sweeps the queue, cancelling what it finds. The future resolves to `Ok` or to
    /// `Cancelled`, the closure is run or dropped exactly once, `Ok` comes only once the
    /// closure's scope has closed with the child's write visible, and `Cancelled` only once
    /// the closure is gone.
    fn a_take_against_shutdown(sweep: Sweep) {
        bounded(super::THREE_THREAD_BOUND, move || {
            let shutdown = Arc::new(ShutdownFlag::new());
            let queued = Arc::new(AtomicBool::new(true));
            let record = Arc::new(Record::default());
            let slot = Arc::new(JobSlot::new(Closure { record: record.clone(), ran: false }));

            let worker = {
                let (shutdown, queued, slot) = (shutdown.clone(), queued.clone(), slot.clone());
                thread::spawn(move || {
                    if queued.swap(false, Ordering::AcqRel)
                        && let Some(job) = slot.take(shutdown.is_set(), drop)
                    {
                        let _ = slot.finish(job.run());
                    }
                })
            };
            let stopper = {
                let (shutdown, queued, slot) = (shutdown.clone(), queued.clone(), slot.clone());
                thread::spawn(move || {
                    shutdown.set();
                    shutdown.before_sweep();
                    if queued.swap(false, Ordering::AcqRel) {
                        match sweep {
                            Sweep::Cancels => slot.cancel(drop),
                            Sweep::Forgets => {}
                            Sweep::ResolvesFirst => {
                                let mut kept = None;
                                slot.cancel(|job| kept = Some(job));
                                drop(kept);
                            }
                        }
                    }
                })
            };

            // What `Executor::block_on` does: poll, and park until woken. A wake that comes
            // before the park leaves a token, which ends the park at once.
            let waker = Waker::from(Arc::new(MainWake(thread::current())));
            let result = loop {
                match slot.poll(&mut Context::from_waker(&waker)) {
                    Poll::Ready(result) => break result,
                    Poll::Pending => thread::park(),
                }
            };
            match result {
                Ok(value) => {
                    assert_eq!(value, 7);
                    assert_eq!(record.ran.load(Ordering::SeqCst), 1, "Ok, not run once");
                    let wrote = record.child_wrote.load(Ordering::Relaxed);
                    assert_eq!(wrote, 1, "Ok before the scope's child was seen to finish");
                }
                Err(error) => {
                    assert_eq!(error.kind(), lactose_core::ErrorKind::Cancelled);
                    assert_eq!(record.ran.load(Ordering::SeqCst), 0, "Cancelled, but it ran");
                    let dropped = record.dropped_unrun.load(Ordering::SeqCst);
                    assert_eq!(dropped, 1, "Cancelled before the closure was dropped");
                }
            }
            worker.join().unwrap();
            stopper.join().unwrap();
            let (ran, dropped) =
                (record.ran.load(Ordering::SeqCst), record.dropped_unrun.load(Ordering::SeqCst));
            assert_eq!(ran + dropped, 1, "run {ran} times and dropped unrun {dropped} times");
        });
    }

    #[test]
    fn a_job_taken_against_shutdown_resolves_once() {
        a_take_against_shutdown(Sweep::Cancels);
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_sweep_that_forgets_the_job() {
        a_take_against_shutdown(Sweep::Forgets);
    }

    #[test]
    #[should_panic(expected = "Cancelled before the closure was dropped")]
    fn weakened_a_cancel_that_resolves_before_the_drop() {
        a_take_against_shutdown(Sweep::ResolvesFirst);
    }

    // The width.

    /// A child that is one unit.
    const UNIT: usize = 1;
    /// A child that is a range of two units, which its taker halves.
    const RANGE: usize = 2;
    /// The upper half of such a range, one unit.
    const HALF: usize = 3;

    /// One job a model queues: the queue slot and kind of each child its scope forks, and
    /// the slot a range's half goes to.
    struct Job {
        children: &'static [(usize, usize)],
        half: usize,
    }

    /// The two queues, each one word of four-bit slots: every entry a model queues has a slot
    /// of its own, so a look at a queue is one load and a take one compare-exchange, which
    /// keeps the search small. A job is its number; a child is its job's number, shifted,
    /// and its kind.
    struct Queues {
        jobs: AtomicUsize,
        children: AtomicUsize,
    }

    /// The bits of one slot.
    const SLOT: usize = 4;

    /// Queues `entry` in its slot with release ordering, as `crossbeam-deque` publishes an
    /// entry. A read-modify-write and not a store: loom 0.7.2 orders a plain store only
    /// partially against another thread's read-modify-write of the same atomic, and then lets
    /// a later load read the value the read-modify-write wrote, on the storing thread too,
    /// which no modification order allows.
    fn push(queue: &AtomicUsize, slot: usize, entry: usize) {
        queue.fetch_or(entry << (SLOT * slot), Ordering::Release);
    }

    fn any(queue: &AtomicUsize) -> bool {
        queue.load(Ordering::Acquire) != 0
    }

    /// Takes the entry of the lowest slot that holds one.
    fn take(queue: &AtomicUsize) -> Option<usize> {
        let mut seen = queue.load(Ordering::Acquire);
        while seen != 0 {
            let shift = seen.trailing_zeros() as usize / SLOT * SLOT;
            let mask = 0xF << shift;
            match queue.compare_exchange(seen, seen & !mask, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some((seen & mask) >> shift),
                Err(now) => seen = now,
            }
        }
        None
    }

    impl Queued for Queues {
        fn jobs(&self) -> bool {
            any(&self.jobs)
        }

        fn children(&self) -> bool {
            any(&self.children)
        }
    }

    /// A frame-critical pool reduced to what the width touches, over the shipped `Width`,
    /// `Sleep`, `gate::idle_park` and `gate::wait_for_children`.
    struct Pool {
        width: Width,
        queues: Queues,
        sleep: Sleep,
        gate: Gate,
        threads: Vec<OnceLock<Thread>>,
        jobs: &'static [Job],
        latches: [Latch; 2],
        openers: [AtomicUsize; 2],
        /// The opener's `look`, or the loop as it was, which never looks after a wake.
        opener_looks: bool,
        /// Places held, as the model counts them.
        inside: AtomicUsize,
        done: AtomicUsize,
        finished: AtomicBool,
        /// One more than the worker that split a range and is held until its half has run.
        held: AtomicUsize,
        half_run: AtomicBool,
    }

    impl Wake for Pool {
        fn idle_one(&self) {
            if let Some(worker) = self.sleep.notify_one() {
                self.sleep.wake_worker(worker, || self.unpark(worker));
            }
        }

        /// A worker that has not started yet has neither announced nor taken a job, so
        /// nothing can be waiting for it to wake.
        fn unpark(&self, worker: usize) {
            if let Some(thread) = self.threads[worker].get() {
                thread.unpark();
            }
        }
    }

    impl Pool {
        fn new(
            workers: usize,
            jobs: &'static [Job],
            weakened: Option<Weakened>,
            opener_looks: bool,
        ) -> Self {
            let mut width = Width::new(workers);
            width.weakened = weakened;
            Self {
                width,
                queues: Queues { jobs: AtomicUsize::new(0), children: AtomicUsize::new(0) },
                sleep: Sleep::new(workers),
                gate: Gate::new(workers),
                threads: (0..workers).map(|_| OnceLock::new()).collect(),
                jobs,
                latches: [Latch::new(), Latch::new()],
                openers: [AtomicUsize::new(0), AtomicUsize::new(0)],
                opener_looks,
                inside: AtomicUsize::new(0),
                done: AtomicUsize::new(0),
                finished: AtomicBool::new(false),
                held: AtomicUsize::new(0),
                half_run: AtomicBool::new(false),
            }
        }

        fn enter(&self) {
            let inside = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
            let limit = self.width.limit();
            assert!(inside <= limit, "{inside} workers hold places of a width of {limit}");
        }

        fn count_down(&self, job: usize, me: usize) {
            let opener = self.openers[job - 1].load(Ordering::SeqCst);
            if self.latches[job - 1].count_down(1) && opener != me {
                self.unpark(opener);
            }
        }

        /// Runs one child entry on `me`. A worker that holds a place of its own for it and
        /// splits a range is held until the range's half has run, so that nothing but the
        /// opener can run that half.
        fn run_child(&self, me: usize, child: usize, holds_a_place: bool) {
            let (job, kind) = (child >> 2, child & 3);
            match kind {
                RANGE => {
                    if holds_a_place {
                        self.held.store(me + 1, Ordering::SeqCst);
                    }
                    let opener = self.openers[job - 1].load(Ordering::SeqCst);
                    let half = self.jobs[job - 1].half;
                    push(&self.queues.children, half, job << 2 | HALF);
                    self.width.queued(Place::Child, (opener != me).then_some(opener), self);
                    self.count_down(job, me);
                    if holds_a_place {
                        while !self.half_run.load(Ordering::SeqCst) {
                            thread::park();
                        }
                    }
                }
                HALF => {
                    self.count_down(job, me);
                    self.half_run.store(true, Ordering::SeqCst);
                    if let Some(holder) = self.held.load(Ordering::SeqCst).checked_sub(1) {
                        self.unpark(holder);
                    }
                }
                _ => self.count_down(job, me),
            }
        }

        fn run_job(&self, me: usize, job: usize) {
            self.openers[job - 1].store(me, Ordering::SeqCst);
            let latch = &self.latches[job - 1];
            let children = self.jobs[job - 1].children;
            latch.add(children.iter().map(|&(_, kind)| if kind == RANGE { 2 } else { 1 }).sum());
            for &(slot, kind) in children {
                push(&self.queues.children, slot, job << 2 | kind);
                self.width.queued(Place::Child, None, self);
            }
            // The job holds a permit, as a worker running one does.
            self.gate.retake();
            let mut opener = ModelOpener { pool: self, me, job };
            gate::wait_for_children(latch, &self.gate, me, self, &mut opener);
            self.gate.release(self);
        }

        fn finish(&self) {
            self.finished.store(true, Ordering::SeqCst);
            self.sleep.notify_all(|_| {});
            (0..self.threads.len()).for_each(|worker| {
                self.sleep.wake_worker(worker, || self.unpark(worker));
            });
        }

        /// A worker that holds no place: what `find_work` and `idle` do about such work.
        /// A worker that has run a job stops, as if held by other work, so a job queued
        /// behind it has to come by the wake-up its lowering of the count gives.
        fn work(&self, me: usize) {
            self.threads[me].set(thread::current()).unwrap();
            while !self.finished.load(Ordering::SeqCst) {
                if self.queues.children() && self.width.try_enter(Place::Child) {
                    let child = take(&self.queues.children);
                    if let Some(child) = child {
                        self.enter();
                        self.run_child(me, child, true);
                        self.inside.fetch_sub(1, Ordering::SeqCst);
                    }
                    self.width.leave(Place::Child, &self.queues, self);
                    if child.is_some() {
                        continue;
                    }
                }
                if self.queues.jobs() && self.width.try_enter(Place::Job) {
                    let job = take(&self.queues.jobs);
                    if let Some(job) = job {
                        self.enter();
                        self.run_job(me, job);
                        self.inside.fetch_sub(1, Ordering::SeqCst);
                    }
                    self.width.leave(Place::Job, &self.queues, self);
                    if job.is_some() {
                        if self.done.fetch_add(1, Ordering::SeqCst) + 1 == self.jobs.len() {
                            self.finish();
                        }
                        return;
                    }
                }
                gate::idle_park(&self.gate, &self.sleep, me, || {
                    let seen = if self.finished.load(Ordering::SeqCst) {
                        Look::Stop
                    } else if self.width.admits(&self.queues) {
                        Look::Work
                    } else {
                        Look::Nothing
                    };
                    (seen, None)
                });
            }
        }
    }

    /// The opener of a job's scope, whose children wait on the pool's queue.
    struct ModelOpener<'a> {
        pool: &'a Pool,
        me: usize,
        job: usize,
    }

    impl Opener for ModelOpener<'_> {
        /// The gate is open in these models.
        fn run_children(&mut self) -> bool {
            while let Some(child) = take(&self.pool.queues.children) {
                let of = child >> 2;
                assert_eq!(of, self.job, "the opener of job {} ran a child of job {of}", self.job);
                self.pool.run_child(self.me, child, false);
            }
            false
        }

        fn shutting_down(&self) -> bool {
            false
        }

        fn spin(&mut self, latch: &Latch) -> bool {
            latch.is_set()
        }

        fn look(&mut self) -> bool {
            self.pool.opener_looks && self.pool.queues.children()
        }
    }

    /// Runs `jobs` on `workers` frame-critical workers. With `main_queues` the main thread
    /// queues them one after the other while the workers run; otherwise they are queued
    /// before the workers start, which leaves the wakes to the workers' own queueing and
    /// lowering and keeps a search of three workers within reach. Every job is taken and
    /// every child run, which a lost wake-up turns into a deadlock.
    fn the_width(
        workers: usize,
        jobs: &'static [Job],
        main_queues: bool,
        weakened: Option<Weakened>,
        opener_looks: bool,
        bound: usize,
    ) {
        bounded(bound, move || {
            let pool = Arc::new(Pool::new(workers, jobs, weakened, opener_looks));
            if !main_queues {
                for job in 0..jobs.len() {
                    push(&pool.queues.jobs, job, job + 1);
                }
            }
            let threads: Vec<_> = (0..workers)
                .map(|me| {
                    let pool = pool.clone();
                    thread::spawn(move || pool.work(me))
                })
                .collect();
            if main_queues {
                for job in 0..jobs.len() {
                    push(&pool.queues.jobs, job, job + 1);
                    pool.width.queued(Place::Job, None, &*pool);
                }
            }
            for thread in threads {
                thread.join().unwrap();
            }
            assert!(pool.latches.iter().all(Latch::is_set));
            assert_eq!(pool.width.occupied(), 0);
        });
    }

    /// The width of one: two frame-critical workers, two jobs, the first forking a range
    /// that its opener halves and runs alone.
    const ONE: &[Job] = &[Job { children: &[(0, RANGE)], half: 1 }, Job { children: &[], half: 0 }];
    /// A width of two with two jobs in flight on three workers: the first forks two
    /// children, which a worker without a place competes for, and the second one.
    const TWO_JOBS: &[Job] = &[
        Job { children: &[(0, UNIT), (1, UNIT)], half: 0 },
        Job { children: &[(2, UNIT)], half: 0 },
    ];
    /// A width of two with one job on three workers, whose scope forks a range: the worker
    /// that takes it and queues the half is held until the half has run, which only the
    /// opener can then do.
    const RANGE_HELD: &[Job] = &[Job { children: &[(0, RANGE)], half: 1 }];

    /// The preemption bound of the width models: four threads with the sleep protocol.
    const WIDTH_BOUND: usize = 2;

    #[test]
    fn a_width_of_one() {
        the_width(2, ONE, true, None, true, WIDTH_BOUND);
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_width_of_one_without_the_push_fence() {
        the_width(2, ONE, true, Some(Weakened::PushFence), true, WIDTH_BOUND);
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_width_of_one_without_the_leave_fence() {
        the_width(2, ONE, true, Some(Weakened::LeaveFence), true, WIDTH_BOUND);
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_width_of_one_with_acquire_release_fences() {
        the_width(2, ONE, true, Some(Weakened::AcqRelFences), true, WIDTH_BOUND);
    }

    /// The count read and raised in two steps. Two workers with no place racing for a
    /// child of `TWO_JOBS` is the same race, but beside its opener it needs three
    /// preemptions, past `WIDTH_BOUND`.
    #[test]
    #[should_panic(expected = "workers hold places of a width of")]
    fn weakened_a_width_of_one_raised_in_two_steps() {
        the_width(2, ONE, true, Some(Weakened::SplitRaise), true, WIDTH_BOUND);
    }

    #[test]
    fn a_width_of_two_with_two_jobs() {
        the_width(3, TWO_JOBS, false, None, true, WIDTH_BOUND);
    }

    /// Two jobs side by side, a job raising the count as a child does. Dropping the leave
    /// fence fails nothing here: both jobs are queued before the workers start, so a worker
    /// that lowers the count always finds the second one, and an opener takes its own
    /// children whatever it misses. The width of one is where that fence is shown.
    #[test]
    #[should_panic(expected = "ran a child of job")]
    fn weakened_two_jobs_side_by_side() {
        the_width(3, TWO_JOBS, false, Some(Weakened::NoJobFlag), true, WIDTH_BOUND);
    }

    #[test]
    fn a_width_of_two_with_a_range_held() {
        the_width(3, RANGE_HELD, false, None, true, WIDTH_BOUND);
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_range_held_beside_an_opener_that_does_not_look() {
        the_width(3, RANGE_HELD, false, None, false, WIDTH_BOUND);
    }

    /// One job with no children, which is all a model of the count's wakes needs queued.
    const ONE_JOB: &[Job] = &[Job { children: &[], half: 0 }];

    /// A job queued while the count is at a width of two with the flag clear, because a
    /// worker that took a place for a child it then found gone has not given it back yet,
    /// against that worker's `leave`: a job arriving against a lowering at a width above one,
    /// which the configurations above leave out. A straggling child's place is held
    /// throughout; a third worker sleeps, and only a wake from the queueing or the lowering
    /// reaches it.
    fn the_transient_place(weakened: Option<Weakened>) {
        bounded(TRANSIENT_BOUND, move || {
            let pool = Arc::new(Pool::new(3, ONE_JOB, weakened, true));
            assert!(pool.width.try_enter(Place::Child));
            pool.inside.fetch_add(1, Ordering::SeqCst);
            let transient = {
                let pool = pool.clone();
                thread::spawn(move || {
                    // It goes on to other work without looking at the jobs.
                    if pool.width.try_enter(Place::Child) {
                        pool.width.leave(Place::Child, &pool.queues, &*pool);
                    }
                })
            };
            let sleeper = {
                let pool = pool.clone();
                thread::spawn(move || pool.work(2))
            };
            push(&pool.queues.jobs, 0, 1);
            pool.width.queued(Place::Job, None, &*pool);
            transient.join().unwrap();
            sleeper.join().unwrap();
            assert_eq!(pool.done.load(Ordering::SeqCst), 1);
        });
    }

    /// The preemption bound of the transient place's model.
    const TRANSIENT_BOUND: usize = 2;

    #[test]
    fn a_job_against_a_transient_child_place() {
        the_transient_place(None);
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_transient_child_place_without_the_push_fence() {
        the_transient_place(Some(Weakened::PushFence));
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_transient_child_place_without_the_leave_fence() {
        the_transient_place(Some(Weakened::LeaveFence));
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_transient_child_place_with_acquire_release_fences() {
        the_transient_place(Some(Weakened::AcqRelFences));
    }

    /// A child queued while a width of two is full, which only the wake of a holder's
    /// `leave` can deliver. The main thread is the scope's opener: it holds the job's place,
    /// waits on the latch and takes no child. A holder gives its child's place up; a third
    /// worker, which holds no place, is the only one that can run the queued child.
    fn the_leave(weakened: Option<Weakened>) {
        bounded(LEAVE_BOUND, move || {
            let pool = Arc::new(Pool::new(3, ONE_JOB, weakened, true));
            // The main thread is worker 0, the opener of job 1.
            pool.threads[0].set(thread::current()).unwrap();
            assert!(pool.width.try_enter(Place::Job));
            assert!(pool.width.try_enter(Place::Child));
            pool.inside.fetch_add(2, Ordering::SeqCst);
            pool.latches[0].add(1);
            push(&pool.queues.children, 0, 1 << 2 | UNIT);
            let holder = {
                let pool = pool.clone();
                thread::spawn(move || {
                    pool.inside.fetch_sub(1, Ordering::SeqCst);
                    pool.width.leave(Place::Child, &pool.queues, &*pool);
                })
            };
            let sleeper = {
                let pool = pool.clone();
                thread::spawn(move || pool.work(2))
            };
            while !pool.latches[0].is_set() {
                thread::park();
            }
            pool.finish();
            holder.join().unwrap();
            sleeper.join().unwrap();
        });
    }

    /// The preemption bound of the leave's model.
    const LEAVE_BOUND: usize = 1;

    #[test]
    fn a_child_delivered_by_a_leave() {
        the_leave(None);
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn weakened_a_leave_that_does_not_count_the_children() {
        the_leave(Some(Weakened::ChildrenNotAdmitted));
    }

    // The shutdown flag.

    /// A job's queue and a task's queue, and how many entries were refused or swept.
    struct Submissions {
        flag: ShutdownFlag,
        jobs: AtomicUsize,
        tasks: AtomicUsize,
        refused: AtomicUsize,
        swept: AtomicUsize,
    }

    impl Submissions {
        /// What `Shared::submit_job` and `Shared::submit_task` do: `ShutdownFlag::queue`.
        fn submit(&self, queue: &AtomicUsize) {
            self.flag.queue(
                1,
                |_| {
                    self.refused.fetch_add(1, Ordering::SeqCst);
                },
                |entry| push(queue, 0, entry),
                || self.sweep(),
            );
        }

        /// What `Shared::discard_queued` does, in its order: the tasks' queue, then the
        /// jobs'. A queue that looks empty is not written, as a `crossbeam-deque` steal from
        /// an empty injector writes nothing.
        fn sweep(&self) {
            self.flag.before_sweep();
            for queue in [&self.tasks, &self.jobs] {
                if queue.load(Ordering::Acquire) != 0 && queue.swap(0, Ordering::AcqRel) != 0 {
                    self.swept.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    /// A job and a task queued against shutdown, whose thread sets the flag and sweeps both
    /// queues: each entry is refused or swept, whichever side sees the other.
    fn submissions_against_shutdown(weakened: Option<shutdown::Weakened>) {
        bounded(SUBMISSIONS_BOUND, move || {
            let mut flag = ShutdownFlag::new();
            flag.weakened = weakened;
            let state = Arc::new(Submissions {
                flag,
                jobs: AtomicUsize::new(0),
                tasks: AtomicUsize::new(0),
                refused: AtomicUsize::new(0),
                swept: AtomicUsize::new(0),
            });
            let submitters: Vec<_> = [false, true]
                .into_iter()
                .map(|job| {
                    let state = state.clone();
                    thread::spawn(move || {
                        state.submit(if job { &state.jobs } else { &state.tasks });
                    })
                })
                .collect();
            state.flag.set();
            state.sweep();
            for submitter in submitters {
                submitter.join().unwrap();
            }
            let resolved =
                state.refused.load(Ordering::SeqCst) + state.swept.load(Ordering::SeqCst);
            assert_eq!(resolved, 2, "an entry stayed queued after shutdown's sweep");
        });
    }

    /// The preemption bound of the submissions' model.
    const SUBMISSIONS_BOUND: usize = 1;

    #[test]
    fn submissions_against_shutdown_are_refused_or_swept() {
        submissions_against_shutdown(None);
    }

    #[test]
    #[should_panic(expected = "an entry stayed queued after shutdown's sweep")]
    fn weakened_a_submission_that_does_not_check_after_queueing() {
        submissions_against_shutdown(Some(shutdown::Weakened::NoCheck));
    }

    #[test]
    #[should_panic(expected = "an entry stayed queued after shutdown's sweep")]
    fn weakened_a_submission_without_its_fence() {
        submissions_against_shutdown(Some(shutdown::Weakened::QueueFence));
    }

    #[test]
    #[should_panic(expected = "an entry stayed queued after shutdown's sweep")]
    fn weakened_a_sweep_without_its_fence() {
        submissions_against_shutdown(Some(shutdown::Weakened::SweepFence));
    }
}
