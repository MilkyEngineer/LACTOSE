//! What `#[system]` and `#[finalize]` refuse to compile, what `World::insert_resource`
//! refuses to build, and what the author is told.
//!
//! Each file in `tests/ui` is a binary of a scratch package whose dependencies are this
//! crate and the crates a game crate names beside it. A case passes when the check fails
//! and the compiler's output contains the listed text. The controls, `control.rs` and
//! `section_6_1.rs` (the systems of architecture section 6.1 as written), must compile in a
//! crate that forbids `unsafe` and denies warnings. A build case is built, not checked: its
//! refusal is an assertion the compiler evaluates where a call is instantiated, which a
//! check never does.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
#[expect(clippy::disallowed_types, reason = "a test that runs cargo on a scratch package")]
use std::process::Command;
use std::sync::Arc;
use std::{env, fs, process};

use lactose_core::EntityRef;
use lactose_ecs::{
    ClaimedBy, InRegion, OwnedBy, PredictedOrigin, RegionConfig, RegionId, RegionMode,
    WorldBuilder, WorldKind,
};
use lactose_reflect::{Registry, resource};

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

/// File stem, then text a build's output must contain: the assertion's text, and each
/// instantiation that fails it, which the compiler's note names.
const BUILD_CASES: &[(&str, &[&str])] = &[(
    "world_resource_reference",
    &[
        "a world resource holds no entity reference (D142)",
        "insert_resource::<Target>",
        "insert_resource::<NestedTarget>",
        "insert_resource::<TargetList>",
    ],
)];

/// The directory of the cases.
fn ui() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("ui")
}

/// A scratch package holding every file of `tests/ui` as a binary, and the target
/// directory every run shares: cargo locks it, and the dependencies are built once. The
/// sources are per process and per test, so two runs cannot overwrite each other's files.
#[expect(clippy::disallowed_methods, reason = "a test that writes a scratch package")]
fn scratch_package(name: &str) -> (PathBuf, PathBuf) {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let project = scratch.join(format!("ecs-{name}-{}", process::id()));
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
fn rejected_systems() {
    let (project, target) = scratch_package("compile-fail");
    let check = |case: &str| cargo("check", &project, &target, case);

    for control in CONTROLS {
        let (compiled, stderr) = check(control);
        assert!(compiled, "the control `{control}` must compile:\n{stderr}");
    }

    let listed = CASES.len() + CONTROLS.len() + BUILD_CASES.len();
    assert_eq!(fs::read_dir(ui()).unwrap().count(), listed, "every file in tests/ui is a case");
    for (case, messages) in CASES {
        let (compiled, stderr) = check(case);
        assert!(!compiled, "`{case}` must not compile");
        for message in *messages {
            assert!(stderr.contains(message), "`{case}` does not say `{message}`:\n{stderr}");
        }
    }

    let _ = fs::remove_dir_all(&project);
}

/// What a control inserts as a world resource: no reference.
#[resource(key = "compile_fail::plain", derived)]
pub struct Plain(#[field(id = 1)] pub u32);

/// What the build case inserts as a world resource, legal as a region resource.
#[resource(key = "compile_fail::target", persistent)]
#[derive(PartialEq, Debug)]
pub struct Target {
    /// The entity it targets.
    #[field(id = 1)]
    pub entity: EntityRef,
}

/// D142: `World::insert_resource` of a type that holds an `EntityRef`, in a field, in a
/// nested `#[value]` struct or in a `PlainArray`, fails the build with the assertion's text
/// and each failing instantiation, while a check of the same binary passes. The controls
/// build and run here: a type without a reference through `insert_resource`, and one with
/// a reference as a region resource.
#[test]
#[cfg_attr(miri, ignore = "runs cargo")]
#[expect(clippy::disallowed_methods, reason = "removes the scratch package")]
fn a_world_resource_that_holds_an_entity_reference_does_not_build() {
    let (project, target) = scratch_package("build-fail");
    for (case, messages) in BUILD_CASES {
        let (checked, stderr) = cargo("check", &project, &target, case);
        assert!(checked, "a check of `{case}` instantiates nothing and passes:\n{stderr}");
        let (built, stderr) = cargo("build", &project, &target, case);
        assert!(!built, "`{case}` must not build");
        for message in *messages {
            assert!(stderr.contains(message), "`{case}` does not say `{message}`:\n{stderr}");
        }
    }
    let _ = fs::remove_dir_all(&project);

    let mut registry = Registry::new();
    registry.register::<InRegion>().unwrap();
    registry.register::<OwnedBy>().unwrap();
    registry.register::<ClaimedBy>().unwrap();
    registry.register::<PredictedOrigin>().unwrap();
    registry.register::<Target>().unwrap();
    let mut world = WorldBuilder::new(WorldKind::Server, Arc::new(registry)).build().unwrap();
    let region = RegionId(1);
    world.create_region(RegionConfig { id: region, seed: 1, mode: RegionMode::Simulated }).unwrap();
    assert!(world.insert_resource(Plain(7)).is_none());
    assert_eq!(world.resource::<Plain>().map(|plain| plain.0), Some(7));
    let target = Target { entity: EntityRef::NONE };
    world.insert_region_resource(region, target).unwrap();
    assert_eq!(world.region_resource::<Target>(region), Some(&Target { entity: EntityRef::NONE }));
}
