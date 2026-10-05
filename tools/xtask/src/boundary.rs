//! The static boundary checks: the facts about files and manifests that architecture
//! sections 8.2, 8.3, 20 and 21 require and that need no compiler to verify.
//!
//! What clippy enforces through `clippy.toml` is not repeated here. This module checks that
//! the configuration clippy reads is the agreed one, and that none of the ways of switching
//! it off is in use: a lint attribute on a whole crate or module, another configuration
//! file, a `[lints]` table, or a flag in a workflow or script.

use std::collections::BTreeSet;

use crate::policy::Policy;
use crate::repo::{Files, is_build_configuration, is_cargo_config};
use crate::rust_source::attributes;
use crate::tiers::TierFile;
use crate::workspace::{DepKind, DepSource, Package, Workspace};
use crate::{Finding, TIER_FILE};

/// The boundary lint configuration every boundary crate copies byte for byte.
pub const REFERENCE_CLIPPY: &str = "crates/core/clippy.toml";

/// The cargo configuration file that carries the x86-64 pin.
pub const CARGO_CONFIG: &str = ".cargo/config.toml";

/// The workspace manifest.
const ROOT_MANIFEST: &str = "Cargo.toml";

/// The `[target]` key and the flags of the pin (architecture section 8.2).
const PIN_KEY: &str = r#"cfg(target_arch = "x86_64")"#;
const PIN_CPU: &str = "x86-64-v3";
const PIN_FLAGS: [&str; 2] = ["-C", "target-cpu=x86-64-v3"];

/// Entries the reference lint file must keep, one or two from each family of rule in
/// section 8.2, so a file that was emptied and then copied everywhere is still caught.
const REQUIRED_LINT_PATHS: [&str; 8] = [
    "f32::min",
    "f64::sin",
    "f64::to_bits",
    "f64::powi",
    "std::time::Instant",
    "std::collections::HashMap",
    "std::hash::RandomState",
    "std::thread::LocalKey",
];

/// Lint levels that leave a lint weaker than `deny`.
const WEAK_LEVELS: [&str; 3] = ["allow", "expect", "warn"];

/// Whether `lint` is one of the lints that carry the boundary rules, or a group that contains
/// them. `clippy::disallowed_methods` and `clippy::disallowed_types` are in `clippy::style`
/// and `clippy::all`, and CI turns every warning into an error through `warnings`.
fn is_boundary_lint(lint: &str) -> bool {
    matches!(
        lint.replace('-', "_").as_str(),
        "clippy::disallowed_methods"
            | "clippy::disallowed_types"
            | "clippy::all"
            | "clippy::style"
            | "warnings"
    )
}

/// Runs every static boundary check.
#[must_use]
pub fn check(tiers: &TierFile, policy: &Policy, ws: &Workspace, files: &Files) -> Vec<Finding> {
    let mut findings = Vec::new();
    reference_lint_file(files, &mut findings);
    let mut stray_configs = BTreeSet::new();
    for package in &ws.packages {
        readme(package, files, &mut findings);
        unsafe_policy(policy, package, files, &mut findings);
        let boundary = tiers.in_boundary(&package.name);
        if boundary || policy.lint_file_carriers.contains(&package.name) {
            lint_file_copy(package, files, &mut findings);
            stray_lint_files(package, files, &mut stray_configs);
            manifest_lints(package, files, &mut findings);
        }
        if boundary {
            lint_attributes(package, files, &mut findings);
            dependency_allowlist(tiers, ws, package, &mut findings);
        }
    }
    findings.extend(stray_configs.into_iter().map(|path| {
        Finding::new(
            path,
            "clippy reads `.clippy.toml` in preference to `clippy.toml`, so this file replaces \
             the boundary lint configuration of every crate at or below it",
        )
    }));
    workspace_lints(files, &mut findings);
    build_configuration(files, &mut findings);
    cargo_configs(files, &mut findings);
    findings
}

/// Architecture section 20: each crate's `README.md` states responsibility, tier, boundary
/// status and unsafe status. Only its presence is checked.
fn readme(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let path = format!("{}/README.md", package.dir);
    if !files.contains(&path) {
        findings.push(Finding::new(path, format!("`{}` has no README.md", package.name)));
    }
}

/// Architecture section 21: `unsafe` is confined to the listed crates, each of which carries
/// `SAFETY.md`. Every other crate forbids it in every crate root it has, with an inner
/// attribute ahead of the first item: one further down applies to a nested module only.
fn unsafe_policy(policy: &Policy, package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    if policy.unsafe_crates.contains(&package.name) {
        let path = format!("{}/SAFETY.md", package.dir);
        if !files.contains(&path) {
            findings.push(Finding::new(
                path,
                format!(
                    "`{}` is on the unsafe list (architecture section 21) and has no SAFETY.md",
                    package.name
                ),
            ));
        }
        return;
    }
    for root in &package.crate_roots {
        let forbids = files.text(root).is_some_and(|source| {
            attributes(&source).iter().any(|a| a.leading && forbids_unsafe(&a.body))
        });
        if !forbids {
            findings.push(Finding::new(
                root,
                format!(
                    "`{}` is not on the unsafe list (architecture section 21), so this crate \
                     root needs `#![forbid(unsafe_code)]` before its first item",
                    package.name
                ),
            ));
        }
    }
}

/// Whether an attribute body is `forbid(unsafe_code)`, alone or among other lints. A
/// `cfg_attr` does not count: it forbids only sometimes.
fn forbids_unsafe(body: &str) -> bool {
    body.strip_prefix("forbid(")
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(|lints| lints.split(',').any(|lint| lint == "unsafe_code"))
}

