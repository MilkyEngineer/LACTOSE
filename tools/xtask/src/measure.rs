//! `cargo xtask measure`: the measurements the milestone criteria rest on, each written to a
//! result file in `docs/results` (architecture section 20; milestone 1 plan, section 4 and
//! decision 5.11).
//!
//! A measurement is an ignored release test that prints its figures, the netcode spike's
//! ten-minute replays, the WHISK benchmark, the headless binary's 120-tick run, or the
//! edit-to-playing loop. A test prints each figure
//! as a line `measure: <name> <value>` on its standard output, which nextest passes through
//! under `--no-capture`: the test needs no path from this command and stays runnable by
//! hand, and the line is the one contract between them; for the spike's replays the contract
//! is instead the one JSON object each prints. A test that prints no figure, a figure line
//! that does not read, or a figure named twice fails the measurement. Every result file opens
//! with the
//! header the plan names: the CPU and operating system, the topology the executor detected,
//! the workers used, the commit, the build profile, the command, and the load average before
//! and after the run (on Windows, the processor's idle share over it, sampled through WMI).
//!
//! Not a check of any criterion (`spike-net`'s two counts aside): nothing is printed to
//! standard output, progress and the measurements' own output go to standard error, and times
//! are printed and written, never compared with a threshold.

use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The variable a test reads to cut its repetitions in a short run.
pub const SHORT_VAR: &str = "LACTOSE_MEASURE_SHORT";
/// The variable that names the machine in the file names, in place of the CPU's slug.
pub const MACHINE_VAR: &str = "LACTOSE_MEASURE_MACHINE";
/// The variable the cost tests read for N, the frame workers of plan decision 5.1, in place
/// of the topology's: a count above zero, refused before anything builds otherwise, passed
/// to the tests and recorded in the header; empty, it is unset, as the tests read it.
pub const WORKERS_VAR: &str = "LACTOSE_MEASURE_WORKERS";
/// The prefix of a figure line in a measurement's standard output.
pub const FIGURE_PREFIX: &str = "measure: ";
/// The edit-to-playing figure is the median of this many edits at each site (decision 5.9).
const EDITS_PER_SITE: u32 = 5;
/// The default output directory, relative to the repository root.
pub const RESULTS: &str = "docs/results";
/// The edit-to-playing copy's directory, relative to the target directory: the worktree in
/// `tree`, its target directory in `target`.
const COPY_DIR: &str = "measure/edit-to-playing";

/// How a measurement runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    /// An ignored test, run in release.
    Test {
        /// The package.
        package: &'static str,
        /// Its integration test file, relative to the repository root: a module of the
        /// package's one test target, `integration`, named after the file (D150).
        file: &'static str,
        /// The test's path inside the file's module.
        test: &'static str,
    },
    /// The netcode spike's ten-minute replays (`REPLAYS`), ignored tests run in release one
    /// at a time, each of which prints its figures as one JSON object.
    SpikeReplays,
    /// `cargo bench -p lactose-task --bench whisk`, which writes its own pair of files.
    TaskBench,
    /// `cargo run -p lactose-app-headless -- 120`, dev profile.
    Headless,
    /// The edit-to-playing loop of decision 5.9.
    EditToPlaying,
}

impl Run {
    /// Whether `--short` cuts its repetitions, or the spike's replays, and so marks its files
    /// as short. The headless run has none to cut, and edit-to-playing keeps its five so that
    /// a runner's short run still yields the criterion's figure.
    #[must_use]
    pub const fn cut_by_short(self) -> bool {
        matches!(self, Self::Test { .. } | Self::SpikeReplays | Self::TaskBench)
    }
}

/// One measurement: its name, the file stem of its result files.
#[derive(Clone, Copy, Debug)]
pub struct Measurement {
    /// The name `cargo xtask measure` takes and the stem its result files start with.
    pub name: &'static str,
    /// What it measures, one sentence for the file's opening line.
    pub what: &'static str,
    /// Where its threshold is stated, or that it has none.
    pub threshold: &'static str,
    /// How it runs.
    pub run: Run,
}

const fn test(package: &'static str, file: &'static str, test: &'static str) -> Run {
    Run::Test { package, file, test }
}

/// Every measurement, in the order `--all` runs them.
pub const MEASUREMENTS: [Measurement; 12] = [
    Measurement {
        name: "tick-100k",
        what: "Criterion 1: the median step of the 100k scene on one frame worker and on N, S and the projected eight-core tick",
        threshold: "architecture section 22; milestone 1 plan, section 4 and decision 5.1",
        run: test("lactose-sim", "crates/sim/tests/cost.rs", "cost_of_a_tick_at_100k_rows"),
    },
    Measurement {
        name: "physics-phase",
        what: "Criterion 4: the sync, step and write-back of 100k rows with 2,000 awake bodies on Rapier",
        threshold: "architecture section 22; milestone 1 plan, section 4",
        run: test(
            "lactose-app",
            "crates/app/tests/cost.rs",
            "cost_of_the_physics_phase_at_100k_rows",
        ),
    },
    Measurement {
        name: "composed-tick",
        what: "The 100k scene with 2,000 bodies on Rapier beside it, on one frame worker and on N",
        threshold: "none: recorded beside criteria 1 and 4 (milestone 1 plan, section 4)",
        run: test("lactose-app", "crates/app/tests/cost.rs", "composed::cost_of_the_composed_tick"),
    },
    Measurement {
        name: "reload-100k",
        what: "The full diff of a 100k-row region reloaded in place on Rapier (D124)",
        threshold: "none: recorded (docs/verification.md)",
        run: test("lactose-app", "crates/app/tests/cost.rs", "cost_of_a_100k_reload"),
    },
    Measurement {
        name: "bone-bodies",
        what: "200 characters with a Rapier body on each of their sixteen bones, against none",
        threshold: "none: recorded (docs/verification.md)",
        run: test("lactose-app", "crates/app/tests/cost.rs", "cost_of_bone_bodies_on_rapier"),
    },
    Measurement {
        name: "prediction-slot",
        what: "Criterion 5: the take and the restore of a prediction slot on the spike's thrown scene with Rapier",
        threshold: "architecture section 22; milestone 1 plan, section 4",
        run: test("lactose-app", "crates/app/tests/spike.rs", "cost_of_a_prediction_slot"),
    },
    Measurement {
        name: "rollback-seven-ticks",
        what: "Criterion 7: a seven-tick rollback on the spike's thrown scene with Rapier",
        threshold: "none: recorded (architecture section 22; milestone 1 plan, section 4)",
        run: test("lactose-app", "crates/app/tests/spike.rs", "cost_of_a_seven_tick_rollback"),
    },
    Measurement {
        name: "spike-net",
        what: "Criterion 7: the netcode spike's four scenes under the clock script and on the server's clock: the correction rate, the largest absorbed difference per kind, rollback cost per tick, and divergence",
        threshold: "none: recorded (architecture section 22; milestone 1 plan, section 4)",
        run: Run::SpikeReplays,
    },
    Measurement {
        name: "sim-skeleton",
        what: "Criterion 6: the sim skeleton of 200 characters; figure (b) is the criterion's",
        threshold: "architecture section 22; milestone 1 plan, section 4",
        run: test("lactose-anim", "crates/anim/tests/cost.rs", "the_sim_skeleton_budget"),
    },
    Measurement {
        name: "whisk",
        what: "Milestone 0's WHISK benchmark, which writes its own pair of files",
        threshold: "crates/task/README.md",
        run: Run::TaskBench,
    },
    Measurement {
        name: "headless-120",
        what: "The headless binary's 120-tick run and the world hash it prints",
        threshold: "none: its hash on x86-64 equals the M1's at the same commit (milestone 1 plan, section 4)",
        run: Run::Headless,
    },
    Measurement {
        name: "edit-to-playing",
        what: "Criterion 9: from saving a one-line change to the headless binary's report line, both edits of decision 5.9",
        threshold: "architecture section 22; milestone 1 plan, section 4 and decision 5.9",
        run: Run::EditToPlaying,
    },
];

/// The measurement named `name`.
#[must_use]
pub fn find(name: &str) -> Option<&'static Measurement> {
    MEASUREMENTS.iter().find(|m| m.name == name)
}

/// The netcode spike's test file, whose replays [`Run::SpikeReplays`] runs.
const SPIKE_FILE: &str = "crates/app/tests/spike.rs";

/// One of the netcode spike's ten-minute replays: an ignored test that replays one scene on
/// one clock and prints its report.
#[derive(Clone, Copy, Debug)]
struct Replay {
    /// The test's name in [`SPIKE_FILE`].
    test: &'static str,
    /// Whether a short run keeps it: the dynamic capsule on each clock, the cheapest pair
    /// (about 28 s of the eight's 223 s on the M1), since a replay is ten minutes of ticks
    /// however it is run and a runner's short run must fit its job beside the rest.
    short: bool,
}

impl Replay {
    const fn run(self) -> Run {
        test("lactose-app", SPIKE_FILE, self.test)
    }
}

/// The replays `spike-net` runs, in the file's order: every `ten_minutes_of_` test but the
/// claim on approach, which the file does not record.
const REPLAYS: [Replay; 8] = [
    Replay { test: "ten_minutes_of_the_dynamic_capsule_with_the_clock_script", short: true },
    Replay { test: "ten_minutes_of_the_dynamic_capsule_on_the_server_clock", short: true },
    Replay { test: "ten_minutes_of_the_kinematic_character_with_the_clock_script", short: false },
    Replay { test: "ten_minutes_of_the_kinematic_character_on_the_server_clock", short: false },
    Replay { test: "ten_minutes_of_fifty_thrown_bodies_with_the_clock_script", short: false },
    Replay { test: "ten_minutes_of_fifty_thrown_bodies_on_the_server_clock", short: false },
    Replay { test: "ten_minutes_of_the_claim_in_contact_with_the_clock_script", short: false },
    Replay { test: "ten_minutes_of_the_claim_in_contact_on_the_server_clock", short: false },
];

/// The frame workers each replay's executor runs, `executor(4)` in the spike's `measure`.
const SPIKE_WORKERS: u32 = 4;

/// The most fixed colliders and proxies together a prediction world may hold at milestone 1
/// (plan decision 5.5), against each replay's `prediction_fixed_and_proxies`.
const PREDICTION_BOUND: u64 = 64;

/// The topology probe: an ignored test of `lactose-app` that prints what the executor
/// detects, run once before the measurements.
const PROBE: Run =
    test("lactose-app", "crates/app/tests/cost.rs", "the_topology_the_executor_detects");

// ---- The pure parts ----

