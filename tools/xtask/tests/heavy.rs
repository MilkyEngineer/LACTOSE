//! `heavy` through the binary: the exit status, the streams and the command line.
//!
//! Each run gets its own temporary directory, so these tests never wait for, or hold, the
//! machine's real slots.

#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

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

    /// Runs `cargo xtask heavy` with `args`, with this directory as its temporary directory.
    fn heavy(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_lactose-xtask"))
            .arg("heavy")
            .args(args)
            // `std::env::temp_dir` reads `TMPDIR` on Unix and `TMP`, then `TEMP`, on Windows.
            .env("TMPDIR", &self.0)
            .env("TMP", &self.0)
            .env("TEMP", &self.0)
            // Set when these tests themselves run under `heavy`.
            .env_remove("LACTOSE_HEAVY_SLOT")
            .output()
            .expect("the xtask binary runs")
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
    assert_eq!(lines[1], "heavy: inside slot 0 already");
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
