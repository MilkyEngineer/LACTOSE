//! The repository's files as the checks see them: a map from repository-relative path to
//! contents, so every check that reads files can be tested on a map built in memory.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::Finding;

/// The coordinator's local scratchpad at the repository root, which git ignores.
const SCRATCHPAD: &str = ".scratchpad";

/// File contents keyed by repository-relative path with `/` separators, the symbolic links
/// that were not followed, and the directories that were not entered.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Files {
    files: BTreeMap<String, Vec<u8>>,
    links: BTreeSet<String>,
    skipped: BTreeSet<String>,
}

impl Files {
    /// Adds or replaces a file.
    pub fn insert(&mut self, path: impl Into<String>, contents: impl Into<Vec<u8>>) {
        self.files.insert(path.into(), contents.into());
    }

    /// The bytes of `path`, if the file exists.
    #[must_use]
    pub fn bytes(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// The text of `path`, if the file exists. Invalid UTF-8 is replaced, not rejected.
    #[must_use]
    pub fn text(&self, path: &str) -> Option<Cow<'_, str>> {
        self.bytes(path).map(String::from_utf8_lossy)
    }

    /// Whether `path` exists.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// Every path, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Records a symbolic link at `path`, which is not followed.
    pub fn insert_link(&mut self, path: impl Into<String>) {
        self.links.insert(path.into());
    }

    /// Every symbolic link met, to a file or a directory, sorted.
    pub fn links(&self) -> impl Iterator<Item = &str> {
        self.links.iter().map(String::as_str)
    }

    /// Records a directory at `path` that was not entered.
    pub fn insert_skipped(&mut self, path: impl Into<String>) {
        self.skipped.insert(path.into());
    }

    /// Every directory not entered (build output, version-control metadata, nested
    /// checkouts, the root's scratchpad), sorted.
    pub fn skipped(&self) -> impl Iterator<Item = &str> {
        self.skipped.iter().map(String::as_str)
    }

    /// Reads the files the checks look at from the repository at `root`.
    ///
    /// Build output, version-control metadata, nested checkouts (a directory other than
    /// `root` that has its own `.git`) and the scratchpad `.scratchpad` directly under `root`
    /// are skipped and recorded. Symbolic links are recorded, not followed.
    ///
    /// # Errors
    /// If a directory or a wanted file cannot be read.
    pub fn load(root: &Path) -> Result<Self, String> {
        let mut files = Self::default();
        walk(root, "", &mut files)?;
        Ok(files)
    }
}

fn walk(dir: &Path, prefix: &str, files: &mut Files) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(|e| format!("{}: {e}", entry.path().display()))?;
        let relative = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if kind.is_dir() {
            if name == ".git"
                || name == "target"
                || (prefix.is_empty() && name == SCRATCHPAD)
                || entry.path().join(".git").exists()
            {
                files.insert_skipped(relative);
                continue;
            }
            walk(&entry.path(), &relative, files)?;
        } else if kind.is_symlink() {
            files.insert_link(relative);
        } else if kind.is_file() && wanted(&relative) {
            let contents =
                fs::read(entry.path()).map_err(|e| format!("{}: {e}", entry.path().display()))?;
            files.insert(relative, contents);
        }
    }
    Ok(())
}

/// Whether a check reads the file: Rust sources, manifests and configuration, the two
/// per-crate documents, and everything [`is_build_configuration`] names.
fn wanted(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".rs")
        || name.ends_with(".toml")
        || name == "README.md"
        || name == "SAFETY.md"
        || is_build_configuration(path)
}

/// Whether the file can decide how cargo is invoked: a cargo configuration file, anything
/// under `.github`, or a script.
#[must_use]
pub fn is_build_configuration(path: &str) -> bool {
    const SCRIPT_EXTENSIONS: [&str; 9] =
        [".sh", ".bash", ".zsh", ".fish", ".ps1", ".bat", ".cmd", ".py", ".mk"];
    const SCRIPT_NAMES: [&str; 5] = ["Makefile", "makefile", "justfile", "Justfile", "Dockerfile"];
    let name = path.rsplit('/').next().unwrap_or(path);
    is_cargo_config(path)
        || ((path.starts_with(".github/") || path.contains("/.github/"))
            && (name.ends_with(".yml") || name.ends_with(".yaml")))
        || SCRIPT_EXTENSIONS.iter().any(|ext| name.ends_with(ext))
        || SCRIPT_NAMES.contains(&name)
}

