//! The tests that open a window (`README.md`, "Proof"), as one binary with its own `main`.
//!
//! A window's event loop runs on the process's main thread, which a test harness keeps for
//! itself, so each case is this binary run again as a subprocess whose `main` is the case:
//! the parent, which speaks enough of the test harness's arguments for `cargo nextest` and
//! `cargo test` to list and run the cases, keeps the child under a deadline and checks how
//! it ended.
//!
//! Every case is ignored, so a run that does not ask for them, as CI's does, lists them and
//! runs none: no case passes where no window opens. On a machine with a display:
//!
//! ```text
//! cargo nextest run -p lactose-platform --test window --run-ignored only
//! ```

// The one module that reaches the operating system directly, to do what a user does to a
// window and to read back a thread's scheduling, allows it (`SAFETY.md`).
#![deny(unsafe_code)]

use core::fmt::Debug;
use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::io::Read;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::task::Wake;
use std::time::{Duration, Instant};

use lactose_core::{Error, ErrorKind, Result, ThreadClass};
use lactose_platform::{
    ClientPlatform, EventPump, LOG_TARGET, MainThread, PixelSize, Window, WindowDesc, event_pump,
    present_thread,
};
use lactose_task::{Executor, Lane, Platform, StdPlatform, ThreadHandle, ThreadSpec, WorkerClass};
use raw_window_handle::{HandleError, HasDisplayHandle, HasWindowHandle};

/// The variable that makes this binary a case's child, naming the case.
const CHILD: &str = "PAIL_WINDOW_CASE";
/// The stage a child prints last when every check passed.
const FINAL_STAGE: &str = "done";
/// How long a check waits for anything that should come at once: far past what it takes,
/// so that only a hang reaches it.
const BOUND: Duration = Duration::from_secs(10);
/// How long a child waits for the operating system to answer a request of the window.
const ANSWER: Duration = Duration::from_secs(5);

