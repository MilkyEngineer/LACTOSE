//! The checks run against the real workspace, through the binary, from the repository root.
//!
//! These prove the plumbing: that the real `crate-tiers.toml`, policy file, `cargo metadata`
//! and `cargo tree` output parse and reach the checks. The tier check must also come back
//! clean. The boundary check is not required to: its findings are facts about other crates,
//! which gate the `policy` CI job and not this crate's tests.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// Exit status of a run that found violations, as opposed to one that could not run (2).
const FINDINGS: i32 = 1;

fn xtask(args: &[&str]) -> Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Command::new(env!("CARGO_BIN_EXE_lactose-xtask"))
        .args(args)
        .current_dir(root)
        .output()
        .expect("the xtask binary runs")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn the_workspace_passes_the_tier_check() {
    let output = xtask(&["tiers"]);
    assert!(output.status.success(), "{}", text(&output));
}

#[test]
fn the_boundary_check_runs_and_prints_findings_as_path_and_message() {
    let output = xtask(&["boundary"]);
    let code = output.status.code();
    assert!(code == Some(0) || code == Some(FINDINGS), "{}", text(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(code == Some(FINDINGS), !stdout.is_empty(), "{stdout}");
    for finding in stdout.lines() {
        let (path, message) = finding.split_once(": ").expect(finding);
        assert!(!path.contains(' ') && !message.is_empty(), "{finding}");
    }
}

#[test]
fn the_feature_check_passes_with_no_roots_and_reads_real_trees() {
    let output = xtask(&["features"]);
    assert!(output.status.success(), "{}", text(&output));

    // Two packages that both compile `lactose-core`. Whether their trees agree is not this
    // test's business; that the real `cargo tree` output parses is.
    let output = xtask(&["features", "--package", "lactose-core", "--package", "lactose-task"]);
    let code = output.status.code();
    assert!(code == Some(0) || code == Some(FINDINGS), "{}", text(&output));
    assert!(text(&output).contains("2 roots"), "{}", text(&output));
}

#[test]
fn the_mode_matrix_prints_every_mode() {
    let output = xtask(&["sour", "matrix"]);
    assert!(output.status.success(), "{}", text(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for mode in ["workers:", "repeat:", "layout:", "chaos:", "architecture:"] {
        assert!(stdout.lines().any(|line| line.starts_with(mode)), "{stdout}");
    }
}

#[test]
fn a_bad_command_line_is_a_usage_error() {
    for args in
        [&["bogus"][..], &["sour", "run"], &["sour", "run", "--mode", "speed"], &["tiers", "extra"]]
    {
        assert_eq!(xtask(args).status.code(), Some(2), "{args:?}");
    }
    assert!(xtask(&["help"]).status.success());
}

fn repository_file(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // A Windows checkout may carry carriage returns; the checks below are on lines.
    std::fs::read_to_string(root.join(path)).expect(path).replace("\r\n", "\n")
}

#[test]
fn every_cargo_command_ci_runs_is_locked() {
    // Review item 20. `cargo fmt` reads no lock file; `cargo xtask` is locked by its alias.
    let alias = repository_file(".cargo/config.toml");
    assert!(alias.contains("xtask = \"run --quiet --locked -p lactose-xtask --\""), "{alias}");
    for workflow in [".github/workflows/ci.yml", ".github/workflows/nightly.yml"] {
        let text = repository_file(workflow);
        assert!(text.contains("\npermissions:\n  contents: read\n"), "{workflow}");
        let commands = text
            .lines()
            .map(|line| line.trim().trim_start_matches("# ").trim_start_matches("- "))
            .filter_map(|line| line.strip_prefix("run: cargo "));
        for command in commands {
            let exempt = command.starts_with("fmt ") || command.starts_with("xtask ");
            assert!(exempt || command.contains("--locked"), "{workflow}: cargo {command}");
        }
    }
}
