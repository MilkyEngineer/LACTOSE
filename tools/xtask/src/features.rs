//! The feature-tree comparison of architecture section 8.3.
//!
//! Cargo resolves features per build, so two binaries can compile the same boundary crate
//! with different features and stop agreeing on simulation results. For every root package
//! (each shipped binary and each harness package) and every platform CI builds for, this
//! module takes the resolved dependency tree, keeps the boundary crates and everything they
//! depend on, and requires the result to agree across roots. Across the shipped binaries
//! (`[feature_roots]`) it must be identical: the same packages, at the same versions, with
//! the same features. A harness package that is not a shipped binary is held to its shared
//! packages only: every package its tree has in common with another root's must be compiled
//! with the same features, but its tree may hold packages another lacks and lack packages
//! another holds. Two harness packages always differ by membership (each holds itself and
//! its own dev-dependencies, which no other tree reaches), and a feature difference in a
//! package both compile is what would make them disagree on a simulation result.
//!
//! The tree is read from `cargo tree --format "{p}|{f}"`, which prints the resolved feature
//! list of every package. It carries the same information as the feature nodes of
//! `cargo tree -e features` in a form that is compared per package.
//!
//! A package can be compiled twice in one build, once for the target and once for the host
//! (as a build-dependency or below a proc macro), with different features each time. The two
//! are compared separately. A harness package's tree includes its dev-dependencies, because
//! a test build compiles them and they take part in feature resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use crate::workspace::Workspace;
use crate::{Finding, POLICY_FILE};

/// The platforms CI builds for. Each has its own feature resolution.
pub const TRIPLES: [&str; 3] =
    ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-apple-darwin"];

/// What a package of the tree is compiled for.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Built {
    /// For the platform the binary runs on.
    Target,
    /// For the build machine: a build-dependency, a proc macro, or anything below either.
    Host,
}

/// The packages of one root's boundary tree: (`name version`, what it is built for) to the
/// feature lists it is compiled with. There is one list unless cargo printed the package
/// with different features in the same role, which is itself worth seeing in a finding.
pub type BoundaryTree = BTreeMap<(String, Built), BTreeSet<String>>;

/// The section of a package's dependencies that the following lines belong to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Normal,
    Build,
    Dev,
}

/// One line of the tree: a package, or the header of a dependency section.
enum Line {
    Package { depth: usize, name: String, package: String, features: String, proc_macro: bool },
    Header { depth: usize, section: Section },
}

fn parse_line(line: &str) -> Result<Line, String> {
    let bad = || format!("unexpected `cargo tree` line: `{line}`");
    // Each level of the tree is drawn with four characters.
    let drawn = line.chars().take_while(|c| matches!(c, '│' | '├' | '└' | '─' | ' ')).count();
    let depth = drawn / 4;
    let rest: String = line.chars().skip(drawn).collect();
    if rest.starts_with('[') {
        let section = match rest.as_str() {
            "[build-dependencies]" => Section::Build,
            "[dev-dependencies]" => Section::Dev,
            _ => return Err(bad()),
        };
        return Ok(Line::Header { depth, section });
    }
    let rest = rest.strip_suffix(" (*)").unwrap_or(&rest);
    let (package, features) = rest.rsplit_once('|').ok_or_else(bad)?;
    let mut words = package.split_whitespace();
    let name = words.next().ok_or_else(bad)?;
    let version = words.next().filter(|v| v.starts_with('v')).ok_or_else(bad)?;
    Ok(Line::Package {
        depth,
        name: name.to_owned(),
        package: format!("{name} {version}"),
        features: features.trim().to_owned(),
        proc_macro: package.contains("(proc-macro)"),
    })
}

/// A node of the tree: a package in one role with one feature list.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Node {
    name: String,
    package: String,
    built: Built,
    features: String,
}

