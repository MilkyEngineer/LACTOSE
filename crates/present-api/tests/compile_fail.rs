//! What the crate refuses to build, and what the author is told: a view-model that is not
//! `derived`, and a snapshot read of a component that is not `extracted`.
//!
//! Each file in `tests/ui` is a binary of a scratch package whose dependencies are this
//! crate and the crates a game crate names beside it. The refusals are assertions the
//! compiler evaluates where a call is instantiated, which a check never does, so every case
//! is built: the control must build in a crate that forbids `unsafe`, and a case passes
//! when its check passes, its build fails, and the output names the assertion and each
//! failing instantiation.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
#[expect(clippy::disallowed_types, reason = "a test that runs cargo on a scratch package")]
use std::process::Command;
use std::{env, fs, process};

/// File stem, then text a build's output must contain.
const CASES: &[(&str, &[&str])] = &[
    (
        "view_model_not_derived",
        &[
            "a view-model is declared `derived`",
            "ViewModels::set::<SavedHud>",
            "ViewModels::get::<SavedHud>",
            "ViewModels::remove::<SavedHud>",
        ],
    ),
    (
        "snapshot_not_extracted",
        &[
            "a snapshot holds `extracted` components alone",
            "PresentSnapshot::get::<Hidden>",
            "PresentSnapshot::rows::<Hidden>",
        ],
    ),
];

/// The directory of the cases.
fn ui() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("ui")
}

/// A scratch package holding every file of `tests/ui` as a binary, and the target directory
/// every run shares: cargo locks it, and the dependencies are built once. The sources are
/// per process, so two runs cannot overwrite each other's files.
#[expect(clippy::disallowed_methods, reason = "a test that writes a scratch package")]
fn scratch_package() -> (PathBuf, PathBuf) {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let project = scratch.join(format!("present-api-build-fail-{}", process::id()));
    let target = scratch.join("present-api-build-fail-target");
    let bins = project.join("src").join("bin");
    let _ = fs::remove_dir_all(&project);
    fs::create_dir_all(&bins).unwrap();

    // Its own workspace, so it is not a member of the engine's; the engine's lock file, so it
    // builds the same dependency versions without touching the network.
    let crates = repository.join("crates");
    let path = |name: &str| crates.join(name).display().to_string();
    let manifest = format!(
        "[package]\nname = \"present-api-build-fail\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
         [workspace]\n\n[dependencies]\nlactose-present-api = {{ path = '{}' }}\n\
         lactose-reflect = {{ path = '{}' }}\nlactose-core = {{ path = '{}' }}\n\
         bytemuck = {{ version = \"1\", features = [\"derive\"] }}\n",
        path("present-api"),
        path("reflect"),
        path("core"),
    );
    fs::write(project.join("Cargo.toml"), manifest).unwrap();
    fs::copy(repository.join("Cargo.lock"), project.join("Cargo.lock")).unwrap();
    for entry in fs::read_dir(ui()).unwrap() {
        let path = entry.unwrap().path();
        fs::copy(&path, bins.join(path.file_name().unwrap())).unwrap();
    }
    (project, target)
}

/// `cargo <mode>` of the binary `case` in `project`: whether it succeeded, and its error
/// output.
///
/// Cargo reads `.cargo/config.toml` from the working directory upwards, which reaches the
/// repository's only when the target directory is inside it. The file is passed explicitly
/// so the scratch build is configured like every other build either way.
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a test that runs cargo, found through the environment, on a scratch package"
)]
fn cargo(mode: &str, project: &Path, target: &Path, case: &str) -> (bool, String) {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([mode, "--offline", "--color", "never", "--bin", case, "--config"])
        .arg(repository.join(".cargo").join("config.toml"))
        .arg("--target-dir")
        .arg(target)
        .current_dir(project)
        .output()
        .unwrap();
    (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
#[cfg_attr(miri, ignore = "runs cargo")]
#[expect(clippy::disallowed_methods, reason = "removes the scratch package")]
fn a_view_model_not_derived_and_a_snapshot_read_not_extracted_do_not_build() {
    let (project, target) = scratch_package();
    assert_eq!(fs::read_dir(ui()).unwrap().count(), CASES.len() + 1, "every file is a case");
    let (built, stderr) = cargo("build", &project, &target, "control");
    assert!(built, "the control must build:\n{stderr}");
    for (case, messages) in CASES {
        let (checked, stderr) = cargo("check", &project, &target, case);
        assert!(checked, "a check of `{case}` instantiates nothing and passes:\n{stderr}");
        let (built, stderr) = cargo("build", &project, &target, case);
        assert!(!built, "`{case}` must not build");
        for message in *messages {
            assert!(stderr.contains(message), "`{case}` does not say `{message}`:\n{stderr}");
        }
    }
    let _ = fs::remove_dir_all(&project);
}