/// How a case's child must end.
#[derive(Clone, Copy)]
enum Expect {
    /// With a zero status, the final stage printed last.
    Done,
    /// With a status that is not zero, this stage printed last and this text on stderr.
    FailsAfter { stage: &'static str, says: &'static str },
    /// Aborted, having printed this text on stderr.
    Aborts { says: &'static str },
}

struct Case {
    name: &'static str,
    child: fn() -> Result<()>,
    expect: Expect,
}

const CASES: &[Case] = &[
    Case { name: "opens_pumps_and_closes", child: opens_pumps_and_closes, expect: Expect::Done },
    Case {
        name: "a_call_queued_before_run_runs_and_one_after_is_cancelled",
        child: a_call_queued_before_run_runs_and_one_after_is_cancelled,
        expect: Expect::Done,
    },
    Case {
        name: "a_dropped_pump_cancels_its_calls_and_a_second_pump_is_unsupported",
        child: a_dropped_pump_cancels_its_calls_and_a_second_pump_is_unsupported,
        expect: Expect::Done,
    },
    Case {
        name: "the_handles_are_lent_on_the_main_thread_alone",
        child: the_handles_are_lent_on_the_main_thread_alone,
        expect: Expect::Done,
    },
    Case {
        name: "a_requested_size_and_a_minimise_are_reported",
        child: a_requested_size_and_a_minimise_are_reported,
        expect: Expect::Done,
    },
    Case {
        name: "a_close_sent_to_the_window_is_reported_and_the_window_stays_open",
        child: a_close_sent_to_the_window_is_reported_and_the_window_stays_open,
        expect: Expect::Done,
    },
    Case {
        name: "run_waits_for_the_last_window_and_warns",
        child: run_waits_for_the_last_window_and_warns,
        expect: Expect::Done,
    },
    Case {
        name: "a_driver_that_panics_still_lets_the_pump_return",
        child: a_driver_that_panics_still_lets_the_pump_return,
        expect: Expect::FailsAfter {
            stage: "the pump returned",
            says: "the driver panics, on purpose",
        },
    },
    Case {
        name: "a_last_device_whose_drop_blocks_holds_its_own_thread_alone",
        child: a_last_device_whose_drop_blocks_holds_its_own_thread_alone,
        expect: Expect::Done,
    },
    #[cfg(target_os = "macos")]
    Case {
        name: "after_an_error_from_run_main_returns_within_its_bound",
        child: after_an_error_from_run_main_returns_within_its_bound,
        expect: Expect::Done,
    },
    Case {
        name: "a_worker_that_waits_on_the_main_thread_aborts",
        child: a_worker_that_waits_on_the_main_thread_aborts,
        expect: Expect::Aborts { says: "block_on called on a worker thread stalls the frame" },
    },
    Case {
        name: "a_present_call_that_blocks_holds_only_the_calls_behind_it",
        child: a_present_call_that_blocks_holds_only_the_calls_behind_it,
        expect: Expect::Done,
    },
];

fn main() -> ExitCode {
    match std::env::var(CHILD) {
        Ok(name) => child(&name),
        Err(_) => harness(),
    }
}

// ---- The parent: the test harness's arguments, and each child under its deadline ----

/// Lists the cases, or runs those the arguments select, as `cargo nextest` and `cargo test`
/// call a test binary: `--list [--ignored]`, and `[--exact] [FILTER…] [--ignored |
/// --include-ignored]`. Options it has no use for are taken and ignored.
fn harness() -> ExitCode {
    const TAKE_A_VALUE: &[&str] =
        &["--format", "--test-threads", "--skip", "--color", "--logfile", "-Z", "--shuffle-seed"];
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        if TAKE_A_VALUE.contains(&arg) {
            if arg == "--skip" {
                skips.extend(args.get(at + 1).cloned());
            }
            at += 2;
            continue;
        }
        if !arg.starts_with('-') {
            filters.push(arg.to_owned());
        }
        at += 1;
    }
    let flag = |name: &str| args.iter().any(|arg| arg == name);
    let exact = flag("--exact");
    let selected = |name: &str| {
        let matches =
            |filter: &String| if exact { name == filter } else { name.contains(filter.as_str()) };
        (filters.is_empty() || filters.iter().any(matches)) && !skips.iter().any(matches)
    };
    let cases = CASES.iter().filter(|case| selected(case.name));
    if flag("--list") {
        // Every case is ignored, so `--ignored` lists them all as well.
        for case in cases {
            println!("{}: test", case.name);
        }
        return ExitCode::SUCCESS;
    }
    let run = flag("--ignored") || flag("--include-ignored");
    let (mut passed, mut failed, mut ignored) = (0, 0, 0);
    for case in cases {
        if !run {
            println!("test {} ... ignored, opens a window", case.name);
            ignored += 1;
            continue;
        }
        println!("test {} ...", case.name);
        if std::panic::catch_unwind(|| parent(case)).is_ok() {
            println!("test {} ... ok", case.name);
            passed += 1;
        } else {
            println!("test {} ... FAILED", case.name);
            failed += 1;
        }
    }
    let result = if failed == 0 { "ok" } else { "FAILED" };
    println!("\ntest result: {result}. {passed} passed; {failed} failed; {ignored} ignored");
    if failed == 0 { ExitCode::SUCCESS } else { ExitCode::from(101) }
}

/// Runs `case`'s child under its deadline. It reads the child's output while it waits, so a
/// child that writes more than a pipe holds is not blocked into a false hang; past the
/// deadline it kills the child and fails, naming the last stage the child printed. It
/// takes the output within a bound too, since a process the child started may keep the
/// pipe, and then checks the child's status and its last stage against `case.expect`.
fn parent(case: &Case) {
    const DEADLINE: Duration = Duration::from_secs(60);
    let mut child = Command::new(std::env::current_exe().expect("this binary's path"))
        .env(CHILD, case.name)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the subprocess starts");
    let read = |mut pipe: Box<dyn Read + Send>| {
        let (read, reader) = mpsc::channel();
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = pipe.read_to_end(&mut output);
            let _ = read.send(String::from_utf8_lossy(&output).into_owned());
        });
        reader
    };
    let stdout = read(Box::new(child.stdout.take().expect("piped")));
    let stderr = read(Box::new(child.stderr.take().expect("piped")));
    let until = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().expect("the subprocess is waited for") {
            break Some(status);
        }
        if Instant::now() >= until {
            child.kill().expect("the subprocess is killed");
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = stdout.recv_timeout(BOUND).expect("stdout held open");
    let stderr = stderr.recv_timeout(BOUND).expect("stderr held open");
    print!("{stdout}");
    eprint!("{stderr}");
    let stage = stdout.lines().rev().find_map(|line| line.strip_prefix("stage: "));
    let Some(status) = status else {
        panic!(
            "the subprocess passed its deadline of {DEADLINE:?} after the stage {:?}",
            stage.unwrap_or("none")
        )
    };
    match case.expect {
        Expect::Done => {
            assert!(status.success(), "the subprocess failed with {status} after {stage:?}");
            assert_eq!(stage, Some(FINAL_STAGE), "the subprocess ended early");
        }
        Expect::FailsAfter { stage: expected, says } => {
            assert!(!status.success(), "the subprocess succeeded");
            assert_eq!(stage, Some(expected));
            assert!(stderr.contains(says), "stderr does not say {says:?}");
        }
        Expect::Aborts { says } => {
            assert!(!status.success(), "the subprocess survived");
            assert!(stderr.contains(says), "stderr does not say {says:?}");
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                assert_eq!(status.signal(), Some(6), "expected SIGABRT, got {status:?}");
            }
        }
    }
}

