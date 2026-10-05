//! The SOUR mode runner (architecture section 8.3).
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
//! # Output layout
//!
//! Under the output directory (`target/sour` in the binary), each run owns
//! `<mode>/<variant>/`:
//!
//! - `hashes.tsv`: the hash records of that run;
//! - `logs/<test>.execlog`: the execution log a failing chaos test wrote (see
//!   [`protocol::execution_log_path`]). CI uploads the whole output directory when the job
//!   fails, so the log survives the runner.
//!
//! A configuration that two selected modes share (every mode is measured against the same
//! baseline) is launched once and its records reused, except where the mode is the repeat.

pub mod protocol;
pub mod stand_in;

use std::collections::BTreeMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use crate::Finding;
use protocol::{Capacity, ENV_HASH_FILE, ENV_LOG_DIR, Layout, Record, RunConfig, parse_records};

/// Workers in every configuration that does not vary them. Fixed, not the machine's core
/// count, so the workers mode compares one worker against several on any runner.
const BASELINE_WORKERS: NonZeroU32 = NonZeroU32::new(4).unwrap();

/// The seed of the layout permutations. Fixed so a failure reproduces from the mode name.
const LAYOUT_SEED: u64 = 0x5eed_1a70;

/// The seeds of the chaos runs.
const CHAOS_SEEDS: [(&str, u64); 3] = [("seed-1", 1), ("seed-2", 2), ("seed-3", 3)];

/// The first line of a hash file the runner emits.
const HASH_FILE_HEADER: &str = "# lactose-sour-hashes 1";

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

/// The hashes one run recorded, keyed by (test, tick).
pub type Hashes = BTreeMap<(String, u32), u64>;

