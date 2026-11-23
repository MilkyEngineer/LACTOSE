//! The crash hook (architecture section 5.1, D155): the panic hook every engine binary
//! installs first in `main`. A panic writes a text report to stderr and then to a file, and
//! aborts.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Held from the first panic's report to the abort, so that a panic on another thread
/// waits for the abort and the report is of the process's first panic. It is never
/// released, so it is never poisoned. While it is held the hook takes `std`'s backtrace
/// lock alone, inside `std`'s capture and symbol lookup, where no user code runs; never
/// `std`'s stderr lock ([`write_stderr`]).
static REPORTING: Mutex<()> = Mutex::new(());

/// Installs the process's panic hook (`README.md`, "The crash hook"). A panic then writes to
/// stderr `binary` and "crashed" on its first line, the thread, and the panic's location
/// and message; then a forced backtrace; then, when `dir` is `Some`, the whole report to a
/// new file in `dir`, created with its parents if missing, named
/// `<binary>-<UTC time>-<process id>.txt`; then one line on stderr naming the file, or
/// saying why there is none; then calls `std::process::abort`, in every profile.
///
/// `None` writes stderr alone. A file that cannot be created or written costs the file and
/// nothing else: a file whose write failed is removed, the hook does not panic, and it
/// aborts either way. The hook handles panics only: a hardware fault or a signal is the
/// operating system's to report (D155).
///
/// Called by a binary's `main` before anything else, once. A test never calls it (D16): a
/// test that expects an abort runs a binary in a subprocess.
///
/// # Panics
///
/// If `binary` is not one non-empty path component, since it names the report's file: a
/// separator, `..` or an absolute path would put the file outside `dir`.
pub fn install_crash_hook(binary: &'static str, dir: Option<PathBuf>) {
    assert!(is_file_name(binary), "the crash hook's binary name {binary:?} is not a file name");
    // Made while descriptors are free, for a crash whose own duplicate fails: a panic is
    // often a full descriptor table's.
    let reserve = duplicate_stderr().ok();
    std::panic::set_hook(Box::new(move |info| {
        let _first = REPORTING.lock();
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        // Stderr first, and the panic before its backtrace, whose symbols are read from the
        // executable's files: neither a symbol lookup nor a file system that hangs or fails
        // can cost the thread and the message. Nothing is left to report a failed stderr
        // write to.
        let mut report = format!(
            "{binary} crashed\nthread: {}\n{info}\n",
            thread.name().unwrap_or("<unnamed>"),
        );
        let _ = write_stderr(&report, reserve.as_ref());
        let start = report.len();
        let _ = write!(report, "backtrace:\n{backtrace}\n");
        let _ = write_stderr(&report[start..], reserve.as_ref());
        if let Some(dir) = &dir {
            let line = match write_file(binary, dir, &report) {
                Ok(path) => format!("{binary}: the crash report is in {}\n", path.display()),
                Err((path, error)) => {
                    format!("{binary}: no crash report file at {}: {error}\n", path.display())
                }
            };
            let _ = write_stderr(&line, reserve.as_ref());
        }
        std::process::abort();
    }));
}

/// Whether `binary` is one path component that names a file: no separator, no root or
/// prefix, not `.` or `..`, not empty.
fn is_file_name(binary: &str) -> bool {
    !binary.is_empty() && Path::new(binary).file_name() == Some(OsStr::new(binary))
}

/// Writes `text` to stderr through a duplicate of its handle made now, or else through
/// `reserve`, the one made at install; never through `std`'s stderr, whose lock a thread
/// that holds it inside `eprintln!`, and then panics, or waits there on a lock the panicking
/// thread holds, would hold for ever. With neither handle, nothing is written. The
/// duplicate made now comes first so that a stderr redirected since install is followed.
fn write_stderr(text: &str, reserve: Option<&File>) -> std::io::Result<()> {
    match (duplicate_stderr(), reserve) {
        (Ok(mut stderr), _) => stderr.write_all(text.as_bytes()),
        (Err(_), Some(mut reserve)) => reserve.write_all(text.as_bytes()),
        (Err(error), None) => Err(error),
    }
}

/// A file over a new handle to the process's stderr; `std`'s `Stderr` is unbuffered, so
/// nothing written through it is still in `std`'s hands. On Windows a process with no
/// console has a null stderr, which `std` copies without duplicating: a write to it fails,
/// and nothing is written, as through `std`'s stderr.
fn duplicate_stderr() -> std::io::Result<File> {
    #[cfg(unix)]
    let handle = std::os::fd::AsFd::as_fd(&std::io::stderr()).try_clone_to_owned()?;
    #[cfg(windows)]
    let handle =
        std::os::windows::io::AsHandle::as_handle(&std::io::stderr()).try_clone_to_owned()?;
    Ok(File::from(handle))
}

