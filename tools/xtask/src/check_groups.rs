//! The package groups of CI's check jobs (`.github/workflows/ci.yml`): each check job builds,
//! lints and tests one group, on each platform, so that no one job builds the whole workspace's
//! tests (milestone 2 plan, decision 5.15). The groups are `[check_groups]` of the policy file,
//! and a job reads its group's packages from [`arguments`] at run time, so the table is the
//! only list.
//!
//! [`check`] fails a workspace package that is in no group, which no check job would build or
//! test, or in two, which two jobs would; an entry that names no workspace package; and a group
//! that names none, which as cargo arguments would select the whole workspace.

use std::collections::BTreeMap;

use crate::policy::Policy;
use crate::workspace::Workspace;
use crate::{Finding, POLICY_FILE};

/// Checks that the groups divide the workspace's packages between them.
#[must_use]
pub fn check(policy: &Policy, ws: &Workspace) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut groups_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for group in &policy.check_groups {
        if group.packages.is_empty() {
            findings.push(Finding::new(
                POLICY_FILE,
                format!(
                    "[check_groups] `{}` lists no package, and as cargo arguments selects the \
                     whole workspace",
                    group.name
                ),
            ));
        }
        for name in &group.packages {
            let groups = groups_of.entry(name).or_default();
            if groups.contains(&group.name) {
                findings.push(Finding::new(
                    POLICY_FILE,
                    format!("[check_groups] `{}` lists `{name}` twice", group.name),
                ));
            } else {
                groups.push(group.name);
            }
            if ws.package(name).is_none() {
                findings.push(Finding::new(
                    POLICY_FILE,
                    format!(
                        "[check_groups] `{}` lists `{name}`, which is not a workspace package",
                        group.name
                    ),
                ));
            }
        }
    }
    for package in &ws.packages {
        let groups = groups_of.get(package.name.as_str()).map_or(&[][..], Vec::as_slice);
        let message = match groups {
            [_] => continue,
            [] => format!(
                "`{}` is in no group of [check_groups] in {POLICY_FILE}, so no CI check job \
                 builds, lints or tests it; add it to one",
                package.name
            ),
            _ => format!(
                "`{}` is in more than one group of [check_groups] in {POLICY_FILE} ({}), so \
                 more than one CI check job builds and tests it; keep it in one",
                package.name,
                groups.iter().map(|group| format!("`{group}`")).collect::<Vec<_>>().join(", ")
            ),
        };
        findings.push(Finding::new(package.manifest(), message));
    }
    findings
}