// ---- The child: what each case shares ----

/// Runs the case named on the process's main thread and exits with its result.
fn child(name: &str) -> ExitCode {
    tracing::subscriber::set_global_default(Warnings).expect("the process's one subscriber");
    let case = CASES.iter().find(|case| case.name == name).expect("a case of this binary");
    match (case.child)() {
        Ok(()) => {
            println!("stage: {FINAL_STAGE}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("the case failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The warnings on `pail` so far.
static WARNINGS: AtomicUsize = AtomicUsize::new(0);

fn warnings() -> usize {
    WARNINGS.load(Ordering::Relaxed)
}

/// The child's subscriber: it counts the warnings on `pail` and prints the events it takes
/// on stderr, where the parent shows them.
struct Warnings;

impl tracing::Subscriber for Warnings {
    /// Everything on `pail`, and what any other target says at `INFO` and above: winit's
    /// traces would bury the case's output.
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == LOG_TARGET || *metadata.level() <= tracing::Level::INFO
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn Debug) {
                self.0.push_str(&format!(" {}={value:?}", field.name()));
            }
        }
        let metadata = event.metadata();
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        eprintln!("{} {}:{}", metadata.level(), metadata.target(), fields.0);
        if metadata.target() == LOG_TARGET && *metadata.level() == tracing::Level::WARN {
            WARNINGS.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

fn desc() -> WindowDesc {
    WindowDesc { title: "PAIL".to_owned(), size: PixelSize { width: 640, height: 480 } }
}

fn spec(name: &str, class: ThreadClass) -> ThreadSpec {
    ThreadSpec { name: name.to_owned(), class, index_in_class: 0 }
}

fn failed(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Cancelled, message.into())
}

/// Polls `future` on the calling thread until it resolves, and panics past [`BOUND`]. The
/// driver is no worker, so it may wait.
fn wait<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    let until = Instant::now() + BOUND;
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        let now = Instant::now();
        assert!(now < until, "a wait passed its bound of {BOUND:?}");
        std::thread::park_timeout(until - now);
    }
}

/// Polls `done` every 10 ms until it holds, and panics past `bound`: for what wakes no one,
/// as a resize and a minimise do not.
fn poll_until(bound: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + bound;
    while !done() {
        assert!(Instant::now() < until, "{what}: not within {bound:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Queues `exit` when it is dropped, a panic's unwinding included, and does not wait for
/// it: the first step of every shutdown (`README.md`, "Shutdown").
struct ExitOnDrop(Lane<MainThread>);

impl Drop for ExitOnDrop {
    fn drop(&mut self) {
        drop(self.0.run(MainThread::exit));
    }
}

/// A thread that stands where a client's tasks do, holding the `exit` guard from its
/// start. `ThreadHandle::join` discards a panic, so the result comes back on a channel as
/// the thread's last act.
struct Driver {
    thread: ThreadHandle,
    outcome: mpsc::Receiver<Result<()>>,
}

fn drive(
    main: &Lane<MainThread>,
    body: impl FnOnce(Lane<MainThread>) -> Result<()> + Send + 'static,
) -> Result<Driver> {
    let main = main.clone();
    let (result, outcome) = mpsc::channel();
    let thread = StdPlatform.spawn_thread(
        spec("driver", ThreadClass::Background),
        Box::new(move || {
            let exit = ExitOnDrop(main.clone());
            let checked = body(main);
            drop(exit);
            let _ = result.send(checked);
        }),
    )?;
    Ok(Driver { thread, outcome })
}

impl Driver {
    fn finish(self) -> Result<()> {
        self.thread.join();
        self.outcome.try_recv().unwrap_or_else(|_| Err(failed("the driver ended unanswered")))
    }
}

/// Pumps on this thread while `body` drives on another, and returns the pump's error, or
/// else the driver's.
fn pump_with(
    main: &Lane<MainThread>,
    pump: EventPump,
    body: impl FnOnce(Lane<MainThread>) -> Result<()> + Send + 'static,
) -> Result<()> {
    let driver = drive(main, body)?;
    println!("stage: pumping");
    let pumped = pump.run();
    println!("stage: the pump returned");
    let checked = driver.finish();
    pumped?;
    checked
}

// ---- The cases ----

/// A window opens, events are pumped, one call runs on the main thread and one on the
/// present thread, which `ClientPlatform` put in the present class, and the process closes.
fn opens_pumps_and_closes() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    let (present, thread) = present_thread(&ClientPlatform::new(), 0u32)?;
    pump_with(&main, pump, move |main| {
        let size = wait(main.run(|main| main.window().size()))?;
        assert!(!size.is_empty(), "the window opened with no area: {size:?}");
        println!("stage: a main-lane call ran, the window's size {size:?}");
        let (name, class) = wait(present.run(|count| {
            *count += 1;
            (std::thread::current().name().map(str::to_owned), os::in_the_present_class())
        }))?;
        assert_eq!(name.as_deref(), Some("present"));
        assert_ne!(class, Some(false), "the present thread is not in its class");
        println!("stage: a present-thread call ran, in the present class: {class:?}");
        assert_eq!(wait(thread.stop())?, 1);
        assert_eq!(warnings(), 0, "a warning on pail");
        println!("stage: stopped");
        Ok(())
    })
}

/// A call queued before `run` runs after the window opens; once `run` has returned, a task
/// whose call came back `Cancelled` reads the close, and a call queued later is
/// `Cancelled`.
fn a_call_queued_before_run_runs_and_one_after_is_cancelled() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    let before = main.run(|main| {
        let window = main.window();
        (window.size(), window.reports())
    });
    pump_with(&main, pump, move |main| {
        let (size, reports) = wait(before)?;
        assert!(!size.is_empty(), "the call ran before the window opened");
        assert!(!reports.close_requested());
        println!("stage: the call queued before run ran");
        wait(main.run(MainThread::exit))?;
        let executor = Executor::builder().build()?;
        let task = executor.spawn(WorkerClass::Background, async move {
            loop {
                if let Err(error) = main.run(|_| ()).await {
                    assert_eq!(error.kind(), ErrorKind::Cancelled);
                    return reports.close_requested();
                }
            }
        });
        assert!(executor.block_on(task)?, "a task whose call was cancelled read no close");
        executor.shutdown();
        println!("stage: a call came back cancelled, and the close read true");
        Ok(())
    })?;
    let after = wait(main.run(|_| ())).expect_err("nothing runs after run returned");
    assert_eq!(after.kind(), ErrorKind::Cancelled);
    println!("stage: a call queued after run returned is cancelled");
    Ok(())
}

fn a_dropped_pump_cancels_its_calls_and_a_second_pump_is_unsupported() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    let queued = main.run(|main| main.window().size());
    drop(pump);
    assert_eq!(wait(queued).expect_err("never run").kind(), ErrorKind::Cancelled);
    assert_eq!(wait(main.run(|_| ())).expect_err("never run").kind(), ErrorKind::Cancelled);
    println!("stage: a dropped pump cancelled its calls");
    let second = event_pump(desc()).err().expect("a second pump is refused");
    assert_eq!(second.kind(), ErrorKind::Unsupported);
    println!("stage: a second event_pump is unsupported");
    Ok(())
}

/// The handles inside a main-lane call, the same at every call; `Unavailable` on the
/// driver and on the present thread.
fn the_handles_are_lent_on_the_main_thread_alone() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    let (present, thread) = present_thread(&ClientPlatform::new(), ())?;
    pump_with(&main, pump, move |main| {
        let lend = |main: &mut MainThread| {
            let window = main.window();
            let handles = (window.window_handle(), window.display_handle());
            let lent =
                format!("{:?}", (handles.0.map(|h| h.as_raw()), handles.1.map(|h| h.as_raw())));
            (window, lent)
        };
        let (window, first) = wait(main.run(lend))?;
        let (again, second) = wait(main.run(lend))?;
        assert!(first.starts_with("(Ok(") && first.contains("), Ok("), "not lent: {first}");
        assert_eq!(first, second, "the handles changed between calls");
        assert!(Arc::ptr_eq(&window, &again), "two windows");
        drop(again);
        println!("stage: the handles were lent on the main thread: {first}");
        assert!(matches!(window.window_handle(), Err(HandleError::Unavailable)));
        assert!(matches!(window.display_handle(), Err(HandleError::Unavailable)));
        let on_present = window.clone();
        let unavailable = wait(present.run(move |()| {
            let unavailable = matches!(on_present.window_handle(), Err(HandleError::Unavailable))
                && matches!(on_present.display_handle(), Err(HandleError::Unavailable));
            drop(on_present);
            unavailable
        }))?;
        assert!(unavailable, "a handle was lent on the present thread");
        println!("stage: no handle was lent on the driver or the present thread");
        drop(window);
        wait(thread.stop())?;
        Ok(())
    })
}

