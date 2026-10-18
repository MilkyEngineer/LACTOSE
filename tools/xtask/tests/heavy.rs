//! `heavy` through the binary: the exit status, the streams and the command line.
//!
//! Each run gets its own temporary directory, so these tests never wait for, or hold, the
//! machine's real slots.

#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use lactose_xtask::heavy::{self, HELD_VAR};

/// A temporary directory for one test's runs, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("lactose-heavy-bin-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    /// `cargo xtask heavy` with `args`, with this directory as its temporary directory.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lactose-xtask"));
        command
            .arg("heavy")
            .args(args)
            // `std::env::temp_dir` reads `TMPDIR` on Unix and `TMP`, then `TEMP`, on Windows.
            .env("TMPDIR", &self.0)
            .env("TMP", &self.0)
            .env("TEMP", &self.0)
            // Set when these tests themselves run under `heavy`.
            .env_remove(HELD_VAR);
        command
    }

    /// Runs `cargo xtask heavy` with `args`.
    fn heavy(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("the xtask binary runs")
    }

    /// The slot directory the binary uses.
    fn slots(&self) -> PathBuf {
        self.0.join("lactose-heavy")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A shell command line for the platform's shell.
fn shell(line: &str) -> Vec<&str> {
    if cfg!(windows) { vec!["cmd", "/C", line] } else { vec!["sh", "-c", line] }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_program_s_exit_status_is_heavy_s() {
    let scratch = Scratch::new("status");
    for (code, slots) in [(3, None), (0, Some("1"))] {
        let mut args = slots.map_or_else(Vec::new, |n| vec!["--slots", n]);
        args.push("--");
        let line = format!("exit {code}");
        args.extend(shell(&line));
        let output = scratch.heavy(&args);
        assert_eq!(output.status.code(), Some(code), "{}", stderr(&output));
        assert!(output.stdout.is_empty());
        assert!(stderr(&output).starts_with("heavy: slot 0 after "), "{}", stderr(&output));
    }
    assert!(scratch.0.join("lactose-heavy").join("slot-0.lock").is_file());
}

#[test]
fn the_program_s_streams_are_its_own() {
    let scratch = Scratch::new("streams");
    let mut args = vec!["--"];
    args.extend(shell("echo out"));
    let output = scratch.heavy(&args);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "out");
}

#[cfg(unix)]
#[test]
fn a_program_killed_by_a_signal_exits_128_plus_the_signal() {
    let scratch = Scratch::new("signal");
    let output = scratch.heavy(&["--", "sh", "-c", "kill -9 $$"]);
    assert_eq!(output.status.code(), Some(128 + 9), "{}", stderr(&output));
}

#[test]
fn a_nested_heavy_runs_inside_its_caller_s_slot() {
    let scratch = Scratch::new("nested");
    let mut args = vec!["--slots", "1", "--", env!("CARGO_BIN_EXE_lactose-xtask")];
    args.extend(["heavy", "--slots", "1", "--"]);
    args.extend(shell("exit 4"));
    let output = scratch.heavy(&args);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let stderr = stderr(&output);
    let lines: Vec<&str> = stderr.lines().collect();
    assert!(lines[0].starts_with("heavy: slot 0 after "), "{lines:?}");
    assert!(lines[1].starts_with("heavy: inside slot 0:"), "{lines:?}");
    assert!(lines[1].ends_with(" already"), "{lines:?}");
}

const ONE: NonZeroUsize = NonZeroUsize::new(1).unwrap();

/// Runs `heavy --slots 1` with `HELD_VAR` set to `value` while this process holds the one
/// slot, and requires it to wait like any caller until the slot is let go.
fn waits_for_the_slot(scratch: &Scratch, value: &str) {
    let held = heavy::try_acquire(&scratch.slots(), ONE).unwrap().expect("the slot is free");
    let mut args = vec!["--slots", "1", "--"];
    args.extend(shell("exit 0"));
    let mut child = scratch
        .command(&args)
        .env(HELD_VAR, value)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut before = Vec::new();
    loop {
        let mut line = String::new();
        // End of file here means the program ran without waiting.
        assert_ne!(stderr.read_line(&mut line).unwrap(), 0, "{value:?}: {before:?}");
        let waiting = line.starts_with("heavy: waiting for one of 1 slots");
        before.push(line);
        if waiting {
            break;
        }
    }
    assert_eq!(
        before[0].trim_end(),
        format!("heavy: {HELD_VAR}={value} names no slot held for it"),
        "{before:?}"
    );
    let pid = std::process::id().to_string();
    assert!(before[1].trim_end().ends_with(&format!("held by pid {pid}")), "{before:?}");
    drop(held);
    let mut rest = String::new();
    stderr.read_to_string(&mut rest).unwrap();
    assert!(child.wait().unwrap().success(), "{rest}");
    assert!(rest.starts_with("heavy: slot 0 after "), "{rest}");
}

#[test]
fn the_waiting_line_names_no_pid_when_no_owner_file_can_be_read() {
    let scratch = Scratch::new("no-owner");
    fs::create_dir_all(scratch.slots()).unwrap();
    // A holder that has locked the slot and not yet written its token.
    let lock = fs::File::create(scratch.slots().join("slot-0.lock")).unwrap();
    lock.try_lock().unwrap();
    let mut args = vec!["--slots", "1", "--"];
    args.extend(shell("exit 0"));
    let mut child =
        scratch.command(&args).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    assert_eq!(line, "heavy: waiting for one of 1 slots\n");
    drop(lock);
    assert!(child.wait().unwrap().success());
}

#[test]
fn a_forged_held_value_waits_like_any_caller() {
    let scratch = Scratch::new("forged");
    for value in ["", "0", "nonsense", "0:1-2"] {
        waits_for_the_slot(&scratch, value);
    }
}

#[test]
fn a_stale_held_value_waits_once_another_holder_has_the_slot() {
    let scratch = Scratch::new("stale-held");
    let stale = heavy::try_acquire(&scratch.slots(), ONE).unwrap().unwrap().held_value();
    waits_for_the_slot(&scratch, &stale);
}

#[test]
fn a_stale_held_value_whose_slot_is_free_takes_the_slot() {
    let scratch = Scratch::new("stale-free");
    let stale = heavy::try_acquire(&scratch.slots(), ONE).unwrap().unwrap().held_value();
    let mut args = vec!["--slots", "1", "--"];
    args.extend(shell("exit 0"));
    let output = scratch.command(&args).env(HELD_VAR, &stale).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stderr(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], format!("heavy: {HELD_VAR}={stale} names no slot held for it"));
    assert!(lines[1].starts_with("heavy: slot 0 after "), "{lines:?}");
    let owner = fs::read_to_string(scratch.slots().join("slot-0.owner")).unwrap();
    assert_eq!(owner, "", "the new holder cleared its token when it let the slot go");
}

#[test]
fn a_program_that_cannot_start_exits_2() {
    let scratch = Scratch::new("missing");
    let output = scratch.heavy(&["--", "lactose-heavy-no-such-program"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot run `lactose-heavy-no-such-program`"));
}

#[test]
fn a_bad_command_line_exits_2_without_taking_a_slot() {
    let scratch = Scratch::new("usage");
    for args in [&["sh"][..], &["--"], &["--slots", "0", "--", "sh"], &["--slots", "x"]] {
        let output = scratch.heavy(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", stderr(&output));
        assert!(output.stdout.is_empty());
        assert!(stderr(&output).starts_with("heavy: "), "{}", stderr(&output));
    }
    assert!(!scratch.0.join("lactose-heavy").exists());
}
