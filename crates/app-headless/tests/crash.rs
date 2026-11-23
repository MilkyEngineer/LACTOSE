//! The crash hook's abort path (README, "Rules a caller should know"; D155): the binary run
//! in a subprocess with its test hook set panics on a frame worker, writes the report to
//! stderr and to a file in the directory it names, and aborts; a directory that cannot be
//! written costs the file alone; another thread that holds stderr's lock, or waits inside
//! `eprintln!`, does not hold the report back, even with the descriptor table full. No test
//! installs the hook (D16).

#![forbid(unsafe_code)]

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_lactose-app-headless");

/// How long a run may take before it counts as hung and is killed: far above the second a
/// one-tick run takes on a loaded machine, and inside nextest's own timeout.
const BOUND: Duration = Duration::from_secs(30);

/// A directory of the temporary directory's own for `test`, empty.
fn scratch(test: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("lactose-app-headless-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// What a run gave.
struct Run {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    id: u32,
}

/// The binary run for one tick in `cwd`, with `LACTOSE_TEST_PANIC` set to `hook` or unset,
/// and the crash directory named or not. A run that has not exited within [`BOUND`] is
/// killed, and the test fails: a hook that hangs is the failure.
fn run(cwd: &Path, hook: Option<&str>, crash_dir: Option<&Path>) -> Run {
    run_command(Command::new(BINARY), cwd, hook, crash_dir)
}

/// [`run`], with the process's descriptor limit lowered to `descriptors` by the shell that
/// starts it, so that filling the table is quick on a machine whose limit is high.
#[cfg(unix)]
fn run_with_descriptors(
    descriptors: u32,
    cwd: &Path,
    hook: Option<&str>,
    crash_dir: Option<&Path>,
) -> Run {
    let mut command = Command::new("/bin/sh");
    // `exec`, so that the process the test waits on and kills is the binary.
    command.arg("-c").arg(format!("ulimit -n {descriptors} && exec \"$0\" \"$@\"")).arg(BINARY);
    run_command(command, cwd, hook, crash_dir)
}

/// `command`, the binary or the shell that execs it, run as [`run`] says.
fn run_command(
    mut command: Command,
    cwd: &Path,
    hook: Option<&str>,
    crash_dir: Option<&Path>,
) -> Run {
    command.arg("1").current_dir(cwd);
    command.env_remove("LACTOSE_TEST_PANIC").env_remove("LACTOSE_CRASH_DIR");
    if let Some(hook) = hook {
        command.env("LACTOSE_TEST_PANIC", hook);
    }
    if let Some(dir) = crash_dir {
        command.env("LACTOSE_CRASH_DIR", dir);
    }
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let id = child.id();
    // Read on threads of their own, so that a full pipe cannot stall the child.
    let read = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let stdout = read(Box::new(child.stdout.take().unwrap()));
    let stderr = read(Box::new(child.stderr.take().unwrap()));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if start.elapsed() > BOUND {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
    let Some(status) = status else {
        panic!("the run did not exit within {BOUND:?} and was killed\n{stderr}");
    };
    Run { status, stdout, stderr, id }
}

/// The exit of `std::process::abort`: SIGABRT on unix, and on Windows the fast-fail status
/// Rust's abort raises, `STATUS_STACK_BUFFER_OVERRUN`.
fn assert_aborted(run: &Run) {
    let (status, stderr) = (run.status, &run.stderr);
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        assert_eq!(status.signal(), Some(6), "{status:?}\n{stderr}");
    }
    #[cfg(windows)]
    assert_eq!(status.code(), Some(0xC000_0409_u32 as i32), "{status:?}\n{stderr}");
}

/// The report on stderr, from its first line to the line after it, with its thread and its
/// message: the thread, the panic's location and message, and a backtrace with frames.
fn report_in(stderr: &str) -> (&str, &str, &str) {
    let start = stderr.find("lactose-app-headless crashed\n").expect(stderr);
    let report = &stderr[start..];
    let end = report.find("\nlactose-app-headless: ").map_or(report.len(), |end| end + 1);
    let report = &report[..end];
    let mut lines = report.lines();
    assert_eq!(lines.next(), Some("lactose-app-headless crashed"));
    let thread = lines.next().unwrap().strip_prefix("thread: ").expect(report);
    assert!(lines.next().unwrap().starts_with("panicked at "), "{report}");
    let message = lines.next().unwrap();
    assert_eq!(lines.next(), Some("backtrace:"), "{report}");
    let frames = lines.filter(|line| line.trim_start().split_once(": ").is_some()).count();
    assert!(frames > 0, "{report}");
    (report, thread, message)
}

/// The report of the first frame's panic, with `message`.
fn frame_report<'a>(stderr: &'a str, message: &str) -> &'a str {
    let (report, thread, said) = report_in(stderr);
    assert!(thread.starts_with("whisk-frame-"), "{report}");
    assert_eq!(said, message, "{report}");
    // The panic is the timer root's, which a frame names. Frames are named where the build
    // has symbols; a Windows build's are not asserted.
    #[cfg(unix)]
    assert!(report.contains("timer_root"), "{report}");
    report
}

