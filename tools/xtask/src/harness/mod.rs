//! The mode runner of SOUR, the determinism harness (architecture section 8.3).
//!
//! A mode is a list of run configurations whose world hashes must agree. The runner launches
//! the simulation tests once per configuration, each time naming a fresh hash file through
//! the environment protocol of [`protocol`], and then compares the files: every (test, tick)
//! recorded under one configuration must be recorded with the same hash under the others.
//!
//! The runner does not know what a simulation is. It launches whatever it is given (in the
//! binary, `cargo nextest run` over the packages listed in the policy file) and reads what
//! the tests recorded, so [`run`] is tested here with an in-process launcher.
//!
//! Besides the hashes, the runner checks two things each test owes it: the configuration the
//! test says it applied must be the one the run was launched with, and a test under a chaos
//! seed must have left its execution log.
//!
//! # Output layout
//!
//! Under the output directory (`target/sour` in the binary), each run owns
//! `<mode>/<variant>/`:
//!
//! - `hashes.tsv`: the records of that run;
//! - `logs/<test>.execlog`: the execution logs of a chaos run (see
//!   [`protocol::execution_log_path`]). They are deleted when the run agreed with its
//!   reference and kept when anything about it was reported, and CI uploads the output
//!   directory when the job fails.
//!
//! A configuration that two selected modes share (every mode is measured against the same
//! baseline) is launched once and its records reused, except where the mode is the repeat.

#[allow(
    unfulfilled_lint_expectations,
    reason = "the boundary lints these expectations name are configured only in the boundary crates that include this file"
)]
pub mod protocol;
pub mod stand_in;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use crate::workspace::Workspace;
use crate::{Finding, POLICY_FILE};
use protocol::{
    Capacity, ENV_HASH_FILE, ENV_LOG_DIR, Layout, Line, RECORD_VERSION, Record, RunConfig,
    execution_log_path, parse_lines,
};

/// Workers in every configuration that does not vary them. Fixed, not the machine's core
/// count, so the workers mode compares one worker against several on any runner.
const BASELINE_WORKERS: NonZeroU32 = NonZeroU32::new(4).unwrap();

/// The seed of the layout permutations. Fixed so a failure reproduces from the mode name.
const LAYOUT_SEED: u64 = 0x5eed_1a70;

/// The seeds of the chaos runs.
const CHAOS_SEEDS: [(&str, u64); 3] = [("seed-1", 1), ("seed-2", 2), ("seed-3", 3)];

/// The crate whose existence means there are simulation tests to run.
const SIMULATION_CRATE: &str = "lactose-sim";

/// One run configuration of a mode.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Variant {
    /// Names the run's output directory and appears in findings.
    pub label: &'static str,
    /// What the test processes are told.
    pub config: RunConfig,
    /// Whether a pull request runs it; nightly runs everything.
    pub pull_request: bool,
    /// Whether the run must be launched even if an identical configuration already ran.
    pub fresh: bool,
}

/// One row of the mode table of architecture section 8.3. The first variant is the reference
/// the others are compared against.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Mode {
    /// The mode name, as `--mode` takes it.
    pub name: &'static str,
    /// What the mode varies, for `cargo xtask sour matrix`.
    pub varies: &'static str,
    /// The run configurations.
    pub variants: Vec<Variant>,
}

/// The mode matrix. A pull request runs workers mode and one layout capacity; the rest runs
/// nightly. Architecture mode has a single variant here because its comparison is across
/// machines: each machine emits its baseline hashes and [`compare_files`] compares them.
#[must_use]
pub fn matrix() -> Vec<Mode> {
    let baseline_config =
        RunConfig { workers: Some(BASELINE_WORKERS), layout: Layout::Production, chaos_seed: None };
    let baseline = |pull_request| Variant {
        label: "baseline",
        config: baseline_config,
        pull_request,
        fresh: false,
    };
    let layout = |label, capacity, pull_request| Variant {
        label,
        config: RunConfig {
            layout: Layout::Perturbed { capacity, seed: LAYOUT_SEED },
            ..baseline_config
        },
        pull_request,
        fresh: false,
    };
    let mut chaos = vec![baseline(false)];
    chaos.extend(CHAOS_SEEDS.map(|(label, seed)| Variant {
        label,
        config: RunConfig { chaos_seed: Some(seed), ..baseline_config },
        pull_request: false,
        fresh: false,
    }));
    vec![
        Mode {
            name: "workers",
            varies: "1 worker versus 4",
            variants: vec![
                baseline(true),
                Variant {
                    label: "one-worker",
                    config: RunConfig { workers: NonZeroU32::new(1), ..baseline_config },
                    pull_request: true,
                    fresh: false,
                },
            ],
        },
        Mode {
            name: "repeat",
            varies: "the same configuration twice",
            variants: vec![
                baseline(false),
                Variant { label: "again", fresh: true, ..baseline(false) },
            ],
        },
        Mode {
            name: "layout",
            varies: "chunk capacity (1, 7, 64 rows, 16 KiB) with seeded permutation",
            variants: vec![
                baseline(true),
                layout("rows-1", Capacity::Rows(1), false),
                // The pull-request capacity: small enough that every archetype spans many
                // chunks and splits fall mid-batch, without the cost of one row per chunk.
                layout("rows-7", Capacity::Rows(7), true),
                layout("rows-64", Capacity::Rows(64), false),
                layout("bytes-16384", Capacity::Bytes(16 * 1024), false),
            ],
        },
        Mode {
            name: "chaos",
            varies: "seeded steal order, injected yields, forced split-on-steal",
            variants: chaos,
        },
        Mode {
            name: "architecture",
            varies: "the machine: emit hashes on each, then `sour compare`",
            variants: vec![baseline(false)],
        },
    ]
}

