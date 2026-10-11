//! The check of crates built from a patched copy (architecture sections 8.2 and 8.3, D123).
//!
//! `vendor/<name>-<version>/` holds a published archive as `cargo vendor --versioned-dirs`
//! unpacks it, with the `.cargo-checksum.json` it writes (the record), and the patch beside
//! it, `vendor/<name>-<version>.patch`, applied. The root `Cargo.toml` names the directory
//! under `[patch.crates-io]`. The check proves in ten steps that nothing else replaces a
//! crate's source, that each copy is its patch applied to its record and the patch within
//! the rule, and then, online, that the record is the crates.io archive's and that the
//! upstream versions, which `cargo-deny` cannot see behind a path source, carry no advisory.
//!
//! [`check`] (steps 1 to 7) and [`prose`] (step 9) are pure, over what [`Repository::load`]
//! read, git's reading of each patch included. [`reverse`] (step 8) copies a tree under the
//! target directory and runs `git apply` there, and [`online`] (step 10) runs `cargo vendor`
//! and `cargo deny`; what they decide from the output is pure and tested apart from the
//! processes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Output};

use sha2::{Digest, Sha256};
use toml::{Table, Value};

use crate::Finding;
use crate::repo::{Files, is_build_configuration, is_cargo_config, unread_cargo_configuration};
use crate::workspace::{ResolvedPackage, Workspace};

/// The root manifest, which alone may hold a `[patch]` table.
const ROOT_MANIFEST: &str = "Cargo.toml";
/// The record `cargo vendor` writes into each package directory.
const RECORD: &str = ".cargo-checksum.json";
/// The prose beside the copies.
const README: &str = "vendor/README.md";
/// The changed lines a patch may hold, comments included (architecture section 8.2).
const MAX_CHANGED_LINES: usize = 50;
/// The characters an added line may hold (D123), so the line cap bounds the code too.
const MAX_LINE_CHARS: usize = 200;
/// What an added line may not hold, because each reaches unsafe code or code outside the
/// diff without the `unsafe` token: unsafe attributes before edition 2024, assembly, and
/// source pulled in from elsewhere. Matched as whole identifiers; a `!` is part of the
/// token where it stands, and `#[path` is the `path` attribute.
const REFUSED_TOKENS: [&str; 12] = [
    "unsafe",
    "no_mangle",
    "export_name",
    "link_section",
    "link_name",
    "global_asm",
    "asm!",
    "naked",
    "include!",
    "include_str!",
    "include_bytes!",
    "#[path",
];
/// The text an added line of every file a patch changes holds: the notice Apache-2.0
/// section 4(b) asks of a modified file.
const NOTICE: &str = "LACTOSE patch:";
/// The files a patch never touches, so the version, features, dependencies and closure the
/// allowlist review saw stay upstream's.
const UNTOUCHABLE: [&str; 5] =
    ["Cargo.toml", "Cargo.toml.orig", "Cargo.lock", ".cargo_vcs_info.json", RECORD];
/// The package of the manifest the online step writes.
const SHADOW_PACKAGE: &str = "vendor-upstream";

/// What the check reads from the repository.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Repository {
    /// The root manifest, the member manifests, every file that can decide how cargo is
    /// invoked (cargo configuration, workflows, scripts) with the symbolic links met,
    /// `.gitattributes`, `vendor/README.md`, and for each copy its `Cargo.toml`, its
    /// `.cargo-checksum.json` and the `.patch` beside it, where they exist.
    pub files: Files,
    /// The member manifests other than the root's, relative to the root.
    pub members: Vec<String>,
    /// Every name directly under `vendor/`, sorted.
    pub entries: Vec<String>,
    /// The directories among [`entries`](Self::entries): the copies.
    pub copies: Vec<String>,
    /// For each copy with a patch, what `git apply --numstat -z` prints for it, or why git
    /// refused it. git decides what a patch touches; the check's own reading must agree.
    pub numstat: BTreeMap<String, Result<String, String>>,
}

impl Repository {
    /// Reads what the check needs from the repository at `root`, whose member manifests
    /// (relative to `root`) are `members`, and has git read each patch in a directory under
    /// `scratch`.
    ///
    /// # Errors
    /// If a directory or a file that exists cannot be read, or git cannot be run.
    pub fn load(root: &Path, scratch: &Path, members: Vec<String>) -> Result<Self, String> {
        let all = Files::load(root)?;
        let mut files = Files::default();
        let members: Vec<String> = members.into_iter().filter(|m| m != ROOT_MANIFEST).collect();
        for path in all.paths() {
            let wanted = path == ROOT_MANIFEST || members.iter().any(|m| m == path);
            if wanted || is_build_configuration(path) {
                files.insert(path, all.bytes(path).unwrap_or_default());
            }
        }
        for link in all.links() {
            files.insert_link(link);
        }
        let (mut entries, mut copies) = (Vec::new(), Vec::new());
        let vendor = root.join("vendor");
        if vendor.is_dir() {
            let listing =
                fs::read_dir(&vendor).map_err(|e| format!("{}: {e}", vendor.display()))?;
            for entry in listing {
                let entry = entry.map_err(|e| format!("{}: {e}", vendor.display()))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                // Through a symbolic link: what cargo builds is what the link names.
                if entry.path().is_dir() {
                    copies.push(name.clone());
                }
                entries.push(name);
            }
        }
        entries.sort();
        copies.sort();
        let mut wanted = vec![".gitattributes".to_owned(), README.to_owned()];
        for dir in &copies {
            wanted.push(format!("vendor/{dir}/Cargo.toml"));
            wanted.push(format!("vendor/{dir}/{RECORD}"));
            wanted.push(format!("vendor/{dir}.patch"));
        }
        for path in wanted {
            match fs::read(root.join(&path)) {
                Ok(bytes) => files.insert(path, bytes),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{path}: {e}")),
            }
        }
        let mut numstat = BTreeMap::new();
        for dir in copies.iter().filter(|d| files.contains(&format!("vendor/{d}.patch"))) {
            let empty = scratch.join("numstat");
            remove(&empty)?;
            fs::create_dir_all(&empty).map_err(|e| format!("{}: {e}", empty.display()))?;
            let patch = root.join(format!("vendor/{dir}.patch"));
            let output = git_apply(scratch, &empty, &["--numstat", "-z"], &patch)?;
            let read = if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            } else {
                Err(first_line(&output))
            };
            numstat.insert(dir.clone(), read);
        }
        Ok(Self { files, members, entries, copies, numstat })
    }

    fn table(&self, path: &str) -> Option<Table> {
        self.files.text(path)?.parse::<Table>().ok()
    }

    /// The name and version the copy's manifest holds.
    fn package(&self, dir: &str) -> Option<(String, String)> {
        let manifest = self.table(&format!("vendor/{dir}/Cargo.toml"))?;
        let package = manifest.get("package")?.as_table()?;
        let name = package.get("name")?.as_str()?;
        let version = package.get("version")?.as_str()?;
        Some((name.to_owned(), version.to_owned()))
    }

    /// The copy's record, if it has a well-formed one.
    fn record(&self, dir: &str) -> Option<Record> {
        Record::parse(&self.files.text(&format!("vendor/{dir}/{RECORD}"))?)
    }
}

/// One `[patch.crates-io]` entry of the root manifest that names a directory under
/// `vendor/`.
struct Entry {
    key: String,
    /// The crate it replaces: the entry's `package` key, else the key itself.
    name: String,
    dir: String,
}

/// The directory `vendor/<dir>` that a `path` value names, if it names one.
fn vendor_dir(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path);
    let dir = path.strip_prefix("vendor/")?.trim_end_matches('/');
    let plain = !dir.is_empty() && dir != "." && dir != ".." && !dir.contains('/');
    plain.then(|| dir.to_owned())
}

/// The directory of a resolved package whose manifest is `vendor/<dir>/Cargo.toml`.
fn vendored(manifest: &str) -> Option<&str> {
    let dir = manifest.strip_prefix("vendor/")?.strip_suffix("/Cargo.toml")?;
    (!dir.is_empty() && !dir.contains('/')).then_some(dir)
}

/// Whether a resolved package came from a registry.
fn from_registry(package: &ResolvedPackage) -> bool {
    package
        .source
        .as_deref()
        .is_some_and(|s| s.starts_with("registry+") || s.starts_with("sparse+"))
}

/// Steps 1 to 7: the patch sources, the correspondence between `[patch.crates-io]`, the
/// resolved graph and `vendor/`, the names, one version, the records and `.gitattributes`,
/// the patch files and the rule. `graph` is `None` when the graph could not be resolved
/// (cargo refuses, for one, a patch path that does not exist); the parts of steps 2 and 4
/// that read it are then left out.
#[must_use]
pub fn check(repo: &Repository, graph: Option<&Workspace>) -> Vec<Finding> {
    let root = repo.table(ROOT_MANIFEST).unwrap_or_default();
    let mut findings = Vec::new();
    let entries = patch_sources(repo, &root, &mut findings);
    correspondence(repo, &entries, graph, &mut findings);
    names(repo, &entries, &mut findings);
    if let Some(graph) = graph {
        one_version(graph, &mut findings);
    }
    records(repo, &mut findings);
    for dir in &repo.copies {
        patch_rule(repo, dir, &mut findings);
    }
    findings
}