const PLAIN: &str = "LACTOSE_TEST_PANIC is set: the first frame panics";

/// The files in `dir`.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> =
        fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect();
    files.sort();
    files
}

/// The one file in `dir`, which holds `report`.
fn assert_one_file(dir: &Path, report: &str) -> PathBuf {
    let files = files_in(dir);
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(fs::read_to_string(&files[0]).unwrap(), report);
    files[0].clone()
}

#[test]
fn a_panic_writes_the_report_to_stderr_then_a_file_and_aborts() {
    let cwd = scratch("named");
    let dir = cwd.join("reports").join("crash");
    let run = run(&cwd, Some("1"), Some(&dir));
    assert_aborted(&run);
    let report = frame_report(&run.stderr, PLAIN);
    // The file is the stderr report's text, and the line after it names the file.
    let file = assert_one_file(&dir, report);
    let name = file.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("lactose-app-headless-"), "{name}");
    assert!(name.ends_with(&format!("Z-{}.txt", run.id)), "{name}");
    let named = format!("lactose-app-headless: the crash report is in {}", file.display());
    assert!(run.stderr.contains(&named), "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    fs::remove_dir_all(&cwd).unwrap();
}

#[test]
fn the_report_file_is_in_crash_of_the_working_directory_by_default() {
    let cwd = scratch("default");
    let run = run(&cwd, Some("1"), None);
    assert_aborted(&run);
    let report = frame_report(&run.stderr, PLAIN);
    assert_one_file(&cwd.join("crash"), report);
    fs::remove_dir_all(&cwd).unwrap();
}

#[test]
fn a_directory_that_cannot_be_made_costs_the_file_alone() {
    let cwd = scratch("blocked");
    // A regular file where a parent directory would be: no user can create the directory.
    let file = cwd.join("a file");
    fs::write(&file, "").unwrap();
    let dir = file.join("crash");
    let run = run(&cwd, Some("1"), Some(&dir));
    assert_aborted(&run);
    frame_report(&run.stderr, PLAIN);
    let refused = format!("lactose-app-headless: no crash report file at {}: ", dir.display());
    assert!(run.stderr.contains(&refused), "{}", run.stderr);
    assert_eq!(files_in(&cwd), vec![file]);
    fs::remove_dir_all(&cwd).unwrap();
}

/// A directory the process may not write into: on unix, one without write permission. A
/// process that may write it anyway (root's) has nothing to show, and the case is skipped.
#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_written_costs_the_file_alone() {
    use std::os::unix::fs::PermissionsExt as _;

    let cwd = scratch("read-only");
    let dir = cwd.join("crash");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    let writable = fs::write(dir.join("probe"), "").is_ok();
    if !writable {
        let run = run(&cwd, Some("1"), Some(&dir));
        assert_aborted(&run);
        frame_report(&run.stderr, PLAIN);
        let refused = "lactose-app-headless: no crash report file at ";
        assert!(run.stderr.contains(refused), "{}", run.stderr);
        assert!(files_in(&dir).is_empty());
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_dir_all(&cwd).unwrap();
}