/// Which part of the matrix to run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Selection {
    /// The pull-request variants of every mode that has any.
    PullRequest,
    /// Every variant of every mode.
    Nightly,
    /// Every variant of the named modes.
    Modes(Vec<String>),
}

/// The variants of one mode that a selection runs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan<'a> {
    /// The mode.
    pub mode: &'a Mode,
    /// Its selected variants, reference first.
    pub variants: Vec<&'a Variant>,
}

/// Applies a selection to the matrix.
///
/// # Errors
/// If the selection names a mode the matrix does not have.
pub fn plan<'a>(matrix: &'a [Mode], selection: &Selection) -> Result<Vec<Plan<'a>>, String> {
    if let Selection::Modes(names) = selection {
        for name in names {
            if !matrix.iter().any(|mode| mode.name == name) {
                let known: Vec<&str> = matrix.iter().map(|mode| mode.name).collect();
                return Err(format!("unknown mode `{name}`; the modes are {}", known.join(", ")));
            }
        }
    }
    let plans = matrix.iter().filter_map(|mode| {
        let variants: Vec<&Variant> = match selection {
            Selection::PullRequest => mode.variants.iter().filter(|v| v.pull_request).collect(),
            Selection::Nightly => mode.variants.iter().collect(),
            Selection::Modes(names) if names.iter().any(|n| n == mode.name) => {
                mode.variants.iter().collect()
            }
            Selection::Modes(_) => Vec::new(),
        };
        (!variants.is_empty()).then_some(Plan { mode, variants })
    });
    Ok(plans.collect())
}

/// A workspace that has a simulation and no harness package: every `sour` command would be
/// green without running anything, so this is reported until `[harness]` lists a package.
#[must_use]
pub fn unlisted_simulation(ws: &Workspace, packages: &[String]) -> Vec<Finding> {
    match ws.package(SIMULATION_CRATE) {
        Some(simulation) if packages.is_empty() => vec![Finding::new(
            simulation.manifest(),
            format!(
                "`{SIMULATION_CRATE}` exists but [harness] in {POLICY_FILE} lists no packages, \
                 so no simulation test runs under SOUR"
            ),
        )],
        _ => Vec::new(),
    }
}

/// The hashes one run recorded, keyed by (test, tick).
pub type Hashes = BTreeMap<(String, u32), u64>;

/// What one hash file holds.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Recorded {
    hashes: Hashes,
    /// Every configuration each test said it applied.
    applied: BTreeMap<String, BTreeSet<RunConfig>>,
}

/// Collects the lines of a hash file. A (test, tick) recorded twice with one hash is one
/// entry; recorded with two hashes it is a finding against `path`, since a single run then
/// disagrees with itself.
fn collect(lines: Vec<Line>, path: &str, findings: &mut Vec<Finding>) -> Recorded {
    let mut recorded = Recorded::default();
    for line in lines {
        match line {
            Line::Applied { test, config } => {
                recorded.applied.entry(test).or_default().insert(config);
            }
            Line::Hash(Record { test, tick, hash }) => {
                let key = (test, tick);
                match recorded.hashes.get(&key) {
                    Some(&earlier) if earlier != hash => findings.push(Finding::new(
                        path,
                        format!(
                            "test `{}` recorded both {earlier:016x} and {hash:016x} at tick {} \
                             in one run",
                            key.0, key.1
                        ),
                    )),
                    Some(_) => {}
                    None => {
                        recorded.hashes.insert(key, hash);
                    }
                }
            }
        }
    }
    recorded
}

/// Compares the hashes of a run against the reference run's. For each test, the first tick
/// at which the two differ (or at which only one has a record) is one finding against
/// `path`; later ticks of that test are consequences and are not reported.
#[must_use]
pub fn compare(
    reference_path: &str,
    reference: &Hashes,
    path: &str,
    hashes: &Hashes,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut reported: Option<&str> = None;
    let keys: BTreeSet<&(String, u32)> = reference.keys().chain(hashes.keys()).collect();
    for key in keys {
        let (test, tick) = (key.0.as_str(), key.1);
        let (here, there) = (hashes.get(key), reference.get(key));
        if here == there || reported == Some(test) {
            continue;
        }
        reported = Some(test);
        let show = |hash: Option<&u64>| {
            hash.map_or_else(|| "no record".to_owned(), |h| format!("{h:016x}"))
        };
        findings.push(Finding::new(
            path,
            format!(
                "test `{test}` first differs at tick {tick}: {} here, {} in {reference_path}",
                show(here),
                show(there)
            ),
        ));
    }
    findings
}

