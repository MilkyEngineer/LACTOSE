//! The feature-tree comparison of architecture section 8.3.
//!
//! Cargo resolves features per build, so two binaries can compile the same boundary crate
//! with different features and stop agreeing on simulation results. For every root package
//! (each shipped binary and the SOUR test crate) this module takes the resolved dependency
//! tree, keeps the boundary crates and everything they depend on, and requires the result to
//! be identical across roots: the same packages, at the same versions, with the same features.
//!
//! The tree is read from `cargo tree --prefix depth --format "{p}|{f}"`, which prints the
//! resolved feature list of every package. It carries the same information as the feature
//! nodes of `cargo tree -e features` in a form that is compared per package.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use crate::Finding;

/// The packages of one root's boundary tree: `name version` to the feature lists it is
/// compiled with. A package compiled twice in one build (for the host and for the target)
/// can have two lists.
pub type BoundaryTree = BTreeMap<String, BTreeSet<String>>;

/// One line of the tree.
struct Node {
    depth: usize,
    /// The package name.
    name: String,
    /// `name version`.
    package: String,
    /// The comma-separated resolved features.
    features: String,
}

fn parse_line(line: &str) -> Result<Option<Node>, String> {
    let bad = || format!("unexpected `cargo tree` line: `{line}`");
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let depth: usize = line[..digits].parse().map_err(|_| bad())?;
    let rest = &line[digits..];
    // Section headers such as `[build-dependencies]` carry no package.
    if rest.starts_with('[') {
        return Ok(None);
    }
    let rest = rest.strip_suffix(" (*)").unwrap_or(rest);
    let (package, features) = rest.rsplit_once('|').ok_or_else(bad)?;
    let mut words = package.split_whitespace();
    let name = words.next().ok_or_else(bad)?;
    let version = words.next().filter(|v| v.starts_with('v')).ok_or_else(bad)?;
    Ok(Some(Node {
        depth,
        name: name.to_owned(),
        package: format!("{name} {version}"),
        features: features.trim().to_owned(),
    }))
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
    // line without them, so edges are collected per (package, features) key.
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();
    for line in tree.lines().filter(|line| !line.trim().is_empty()) {
        let Some(node) = parse_line(line)? else {
            continue;
        };
        let key = format!("{}|{}", node.package, node.features);
        stack.truncate(node.depth);
        if let Some(parent) = stack.last() {
            children.entry(parent.clone()).or_default().insert(key.clone());
        }
        stack.push(key.clone());
        nodes.insert(key, node);
    }

    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut pending: Vec<&str> = nodes
        .iter()
        .filter(|(_, node)| in_boundary(&node.name))
        .map(|(key, _)| key.as_str())
        .collect();
    while let Some(key) = pending.pop() {
        if reached.insert(key) {
            pending.extend(children.get(key).into_iter().flatten().map(String::as_str));
        }
    }
    let mut result = BoundaryTree::new();
    for key in reached {
        let node = &nodes[key];
        result
            .entry(node.package.clone())
            .or_default()
            .insert(format!("[{}]", node.features.replace(',', ", ")));
    }
    Ok(result)
}

/// Compares the boundary tree of every root against the first root's. `roots` pairs a path
/// to report against (the root package's manifest) with its tree.
#[must_use]
pub fn compare(roots: &[(String, BoundaryTree)]) -> Vec<Finding> {
    let Some(((reference_path, reference), others)) = roots.split_first() else {
        return Vec::new();
    };
    let describe = |features: Option<&BTreeSet<String>>| match features {
        Some(lists) => lists.iter().cloned().collect::<Vec<_>>().join(" and "),
        None => "absent".to_owned(),
    };
    let mut findings = Vec::new();
    for (path, tree) in others {
        let packages: BTreeSet<&String> = reference.keys().chain(tree.keys()).collect();
        for package in packages {
            let (here, there) = (tree.get(package), reference.get(package));
            if here != there {
                findings.push(Finding::new(
                    path,
                    format!(
                        "boundary dependency `{package}` is {} here but {} for {reference_path}; \
                         every build must compile the boundary identically",
                        describe(here),
                        describe(there)
                    ),
                ));
            }
        }
    }
    findings
}

