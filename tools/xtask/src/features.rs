//! The feature-tree comparison of architecture section 8.3.
//!
//! Cargo resolves features per build, so two binaries can compile the same boundary crate
//! with different features and stop agreeing on simulation results. For every root package
//! (each shipped binary and each harness package) and every platform CI builds for, this
//! module takes the resolved dependency tree, keeps the boundary crates and everything they
//! depend on, and requires the result to be identical across roots: the same packages, at the
//! same versions, with the same features.
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

/// Compares the boundary tree of every root against the first root's, for one platform.
/// `roots` pairs a path to report against (the root package's manifest) with its tree.
#[must_use]
pub fn compare(triple: &str, roots: &[(String, BoundaryTree)]) -> Vec<Finding> {
    let Some(((reference_path, reference), others)) = roots.split_first() else {
        return Vec::new();
    };
    let describe = |features: Option<&BTreeSet<String>>| match features {
        Some(lists) => lists.iter().cloned().collect::<Vec<_>>().join(" and "),
        None => "absent".to_owned(),
    };
    let mut findings = Vec::new();
    for (path, tree) in others {
        let packages: BTreeSet<&(String, Built)> = reference.keys().chain(tree.keys()).collect();
        for key in packages {
            let (here, there) = (tree.get(key), reference.get(key));
            if here != there {
                let (package, built) = key;
                let role = if *built == Built::Host { "host" } else { "target" };
                findings.push(Finding::new(
                    path,
                    format!(
                        "on {triple}, boundary dependency `{package}` ({role}) is {} here but {} \
                         for {reference_path}; every build must compile the boundary identically",
                        describe(here),
                        describe(there)
                    ),
                ));
            }
        }
    }
    findings
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

    fn roots(server: &str, client: &str) -> [(String, BoundaryTree); 2] {
        [
            ("crates/app-server/Cargo.toml".to_owned(), tree(server)),
            ("crates/app-client/Cargo.toml".to_owned(), tree(client)),
        ]
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
