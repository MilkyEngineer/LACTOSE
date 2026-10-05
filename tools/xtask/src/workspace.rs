//! The workspace as the checks see it, read from `cargo metadata`: the members with their
//! crate roots and declared dependencies, and the resolved dependency graph.
//!
//! The tier rules are checked on declared dependencies, so an optional or platform-specific
//! edge is checked on every machine. Only questions about third-party crates behind those
//! edges (which crates a boundary crate ends up compiling) use the resolved graph.

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

/// Where a declared dependency comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepSource {
    /// A path dependency on a workspace member.
    Member,
    /// A path dependency on a package that is not a workspace member.
    Path,
    /// A registry or git dependency.
    External,
}

/// One declared dependency.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dep {
    /// The package name, not the rename.
    pub name: String,
    /// The manifest section it is declared in.
    pub kind: DepKind,
    /// Where it comes from.
    pub source: DepSource,
}

/// One workspace member.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Package {
    /// The package name.
    pub name: String,
    /// The package directory relative to the workspace root, with `/` separators.
    pub dir: String,
    /// The root source file of every target (library, binaries, tests, benches, examples and
    /// the build script), relative to the workspace root. Each is a crate of its own.
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

/// One package of the resolved graph.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedPackage {
    /// The package name.
    pub name: String,
    /// True for a workspace member.
    pub member: bool,
    /// The package IDs of its normal and build dependencies, with every feature enabled.
    pub deps: Vec<String>,
}

/// The workspace members and the resolved graph they sit in.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Workspace {
    /// The members, sorted by name.
    pub packages: Vec<Package>,
    /// The resolved graph, keyed by package ID. Empty when the metadata carried none.
    pub resolved: BTreeMap<String, ResolvedPackage>,
}

impl Workspace {
    /// The member called `name`.
    #[must_use]
    pub fn package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// The shortest chain of crate names from the `start` crates to each crate for which
    /// `is_target` holds, following declared edges that ship (normal and build). A start
    /// crate that is itself a target is a chain of one. A name that is no workspace member
    /// can end a chain but is not followed further. Sorted by target name.
    #[must_use]
    pub fn chains_to(&self, start: &[&str], is_target: &dyn Fn(&str) -> bool) -> Vec<Vec<String>> {
        let mut parent: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        for &name in start {
            if parent.insert(name, None).is_none() {
                queue.push_back(name);
            }
        }
        let mut found = BTreeSet::new();
        while let Some(name) = queue.pop_front() {
            if is_target(name) {
                found.insert(name);
            }
            let Some(package) = self.package(name) else {
                continue;
            };
            for dep in package.deps.iter().filter(|d| d.kind.ships()) {
                if !parent.contains_key(dep.name.as_str()) {
                    parent.insert(&dep.name, Some(&package.name));
                    queue.push_back(&dep.name);
                }
            }
        }
        found.into_iter().map(|target| chain(&parent, target)).collect()
    }

    /// Every third-party crate that the member `name` compiles through normal and build
    /// edges, at any depth and through any workspace crate, each with the shortest chain of
    /// crate names that reaches it. Empty when the metadata carried no resolved graph.
    #[must_use]
    pub fn third_party_closure(&self, name: &str) -> BTreeMap<String, Vec<String>> {
        let Some((start, _)) = self.resolved.iter().find(|(_, p)| p.member && p.name == name)
        else {
            return BTreeMap::new();
        };
        let mut parent: BTreeMap<&str, Option<&str>> = BTreeMap::from([(start.as_str(), None)]);
        let mut queue = VecDeque::from([start.as_str()]);
        let mut closure = BTreeMap::new();
        while let Some(id) = queue.pop_front() {
            let Some(package) = self.resolved.get(id) else {
                continue;
            };
            if !package.member && !closure.contains_key(&package.name) {
                let ids = chain(&parent, id);
                let names =
                    ids.iter().filter_map(|id| self.resolved.get(id)).map(|p| p.name.clone());
                closure.insert(package.name.clone(), names.collect());
            }
            for dep in &package.deps {
                if !parent.contains_key(dep.as_str()) {
                    parent.insert(dep, Some(id));
                    queue.push_back(dep);
                }
            }
        }
        closure
    }
}