/// `request_size` is answered by `Window::size` and by a `WindowReports`; a minimise by
/// `is_minimised`, and on Windows by an empty size.
fn a_requested_size_and_a_minimise_are_reported() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    pump_with(&main, pump, move |main| {
        let (reports, before) = wait(main.run(|main| {
            let window = main.window();
            (window.reports(), window.size())
        }))?;
        // Halves of even sizes, which a display scale of 1 or 2 gives back whole.
        let asked = PixelSize { width: before.width / 4 * 2, height: before.height / 4 * 2 };
        wait(main.run(move |main| main.request_size(asked)))?;
        poll_until(ANSWER, "the asked size reported", || reports.size() == asked);
        assert_eq!(wait(main.run(|main| main.window().size()))?, asked);
        println!("stage: a requested size was reported: {before:?} to {asked:?}");

        wait(main.run(|main| main.set_minimised(true)))?;
        poll_until(ANSWER, "a minimise reported", || {
            wait(main.run(|main| main.is_minimised())).ok().flatten() == Some(true)
        });
        if cfg!(windows) {
            poll_until(ANSWER, "an empty size while minimised", || reports.size().is_empty());
        }
        println!("stage: a minimise was reported, at {:?}", reports.size());
        wait(main.run(|main| main.set_minimised(false)))?;
        poll_until(ANSWER, "a restore reported", || {
            wait(main.run(|main| main.is_minimised())).ok().flatten() == Some(false)
        });
        poll_until(ANSWER, "the size back", || !reports.size().is_empty());
        println!("stage: a restore was reported, at {:?}", reports.size());
        Ok(())
    })
}