/// Compares two hash files, as architecture mode does with the files two machines emitted.
/// A reference with no hashes is a finding: the comparison would otherwise pass having
/// compared nothing.
///
/// # Errors
/// If either text is not a hash file.
pub fn compare_files(
    reference_path: &str,
    reference: &str,
    path: &str,
    text: &str,
) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    let parse = |path: &str, text: &str, findings: &mut Vec<Finding>| {
        parse_lines(text)
            .map(|lines| collect(lines, path, findings).hashes)
            .map_err(|e| format!("{path}: {e}"))
    };
    let reference_hashes = parse(reference_path, reference, &mut findings)?;
    let hashes = parse(path, text, &mut findings)?;
    if reference_hashes.is_empty() {
        findings.push(Finding::new(
            reference_path,
            "holds no hashes, so the comparison would compare nothing",
        ));
    }
    findings.extend(compare(reference_path, &reference_hashes, path, &hashes));
    Ok(findings)
}

/// The text of a hash file holding `hashes`, sorted, under a header comment that names the
/// record format version.
#[must_use]
pub fn emit(hashes: &Hashes) -> String {
    let mut text = format!("# lactose-sour-hashes {RECORD_VERSION}\n");
    for ((test, tick), &hash) in hashes {
        let line = Line::Hash(Record { test: test.clone(), tick: *tick, hash });
        text.push_str(&format!("{line}\n"));
    }
    text
}

/// Where a run happens.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dirs {
    /// The absolute repository root; finding paths are shown relative to it.
    pub root: PathBuf,
    /// The absolute output directory.
    pub out: PathBuf,
}

impl Dirs {
    fn shown(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().replace('\\', "/")
    }
}

/// Launches the simulation tests once with the given environment variables set, and reports
/// whether the launch itself succeeded (for `cargo nextest run`, whether every test passed).
pub type Launcher<'a> = dyn FnMut(&[(&'static str, String)]) -> Result<(), String> + 'a;

/// What [`run`] did.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Outcome {
    /// Every mismatch and failure, in plan order.
    pub findings: Vec<Finding>,
    /// How many times the launcher was called.
    pub launches: usize,
    /// The hashes of the first reference run, which is the baseline of every mode.
    pub baseline: Option<Hashes>,
}

/// One finished run of a variant.
struct Run {
    config: RunConfig,
    hash_file: String,
    log_dir: PathBuf,
    hashes: Hashes,
    /// Whether anything about the run was reported; its execution logs are then kept.
    reported: bool,
}