/// Step 1. Returns the `[patch.crates-io]` entries that name a directory under `vendor/`.
fn patch_sources(repo: &Repository, root: &Table, findings: &mut Vec<Finding>) -> Vec<Entry> {
    let mut entries = Vec::new();
    for (source, table) in root.get("patch").and_then(Value::as_table).into_iter().flatten() {
        if source != "crates-io" {
            findings.push(Finding::new(
                ROOT_MANIFEST,
                format!("[patch.\"{source}\"] is not allowed; only crates-io is patched"),
            ));
            continue;
        }
        for (key, value) in table.as_table().into_iter().flatten() {
            let dir = value.get("path").and_then(Value::as_str).and_then(vendor_dir);
            let Some(dir) = dir.filter(|_| value.get("git").is_none()) else {
                findings.push(Finding::new(
                    ROOT_MANIFEST,
                    format!("[patch.crates-io] \"{key}\" is not a path under vendor/"),
                ));
                continue;
            };
            let name = value.get("package").and_then(Value::as_str).unwrap_or(key);
            entries.push(Entry { key: key.clone(), name: name.to_owned(), dir });
        }
    }
    // `[replace]` is the older form of `[patch]`, which cargo still honours.
    for manifest in std::iter::once(ROOT_MANIFEST).chain(repo.members.iter().map(String::as_str)) {
        let Some(table) = repo.table(manifest) else { continue };
        if manifest != ROOT_MANIFEST && table.contains_key("patch") {
            findings.push(Finding::new(manifest, "holds a [patch] table; only the root may"));
        }
        if table.contains_key("replace") {
            findings.push(Finding::new(
                manifest,
                "holds a [replace] table; a crate is replaced only by [patch.crates-io] in the \
                 root Cargo.toml, under vendor/",
            ));
        }
    }
    for config in repo.files.paths().filter(|path| is_cargo_config(path)) {
        let text = repo.files.text(config).unwrap_or_default();
        match text.parse::<Table>() {
            Ok(table) if ["patch", "source", "paths"].iter().any(|k| table.contains_key(*k)) => {
                findings.push(Finding::new(
                    config,
                    "sets [patch], [source] or paths; a patch lives in the root Cargo.toml only",
                ));
            }
            Ok(_) => {}
            Err(_) => findings.push(Finding::new(
                config,
                "is not TOML, so it cannot be shown to set no [patch], [source] or paths",
            )),
        }
    }
    findings.extend(unread_cargo_configuration(&repo.files));
    for config in repo.files.paths().filter(|path| is_cargo_config(path)) {
        let Some(table) = repo.table(config) else { continue };
        let aliases = table.get("alias").and_then(Value::as_table).into_iter().flatten();
        for (name, value) in aliases {
            let words: Vec<String> = match value {
                Value::String(line) => line.split_whitespace().map(str::to_owned).collect(),
                Value::Array(items) => {
                    items.iter().filter_map(Value::as_str).map(str::to_owned).collect()
                }
                _ => Vec::new(),
            };
            if words.iter().any(|w| w.starts_with("--config") || w.starts_with("-Z")) {
                findings.push(Finding::new(
                    config,
                    format!(
                        "alias {name} passes --config or -Z, through which a [patch], [source] \
                         or paths setting reaches cargo unseen"
                    ),
                ));
            }
        }
    }
    command_lines(repo, findings);
    entries
}

/// Step 1 for the files that can decide how cargo is invoked, line by line as `cargo xtask
/// boundary` reads them: a `--config` naming `patch`, `source` or `paths` sets them outside
/// every file the check reads, and `CARGO_HOME` moves the configuration cargo reads last.
/// Only whole-line `#` comments are skipped.
fn command_lines(repo: &Repository, findings: &mut Vec<Finding>) {
    for path in repo.files.paths().filter(|path| is_build_configuration(path)) {
        let text = repo.files.text(path).unwrap_or_default();
        for (index, line) in text.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let names = ["patch", "source", "paths"].iter().any(|key| holds_token(line, key));
            if line.contains("--config") && names {
                findings.push(Finding::new(
                    path,
                    format!(
                        "line {}: passes --config naming patch, source or paths; a patch lives \
                         in the root Cargo.toml only",
                        index + 1
                    ),
                ));
            }
            let sets_home = line.match_indices("CARGO_HOME").any(|(at, _)| {
                let rest = line[at + "CARGO_HOME".len()..].trim_start_matches(['"', '\'', ' ']);
                rest.starts_with('=') || rest.starts_with(':')
            });
            if sets_home {
                findings.push(Finding::new(
                    path,
                    format!(
                        "line {}: sets CARGO_HOME, whose cargo configuration no check reads",
                        index + 1
                    ),
                ));
            }
        }
    }
}

/// Step 2.
fn correspondence(
    repo: &Repository,
    entries: &[Entry],
    graph: Option<&Workspace>,
    findings: &mut Vec<Finding>,
) {
    let named: BTreeSet<&str> = entries.iter().map(|e| e.dir.as_str()).collect();
    let mut built: BTreeSet<&str> = BTreeSet::new();
    let mut members: BTreeSet<&str> = BTreeSet::new();
    let mut outside = Vec::new();
    for package in graph.iter().flat_map(|g| g.resolved.values()) {
        match (package.member, vendored(&package.manifest)) {
            (true, Some(dir)) => {
                members.insert(dir);
            }
            (true, None) => {}
            (false, _) if package.source.is_some() => {}
            (false, Some(dir)) => {
                built.insert(dir);
            }
            (false, None) => outside.push(Finding::new(
                &package.manifest,
                format!(
                    "non-member path package {} {} outside vendor/",
                    package.name, package.version
                ),
            )),
        }
    }
    outside.sort();
    findings.extend(outside);
    let dirs: BTreeSet<&str> =
        repo.copies.iter().map(String::as_str).chain(built.iter().copied()).collect();
    for dir in &dirs {
        if !named.contains(dir) {
            findings
                .push(Finding::new(format!("vendor/{dir}"), "no [patch.crates-io] entry names it"));
        }
    }
    for entry in entries {
        if !repo.copies.contains(&entry.dir) {
            findings.push(Finding::new(
                ROOT_MANIFEST,
                format!(
                    "[patch.crates-io] \"{}\" names vendor/{}, which does not exist",
                    entry.key, entry.dir
                ),
            ));
        }
    }
    for entry in &repo.entries {
        let copy = |dir: &str| repo.copies.iter().any(|c| c == dir);
        if copy(entry) || entry == "README.md" {
            continue;
        }
        let message = match entry.strip_suffix(".patch") {
            Some(dir) if copy(dir) => continue,
            Some(dir) => format!("no copy vendor/{dir} beside it"),
            None => "neither a copy, its patch nor README.md; nothing else lives under vendor/"
                .to_owned(),
        };
        findings.push(Finding::new(format!("vendor/{entry}"), message));
    }
    if graph.is_none() {
        return;
    }
    for dir in repo.copies.iter().filter(|d| named.contains(d.as_str())) {
        if members.contains(dir.as_str()) {
            findings.push(Finding::new(
                format!("vendor/{dir}"),
                "is a workspace member; a copy is a dependency, excluded from the workspace",
            ));
        } else if !built.contains(dir.as_str()) {
            findings.push(Finding::new(
                format!("vendor/{dir}"),
                "no package in the resolved graph is built from it (the patch is unused)",
            ));
        }
    }
}

/// Step 3.
fn names(repo: &Repository, entries: &[Entry], findings: &mut Vec<Finding>) {
    for dir in &repo.copies {
        match repo.package(dir) {
            None => findings.push(Finding::new(
                format!("vendor/{dir}/Cargo.toml"),
                "missing, or without a [package] name and version",
            )),
            Some((name, version)) if *dir != format!("{name}-{version}") => {
                findings.push(Finding::new(
                    format!("vendor/{dir}"),
                    format!("holds {name} {version}, so it must be named vendor/{name}-{version}"),
                ));
            }
            Some(_) => {}
        }
    }
    for entry in entries {
        if let Some((name, _)) = repo.package(&entry.dir)
            && name != entry.name
        {
            findings.push(Finding::new(
                ROOT_MANIFEST,
                format!(
                    "[patch.crates-io] \"{}\" names vendor/{}, which holds {name}",
                    entry.key, entry.dir
                ),
            ));
        }
    }
}

/// Step 4: a name built from a copy resolves at no other version, from anywhere.
fn one_version(graph: &Workspace, findings: &mut Vec<Finding>) {
    let mut found = BTreeSet::new();
    for package in graph.resolved.values().filter(|p| !p.member && p.source.is_none()) {
        let Some(dir) = vendored(&package.manifest) else { continue };
        for other in graph.resolved.values() {
            if other.name != package.name || other.version == package.version {
                continue;
            }
            let from = if from_registry(other) {
                "crates.io".to_owned()
            } else if let Some(other_dir) = vendored(&other.manifest) {
                format!("vendor/{other_dir}")
            } else {
                other.source.clone().unwrap_or_else(|| other.manifest.clone())
            };
            found.insert(Finding::new(
                format!("vendor/{dir}"),
                format!(
                    "{} also resolves from {from} at {}; a patch covers one version",
                    package.name, other.version
                ),
            ));
        }
    }
    findings.extend(found);
}

/// Step 5.
fn records(repo: &Repository, findings: &mut Vec<Finding>) {
    for dir in &repo.copies {
        if repo.record(dir).is_none() {
            findings.push(Finding::new(
                format!("vendor/{dir}/{RECORD}"),
                "missing, or without a \"files\" map and a \"package\" checksum; write it with \
                 cargo vendor --versioned-dirs",
            ));
        }
    }
    let root = repo.files.text(".gitattributes").unwrap_or_default();
    if !keeps_vendor_bytes(&root) {
        findings.push(Finding::new(".gitattributes", "does not mark vendor/** -text"));
    }
}

/// The lines of a `.gitattributes` file as (pattern, whether the line leaves `text` unset)
/// for each line that names `text`, in order. `binary` is `-diff -merge -text`.
fn sets_text(text: &str) -> impl Iterator<Item = (&str, bool)> {
    text.lines().filter_map(|line| {
        let mut words = line.split_whitespace();
        let pattern = words.next().filter(|p| !p.starts_with('#'))?;
        let mut unset = None;
        for attribute in words {
            match attribute {
                "-text" | "binary" => unset = Some(true),
                "text" | "!text" => unset = Some(false),
                _ if attribute.starts_with("text=") => unset = Some(false),
                _ => {}
            }
        }
        unset.map(|unset| (pattern, unset))
    })
}

/// Whether the root `.gitattributes` leaves every vendored file unconverted: a line
/// `vendor/** -text`, and no later line that can match a file under `vendor/` setting
/// `text` again or making it unspecified (which hands it to `core.autocrlf`).
fn keeps_vendor_bytes(text: &str) -> bool {
    // A pattern without a slash matches at any depth; one with a leading `**/` too.
    let reaches = |pattern: &str| {
        let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
        pattern.starts_with("vendor/") || pattern.starts_with("**") || !pattern.contains('/')
    };
    let mut kept = false;
    for (pattern, unset) in sets_text(text) {
        if pattern == "vendor/**" || pattern == "/vendor/**" {
            kept = unset;
        } else if reaches(pattern) && !unset {
            kept = false;
        }
    }
    kept
}

