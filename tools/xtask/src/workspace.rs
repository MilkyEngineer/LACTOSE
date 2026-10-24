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
    /// The name the declaring package uses for it, when it renames it.
    pub rename: Option<String>,
    /// The features the declaration turns on, `default` aside.
    pub features: Vec<String>,
}

impl Dep {
    /// The name the declaring package's `[features]` values use for the dependency.
    #[must_use]
    pub fn key(&self) -> &str {
        self.rename.as_deref().unwrap_or(&self.name)
    }
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
    /// The `[features]` table: each feature and the values it turns on.
    pub features: BTreeMap<String, Vec<String>>,
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
    /// The package version.
    pub version: String,
    /// The source cargo took it from (`registry+https://...` for crates.io), or `None` for a
    /// path package: a member, or a dependency given by `path` or by a `[patch]` path.
    pub source: Option<String>,
    /// The package manifest: relative to the workspace root with `/` separators when it is
    /// inside it, else absolute.
    pub manifest: String,
    /// True for a workspace member.
    pub member: bool,
    /// The package IDs of its normal and build dependencies, under the workspace's own
    /// feature resolution: an optional dependency none of its activated features turns on is
    /// left out.
    pub deps: Vec<String>,
}

/// The workspace members and the resolved graph they sit in.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Workspace {
    /// The members, sorted by name.
    pub packages: Vec<Package>,
    /// The resolved graph, keyed by package ID. Empty when the metadata carried none.
    pub resolved: BTreeMap<String, ResolvedPackage>,
    /// What the `default` feature of each package in the metadata turns on, keyed as
    /// `cargo tree` prints the package (`name vX.Y.Z`); absent for a package without one.
    pub defaults: BTreeMap<String, Vec<String>>,
    /// The directory holding each package's manifest, keyed as `defaults` is.
    pub dirs: BTreeMap<String, String>,
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

    /// The shortest chain of crate names from `start` to each crate it reaches through
    /// declared edges that ship (normal and build), passing only through workspace members
    /// for which `through` holds: a crate for which it does not hold ends its chains. `start`
    /// itself is always left and is not reported. Sorted by the crate reached.
    #[must_use]
    pub fn reach(&self, start: &str, through: &dyn Fn(&str) -> bool) -> Vec<Vec<String>> {
        let mut parent: BTreeMap<&str, Option<&str>> = BTreeMap::from([(start, None)]);
        let mut queue = VecDeque::from([start]);
        while let Some(name) = queue.pop_front() {
            let Some(package) = self.package(name) else {
                continue;
            };
            for dep in package.deps.iter().filter(|d| d.kind.ships()) {
                if parent.contains_key(dep.name.as_str()) {
                    continue;
                }
                parent.insert(&dep.name, Some(&package.name));
                if through(&dep.name) {
                    queue.push_back(&dep.name);
                }
            }
        }
        parent.keys().filter(|&&name| name != start).map(|name| chain(&parent, name)).collect()
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
    version: String,
    id: String,
    source: Option<String>,
    manifest_path: String,
    dependencies: Vec<RawDep>,
    targets: Vec<RawTarget>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct RawDep {
    name: String,
    kind: Option<String>,
    path: Option<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    rename: Option<String>,
    #[serde(default)]
    features: Vec<String>,
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
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Deserialize)]
struct RawNodeDep {
    /// The name the depending crate's code uses: the rename, or the library's name.
    #[serde(default)]
    name: String,
    pkg: String,
    dep_kinds: Vec<RawDepKind>,
}

