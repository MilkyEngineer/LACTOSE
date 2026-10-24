//! The tier check: `crate-tiers.toml` enforced against the workspace (architecture section 3,
//! "One dependency rule, in one file").
//!
//! The file's header comments are the rules, and this module implements them as written.
//! Three points are about how they are applied:
//!
//! - **A dependency is an edge to a tiered crate if its name is in the file**, whether it is
//!   a workspace member, a path dependency or comes from a registry, so a crate cannot leave
//!   the rules by being fetched from somewhere else.
//! - **Host code is reached through normal and build edges.** A proc-macro crate's tests and a
//!   build-script crate's tests are ordinary target artifacts, so dev edges do not count
//!   towards "build scripts and proc macros may not depend on boundary crates".
//! - **The server denylist follows normal and build edges**, which are the ones that reach the
//!   server binary's link line.
//! - **A dev-only feature is one the `[dev_only_features]` table lists, or one of its owner's
//!   features that turns a listed one on**, `default` included, so an alias cannot carry it out
//!   of the dev edges.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::workspace::{DepKind, DepSource, Package, Workspace};
use crate::{Finding, TIER_FILE};

/// The binary that `[server_denylist]` constrains.
pub const SERVER_BINARY: &str = "lactose-app-server";

/// A tier section of the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// `[foundation]`.
    Foundation,
    /// `[backend]`.
    Backend,
    /// `[assembly]`.
    Assembly,
    /// `[leaf]`.
    Leaf,
    /// `[game]`.
    Game,
}

impl Tier {
    fn name(self) -> &'static str {
        match self {
            Self::Foundation => "foundation",
            Self::Backend => "backend",
            Self::Assembly => "assembly",
            Self::Leaf => "leaf",
            Self::Game => "game",
        }
    }

    /// The rule that forbids a crate of this tier from depending on a crate of tier `to`,
    /// or `None` if the edge is allowed. `excepted` says whether `[exceptions]` names it.
    fn forbids(self, to: Self, excepted: bool) -> Option<&'static str> {
        match (self, to) {
            (_, Self::Foundation) | (Self::Leaf, _) => None,
            (_, Self::Leaf) => Some("only leaf crates may depend on a leaf crate"),
            (_, Self::Assembly) => Some("only leaf crates may depend on an assembly crate"),
            (Self::Foundation, _) => Some("foundation crates may depend on foundation crates only"),
            (Self::Game, Self::Game) | (Self::Assembly, Self::Backend | Self::Game) => None,
            (Self::Game, Self::Backend) => {
                Some("game crates may depend on foundation and game crates only")
            }
            (Self::Backend, Self::Backend) if excepted => None,
            (Self::Backend, Self::Backend) => Some(
                "a backend crate may be named only by assembly crates, leaf crates and the \
                 [exceptions] table",
            ),
            (Self::Backend, Self::Game) => Some("backend crates may not depend on game crates"),
        }
    }
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
struct CrateList {
    crates: Vec<String>,
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
struct Exception {
    may_depend_on: Vec<String>,
    #[serde(rename = "reason")]
    _reason: String,
}

/// The non-boundary workspace crates a boundary crate may depend on.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct InternalAllowlist {
    /// Permitted for every boundary crate.
    pub any: Vec<String>,
    /// Permitted for the named boundary crate alone.
    #[serde(default)]
    pub only: BTreeMap<String, Vec<String>>,
    /// Permitted for every boundary crate as a dev-dependency only, for its tests.
    #[serde(default)]
    pub dev: Vec<String>,
}

/// The third-party crates one boundary crate may name.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct DirectAllowlist {
    /// Permitted as a normal, build or dev dependency.
    #[serde(default)]
    pub normal: Vec<String>,
    /// Permitted as a dev-dependency only.
    #[serde(default)]
    pub dev: Vec<String>,
}

/// The third-party crates the boundary may compile.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct ThirdPartyAllowlist {
    /// Permitted anywhere below a boundary crate, but not to be named by one.
    pub transitive: Vec<String>,
    /// What each boundary crate may name.
    #[serde(default)]
    pub direct: BTreeMap<String, DirectAllowlist>,
}