/// Steps 6 and 7 for one copy. git's reading of the patch decides which files it touches
/// and how many lines it changes; the check's own reading, which finds the tokens and the
/// notice, must agree with it file by file.
fn patch_rule(repo: &Repository, dir: &str, findings: &mut Vec<Finding>) {
    let path = format!("vendor/{dir}.patch");
    let Some(text) = repo.files.text(&path) else {
        findings.push(Finding::new(
            path,
            "missing; a copy without a patch has no reason to exist, use the registry",
        ));
        return;
    };
    let patch = match Patch::parse(&text) {
        Ok(patch) => patch,
        Err(None) => return findings.push(Finding::new(path, "not a unified diff")),
        Err(Some(why)) => {
            return findings.push(Finding::new(path, format!("not a unified diff ({why})")));
        }
    };
    let stats = match repo.numstat.get(dir) {
        Some(Ok(numstat)) => parse_numstat(numstat),
        Some(Err(why)) => {
            return findings.push(Finding::new(path, format!("not a patch git applies ({why})")));
        }
        None => return findings.push(Finding::new(path, "git has not read it")),
    };
    let mut theirs: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut changed = 0;
    let mut named: BTreeSet<&str> = BTreeSet::new();
    let mut binary: BTreeSet<&str> = BTreeSet::new();
    for stat in &stats {
        named.extend(stat.paths.iter().map(String::as_str));
        let target = stat.paths.last().map_or("a file", String::as_str);
        let (Some(added), Some(removed)) = (stat.added, stat.removed) else {
            binary.insert(target);
            findings.push(Finding::new(
                &path,
                format!("not a unified diff (git reads a binary change to {target})"),
            ));
            continue;
        };
        changed += added + removed;
        let counts = theirs.entry(target).or_default();
        *counts = (counts.0 + added, counts.1 + removed);
    }
    let mut ours: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for file in &patch.files {
        named.extend(file.paths.iter().map(String::as_str));
        let counts = ours.entry(file.name()).or_default();
        *counts = (counts.0 + file.added, counts.1 + file.removed);
    }
    let files: BTreeSet<&str> = theirs.keys().chain(ours.keys()).copied().collect();
    for file in files.into_iter().filter(|file| !binary.contains(file)) {
        let message = match (theirs.get(file), ours.get(file)) {
            (Some(_), None) => format!("git applies it to {file}, which the check does not read"),
            (None, Some(_)) => {
                format!("the check reads a change to {file}, which git does not apply")
            }
            (Some(git), Some(read)) if git != read => format!(
                "git reads {file} as {} added and {} removed lines, the check as {} and {}",
                git.0, git.1, read.0, read.1
            ),
            _ => continue,
        };
        findings.push(Finding::new(&path, message));
    }
    if changed > MAX_CHANGED_LINES {
        findings.push(Finding::new(
            &path,
            format!("{changed} changed lines, over the fifty of architecture section 8.2"),
        ));
    }
    for file in named {
        let name = file.rsplit('/').next().unwrap_or(file);
        if UNTOUCHABLE.contains(&name) {
            findings
                .push(Finding::new(&path, format!("names {file}, which a patch never touches")));
        }
    }
    for (line, token) in patch.files.iter().flat_map(|f| &f.refused) {
        findings.push(Finding::new(&path, format!("adds {token} at line {line}")));
    }
    for (line, chars) in patch.files.iter().flat_map(|f| &f.long) {
        findings.push(Finding::new(
            &path,
            format!(
                "line {line} adds {chars} characters, over the {MAX_LINE_CHARS} an added line may \
                 hold"
            ),
        ));
    }
    for file in patch.files.iter().filter(|f| !f.notice) {
        findings
            .push(Finding::new(&path, format!("{} carries no \"{NOTICE}\" notice", file.name())));
    }
}

/// One file as `git apply --numstat -z` reports it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Stat {
    /// The lines added and removed, `None` for a binary change.
    added: Option<usize>,
    removed: Option<usize>,
    /// The path, or the old and the new path of a rename or copy.
    paths: Vec<String>,
}

/// Reads `git apply --numstat -z`: `added TAB removed TAB path NUL`, or for a rename or copy
/// `added TAB removed TAB NUL old NUL new NUL`; `-` counts a binary change.
fn parse_numstat(text: &str) -> Vec<Stat> {
    let mut fields = text.split('\0');
    let mut stats = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let mut parts = field.splitn(3, '\t');
        let added = parts.next().and_then(|n| n.parse().ok());
        let removed = parts.next().and_then(|n| n.parse().ok());
        let paths = match parts.next() {
            Some(path) if !path.is_empty() => vec![path.to_owned()],
            _ => fields.by_ref().take(2).map(str::to_owned).collect(),
        };
        stats.push(Stat { added, removed, paths });
    }
    stats
}

/// A patch as `git apply` reads it, reduced to what the rule asks about.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Patch {
    files: Vec<PatchedFile>,
}

/// One file of a patch.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct PatchedFile {
    /// The path the file has after the patch, or before it for a deletion.
    target: Option<String>,
    /// Every path the patch names for the file, without the `a/` and `b/` prefixes and
    /// never `/dev/null`: the old and new paths, rename and copy sources and targets.
    paths: Vec<String>,
    /// Whether `---` and `+++` lines introduced it.
    headed: bool,
    hunks: usize,
    /// The lines it adds.
    added: usize,
    /// The lines it removes.
    removed: usize,
    /// The patch's line numbers, from 1, of added lines that hold a refused token, with it.
    refused: Vec<(usize, &'static str)>,
    /// The patch's line numbers of added lines over the length cap, with their length.
    long: Vec<(usize, usize)>,
    /// Whether an added line holds the notice.
    notice: bool,
}

impl PatchedFile {
    fn name(&self) -> &str {
        self.target.as_deref().or(self.paths.first().map(String::as_str)).unwrap_or("a file")
    }

    fn add_path(&mut self, path: String) {
        if !self.paths.contains(&path) {
            self.paths.push(path);
        }
    }
}

impl Patch {
    /// Reads `text` the way `git apply` does: file headers, then hunks whose length is the
    /// one their `@@` header states, so a removed line that begins `--` is not a header.
    /// Text outside the headers and hunks is ignored, as git ignores it.
    ///
    /// # Errors
    /// `None` if the text has no file header with a hunk; otherwise why it is not a unified
    /// diff `git apply` would take line by line (a binary patch, a hunk cut short).
    fn parse(text: &str) -> Result<Self, Option<String>> {
        let lines: Vec<&str> =
            text.split_inclusive('\n').map(|l| l.strip_suffix('\n').unwrap_or(l)).collect();
        let mut files: Vec<PatchedFile> = Vec::new();
        let mut current: Option<PatchedFile> = None;
        let mut at = 0;
        while at < lines.len() {
            let line = lines[at];
            let number = at + 1;
            at += 1;
            if let Some(rest) = line.strip_prefix("diff --git ") {
                files.extend(current.take());
                let mut file = PatchedFile::default();
                if let Some(path) = git_header_path(rest) {
                    file.target = Some(path.clone());
                    file.add_path(path);
                }
                current = Some(file);
            } else if let (Some(old), Some(new)) =
                (line.strip_prefix("--- "), lines.get(at).and_then(|l| l.strip_prefix("+++ ")))
            {
                at += 1;
                let mut file = match current.take() {
                    Some(file) if !file.headed && file.hunks == 0 => file,
                    other => {
                        files.extend(other);
                        PatchedFile::default()
                    }
                };
                file.headed = true;
                let (old, new) = (header_path(old), header_path(new));
                if let Some(target) = new.clone().or_else(|| old.clone()) {
                    file.target = Some(target);
                }
                for path in [old, new].into_iter().flatten() {
                    file.add_path(path);
                }
                current = Some(file);
            } else if line.starts_with("@@ ") {
                let file = current
                    .as_mut()
                    .filter(|f| f.headed)
                    .ok_or_else(|| format!("the hunk at line {number} has no file header"))?;
                let (mut old, mut new) = hunk_lengths(line)
                    .ok_or_else(|| format!("line {number} is not a hunk header"))?;
                while old > 0 || new > 0 {
                    let cut = || format!("the hunk at line {number} is cut short");
                    let body = lines.get(at).ok_or_else(cut)?;
                    match body.as_bytes().first() {
                        // An empty line is an empty context line to git.
                        Some(b' ') | None if old > 0 && new > 0 => {
                            old -= 1;
                            new -= 1;
                        }
                        Some(b'-') if old > 0 => {
                            old -= 1;
                            file.removed += 1;
                        }
                        Some(b'+') if new > 0 => {
                            new -= 1;
                            file.added += 1;
                            let added = &body[1..];
                            for token in REFUSED_TOKENS.iter().filter(|t| refuses(added, t)) {
                                file.refused.push((at + 1, token));
                            }
                            let chars = added.trim_end_matches('\r').chars().count();
                            if chars > MAX_LINE_CHARS {
                                file.long.push((at + 1, chars));
                            }
                            file.notice |= added.contains(NOTICE);
                        }
                        Some(b'\\') => {}
                        _ => return Err(Some(cut())),
                    }
                    at += 1;
                }
                file.hunks += 1;
            } else if line.starts_with("GIT binary patch")
                || (line.starts_with("Binary files ") && line.ends_with(" differ"))
            {
                return Err(Some(format!("a binary patch at line {number}")));
            } else if let Some(path) = ["rename from ", "rename to ", "copy from ", "copy to "]
                .iter()
                .find_map(|header| line.strip_prefix(header))
            {
                let file = current.get_or_insert_with(PatchedFile::default);
                let path = unquote(path);
                if line.starts_with("rename to ") || line.starts_with("copy to ") {
                    file.target = Some(path.clone());
                }
                file.add_path(path);
            }
        }
        files.extend(current);
        if !files.iter().any(|f| f.headed && f.hunks > 0) {
            return Err(None);
        }
        Ok(Self { files })
    }
}

/// The path of a `---` or `+++` line with its first component (`a/`, `b/`) removed, or
/// `None` for `/dev/null`.
fn header_path(rest: &str) -> Option<String> {
    let path = if rest.starts_with('"') {
        unquote(rest)
    } else {
        // Anything after a tab is a timestamp.
        rest.split('\t').next().unwrap_or(rest).trim_end_matches('\r').to_owned()
    };
    if path == "/dev/null" {
        return None;
    }
    Some(path.split_once('/').map_or(path.clone(), |(_, rest)| rest.to_owned()))
}

/// The path of `diff --git a/<path> b/<path>` when both halves name the same path. Renames
/// and quoted paths are left to the lines that follow.
fn git_header_path(rest: &str) -> Option<String> {
    rest.match_indices(" b/").find_map(|(at, _)| {
        let old = rest[..at].strip_prefix("a/")?;
        (old == &rest[at + 3..]).then(|| old.to_owned())
    })
}

