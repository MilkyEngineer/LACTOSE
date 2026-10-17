//! The step never waits on work that needs a pool thread to be woken outside rayon's guarantee.
//! Its waits are rayon's own, which run other work meanwhile and wake every thread they wait
//! on, or the staged solver's spin on a batch another thread is already running. Unpatched, Rapier queued
//! the broad phase's deferred tree optimisation as a detached job and blocked the stepping
//! thread on its result in a channel, so the step finished only when another thread took the
//! job, and hung when none did (`vendor/README.md`).
//!
//! The test holds every other thread of the pool with a job that does not return until the
//! test releases it, and steps a small scene. The scene keeps under 64 updating contact pairs:
//! above that the narrow phase broadcasts to every thread of the pool and waits for each of
//! them, so a larger scene would stall on the held threads by design (`README.md`).

#![forbid(unsafe_code)]

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Barrier};
use std::time::Duration;

use support::*;

/// Far beyond any step of the scene; the step either finishes or never does.
const PATIENCE: Duration = Duration::from_secs(20);
/// Enough steps of [`Walk`] for the broad phase to optimise its tree many times.
const TICKS: u32 = 120;

/// Jobs that hold `count` threads of a pool, one each, until released.
struct Held {
    armed: Arc<AtomicBool>,
    holding: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl Held {
    /// Spawns the jobs and returns once each runs on a thread of its own. Until armed, a job
    /// runs only what its own thread was given (the backend's per-thread setup among it), so
    /// no job picks up another and no pool work is lost to them.
    fn spawn(pool: &rayon::ThreadPool, count: usize) -> Self {
        let armed = Arc::new(AtomicBool::new(false));
        let running = Arc::new(Barrier::new(count + 1));
        let holding = Arc::new(Barrier::new(count + 1));
        let release = Arc::new(Barrier::new(count + 1));
        for _ in 0..count {
            let (armed, running) = (Arc::clone(&armed), Arc::clone(&running));
            let (holding, release) = (Arc::clone(&holding), Arc::clone(&release));
            pool.spawn(move || {
                running.wait();
                while !armed.load(Ordering::Acquire) {
                    if rayon::yield_local() != Some(rayon::Yield::Executed) {
                        std::thread::yield_now();
                    }
                }
                holding.wait();
                release.wait();
            });
        }
        running.wait();
        Self { armed, holding, release }
    }

    /// From here on each job blocks its thread outright.
    fn arm(&self) {
        self.armed.store(true, Ordering::Release);
        self.holding.wait();
    }

    fn release(&self) {
        self.release.wait();
    }
}

/// Steps [`Walk`] on a pool of `threads` threads with all but one held. Returns
/// whether every step finished while they were held.
fn steps_with_one_free_thread(threads: usize) -> bool {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    let held = Held::spawn(&pool, threads - 1);
    let mut b = lactose_physics_rapier::backend(pool);
    held.arm();
    let (done, finished) = mpsc::channel();
    let stepping = std::thread::spawn(move || {
        Walk::create(&mut *b);
        Walk::run(&mut *b, TICKS);
        done.send(()).unwrap();
        b
    });
    let in_time = match finished.recv_timeout(PATIENCE) {
        Ok(()) => true,
        Err(RecvTimeoutError::Timeout) => false,
        Err(RecvTimeoutError::Disconnected) => panic!("the stepping thread panicked"),
    };
    // A step stuck waiting for a held thread finishes once they are released, so the test
    // ends either way.
    held.release();
    drop(stepping.join().unwrap());
    in_time
}

#[test]
fn a_step_finishes_on_the_one_thread_not_held() {
    for threads in [2, 4] {
        assert!(
            steps_with_one_free_thread(threads),
            "{threads} threads, all but one held: {TICKS} steps did not finish in {PATIENCE:?}; \
             the step waits outside rayon on work only another solver thread can run",
        );
    }
}