/// Whether the file is a cargo configuration file, at any depth.
#[must_use]
pub fn is_cargo_config(path: &str) -> bool {
    ["config.toml", "config"].iter().any(|file| {
        path.strip_suffix(file).is_some_and(|dir| dir == ".cargo/" || dir.ends_with("/.cargo/"))
    })
}

/// What cargo reads through a cargo configuration file that no check can read (architecture
/// section 8.2, D123): an `include` key, which makes cargo load further files, and a cargo
/// configuration file or `.cargo` directory committed as a symbolic link, which cargo follows
/// and [`Files::load`] does not. Through either, a `[source]` replacement or a `rustflags`
/// key would reach every build unseen, so each is refused wherever it is.
#[must_use]
pub fn unread_cargo_configuration(files: &Files) -> Vec<Finding> {
    let mut findings = Vec::new();
    for path in files.paths().filter(|path| is_cargo_config(path)) {
        let table = files.text(path).and_then(|text| text.parse::<toml::Table>().ok());
        if table.is_some_and(|table| table.contains_key("include")) {
            findings.push(Finding::new(
                path,
                "has an include key; cargo reads the included files, which no check does",
            ));
        }
    }
    let linked = |path: &&str| is_cargo_config(path) || path.rsplit('/').next() == Some(".cargo");
    for path in files.links().filter(linked) {
        findings.push(Finding::new(
            path,
            "is a symbolic link; cargo follows it, which no check does, so a cargo \
             configuration file or directory is committed as itself",
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_include_or_a_linked_configuration_is_refused() {
        let mut files = Files::default();
        files.insert(".cargo/config.toml", "include = [\"../tools/settings.toml\"]\n");
        files.insert("a/.cargo/config", "[build]\njobs = 1\n");
        files.insert("docs/include.toml", "include = 1\n");
        files.insert_link("b/.cargo");
        files.insert_link("c/.cargo/config");
        files.insert_link("d/.cargo/config.toml");
        files.insert_link("e/notes.md");
        let found: Vec<String> =
            unread_cargo_configuration(&files).iter().map(ToString::to_string).collect();
        let link = "is a symbolic link; cargo follows it, which no check does, so a cargo \
                    configuration file or directory is committed as itself";
        assert_eq!(
            found,
            [
                ".cargo/config.toml: has an include key; cargo reads the included files, which no \
             check does"
                    .to_owned(),
                format!("b/.cargo: {link}"),
                format!("c/.cargo/config: {link}"),
                format!("d/.cargo/config.toml: {link}"),
            ]
        );
    }

    #[test]
    fn the_root_scratchpad_is_skipped_and_a_nested_one_is_read() -> Result<(), String> {
        let root = std::env::temp_dir()
            .join(format!("lactose-xtask-repo-{}-scratchpad", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
        }
        let script = "RUSTFLAGS=-Ctarget-cpu=native cargo build\n";
        for dir in [".scratchpad", "crates/a/.scratchpad"] {
            fs::create_dir_all(root.join(dir)).map_err(|e| e.to_string())?;
            fs::write(root.join(dir).join("x.sh"), script).map_err(|e| e.to_string())?;
        }
        let files = Files::load(&root);
        fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
        let files = files?;
        assert_eq!(files.paths().collect::<Vec<_>>(), ["crates/a/.scratchpad/x.sh"]);
        assert_eq!(files.skipped().collect::<Vec<_>>(), [".scratchpad"]);
        Ok(())
    }

    #[test]
    fn build_configuration_is_recognised() {
        for path in [
            ".cargo/config.toml",
            "tools/cook/.cargo/config",
            ".github/workflows/ci.yml",
            ".github/actions/setup/action.yaml",
            "scripts/bench.sh",
            "scripts/bench.ps1",
            "Makefile",
            "tools/justfile",
        ] {
            assert!(is_build_configuration(path), "{path}");
        }
        for path in ["Cargo.toml", "docs/config.toml", "crates/core/src/lib.rs", "notes.yml"] {
            assert!(!is_build_configuration(path), "{path}");
        }
    }
}
