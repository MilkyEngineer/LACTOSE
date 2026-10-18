//! A machine-wide bound on concurrent heavy commands, for `cargo xtask heavy`.
//!
//! Worktrees on one machine each build in their own target directory, so cargo's locks never
//! make one wait for another. A slot is an exclusive lock on `slot-<i>.lock` in a directory
//! every caller names the same way. The operating system releases the lock when the holding
//! process exits, however it exits, so a killed holder never leaves a stale slot. Beside it,
//! `slot-<i>.owner` holds the token of the slot's latest holder: `<pid>-<nanoseconds>`.
//!
//! Lock files are never truncated or deleted: deleting a held one would let a second holder in
//! on a new file of the same name. There is no queue; when a slot frees, whichever caller
//! tries first takes it, which favours a script that runs wrapped commands back to back over
//! a waiter that polls.
//!
//! The directory, the file names, [`DEFAULT_SLOTS`] and the form of [`HELD_VAR`] are a
//! protocol between every worktree on the machine, at whatever commit each one is.

use std::ffi::OsStr;
use std::fs::{self, File, FileTimes, OpenOptions, TryLockError};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The slot count every caller uses unless told otherwise. The bound holds only while every
/// caller on the machine passes the same count.
pub const DEFAULT_SLOTS: NonZeroUsize = NonZeroUsize::new(2).unwrap();

/// How long `cargo xtask heavy` waits between attempts while every slot is held. Short, so a
/// waiter has a fair chance at the gap between two commands of a script; an attempt costs one
/// open and one lock call per slot.
pub const RETRY: Duration = Duration::from_millis(200);

/// The variable `cargo xtask heavy` sets in the environment of the program it runs, to
/// [`Slot::held_value`]. A `heavy` that finds it naming a slot still held for it (see
/// [`is_held`]) runs its program without taking a slot: its caller already holds one, and
/// waiting for a second deadlocks once every slot is held by a caller doing the same.
pub const HELD_VAR: &str = "LACTOSE_HEAVY_SLOT";

/// The directory of the machine-wide slots: `lactose-heavy` in the temporary directory.
#[must_use]
pub fn dir() -> PathBuf {
    std::env::temp_dir().join("lactose-heavy")
}

fn lock_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("slot-{index}.lock"))
}

/// The token cannot live in the lock file: a Windows lock is mandatory, so no other handle
/// could read it.
fn owner_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("slot-{index}.owner"))
}

/// One slot, held until it is dropped or the process ends. A child process never holds it:
/// std opens files close-on-exec on Unix and not inheritable on Windows.
#[derive(Debug)]
pub struct Slot {
    index: usize,
    token: String,
    owner: PathBuf,
    _lock: File,
}

impl Drop for Slot {
    /// Clears the owner file before the lock goes, so a value left to a program that outlives
    /// this `heavy` never matches while the next holder is between its lock and its write. A
    /// holder killed by a signal runs no drop and leaves its token for that instant.
    fn drop(&mut self) {
        let _ = fs::write(&self.owner, "");
    }
}

impl Slot {
    /// Which slot this is, from 0.
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    /// The value of [`HELD_VAR`] for the program run in this slot: `<index>:<token>`, where
    /// the token is what this holder wrote to `slot-<index>.owner`.
    #[must_use]
    pub fn held_value(&self) -> String {
        format!("{}:{}", self.index, self.token)
    }
}

/// Takes the first free slot of `slots` in `dir`, creating the directory and its files as
/// needed, or returns `None` if every slot is held.
///
/// Each attempt opens its own handle, so two attempts in one process conflict as two
/// processes do.
pub fn try_acquire(dir: &Path, slots: NonZeroUsize) -> Result<Option<Slot>, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for index in 0..slots.get() {
        let path = lock_path(dir, index);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => continue,
            Err(TryLockError::Error(e)) => return Err(format!("{}: {e}", path.display())),
        }
        // macOS deletes files in the temporary directory untouched for three days, held or
        // not, and deleting a held lock file lets a second holder in. Taking a lock changes no
        // time, so the times are refreshed here.
        let now = SystemTime::now();
        lock.set_times(FileTimes::new().set_accessed(now).set_modified(now))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let nanos = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let token = format!("{}-{nanos}", std::process::id());
        let owner = owner_path(dir, index);
        fs::write(&owner, format!("{token}\n")).map_err(|e| format!("{}: {e}", owner.display()))?;
        return Ok(Some(Slot { index, token, owner, _lock: lock }));
    }
    Ok(None)
}

