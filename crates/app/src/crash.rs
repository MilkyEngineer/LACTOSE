//! The crash hook (architecture section 5.1, D155): the panic hook every engine binary
//! installs first in `main`. A panic writes a text report to stderr and then to a file, and
//! aborts.

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Held from the first panic's report to the abort, so that a panic on another thread
/// waits for the abort and the report is of the process's first panic. It is never
/// released, so it is never poisoned.
static REPORTING: Mutex<()> = Mutex::new(());

/// Installs the process's panic hook (`README.md`, "The crash hook"). A panic then writes a
/// text report, `binary` and "crashed" on its first line, the thread, the panic's location
/// and message, and a forced backtrace, to stderr; then, when `dir` is `Some`, the same
/// text to a new file in `dir`, created with its parents if missing, named
/// `<binary>-<UTC time>-<process id>.txt`; then one line on stderr naming the file, or
/// saying why it was not written; then calls `std::process::abort`, in every profile.
///
/// `binary` is a file name's prefix, so it holds no path separator. `None` writes stderr
/// alone. A file that cannot be created or written costs the file and nothing else: the
/// hook does not panic, and it aborts either way. The hook handles panics only: a hardware
/// fault or a signal is the operating system's to report (D155).
///
/// Called by a binary's `main` before anything else, once. A test never calls it (D16): a
/// test that expects an abort runs a binary in a subprocess.
pub fn install_crash_hook(binary: &'static str, dir: Option<PathBuf>) {
    std::panic::set_hook(Box::new(move |info| {
        let _first = REPORTING.lock();
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let report = format!(
            "{binary} crashed\nthread: {}\n{info}\nbacktrace:\n{backtrace}\n",
            thread.name().unwrap_or("<unnamed>"),
        );
        // Stderr first, so that a file system that hangs or fails cannot cost it.
        // Nothing is left to report a failed stderr write to.
        let _ = write_stderr(&report);
        if let Some(dir) = &dir {
            let line = match write_file(binary, dir, &report) {
                Ok(path) => format!("{binary}: the crash report is in {}\n", path.display()),
                Err((path, error)) => {
                    format!("{binary}: no crash report file at {}: {error}\n", path.display())
                }
            };
            let _ = write_stderr(&line);
        }
        std::process::abort();
    }));
}

fn write_stderr(text: &str) -> std::io::Result<()> {
    let mut stderr = std::io::stderr().lock();
    stderr.write_all(text.as_bytes())?;
    stderr.flush()
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
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
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
}
