//! Panics abort: a job that panics takes the process down, it does not unwind into the
//! executor. Checked the way the architecture prescribes, by running the test binary again
//! as a subprocess and looking at how it died.

#![cfg(not(feature = "loom"))]
// Miri cannot spawn processes.
#![cfg(not(miri))]

mod common;

use std::process::{Command, Stdio};

use common::{Node, executor, graph};
use lactose_task::WorkerClass;

const CHILD_ENV: &str = "WHISK_ABORT_CASE";

/// In the parent, re-runs the calling test in a child process with `CHILD_ENV` set to
/// `case` and asserts that it aborted. In the child, returns `true` so the caller runs the
/// case.
fn aborts(test: &str, case: &str) -> bool {
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
    assert!(stderr.contains("deliberate panic"), "{case}: the case did not run:\n{stderr}");
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
        executor.block_on(executor.run_graph(graph(vec![node], &[])));
        unreachable!("the process aborts before the run resolves");
    }
    if aborts(TEST, "finalize") {
        let executor = executor(2, 1);
        let node = Node::new("abort::finalize")
            .finalize(|_| panic!("deliberate panic in finalize"))
            .build();
        executor.block_on(executor.run_graph(graph(vec![node], &[])));
        unreachable!("the process aborts before the run resolves");
    }
    if aborts(TEST, "scope child") {
        let executor = executor(2, 1);
        let node = Node::new("abort::child")
            .run(|_, ctx| ctx.scope(|scope| scope.fork(|_| panic!("deliberate panic in a child"))))
            .build();
        executor.block_on(executor.run_graph(graph(vec![node], &[])));
        unreachable!("the process aborts before the run resolves");
    }
    if aborts(TEST, "scope body") {
        let executor = executor(2, 1);
        let node = Node::new("abort::body")
            .run(|_, ctx| ctx.scope(|_| panic!("deliberate panic in a scope body")))
            .build();
        executor.block_on(executor.run_graph(graph(vec![node], &[])));
        unreachable!("the process aborts before the run resolves");
    }
    if aborts(TEST, "task") {
        let executor = executor(1, 1);
        let task =
            executor.spawn(WorkerClass::Background, async { panic!("deliberate panic in a task") });
        executor.block_on(task);
        unreachable!("the process aborts before the task resolves");
    }
}
