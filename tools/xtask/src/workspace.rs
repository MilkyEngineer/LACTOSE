//! The workspace as the checks see it: packages, their crate roots and their declared
//! dependencies, read from `cargo metadata --no-deps`.
//!
//! Declared dependencies are used, not the resolved graph, so an optional or platform-specific
//! edge is checked on every machine. Workspace crates are path dependencies that no registry
//! crate can depend on, so transitive questions about workspace crates are answered inside
//! this graph.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

/// The section of a manifest a dependency is declared in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepKind {
    /// `[dependencies]`.
    Normal,
    /// `[build-dependencies]`.
    Build,
    /// `[dev-dependencies]`.
    Dev,
}

impl DepKind {
    /// Whether the edge is compiled when the declaring crate is built as a dependency. Dev
    /// edges are not: Cargo never follows the dev-dependencies of a dependency.
    #[must_use]
    pub fn ships(self) -> bool {
        self != Self::Dev
    }
}

/// One declared dependency.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dep {
    /// The package name, not the rename.
    pub name: String,
    /// The manifest section it is declared in.
    pub kind: DepKind,
    /// True for a path dependency on another workspace member.
    pub workspace: bool,
}

/// One workspace member.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Package {
    /// The package name.
    pub name: String,
    /// The package directory relative to the workspace root, with `/` separators.
    pub dir: String,
    /// The root source file of each library, proc-macro and binary target, relative to the
    /// workspace root.
    pub crate_roots: Vec<String>,
    /// True if the package has a proc-macro target.
    pub proc_macro: bool,
    /// True if the package has a build script.
    pub build_script: bool,
    /// Every declared dependency, in manifest order.
    pub deps: Vec<Dep>,
}

impl Package {
    /// The repository-relative path of the package manifest.
    #[must_use]
    pub fn manifest(&self) -> String {
        format!("{}/Cargo.toml", self.dir)
    }
}

/// The workspace members.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Workspace {
    /// The members, sorted by name.
    pub packages: Vec<Package>,
}

