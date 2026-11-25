//! `cargo xtask measure`: the measurements the milestone criteria rest on, each written to a
//! result file in `docs/results` (architecture section 20; milestone 1 plan, section 4 and
//! decision 5.11).
//!
//! A measurement is an ignored release test that prints its figures, the netcode spike's
//! ten-minute replays, the WHISK benchmark, the headless binary's 120-tick run, or the
//! edit-to-playing loop. A test on a Vulkan device runs under the driver `--driver` names with
//! no layer (`vulkan::Run::measured`), and its file's header adds the device's identity as
//! `vulkaninfo` reports it under the same environment. A test prints each figure
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

use crate::{pace, vulkan};

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
    /// An ignored test on a Vulkan device, run in release as [`Test`](Self::Test) is, under
    /// the environment of [`vulkan::Run::measured`]; `--no-device` leaves it out.
    Device {
        /// The package.
        package: &'static str,
        /// Its integration test file, as [`Test`](Self::Test)'s.
        file: &'static str,
        /// The test's path inside the file's module.
        test: &'static str,
    },
    /// An ignored test on a Vulkan device, run as [`Device`](Self::Device) is, once under each
    /// of [`Options::drivers`], each run with the engine's shaders cooked and a source salted
    /// for it ([`salted_source`]) cooked before its window opens: `gpu-cost`.
    GpuCost {
        /// The package.
        package: &'static str,
        /// Its integration test file, as [`Test`](Self::Test)'s.
        file: &'static str,
        /// The test's path inside the file's module.
        test: &'static str,
    },
    /// Criterion 1 (`triangle`): the client binary's fixed-frame run, in release, under every
    /// validator the host's pinned layer has, and each validator's canary in a process of its
    /// own beside it, once under each of [`Options::drivers`], judged by decision 5.10's rule
    /// ([`vulkan::judge`]).
    Triangle,
    /// `frame-pace`: the client binary's paced run with its stamps, in release, no validator
    /// on, once under each of [`Options::drivers`]: at real time, then, where the swapchain
    /// reports display times, the floor run, made again from the fitted refresh where it
    /// swept fewer than twice, and on Windows the run again minimised ([`pace`]).
    FramePace,
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
    /// as short. The headless run has none to cut, and edit-to-playing keeps its five and
    /// `triangle` its 600 frames so that a short run still yields the criterion's proof.
    #[must_use]
    pub const fn cut_by_short(self) -> bool {
        matches!(
            self,
            Self::Test { .. }
                | Self::Device { .. }
                | Self::GpuCost { .. }
                | Self::FramePace
                | Self::SpikeReplays
                | Self::TaskBench
        )
    }

    /// Whether it needs a Vulkan device.
    #[must_use]
    pub const fn on_device(self) -> bool {
        matches!(
            self,
            Self::Device { .. } | Self::GpuCost { .. } | Self::Triangle | Self::FramePace
        )
    }

    /// The package, file and test of an ignored test, on a device or not.
    const fn ignored_test(self) -> Option<(&'static str, &'static str, &'static str)> {
        match self {
            Self::Test { package, file, test }
            | Self::Device { package, file, test }
            | Self::GpuCost { package, file, test } => Some((package, file, test)),
            _ => None,
        }
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

const fn device(package: &'static str, file: &'static str, test: &'static str) -> Run {
    Run::Device { package, file, test }
}

/// Every measurement, in the order `--all` runs them.
pub const MEASUREMENTS: [Measurement; 18] = [
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
    Measurement {
        name: "extract-50k",
        what: "Milestone 2's criterion 4: extract's pass over 100k rows with 50,000 moved, writing the device's ring and a snapshot of 10,000 presented rows, each alone, on one worker, and again on null",
        threshold: "1 ms for the median on the device (milestone 2 plan, section 4, criterion 4; verification row 16)",
        run: device(
            "lactose-app-client",
            "crates/app-client/tests/extract.rs",
            "cost_of_extract_at_50k_dirty_rows",
        ),
    },
    Measurement {
        name: "remote-upload-5k",
        what: "Milestone 2's criterion 5: the remote upload of 5,000 interpolated entities into the device's ring, paced at 144 Hz, one writer pushing every slot at 30 Hz",
        threshold: "1 ms for the median of the paced frames that began and ended on a performance core, every frame on a machine with one class of core, with their share, and no verdict with under a tenth of them; their P99 over 2 ms a finding (milestone 2 plan, section 4, criterion 5; D164; verification rows 19 and 102)",
        run: device(
            "lactose-app-client",
            "crates/app-client/tests/upload.rs",
            "cost_of_the_remote_upload_of_5k",
        ),
    },
    Measurement {
        name: "record-stall",
        what: "A frame's record of the minimal renderer on null beside the composed tick's physics phase, at a physics reservation of one frame worker and of two, and alone",
        threshold: "none: recorded, and `RECORD_RESERVATION` is read from it (milestone 2 plan, section 4 and decision 5.16; verification row 21; D162)",
        run: test(
            "lactose-app-client",
            "crates/app-client/tests/stall.rs",
            "the_record_beside_the_physics_phase",
        ),
    },
    Measurement {
        name: "triangle",
        what: "Milestone 2's criterion 1: the client binary's fixed-frame run of 600 frames and seven instances under every validator the host's pinned layer has, the last frame's offscreen target read back and held to the CPU reference, and each validator's canary in a process of its own beside it",
        threshold: "no pixel outside decision 5.1's margin wrong, each canary heard, no other message and nothing at warn or above (milestone 2 plan, section 4, criterion 1; decision 5.10)",
        run: Run::Triangle,
    },
    Measurement {
        name: "gpu-cost",
        what: "Ten thousand indexed draws as draws, as one multi-draw and as one indirect draw over a compacted index buffer, record and submit apart with each one's thread and the render pass's GPU time by timestamps; 96 pipelines source-cold, specialisation-cold and from a reloaded cache, with the hit bits",
        threshold: "none: recorded (milestone 2 plan, section 4; verification rows 15 and 34, and the plan's decision 5.7)",
        run: Run::GpuCost {
            package: "lactose-app-client",
            file: "crates/app-client/tests/gpu.rs",
            test: "cost_of_draws_and_pipelines",
        },
    },
    Measurement {
        name: "frame-pace",
        what: "The client binary shown and paced by its timer root, each frame and each present call stamped: the lateness of each frame's first tick, the record, its width and longest job, the submit and the hold, the present call's parts, `waiting`, and the latch's age at the display against the floor swept across the refresh",
        threshold: "none: recorded; the lateness read against D144's 1 ms on Windows and D156's 50 us on macOS, and D162's four readings (milestone 2 plan, section 4, row `frame-pace`)",
        run: Run::FramePace,
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

/// One physical device as `vulkaninfo`'s text output prints it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Gpu {
    /// Each `key = value` line of its sections, the first of each key.
    properties: Vec<(String, String)>,
    /// Each memory type's property flags, in the device's order.
    memory_types: Vec<u32>,
}

impl Gpu {
    /// The value of `key`, its parenthesised raw form dropped (`1.4.357 (4211045)`).
    fn get(&self, key: &str) -> Option<&str> {
        let (_, value) = self.properties.iter().find(|(name, _)| name == key)?;
        Some(value.split(" (").next().unwrap_or(value))
    }
}

/// What `vulkaninfo` reports: the instance's version and every device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct VulkanInfo {
    instance: Option<String>,
    gpus: Vec<Gpu>,
}

/// Reads `vulkaninfo`'s full text output: the instance version's line, and after each
/// `GPU<n>:` line that device's properties and its memory types' `propertyFlags`.
fn vulkaninfo(text: &str) -> VulkanInfo {
    let mut info = VulkanInfo::default();
    let is_gpu = |line: &str| {
        line.strip_prefix("GPU")
            .and_then(|rest| rest.strip_suffix(':'))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(version) = line.strip_prefix("Vulkan Instance Version:") {
            info.instance = Some(version.trim().to_owned());
        } else if is_gpu(line) {
            info.gpus.push(Gpu::default());
        } else if let Some(gpu) = info.gpus.last_mut()
            && let Some((key, value)) = line.split_once(" = ")
        {
            let (key, value) = (key.trim(), value.trim());
            if key == "propertyFlags" {
                let hex = value.split(':').next().unwrap_or(value).trim_start_matches("0x");
                gpu.memory_types.push(u32::from_str_radix(hex, 16).unwrap_or(0));
            } else if !gpu.properties.iter().any(|(name, _)| name == key) {
                gpu.properties.push((key.to_owned(), value.to_owned()));
            }
        }
    }
    info
}

/// `VkMemoryPropertyFlagBits` by bit, as the header names them.
const MEMORY_FLAGS: [(u32, &str); 9] = [
    (0x1, "DEVICE_LOCAL"),
    (0x2, "HOST_VISIBLE"),
    (0x4, "HOST_COHERENT"),
    (0x8, "HOST_CACHED"),
    (0x10, "LAZILY_ALLOCATED"),
    (0x20, "PROTECTED"),
    (0x40, "DEVICE_COHERENT_AMD"),
    (0x80, "DEVICE_UNCACHED_AMD"),
    (0x100, "RDMA_CAPABLE_NV"),
];

/// `flags` by name, then in hexadecimal.
fn memory_flags(flags: u32) -> String {
    let names: Vec<&str> =
        MEMORY_FLAGS.iter().filter(|(bit, _)| flags & bit != 0).map(|(_, name)| *name).collect();
    let names = if names.is_empty() { "none".to_owned() } else { names.join(" + ") };
    format!("{names} (0x{flags:04x})")
}

/// The memory type `lactose-rhi-vulkan` gives `MemoryClass::Upload` among `types`, by its
/// rule (`raw.rs`, `choose`): the first host-visible and coherent type, then the first
/// host-visible one, never a lazily allocated one. A buffer's own memory-type bits may rule
/// some out, which `vulkaninfo` does not show.
fn upload_type(types: &[u32]) -> Option<usize> {
    const VISIBLE: u32 = 0x2;
    const COHERENT: u32 = 0x4;
    const LAZY: u32 = 0x10;
    [VISIBLE | COHERENT, VISIBLE]
        .iter()
        .find_map(|&has| types.iter().position(|&t| t & has == has && t & LAZY == 0))
}

/// `value` with a path inside `sdk` written from `<sdk>`, and any other absolute path cut to
/// its file name, so that a result file names no directory of the machine.
fn redacted(value: &str, sdk: Option<&Path>) -> String {
    let path = Path::new(value);
    if let Some(inside) = sdk.and_then(|sdk| path.strip_prefix(sdk).ok()) {
        return format!("<sdk>/{}", inside.to_string_lossy().replace('\\', "/"));
    }
    match path.file_name() {
        Some(name) if path.is_absolute() => format!(".../{}", name.to_string_lossy()),
        _ => value.to_owned(),
    }
}

/// How the measurements on a device run, for their headers.
#[derive(Clone, Debug)]
struct DeviceSetting {
    /// `--driver`'s name.
    driver: &'static str,
    /// The pinned SDK's version where its loader is the one loaded, `None` for the system's.
    sdk_version: Option<String>,
    /// Every variable the run sets, paths redacted.
    environment: Vec<(String, String)>,
    /// What `vulkaninfo` reported under the same environment, or why it reported nothing.
    info: Result<VulkanInfo, String>,
}

/// The header's rows for a test on a device that named its adapter `adapter`.
fn device_notes(setting: &DeviceSetting, adapter: Option<&str>) -> Vec<(String, String)> {
    let mut notes = Vec::new();
    let mut note = |name: &str, value: String| notes.push((name.to_owned(), value));
    let gpu = match (&setting.info, adapter) {
        (Err(why), _) => Err(format!("unreported: {why}")),
        (Ok(_), None) => Err("unreported: the test printed no `device.adapter`".to_owned()),
        (Ok(info), Some(adapter)) => {
            info.gpus.iter().find(|gpu| gpu.get("deviceName") == Some(adapter)).ok_or_else(|| {
                format!("unreported: `vulkaninfo` lists no device named `{adapter}`")
            })
        }
    };
    let field = |gpu: &Gpu, key: &str| gpu.get(key).unwrap_or("unreported").to_owned();
    match &gpu {
        Ok(gpu) => {
            let (name, vendor, id) =
                (field(gpu, "deviceName"), field(gpu, "vendorID"), field(gpu, "deviceID"));
            note("Device", format!("{name}, vendor {vendor}, device {id}"));
            let (driver, info) = (field(gpu, "driverName"), field(gpu, "driverInfo"));
            let version = field(gpu, "driverVersion");
            let flag = setting.driver;
            note("Driver", format!("{driver} {info}, driver version {version}, `--driver {flag}`"));
            let instance = setting.info.as_ref().ok().and_then(|info| info.instance.as_deref());
            note(
                "Vulkan",
                format!(
                    "the device's API {}, the instance's {}",
                    field(gpu, "apiVersion"),
                    instance.unwrap_or("unreported")
                ),
            );
        }
        Err(why) => note("Device", format!("{why}; the test's `device.*` figures name it")),
    }
    let loader = setting
        .sdk_version
        .as_ref()
        .map_or_else(|| "the system's".to_owned(), |version| format!("the Vulkan SDK {version}'s"));
    note("Loader", loader);
    note(
        "Validation",
        "off: no layer enabled and every implicit one disabled \
         (`VK_LOADER_LAYERS_DISABLE=~implicit~`), and no Metal API or shader validation"
            .to_owned(),
    );
    note(
        "Presentation",
        "none: the run makes no surface and presents nothing, so it has no present mode and \
         reads no display's refresh rate"
            .to_owned(),
    );
    if let Ok(gpu) = &gpu {
        let ring = upload_type(&gpu.memory_types).map_or_else(
            || "no host-visible type is listed".to_owned(),
            |at| {
                format!(
                    "type {at} of {}, {}: the first `lactose-rhi-vulkan` chooses for \
                     `MemoryClass::Upload`, the buffer's own memory-type bits unreported",
                    gpu.memory_types.len(),
                    memory_flags(gpu.memory_types[at])
                )
            },
        );
        note("Ring memory", ring);
    }
    let environment: Vec<String> =
        setting.environment.iter().map(|(name, value)| format!("`{name}={value}`")).collect();
    note("Environment", environment.join(", "));
    notes
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
    /// What the files of a measurement that writes several are named for beside it: the
    /// driver's flag of one run under each driver.
    variant: Option<&'static str>,
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
    let under = header.variant.map_or_else(String::new, |variant| format!(" under {variant}"));
    let _ = writeln!(text, "# {}{under}, {}, {}{short}\n", m.name, header.machine, header.date);
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
    /// `--driver`: the driver of the measurements on a device, or the host's default.
    driver: Option<vulkan::Driver>,
    /// `--adapter`: the device's name, or the loader's first.
    adapter: Option<String>,
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

    /// The driver of the measurements on a device: `--driver`'s, or MoltenVK on macOS and the
    /// system's elsewhere.
    fn driver(&self) -> vulkan::Driver {
        self.driver.unwrap_or(if cfg!(target_os = "macos") {
            vulkan::Driver::MoltenVk
        } else {
            vulkan::Driver::System
        })
    }

    /// The drivers of a measurement on a device that runs under each of the host's:
    /// `--driver`'s alone, or MoltenVK and KosmicKrisp on macOS and the system's elsewhere.
    fn drivers(&self) -> Vec<vulkan::Driver> {
        match self.driver {
            Some(driver) => vec![driver],
            None if cfg!(target_os = "macos") => vulkan::MACOS_GATE.to_vec(),
            None => vec![vulkan::Driver::System],
        }
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

/// Parses the arguments after `measure`: `--all` or names, `--short`, `--no-device` (which
/// leaves out of `--all` the measurements on a device, and refuses one named), `--driver
/// <name>`, `--adapter <name>` and `--out <dir>`.
pub fn parse(args: &[&str]) -> Result<Options, String> {
    let names = || MEASUREMENTS.iter().map(|m| m.name).collect::<Vec<_>>().join(", ");
    let (mut selected, mut all, mut short, mut out) = (Vec::new(), false, false, None);
    let (mut no_device, mut driver, mut adapter) = (false, None, None);
    let mut rest = args;
    while let [first, tail @ ..] = rest {
        rest = tail;
        match *first {
            "--all" => all = true,
            "--short" => short = true,
            "--no-device" => no_device = true,
            "--out" => {
                let [dir, tail @ ..] = rest else { return Err("`--out` takes a directory".into()) };
                if out.replace(PathBuf::from(dir)).is_some() {
                    return Err("`--out` is given once".into());
                }
                rest = tail;
            }
            "--driver" => {
                let [name, tail @ ..] = rest else { return Err("`--driver` takes a name".into()) };
                let given = vulkan::Driver::of(name).ok_or_else(|| {
                    format!("no driver is named `{name}`: lavapipe, moltenvk, kosmickrisp, system")
                })?;
                if driver.replace(given).is_some() {
                    return Err("`--driver` is given once".into());
                }
                rest = tail;
            }
            "--adapter" => {
                let [name, tail @ ..] = rest else { return Err("`--adapter` takes a name".into()) };
                if adapter.replace((*name).to_owned()).is_some() {
                    return Err("`--adapter` is given once".into());
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
        selected = MEASUREMENTS.iter().filter(|m| !(no_device && m.run.on_device())).collect();
    } else if let Some(m) = selected.iter().find(|m| no_device && m.run.on_device()) {
        return Err(format!("`--no-device` leaves out `{}`, which runs on a device", m.name));
    }
    if (driver.is_some() || adapter.is_some()) && !selected.iter().any(|m| m.run.on_device()) {
        return Err("`--driver` and `--adapter` name the device, and no measurement on one \
                    is selected"
            .into());
    }
    Ok(Options {
        selected,
        short,
        driver,
        adapter,
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
    // Each driver's, made before the first measurement under it; its failure fails each of
    // them.
    let mut devices = Devices::default();
    for m in &options.selected {
        eprintln!("measure: {} ...", m.name);
        let result = match m.run {
            Run::Test { .. } => run_test(workspace, options, &machine, m, &out, None),
            Run::Device { .. } => match devices.get(workspace, options, options.driver()) {
                Ok(device) => run_test(workspace, options, &machine, m, &out, Some(device)),
                Err(error) => Err(error),
            },
            Run::GpuCost { .. } => {
                run_gpu_cost(workspace, options, &machine, m, &out, &mut devices)
            }
            Run::Triangle => run_triangle(workspace, options, &machine, m, &out, &mut devices),
            Run::FramePace => run_frame_pace(workspace, options, &machine, m, &out, &mut devices),
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
    let Some((package, file, test)) = run.ignored_test() else { return Vec::new() };
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
        variant: None,
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

/// The stem of `header`'s files: [`file_stem`]'s, the measurement's name followed by its
/// variant where it has one.
fn header_stem(header: &Header) -> String {
    let name = match header.variant {
        Some(variant) => format!("{}-{variant}", header.measurement.name),
        None => header.measurement.name.to_owned(),
    };
    file_stem(&name, &header.machine, &header.date, header.short)
}

fn write(
    out: &Path,
    header: &Header,
    figures: &[(String, String)],
    output: &str,
) -> Result<Vec<PathBuf>, String> {
    let stem = header_stem(header);
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

/// The environment of the measurements on a device and what their headers say of it.
struct OnDevice {
    run: vulkan::Run,
    setting: DeviceSetting,
}

/// How long `vulkaninfo` may run.
const VULKANINFO_DEADLINE: Duration = Duration::from_secs(60);

/// What the runs on a device share, each made once a run: each driver's [`OnDevice`], and the
/// engine's shaders cooked with the pinned Slang.
#[derive(Default)]
struct Devices {
    made: Vec<(vulkan::Driver, Result<OnDevice, String>)>,
    shaders: Option<Result<PathBuf, String>>,
}

impl Devices {
    fn get(
        &mut self,
        workspace: &Workspace,
        options: &Options,
        driver: vulkan::Driver,
    ) -> Result<&OnDevice, String> {
        let at = match self.made.iter().position(|(made, _)| *made == driver) {
            Some(at) => at,
            None => {
                self.made.push((driver, on_device(workspace, options, driver)));
                self.made.len() - 1
            }
        };
        self.made[at].1.as_ref().map_err(Clone::clone)
    }

    /// The directory of the engine's whole cooked shaders, as `cargo xtask gpu` gives its
    /// tests: `shaders` under `target/measure/shaders`.
    fn shaders(&mut self, workspace: &Workspace) -> Result<PathBuf, String> {
        self.shaders.get_or_insert_with(|| cook_engine_shaders(workspace)).clone()
    }
}

/// The environment of `driver`, the pinned SDK's loader where the driver is the SDK's or the
/// host has no loader of its own (macOS), and `vulkaninfo`'s report under it: the SDK's copy,
/// or the one on the path with the system's loader.
fn on_device(
    workspace: &Workspace,
    options: &Options,
    driver: vulkan::Driver,
) -> Result<OnDevice, String> {
    let sdk = if driver == vulkan::Driver::System && !cfg!(target_os = "macos") {
        None
    } else {
        let file = workspace.root.join(vulkan::PIN_FILE);
        let pin = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let pin = vulkan::Pin::parse(&pin)?;
        let sdk = vulkan::sdk(&pin, &workspace.target_dir)?;
        Some((pin.version, sdk))
    };
    let run = vulkan::Run::measured(
        driver,
        sdk.as_ref().map(|(_, sdk)| sdk),
        options.adapter.as_deref(),
    )?;
    let root = sdk.as_ref().and_then(|(_, sdk)| sdk.loader.parent());
    let program =
        root.map_or_else(|| PathBuf::from("vulkaninfo"), |root| root.join("bin/vulkaninfo"));
    let mut command = Command::new(&program);
    run.apply(&mut command).stdin(Stdio::null());
    let info = match vulkan::run_bounded(&mut command, VULKANINFO_DEADLINE) {
        Err(e) => Err(format!("`vulkaninfo` did not run: {e}")),
        Ok(ended) if ended.killed_at.is_some() => {
            Err(format!("`vulkaninfo` ran past {VULKANINFO_DEADLINE:?}"))
        }
        Ok(ended) if !ended.succeeded => Err(format!("`vulkaninfo` ended with {}", ended.status)),
        Ok(ended) => Ok(vulkaninfo(&ended.stdout)),
    };
    if let Err(why) = &info {
        eprintln!("measure: the device's identity is {why}");
    }
    let environment =
        run.set.iter().map(|(name, value)| (name.clone(), redacted(value, root))).collect();
    let setting = DeviceSetting {
        driver: driver.name(),
        sdk_version: sdk.map(|(version, _)| version),
        environment,
        info,
    };
    Ok(OnDevice { run, setting })
}

fn run_test(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
    device: Option<&OnDevice>,
) -> Result<Vec<PathBuf>, String> {
    run_test_with(workspace, options, machine, m, out, device, None, &[])
}

/// [`run_test`], its files named for `variant` where there is one, and the test given the
/// directories `dirs` by variable, each as its absolute path, which the header's command
/// shows relative to the repository root.
#[expect(clippy::too_many_arguments, reason = "run_test's, and the two a variant adds")]
fn run_test_with(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
    device: Option<&OnDevice>,
    variant: Option<&'static str>,
    dirs: &[(&'static str, PathBuf)],
) -> Result<Vec<PathBuf>, String> {
    let args = nextest_args(m.run);
    build(workspace, &no_run(&args))?;
    let env = short_env(options);
    let mut command = Command::new(&workspace.cargo);
    command.args(&args).current_dir(&workspace.root);
    // The device's environment first: it clears what it does not set.
    if let Some(device) = device {
        device.run.apply(&mut command);
    }
    // A short run sets the variable; a full one makes sure the caller's does not leak in.
    command.env_remove(SHORT_VAR).envs(env.iter().map(|(name, value)| (name, value)));
    // A test runs in its package's directory, so a directory is given whole.
    command.envs(dirs.iter().map(|(name, dir)| (name, dir)));
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
    shown.extend(dirs.iter().map(|(name, dir)| (*name, shown_path(&workspace.root, dir))));
    if let Some(n) = machine.workers_override {
        shown.push((WORKERS_VAR, n.to_string()));
    }
    let ran = shell_line(&shown, "cargo", &args);
    let mut header = header(options, machine, m, "release", workers, ran, load);
    header.variant = variant;
    if let Some(n) = machine.workers_override {
        header.notes.push((
            "Workers override".to_owned(),
            format!("`{WORKERS_VAR}={n}`: N, in place of the topology's"),
        ));
    }
    if let Some(device) = device {
        let adapter = found.iter().find(|(name, _)| name == "device.adapter");
        let adapter = adapter.map(|(_, value)| value.as_str());
        header.notes.extend(device_notes(&device.setting, adapter));
    }
    write(out, &header, &found, &output)
}

/// The variable naming the directory of the engine's cooked shaders for a test on a device, as
/// `cargo xtask gpu` sets it.
const SHADERS_VAR: &str = "LACTOSE_GPU_SHADERS";
/// The variable naming the directory of `gpu-cost`'s cooked salted source and its salt.
const SALTED_VAR: &str = "LACTOSE_MEASURE_SALTED";
/// The pipelines `gpu-cost` makes from each source, the salted one's entry pairs among them.
const COST_PIPELINES: u32 = 96;
/// The engine's shader sources, relative to the repository root.
const SHADER_SOURCES: &str = "shaders";

/// The source `gpu-cost` makes its source-cold pipelines from, `salted.slang`: `pipelines`
/// pairs of entry points, `vs_NN` and `fs_NN`, each folding the link-time constant `SALT`
/// with its own number into a literal, so that each entry's SPIR-V and MSL differ from every
/// other entry's and, under another salt, from every earlier run's. The vertex stage reads
/// `shaders/measure/cost.slang`'s push block.
#[must_use]
pub fn salted_source(pipelines: u32) -> String {
    let mut text = String::from(
        "// Written by `cargo xtask measure gpu-cost` for one run, never checked in: its \
         source-cold\n// pipelines, each entry pair folding the salted link-time constant \
         into its own literal.\n\nextern static const uint SALT;\n\nstruct Push\n{\n    \
         float4* positions;\n};\n\n[[vk::push_constant]] ConstantBuffer<Push> push;\n\n\
         struct Varyings\n{\n    float4 position : SV_Position;\n};\n\nstruct Colour\n{\n    \
         [[vk::location(0)]] float4 colour : SV_Target0;\n};\n",
    );
    for at in 0..pipelines {
        let _ = write!(
            text,
            "\n[shader(\"vertex\")]\nVaryings vs_{at:02}(uint vertex : SV_VulkanVertexID)\n{{\n    \
             Varyings result;\n    result.position = push.positions[vertex];\n    \
             result.position.z = float((SALT + {at}u) % 16777216u) / 33554432.0;\n    \
             return result;\n}}\n\n[shader(\"fragment\")]\nColour fs_{at:02}()\n{{\n    \
             Colour result;\n    result.colour = float4(float((SALT + {at}u) % 16777216u) / \
             16777216.0, 0.5, 0.25, 1.0);\n    return result;\n}}\n"
        );
    }
    text
}

/// `salted.slang`'s link module, which gives `SALT` its value.
#[must_use]
pub fn salted_link(salt: u32) -> String {
    format!(
        "// Written by `cargo xtask measure gpu-cost` for one run: the salt of its source-cold \
         pipelines.\nmodule salted_link;\n\nexport static const uint SALT = {salt}u;\n"
    )
}

/// A path for a header: relative to the repository root where it is inside it, else cut to
/// its file name, so that no file names a path of this machine.
fn shown_path(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(inside) => inside.to_string_lossy().replace('\\', "/"),
        Err(_) => redacted(&path.to_string_lossy(), None),
    }
}

/// The pinned Slang compiler, installed under the target directory where it is not.
fn slang_compiler(workspace: &Workspace) -> Result<PathBuf, String> {
    let pin = crate::slang::Pin::parse(&read_file(&workspace.root.join(crate::slang::PIN_FILE))?)?;
    crate::slang::install(&pin, &workspace.target_dir)
}

/// The engine's shaders cooked with the pinned Slang under `target/measure/shaders`, as
/// `cargo xtask gpu` cooks them: the whole cooked shaders' directory.
fn cook_engine_shaders(workspace: &Workspace) -> Result<PathBuf, String> {
    let slangc = slang_compiler(workspace)?;
    let dir = workspace.target_dir.join("measure").join("shaders");
    eprintln!("measure: cooking {SHADER_SOURCES}");
    let sources = workspace.root.join(SHADER_SOURCES);
    crate::slang::cook(&workspace.cargo, &workspace.root, &slangc, &sources, &dir)?;
    Ok(dir.join("shaders"))
}

/// `gpu-cost`: the engine's shaders cooked once, then under each driver a source salted for
/// that run cooked and the test run, each driver's pair of files named for it. A salt is the
/// clock's nanoseconds, folded to 24 bits, as the source folds it; the cook and the build
/// are before each window opens.
fn run_gpu_cost(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
    devices: &mut Devices,
) -> Result<Vec<PathBuf>, String> {
    let shaders = devices.shaders(workspace)?;
    let slangc = slang_compiler(workspace)?;
    let dir = workspace.target_dir.join("measure").join("gpu-cost");
    let mut written = Vec::new();
    for driver in options.drivers() {
        let device = devices.get(workspace, options, driver)?;
        let salt = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos() ^ (since.as_secs() as u32))
            % (1 << 24);
        let salted = dir.join(format!("salted-{}", driver.name()));
        let source = salted.join("src");
        let _ = fs::remove_dir_all(&salted);
        fs::create_dir_all(&source).map_err(|e| format!("{}: {e}", source.display()))?;
        for (name, text) in [
            ("salted.slang", salted_source(COST_PIPELINES)),
            ("salted.link.slang", salted_link(salt)),
        ] {
            let file = source.join(name);
            fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
        }
        eprintln!("measure: cooking the source salted {salt} for {}", driver.name());
        crate::slang::cook(&workspace.cargo, &workspace.root, &slangc, &source, &salted)?;
        // The whole cooked shaders, as `cargo xtask gpu` gives the tests, not the code alone.
        let cooked = salted.join("shaders");
        let file = cooked.join("salt");
        fs::write(&file, salt.to_string()).map_err(|e| format!("{}: {e}", file.display()))?;
        let extra = [(SHADERS_VAR, shaders.clone()), (SALTED_VAR, cooked)];
        written.extend(run_test_with(
            workspace,
            options,
            machine,
            m,
            out,
            Some(device),
            Some(driver.name()),
            &extra,
        )?);
    }
    Ok(written)
}

/// The frames of `triangle`'s fixed-frame run, criterion 1's.
const TRIANGLE_FRAMES: &str = "600";

/// How long `triangle`'s run of the client binary may take: the run is about 20 seconds on
/// the M1 under its validators, so this is a margin for a loaded machine.
const TRIANGLE_DEADLINE: Duration = Duration::from_secs(600);

/// The client binary's package and binary.
const CLIENT: &str = "lactose-app-client";

/// The test binaries of `lactose-rhi-vulkan` that hold the layer's canaries: its library's
/// unit tests and its `integration` target.
const CANARY_BINARIES: [Option<&str>; 2] = [None, Some("integration")];

/// The figures of the client binary's `Outcome` line, as its `Debug` prints it: each count
/// under its field's name, the comparison's counts as `counts.<field>` and `surely` as its
/// list, `none` for a field that is `None`. `None` where the line is not one.
#[must_use]
pub fn outcome_figures(line: &str) -> Option<Vec<(String, String)>> {
    let body = line.trim().strip_prefix("Outcome { ")?.strip_suffix(" }")?;
    // A field's value: what follows `name: ` at the start of `text` or after a `, ` or `{ `.
    let field = |text: &str, name: &str| -> Option<String> {
        let key = format!("{name}: ");
        let at = text
            .match_indices(&key)
            .map(|(at, _)| at)
            .find(|&at| at == 0 || text[..at].ends_with(", ") || text[..at].ends_with("{ "))?;
        let rest = &text[at + key.len()..];
        let end = if rest.starts_with("Some(") {
            // To the parenthesis that closes `Some(`.
            let mut depth = 0usize;
            rest.char_indices().find_map(|(at, c)| match c {
                '(' | '[' | '{' => {
                    depth += 1;
                    None
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    (depth == 0).then_some(at + 1)
                }
                _ => None,
            })?
        } else {
            rest.find(", ").unwrap_or(rest.len())
        };
        Some(rest[..end].to_owned())
    };
    let mut found = Vec::new();
    for name in ["frames", "drew", "held", "shed", "dropped", "busy", "messages"] {
        let value = field(body, name)?;
        value.parse::<u64>().ok()?;
        found.push((name.to_owned(), value));
    }
    match field(body, "counts")?.as_str() {
        "None" => found.push(("counts".to_owned(), "none".to_owned())),
        counts => {
            let inner = counts.strip_prefix("Some(Counts { ")?.strip_suffix(" })")?;
            for name in ["drawn", "clear", "margin"] {
                let value = field(inner, name)?;
                value.parse::<u64>().ok()?;
                found.push((format!("counts.{name}"), value));
            }
        }
    }
    match field(body, "surely")?.as_str() {
        "None" => found.push(("surely".to_owned(), "none".to_owned())),
        surely => {
            let list = surely.strip_prefix("Some([")?.strip_suffix("])")?;
            found.push(("surely".to_owned(), list.to_owned()));
        }
    }
    Some(found)
}

/// Why the client binary's fixed-frame run fails criterion 1 beside what [`vulkan::judge`]
/// reads: no `Outcome` line, the comparison not made, a slot with no pixel surely its
/// colour, or a line its log wrote at warn or above.
#[must_use]
pub fn triangle_failures(output: &str) -> Vec<String> {
    let mut failures = Vec::new();
    let found = output.lines().find_map(outcome_figures);
    match &found {
        None => failures.push("the binary printed no `Outcome` line".to_owned()),
        Some(found) => {
            let get = |name: &str| found.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
            if get("counts") == Some("none") {
                failures.push("the last frame was not compared with the reference".to_owned());
            }
            match get("surely") {
                Some("none") | None => {
                    failures.push("the run counted no pixels surely drawn".to_owned());
                }
                Some(list) => {
                    if list.split(", ").any(|n| n == "0") {
                        failures.push(format!("a slot has no pixel surely its colour: [{list}]"));
                    }
                }
            }
        }
    }
    // `tracing`'s format: the time, then the level.
    let loud = output
        .lines()
        .filter(|line| {
            let mut words = line.split_whitespace();
            words.next().is_some_and(|time| time.ends_with('Z') && time.contains('T'))
                && words.next().is_some_and(|level| level == "WARN" || level == "ERROR")
        })
        .count();
    if loud > 0 {
        failures.push(format!("{loud} line(s) of the binary's log at warn or above"));
    }
    failures
}

/// `triangle`: under each driver, the layer's canaries, Metal's API validation's canary under
/// MoltenVK, then the client binary's fixed-frame run, each in a process of its own under the
/// environment `cargo xtask gpu` gives its tests; each driver's pair of files named for it.
/// Every run and canary is judged and recorded before the measurement fails on any of them.
fn run_triangle(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
    devices: &mut Devices,
) -> Result<Vec<PathBuf>, String> {
    let shaders = devices.shaders(workspace)?;
    let pin = vulkan::Pin::parse(&read_file(&workspace.root.join(vulkan::PIN_FILE))?)?;
    let sdk = vulkan::sdk(&pin, &workspace.target_dir)?;
    let dir = workspace.target_dir.join("measure").join("triangle");
    let [layers, no_layers] = vulkan::layer_dirs(&sdk, &dir)?;
    let args: Vec<String> = ["build", "--locked", "--release", "-p", CLIENT, "--bin", CLIENT]
        .map(str::to_owned)
        .to_vec();
    build(workspace, &args)?;
    let binary = workspace
        .target_dir
        .join("release")
        .join(format!("{CLIENT}{}", std::env::consts::EXE_SUFFIX));
    let mut canary_binaries = Vec::new();
    for target in CANARY_BINARIES {
        let binary =
            vulkan::test_binary(&workspace.cargo, &workspace.root, "lactose-rhi-vulkan", target)?;
        let tests = vulkan::ignored_tests(&binary)?;
        canary_binaries.push((binary, tests));
    }
    let mut written = Vec::new();
    let mut failed = Vec::new();
    for driver in options.drivers() {
        // The device's identity, `vulkaninfo`'s under the measured environment.
        let device = devices.get(workspace, options, driver)?;
        let run = vulkan::Run::environment(
            driver,
            &sdk,
            [&layers, &no_layers, &shaders],
            vulkan::Validators::Plain,
            options.adapter.as_deref(),
        )?;
        let mut found = Vec::new();
        let mut output = String::new();
        let mut failures = Vec::new();
        // The canaries first: a layer that did not load reports nothing, and the run after
        // would pass for clean.
        for (canary, ids) in &run.canaries {
            let name = canary.rsplit("::").next().unwrap_or(canary);
            let Some((binary, _)) =
                canary_binaries.iter().find(|(_, tests)| tests.iter().any(|t| t == canary))
            else {
                failures.push(format!("{canary}: a listed canary no test binary holds"));
                found
                    .push((format!("canary.{name}"), "FAILED, no test binary holds it".to_owned()));
                continue;
            };
            let mut command = Command::new(binary);
            run.apply(&mut command)
                .args(["--exact", canary, "--ignored", "--nocapture", "--test-threads", "1"])
                .stdin(Stdio::null());
            let ended = vulkan::run_bounded(&mut command, vulkan::DEADLINE)
                .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
            let verdict = vulkan::judge_ended(canary, &ended, Some(ids), run.metal);
            output.push_str(&vulkan::entry("lactose-rhi-vulkan", canary, &verdict, &ended));
            let word = match (&verdict.failure, &verdict.said) {
                (Some(_), _) => {
                    failures.push(format!("{canary}: {}", verdict.word()));
                    verdict.word()
                }
                // A canary skipped for what its device lacks, which it names.
                (None, Some(said)) if said.starts_with("skipped, ") => said.clone(),
                (None, _) => format!("heard {}", verdict.messages.len() + verdict.reports.len()),
            };
            found.push((format!("canary.{name}"), word));
        }
        if run.metal {
            let (canary, phrasings, aborts) = vulkan::METAL_CANARIES[0];
            let step = vulkan::metal_step(&workspace.root, &["--canary", canary])?;
            let word = match vulkan::said(phrasings, &step.stderr) {
                Some(says) if step.status.success() != aborts => format!("heard `{says}`"),
                _ => {
                    failures.push(format!(
                        "Metal {canary} validation's canary was not heard: it ended with {}",
                        step.status
                    ));
                    format!("FAILED, not heard; it ended with {}", step.status)
                }
            };
            output.push_str(&format!("{} --canary {canary}: {word}\n", vulkan::METAL_SCRIPT));
            found.push((format!("canary.metal_{canary}"), word));
        }
        let run_dir = dir.join(driver.name());
        let _ = fs::remove_dir_all(&run_dir);
        fs::create_dir_all(&run_dir).map_err(|e| format!("{}: {e}", run_dir.display()))?;
        let (cache, dump) = (run_dir.join("pipeline-cache.bin"), run_dir.join("dump"));
        let run_args: Vec<String> = [
            "--shaders".to_owned(),
            shaders.to_string_lossy().into_owned(),
            "--cache".to_owned(),
            cache.to_string_lossy().into_owned(),
            "--fixed".to_owned(),
            TRIANGLE_FRAMES.to_owned(),
            dump.to_string_lossy().into_owned(),
        ]
        .to_vec();
        let mut command = Command::new(&binary);
        run.apply(&mut command).args(&run_args).current_dir(&run_dir).stdin(Stdio::null());
        pass_workers(&mut command, machine);
        eprintln!("measure: triangle under {}, {TRIANGLE_FRAMES} frames", driver.name());
        let sampler = Sampler::start();
        let ended = vulkan::run_bounded(&mut command, TRIANGLE_DEADLINE);
        let load = sampler.stop();
        let ended = ended.map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
        let text = ended.text();
        let verdict = vulkan::judge_ended(CLIENT, &ended, None, run.metal);
        let mut run_failures: Vec<String> = verdict.failure.iter().cloned().collect();
        run_failures.extend(triangle_failures(&text));
        found.push(("run".to_owned(), ended.status.clone()));
        found.extend(text.lines().find_map(outcome_figures).unwrap_or_default());
        found.push(("validator_messages".to_owned(), verdict.messages.len().to_string()));
        found.push(("driver_lines".to_owned(), verdict.driver.len().to_string()));
        failures.extend(run_failures.iter().map(|why| format!("the run: {why}")));
        found.push((
            "criterion".to_owned(),
            if failures.is_empty() {
                "met".to_owned()
            } else {
                format!("not met: {}", failures.join("; "))
            },
        ));
        let shown: Vec<String> =
            run_args.iter().map(|arg| shown_path(&workspace.root, Path::new(arg))).collect();
        let ran = shell_line(&[], &format!("target/release/{CLIENT}"), &shown);
        let mut header =
            header(options, machine, m, "release", default_workers(machine), ran, load);
        header.variant = Some(driver.name());
        let adapter = text.lines().find_map(|line| {
            let at = line.find("adapter: \"")? + "adapter: \"".len();
            line[at..].split('"').next().map(str::to_owned)
        });
        header.notes.extend(device_notes(&device.setting, adapter.as_deref()));
        // Paths inside the repository relative to its root, the SDK's from `<sdk>`.
        let environment: Vec<String> = run
            .set
            .iter()
            .map(|(name, value)| {
                let value = if Path::new(value).starts_with(&workspace.root) {
                    shown_path(&workspace.root, Path::new(value))
                } else {
                    redacted(value, sdk.loader.parent())
                };
                format!("`{name}={value}`")
            })
            .collect();
        for (name, value) in &mut header.notes {
            match name.as_str() {
                "Validation" => {
                    *value = format!(
                        "the pinned layer {}'s core, synchronisation (with load after store) and \
                         thread-safety validation{}, each canary run in a process of its own \
                         first",
                        pin.version,
                        if run.metal { ", and Metal's API and shader validation" } else { "" },
                    );
                }
                "Environment" => *value = environment.join(", "),
                "Presentation" => {
                    *value = "a window, the present thread drawing each frame's offscreen \
                              target to the swapchain's image; the display held awake by the \
                              caller"
                        .to_owned();
                }
                _ => {}
            }
        }
        header.notes.push((
            "Present split".to_owned(),
            "the frames that drew split between queued (`drew`), `held`, `shed` and `dropped` \
             as this one run's present thread met them: another run splits them otherwise, and \
             criterion 1 reads none of them"
                .to_owned(),
        ));
        output.push_str(&vulkan::entry(CLIENT, "--fixed", &verdict, &ended));
        output.push_str(&vulkan::output(CLIENT, &ended));
        written.extend(write(out, &header, &found, &output)?);
        if !failures.is_empty() {
            failed.push(format!("under {}: {}", driver.name(), failures.join("; ")));
        }
    }
    if failed.is_empty() {
        Ok(written)
    } else {
        for file in &written {
            eprintln!("measure: wrote {}", file.display());
        }
        Err(format!("criterion 1 is not met {}", failed.join(", and ")))
    }
}

/// `frame-pace`'s real-time run and floor run, in seconds: a full run's, then a short
/// run's. The real-time run is both the plan's shown run of ten seconds or more and its run
/// of a minute or more at real time.
const PACE_SECONDS: [(u32, u32); 2] = [(60, 60), (12, 20)];
/// How long past its seconds a run of `frame-pace` may take: its device, its pipelines and
/// its shutdown.
const PACE_GRACE: Duration = Duration::from_secs(120);
/// The Swift script that reads each active display's refresh rate on macOS.
pub const DISPLAY_SCRIPT: &str = "tools/xtask/display/rate.swift";

/// The display's refresh rate as `frame-pace` reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayRate {
    /// The display mode's exact rate in hertz, where it is read.
    pub hz: Option<f64>,
    /// Where it came from, or why it is unread, with what the screen reports beside it and
    /// every other display's rate.
    pub source: String,
}

/// The rate in the display script's output, one line a display: the main display's, where
/// the window opens, its mode's where that is not zero and its display link's nominal period
/// where it is (a display of variable rate reports none); its screen's greatest frames a
/// second, which MoltenVK falls back to only where the mode reports 0, named beside it; and
/// the mode's rate of each other display.
#[must_use]
pub fn display_rate_of(output: &str) -> DisplayRate {
    let displays: Vec<Vec<&str>> = output
        .lines()
        .filter_map(|line| line.strip_prefix("display "))
        .map(|line| line.split(' ').collect())
        .collect();
    let value = |fields: &[&str], name: &str| {
        let at = fields.iter().position(|field| *field == name)?;
        fields.get(at + 1)?.parse::<f64>().ok()
    };
    let others: Vec<String> = displays
        .iter()
        .filter(|fields| !fields.contains(&"main"))
        .map(|fields| {
            value(fields, "mode_hz").map_or_else(
                || format!("display {} no mode's rate", fields[0]),
                |hz| format!("display {} {hz:.6} Hz", fields[0]),
            )
        })
        .collect();
    let others = if others.is_empty() {
        "no other display".to_owned()
    } else {
        format!("the other displays: {}", others.join(", "))
    };
    let Some(main) = displays.iter().find(|fields| fields.contains(&"main")) else {
        return DisplayRate { hz: None, source: format!("unread: no main display; {others}") };
    };
    let screen = value(main, "screen_fps").map_or_else(
        || "the screen's greatest frames a second unreported".to_owned(),
        |fps| format!("the screen's greatest frames a second {fps}"),
    );
    let mode = value(main, "mode_hz").filter(|hz| *hz > 0.0);
    match (mode, value(main, "link_hz").filter(|hz| *hz > 0.0)) {
        (Some(hz), link) => DisplayRate {
            hz: Some(hz),
            source: format!(
                "{hz:.6} Hz, the main display's mode (`CGDisplayModeGetRefreshRate`); {}; \
                 {screen}; {others}",
                link.map_or_else(
                    || "the display link's nominal period unreported".to_owned(),
                    |link| format!("the display link's nominal period {link:.6} Hz"),
                ),
            ),
        },
        (None, Some(hz)) => DisplayRate {
            hz: Some(hz),
            source: format!(
                "{hz:.6} Hz, the main display's link's nominal period, the mode reporting no \
                 rate; {screen}; {others}"
            ),
        },
        (None, None) => DisplayRate {
            hz: None,
            source: format!(
                "unread: neither the main display's mode nor its display link gave a rate; \
                 {screen}; {others}"
            ),
        },
    }
}

fn display_rate(root: &Path) -> DisplayRate {
    if !cfg!(target_os = "macos") {
        return DisplayRate {
            hz: None,
            source: "unread: `frame-pace` reads the display mode's rate on macOS alone".to_owned(),
        };
    }
    let output =
        Command::new("swift").current_dir(root).arg(DISPLAY_SCRIPT).stdin(Stdio::null()).output();
    match output {
        Ok(output) if output.status.success() => {
            display_rate_of(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(output) => DisplayRate {
            hz: None,
            source: format!("unread: {DISPLAY_SCRIPT} ended with {}", output.status),
        },
        Err(e) => DisplayRate { hz: None, source: format!("unread: cannot run swift: {e}") },
    }
}

/// Why a swapchain reports no display times, as `frame-pace`'s file says it.
const NO_DISPLAY_TIMES: &str = "`lactose-rhi-vulkan` reads display times from \
    `VK_GOOGLE_display_timing` alone, and D162 admits `VK_EXT_present_timing` and \
    `VK_KHR_present_wait`'s completions only once a run shows they follow the display: a \
    driver that offers neither of the first two, as KosmicKrisp offers neither, \
    reports none";

/// What `frame-pace`'s figures hold, as its file's header says it.
const READINGS: &str = "the lateness is the first tick's start against the deadline, split \
    into the timer's wake (the deadline to `Shown::frame`'s call) and the engine's start (the \
    call to the first tick), and read at its P99, the count over the criterion a figure beside \
    it; the frame before a late one is classed by what it deferred, or as `joined_at_limit` \
    where its call returned at or after this frame's deadline less `JOIN_MARGIN`, its join \
    having waited to its limit, or as `joined` where it returned sooner; reading 1 counts the \
    submits that began during a present's driver call by the two spans' stamps, a submit that \
    began before a call and ran into it being its own cost, and reads them against the driver \
    call on a shared queue and, on two queues, by their P99's excess over the P99 of the \
    submits begun outside one, a defect past the latter's P50-to-P99 spread; a missing \
    `join_margin_ns` or `shed_depth` is written as unread; a present call's \
    parts are its waits (for its last read, the resize and the encoder's drop, which waits for \
    the frame's GPU work and the blit), the acquire, the record (the encoder's creation and \
    the blit's recording), the driver's call (`Swapchain::present`: the wait for the image's \
    last-present fence, then the submit and the present under the queue's lock) and the rest; \
    `waiting` counts presents displayed and not yet reported as well, and `undisplayed` counts \
    those not yet displayed by their own display times; a display report earlier than its \
    present's return is dropped and counted, and a present with no display report kept, the \
    last calls' among them and those whose report was dropped, counted as missing; the latch's median age at the display depends \
    on the latches' phase against the refresh, which `latch_phase` gives; the grid test is \
    D162's, the reported refresh refined by the floor run's fit to five parts in 10,000 at \
    most, beside the period the free fit lands on";

/// One run of the client binary for `frame-pace`.
struct PaceRun<'a> {
    binary: &'a Path,
    device: &'a OnDevice,
    machine: &'a Machine,
    dir: &'a Path,
    shaders: &'a Path,
}

impl PaceRun<'_> {
    /// Runs the binary paced for `seconds` at `scale`, minimised or not, its stamps written
    /// to `<label>.stamps`; returns them, the command as the header shows it and the run's
    /// output, with the load before and after.
    fn run(
        &self,
        root: &Path,
        label: &str,
        seconds: u32,
        scale: Option<u32>,
        minimised: bool,
    ) -> Result<(pace::Stamps, String, String), String> {
        let stamps = self.dir.join(format!("{label}.stamps"));
        let mut args = vec![
            "--shaders".to_owned(),
            self.shaders.to_string_lossy().into_owned(),
            "--cache".to_owned(),
            self.dir.join("pipeline-cache.bin").to_string_lossy().into_owned(),
            "--seconds".to_owned(),
            seconds.to_string(),
        ];
        if let Some(scale) = scale {
            args.extend(["--time-scale".to_owned(), scale.to_string()]);
        }
        if minimised {
            args.push("--minimised".to_owned());
        }
        args.extend(["--stamps".to_owned(), stamps.to_string_lossy().into_owned()]);
        let mut command = Command::new(self.binary);
        self.device.run.apply(&mut command).args(&args).current_dir(self.dir).stdin(Stdio::null());
        pass_workers(&mut command, self.machine);
        eprintln!("measure: frame-pace's {label} run, {seconds} s");
        let before = uptime();
        let ended =
            vulkan::run_bounded(&mut command, Duration::from_secs(seconds.into()) + PACE_GRACE)
                .map_err(|e| format!("cannot run {}: {e}", self.binary.display()))?;
        let after = uptime();
        let shown: Vec<String> = args.iter().map(|arg| shown_path(root, Path::new(arg))).collect();
        let ran = shell_line(&[], &format!("target/release/{CLIENT}"), &shown);
        let output = format!(
            "{label}: `{ran}`, load {before} before, {after} after\n{}",
            vulkan::output(CLIENT, &ended)
        );
        if !ended.succeeded {
            return Err(format!("the {label} run ended with {}:\n{output}", ended.status));
        }
        let stamps = pace::Stamps::parse(&read_file(&stamps)?)?;
        Ok((stamps, ran, output))
    }
}

fn run_frame_pace(
    workspace: &Workspace,
    options: &Options,
    machine: &Machine,
    m: &'static Measurement,
    out: &Path,
    devices: &mut Devices,
) -> Result<Vec<PathBuf>, String> {
    let shaders = devices.shaders(workspace)?;
    let args: Vec<String> = ["build", "--locked", "--release", "-p", CLIENT, "--bin", CLIENT]
        .map(str::to_owned)
        .to_vec();
    build(workspace, &args)?;
    let binary = workspace
        .target_dir
        .join("release")
        .join(format!("{CLIENT}{}", std::env::consts::EXE_SUFFIX));
    let rate = display_rate(&workspace.root);
    let mode_period = rate.hz.map(|hz| 1e9 / hz);
    let (real_seconds, floor_seconds) = PACE_SECONDS[usize::from(options.short)];
    let lateness = if cfg!(windows) {
        Some(1_000_000)
    } else if cfg!(target_os = "macos") {
        Some(50_000)
    } else {
        None
    };
    let mut written = Vec::new();
    for driver in options.drivers() {
        let device = devices.get(workspace, options, driver)?;
        let dir = workspace.target_dir.join("measure").join("frame-pace").join(driver.name());
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let runner = PaceRun { binary: &binary, device, machine, dir: &dir, shaders: &shaders };
        let mut output = String::new();
        let mut found = vec![("display.rate".to_owned(), rate.source.clone())];
        let sampler = Sampler::start();
        let (real, ran, said) = runner.run(&workspace.root, "real", real_seconds, None, false)?;
        output.push_str(&said);
        // The floor, where the swapchain reports display times and the display mode's rate
        // is read, from that rate; once more from the floor's own fit where it swept fewer
        // than twice.
        let mut floor = None;
        let mut runs = vec![format!("the real-time run of {real_seconds} s")];
        match pace::floor_period(mode_period, real.reports_display_times()) {
            Ok(period) => {
                let tick = real.tick_period().ok_or("the stamps name no tick period")?;
                let scale = pace::floor_scale(1e9 / period, tick);
                found.push(("floor.scale_thousandths".to_owned(), scale.to_string()));
                let (stamps, _, said) =
                    runner.run(&workspace.root, "floor", floor_seconds, Some(scale), false)?;
                output.push_str(&said);
                runs.push(format!("the floor run of {floor_seconds} s at {scale} thousandths"));
                let first = pace::floor_figures("floor", &stamps, period);
                found.extend(first.figures);
                floor = first.ages;
                if first.ages.is_none()
                    && let Some(fitted) = first.period
                {
                    // A scale equal to the first run's is run again all the same: the
                    // thousandths are the finest step the flag takes.
                    let again = pace::floor_scale(1e9 / fitted, tick);
                    found.push(("floor_again.scale_thousandths".to_owned(), again.to_string()));
                    let (stamps, _, said) = runner.run(
                        &workspace.root,
                        "floor_again",
                        floor_seconds,
                        Some(again),
                        false,
                    )?;
                    output.push_str(&said);
                    runs.push(format!(
                        "the floor run made again, `floor_again`, of {floor_seconds} s at {again} \
                         thousandths, from the floor run's fitted refresh"
                    ));
                    let again = pace::floor_figures("floor_again", &stamps, period);
                    found.extend(again.figures);
                    floor = again.ages;
                }
            }
            Err(none) => found.push(("floor".to_owned(), none)),
        }
        if !real.reports_display_times() {
            found.push(("display_times.why".to_owned(), NO_DISPLAY_TIMES.to_owned()));
        }
        let refresh = mode_period;
        let against = pace::Against { lateness, floor, refresh };
        found.extend(pace::figures("real", &real, against));
        if cfg!(windows) {
            let (minimised, _, said) =
                runner.run(&workspace.root, "minimised", real_seconds, None, true)?;
            output.push_str(&said);
            found.extend(pace::figures(
                "minimised",
                &minimised,
                pace::Against { floor: None, ..against },
            ));
        }
        let load = sampler.stop();
        let mut header =
            header(options, machine, m, "release", default_workers(machine), ran, load);
        header.variant = Some(driver.name());
        let adapter = output.lines().find_map(|line| {
            let at = line.find("adapter: \"")? + "adapter: \"".len();
            line[at..].split('"').next().map(str::to_owned)
        });
        header.notes.extend(device_notes(&device.setting, adapter.as_deref()));
        for (name, value) in &mut header.notes {
            if name == "Presentation" {
                *value = format!(
                    "a window, the present thread drawing each frame's offscreen target to the \
                     swapchain's image, no validator on; the display's rate {}; whether the \
                     window was occluded is not read, `lactose-platform` forwarding none of \
                     winit's `WindowEvent::Occluded`, and whether another client was drawing is \
                     not read; the display held awake by the caller",
                    rate.source
                );
            }
        }
        if cfg!(windows) {
            runs.push("the minimised run".to_owned());
        }
        header.notes.push((
            "Runs".to_owned(),
            format!(
                "{}, each run's command and load in the output below; every figure is the \
                 binary's own stamps, taken while a run asks for them, each stamp one lock",
                runs.join(", ")
            ),
        ));
        header.notes.push(("Readings".to_owned(), READINGS.to_owned()));
        written.extend(write(out, &header, &found, &output)?);
    }
    Ok(written)
}

/// A file's text.
fn read_file(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
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
            if let Some((_, _, test)) = run.ignored_test() {
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

    /// A measurement run under each driver writes each driver's files apart, named and
    /// headed for it.
    #[test]
    fn a_run_under_each_driver_names_its_files_for_the_driver() {
        let header = Header { variant: Some("kosmickrisp"), short: false, ..sample_header() };
        assert_eq!(header_stem(&header), "tick-100k-kosmickrisp-apple-m1-2026-11-15");
        let text = markdown(&header, &[], "");
        assert!(text.starts_with("# tick-100k under kosmickrisp, apple-m1, 2026-11-15\n"));
        assert_eq!(header_stem(&sample_header()), "tick-100k-apple-m1-2026-11-15-short");
        let both = parse(&["gpu-cost"]).unwrap().drivers();
        let host: &[vulkan::Driver] =
            if cfg!(target_os = "macos") { &vulkan::MACOS_GATE } else { &[vulkan::Driver::System] };
        assert_eq!(both, host);
        let one = parse(&["gpu-cost", "--driver", "kosmickrisp"]).unwrap().drivers();
        assert_eq!(one, [vulkan::Driver::KosmicKrisp]);
    }

    /// `gpu-cost`'s salted source: every pipeline's pair of entries named once, each folding
    /// the salt with its own number, and the salt given by the link module alone.
    #[test]
    fn the_salted_source_gives_each_pipeline_its_own_entries() {
        let source = salted_source(COST_PIPELINES);
        for at in 0..COST_PIPELINES {
            for entry in [format!("Varyings vs_{at:02}("), format!("Colour fs_{at:02}(")] {
                assert_eq!(source.matches(&entry).count(), 1, "{entry}");
            }
            assert_eq!(source.matches(&format!("(SALT + {at}u)")).count(), 2, "{at}");
        }
        assert_eq!(source.matches("[shader(").count(), 2 * COST_PIPELINES as usize);
        assert!(source.contains("extern static const uint SALT;"));
        assert!(salted_link(7).contains("export static const uint SALT = 7u;"));
        assert_ne!(salted_link(7), salted_link(8));
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
            if let Some((_, file, test)) = m.run.ignored_test() {
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

    /// The client binary's `Outcome` line, as `frame-impl`'s run on the M1 printed it.
    const OUTCOME: &str = "Outcome { packs: 0, frames: 600, drew: 571, held: 0, shed: 2, \
                           dropped: 27, busy: 0, counts: Some(Counts { drawn: 1044, clear: \
                           195304, margin: 260 }), surely: Some([140, 151, 149, 150, 152, 151, \
                           151]), messages: 0 }";

    #[test]
    fn the_outcome_line_gives_each_count_and_the_comparison() {
        let found = outcome_figures(OUTCOME).unwrap();
        let get = |name: &str| found.iter().find(|(n, _)| n == name).unwrap().1.as_str();
        assert_eq!(get("frames"), "600");
        assert_eq!(get("drew"), "571");
        assert_eq!(get("held"), "0");
        assert_eq!(get("shed"), "2");
        assert_eq!(get("dropped"), "27");
        assert_eq!(get("messages"), "0");
        assert_eq!(get("counts.drawn"), "1044");
        assert_eq!(get("counts.clear"), "195304");
        assert_eq!(get("counts.margin"), "260");
        assert_eq!(get("surely"), "140, 151, 149, 150, 152, 151, 151");
        let paced = OUTCOME
            .replace("Some(Counts { drawn: 1044, clear: 195304, margin: 260 })", "None")
            .replace("Some([140, 151, 149, 150, 152, 151, 151])", "None");
        let found = outcome_figures(&paced).unwrap();
        assert!(found.contains(&("counts".to_owned(), "none".to_owned())));
        assert!(found.contains(&("surely".to_owned(), "none".to_owned())));
        assert!(outcome_figures("Outcome { frames: 600 }").is_none(), "a field missing");
        assert!(outcome_figures(&OUTCOME.replace("drew: 571", "drew: many")).is_none());
        assert!(outcome_figures("a line of the log").is_none());
    }

    #[test]
    fn the_run_fails_without_a_comparison_a_slot_unseen_or_a_warning() {
        let log = "2026-11-25T03:00:00.000000Z  INFO lactose_app_client: started";
        assert!(triangle_failures(&format!("{log}\n{OUTCOME}\n")).is_empty());
        assert_eq!(triangle_failures(log), ["the binary printed no `Outcome` line"]);
        let unseen = OUTCOME.replace("[140, 151", "[0, 151");
        let failures = triangle_failures(&unseen);
        assert!(failures[0].starts_with("a slot has no pixel surely its colour"), "{failures:?}");
        let uncompared =
            OUTCOME.replace("Some(Counts { drawn: 1044, clear: 195304, margin: 260 })", "None");
        assert_eq!(
            triangle_failures(&uncompared),
            ["the last frame was not compared with the reference"]
        );
        for level in ["WARN", "ERROR"] {
            let loud = format!("2026-11-25T03:00:00.000000Z  {level} lactose_app: late\n{OUTCOME}");
            assert_eq!(
                triangle_failures(&loud),
                ["1 line(s) of the binary's log at warn or above"],
                "{level}"
            );
        }
        // A word in a message is not a level.
        assert!(triangle_failures(&format!("the WARN word\n{OUTCOME}")).is_empty());
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

    #[test]
    fn the_display_s_rate_is_the_main_mode_s_then_its_link_s() {
        let both = display_rate_of(
            "display 1 main mode_hz 60.000000 link_hz 59.999832 screen_fps 60\n\
             display 3 mode_hz 120.000000 screen_fps 120\n",
        );
        assert_eq!(both.hz, Some(60.0));
        assert!(both.source.contains("mode") && both.source.contains("second 60"), "{both:?}");
        assert!(both.source.contains("display 3 120.000000 Hz"), "{both:?}");
        let variable = display_rate_of("display 2 main mode_hz 0.000000 link_hz 120.000000\n");
        assert_eq!(variable.hz, Some(120.0));
        assert!(variable.source.contains("link") && variable.source.contains("unreported"));
        assert!(variable.source.ends_with("no other display"), "{variable:?}");
        assert_eq!(display_rate_of("display 1 main screen_fps 60\n").hz, None);
        assert_eq!(display_rate_of("display 1 mode_hz 60.0\n").hz, None);
    }

    /// The measure workflow's runners have no device: `--no-device` leaves those that need
    /// one out of `--all`, and refuses one named.
    #[test]
    fn the_arguments_select_the_device_and_leave_it_out() {
        let all = parse(&["--all", "--short", "--no-device"]).unwrap();
        let names: Vec<&str> = all.selected.iter().map(|m| m.name).collect();
        assert!(names.contains(&"record-stall") && names.contains(&"tick-100k"), "{names:?}");
        assert!(!names.contains(&"extract-50k") && !names.contains(&"remote-upload-5k"));
        assert!(!names.contains(&"gpu-cost") && !names.contains(&"triangle"), "{names:?}");
        assert!(!names.contains(&"frame-pace"), "{names:?}");
        assert_eq!(all.selected.len() + 5, MEASUREMENTS.len());
        let error = parse(&["extract-50k", "--no-device"]).unwrap_err();
        assert!(error.contains("`extract-50k`"), "{error}");
        let named = parse(&["extract-50k", "--driver", "kosmickrisp", "--adapter", "Apple M1"]);
        let named = named.unwrap();
        assert_eq!(named.driver(), vulkan::Driver::KosmicKrisp);
        assert_eq!(named.adapter.as_deref(), Some("Apple M1"));
        let default = parse(&["remote-upload-5k"]).unwrap().driver();
        let host = if cfg!(target_os = "macos") {
            vulkan::Driver::MoltenVk
        } else {
            vulkan::Driver::System
        };
        assert_eq!(default, host);
        assert!(parse(&["extract-50k", "--driver", "metal"]).is_err());
        assert!(parse(&["extract-50k", "--driver"]).is_err());
        assert!(parse(&["extract-50k", "--driver", "system", "--driver", "system"]).is_err());
        assert!(parse(&["tick-100k", "--driver", "system"]).is_err());
        assert!(parse(&["--all", "--no-device", "--adapter", "Apple M1"]).is_err());
    }

    /// `vulkaninfo`'s text output under MoltenVK on the M1, cut to what the header reads, and
    /// a second device after it.
    const VULKANINFO: &str = "==========\nVULKANINFO\n==========\n\n\
        Vulkan Instance Version: 1.4.363\r\n\n\
        Presentable Surfaces:\n=====================\nGPU id : 0 (Apple M1) [VK_EXT_metal_surface]:\n\
        Device Properties and Extensions:\n=================================\nGPU0:\n\
        VkPhysicalDeviceProperties:\n---------------------------\n\
        \tapiVersion        = 1.4.357 (4211045)\n\tdriverVersion     = 0.2.2210 (10402)\n\
        \tvendorID          = 0x106b\n\tdeviceID          = 0x1b020207\n\
        \tdeviceType        = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n\
        \tdeviceName        = Apple M1\n\
        VkPhysicalDeviceDriverProperties:\n\
        \tdriverID                                             = DRIVER_ID_MOLTENVK\n\
        \tdriverName                                           = MoltenVK\n\
        \tdriverInfo                                           = 1.4.2\n\
        VkPhysicalDeviceLayeredApiPropertiesListKHR:\n\t\tdeviceName = Metal\n\
        memoryTypes: count = 3\n\
        \tmemoryTypes[0]:\n\t\theapIndex     = 0\n\t\tpropertyFlags = 0x0001: count = 1\n\
        \t\t\tMEMORY_PROPERTY_DEVICE_LOCAL_BIT\n\
        \tmemoryTypes[1]:\n\t\theapIndex     = 0\n\t\tpropertyFlags = 0x000f: count = 4\n\
        \tmemoryTypes[2]:\n\t\theapIndex     = 0\n\t\tpropertyFlags = 0x0011: count = 2\n\
        GPU1:\n\tdeviceName = Other\n\t\tpropertyFlags = 0x0012: count = 2\n\
        \t\tpropertyFlags = 0x0002: count = 1\n";

    #[test]
    fn the_device_s_identity_is_read_from_vulkaninfo() {
        let info = vulkaninfo(VULKANINFO);
        assert_eq!(info.instance.as_deref(), Some("1.4.363"));
        assert_eq!(info.gpus.len(), 2, "the surfaces' `GPU id` line is no device");
        let m1 = &info.gpus[0];
        assert_eq!(m1.get("deviceName"), Some("Apple M1"), "the first of a key is kept");
        assert_eq!(m1.get("apiVersion"), Some("1.4.357"));
        assert_eq!(m1.get("driverVersion"), Some("0.2.2210"));
        assert_eq!(m1.get("driverInfo"), Some("1.4.2"));
        assert_eq!(m1.memory_types, [0x1, 0xf, 0x11]);
        assert_eq!(upload_type(&m1.memory_types), Some(1));
        assert_eq!(
            memory_flags(0xf),
            "DEVICE_LOCAL + HOST_VISIBLE + HOST_COHERENT + HOST_CACHED (0x000f)"
        );
        assert_eq!(memory_flags(0), "none (0x0000)");
        // A lazily allocated type is never chosen, and a coherent one before any other.
        assert_eq!(upload_type(&info.gpus[1].memory_types), Some(1));
        assert_eq!(upload_type(&[0x2, 0x6]), Some(1));
        assert_eq!(upload_type(&[0x1, 0x11]), None);
        assert_eq!(vulkaninfo("no output"), VulkanInfo::default());
    }

    #[test]
    fn a_device_s_header_names_it_and_no_directory_of_the_machine() {
        let sdk = std::env::temp_dir().join("VulkanSDK").join("macOS");
        let icd = sdk.join("share").join("vulkan").join("icd.d").join("MoltenVK_icd.json");
        let icd = icd.to_str().unwrap();
        assert_eq!(redacted(icd, Some(&sdk)), "<sdk>/share/vulkan/icd.d/MoltenVK_icd.json");
        let elsewhere = std::env::temp_dir().join("lvp_icd.json");
        assert_eq!(redacted(elsewhere.to_str().unwrap(), Some(&sdk)), ".../lvp_icd.json");
        assert_eq!(redacted("~implicit~", Some(&sdk)), "~implicit~");
        let mut setting = DeviceSetting {
            driver: "moltenvk",
            sdk_version: Some("1.4.363.1".into()),
            environment: vec![("VK_DRIVER_FILES".into(), redacted(icd, Some(&sdk)))],
            info: Ok(vulkaninfo(VULKANINFO)),
        };
        let notes = device_notes(&setting, Some("Apple M1"));
        let text: String =
            notes.iter().map(|(name, value)| format!("| {name} | {value} |\n")).collect();
        for expected in [
            "| Device | Apple M1, vendor 0x106b, device 0x1b020207 |",
            "| Driver | MoltenVK 1.4.2, driver version 0.2.2210, `--driver moltenvk` |",
            "| Vulkan | the device's API 1.4.357, the instance's 1.4.363 |",
            "| Loader | the Vulkan SDK 1.4.363.1's |",
            "| Ring memory | type 1 of 3, DEVICE_LOCAL + HOST_VISIBLE + HOST_COHERENT + HOST_CACHED (0x000f):",
            "| Environment | `VK_DRIVER_FILES=<sdk>/share/vulkan/icd.d/MoltenVK_icd.json` |",
            "| Validation | off: no layer enabled",
            "| Presentation | none:",
        ] {
            assert!(text.contains(expected), "{expected} in\n{text}");
        }
        let unnamed = device_notes(&setting, Some("Apple M2"));
        assert!(unnamed[0].1.contains("lists no device named `Apple M2`"), "{unnamed:?}");
        assert!(!unnamed.iter().any(|(name, _)| name == "Ring memory"));
        setting.info = Err("`vulkaninfo` did not run: not found".into());
        let failed = device_notes(&setting, Some("Apple M1"));
        assert!(failed[0].1.starts_with("unreported: `vulkaninfo` did not run"), "{failed:?}");
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
            variant: None,
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