/// The reference lint file exists and still bans what section 8.2 bans.
fn reference_lint_file(files: &Files, findings: &mut Vec<Finding>) {
    let Some(text) = files.text(REFERENCE_CLIPPY) else {
        findings.push(Finding::new(
            REFERENCE_CLIPPY,
            "the reference boundary lint configuration is missing",
        ));
        return;
    };
    let config = match toml::from_str::<toml::Table>(&text) {
        Ok(config) => config,
        Err(e) => {
            findings.push(Finding::new(REFERENCE_CLIPPY, format!("not valid TOML: {e}")));
            return;
        }
    };
    let banned: BTreeSet<&str> = ["disallowed-methods", "disallowed-types"]
        .iter()
        .filter_map(|key| config.get(*key).and_then(toml::Value::as_array))
        .flatten()
        .filter_map(|entry| entry.as_str().or_else(|| entry.get("path")?.as_str()))
        .collect();
    for required in REQUIRED_LINT_PATHS {
        if !banned.contains(required) {
            findings.push(Finding::new(
                REFERENCE_CLIPPY,
                format!(
                    "no longer disallows `{required}`, which architecture section 8.2 bans \
                     inside the boundary"
                ),
            ));
        }
    }
}

/// Architecture section 8.3: one `clippy.toml` per boundary crate, all identical.
fn lint_file_copy(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let path = format!("{}/clippy.toml", package.dir);
    let Some(reference) = files.bytes(REFERENCE_CLIPPY) else {
        return;
    };
    match files.bytes(&path) {
        None => findings.push(Finding::new(
            path,
            format!(
                "`{}` must carry the boundary lint file; copy {REFERENCE_CLIPPY}",
                package.name
            ),
        )),
        Some(copy) if copy != reference => findings.push(Finding::new(
            path,
            format!("differs from {REFERENCE_CLIPPY}; the two must be byte-identical"),
        )),
        Some(_) => {}
    }
}

/// Collects every `.clippy.toml` in the package directory or a directory above it.
fn stray_lint_files(package: &Package, files: &Files, found: &mut BTreeSet<String>) {
    let mut dir = Some(package.dir.as_str());
    while let Some(at) = dir {
        let path =
            if at.is_empty() { ".clippy.toml".to_owned() } else { format!("{at}/.clippy.toml") };
        if files.contains(&path) {
            found.insert(path);
        }
        dir = match at.rsplit_once('/') {
            Some((parent, _)) => Some(parent),
            None if at.is_empty() => None,
            None => Some(""),
        };
    }
}

/// The Rust sources of a package: every `.rs` file under its directory that is not inside a
/// nested package.
fn package_sources<'a>(package: &Package, files: &'a Files) -> Vec<&'a str> {
    let prefix = format!("{}/", package.dir);
    files
        .paths()
        .filter(|path| path.ends_with(".rs"))
        .filter(|path| {
            let Some(inside) = path.strip_prefix(&prefix) else {
                return false;
            };
            // Any directory between the package and the file that has a manifest of its own.
            !inside
                .match_indices('/')
                .any(|(at, _)| files.contains(&format!("{prefix}{}/Cargo.toml", &inside[..at])))
        })
        .collect()
}

/// The boundary lints an attribute body weakens, each as `level(lint)`.
fn weakened_lints(body: &str) -> Vec<String> {
    let mut found = Vec::new();
    for level in WEAK_LEVELS {
        let open = format!("{level}(");
        for (at, _) in body.match_indices(&open) {
            let standalone = at == 0
                || !body.as_bytes()[at - 1].is_ascii_alphanumeric()
                    && body.as_bytes()[at - 1] != b'_';
            if !standalone {
                continue;
            }
            // The arguments run to the matching parenthesis.
            let arguments = &body[at + open.len()..];
            let mut depth = 1usize;
            let end = arguments.find(|c: char| {
                depth = match c {
                    '(' => depth + 1,
                    ')' => depth - 1,
                    _ => depth,
                };
                depth == 0
            });
            for lint in arguments[..end.unwrap_or(arguments.len())].split(',') {
                if is_boundary_lint(lint) {
                    found.push(format!("{level}({lint})"));
                }
            }
        }
    }
    found
}

/// A boundary crate lifts a boundary lint with an `#[expect]` on the one item that needs it.
/// An inner attribute, or an outer attribute on a `mod`, weakens it for a whole crate or
/// module, which is what this rejects, in every source file of the package.
fn lint_attributes(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    for path in package_sources(package, files) {
        let Some(source) = files.text(path) else {
            continue;
        };
        for attribute in attributes(&source).iter().filter(|a| a.inner || a.on_module) {
            for weakened in weakened_lints(&attribute.body) {
                let scope = if attribute.inner { "a whole crate or module" } else { "a module" };
                findings.push(Finding::new(
                    path,
                    format!(
                        "line {}: `{weakened}` applies to {scope} of boundary crate `{}`; the \
                         only sanctioned form is `#[expect(clippy::disallowed_methods, reason = \
                         \"...\")]` (or `disallowed_types`) on the one item that needs it",
                        attribute.line, package.name
                    ),
                ));
            }
        }
    }
}