/// Reduces the output of [`dump`] to the boundary crates and everything they depend on.
///
/// # Errors
/// If a line is not in the format [`dump`] asks cargo for.
pub fn boundary_tree(
    tree: &str,
    in_boundary: &dyn Fn(&str) -> bool,
) -> Result<BoundaryTree, String> {
    // A repeated package is printed once with its dependencies and afterwards as a `(*)`
    // line without them, so edges are collected per node and not per line.
    let mut children: BTreeMap<Node, BTreeSet<Node>> = BTreeMap::new();
    // The node at each depth of the current path, and the section its next children are in.
    let mut path: Vec<(Node, Section)> = Vec::new();
    for line in tree.lines().filter(|line| !line.trim().is_empty()) {
        match parse_line(line)? {
            Line::Header { depth, section } => {
                path.truncate(depth + 1);
                let (_, current) = path
                    .last_mut()
                    .ok_or_else(|| format!("section header with no package: `{line}`"))?;
                *current = section;
            }
            Line::Package { depth, name, package, features, proc_macro } => {
                path.truncate(depth);
                let built = match path.last() {
                    Some((parent, section))
                        if parent.built == Built::Host || *section == Section::Build =>
                    {
                        Built::Host
                    }
                    _ if proc_macro => Built::Host,
                    _ => Built::Target,
                };
                let node = Node { name, package, built, features };
                if let Some((parent, _)) = path.last() {
                    children.entry(parent.clone()).or_default().insert(node.clone());
                }
                children.entry(node.clone()).or_default();
                path.push((node, Section::Normal));
            }
        }
    }

    let mut reached: BTreeSet<&Node> = BTreeSet::new();
    let mut pending: Vec<&Node> = children.keys().filter(|node| in_boundary(&node.name)).collect();
    while let Some(node) = pending.pop() {
        if reached.insert(node) {
            pending.extend(children.get(node).into_iter().flatten());
        }
    }
    let mut result = BoundaryTree::new();
    for node in reached {
        result
            .entry((node.package.clone(), node.built))
            .or_default()
            .insert(format!("[{}]", node.features.replace(',', ", ")));
    }
    Ok(result)
}

/// One root of the comparison: the path to report against (the root package's manifest),
/// its boundary tree, and whether its membership is compared too.
pub struct Root {
    /// The root package's manifest.
    pub path: String,
    /// Its boundary tree on the platform being compared.
    pub tree: BoundaryTree,
    /// A shipped binary (`[feature_roots]`): its tree must be identical to every other
    /// shipped binary's. A harness package that is not one is compared on shared packages.
    pub full: bool,
}

/// Compares the roots' boundary trees for one platform. The shipped binaries are compared
/// in full against the first of them; each harness package is compared against every root
/// before it, on the packages both trees hold.
#[must_use]
pub fn compare(triple: &str, roots: &[Root]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let full: Vec<&Root> = roots.iter().filter(|root| root.full).collect();
    if let Some((reference, others)) = full.split_first() {
        for root in others {
            differences(triple, root, reference, true, &mut findings);
        }
    }
    for (index, root) in roots.iter().enumerate().filter(|(_, root)| !root.full) {
        for reference in &roots[..index] {
            differences(triple, root, reference, false, &mut findings);
        }
    }
    findings
}

/// The packages whose feature lists differ between `root` and `reference`: every package
/// of either tree when `membership` is compared, else those both trees hold.
fn differences(
    triple: &str,
    root: &Root,
    reference: &Root,
    membership: bool,
    findings: &mut Vec<Finding>,
) {
    let describe = |features: Option<&BTreeSet<String>>| match features {
        Some(lists) => lists.iter().cloned().collect::<Vec<_>>().join(" and "),
        None => "absent".to_owned(),
    };
    let packages: BTreeSet<&(String, Built)> =
        reference.tree.keys().chain(root.tree.keys()).collect();
    for key in packages {
        let (here, there) = (root.tree.get(key), reference.tree.get(key));
        let compared = membership || (here.is_some() && there.is_some());
        if compared && here != there {
            let (package, built) = key;
            let role = if *built == Built::Host { "host" } else { "target" };
            findings.push(Finding::new(
                &root.path,
                format!(
                    "on {triple}, boundary dependency `{package}` ({role}) is {} here but {} \
                     for {}; every build must compile the boundary identically",
                    describe(here),
                    describe(there),
                    reference.path
                ),
            ));
        }
    }
}