/// Runs the planned modes and compares the hashes each recorded.
///
/// A reference run that records nothing is itself a finding: with no hashes, a mode would
/// pass without having checked anything.
pub fn run(plans: &[Plan<'_>], dirs: &Dirs, launch: &mut Launcher<'_>) -> Outcome {
    let mut outcome = Outcome::default();
    let mut finished: Vec<Run> = Vec::new();
    for plan in plans {
        let mut indices = Vec::new();
        for variant in &plan.variants {
            let reusable = finished.iter().position(|run| run.config == variant.config);
            let index = match reusable {
                Some(index) if !variant.fresh => index,
                _ => {
                    let dir = dirs.out.join(plan.mode.name).join(variant.label);
                    finished.push(launch_variant(variant, &dir, dirs, launch, &mut outcome));
                    finished.len() - 1
                }
            };
            indices.push(index);
        }
        let Some((&reference, others)) = indices.split_first() else {
            continue;
        };
        if finished[reference].hashes.is_empty() {
            outcome.findings.push(Finding::new(
                &finished[reference].hash_file,
                format!(
                    "{} mode recorded no hashes, so it compared nothing; simulation tests record \
                     through the protocol in tools/xtask/src/harness/protocol.rs",
                    plan.mode.name
                ),
            ));
        }
        for &other in others {
            let (reference, other_run) = (&finished[reference], &finished[other]);
            let mismatches = compare(
                &reference.hash_file,
                &reference.hashes,
                &other_run.hash_file,
                &other_run.hashes,
            );
            if !mismatches.is_empty() {
                finished[other].reported = true;
                outcome.findings.extend(mismatches);
            }
        }
    }
    // A log is evidence only for a run that was reported.
    for run in finished.iter().filter(|run| !run.reported) {
        let _ = fs::remove_dir_all(&run.log_dir);
    }
    outcome.baseline = finished.into_iter().next().map(|run| run.hashes);
    outcome
}

fn launch_variant(
    variant: &Variant,
    dir: &Path,
    dirs: &Dirs,
    launch: &mut Launcher<'_>,
    outcome: &mut Outcome,
) -> Run {
    let hash_path = dir.join("hashes.tsv");
    let log_dir = dir.join("logs");
    let hash_file = dirs.shown(&hash_path);
    let before = outcome.findings.len();
    let mut run = Run {
        config: variant.config,
        hash_file: hash_file.clone(),
        log_dir: log_dir.clone(),
        hashes: Hashes::new(),
        reported: true,
    };
    // A stale hash file would be appended to, so the directory starts empty.
    let prepared = match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => fs::create_dir_all(&log_dir),
    };
    if let Err(e) = prepared {
        outcome
            .findings
            .push(Finding::new(dirs.shown(dir), format!("cannot prepare the run directory: {e}")));
        return run;
    }

    let mut vars = variant.config.to_vars();
    vars.push((ENV_HASH_FILE, hash_path.to_string_lossy().into_owned()));
    vars.push((ENV_LOG_DIR, log_dir.to_string_lossy().into_owned()));
    outcome.launches += 1;
    if let Err(e) = launch(&vars) {
        outcome.findings.push(Finding::new(
            dirs.shown(dir),
            format!("the tests failed under `{}` ({e})", variant.label),
        ));
    }

    let text = match fs::read_to_string(&hash_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            outcome.findings.push(Finding::new(hash_file, e.to_string()));
            return run;
        }
    };
    let recorded = match parse_lines(&text) {
        Ok(lines) => collect(lines, &hash_file, &mut outcome.findings),
        Err(e) => {
            outcome.findings.push(Finding::new(hash_file, e.to_string()));
            return run;
        }
    };

    let tests: BTreeSet<&str> = recorded.hashes.keys().map(|(test, _)| test.as_str()).collect();
    for test in tests {
        let applied = recorded.applied.get(test);
        let honest = applied.is_some_and(|set| set.iter().all(|config| *config == variant.config));
        if !honest {
            let said = applied.map_or_else(
                || "did not say what it applied".to_owned(),
                |set| {
                    let all: Vec<String> = set.iter().map(|c| format!("`{c}`")).collect();
                    format!("says it applied {}", all.join(" and "))
                },
            );
            outcome.findings.push(Finding::new(
                &hash_file,
                format!(
                    "test `{test}` was launched with `{}` but {said}; its hashes prove nothing \
                     about this configuration",
                    variant.config
                ),
            ));
        }
        if variant.config.chaos_seed.is_some() && !execution_log_path(&log_dir, test).is_file() {
            outcome.findings.push(Finding::new(
                dirs.shown(&execution_log_path(&log_dir, test)),
                format!(
                    "test `{test}` ran under a chaos seed and left no execution log, so a \
                     divergence could not be replayed"
                ),
            ));
        }
    }
    run.hashes = recorded.hashes;
    run.reported = outcome.findings.len() > before;
    run
}