/// The level of each boundary lint that a `[lints]`-shaped table sets, as (lint, level).
fn lint_table_levels(table: Option<&toml::Value>) -> Vec<(String, String)> {
    let mut levels = Vec::new();
    for (tool, prefix) in [("rust", ""), ("clippy", "clippy::")] {
        let Some(lints) = table.and_then(|t| t.get(tool)).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, value) in lints {
            let lint = format!("{prefix}{name}");
            let level = value.as_str().or_else(|| value.get("level")?.as_str());
            if let Some(level) = level.filter(|_| is_boundary_lint(&lint)) {
                levels.push((lint, level.to_owned()));
            }
        }
    }
    levels
}

fn level_rank(level: &str) -> u8 {
    match level {
        "allow" => 0,
        "warn" => 1,
        "deny" => 2,
        _ => 3,
    }
}

fn manifest_table(files: &Files, path: &str) -> Option<toml::Table> {
    toml::from_str(&files.text(path)?).ok()
}

/// `[workspace.lints]` may not set a boundary lint below `warn`, the level at which CI's
/// `-D warnings` still fails the build.
fn workspace_lints(files: &Files, findings: &mut Vec<Finding>) {
    let Some(manifest) = manifest_table(files, ROOT_MANIFEST) else {
        return;
    };
    let table = manifest.get("workspace").and_then(|w| w.get("lints"));
    for (lint, level) in lint_table_levels(table) {
        if level_rank(&level) < level_rank("warn") {
            findings.push(Finding::new(
                ROOT_MANIFEST,
                format!(
                    "[workspace.lints] sets `{lint}` to `{level}`, which switches the boundary \
                     lints off for every crate that inherits it"
                ),
            ));
        }
    }
}

/// A crate that carries the boundary lint file may not set a boundary lint in its own
/// `[lints]` table below the workspace's level (`warn` where the workspace says nothing).
fn manifest_lints(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let workspace = manifest_table(files, ROOT_MANIFEST);
    let workspace_levels =
        lint_table_levels(workspace.as_ref().and_then(|m| m.get("workspace")?.get("lints")));
    let Some(manifest) = manifest_table(files, &package.manifest()) else {
        return;
    };
    for (lint, level) in lint_table_levels(manifest.get("lints")) {
        let floor = workspace_levels
            .iter()
            .find(|(name, _)| *name == lint)
            .map_or(1, |(_, level)| level_rank(level).max(1));
        if level_rank(&level) < floor {
            findings.push(Finding::new(
                package.manifest(),
                format!(
                    "[lints] sets `{lint}` to `{level}`, weaker than the workspace default; \
                     `{}` carries the boundary lints",
                    package.name
                ),
            ));
        }
    }
}

/// Architecture section 8.2: the edges out of the boundary are the ones `crate-tiers.toml`
/// names, directly and at any depth.
fn dependency_allowlist(
    tiers: &TierFile,
    ws: &Workspace,
    package: &Package,
    findings: &mut Vec<Finding>,
) {
    let internal = &tiers.boundary_internal;
    let third_party = &tiers.boundary_third_party;
    let direct = third_party.direct.get(&package.name);
    for dep in &package.deps {
        // A name the tier file lists is a workspace crate wherever it is fetched from.
        let is_internal = dep.source == DepSource::Member || tiers.tier(&dep.name).is_some();
        let allowed = if is_internal {
            // The tier check owns workspace dev-dependencies.
            dep.kind == DepKind::Dev
                || tiers.in_boundary(&dep.name)
                || internal.any.contains(&dep.name)
                || internal.only.get(&package.name).is_some_and(|only| only.contains(&dep.name))
        } else {
            direct.is_some_and(|list| {
                list.normal.contains(&dep.name)
                    || (dep.kind == DepKind::Dev && list.dev.contains(&dep.name))
            })
        };
        if allowed {
            continue;
        }
        let (what, table) = if is_internal {
            ("non-boundary crate", "[boundary_internal]".to_owned())
        } else {
            ("third-party crate", format!("[boundary_third_party.direct] \"{}\"", package.name))
        };
        let section = match dep.kind {
            DepKind::Normal => "depends",
            DepKind::Build => "build-depends",
            DepKind::Dev => "dev-depends",
        };
        findings.push(Finding::new(
            package.manifest(),
            format!(
                "boundary crate `{}` {section} on {what} `{}`, which {table} in {TIER_FILE} \
                 does not allow",
                package.name, dep.name
            ),
        ));
    }

    // Everything the crate ends up compiling. Its own direct dependencies were judged above.
    let named: BTreeSet<&str> =
        package.deps.iter().filter(|d| d.kind.ships()).map(|d| d.name.as_str()).collect();
    for (name, chain) in ws.third_party_closure(&package.name) {
        if !named.contains(name.as_str()) && !third_party.vetted(&name) {
            findings.push(Finding::new(
                package.manifest(),
                format!(
                    "boundary crate `{}` compiles third-party crate `{name}` ({}), which is on \
                     no `normal` list and not in `transitive` of [boundary_third_party] in \
                     {TIER_FILE}",
                    package.name,
                    chain.join(" -> ")
                ),
            ));
        }
    }
}

/// The lint named by each `-A`/`--allow` flag on the line.
fn allowed_by_flag(line: &str) -> Vec<&str> {
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '[' | ']'))
        .filter(|word| !word.is_empty())
        .collect();
    let mut lints = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let lint = if *word == "-A" || *word == "--allow" {
            words.get(index + 1).copied()
        } else {
            word.strip_prefix("--allow=").or_else(|| word.strip_prefix("-A"))
        };
        lints.extend(lint.filter(|lint| {
            is_boundary_lint(lint) || lint.replace('-', "_").starts_with("clippy::disallowed")
        }));
    }
    lints
}