/// A path as git writes it: plain, or in double quotes with C escapes.
fn unquote(text: &str) -> String {
    let Some(quoted) = text.strip_prefix('"') else {
        return text.trim_end_matches('\r').to_owned();
    };
    let mut bytes = Vec::new();
    let mut chars = quoted.bytes();
    while let Some(byte) = chars.next() {
        match byte {
            b'"' => break,
            b'\\' => match chars.next() {
                Some(b't') => bytes.push(b'\t'),
                Some(b'n') => bytes.push(b'\n'),
                Some(digit @ b'0'..=b'7') => {
                    let mut value = u32::from(digit - b'0');
                    for _ in 0..2 {
                        if let Some(d @ b'0'..=b'7') = chars.clone().next() {
                            chars.next();
                            value = value * 8 + u32::from(d - b'0');
                        }
                    }
                    bytes.push(u8::try_from(value & 0xff).unwrap_or_default());
                }
                Some(other) => bytes.push(other),
                None => break,
            },
            other => bytes.push(other),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The old and new line counts of a hunk header `@@ -l[,n] +l[,n] @@`.
fn hunk_lengths(line: &str) -> Option<(usize, usize)> {
    let mut words = line.strip_prefix("@@ ")?.split(' ');
    let count = |range: &str| -> Option<usize> {
        match range.split_once(',') {
            Some((start, count)) => {
                start.parse::<usize>().ok()?;
                count.parse().ok()
            }
            None => range.parse::<usize>().ok().map(|_| 1),
        }
    };
    let old = count(words.next()?.strip_prefix('-')?)?;
    let new = count(words.next()?.strip_prefix('+')?)?;
    (words.next()? == "@@").then_some((old, new))
}

/// Whether `token` occurs in `line` as a whole identifier.
fn holds_token(line: &str, token: &str) -> bool {
    identifiers(line, token).next().is_some()
}

/// The text after each occurrence of `token` in `line` as a whole identifier.
fn identifiers<'a>(line: &'a str, token: &'a str) -> impl Iterator<Item = &'a str> {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    line.match_indices(token).filter_map(move |(at, _)| {
        let rest = &line[at + token.len()..];
        (!word(line[..at].chars().next_back()) && !word(rest.chars().next())).then_some(rest)
    })
}

/// Whether an added line holds one of [`REFUSED_TOKENS`]: the identifier, followed by `!`
/// for a macro, or for `#[path` a `path =` in an attribute.
fn refuses(line: &str, token: &str) -> bool {
    if token == "#[path" {
        let attribute = line.contains("#[") || line.contains("#!");
        return attribute
            && identifiers(line, "path").any(|rest| rest.trim_start().starts_with('='));
    }
    match token.strip_suffix('!') {
        Some(name) => identifiers(line, name).any(|rest| rest.trim_start().starts_with('!')),
        None => holds_token(line, token),
    }
}

/// What `cargo vendor` records of a package: the sha256 of every file but the record
/// itself, and of the archive.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Record {
    files: BTreeMap<String, String>,
    package: String,
}

impl Record {
    fn parse(text: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let package = value.get("package")?.as_str()?.to_owned();
        let mut files = BTreeMap::new();
        for (path, hash) in value.get("files")?.as_object()? {
            files.insert(path.replace('\\', "/"), hash.as_str()?.to_owned());
        }
        Some(Self { files, package })
    }
}

/// Step 8: copies `vendor/<dir>` afresh to `<scratch>/<dir>`, reverses the patch there with
/// `git apply`, and compares the sha256 of every file with the record; then applies the
/// patch forward to the tree that matched and requires the copy back, byte for byte. Every
/// hunk must apply where it says: git moving one to an offset, which repeated context
/// allows, is a finding both ways. A copy without a patch is compared as it is; one without
/// a record is left to step 5's finding.
///
/// # Errors
/// If the copy cannot be made or hashed, or git cannot be run.
pub fn reverse(
    root: &Path,
    scratch: &Path,
    repo: &Repository,
    dir: &str,
) -> Result<Vec<Finding>, String> {
    let Some(record) = repo.record(dir) else {
        return Ok(Vec::new());
    };
    let original = root.join("vendor").join(dir);
    let copy = scratch.join(dir);
    remove(&copy)?;
    copy_tree(&original, &copy)?;
    let path = format!("vendor/{dir}.patch");
    let patch = root.join(&path);
    let patched = repo.files.contains(&path);
    if patched {
        let output = git_apply(scratch, &copy, &["-v", "--reverse"], &patch)?;
        if !output.status.success() {
            let why = format!("does not reverse-apply to vendor/{dir} ({})", first_line(&output));
            return Ok(vec![Finding::new(path, why)]);
        }
        if let Some(line) = shifted(&output) {
            let why =
                format!("reverse-applies to vendor/{dir} only away from where it says ({line})");
            return Ok(vec![Finding::new(path, why)]);
        }
    }
    let findings = compare_record(dir, &record, &hash_tree(&copy)?);
    if !findings.is_empty() || !patched {
        return Ok(findings);
    }
    let output = git_apply(scratch, &copy, &["-v"], &patch)?;
    if !output.status.success() {
        let why = format!("does not apply to the record of vendor/{dir} ({})", first_line(&output));
        return Ok(vec![Finding::new(path, why)]);
    }
    if let Some(line) = shifted(&output) {
        let why =
            format!("applies to the record of vendor/{dir} only away from where it says ({line})");
        return Ok(vec![Finding::new(path, why)]);
    }
    let (forward, committed) = (hash_tree(&copy)?, hash_tree(&original)?);
    let files: BTreeSet<&String> = forward.keys().chain(committed.keys()).collect();
    let differ = files.into_iter().filter(|file| forward.get(*file) != committed.get(*file));
    Ok(differ
        .map(|file| {
            Finding::new(format!("vendor/{dir}/{file}"), "is not the record with the patch applied")
        })
        .collect())
}

/// Runs `git apply` with `args` on `patch` in `dir`, a directory under `scratch`. No
/// repository above `scratch` is consulted (the target directory may sit inside one, or not),
/// and nothing is converted: on a Windows runner `core.autocrlf` is set. git speaks
/// English, which [`shifted`] reads.
fn git_apply(scratch: &Path, dir: &Path, args: &[&str], patch: &Path) -> Result<Output, String> {
    Command::new("git")
        .args(["-c", "core.autocrlf=false", "apply", "--whitespace=nowarn"])
        .args(args)
        .arg(patch)
        .current_dir(dir)
        .env("GIT_CEILING_DIRECTORIES", scratch)
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .output()
        .map_err(|e| format!("cannot run `git apply`: {e}"))
}

/// git's first error line.
fn first_line(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let first = stderr.lines().map(str::trim).find(|line| !line.is_empty());
    first.unwrap_or("git gave no reason").to_owned()
}

/// The line of `git apply -v` that reports a hunk applied away from where the patch says it
/// is: at an offset, or with fuzz. Only git's `Hunk #` lines are read, so a file name that
/// holds either word is not one.
fn shifted(output: &Output) -> Option<String> {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let moved =
        |l: &&str| l.starts_with("Hunk #") && (l.contains("(offset ") || l.contains("fuzz"));
    text.lines().map(str::trim).find(moved).map(str::to_owned)
}

/// The sha256 of every file under `dir`, keyed by its path relative to `dir` with `/`
/// separators, except the record at the top. Symbolic links are followed.
fn hash_tree(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, String>) -> Result<(), String> {
        let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if path.is_dir() {
                walk(&path, &relative, out)?;
            } else if relative != RECORD {
                let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                out.insert(relative, hex(&Sha256::digest(&bytes)));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(dir, "", &mut out)?;
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The findings of step 8 for one copy: every file whose hash differs from the record, is
/// not in it, or is in it but missing.
fn compare_record(dir: &str, record: &Record, hashes: &BTreeMap<String, String>) -> Vec<Finding> {
    let paths: BTreeSet<&String> = record.files.keys().chain(hashes.keys()).collect();
    let mut findings = Vec::new();
    for path in paths {
        match (record.files.get(path), hashes.get(path)) {
            (Some(recorded), Some(hash)) if recorded != hash => findings
                .push(Finding::new(format!("vendor/{dir}/{path}"), "differs from the record")),
            (None, Some(_)) => {
                findings.push(Finding::new(format!("vendor/{dir}/{path}"), "is not in the record"))
            }
            (Some(_), None) => findings.push(Finding::new(
                format!("vendor/{dir}"),
                format!("the record's {path} is missing"),
            )),
            _ => {}
        }
    }
    findings
}

fn remove(dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(format!("{}: {e}", dir.display())),
        _ => Ok(()),
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", from.display()))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            fs::copy(&source, &target).map_err(|e| format!("{}: {e}", source.display()))?;
        }
    }
    Ok(())
}

/// Step 9: `vendor/README.md` has a section `## <name> <version>` for every copy and none
/// for a directory that is gone.
#[must_use]
pub fn prose(repo: &Repository) -> Vec<Finding> {
    let readme = repo.files.text(README);
    if repo.copies.is_empty() && readme.is_none() {
        return Vec::new();
    }
    let readme = readme.unwrap_or_default();
    let mut fenced = false;
    let mut sections = Vec::new();
    for line in readme.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if let Some(heading) = line.strip_prefix("## ").filter(|_| !fenced) {
            sections.push(heading.trim());
        }
    }
    let mut findings = Vec::new();
    for (name, version) in repo.copies.iter().filter_map(|dir| repo.package(dir)) {
        let section = format!("{name} {version}");
        if !sections.contains(&section.as_str()) {
            findings.push(Finding::new(README, format!("has no section \"## {section}\"")));
        }
    }
    for section in sections {
        let words: Vec<&str> = section.split_whitespace().collect();
        let names_a_copy = match words[..] {
            [name, version] => repo.copies.contains(&format!("{name}-{version}")),
            _ => false,
        };
        if !names_a_copy {
            findings.push(Finding::new(
                README,
                format!("section \"## {section}\" names no directory under vendor/"),
            ));
        }
    }
    findings
}

/// What step 10 found, and whether its advisory run took place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Online {
    /// Records that differ from the crates.io archives' and advisories of the upstream
    /// versions.
    pub findings: Vec<Finding>,
    /// How many records were re-derived.
    pub records: usize,
    /// Why the advisory check did not run, when it did not: only with `--offline`, and only
    /// because cargo-deny is not installed.
    pub advisories_skipped: Option<String>,
}

/// One copy as step 10 sees it.
struct Copy {
    dir: String,
    name: String,
    version: String,
}

