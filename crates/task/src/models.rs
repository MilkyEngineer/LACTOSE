//! loom models of the scope latch, the sleep-and-wake protocol, the permit gate and run
//! quiescence.
//!
//! Run with `cargo test -p lactose-task --features loom --release`. Every model calls the
//! functions the executor ships: `gate::wait_for_children` is the opener's wait,
//! `Sleep::announce_entry` is what a worker does after queueing work, `Quiesce` is what
//! holds a run's future back. What a model supplies is only what loom does not model: a
//! deque is a counter, a worker is a loom thread, and a scope's children are a number.
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
        workers[worker].unpark();
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
                    main.unpark();
                }
                flag.store(2, Ordering::Release);
                sleep.notify_all(|worker| {
                    assert_eq!(worker, 0);
                    main.unpark();
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
            self.threads[worker].unpark();
        }
    }

    fn unpark(&self, worker: usize) {
        self.threads[worker].unpark();
    }
}

/// Two idle workers under a closed gate that reserves one permit, and one entry that is
/// queued while they are going to sleep. A worker that scans and finds nothing gives its
/// permit up without waking the other (`Gate::release_idle`), then sleeps through
/// `gate::idle_park`. Whatever the interleaving, the entry is taken: by the releasing
/// worker's own re-check if it was queued before, by the pusher's wake-up if after.
#[test]
fn an_idle_release_wakes_nobody_and_loses_no_work() {
    /// The worker loop reduced to its permit and sleep steps. Returns when the entry has
    /// been taken, by this worker or the other.
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
                    // Stands for shutdown: lets the other worker's model thread end.
                    taken.store(true, Ordering::SeqCst);
                    sleep.notify_all(|worker| wake.unpark(worker));
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
/// stepped timed park; under loom a step is a plain park, so the keeper can only be woken
/// by the hand-off itself, and a lost hand-off is a deadlock here.
#[test]
fn an_earlier_timer_reaches_the_parked_keeper() {
    use core::task::Waker;
    use std::time::{Duration, Instant};

    use crate::time::{Rearm, Timers};

    loom::model(|| {
        let gate = Arc::new(Gate::new(1));
        let sleep = Arc::new(Sleep::new(1));
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
                // Once the keeper watches the later deadline, register the earlier one.
                while watching.load(Ordering::SeqCst) != 2 {
                    thread::yield_now();
                }
                let mut earlier_id = None;
                match timers.register(earlier, &mut earlier_id, Waker::noop()) {
                    Rearm::Keeper(worker) => {
                        if sleep.notify(worker) {
                            keeper.unpark();
                        }
                    }
                    Rearm::AnySleeper => {
                        if let Some(worker) = sleep.notify_one() {
                            assert_eq!(worker, 0);
                            keeper.unpark();
                        }
                    }
                    Rearm::Nobody => unreachable!("the earlier deadline is new"),
                }
                // The keeper must come to watch the earlier deadline on its own.
                while watching.load(Ordering::SeqCst) != 1 {
                    thread::yield_now();
                }
                stop.store(true, Ordering::SeqCst);
                sleep.notify_all(|_| keeper.unpark());
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