/// The cargo arguments that select the packages of the group `name`, `-p <package>` each in
/// the table's order, for a check job to pass to clippy, nextest and the doc tests.
///
/// # Errors
/// If no group is called `name`, or [`check`] finds anything: a job must not test a division
/// of the workspace that leaves a package out.
pub fn arguments(policy: &Policy, ws: &Workspace, name: &str) -> Result<String, String> {
    let findings = check(policy, ws);
    if !findings.is_empty() {
        let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
        return Err(format!(
            "the check groups do not divide the workspace, so no group is printed:\n{}",
            lines.join("\n")
        ));
    }
    let group = policy.check_groups.iter().find(|group| group.name == name).ok_or_else(|| {
        let names: Vec<String> =
            policy.check_groups.iter().map(|group| format!("`{}`", group.name)).collect();
        format!("no check group is called `{name}`; [check_groups] has {}", names.join(" and "))
    })?;
    Ok(group.packages.iter().map(|package| format!("-p {package}")).collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::CheckGroup;
    use crate::workspace::fixture::{package, workspace};

    /// A workspace of `names`, each a package with no edges.
    fn members(names: &[&str]) -> Workspace {
        workspace(names.iter().map(|name| package(name, &[])).collect())
    }

    /// A policy with only the groups set.
    fn groups(first: &[&str], second: &[&str]) -> Policy {
        let group = |name, packages: &[&str]| CheckGroup {
            name,
            packages: packages.iter().map(|package| (*package).to_owned()).collect(),
        };
        Policy {
            check_groups: vec![group("base", first), group("physics", second)],
            ..Policy::default()
        }
    }

    fn run(policy: &Policy, ws: &Workspace) -> Vec<String> {
        check(policy, ws).iter().map(ToString::to_string).collect()
    }

    #[test]
    fn groups_that_divide_the_workspace_pass_and_print_their_packages() {
        let ws = members(&["lactose-core", "lactose-math", "lactose-sim"]);
        let policy = groups(&["lactose-math", "lactose-core"], &["lactose-sim"]);
        assert_eq!(run(&policy, &ws), [] as [&str; 0]);
        assert_eq!(arguments(&policy, &ws, "base").unwrap(), "-p lactose-math -p lactose-core");
        assert_eq!(arguments(&policy, &ws, "physics").unwrap(), "-p lactose-sim");
    }

    #[test]
    fn a_package_in_no_group_fails_the_check() {
        // A crate a later lane adds and leaves out of the table: no check job would build,
        // lint or test it.
        let ws = members(&["lactose-core", "lactose-new", "lactose-sim"]);
        let policy = groups(&["lactose-core"], &["lactose-sim"]);
        assert_eq!(
            run(&policy, &ws),
            ["crates/new/Cargo.toml: `lactose-new` is in no group of [check_groups] in \
              tools/xtask/ci-policy.toml, so no CI check job builds, lints or tests it; add it \
              to one"]
        );
        let refused = arguments(&policy, &ws, "base").unwrap_err();
        assert!(refused.contains("`lactose-new` is in no group"), "{refused}");
    }

    #[test]
    fn a_package_in_two_groups_or_twice_in_one_is_reported() {
        let ws = members(&["lactose-core", "lactose-sim"]);
        let policy = groups(&["lactose-core", "lactose-sim"], &["lactose-sim", "lactose-sim"]);
        assert_eq!(
            run(&policy, &ws),
            [
                "tools/xtask/ci-policy.toml: [check_groups] `physics` lists `lactose-sim` \
                 twice",
                "crates/sim/Cargo.toml: `lactose-sim` is in more than one group of \
                 [check_groups] in tools/xtask/ci-policy.toml (`base`, `physics`), so \
                 more than one CI check job builds and tests it; keep it in one",
            ]
        );
    }

    #[test]
    fn an_entry_that_names_no_package_and_an_empty_group_are_reported() {
        let ws = members(&["lactose-core"]);
        let policy = groups(&["lactose-core", "lactose-gone"], &[]);
        assert_eq!(
            run(&policy, &ws),
            [
                "tools/xtask/ci-policy.toml: [check_groups] `base` lists `lactose-gone`, \
                 which is not a workspace package",
                "tools/xtask/ci-policy.toml: [check_groups] `physics` lists no package, and \
                 as cargo arguments selects the whole workspace",
            ]
        );
        assert!(arguments(&policy, &ws, "physics").is_err());
    }

    #[test]
    fn an_unknown_group_is_refused() {
        let ws = members(&["lactose-core", "lactose-sim"]);
        let policy = groups(&["lactose-core"], &["lactose-sim"]);
        assert_eq!(
            arguments(&policy, &ws, "rest").unwrap_err(),
            "no check group is called `rest`; [check_groups] has `base` and `physics`"
        );
    }

    #[test]
    fn the_checked_in_groups_name_every_package_once() {
        // The real division is checked against the real workspace by `cargo xtask boundary`;
        // here, only that the table is two groups of distinct names.
        let policy = Policy::parse(include_str!("../ci-policy.toml")).unwrap();
        let names: Vec<&str> = policy.check_groups.iter().map(|group| group.name).collect();
        assert_eq!(names, ["base", "physics"]);
        let mut all: Vec<&String> =
            policy.check_groups.iter().flat_map(|group| &group.packages).collect();
        let listed = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), listed);
    }

    /// The line `head` of `text` and every line after it that is blank or indented deeper
    /// than `head`, so that a key added anywhere in a block is inside it; `None` unless `head`
    /// is exactly one line of `text`.
    fn block(text: &str, head: &str) -> Option<String> {
        let lines: Vec<&str> = text.lines().collect();
        let mut at = lines.iter().enumerate().filter(|(_, line)| **line == head);
        let (Some((start, _)), None) = (at.next(), at.next()) else { return None };
        let indent = |line: &str| line.len() - line.trim_start().len();
        let end = lines[start + 1..]
            .iter()
            .position(|line| !line.trim().is_empty() && indent(line) <= indent(head))
            .map_or(lines.len(), |after| start + 1 + after);
        let block: String = lines[start..end].iter().map(|line| format!("{line}\n")).collect();
        Some(format!("{}\n", block.trim_end()))
    }

    /// What the workflow's `check` job must hold for the groups `names` to be tested: a job
    /// head whose matrix runs each group on each platform with no `include`, `exclude` or job
    /// condition, and steps that hand each group its packages with no condition that names
    /// a group. Each is compared as the whole text of its block, so a key added to one fails.
    /// The messages of what differs.
    fn workflow_misses(names: &[&str], workflow: &str) -> Vec<String> {
        const HEAD: &str = "  check:\n    name: check (${{ matrix.os }}, ${{ matrix.group }})\n    \
             runs-on: ${{ matrix.os }}\n    timeout-minutes: 15\n    strategy:\n      \
             fail-fast: false\n      matrix:\n        os: [ubuntu-latest, windows-latest, \
             macos-latest]\n        group: [GROUPS]\n    steps:\n";
        const PACKAGES: &str = "${{ steps.group.outputs.packages }}";
        let Some(job) = block(workflow, "  check:") else {
            return vec!["ci.yml has no one `check` job".to_owned()];
        };
        let head = HEAD.replace("GROUPS", &names.join(", "));
        let mut misses = Vec::new();
        // After the head, only the steps' lines, each indented deeper than the job's keys.
        let steps = job.strip_prefix(&head).filter(|steps| {
            steps.lines().all(|line| line.trim().is_empty() || line.starts_with("      "))
        });
        let Some(steps) = steps else {
            return vec![format!(
                "the `check` job does not begin, and hold no other key than:\n{head}"
            )];
        };
        let expected = [
            (
                "      - name: Packages",
                "      - name: Packages\n        id: group\n        shell: bash\n        env:\n          \
                 GROUP: ${{ matrix.group }}\n        run: |\n          \
                 packages=$(cargo xtask group \"$GROUP\")\n          \
                 echo \"packages=$packages\" >> \"$GITHUB_OUTPUT\"\n"
                    .to_owned(),
            ),
            (
                "      - name: Clippy",
                format!(
                    "      - name: Clippy\n        if: matrix.os != 'windows-latest'\n        run: \
                     cargo clippy {PACKAGES} --all-targets --locked -- -D warnings\n"
                ),
            ),
            (
                "      - name: Test",
                format!(
                    "      - name: Test\n        run: cargo nextest run {PACKAGES} --locked \
                     --no-tests=pass\n"
                ),
            ),
            (
                "      - name: Doctests",
                format!("      - name: Doctests\n        run: cargo test --doc {PACKAGES} --locked\n"),
            ),
        ];
        for (head, text) in expected {
            if block(steps, head).as_deref() != Some(text.as_str()) {
                misses.push(format!("the `check` job's step is not, as one step:\n{text}"));
            }
        }
        misses
    }

    #[test]
    fn the_workflow_runs_every_group_and_hands_each_step_its_packages() {
        // `check` above holds the table to the workspace; this holds the workflow to the
        // table. A group the matrix leaves out, or a step that skips a group, is a group no
        // job tests, and `cargo xtask group` cannot see either. A change to these lines of
        // ci.yml is a change to this test, made on purpose.
        let policy = Policy::parse(include_str!("../ci-policy.toml")).unwrap();
        let names: Vec<&str> = policy.check_groups.iter().map(|group| group.name).collect();
        let workflow = include_str!("../../../.github/workflows/ci.yml");
        assert_eq!(workflow_misses(&names, workflow), [] as [&str; 0]);
    }

    #[test]
    fn a_workflow_that_leaves_a_group_untested_fails() {
        let workflow = include_str!("../../../.github/workflows/ci.yml");
        let edited = |old: &str, new: &str| {
            assert_eq!(workflow.matches(old).count(), 1, "{old}");
            workflow.replace(old, new)
        };
        let test = "      - name: Test\n        run: cargo nextest";
        let doctests = "      - name: Doctests\n        run: cargo test --doc ${{ steps.group.outputs.packages }} --locked\n";
        let cases = [
            ("the matrix drops a group", edited("group: [base, physics]", "group: [base]")),
            (
                "a condition before a step's run",
                edited(
                    test,
                    &test.replace(
                        "        run:",
                        "        if: matrix.group == 'base'\n        run:",
                    ),
                ),
            ),
            (
                "a condition after a step's run",
                edited(doctests, &format!("{doctests}        if: matrix.group == 'base'\n")),
            ),
            (
                "a matrix exclusion",
                edited(
                    "        group: [base, physics]\n",
                    "        group: [base, physics]\n        exclude:\n          \
                     - { os: windows-latest, group: physics }\n",
                ),
            ),
            (
                "a job condition after its steps",
                edited(
                    "--release --locked\n\n  # Clippy and the release",
                    "--release --locked\n    if: matrix.group == 'base'\n\n  # Clippy and the release",
                ),
            ),
            (
                "one group's packages for every job",
                edited("GROUP: ${{ matrix.group }}", "GROUP: base"),
            ),
        ];
        for (case, text) in &cases {
            assert!(!workflow_misses(&["base", "physics"], text).is_empty(), "{case}");
        }
        // A third group in the policy, with no job in the matrix.
        assert!(!workflow_misses(&["base", "physics", "rest"], workflow).is_empty());
    }
}