/// The path from a search root to `end`, following `parent` links.
fn chain(parent: &BTreeMap<&str, Option<&str>>, end: &str) -> Vec<String> {
    let mut chain = vec![end.to_owned()];
    let mut at = end;
    while let Some(Some(up)) = parent.get(at) {
        chain.push((*up).to_owned());
        at = up;
    }
    chain.reverse();
    chain
}

#[derive(Deserialize)]
struct RawMetadata {
    packages: Vec<RawPackage>,
    workspace_members: Vec<String>,
    workspace_root: String,
    target_directory: String,
    resolve: Option<RawResolve>,
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

#[derive(Deserialize)]
struct RawResolve {
    nodes: Vec<RawNode>,
}

#[derive(Deserialize)]
struct RawNode {
    id: String,
    deps: Vec<RawNodeDep>,
}

#[derive(Deserialize)]
struct RawNodeDep {
    pkg: String,
    dep_kinds: Vec<RawDepKind>,
}

#[derive(Deserialize)]
struct RawDepKind {
    kind: Option<String>,
}

/// What `cargo metadata` says about the workspace.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Metadata {
    /// The absolute workspace root.
    pub root: String,
    /// The absolute target directory.
    pub target_dir: String,
    /// The members and the resolved graph.
    pub workspace: Workspace,
}