/// Shipped binaries that the comparison would silently skip: workspace packages named
/// `lactose-app-*` that are not among `roots`, the packages of `[feature_roots]`.
#[must_use]
pub fn unlisted_binaries(ws: &Workspace, roots: &[String]) -> Vec<Finding> {
    ws.packages
        .iter()
        .filter(|p| p.name.starts_with("lactose-app-") && !roots.contains(&p.name))
        .map(|p| {
            Finding::new(
                p.manifest(),
                format!(
                    "`{}` is a shipped binary but is not in [feature_roots] in {POLICY_FILE}, so \
                     its boundary feature tree is compared with nothing",
                    p.name
                ),
            )
        })
        .collect()
}

/// Runs `cargo tree` for `package` as built for `triple`, over normal and build edges and,
/// if `dev` is set, dev edges too.
///
/// # Errors
/// If cargo cannot be run or fails.
pub fn dump(
    cargo: &str,
    dir: &Path,
    package: &str,
    triple: &str,
    dev: bool,
) -> Result<String, String> {
    let edges = if dev { "normal,build,dev" } else { "normal,build" };
    let output = Command::new(cargo)
        .args(["tree", "--locked", "--package", package])
        .args(["--edges", edges, "--target", triple])
        // Never coloured: CI sets `CARGO_TERM_COLOR=always`, and a colour code is not a tree
        // character.
        .args(["--format", "{p}|{f}", "--color", "never"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run `{cargo} tree`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{cargo} tree --package {package} --target {triple}` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::{package, workspace};

    fn boundary(name: &str) -> bool {
        name == "lactose-core" || name == "lactose-math"
    }

    /// The server: `core` with bytemuck's derive, `math` with libm and a build script.
    const SERVER: &str = "\
lactose-app-server v0.0.0 (/ws/crates/app-server)|
├── lactose-core v0.0.0 (/ws/crates/core)|
│   └── bytemuck v1.25.2|bytemuck_derive,derive
│       └── bytemuck_derive v1.12.1 (proc-macro)|
│           └── syn v3.0.6|default,derive
├── lactose-math v0.0.0 (/ws/crates/math)|
│   ├── lactose-core v0.0.0 (/ws/crates/core)| (*)
│   └── libm v0.2.16|default
│   [build-dependencies]
│   └── cfg-if v1.0.4|
└── lactose-net v0.0.0 (/ws/crates/net)|
    └── tracing v0.1.44|std
";

    fn tree(text: &str) -> BoundaryTree {
        boundary_tree(text, &boundary).unwrap()
    }

    fn roots(server: &str, client: &str) -> [Root; 2] {
        [
            Root {
                path: "crates/app-server/Cargo.toml".to_owned(),
                tree: tree(server),
                full: true,
            },
            Root {
                path: "crates/app-client/Cargo.toml".to_owned(),
                tree: tree(client),
                full: true,
            },
        ]
    }

    fn boundary_three(name: &str) -> bool {
        ["lactose-core", "lactose-math", "lactose-sim", "lactose-physics-api"].contains(&name)
    }

    /// Two harness packages as the workspace has them: `sim` with a dev-dependency of its
    /// own, and `physics-api`, which depends on `sim` and holds itself.
    const SIM: &str = "\
lactose-sim v0.0.0 (/ws/crates/sim)|
├── lactose-core v0.0.0 (/ws/crates/core)|
│   └── bytemuck v1.25.2|bytemuck_derive,derive
├── lactose-math v0.0.0 (/ws/crates/math)|
│   └── libm v0.2.16|arch
[dev-dependencies]
└── xxhash-rust v0.8.19|xxh64
";
    const PHYSICS: &str = "\
lactose-physics-api v0.0.0 (/ws/crates/physics-api)|
├── lactose-core v0.0.0 (/ws/crates/core)|
│   └── bytemuck v1.25.2|bytemuck_derive,derive
└── lactose-sim v0.0.0 (/ws/crates/sim)|
    ├── lactose-core v0.0.0 (/ws/crates/core)| (*)
    └── lactose-math v0.0.0 (/ws/crates/math)|
        └── libm v0.2.16|arch
";

    fn harness(sim: &str, physics: &str) -> [Root; 2] {
        let tree = |text: &str| boundary_tree(text, &boundary_three).unwrap();
        [
            Root { path: "crates/sim/Cargo.toml".to_owned(), tree: tree(sim), full: false },
            Root {
                path: "crates/physics-api/Cargo.toml".to_owned(),
                tree: tree(physics),
                full: false,
            },
        ]
    }

    #[test]
    fn harness_packages_that_differ_only_by_membership_pass() {
        let roots = harness(SIM, PHYSICS);
        // The trees do differ in membership: each holds a package the other lacks.
        assert!(roots[0].tree.keys().any(|key| key.0.starts_with("xxhash-rust")));
        assert!(roots[1].tree.keys().any(|key| key.0.starts_with("lactose-physics-api")));
        assert!(!roots[1].tree.keys().any(|key| key.0.starts_with("xxhash-rust")));
        assert_eq!(compare("t", &roots), []);
        // As shipped binaries, the same two trees would be findings.
        let full = roots.map(|root| Root { full: true, ..root });
        assert_eq!(compare("t", &full).len(), 2);
    }

    #[test]
    fn a_shared_package_compiled_differently_by_two_harness_packages_is_reported() {
        let physics = PHYSICS.replace("libm v0.2.16|arch", "libm v0.2.16|arch,unstable");
        let found: Vec<String> =
            compare("t", &harness(SIM, &physics)).iter().map(ToString::to_string).collect();
        assert_eq!(
            found,
            ["crates/physics-api/Cargo.toml: on t, boundary dependency `libm v0.2.16` (target) \
                 is [arch, unstable] here but [arch] for crates/sim/Cargo.toml; every build must \
                 compile the boundary identically"]
        );
        // And against a shipped binary as well as another harness package.
        let mut roots: Vec<Root> = harness(SIM, &physics).into_iter().collect();
        let server = boundary_tree(SIM, &boundary_three).unwrap();
        roots.insert(
            0,
            Root { path: "crates/app-server/Cargo.toml".to_owned(), tree: server, full: true },
        );
        let found = compare("t", &roots);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|finding| finding.to_string().contains("[arch, unstable] here")));
    }

    #[test]
    fn the_boundary_tree_is_the_boundary_crates_and_their_dependencies() {
        let found = tree(SERVER);
        let packages: Vec<(&str, Built, Vec<&str>)> = found
            .iter()
            .map(|((p, built), f)| (p.as_str(), *built, f.iter().map(String::as_str).collect()))
            .collect();
        assert_eq!(
            packages,
            [
                ("bytemuck v1.25.2", Built::Target, vec!["[bytemuck_derive, derive]"]),
                ("bytemuck_derive v1.12.1", Built::Host, vec!["[]"]),
                ("cfg-if v1.0.4", Built::Host, vec!["[]"]),
                ("lactose-core v0.0.0", Built::Target, vec!["[]"]),
                ("lactose-math v0.0.0", Built::Target, vec!["[]"]),
                ("libm v0.2.16", Built::Target, vec!["[default]"]),
                ("syn v3.0.6", Built::Host, vec!["[default, derive]"]),
            ],
            "the root, `net` and `tracing` are outside the boundary tree"
        );
    }

    #[test]
    fn identical_boundary_trees_pass_whatever_surrounds_them() {
        let client = SERVER.replace("lactose-app-server", "lactose-app-client").replace(
            "    └── tracing v0.1.44|std",
            "    ├── tracing v0.1.44|std,log\n    └── winit v0.30.0|x11",
        );
        let roots = roots(SERVER, &client);
        assert_eq!(compare("t", &roots), []);
        assert_eq!(compare("t", &roots[..1]), []);
        assert_eq!(compare("t", &[]), []);
    }

    #[test]
    fn a_feature_a_version_or_a_package_that_differs_is_reported() {
        // The client unifies an extra bytemuck feature in and resolves another libm.
        let client = SERVER
            .replace(
                "bytemuck v1.25.2|bytemuck_derive,derive",
                "bytemuck v1.25.2|bytemuck_derive,derive,extern_crate_alloc",
            )
            .replace("libm v0.2.16|default", "libm v0.2.15|default");
        let found: Vec<String> = compare("aarch64-apple-darwin", &roots(SERVER, &client))
            .iter()
            .map(ToString::to_string)
            .collect();
        let head = "crates/app-client/Cargo.toml: on aarch64-apple-darwin, boundary dependency";
        let tail =
            "for crates/app-server/Cargo.toml; every build must compile the boundary identically";
        assert_eq!(
            found,
            [
                format!(
                    "{head} `bytemuck v1.25.2` (target) is [bytemuck_derive, derive, extern_crate_alloc] here but [bytemuck_derive, derive] {tail}"
                ),
                format!("{head} `libm v0.2.15` (target) is [default] here but absent {tail}"),
                format!("{head} `libm v0.2.16` (target) is absent here but [default] {tail}"),
            ]
        );
    }

    #[test]
    fn host_and_target_feature_lists_are_compared_separately() {
        // Review item 4. `syn` is compiled for the host below a proc macro and for the
        // target by `math`. In the server the host copy has `full`; in the client the two
        // copies have swapped features. As one unordered set per package they are equal.
        let server = "\
root v0.0.0 (/ws)|
├── lactose-core v0.0.0 (/ws/crates/core)|
│   └── helper-derive v1.0.0 (proc-macro)|
│       └── syn v3.0.6|full
└── lactose-math v0.0.0 (/ws/crates/math)|
    └── syn v3.0.6|derive
";
        let client = server
            .replace("syn v3.0.6|full", "syn v3.0.6|X")
            .replace("syn v3.0.6|derive", "syn v3.0.6|full")
            .replace("syn v3.0.6|X", "syn v3.0.6|derive");
        let found = compare("t", &roots(server, &client));
        let messages: Vec<&str> = found.iter().map(|f| f.message.as_str()).collect();
        assert_eq!(messages.len(), 2, "{messages:#?}");
        assert!(messages[0].contains("`syn v3.0.6` (target) is [full] here but [derive]"));
        assert!(messages[1].contains("`syn v3.0.6` (host) is [derive] here but [full]"));
    }

    #[test]
    fn a_build_dependency_and_everything_below_it_is_host_code() {
        let text = "\
root v0.0.0 (/ws)|
└── lactose-core v0.0.0 (/ws/crates/core)|
    ├── a v1.0.0|
    [build-dependencies]
    └── cc v1.0.0|
        └── shlex v1.0.0|
    [dev-dependencies]
    └── proptest v1.0.0|std
";
        let found = tree(text);
        let built: Vec<(&str, Built)> = found.keys().map(|(p, b)| (p.as_str(), *b)).collect();
        assert_eq!(
            built,
            [
                ("a v1.0.0", Built::Target),
                ("cc v1.0.0", Built::Host),
                ("lactose-core v0.0.0", Built::Target),
                // Review item 3: a harness package's dev-dependencies are in its tree.
                ("proptest v1.0.0", Built::Target),
                ("shlex v1.0.0", Built::Host),
            ]
        );
    }

    #[test]
    fn a_line_in_another_format_is_an_error() {
        assert!(boundary_tree("lactose-core\n", &boundary).is_err());
        assert!(boundary_tree("lactose-core 1.0|\n", &boundary).is_err());
        assert!(boundary_tree("[build-dependencies]\n", &boundary).is_err());
        assert!(boundary_tree("root v1.0.0|\n[features]\n", &boundary).is_err());
    }

    #[test]
    fn a_shipped_binary_missing_from_the_roots_is_reported() {
        // Review item 5.
        let ws = workspace(vec![
            package("lactose-app", &[]),
            package("lactose-app-server", &[]),
            package("lactose-app-client", &[]),
        ]);
        let found = unlisted_binaries(&ws, &["lactose-app-server".to_owned()]);
        assert_eq!(found.len(), 1);
        assert!(found[0].to_string().starts_with(
            "crates/app-client/Cargo.toml: `lactose-app-client` is a shipped binary but is not in [feature_roots]"
        ));
    }
}
