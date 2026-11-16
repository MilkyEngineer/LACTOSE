//! Each package's integration tests build as one test binary (D125): the target
//! `integration`, whose root `tests/integration.rs` declares every other file of `tests/` as a
//! module, with `autotests = false` in the manifest so that cargo makes no target of each file.
//! A test target that must stay a binary of its own is listed, with its reason, in the policy
//! file's `[test_targets]`.
//!
//! The check reads the targets `cargo metadata` reports and walks the module tree of each
//! test target from its root, as rustc resolves it: out-of-line modules, inline ones and
//! literal `include!`s, each file relative to the one that declares it. It fails a package
//! with more than one test target outside the list, an entry that names no test target, a
//! file cargo would make a target of by itself (`tests/*.rs`, `tests/*/main.rs`) that no test
//! target compiles, and a module of a test target named with a codename (`CLAUDE.md`, Names),
//! which a file named after one becomes when it is declared under its own name.

use std::collections::{BTreeSet, VecDeque};

use crate::boundary::{children, parent};
use crate::policy::Policy;
use crate::repo::Files;
use crate::rust_source::items;
use crate::workspace::{Package, Workspace};
use crate::{Finding, NAMES, POLICY_FILE};

/// The name of the one test target that holds a package's integration tests.
pub const TARGET: &str = "integration";

/// Checks every package's test targets against `[test_targets]`.
#[must_use]
pub fn check(policy: &Policy, ws: &Workspace, files: &Files) -> Vec<Finding> {
    let mut findings = Vec::new();
    let codenames = match codenames(files) {
        Ok(codenames) => codenames,
        Err(message) => {
            findings.push(Finding::new(NAMES, message));
            BTreeSet::new()
        }
    };
    for entry in &policy.separate_test_targets {
        let named = entry.split_once("::").and_then(|(package, target)| {
            ws.package(package).filter(|package| package.tests.contains_key(target))
        });
        if named.is_none() {
            findings.push(Finding::new(
                POLICY_FILE,
                format!(
                    "[test_targets] `separate` lists `{entry}`, which names no test target of a \
                     workspace member as `package::target`"
                ),
            ));
        }
    }
    // A file another package's tests include (`physics-api`'s test support) is reported once.
    let mut named = BTreeSet::new();
    for package in &ws.packages {
        targets(policy, package, &mut findings);
        files_and_modules(package, files, &codenames, &mut named, &mut findings);
    }
    findings.extend(named);
    findings
}

/// The finding for a package with more than one test target outside the list.
fn targets(policy: &Policy, package: &Package, findings: &mut Vec<Finding>) {
    let separate = |target: &str| {
        policy
            .separate_test_targets
            .iter()
            .any(|entry| *entry == format!("{}::{target}", package.name))
    };
    let joined: Vec<&str> =
        package.tests.keys().map(String::as_str).filter(|target| !separate(target)).collect();
    if joined.len() > 1 {
        findings.push(Finding::new(
            package.manifest(),
            format!(
                "`{}` builds {} test targets outside [test_targets] in {POLICY_FILE}: {}; build \
                 them as one, the target `{TARGET}` whose root `tests/{TARGET}.rs` declares each \
                 file as a module, with `autotests = false` (D125), or list a target that must \
                 be a binary of its own there with its reason",
                package.name,
                joined.len(),
                joined.iter().map(|target| format!("`{target}`")).collect::<Vec<_>>().join(", ")
            ),
        ));
    }
}

