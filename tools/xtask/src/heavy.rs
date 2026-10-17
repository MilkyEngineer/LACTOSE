//! A machine-wide bound on concurrent heavy commands, for `cargo xtask heavy`.
//!
//! Worktrees on one machine each build in their own target directory, so cargo's locks never
//! make one wait for another. A slot is an exclusive lock on `slot-<i>.lock` in a directory
//! every caller names the same way. The operating system releases the lock when the holding
//! process exits, however it exits, so a killed holder never leaves a stale slot.
//!
//! Lock files are never truncated or deleted: deleting a held one would let a second holder in
//! on a new file of the same name. There is no queue; when a slot frees, whichever waiter
//! polls first takes it.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::thread;
use std::time::Duration;

/// The slot count every caller uses unless told otherwise. The bound holds only while every
/// caller on the machine passes the same count.
pub const DEFAULT_SLOTS: NonZeroUsize = NonZeroUsize::new(2).unwrap();

/// How long `cargo xtask heavy` waits between attempts while every slot is held.
pub const RETRY: Duration = Duration::from_secs(2);

/// The variable `cargo xtask heavy` sets, to the slot's index, in the environment of the
/// program it runs. A `heavy` that finds it set runs its program without taking a slot: its
/// caller already holds one, and waiting for a second deadlocks once every slot is held by a
/// caller doing the same.
pub const HELD_VAR: &str = "LACTOSE_HEAVY_SLOT";

/// The directory of the machine-wide slots: `lactose-heavy` in the temporary directory.
#[must_use]
pub fn dir() -> PathBuf {
    std::env::temp_dir().join("lactose-heavy")
}

/// One slot, held until it is dropped or the process ends. A child process never holds it:
/// std opens files close-on-exec on Unix and not inheritable on Windows.
#[derive(Debug)]
pub struct Slot {
    index: usize,
    _lock: File,
}

impl Slot {
    /// Which slot this is, from 0.
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }
}

/// Takes the first free slot of `slots` in `dir`, creating the directory and its lock files
/// as needed, or returns `None` if every slot is held.
///
/// Each attempt opens its own handle, so two attempts in one process conflict as two
/// processes do.
pub fn try_acquire(dir: &Path, slots: NonZeroUsize) -> Result<Option<Slot>, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for index in 0..slots.get() {
        let path = dir.join(format!("slot-{index}.lock"));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        match lock.try_lock() {
            Ok(()) => return Ok(Some(Slot { index, _lock: lock })),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => return Err(format!("{}: {e}", path.display())),
        }
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
}
