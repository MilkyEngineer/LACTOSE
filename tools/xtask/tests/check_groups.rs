//! `cargo xtask boundary` and `cargo xtask group` over a temporary workspace whose check groups
//! leave a member out: the binary, not only the check, must report it, since CI's policy job
//! reads `boundary` and each check job reads `group`.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Exit status of a run that found violations, as opposed to one that could not run (2).
const FINDINGS: i32 = 1;

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{}: {e}", parent.display()));
    }
    fs::write(path, contents).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn run(command: &mut Command, ws: &Path) -> Output {
    command
        .current_dir(ws)
        .env("CARGO_TARGET_DIR", ws.join("target"))
        .output()
        .unwrap_or_else(|e| panic!("{command:?}: {e}"))
}

#[test]
fn a_member_in_no_check_group_fails_boundary_and_prints_no_group() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ws = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("check-groups");
    if ws.exists() {
        fs::remove_dir_all(&ws).unwrap_or_else(|e| panic!("{}: {e}", ws.display()));
    }
    let members = ["one", "two", "three"];
    let list: Vec<String> = members.iter().map(|name| format!("\"crates/{name}\"")).collect();
    write(
        &ws.join("Cargo.toml"),
        &format!("[workspace]\nmembers = [{}]\nresolver = \"3\"\n", list.join(", ")),
    );
    for name in members {
        write(
            &ws.join(format!("crates/{name}/Cargo.toml")),
            &format!(
                "[package]\nname = \"lactose-{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
                 publish = false\n"
            ),
        );
        write(&ws.join(format!("crates/{name}/src/lib.rs")), "#![forbid(unsafe_code)]\n");
    }
    let tiers = fs::read_to_string(repository.join("crate-tiers.toml")).unwrap();
    write(&ws.join("crate-tiers.toml"), &tiers);
    // The repository's policy with only the groups replaced: `lactose-three` is in neither.
    let policy = fs::read_to_string(repository.join("tools/xtask/ci-policy.toml")).unwrap();
    let policy = policy.replace("\r\n", "\n");
    let (before, _) = policy.split_once("\n[check_groups]\n").expect("a [check_groups] table");
    write(
        &ws.join("tools/xtask/ci-policy.toml"),
        &format!(
            "{before}\n[check_groups]\nbase = [\"lactose-one\"]\nphysics = [\"lactose-two\"]\n"
        ),
    );
    let cargo = env::var("CARGO").unwrap_or_else(|_| env!("CARGO").to_owned());
    let lock = run(Command::new(&cargo).args(["generate-lockfile", "--offline"]), &ws);
    assert!(lock.status.success(), "{}", text(&lock));

    let xtask = env!("CARGO_BIN_EXE_lactose-xtask");
    let boundary = run(Command::new(xtask).arg("boundary"), &ws);
    assert_eq!(boundary.status.code(), Some(FINDINGS), "{}", text(&boundary));
    let finding = "crates/three/Cargo.toml: `lactose-three` is in no group of [check_groups] in \
                   tools/xtask/ci-policy.toml, so no CI check job builds, lints or tests it; add \
                   it to one";
    let stdout = String::from_utf8_lossy(&boundary.stdout);
    assert!(stdout.lines().any(|line| line == finding), "{stdout}");

    let group = run(Command::new(xtask).args(["group", "base"]), &ws);
    assert_eq!(group.status.code(), Some(2), "{}", text(&group));
    assert!(group.stdout.is_empty(), "{}", text(&group));
    assert!(String::from_utf8_lossy(&group.stderr).contains(finding), "{}", text(&group));
}