/// The finding for each file cargo would discover under the package's `tests/` that no test
/// target compiles, and, into `named`, for each module of a test target named with a
/// codename.
fn files_and_modules(
    package: &Package,
    files: &Files,
    codenames: &BTreeSet<String>,
    named: &mut BTreeSet<Finding>,
    findings: &mut Vec<Finding>,
) {
    // (file, the directory its out-of-line modules sit in): a root's are beside it.
    let mut queue: VecDeque<(String, String)> =
        package.tests.values().map(|root| (root.clone(), parent(root).to_owned())).collect();
    let mut read = BTreeSet::new();
    let mut compiled = BTreeSet::new();
    while let Some(key) = queue.pop_front() {
        if !read.insert(key.clone()) {
            continue;
        }
        let (file, dir) = key;
        compiled.insert(file.clone());
        let Some(source) = files.text(&file) else {
            continue;
        };
        let items = items(&source, &[]);
        for module in items.modules.iter().chain(&items.inline_modules) {
            if let Some(codename) = module.name.split('_').find(|word| codenames.contains(*word)) {
                // Its path in the file, through the inline modules around it.
                let path: Vec<&str> = module
                    .inline
                    .iter()
                    .map(|inline| inline.name.as_str())
                    .chain([module.name.as_str()])
                    .collect();
                named.insert(Finding::new(
                    file.clone(),
                    format!(
                        "declares the module `{}`, whose name holds the codename `{codename}`, \
                         which {NAMES} keeps off modules; name it for what it holds, declaring \
                         its file with `#[path]`",
                        path.join("::")
                    ),
                ));
            }
        }
        for (child, child_dir, _) in children(&file, &dir, &items, files) {
            queue.push_back((child, child_dir));
        }
    }
    let tests = format!("{}/tests/", package.dir);
    let discovered = files.paths().filter(|path| {
        path.strip_prefix(&tests).is_some_and(|rest| match rest.split_once('/') {
            None => rest.ends_with(".rs"),
            Some((_, file)) => file == "main.rs",
        })
    });
    for path in discovered.filter(|path| !compiled.contains(*path)) {
        findings.push(Finding::new(
            path,
            format!(
                "no test target of `{}` compiles this file: declare it as a module of \
                 `tests/{TARGET}.rs` (D125)",
                package.name
            ),
        ));
    }
}

