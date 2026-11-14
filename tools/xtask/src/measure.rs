//! `cargo xtask measure`: the measurements the milestone criteria rest on, each written to a
//! result file in `docs/results` (architecture section 20; milestone 1 plan, section 4 and
//! decision 5.11).
//!
//! A measurement is an ignored release test that prints its figures, the WHISK benchmark,
//! the headless binary's 120-tick run, or the edit-to-playing loop. A test prints each figure
//! as a line `measure: <name> <value>` on its standard output, which nextest passes through
//! under `--no-capture`: the test needs no path from this command and stays runnable by
//! hand, and the line is the one contract between them. Every result file opens with the
//! header the plan names: the CPU and operating system, the topology the executor detected,
//! the workers used, the commit, the build profile, the command, and the load average before
//! and after the run (on Windows, the processor's idle share over it).
//!
//! Not a check: nothing is printed to standard output, progress and the measurements' own
//! output go to standard error, and times are printed and written, never compared with a
//! threshold.

use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The variable a test reads to cut its repetitions in a short run.
pub const SHORT_VAR: &str = "LACTOSE_MEASURE_SHORT";
/// The variable that names the machine in the file names, in place of the CPU's slug.
pub const MACHINE_VAR: &str = "LACTOSE_MEASURE_MACHINE";
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
        /// Its integration test target.
        target: &'static str,
        /// The target's file, relative to the repository root.
        file: &'static str,
        /// The test's path inside the target.
        test: &'static str,
    },
    /// `cargo bench -p lactose-task --bench whisk`, which writes its own pair of files.
    Whisk,
    /// `cargo run -p lactose-app-headless -- 120`, dev profile.
    Headless,
    /// The edit-to-playing loop of decision 5.9.
    EditToPlaying,
}