/// Writes `report` to a file of its own in `dir`. Gives the file's path, or the path that
/// failed with its error.
fn write_file(
    binary: &str,
    dir: &Path,
    report: &str,
) -> Result<PathBuf, (PathBuf, std::io::Error)> {
    fs::create_dir_all(dir).map_err(|error| (dir.to_path_buf(), error))?;
    // Before the epoch is a clock that is wrong, and the name still has the process ID.
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let path = dir.join(file_name(binary, now, std::process::id()));
    match write_new(&path, report) {
        Ok(()) => Ok(path),
        Err(error) => Err((path, error)),
    }
}

/// The report's file name. Two processes that crash into one directory differ by the
/// process ID or the time.
fn file_name(binary: &str, since_epoch: Duration, process: u32) -> String {
    format!("{binary}-{}-{process}.txt", utc(since_epoch))
}

/// Creates `path` and writes `text` to it, and refuses a path that exists, so that no
/// report replaces another.
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    write_or_remove(file, path, text)
}

/// Writes `text` to `file`, the file at `path`; when the write fails (a full disk), closes
/// and removes it, so that no file a failure line names is left empty or cut short.
fn write_or_remove(mut file: File, path: &Path, text: &str) -> std::io::Result<()> {
    let written = file.write_all(text.as_bytes());
    if written.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    written
}

/// `since_epoch` as UTC, `YYYYMMDDTHHMMSS.mmmZ`: sorted by name, the files are in time
/// order. The date is Howard Hinnant's `civil_from_days`, for days on or after the epoch.
fn utc(since_epoch: Duration) -> String {
    let seconds = since_epoch.as_secs();
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    let z = days + 719_468;
    let era = z / 146_097;
    let of_era = z % 146_097;
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    let mut text = String::with_capacity(20);
    let _ = write!(
        text,
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}.{:03}Z",
        of_day / 3_600,
        of_day / 60 % 60,
        of_day % 60,
        since_epoch.subsec_millis(),
    );
    text
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    #[test]
    fn the_time_in_a_name_is_utc() {
        let at = |seconds: u64, millis: u64| utc(Duration::from_millis(seconds * 1_000 + millis));
        assert_eq!(at(0, 0), "19700101T000000.000Z");
        // A leap day, and the last instant of a day.
        assert_eq!(at(951_868_799, 999), "20000229T235959.999Z");
        // 2100 is not a leap year.
        assert_eq!(at(4_107_542_400, 7), "21000301T000000.007Z");
        let at = Duration::from_millis(1_795_452_540_120);
        assert_eq!(file_name("binary", at, 42), "binary-20261123T164900.120Z-42.txt");
    }

    #[test]
    fn a_binary_name_is_one_file_name() {
        for name in ["lactose-app-headless", "ricotta", "a.b", "..x"] {
            assert!(is_file_name(name), "{name}");
        }
        for name in ["", ".", "..", "../escaped", "a/b", "a/", "/tmp/abs", "./a"] {
            assert!(!is_file_name(name), "{name}");
        }
        #[cfg(windows)]
        for name in ["a\\b", "C:", "C:a", "\\\\server\\share"] {
            assert!(!is_file_name(name), "{name}");
        }
    }

    /// A directory of the temporary directory's own, for one test in one process.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lactose-app-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_report_file_is_new_in_a_directory_created_for_it() {
        let dir = scratch("crash-new");
        let nested = dir.join("crash");
        let path = write_file("binary", &nested, "the report\n").unwrap();
        assert_eq!(path.parent(), Some(nested.as_path()));
        assert_eq!(fs::read_to_string(&path).unwrap(), "the report\n");
        let error = write_new(&path, "another report\n").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "the report\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_directory_that_cannot_be_made_is_the_path_that_failed() {
        let dir = scratch("crash-blocked");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a file");
        fs::write(&file, "").unwrap();
        let (failed, _) = write_file("binary", &file.join("crash"), "the report\n").unwrap_err();
        assert_eq!(failed, file.join("crash"));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A write that fails after the file was created leaves no file. The failure is a handle
    /// opened for reading, which every system refuses a write through, standing in for a
    /// full disk.
    #[test]
    fn a_file_whose_write_failed_is_removed() {
        let dir = scratch("crash-unwritten");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.txt");
        fs::write(&path, "").unwrap();
        let read_only = File::open(&path).unwrap();
        assert!(write_or_remove(read_only, &path, "the report\n").is_err());
        assert!(!path.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