/// Parses the JSON printed by `cargo metadata --format-version 1`.
///
/// # Errors
/// If the text is not that JSON, or a member lies outside the workspace root.
pub fn parse_metadata(json: &str) -> Result<Metadata, String> {
    let raw: RawMetadata =
        serde_json::from_str(json).map_err(|e| format!("cargo metadata output: {e}"))?;
    let members: BTreeSet<&str> = raw.workspace_members.iter().map(String::as_str).collect();
    let member_names: BTreeSet<&str> = raw
        .packages
        .iter()
        .filter(|p| members.contains(p.id.as_str()))
        .map(|p| p.name.as_str())
        .collect();

    let mut packages = Vec::new();
    for package in raw.packages.iter().filter(|p| members.contains(p.id.as_str())) {
        let manifest = relative(&raw.workspace_root, &package.manifest_path)?;
        let dir = manifest.strip_suffix("/Cargo.toml").unwrap_or_default().to_owned();
        let mut crate_roots = Vec::new();
        let mut proc_macro = false;
        let mut build_script = false;
        for target in &package.targets {
            let is = |kind: &str| target.kind.iter().any(|k| k == kind);
            proc_macro |= is("proc-macro");
            build_script |= is("custom-build");
            crate_roots.push(relative(&raw.workspace_root, &target.src_path)?);
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
                source: match &dep.path {
                    Some(_) if member_names.contains(dep.name.as_str()) => DepSource::Member,
                    Some(_) => DepSource::Path,
                    None => DepSource::External,
                },
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

    let names: BTreeMap<&str, &str> =
        raw.packages.iter().map(|p| (p.id.as_str(), p.name.as_str())).collect();
    let mut resolved = BTreeMap::new();
    for node in raw.resolve.iter().flat_map(|resolve| &resolve.nodes) {
        let ships = |dep: &&RawNodeDep| {
            dep.dep_kinds.is_empty()
                || dep.dep_kinds.iter().any(|k| k.kind.as_deref() != Some("dev"))
        };
        resolved.insert(
            node.id.clone(),
            ResolvedPackage {
                name: names.get(node.id.as_str()).copied().unwrap_or(&node.id).to_owned(),
                member: members.contains(node.id.as_str()),
                deps: node.deps.iter().filter(ships).map(|dep| dep.pkg.clone()).collect(),
            },
        );
    }
    Ok(Metadata {
        root: raw.workspace_root,
        target_dir: raw.target_directory,
        workspace: Workspace { packages, resolved },
    })
}

/// Runs `cargo metadata` in `dir`, with every feature enabled so optional dependencies are in
/// the resolved graph, and parses the result.
///
/// # Errors
/// If cargo cannot be run, fails (it does if `Cargo.lock` is out of date), or prints
/// something [`parse_metadata`] rejects.
pub fn load_metadata(cargo: &str, dir: &Path) -> Result<Metadata, String> {
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--all-features", "--locked"])
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
    use super::{Dep, DepKind, DepSource, Package, ResolvedPackage, Workspace};

    /// A package at `crates/<role>` with a library root and the given edges. A dependency is
    /// a workspace member when its name starts with `lactose-` or `game-`.
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
                .map(|&(dep, kind)| Dep { name: dep.to_owned(), kind, source: source_of(dep) })
                .collect(),
        }
    }

    fn source_of(name: &str) -> DepSource {
        if name.starts_with("lactose-") || name.starts_with("game-") {
            DepSource::Member
        } else {
            DepSource::External
        }
    }

    /// A workspace of `packages` whose resolved graph mirrors their shipping edges, extended
    /// by `third_party`: the dependencies of crates outside the workspace.
    pub(crate) fn workspace_with(
        mut packages: Vec<Package>,
        third_party: &[(&str, &[&str])],
    ) -> Workspace {
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        let mut resolved = std::collections::BTreeMap::new();
        for package in &packages {
            let deps = package.deps.iter().filter(|d| d.kind.ships()).map(|d| d.name.clone());
            resolved.insert(
                package.name.clone(),
                ResolvedPackage { name: package.name.clone(), member: true, deps: deps.collect() },
            );
            for dep in package.deps.iter().filter(|d| d.source != DepSource::Member) {
                resolved.entry(dep.name.clone()).or_insert_with(|| ResolvedPackage {
                    name: dep.name.clone(),
                    member: false,
                    deps: Vec::new(),
                });
            }
        }
        for &(name, deps) in third_party {
            let deps: Vec<String> = deps.iter().map(|d| (*d).to_owned()).collect();
            for dep in &deps {
                resolved.entry(dep.clone()).or_insert_with(|| ResolvedPackage {
                    name: dep.clone(),
                    member: false,
                    deps: Vec::new(),
                });
            }
            resolved.insert(
                name.to_owned(),
                ResolvedPackage { name: name.to_owned(), member: false, deps },
            );
        }
        Workspace { packages, resolved }
    }

    pub(crate) fn workspace(packages: Vec<Package>) -> Workspace {
        workspace_with(packages, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{package, workspace, workspace_with};
    use super::*;

    const JSON: &str = r#"{
      "packages": [
        {"name": "lactose-core", "id": "core-id", "manifest_path": "C:\\ws\\crates\\core\\Cargo.toml",
         "dependencies": [
           {"name": "bytemuck", "kind": null, "path": null},
           {"name": "xxhash-rust", "kind": "dev", "path": null},
           {"name": "shim", "kind": null, "path": "C:\\elsewhere"}],
         "targets": [{"kind": ["lib"], "src_path": "C:\\ws\\crates\\core\\src\\lib.rs"},
                     {"kind": ["test"], "src_path": "C:\\ws\\crates\\core\\tests\\t.rs"}]},
        {"name": "lactose-reflect-macros", "id": "macros-id", "manifest_path": "C:\\ws\\crates\\reflect-macros\\Cargo.toml",
         "dependencies": [
           {"name": "lactose-core", "kind": "build", "path": "C:\\ws\\crates\\core"}],
         "targets": [{"kind": ["proc-macro"], "src_path": "C:\\ws\\crates\\reflect-macros\\src\\lib.rs"},
                     {"kind": ["custom-build"], "src_path": "C:\\ws\\crates\\reflect-macros\\build.rs"}]},
        {"name": "bytemuck", "id": "bytemuck-id", "manifest_path": "C:\\registry\\bytemuck\\Cargo.toml",
         "dependencies": [], "targets": []},
        {"name": "bytemuck_derive", "id": "derive-id", "manifest_path": "C:\\registry\\derive\\Cargo.toml",
         "dependencies": [], "targets": []},
        {"name": "xxhash-rust", "id": "xxhash-id", "manifest_path": "C:\\registry\\xxhash\\Cargo.toml",
         "dependencies": [], "targets": []}
      ],
      "workspace_members": ["core-id", "macros-id"],
      "workspace_root": "C:\\ws",
      "target_directory": "C:\\ws\\target",
      "resolve": {"nodes": [
        {"id": "core-id", "deps": [
          {"pkg": "bytemuck-id", "dep_kinds": [{"kind": null}]},
          {"pkg": "xxhash-id", "dep_kinds": [{"kind": "dev"}]}]},
        {"id": "macros-id", "deps": [{"pkg": "core-id", "dep_kinds": [{"kind": "build"}]}]},
        {"id": "bytemuck-id", "deps": [{"pkg": "derive-id", "dep_kinds": [{"kind": null}]}]},
        {"id": "derive-id", "deps": []},
        {"id": "xxhash-id", "deps": []}
      ]}
    }"#;

    #[test]
    fn metadata_becomes_relative_packages() {
        let ws = parse_metadata(JSON).unwrap().workspace;
        assert_eq!(ws.packages.len(), 2, "non-members are dropped");
        let core = ws.package("lactose-core").unwrap();
        assert_eq!(core.dir, "crates/core");
        assert_eq!(core.crate_roots, ["crates/core/src/lib.rs", "crates/core/tests/t.rs"]);
        assert!(!core.proc_macro && !core.build_script);
        let sources: Vec<DepSource> = core.deps.iter().map(|d| d.source).collect();
        assert_eq!(sources, [DepSource::External, DepSource::External, DepSource::Path]);
        assert_eq!(core.deps[1].kind, DepKind::Dev);
        let macros = ws.package("lactose-reflect-macros").unwrap();
        assert!(macros.proc_macro && macros.build_script);
        assert_eq!(
            macros.crate_roots,
            ["crates/reflect-macros/build.rs", "crates/reflect-macros/src/lib.rs"]
        );
        assert_eq!(
            (macros.deps[0].kind, macros.deps[0].source),
            (DepKind::Build, DepSource::Member)
        );
        assert_eq!(macros.manifest(), "crates/reflect-macros/Cargo.toml");
    }

    #[test]
    fn the_third_party_closure_follows_the_resolved_graph_without_dev_edges() {
        let ws = parse_metadata(JSON).unwrap().workspace;
        let closure = ws.third_party_closure("lactose-reflect-macros");
        let expect = BTreeMap::from([
            (
                "bytemuck".to_owned(),
                vec![
                    "lactose-reflect-macros".to_owned(),
                    "lactose-core".to_owned(),
                    "bytemuck".to_owned(),
                ],
            ),
            (
                "bytemuck_derive".to_owned(),
                ["lactose-reflect-macros", "lactose-core", "bytemuck", "bytemuck_derive"]
                    .map(str::to_owned)
                    .to_vec(),
            ),
        ]);
        assert_eq!(closure, expect, "xxhash-rust is a dev edge");
        assert!(ws.third_party_closure("no-such-crate").is_empty());
    }

    #[test]
    fn the_fixture_graph_reaches_through_third_party_crates() {
        let ws = workspace_with(
            vec![package("lactose-a", &[("serde", DepKind::Normal), ("rand", DepKind::Dev)])],
            &[("serde", &["serde_derive"])],
        );
        let closure: Vec<String> = ws.third_party_closure("lactose-a").into_keys().collect();
        assert_eq!(closure, ["serde", "serde_derive"]);
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
        // A name with no package behind it still ends a chain.
        let outside = ws.chains_to(&["lactose-b"], &|name| name == "lactose-c");
        assert_eq!(outside, [["lactose-b", "lactose-c"]]);
    }
}