/// Runs `cargo tree` for `package` over normal and build edges on every target platform.
///
/// # Errors
/// If cargo cannot be run or fails.
pub fn dump(cargo: &str, dir: &Path, package: &str) -> Result<String, String> {
    let output = Command::new(cargo)
        .args(["tree", "--locked", "--package", package])
        .args(["--edges", "normal,build", "--target", "all"])
        .args(["--prefix", "depth", "--format", "{p}|{f}"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run `{cargo} tree`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{cargo} tree --package {package}` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary(name: &str) -> bool {
        name == "lactose-core" || name == "lactose-math"
    }

    /// The server: `core` with bytemuck's derive, `math` with libm.
    const SERVER: &str = "\
0lactose-app-server v0.0.0 (/ws/crates/app-server)|
1lactose-core v0.0.0 (/ws/crates/core)|
2bytemuck v1.25.2|bytemuck_derive,derive
3bytemuck_derive v1.12.1 (proc-macro)|
4syn v3.0.6|default,derive
1lactose-math v0.0.0 (/ws/crates/math)|
2lactose-core v0.0.0 (/ws/crates/core)| (*)
2libm v0.2.16|default
1lactose-net v0.0.0 (/ws/crates/net)|
2tracing v0.1.44|std
";

    #[test]
    fn the_boundary_tree_is_the_boundary_crates_and_their_dependencies() {
        let tree = boundary_tree(SERVER, &boundary).unwrap();
        let packages: Vec<(&str, Vec<&str>)> = tree
            .iter()
            .map(|(p, f)| (p.as_str(), f.iter().map(String::as_str).collect()))
            .collect();
        assert_eq!(
            packages,
            [
                ("bytemuck v1.25.2", vec!["[bytemuck_derive, derive]"]),
                ("bytemuck_derive v1.12.1", vec!["[]"]),
                ("lactose-core v0.0.0", vec!["[]"]),
                ("lactose-math v0.0.0", vec!["[]"]),
                ("libm v0.2.16", vec!["[default]"]),
                ("syn v3.0.6", vec!["[default, derive]"]),
            ],
            "the root, `net` and `tracing` are outside the boundary tree"
        );
    }

    #[test]
    fn identical_boundary_trees_pass_whatever_surrounds_them() {
        let client = SERVER
            .replace("lactose-app-server", "lactose-app-client")
            .replace("2tracing v0.1.44|std", "2tracing v0.1.44|std,log\n1winit v0.30.0|x11");
        let roots = [
            ("crates/app-server/Cargo.toml".to_owned(), boundary_tree(SERVER, &boundary).unwrap()),
            ("crates/app-client/Cargo.toml".to_owned(), boundary_tree(&client, &boundary).unwrap()),
        ];
        assert_eq!(compare(&roots), []);
        assert_eq!(compare(&roots[..1]), []);
        assert_eq!(compare(&[]), []);
    }

    #[test]
    fn a_feature_a_version_or_a_package_that_differs_is_reported() {
        // The client unifies an extra bytemuck feature in and resolves another libm.
        let client = SERVER
            .replace(
                "2bytemuck v1.25.2|bytemuck_derive,derive",
                "2bytemuck v1.25.2|bytemuck_derive,derive,extern_crate_alloc",
            )
            .replace("2libm v0.2.16|default", "2libm v0.2.15|default");
        let roots = [
            ("crates/app-server/Cargo.toml".to_owned(), boundary_tree(SERVER, &boundary).unwrap()),
            ("crates/app-client/Cargo.toml".to_owned(), boundary_tree(&client, &boundary).unwrap()),
        ];
        let found: Vec<String> = compare(&roots).iter().map(ToString::to_string).collect();
        let tail =
            "for crates/app-server/Cargo.toml; every build must compile the boundary identically";
        assert_eq!(
            found,
            [
                format!(
                    "crates/app-client/Cargo.toml: boundary dependency `bytemuck v1.25.2` is [bytemuck_derive, derive, extern_crate_alloc] here but [bytemuck_derive, derive] {tail}"
                ),
                format!(
                    "crates/app-client/Cargo.toml: boundary dependency `libm v0.2.15` is [default] here but absent {tail}"
                ),
                format!(
                    "crates/app-client/Cargo.toml: boundary dependency `libm v0.2.16` is absent here but [default] {tail}"
                ),
            ]
        );
    }

    #[test]
    fn a_package_built_twice_keeps_both_feature_lists() {
        let text = "\
0root v0.0.0 (/ws)|
1lactose-core v0.0.0 (/ws/crates/core)|
2bytemuck v1.25.2|derive
2helper v1.0.0|
3bytemuck v1.25.2|
";
        let tree = boundary_tree(text, &boundary).unwrap();
        assert_eq!(tree["bytemuck v1.25.2"].len(), 2);
    }

    #[test]
    fn a_line_in_another_format_is_an_error() {
        assert!(boundary_tree("lactose-core v0.0.0\n", &boundary).is_err());
        assert!(boundary_tree("0lactose-core\n", &boundary).is_err());
        assert!(boundary_tree("0[build-dependencies]\n", &boundary).unwrap().is_empty());
    }
}
