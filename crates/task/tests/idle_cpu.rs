//! Waiting must not cost CPU. These measure the process's CPU time, not wall-clock time,
//! so a loaded machine cannot fail them: a waiting thread that is not scheduled burns
//! nothing either.
//!
//! One test only, and a test binary of its own: the measure is the whole process, and tests
//! of one binary share a process under `cargo test`.

#![cfg(not(feature = "loom"))]
// CPU time comes from `getrusage`, and Miri has neither it nor a real clock.
#![cfg(all(unix, not(miri)))]

mod common;

use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Node, executor, graph, wait_for};

/// The CPU time this process has used so far, user and system.
fn process_cpu() -> Duration {
    let mut usage = core::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` points to writable memory of the size and alignment of `rusage`,
    // which `getrusage` fills in when it returns zero, as asserted.
    let usage = unsafe {
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()), 0);
        usage.assume_init()
    };
    let time = |time: libc::timeval| {
        Duration::from_secs(time.tv_sec as u64) + Duration::from_micros(time.tv_usec as u64)
    };
    time(usage.ru_utime) + time(usage.ru_stime)
}

const WAIT: Duration = Duration::from_secs(1);
/// A busy core would cost the whole wait; idle workers cost a few milliseconds. The bound
/// leaves room for whatever else the test process does.
const BOUND: Duration = Duration::from_millis(300);

/// A `block_in_place` that reserves workers, with no work for them: the reserved permit
/// must not be handed from idle worker to idle worker, each waking the next.
fn idle_block(reserved: u32) -> Duration {
    let executor = executor(4, 1);
    let used = Arc::new(Mutex::new(Duration::ZERO));
    let node = {
        let used = used.clone();
        Node::new("idle::block")
            .run(move |_, ctx| {
                ctx.block_in_place(reserved, || {
                    // Let the other workers finish scanning and settle.
                    std::thread::sleep(Duration::from_millis(50));
                    let before = process_cpu();
                    std::thread::sleep(WAIT);
                    *used.lock().unwrap() = process_cpu() - before;
                });
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    executor.shutdown();
    *used.lock().unwrap()
}

/// Shutdown while an opener waits for a child that another worker is running: the opener
/// has nothing on its own deque and must park until the child is done, not spin.
fn opener_during_shutdown() -> Duration {
    let executor = executor(2, 1);
    let stolen = Arc::new(AtomicBool::new(false));
    let node = {
        let stolen = stolen.clone();
        Node::new("idle::opener")
            .run(move |_, ctx| {
                let stolen = &stolen;
                ctx.scope(|scope| {
                    // Forked first, so it is the one a thief takes: it marks itself stolen
                    // and then outlasts the shutdown call.
                    scope.fork(move |_| {
                        stolen.store(true, Ordering::Release);
                        std::thread::sleep(WAIT);
                    });
                    // Keeps the opener busy until the other child is with the thief, so
                    // the opener cannot end up running the long one itself.
                    scope.fork(move |_| wait_for(stolen));
                });
            })
            .build()
    };
    let run = executor.run_graph(graph(vec![node], &[]));
    wait_for(&stolen);
    // The opener has run its own child, spun on the latch and parked.
    std::thread::sleep(Duration::from_millis(50));
    let before = process_cpu();
    // Joins the workers, which takes as long as the stolen child.
    executor.clone().shutdown();
    let used = process_cpu() - before;
    // Its job had already started, so the run is cancelled only once the job returned.
    assert!(executor.block_on(run).is_err());
    used
}

#[test]
fn waiting_workers_do_not_burn_cpu() {
    for reserved in [0, 1, 2] {
        let used = idle_block(reserved);
        assert!(
            used < BOUND,
            "{used:?} of CPU during a {WAIT:?} block_in_place({reserved}) with nothing to run"
        );
    }
    let used = opener_during_shutdown();
    assert!(used < BOUND, "{used:?} of CPU while an opener waited {WAIT:?} during shutdown");
}