/// The codenames of `CLAUDE.md`'s Names table, lower-cased: the first cell of each row of the
/// first table under its `## Names` heading that is one upper-case word.
///
/// # Errors
/// The file missing, or no codename found, which would let every name pass.
fn codenames(files: &Files) -> Result<BTreeSet<String>, String> {
    let text = files.text(NAMES).ok_or("is missing; the check reads its Names table")?;
    let section = text
        .lines()
        .skip_while(|line| line.trim_end() != "## Names")
        .skip(1)
        .take_while(|line| !line.starts_with("## "));
    let found: BTreeSet<String> = section
        .filter_map(|line| line.strip_prefix('|')?.split('|').next())
        .map(str::trim)
        .filter(|cell| !cell.is_empty() && cell.chars().all(|c| c.is_ascii_uppercase()))
        .map(str::to_ascii_lowercase)
        .collect();
    if found.is_empty() {
        return Err("has no codename in the table under `## Names`, which the check reads".into());
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::{package, workspace};

    const CLAUDE: &str = "# X\n\n## Names\n\n| Codename | Means |\n|---|---|\n| LACTOSE | the \
                          engine |\n| SOUR | the harness |\n\n## Rules\n\n| NOT | a codename |\n";

    fn policy(separate: &[&str]) -> Policy {
        Policy {
            separate_test_targets: separate.iter().map(|s| (*s).to_owned()).collect(),
            ..Policy::default()
        }
    }

    /// The findings for `lactose-sim` with the test targets `targets`, each as its name and
    /// its root below `crates/sim/tests/`, and `files`.
    fn run_rooted(
        policy: &Policy,
        targets: &[(&str, &str)],
        files: &[(&str, &str)],
    ) -> Vec<String> {
        let mut sim = package("lactose-sim", &[]);
        sim.tests = targets
            .iter()
            .map(|(name, root)| ((*name).to_owned(), format!("crates/sim/tests/{root}")))
            .collect();
        let mut repo = Files::default();
        repo.insert(NAMES, CLAUDE);
        for (path, text) in files {
            repo.insert(*path, *text);
        }
        check(policy, &workspace(vec![sim]), &repo).iter().map(ToString::to_string).collect()
    }

    /// The same, each target rooted at `tests/<name>.rs`.
    fn run(policy: &Policy, targets: &[&str], files: &[(&str, &str)]) -> Vec<String> {
        let roots: Vec<(&str, String)> = targets.iter().map(|t| (*t, format!("{t}.rs"))).collect();
        let roots: Vec<(&str, &str)> = roots.iter().map(|(t, r)| (*t, r.as_str())).collect();
        run_rooted(policy, &roots, files)
    }

    const ROOT: &str = "crates/sim/tests/integration.rs";

    #[test]
    fn one_target_with_every_file_declared_passes() {
        let root = "mod common;\n\nmod a;\n#[path = \"b.rs\"]\nmod bee;\n#[path = \"proof/main.rs\"]\n\
                    mod proof;\n";
        let files = [
            (ROOT, root),
            ("crates/sim/tests/a.rs", ""),
            ("crates/sim/tests/b.rs", ""),
            ("crates/sim/tests/common/mod.rs", ""),
            ("crates/sim/tests/proof/main.rs", ""),
            ("crates/sim/tests/proof/client.rs", ""),
            ("crates/sim/tests/alloc.rs", ""),
        ];
        let separate = policy(&["lactose-sim::alloc"]);
        assert_eq!(run(&separate, &["alloc", "integration"], &files), [] as [&str; 0]);
        // A package of one test file needs no root of its own.
        assert_eq!(run(&policy(&[]), &["a"], &[("crates/sim/tests/a.rs", "")]), [] as [&str; 0]);
    }

    #[test]
    fn a_second_target_outside_the_list_is_named() {
        let files =
            [(ROOT, "mod a;\n"), ("crates/sim/tests/a.rs", ""), ("crates/sim/tests/b.rs", "")];
        let found = run(&policy(&[]), &["b", "integration"], &files);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with(
                "crates/sim/Cargo.toml: `lactose-sim` builds 2 test targets outside \
                 [test_targets] in tools/xtask/ci-policy.toml: `b`, `integration`; build them as \
                 one"
            ),
            "{found:?}"
        );
    }

    #[test]
    fn a_stale_entry_is_named() {
        let files = [(ROOT, ""), ("crates/sim/tests/alloc.rs", "")];
        let found = run(
            &policy(&["lactose-sim::alloc", "lactose-sim::gone", "lactose-none::alloc", "x"]),
            &["alloc", "integration"],
            &files,
        );
        assert_eq!(
            found,
            [
                "tools/xtask/ci-policy.toml: [test_targets] `separate` lists \
                 `lactose-sim::gone`, which names no test target of a workspace member as \
                 `package::target`",
                "tools/xtask/ci-policy.toml: [test_targets] `separate` lists \
                 `lactose-none::alloc`, which names no test target of a workspace member as \
                 `package::target`",
                "tools/xtask/ci-policy.toml: [test_targets] `separate` lists `x`, which names \
                 no test target of a workspace member as `package::target`",
            ]
        );
    }

    #[test]
    fn a_file_no_target_compiles_is_named() {
        let files = [
            (ROOT, "mod a;\n#[cfg(not(miri))]\nmod c;\nmod inline {\n    mod d;\n}\n"),
            ("crates/sim/tests/a.rs", "mod nested;\n"),
            ("crates/sim/tests/b.rs", ""),
            ("crates/sim/tests/c.rs", ""),
            // `inline::d` is `tests/inline/d.rs`.
            ("crates/sim/tests/d.rs", ""),
            ("crates/sim/tests/dir/main.rs", ""),
            ("crates/sim/tests/dir/helper.rs", ""),
            ("crates/sim/tests/data/golden.txt", ""),
        ];
        let found = run(&policy(&[]), &["integration"], &files);
        let message = "no test target of `lactose-sim` compiles this file: declare it as a \
                       module of `tests/integration.rs` (D125)";
        assert_eq!(
            found,
            [
                format!("crates/sim/tests/b.rs: {message}"),
                format!("crates/sim/tests/d.rs: {message}"),
                format!("crates/sim/tests/dir/main.rs: {message}"),
            ]
        );
    }

    #[test]
    fn a_file_a_module_below_the_root_declares_is_compiled() {
        // `proof/main.rs` is a module of the root, and declares `proof/extra.rs`, which a
        // `#[path]` from `group.rs` names too; `group/inner.rs` is `group`'s own module.
        let files = [
            (ROOT, "#[path = \"proof/main.rs\"]\nmod proof;\nmod group;\n"),
            ("crates/sim/tests/proof/main.rs", "mod extra;\n"),
            ("crates/sim/tests/proof/extra.rs", ""),
            ("crates/sim/tests/group.rs", "mod inner;\n#[path = \"lone.rs\"]\nmod lone;\n"),
            ("crates/sim/tests/group/inner.rs", ""),
            ("crates/sim/tests/lone.rs", ""),
        ];
        assert_eq!(run(&policy(&[]), &["integration"], &files), [] as [&str; 0]);
    }

    #[test]
    fn a_root_in_a_directory_declares_its_modules_there() {
        // Review round 1, M3: `stand`'s `mod helper;` is `tests/stand/helper.rs`, so the
        // `tests/helper.rs` no target compiles is reported.
        let files = [
            (ROOT, "mod a;\n"),
            ("crates/sim/tests/a.rs", ""),
            ("crates/sim/tests/stand/main.rs", "mod helper;\n"),
            ("crates/sim/tests/stand/helper.rs", ""),
            ("crates/sim/tests/helper.rs", ""),
        ];
        let found = run_rooted(
            &policy(&["lactose-sim::stand"]),
            &[("integration", "integration.rs"), ("stand", "stand/main.rs")],
            &files,
        );
        assert_eq!(
            found,
            ["crates/sim/tests/helper.rs: no test target of `lactose-sim` compiles this file: \
                 declare it as a module of `tests/integration.rs` (D125)"]
        );
    }

    #[test]
    fn a_module_named_with_a_codename_is_named() {
        let root = "mod sour;\nmod sour_glue;\n#[path = \"lactose.rs\"]\nmod engine;\nmod not;\n";
        let files = [
            (ROOT, root),
            ("crates/sim/tests/sour.rs", ""),
            ("crates/sim/tests/sour_glue.rs", ""),
            ("crates/sim/tests/lactose.rs", ""),
            ("crates/sim/tests/not.rs", ""),
        ];
        let found = run(&policy(&[]), &["integration"], &files);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("the module `sour_glue`, whose name holds the codename `sour`"));
        assert!(found[1].contains("the module `sour`, whose name holds the codename `sour`"));
    }

    #[test]
    fn a_codename_module_below_the_root_is_named() {
        // Review round 1, M2: an inline module, an out-of-line one inside an inline one, and
        // one a module file below the root declares.
        let root = "#[path = \"proof/main.rs\"]\nmod proof;\nmod sour {\n    #[test]\n    fn x() \
                    {}\n}\nmod group {\n    mod sour;\n}\n";
        let files = [
            (ROOT, root),
            ("crates/sim/tests/proof/main.rs", "mod sour;\n"),
            ("crates/sim/tests/proof/sour.rs", "mod lactose_glue {}\n"),
            ("crates/sim/tests/group/sour.rs", ""),
        ];
        let found = run(&policy(&[]), &["integration"], &files);
        let message = |codename: &str, module: &str| {
            format!(
                "declares the module `{module}`, whose name holds the codename `{codename}`, \
                 which CLAUDE.md keeps off modules; name it for what it holds, declaring its \
                 file with `#[path]`"
            )
        };
        assert_eq!(
            found,
            [
                format!("{ROOT}: {}", message("sour", "group::sour")),
                format!("{ROOT}: {}", message("sour", "sour")),
                format!("crates/sim/tests/proof/main.rs: {}", message("sour", "sour")),
                format!("crates/sim/tests/proof/sour.rs: {}", message("lactose", "lactose_glue")),
            ]
        );
    }

    #[test]
    fn the_names_table_is_read_or_reported() {
        let mut repo = Files::default();
        repo.insert(NAMES, CLAUDE);
        assert_eq!(codenames(&repo).unwrap(), ["lactose".to_owned(), "sour".to_owned()].into());
        repo.insert(NAMES, "# X\n\n## Rules\n\n| SOUR | x |\n");
        assert!(codenames(&repo).unwrap_err().contains("no codename"));
        assert!(codenames(&Files::default()).unwrap_err().contains("is missing"));
    }

    #[test]
    fn the_checked_in_names_table_reads() {
        let mut repo = Files::default();
        repo.insert(NAMES, include_str!("../../../CLAUDE.md"));
        let found = codenames(&repo).unwrap();
        assert!(found.contains("sour") && found.contains("whisk") && found.len() > 10, "{found:?}");
    }
}
