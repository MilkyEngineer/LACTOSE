//! loom models of the scope latch and the sleep-and-wake protocol.
//!
//! Run with `cargo test -p lactose-task --features loom --release`. Each model drives the
//! real `Latch` or `Sleep` the way the pool does and stands in only for what loom does not
//! model: the deque is one atomic counter, and a worker is a loom thread.
//!
//! A lost wake-up shows as a deadlock, which loom reports. The models are exhaustive: no
//! preemption bound is set unless `LOOM_MAX_PREEMPTIONS` asks for one.

use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::thread;

use crate::latch::Latch;
use crate::sleep::Sleep;

/// The preemption bound of the three-thread sleep model, which is too large to explore
/// exhaustively. `LOOM_MAX_PREEMPTIONS` overrides it.
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

/// A scope with two children stolen by other workers. The opener must not get past the wait
/// before both have run, and neither child may touch the latch after the decrement that
/// releases the opener (`Latch::retire` ends the modelled stack frame).
#[test]
fn latch_releases_the_opener_after_the_last_child() {
    loom::model(|| {
        let latch = Arc::new(Latch::new());
        let ran = Arc::new(AtomicUsize::new(0));
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

        latch.wait();
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
        let opener = thread::current();
        // What an unrelated wake-up of this worker leaves behind.
        opener.unpark();

        latch.add(2);
        let thief = {
            let (latch, ran, opener) = (latch.clone(), ran.clone(), opener.clone());
            thread::spawn(move || {
                ran.fetch_add(2, Ordering::Relaxed);
                if latch.count_down(2) {
                    opener.unpark();
                }
            })
        };
        latch.add(1);
        ran.fetch_add(1, Ordering::Relaxed);
        // The opener's own decrement needs no unpark.
        let _last = latch.count_down(1);

        latch.wait();
        assert_eq!(ran.load(Ordering::Relaxed), 3);
        latch.retire();
        thief.join().unwrap();
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

/// What a worker does after it makes an entry visible.
fn push_one_entry(sleep: &Sleep, queue: &AtomicUsize, workers: &[thread::Thread]) {
    queue.fetch_add(1, Ordering::Release);
    if let Some(worker) = sleep.notify_one() {
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
            thread::spawn(move || push_one_entry(&sleep, &queue, &workers))
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
                push_one_entry(&sleep, &queue, &workers);
                push_one_entry(&sleep, &queue, &workers);
            })
        };
        take_one_entry(&sleep, &queue, 0);
        other.join().unwrap();
        pusher.join().unwrap();
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
