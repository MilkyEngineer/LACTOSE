//! `cargo xtask vendor --offline` over a temporary workspace that builds `cfg-if` from a
//! patched copy, vendored from the registry cache.
//!
//! `cfg-if` is in the repository's own lock (this package compiles it), so its index entry
//! and archive are cached wherever these tests are built. Each test vendors it afresh into a
//! workspace under the target directory with its own `[workspace]` root, patches one place
//! of `src/lib.rs`, and then tampers with one thing.
//!
//! The advisory run needs cargo-deny and its database. Where cargo-deny is not installed,
//! `--offline` skips it and says so, and the clean case checks that it says so.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

/// Exit status of a run that found violations.
const FINDINGS: i32 = 1;

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| env!("CARGO").to_owned())
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{}: {e}", parent.display()));
    }
    fs::write(path, contents).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|e| panic!("{}: {e}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|e| panic!("{}: {e}", from.display())) {
        let entry = entry.unwrap_or_else(|e| panic!("{e}"));
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap_or_else(|e| panic!("{target:?}: {e}"));
        }
    }
}

/// The version of `cfg-if` the repository's lock holds.
fn cached_version() -> String {
    let lock = read(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"));
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line == "name = \"cfg-if\"" {
            let version = lines.next().and_then(|l| l.strip_prefix("version = \""));
            return version.and_then(|v| v.strip_suffix('"')).expect("a version line").to_owned();
        }
    }
    panic!("Cargo.lock holds no cfg-if");
}

/// Runs `command` with the target directory inside `dir` and fails the test if it fails.
fn run(command: &mut Command, dir: &Path) -> Output {
    let output = command
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .unwrap_or_else(|e| panic!("{command:?}: {e}"));
    assert!(output.status.success(), "{command:?}\n{}", text(&output));
    output
}

/// A unified diff of one file, as `git diff` writes it: one hunk around the one contiguous
/// region where `old` and `new` differ, with three lines of context.
fn unified(path: &str, old: &str, new: &str) -> String {
    assert!(old.ends_with('\n') && new.ends_with('\n'), "{path} ends without a newline");
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let start = prefix.saturating_sub(3);
    let (old_end, new_end) = (old.len() - suffix, new.len() - suffix);
    let after = suffix.min(3);
    let mut diff = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    let old_count = old_end + after - start;
    let new_count = new_end + after - start;
    diff.push_str(&format!("@@ -{},{old_count} +{},{new_count} @@\n", start + 1, start + 1));
    for line in &old[start..prefix] {
        diff.push_str(&format!(" {line}\n"));
    }
    for line in &old[prefix..old_end] {
        diff.push_str(&format!("-{line}\n"));
    }
    for line in &new[prefix..new_end] {
        diff.push_str(&format!("+{line}\n"));
    }
    for line in &old[old_end..old_end + after] {
        diff.push_str(&format!(" {line}\n"));
    }
    diff
}