/// Launches one named stand-in test with the given variables; see [`self_test`].
pub type StandInLauncher<'a> =
    dyn FnMut(&str, &[(&'static str, String)]) -> Result<(), String> + 'a;

/// What the runner must report for one flawed stand-in, run under one mode.
struct Expectation {
    flaw: stand_in::Flaw,
    mode: &'static str,
    /// How many findings, each of which must contain `needle`.
    findings: usize,
    needle: String,
    /// Whether every non-reference run must have kept its execution log.
    logs_kept: bool,
}

/// Proves the runner on the stand-in simulations of [`stand_in`], through whatever `launch`
/// does (in the binary, a real `cargo nextest run` per configuration):
///
/// - the sound stand-in passes every mode of the nightly matrix, and no execution log of it
///   survives;
/// - each flawed stand-in is reported by the mode that exists to catch it, in every varied
///   configuration, at the tick its flaw starts, and a chaos divergence keeps its logs;
/// - a stand-in that ignores its configuration is reported for that, though its hashes agree.
///
/// Returns the reasons the proof failed; none means it held.
pub fn self_test(dirs: &Dirs, launch: &mut StandInLauncher<'_>) -> Vec<Finding> {
    use stand_in::{FLAW_STARTS_AT, Flaw, TICKS, test_name};

    let matrix = matrix();
    let mut findings = Vec::new();
    let under =
        |name: &str| Dirs { root: dirs.root.clone(), out: dirs.out.join("self-test").join(name) };

    let sound = under(test_name(Flaw::None));
    let plans = plan(&matrix, &Selection::Nightly).unwrap_or_default();
    let outcome = run(&plans, &sound, &mut |vars| launch(test_name(Flaw::None), vars));
    let recorded = outcome.baseline.as_ref().map_or(0, BTreeMap::len);
    if recorded != TICKS as usize {
        findings.push(Finding::new(
            dirs.shown(&sound.out),
            format!(
                "the sound stand-in recorded {recorded} hashes under the baseline, expected {TICKS}"
            ),
        ));
    }
    findings.extend(outcome.findings.into_iter().map(|finding| {
        Finding::new(
            finding.path,
            format!("the runner reported the sound stand-in: {}", finding.message),
        )
    }));
    let kept = kept_logs(&sound.out);
    if !kept.is_empty() {
        findings.push(Finding::new(
            dirs.shown(&kept[0]),
            "the runner kept an execution log of a chaos run that agreed with its reference",
        ));
    }

    let differs =
        |flaw| format!("test `{}` first differs at tick {FLAW_STARTS_AT}: ", test_name(flaw));
    let varied =
        |mode: &str| matrix.iter().find(|m| m.name == mode).map_or(0, |m| m.variants.len() - 1);
    let expectations = [
        Expectation {
            flaw: Flaw::WorkerOrder,
            mode: "workers",
            findings: varied("workers"),
            needle: differs(Flaw::WorkerOrder),
            logs_kept: false,
        },
        Expectation {
            flaw: Flaw::Unrepeatable,
            mode: "repeat",
            findings: varied("repeat"),
            needle: differs(Flaw::Unrepeatable),
            logs_kept: false,
        },
        Expectation {
            flaw: Flaw::LayoutOrder,
            mode: "layout",
            findings: varied("layout"),
            needle: differs(Flaw::LayoutOrder),
            logs_kept: false,
        },
        Expectation {
            flaw: Flaw::StealOrder,
            mode: "chaos",
            findings: varied("chaos"),
            needle: differs(Flaw::StealOrder),
            logs_kept: true,
        },
        // Both runs of workers mode: neither applied what it was launched with.
        Expectation {
            flaw: Flaw::IgnoresConfiguration,
            mode: "workers",
            findings: varied("workers") + 1,
            needle: "but says it applied `workers=default layout=production chaos=off`".to_owned(),
            logs_kept: false,
        },
    ];
    for expected in expectations {
        let name = test_name(expected.flaw);
        let here = under(name);
        let selection = Selection::Modes(vec![expected.mode.to_owned()]);
        let plans = plan(&matrix, &selection).unwrap_or_default();
        let outcome = run(&plans, &here, &mut |vars| launch(name, vars));
        let caught = outcome.findings.len() == expected.findings
            && outcome.findings.iter().all(|f| f.message.contains(&expected.needle));
        if !caught {
            let got: Vec<String> = outcome.findings.iter().map(ToString::to_string).collect();
            findings.push(Finding::new(
                dirs.shown(&here.out),
                format!(
                    "{} mode should report the `{name}` stand-in {} time(s), each containing \
                     `{}`; the runner reported {got:?}",
                    expected.mode, expected.findings, expected.needle
                ),
            ));
        }
        if expected.logs_kept && kept_logs(&here.out).len() != expected.findings {
            findings.push(Finding::new(
                dirs.shown(&here.out),
                format!(
                    "{} mode should keep the execution log of each of the {} diverging runs of \
                     `{name}`; it kept {:?}",
                    expected.mode,
                    expected.findings,
                    kept_logs(&here.out)
                ),
            ));
        }
    }
    findings
}

/// Every execution log under `dir`, sorted.
fn kept_logs(dir: &Path) -> Vec<PathBuf> {
    let mut logs = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "execlog") {
                logs.push(path);
            }
        }
    }
    logs.sort();
    logs
}

#[cfg(test)]
mod tests {
    use super::protocol::Session;
    use super::stand_in::{Flaw, STAND_INS};
    use super::*;
    use crate::workspace::fixture::{package, workspace};

