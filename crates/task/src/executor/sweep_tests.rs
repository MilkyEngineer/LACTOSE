//! Shutdown's sweep against a run that resolves with its graph: what only a pause inside the
//! sweep can show. Every wait is bounded, so a regression fails with a message instead of
//! hanging.

use core::ops::Range;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::{Executor, GraphBuilder, GraphNode, JobContext};

/// How long a wait may last that only a regression runs out.
const LIMIT: Duration = Duration::from_secs(if cfg!(miri) { 600 } else { 60 });

/// Waits until `done` holds, failing the test with `what` after [`LIMIT`].
fn wait_until(what: &str, done: impl Fn() -> bool) {
    let begun = Instant::now();
    while !done() {
        assert!(begun.elapsed() < LIMIT, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Polls `future` on this thread until it resolves, failing the test after [`LIMIT`].
fn resolved<F: Future>(future: F) -> F::Output {
    let mut future = core::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    let begun = Instant::now();
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        assert!(begun.elapsed() < LIMIT, "the run did not resolve in {LIMIT:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A node that holds `token` and whose finalize waits for `go` once it has begun.
struct HeldInFinalize {
    _token: Arc<()>,
    finalizing: Arc<AtomicBool>,
    go: Arc<AtomicBool>,
}

impl GraphNode for HeldInFinalize {
    fn name(&self) -> &'static str {
        "executor::sweep_tests::held_in_finalize"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, _ctx: &mut JobContext<'_>) {}

    fn finalize(&self, _ctx: &mut JobContext<'_>) {
        self.finalizing.store(true, Ordering::SeqCst);
        wait_until("the test lets the finalize go", || self.go.load(Ordering::SeqCst));
    }
}

/// Shutdown's sweep finds a run whose last finalize is past its shutdown check, and is held
/// there, with what it upgraded, while the run resolves with the graph and the test drops
/// it: what the sweep holds is the run's control, never the graph.
#[test]
fn shutdowns_sweep_holds_no_reference_to_a_graph_handed_back() {
    let executor = Executor::builder().frame_workers(2).background_workers(1).build().unwrap();
    let token = Arc::new(());
    let finalizing = Arc::new(AtomicBool::new(false));
    let go = Arc::new(AtomicBool::new(false));
    let (found, wait_found) = mpsc::channel();
    let (looked, wait_looked) = mpsc::channel::<()>();
    let wait_looked = Mutex::new(wait_looked);
    *executor.owner.shared.sweep_pause.lock().unwrap() = Some(Box::new(move || {
        let _ = found.send(());
        // Ends when the test drops its sender, having looked or having failed.
        let _ = wait_looked.lock().unwrap().recv_timeout(LIMIT);
    }));
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(HeldInFinalize {
        _token: token.clone(),
        finalizing: finalizing.clone(),
        go: go.clone(),
    }));
    let run = executor.run_graph(builder.build().unwrap());
    wait_until("the finalize began", || finalizing.load(Ordering::SeqCst));
    let shutting_down = {
        let executor = executor.clone();
        std::thread::spawn(move || executor.shutdown())
    };
    wait_found.recv_timeout(LIMIT).expect("the sweep found the run");
    go.store(true, Ordering::SeqCst);
    let graph = resolved(run).expect("the finalize was past its check, so the run finishes");
    drop(graph);
    let held = Arc::strong_count(&token);
    drop(looked);
    shutting_down.join().unwrap();
    assert_eq!(held, 1, "shutdown's sweep held the graph after the run resolved with it");
}
