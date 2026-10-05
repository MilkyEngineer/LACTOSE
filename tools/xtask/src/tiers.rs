//! The tier check: `crate-tiers.toml` enforced against the workspace (architecture section 3,
//! "One dependency rule, in one file").
//!
//! The file's header comments are the rules. Where they are silent this module takes the
//! narrowest reading that keeps them meaningful, and says so here:
//!
//! - **Foundation crates depend on foundation crates only.** The file says anyone may depend
//!   on a foundation crate and that game crates depend on foundation crates only; if a
//!   foundation crate could name a backend, game or leaf crate, a game crate would reach it
//!   through the foundation crate.
//! - **Only leaf crates depend on leaf crates.** A leaf may depend on anything, so a crate
//!   that named one would inherit that freedom.
//! - **Backend crates may depend on game crates.** The file does not forbid it.
//! - **Dev-dependencies obey the same rules as normal and build dependencies.** A test that
//!   names a backend crate couples the two crates as surely as the library does, and
//!   `[exceptions]` is where a sanctioned edge is recorded. The one concession is the file's
//!   "by their own tests": a crate's dev-dependency on itself (the Cargo idiom for turning on
//!   its own features under test) is never a finding.
//! - **`[exceptions]` lifts only the backend rule.** A game crate stays foundation-only.
//! - **Host code is reached through normal and build edges.** A proc-macro crate's tests and a
//!   build-script crate's tests are ordinary target artifacts, so dev edges do not count
//!   towards "build scripts and proc macros may not depend on boundary crates".
//! - **The server denylist follows normal and build edges**, which are the ones that reach the
//!   server binary's link line.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::workspace::{DepKind, Package, Workspace};
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
            Self::Leaf => "leaf",
            Self::Game => "game",
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

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
struct RawTierFile {
    foundation: CrateList,
    backend: CrateList,
    leaf: CrateList,
    game: CrateList,
    boundary: CrateList,
    server_denylist: CrateList,
    #[serde(default)]
    exceptions: BTreeMap<String, Exception>,
}

/// `crate-tiers.toml`, parsed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TierFile {
    sections: [(Tier, Vec<String>); 4],
    /// The crates inside the determinism boundary.
    pub boundary: Vec<String>,
    /// The crates the server binary may not reach.
    pub server_denylist: Vec<String>,
    /// For each crate with an exception, the backend crates it may name.
    pub exceptions: BTreeMap<String, Vec<String>>,
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
                (Tier::Leaf, raw.leaf.crates),
                (Tier::Game, raw.game.crates),
            ],
            boundary: raw.boundary.crates,
            server_denylist: raw.server_denylist.crates,
            exceptions: raw
                .exceptions
                .into_iter()
                .map(|(name, exception)| (name, exception.may_depend_on))
                .collect(),
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
        if tiers.tier(&package.name).is_none() {
            findings.push(Finding::new(
                package.manifest(),
                format!(
                    "`{}` is in no tier section of {TIER_FILE}; add it to [foundation], \
                     [backend], [leaf] or [game]",
                    package.name
                ),
            ));
            continue;
        }
        direct_edges(tiers, ws, package, &mut findings);
        host_code(tiers, ws, package, &mut findings);
    }
    server_denylist(tiers, ws, &mut findings);
    findings
}

/// Rules about the file alone: one tier per crate, and every name used elsewhere has a tier.
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
    let mut untiered = |name: &str, section: &str| {
        if tiers.tier(name).is_none() {
            findings.push(Finding::new(
                TIER_FILE,
                format!("`{name}` is in [{section}] but in no tier section"),
            ));
        }
    };
    for name in &tiers.boundary {
        untiered(name, "boundary");
    }
    for name in &tiers.server_denylist {
        untiered(name, "server_denylist");
    }
    for (name, allowed) in &tiers.exceptions {
        untiered(name, "exceptions");
        for target in allowed {
            untiered(target, "exceptions");
        }
    }
}