/// A close the test sends the window as its close button does turns the report on, and
/// the window stays open and keeps answering.
fn a_close_sent_to_the_window_is_reported_and_the_window_stays_open() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    pump_with(&main, pump, move |main| {
        let reports = wait(main.run(|main| main.window().reports()))?;
        assert!(!reports.close_requested());
        wait(main.run(|main| os::send_close(&main.window())))?;
        poll_until(ANSWER, "the close reported", || reports.close_requested());
        println!("stage: the close was reported");
        // A later message to the window is delivered after the close.
        let shown = wait(main.run(|main| {
            let window = main.window();
            os::is_shown(&window) && window.window_handle().is_ok() && window.close_requested()
        }))?;
        assert!(shown, "the window closed on the request");
        println!("stage: the window stayed open");
        Ok(())
    })
}

/// `run` does not return after `exit` while an `Arc<Window>` is held: it has not returned a
/// second later and has warned on `pail`, it keeps answering, and it returns within a
/// bound of the last drop, made on another thread. A `WindowReports` held past `exit` does
/// not keep it.
fn run_waits_for_the_last_window_and_warns() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    let returned = Arc::new(AtomicBool::new(false));
    let (dropped, dropped_at) = mpsc::channel();
    let driver = drive(&main, {
        let returned = returned.clone();
        move |main| {
            let (window, reports) = wait(main.run(|main| {
                let window = main.window();
                let reports = window.reports();
                (window, reports)
            }))?;
            wait(main.run(MainThread::exit))?;
            let exited = Instant::now();
            poll_until(BOUND, "a warning on pail", || warnings() >= 1);
            wait(main.run(|_| ()))?;
            assert!(!returned.load(Ordering::SeqCst), "run returned with the window held");
            println!("stage: run had not returned {:?} after exit, and warned", exited.elapsed());
            let _ = dropped.send(Instant::now());
            drop(window);
            poll_until(BOUND, "run's return", || returned.load(Ordering::SeqCst));
            assert!(reports.close_requested());
            Ok(())
        }
    })?;
    println!("stage: pumping");
    let pumped = pump.run();
    let back = Instant::now();
    returned.store(true, Ordering::SeqCst);
    println!("stage: the pump returned");
    let dropped = dropped_at.recv_timeout(BOUND).map_err(|_| failed("the window was not dropped"));
    let checked = driver.finish();
    pumped?;
    checked?;
    let after = back.saturating_duration_since(dropped?);
    println!("stage: run returned {after:?} after the last drop");
    assert!(after < Duration::from_secs(2), "run returned {after:?} after the last drop");
    Ok(())
}