impl ThirdPartyAllowlist {
    /// Whether `name` may appear in the normal and build closure of a boundary crate: it is
    /// on `transitive` or on some boundary crate's `normal` list.
    #[must_use]
    pub fn vetted(&self, name: &str) -> bool {
        self.transitive.iter().any(|c| c == name)
            || self.direct.values().any(|list| list.normal.iter().any(|c| c == name))
    }
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
struct RawTierFile {
    foundation: CrateList,
    backend: CrateList,
    assembly: CrateList,
    leaf: CrateList,
    game: CrateList,
    test_support: CrateList,
    boundary: CrateList,
    server_denylist: CrateList,
    #[serde(default)]
    exceptions: BTreeMap<String, Exception>,
    boundary_internal: InternalAllowlist,
    boundary_third_party: ThirdPartyAllowlist,
    #[serde(default)]
    dev_only_features: BTreeMap<String, Vec<String>>,
}

/// `crate-tiers.toml`, parsed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TierFile {
    sections: [(Tier, Vec<String>); 5],
    /// The crates any crate may dev-depend on.
    pub test_support: Vec<String>,
    /// The crates inside the determinism boundary.
    pub boundary: Vec<String>,
    /// The crates the server binary may not reach.
    pub server_denylist: Vec<String>,
    /// For each backend crate with an exception, the backend crates it may name.
    pub exceptions: BTreeMap<String, Vec<String>>,
    /// The boundary allowlist for workspace crates (architecture section 8.2).
    pub boundary_internal: InternalAllowlist,
    /// The boundary allowlist for third-party crates.
    pub boundary_third_party: ThirdPartyAllowlist,
    /// For each crate, the features of it that only a dev-dependency may turn on.
    pub dev_only_features: BTreeMap<String, Vec<String>>,
}

impl TierFile {
    /// Parses the text of `crate-tiers.toml`.
    ///
    /// # Errors
    /// If the text is not TOML of the expected shape. An unknown section or key is an error,
    /// so a misspelt section cannot silently drop a rule.
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawTierFile = toml::from_str(text).map_err(|e| format!("{TIER_FILE}: {e}"))?;
        Ok(Self {
            sections: [
                (Tier::Foundation, raw.foundation.crates),
                (Tier::Backend, raw.backend.crates),
                (Tier::Assembly, raw.assembly.crates),
                (Tier::Leaf, raw.leaf.crates),
                (Tier::Game, raw.game.crates),
            ],
            test_support: raw.test_support.crates,
            boundary: raw.boundary.crates,
            server_denylist: raw.server_denylist.crates,
            exceptions: raw
                .exceptions
                .into_iter()
                .map(|(name, exception)| (name, exception.may_depend_on))
                .collect(),
            boundary_internal: raw.boundary_internal,
            boundary_third_party: raw.boundary_third_party,
            dev_only_features: raw.dev_only_features,
        })
    }

    /// The tier of `name`: the first section that lists it. [`check`] reports a crate listed
    /// more than once.
    #[must_use]
    pub fn tier(&self, name: &str) -> Option<Tier> {
        self.sections
            .iter()
            .find(|(_, crates)| crates.iter().any(|c| c == name))
            .map(|&(tier, _)| tier)
    }

    /// Whether `name` is inside the determinism boundary.
    #[must_use]
    pub fn in_boundary(&self, name: &str) -> bool {
        self.boundary.iter().any(|c| c == name)
    }

    fn listings(&self, name: &str) -> usize {
        self.sections.iter().map(|(_, crates)| crates.iter().filter(|c| *c == name).count()).sum()
    }
}

/// Checks the workspace against the tier file. Crates the file lists that do not exist yet
/// are not findings.
#[must_use]
pub fn check(tiers: &TierFile, ws: &Workspace) -> Vec<Finding> {
    let mut findings = Vec::new();
    file_consistency(tiers, &mut findings);
    for package in &ws.packages {
        for dep in package.deps.iter().filter(|d| d.source == DepSource::Path) {
            findings.push(Finding::new(
                package.manifest(),
                format!(
                    "`{}` has a path dependency on `{}`, which is not a workspace member; \
                     code outside the workspace is outside every rule in {TIER_FILE}",
                    package.name, dep.name
                ),
            ));
        }
        if tiers.tier(&package.name).is_none() {
            findings.push(Finding::new(
                package.manifest(),
                format!(
                    "`{}` is in no tier section of {TIER_FILE}; add it to [foundation], \
                     [backend], [assembly], [leaf] or [game]",
                    package.name
                ),
            ));
            continue;
        }
        direct_edges(tiers, package, &mut findings);
        host_code(tiers, ws, package, &mut findings);
    }
    server_denylist(tiers, ws, &mut findings);
    dev_only_features(tiers, ws, &mut findings);
    findings
}