/// The figures in `output`: every line `measure: <name> <value>`, in order. A line that
/// starts with the prefix and lacks a name without spaces or a value is an error, not a
/// figure dropped, and so is a name printed twice, which is two runs' output or a test's
/// slip, not one figure.
fn figures(output: &str) -> Result<Vec<(String, String)>, String> {
    let mut seen = std::collections::BTreeSet::new();
    output
        .lines()
        .filter_map(|line| line.strip_prefix(FIGURE_PREFIX).map(str::trim_end))
        .map(|rest| match rest.split_once(' ') {
            Some((name, value)) if !name.is_empty() && !value.trim().is_empty() => {
                if !seen.insert(name) {
                    return Err(format!("the figure `{name}` is printed twice"));
                }
                Ok((name.to_owned(), value.trim().to_owned()))
            }
            _ => Err(format!(
                "a figure line does not read as `{FIGURE_PREFIX}<name> <value>`: `{rest}`"
            )),
        })
        .collect()
}

/// A figure's value in the JSON file: a number where it reads as one; a JSON array, `true`,
/// `false` or `null` where it reads as one, as a replay's report holds them; else its text.
#[must_use]
fn json_value(value: &str) -> serde_json::Value {
    use serde_json::Value;
    if let Ok(whole) = value.parse::<i64>() {
        return whole.into();
    }
    match value.parse::<f64>() {
        Ok(number) if number.is_finite() => number.into(),
        _ => match serde_json::from_str(value) {
            Ok(parsed @ (Value::Array(_) | Value::Bool(_) | Value::Null)) => parsed,
            _ => value.into(),
        },
    }
}

/// The figures of one replay of the netcode spike: the one line of `output` that is a JSON
/// object, its report, with each value under `<scene>.<clock>.` and nested keys joined by
/// dots, an array as its JSON text. The scene is the report's `scene` with underscores for
/// its spaces; the clock is `clock_script` or `server_clock`, as its `clock_script` says.
/// Returns that prefix and the figures. A line that opens with `{` and is not a report is an
/// error, as are no report and two.
fn replay_figures(output: &str) -> Result<(String, Vec<(String, String)>), String> {
    use serde_json::Value;
    let mut reports = Vec::new();
    for line in output.lines().map(str::trim).filter(|line| line.starts_with('{')) {
        let report = match serde_json::from_str(line) {
            Ok(Value::Object(report)) => report,
            _ => return Err(format!("a line that opens with `{{` is not a JSON object: `{line}`")),
        };
        let scene = report.get("scene").and_then(Value::as_str).filter(|s| !s.is_empty());
        let clock = report.get("clock_script").and_then(Value::as_bool);
        let (Some(scene), Some(clock)) = (scene, clock) else {
            return Err(format!("a report names no `scene` and `clock_script`: `{line}`"));
        };
        let clock = if clock { "clock_script" } else { "server_clock" };
        let prefix = format!("{}.{clock}", scene.replace(' ', "_"));
        let mut found = Vec::new();
        for (key, value) in &report {
            if key != "scene" && key != "clock_script" {
                flatten(&format!("{prefix}.{key}"), value, &mut found);
            }
        }
        reports.push((prefix, found));
    }
    match <[_; 1]>::try_from(reports) {
        Ok([report]) => Ok(report),
        Err(reports) => Err(format!("{} lines are a replay's report; one must be", reports.len())),
    }
}

/// What a replay's figures must show for its file to stand: no server frame outside a
/// `Simulated` region, and the prediction world within [`PREDICTION_BOUND`]. Returns the
/// replay's count of fixed colliders and proxies.
fn check_replay(prefix: &str, figures: &[(String, String)]) -> Result<u64, String> {
    let count = |key: &str| {
        let name = format!("{prefix}.{key}");
        figures
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, value)| value.parse::<u64>().ok())
            .ok_or_else(|| format!("the report has no count `{key}`"))
    };
    let frozen = count("server_frozen_ticks")?;
    if frozen != 0 {
        return Err(format!(
            "`{prefix}` ran {frozen} server frames with its region not `Simulated` \
             (`server_frozen_ticks`), so its corrections are not a simulated region's \
             (architecture section 9.4)"
        ));
    }
    let held = count("prediction_fixed_and_proxies")?;
    if held > PREDICTION_BOUND {
        return Err(format!(
            "`{prefix}`'s prediction world held {held} fixed colliders and proxies, over the \
             {PREDICTION_BOUND} of plan decision 5.5 (`prediction_fixed_and_proxies`)"
        ));
    }
    Ok(held)
}

/// `value`'s figures under `name`: an object's members under `name.<key>`, anything else as
/// its text, a string unquoted.
fn flatten(name: &str, value: &serde_json::Value, figures: &mut Vec<(String, String)>) {
    match value {
        serde_json::Value::Object(members) => {
            for (key, member) in members {
                flatten(&format!("{name}.{key}"), member, figures);
            }
        }
        serde_json::Value::String(text) => figures.push((name.to_owned(), text.clone())),
        other => figures.push((name.to_owned(), other.to_string())),
    }
}