fn a_driver_that_panics_still_lets_the_pump_return() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    pump_with(&main, pump, |main| {
        wait(main.run(|main| main.window().size()))?;
        println!("stage: the driver panics");
        panic!("the driver panics, on purpose");
    })
}

/// Stands for the last `Device` of step 4 whose bound expired: it holds the window as its
/// surface, and its drop blocks until the test releases it.
struct Device {
    window: Arc<Window>,
    release: mpsc::Receiver<()>,
}

impl Drop for Device {
    fn drop(&mut self) {
        self.release.recv_timeout(BOUND).expect("the test releases the drop");
        let _ = self.window.size();
    }
}

/// A last device whose drop blocks is dropped on step 4's thread, a thread of its own: the
/// pump warns, a main-lane call queued meanwhile runs, and a worker's task completes.
fn a_last_device_whose_drop_blocks_holds_its_own_thread_alone() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    pump_with(&main, pump, move |main| {
        let executor = Executor::builder().platform(Arc::new(ClientPlatform::new())).build()?;
        let (release, released) = mpsc::channel();
        let device =
            wait(main.run(move |main| Device { window: main.window(), release: released }))?;
        wait(main.run(MainThread::exit))?;
        let settle = executor.platform().spawn_thread(
            spec("settle", ThreadClass::Background),
            Box::new(move || drop(device)),
        )?;
        poll_until(BOUND, "a warning on pail", || warnings() >= 1);
        wait(main.run(|_| ()))?;
        let task = executor.spawn(WorkerClass::FrameCritical, async { 3 });
        assert_eq!(executor.block_on(task)?, 3);
        println!("stage: while the drop blocked, the pump warned, a call ran, a task completed");
        release.send(()).map_err(|_| failed("the drop ended unreleased"))?;
        settle.join();
        executor.shutdown();
        Ok(())
    })
}