/// Rules about the file alone: one tier per crate, every name used elsewhere has a tier, and
/// the tables that qualify a kind of crate name only that kind.
fn file_consistency(tiers: &TierFile, findings: &mut Vec<Finding>) {
    let mut listed: Vec<&String> = tiers.sections.iter().flat_map(|(_, c)| c).collect();
    listed.sort();
    listed.dedup();
    for name in listed {
        if tiers.listings(name) > 1 {
            findings.push(Finding::new(
                TIER_FILE,
                format!("`{name}` is listed in more than one tier section, or twice in one"),
            ));
        }
    }
    let mut messages = Vec::new();
    let mut tiered = |names: &mut dyn Iterator<Item = &String>, section: &str| {
        for name in names.filter(|name| tiers.tier(name).is_none()) {
            messages.push(format!("`{name}` is in [{section}] but in no tier section"));
        }
    };
    let internal = &tiers.boundary_internal;
    tiered(&mut tiers.boundary.iter(), "boundary");
    tiered(&mut tiers.server_denylist.iter(), "server_denylist");
    tiered(&mut tiers.test_support.iter(), "test_support");
    tiered(&mut tiers.dev_only_features.keys(), "dev_only_features");
    tiered(
        &mut internal.any.iter().chain(internal.only.values().flatten()).chain(&internal.dev),
        "boundary_internal",
    );

    let third_party = &tiers.boundary_third_party;
    let keyed = [
        (internal.only.keys().collect::<Vec<_>>(), "boundary_internal.only"),
        (third_party.direct.keys().collect(), "boundary_third_party.direct"),
    ];
    for (names, table) in keyed {
        for name in names.into_iter().filter(|name| !tiers.in_boundary(name)) {
            messages.push(format!("`{name}` has an entry in [{table}] but is not in [boundary]"));
        }
    }
    for (name, allowed) in &tiers.exceptions {
        for crate_name in std::iter::once(name).chain(allowed) {
            match tiers.tier(crate_name) {
                Some(Tier::Backend) => {}
                Some(tier) => messages.push(format!(
                    "[exceptions] entry `{name}` names `{crate_name}` ({}); exceptions lift \
                     backend-to-backend edges only",
                    tier.name()
                )),
                None => messages
                    .push(format!("`{crate_name}` is in [exceptions] but in no tier section")),
            }
        }
    }
    findings.extend(messages.into_iter().map(|message| Finding::new(TIER_FILE, message)));
}

/// The per-edge rules of the file's header.
fn direct_edges(tiers: &TierFile, package: &Package, findings: &mut Vec<Finding>) {
    let Some(from) = tiers.tier(&package.name) else {
        return;
    };
    for dep in &package.deps {
        // Any name the file lists is held to the rules, wherever the crate comes from.
        let Some(to) = tiers.tier(&dep.name) else {
            continue;
        };
        if dep.kind == DepKind::Dev
            && (dep.name == package.name || tiers.test_support.contains(&dep.name))
        {
            continue;
        }
        let excepted =
            tiers.exceptions.get(&package.name).is_some_and(|allowed| allowed.contains(&dep.name));
        let Some(rule) = from.forbids(to, excepted) else {
            continue;
        };
        let section = match dep.kind {
            DepKind::Normal => "dependency",
            DepKind::Build => "build-dependency",
            DepKind::Dev => "dev-dependency",
        };
        findings.push(Finding::new(
            package.manifest(),
            format!(
                "`{}` ({}) has a {section} on `{}` ({}): {rule}",
                package.name,
                from.name(),
                dep.name,
                to.name()
            ),
        ));
    }
}

/// Build scripts and proc macros compile for the host without the pinned target features, so
/// neither may reach a boundary crate.
fn host_code(tiers: &TierFile, ws: &Workspace, package: &Package, findings: &mut Vec<Finding>) {
    let boundary = |name: &str| tiers.in_boundary(name);
    let mut report = |what: &str, chain: &[String]| {
        findings.push(Finding::new(
            package.manifest(),
            format!(
                "{what} reaches boundary crate `{}` ({}); host code is compiled without the \
                 pinned target features",
                chain.last().map_or("", String::as_str),
                chain.join(" -> ")
            ),
        ));
    };
    if package.proc_macro {
        // Starting at the crate itself also reports a proc-macro crate listed in [boundary].
        for chain in ws.chains_to(&[&package.name], &boundary) {
            report("proc-macro crate", &chain);
        }
    }
    if package.build_script {
        let start: Vec<&str> = package
            .deps
            .iter()
            .filter(|d| d.kind == DepKind::Build)
            .map(|d| d.name.as_str())
            .collect();
        for mut chain in ws.chains_to(&start, &boundary) {
            chain.insert(0, format!("{} build script", package.name));
            report("build script", &chain);
        }
    }
}

