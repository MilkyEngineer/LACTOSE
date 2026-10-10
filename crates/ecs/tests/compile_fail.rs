//! What `#[system]` and `#[finalize]` refuse to compile, and what the author is told.
//!
//! Each file in `tests/ui` is checked as a binary of a scratch package whose dependencies
//! are this crate and the crates a game crate names beside it. A case passes when the check
//! fails and the compiler's output contains the listed text. The controls, `control.rs` and
//! `section_6_1.rs` (the systems of architecture section 6.1 as written), must compile in a
//! crate that forbids `unsafe` and denies warnings.

#![forbid(unsafe_code)]

use std::path::Path;
use std::process::Command;
use std::{env, fs, process};

/// The cases that must compile.
const CONTROLS: &[&str] = &["control", "section_6_1"];

/// File stem, then text the compiler's output must contain. Except where noted it is this
/// crate's own wording.
const CASES: &[(&str, &[&str])] = &[
    ("missing_name", &["missing `name = \"crate::name\"`"]),
    ("unknown_world", &["unknown world `servr`: expected `server`, `client` or `prediction`"]),
    ("exclusive_without_world", &["an exclusive system takes `&mut World` as its first parameter"]),
    (
        "world_without_exclusive",
        &["`&mut World` is the first parameter of an exclusive system: add `exclusive`"],
    ),
    (
        "reduce_without_return",
        &["a reduction returns one `R::Item` per entity: give the function a return type"],
    ),
    ("return_without_reduce", &["a system returns nothing unless it is a reduction"]),
    ("not_a_parameter", &["`u32` is not a system parameter"]),
    ("generic", &["a system cannot be generic"]),
    (
        "stray_finalize",
        &[
            "`#[finalize(not_a_reduction)]` names a system that is not a reduction",
            // The compiler's wording: the system the finalize names does not exist.
            "cannot find value `NOTHING_HERE` in this scope",
        ],
    ),
    (
        "stray_finalize_private",
        &[
            "`#[finalize(not_a_reduction)]` names a system that is not a reduction",
            "cannot find value `NOTHING_HERE` in this scope",
        ],
    ),
    // The compiler's wording: `writes` names components only.
    (
        "writes_not_a_component",
        &["the trait bound `u32: lactose_reflect::Component` is not satisfied", "writes = [u32]"],
    ),
    // The compiler's wording: the pairing is by name, and the finalize is not there.
    ("missing_finalize", &["cannot find value `__finalize_total` in this scope"]),
    // The compiler's wording: the finalize's first parameter is not the reducer's output.
    ("finalize_output_mismatch", &["mismatched types"]),
];

#[test]
#[cfg_attr(miri, ignore = "runs cargo")]
fn rejected_systems() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = crate_dir.join("../..");
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR"));
    // Sources are per process, so two runs that share a target directory cannot overwrite
    // each other's files. The build directory is shared: cargo locks it, and the
    // dependencies are built once.
    let project = scratch.join(format!("ecs-compile-fail-{}", process::id()));
    let target = scratch.join("ecs-compile-fail-target");
    let bins = project.join("src").join("bin");
    let _ = fs::remove_dir_all(&project);
    fs::create_dir_all(&bins).unwrap();

    // Its own workspace, so it is not a member of the engine's; the engine's lock file, so
    // it builds the same dependency versions without touching the network.
    let crates = repository.join("crates");
    let path = |name: &str| crates.join(name).display().to_string();
    let manifest = format!(
        "[package]\nname = \"ecs-compile-fail\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
         [workspace]\n\n[dependencies]\nlactose-ecs = {{ path = '{}' }}\n\
         lactose-math = {{ path = '{}' }}\nlactose-reflect = {{ path = '{}' }}\n\
         lactose-core = {{ path = '{}' }}\n\
         bytemuck = {{ version = \"1\", features = [\"derive\"] }}\n",
        path("ecs"),
        path("math"),
        path("reflect"),
        path("core"),
    );
    fs::write(project.join("Cargo.toml"), manifest).unwrap();
    fs::copy(repository.join("Cargo.lock"), project.join("Cargo.lock")).unwrap();
    let ui = crate_dir.join("tests").join("ui");
    for entry in fs::read_dir(&ui).unwrap() {
        let path = entry.unwrap().path();
        fs::copy(&path, bins.join(path.file_name().unwrap())).unwrap();
    }

    // Cargo reads `.cargo/config.toml` from the working directory upwards, which reaches the
    // repository's only when the target directory is inside it. The file is passed
    // explicitly so the scratch build is configured like every other build either way.
    let check = |case: &str| {
        let output = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["check", "--offline", "--color", "never", "--bin", case, "--config"])
            .arg(repository.join(".cargo").join("config.toml"))
            .arg("--target-dir")
            .arg(&target)
            .current_dir(&project)
            .output()
            .unwrap();
        (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
    };

    for control in CONTROLS {
        let (compiled, stderr) = check(control);
        assert!(compiled, "the control `{control}` must compile:\n{stderr}");
    }

    let listed = CASES.len() + CONTROLS.len();
    assert_eq!(fs::read_dir(&ui).unwrap().count(), listed, "every file in tests/ui is a case");
    for (case, messages) in CASES {
        let (compiled, stderr) = check(case);
        assert!(!compiled, "`{case}` must not compile");
        for message in *messages {
            assert!(stderr.contains(message), "`{case}` does not say `{message}`:\n{stderr}");
        }
    }

    let _ = fs::remove_dir_all(&project);
}