/// Collects records into [`Hashes`]. A (test, tick) recorded twice with one hash is one
/// entry; recorded with two hashes it is a finding against `path`, since a single run then
/// disagrees with itself.
fn collect(records: Vec<Record>, path: &str, findings: &mut Vec<Finding>) -> Hashes {
    let mut hashes = Hashes::new();
    for record in records {
        let key = (record.test, record.tick);
        match hashes.get(&key) {
            Some(&earlier) if earlier != record.hash => findings.push(Finding::new(
                path,
                format!(
                    "test `{}` recorded both {earlier:016x} and {:016x} at tick {} in one run",
                    key.0, record.hash, key.1
                ),
            )),
            Some(_) => {}
            None => {
                hashes.insert(key, record.hash);
            }
        }
    }
    hashes
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
    let mut ours = hashes.iter().peekable();
    let mut theirs = reference.iter().peekable();
    loop {
        let (key, here, there) = match (ours.peek(), theirs.peek()) {
            (None, None) => break,
            (Some(&(key, &here)), Some(&(other, &there))) if key == other => {
                ours.next();
                theirs.next();
                (key, Some(here), Some(there))
            }
            (Some(&(key, &here)), Some(&(other, _))) if key < other => {
                ours.next();
                (key, Some(here), None)
            }
            (Some(&(key, &here)), None) => {
                ours.next();
                (key, Some(here), None)
            }
            (_, Some(&(key, &there))) => {
                theirs.next();
                (key, None, Some(there))
            }
        };
        let (test, tick) = (key.0.as_str(), key.1);
        if here == there || reported == Some(test) {
            continue;
        }
        reported = Some(test);
        let show = |hash: Option<u64>| {
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
/// Returns the findings and the number of records in the reference file.
///
/// # Errors
/// If either text is not a hash file.
pub fn compare_files(
    reference_path: &str,
    reference: &str,
    path: &str,
    text: &str,
) -> Result<(Vec<Finding>, usize), String> {
    let mut findings = Vec::new();
    let parse = |path: &str, text: &str, findings: &mut Vec<Finding>| {
        parse_records(text)
            .map(|records| collect(records, path, findings))
            .map_err(|e| format!("{path}: {e}"))
    };
    let reference_hashes = parse(reference_path, reference, &mut findings)?;
    let hashes = parse(path, text, &mut findings)?;
    findings.extend(compare(reference_path, &reference_hashes, path, &hashes));
    Ok((findings, reference_hashes.len()))
}

/// The text of a hash file holding `hashes`, sorted, under a header comment.
#[must_use]
pub fn emit(hashes: &Hashes) -> String {
    let mut text = format!("{HASH_FILE_HEADER}\n");
    for ((test, tick), &hash) in hashes {
        let record = Record { test: test.clone(), tick: *tick, hash };
        text.push_str(&format!("{record}\n"));
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
    /// The records of the first reference run, which is the baseline of every mode.
    pub baseline: Option<Hashes>,
}

/// One finished run of a variant.
struct Run {
    config: RunConfig,
    hash_file: String,
    hashes: Hashes,
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
        let reference = &finished[reference];
        if reference.hashes.is_empty() {
            outcome.findings.push(Finding::new(
                &reference.hash_file,
                format!(
                    "{} mode recorded no hashes, so it compared nothing; simulation tests record \
                     through the protocol in tools/xtask/src/sour/protocol.rs",
                    plan.mode.name
                ),
            ));
        }
        for &other in others {
            let other = &finished[other];
            outcome.findings.extend(compare(
                &reference.hash_file,
                &reference.hashes,
                &other.hash_file,
                &other.hashes,
            ));
        }
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
    let mut run =
        Run { config: variant.config, hash_file: hash_file.clone(), hashes: Hashes::new() };
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
            format!(
                "the tests failed under `{}` ({e}); execution logs of failed chaos tests are in {}",
                variant.label,
                dirs.shown(&log_dir)
            ),
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
    match parse_records(&text) {
        Ok(records) => run.hashes = collect(records, &hash_file, &mut outcome.findings),
        Err(e) => outcome.findings.push(Finding::new(hash_file, e.to_string())),
    }
    run
}

/// Launches one named stand-in test with the given variables; see [`self_test`].
pub type StandInLauncher<'a> =
    dyn FnMut(&str, &[(&'static str, String)]) -> Result<(), String> + 'a;

/// Proves the runner on the stand-in simulation of [`stand_in`]: the pull-request modes must
/// pass on the order-independent stand-in, and workers mode must report the flawed stand-in
/// at the tick its flaw starts. Returns the reasons the proof failed; none means it held.
pub fn self_test(dirs: &Dirs, launch: &mut StandInLauncher<'_>) -> Vec<Finding> {
    let matrix = matrix();
    let mut findings = Vec::new();

    let sound = Dirs { root: dirs.root.clone(), out: dirs.out.join("self-test").join("sound") };
    let plans = plan(&matrix, &Selection::PullRequest).unwrap_or_default();
    let outcome = run(&plans, &sound, &mut |vars| launch(stand_in::SOUND_TEST, vars));
    let recorded = outcome.baseline.as_ref().map_or(0, BTreeMap::len);
    if recorded != stand_in::TICKS as usize {
        findings.push(Finding::new(
            dirs.shown(&sound.out),
            format!(
                "the sound stand-in recorded {recorded} hashes under the baseline, expected {}",
                stand_in::TICKS
            ),
        ));
    }
    findings.extend(outcome.findings.into_iter().map(|finding| {
        Finding::new(
            finding.path,
            format!("the runner reported the sound stand-in: {}", finding.message),
        )
    }));

    let flawed = Dirs { root: dirs.root.clone(), out: dirs.out.join("self-test").join("flawed") };
    let workers = Selection::Modes(vec!["workers".to_owned()]);
    let plans = plan(&matrix, &workers).unwrap_or_default();
    let outcome = run(&plans, &flawed, &mut |vars| launch(stand_in::FLAWED_TEST, vars));
    let expected = format!(
        "test `{}` first differs at tick {}: ",
        stand_in::FLAWED_TEST,
        stand_in::FLAW_STARTS_AT
    );
    let detected = matches!(&outcome.findings[..], [only] if only.message.starts_with(&expected));
    if !detected {
        let got: Vec<String> = outcome.findings.iter().map(ToString::to_string).collect();
        findings.push(Finding::new(
            dirs.shown(&flawed.out),
            format!(
                "the runner did not report exactly the flawed stand-in's mismatch at tick {}; \
                 it reported {got:?}",
                stand_in::FLAW_STARTS_AT
            ),
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::protocol::Session;
    use super::stand_in::Flaw;
    use super::*;

    /// A scratch output directory, removed on drop.
    struct Scratch(Dirs);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("lactose-sour-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            Self(Dirs { out: root.join("target").join("sour"), root })
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.root);
        }
    }

    /// Runs the stand-in in this process, as a test process would after reading its
    /// environment.
    fn simulate(test: &str, flaw: Flaw, vars: &[(&'static str, String)]) -> Result<(), String> {
        let lookup = |name: &str| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone());
        let session = Session::from_vars(&lookup).map_err(|e| e.to_string())?;
        stand_in::run(&session, test, flaw).map_err(|e| e.to_string())
    }

    fn hashes(records: &[(&str, u32, u64)]) -> Hashes {
        records.iter().map(|&(test, tick, hash)| ((test.to_owned(), tick), hash)).collect()
    }

    fn lines(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(ToString::to_string).collect()
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

        let chaos = plan(&matrix, &Selection::Modes(vec!["chaos".to_owned()])).unwrap();
        assert_eq!(chaos.len(), 1);
        assert_eq!(chaos[0].variants.len(), 4);
        assert!(plan(&matrix, &Selection::Modes(vec!["speed".to_owned()])).is_err());
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
            "# lactose-sour-hashes 1\na\t0\t0000000000000001\na\t1\t0000000000000002\n"
        );
        assert_eq!(compare_files("linux", &linux, "macos", &linux), Ok((vec![], 2)));

        let macos = emit(&hashes(&[("a", 0, 1), ("a", 1, 3)]));
        let (findings, _) = compare_files("linux", &linux, "macos", &macos).unwrap();
        assert_eq!(
            lines(&findings),
            [
                "macos: test `a` first differs at tick 1: 0000000000000003 here, 0000000000000002 in linux"
            ]
        );

        // A run that disagrees with itself is reported even when the other file agrees.
        let twice = format!("{linux}a\t1\t0000000000000004\n");
        let (findings, _) = compare_files("linux", &linux, "macos", &twice).unwrap();
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
    fn a_sound_simulation_passes_every_mode_and_shared_baselines_run_once() {
        let scratch = Scratch::new("sound");
        let matrix = matrix();
        let plans = plan(&matrix, &Selection::Nightly).unwrap();
        let outcome = run(&plans, &scratch.0, &mut |vars| simulate("sim", Flaw::None, vars));
        assert_eq!(outcome.findings, []);
        // Ten launches: the baseline once, one-worker, the fresh repeat,
        // four layouts and three chaos seeds. Architecture mode reuses the baseline.
        assert_eq!(outcome.launches, 10);
        assert_eq!(outcome.baseline.unwrap().len(), stand_in::TICKS as usize);
        assert!(scratch.0.out.join("repeat/again/hashes.tsv").is_file());
        assert!(scratch.0.out.join("chaos/seed-2/logs").is_dir());
        assert!(!scratch.0.out.join("layout/baseline").exists(), "the baseline was reused");
    }

    #[test]
    fn an_order_dependent_simulation_is_caught_in_every_mode_that_varies_order() {
        let scratch = Scratch::new("flawed");
        let matrix = matrix();
        let plans = plan(&matrix, &Selection::Nightly).unwrap();
        let outcome =
            run(&plans, &scratch.0, &mut |vars| simulate("sim", Flaw::OrderDependent, vars));
        let found = lines(&outcome.findings);
        // One finding per varied configuration; the repeat and architecture runs agree.
        assert_eq!(found.len(), 8, "{found:#?}");
        let tick = stand_in::FLAW_STARTS_AT;
        assert!(found[0].starts_with(&format!(
            "target/sour/workers/one-worker/hashes.tsv: test `sim` first differs at tick {tick}: "
        )));
        assert!(found[0].ends_with(" in target/sour/workers/baseline/hashes.tsv"));
        assert!(found.iter().all(|f| f.contains(&format!("at tick {tick}: "))));
    }

    #[test]
    fn a_failed_launch_an_empty_run_and_a_corrupt_file_are_findings() {
        let scratch = Scratch::new("failures");
        let matrix = matrix();
        let plans = plan(&matrix, &Selection::Modes(vec!["workers".to_owned()])).unwrap();

        let outcome = run(&plans, &scratch.0, &mut |_| Ok(()));
        assert_eq!(
            lines(&outcome.findings),
            ["target/sour/workers/baseline/hashes.tsv: workers mode recorded no hashes, so it \
              compared nothing; simulation tests record through the protocol in \
              tools/xtask/src/sour/protocol.rs"]
        );

        let outcome = run(&plans, &scratch.0, &mut |vars| {
            simulate("sim", Flaw::None, vars)?;
            Err("exit status 100".to_owned())
        });
        assert_eq!(
            lines(&outcome.findings),
            [
                "target/sour/workers/baseline: the tests failed under `baseline` (exit status 100); execution logs of failed chaos tests are in target/sour/workers/baseline/logs",
                "target/sour/workers/one-worker: the tests failed under `one-worker` (exit status 100); execution logs of failed chaos tests are in target/sour/workers/one-worker/logs",
            ]
        );

        let outcome = run(&plans, &scratch.0, &mut |vars| {
            let file = &vars.iter().find(|(n, _)| *n == ENV_HASH_FILE).unwrap().1;
            fs::write(file, "not a record\n").map_err(|e| e.to_string())
        });
        assert!(
            lines(&outcome.findings)[0].starts_with(
                "target/sour/workers/baseline/hashes.tsv: line 1 is not a hash record"
            )
        );
    }

    #[test]
    fn a_rerun_does_not_see_the_previous_runs_records() {
        let scratch = Scratch::new("rerun");
        let matrix = matrix();
        let plans = plan(&matrix, &Selection::Modes(vec!["workers".to_owned()])).unwrap();
        for _ in 0..2 {
            let outcome = run(&plans, &scratch.0, &mut |vars| simulate("sim", Flaw::None, vars));
            assert_eq!(outcome.findings, []);
        }
        let text = fs::read_to_string(scratch.0.out.join("workers/baseline/hashes.tsv")).unwrap();
        assert_eq!(text.lines().count(), stand_in::TICKS as usize);
    }

    #[test]
    fn the_self_test_holds_only_when_the_runner_tells_the_stand_ins_apart() {
        let scratch = Scratch::new("self-test");
        let mut honest = |test: &str, vars: &[(&'static str, String)]| {
            let flaw =
                if test == stand_in::FLAWED_TEST { Flaw::OrderDependent } else { Flaw::None };
            simulate(test, flaw, vars)
        };
        assert_eq!(self_test(&scratch.0, &mut honest), []);

        // A runner that could not see the flaw would be handed two sound simulations.
        let blind = self_test(&scratch.0, &mut |test, vars| simulate(test, Flaw::None, vars));
        assert_eq!(blind.len(), 1);
        assert!(blind[0].message.starts_with("the runner did not report exactly"), "{blind:?}");

        // And a sound stand-in that mismatches must fail the proof too.
        let noisy =
            self_test(&scratch.0, &mut |test, vars| simulate(test, Flaw::OrderDependent, vars));
        assert!(
            noisy.iter().any(|f| f.message.starts_with("the runner reported the sound stand-in")),
            "{noisy:?}"
        );

        // So must tests that never ran.
        let absent = self_test(&scratch.0, &mut |_, _| Ok(()));
        assert!(
            absent[0].message.starts_with("the sound stand-in recorded 0 hashes"),
            "{absent:?}"
        );
    }
}