/// A feature `[dev_only_features]` lists compiles code for tests alone, so only a dev edge
/// may turn it on: a normal or build edge would ship it, and a `[features]` value forwarding
/// to it would hand it to whatever turns that feature on. `dep:<name>` enables an optional
/// edge with the features it declares, so the edge rule covers it.
fn dev_only_features(tiers: &TierFile, ws: &Workspace, findings: &mut Vec<Finding>) {
    for (owner, listed) in &tiers.dev_only_features {
        let Some(owner_package) = ws.package(owner) else {
            continue;
        };
        for feature in listed.iter().filter(|f| !owner_package.features.contains_key(*f)) {
            findings.push(Finding::new(
                TIER_FILE,
                format!("[dev_only_features] lists `{feature}`, which `{owner}` does not declare"),
            ));
        }
        let dev_only = turning_on(owner_package, listed);
        if dev_only.contains("default") {
            findings.push(Finding::new(
                owner_package.manifest(),
                format!(
                    "`{owner}`'s `default` feature turns on a feature [dev_only_features] holds \
                     to dev-dependencies"
                ),
            ));
        }
        for package in &ws.packages {
            let owner_edges = || package.deps.iter().filter(|d| d.name == *owner);
            for dep in owner_edges().filter(|d| d.kind.ships()) {
                let section =
                    if dep.kind == DepKind::Build { "build-dependency" } else { "dependency" };
                for feature in dep.features.iter().filter(|f| dev_only.contains(f.as_str())) {
                    findings.push(Finding::new(
                        package.manifest(),
                        format!(
                            "`{}` turns on `{owner}`'s feature `{feature}` through a {section}; \
                             [dev_only_features] holds it to dev-dependencies",
                            package.name
                        ),
                    ));
                }
            }
            for (feature, values) in &package.features {
                for value in values {
                    let Some((key, forwarded)) = value.split_once('/') else {
                        continue;
                    };
                    let key = key.strip_suffix('?').unwrap_or(key);
                    if dev_only.contains(forwarded) && owner_edges().any(|d| d.key() == key) {
                        findings.push(Finding::new(
                            package.manifest(),
                            format!(
                                "`{}`'s feature `{feature}` forwards to `{value}`, a feature of \
                                 `{owner}` that [dev_only_features] holds to dev-dependencies",
                                package.name
                            ),
                        ));
                    }
                }
            }
        }
    }
}

