//! What must not compile, and what the author is told.
//!
//! Each file in `tests/ui` is checked as a binary of a scratch package whose dependencies are
//! this crate and the test-only macros of `tests/support`. A case passes when the check
//! fails and the compiler's output contains the listed text. `control.rs` must compile.

#![forbid(unsafe_code)]

use std::path::Path;
#[expect(clippy::disallowed_types, reason = "a test that runs cargo on a scratch package")]
use std::process::Command;
use std::{env, fs, process};

/// File stem, then text the compiler's output must contain. Except where noted it is this
/// crate's own wording.
const CASES: &[(&str, &[&str])] = &[
    ("missing_field_id", &["field `max` has no field ID: add `#[field(id = N)]`"]),
    ("duplicate_field_id", &["field ID 1 is already used by field `current`"]),
    ("retired_field_id", &["field `max` uses the retired field ID 2"]),
    ("conditional_field", &["field `extra` cannot be conditional"]),
    ("generic", &["a component cannot be generic"]),
    ("missing_policy", &["missing persistence policy: add `persistent`, `transient` or `derived`"]),
    ("data_carrying_enum", &["data-carrying enums are not reflected"]),
    (
        "implicit_padding",
        &[
            "`Between` has padding before field `count`: order the fields from the largest \
             alignment to the smallest, or insert `#[field(padding)] _pad: \
             lactose_reflect::Padding<N>` before it",
            "`After` has padding after its last field `flag`",
        ],
    ),
    (
        "padding_type",
        &["a `#[field(padding)]` field must have the type `lactose_reflect::Padding<N>`"],
    ),
    (
        "not_a_field_type",
        &[
            "`usize` cannot be a reflected field",
            "`[u32; 4]` cannot be a reflected field",
            "an array field is a `lactose_core::PlainArray<T, N>`, never a bare `[T; N]`",
        ],
    ),
    // Attributes in raw spelling, which the compiler honours, get the same errors.
    ("raw_repr", &["remove this `#[repr]`"]),
    // The compiler has already applied a true `cfg_attr` when the attribute runs.
    ("raw_cfg_attr", &["remove this `#[repr]`"]),
    ("enum_raw_repr", &["the only representation of a `#[value]` enum is its width"]),
    // An attribute below ours rewrites the type after we read it. Nothing we parsed shows
    // it; the expansion's checks against the final type do.
    ("rewritten_align", &["`Aligned` is not the struct its attribute read"]),
    // The compiler's wording: the recorded field type no longer matches.
    ("rewritten_field", &["mismatched types", "PhantomData<u8>", "PhantomData<bool>"]),
    ("rewritten_field_deref", &["mismatched types", "PhantomData<u8>", "PhantomData<Wrapper>"]),
    ("rewritten_enum_align", &["`Mode` is not the enum its attribute read"]),
    ("rewritten_enum_discriminants", &["`Mode` is not the enum its attribute read"]),
    // The compiler's wording: the match over the recorded variants is not exhaustive.
    ("rewritten_enum_variants", &["`Mode::Added` not covered"]),
];

#[test]
#[cfg_attr(miri, ignore = "runs cargo")]
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a test that writes a scratch package and finds cargo through the environment"
)]
fn rejected_declarations() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = crate_dir.join("../..");
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR"));
    // Sources are per process, so two runs that share a target directory cannot overwrite
    // each other's files. The build directory is shared: cargo locks it, and the
    // dependencies are built once.
    let project = scratch.join(format!("compile_fail-{}", process::id()));
    let target = scratch.join("compile_fail-target");
    let bins = project.join("src").join("bin");
    let macros = project.join("rewrite-macros");
    let _ = fs::remove_dir_all(&project);
    fs::create_dir_all(&bins).unwrap();
    fs::create_dir_all(macros.join("src")).unwrap();

    // Its own workspace, so it is not a member of the engine's; the engine's lock file, so it
    // builds the same dependency versions without touching the network.
    let manifest = format!(
        "[package]\nname = \"reflect-compile-fail\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
         [workspace]\n\n[dependencies]\nlactose-reflect = {{ path = '{}' }}\n\
         rewrite-macros = {{ path = \"rewrite-macros\" }}\n",
        crate_dir.display()
    );
    fs::write(project.join("Cargo.toml"), manifest).unwrap();
    fs::write(
        macros.join("Cargo.toml"),
        "[package]\nname = \"rewrite-macros\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
         [lib]\nproc-macro = true\n",
    )
    .unwrap();
    let support = crate_dir.join("tests").join("support");
    fs::copy(support.join("rewrite_macros.rs"), macros.join("src").join("lib.rs")).unwrap();
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

    let (compiled, stderr) = check("control");
    assert!(compiled, "the control case must compile:\n{stderr}");

    let listed = CASES.len() + 1;
    assert_eq!(fs::read_dir(&ui).unwrap().count(), listed, "every file in tests/ui is a case");
    for (case, messages) in CASES {
        let (compiled, stderr) = check(case);
        assert!(!compiled, "`{case}` must not compile");
        for message in *messages {
            assert!(stderr.contains(message), "`{case}` does not say `{message}`:\n{stderr}");
        }
    }

    // The author reads this crate's explanation of padding before `bytemuck`'s. If
    // `bytemuck` stops reporting padding, or rewords it, this fails and is revisited.
    let (_, stderr) = check("implicit_padding");
    let ours = stderr.find("has padding before field").unwrap();
    let theirs = stderr
        .find("derive(NoUninit) was applied to a type with padding")
        .expect("bytemuck's padding message is absent or reworded");
    assert!(ours < theirs, "the padding explanation is buried:\n{stderr}");

    let _ = fs::remove_dir_all(&project);
}