/// Architecture section 8.2, for every file that can decide how cargo and clippy are invoked
/// (cargo configuration, workflows, scripts): nothing sets rust flags through the environment,
/// selects a CPU or a target feature, points clippy at another configuration, or allows a
/// boundary lint from the command line.
///
/// rustc reads `-C target_cpu` and `-C target-cpu` alike, so underscores are folded before
/// matching. Only whole-line `#` comments are skipped; a trailing comment is scanned as if it
/// were code, which errs towards a finding.
fn build_configuration(files: &Files, findings: &mut Vec<Finding>) {
    for path in files.paths().filter(|path| is_build_configuration(path)) {
        let Some(text) = files.text(path) else {
            continue;
        };
        let cargo_config = is_cargo_config(path);
        for (index, line) in text.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let mut report = |message: String| {
                findings.push(Finding::new(path, format!("line {}: {message}", index + 1)));
            };
            // In a cargo configuration file the `rustflags` key is judged by structure, in
            // `cargo_configs`; anywhere else the word in any spelling is a way to set them
            // (`--config build.rustflags=...`, the environment variables).
            let names_flags = if cargo_config {
                line.contains("RUSTFLAGS")
            } else {
                line.to_ascii_uppercase().replace('-', "_").contains("RUSTFLAGS")
            };
            if names_flags {
                report(format!(
                    "names RUSTFLAGS; rust flags from the environment or the command line \
                     replace the {PIN_CPU} pin in {CARGO_CONFIG} instead of adding to it"
                ));
            }
            let folded = line.replace('_', "-");
            for (at, _) in folded.match_indices("target-cpu") {
                let value = folded[at + "target-cpu".len()..]
                    .strip_prefix('=')
                    .map(|rest| {
                        let end = rest
                            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.')))
                            .unwrap_or(rest.len());
                        &rest[..end]
                    })
                    .unwrap_or_default();
                if !(cargo_config && value == PIN_CPU) {
                    report(format!(
                        "selects a target CPU (`target-cpu` `{value}`); the only permitted \
                         setting is the {PIN_CPU} pin in {CARGO_CONFIG}"
                    ));
                }
            }
            if folded.contains("target-feature") {
                report(
                    "sets `target-feature`; target features come from the pinned target CPU only"
                        .to_owned(),
                );
            }
            if line.contains("CLIPPY_CONF_DIR") {
                report(
                    "names CLIPPY_CONF_DIR, which points clippy away from each crate's \
                     clippy.toml"
                        .to_owned(),
                );
            }
            for lint in allowed_by_flag(line) {
                report(format!(
                    "allows `{lint}` from the command line, which switches the boundary lints off"
                ));
            }
        }
    }
}

/// Every `rustflags` key of a TOML value, as a dotted path with its value.
fn rustflags_keys<'a>(
    value: &'a toml::Value,
    path: &mut Vec<&'a str>,
    found: &mut Vec<(String, &'a toml::Value)>,
) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        path.push(key);
        if key == "rustflags" {
            found.push((path.join("."), child));
        } else {
            rustflags_keys(child, path, found);
        }
        path.pop();
    }
}