impl Run {
    /// Whether `--short` cuts its repetitions, and so marks its files as short. The headless
    /// run has none to cut, and edit-to-playing keeps its five so that a runner's short run
    /// still yields the criterion's figure.
    #[must_use]
    pub const fn cut_by_short(self) -> bool {
        matches!(self, Self::Test { .. } | Self::Whisk)
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

const fn test(
    package: &'static str,
    target: &'static str,
    file: &'static str,
    test: &'static str,
) -> Run {
    Run::Test { package, target, file, test }
}

/// Every measurement, in the order `--all` runs them.
pub const MEASUREMENTS: [Measurement; 11] = [
    Measurement {
        name: "tick-100k",
        what: "Criterion 1: the median step of the 100k scene on one frame worker and on N, S and the projected eight-core tick",
        threshold: "architecture section 22; milestone 1 plan, section 4 and decision 5.1",
        run: test("lactose-sim", "cost", "crates/sim/tests/cost.rs", "cost_of_a_tick_at_100k_rows"),
    },
    Measurement {
        name: "physics-phase",
        what: "Criterion 4: the sync, step and write-back of 100k rows with 2,000 awake bodies on Rapier",
        threshold: "architecture section 22; milestone 1 plan, section 4",
        run: test(
            "lactose-app",
            "cost",
            "crates/app/tests/cost.rs",
            "cost_of_the_physics_phase_at_100k_rows",
        ),
    },
    Measurement {
        name: "composed-tick",
        what: "The 100k scene with 2,000 bodies on Rapier beside it, on one frame worker and on N",
        threshold: "none: recorded beside criteria 1 and 4 (milestone 1 plan, section 4)",
        run: test(
            "lactose-app",
            "cost",
            "crates/app/tests/cost.rs",
            "composed::cost_of_the_composed_tick",
        ),
    },
    Measurement {
        name: "reload-100k",
        what: "The full diff of a 100k-row region reloaded in place on Rapier (D124)",
        threshold: "none: recorded (docs/verification.md)",
        run: test("lactose-app", "cost", "crates/app/tests/cost.rs", "cost_of_a_100k_reload"),
    },
    Measurement {
        name: "bone-bodies",
        what: "200 characters with a Rapier body on each of their sixteen bones, against none",
        threshold: "none: recorded (docs/verification.md)",
        run: test(
            "lactose-app",
            "cost",
            "crates/app/tests/cost.rs",
            "cost_of_bone_bodies_on_rapier",
        ),
    },
    Measurement {
        name: "prediction-slot",
        what: "Criterion 5: the take and the restore of a prediction slot on the spike's thrown scene with Rapier",
        threshold: "architecture section 22; milestone 1 plan, section 4",
        run: test("lactose-app", "spike", "crates/app/tests/spike.rs", "cost_of_a_prediction_slot"),
    },
    Measurement {
        name: "rollback-seven-ticks",
        what: "Criterion 7: a seven-tick rollback on the spike's thrown scene with Rapier",
        threshold: "none: recorded (architecture section 22; milestone 1 plan, section 4)",
        run: test(
            "lactose-app",
            "spike",
            "crates/app/tests/spike.rs",
            "cost_of_a_seven_tick_rollback",
        ),
    },
    Measurement {
        name: "sim-skeleton",
        what: "Criterion 6: the sim skeleton of 200 characters; figure (b) is the criterion's",
        threshold: "architecture section 22; milestone 1 plan, section 4",
        run: test("lactose-anim", "cost", "crates/anim/tests/cost.rs", "the_sim_skeleton_budget"),
    },
    Measurement {
        name: "whisk",
        what: "Milestone 0's WHISK benchmark, which writes its own pair of files",
        threshold: "crates/task/README.md",
        run: Run::Whisk,
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

/// The topology probe: an ignored test of `lactose-app` that prints what the executor
/// detects, run once before the measurements.
const PROBE: Run =
    test("lactose-app", "cost", "crates/app/tests/cost.rs", "the_topology_the_executor_detects");

// ---- The pure parts ----

/// The figures in `output`: every line `measure: <name> <value>`, in order.
#[must_use]
fn figures(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| line.trim_end().strip_prefix(FIGURE_PREFIX))
        .filter_map(|rest| rest.split_once(' '))
        .map(|(name, value)| (name.to_owned(), value.trim().to_owned()))
        .collect()
}

/// A figure's value in the JSON file: a number where it reads as one, else its text.
#[must_use]
fn json_value(value: &str) -> serde_json::Value {
    if let Ok(whole) = value.parse::<i64>() {
        return whole.into();
    }
    match value.parse::<f64>() {
        Ok(number) if number.is_finite() => number.into(),
        _ => value.into(),
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

/// The mean of the samples in `typeperf`'s CSV output, a percentage: every line of two quoted
/// fields whose second is a number. The header line and any message are not.
#[must_use]
fn idle_share(csv: &str) -> Option<f64> {
    let samples: Vec<f64> = csv
        .lines()
        .filter_map(|line| {
            let (_, value) = line.trim().split_once("\",\"")?;
            value.trim_end_matches('"').trim().parse::<f64>().ok()
        })
        .filter(|value| value.is_finite())
        .collect();
    (!samples.is_empty()).then(|| samples.iter().sum::<f64>() / samples.len() as f64)
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
    /// The line the statement follows, which occurs once in the file.
    anchor: &'static str,
}

/// The two edits of decision 5.9: the sketch game's system in the headless binary's crate,
/// and the executor's start in `lactose-app`, a library the binary depends on.
const SITES: [Site; 2] = [
    Site {
        name: "sketch_system",
        file: "crates/app-headless/src/main.rs",
        anchor: "    counter.0 = counter.0.wrapping_add(1);\n",
    },
    Site {
        name: "app_function",
        file: "crates/app/src/runtime.rs",
        anchor: "    let mut builder = Executor::builder();\n",
    },
];

/// `source` with the `n`th edit at `site`: one statement after the anchor, which differs
/// from every other edit's, so each is a one-line change from the build before it. Fails
/// unless the anchor occurs exactly once.
fn edited(source: &str, site: &Site, n: u32) -> Result<String, String> {
    let count = source.matches(site.anchor).count();
    if count != 1 {
        return Err(format!(
            "{}: the edit's anchor `{}` occurs {count} times; it must occur once",
            site.file,
            site.anchor.trim()
        ));
    }
    let statement = format!("    let _ = std::hint::black_box({n}_u32);\n");
    Ok(source.replacen(site.anchor, &format!("{}{statement}", site.anchor), 1))
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
    /// The processor's mean idle share over the run, in percent, where it could be sampled.
    Idle(Option<f64>),
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
        Load::Idle(Some(share)) => ("Processor idle", format!("{share:.1}% over the run")),
        Load::Idle(None) => ("Processor idle", "unavailable".to_owned()),
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
            load.insert("processor_idle_percent".into(), (*share).into());
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
    /// The directory the result files are written to.
    out: PathBuf,
    /// The command as given, for the header.
    command: String,
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
                out = Some(PathBuf::from(dir));
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
    let mut command = String::from("cargo xtask measure");
    for arg in args {
        command.push(' ');
        command.push_str(arg);
    }
    Ok(Options { selected, short, out: out.unwrap_or_else(|| PathBuf::from(RESULTS)), command })
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
    commit: String,
    topology: Option<(u64, u64)>,
    default_workers: Option<String>,
}

/// Runs the selected measurements, each writing its files; one that fails is reported and
/// the rest still run. Returns the files written, or what failed.
pub fn run(workspace: &Workspace, options: &Options) -> Result<Vec<PathBuf>, String> {
    let out = workspace.root.join(&options.out);
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let cpu = cpu_name();
    let mut machine = Machine {
        name: std::env::var(MACHINE_VAR)
            .ok()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| slug(&cpu)),
        os: os_name(),
        cpu,
        date: date(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())),
        commit: commit(&workspace.root),
        topology: None,
        default_workers: None,
    };
    eprintln!(
        "measure: {} ({}), machine `{}`, commit {}, {}",
        machine.cpu,
        machine.os,
        machine.name,
        machine.commit,
        if options.short { "short run" } else { "full run" }
    );
    match probe(workspace) {
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
            Run::Whisk => run_whisk(workspace, options, &machine, &out),
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

/// The CPU's model name, as the WHISK benchmark reads it.
fn cpu_name() -> String {
    let name = if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(target_os = "linux") {
        fs::read_to_string("/proc/cpuinfo").ok().and_then(|info| {
            let line = info.lines().find(|line| line.starts_with("model name"))?;
            Some(line.split_once(':')?.1.trim().to_owned())
        })
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    };
    name.filter(|name| !name.is_empty()).unwrap_or_else(|| "unknown CPU".to_owned())
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

/// The commit, with `-dirty` when a tracked file has changes.
fn commit(root: &Path) -> String {
    let root = root.to_string_lossy();
    let hash = command_output("git", &["-C", &root, "rev-parse", "--short=12", "HEAD"]);
    let dirty =
        command_output("git", &["-C", &root, "status", "--porcelain", "--untracked-files=no"])
            .is_some_and(|status| !status.is_empty());
    match hash {
        Some(hash) if dirty => format!("{hash}-dirty"),
        Some(hash) => hash,
        None => "unknown".to_owned(),
    }
}

/// What the machine is doing, sampled across one run.
enum Sampler {
    Uptime(String),
    /// `typeperf`, sampling the idle share every second, and the thread reading it.
    Typeperf(Child, thread::JoinHandle<String>),
    Unavailable,
}

impl Sampler {
    fn start() -> Self {
        if cfg!(windows) {
            let spawned = Command::new("typeperf")
                .args([r"\Processor(_Total)\% Idle Time", "-si", "1"])
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
                        Self::Typeperf(child, reader)
                    }
                    None => Self::Unavailable,
                },
                Err(_) => Self::Unavailable,
            }
        } else {
            Self::Uptime(uptime())
        }
    }

    fn stop(self) -> Load {
        match self {
            Self::Uptime(before) => Load::Averages { before, after: uptime() },
            Self::Typeperf(mut child, reader) => {
                let _ = child.kill();
                let _ = child.wait();
                Load::Idle(reader.join().ok().as_deref().and_then(idle_share))
            }
            Self::Unavailable => Load::Idle(None),
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
    let Run::Test { package, target, test, .. } = run else { return Vec::new() };
    [
        "nextest",
        "run",
        "--locked",
        "--cargo-profile",
        "release",
        "--package",
        package,
        "--test",
        target,
        "--run-ignored",
        "only",
        "--no-capture",
        "--no-tests=fail",
        "-E",
        &format!("test(={test})"),
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect()
}

fn probe(workspace: &Workspace) -> Result<Vec<(String, String)>, String> {
    let mut command = Command::new(&workspace.cargo);
    command.args(nextest_args(PROBE)).current_dir(&workspace.root);
    Ok(figures(&capture(&mut command, |_, _| {})?))
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
        command: options.command.clone(),
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
    let env = short_env(options);
    let mut command = Command::new(&workspace.cargo);
    command.args(&args).current_dir(&workspace.root);
    // A short run sets the variable; a full one makes sure the caller's does not leak in.
    command.env_remove(SHORT_VAR).envs(env.iter().map(|(name, value)| (name, value)));
    let sampler = Sampler::start();
    let output = capture(&mut command, |_, _| {});
    let load = sampler.stop();
    let output = output?;
    let found = figures(&output);
    let workers = found
        .iter()
        .find(|(name, _)| name == "workers" || name == "frame_workers")
        .map_or_else(|| default_workers(machine), |(_, value)| value.clone());
    let ran = shell_line(&env, "cargo", &args);
    let header = header(options, machine, m, "release", workers, ran, load);
    write(out, &header, &found, &output)
}

/// The WHISK benchmark's own pair, written into a directory under the target directory and
/// moved into `out`; a short run's carry `-short` and say so at the top of the summary.
fn run_whisk(
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
    let mut command = Command::new(&workspace.cargo);
    command
        .args(&args)
        .current_dir(&workspace.root)
        .env("WHISK_BENCH_OUT", &scratch)
        .env("WHISK_BENCH_MACHINE", &machine.name);
    if options.short {
        command.env("WHISK_BENCH_RUNS", "100");
    }
    capture(&mut command, |_, _| {})?;
    let mut moved = Vec::new();
    let entries = fs::read_dir(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    for entry in entries {
        let from = entry.map_err(|e| e.to_string())?.path();
        let (Some(stem), Some(extension)) = (from.file_stem(), from.extension()) else { continue };
        let (stem, extension) = (stem.to_string_lossy(), extension.to_string_lossy());
        let suffix = if options.short { "-short" } else { "" };
        let to = out.join(format!("{stem}{suffix}.{extension}"));
        let mut text = fs::read_to_string(&from).map_err(|e| format!("{}: {e}", from.display()))?;
        if options.short && extension == "md" {
            text = format!(
                "> A short run of `{}` (`WHISK_BENCH_RUNS=100`): its numbers mean nothing.\n\n{text}",
                options.command
            );
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
/// touched and no registration outlives a run that ends normally.
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
        let path = copy.tree.to_string_lossy().into_owned();
        // What a killed run left behind.
        if copy.tree.exists() {
            let _ = copy.git(&["worktree", "remove", "--force", &path]);
            let _ = fs::remove_dir_all(&copy.tree);
        }
        copy.git(&["worktree", "prune"])?;
        copy.git(&["worktree", "add", "--detach", &path, "HEAD"])?;
        Ok(copy)
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let path = self.tree.to_string_lossy().into_owned();
        if self.git(&["worktree", "remove", "--force", &path]).is_err() {
            eprintln!("measure: could not remove the copy at {path}; `git worktree prune` will");
        }
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
    let ran = format!(
        "{} in a detached worktree of `HEAD` at `{COPY_DIR}/tree` under the target directory: \
         one warm build before each site, then {EDITS_PER_SITE} one-line edits there, each timed \
         from its save to the report line",
        shell_line(&[], "cargo", &args),
    );
    let mut header = header(options, machine, m, "dev", default_workers(machine), ran, load);
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
                      measure:missing\nmeasure: hash 0x00ab\r\ntest ... ok\n";
        assert_eq!(
            figures(output),
            [
                ("t1_us".to_owned(), "1234.5".to_owned()),
                ("workers".to_owned(), "1 and 4".to_owned()),
                ("hash".to_owned(), "0x00ab".to_owned()),
            ]
        );
    }

    #[test]
    fn a_figure_is_a_number_in_the_json_where_it_reads_as_one() {
        assert_eq!(json_value("100064"), serde_json::json!(100_064));
        assert_eq!(json_value("28043.3"), serde_json::json!(28_043.3));
        assert_eq!(json_value("0x00ab"), serde_json::json!("0x00ab"));
        assert_eq!(json_value("1 and 4"), serde_json::json!("1 and 4"));
        assert_eq!(json_value("NaN"), serde_json::json!("NaN"));
    }

    #[test]
    fn the_machine_slug_is_whisks() {
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
    fn the_idle_share_is_the_mean_of_typeperf_s_samples() {
        let csv = "\r\n\"(PDH-CSV 4.0)\",\"\\\\HOST\\Processor(_Total)\\% Idle Time\"\r\n\
                   \"11/15/2026 05:41:02.123\",\"90.0\"\r\n\"11/15/2026 05:41:03.123\",\"80.0\"\r\n\
                   Exiting, please wait...\r\n";
        assert_eq!(idle_share(csv), Some(85.0));
        assert_eq!(idle_share("\"(PDH-CSV 4.0)\",\"x\"\n"), None);
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
        let main = repository(SITES[0].file);
        let body = main.split("fn count(").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(body.contains(SITES[0].anchor.trim()), "the sketch system's body");
        let runtime = repository(SITES[1].file);
        let body = runtime.split("pub fn start_executor(").nth(1).unwrap().split("\n}\n").next();
        assert!(body.unwrap().contains(SITES[1].anchor.trim()), "a lactose-app function's body");
        let twice = format!("{0}{0}", SITES[0].anchor);
        assert!(edited(&twice, &SITES[0], 1).is_err());
    }

    /// Every test a measurement names is in its file, ignored, and every measurement is
    /// listed in the results README with its command.
    #[test]
    fn every_measurement_names_a_test_that_exists_and_is_listed() {
        let readme = repository("docs/results/README.md");
        let mut names = std::collections::BTreeSet::new();
        for m in MEASUREMENTS.iter().chain([&Measurement {
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
            if m.name != "probe" {
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
        assert_eq!(all.command, "cargo xtask measure --all --short");
        let two = parse(&["tick-100k", "whisk", "tick-100k", "--out", "elsewhere"]).unwrap();
        let names: Vec<&str> = two.selected.iter().map(|m| m.name).collect();
        assert_eq!(names, ["tick-100k", "whisk"]);
        assert_eq!(two.out, PathBuf::from("elsewhere"));
        assert!(parse(&[]).is_err());
        assert!(parse(&["--all", "whisk"]).is_err());
        assert!(parse(&["nothing"]).is_err());
        assert!(parse(&["--out"]).is_err());
    }

    #[test]
    fn a_result_file_opens_with_the_header() {
        let header = Header {
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
        };
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
        let windows = Header { load: Load::Idle(Some(93.25)), ..header };
        assert!(markdown(&windows, &[], "").contains("| Processor idle | 93.2% over the run |"));
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