/// After an error from `run`, with a root whose last step does not end, `main` returns
/// `run`'s error within its bound, having logged on `pail`, and the root is not dropped on
/// the main thread: `lactose-app-client`'s `main` as `tests/consumers.rs` sketches it. The
/// error is the event loop ended by the application's own stop, which only macOS's loop
/// takes; on Windows winit's loop ends through its `exit` alone.
#[cfg(target_os = "macos")]
fn after_an_error_from_run_main_returns_within_its_bound() -> Result<()> {
    use std::mem::ManuallyDrop;
    use std::sync::Mutex;

    const ROOT_BOUND: Duration = Duration::from_secs(2);
    struct DropsOn(Arc<Mutex<Option<String>>>);
    impl Drop for DropsOn {
        fn drop(&mut self) {
            let name = std::thread::current().name().unwrap_or("unnamed").to_owned();
            *self.0.lock().expect("not poisoned") = Some(name);
        }
    }

    let (main, pump) = event_pump(desc())?;
    let executor = Executor::builder().platform(Arc::new(ClientPlatform::new())).build()?;
    let dropped_on = Arc::new(Mutex::new(None));
    let (_release, released) = mpsc::channel::<()>();
    let root = executor.spawn(WorkerClass::FrameCritical, {
        let (executor, dropped_on) = (executor.clone(), dropped_on.clone());
        async move {
            let guard = DropsOn(dropped_on);
            let exit = ExitOnDrop(main.clone());
            let window = main.run(|main| main.window()).await?;
            main.run(|_| os::end_event_loop()).await?;
            // The last step, which nothing releases within the bound.
            while matches!(released.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                executor.sleep_until(Instant::now() + Duration::from_millis(10)).await?;
            }
            drop((window, exit, guard));
            Ok::<_, Error>(())
        }
    });
    let error = pump.run().expect_err("the event loop ended with the window held");
    assert_eq!(error.kind(), ErrorKind::Platform);
    println!("stage: run returned an error: {error}");
    let started = Instant::now();
    let (answer, answered) = mpsc::channel();
    let waiter = executor.clone();
    let root = ManuallyDrop::new(root);
    let waited = executor.platform().spawn_thread(
        spec("root", ThreadClass::Background),
        Box::new(move || {
            let root = ManuallyDrop::into_inner(root);
            let _ = answer.send(waiter.block_on(root));
        }),
    );
    match waited.map(|_| answered.recv_timeout(ROOT_BOUND)) {
        Ok(Ok(_)) => return Err(failed("the root ended, which nothing let it do")),
        _ => tracing::warn!(target: LOG_TARGET, "the root did not end within its bound"),
    }
    let took = started.elapsed();
    assert!(took < ROOT_BOUND + Duration::from_secs(1), "main took {took:?}");
    assert!(warnings() >= 1, "main did not log on pail");
    assert_eq!(*dropped_on.lock().expect("not poisoned"), None, "the root was dropped");
    println!("stage: main returned run's error after {took:?}, logged, and left the root");
    // The process ends with the root and the window, as a client's does.
    println!("stage: {FINAL_STAGE}");
    std::process::exit(0);
}

/// `Executor::block_on` of a main-lane call on a worker is `lactose-task`'s debug
/// assertion, and the process aborts.
fn a_worker_that_waits_on_the_main_thread_aborts() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    pump_with(&main, pump, |main| {
        let executor = Executor::builder().build()?;
        let on_worker = executor.clone();
        println!("stage: a worker waits on the main thread");
        let task = executor.spawn(WorkerClass::FrameCritical, async move {
            on_worker.block_on(main.run(|main| main.window().size()))
        });
        let _ = wait(task);
        Err(failed("the process did not abort"))
    })
}

/// A present call that blocks holds the calls behind it and no other thread: while it is in
/// progress a main-lane call and a worker's task complete, and the call behind it has not
/// run.
fn a_present_call_that_blocks_holds_only_the_calls_behind_it() -> Result<()> {
    let (main, pump) = event_pump(desc())?;
    let (present, thread) = present_thread(&ClientPlatform::new(), Vec::new())?;
    pump_with(&main, pump, move |main| {
        let executor = Executor::builder().build()?;
        let entered = Arc::new(Barrier::new(2));
        let (release, released) = mpsc::channel::<()>();
        let blocking = present.run({
            let entered = entered.clone();
            move |order: &mut Vec<&str>| {
                order.push("blocking");
                entered.wait();
                released.recv_timeout(BOUND).expect("the test releases the call");
            }
        });
        let behind_ran = Arc::new(AtomicBool::new(false));
        let behind = present.run({
            let behind_ran = behind_ran.clone();
            move |order: &mut Vec<&str>| {
                behind_ran.store(true, Ordering::Relaxed);
                order.push("behind");
            }
        });
        entered.wait();
        let size = wait(main.run(|main| main.window().size()))?;
        let task = executor.spawn(WorkerClass::FrameCritical, async { 7 });
        assert_eq!(executor.block_on(task)?, 7);
        assert!(!behind_ran.load(Ordering::Relaxed), "the call behind ran");
        println!("stage: a main-lane call ({size:?}) and a task completed while a call blocked");
        release.send(()).map_err(|_| failed("the call ended unreleased"))?;
        wait(blocking)?;
        wait(behind)?;
        assert_eq!(wait(thread.stop())?, ["blocking", "behind"]);
        executor.shutdown();
        Ok(())
    })
}

/// What a user does to a window, and a thread's scheduling, through the operating system.
#[allow(unsafe_code, reason = "the one module that calls the operating system (`SAFETY.md`)")]
mod os {
    #[cfg(target_os = "macos")]
    pub use self::macos::*;
    #[cfg(windows)]
    pub use self::windows::*;