/// The per-edge rules of the file's header.
fn direct_edges(tiers: &TierFile, ws: &Workspace, package: &Package, findings: &mut Vec<Finding>) {
    let Some(from) = tiers.tier(&package.name) else {
        return;
    };
    for dep in package.deps.iter().filter(|d| d.workspace) {
        // "By their own tests": the crate naming itself under test.
        if dep.kind == DepKind::Dev && dep.name == package.name {
            continue;
        }
        // A workspace crate with no tier is reported once, at its own manifest.
        let Some(to) = tiers.tier(&dep.name).filter(|_| ws.package(&dep.name).is_some()) else {
            continue;
        };
        let excepted =
            tiers.exceptions.get(&package.name).is_some_and(|allowed| allowed.contains(&dep.name));
        let rule = match (from, to) {
            (_, Tier::Foundation) | (Tier::Leaf, _) | (Tier::Backend, Tier::Game) => continue,
            (Tier::Game, _) => "game crates may depend on foundation crates only",
            (Tier::Foundation, _) => "foundation crates may depend on foundation crates only",
            (Tier::Backend, Tier::Backend) if excepted => continue,
            (Tier::Backend, Tier::Backend) => {
                "a backend crate may be named only by leaf crates and by the [exceptions] table"
            }
            (Tier::Backend, Tier::Leaf) => "only leaf crates may depend on leaf crates",
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
            .filter(|d| d.workspace && d.kind == DepKind::Build)
            .map(|d| d.name.as_str())
            .collect();
        for mut chain in ws.chains_to(&start, &boundary) {
            chain.insert(0, format!("{} build script", package.name));
            report("build script", &chain);
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
mod tests {
    use super::*;
    use crate::workspace::fixture::{package, workspace};

    const FILE: &str = r#"
        [foundation]
        crates = ["lactose-core", "lactose-math", "lactose-reflect-macros", "lactose-future"]
        [backend]
        crates = ["lactose-net", "lactose-renderer", "lactose-render-graph", "lactose-app"]
        [leaf]
        crates = ["lactose-app-server", "lactose-xtask"]
        [game]
        crates = ["game-sim", "game-present"]
        [boundary]
        crates = ["lactose-core", "lactose-math", "game-sim"]
        [server_denylist]
        crates = ["lactose-renderer", "lactose-render-graph"]
        [exceptions]
        "lactose-renderer" = { may_depend_on = ["lactose-render-graph"], reason = "passes" }
    "#;

    fn tiers() -> TierFile {
        TierFile::parse(FILE).unwrap()
    }

    fn messages(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(ToString::to_string).collect()
    }

    use DepKind::{Build, Dev, Normal};

    #[test]
    fn a_conforming_workspace_has_no_findings() {
        let ws = workspace(vec![
            package("lactose-core", &[("bytemuck", Normal)]),
            package("lactose-math", &[("lactose-core", Normal)]),
            package("lactose-net", &[("lactose-core", Normal), ("lactose-net", Dev)]),
            package("lactose-render-graph", &[]),
            package("lactose-renderer", &[("lactose-render-graph", Normal)]),
            package("lactose-app", &[("game-sim", Normal)]),
            package("lactose-app-server", &[("lactose-net", Normal), ("lactose-app", Normal)]),
            package("lactose-xtask", &[]),
            package("game-sim", &[("lactose-math", Normal)]),
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
        let ws = workspace(vec![
            package("lactose-core", &[("lactose-net", Normal)]),
            package("lactose-math", &[("game-sim", Normal)]),
            package("lactose-net", &[("lactose-renderer", Dev), ("lactose-xtask", Build)]),
            package("lactose-renderer", &[("lactose-net", Normal)]),
            package("lactose-xtask", &[]),
            package("game-sim", &[("game-present", Normal), ("lactose-net", Normal)]),
            package("game-present", &[]),
        ]);
        let found = messages(&check(&tiers(), &ws));
        let expect = [
            "crates/core/Cargo.toml: `lactose-core` (foundation) has a dependency on `lactose-net` (backend): foundation crates may depend on foundation crates only",
            "crates/math/Cargo.toml: `lactose-math` (foundation) has a dependency on `game-sim` (game): foundation crates may depend on foundation crates only",
            "crates/net/Cargo.toml: `lactose-net` (backend) has a dev-dependency on `lactose-renderer` (backend): a backend crate may be named only by leaf crates and by the [exceptions] table",
            "crates/net/Cargo.toml: `lactose-net` (backend) has a build-dependency on `lactose-xtask` (leaf): only leaf crates may depend on leaf crates",
            "crates/renderer/Cargo.toml: `lactose-renderer` (backend) has a dependency on `lactose-net` (backend): a backend crate may be named only by leaf crates and by the [exceptions] table",
            "crates/game-sim/Cargo.toml: `game-sim` (game) has a dependency on `game-present` (game): game crates may depend on foundation crates only",
            "crates/game-sim/Cargo.toml: `game-sim` (game) has a dependency on `lactose-net` (backend): game crates may depend on foundation crates only",
        ];
        for line in expect {
            assert!(found.contains(&line.to_owned()), "missing: {line}\nin: {found:#?}");
        }
        assert_eq!(found.len(), 7, "{found:#?}");
    }

    #[test]
    fn the_server_may_not_reach_a_denied_crate() {
        let ws = workspace(vec![
            package("lactose-app-server", &[("lactose-app", Normal)]),
            package("lactose-app", &[]),
            package("lactose-renderer", &[]),
        ]);
        assert!(check(&tiers(), &ws).is_empty(), "nothing denied is reached");

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
            .replace(r#""game-sim"]"#, r#""game-sim", "lactose-ghost"]"#);
        let found = messages(&check(&TierFile::parse(&text).unwrap(), &Workspace::default()));
        assert_eq!(
            found,
            [
                "crate-tiers.toml: `lactose-core` is listed in more than one tier section, or twice in one",
                "crate-tiers.toml: `lactose-ghost` is in [boundary] but in no tier section",
            ]
        );
    }

    #[test]
    fn an_unknown_section_is_a_parse_error() {
        let text = format!("{FILE}\n[backends]\ncrates = []\n");
        assert!(TierFile::parse(&text).unwrap_err().contains("backends"));
    }
}