/// Takes the first free slot of `slots` in `dir`, trying every `retry` while all are held.
/// Calls `waiting` once, after the first attempt finds every slot held.
pub fn acquire(
    dir: &Path,
    slots: NonZeroUsize,
    retry: Duration,
    waiting: impl FnOnce(),
) -> Result<Slot, String> {
    let mut waiting = Some(waiting);
    loop {
        if let Some(slot) = try_acquire(dir, slots)? {
            return Ok(slot);
        }
        if let Some(waiting) = waiting.take() {
            waiting();
        }
        thread::sleep(retry);
    }
}

/// Whether `value`, found in [`HELD_VAR`], names a slot of `dir` that is held now by the
/// `heavy` that set it: the slot's lock is taken and its owner file holds the token. A value
/// left to a program whose `heavy` died fails once the slot is free or another holder has it.
///
/// A free slot is locked for the moment of the check.
#[must_use]
pub fn is_held(dir: &Path, value: &OsStr) -> bool {
    let Some((index, token)) = value.to_str().and_then(|v| v.split_once(':')) else {
        return false;
    };
    let Ok(index) = index.parse::<usize>() else {
        return false;
    };
    let Ok(lock) = OpenOptions::new().read(true).write(true).open(lock_path(dir, index)) else {
        return false;
    };
    if !matches!(lock.try_lock(), Err(TryLockError::WouldBlock)) {
        return false;
    }
    let owner = fs::read_to_string(owner_path(dir, index));
    !token.is_empty() && owner.is_ok_and(|owner| owner.trim_end() == token)
}

/// The process IDs that the owner files of the first `slots` slots of `dir` name, in slot
/// order. A holder clears its file when it lets the slot go, but one killed by a signal
/// leaves it, so an ID may be of a process that has gone.
#[must_use]
pub fn holders(dir: &Path, slots: NonZeroUsize) -> Vec<u32> {
    (0..slots.get())
        .filter_map(|index| fs::read_to_string(owner_path(dir, index)).ok())
        .filter_map(|owner| owner.split('-').next()?.parse().ok())
        .collect()
}