/// A temporary workspace with one member, `app`, that depends on `cfg-if` through
/// `[workspace.dependencies]`, and a copy of `cfg-if` under `vendor/` named by
/// `[patch.crates-io]` with its record and its section in `vendor/README.md`. No patch yet.
struct Fixture {
    ws: PathBuf,
    version: String,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("vendor-{name}"));
        if base.exists() {
            fs::remove_dir_all(&base).unwrap_or_else(|e| panic!("{}: {e}", base.display()));
        }
        let version = cached_version();
        let fetch = base.join("fetch");
        write(
            &fetch.join("Cargo.toml"),
            &format!(
                "[package]\nname = \"fetch\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
                 publish = false\n\n[workspace]\n\n[dependencies]\ncfg-if = \"={version}\"\n"
            ),
        );
        write(&fetch.join("src/lib.rs"), "");
        run(
            Command::new(cargo())
                .args(["vendor", "--offline", "--versioned-dirs", "--manifest-path"])
                .arg(fetch.join("Cargo.toml"))
                .arg(base.join("fetched")),
            &fetch,
        );
        let ws = base.join("ws");
        let dir = format!("cfg-if-{version}");
        copy_tree(&base.join("fetched").join(&dir), &ws.join("vendor").join(&dir));
        write(
            &ws.join("Cargo.toml"),
            &format!(
                "[workspace]\nresolver = \"3\"\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n\n\
                 [workspace.dependencies]\ncfg-if = \"{version}\"\n\n\
                 [patch.crates-io]\ncfg-if = {{ path = \"vendor/{dir}\" }}\n"
            ),
        );
        write(
            &ws.join("app/Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             publish = false\n\n[dependencies]\ncfg-if.workspace = true\n",
        );
        write(&ws.join("app/src/lib.rs"), "");
        write(&ws.join(".gitattributes"), "vendor/** -text\n");
        write(
            &ws.join("vendor/README.md"),
            &format!("# Vendored crates\n\n## cfg-if {version}\n\nThe test's copy.\n"),
        );
        let deny = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../deny.toml");
        write(&ws.join("deny.toml"), &read(&deny));
        let fixture = Self { ws, version };
        fixture.lock();
        fixture
    }

    fn dir(&self) -> String {
        format!("cfg-if-{}", self.version)
    }

    fn copy(&self) -> PathBuf {
        self.ws.join("vendor").join(self.dir())
    }

    fn lock(&self) {
        run(Command::new(cargo()).args(["generate-lockfile", "--offline"]), &self.ws);
    }

    /// Changes each `(file, new contents)` of the copy and writes the patch that made them.
    fn patch(&self, changes: &[(&str, String)]) {
        let mut patch = String::new();
        for (file, new) in changes {
            let path = self.copy().join(file);
            patch.push_str(&unified(file, &read(&path), new));
            write(&path, new);
        }
        write(&self.ws.join(format!("vendor/{}.patch", self.dir())), &patch);
    }

    /// `src/lib.rs` with `lines` appended.
    fn lib_with(&self, lines: &str) -> (&'static str, String) {
        ("src/lib.rs", read(&self.copy().join("src/lib.rs")) + lines)
    }

    /// The patch every test starts from: one line and its notice at the end of `src/lib.rs`.
    fn patched(name: &str) -> Self {
        let fixture = Self::new(name);
        let change = fixture.lib_with(
            "// LACTOSE patch: the integration test's change.\npub const PATCHED: () = ();\n",
        );
        fixture.patch(&[change]);
        fixture
    }

    fn vendor(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_lactose-xtask"))
            .args(["vendor", "--offline"])
            .current_dir(&self.ws)
            .env("CARGO", cargo())
            .env("CARGO_TARGET_DIR", self.ws.join("target"))
            .output()
            .expect("the xtask binary runs")
    }

    /// Runs the check and requires exit status 1 with exactly `expected` on standard output.
    fn findings(&self, expected: &[String]) {
        let output = self.vendor();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let found: Vec<&str> = stdout.lines().collect();
        assert_eq!(found, expected, "{}", text(&output));
        assert_eq!(output.status.code(), Some(FINDINGS), "{}", text(&output));
    }

    /// The first file of the record that is neither the manifest nor `src/lib.rs`.
    fn untouched_file(&self) -> String {
        let record = read(&self.copy().join(".cargo-checksum.json"));
        let record: serde_json::Value = serde_json::from_str(&record).expect("a record");
        let files = record["files"].as_object().expect("a files map");
        let mut names: Vec<&String> = files.keys().collect();
        names.sort_by_key(|name| (name.as_str() != "README.md", name.as_str()));
        let chosen =
            names.into_iter().find(|n| !["Cargo.toml", "src/lib.rs"].contains(&n.as_str()));
        chosen.expect("a file the patch leaves").clone()
    }
}

