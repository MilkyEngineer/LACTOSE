//! Panics abort: a job that panics takes the process down, it does not unwind into the
//! executor. Checked the way the architecture prescribes, by running the test binary again
//! as a subprocess and looking at how it died.

#![cfg(not(feature = "loom"))]
// Miri cannot spawn processes.
#![cfg(not(miri))]

mod common;

use std::process::{Command, Stdio};

use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use common::{Node, executor, graph, wait_for};
use lactose_task::{ChaosConfig, Executor, WorkerClass};

const CHILD_ENV: &str = "WHISK_ABORT_CASE";

/// In the parent, re-runs the calling test in a child process with `CHILD_ENV` set to
/// `case` and asserts that it aborted after printing `expected`. In the child, returns
/// `true` so the caller runs the case.
fn aborts(test: &str, case: &str) -> bool {
    aborts_saying(test, case, "deliberate panic")
}

fn aborts_saying(test: &str, case: &str, expected: &str) -> bool {
    if std::env::var(CHILD_ENV).is_ok_and(|active| active == case) {
        return true;
    }
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture", "--test-threads", "1"])
        .env(CHILD_ENV, case)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(expected), "{case}: the case did not run:\n{stderr}");
    assert!(!output.status.success(), "{case}: the process survived a panicking job");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            output.status.signal(),
            Some(6),
            "{case}: expected SIGABRT, got {:?}",
            output.status
        );
    }
    false
}

#[test]
fn a_panic_in_a_job_child_or_task_aborts_the_process() {
    const TEST: &str = "a_panic_in_a_job_child_or_task_aborts_the_process";

    if aborts(TEST, "graph job") {
        let executor = executor(2, 1);
        let node = Node::new("abort::job").run(|_, _| panic!("deliberate panic in a job")).build();
        executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
        unreachable!("the process aborts before the run resolves");
    }
    if aborts(TEST, "finalize") {
        let executor = executor(2, 1);
        let node = Node::new("abort::finalize")
            .finalize(|_| panic!("deliberate panic in finalize"))
            .build();
        executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
        unreachable!("the process aborts before the run resolves");
    }
    if aborts(TEST, "scope child") {
        let executor = executor(2, 1);
        let node = Node::new("abort::child")
            .run(|_, ctx| ctx.scope(|scope| scope.fork(|_| panic!("deliberate panic in a child"))))
            .build();
        executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
        unreachable!("the process aborts before the run resolves");
    }
    if aborts(TEST, "scope body") {
        let executor = executor(2, 1);
        let node = Node::new("abort::body")
            .run(|_, ctx| ctx.scope(|_| panic!("deliberate panic in a scope body")))
            .build();
        executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
        unreachable!("the process aborts before the run resolves");
    }
    if aborts(TEST, "task") {
        let executor = executor(1, 1);
        let task =
            executor.spawn(WorkerClass::Background, async { panic!("deliberate panic in a task") });
        let _ = executor.block_on(task);
        unreachable!("the process aborts before the task resolves");
    }
}

/// Panics when dropped.
struct PanicsOnDrop;

impl Drop for PanicsOnDrop {
    fn drop(&mut self) {
        panic!("deliberate panic in a destructor");
    }
}

/// A future is the task's code, and so is its destructor: a panic while a cancelled
/// task's future is dropped aborts like a panic in a poll, on whichever thread the
/// cancellation runs.
#[test]
fn a_panic_while_a_cancelled_future_is_dropped_aborts_the_process() {
    const TEST: &str = "a_panic_while_a_cancelled_future_is_dropped_aborts_the_process";
    let suspended = |executor: &Executor| {
        let started = Arc::new(AtomicBool::new(false));
        let task = {
            let started = started.clone();
            executor.spawn(WorkerClass::Background, async move {
                let _held = PanicsOnDrop;
                started.store(true, Ordering::Release);
                core::future::pending::<()>().await;
            })
        };
        wait_for(&started);
        task
    };

    if aborts(TEST, "handle dropped") {
        let executor = executor(1, 1);
        // The task is suspended, so the drop happens here, on the test's own thread.
        drop(suspended(&executor));
        unreachable!("the process aborts in the destructor");
    }
    if aborts(TEST, "shutdown") {
        let executor = executor(1, 1);
        let task = suspended(&executor);
        executor.shutdown();
        drop(task);
        unreachable!("the process aborts in the destructor");
    }
    if aborts(TEST, "last waker dropped") {
        let executor = executor(1, 1);
        suspended(&executor).detach();
        // Detached and suspended on nothing: the worker drops the last reference.
        std::thread::sleep(std::time::Duration::from_secs(30));
        unreachable!("the process aborts in the destructor");
    }
}

/// A log from another program cannot be followed. Replay notices that the job the log
/// names next is not there while other work waits and nothing is running, and stops with
/// a diagnostic instead of hanging until a test timeout.
#[test]
fn a_log_that_cannot_be_followed_aborts_with_a_diagnostic() {
    const TEST: &str = "a_log_that_cannot_be_followed_aborts_with_a_diagnostic";
    let builder = || Executor::builder().frame_workers(2).background_workers(1);
    let program = |units| graph(vec![Node::new("abort::replay").units(units).build()], &[]);

    if aborts_saying(TEST, "replay", "the execution log cannot be replayed by this program") {
        let recorder = builder()
            .chaos(ChaosConfig { record: true, ..ChaosConfig::default() })
            .build()
            .unwrap();
        recorder.block_on(recorder.run_graph(program(4))).unwrap();
        let log = recorder.execution_log().unwrap();
        recorder.shutdown();

        // The same log against a node with one unit: units 1 to 3 never exist.
        let replaying = builder().replay(log).build().unwrap();
        let _ = replaying.block_on(replaying.run_graph(program(1)));
        unreachable!("the process aborts when the replay stalls");
    }
}