/// The frame panics while another thread is inside `eprintln!`, holding stderr's lock, and
/// panics there a second later. A hook that took that lock would wait for the thread, whose
/// own hook waits for the first: the run would hang. The report is the first panic's, or the
/// second's if the frame's hook started after it, and the process aborts.
#[test]
fn a_panic_while_another_thread_holds_stderr_is_reported_and_aborts() {
    let cwd = scratch("stderr-held");
    let dir = cwd.join("crash");
    let run = run(&cwd, Some("stderr-held"), Some(&dir));
    assert_aborted(&run);
    let (report, thread, message) = report_in(&run.stderr);
    let first = "LACTOSE_TEST_PANIC=stderr-held: the first frame panics while another thread \
                 holds stderr";
    let reported = (thread.starts_with("whisk-frame-") && message == first)
        || (thread == "stderr-holder" && message == "the second panic, inside eprintln!");
    assert!(reported, "{report}");
    assert_one_file(&dir, report);
    fs::remove_dir_all(&cwd).unwrap();
}

/// The frame panics holding a lock that another thread waits on inside `eprintln!`, so that
/// thread holds stderr's lock until the abort. A hook that took that lock would never write.
#[test]
fn a_panic_while_stderr_waits_on_the_panicking_thread_is_reported_and_aborts() {
    let cwd = scratch("stderr-blocked");
    let dir = cwd.join("crash");
    let run = run(&cwd, Some("stderr-blocked"), Some(&dir));
    assert_aborted(&run);
    let message = "LACTOSE_TEST_PANIC=stderr-blocked: the first frame panics holding what \
                   stderr waits on";
    let report = frame_report(&run.stderr, message);
    assert_one_file(&dir, report);
    fs::remove_dir_all(&cwd).unwrap();
}

/// As above, with the descriptor table full when the frame panics: the hook cannot duplicate
/// stderr at the panic, so it writes through the duplicate it made at install, and a hook
/// that fell back to `std`'s stderr would never write. The file cannot be created, and the
/// line says so.
#[cfg(unix)]
#[test]
fn a_panic_with_the_descriptor_table_full_is_reported_on_stderr_and_aborts() {
    let cwd = scratch("stderr-blocked-emfile");
    let dir = cwd.join("crash");
    let run = run_with_descriptors(256, &cwd, Some("stderr-blocked-emfile"), Some(&dir));
    assert_aborted(&run);
    // Not `frame_report`: with no descriptor free, the backtrace's symbols may not be read.
    let (report, thread, message) = report_in(&run.stderr);
    assert!(thread.starts_with("whisk-frame-"), "{report}");
    let expected = "LACTOSE_TEST_PANIC=stderr-blocked-emfile: the first frame panics holding \
                    what stderr waits on";
    assert_eq!(message, expected, "{report}");
    let refused = format!("lactose-app-headless: no crash report file at {}", dir.display());
    assert!(run.stderr.contains(&refused), "{}", run.stderr);
    assert!(run.stderr.contains("(os error 24)\n"), "{}", run.stderr);
    assert!(files_in(&dir).is_empty());
    fs::remove_dir_all(&cwd).unwrap();
}

/// The test hook unset, or set to the empty value, the run is the binary's normal one: it
/// reports and exits 0, and a run that does not panic creates no crash directory.
#[test]
fn without_the_test_hook_the_run_is_unchanged() {
    for hook in [None, Some("")] {
        let cwd = scratch("normal");
        let run = run(&cwd, hook, None);
        assert!(run.status.success(), "{hook:?}: {:?}\n{}", run.status, run.stderr);
        assert!(run.stdout.starts_with("1 ticks in "), "{}", run.stdout);
        assert!(!run.stderr.contains("crashed"), "{}", run.stderr);
        assert!(files_in(&cwd).is_empty());
        fs::remove_dir_all(&cwd).unwrap();
    }
}