/// The machine's name in a file name: the lower-case alphanumeric runs of `text` joined by
/// single hyphens, as the WHISK benchmark derives it.
#[must_use]
fn slug(text: &str) -> String {
    let mut slug = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// The UTC date of `seconds` after the epoch as `YYYY-MM-DD` (days-to-civil, Howard
/// Hinnant's algorithm), as the WHISK benchmark computes it.
#[must_use]
fn date(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The stem of a result file: `<measurement>-<machine>-<date>`, then `-short` for a short run.
#[must_use]
fn file_stem(measurement: &str, machine: &str, date: &str, short: bool) -> String {
    let suffix = if short { "-short" } else { "" };
    format!("{measurement}-{machine}-{date}{suffix}")
}

/// The load averages in `uptime`'s output: what follows "load average" or "load averages".
#[must_use]
fn load_average(uptime: &str) -> Option<String> {
    let averages = uptime.rsplit_once("load average")?.1;
    let averages = averages.trim_start_matches(['s', ':', ' ']).trim();
    (!averages.is_empty()).then(|| averages.to_owned())
}

/// [`WORKERS_VAR`]'s value: none when unset or empty, else a count above zero.
fn workers_override(value: Option<&std::ffi::OsStr>) -> Result<Option<u32>, String> {
    let Some(value) = value.filter(|value| !value.is_empty()) else { return Ok(None) };
    value
        .to_str()
        .and_then(|text| text.parse::<u32>().ok())
        .filter(|&n| n > 0)
        .map(Some)
        .ok_or_else(|| {
            format!("{WORKERS_VAR} is `{}`, not a count above zero", value.to_string_lossy())
        })
}

/// The mean of the samples the WMI sampler printed over a run of `seconds`, a percentage:
/// every line that reads as a number but the first, which a fresh session's first query may
/// answer with no interval behind it. Anything else it printed is not a sample.
///
/// # Errors
/// Why there is no share, for the header: the sampler printed no sample, or only the first.
/// A run shorter than Windows PowerShell's start and two samples a second apart has none.
fn idle_share(samples: &str, seconds: f64) -> Result<f64, String> {
    let samples: Vec<f64> = samples
        .lines()
        .filter_map(|line| line.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .collect();
    let printed = match samples.as_slice() {
        [] => "no sample",
        [_] => "only its first sample, which is dropped,",
        [_, rest @ ..] => return Ok(rest.iter().sum::<f64>() / rest.len() as f64),
    };
    Err(format!("the run took {seconds:.1} s, and the sampler printed {printed} in it"))
}

/// The data of the value `name` in `reg query`'s output: what follows its type on the line
/// that names it.
#[must_use]
fn registry_value(output: &str, name: &str) -> Option<String> {
    let line = output.lines().map(str::trim).find(|line| line.starts_with(name))?;
    let (_, data) = line.split_once("REG_SZ")?;
    let data = data.trim();
    (!data.is_empty()).then(|| data.to_owned())
}

/// What the headless binary's report line says.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HeadlessReport {
    ticks: u64,
    frames: u64,
    behind: u64,
    /// As printed, `0x` and sixteen hexadecimal digits.
    hash: String,
}

/// The report line `<ticks> ticks in <frames> frames (<behind> behind), hash <hash>`.
#[must_use]
fn headless_report(line: &str) -> Option<HeadlessReport> {
    let (ticks, rest) = line.trim().split_once(" ticks in ")?;
    let (frames, rest) = rest.split_once(" frames (")?;
    let (behind, hash) = rest.split_once(" behind), hash ")?;
    let hash = hash.trim();
    let hex = hash.strip_prefix("0x")?;
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(HeadlessReport {
        ticks: ticks.parse().ok()?,
        frames: frames.parse().ok()?,
        behind: behind.parse().ok()?,
        hash: hash.to_owned(),
    })
}

/// One site of the edit-to-playing loop: a line in a function body, which each edit follows
/// with one statement of its own.
#[derive(Clone, Copy, Debug)]
struct Site {
    /// The figure's prefix.
    name: &'static str,
    /// The file, relative to the repository root.
    file: &'static str,
    /// The line the statement follows, whole and without its line ending: exactly one line
    /// of the file.
    anchor: &'static str,
}

/// The two edits of decision 5.9: the sketch game's system, in `game-sim` since it left the
/// headless binary (D152), and the executor's start in `lactose-app`; both are libraries
/// the binary depends on.
const SITES: [Site; 2] = [
    Site {
        name: "sketch_system",
        file: "game/game-sim/src/sketch.rs",
        anchor: "    counter.0 = counter.0.wrapping_add(1);",
    },
    Site {
        name: "app_function",
        file: "crates/app/src/runtime.rs",
        anchor: "    let mut builder = Executor::builder();",
    },
];

/// `source` with the `n`th edit at `site`: one statement on a line of its own after the
/// anchor's, which differs from every other edit's, so each is a one-line change from the
/// build before it. The new line ends as the anchor's does, so that a file checked out with
/// CRLF line endings (git's `core.autocrlf` on Windows) keeps one line ending throughout.
/// Fails unless exactly one line is the anchor, and that line has an ending.
fn edited(source: &str, site: &Site, n: u32) -> Result<String, String> {
    let mut found = Vec::new();
    let mut end = 0;
    for line in source.split_inclusive('\n') {
        end += line.len();
        let text = line.strip_suffix('\n').map_or(line, |l| l.strip_suffix('\r').unwrap_or(l));
        if text == site.anchor {
            found.push((end, &line[text.len()..]));
        }
    }
    let [(end, ending)] = found[..] else {
        return Err(format!(
            "{}: {} lines are the edit's anchor `{}`; one must be",
            site.file,
            found.len(),
            site.anchor.trim()
        ));
    };
    if ending.is_empty() {
        return Err(format!("{}: the edit's anchor is the last line, with no ending", site.file));
    }
    let statement = format!("    let _ = std::hint::black_box({n}_u32);{ending}");
    Ok(format!("{}{statement}{}", &source[..end], &source[end..]))
}

/// The median of `values`.
#[must_use]
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(sorted.len() / 2).copied().unwrap_or(f64::NAN)
}

/// What the machine was doing around a run.
#[derive(Clone, Debug, PartialEq)]
enum Load {
    /// The load averages before and after, as `uptime` prints them.
    Averages { before: String, after: String },
    /// The processor's mean idle share over the run, in percent, or why it has none.
    Idle(Result<f64, String>),
}

/// The header of a result file.
#[derive(Clone, Debug)]
struct Header {
    measurement: &'static Measurement,
    machine: String,
    date: String,
    short: bool,
    cpu: String,
    os: String,
    arch: &'static str,
    /// Performance and efficiency cores, as the executor detected them.
    topology: Option<(u64, u64)>,
    workers: String,
    commit: String,
    profile: &'static str,
    /// The command as given, and what it ran.
    command: String,
    ran: String,
    load: Load,
    /// Rows beside the standard ones.
    notes: Vec<(String, String)>,
}

fn load_text(load: &Load) -> (&'static str, String) {
    match load {
        Load::Averages { before, after } => {
            ("Load average", format!("{before} before, {after} after"))
        }
        Load::Idle(Ok(share)) => ("Processor idle", format!("{share:.1}% over the run")),
        Load::Idle(Err(why)) => ("Processor idle", format!("unavailable: {why}")),
    }
}

/// The Markdown result file: the header, the figures and the measurement's own output.
#[must_use]
fn markdown(header: &Header, figures: &[(String, String)], output: &str) -> String {
    let m = header.measurement;
    let mut text = String::new();
    let short = if header.short { ", short run" } else { "" };
    let _ = writeln!(text, "# {}, {}, {}{short}\n", m.name, header.machine, header.date);
    let _ = writeln!(text, "{}. Threshold: {}.\n", m.what, m.threshold);
    if header.short {
        let _ = writeln!(
            text,
            "A short run (`--short`): its repetitions were cut so that the command is known to \
             finish; its numbers mean nothing.\n"
        );
    }
    let topology = header.topology.map_or_else(
        || "unknown".to_owned(),
        |(p, e)| format!("{p} performance + {e} efficiency cores"),
    );
    let (load_name, load) = load_text(&header.load);
    let _ = writeln!(text, "| | |\n|---|---|");
    let _ = writeln!(text, "| CPU | {} ({}, {}) |", header.cpu, header.os, header.arch);
    let _ = writeln!(text, "| Topology | {topology}, as the executor detected them |");
    let _ = writeln!(text, "| Workers | {} |", header.workers);
    let _ = writeln!(text, "| Commit | `{}` |", header.commit);
    let _ = writeln!(text, "| Profile | {} |", header.profile);
    let _ = writeln!(text, "| Command | `{}`, which ran `{}` |", header.command, header.ran);
    let _ = writeln!(text, "| {load_name} | {load} |");
    for (name, value) in &header.notes {
        let _ = writeln!(text, "| {name} | {value} |");
    }
    if !figures.is_empty() {
        let _ = writeln!(text, "\n## Figures\n\n| Figure | Value |\n|---|---|");
        for (name, value) in figures {
            let _ = writeln!(text, "| `{name}` | {value} |");
        }
    }
    if !output.trim().is_empty() {
        let _ = writeln!(text, "\n## Output\n\n```text\n{}\n```", output.trim_end());
    }
    text
}

/// The JSON result file: the header's fields and the figures.
#[must_use]
fn json(header: &Header, figures: &[(String, String)]) -> String {
    let mut load = serde_json::Map::new();
    match &header.load {
        Load::Averages { before, after } => {
            load.insert("load_average_before".into(), before.as_str().into());
            load.insert("load_average_after".into(), after.as_str().into());
        }
        Load::Idle(share) => {
            load.insert("processor_idle_percent".into(), share.as_ref().ok().copied().into());
        }
    }
    let figures: serde_json::Map<String, serde_json::Value> =
        figures.iter().map(|(name, value)| (name.clone(), json_value(value))).collect();
    let notes: serde_json::Map<String, serde_json::Value> =
        header.notes.iter().map(|(name, value)| (name.clone(), value.as_str().into())).collect();
    let value = serde_json::json!({
        "measurement": header.measurement.name,
        "machine": header.machine,
        "date": header.date,
        "short": header.short,
        "cpu": header.cpu,
        "os": header.os,
        "arch": header.arch,
        "topology": header.topology.map(|(p, e)| serde_json::json!({
            "performance": p,
            "efficiency": e,
        })),
        "workers": header.workers,
        "commit": header.commit,
        "profile": header.profile,
        "command": header.command,
        "ran": header.ran,
        "load": load,
        "notes": notes,
        "figures": figures,
    });
    let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
    text.push('\n');
    text
}

/// `program` and `args` as one shell line, each argument quoted where it needs it.
#[must_use]
fn shell_line(env: &[(&str, String)], program: &str, args: &[String]) -> String {
    let quote = |arg: &str| {
        let plain = arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_=./:,".contains(c));
        if plain && !arg.is_empty() { arg.to_owned() } else { format!("'{arg}'") }
    };
    let mut words: Vec<String> =
        env.iter().map(|(name, value)| format!("{name}={value}")).collect();
    words.push(program.to_owned());
    words.extend(args.iter().map(|arg| quote(arg)));
    words.join(" ")
}

// ---- Running ----

/// What `cargo xtask measure` was asked to do.
#[derive(Clone, Debug)]
pub struct Options {
    selected: Vec<&'static Measurement>,
    short: bool,
    /// The directory the result files are written to, relative to the repository root once
    /// [`resolved`](Self::resolved).
    out: PathBuf,
    /// The arguments as given, for the header's command.
    args: Vec<String>,
}

impl Options {
    /// These options with `--out` relative to `root`: an absolute directory inside the
    /// repository is rewritten relative to it, in the command too, so that no result file
    /// names a path of this machine; one outside it, or a relative one that climbs out of it,
    /// is refused.
    fn resolved(&self, root: &Path) -> Result<Self, String> {
        let outside = || {
            format!(
                "`--out {}` is outside the repository; give a directory inside it",
                self.out.display()
            )
        };
        let out = if self.out.is_absolute() {
            self.out.strip_prefix(root).map_err(|_| outside())?.to_owned()
        } else {
            self.out.clone()
        };
        if out.components().any(|part| !matches!(part, Component::Normal(_) | Component::CurDir)) {
            return Err(outside());
        }
        let out = if out.as_os_str().is_empty() { PathBuf::from(".") } else { out };
        let mut args = self.args.clone();
        if let Some(at) = args.iter().position(|arg| arg == "--out")
            && let Some(value) = args.get_mut(at + 1)
        {
            *value = out.to_string_lossy().replace('\\', "/");
        }
        Ok(Self { out, args, ..self.clone() })
    }

    /// The command as given, `--out` as resolved.
    fn command(&self) -> String {
        let mut command = String::from("cargo xtask measure");
        for arg in &self.args {
            command.push(' ');
            command.push_str(arg);
        }
        command
    }
}

/// Parses the arguments after `measure`: `--all` or names, `--short`, `--out <dir>`.
pub fn parse(args: &[&str]) -> Result<Options, String> {
    let names = || MEASUREMENTS.iter().map(|m| m.name).collect::<Vec<_>>().join(", ");
    let (mut selected, mut all, mut short, mut out) = (Vec::new(), false, false, None);
    let mut rest = args;
    while let [first, tail @ ..] = rest {
        rest = tail;
        match *first {
            "--all" => all = true,
            "--short" => short = true,
            "--out" => {
                let [dir, tail @ ..] = rest else { return Err("`--out` takes a directory".into()) };
                if out.replace(PathBuf::from(dir)).is_some() {
                    return Err("`--out` is given once".into());
                }
                rest = tail;
            }
            name => {
                let m = find(name).ok_or_else(|| {
                    format!("no measurement is named `{name}`; the names are {}", names())
                })?;
                if !selected.iter().any(|s: &&Measurement| s.name == m.name) {
                    selected.push(m);
                }
            }
        }
    }
    if all == !selected.is_empty() {
        return Err(format!("give `--all` or one or more of {}", names()));
    }
    if all {
        selected = MEASUREMENTS.iter().collect();
    }
    Ok(Options {
        selected,
        short,
        out: out.unwrap_or_else(|| PathBuf::from(RESULTS)),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
    })
}

/// The workspace the measurements run in.
#[derive(Clone, Debug)]
pub struct Workspace {
    /// The cargo binary.
    pub cargo: String,
    /// The repository root.
    pub root: PathBuf,
    /// The workspace's target directory.
    pub target_dir: PathBuf,
}

/// What every file of one run shares.
struct Machine {
    name: String,
    cpu: String,
    os: String,
    date: String,
    /// `HEAD`, abbreviated.
    head: String,
    /// `head`, with `-dirty` when a tracked file has changes.
    commit: String,
    topology: Option<(u64, u64)>,
    default_workers: Option<String>,
    /// [`WORKERS_VAR`], where it is set and not empty.
    workers_override: Option<u32>,
}

/// Runs the selected measurements, each writing its files; one that fails is reported and
/// the rest still run. Returns the files written, or what failed.
pub fn run(workspace: &Workspace, options: &Options) -> Result<Vec<PathBuf>, String> {
    let options = &options.resolved(&workspace.root)?;
    let workers_override = workers_override(std::env::var_os(WORKERS_VAR).as_deref())?;
    let out = workspace.root.join(&options.out);
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let cpu = cpu_name();
    let (head, dirty) = commit(&workspace.root);
    let mut machine = Machine {
        name: std::env::var(MACHINE_VAR)
            .ok()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| slug(&machine_source(&cpu))),
        os: os_name(),
        cpu,
        date: date(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())),
        commit: if dirty { format!("{head}-dirty") } else { head.clone() },
        head,
        topology: None,
        default_workers: None,
        workers_override,
    };
    eprintln!(
        "measure: {} ({}), machine `{}`, commit {}, {}",
        machine.cpu,
        machine.os,
        machine.name,
        machine.commit,
        if options.short { "short run" } else { "full run" }
    );
    match probe(workspace, &machine) {
        Ok(found) => {
            let get = |name: &str| found.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
            let core = |name: &str| get(name).and_then(|v| v.parse::<u64>().ok());
            machine.topology = core("topology.performance").zip(core("topology.efficiency"));
            machine.default_workers = get("frame_workers");
        }
        Err(error) => eprintln!("measure: the topology probe failed, so it is unknown: {error}"),
    }
    let (mut written, mut failed) = (Vec::new(), Vec::new());
    for m in &options.selected {
        eprintln!("measure: {} ...", m.name);
        let result = match m.run {
            Run::Test { .. } => run_test(workspace, options, &machine, m, &out),
            Run::SpikeReplays => run_spike_replays(workspace, options, &machine, m, &out),
            Run::TaskBench => run_task_bench(workspace, options, &machine, &out),
            Run::Headless => run_headless(workspace, options, &machine, m, &out),
            Run::EditToPlaying => run_edit_to_playing(workspace, options, &machine, m, &out),
        };
        match result {
            Ok(files) => {
                for file in &files {
                    eprintln!("measure: wrote {}", file.display());
                }
                written.extend(files);
            }
            Err(error) => {
                eprintln!("measure: {} failed: {error}", m.name);
                failed.push(m.name);
            }
        }
    }
    if failed.is_empty() {
        Ok(written)
    } else {
        Err(format!("{} failed: {}", failed.len(), failed.join(", ")))
    }
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).stderr(Stdio::null()).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The CPU's model name: as the WHISK benchmark reads it on macOS and Linux, and on Windows
/// the registry's `ProcessorNameString`, `PROCESSOR_IDENTIFIER` where that cannot be read.
fn cpu_name() -> String {
    let name = if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(target_os = "linux") {
        fs::read_to_string("/proc/cpuinfo").ok().and_then(|info| {
            let line = info.lines().find(|line| line.starts_with("model name"))?;
            Some(line.split_once(':')?.1.trim().to_owned())
        })
    } else if cfg!(windows) {
        let key = r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0";
        command_output("reg", &["query", key, "/v", "ProcessorNameString"])
            .and_then(|output| registry_value(&output, "ProcessorNameString"))
            .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok())
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    };
    name.filter(|name| !name.is_empty()).unwrap_or_else(|| "unknown CPU".to_owned())
}

