//! The repository's files as the checks see them: a map from repository-relative path to
//! contents, so every check that reads files can be tested on a map built in memory.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// File contents keyed by repository-relative path with `/` separators.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Files {
    files: BTreeMap<String, Vec<u8>>,
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

    /// Reads the files the checks look at from the repository at `root`.
    ///
    /// Build output, version-control metadata and nested checkouts (a directory other than
    /// `root` that has its own `.git`) are skipped, as are symbolic links.
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
            if name == ".git" || name == "target" || entry.path().join(".git").exists() {
                continue;
            }
            walk(&entry.path(), &relative, files)?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