impl Workspace {
    /// The member called `name`.
    #[must_use]
    pub fn package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// The shortest chain of workspace crates from the `start` crates to a crate for which
    /// `is_target` holds, following only edges that ship (normal and build). A start crate
    /// that is itself a target is a chain of one. Returns one chain per reachable target,
    /// sorted by target name.
    #[must_use]
    pub fn chains_to(&self, start: &[&str], is_target: &dyn Fn(&str) -> bool) -> Vec<Vec<String>> {
        let mut parent: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        let mut queue = VecDeque::new();
        for &name in start {
            if let Some(package) = self.package(name)
                && parent.insert(&package.name, None).is_none()
            {
                queue.push_back(package);
            }
        }
        let mut found = BTreeSet::new();
        while let Some(package) = queue.pop_front() {
            if is_target(&package.name) {
                found.insert(package.name.as_str());
            }
            for dep in package.deps.iter().filter(|d| d.workspace && d.kind.ships()) {
                if let Some(next) = self.package(&dep.name)
                    && !parent.contains_key(next.name.as_str())
                {
                    parent.insert(&next.name, Some(&package.name));
                    queue.push_back(next);
                }
            }
        }
        found
            .into_iter()
            .map(|target| {
                let mut chain = vec![target.to_owned()];
                let mut at = target;
                while let Some(Some(up)) = parent.get(at) {
                    chain.push((*up).to_owned());
                    at = up;
                }
                chain.reverse();
                chain
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct RawMetadata {
    packages: Vec<RawPackage>,
    workspace_members: Vec<String>,
    workspace_root: String,
    target_directory: String,
}

#[derive(Deserialize)]
struct RawPackage {
    name: String,
    id: String,
    manifest_path: String,
    dependencies: Vec<RawDep>,
    targets: Vec<RawTarget>,
}

#[derive(Deserialize)]
struct RawDep {
    name: String,
    kind: Option<String>,
    path: Option<String>,
}

#[derive(Deserialize)]
struct RawTarget {
    kind: Vec<String>,
    src_path: String,
}

/// What `cargo metadata` says about the workspace.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Metadata {
    /// The absolute workspace root.
    pub root: String,
    /// The absolute target directory.
    pub target_dir: String,
    /// The members.
    pub workspace: Workspace,
}

/// Parses the JSON printed by `cargo metadata --format-version 1 --no-deps`.
///
/// # Errors
/// If the text is not that JSON, or a member lies outside the workspace root.
pub fn parse_metadata(json: &str) -> Result<Metadata, String> {
    let raw: RawMetadata =
        serde_json::from_str(json).map_err(|e| format!("cargo metadata output: {e}"))?;
    let members: BTreeSet<&str> = raw.workspace_members.iter().map(String::as_str).collect();
    let raw_packages: Vec<&RawPackage> =
        raw.packages.iter().filter(|p| members.contains(p.id.as_str())).collect();
    let names: BTreeSet<&str> = raw_packages.iter().map(|p| p.name.as_str()).collect();

    let mut packages = Vec::new();
    for package in raw_packages {
        let manifest = relative(&raw.workspace_root, &package.manifest_path)?;
        let dir = manifest.strip_suffix("/Cargo.toml").unwrap_or_default().to_owned();
        let mut crate_roots = Vec::new();
        let mut proc_macro = false;
        let mut build_script = false;
        for target in &package.targets {
            let is = |kind: &str| target.kind.iter().any(|k| k == kind);
            proc_macro |= is("proc-macro");
            build_script |= is("custom-build");
            let code = ["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro", "bin"];
            if code.iter().any(|k| is(k)) {
                crate_roots.push(relative(&raw.workspace_root, &target.src_path)?);
            }
        }
        crate_roots.sort();
        crate_roots.dedup();
        let deps = package
            .dependencies
            .iter()
            .map(|dep| Dep {
                name: dep.name.clone(),
                kind: match dep.kind.as_deref() {
                    Some("build") => DepKind::Build,
                    Some("dev") => DepKind::Dev,
                    _ => DepKind::Normal,
                },
                workspace: dep.path.is_some() && names.contains(dep.name.as_str()),
            })
            .collect();
        packages.push(Package {
            name: package.name.clone(),
            dir,
            crate_roots,
            proc_macro,
            build_script,
            deps,
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Metadata {
        root: raw.workspace_root,
        target_dir: raw.target_directory,
        workspace: Workspace { packages },
    })
}

/// Runs `cargo metadata` in `dir` and parses the result.
///
/// # Errors
/// If cargo cannot be run, fails, or prints something [`parse_metadata`] rejects.
pub fn load_metadata(cargo: &str, dir: &Path) -> Result<Metadata, String> {
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run `{cargo} metadata`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{cargo} metadata` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_metadata(&String::from_utf8_lossy(&output.stdout))
}

/// `path` relative to `root`, with `/` separators.
fn relative(root: &str, path: &str) -> Result<String, String> {
    let root = root.replace('\\', "/");
    let path = path.replace('\\', "/");
    path.strip_prefix(root.trim_end_matches('/'))
        .and_then(|rest| rest.strip_prefix('/'))
        .map(str::to_owned)
        .ok_or_else(|| format!("`{path}` is outside the workspace root `{root}`"))
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::{Dep, DepKind, Package, Workspace};

    /// A package at `crates/<role>` with a forbidding library root and the given edges. A
    /// dependency is a workspace edge when its name starts with `lactose-` or `game-`.
    pub(crate) fn package(name: &str, deps: &[(&str, DepKind)]) -> Package {
        let role = name.strip_prefix("lactose-").unwrap_or(name);
        Package {
            name: name.to_owned(),
            dir: format!("crates/{role}"),
            crate_roots: vec![format!("crates/{role}/src/lib.rs")],
            proc_macro: false,
            build_script: false,
            deps: deps
                .iter()
                .map(|&(dep, kind)| Dep {
                    name: dep.to_owned(),
                    kind,
                    workspace: dep.starts_with("lactose-") || dep.starts_with("game-"),
                })
                .collect(),
        }
    }

    pub(crate) fn workspace(mut packages: Vec<Package>) -> Workspace {
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Workspace { packages }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{package, workspace};
    use super::*;

    const JSON: &str = r#"{
      "packages": [
        {"name": "lactose-core", "id": "core-id", "manifest_path": "C:\\ws\\crates\\core\\Cargo.toml",
         "dependencies": [
           {"name": "bytemuck", "kind": null, "path": null},
           {"name": "xxhash-rust", "kind": "dev", "path": null}],
         "targets": [{"kind": ["lib"], "src_path": "C:\\ws\\crates\\core\\src\\lib.rs"},
                     {"kind": ["test"], "src_path": "C:\\ws\\crates\\core\\tests\\t.rs"}]},
        {"name": "lactose-reflect-macros", "id": "macros-id", "manifest_path": "C:\\ws\\crates\\reflect-macros\\Cargo.toml",
         "dependencies": [
           {"name": "lactose-core", "kind": "build", "path": "C:\\ws\\crates\\core"}],
         "targets": [{"kind": ["proc-macro"], "src_path": "C:\\ws\\crates\\reflect-macros\\src\\lib.rs"},
                     {"kind": ["custom-build"], "src_path": "C:\\ws\\crates\\reflect-macros\\build.rs"}]},
        {"name": "stranger", "id": "stranger-id", "manifest_path": "C:\\elsewhere\\Cargo.toml",
         "dependencies": [], "targets": []}
      ],
      "workspace_members": ["core-id", "macros-id"],
      "workspace_root": "C:\\ws",
      "target_directory": "C:\\ws\\target"
    }"#;

    #[test]
    fn metadata_becomes_relative_packages() {
        let metadata = parse_metadata(JSON).unwrap();
        let ws = metadata.workspace;
        assert_eq!(ws.packages.len(), 2, "non-members are dropped");
        let core = ws.package("lactose-core").unwrap();
        assert_eq!(core.dir, "crates/core");
        assert_eq!(core.crate_roots, ["crates/core/src/lib.rs"]);
        assert!(!core.proc_macro && !core.build_script);
        assert_eq!(core.deps[1].kind, DepKind::Dev);
        assert!(!core.deps[0].workspace);
        let macros = ws.package("lactose-reflect-macros").unwrap();
        assert!(macros.proc_macro && macros.build_script);
        assert_eq!(macros.deps[0].kind, DepKind::Build);
        assert!(macros.deps[0].workspace);
        assert_eq!(macros.manifest(), "crates/reflect-macros/Cargo.toml");
    }

    #[test]
    fn chains_follow_shipping_edges_only() {
        let ws = workspace(vec![
            package("lactose-a", &[("lactose-b", DepKind::Normal), ("lactose-d", DepKind::Dev)]),
            package("lactose-b", &[("lactose-c", DepKind::Build)]),
            package("lactose-c", &[]),
            package("lactose-d", &[]),
        ]);
        let chains =
            ws.chains_to(&["lactose-a"], &|name| name == "lactose-c" || name == "lactose-d");
        assert_eq!(chains, [["lactose-a", "lactose-b", "lactose-c"]]);
        let own = ws.chains_to(&["lactose-c"], &|name| name == "lactose-c");
        assert_eq!(own, [["lactose-c"]]);
    }
}