    /// A scratch output directory, removed on drop.
    struct Scratch(Dirs);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("lactose-harness-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            Self(Dirs { out: root.join("target").join("sour"), root })
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.root);
        }
    }

    fn session(vars: &[(&'static str, String)]) -> Result<Session, String> {
        let lookup = |name: &str| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone());
        Session::from_vars(&lookup).map_err(|e| e.to_string())
    }

    /// Runs a stand-in in this process, as a test process would after reading its
    /// environment.
    fn simulate(test: &str, flaw: Flaw, vars: &[(&'static str, String)]) -> Result<(), String> {
        stand_in::run(&session(vars)?, test, flaw).map_err(|e| e.to_string())
    }

    /// What `tests/harness_stand_in.rs` does: the stand-in the test name stands for.
    fn honest(test: &str, vars: &[(&'static str, String)]) -> Result<(), String> {
        let flaw = STAND_INS.iter().find(|(name, _)| *name == test).map(|(_, flaw)| *flaw);
        simulate(test, flaw.ok_or("no such stand-in")?, vars)
    }

    fn hashes(records: &[(&str, u32, u64)]) -> Hashes {
        records.iter().map(|&(test, tick, hash)| ((test.to_owned(), tick), hash)).collect()
    }

    fn lines(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(ToString::to_string).collect()
    }

    fn only(mode: &str) -> Selection {
        Selection::Modes(vec![mode.to_owned()])
    }

    #[test]
    fn the_matrix_is_the_table_of_section_8_3() {
        let matrix = matrix();
        let names: Vec<&str> = matrix.iter().map(|mode| mode.name).collect();
        assert_eq!(names, ["workers", "repeat", "layout", "chaos", "architecture"]);

        let layout = &matrix[2];
        let capacities: Vec<Layout> =
            layout.variants[1..].iter().map(|v| v.config.layout).collect();
        let perturbed = |capacity| Layout::Perturbed { capacity, seed: LAYOUT_SEED };
        assert_eq!(
            capacities,
            [
                perturbed(Capacity::Rows(1)),
                perturbed(Capacity::Rows(7)),
                perturbed(Capacity::Rows(64)),
                perturbed(Capacity::Bytes(16 * 1024)),
            ]
        );

        // Every mode is measured against the same baseline, and labels name directories.
        for mode in &matrix {
            assert_eq!(mode.variants[0].config, matrix[0].variants[0].config, "{}", mode.name);
            let mut labels: Vec<&str> = mode.variants.iter().map(|v| v.label).collect();
            labels.sort_unstable();
            labels.dedup();
            assert_eq!(labels.len(), mode.variants.len(), "{}", mode.name);
        }
        assert!(matrix[3].variants[1..].iter().all(|v| v.config.chaos_seed.is_some()));
        assert_eq!(matrix[0].variants[1].config.workers, NonZeroU32::new(1));
    }

    #[test]
    fn a_pull_request_runs_workers_mode_and_one_layout_capacity() {
        let matrix = matrix();
        let plans = plan(&matrix, &Selection::PullRequest).unwrap();
        let shape: Vec<(&str, Vec<&str>)> = plans
            .iter()
            .map(|p| (p.mode.name, p.variants.iter().map(|v| v.label).collect()))
            .collect();
        assert_eq!(
            shape,
            [("workers", vec!["baseline", "one-worker"]), ("layout", vec!["baseline", "rows-7"])]
        );

        let nightly = plan(&matrix, &Selection::Nightly).unwrap();
        assert_eq!(nightly.len(), matrix.len());
        assert!(nightly.iter().zip(&matrix).all(|(p, m)| p.variants.len() == m.variants.len()));

        let chaos = plan(&matrix, &only("chaos")).unwrap();
        assert_eq!(chaos.len(), 1);
        assert_eq!(chaos[0].variants.len(), 4);
        assert!(plan(&matrix, &only("speed")).is_err());
    }

    #[test]
    fn comparison_reports_the_first_differing_tick_of_each_test() {
        let reference =
            hashes(&[("a", 0, 1), ("a", 1, 2), ("a", 2, 3), ("b", 0, 9), ("c", 0, 5), ("c", 1, 6)]);
        assert_eq!(compare("ref", &reference, "run", &reference), []);

        let run = hashes(&[("a", 0, 1), ("a", 1, 7), ("a", 2, 8), ("c", 0, 5), ("d", 4, 4)]);
        assert_eq!(
            lines(&compare("ref", &reference, "run", &run)),
            [
                "run: test `a` first differs at tick 1: 0000000000000007 here, 0000000000000002 in ref",
                "run: test `b` first differs at tick 0: no record here, 0000000000000009 in ref",
                "run: test `c` first differs at tick 1: no record here, 0000000000000006 in ref",
                "run: test `d` first differs at tick 4: 0000000000000004 here, no record in ref",
            ]
        );
    }

    #[test]
    fn hash_files_compare_across_machines() {
        let linux = emit(&hashes(&[("a", 0, 1), ("a", 1, 2)]));
        assert_eq!(
            linux,
            "# lactose-sour-hashes 2\nhash\ta\t0\t0000000000000001\nhash\ta\t1\t0000000000000002\n"
        );
        assert_eq!(compare_files("linux", &linux, "macos", &linux), Ok(vec![]));

        let macos = emit(&hashes(&[("a", 0, 1), ("a", 1, 3)]));
        let findings = compare_files("linux", &linux, "macos", &macos).unwrap();
        assert_eq!(
            lines(&findings),
            [
                "macos: test `a` first differs at tick 1: 0000000000000003 here, 0000000000000002 in linux"
            ]
        );

        // A run that disagrees with itself is reported even when the other file agrees.
        let twice = format!("{linux}hash\ta\t1\t0000000000000004\n");
        let findings = compare_files("linux", &linux, "macos", &twice).unwrap();
        assert_eq!(
            lines(&findings),
            [
                "macos: test `a` recorded both 0000000000000002 and 0000000000000004 at tick 1 in one run"
            ]
        );
        assert!(
            compare_files("linux", &linux, "macos", "garbage\n")
                .unwrap_err()
                .starts_with("macos: line 1")
        );
    }

    #[test]
    fn comparing_against_an_empty_reference_is_a_finding() {
        // Review item 5: two header-only files used to compare equal.
        let empty = emit(&Hashes::new());
        assert_eq!(
            lines(&compare_files("linux", &empty, "macos", &empty).unwrap()),
            ["linux: holds no hashes, so the comparison would compare nothing"]
        );
    }

    #[test]
    fn a_simulation_without_a_harness_package_is_a_finding() {
        // Review item 5.
        let with = workspace(vec![package("lactose-sim", &[]), package("lactose-core", &[])]);
        let without = workspace(vec![package("lactose-core", &[])]);
        assert_eq!(
            lines(&unlisted_simulation(&with, &[])),
            [
                "crates/sim/Cargo.toml: `lactose-sim` exists but [harness] in tools/xtask/ci-policy.toml \
              lists no packages, so no simulation test runs under SOUR"
            ]
        );
        assert_eq!(unlisted_simulation(&with, &["sim-tests".to_owned()]), []);
        assert_eq!(unlisted_simulation(&without, &[]), []);
    }

    #[test]
    fn a_sound_simulation_passes_every_mode_and_shared_baselines_run_once() {
        let scratch = Scratch::new("sound");
        let matrix = matrix();
        let plans = plan(&matrix, &Selection::Nightly).unwrap();
        let outcome = run(&plans, &scratch.0, &mut |vars| simulate("sim", Flaw::None, vars));
        assert_eq!(outcome.findings, []);
        // Ten launches: the baseline once, one-worker, the fresh repeat, four layouts and
        // three chaos seeds. Architecture mode reuses the baseline.
        assert_eq!(outcome.launches, 10);
        assert_eq!(outcome.baseline.unwrap().len(), stand_in::TICKS as usize);
        assert!(scratch.0.out.join("repeat/again/hashes.tsv").is_file());
        assert!(!scratch.0.out.join("layout/baseline").exists(), "the baseline was reused");
        // Review item 7: the logs of chaos runs that agreed are deleted.
        assert!(scratch.0.out.join("chaos/seed-2/hashes.tsv").is_file());
        assert_eq!(kept_logs(&scratch.0.out), [] as [PathBuf; 0]);
    }

    #[test]
    fn each_flaw_is_caught_by_its_mode() {
        let matrix = matrix();
        // An ordering flaw is run under the whole matrix: only its own mode reports it. A
        // run that never repeats differs from every other run, so it gets repeat mode alone.
        let cases = [
            (Flaw::WorkerOrder, Selection::Nightly, "workers", 1),
            (Flaw::LayoutOrder, Selection::Nightly, "layout", 4),
            (Flaw::StealOrder, Selection::Nightly, "chaos", 3),
            (Flaw::Unrepeatable, only("repeat"), "repeat", 1),
        ];
        for (flaw, selection, mode, count) in cases {
            let scratch = Scratch::new(mode);
            let plans = plan(&matrix, &selection).unwrap();
            let outcome = run(&plans, &scratch.0, &mut |vars| simulate("sim", flaw, vars));
            let found = lines(&outcome.findings);
            assert_eq!(found.len(), count, "{flaw:?}: {found:#?}");
            let start = format!("target/sour/{mode}/");
            let tick = format!(": test `sim` first differs at tick {}: ", stand_in::FLAW_STARTS_AT);
            for finding in &found {
                assert!(finding.starts_with(&start) && finding.contains(&tick), "{finding}");
                assert!(finding.ends_with("/baseline/hashes.tsv"), "{finding}");
            }
        }
    }

    #[test]
    fn a_diverging_chaos_run_keeps_its_execution_log_and_an_agreeing_one_does_not() {
        // Review item 7.
        let scratch = Scratch::new("chaos-logs");
        let matrix = matrix();
        let plans = plan(&matrix, &only("chaos")).unwrap();
        // Diverges under seed 2 only.
        let outcome = run(&plans, &scratch.0, &mut |vars| {
            let seed = session(vars)?.config().chaos_seed;
            simulate("sim::t", if seed == Some(2) { Flaw::StealOrder } else { Flaw::None }, vars)
        });
        assert_eq!(outcome.findings.len(), 1, "{:#?}", outcome.findings);
        assert_eq!(
            kept_logs(&scratch.0.out),
            [scratch.0.out.join("chaos/seed-2/logs/sim_3a_3at.execlog")]
        );
    }

    #[test]
    fn a_chaos_run_without_an_execution_log_is_a_finding() {
        // Review item 7: a real test that forgets the log must not pass.
        let scratch = Scratch::new("chaos-no-log");
        let matrix = matrix();
        let plans = plan(&matrix, &only("chaos")).unwrap();
        let outcome = run(&plans, &scratch.0, &mut |vars| {
            simulate("sim", Flaw::None, vars)?;
            let log_dir = &vars.iter().find(|(n, _)| *n == ENV_LOG_DIR).unwrap().1;
            fs::remove_dir_all(log_dir).map_err(|e| e.to_string())
        });
        let found = lines(&outcome.findings);
        assert_eq!(found.len(), 3, "{found:#?}");
        assert_eq!(
            found[0],
            "target/sour/chaos/seed-1/logs/sim.execlog: test `sim` ran under a chaos seed and left \
             no execution log, so a divergence could not be replayed"
        );
    }

    #[test]
    fn a_test_that_ignores_or_withholds_its_configuration_is_a_finding() {
        // Review item 6: the hashes agree, because the configuration was never applied.
        let scratch = Scratch::new("ignored");
        let matrix = matrix();
        let plans = plan(&matrix, &only("workers")).unwrap();
        let outcome =
            run(&plans, &scratch.0, &mut |vars| simulate("sim", Flaw::IgnoresConfiguration, vars));
        assert_eq!(
            lines(&outcome.findings),
            [
                "target/sour/workers/baseline/hashes.tsv: test `sim` was launched with `workers=4 layout=production chaos=off` but says it applied `workers=default layout=production chaos=off`; its hashes prove nothing about this configuration",
                "target/sour/workers/one-worker/hashes.tsv: test `sim` was launched with `workers=1 layout=production chaos=off` but says it applied `workers=default layout=production chaos=off`; its hashes prove nothing about this configuration",
            ]
        );

        // Hashes with no `applied` line at all, as a version 1 recorder would have written.
        let outcome = run(&plans, &scratch.0, &mut |vars| {
            let file = &vars.iter().find(|(n, _)| *n == ENV_HASH_FILE).unwrap().1;
            fs::write(file, "hash\tsim\t0\t0000000000000001\n").map_err(|e| e.to_string())
        });
        let found = lines(&outcome.findings);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(found[0].contains("but did not say what it applied"), "{}", found[0]);
    }

    #[test]
    fn a_failed_launch_an_empty_run_and_a_corrupt_file_are_findings() {
        let scratch = Scratch::new("failures");
        let matrix = matrix();
        let plans = plan(&matrix, &only("workers")).unwrap();

        let outcome = run(&plans, &scratch.0, &mut |_| Ok(()));
        assert_eq!(
            lines(&outcome.findings),
            ["target/sour/workers/baseline/hashes.tsv: workers mode recorded no hashes, so it \
              compared nothing; simulation tests record through the protocol in \
              tools/xtask/src/harness/protocol.rs"]
        );

        let outcome = run(&plans, &scratch.0, &mut |vars| {
            simulate("sim", Flaw::None, vars)?;
            Err("exit status 100".to_owned())
        });
        assert_eq!(
            lines(&outcome.findings),
            [
                "target/sour/workers/baseline: the tests failed under `baseline` (exit status 100)",
                "target/sour/workers/one-worker: the tests failed under `one-worker` (exit status 100)",
            ]
        );

        let outcome = run(&plans, &scratch.0, &mut |vars| {
            let file = &vars.iter().find(|(n, _)| *n == ENV_HASH_FILE).unwrap().1;
            fs::write(file, "not a record\n").map_err(|e| e.to_string())
        });
        assert!(lines(&outcome.findings)[0].starts_with(
            "target/sour/workers/baseline/hashes.tsv: line 1 is not a version 2 record"
        ));
    }

    #[test]
    fn a_rerun_does_not_see_the_previous_runs_records() {
        let scratch = Scratch::new("rerun");
        let matrix = matrix();
        let plans = plan(&matrix, &only("workers")).unwrap();
        for _ in 0..2 {
            let outcome = run(&plans, &scratch.0, &mut |vars| simulate("sim", Flaw::None, vars));
            assert_eq!(outcome.findings, []);
        }
        let text = fs::read_to_string(scratch.0.out.join("workers/baseline/hashes.tsv")).unwrap();
        // One `applied` line and one hash per tick.
        assert_eq!(text.lines().count(), 1 + stand_in::TICKS as usize);
    }

    #[test]
    fn the_self_test_holds_only_when_the_runner_tells_the_stand_ins_apart() {
        let scratch = Scratch::new("self-test");
        let mut launches = 0;
        let held = self_test(&scratch.0, &mut |test, vars| {
            launches += 1;
            honest(test, vars)
        });
        assert_eq!(held, []);
        // The nightly matrix for the sound stand-in, then workers, repeat, layout, chaos and
        // workers again for the flawed ones.
        assert_eq!(launches, 10 + 2 + 2 + 5 + 4 + 2);

        // A runner that could not see any flaw would be handed six sound simulations.
        let blind = self_test(&scratch.0, &mut |test, vars| simulate(test, Flaw::None, vars));
        let reasons = lines(&blind);
        assert_eq!(reasons.len(), 6, "{reasons:#?}");
        for (mode, name) in [
            ("workers", "worker_order"),
            ("repeat", "unrepeatable"),
            ("layout", "layout_order"),
            ("chaos", "steal_order"),
            ("workers", "ignores_configuration"),
        ] {
            let start = format!(
                "target/sour/self-test/{name}: {mode} mode should report the `{name}` stand-in"
            );
            assert!(reasons.iter().any(|r| r.starts_with(&start)), "{start}\n{reasons:#?}");
        }
        assert!(reasons.iter().any(|r| r.contains("chaos mode should keep the execution log")));

        // A sound stand-in that mismatches fails the proof too.
        let noisy =
            self_test(&scratch.0, &mut |test, vars| simulate(test, Flaw::WorkerOrder, vars));
        assert!(
            noisy.iter().any(|f| f.message.starts_with("the runner reported the sound stand-in")),
            "{noisy:?}"
        );

        // So do tests that never ran.
        let absent = self_test(&scratch.0, &mut |_, _| Ok(()));
        assert!(
            absent[0].message.starts_with("the sound stand-in recorded 0 hashes"),
            "{absent:?}"
        );
    }
}