/// Whether cargo-deny can run here, offline: installed, with an advisory database fetched.
/// Without either, the clean case cannot finish its advisory run.
fn advisories_can_run() -> Option<bool> {
    let installed = Command::new(cargo())
        .args(["deny", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !installed {
        return Some(false);
    }
    let home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")));
    let db = home.map(|h| h.join("advisory-dbs"));
    let fetched = db.and_then(|db| fs::read_dir(db).ok()).is_some_and(|mut e| e.next().is_some());
    fetched.then_some(true)
}

#[test]
fn a_patched_copy_passes_all_ten_steps() {
    let fixture = Fixture::patched("clean");
    let output = fixture.vendor();
    let stderr = String::from_utf8_lossy(&output.stderr);
    match advisories_can_run() {
        None => {
            eprintln!("cargo-deny is installed without an advisory database; not asserting");
            return;
        }
        Some(true) => assert!(stderr.contains("the advisory check ran"), "{}", text(&output)),
        Some(false) => assert!(
            stderr.contains("advisory check of the upstream versions was skipped"),
            "{}",
            text(&output)
        ),
    }
    assert!(output.status.success(), "{}", text(&output));
    assert!(output.stdout.is_empty(), "{}", text(&output));
    assert!(stderr.contains("1 record(s) re-derived"), "{}", text(&output));
    assert!(!stderr.contains("nothing is vendored"), "{}", text(&output));
}

#[test]
fn nothing_vendored_still_checks_the_gitattributes_line() {
    let fixture = Fixture::new("nothing");
    fs::remove_dir_all(fixture.ws.join("vendor")).expect("vendor/ removed");
    let root = read(&fixture.ws.join("Cargo.toml"));
    let root = root.split("[patch.crates-io]").next().expect("a manifest").to_owned();
    write(&fixture.ws.join("Cargo.toml"), &root);
    fixture.lock();
    let output = fixture.vendor();
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("vendor: nothing is vendored"), "{}", text(&output));
    fs::remove_file(fixture.ws.join(".gitattributes")).expect(".gitattributes removed");
    fixture.findings(&[".gitattributes: does not mark vendor/** -text".to_owned()]);
}

#[test]
fn an_edit_with_its_hash_rewritten_in_the_record_is_caught_online() {
    let fixture = Fixture::patched("rewritten");
    let file = fixture.untouched_file();
    let path = fixture.copy().join(&file);
    let edited = format!("{}tampered\n", read(&path));
    write(&path, &edited);
    let record_path = fixture.copy().join(".cargo-checksum.json");
    let mut record: serde_json::Value =
        serde_json::from_str(&read(&record_path)).expect("a record");
    let hash: String =
        Sha256::digest(edited.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    record["files"][&file] = serde_json::Value::from(hash);
    write(&record_path, &record.to_string());
    fixture.findings(&[format!(
        "vendor/{}/.cargo-checksum.json: does not match the crates.io archive ({file})",
        fixture.dir()
    )]);
}

#[test]
fn an_edit_outside_the_patch_differs_from_the_record() {
    let fixture = Fixture::patched("edited");
    let file = fixture.untouched_file();
    let path = fixture.copy().join(&file);
    write(&path, &format!("{}tampered\n", read(&path)));
    fixture.findings(&[format!("vendor/{}/{file}: differs from the record", fixture.dir())]);
}

#[test]
fn a_patch_that_does_not_match_the_copy_does_not_reverse_apply() {
    let fixture = Fixture::patched("stale");
    let lib = fixture.copy().join("src/lib.rs");
    write(&lib, &read(&lib).replace("pub const PATCHED: () = ();", "pub const PATCHED: u8 = 1;"));
    let output = fixture.vendor();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let prefix = format!("vendor/{0}.patch: does not reverse-apply to vendor/{0} (", fixture.dir());
    assert!(stdout.lines().count() == 1 && stdout.starts_with(&prefix), "{}", text(&output));
    assert_eq!(output.status.code(), Some(FINDINGS), "{}", text(&output));
}

#[test]
fn a_patch_in_a_cargo_configuration_file_is_a_finding() {
    let fixture = Fixture::patched("config");
    let root = read(&fixture.ws.join("Cargo.toml"));
    let (manifest, patch) = root.split_once("[patch.crates-io]").expect("a patch table");
    write(&fixture.ws.join("Cargo.toml"), manifest);
    write(&fixture.ws.join(".cargo/config.toml"), &format!("[patch.crates-io]{patch}"));
    fixture.lock();
    fixture.findings(&[
        ".cargo/config.toml: sets [patch], [source] or paths; a patch lives in the root \
         Cargo.toml only"
            .to_owned(),
        format!("vendor/{}: no [patch.crates-io] entry names it", fixture.dir()),
    ]);
}

#[test]
fn a_path_package_outside_vendor_is_a_finding() {
    let fixture = Fixture::patched("outside");
    let root = read(&fixture.ws.join("Cargo.toml"));
    let root = root.replace("exclude = [\"vendor\"]", "exclude = [\"vendor\", \"other\"]");
    write(&fixture.ws.join("Cargo.toml"), &root);
    write(
        &fixture.ws.join("other/Cargo.toml"),
        "[package]\nname = \"helper\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
         publish = false\n",
    );
    write(&fixture.ws.join("other/src/lib.rs"), "");
    let app = fixture.ws.join("app/Cargo.toml");
    write(&app, &(read(&app) + "helper = { path = \"../other\" }\n"));
    fixture.lock();
    fixture.findings(&[
        "other/Cargo.toml: non-member path package helper 0.0.0 outside vendor/".to_owned()
    ]);
}

#[test]
fn a_crate_at_two_versions_with_one_patched_is_a_finding() {
    // The registry cache holds one version, so the second is made: the copy claims 0.1.99,
    // which a `0.1` requirement takes from the patch while `=<version>` takes the registry.
    let fixture = Fixture::new("two");
    let version = fixture.version.clone();
    let manifest = read(&fixture.copy().join("Cargo.toml"));
    let claimed = manifest.replacen(
        &format!("version = \"{version}\""),
        "version = \"0.1.99\" # LACTOSE patch: a second version for the test.",
        1,
    );
    assert_ne!(manifest, claimed);
    fixture.patch(&[("Cargo.toml", claimed)]);
    let ws = &fixture.ws;
    fs::rename(fixture.copy(), ws.join("vendor/cfg-if-0.1.99")).expect("renamed");
    let patch = ws.join(format!("vendor/{}.patch", fixture.dir()));
    fs::rename(&patch, ws.join("vendor/cfg-if-0.1.99.patch")).expect("renamed");
    let root = read(&ws.join("Cargo.toml"));
    let root = root.replace(&format!("vendor/{}", fixture.dir()), "vendor/cfg-if-0.1.99");
    write(&ws.join("Cargo.toml"), &root);
    let readme = read(&ws.join("vendor/README.md"));
    write(&ws.join("vendor/README.md"), &readme.replace(&version, "0.1.99"));
    write(
        &ws.join("app/Cargo.toml"),
        &read(&ws.join("app/Cargo.toml"))
            .replace("cfg-if.workspace = true", &format!("cfg-if = \"={version}\"")),
    );
    let root =
        read(&ws.join("Cargo.toml")).replace("members = [\"app\"]", "members = [\"app\", \"old\"]");
    write(&ws.join("Cargo.toml"), &root);
    write(
        &ws.join("old/Cargo.toml"),
        "[package]\nname = \"old\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
         [dependencies]\ncfg-if = \"0.1\"\n",
    );
    write(&ws.join("old/src/lib.rs"), "");
    fixture.lock();
    fixture.findings(&[
        format!(
            "vendor/cfg-if-0.1.99: cfg-if also resolves from crates.io at {version}; a patch \
             covers one version"
        ),
        "vendor/cfg-if-0.1.99.patch: names Cargo.toml, which a patch never touches".to_owned(),
    ]);
}

#[test]
fn a_patch_over_fifty_lines_is_a_finding() {
    let fixture = Fixture::new("over");
    let body: String = (0..50).map(|i| format!("pub const C{i}: () = ();\n")).collect();
    let change = fixture.lib_with(&format!("// LACTOSE patch: too long.\n{body}"));
    fixture.patch(&[change]);
    fixture.findings(&[format!(
        "vendor/{}.patch: 51 changed lines, over the fifty of architecture section 8.2",
        fixture.dir()
    )]);
}

#[test]
fn a_patch_to_the_manifest_is_a_finding() {
    let fixture = Fixture::new("manifest");
    let lib = fixture.lib_with("// LACTOSE patch: the change.\npub const PATCHED: () = ();\n");
    let manifest = read(&fixture.copy().join("Cargo.toml")) + "# LACTOSE patch: no.\n";
    fixture.patch(&[lib, ("Cargo.toml", manifest)]);
    fixture.findings(&[format!(
        "vendor/{}.patch: names Cargo.toml, which a patch never touches",
        fixture.dir()
    )]);
}

#[test]
fn a_patch_adding_unsafe_is_a_finding() {
    let fixture = Fixture::new("unsafe");
    let change = fixture.lib_with("// LACTOSE patch: no.\npub fn f() { unsafe {} }\n");
    fixture.patch(&[change]);
    // Line 9: four header lines, three of context, the notice, then the `unsafe`.
    fixture.findings(&[format!("vendor/{}.patch: adds unsafe at line 9", fixture.dir())]);
}

#[test]
fn a_patch_without_the_notice_is_a_finding() {
    let fixture = Fixture::new("notice");
    let change = fixture.lib_with("pub const PATCHED: () = ();\n");
    fixture.patch(&[change]);
    fixture.findings(&[format!(
        "vendor/{}.patch: src/lib.rs carries no \"LACTOSE patch:\" notice",
        fixture.dir()
    )]);
}
