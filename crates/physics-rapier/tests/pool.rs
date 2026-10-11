//! A step never blocks a pool thread on a job only another thread can run. Unpatched, Rapier
//! queued the broad phase's deferred tree optimisation as a detached job and blocked the
//! stepping thread on its result, so the step finished only when another thread took the job,
//! and hung when none did (`vendor/README.md`).
//!
//! The first test holds every other thread of the pool with a job that does not return until
//! the test releases it, and steps a small scene. The scene keeps under 64 updating contact
//! pairs: above that the narrow phase broadcasts to every thread of the pool and waits for each
//! of them, so a larger scene would stall on the held threads by design (`README.md`). The
//! second, ignored, holds nothing: it steps the scene on six two-thread pools in a row, the
//! shape that hung the unpatched build without help.

#![forbid(unsafe_code)]

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Barrier};
use std::time::Duration;

use lactose_physics_api::{PhysicsBackend, QueryFilter, Ray, Shape};
use support::*;

/// Far beyond any step of the scene; the step either finishes or never does.
const PATIENCE: Duration = Duration::from_secs(20);
/// Enough steps of [`Walk`] for the broad phase to optimise its tree many times.
const TICKS: u32 = 120;
/// Two-thread backends the stress test steps one after another.
const BACKENDS: usize = 6;
/// Steps of [`Walk`] per backend in the stress test.
const STRESS_TICKS: u32 = 900;

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
             the step waits on work only another solver thread can run",
        );
    }
}

/// Queries between steps, as a game makes them; their answers are not checked here.
fn query(backend: &dyn PhysicsBackend) {
    for i in 0..7 {
        let origin = v(-3.0 + f64::from(i), 12.0, 0.3);
        let ray = Ray { origin, direction: v(0.0, -1.0, 0.0), max_distance: s(30.0) };
        std::hint::black_box(backend.cast_ray(&ray, &QueryFilter::ALL));
    }
    let mut hits = Vec::new();
    backend.overlap(&Shape::sphere(s(2.0)), &at(0.0, 1.0, 0.0), &QueryFilter::ALL, &mut hits);
    let sphere = Shape::sphere(s(0.3));
    let towards = v(1.0, 0.0, 0.0);
    std::hint::black_box(backend.cast_shape(
        &sphere,
        &at(-6.0, 0.5, 0.0),
        towards,
        s(12.0),
        &QueryFilter::ALL,
    ));
}

#[test]
#[ignore = "a stress run of a rare race: cargo nextest run --run-ignored only"]
fn walks_on_two_thread_pools_in_a_row_finish() {
    let (done, finished) = mpsc::channel();
    // Not joined: when a step hangs the test fails and the process ends with it.
    std::thread::spawn(move || {
        for _ in 0..BACKENDS {
            let mut b = backend(2);
            Walk::create(&mut *b);
            let mut last = None;
            for tick in 1..=STRESS_TICKS {
                let commands = Walk::drive(&mut *b, tick, last);
                let output = step(&mut *b, tick, &commands);
                last = state(&output, key(WALKER)).map(|s| s.velocity);
                query(&*b);
                if tick % 20 == 0 {
                    std::hint::black_box(snapshot(&*b));
                }
            }
            done.send(()).unwrap();
        }
    });
    for n in 1..=BACKENDS {
        match finished.recv_timeout(PATIENCE) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => panic!(
                "backend {n} of {BACKENDS}: {STRESS_TICKS} two-thread steps did not finish in \
                 {PATIENCE:?}"
            ),
            Err(RecvTimeoutError::Disconnected) => panic!("the stepping thread panicked"),
        }
    }
}