/// The features of `package` that turn on one of `listed`, the listed ones included: a
/// feature is in the set when one of its plain values is.
fn turning_on<'a>(package: &'a Package, listed: &'a [String]) -> BTreeSet<&'a str> {
    let mut set: BTreeSet<&str> = listed.iter().map(String::as_str).collect();
    loop {
        let before = set.len();
        for (feature, values) in &package.features {
            if values.iter().any(|value| set.contains(value.as_str())) {
                set.insert(feature);
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

fn server_denylist(tiers: &TierFile, ws: &Workspace, findings: &mut Vec<Finding>) {
    let Some(server) = ws.package(SERVER_BINARY) else {
        return;
    };
    let denied = |name: &str| tiers.server_denylist.iter().any(|c| c == name);
    for chain in ws.chains_to(&[SERVER_BINARY], &denied) {
        findings.push(Finding::new(
            server.manifest(),
            format!(
                "`{SERVER_BINARY}` reaches `{}`, which is in [server_denylist] ({})",
                chain.last().map_or("", String::as_str),
                chain.join(" -> ")
            ),
        ));
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    /// A small tier file with every section. Tests replace parts of it.
    pub(crate) const FILE: &str = r#"
        [foundation]
        crates = ["lactose-core", "lactose-math", "lactose-task", "lactose-ecs", "lactose-reflect",
                  "lactose-reflect-macros", "lactose-ecs-macros", "lactose-io", "lactose-future"]
        [backend]
        crates = ["lactose-net", "lactose-renderer", "lactose-render-graph", "lactose-rhi-null"]
        [assembly]
        crates = ["lactose-app"]
        [leaf]
        crates = ["lactose-app-server", "lactose-xtask"]
        [game]
        crates = ["game-sim", "game-present"]
        [test_support]
        crates = ["lactose-rhi-null"]
        [boundary]
        crates = ["lactose-core", "lactose-math", "lactose-ecs", "game-sim"]
        [server_denylist]
        crates = ["lactose-renderer", "lactose-render-graph", "lactose-rhi-null"]
        [exceptions]
        "lactose-renderer" = { may_depend_on = ["lactose-render-graph"], reason = "passes" }
        [boundary_internal]
        any = ["lactose-reflect", "lactose-reflect-macros"]
        dev = ["lactose-task"]
        [boundary_internal.only]
        "lactose-ecs" = ["lactose-task", "lactose-ecs-macros"]
        [boundary_third_party]
        transitive = ["bytemuck_derive"]
        [boundary_third_party.direct]
        "lactose-core" = { normal = ["bytemuck"], dev = ["xxhash-rust"] }
        "lactose-math" = { normal = ["libm"] }
    "#;
}

#[cfg(test)]
mod tests {
    use super::fixture::FILE;
    use super::*;
    use crate::workspace::Dep;
    use crate::workspace::fixture::{package, workspace};

    fn tiers() -> TierFile {
        TierFile::parse(FILE).unwrap()
    }

    fn messages(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(ToString::to_string).collect()
    }

    use DepKind::{Build, Dev, Normal};

    #[test]
    fn the_checked_in_file_parses_and_is_consistent() {
        let tiers = TierFile::parse(include_str!("../../../crate-tiers.toml")).unwrap();
        assert_eq!(messages(&check(&tiers, &Workspace::default())), [] as [&str; 0]);
        assert_eq!(tiers.tier("lactose-app"), Some(Tier::Assembly));
        assert!(tiers.boundary_third_party.vetted("libm"));
        assert!(tiers.boundary_third_party.vetted("syn"));
        assert!(!tiers.boundary_third_party.vetted("xxhash-rust"), "dev only");
        assert_eq!(tiers.dev_only_features["lactose-asset-api"], ["conformance"]);
    }

    #[test]
    fn a_conforming_workspace_has_no_findings() {
        let ws = workspace(vec![
            package("lactose-core", &[("bytemuck", Normal)]),
            package("lactose-math", &[("lactose-core", Normal)]),
            package("lactose-net", &[("lactose-core", Normal), ("lactose-net", Dev)]),
            package("lactose-render-graph", &[("lactose-rhi-null", Dev)]),
            package("lactose-renderer", &[("lactose-render-graph", Normal)]),
            package("lactose-rhi-null", &[]),
            package("lactose-app", &[("game-sim", Normal), ("lactose-net", Normal)]),
            package("lactose-app-server", &[("lactose-net", Normal), ("lactose-app", Normal)]),
            package("lactose-xtask", &[]),
            package("game-sim", &[("lactose-math", Normal), ("lactose-rhi-null", Dev)]),
            package("game-present", &[("game-sim", Normal)]),
        ]);
        assert_eq!(messages(&check(&tiers(), &ws)), [] as [&str; 0]);
    }

    #[test]
    fn a_workspace_crate_missing_from_the_file_is_reported() {
        let ws = workspace(vec![package("lactose-stray", &[("lactose-net", Normal)])]);
        let found = messages(&check(&tiers(), &ws));
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with("crates/stray/Cargo.toml: `lactose-stray` is in no tier"));
    }

    #[test]
    fn each_tier_rule_is_enforced() {
        let backend_rule = "a backend crate may be named only by assembly crates, leaf crates and the [exceptions] table";
        let cases: [(&str, (&str, DepKind), Option<String>); 14] = [
            ("lactose-core", ("lactose-net", Normal), Some("`lactose-core` (foundation) has a dependency on `lactose-net` (backend): foundation crates may depend on foundation crates only".into())),
            ("lactose-math", ("game-sim", Normal), Some("`lactose-math` (foundation) has a dependency on `game-sim` (game): foundation crates may depend on foundation crates only".into())),
            ("lactose-net", ("lactose-renderer", Dev), Some(format!("`lactose-net` (backend) has a dev-dependency on `lactose-renderer` (backend): {backend_rule}"))),
            ("lactose-net", ("lactose-xtask", Build), Some("`lactose-net` (backend) has a build-dependency on `lactose-xtask` (leaf): only leaf crates may depend on a leaf crate".into())),
            ("lactose-net", ("game-sim", Normal), Some("`lactose-net` (backend) has a dependency on `game-sim` (game): backend crates may not depend on game crates".into())),
            ("lactose-net", ("lactose-app", Normal), Some("`lactose-net` (backend) has a dependency on `lactose-app` (assembly): only leaf crates may depend on an assembly crate".into())),
            // The exception is for renderer -> render-graph, not the reverse.
            ("lactose-render-graph", ("lactose-renderer", Normal), Some(format!("`lactose-render-graph` (backend) has a dependency on `lactose-renderer` (backend): {backend_rule}"))),
            // A test-support crate is free as a dev-dependency only.
            ("lactose-net", ("lactose-rhi-null", Normal), Some(format!("`lactose-net` (backend) has a dependency on `lactose-rhi-null` (backend): {backend_rule}"))),
            ("lactose-net", ("lactose-rhi-null", Dev), None),
            ("game-sim", ("lactose-net", Normal), Some("`game-sim` (game) has a dependency on `lactose-net` (backend): game crates may depend on foundation and game crates only".into())),
            ("game-sim", ("game-present", Normal), None),
            ("game-sim", ("lactose-app", Dev), Some("`game-sim` (game) has a dev-dependency on `lactose-app` (assembly): only leaf crates may depend on an assembly crate".into())),
            ("lactose-app", ("lactose-renderer", Normal), None),
            ("lactose-app", ("lactose-xtask", Normal), Some("`lactose-app` (assembly) has a dependency on `lactose-xtask` (leaf): only leaf crates may depend on a leaf crate".into())),
        ];
        for (from, edge, expected) in cases {
            let ws = workspace(vec![package(from, &[edge])]);
            let found: Vec<String> = check(&tiers(), &ws).into_iter().map(|f| f.message).collect();
            assert_eq!(found, expected.into_iter().collect::<Vec<_>>(), "{from} -> {edge:?}");
        }
    }

    #[test]
    fn a_tiered_name_is_an_edge_wherever_the_crate_comes_from() {
        // `lactose-renderer` fetched from a registry instead of the workspace.
        let mut package = package("game-sim", &[]);
        package.deps.push(Dep {
            name: "lactose-renderer".to_owned(),
            kind: Normal,
            source: DepSource::External,
            rename: None,
            features: Vec::new(),
        });
        let found = messages(&check(&tiers(), &workspace(vec![package])));
        assert_eq!(
            found,
            [
                "crates/game-sim/Cargo.toml: `game-sim` (game) has a dependency on `lactose-renderer` \
              (backend): game crates may depend on foundation and game crates only"
            ]
        );
    }

    #[test]
    fn a_path_dependency_outside_the_workspace_is_reported() {
        let mut package = package("lactose-math", &[]);
        package.deps.push(Dep {
            name: "shim".to_owned(),
            kind: Normal,
            source: DepSource::Path,
            rename: None,
            features: Vec::new(),
        });
        let found = messages(&check(&tiers(), &workspace(vec![package])));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(found[0].starts_with(
            "crates/math/Cargo.toml: `lactose-math` has a path dependency on `shim`, which is \
             not a workspace member"
        ));
    }

    #[test]
    fn the_server_may_not_reach_a_denied_crate() {
        let ws = workspace(vec![
            package("lactose-app-server", &[("lactose-app", Normal)]),
            package("lactose-app", &[]),
            package("lactose-renderer", &[]),
        ]);
        assert!(check(&tiers(), &ws).is_empty(), "nothing denied is reached");

        // The assembly crate is on the server's path like anything else.
        let ws = workspace(vec![
            package("lactose-app-server", &[("lactose-app", Normal)]),
            package("lactose-app", &[("lactose-renderer", Normal)]),
            package("lactose-renderer", &[("lactose-render-graph", Normal)]),
            package("lactose-render-graph", &[]),
        ]);
        let found = messages(&check(&tiers(), &ws));
        assert!(
            found.contains(
                &"crates/app-server/Cargo.toml: `lactose-app-server` reaches `lactose-renderer`, \
              which is in [server_denylist] (lactose-app-server -> lactose-app -> lactose-renderer)"
                    .to_owned()
            )
        );
        assert!(found.iter().any(|f| f.contains("reaches `lactose-render-graph`")));
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn host_code_may_not_reach_the_boundary() {
        let mut macros =
            package("lactose-reflect-macros", &[("lactose-future", Normal), ("lactose-math", Dev)]);
        macros.proc_macro = true;
        let mut scripted =
            package("lactose-net", &[("lactose-core", Normal), ("lactose-future", Build)]);
        scripted.build_script = true;
        let ws = workspace(vec![
            macros.clone(),
            scripted.clone(),
            package("lactose-future", &[]),
            package("lactose-core", &[]),
            package("lactose-math", &[]),
        ]);
        assert!(check(&tiers(), &ws).is_empty(), "dev and normal edges of the owner do not count");

        let ws = workspace(vec![
            macros,
            scripted,
            package("lactose-future", &[("lactose-core", Normal)]),
            package("lactose-core", &[]),
            package("lactose-math", &[]),
        ]);
        let found = messages(&check(&tiers(), &ws));
        assert_eq!(
            found,
            [
                "crates/net/Cargo.toml: build script reaches boundary crate `lactose-core` \
                 (lactose-net build script -> lactose-future -> lactose-core); host code is \
                 compiled without the pinned target features",
                "crates/reflect-macros/Cargo.toml: proc-macro crate reaches boundary crate \
                 `lactose-core` (lactose-reflect-macros -> lactose-future -> lactose-core); \
                 host code is compiled without the pinned target features",
            ]
        );
    }

    #[test]
    fn the_file_itself_is_checked() {
        let text = FILE
            .replace(r#""lactose-app-server", "lactose-xtask""#, r#""lactose-core""#)
            .replace(r#""lactose-ecs", "game-sim"]"#, r#""lactose-ecs", "game-sim", "lactose-ghost"]"#)
            .replace(
                r#""lactose-renderer" = { may_depend_on = ["lactose-render-graph"], reason = "passes" }"#,
                r#""lactose-app" = { may_depend_on = ["lactose-net", "game-sim"], reason = "assembles" }"#,
            )
            .replace(r#""lactose-math" = { normal = ["libm"] }"#, r#""lactose-task" = { normal = ["libm"] }"#)
            .replace(r#""lactose-ecs" = ["lactose-task", "lactose-ecs-macros"]"#, r#""lactose-io" = ["lactose-nowhere"]"#)
            .replace(r#"dev = ["lactose-task"]"#, r#"dev = ["lactose-task", "lactose-elsewhere"]"#);
        let found = messages(&check(&TierFile::parse(&text).unwrap(), &Workspace::default()));
        assert_eq!(
            found,
            [
                "crate-tiers.toml: `lactose-core` is listed in more than one tier section, or twice in one",
                "crate-tiers.toml: `lactose-ghost` is in [boundary] but in no tier section",
                "crate-tiers.toml: `lactose-nowhere` is in [boundary_internal] but in no tier section",
                "crate-tiers.toml: `lactose-elsewhere` is in [boundary_internal] but in no tier section",
                "crate-tiers.toml: `lactose-io` has an entry in [boundary_internal.only] but is not in [boundary]",
                "crate-tiers.toml: `lactose-task` has an entry in [boundary_third_party.direct] but is not in [boundary]",
                "crate-tiers.toml: [exceptions] entry `lactose-app` names `lactose-app` (assembly); exceptions lift backend-to-backend edges only",
                "crate-tiers.toml: [exceptions] entry `lactose-app` names `game-sim` (game); exceptions lift backend-to-backend edges only",
            ]
        );
    }

    #[test]
    fn an_exception_does_not_lift_a_rule_for_a_crate_that_is_not_backend() {
        let text = FILE.replace(
            r#""lactose-renderer" = { may_depend_on = ["lactose-render-graph"], reason = "passes" }"#,
            r#""game-sim" = { may_depend_on = ["lactose-net"], reason = "wishful" }"#,
        );
        let ws = workspace(vec![package("game-sim", &[("lactose-net", Normal)])]);
        let found = messages(&check(&TierFile::parse(&text).unwrap(), &ws));
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(found[0].contains("[exceptions] entry `game-sim` names `game-sim` (game)"));
        assert!(found[1].ends_with("game crates may depend on foundation and game crates only"));
    }

    /// The fixture file with `lactose-io`'s `conformance` held to dev edges.
    fn dev_only_tiers() -> TierFile {
        TierFile::parse(&format!(
            "{FILE}\n[dev_only_features]\n\"lactose-io\" = [\"conformance\"]\n"
        ))
        .unwrap()
    }

    /// `lactose-io` with a `conformance` feature, `alias` turning it on, and `other`.
    fn io() -> Package {
        let mut io = package("lactose-io", &[("lactose-io", Dev)]);
        io.deps[0].features = vec!["conformance".to_owned()];
        io.features.insert("conformance".to_owned(), Vec::new());
        io.features.insert("alias".to_owned(), vec!["conformance".to_owned()]);
        io.features.insert("other".to_owned(), Vec::new());
        io
    }

    /// `package(name, edges)` whose edge on `lactose-io` turns on `features`.
    fn turning_on_io(name: &str, kind: DepKind, features: &[&str]) -> Package {
        let mut package = package(name, &[("lactose-io", kind)]);
        package.deps[0].features = features.iter().map(|f| (*f).to_owned()).collect();
        package
    }

    #[test]
    fn a_dev_only_feature_may_be_turned_on_by_a_dev_edge_alone() {
        let allowed = workspace(vec![
            io(),
            turning_on_io("lactose-net", Dev, &["conformance", "alias"]),
            turning_on_io("lactose-math", Normal, &["other"]),
        ]);
        assert_eq!(messages(&check(&dev_only_tiers(), &allowed)), [] as [&str; 0]);

        let mut optional = turning_on_io("lactose-ecs", Normal, &["conformance"]);
        optional.features.insert("io".to_owned(), vec!["dep:lactose-io".to_owned()]);
        let ws = workspace(vec![
            io(),
            turning_on_io("lactose-net", Normal, &["conformance"]),
            turning_on_io("lactose-math", Build, &["alias"]),
            optional,
        ]);
        assert_eq!(
            messages(&check(&dev_only_tiers(), &ws)),
            [
                "crates/ecs/Cargo.toml: `lactose-ecs` turns on `lactose-io`'s feature `conformance` through a dependency; [dev_only_features] holds it to dev-dependencies",
                "crates/math/Cargo.toml: `lactose-math` turns on `lactose-io`'s feature `alias` through a build-dependency; [dev_only_features] holds it to dev-dependencies",
                "crates/net/Cargo.toml: `lactose-net` turns on `lactose-io`'s feature `conformance` through a dependency; [dev_only_features] holds it to dev-dependencies",
            ]
        );
    }

    #[test]
    fn no_feature_may_forward_to_a_dev_only_feature() {
        let mut plain = package("lactose-net", &[("lactose-io", Normal)]);
        plain.features.insert("tests".to_owned(), vec!["lactose-io/conformance".to_owned()]);
        plain.features.insert("fine".to_owned(), vec!["lactose-io/other".to_owned()]);
        let mut weak = package("lactose-math", &[("lactose-io", Normal)]);
        weak.features.insert("tests".to_owned(), vec!["lactose-io?/alias".to_owned()]);
        let mut renamed = package("lactose-ecs", &[("lactose-io", Normal)]);
        renamed.deps[0].rename = Some("io".to_owned());
        renamed.features.insert("tests".to_owned(), vec!["io/conformance".to_owned()]);
        // A value naming some other crate's feature of the same name is not a forward.
        renamed.features.insert("unrelated".to_owned(), vec!["lactose-io/conformance".to_owned()]);
        let ws = workspace(vec![io(), plain, weak, renamed]);
        assert_eq!(
            messages(&check(&dev_only_tiers(), &ws)),
            [
                "crates/ecs/Cargo.toml: `lactose-ecs`'s feature `tests` forwards to `io/conformance`, a feature of `lactose-io` that [dev_only_features] holds to dev-dependencies",
                "crates/math/Cargo.toml: `lactose-math`'s feature `tests` forwards to `lactose-io?/alias`, a feature of `lactose-io` that [dev_only_features] holds to dev-dependencies",
                "crates/net/Cargo.toml: `lactose-net`'s feature `tests` forwards to `lactose-io/conformance`, a feature of `lactose-io` that [dev_only_features] holds to dev-dependencies",
            ]
        );
    }

    #[test]
    fn a_dev_only_feature_is_declared_and_not_on_by_default() {
        let mut io = io();
        io.features.insert("default".to_owned(), vec!["alias".to_owned()]);
        io.features.remove("conformance");
        assert_eq!(
            messages(&check(&dev_only_tiers(), &workspace(vec![io]))),
            [
                "crate-tiers.toml: [dev_only_features] lists `conformance`, which `lactose-io` does not declare",
                "crates/io/Cargo.toml: `lactose-io`'s `default` feature turns on a feature [dev_only_features] holds to dev-dependencies",
            ]
        );
        let text = format!("{FILE}\n[dev_only_features]\n\"lactose-ghost\" = [\"conformance\"]\n");
        assert_eq!(
            messages(&check(&TierFile::parse(&text).unwrap(), &Workspace::default())),
            ["crate-tiers.toml: `lactose-ghost` is in [dev_only_features] but in no tier section"]
        );
    }

    #[test]
    fn an_unknown_section_is_a_parse_error() {
        let text = format!("{FILE}\n[backends]\ncrates = []\n");
        assert!(TierFile::parse(&text).unwrap_err().contains("backends"));
    }
}
