//! The reference lint file, `crates/core/clippy.toml`, read by clippy itself over a probe crate
//! in a temporary workspace that names the solver's math directly.
//!
//! The trait routes of D119 are banned by trait path with `allow-invalid`, which silences a
//! path that does not resolve; so a typo, or a path that moves in an upgrade, would ban
//! nothing and say nothing. This proves each route is refused by the def ids clippy resolves,
//! at the versions the repository's lock holds, which are in the registry cache wherever this
//! package is built (the physics backend compiles them).

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| env!("CARGO").to_owned())
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{}: {e}", parent.display()));
    }
    fs::write(path, contents).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// Each line of the probe that clippy must refuse, and the path its finding names.
const REFUSED: [(&str, &str); 6] = [
    ("use num_traits::Float;", "num_traits::Float"),
    ("pub fn generic<T: num_traits::real::Real>(x: T) -> T { x.sin() }", "num_traits::real::Real"),
    (
        "pub fn qualified(x: f64) -> f64 { simba::scalar::RealField::max(x, 0.0) }",
        "simba::scalar::RealField::max",
    ),
    (
        "pub fn bounded<T: simba::scalar::RealField>(x: T) -> bool { x.is_sign_negative() }",
        "simba::scalar::RealField::is_sign_negative",
    ),
    (
        "pub fn lanes<T: simba::simd::SimdRealField>(x: T, y: T) -> T { x.simd_copysign(y) }",
        "simba::simd::SimdRealField::simd_copysign",
    ),
    // The control: the inherent method the routes reach is refused as it always was.
    ("pub fn control(x: f64) -> f64 { x.max(0.0) }", "f64::max"),
];

#[test]
fn the_trait_routes_to_the_platform_float_methods_are_refused() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("boundary-lints");
    let ws = base.join("ws");
    if ws.exists() {
        fs::remove_dir_all(&ws).unwrap_or_else(|e| panic!("{}: {e}", ws.display()));
    }
    write(
        &ws.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
         [workspace]\n\n[dependencies]\nnum-traits = \"0.2\"\nsimba = \"0.10\"\n",
    );
    // The repository's lock, so cargo keeps its versions and drops what the probe does not use.
    let lock = fs::read_to_string(repository.join("Cargo.lock")).unwrap();
    write(&ws.join("Cargo.lock"), &lock);
    let reference = fs::read_to_string(repository.join("crates/core/clippy.toml")).unwrap();
    write(&ws.join("clippy.toml"), &reference);
    let source: Vec<&str> = REFUSED.iter().map(|(line, _)| *line).collect();
    write(&ws.join("src/lib.rs"), &format!("{}\n", source.join("\n")));

    let output = Command::new(cargo())
        .args(["clippy", "--offline", "--quiet", "--message-format", "short", "--"])
        .args(["-D", "clippy::disallowed_methods", "-D", "clippy::disallowed_types"])
        .current_dir(&ws)
        .env("CARGO_TARGET_DIR", base.join("target"))
        .output()
        .unwrap_or_else(|e| panic!("cannot run cargo clippy: {e}"));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "clippy refused nothing:\n{text}");
    for (at, (line, path)) in REFUSED.iter().enumerate() {
        // Windows writes the path with a backslash.
        let finding = text.lines().map(|finding| finding.replace('\\', "/")).any(|finding| {
            finding.starts_with(&format!("src/lib.rs:{}:", at + 1))
                && finding.contains("disallowed")
                && finding.contains(&format!("`{path}`"))
        });
        assert!(finding, "line {}, `{line}`, is not refused for `{path}`:\n{text}", at + 1);
    }
}