/// Step 10: writes `<scratch>/upstream/Cargo.toml`, a manifest that depends on each
/// copy's crate at its exact version as the root `[workspace.dependencies]` entry declares
/// it, with no `[patch]` and a copy of the workspace lock; re-derives each record from the
/// crates.io archive with `cargo vendor` and requires it to equal the committed one; and runs
/// `cargo deny check advisories` over the manifest, since cargo-deny skips a path source.
/// Both resolve the manifest's lock without `--locked`. `offline` passes `--offline` to
/// both, and lets a machine without cargo-deny skip the advisory run.
///
/// Expects copies that passed steps 1 to 9.
///
/// # Errors
/// If a command cannot run or fails without a finding, or a file cannot be written.
pub fn online(
    cargo: &str,
    root: &Path,
    scratch: &Path,
    repo: &Repository,
    offline: bool,
) -> Result<Online, String> {
    let copies: Vec<Copy> = repo
        .copies
        .iter()
        .filter_map(|dir| {
            let (name, version) = repo.package(dir)?;
            Some(Copy { dir: dir.clone(), name, version })
        })
        .collect();
    let manifest = shadow_manifest(&repo.table(ROOT_MANIFEST).unwrap_or_default(), &copies);
    let upstream = scratch.join("upstream");
    remove(&upstream)?;
    let write = |path: &Path, contents: &[u8]| {
        fs::write(path, contents).map_err(|e| format!("{}: {e}", path.display()))
    };
    fs::create_dir_all(upstream.join("src")).map_err(|e| format!("{}: {e}", upstream.display()))?;
    write(&upstream.join("Cargo.toml"), manifest.as_bytes())?;
    write(&upstream.join("src").join("lib.rs"), b"")?;
    let lock = root.join("Cargo.lock");
    if lock.is_file() {
        fs::copy(&lock, upstream.join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;
    }
    let shadow = upstream.join("Cargo.toml");

    let derived = scratch.join("upstream-src");
    remove(&derived)?;
    let mut vendor = Command::new(cargo);
    vendor.args(["vendor", "--versioned-dirs", "--manifest-path"]).arg(&shadow).arg(&derived);
    if offline {
        vendor.arg("--offline");
    }
    let output = run(&mut vendor, "cargo vendor")?;
    if !output.status.success() {
        return Err(format!(
            "`cargo vendor` of the upstream versions failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut findings = Vec::new();
    for copy in &copies {
        let path = derived.join(format!("{}-{}", copy.name, copy.version)).join(RECORD);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let upstream = Record::parse(&text)
            .ok_or_else(|| format!("{}: not a record cargo vendor writes", path.display()))?;
        let committed = repo.record(&copy.dir).ok_or_else(|| {
            format!("vendor/{}/{RECORD}: not a record cargo vendor writes", copy.dir)
        })?;
        findings.extend(compare_upstream(&copy.dir, &committed, &upstream));
    }

    let installed = Command::new(cargo)
        .args(["deny", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    if !installed {
        if offline {
            return Ok(Online {
                findings,
                records: copies.len(),
                advisories_skipped: Some("cargo-deny is not installed".to_owned()),
            });
        }
        return Err("cargo-deny is not installed; the advisory check of the upstream versions \
                    needs it (`cargo install cargo-deny`)"
            .to_owned());
    }
    let mut deny = Command::new(cargo);
    deny.arg("deny");
    if offline {
        deny.arg("--offline");
    }
    deny.args(["--color", "never", "--manifest-path"]).arg(&shadow);
    deny.arg("--config").arg(root.join("deny.toml")).args(["check", "advisories"]);
    let output = run(&mut deny, "cargo deny")?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    findings.extend(advisories(output.status.success(), &text, &copies)?);
    Ok(Online { findings, records: copies.len(), advisories_skipped: None })
}

fn run(command: &mut Command, name: &str) -> Result<Output, String> {
    command.output().map_err(|e| format!("cannot run `{name}`: {e}"))
}

/// The manifest step 10 resolves: each copy's crate pinned to its version, with the
/// `default-features` and `features` of the root `[workspace.dependencies]` entry that names
/// it (by key, or by `package` for a renamed entry); default features when none does.
fn shadow_manifest(root: &Table, copies: &[Copy]) -> String {
    let quote = |text: &str| serde_json::Value::from(text).to_string();
    let declared =
        root.get("workspace").and_then(|w| w.get("dependencies")).and_then(Value::as_table);
    let mut text = format!(
        "[package]\nname = \"{SHADOW_PACKAGE}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
         publish = false\n\n\
         # Its own workspace: cargo refuses a package under the target directory that it\n\
         # believes belongs to the repository's.\n[workspace]\n\n[dependencies]\n"
    );
    for copy in copies {
        let entry = declared.into_iter().flatten().find_map(|(key, value)| {
            let name = value.get("package").and_then(Value::as_str).unwrap_or(key);
            (name == copy.name).then_some(value)
        });
        let default_features = entry
            .and_then(|e| e.get("default-features").or_else(|| e.get("default_features")))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let features: Vec<String> = entry
            .and_then(|e| e.get("features"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(quote)
            .collect();
        text.push_str(&format!(
            "{} = {{ version = {}, default-features = {default_features}, features = [{}] }}\n",
            quote(&copy.name),
            quote(&format!("={}", copy.version)),
            features.join(", ")
        ));
    }
    text
}

/// The findings of a committed record against the one `cargo vendor` re-derived from the
/// crates.io archive: one for the archive checksum, one per file that differs.
fn compare_upstream(dir: &str, committed: &Record, upstream: &Record) -> Vec<Finding> {
    let path = format!("vendor/{dir}/{RECORD}");
    let mut differ = Vec::new();
    if committed.package != upstream.package {
        differ.push("the archive's checksum".to_owned());
    }
    let files: BTreeSet<&String> = committed.files.keys().chain(upstream.files.keys()).collect();
    for file in files {
        if committed.files.get(file) != upstream.files.get(file) {
            differ.push(file.clone());
        }
    }
    differ
        .into_iter()
        .map(|what| Finding::new(&path, format!("does not match the crates.io archive ({what})")))
        .collect()
}

/// The findings of `cargo deny check advisories` from its exit status and output: none when
/// it succeeded; else one per `error[` line, against each copy whose crate the diagnostic's
/// inclusion graph names (`<name> v<version>`), or against every copy when it names none.
///
/// # Errors
/// If it failed with no `error[` line: it did not check, so there is nothing to report but
/// its output.
fn advisories(success: bool, output: &str, copies: &[Copy]) -> Result<Vec<Finding>, String> {
    if success {
        return Ok(Vec::new());
    }
    // Each diagnostic runs from its `severity[code]` line to the next one.
    let mut diagnostics: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in output.lines() {
        let starts = ["error[", "warning[", "note[", "help["].iter().any(|s| line.starts_with(s));
        if starts {
            diagnostics.push((line, Vec::new()));
        } else if let Some((_, body)) = diagnostics.last_mut() {
            body.push(line);
        }
    }
    let mut findings = Vec::new();
    for (head, body) in diagnostics.iter().filter(|(head, _)| head.starts_with("error[")) {
        let names = |copy: &&Copy| {
            let node = format!("{} v{}", copy.name, copy.version);
            body.iter().any(|line| holds_node(line, &node))
        };
        let named: Vec<&Copy> = copies.iter().filter(names).collect();
        let against = if named.is_empty() { copies.iter().collect() } else { named };
        for copy in against {
            findings.push(Finding::new(format!("vendor/{}", copy.dir), head.trim()));
        }
    }
    if findings.is_empty() && !copies.is_empty() {
        return Err(format!(
            "`cargo deny check advisories` failed without a finding: {}",
            output.trim()
        ));
    }
    Ok(findings)
}

/// Whether `line` holds `node` (`<name> v<version>`) as a whole node of a dependency tree.
fn holds_node(line: &str, node: &str) -> bool {
    let name = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || "-_".contains(c));
    let version = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || ".-+".contains(c));
    line.match_indices(node).any(|(at, _)| {
        !name(line[..at].chars().next_back()) && !version(line[at + node.len()..].chars().next())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::{CRATES_IO, registry};

    const DIR: &str = "cfg-if-1.0.5";

    const PATCH: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 line one
-line two
+// LACTOSE patch: the reason.
+line 2
 line three
";

    const ROOT: &str = r#"
[workspace]
members = ["crates/a"]
exclude = ["vendor"]

[workspace.dependencies]
cfg-if = { version = "1", default-features = false, features = ["core"] }

[patch.crates-io]
cfg-if = { path = "vendor/cfg-if-1.0.5" }
"#;

    const RECORD_JSON: &str = r#"{"files":{"Cargo.toml":"aa","src/lib.rs":"bb"},"package":"cc"}"#;

    fn repo() -> Repository {
        let mut files = Files::default();
        files.insert(ROOT_MANIFEST, ROOT);
        files.insert("crates/a/Cargo.toml", "[package]\nname = \"a\"\n");
        files.insert(".gitattributes", "vendor/** -text\n");
        files.insert(README, "# Vendored crates\n\n## cfg-if 1.0.5\n\nWhy.\n");
        files.insert(
            format!("vendor/{DIR}/Cargo.toml"),
            "[package]\nname = \"cfg-if\"\nversion = \"1.0.5\"\n",
        );
        files.insert(format!("vendor/{DIR}/{RECORD}"), RECORD_JSON);
        files.insert(format!("vendor/{DIR}.patch"), PATCH);
        Repository {
            files,
            members: vec!["crates/a/Cargo.toml".to_owned()],
            entries: vec![format!("{DIR}.patch"), DIR.to_owned(), "README.md".to_owned()],
            copies: vec![DIR.to_owned()],
            numstat: BTreeMap::new(),
        }
    }

    /// [`check`], with git's reading of each patch the test did not set taken to be the
    /// check's own, as it is for every patch these tests write that git reads at all. The
    /// integration tests run git itself.
    fn run(repo: &Repository, graph: Option<&Workspace>) -> Vec<String> {
        let mut repo = repo.clone();
        for dir in repo.copies.clone() {
            let text = repo.files.text(&format!("vendor/{dir}.patch")).map(|t| t.into_owned());
            let Some(Ok(patch)) = text.as_deref().map(Patch::parse) else { continue };
            let numstat =
                patch.files.iter().map(|f| format!("{}\t{}\t{}\0", f.added, f.removed, f.name()));
            repo.numstat.entry(dir).or_insert_with(|| Ok(numstat.collect()));
        }
        lines(&check(&repo, graph))
    }

    fn path_package(name: &str, version: &str, manifest: &str) -> ResolvedPackage {
        ResolvedPackage { source: None, manifest: manifest.to_owned(), ..registry(name, version) }
    }

    fn graph() -> Workspace {
        let mut graph = Workspace::default();
        let member =
            ResolvedPackage { member: true, ..path_package("a", "0.0.0", "crates/a/Cargo.toml") };
        graph.resolved.insert("a".to_owned(), member);
        let copy = path_package("cfg-if", "1.0.5", &format!("vendor/{DIR}/Cargo.toml"));
        graph.resolved.insert("cfg-if".to_owned(), copy);
        graph
    }

    fn lines(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(ToString::to_string).collect()
    }

    fn checked(repo: &Repository) -> Vec<String> {
        run(repo, Some(&graph()))
    }

    fn with_patch(text: &str) -> Repository {
        let mut repo = repo();
        repo.files.insert(format!("vendor/{DIR}.patch"), text);
        repo
    }

    /// `repo` without the file at `path`.
    fn without(repo: &Repository, path: &str) -> Repository {
        let mut files = Files::default();
        for kept in repo.files.paths().filter(|p| *p != path) {
            files.insert(kept, repo.files.bytes(kept).unwrap_or_default());
        }
        Repository { files, ..repo.clone() }
    }

    #[test]
    fn a_clean_copy_has_no_finding() {
        assert_eq!(checked(&repo()), Vec::<String>::new());
        assert_eq!(lines(&prose(&repo())), Vec::<String>::new());
    }

    #[test]
    fn nothing_vendored_needs_only_the_gitattributes_line() {
        let mut files = Files::default();
        files.insert(ROOT_MANIFEST, "[workspace]\n");
        let bare = Repository { files, ..Repository::default() };
        assert_eq!(
            run(&bare, Some(&Workspace::default())),
            [".gitattributes: does not mark vendor/** -text"]
        );
        assert_eq!(lines(&prose(&bare)), Vec::<String>::new());
    }

    #[test]
    fn step_1_names_every_other_patch_source() {
        let mut repo = repo();
        let root = ROOT.replace(
            "[patch.crates-io]\n",
            "[patch.\"https://example.com/x\"]\nx = { path = \"vendor/x\" }\n\n\
             [patch.crates-io]\nfar = { path = \"elsewhere/far\" }\n\
             forked = { git = \"https://example.com/forked\" }\n",
        ) + "\n[replace]\n\"y:1.0.0\" = { path = \"y\" }\n";
        repo.files.insert(ROOT_MANIFEST, root);
        repo.files.insert("crates/a/Cargo.toml", "[package]\nname = \"a\"\n[patch.crates-io]\n");
        repo.files.insert(".cargo/config.toml", "[patch.crates-io]\nz = { path = \"z\" }\n");
        repo.files.insert("crates/a/.cargo/config", "paths = [\"../z\"]\n");
        repo.files.insert("tools/.cargo/config.toml", "[build]\njobs = 2\n");
        repo.files.insert("other/.cargo/config.toml", "not = toml = at all");
        assert_eq!(
            checked(&repo),
            [
                "Cargo.toml: [patch.crates-io] \"far\" is not a path under vendor/",
                "Cargo.toml: [patch.crates-io] \"forked\" is not a path under vendor/",
                "Cargo.toml: [patch.\"https://example.com/x\"] is not allowed; only crates-io is \
             patched",
                "Cargo.toml: holds a [replace] table; a crate is replaced only by \
             [patch.crates-io] in the root Cargo.toml, under vendor/",
                "crates/a/Cargo.toml: holds a [patch] table; only the root may",
                ".cargo/config.toml: sets [patch], [source] or paths; a patch lives in the root \
             Cargo.toml only",
                "crates/a/.cargo/config: sets [patch], [source] or paths; a patch lives in the root \
             Cargo.toml only",
                "other/.cargo/config.toml: is not TOML, so it cannot be shown to set no [patch], \
             [source] or paths",
            ]
        );
    }

    #[test]
    fn a_patch_path_names_one_directory_directly_under_vendor() {
        assert_eq!(vendor_dir("vendor/cfg-if-1.0.5"), Some(DIR.to_owned()));
        assert_eq!(vendor_dir("./vendor/cfg-if-1.0.5/"), Some(DIR.to_owned()));
        assert_eq!(vendor_dir("vendor\\cfg-if-1.0.5"), Some(DIR.to_owned()));
        for path in
            ["vendor", "vendor/", "vendor/..", "vendor/a/b", "other/vendor/a", "../vendor/a"]
        {
            assert_eq!(vendor_dir(path), None, "{path}");
        }
    }

    #[test]
    fn step_2_ties_the_graph_the_entries_and_the_directories_together() {
        let mut repo = repo();
        repo.copies.push("stray-0.1.0".to_owned());
        let root = ROOT.to_owned() + "gone = { path = \"vendor/gone-1.0.0\" }\n";
        repo.files.insert(ROOT_MANIFEST, root);
        repo.files.insert(README, "## cfg-if 1.0.5\n\n## stray 0.1.0\n");
        let manifest = "[package]\nname = \"stray\"\nversion = \"0.1.0\"\n";
        repo.files.insert("vendor/stray-0.1.0/Cargo.toml", manifest);
        repo.files.insert(format!("vendor/stray-0.1.0/{RECORD}"), RECORD_JSON);
        repo.files.insert("vendor/stray-0.1.0.patch", PATCH);
        let mut graph = graph();
        let helper = path_package("helper", "0.0.0", "other/Cargo.toml");
        graph.resolved.insert("helper".to_owned(), helper);
        assert_eq!(
            run(&repo, Some(&graph)),
            [
                "other/Cargo.toml: non-member path package helper 0.0.0 outside vendor/",
                "vendor/stray-0.1.0: no [patch.crates-io] entry names it",
                "Cargo.toml: [patch.crates-io] \"gone\" names vendor/gone-1.0.0, which does not exist",
            ]
        );
    }

    #[test]
    fn step_2_names_a_copy_nothing_builds_and_a_copy_that_is_a_member() {
        let unused = Workspace::default();
        assert_eq!(
            run(&repo(), Some(&unused)),
            ["vendor/cfg-if-1.0.5: no package in the resolved graph is built from it (the patch \
             is unused)"]
        );
        let mut member = graph();
        if let Some(copy) = member.resolved.get_mut("cfg-if") {
            copy.member = true;
        }
        assert_eq!(
            run(&repo(), Some(&member)),
            ["vendor/cfg-if-1.0.5: is a workspace member; a copy is a dependency, excluded from \
             the workspace"]
        );
        // Without a graph, only what the files show.
        assert_eq!(run(&repo(), None), Vec::<String>::new());
    }

    #[test]
    fn step_3_requires_the_directory_and_the_entry_to_name_the_crate() {
        let mut repo = repo();
        repo.files.insert(
            format!("vendor/{DIR}/Cargo.toml"),
            "[package]\nname = \"cfg_if\"\nversion = \"1.0.6\"\n",
        );
        let mut graph = graph();
        if let Some(copy) = graph.resolved.get_mut("cfg-if") {
            copy.name = "cfg_if".to_owned();
            copy.version = "1.0.6".to_owned();
        }
        assert_eq!(
            run(&repo, Some(&graph)),
            [
                "vendor/cfg-if-1.0.5: holds cfg_if 1.0.6, so it must be named vendor/cfg_if-1.0.6",
                "Cargo.toml: [patch.crates-io] \"cfg-if\" names vendor/cfg-if-1.0.5, which holds \
             cfg_if",
            ]
        );
        repo.files.insert(format!("vendor/{DIR}/Cargo.toml"), "[workspace]\n");
        assert_eq!(
            checked(&repo),
            ["vendor/cfg-if-1.0.5/Cargo.toml: missing, or without a [package] name and version"]
        );
    }

    #[test]
    fn step_3_takes_the_crate_name_from_the_package_key() {
        let mut repo = repo();
        let root = ROOT.replace("cfg-if = { path", "renamed = { package = \"cfg-if\", path");
        repo.files.insert(ROOT_MANIFEST, root);
        assert_eq!(checked(&repo), Vec::<String>::new());
    }

    #[test]
    fn step_4_finds_the_name_at_another_version() {
        let mut graph = graph();
        graph.resolved.insert("cfg-if 0.1".to_owned(), registry("cfg-if", "0.1.10"));
        let other = path_package("cfg-if", "0.1.99", "vendor/cfg-if-0.1.99/Cargo.toml");
        graph.resolved.insert("cfg-if 0.1.99".to_owned(), other);
        let found = run(&repo(), Some(&graph));
        for expected in [
            "vendor/cfg-if-1.0.5: cfg-if also resolves from crates.io at 0.1.10; a patch covers \
             one version",
            "vendor/cfg-if-1.0.5: cfg-if also resolves from vendor/cfg-if-0.1.99 at 0.1.99; a \
             patch covers one version",
            "vendor/cfg-if-0.1.99: cfg-if also resolves from crates.io at 0.1.10; a patch \
             covers one version",
        ] {
            assert!(found.iter().any(|f| f == expected), "{expected}\n{found:#?}");
        }
    }

    #[test]
    fn step_5_requires_a_record_and_unconverted_bytes() {
        let mut repo = repo();
        repo.files.insert(format!("vendor/{DIR}/{RECORD}"), r#"{"files":{}}"#);
        repo.files.insert(".gitattributes", "*.rs text eol=lf\n");
        assert_eq!(
            checked(&repo),
            [
                "vendor/cfg-if-1.0.5/.cargo-checksum.json: missing, or without a \"files\" map and \
             a \"package\" checksum; write it with cargo vendor --versioned-dirs",
                ".gitattributes: does not mark vendor/** -text",
            ]
        );
        assert_eq!(Record::parse(r#"{"files":{"a":1},"package":"x"}"#), None);
        let record = Record::parse(r#"{"files":{"src\\a.rs":"1"},"package":"x"}"#);
        let keys: Vec<String> = record.into_iter().flat_map(|r| r.files.into_keys()).collect();
        assert_eq!(keys, ["src/a.rs"]);
    }

    #[test]
    fn the_last_gitattributes_line_that_reaches_vendor_decides() {
        for (text, kept) in [
            ("vendor/** -text\n", true),
            ("/vendor/** binary\n", true),
            ("* text=auto\nvendor/** -text\n", true),
            ("vendor/** -text\n*.json text\n", false),
            ("vendor/** -text\n**/*.rs text=auto\n", false),
            ("vendor/** -text\nvendor/a/** !text\n", false),
            ("vendor/** -text\ncrates/** text\n", true),
            ("vendor/** -text\n# *.rs text\n", true),
            ("vendor/** text\n", false),
            ("vendor/* -text\n", false),
            ("", false),
        ] {
            assert_eq!(keeps_vendor_bytes(text), kept, "{text:?}");
        }
    }

    #[test]
    fn step_6_requires_a_unified_diff() {
        let patch = format!("vendor/{DIR}.patch");
        assert_eq!(
            checked(&without(&repo(), &patch)),
            ["vendor/cfg-if-1.0.5.patch: missing; a copy without a patch has no reason to exist, \
             use the registry"]
        );
        for text in ["", "Just some words about a change.\n", "--- a/x\n+++ b/x\n"] {
            assert_eq!(
                checked(&with_patch(text)),
                ["vendor/cfg-if-1.0.5.patch: not a unified diff"]
            );
        }
        let binary = with_patch(&format!("{PATCH}diff --git a/x.png b/x.png\nGIT binary patch\n"));
        assert_eq!(
            checked(&binary),
            ["vendor/cfg-if-1.0.5.patch: not a unified diff (a binary patch at line 11)"]
        );
        let cut = with_patch(PATCH.strip_suffix(" line three\n").unwrap_or_default());
        assert_eq!(
            checked(&cut),
            ["vendor/cfg-if-1.0.5.patch: not a unified diff (the hunk at line 4 is cut short)"]
        );
    }

    #[test]
    fn step_7_counts_every_changed_line_comments_included() {
        let body: String = (0..48).map(|i| format!("+// LACTOSE patch: line {i}\n")).collect();
        let patch = format!(
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,50 @@\n-old\n{body}+new\n context\n"
        );
        let parsed = Patch::parse(&patch);
        assert_eq!(parsed.map(|p| (p.files[0].added, p.files[0].removed)), Ok((49, 1)));
        assert_eq!(checked(&with_patch(&patch)), Vec::<String>::new());
        let over = patch.replace("+1,50", "+1,51").replace("+new\n", "+new\n+one more\n");
        assert_eq!(
            checked(&with_patch(&over)),
            ["vendor/cfg-if-1.0.5.patch: 51 changed lines, over the fifty of architecture \
             section 8.2"]
        );
    }

    #[test]
    fn step_7_refuses_the_manifest_records_at_any_depth() {
        let patch = format!(
            "{PATCH}\
diff --git a/Cargo.toml b/Cargo.toml
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1 +1 @@
-version = \"1.0.5\"
+version = \"1.0.6\" # LACTOSE patch: no.
diff --git a/sub/Cargo.lock b/sub/Cargo.lock
deleted file mode 100644
--- a/sub/Cargo.lock
+++ /dev/null
@@ -1 +0,0 @@
-x
"
        );
        assert_eq!(
            checked(&with_patch(&patch)),
            [
                "vendor/cfg-if-1.0.5.patch: names Cargo.toml, which a patch never touches",
                "vendor/cfg-if-1.0.5.patch: names sub/Cargo.lock, which a patch never touches",
                "vendor/cfg-if-1.0.5.patch: sub/Cargo.lock carries no \"LACTOSE patch:\" notice",
            ]
        );
        let renamed = format!(
            "{PATCH}diff --git a/x b/y\nsimilarity index 100%\nrename from \
             .cargo_vcs_info.json\nrename to src/vcs.json\n"
        );
        assert_eq!(
            checked(&with_patch(&renamed)).first().map(String::as_str),
            Some(
                "vendor/cfg-if-1.0.5.patch: names .cargo_vcs_info.json, which a patch never touches"
            )
        );
    }

    #[test]
    fn step_7_refuses_added_unsafe_and_a_file_without_the_notice() {
        let patch = format!(
            "{PATCH}\
--- a/src/other.rs
+++ b/src/other.rs
@@ -10,3 +10,3 @@
 fn f() {{
+    let p = unsafe {{ g() }};
-    // unsafe was removed here
 }}
"
        );
        assert_eq!(
            checked(&with_patch(&patch)),
            [
                "vendor/cfg-if-1.0.5.patch: adds unsafe at line 14",
                "vendor/cfg-if-1.0.5.patch: src/other.rs carries no \"LACTOSE patch:\" notice",
            ]
        );
        assert!(holds_token("unsafe { x }", "unsafe"));
        assert!(holds_token("#[unsafe(no_mangle)]", "unsafe"));
        assert!(!holds_token("unsafe_op_in_unsafe_fn", "unsafe"));
        assert!(!holds_token("is_unsafe()", "unsafe"));
    }

    #[test]
    fn step_7_refuses_every_token_that_reaches_past_the_safe_language() {
        for (line, token) in [
            ("#[no_mangle]", "no_mangle"),
            ("#[export_name = \"malloc\"]", "export_name"),
            ("#[link_section = \".init_array\"]", "link_section"),
            ("    #[link_name = \"write\"]", "link_name"),
            ("global_asm!(\"nop\");", "global_asm"),
            ("core::arch::asm !(\"nop\")", "asm!"),
            ("#[naked]", "naked"),
            ("const X: &str = include!(\"x\");", "include!"),
            ("include_str! (\"x\")", "include_str!"),
            ("static B: &[u8] = include_bytes!(\"x\");", "include_bytes!"),
            ("#[path = \"../../payload.rs\"]", "#[path"),
            ("#![cfg_attr(x, path = \"y.rs\")]", "#[path"),
        ] {
            assert!(refuses(line, token), "{line}");
            let patch = format!("{PATCH}--- a/src/x.rs\n+++ b/src/x.rs\n@@ -0,0 +1 @@\n+{line}\n");
            let found = checked(&with_patch(&patch));
            let expected = format!("vendor/cfg-if-1.0.5.patch: adds {token} at line 13");
            assert!(found.contains(&expected), "{line}\n{found:#?}");
        }
        for (line, token) in [
            ("// asm is not a macro call", "asm!"),
            ("let path = include_dir;", "include!"),
            ("let path = \"x\";", "#[path"),
            ("fn no_mangled() {}", "no_mangle"),
            ("#[inline] fn naked_eye() {}", "naked"),
        ] {
            assert!(!refuses(line, token), "{line}");
        }
    }

    #[test]
    fn step_7_caps_the_length_of_an_added_line() {
        let fits = format!(
            "{PATCH}--- a/src/x.rs\n+++ b/src/x.rs\n@@ -0,0 +1 @@\n+{}\r\n",
            "a".repeat(200)
        );
        assert!(!checked(&with_patch(&fits)).iter().any(|f| f.contains("characters")));
        let long = format!(
            "{PATCH}--- a/src/x.rs\n+++ b/src/x.rs\n@@ -0,0 +1 @@\n+{}\n",
            "\u{e9}".repeat(201)
        );
        assert!(
            checked(&with_patch(&long)).contains(
                &"vendor/cfg-if-1.0.5.patch: line 13 adds 201 characters, over the 200 an added \
                  line may hold"
                    .to_owned()
            )
        );
    }

    #[test]
    fn step_7_takes_the_files_and_counts_from_git() {
        let mut repo = repo();
        repo.numstat
            .insert(DIR.to_owned(), Ok("2\t1\tsrc/lib.rs\x001\t0\tCargo.toml\0".to_owned()));
        assert_eq!(
            checked(&repo),
            [
                "vendor/cfg-if-1.0.5.patch: git applies it to Cargo.toml, which the check does \
                 not read",
                "vendor/cfg-if-1.0.5.patch: names Cargo.toml, which a patch never touches",
            ]
        );
        repo.numstat.insert(DIR.to_owned(), Ok("3\t1\tsrc/lib.rs\0".to_owned()));
        assert_eq!(
            checked(&repo),
            [
                "vendor/cfg-if-1.0.5.patch: git reads src/lib.rs as 3 added and 1 removed lines, the \
              check as 2 and 1"
            ]
        );
        repo.numstat.insert(DIR.to_owned(), Ok("-\t-\tsrc/lib.rs\0".to_owned()));
        assert_eq!(
            checked(&repo),
            ["vendor/cfg-if-1.0.5.patch: not a unified diff (git reads a binary change to \
              src/lib.rs)"]
        );
        repo.numstat.insert(DIR.to_owned(), Ok(String::new()));
        assert_eq!(
            checked(&repo),
            ["vendor/cfg-if-1.0.5.patch: the check reads a change to src/lib.rs, which git does \
              not apply"]
        );
        repo.numstat.insert(DIR.to_owned(), Err("error: corrupt patch at line 9".to_owned()));
        assert_eq!(
            checked(&repo),
            ["vendor/cfg-if-1.0.5.patch: not a patch git applies (error: corrupt patch at line 9)"]
        );
        let counted = "60\t0\tsrc/lib.rs\0";
        repo.numstat.insert(DIR.to_owned(), Ok(counted.to_owned()));
        assert!(checked(&repo).contains(
            &"vendor/cfg-if-1.0.5.patch: 60 changed lines, over the fifty of architecture section \
              8.2"
                .to_owned()
        ));
    }

    #[test]
    fn step_8_reads_a_hunk_git_moved_from_its_hunk_line_only() {
        let output = |stderr: &str| Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let clean = "Checking patch src/fuzz (offset 1).rs...\nHunk #1 applied cleanly.\n";
        assert_eq!(shifted(&output(clean)), None);
        let moved = format!("{clean}Hunk #2 succeeded at 122 (offset -10 lines).\n");
        assert_eq!(
            shifted(&output(&moved)).as_deref(),
            Some("Hunk #2 succeeded at 122 (offset -10 lines).")
        );
    }

    #[test]
    fn git_numstat_is_read_with_renames() {
        let text = "1\t2\ta.rs\x00-\t-\tlogo.png\x000\t0\t\x00old.rs\x00new dir/new.rs\x00";
        let stat = |added, removed, paths: &[&str]| Stat {
            added,
            removed,
            paths: paths.iter().map(|p| (*p).to_owned()).collect(),
        };
        assert_eq!(
            parse_numstat(text),
            [
                stat(Some(1), Some(2), &["a.rs"]),
                stat(None, None, &["logo.png"]),
                stat(Some(0), Some(0), &["old.rs", "new dir/new.rs"]),
            ]
        );
    }

    #[test]
    fn step_2_refuses_anything_else_under_vendor() {
        let mut repo = repo();
        repo.entries
            .extend(["payload.rs", "orphan-1.0.0.patch", ".gitattributes"].map(str::to_owned));
        assert_eq!(
            checked(&repo),
            [
                "vendor/payload.rs: neither a copy, its patch nor README.md; nothing else lives \
                 under vendor/",
                "vendor/orphan-1.0.0.patch: no copy vendor/orphan-1.0.0 beside it",
                "vendor/.gitattributes: neither a copy, its patch nor README.md; nothing else \
                 lives under vendor/",
            ]
        );
    }

    #[test]
    fn step_1_refuses_what_reaches_cargo_around_the_files_it_reads() {
        let mut repo = repo();
        repo.files.insert(
            ".cargo/config.toml",
            "include = [\"tools/ship.toml\"]\n[alias]\nship = \"build --config tools/ship.toml\"\n\
             nightly = [\"build\", \"-Zbuild-std\"]\nok = \"build --release\"\n",
        );
        repo.files.insert_link("tools/.cargo");
        repo.files.insert(
            ".github/workflows/ci.yml",
            "# cargo --config patch.crates-io.x.path=y\n\
             run: cargo build --config 'patch.crates-io.x.path=\"y\"'\n\
             env:\n  CARGO_HOME: /tmp/home\n\
             run: cargo build --config profile.release.lto=true\n",
        );
        repo.files
            .insert("tools/release.sh", "export CARGO_HOME=\"$PWD/home\"\necho $CARGO_HOME\n");
        assert_eq!(
            checked(&repo),
            [
                ".cargo/config.toml: has an include key; cargo reads the included files, which no \
                 check does",
                "tools/.cargo: is a symbolic link; cargo follows it, which no check does, so a \
                 cargo configuration file or directory is committed as itself",
                ".cargo/config.toml: alias nightly passes --config or -Z, through which a \
                 [patch], [source] or paths setting reaches cargo unseen",
                ".cargo/config.toml: alias ship passes --config or -Z, through which a [patch], \
                 [source] or paths setting reaches cargo unseen",
                ".github/workflows/ci.yml: line 2: passes --config naming patch, source or paths; \
                 a patch lives in the root Cargo.toml only",
                ".github/workflows/ci.yml: line 4: sets CARGO_HOME, whose cargo configuration no \
                 check reads",
                "tools/release.sh: line 1: sets CARGO_HOME, whose cargo configuration no check \
                 reads",
            ]
        );
    }

    #[test]
    fn a_patch_is_read_as_git_apply_reads_it() {
        // A removed line that begins `--` and an added one that begins `++` are hunk lines,
        // an empty line is empty context, and text between files is ignored.
        let patch = "\
Subject: a mail header that is ignored

--- a/src/a.rs\t2026-10-11 10:00:00
+++ \"b/src/sp ace\\303\\251.rs\"
@@ -1,3 +1,3 @@
 one

--- removed
+++ LACTOSE patch: added
\\ No newline at end of file
not part of a hunk
--- /dev/null
+++ b/new.rs
@@ -0,0 +1 @@
+// LACTOSE patch: new file.
";
        let parsed = Patch::parse(patch).unwrap_or_default();
        let [a, new] = &parsed.files[..] else { panic!("{parsed:?}") };
        assert_eq!(a.paths, ["src/a.rs", "src/sp ace\u{e9}.rs"]);
        assert_eq!((a.added, a.removed, a.notice, a.hunks), (1, 1, true, 1));
        assert_eq!(new.paths, ["new.rs"]);
        assert_eq!(new.target.as_deref(), Some("new.rs"));
        assert_eq!((new.added, new.removed, new.notice), (1, 0, true));
        assert_eq!(hunk_lengths("@@ -3 +3,0 @@ fn f()"), Some((1, 0)));
        assert_eq!(hunk_lengths("@@ -3,x +3 @@"), None);
        assert_eq!(
            Patch::parse("@@ -1 +1 @@\n-a\n+b\n"),
            Err(Some("the hunk at line 1 has no file header".to_owned()))
        );
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("lactose-xtask-vendor-{}-{name}", std::process::id()));
        remove(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    #[test]
    fn step_8_compares_every_file_with_the_record() -> Result<(), String> {
        let dir = temp("hash");
        let write =
            |path: &str, text: &str| fs::write(dir.join(path), text).map_err(|e| e.to_string());
        fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
        write("Cargo.toml", "same")?;
        write("src/lib.rs", "changed")?;
        write("src/added.rs", "")?;
        write(RECORD, "not hashed")?;
        let hashes = hash_tree(&dir)?;
        assert_eq!(hashes.keys().collect::<Vec<_>>(), ["Cargo.toml", "src/added.rs", "src/lib.rs"]);
        // The sha256 of "same".
        let same = "0967115f2813a3541eaef77de9d9d5773f1c0c04314b0bbfe4ff3b3b1c55b5d5";
        assert_eq!(hashes["Cargo.toml"], same);
        let record = Record {
            files: BTreeMap::from([
                ("Cargo.toml".to_owned(), same.to_owned()),
                ("src/lib.rs".to_owned(), "0".repeat(64)),
                ("README.md".to_owned(), "0".repeat(64)),
            ]),
            package: "p".to_owned(),
        };
        assert_eq!(
            lines(&compare_record(DIR, &record, &hashes)),
            [
                "vendor/cfg-if-1.0.5: the record's README.md is missing",
                "vendor/cfg-if-1.0.5/src/added.rs: is not in the record",
                "vendor/cfg-if-1.0.5/src/lib.rs: differs from the record",
            ]
        );
        let copy = temp("copy");
        copy_tree(&dir, &copy.join("x"))?;
        assert_eq!(hash_tree(&copy.join("x"))?, hashes);
        remove(&dir)?;
        remove(&copy)
    }

    #[test]
    fn step_9_requires_a_section_per_copy_and_none_for_a_gone_one() {
        let mut repo = repo();
        repo.files
            .insert(README, "## cfg-if 1.0.4\n\n```text\n## not a heading\n```\n\n## Notes\n");
        assert_eq!(
            lines(&prose(&repo)),
            [
                "vendor/README.md: has no section \"## cfg-if 1.0.5\"",
                "vendor/README.md: section \"## cfg-if 1.0.4\" names no directory under vendor/",
                "vendor/README.md: section \"## Notes\" names no directory under vendor/",
            ]
        );
        assert_eq!(
            lines(&prose(&without(&repo, README))),
            ["vendor/README.md: has no section \"## cfg-if 1.0.5\"",]
        );
    }

    fn copy(name: &str, version: &str) -> Copy {
        Copy {
            dir: format!("{name}-{version}"),
            name: name.to_owned(),
            version: version.to_owned(),
        }
    }

    #[test]
    fn step_10_pins_the_upstream_version_as_the_workspace_declares_it() {
        let shadow = |root: &str, copies: &[Copy]| -> Table {
            let root: Table = root.parse().unwrap_or_default();
            shadow_manifest(&root, copies).parse().unwrap_or_else(|e| panic!("{e}"))
        };
        let parsed = shadow(ROOT, &[copy("cfg-if", "1.0.5"), copy("b", "2.0.0")]);
        assert_eq!(parsed["package"]["name"].as_str(), Some(SHADOW_PACKAGE));
        assert_eq!(parsed["package"]["publish"].as_bool(), Some(false));
        assert_eq!(parsed["workspace"].as_table().map(Table::len), Some(0));
        assert!(!parsed.contains_key("patch"));
        let deps = &parsed["dependencies"];
        assert_eq!(deps["cfg-if"]["version"].as_str(), Some("=1.0.5"));
        assert_eq!(deps["cfg-if"]["default-features"].as_bool(), Some(false));
        assert_eq!(deps["cfg-if"]["features"][0].as_str(), Some("core"));
        // Not declared at the root: default features.
        assert_eq!(deps["b"]["version"].as_str(), Some("=2.0.0"));
        assert_eq!(deps["b"]["default-features"].as_bool(), Some(true));
        let renamed = "[workspace.dependencies]\n\
                       alias = { package = \"cfg-if\", version = \"1\", features = [\"x\"] }\n";
        let parsed = shadow(renamed, &[copy("cfg-if", "1.0.5")]);
        assert_eq!(parsed["dependencies"]["cfg-if"]["features"][0].as_str(), Some("x"));
    }

    #[test]
    fn step_10_compares_the_record_with_the_crates_io_archive() {
        let committed = Record::parse(RECORD_JSON).unwrap_or_else(|| panic!());
        assert_eq!(compare_upstream(DIR, &committed, &committed), Vec::new());
        let upstream = r#"{"files":{"Cargo.toml":"aa","src/lib.rs":"b0"},"package":"c0"}"#;
        let upstream = Record::parse(upstream).unwrap_or_else(|| panic!());
        assert_eq!(
            lines(&compare_upstream(DIR, &committed, &upstream)),
            [
                "vendor/cfg-if-1.0.5/.cargo-checksum.json: does not match the crates.io archive \
             (the archive's checksum)",
                "vendor/cfg-if-1.0.5/.cargo-checksum.json: does not match the crates.io archive \
             (src/lib.rs)",
            ]
        );
    }

    /// cargo-deny 0.20.2's diagnostics for an advisory, in the shape its probe printed them.
    const DENY_FAILED: &str = "\
error[vulnerability]: Buffer overflow in SmallVec::insert_many
   ┌─ /target/vendor/upstream/Cargo.lock:4:1
   │
 4 │ smallvec 1.6.0 registry+https://github.com/rust-lang/crates.io-index
   │ ──────────────────────────────────────────────────────────── security vulnerability detected
   │
   ├ ID: RUSTSEC-2021-0003
   ├ smallvec v1.6.0
     └── vendor-upstream v0.0.0

warning[unmaintained]: something else
   ├ cfg-if v1.0.5
     └── vendor-upstream v0.0.0

error[vulnerability]: in a crate no copy is
   ├ other v1.0.0
     └── vendor-upstream v0.0.0
advisories FAILED
";

    #[test]
    fn step_10_reads_cargo_denys_exit_status_and_error_lines() {
        let copies = [copy("cfg-if", "1.0.5"), copy("smallvec", "1.6.0")];
        assert_eq!(advisories(true, DENY_FAILED, &copies), Ok(Vec::new()));
        let found = advisories(false, DENY_FAILED, &copies).map(|f| lines(&f));
        assert_eq!(
            found,
            Ok(vec![
                "vendor/smallvec-1.6.0: error[vulnerability]: Buffer overflow in \
             SmallVec::insert_many"
                    .to_owned(),
                "vendor/cfg-if-1.0.5: error[vulnerability]: in a crate no copy is".to_owned(),
                "vendor/smallvec-1.6.0: error[vulnerability]: in a crate no copy is".to_owned(),
            ])
        );
        let unchecked = advisories(false, "advisory database not found\n", &copies);
        assert!(unchecked.is_err_and(|e| e.contains("advisory database not found")));
        assert!(holds_node("   ├ smallvec v1.6.0", "smallvec v1.6.0"));
        assert!(!holds_node("   ├ smallvec v1.6.0-rc1", "smallvec v1.6.0"));
        assert!(!holds_node("   ├ smallvec v1.6.01", "smallvec v1.6.0"));
        assert!(!holds_node("   ├ my-smallvec v1.6.0", "smallvec v1.6.0"));
    }

    #[test]
    fn a_registry_source_is_crates_io() {
        assert!(from_registry(&registry("x", "1.0.0")));
        let sparse = ResolvedPackage {
            source: Some("sparse+https://index.crates.io/".to_owned()),
            ..registry("x", "1.0.0")
        };
        assert!(from_registry(&sparse));
        assert!(!from_registry(&path_package("x", "1.0.0", "x/Cargo.toml")));
        assert!(CRATES_IO.starts_with("registry+"));
    }
}
