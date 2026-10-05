//! What must not compile, and what the author is told.
//!
//! Each file in `tests/ui` is checked as a binary of a scratch package that depends on this
//! crate alone. A case passes when the check fails and the compiler's output contains the
//! listed text, all of which is this crate's own wording; nothing here pins the compiler's
//! or `bytemuck`'s. `control.rs` must compile.

use std::path::Path;
use std::process::Command;
use std::{env, fs};

/// File stem, then the messages the author must see.
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
];

#[test]
#[cfg_attr(miri, ignore = "runs cargo")]
fn rejected_declarations() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let project = Path::new(env!("CARGO_TARGET_TMPDIR")).join("compile_fail");
    let bins = project.join("src").join("bin");
    let _ = fs::remove_dir_all(project.join("src"));
    fs::create_dir_all(&bins).unwrap();
    // Its own workspace, so it is not a member of the engine's; the engine's lock file, so it
    // builds the same dependency versions without touching the network.
    let manifest = format!(
        "[package]\nname = \"reflect-compile-fail\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
         [workspace]\n\n[dependencies]\nlactose-reflect = {{ path = '{}' }}\n",
        crate_dir.display()
    );
    fs::write(project.join("Cargo.toml"), manifest).unwrap();
    fs::copy(crate_dir.join("../../Cargo.lock"), project.join("Cargo.lock")).unwrap();
    let ui = crate_dir.join("tests").join("ui");
    for entry in fs::read_dir(&ui).unwrap() {
        let path = entry.unwrap().path();
        fs::copy(&path, bins.join(path.file_name().unwrap())).unwrap();
    }

    // Run from inside the repository so `.cargo/config.toml` applies, as it does to every
    // other build.
    let check = |case: &str| {
        let output = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["check", "--offline", "--color", "never", "--bin", case, "--target-dir"])
            .arg(project.join("target"))
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

    // The author reads this crate's explanation of padding before `bytemuck`'s.
    let (_, stderr) = check("implicit_padding");
    let ours = stderr.find("has padding before field").unwrap();
    if let Some(theirs) = stderr.find("derive(NoUninit)") {
        assert!(ours < theirs, "the padding explanation is buried:\n{stderr}");
    }
}