/// The exit status that passes on a finished program's: its code, or on Unix 128 plus the
/// signal that killed it, as a shell reports it. A status with neither, which no supported
/// platform produces, is 2.
#[must_use]
pub fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    2
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    /// A scratch directory whose `lactose-heavy` child does not exist yet, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("lactose-heavy-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            Self(root)
        }

        fn slots(&self) -> PathBuf {
            self.0.join("lactose-heavy")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const ONE: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const TWO: NonZeroUsize = NonZeroUsize::new(2).unwrap();

    #[test]
    fn one_slot_serialises_two_holders() {
        let scratch = Scratch::new("serialise");
        let dir = scratch.slots();
        let first = try_acquire(&dir, ONE).unwrap().expect("the slot is free");
        assert_eq!(first.index(), 0);

        let (events, received) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let waiter = {
            let dir = dir.clone();
            thread::spawn(move || {
                let waiting = events.clone();
                let slot = acquire(&dir, ONE, Duration::from_millis(10), move || {
                    waiting.send("waiting").unwrap();
                })
                .unwrap();
                events.send("acquired").unwrap();
                released.recv().unwrap();
                drop(slot);
            })
        };
        assert_eq!(received.recv().unwrap(), "waiting");
        // The waiter found the slot held and cannot take it while `first` lives.
        assert!(received.try_recv().is_err());
        drop(first);
        assert_eq!(received.recv().unwrap(), "acquired");
        assert!(try_acquire(&dir, ONE).unwrap().is_none(), "the waiter holds the slot");
        release.send(()).unwrap();
        waiter.join().unwrap();
        assert!(try_acquire(&dir, ONE).unwrap().is_some());
    }

    #[test]
    fn two_slots_admit_two_and_block_the_third() {
        let scratch = Scratch::new("two");
        let dir = scratch.slots();
        let first = try_acquire(&dir, TWO).unwrap().expect("slot 0 is free");
        let second = try_acquire(&dir, TWO).unwrap().expect("slot 1 is free");
        assert_eq!((first.index(), second.index()), (0, 1));
        assert!(try_acquire(&dir, TWO).unwrap().is_none());
    }

    #[test]
    fn a_dropped_holder_frees_its_slot() {
        let scratch = Scratch::new("drop");
        let dir = scratch.slots();
        let first = try_acquire(&dir, TWO).unwrap().unwrap();
        let second = try_acquire(&dir, TWO).unwrap().unwrap();
        drop(first);
        let third = try_acquire(&dir, TWO).unwrap().expect("slot 0 was freed");
        assert_eq!((third.index(), second.index()), (0, 1));
        assert!(try_acquire(&dir, TWO).unwrap().is_none());
    }

    #[test]
    fn acquire_without_contention_never_waits() {
        let scratch = Scratch::new("free");
        let slot = acquire(&scratch.slots(), ONE, Duration::from_secs(3600), || {
            panic!("no slot was held")
        });
        assert_eq!(slot.unwrap().index(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_child_process_does_not_keep_the_slot() {
        let scratch = Scratch::new("child");
        let dir = scratch.slots();
        let slot = try_acquire(&dir, ONE).unwrap().unwrap();
        let mut child = std::process::Command::new("sh")
            .args(["-c", "echo running; exec sleep 60"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // `spawn` can return before the kernel has closed every close-on-exec descriptor in
        // the child; a line the shell writes proves its exec is complete.
        let mut line = String::new();
        let stdout = child.stdout.take().unwrap();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(stdout), &mut line).unwrap();
        assert_eq!(line, "running\n");
        drop(slot);
        let free = try_acquire(&dir, ONE).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(free.is_some(), "the child inherited the lock");
    }

    #[test]
    fn the_lock_files_outlive_their_holders() {
        let scratch = Scratch::new("files");
        let dir = scratch.slots();
        drop(try_acquire(&dir, TWO).unwrap().unwrap());
        assert!(dir.join("slot-0.lock").is_file());
    }

    #[test]
    fn taking_a_slot_refreshes_the_lock_file_s_times_and_writes_the_owner() {
        let scratch = Scratch::new("times");
        let dir = scratch.slots();
        fs::create_dir_all(&dir).unwrap();
        let old = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let lock = File::create(dir.join("slot-0.lock")).unwrap();
        lock.set_times(FileTimes::new().set_accessed(old).set_modified(old)).unwrap();
        drop(lock);

        let slot = try_acquire(&dir, ONE).unwrap().unwrap();
        let metadata = fs::metadata(dir.join("slot-0.lock")).unwrap();
        assert!(metadata.modified().unwrap() > old);
        assert!(metadata.accessed().unwrap() > old);
        let owner = fs::read_to_string(dir.join("slot-0.owner")).unwrap();
        assert_eq!(format!("0:{}", owner.trim_end()), slot.held_value());
        assert_eq!(holders(&dir, ONE), [std::process::id()]);
    }

    #[test]
    fn a_held_value_holds_only_while_its_slot_is_held_by_it() {
        let scratch = Scratch::new("held");
        let dir = scratch.slots();
        let slot = try_acquire(&dir, TWO).unwrap().unwrap();
        let value = slot.held_value();
        assert!(is_held(&dir, OsStr::new(&value)));
        let token = value.split_once(':').unwrap().1;

        for forged in ["", "0", "0:", "nonsense", "1:x", "x:y", &format!("1:{token}")] {
            assert!(!is_held(&dir, OsStr::new(forged)), "{forged:?}");
        }
        assert!(!is_held(&dir, OsStr::new(&format!("0:{token}x"))));
        // Slot 2 does not exist under two slots, and checking it creates nothing.
        assert!(!is_held(&dir, OsStr::new(&format!("2:{token}"))));
        assert!(!dir.join("slot-2.lock").exists());

        // Stale: the slot is free, then another holder has it.
        drop(slot);
        assert!(!is_held(&dir, OsStr::new(&value)));
        let other = try_acquire(&dir, TWO).unwrap().unwrap();
        assert_eq!(other.index(), 0, "the check left the slot free");
        assert!(!is_held(&dir, OsStr::new(&value)));
        assert!(is_held(&dir, OsStr::new(&other.held_value())));
    }

    #[test]
    fn a_slot_let_go_clears_its_token_before_the_next_holder_writes_one() {
        let scratch = Scratch::new("clear");
        let dir = scratch.slots();
        let slot = try_acquire(&dir, ONE).unwrap().unwrap();
        let value = slot.held_value();
        drop(slot);
        assert_eq!(fs::read_to_string(dir.join("slot-0.owner")).unwrap(), "");
        assert!(holders(&dir, ONE).is_empty());
        // A new holder between its lock and its token write.
        let lock = OpenOptions::new().read(true).write(true).open(dir.join("slot-0.lock")).unwrap();
        lock.try_lock().unwrap();
        assert!(!is_held(&dir, OsStr::new(&value)));
    }
}