/// Cargo configuration files set `rustflags` in exactly one place: the x86-64-v3 pin in
/// `.cargo/config.toml`. Any other `rustflags` key, in any table of any cargo configuration
/// file, is a finding, because cargo joins or replaces flags by rules nobody should have to
/// re-derive to know what the boundary was compiled with.
fn cargo_configs(files: &Files, findings: &mut Vec<Finding>) {
    let pin_path = format!("target.{PIN_KEY}.rustflags");
    let mut pinned = false;
    for path in files.paths().filter(|path| is_cargo_config(path)) {
        let Some(text) = files.text(path) else {
            continue;
        };
        let config = match toml::from_str::<toml::Value>(&text) {
            Ok(config) => config,
            Err(e) => {
                findings.push(Finding::new(path, format!("not valid TOML: {e}")));
                continue;
            }
        };
        let mut found = Vec::new();
        rustflags_keys(&config, &mut Vec::new(), &mut found);
        for (key, value) in found {
            let flags: Option<Vec<&str>> = value
                .as_array()
                .map(|items| items.iter().filter_map(toml::Value::as_str).collect());
            if path == CARGO_CONFIG && key == pin_path && flags.as_deref() == Some(&PIN_FLAGS[..]) {
                pinned = true;
            } else {
                findings.push(Finding::new(
                    path,
                    format!(
                        "sets `{key}`; the only rust flags permitted in a cargo configuration \
                         file are the pin, [target.'{PIN_KEY}'] rustflags = {PIN_FLAGS:?} in \
                         {CARGO_CONFIG}"
                    ),
                ));
            }
        }
    }
    if !pinned {
        findings.push(Finding::new(
            CARGO_CONFIG,
            format!(
                "the x86-64 pin is missing: [target.'{PIN_KEY}'] must set rustflags = \
                 {PIN_FLAGS:?}"
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiers::fixture::FILE as TIERS;
    use crate::workspace::fixture::{package, workspace, workspace_with};
    use DepKind::{Build, Dev, Normal};

    const PIN: &str = "[target.'cfg(target_arch = \"x86_64\")']\nrustflags = [\"-C\", \"target-cpu=x86-64-v3\"]\n";
    const FORBID: &str = "//! Docs.\n#![forbid(unsafe_code)]\n";
    const MANIFEST: &str = "[package]\nname = \"x\"\n[lints]\nworkspace = true\n";

    fn lint_file() -> String {
        let mut text = String::from("disallowed-methods = [\n");
        for path in &REQUIRED_LINT_PATHS[..4] {
            text.push_str(&format!("  {{ path = \"{path}\", reason = \"r\" }},\n"));
        }
        text.push_str("]\ndisallowed-types = [\n");
        for path in &REQUIRED_LINT_PATHS[4..] {
            text.push_str(&format!("  \"{path}\",\n"));
        }
        text.push_str("]\n");
        text
    }

    fn policy() -> Policy {
        Policy::parse(include_str!("../ci-policy.toml")).unwrap()
    }

    /// A conforming repository: `core` (unsafe, boundary), `math` (boundary), `task` (unsafe).
    fn conforming() -> (Workspace, Files) {
        let ws = workspace_with(
            vec![
                package("lactose-core", &[("bytemuck", Normal), ("xxhash-rust", Dev)]),
                package("lactose-math", &[("lactose-core", Normal), ("libm", Normal)]),
                package("lactose-task", &[("lactose-core", Normal), ("crossbeam-deque", Normal)]),
            ],
            &[("bytemuck", &["bytemuck_derive"]), ("crossbeam-deque", &["crossbeam-epoch"])],
        );
        let mut files = Files::default();
        files.insert(CARGO_CONFIG, PIN);
        files.insert(
            ROOT_MANIFEST,
            "[workspace]\n[workspace.lints.rust]\nmissing_docs = \"warn\"\n",
        );
        files.insert(REFERENCE_CLIPPY, lint_file());
        files.insert("crates/math/clippy.toml", lint_file());
        for role in ["core", "math", "task"] {
            files.insert(format!("crates/{role}/README.md"), "# readme\n");
            files.insert(format!("crates/{role}/Cargo.toml"), MANIFEST);
        }
        files.insert("crates/core/SAFETY.md", "# safety\n");
        files.insert("crates/task/SAFETY.md", "# safety\n");
        files.insert("crates/core/src/lib.rs", "pub fn f() {}\n");
        files.insert("crates/task/src/lib.rs", "pub fn f() {}\n");
        files.insert("crates/math/src/lib.rs", FORBID);
        (ws, files)
    }

    fn without(files: &Files, removed: &[&str]) -> Files {
        let mut kept = Files::default();
        for path in files.paths().filter(|path| !removed.contains(path)) {
            kept.insert(path, files.bytes(path).unwrap());
        }
        kept
    }

    fn run(ws: &Workspace, files: &Files) -> Vec<String> {
        let tiers = TierFile::parse(TIERS).unwrap();
        check(&tiers, &policy(), ws, files).iter().map(ToString::to_string).collect()
    }

    const NONE: [&str; 0] = [];

    #[test]
    fn a_conforming_repository_has_no_findings() {
        let (ws, files) = conforming();
        assert_eq!(run(&ws, &files), NONE);
    }

    #[test]
    fn missing_documents_are_reported() {
        let (ws, files) = conforming();
        let files = without(&files, &["crates/task/SAFETY.md", "crates/math/README.md"]);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/README.md: `lactose-math` has no README.md",
                "crates/task/SAFETY.md: `lactose-task` is on the unsafe list (architecture \
                 section 21) and has no SAFETY.md",
            ]
        );
    }

    #[test]
    fn a_crate_off_the_unsafe_list_must_forbid_unsafe_before_its_first_item() {
        let (ws, mut files) = conforming();
        for source in [
            "pub fn f() {}\n",
            "// #![forbid(unsafe_code)]\n",
            "const S: &str = \"#![forbid(unsafe_code)]\";\n",
            "#![deny(unsafe_code)]\n",
            "#![cfg_attr(not(test), forbid(unsafe_code))]\n",
            "#[forbid(unsafe_code)]\nmod m {}\n",
            // Review item 9: the attribute inside a nested module forbids nothing outside it.
            "mod quarantine {\n    #![forbid(unsafe_code)]\n}\npub unsafe fn f() {}\n",
            "pub unsafe fn f() {}\nmod m { #![forbid(unsafe_code)] }\n",
        ] {
            files.insert("crates/math/src/lib.rs", source);
            let found = run(&ws, &files);
            assert_eq!(found.len(), 1, "{source:?} gave {found:?}");
            assert!(
                found[0].starts_with(
                    "crates/math/src/lib.rs: `lactose-math` is not on the unsafe list"
                )
            );
        }
        for source in [
            "#![forbid(unsafe_code)]\n",
            "#! [ forbid( unsafe_code ) ]\n",
            "#![forbid(missing_docs, unsafe_code)]\n",
            "//! Docs.\n#![allow(dead_code, reason = \"a ] bracket\")]\n#![forbid(unsafe_code)]\nfn f() {}\n",
        ] {
            files.insert("crates/math/src/lib.rs", source);
            assert_eq!(run(&ws, &files), NONE, "{source:?}");
        }
    }

    #[test]
    fn every_crate_root_is_checked_tests_benches_examples_and_build_script_included() {
        // Review item 16.
        let (mut ws, mut files) = conforming();
        let roots = ["build.rs", "benches/b.rs", "examples/e.rs", "src/bin/tool.rs", "tests/t.rs"];
        for root in roots {
            ws.packages[1].crate_roots.push(format!("crates/math/{root}"));
            files.insert(format!("crates/math/{root}"), "fn main() {}\n");
        }
        let found = run(&ws, &files);
        assert_eq!(found.len(), roots.len(), "{found:#?}");
        for root in roots {
            assert!(
                found.iter().any(|f| f.starts_with(&format!("crates/math/{root}: "))),
                "{root}"
            );
            files.insert(format!("crates/math/{root}"), FORBID);
        }
        assert_eq!(run(&ws, &files), NONE);
    }

    #[test]
    fn the_boundary_lint_file_must_be_an_exact_copy() {
        let (ws, mut files) = conforming();
        files.insert("crates/math/clippy.toml", format!("{}\n", lint_file()));
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/clippy.toml: differs from crates/core/clippy.toml; the two must be byte-identical"
            ]
        );
        let files = without(&files, &["crates/math/clippy.toml"]);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/clippy.toml: `lactose-math` must carry the boundary lint file; copy crates/core/clippy.toml"
            ]
        );
    }

    #[test]
    fn a_proc_macro_crate_on_the_carrier_list_carries_the_lint_file_too() {
        // Review item 13.
        let (mut ws, mut files) = conforming();
        ws.packages.push(package("lactose-reflect-macros", &[("syn", Normal)]));
        files.insert("crates/reflect-macros/README.md", "");
        files.insert("crates/reflect-macros/Cargo.toml", MANIFEST);
        files.insert("crates/reflect-macros/src/lib.rs", FORBID);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/reflect-macros/clippy.toml: `lactose-reflect-macros` must carry the boundary lint file; copy crates/core/clippy.toml"
            ]
        );
        files.insert("crates/reflect-macros/clippy.toml", lint_file());
        assert_eq!(run(&ws, &files), NONE, "and its dependencies are not held to the allowlist");
    }

    #[test]
    fn a_weakened_reference_lint_file_is_caught_even_when_every_copy_matches() {
        // Review item 19.
        let (ws, mut files) = conforming();
        let weakened = lint_file()
            .replace("  \"std::time::Instant\",\n", "")
            .replace("  { path = \"f64::sin\", reason = \"r\" },\n", "");
        files.insert(REFERENCE_CLIPPY, weakened.clone());
        files.insert("crates/math/clippy.toml", weakened);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/core/clippy.toml: no longer disallows `f64::sin`, which architecture section 8.2 bans inside the boundary",
                "crates/core/clippy.toml: no longer disallows `std::time::Instant`, which architecture section 8.2 bans inside the boundary",
            ]
        );
    }

    #[test]
    fn the_real_reference_lint_file_has_the_required_entries() {
        let mut files = Files::default();
        files.insert(REFERENCE_CLIPPY, include_str!("../../../crates/core/clippy.toml"));
        let mut findings = Vec::new();
        reference_lint_file(&files, &mut findings);
        assert_eq!(findings, []);
    }

    #[test]
    fn a_boundary_lint_may_not_be_weakened_for_a_crate_or_a_module() {
        // Review item 1, attributes.
        let (ws, files) = conforming();
        let reported = [
            "#![allow(clippy::disallowed_methods)]",
            "#![expect(clippy::disallowed_types, reason = \"x\")]",
            "#![warn(clippy::disallowed_methods)]",
            "#![allow(clippy::all)]",
            "#![allow(clippy::style)]",
            "#![allow(warnings)]",
            "#![allow(dead_code, warnings)]",
            "#![cfg_attr(not(test), allow(clippy::all))]",
            "mod inner {\n    #![allow(clippy::disallowed_methods)]\n}",
            "#[allow(clippy::disallowed_methods)]\nmod trig {}",
            "#[allow(clippy::all)]\n/// Docs.\n#[cfg(test)]\npub(crate) mod trig;",
            "#[expect(clippy::disallowed_types, reason = \"x\")]\npub mod maps {}",
        ];
        for attribute in reported {
            let mut files = files.clone();
            files.insert("crates/math/src/lib.rs", format!("{FORBID}{attribute}\n"));
            let found = run(&ws, &files);
            assert_eq!(found.len(), 1, "{attribute:?} gave {found:#?}");
            let line = if attribute.starts_with("mod inner") { 4 } else { 3 };
            let start = format!("crates/math/src/lib.rs: line {line}: `");
            assert!(found[0].starts_with(&start), "{}", found[0]);
        }
        let accepted = [
            // The sanctioned form, and attributes that weaken nothing.
            "#[expect(clippy::disallowed_methods, reason = \"x\")]\nfn f() {}",
            "#[expect(clippy::disallowed_types, reason = \"x\")]\nimpl S {}",
            "#![deny(warnings)]",
            "#![allow(dead_code, reason = \"not warnings, not clippy::all\")]",
            "#![allow(unused_warnings)]",
            "// #![allow(clippy::all)]",
        ];
        for attribute in accepted {
            let source = if attribute.starts_with("#!") || attribute.starts_with("//") {
                format!("{FORBID}{attribute}\n")
            } else {
                format!("{FORBID}struct S;\n{attribute}\n")
            };
            let mut files = files.clone();
            files.insert("crates/math/src/lib.rs", source);
            assert_eq!(run(&ws, &files), NONE, "{attribute:?}");
        }
        // Outside the boundary the same attribute is nobody's business here.
        let mut files = files.clone();
        files.insert("crates/task/src/lib.rs", "#![allow(clippy::all)]\n");
        assert_eq!(run(&ws, &files), NONE);
    }

    #[test]
    fn every_source_file_of_the_package_is_scanned_but_not_a_nested_package() {
        // Review item 10.
        let (ws, mut files) = conforming();
        let bad = "#![allow(clippy::disallowed_methods)]\n";
        for path in [
            "tests/t.rs",
            "benches/b.rs",
            "examples/e.rs",
            "build.rs",
            "src/a/b.rs",
            "generated/g.rs",
        ] {
            files.insert(format!("crates/math/{path}"), bad);
        }
        files.insert("crates/math/fuzz/Cargo.toml", "[package]\nname = \"fuzz\"\n");
        files.insert("crates/math/fuzz/src/main.rs", bad);
        let found = run(&ws, &files);
        assert_eq!(found.len(), 6, "{found:#?}");
        assert!(found.iter().all(|f| f.contains(": line 1: `allow(clippy::disallowed_methods)`")));
        assert!(!found.iter().any(|f| f.contains("fuzz")));
    }

    #[test]
    fn another_clippy_configuration_file_is_reported() {
        // Review item 1, `.clippy.toml`.
        let (ws, mut files) = conforming();
        files.insert(".clippy.toml", "");
        files.insert("crates/.clippy.toml", "");
        files.insert("crates/math/.clippy.toml", "");
        files.insert("crates/task/.clippy.toml", "");
        let found = run(&ws, &files);
        let paths: Vec<&str> = found.iter().map(|f| f.split(": ").next().unwrap()).collect();
        assert_eq!(paths, [".clippy.toml", "crates/.clippy.toml", "crates/math/.clippy.toml"]);
    }

    #[test]
    fn a_lints_table_may_not_weaken_a_boundary_lint() {
        // Review item 1, `[lints]`.
        let (ws, mut files) = conforming();
        files.insert(
            ROOT_MANIFEST,
            "[workspace.lints.clippy]\nall = { level = \"allow\", priority = -1 }\ndisallowed_types = \"deny\"\n[workspace.lints.rust]\nwarnings = \"allow\"\n",
        );
        files.insert(
            "crates/math/Cargo.toml",
            "[lints.clippy]\ndisallowed-methods = \"allow\"\ndisallowed_types = \"warn\"\nstyle = \"warn\"\n",
        );
        // Not a carrier of the boundary lints.
        files.insert("crates/task/Cargo.toml", "[lints.clippy]\nall = \"allow\"\n");
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/Cargo.toml: [lints] sets `clippy::disallowed-methods` to `allow`, weaker than the workspace default; `lactose-math` carries the boundary lints",
                "crates/math/Cargo.toml: [lints] sets `clippy::disallowed_types` to `warn`, weaker than the workspace default; `lactose-math` carries the boundary lints",
                "Cargo.toml: [workspace.lints] sets `warnings` to `allow`, which switches the boundary lints off for every crate that inherits it",
                "Cargo.toml: [workspace.lints] sets `clippy::all` to `allow`, which switches the boundary lints off for every crate that inherits it",
            ]
        );
    }

    #[test]
    fn boundary_dependencies_must_be_on_the_allowlist() {
        let (_, mut files) = conforming();
        files.insert("crates/ecs/README.md", "");
        files.insert("crates/ecs/SAFETY.md", "");
        files.insert("crates/ecs/Cargo.toml", MANIFEST);
        files.insert("crates/ecs/clippy.toml", lint_file());
        let ws = workspace(vec![
            package(
                "lactose-core",
                &[("bytemuck", Normal), ("xxhash-rust", Normal), ("rand", Dev)],
            ),
            package(
                "lactose-math",
                &[
                    ("libm", Build),
                    ("bytemuck", Normal),
                    ("lactose-task", Normal),
                    ("lactose-io", Dev),
                ],
            ),
            package(
                "lactose-ecs",
                &[
                    ("lactose-core", Normal),
                    ("lactose-task", Normal),
                    ("lactose-ecs-macros", Normal),
                    ("lactose-reflect", Normal),
                    ("lactose-io", Normal),
                ],
            ),
        ]);
        let found = run(&ws, &files);
        let tail = "in crate-tiers.toml does not allow";
        assert_eq!(
            found,
            [
                format!("crates/core/Cargo.toml: boundary crate `lactose-core` depends on third-party crate `xxhash-rust`, which [boundary_third_party.direct] \"lactose-core\" {tail}"),
                format!("crates/core/Cargo.toml: boundary crate `lactose-core` dev-depends on third-party crate `rand`, which [boundary_third_party.direct] \"lactose-core\" {tail}"),
                format!("crates/ecs/Cargo.toml: boundary crate `lactose-ecs` depends on non-boundary crate `lactose-io`, which [boundary_internal] {tail}"),
                // What `core` wrongly names reaches everything that depends on `core`.
                "crates/ecs/Cargo.toml: boundary crate `lactose-ecs` compiles third-party crate `xxhash-rust` (lactose-ecs -> lactose-core -> xxhash-rust), which is on no `normal` list and not in `transitive` of [boundary_third_party] in crate-tiers.toml".to_owned(),
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` depends on third-party crate `bytemuck`, which [boundary_third_party.direct] \"lactose-math\" {tail}"),
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` depends on non-boundary crate `lactose-task`, which [boundary_internal] {tail}"),
            ]
        );
    }

    #[test]
    fn everything_a_boundary_crate_compiles_must_be_vetted() {
        // Review item 14: an allowlisted crate, and an allowlisted workspace crate, each
        // bringing in a crate nobody reviewed.
        let (_, mut files) = conforming();
        files.insert("crates/reflect/README.md", "");
        files.insert("crates/reflect/src/lib.rs", FORBID);
        let ws = workspace_with(
            vec![
                package("lactose-core", &[("bytemuck", Normal), ("lactose-reflect", Normal)]),
                package("lactose-reflect", &[("tracing", Normal), ("trybuild", Dev)]),
            ],
            &[("bytemuck", &["bytemuck_derive", "fast-float"]), ("tracing", &["tracing-core"])],
        );
        let found = run(&ws, &files);
        let tail = "which is on no `normal` list and not in `transitive` of [boundary_third_party] in crate-tiers.toml";
        assert_eq!(
            found,
            [
                format!(
                    "crates/core/Cargo.toml: boundary crate `lactose-core` compiles third-party crate `fast-float` (lactose-core -> bytemuck -> fast-float), {tail}"
                ),
                format!(
                    "crates/core/Cargo.toml: boundary crate `lactose-core` compiles third-party crate `tracing` (lactose-core -> lactose-reflect -> tracing), {tail}"
                ),
                format!(
                    "crates/core/Cargo.toml: boundary crate `lactose-core` compiles third-party crate `tracing-core` (lactose-core -> lactose-reflect -> tracing -> tracing-core), {tail}"
                ),
            ]
        );
    }

    #[test]
    fn the_pin_is_the_only_rust_flag_setting_in_any_cargo_configuration() {
        // Review item 2, cargo configuration files.
        let (ws, mut files) = conforming();
        files.insert(
            CARGO_CONFIG,
            "# rustflags = [\"-C\", \"target-cpu=x86-64-v3\"]\n[alias]\nx = \"run\"\n",
        );
        assert_eq!(
            run(&ws, &files),
            [
                ".cargo/config.toml: the x86-64 pin is missing: [target.'cfg(target_arch = \"x86_64\")'] must set rustflags = [\"-C\", \"target-cpu=x86-64-v3\"]"
            ]
        );

        let only = "the only rust flags permitted in a cargo configuration file are the pin, [target.'cfg(target_arch = \"x86_64\")'] rustflags = [\"-C\", \"target-cpu=x86-64-v3\"] in .cargo/config.toml";
        files.insert(
            CARGO_CONFIG,
            format!("{PIN}[build]\nrustflags = [\"-C\", \"target_feature=+avx512f\"]\n[target.x86_64-unknown-linux-gnu]\nrustflags = [\"-Ctarget_cpu=native\"]\n[env]\nCLIPPY_CONF_DIR = \"elsewhere\"\n"),
        );
        files.insert("tools/cook/.cargo/config.toml", PIN);
        assert_eq!(
            run(&ws, &files),
            [
                ".cargo/config.toml: line 4: sets `target-feature`; target features come from the pinned target CPU only".to_owned(),
                ".cargo/config.toml: line 6: selects a target CPU (`target-cpu` `native`); the only permitted setting is the x86-64-v3 pin in .cargo/config.toml".to_owned(),
                ".cargo/config.toml: line 8: names CLIPPY_CONF_DIR, which points clippy away from each crate's clippy.toml".to_owned(),
                format!(".cargo/config.toml: sets `build.rustflags`; {only}"),
                format!(".cargo/config.toml: sets `target.x86_64-unknown-linux-gnu.rustflags`; {only}"),
                format!("tools/cook/.cargo/config.toml: sets `target.cfg(target_arch = \"x86_64\").rustflags`; {only}"),
            ]
        );

        // The pin with anything added to it is not the pin.
        files.insert(CARGO_CONFIG, PIN.replace("\"]", "\", \"-C\", \"target-feature=+fma\"]"));
        let files = without(&files, &["tools/cook/.cargo/config.toml"]);
        let found = run(&ws, &files);
        assert_eq!(found.len(), 3, "{found:#?}");
        assert!(found[2].contains("the x86-64 pin is missing"));
    }

    #[test]
    fn workflows_and_scripts_may_not_touch_flags_cpu_features_or_lints() {
        // Review items 1 and 2, build configuration.
        let (ws, mut files) = conforming();
        files.insert(
            ".github/workflows/ci.yml",
            "env:\n  # Never set RUSTFLAGS here.\n  RUSTFLAGS: -D warnings\njobs:\n  a:\n    run: cargo rustc -- -C target_cpu=native\n    run: cargo --config 'build.rustflags=[\"-Ctarget-feature=+avx2\"]' build\n    run: cargo clippy -- -A clippy::disallowed_methods -Aclippy::all --allow=warnings -A dead_code\n    env:\n      CLIPPY_CONF_DIR: /tmp\n",
        );
        files.insert(
            "scripts/bench.sh",
            "CARGO_ENCODED_RUSTFLAGS=x cargo bench\ncargo rustc -- --codegen target-cpu=haswell\n",
        );
        // Files that are neither configuration nor scripts are not scanned.
        files.insert("docs/notes.toml", "x = \"RUSTFLAGS target-cpu=native\"\n");
        let found = run(&ws, &files);
        let expect = [
            ".github/workflows/ci.yml: line 3: names RUSTFLAGS;",
            ".github/workflows/ci.yml: line 6: selects a target CPU (`target-cpu` `native`);",
            ".github/workflows/ci.yml: line 7: names RUSTFLAGS;",
            ".github/workflows/ci.yml: line 7: sets `target-feature`;",
            ".github/workflows/ci.yml: line 8: allows `clippy::disallowed_methods` from the command line",
            ".github/workflows/ci.yml: line 8: allows `clippy::all` from the command line",
            ".github/workflows/ci.yml: line 8: allows `warnings` from the command line",
            ".github/workflows/ci.yml: line 10: names CLIPPY_CONF_DIR,",
            "scripts/bench.sh: line 1: names RUSTFLAGS;",
            "scripts/bench.sh: line 2: selects a target CPU (`target-cpu` `haswell`);",
        ];
        assert_eq!(found.len(), expect.len(), "{found:#?}");
        for (found, expect) in found.iter().zip(expect) {
            assert!(found.starts_with(expect), "{found}\n  expected: {expect}");
        }
    }
}