/// What the machine's slug is derived from, by the WHISK benchmark's rule, so that its files
/// and these share the name: the CPU's model name, but `PROCESSOR_IDENTIFIER` on Windows.
fn machine_source(cpu: &str) -> String {
    if cfg!(windows) {
        std::env::var("PROCESSOR_IDENTIFIER")
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "unknown CPU".to_owned())
    } else {
        cpu.to_owned()
    }
}

/// The operating system and its version, where it says.
fn os_name() -> String {
    let os = std::env::consts::OS;
    let version = if cfg!(target_os = "macos") {
        command_output("sw_vers", &["-productVersion"])
    } else if cfg!(target_os = "linux") {
        fs::read_to_string("/etc/os-release").ok().and_then(|release| {
            let line = release.lines().find(|line| line.starts_with("PRETTY_NAME="))?;
            Some(line.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_owned())
        })
    } else if cfg!(windows) {
        command_output("cmd", &["/C", "ver"])
    } else {
        None
    };
    match version.filter(|v| !v.is_empty()) {
        Some(version) => format!("{os} {version}"),
        None => os.to_owned(),
    }
}

/// `HEAD`, abbreviated, and whether a tracked file has changes.
fn commit(root: &Path) -> (String, bool) {
    let root = root.to_string_lossy();
    let hash = command_output("git", &["-C", &root, "rev-parse", "--short=12", "HEAD"]);
    let dirty =
        command_output("git", &["-C", &root, "status", "--porcelain", "--untracked-files=no"])
            .is_some_and(|status| !status.is_empty());
    (hash.unwrap_or_else(|| "unknown".to_owned()), dirty)
}

/// The Windows sampler, run by the Windows PowerShell the platform ships: the processor's
/// idle share every second, one number per line, from WMI's formatted performance class,
/// whose names are not localized as `typeperf`'s counter paths are. No double quote, so
/// that the argument needs no escaping on the command line.
const IDLE_SAMPLER: &str = "while ($true) { \
    $total = Get-CimInstance -ClassName Win32_PerfFormattedData_PerfOS_Processor \
    -Filter 'Name=''_Total'''; \
    [Console]::Out.WriteLine($total.PercentIdleTime); [Console]::Out.Flush(); \
    Start-Sleep -Seconds 1 }";

/// What the machine is doing, sampled across one run. A sampler dropped unstopped, on an
/// early return, stops its process all the same.
enum Sampler {
    Uptime(String),
    /// [`IDLE_SAMPLER`], the thread reading it, and when it was started, until stopped.
    Wmi(Option<(Child, thread::JoinHandle<String>, Instant)>),
    /// Why the sampler could not be started.
    Unavailable(&'static str),
}

impl Sampler {
    fn start() -> Self {
        if cfg!(windows) {
            let spawned = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", IDLE_SAMPLER])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            match spawned {
                Ok(mut child) => match child.stdout.take() {
                    Some(mut stdout) => {
                        let reader = thread::spawn(move || {
                            let mut text = String::new();
                            let _ = stdout.read_to_string(&mut text);
                            text
                        });
                        Self::Wmi(Some((child, reader, Instant::now())))
                    }
                    None => Self::Unavailable("the sampler's output could not be read"),
                },
                Err(_) => Self::Unavailable("Windows PowerShell could not be started"),
            }
        } else {
            Self::Uptime(uptime())
        }
    }

