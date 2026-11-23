//! The crash hook's abort path (README, "Rules a caller should know"; D155): the binary run
//! in a subprocess with its test hook set panics on a frame worker, writes the report to
//! stderr and to a file in the directory it names, and aborts; a directory that cannot be
//! written costs the file alone. No test installs the hook (D16).

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

const BINARY: &str = env!("CARGO_BIN_EXE_lactose-app-headless");

/// A directory of the temporary directory's own for `test`, empty.
fn scratch(test: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("lactose-app-headless-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The binary run for one tick in `cwd`, its test hook set or not, the crash directory
/// named or not; its output and its process ID.
fn run(cwd: &Path, panic: bool, crash_dir: Option<&Path>) -> (Output, u32) {
    let mut command = Command::new(BINARY);
    command.arg("1").current_dir(cwd).env_remove("LACTOSE_TEST_PANIC");
    command.env_remove("LACTOSE_CRASH_DIR");
    if panic {
        command.env("LACTOSE_TEST_PANIC", "1");
    }
    if let Some(dir) = crash_dir {
        command.env("LACTOSE_CRASH_DIR", dir);
    }
    let child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let id = child.id();
    (child.wait_with_output().unwrap(), id)
}

/// The exit of `std::process::abort`: SIGABRT on unix, and on Windows the fast-fail status
/// Rust's abort raises, `STATUS_STACK_BUFFER_OVERRUN`.
fn assert_aborted(status: ExitStatus, stderr: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        assert_eq!(status.signal(), Some(6), "{status:?}\n{stderr}");
    }
    #[cfg(windows)]
    assert_eq!(status.code(), Some(0xC000_0409_u32 as i32), "{status:?}\n{stderr}");
}

/// The report on stderr, from its first line to the line after it: the thread, the panic's
/// location and message, and a backtrace with frames.
fn report_of(stderr: &str) -> &str {
    let start = stderr.find("lactose-app-headless crashed\n").expect(stderr);
    let report = &stderr[start..];
    let end = report.find("\nlactose-app-headless: ").map_or(report.len(), |end| end + 1);
    let report = &report[..end];
    let mut lines = report.lines();
    assert_eq!(lines.next(), Some("lactose-app-headless crashed"));
    let thread = lines.next().unwrap();
    assert!(thread.starts_with("thread: whisk-frame-"), "{report}");
    assert!(lines.next().unwrap().starts_with("panicked at "), "{report}");
    assert_eq!(lines.next(), Some("LACTOSE_TEST_PANIC is set: the first frame panics"));
    assert_eq!(lines.next(), Some("backtrace:"), "{report}");
    let frames = lines.filter(|line| line.trim_start().split_once(": ").is_some()).count();
    assert!(frames > 0, "{report}");
    // The panic is the timer root's, which a frame names. Frames are named where the build
    // has symbols; a Windows build's are not asserted.
    #[cfg(unix)]
    assert!(report.contains("timer_root"), "{report}");
    report
}

/// The files in `dir`.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> =
        fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect();
    files.sort();
    files
}

#[test]
fn a_panic_writes_the_report_to_stderr_then_a_file_and_aborts() {
    let cwd = scratch("named");
    let dir = cwd.join("reports").join("crash");
    let (output, id) = run(&cwd, true, Some(&dir));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_aborted(output.status, &stderr);
    let report = report_of(&stderr);
    let files = files_in(&dir);
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("lactose-app-headless-"), "{name}");
    assert!(name.ends_with(&format!("Z-{id}.txt")), "{name}");
    // The file is the stderr report's text, and the line after it names the file.
    assert_eq!(fs::read_to_string(&files[0]).unwrap(), report);
    let named = format!("lactose-app-headless: the crash report is in {}", files[0].display());
    assert!(stderr.contains(&named), "{stderr}");
    assert!(output.stdout.is_empty(), "{}", String::from_utf8_lossy(&output.stdout));
    fs::remove_dir_all(&cwd).unwrap();
}

#[test]
fn the_report_file_is_in_crash_of_the_working_directory_by_default() {
    let cwd = scratch("default");
    let (output, _) = run(&cwd, true, None);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_aborted(output.status, &stderr);
    let report = report_of(&stderr);
    let files = files_in(&cwd.join("crash"));
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(fs::read_to_string(&files[0]).unwrap(), report);
    fs::remove_dir_all(&cwd).unwrap();
}

#[test]
fn a_directory_that_cannot_be_made_costs_the_file_alone() {
    let cwd = scratch("blocked");
    // A regular file where a parent directory would be: no user can create the directory.
    let file = cwd.join("a file");
    fs::write(&file, "").unwrap();
    let dir = file.join("crash");
    let (output, _) = run(&cwd, true, Some(&dir));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_aborted(output.status, &stderr);
    report_of(&stderr);
    let refused = format!("lactose-app-headless: no crash report file at {}: ", dir.display());
    assert!(stderr.contains(&refused), "{stderr}");
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
        let (output, _) = run(&cwd, true, Some(&dir));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_aborted(output.status, &stderr);
        report_of(&stderr);
        let refused = "lactose-app-headless: no crash report file at ";
        assert!(stderr.contains(refused), "{stderr}");
        assert!(files_in(&dir).is_empty());
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_dir_all(&cwd).unwrap();
}

/// The test hook unset, the run is the binary's normal one: it reports and exits 0, and a
/// run that does not panic creates no crash directory.
#[test]
fn without_the_test_hook_the_run_is_unchanged() {
    let cwd = scratch("normal");
    let (output, _) = run(&cwd, false, None);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{:?}\n{stderr}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("1 ticks in "), "{stdout}");
    assert!(!stderr.contains("crashed"), "{stderr}");
    assert!(files_in(&cwd).is_empty());
    fs::remove_dir_all(&cwd).unwrap();
}