/// Whether `node`'s package compiles its edge `dep` (to the package `to`) when built as a
/// dependency.
///
/// The resolved graph of `cargo metadata` can hold an edge to an optional dependency that
/// the node's own resolved features leave off (in Rapier's tree: parry's `spade`,
/// nalgebra's older glam versions, num-rational's `num-bigint`, postcard's `embedded-io`); no
/// build compiles it. An optional dependency is compiled when one of the package's activated
/// features turns it on: its implicit feature, `dep:<name>`, or `<name>/<feature>` (not
/// `<name>?/<feature>`). An edge no declaration can be matched to is kept.
fn compiles(package: &RawPackage, node: &RawNode, dep: &RawNodeDep, to: &str) -> bool {
    let activated: BTreeSet<&str> = node.features.iter().map(String::as_str).collect();
    let turned_on = |key: &str| {
        activated.contains(key)
            || activated.iter().filter_map(|f| package.features.get(*f)).flatten().any(|value| {
                value.strip_prefix("dep:") == Some(key)
                    || value.split_once('/').is_some_and(|(name, _)| name == key)
            })
    };
    let mut matched = package
        .dependencies
        .iter()
        .filter(|d| d.kind.as_deref() != Some("dev"))
        .filter(|d| match &d.rename {
            Some(rename) => rename.replace('-', "_") == dep.name,
            None => d.name == to,
        })
        .peekable();
    if matched.peek().is_none() {
        return true;
    }
    matched.any(|d| !d.optional || turned_on(d.rename.as_deref().unwrap_or(&d.name)))
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
                rename: dep.rename.clone(),
                features: dep.features.clone(),
            })
            .collect();
        packages.push(Package {
            name: package.name.clone(),
            dir,
            crate_roots,
            proc_macro,
            build_script,
            deps,
            features: package.features.clone(),
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    let by_id: BTreeMap<&str, &RawPackage> =
        raw.packages.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut resolved = BTreeMap::new();
    for node in raw.resolve.iter().flat_map(|resolve| &resolve.nodes) {
        let package = by_id
            .get(node.id.as_str())
            .copied()
            .ok_or_else(|| format!("cargo metadata output: no package `{}`", node.id))?;
        let ships = |dep: &&RawNodeDep| {
            let shipping_kind = dep.dep_kinds.is_empty()
                || dep.dep_kinds.iter().any(|k| k.kind.as_deref() != Some("dev"));
            let to = by_id.get(dep.pkg.as_str()).map_or(dep.pkg.as_str(), |p| p.name.as_str());
            shipping_kind && compiles(package, node, dep, to)
        };
        resolved.insert(
            node.id.clone(),
            ResolvedPackage {
                name: package.name.clone(),
                version: package.version.clone(),
                source: package.source.clone(),
                manifest: relative(&raw.workspace_root, &package.manifest_path)
                    .unwrap_or_else(|_| package.manifest_path.replace('\\', "/")),
                member: members.contains(node.id.as_str()),
                deps: node.deps.iter().filter(ships).map(|dep| dep.pkg.clone()).collect(),
            },
        );
    }
    let defaults = raw
        .packages
        .iter()
        .filter_map(|p| Some((format!("{} v{}", p.name, p.version), p.features.get("default")?)))
        .map(|(package, default)| (package, default.clone()))
        .collect();
    let dirs = raw
        .packages
        .iter()
        .map(|p| {
            let dir = p.manifest_path.strip_suffix("Cargo.toml").unwrap_or(&p.manifest_path);
            (format!("{} v{}", p.name, p.version), dir.trim_end_matches(['/', '\\']).to_owned())
        })
        .collect();
    Ok(Metadata {
        root: raw.workspace_root,
        target_dir: raw.target_directory,
        workspace: Workspace { packages, resolved, defaults, dirs },
    })
}

/// Runs `cargo metadata` in `dir` and parses the result.
///
/// Features are resolved as a build of the workspace resolves them: the union of what every
/// member asks of its dependencies, which is what the shipped binaries compile once they
/// exist. `--all-features` is deliberately not passed. It would select features that only a
/// test job enables (`lactose-task/loom` brings `loom`, `tracing-subscriber` and their trees
/// into every boundary crate that depends on `task`), and the third-party closure of a
/// boundary crate would then name crates no boundary build compiles.
///
/// # Errors
/// If cargo cannot be run, fails (it does if `Cargo.lock` is out of date), or prints
/// something [`parse_metadata`] rejects.
pub fn load_metadata(cargo: &str, dir: &Path) -> Result<Metadata, String> {
    cargo_metadata(cargo, dir, &[])
}

/// [`load_metadata`] with `extra` arguments for `cargo metadata`: `--no-deps` reads the
/// members without resolving the graph, so no `[patch]` path is loaded, and `--offline`
/// keeps cargo off the network.
///
/// # Errors
/// As [`load_metadata`].
pub fn cargo_metadata(cargo: &str, dir: &Path, extra: &[&str]) -> Result<Metadata, String> {
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .args(extra)
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
                .map(|&(dep, kind)| Dep {
                    name: dep.to_owned(),
                    kind,
                    source: source_of(dep),
                    rename: None,
                    features: Vec::new(),
                })
                .collect(),
            features: std::collections::BTreeMap::new(),
        }
    }

    fn source_of(name: &str) -> DepSource {
        if name.starts_with("lactose-") || name.starts_with("game-") {
            DepSource::Member
        } else {
            DepSource::External
        }
    }

    /// The source cargo reports for a crates.io package.
    pub(crate) const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

    /// A crates.io package of the resolved graph with no dependencies.
    pub(crate) fn registry(name: &str, version: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            source: Some(CRATES_IO.to_owned()),
            manifest: format!("/registry/{name}-{version}/Cargo.toml"),
            member: false,
            deps: Vec::new(),
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
                ResolvedPackage {
                    source: None,
                    manifest: package.manifest(),
                    member: true,
                    deps: deps.collect(),
                    ..registry(&package.name, "0.0.0")
                },
            );
            for dep in package.deps.iter().filter(|d| d.source != DepSource::Member) {
                resolved.entry(dep.name.clone()).or_insert_with(|| registry(&dep.name, "1.0.0"));
            }
        }
        for &(name, deps) in third_party {
            let deps: Vec<String> = deps.iter().map(|d| (*d).to_owned()).collect();
            for dep in &deps {
                resolved.entry(dep.clone()).or_insert_with(|| registry(dep, "1.0.0"));
            }
            resolved.insert(name.to_owned(), ResolvedPackage { deps, ..registry(name, "1.0.0") });
        }
        Workspace {
            packages,
            resolved,
            defaults: std::collections::BTreeMap::new(),
            dirs: std::collections::BTreeMap::new(),
        }
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
        {"name": "lactose-core", "version": "0.0.0", "id": "core-id", "manifest_path": "C:\\ws\\crates\\core\\Cargo.toml",
         "dependencies": [
           {"name": "bytemuck", "kind": null, "path": null},
           {"name": "xxhash-rust", "kind": "dev", "path": null},
           {"name": "shim", "kind": null, "path": "C:\\elsewhere", "rename": "shim2", "features": ["fast"]}],
         "features": {"default": ["extra"], "extra": ["shim2/fast"]},
         "targets": [{"kind": ["lib"], "src_path": "C:\\ws\\crates\\core\\src\\lib.rs"},
                     {"kind": ["test"], "src_path": "C:\\ws\\crates\\core\\tests\\t.rs"}]},
        {"name": "lactose-reflect-macros", "version": "0.0.0", "id": "macros-id", "manifest_path": "C:\\ws\\crates\\reflect-macros\\Cargo.toml",
         "dependencies": [
           {"name": "lactose-core", "kind": "build", "path": "C:\\ws\\crates\\core"}],
         "targets": [{"kind": ["proc-macro"], "src_path": "C:\\ws\\crates\\reflect-macros\\src\\lib.rs"},
                     {"kind": ["custom-build"], "src_path": "C:\\ws\\crates\\reflect-macros\\build.rs"}]},
        {"name": "bytemuck", "version": "1.25.2", "id": "bytemuck-id", "source": "registry+https://github.com/rust-lang/crates.io-index",
         "manifest_path": "C:\\registry\\bytemuck\\Cargo.toml",
         "dependencies": [], "targets": []},
        {"name": "bytemuck_derive", "version": "1.12.1", "id": "derive-id", "source": null,
         "manifest_path": "C:\\ws\\vendor\\bytemuck_derive-1.12.1\\Cargo.toml",
         "dependencies": [], "targets": []},
        {"name": "xxhash-rust", "version": "0.8.19", "id": "xxhash-id", "manifest_path": "C:\\registry\\xxhash\\Cargo.toml",
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
        assert_eq!(
            (core.deps[2].key(), core.deps[2].features.as_slice()),
            ("shim2", &["fast".to_owned()][..])
        );
        assert_eq!((core.deps[0].key(), core.deps[0].rename.as_deref()), ("bytemuck", None));
        assert_eq!(core.features["extra"], ["shim2/fast"]);
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
    fn the_resolved_graph_carries_versions_sources_and_manifests() {
        let ws = parse_metadata(JSON).unwrap().workspace;
        let node = |id: &str| {
            let p = &ws.resolved[id];
            (p.name.as_str(), p.version.as_str(), p.source.as_deref(), p.manifest.as_str())
        };
        assert_eq!(node("core-id"), ("lactose-core", "0.0.0", None, "crates/core/Cargo.toml"));
        // A patched copy: no source, its manifest inside the workspace.
        assert_eq!(
            node("derive-id"),
            ("bytemuck_derive", "1.12.1", None, "vendor/bytemuck_derive-1.12.1/Cargo.toml")
        );
        // Outside the workspace the manifest stays absolute.
        assert_eq!(
            node("bytemuck-id"),
            (
                "bytemuck",
                "1.25.2",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
                "C:/registry/bytemuck/Cargo.toml"
            )
        );
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

    /// `lib` has five optional dependencies with resolved edges, and its activated features
    /// turn on three: `on` by its implicit feature, `hasher` by `dep:`, and the renamed
    /// `glam`-like `new` by `new/std`. `off` is turned on by no activated feature and `weak`
    /// only by `weak?/std`.
    const OPTIONAL: &str = r#"{
      "packages": [
        {"name": "lactose-a", "version": "0.0.0", "id": "a-id", "manifest_path": "/ws/crates/a/Cargo.toml",
         "dependencies": [{"name": "lib", "kind": null, "path": null}], "targets": []},
        {"name": "lib", "version": "1.0.0", "id": "lib-id", "manifest_path": "/r/lib/Cargo.toml",
         "dependencies": [
           {"name": "on", "kind": null, "path": null, "optional": true},
           {"name": "hasher", "kind": null, "path": null, "optional": true},
           {"name": "off", "kind": null, "path": null, "optional": true},
           {"name": "weak", "kind": null, "path": null, "optional": true},
           {"name": "glam", "kind": null, "path": null, "optional": true, "rename": "old"},
           {"name": "glam", "kind": null, "path": null, "optional": true, "rename": "new"}],
         "targets": [],
         "features": {"default": ["dep:hasher", "weak?/std", "new/std"], "on": ["dep:on"],
                      "off": ["dep:off"], "old": ["dep:old"]}},
        {"name": "on", "version": "1.0.0", "id": "on-id", "manifest_path": "/r/on/Cargo.toml", "dependencies": [], "targets": []},
        {"name": "hasher", "version": "1.0.0", "id": "hasher-id", "manifest_path": "/r/h/Cargo.toml", "dependencies": [], "targets": []},
        {"name": "off", "version": "1.0.0", "id": "off-id", "manifest_path": "/r/off/Cargo.toml", "dependencies": [], "targets": []},
        {"name": "weak", "version": "1.0.0", "id": "weak-id", "manifest_path": "/r/weak/Cargo.toml", "dependencies": [], "targets": []},
        {"name": "glam", "version": "0.29.0", "id": "glam-old-id", "manifest_path": "/r/g1/Cargo.toml", "dependencies": [], "targets": []},
        {"name": "glam", "version": "0.30.0", "id": "glam-new-id", "manifest_path": "/r/g2/Cargo.toml", "dependencies": [], "targets": []}
      ],
      "workspace_members": ["a-id"],
      "workspace_root": "/ws",
      "target_directory": "/ws/target",
      "resolve": {"nodes": [
        {"id": "a-id", "deps": [{"name": "lib", "pkg": "lib-id", "dep_kinds": [{"kind": null}]}]},
        {"id": "lib-id", "features": ["default", "on"], "deps": [
          {"name": "on", "pkg": "on-id", "dep_kinds": [{"kind": null}]},
          {"name": "hasher", "pkg": "hasher-id", "dep_kinds": [{"kind": null}]},
          {"name": "off", "pkg": "off-id", "dep_kinds": [{"kind": null}]},
          {"name": "weak", "pkg": "weak-id", "dep_kinds": [{"kind": null}]},
          {"name": "old", "pkg": "glam-old-id", "dep_kinds": [{"kind": null}]},
          {"name": "new", "pkg": "glam-new-id", "dep_kinds": [{"kind": null}]}]},
        {"id": "on-id", "deps": []}, {"id": "hasher-id", "deps": []}, {"id": "off-id", "deps": []},
        {"id": "weak-id", "deps": []}, {"id": "glam-old-id", "deps": []}, {"id": "glam-new-id", "deps": []}
      ]}
    }"#;

    #[test]
    fn the_third_party_closure_skips_optional_edges_no_activated_feature_turns_on() {
        let ws = parse_metadata(OPTIONAL).unwrap().workspace;
        let closure: Vec<String> = ws.third_party_closure("lactose-a").into_keys().collect();
        assert_eq!(closure, ["glam", "hasher", "lib", "on"]);
        assert_eq!(ws.resolved["lib-id"].deps, ["on-id", "hasher-id", "glam-new-id"]);
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

    #[test]
    fn reach_passes_only_through_the_crates_it_is_told_to() {
        let ws = workspace(vec![
            package(
                "lactose-a",
                &[
                    ("lactose-b", DepKind::Normal),
                    ("lactose-e", DepKind::Build),
                    ("lactose-d", DepKind::Dev),
                ],
            ),
            package("lactose-b", &[("lactose-c", DepKind::Normal), ("serde", DepKind::Normal)]),
            package("lactose-c", &[("lactose-f", DepKind::Normal)]),
            package("lactose-d", &[]),
            package("lactose-e", &[("lactose-a", DepKind::Normal)]),
            package("lactose-f", &[]),
        ]);
        // `c` is reached but not passed through, so `f` is not reached; the start is not
        // reported although `e` depends on it; `d` is a dev edge.
        let chains = ws.reach("lactose-a", &|name| name != "lactose-c");
        assert_eq!(
            chains,
            [
                vec!["lactose-a", "lactose-b"],
                vec!["lactose-a", "lactose-b", "lactose-c"],
                vec!["lactose-a", "lactose-e"],
                vec!["lactose-a", "lactose-b", "serde"],
            ]
        );
        let all: Vec<String> = ws
            .reach("lactose-a", &|_| true)
            .into_iter()
            .filter_map(|c| c.last().cloned())
            .collect();
        assert_eq!(all, ["lactose-b", "lactose-c", "lactose-e", "lactose-f", "serde"]);
    }
}