    fn stop(mut self) -> Load {
        match &mut self {
            Self::Uptime(before) => {
                Load::Averages { before: std::mem::take(before), after: uptime() }
            }
            Self::Wmi(running) => Load::Idle(match running.take() {
                Some((mut child, reader, started)) => {
                    let seconds = started.elapsed().as_secs_f64();
                    let _ = child.kill();
                    let _ = child.wait();
                    match reader.join() {
                        Ok(samples) => idle_share(&samples, seconds),
                        Err(_) => Err("the sampler's output could not be read".to_owned()),
                    }
                }
                None => Err("the sampler was stopped twice".to_owned()),
            }),
            Self::Unavailable(why) => Load::Idle(Err((*why).to_owned())),
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        if let Self::Wmi(running) = self
            && let Some((mut child, _, _)) = running.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn uptime() -> String {
    command_output("uptime", &[])
        .as_deref()
        .and_then(load_average)
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Runs `command` with its standard output read line by line, each line echoed to standard
/// error as it comes and handed to `line` with the time since the start. Returns the output,
/// or why the command failed.
fn capture(command: &mut Command, mut line: impl FnMut(&str, Duration)) -> Result<String, String> {
    let start = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{}`: {e}", command.get_program().to_string_lossy()))?;
    let stdout = child.stdout.take().ok_or("no standard output")?;
    let mut output = String::new();
    for read in BufReader::new(stdout).lines() {
        let text = read.map_err(|e| format!("reading the output: {e}"))?;
        line(&text, start.elapsed());
        eprintln!("{text}");
        output.push_str(&text);
        output.push('\n');
    }
    let status = child.wait().map_err(|e| format!("waiting for the command: {e}"))?;
    if status.success() { Ok(output) } else { Err(format!("the command ended with {status}")) }
}

/// The arguments of `cargo nextest run` for one ignored test in release.
fn nextest_args(run: Run) -> Vec<String> {
    let Run::Test { package, file, test } = run else { return Vec::new() };
    let module = file.rsplit('/').next().and_then(|name| name.strip_suffix(".rs")).unwrap_or(file);
    [
        "nextest",
        "run",
        "--locked",
        "--cargo-profile",
        "release",
        "--package",
        package,
        "--test",
        "integration",
        "--run-ignored",
        "only",
        // A retried test's output holds every attempt, and its time the failed ones'.
        "--retries",
        "0",
        "--no-capture",
        "--no-tests=fail",
        "-E",
        &format!("test(={module}::{test})"),
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect()
}

fn probe(workspace: &Workspace, machine: &Machine) -> Result<Vec<(String, String)>, String> {
    let mut command = Command::new(&workspace.cargo);
    command.args(nextest_args(PROBE)).current_dir(&workspace.root);
    pass_workers(&mut command, machine);
    figures(&capture(&mut command, |_, _| {})?)
}

/// Gives a test the workers' override as read here, and no empty one.
fn pass_workers(command: &mut Command, machine: &Machine) {
    command.env_remove(WORKERS_VAR);
    if let Some(n) = machine.workers_override {
        command.env(WORKERS_VAR, n.to_string());
    }
}

/// The header's common fields for `m`.
fn header(
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    profile: &'static str,
    workers: String,
    ran: String,
    load: Load,
) -> Header {
    Header {
        measurement: m,
        machine: machine.name.clone(),
        date: machine.date.clone(),
        short: options.short && m.run.cut_by_short(),
        cpu: machine.cpu.clone(),
        os: machine.os.clone(),
        arch: std::env::consts::ARCH,
        topology: machine.topology,
        workers,
        commit: machine.commit.clone(),
        profile,
        command: options.command(),
        ran,
        load,
        notes: Vec::new(),
    }
}

/// The default executor's frame workers, as the probe found them.
fn default_workers(machine: &Machine) -> String {
    machine.default_workers.as_ref().map_or_else(
        || "the default executor's".to_owned(),
        |n| format!("{n} frame workers, the default executor's"),
    )
}

fn write(
    out: &Path,
    header: &Header,
    figures: &[(String, String)],
    output: &str,
) -> Result<Vec<PathBuf>, String> {
    let stem = file_stem(header.measurement.name, &header.machine, &header.date, header.short);
    let md = out.join(format!("{stem}.md"));
    let json_file = out.join(format!("{stem}.json"));
    fs::write(&md, markdown(header, figures, output))
        .map_err(|e| format!("{}: {e}", md.display()))?;
    if figures.is_empty() {
        return Ok(vec![md]);
    }
    fs::write(&json_file, json(header, figures))
        .map_err(|e| format!("{}: {e}", json_file.display()))?;
    Ok(vec![md, json_file])
}

/// Runs the cargo command `args` to build what a measured command then runs, before its
/// sampler starts, so that the sampled window holds the run alone on a cold tree too. The
/// variables a measured command sets are read when it runs, not when it builds. A failed
/// build fails the measurement.
fn build(workspace: &Workspace, args: &[String]) -> Result<(), String> {
    let status = Command::new(&workspace.cargo)
        .args(args)
        .current_dir(&workspace.root)
        // Standard output stays the measurement's; cargo's own is on standard error.
        .stdout(std::io::stderr())
        .status()
        .map_err(|e| format!("cannot run `{}`: {e}", workspace.cargo))?;
    if status.success() { Ok(()) } else { Err(format!("the build ended with {status}")) }
}

/// `args`, a `cargo nextest run` or `cargo bench` command, building only.
fn no_run(args: &[String]) -> Vec<String> {
    args.iter().cloned().chain(["--no-run".to_owned()]).collect()
}

fn short_env(options: &Options) -> Vec<(&'static str, String)> {
    if options.short { vec![(SHORT_VAR, "1".to_owned())] } else { Vec::new() }
}

fn run_test(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
) -> Result<Vec<PathBuf>, String> {
    let args = nextest_args(m.run);
    build(workspace, &no_run(&args))?;
    let env = short_env(options);
    let mut command = Command::new(&workspace.cargo);
    command.args(&args).current_dir(&workspace.root);
    // A short run sets the variable; a full one makes sure the caller's does not leak in.
    command.env_remove(SHORT_VAR).envs(env.iter().map(|(name, value)| (name, value)));
    pass_workers(&mut command, machine);
    let sampler = Sampler::start();
    let output = capture(&mut command, |_, _| {});
    let load = sampler.stop();
    let output = output?;
    let found = figures(&output)?;
    if found.is_empty() {
        return Err(format!("the test printed no line `{FIGURE_PREFIX}<name> <value>`"));
    }
    let workers = found
        .iter()
        .find(|(name, _)| name == "workers" || name == "frame_workers")
        .map_or_else(|| default_workers(machine), |(_, value)| value.clone());
    let mut shown = env.clone();
    if let Some(n) = machine.workers_override {
        shown.push((WORKERS_VAR, n.to_string()));
    }
    let ran = shell_line(&shown, "cargo", &args);
    let mut header = header(options, machine, m, "release", workers, ran, load);
    if let Some(n) = machine.workers_override {
        header.notes.push((
            "Workers override".to_owned(),
            format!("`{WORKERS_VAR}={n}`: N, in place of the topology's"),
        ));
    }
    write(out, &header, &found, &output)
}

/// The replays a run of `spike-net` makes: every one, or a short run's.
fn replays(short: bool) -> impl Iterator<Item = Replay> {
    REPLAYS.into_iter().filter(move |replay| replay.short || !short)
}

/// The netcode spike's replays, one `cargo nextest run` each so that each replay's rollback
/// times are its own, into one pair of files: each replay's report under its scene and
/// clock, and its seconds, `<scene>.<clock>.wall_s`, from the test's first line of output
/// to its report. The first replay that fails fails the measurement.
fn run_spike_replays(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
) -> Result<Vec<PathBuf>, String> {
    // Every replay is a test of one target.
    build(workspace, &no_run(&nextest_args(REPLAYS[0].run())))?;
    let sampler = Sampler::start();
    let (mut found, mut output, mut ran) = (Vec::new(), String::new(), Vec::new());
    let mut reported = std::collections::BTreeSet::new();
    let mut most_held = (0, String::new());
    for replay in replays(options.short) {
        let mut command = Command::new(&workspace.cargo);
        command.args(nextest_args(replay.run())).current_dir(&workspace.root);
        // The replays read neither; a caller's must not reach them unrecorded.
        command.env_remove(SHORT_VAR).env_remove(WORKERS_VAR);
        // From the test's first line to its report: the build, whose output is not on
        // standard output, is not the replay's.
        let (mut first, mut last) = (None, Duration::ZERO);
        let text = capture(&mut command, |line, at| {
            first.get_or_insert(at);
            if line.trim_start().starts_with('{') {
                last = at;
            }
        })
        .map_err(|e| format!("{}: {e}", replay.test))?;
        let seconds = last.saturating_sub(first.unwrap_or_default()).as_secs_f64();
        let (prefix, figures) =
            replay_figures(&text).map_err(|e| format!("{}: {e}", replay.test))?;
        if !reported.insert(prefix.clone()) {
            return Err(format!("{}: a second report of `{prefix}`", replay.test));
        }
        let held = check_replay(&prefix, &figures).map_err(|e| format!("{}: {e}", replay.test))?;
        if held >= most_held.0 {
            most_held = (held, prefix.clone());
        }
        found.extend(figures);
        found.push((format!("{prefix}.wall_s"), format!("{seconds:.1}")));
        output.push_str(&text);
        ran.push(format!("`{}`", replay.test));
    }
    let load = sampler.stop();
    let args = nextest_args(test("lactose-app", SPIKE_FILE, "<replay>"));
    let workers = format!("{SPIKE_WORKERS}, the spike's");
    let mut header =
        header(options, machine, m, "release", workers, shell_line(&[], "cargo", &args), load);
    header
        .notes
        .push(("Replays".to_owned(), format!("{}, one at a time, in this order", ran.join(", "))));
    // Written only once every replay has passed `check_replay`.
    header.notes.push((
        "Server's regions".to_owned(),
        "every replay's `server_frozen_ticks` is 0: the server held every loaded region \
         `Simulated` on every frame from the load to the end, so no correction is a \
         server-`Frozen` region's (architecture section 9.4); the measurement fails otherwise"
            .to_owned(),
    ));
    header.notes.push((
        "Prediction bound".to_owned(),
        format!(
            "at most {PREDICTION_BOUND} fixed colliders and proxies together in a prediction world \
             (plan decision 5.5); every replay's `prediction_fixed_and_proxies`, the most its \
             prediction world held after a client frame, is within it, the largest {} \
             (`{}`); the measurement fails otherwise",
            most_held.0, most_held.1
        ),
    ));
    if options.short {
        header.notes.push((
            "Short run".to_owned(),
            "the dynamic capsule on each clock, the cheapest pair; a replay is ten minutes of \
             ticks either way"
                .to_owned(),
        ));
    }
    write(out, &header, &found, &output)
}

/// The WHISK benchmark's own pair, written into a directory under the target directory and
/// moved into `out`, the summary prefaced with this command's rows (the command, the
/// profile, the operating system and the load); a short run's carry `-short` and say so at
/// the top of the summary.
fn run_task_bench(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    out: &Path,
) -> Result<Vec<PathBuf>, String> {
    let scratch = workspace.target_dir.join("measure").join("whisk");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let args: Vec<String> = ["bench", "--locked", "-p", "lactose-task", "--bench", "whisk"]
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect();
    let mut env = vec![("WHISK_BENCH_MACHINE", machine.name.clone())];
    if options.short {
        env.push(("WHISK_BENCH_RUNS", "100".to_owned()));
    }
    let mut command = Command::new(&workspace.cargo);
    command
        .args(&args)
        .current_dir(&workspace.root)
        .env("WHISK_BENCH_OUT", &scratch)
        // A short run sets the runs; a full one makes sure the caller's do not leak in.
        .env_remove("WHISK_BENCH_RUNS")
        .envs(env.iter().map(|(name, value)| (name, value)));
    build(workspace, &no_run(&args))?;
    let sampler = Sampler::start();
    let output = capture(&mut command, |_, _| {});
    let load = sampler.stop();
    output?;
    let (load_name, load) = load_text(&load);
    let mut preface = String::new();
    if options.short {
        preface.push_str(
            "> A short run (`WHISK_BENCH_RUNS=100`): its repetitions were cut so that the \
             command is known to finish; its numbers mean nothing.\n\n",
        );
    }
    let ran = shell_line(&env, "cargo", &args);
    let _ = writeln!(preface, "| | |\n|---|---|");
    let _ = writeln!(
        preface,
        "| Command | `{}`, which ran `{ran}`, its files written under the target directory and \
         moved here |",
        options.command()
    );
    let _ = writeln!(preface, "| Profile | bench |");
    let _ = writeln!(preface, "| OS | {} ({}) |", machine.os, std::env::consts::ARCH);
    let _ = writeln!(preface, "| {load_name} | {load} |\n");
    let mut moved = Vec::new();
    let entries = fs::read_dir(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    for entry in entries {
        let from = entry.map_err(|e| e.to_string())?.path();
        let (Some(stem), Some(extension)) = (from.file_stem(), from.extension()) else { continue };
        let (stem, extension) = (stem.to_string_lossy(), extension.to_string_lossy());
        let suffix = if options.short { "-short" } else { "" };
        let to = out.join(format!("{stem}{suffix}.{extension}"));
        let mut text = fs::read_to_string(&from).map_err(|e| format!("{}: {e}", from.display()))?;
        if extension == "md" {
            text = format!("{preface}{text}");
        }
        fs::write(&to, text).map_err(|e| format!("{}: {e}", to.display()))?;
        moved.push(to);
    }
    if moved.is_empty() {
        return Err("the benchmark wrote no file".to_owned());
    }
    moved.sort();
    Ok(moved)
}

fn headless_args(ticks: &str, target_dir: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec!["run".into(), "--locked".into()];
    if let Some(dir) = target_dir {
        args.push("--target-dir".into());
        args.push(dir.to_owned());
    }
    args.extend(["-p", "lactose-app-headless", "--", ticks].iter().map(|arg| (*arg).to_owned()));
    args
}

fn run_headless(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
) -> Result<Vec<PathBuf>, String> {
    let args = headless_args("120", None);
    // `cargo run` has no `--no-run`: `cargo build` with its options builds the binary.
    let until = args.iter().position(|arg| arg == "--").unwrap_or(args.len());
    build(workspace, &[&["build".to_owned()], &args[1..until]].concat())?;
    let mut command = Command::new(&workspace.cargo);
    command.args(&args).current_dir(&workspace.root);
    let sampler = Sampler::start();
    let output = capture(&mut command, |_, _| {});
    let load = sampler.stop();
    let output = output?;
    let report =
        output.lines().find_map(headless_report).ok_or("the binary printed no report line")?;
    let found = vec![
        ("ticks".to_owned(), report.ticks.to_string()),
        ("frames".to_owned(), report.frames.to_string()),
        ("behind".to_owned(), report.behind.to_string()),
        ("hash".to_owned(), report.hash),
    ];
    let ran = shell_line(&[], "cargo", &args);
    let header = header(options, machine, m, "dev", default_workers(machine), ran, load);
    write(out, &header, &found, &output)
}

/// The detached worktree of the edit-to-playing loop: registered under the target
/// directory, removed again however the loop ends, so the developer's tree is never
/// touched and no registration outlives a run that ends normally. Only this worktree is
/// ever removed; the repository's other registrations are not this command's.
struct Copy {
    root: PathBuf,
    tree: PathBuf,
}

impl Copy {
    fn git(&self, args: &[&str]) -> Result<(), String> {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .stdout(Stdio::null())
            .status()
            .map_err(|e| format!("cannot run git: {e}"))?;
        if status.success() { Ok(()) } else { Err(format!("`git {}` failed", args.join(" "))) }
    }

    fn add(root: &Path, tree: PathBuf) -> Result<Self, String> {
        let copy = Self { root: root.to_owned(), tree };
        // What a killed run left behind.
        copy.remove();
        let path = copy.tree.to_string_lossy().into_owned();
        copy.git(&["worktree", "add", "--detach", &path, "HEAD"])?;
        Ok(copy)
    }

    /// Removes this worktree's registration, which fails harmlessly when there is none, then
    /// its directory. Forced twice, so that a registration a kill left locked mid-`add`
    /// ("initializing") goes too; quiet, since on a first run there is nothing to remove.
    fn remove(&self) {
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["worktree", "remove", "--force", "--force"])
            .arg(&self.tree)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_dir_all(&self.tree);
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        self.remove();
    }
}

/// The bytes of every file under `dir`.
fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

fn run_edit_to_playing(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
) -> Result<Vec<PathBuf>, String> {
    let dir = workspace.target_dir.join(COPY_DIR);
    let copy = Copy::add(&workspace.root, dir.join("tree"))?;
    // Relative to the copy, so that no file names a path of this machine.
    let args = headless_args("1", Some("../target"));
    // Builds and runs the binary in the copy; with the time the edit was saved, returns the
    // time from then to the report line.
    let run = |saved: Option<Instant>| -> Result<Option<Duration>, String> {
        let mut command = Command::new(&workspace.cargo);
        command.args(&args).current_dir(&copy.tree);
        let mut reported = None;
        capture(&mut command, |line, _| {
            if reported.is_none() && headless_report(line).is_some() {
                reported = saved.map(|saved| saved.elapsed());
            }
        })?;
        if saved.is_some() && reported.is_none() {
            return Err("the binary printed no report line".to_owned());
        }
        Ok(reported)
    };
    let sampler = Sampler::start();
    let mut found = Vec::new();
    let mut medians = Vec::new();
    let mut outcome = Ok(());
    'sites: for site in &SITES {
        let file = copy.tree.join(site.file);
        let original = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        eprintln!("measure: edit-to-playing: the warm build before {}", site.name);
        if let Err(error) = run(None) {
            outcome = Err(error);
            break;
        }
        let mut seconds = Vec::new();
        for n in 1..=EDITS_PER_SITE {
            let text = edited(&original, site, n)?;
            fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
            match run(Some(Instant::now())) {
                Ok(Some(at)) => seconds.push(at.as_secs_f64()),
                Ok(None) => unreachable!("a timed run reports or fails"),
                Err(error) => {
                    outcome = Err(error);
                    break 'sites;
                }
            }
        }
        // The next site's warm build starts from the tree as committed.
        fs::write(&file, &original).map_err(|e| format!("{}: {e}", file.display()))?;
        let runs: Vec<String> = seconds.iter().map(|s| format!("{s:.2}")).collect();
        found.push((format!("{}.runs_s", site.name), runs.join(", ")));
        found.push((format!("{}.median_s", site.name), format!("{:.2}", median(&seconds))));
        medians.push(median(&seconds));
    }
    let load = sampler.stop();
    drop(copy);
    outcome?;
    let slower = medians.iter().copied().fold(0.0, f64::max);
    found.push(("slower_median_s".to_owned(), format!("{slower:.2}")));
    let gib = dir_size(&dir.join("target")) as f64 / f64::from(1 << 30);
    found.push(("copy_target_gib".to_owned(), format!("{gib:.1}")));
    let ran = shell_line(&[], "cargo", &args);
    let mut header = header(options, machine, m, "dev", default_workers(machine), ran, load);
    // The copy is `HEAD` as committed, whatever the developer's tree holds.
    header.commit.clone_from(&machine.head);
    header.notes.push((
        "Local changes".to_owned(),
        "not measured: the loop runs on `HEAD` as committed".to_owned(),
    ));
    header.notes.push((
        "Loop".to_owned(),
        format!(
            "in a detached worktree of `HEAD` at `{COPY_DIR}/tree` under the target directory: \
             one warm build before each site, then {EDITS_PER_SITE} one-line edits there, each \
             timed from its save to the report line"
        ),
    ));
    for site in &SITES {
        header.notes.push((
            format!("Edit `{}`", site.name),
            format!("`{}`, after `{}`", site.file, site.anchor.trim()),
        ));
    }
    header.notes.push((
        "Copy".to_owned(),
        format!(
            "the worktree is removed after the run; its target directory, `{COPY_DIR}/target` \
             under the workspace's, {gib:.1} GiB, is kept for the next run's warm build"
        ),
    ));
    write(out, &header, &found, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository(path: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        fs::read_to_string(root.join(path)).unwrap()
    }

    #[test]
    fn figures_are_the_measure_lines_in_order() {
        let output = "running 1 test\nmeasure: t1_us 1234.5\nT1 1234.5 µs\nmeasure: workers 1 and 4\n\
                      measure:unprefixed\nmeasure: hash 0x00ab\r\ntest ... ok\n";
        assert_eq!(
            figures(output).unwrap(),
            [
                ("t1_us".to_owned(), "1234.5".to_owned()),
                ("workers".to_owned(), "1 and 4".to_owned()),
                ("hash".to_owned(), "0x00ab".to_owned()),
            ]
        );
        assert_eq!(figures("running 1 test\ntest ... ok\n").unwrap(), []);
    }

    /// Review r1 of spike-net-file, M2: a retried test prints every attempt's figures, and
    /// the parse took both.
    #[test]
    fn a_figure_printed_twice_is_an_error() {
        let error = figures("measure: n 4\nmeasure: t1_us 1.5\nmeasure: n 4\n").unwrap_err();
        assert!(error.contains("`n` is printed twice"), "{error}");
        assert_eq!(figures("measure: n 4\nmeasure: n.w4 4\n").unwrap().len(), 2);
    }

    /// Review r1 of spike-net-file, M2: nextest retried a failed test under `NEXTEST_RETRIES`
    /// or a user's config, and the measurement read both attempts.
    #[test]
    fn a_measured_test_is_never_retried() {
        for run in [PROBE, REPLAYS[0].run()].into_iter().chain(MEASUREMENTS.iter().map(|m| m.run)) {
            if let Run::Test { test, .. } = run {
                let args = nextest_args(run);
                let at = args.iter().position(|arg| arg == "--retries").unwrap();
                assert_eq!(args[at + 1], "0", "{test}");
            }
        }
    }

    /// A test is selected in its package's one test binary by its path there (D150).
    #[test]
    fn a_measured_test_is_named_by_its_path_in_the_integration_target() {
        let args = nextest_args(PROBE);
        let at = args.iter().position(|arg| arg == "--test").unwrap();
        assert_eq!(args[at + 1], "integration");
        assert_eq!(args.last().unwrap(), "test(=cost::the_topology_the_executor_detects)");
        let args =
            nextest_args(MEASUREMENTS.iter().find(|m| m.name == "composed-tick").unwrap().run);
        assert_eq!(args.last().unwrap(), "test(=cost::composed::cost_of_the_composed_tick)");
    }

    /// Review r1 of spike-net-file, M1: serde_json's default float parse is not correctly
    /// rounded, and some figures were 1 ULP from the printed report.
    #[test]
    fn a_replay_s_floats_are_the_printed_values_bit_for_bit() {
        let printed = "1.9990871056907666e-2";
        let output = REPLAY_OUTPUT.replace("1.99995710109393e-2", printed);
        let (prefix, figures) = replay_figures(&output).unwrap();
        let name = format!("{prefix}.absorbed.angular");
        let value = &figures.iter().find(|(n, _)| *n == name).unwrap().1;
        let expected = printed.parse::<f64>().unwrap();
        assert_eq!(value.parse::<f64>().unwrap().to_bits(), expected.to_bits(), "{value}");
        assert_eq!(json_value(value), serde_json::json!(expected));
    }

    /// Review r1 of spike-net-file, S1: the header said the server's region was `Simulated`
    /// whatever the replay did. A replay that held it otherwise, or a prediction world over
    /// the bound, fails the measurement and names its count.
    #[test]
    fn a_replay_outside_a_simulated_region_or_over_the_bound_fails() {
        let (prefix, figures) = replay_figures(REPLAY_OUTPUT).unwrap();
        let with = |key: &str, value: &str| {
            let name = format!("{prefix}.{key}");
            let mut changed = figures.clone();
            changed.iter_mut().find(|(n, _)| *n == name).unwrap().1 = value.to_owned();
            changed
        };
        let frozen = check_replay(&prefix, &with("server_frozen_ticks", "36000")).unwrap_err();
        assert!(frozen.contains("36000 server frames") && frozen.contains(&prefix), "{frozen}");
        assert_eq!(check_replay(&prefix, &with("prediction_fixed_and_proxies", "64")), Ok(64));
        let over = check_replay(&prefix, &with("prediction_fixed_and_proxies", "65")).unwrap_err();
        assert!(over.contains("held 65") && over.contains(&prefix), "{over}");
        for key in ["server_frozen_ticks", "prediction_fixed_and_proxies"] {
            let missing: Vec<_> =
                figures.iter().filter(|(n, _)| !n.ends_with(key)).cloned().collect();
            let error = check_replay(&prefix, &missing).unwrap_err();
            assert!(error.contains(key), "{error}");
            let error = check_replay(&prefix, &with(key, "-1")).unwrap_err();
            assert!(error.contains(key), "{error}");
        }
    }

    /// Review r1, M1: a figure line that does not read was dropped, and the file written
    /// without it.
    #[test]
    fn a_figure_line_that_does_not_read_is_an_error() {
        for line in ["measure: t1_us", "measure: t1_us   ", "measure:  1.5", "measure: "] {
            let output = format!("measure: n 4\n{line}\n");
            let error = figures(&output).unwrap_err();
            assert!(
                error.contains(line.trim_start_matches(FIGURE_PREFIX).trim_end()),
                "{line}: {error}"
            );
        }
    }

    #[test]
    fn a_figure_is_a_number_in_the_json_where_it_reads_as_one() {
        assert_eq!(json_value("100064"), serde_json::json!(100_064));
        assert_eq!(json_value("28043.3"), serde_json::json!(28_043.3));
        assert_eq!(json_value("0x00ab"), serde_json::json!("0x00ab"));
        assert_eq!(json_value("1 and 4"), serde_json::json!("1 and 4"));
        assert_eq!(json_value("NaN"), serde_json::json!("NaN"));
        assert_eq!(json_value("[0,3,95]"), serde_json::json!([0, 3, 95]));
        assert_eq!(json_value("true"), serde_json::json!(true));
        assert_eq!(json_value("null"), serde_json::Value::Null);
        assert_eq!(json_value("-0.0"), serde_json::json!(-0.0));
        assert_eq!(json_value("[0, 3"), serde_json::json!("[0, 3"));
        assert_eq!(json_value("\"quoted\""), serde_json::json!("\"quoted\""));
    }

    /// One replay's output as nextest passes it through, its report line as the spike's
    /// `report` prints it, shortened.
    const REPLAY_OUTPUT: &str = "\nrunning 1 test\n\
        {\"scene\":\"fifty thrown bodies\",\"clock_script\":true,\"ticks\":36000,\"prefix_equal\":true,\
        \"corrections_per_minute\":{\"mismatch\":772.60,\"substituted\":17.90,\"repaired\":3.00},\
        \"absorbed\":{\"fields\":1235914,\"translation\":1.998241455073213e-3,\"angular\":1.99995710109393e-2},\
        \"rollback_us_per_tick\":{\"median\":550.9,\"worst\":16981.9,\"rollbacks\":7935},\
        \"departure_tick\":null,\"mismatches_per_cycle\":[3, 49, 5],\
        \"yaw_error_rad_per_tick\":{\"mean\":-0e0,\"worst\":0e0},\
        \"server_frozen_ticks\":0,\"prediction_fixed_and_proxies\":12}\n\
        test ten_minutes_of_fifty_thrown_bodies_with_the_clock_script ... ok\n\n\
        test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out\n";

    #[test]
    fn a_replay_s_figures_are_its_report_under_its_scene_and_clock() {
        let (prefix, figures) = replay_figures(REPLAY_OUTPUT).unwrap();
        assert_eq!(prefix, "fifty_thrown_bodies.clock_script");
        let get = |name: &str| {
            let name = format!("{prefix}.{name}");
            figures.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
        };
        assert_eq!(get("ticks"), Some("36000"));
        assert_eq!(get("prefix_equal"), Some("true"));
        assert_eq!(get("corrections_per_minute.mismatch"), Some("772.6"));
        assert_eq!(get("absorbed.translation"), Some("0.001998241455073213"));
        assert_eq!(get("rollback_us_per_tick.median"), Some("550.9"));
        assert_eq!(get("departure_tick"), Some("null"));
        assert_eq!(get("mismatches_per_cycle"), Some("[3,49,5]"));
        assert_eq!(get("yaw_error_rad_per_tick.mean"), Some("-0.0"));
        assert_eq!(get("scene"), None);
        assert_eq!(get("clock_script"), None);
        assert_eq!(figures.len(), 17);
        assert_eq!(check_replay(&prefix, &figures), Ok(12));
        assert!(figures.iter().all(|(name, _)| !name.contains(' ')), "{figures:?}");
        let header = Header { measurement: find("spike-net").unwrap(), ..sample_header() };
        let value: serde_json::Value = serde_json::from_str(&json(&header, &figures)).unwrap();
        let figure = |name: &str| &value["figures"][format!("{prefix}.{name}")];
        assert_eq!(figure("prefix_equal"), &serde_json::json!(true));
        assert_eq!(figure("departure_tick"), &serde_json::Value::Null);
        assert_eq!(figure("mismatches_per_cycle"), &serde_json::json!([3, 49, 5]));
        assert_eq!(figure("absorbed.angular"), &serde_json::json!(1.99995710109393e-2));
        assert_eq!(figure("rollback_us_per_tick.rollbacks"), &serde_json::json!(7935));

        let server = REPLAY_OUTPUT.replace("\"clock_script\":true", "\"clock_script\":false");
        assert_eq!(replay_figures(&server).unwrap().0, "fifty_thrown_bodies.server_clock");
    }

    /// bef3455: a line read from a child process may end in CRLF on Windows. The report reads
    /// the same from output with CRLF line endings, and from a last line that ends in CR.
    #[test]
    fn a_replay_s_report_reads_with_crlf_line_endings() {
        let crlf = REPLAY_OUTPUT.replace('\n', "\r\n");
        assert_eq!(replay_figures(&crlf).unwrap(), replay_figures(REPLAY_OUTPUT).unwrap());
        let last = format!("{}\r", REPLAY_OUTPUT.lines().nth(2).unwrap());
        assert_eq!(replay_figures(&last).unwrap(), replay_figures(REPLAY_OUTPUT).unwrap());
    }

    #[test]
    fn a_replay_prints_one_report_that_reads() {
        let report = REPLAY_OUTPUT.lines().find(|line| line.starts_with('{')).unwrap();
        let none = REPLAY_OUTPUT.replace(report, "");
        assert!(replay_figures(&none).unwrap_err().contains("0 lines"));
        let twice = format!("{REPLAY_OUTPUT}{report}\n");
        assert!(replay_figures(&twice).unwrap_err().contains("2 lines"));
        let cut = REPLAY_OUTPUT.replace(report, &report[..report.len() / 2]);
        assert!(replay_figures(&cut).unwrap_err().contains("not a JSON object"));
        let unnamed = REPLAY_OUTPUT.replace("\"scene\":\"fifty thrown bodies\",", "");
        assert!(replay_figures(&unnamed).unwrap_err().contains("names no `scene`"));
        let clockless = REPLAY_OUTPUT.replace("\"clock_script\":true", "\"clock_script\":1");
        assert!(replay_figures(&clockless).unwrap_err().contains("names no `scene`"));
    }

    /// The replays are the file's ten-minute tests but the claim on approach, in its order,
    /// and a short run keeps one scene on each clock.
    #[test]
    fn the_replays_are_the_spike_s_and_a_short_run_keeps_one_scene_on_each_clock() {
        let source = repository(SPIKE_FILE).replace("\r\n", "\n");
        let in_file: Vec<&str> = source
            .split("fn ")
            .skip(1)
            .filter_map(|rest| rest.split_once('(').map(|(name, _)| name))
            .filter(|name| name.starts_with("ten_minutes_of_"))
            .collect();
        let named: Vec<&str> = REPLAYS.iter().map(|replay| replay.test).collect();
        let mut expected = in_file.clone();
        expected.retain(|name| *name != "ten_minutes_of_a_claim_on_approach_on_the_server_clock");
        assert_eq!(named, expected, "every replay but the claim on approach");
        assert_eq!(in_file.len(), named.len() + 1);

        assert_eq!(replays(false).count(), REPLAYS.len());
        let short: Vec<&str> = replays(true).map(|replay| replay.test).collect();
        let on = |clock: &str| short.iter().filter(|name| name.ends_with(clock)).count();
        assert_eq!(short.len(), 2, "{short:?}");
        assert_eq!((on("_with_the_clock_script"), on("_on_the_server_clock")), (1, 1), "{short:?}");
        let scene = |name: &str| {
            name.trim_end_matches("_with_the_clock_script")
                .trim_end_matches("_on_the_server_clock")
                .to_owned()
        };
        assert_eq!(scene(short[0]), scene(short[1]), "one scene: {short:?}");
        assert!(Run::SpikeReplays.cut_by_short());

        let body = source.split("fn measure(").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(
            body.contains(&format!("let executor = executor({SPIKE_WORKERS});")),
            "the replays' workers"
        );
    }

    #[test]
    fn the_machine_slug_follows_the_task_benchmark() {
        assert_eq!(slug("Apple M1"), "apple-m1");
        assert_eq!(slug("Intel(R) Core(TM) i9-13900K"), "intel-r-core-tm-i9-13900k");
        assert_eq!(
            slug("  AMD Ryzen 9 7950X 16-Core Processor  "),
            "amd-ryzen-9-7950x-16-core-processor"
        );
    }

    #[test]
    fn the_date_is_utc_days_to_civil() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_794_787_199), "2026-11-15");
        assert_eq!(date(1_794_787_200), "2026-11-16");
    }

    #[test]
    fn a_short_run_s_files_say_so_in_their_name() {
        assert_eq!(
            file_stem("tick-100k", "apple-m1", "2026-11-15", false),
            "tick-100k-apple-m1-2026-11-15"
        );
        assert_eq!(
            file_stem("tick-100k", "apple-m1", "2026-11-15", true),
            "tick-100k-apple-m1-2026-11-15-short"
        );
    }

    #[test]
    fn the_load_average_is_read_from_uptime_on_macos_and_linux() {
        let mac = " 5:41  up 7 days, 16:08, 2 users, load averages: 10.73 46.34 37.61";
        assert_eq!(load_average(mac).as_deref(), Some("10.73 46.34 37.61"));
        let linux = " 05:41:02 up 3 days,  1:02,  1 user,  load average: 0.52, 0.58, 0.59";
        assert_eq!(load_average(linux).as_deref(), Some("0.52, 0.58, 0.59"));
        assert_eq!(load_average("no such line"), None);
    }

    #[test]
    fn the_idle_share_is_the_mean_of_the_wmi_sampler_s_lines_but_the_first() {
        assert_eq!(idle_share("0\r\n90\r\n80\r\n", 3.0), Ok(85.0));
        assert_eq!(
            idle_share("\r\n100\r\n97\r\nGet-CimInstance : Access denied\r\n93\r\n", 4.0),
            Ok(95.0)
        );
        // Two lines: the first dropped, the second the share.
        assert_eq!(idle_share("0\r\n90\r\n", 2.5), Ok(90.0));
    }

    #[test]
    fn a_run_with_no_idle_share_says_why() {
        let one = "the run took 1.6 s, and the sampler printed only its first sample, which is \
                   dropped, in it";
        assert_eq!(idle_share("97\r\n", 1.64), Err(one.to_owned()));
        assert_eq!(idle_share("97", 1.64), Err(one.to_owned()));
        let none = "the run took 0.8 s, and the sampler printed no sample in it";
        assert_eq!(idle_share("", 0.8), Err(none.to_owned()));
        assert_eq!(idle_share("\r\n", 0.8), Err(none.to_owned()));
        // A sampler whose query fails every time prints no sample, however long the run.
        assert_eq!(
            idle_share("Get-CimInstance : Invalid class\r\nNaN\r\n", 60.0),
            Err("the run took 60.0 s, and the sampler printed no sample in it".to_owned())
        );
    }

    /// Review r2, m1: an empty override was unset here but an error to every cost test, and
    /// a value that is not a count reached them.
    #[test]
    fn the_workers_override_is_a_count_above_zero_or_unset() {
        use std::ffi::OsStr;
        assert_eq!(workers_override(None), Ok(None));
        assert_eq!(workers_override(Some(OsStr::new(""))), Ok(None));
        assert_eq!(workers_override(Some(OsStr::new("2"))), Ok(Some(2)));
        for value in ["0", "-1", "two", " 2", "2.5"] {
            let error = workers_override(Some(OsStr::new(value))).unwrap_err();
            assert!(error.starts_with(WORKERS_VAR), "{value}: {error}");
            assert!(error.contains(&format!("`{value}`")), "{value}: {error}");
        }
    }

    #[test]
    fn the_cpu_name_is_read_from_reg_query_s_output() {
        let output = "\r\nHKEY_LOCAL_MACHINE\\HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0\r\n\
                      \x20   ProcessorNameString    REG_SZ    AMD EPYC 7763 64-Core Processor\r\n\r\n";
        assert_eq!(
            registry_value(output, "ProcessorNameString").as_deref(),
            Some("AMD EPYC 7763 64-Core Processor")
        );
        assert_eq!(registry_value("ERROR: not found", "ProcessorNameString"), None);
    }

    /// The WHISK benchmark names its machine and reads the CPU name with copies of these
    /// functions, since a benchmark cannot depend on this tool and one without the test
    /// harness runs no test; this keeps the copies the same.
    #[test]
    fn the_benchmark_s_copies_of_the_naming_functions_are_the_same() {
        fn function<'a>(source: &'a str, name: &str) -> &'a str {
            let start = source.find(&format!("\nfn {name}(")).unwrap_or_else(|| panic!("{name}"));
            let end = start + source[start..].find("\n}\n").unwrap_or_else(|| panic!("{name}"));
            &source[start..end]
        }
        let measure = repository("tools/xtask/src/measure.rs").replace("\r\n", "\n");
        let bench = repository("crates/task/benches/whisk.rs").replace("\r\n", "\n");
        for name in ["registry_value", "slug"] {
            assert_eq!(function(&bench, name), function(&measure, name), "{name}");
        }
    }

    #[test]
    fn the_headless_report_line_is_read_whole() {
        let line = "120 ticks in 121 frames (0 behind), hash 0x3a5f00c1d2e3f405";
        assert_eq!(
            headless_report(line),
            Some(HeadlessReport {
                ticks: 120,
                frames: 121,
                behind: 0,
                hash: "0x3a5f00c1d2e3f405".to_owned()
            })
        );
        assert_eq!(headless_report("saved to slot"), None);
        assert_eq!(headless_report("1 ticks in 1 frames (0 behind), hash 12"), None);
    }

    /// The edits land in the files as they are now: each anchor occurs once, inside the
    /// function decision 5.9 names, and the edit adds one line.
    #[test]
    fn each_edit_s_anchor_occurs_once_in_its_file() {
        for site in &SITES {
            let source = repository(site.file);
            let first = edited(&source, site, 1).unwrap();
            let second = edited(&source, site, 2).unwrap();
            assert_eq!(first.lines().count(), source.lines().count() + 1, "{}", site.file);
            assert_ne!(first, second);
        }
        // As the tree is checked out: with CRLF line endings on a Windows runner.
        let body_of = |site: &Site, function: &str| {
            let source = repository(site.file).replace("\r\n", "\n");
            let body = source.split(function).nth(1).unwrap().split("\n}\n").next().unwrap();
            body.lines().any(|line| line == site.anchor)
        };
        assert!(body_of(&SITES[0], "fn count("), "the sketch system's body");
        assert!(body_of(&SITES[1], "pub fn start_executor("), "a lactose-app function's body");
        let twice = format!("{0}\n{0}\n", SITES[0].anchor);
        assert!(edited(&twice, &SITES[0], 1).unwrap_err().contains("2 lines"));
        let indented = format!("    {}\n", SITES[0].anchor);
        assert!(edited(&indented, &SITES[0], 1).unwrap_err().contains("0 lines"));
        assert!(edited(SITES[0].anchor, &SITES[0], 1).unwrap_err().contains("no ending"));
    }

    /// CI run 43654643980: the Windows runner checks the tree out with CRLF line endings,
    /// and an anchor that ended in "\n" was in neither file. Each site's edit of a CRLF copy
    /// of its file is the edit of the LF file with CRLF endings: one line more, every line
    /// still ended by CRLF.
    #[test]
    fn each_edit_keeps_a_crlf_file_s_line_endings() {
        for site in &SITES {
            let lf = repository(site.file).replace("\r\n", "\n");
            let crlf = lf.replace('\n', "\r\n");
            let first = edited(&crlf, site, 1).unwrap();
            let second = edited(&crlf, site, 2).unwrap();
            assert_eq!(first.lines().count(), crlf.lines().count() + 1, "{}", site.file);
            assert_eq!(first.matches('\n').count(), first.matches("\r\n").count(), "{}", site.file);
            assert_eq!(first.matches('\r').count(), first.matches("\r\n").count(), "{}", site.file);
            assert_ne!(first, second);
            assert_eq!(first, edited(&lf, site, 1).unwrap().replace('\n', "\r\n"), "{}", site.file);
            assert!(!edited(&lf, site, 1).unwrap().contains('\r'), "{}", site.file);
        }
    }

    /// Every test a measurement names is in its file, ignored, and every measurement is
    /// listed in the results README with its command.
    #[test]
    fn every_measurement_names_a_test_that_exists_and_is_listed() {
        let readme = repository("docs/results/README.md");
        let mut names = std::collections::BTreeSet::new();
        let replays = REPLAYS.map(|replay| Measurement {
            name: replay.test,
            what: "",
            threshold: "",
            run: replay.run(),
        });
        for m in MEASUREMENTS.iter().chain(&replays).chain([&Measurement {
            name: "probe",
            what: "",
            threshold: "",
            run: PROBE,
        }]) {
            assert!(names.insert(m.name), "{} is named twice", m.name);
            if let Run::Test { file, test, .. } = m.run {
                let source = repository(file);
                let name = test.rsplit("::").next().unwrap();
                let at = source
                    .find(&format!("fn {name}("))
                    .unwrap_or_else(|| panic!("{test} in {file}"));
                let before = &source[..at];
                let attributes = &before[before.rfind("#[test]").unwrap()..];
                assert!(attributes.contains("#[ignore"), "{test} is ignored");
            }
            if m.name != "probe" && !m.name.starts_with("ten_minutes_of_") {
                assert!(readme.contains(&format!("`{}`", m.name)), "{} in the README", m.name);
            }
        }
    }

    #[test]
    fn the_arguments_select_measurements() {
        let all = parse(&["--all", "--short"]).unwrap();
        assert_eq!(all.selected.len(), MEASUREMENTS.len());
        assert!(all.short);
        assert_eq!(all.out, PathBuf::from(RESULTS));
        assert_eq!(all.command(), "cargo xtask measure --all --short");
        let two = parse(&["tick-100k", "whisk", "tick-100k", "--out", "elsewhere"]).unwrap();
        let names: Vec<&str> = two.selected.iter().map(|m| m.name).collect();
        assert_eq!(names, ["tick-100k", "whisk"]);
        assert_eq!(two.out, PathBuf::from("elsewhere"));
        assert!(parse(&[]).is_err());
        assert!(parse(&["--all", "whisk"]).is_err());
        assert!(parse(&["nothing"]).is_err());
        assert!(parse(&["--out"]).is_err());
        assert!(parse(&["--all", "--out", "a", "--out", "b"]).is_err());
    }

    /// Review r1, M7: an absolute `--out` was written into the Command row as given, a
    /// path of the machine, and one outside the repository was taken.
    #[test]
    fn the_output_directory_is_inside_the_repository_and_written_relative_to_it() {
        let root = std::env::temp_dir().join("repository");
        let inside = root.join("target").join("results");
        let inside = inside.to_str().unwrap();
        let options = parse(&["--all", "--out", inside, "--short"]).unwrap();
        let resolved = options.resolved(&root).unwrap();
        assert_eq!(resolved.out, Path::new("target").join("results"));
        assert_eq!(resolved.command(), "cargo xtask measure --all --out target/results --short");
        let relative = parse(&["--all", "--out", "target/results"]).unwrap();
        assert_eq!(relative.resolved(&root).unwrap().out, PathBuf::from("target/results"));
        let default = parse(&["--all"]).unwrap().resolved(&root).unwrap();
        assert_eq!(default.out, PathBuf::from(RESULTS));
        let elsewhere = std::env::temp_dir().join("elsewhere");
        for out in [elsewhere.to_str().unwrap(), "../elsewhere", "target/../../elsewhere"] {
            let error = parse(&["--all", "--out", out]).unwrap().resolved(&root).unwrap_err();
            assert!(error.contains("outside the repository"), "{out}: {error}");
        }
    }

    fn sample_header() -> Header {
        Header {
            measurement: find("tick-100k").unwrap(),
            machine: "apple-m1".into(),
            date: "2026-11-15".into(),
            short: true,
            cpu: "Apple M1".into(),
            os: "macos 14.6".into(),
            arch: "aarch64",
            topology: Some((4, 4)),
            workers: "1 and 4".into(),
            commit: "0123456789ab-dirty".into(),
            profile: "release",
            command: "cargo xtask measure tick-100k --short".into(),
            ran: "LACTOSE_MEASURE_SHORT=1 cargo nextest run".into(),
            load: Load::Averages { before: "1.0 2.0 3.0".into(), after: "2.0 2.0 3.0".into() },
            notes: Vec::new(),
        }
    }

    #[test]
    fn a_result_file_opens_with_the_header() {
        let header = sample_header();
        let figures = vec![("t1_us".to_owned(), "1.5".to_owned())];
        let text = markdown(&header, &figures, "measure: t1_us 1.5\n");
        for expected in [
            "# tick-100k, apple-m1, 2026-11-15, short run",
            "| CPU | Apple M1 (macos 14.6, aarch64) |",
            "| Topology | 4 performance + 4 efficiency cores, as the executor detected them |",
            "| Workers | 1 and 4 |",
            "| Commit | `0123456789ab-dirty` |",
            "| Profile | release |",
            "| Load average | 1.0 2.0 3.0 before, 2.0 2.0 3.0 after |",
            "| `t1_us` | 1.5 |",
        ] {
            assert!(text.contains(expected), "{expected} in\n{text}");
        }
        let value: serde_json::Value = serde_json::from_str(&json(&header, &figures)).unwrap();
        assert_eq!(value["figures"]["t1_us"], serde_json::json!(1.5));
        assert_eq!(value["topology"]["performance"], serde_json::json!(4));
        assert_eq!(value["short"], serde_json::json!(true));
        let windows = Header { load: Load::Idle(Ok(93.25)), ..header.clone() };
        assert!(markdown(&windows, &[], "").contains("| Processor idle | 93.2% over the run |"));
        let value: serde_json::Value = serde_json::from_str(&json(&windows, &figures)).unwrap();
        assert_eq!(value["load"]["processor_idle_percent"], serde_json::json!(93.25));
        let why = idle_share("97\r\n", 1.64).unwrap_err();
        let short = Header { load: Load::Idle(Err(why)), ..header };
        assert!(markdown(&short, &[], "").contains(
            "| Processor idle | unavailable: the run took 1.6 s, and the sampler printed only its \
             first sample, which is dropped, in it |"
        ));
        let value: serde_json::Value = serde_json::from_str(&json(&short, &figures)).unwrap();
        assert_eq!(value["load"]["processor_idle_percent"], serde_json::Value::Null);
    }

    #[test]
    fn a_shell_line_quotes_what_needs_it() {
        let args: Vec<String> =
            ["nextest", "-E", "test(=a::b)"].iter().map(|a| (*a).to_owned()).collect();
        assert_eq!(
            shell_line(&[(SHORT_VAR, "1".to_owned())], "cargo", &args),
            "LACTOSE_MEASURE_SHORT=1 cargo nextest -E 'test(=a::b)'"
        );
    }
}