    #[cfg(target_os = "macos")]
    mod macos {
        use lactose_platform::Window;
        use objc2::rc::Retained;
        use objc2_app_kit::{
            NSApplication, NSEvent, NSEventModifierFlags, NSEventSubtype, NSEventType, NSView,
            NSWindow,
        };
        use objc2_foundation::{MainThreadMarker, NSPoint};
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        fn ns_window(window: &Window) -> Retained<NSWindow> {
            let handle = window.window_handle().expect("on the main thread").as_raw();
            let RawWindowHandle::AppKit(handle) = handle else { panic!("not AppKit: {handle:?}") };
            // SAFETY: winit's handle is its content view, which lives while the window
            // does, and the window is borrowed for this call; a view is an `NSView`.
            let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
            view.window().expect("the view is in its window")
        }

        /// What the close button sends: `performClose:`, which asks the delegate, and so
        /// winit, whether to close. On the main thread.
        pub fn send_close(window: &Window) {
            let window = ns_window(window);
            // SAFETY: `performClose:` takes any sender or none, on the main thread, which a
            // main-lane call is on.
            unsafe { window.performClose(None) };
        }

        pub fn is_shown(window: &Window) -> bool {
            ns_window(window).isVisible()
        }

        /// Whether the calling thread has the quality of service `ThreadClass::Present`
        /// asks for.
        pub fn in_the_present_class() -> Option<bool> {
            let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
            let mut relative = 0;
            // SAFETY: both pointers are to live locals, and the thread is the calling one.
            let status = unsafe {
                libc::pthread_get_qos_class_np(
                    libc::pthread_self(),
                    &raw mut class,
                    &raw mut relative,
                )
            };
            Some(status == 0 && matches!(class, libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE))
        }

        /// Stops the application's run loop as an application's own stop does, which ends
        /// winit's loop without its `exit`. On the main thread.
        pub fn end_event_loop() {
            let app = NSApplication::sharedApplication(MainThreadMarker::new().expect("main"));
            app.stop(None);
            // SAFETY: an application-defined event with no window and no context, which is
            // what winit posts to make its own stop take effect.
            let event = unsafe {
                NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                    NSEventType::ApplicationDefined,
                    NSPoint::new(0.0, 0.0),
                    NSEventModifierFlags(0),
                    0.0,
                    0,
                    None,
                    NSEventSubtype::WindowExposed.0,
                    0,
                    0,
                )
            };
            app.postEvent_atStart(&event.expect("an event"), true);
        }
    }

    #[cfg(windows)]
    mod windows {
        use lactose_platform::Window;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsWindowVisible, PostMessageW, WM_CLOSE,
        };

        fn hwnd(window: &Window) -> HWND {
            let handle = window.window_handle().expect("on the main thread").as_raw();
            let RawWindowHandle::Win32(handle) = handle else { panic!("not Win32: {handle:?}") };
            handle.hwnd.get() as HWND
        }

        /// What the close button sends: `WM_CLOSE`, which winit reports and does not act on.
        pub fn send_close(window: &Window) {
            // SAFETY: the handle is the window's, which the borrow keeps alive, and
            // `WM_CLOSE` carries no parameters.
            let posted = unsafe { PostMessageW(hwnd(window), WM_CLOSE, 0, 0) };
            assert_ne!(posted, 0, "WM_CLOSE was not posted");
        }

        pub fn is_shown(window: &Window) -> bool {
            // SAFETY: the handle is the window's, which the borrow keeps alive.
            unsafe { IsWindowVisible(hwnd(window)) != 0 }
        }

        /// Whether the calling thread has the priority `ThreadClass::Present` asks for.
        pub fn in_the_present_class() -> Option<bool> {
            // SAFETY: the pseudo-handle of the calling thread, which is always valid.
            Some(unsafe { GetThreadPriority(GetCurrentThread()) } == THREAD_PRIORITY_ABOVE_NORMAL)
        }
    }

    /// No case runs where winit is not run: these keep the binary building there.
    #[cfg(not(any(target_os = "macos", windows)))]
    mod elsewhere {
        use lactose_platform::Window;

        pub fn send_close(_: &Window) {
            unimplemented!("no window test runs on this operating system")
        }

        pub fn is_shown(_: &Window) -> bool {
            unimplemented!("no window test runs on this operating system")
        }

        pub fn in_the_present_class() -> Option<bool> {
            None
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    pub use self::elsewhere::*;
}
