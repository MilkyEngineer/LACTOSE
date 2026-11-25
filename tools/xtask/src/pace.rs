//! `frame-pace`'s reading (`README.md`, "frame-pace"): the stamps file the client binary
//! writes under `--stamps` (`crates/app-client/README.md`, "Stamps") read into the figures of
//! the milestone 2 plan's row for `frame-pace`, with the floor run's sweeps against a refresh
//! fitted freely to its display times and D162's grid test. Every time is in nanoseconds
//! since the file's epoch; every figure is printed in microseconds.

use std::collections::HashMap;
use std::f64::consts::TAU;

/// How far D162's grid test lets a fit to the display times move the display's reported
/// refresh: five parts in ten thousand.
const GRID_BOUND: f64 = 5e-4;

/// A paced frame, as the root stamped it.
#[derive(Clone, Debug, PartialEq)]
struct Frame {
    deadline: u64,
    first_tick: Option<u64>,
    called: u64,
    returned: u64,
    /// What the frame in flight did, taken back by this frame: `-` for nothing.
    joined: String,
    /// What this frame deferred: `-` for nothing.
    deferred: String,
    behind: bool,
}

/// A closure of a record: a recorder or the writer's closure.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Closure {
    thread: u32,
    entered: u64,
    left: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Job {
    thread: u32,
    start: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Submit {
    entered: u64,
    latched: u64,
    returned: u64,
    point: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Present {
    began: u64,
    waits: u64,
    acquire: u64,
    record: u64,
    driver: u64,
    /// The driver call's start, `None` where the call made none.
    driver_at: Option<u64>,
    returned: u64,
    waiting: Option<u32>,
    source: bool,
    point: u64,
    id: Option<u64>,
}

/// One run's stamps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stamps {
    info: Vec<(String, String)>,
    frames: Vec<Frame>,
    jobs: Vec<Job>,
    drews: Vec<u64>,
    closures: Vec<Closure>,
    submits: Vec<Submit>,
    presents: Vec<Present>,
    displayed: Vec<(u64, u64)>,
    images: Vec<u32>,
    stats: Option<String>,
}

fn number<T: std::str::FromStr>(field: Option<&str>, line: &str) -> Result<T, String> {
    field.and_then(|field| field.parse().ok()).ok_or_else(|| format!("a malformed stamp: `{line}`"))
}

fn optional<T: std::str::FromStr>(field: Option<&str>, line: &str) -> Result<Option<T>, String> {
    match field {
        Some("-") => Ok(None),
        field => number(field, line).map(Some),
    }
}

fn flag(field: Option<&str>, line: &str) -> Result<bool, String> {
    match field {
        Some("0") => Ok(false),
        Some("1") => Ok(true),
        _ => Err(format!("a malformed stamp: `{line}`")),
    }
}

fn word(field: Option<&str>, line: &str) -> Result<String, String> {
    field.map(str::to_owned).ok_or_else(|| format!("a malformed stamp: `{line}`"))
}

impl Stamps {
    /// The stamps of the file `text`.
    ///
    /// # Errors
    /// For a line of no known kind or with a field missing or malformed.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut stamps = Self::default();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let mut fields = line.split(' ');
            let kind = fields.next().unwrap_or_default();
            let mut next = || fields.next();
            match kind {
                "info" => {
                    let name = word(next(), line)?;
                    let value = line.splitn(3, ' ').nth(2).unwrap_or_default().to_owned();
                    stamps.info.push((name, value));
                }
                "frame" => stamps.frames.push(Frame {
                    deadline: number(next(), line)?,
                    first_tick: optional(next(), line)?,
                    called: number(next(), line)?,
                    returned: number(next(), line)?,
                    joined: word(next(), line)?,
                    deferred: word(next(), line)?,
                    behind: flag(next(), line)?,
                }),
                "job" => stamps
                    .jobs
                    .push(Job { thread: number(next(), line)?, start: number(next(), line)? }),
                "drew" => stamps.drews.push(number(next(), line)?),
                "closure" => {
                    word(next(), line)?;
                    stamps.closures.push(Closure {
                        thread: number(next(), line)?,
                        entered: number(next(), line)?,
                        left: number(next(), line)?,
                    });
                }
                "submit" => stamps.submits.push(Submit {
                    entered: number(next(), line)?,
                    latched: number(next(), line)?,
                    returned: number(next(), line)?,
                    point: number(next(), line)?,
                }),
                "present" => stamps.presents.push(Present {
                    began: number(next(), line)?,
                    waits: number(next(), line)?,
                    acquire: number(next(), line)?,
                    record: number(next(), line)?,
                    driver: number(next(), line)?,
                    driver_at: optional(next(), line)?,
                    returned: number(next(), line)?,
                    waiting: optional(next(), line)?,
                    source: flag(next(), line)?,
                    point: number(next(), line)?,
                    id: optional(next(), line)?,
                }),
                "displayed" => {
                    stamps.displayed.push((number(next(), line)?, number(next(), line)?));
                }
                "images" => stamps.images.push(number(next(), line)?),
                "stats" => stamps.stats = Some(line["stats ".len().min(line.len())..].to_owned()),
                _ => return Err(format!("a stamp of no known kind: `{line}`")),
            }
        }
        Ok(stamps)
    }

    fn info(&self, name: &str) -> Option<&str> {
        self.info.iter().find(|(n, _)| n == name).map(|(_, value)| value.as_str())
    }

    fn info_number<T: std::str::FromStr>(&self, name: &str) -> Option<T> {
        self.info(name)?.parse().ok()
    }

    /// Whether the swapchain had a source of display times at any present call.
    #[must_use]
    pub fn reports_display_times(&self) -> bool {
        self.presents.iter().any(|call| call.source)
    }

    /// The tick period the run was paced by, at a time scale of one.
    #[must_use]
    pub fn tick_period(&self) -> Option<u64> {
        self.info_number("tick_period_ns")
    }

    /// The display reports kept, in the order reported, and how many were dropped: a report
    /// earlier than its present's return is no display (MoltenVK files a reading of its own
    /// clock for a drawable that reports no presented time, `crates/rhi-vulkan/README.md`),
    /// and is dropped by that order.
    fn displays(&self) -> (Vec<(u64, u64)>, usize) {
        let returned: HashMap<u64, u64> =
            self.presents.iter().filter_map(|call| Some((call.id?, call.returned))).collect();
        let (mut kept, mut dropped) = (Vec::new(), 0);
        for &(id, at) in &self.displayed {
            if returned.get(&id).is_none_or(|&back| at >= back) {
                kept.push((id, at));
            } else {
                dropped += 1;
            }
        }
        (kept, dropped)
    }

    /// The display times kept, in order of time.
    fn display_times(&self) -> Vec<u64> {
        let mut times: Vec<u64> = self.displays().0.iter().map(|&(_, at)| at).collect();
        times.sort_unstable();
        times
    }

    /// The latches of the frames submitted, in order.
    fn latches(&self) -> Vec<u64> {
        self.submits.iter().map(|submit| submit.latched).collect()
    }

    /// The latch's age at the display for each display report kept, in order of the
    /// reports.
    fn display_ages(&self) -> Vec<u64> {
        let latched: HashMap<u64, u64> =
            self.submits.iter().map(|submit| (submit.point, submit.latched)).collect();
        let point: HashMap<u64, u64> =
            self.presents.iter().filter_map(|call| Some((call.id?, call.point))).collect();
        self.displays()
            .0
            .iter()
            .filter_map(|(id, at)| Some(at.saturating_sub(*latched.get(point.get(id)?)?)))
            .collect()
    }

    /// The figures of the display reports: those kept, those dropped by order, and the
    /// presents made with none kept.
    fn display_figures(&self, put: &mut impl FnMut(&str, String)) {
        let (kept, dropped) = self.displays();
        let shown: HashMap<u64, u64> = kept.iter().copied().collect();
        let missing =
            self.presents.iter().filter_map(|call| call.id).filter(|id| !shown.contains_key(id));
        put("display.kept", kept.len().to_string());
        put("display.dropped_before_return", dropped.to_string());
        put("display.missing", missing.count().to_string());
    }
}

/// The value at `p` (0 to 1) of `values` by nearest rank, `None` for none.
fn percentile(values: &[f64], p: f64) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.clamp(1, sorted.len().max(1)) - 1).copied()
}

/// A refresh fitted to display times: the period and the instant of one refresh.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    /// The period, in nanoseconds.
    period: f64,
    /// One refresh, in nanoseconds since the epoch.
    origin: f64,
}

/// The refresh fitted freely to `times` from `guess`: least squares of each time against
/// its refresh's index, the index each time's rounding by the period fitted so far. The
/// window starts at 32 guessed periods and doubles, so a guess off by a part in a hundred
/// still gives each time its own refresh. `None` for fewer than three times, or for times
/// that fall on one refresh. The guess is the display's reported refresh: from the gaps
/// of a run paced at the ticks no refresh can be guessed, since on a display faster than
/// them each gap is a multiple of it.
fn fit(times: &[u64], guess: f64) -> Option<Fit> {
    let (&first, &last) = (times.first()?, times.last()?);
    if times.len() < 3 || guess.is_nan() || guess <= 0.0 {
        return None;
    }
    let total = (last - first) as f64;
    let (mut period, mut origin) = (guess, 0.0);
    let mut span = (guess * 32.0).min(total);
    loop {
        let points: Vec<(f64, f64)> = times
            .iter()
            .map(|&t| (t - first) as f64)
            .take_while(|&x| x <= span)
            .map(|x| (((x - origin) / period).round(), x))
            .collect();
        let count = points.len() as f64;
        let (mean_n, mean_x) =
            points.iter().fold((0.0, 0.0), |(n, x), &(pn, px)| (n + pn / count, x + px / count));
        let (sxx, sxy) = points.iter().fold((0.0, 0.0), |(sxx, sxy), &(n, x)| {
            (sxx + (n - mean_n) * (n - mean_n), sxy + (n - mean_n) * (x - mean_x))
        });
        if sxx == 0.0 {
            return None;
        }
        period = sxy / sxx;
        origin = mean_x - period * mean_n;
        if span >= total {
            break;
        }
        span = (span * 2.0).min(total);
    }
    Some(Fit { period, origin: origin + first as f64 })
}

/// Each of `times`' phase against `fit`'s refresh, from 0 at a refresh to 1 at the next.
fn phases(times: &[u64], fit: Fit) -> impl Iterator<Item = f64> {
    times.iter().map(move |&t| ((t as f64 - fit.origin) / fit.period).rem_euclid(1.0))
}

/// The circular mean of `phases`, from 0 to 1, `None` for none or for phases spread evenly.
fn mean_phase(phases: impl Iterator<Item = f64>) -> Option<f64> {
    let (sin, cos) = phases
        .fold((0.0, 0.0), |(s, c), phase: f64| (s + (phase * TAU).sin(), c + (phase * TAU).cos()));
    (sin != 0.0 || cos != 0.0).then(|| (sin.atan2(cos) / TAU).rem_euclid(1.0))
}

/// How many times `latches`' phase against `fit`'s refresh went round, unwrapped from one
/// latch to the next: each step taken as its distance to the nearest whole refresh.
fn sweeps(latches: &[u64], fit: Fit) -> f64 {
    let phase: Vec<f64> = phases(latches, fit).collect();
    let total: f64 = phase
        .windows(2)
        .map(|pair| {
            let step = pair[1] - pair[0];
            step - step.round()
        })
        .sum();
    total.abs()
}

/// How far `times` lie from one phase of a refresh of `period`, as the P99 and the greatest
/// distance of each from their circular mean phase, in nanoseconds. `None` for no times.
fn grid(times: &[u64], period: f64) -> Option<(f64, f64)> {
    let first = *times.first()? as f64;
    let at = Fit { period, origin: first };
    let mean = mean_phase(phases(times, at)).unwrap_or(0.0);
    let distances: Vec<f64> = phases(times, at)
        .map(|phase| {
            let d = (phase - mean).rem_euclid(1.0);
            d.min(1.0 - d) * period
        })
        .collect();
    Some((percentile(&distances, 0.99)?, distances.iter().copied().fold(0.0, f64::max)))
}

/// D162's grid test of `times` against the display's `reported` refresh period: the period
/// refined by the free fit `fitted`, moved [`GRID_BOUND`] at most from the reported one, and
/// the test held where 99 in 100 of the times lie within a tenth of that period of one
/// phase; refused, at the bound, where the free fit lands past it. The period the test
/// used, the P99 and greatest distances, and the verdict.
fn grid_test(times: &[u64], reported: f64, fitted: Fit) -> Option<(f64, f64, f64, String)> {
    let ratio = fitted.period / reported - 1.0;
    let period = reported * (1.0 + ratio.clamp(-GRID_BOUND, GRID_BOUND));
    let (p99, max) = grid(times, period)?;
    let verdict = if ratio.abs() > GRID_BOUND {
        format!(
            "refused: the free fit lands {:.1} parts in 10,000 from the reported refresh, past \
             the bound of 5",
            ratio * 1e4
        )
    } else if p99 <= period / 10.0 {
        "held: 99 in 100 of the times lie within a tenth of the period of one phase".to_owned()
    } else {
        "refused: more than 1 in 100 of the times lie past a tenth of the period from one phase"
            .to_owned()
    };
    Some((period, p99, max, verdict))
}

/// The time scale in thousandths whose frames are a thousandth slower than half `rate_hz`,
/// for a tick period of `tick_ns` at a scale of one, rounded down: the floor run's.
#[must_use]
pub fn floor_scale(rate_hz: f64, tick_ns: u64) -> u32 {
    let half = rate_hz / 2.0 * 0.999;
    let scale = half * tick_ns as f64 / 1e9;
    (scale * 1000.0).floor().max(1.0) as u32
}

/// The refresh period a floor run is set from, in nanoseconds: the display mode's,
/// `mode_period`, where its rate is read and the real-time run's swapchain reported display
/// times (`reports`). The refresh is never fitted to the real-time run's display times: its
/// frames are paced at the ticks, and on a display faster than them each gap is a multiple
/// of the refresh.
///
/// # Errors
/// Why there is no floor, as the file states it.
pub fn floor_period(mode_period: Option<f64>, reports: bool) -> Result<f64, String> {
    match (reports, mode_period) {
        (false, _) => Err("no floor: the swapchain reports no display times".to_owned()),
        (true, None) => Err("no floor: the display's rate is unread".to_owned()),
        (true, Some(period)) => Ok(period),
    }
}

/// A record: the job's own thread from its start to `draw`'s return, and the closures
/// entered inside it. Its greatest width, the most threads inside it at one moment, and its
/// longest closure.
fn width(job: Job, drew: u64, closures: &[Closure]) -> (u32, Option<u64>) {
    let inside: Vec<Closure> = closures
        .iter()
        .filter(|closure| closure.entered >= job.start && closure.entered <= drew)
        .copied()
        .collect();
    let longest = inside.iter().map(|closure| closure.left.saturating_sub(closure.entered)).max();
    // Each thread's spans merged first, so a closure on the job's own thread, or one inside
    // another, counts its thread once.
    let mut by_thread: HashMap<u32, Vec<(u64, u64)>> = HashMap::new();
    by_thread.entry(job.thread).or_default().push((job.start, drew));
    for closure in &inside {
        by_thread.entry(closure.thread).or_default().push((closure.entered, closure.left));
    }
    let mut edges = Vec::new();
    for spans in by_thread.values_mut() {
        spans.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for &(start, end) in spans.iter() {
            match merged.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        for (start, end) in merged {
            edges.push((start, 1i32));
            edges.push((end, -1i32));
        }
    }
    // At one instant an end is taken before a start: spans that only touch do not overlap.
    edges.sort_unstable();
    let (mut now, mut most) = (0i32, 0i32);
    for (_, edge) in edges {
        now += edge;
        most = most.max(now);
    }
    (most.unsigned_abs(), longest)
}

/// What a run's figures are read against.
#[derive(Clone, Copy, Debug, Default)]
pub struct Against {
    /// The lateness of a tick's start the host's criterion allows at P99, in nanoseconds:
    /// D144's 1 ms on Windows, D156's 50 µs on macOS; `None` elsewhere.
    pub lateness: Option<u64>,
    /// The floor run's median and P99 of the latch's age at the display, where it swept.
    pub floor: Option<(f64, f64)>,
    /// The display's reported refresh period, in nanoseconds, where it is read.
    pub refresh: Option<f64>,
}

fn us(ns: f64) -> String {
    format!("{:.1}", ns / 1000.0)
}

/// Pushes `name.p50_us`, `name.p99_us` and, with `p90`, `name.p90_us` of `values`, or
/// `name` as `none` for none.
fn spread(figures: &mut Vec<(String, String)>, name: &str, values: &[f64], p90: bool) {
    let mut put = |suffix: &str, p: f64| {
        if let Some(value) = percentile(values, p) {
            figures.push((format!("{name}.{suffix}_us"), us(value)));
        }
    };
    if values.is_empty() {
        figures.push((name.to_owned(), "none".to_owned()));
        return;
    }
    put("p50", 0.50);
    if p90 {
        put("p90", 0.90);
    }
    put("p99", 0.99);
    put("max", 1.0);
}

/// What the frame before frame `at` did, by which its lateness is read: what it deferred
/// (`busy` waited to its limit); that it took a frame back with its call returning at or
/// after this frame's deadline less `margin`, `JOIN_MARGIN`, so that its join waited to its
/// limit (`joined_at_limit`); that it took one back sooner; or neither. With `margin`
/// unread, every frame taken back is `joined`.
fn before(frames: &[Frame], at: usize, margin: Option<u64>) -> &'static str {
    let Some(frame) = at.checked_sub(1).map(|before| &frames[before]) else { return "first" };
    let at_limit = margin.is_some_and(|margin| frame.returned + margin >= frames[at].deadline);
    match (frame.deferred.as_str(), frame.joined.as_str()) {
        ("busy", _) => "busy",
        ("waited", _) => "waited",
        ("torn", _) => "torn",
        (_, "-") => "nothing_joined",
        _ if at_limit => "joined_at_limit",
        _ => "joined",
    }
}

/// The circular mean of `latches`' phase against the refresh fitted to `stamps`' display
/// times from the reported period `refresh`, as a fraction of a refresh after a display;
/// why it is unread otherwise.
fn latch_phase(stamps: &Stamps, refresh: Option<f64>) -> String {
    let Some(refresh) = refresh else { return "unread: the display's rate is unread".to_owned() };
    let times = stamps.display_times();
    let Some(fitted) = fit(&times, refresh) else {
        return "unread: no refresh could be fitted to the display times".to_owned();
    };
    if (fitted.period / refresh - 1.0).abs() > GRID_BOUND {
        return "unread: the fit lands past five parts in 10,000 from the reported refresh"
            .to_owned();
    }
    mean_phase(phases(&stamps.latches(), fitted)).map_or_else(
        || "unread: the latches' phases spread evenly".to_owned(),
        |phase| format!("{phase:.3} of a refresh after a display"),
    )
}

/// The figures of one run, each name prefixed with `run.`, and the four readings of D162
/// where `against` gives what they read against.
#[must_use]
pub fn figures(run: &str, stamps: &Stamps, against: Against) -> Vec<(String, String)> {
    let mut f = Vec::new();
    let put = |f: &mut Vec<(String, String)>, name: &str, value: String| {
        f.push((format!("{run}.{name}"), value));
    };
    let name = |suffix: &str| format!("{run}.{suffix}");
    let frame_workers: Option<u32> = stamps.info_number("frame_worker_count");
    let margin: Option<u64> = stamps.info_number("join_margin_ns");
    let shed_depth: Option<u32> = stamps.info_number("shed_depth");
    let recorders = stamps.info("recorders") == Some("installed");
    for (key, value) in &stamps.info {
        put(&mut f, key, value.clone());
    }
    let shared = stamps.info("queues").is_some_and(|queues| queues.contains("present: Shared"));

    // Each paced frame: its lateness, split into the timer's wake, from the deadline to the
    // call, and the engine's start, from the call to the first tick.
    let frames = &stamps.frames;
    put(&mut f, "frames", frames.len().to_string());
    let lateness: Vec<(usize, f64)> = frames
        .iter()
        .enumerate()
        .filter_map(|(at, frame)| {
            Some((at, frame.first_tick?.saturating_sub(frame.deadline) as f64))
        })
        .collect();
    let late: Vec<f64> = lateness.iter().map(|&(_, l)| l).collect();
    spread(&mut f, &name("lateness"), &late, false);
    let wake = |frame: &Frame| frame.called.saturating_sub(frame.deadline) as f64;
    let wakes: Vec<f64> = frames.iter().map(wake).collect();
    spread(&mut f, &name("lateness.wake"), &wakes, false);
    let starts: Vec<f64> = frames
        .iter()
        .filter_map(|frame| Some(frame.first_tick?.saturating_sub(frame.called) as f64))
        .collect();
    spread(&mut f, &name("lateness.start"), &starts, false);
    let calls: Vec<f64> =
        frames.iter().map(|frame| frame.returned.saturating_sub(frame.called) as f64).collect();
    spread(&mut f, &name("frame_call"), &calls, false);
    let mut outcomes: Vec<(String, usize)> = Vec::new();
    for frame in frames {
        for (side, word) in [("joined", &frame.joined), ("deferred", &frame.deferred)] {
            if word != "-" {
                let key = format!("{side}.{word}");
                match outcomes.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, n)) => *n += 1,
                    None => outcomes.push((key, 1)),
                }
            }
        }
    }
    outcomes.sort();
    for (key, n) in outcomes {
        put(&mut f, &key, n.to_string());
    }
    put(&mut f, "behind", frames.iter().filter(|frame| frame.behind).count().to_string());
    if let Some(stats) = &stamps.stats {
        put(&mut f, "stats", stats.clone());
    }

    // Each record: the latest job begun before each `draw`'s return. Its width and longest
    // job are read from the recorders' layer alone.
    let mut records = Vec::new();
    let mut jobs = stamps.jobs.iter().peekable();
    let mut current = None;
    for &drew in &stamps.drews {
        while let Some(job) = jobs.next_if(|job| job.start <= drew) {
            current = Some(*job);
        }
        if let Some(job) = current.take() {
            let (wide, longest) = width(job, drew, &stamps.closures);
            records.push((job, drew, wide, longest));
        }
    }
    let durations: Vec<f64> =
        records.iter().map(|(job, drew, ..)| drew.saturating_sub(job.start) as f64).collect();
    spread(&mut f, &name("record"), &durations, false);
    let full = |wide: u32| recorders && frame_workers.is_some_and(|n| wide >= n);
    if recorders {
        let widest = records.iter().map(|&(_, _, wide, _)| wide).max();
        put(&mut f, "width.max", widest.map_or_else(|| "none".to_owned(), |w| w.to_string()));
        let at_full = records.iter().filter(|&&(_, _, wide, _)| full(wide)).count();
        put(&mut f, "width.at_frame_worker_count", at_full.to_string());
        let longest: Vec<f64> = records.iter().filter_map(|r| r.3).map(|ns| ns as f64).collect();
        spread(&mut f, &name("longest_job"), &longest, false);
    } else {
        let unread = "unread: the stamps carry no recorder layer".to_owned();
        put(&mut f, "width", unread.clone());
        put(&mut f, "longest_job", unread);
    }
    put(&mut f, "closures", stamps.closures.len().to_string());

    // The submit, those that overlapped a present's driver call, and the hold on a shared
    // queue. The present calls are one thread's, so their driver calls are disjoint and in
    // order, and the last to begin before a submit's return is the one that can overlap it.
    let cost = |s: &Submit| s.returned.saturating_sub(s.entered) as f64;
    let submits: Vec<f64> = stamps.submits.iter().map(cost).collect();
    spread(&mut f, &name("submit"), &submits, false);
    let mut spans: Vec<(u64, u64)> = stamps
        .presents
        .iter()
        .filter_map(|call| Some((call.driver_at?, call.driver_at? + call.driver)))
        .collect();
    spans.sort_unstable();
    let overlaps = |s: &&Submit| {
        let begun = spans.partition_point(|&(start, _)| start < s.returned);
        spans[..begun].last().is_some_and(|&(_, end)| end > s.entered)
    };
    let (overlapping, alone): (Vec<&Submit>, Vec<&Submit>) =
        stamps.submits.iter().partition(overlaps);
    let overlapping: Vec<f64> = overlapping.into_iter().map(cost).collect();
    let alone: Vec<f64> = alone.into_iter().map(cost).collect();
    put(&mut f, "submit.overlapping_driver", format!("{} of {}", overlapping.len(), submits.len()));
    spread(&mut f, &name("submit.overlapping"), &overlapping, false);
    spread(&mut f, &name("submit.alone"), &alone, false);
    let holds: Vec<f64> = stamps
        .submits
        .iter()
        .filter_map(|s| {
            let drew = stamps.drews.iter().rev().find(|&&drew| drew <= s.entered)?;
            Some(s.entered.saturating_sub(*drew) as f64)
        })
        .collect();
    if shared {
        spread(&mut f, &name("hold"), &holds, false);
    }

    // Each present call, by its parts: its waits with the encoder's drop, the acquire, the
    // record, the driver's call, and the rest.
    put(&mut f, "present.calls", stamps.presents.len().to_string());
    let presented: Vec<&Present> = stamps.presents.iter().filter(|c| c.id.is_some()).collect();
    put(&mut f, "present.presented", presented.len().to_string());
    let part = |get: fn(&Present) -> u64| -> Vec<f64> {
        stamps.presents.iter().map(|c| get(c) as f64).collect()
    };
    let (waits, acquire) = (part(|c| c.waits), part(|c| c.acquire));
    let (record, driver) = (part(|c| c.record), part(|c| c.driver));
    let call = part(|c| c.returned.saturating_sub(c.began));
    let rest = part(|c| {
        c.returned.saturating_sub(c.began).saturating_sub(c.waits + c.acquire + c.record + c.driver)
    });
    spread(&mut f, &name("present.waits"), &waits, true);
    spread(&mut f, &name("present.acquire"), &acquire, true);
    spread(&mut f, &name("present.record"), &record, true);
    spread(&mut f, &name("present.driver"), &driver, true);
    spread(&mut f, &name("present.rest"), &rest, true);
    spread(&mut f, &name("present.call"), &call, true);
    let reported: Vec<u32> = stamps.presents.iter().filter_map(|c| c.waiting).collect();
    let most_waiting = reported.iter().copied().max();
    let share = |count: usize, of: usize| {
        format!("{:.1}", if of == 0 { 0.0 } else { 100.0 * count as f64 / of as f64 })
    };
    put(&mut f, "waiting.reported", reported.len().to_string());
    put(&mut f, "waiting.max", most_waiting.map_or_else(|| "none".to_owned(), |w| w.to_string()));
    let two = reported.iter().filter(|&&w| w >= 2).count();
    put(&mut f, "waiting.two_or_more_percent", share(two, reported.len()));
    let images: Vec<String> = stamps.images.iter().map(u32::to_string).collect();
    put(
        &mut f,
        "images",
        if images.is_empty() { "none".to_owned() } else { images.join(" then ") },
    );
    let source = stamps.reports_display_times();
    put(&mut f, "display_times", if source { "reported" } else { "none" }.to_owned());
    stamps.display_figures(&mut |key, value| put(&mut f, key, value));

    // The presents made and not yet displayed as each call began, by their own display
    // times: `waiting` also counts those displayed and not yet reported.
    let undisplayed = if source {
        let (kept, _) = stamps.displays();
        let shown: HashMap<u64, u64> = kept.into_iter().collect();
        let made: Vec<(u64, u64)> =
            presented.iter().filter_map(|c| Some((c.returned, *shown.get(&c.id?)?))).collect();
        let counts: Vec<u32> = stamps
            .presents
            .iter()
            .map(|c| {
                let pending = made.iter().filter(|&&(back, at)| back <= c.began && at > c.began);
                u32::try_from(pending.count()).unwrap_or(u32::MAX)
            })
            .collect();
        let most = counts.iter().copied().max();
        put(&mut f, "undisplayed.max", most.map_or_else(|| "none".to_owned(), |n| n.to_string()));
        let two = counts.iter().filter(|&&n| n >= 2).count();
        put(&mut f, "undisplayed.two_or_more_percent", share(two, counts.len()));
        most
    } else {
        put(&mut f, "undisplayed", "unread: no display times".to_owned());
        None
    };

    // The latch's age.
    let latched: HashMap<u64, u64> = stamps.submits.iter().map(|s| (s.point, s.latched)).collect();
    let at_return: Vec<f64> = presented
        .iter()
        .filter_map(|c| Some(c.returned.saturating_sub(*latched.get(&c.point)?) as f64))
        .collect();
    spread(&mut f, &name("age_return"), &at_return, false);
    let ages = stamps.display_ages();
    let cut = against.floor.map_or(0.0, |(median, _)| median / 2.0);
    let (kept, dropped): (Vec<f64>, Vec<f64>) =
        ages.iter().map(|&age| age as f64).partition(|&age| age >= cut);
    spread(&mut f, &name("age_display"), &kept, false);
    put(&mut f, "age_display.dropped_below_half_floor", dropped.len().to_string());
    let phase = latch_phase(stamps, against.refresh);
    put(&mut f, "latch_phase", phase.clone());

    // The flag: a device with display times whose count stayed at one or none through
    // present calls that blocked.
    let p90 = |values: &[f64]| percentile(values, 0.9).unwrap_or(0.0);
    let blocked = p90(&acquire) > 1e6 || p90(&driver) > 1e6;
    let flagged = source && most_waiting.unwrap_or(0) <= 1 && blocked;
    put(
        &mut f,
        "flag.waiting_stayed_low",
        if flagged {
            "FLAGGED: display times are reported, the greatest `waiting` stayed at one or none, \
             and the acquire or the driver's call took over 1 ms at P90"
                .to_owned()
        } else {
            "no".to_owned()
        },
    );

    // D162's four readings.
    let p99 = |values: &[f64]| percentile(values, 0.99);
    let layout = if shared { "on the queue the roles share" } else { "on two queues" };
    let count = format!("{} of {}", overlapping.len(), submits.len());
    let reading = match (p99(&overlapping), p99(&submits), p99(&driver)) {
        (_, None, _) | (_, _, None) => "unread: no submit or no driver call".to_owned(),
        (Some(over), _, Some(call)) if over >= call => format!(
            "a defect to report: {count} submits overlapped a present's driver call {layout}, \
             their P99 {} us as long as the driver call's {} us",
            us(over),
            us(call)
        ),
        (Some(over), _, Some(call)) => format!(
            "recorded: {count} submits overlapped a present's driver call {layout}, their P99 \
             {} us under the driver call's {} us",
            us(over),
            us(call)
        ),
        (None, Some(all), Some(call)) => format!(
            "recorded: no submit overlapped a present's driver call ({count}) {layout}: the \
             submit's P99 {} us beside the driver call's {} us is the submit's own",
            us(all),
            us(call)
        ),
    };
    put(&mut f, "reading.submit_against_driver", reading);
    let reading = if shared {
        match (percentile(&holds, 0.5), p99(&holds), p99(&driver)) {
            (Some(p50), Some(hold), Some(call)) => format!(
                "the hold's P50 {} us and P99 {} us beside the driver call's P99 {} us",
                us(p50),
                us(hold),
                us(call)
            ),
            _ => "unread: no hold or no driver call".to_owned(),
        }
    } else {
        "not read: the present role has a queue of its own".to_owned()
    };
    put(&mut f, "reading.hold_against_driver", reading);
    let reading = match (against.floor, percentile(&kept, 0.5)) {
        (None, _) => "unread: no floor".to_owned(),
        (Some(_), None) => "unread: no display time joined a latch".to_owned(),
        (Some((_, floor)), Some(median)) => {
            let over = undisplayed.zip(shed_depth).is_some_and(|(most, depth)| most > depth);
            let refresh = against.refresh.unwrap_or(f64::INFINITY);
            let base = format!(
                "the median {} us against the floor's P99 {} us, the latches {phase}, on which \
                 the median depends",
                us(median),
                us(floor)
            );
            if median > floor + refresh || over {
                format!(
                    "a finding: {base}{}{}",
                    if median > floor + refresh { "; more than a refresh above it" } else { "" },
                    if over { "; the most not yet displayed above SHED_DEPTH" } else { "" }
                )
            } else if median > floor {
                format!("recorded: {base}; above it: an image waits behind another")
            } else {
                format!("recorded: {base}; at or under it")
            }
        }
    };
    put(&mut f, "reading.latch_against_floor", reading);
    let busy = frames.iter().filter(|frame| frame.deferred == "busy").count();
    put(
        &mut f,
        "reading.busy_beside_record",
        format!(
            "{busy} busy beside the record's P99 {} us",
            p99(&durations).map_or_else(|| "none".to_owned(), us)
        ),
    );
    if let Some(limit) = against.lateness {
        let limit = limit as f64;
        let over: Vec<usize> =
            lateness.iter().filter(|&&(_, l)| l > limit).map(|&(at, _)| at).collect();
        let by_wake = over.iter().filter(|&&at| wake(&frames[at]) > limit).count();
        put(&mut f, "lateness.over_limit", format!("{} of {}", over.len(), lateness.len()));
        put(&mut f, "lateness.over_limit_by_the_wake", by_wake.to_string());
        let reading = match p99(&late) {
            None => "unread: no frame ran a tick".to_owned(),
            Some(p) if p <= limit => format!(
                "recorded: the P99 {} us is within the criterion's {} us; {} of {} frames over \
                 it, {by_wake} of them by the timer's wake alone",
                us(p),
                us(limit),
                over.len(),
                lateness.len()
            ),
            Some(p) => {
                // The late frames beside the record last begun before their first tick, and
                // after a frame that waited to its limit.
                let beside_full = over
                    .iter()
                    .filter(|&&at| {
                        let tick = frames[at].first_tick.unwrap_or(0);
                        records
                            .iter()
                            .rev()
                            .find(|(job, ..)| job.start <= tick)
                            .is_some_and(|r| full(r.2))
                    })
                    .count();
                let after_wait = over
                    .iter()
                    .filter(|&&at| matches!(before(frames, at, margin), "busy" | "joined_at_limit"))
                    .count();
                format!(
                    "a finding for the planner: the P99 {} us is over the criterion's {} us; {} \
                     of {} frames over it, {by_wake} of them by the timer's wake alone, {} and \
                     {after_wait} after a frame that waited to its limit",
                    us(p),
                    us(limit),
                    over.len(),
                    lateness.len(),
                    if recorders {
                        format!(
                            "{beside_full} beside a record as wide as the frame-critical workers"
                        )
                    } else {
                        "the width unread".to_owned()
                    },
                )
            }
        };
        put(&mut f, "reading.lateness_beside_width", reading);
    } else {
        put(
            &mut f,
            "reading.lateness_beside_width",
            "unread: the host has no criterion of a tick's start".to_owned(),
        );
    }
    // The lateness by what the frame before did.
    let mut groups: Vec<(&str, Vec<f64>)> = Vec::new();
    for &(at, l) in &lateness {
        let key = before(frames, at, margin);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, values)) => values.push(l),
            None => groups.push((key, vec![l])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(b.0));
    for (key, values) in groups {
        put(&mut f, &format!("lateness.after_{key}.count"), values.len().to_string());
        spread(&mut f, &name(&format!("lateness.after_{key}")), &values, false);
    }
    f
}

/// The floor run's reading: its figures, the floor where it swept twice or more, and the
/// period a free fit to its display times lands on, which a floor run made again is set
/// from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Floor {
    /// The figures, each name prefixed with the run's.
    pub figures: Vec<(String, String)>,
    /// The median and the P99 of the latch's age at the display, in nanoseconds, where the
    /// run swept twice or more.
    pub ages: Option<(f64, f64)>,
    /// The period fitted freely to the display times, in nanoseconds.
    pub period: Option<f64>,
}

/// The floor run's reading, its figures prefixed with `run.`: the refresh fitted freely to
/// its display times from the display's `reported` refresh period, how many times its
/// latches swept that fit, and D162's grid test against the reported period.
#[must_use]
pub fn floor_figures(run: &str, stamps: &Stamps, reported: f64) -> Floor {
    let mut floor = Floor::default();
    let f = &mut floor.figures;
    let mut put = |name: &str, value: String| f.push((format!("{run}.{name}"), value));
    stamps.display_figures(&mut put);
    let times = stamps.display_times();
    let Some(fitted) = fit(&times, reported) else {
        put("floor", "no floor: no refresh could be fitted to the display times".to_owned());
        return floor;
    };
    put("fit.hz", format!("{:.4}", 1e9 / fitted.period));
    put(
        "fit.from_reported_parts_per_10000",
        format!("{:.2}", (fitted.period / reported - 1.0) * 1e4),
    );
    if let Some((period, p99, max, verdict)) = grid_test(&times, reported, fitted) {
        put("grid.period_us", format!("{:.3}", period / 1000.0));
        put("grid.p99_us", us(p99));
        put("grid.max_us", us(max));
        put("grid.test", verdict);
    }
    floor.period = Some(fitted.period);
    let swept = sweeps(&stamps.latches(), fitted);
    put("sweeps", format!("{swept:.2}"));
    if swept < 2.0 {
        put("floor", format!("no floor: it swept {swept:.2} times, fewer than two"));
        return floor;
    }
    let ages: Vec<f64> = stamps.display_ages().iter().map(|&age| age as f64).collect();
    let median = percentile(&ages, 0.5).unwrap_or(0.0);
    let kept: Vec<f64> = ages.iter().copied().filter(|&age| age >= median / 2.0).collect();
    put("floor", "a floor".to_owned());
    put("age_display.dropped_below_half_median", (ages.len() - kept.len()).to_string());
    spread(f, &format!("{run}.age_display"), &kept, false);
    floor.ages = percentile(&kept, 0.5).zip(percentile(&kept, 0.99));
    floor
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    const MS: u64 = 1_000_000;
    const P60: f64 = 1e9 / 60.0;

    fn get<'a>(figures: &'a [(String, String)], name: &str) -> &'a str {
        figures.iter().find(|(n, _)| n == name).map_or("absent", |(_, v)| v.as_str())
    }

    /// A stamps file's head: two frame-critical workers, the queue layout, the recorders'
    /// layer, `JOIN_MARGIN` and `SHED_DEPTH`.
    fn head(shared: bool, recorders: bool) -> String {
        format!(
            "info frame_worker_count 2\ninfo queues QueueLayout {{ compute: Own, present: {} }}\n\
             info recorders {}\ninfo join_margin_ns 2000000\ninfo shed_depth 3\n\
             info tick_period_ns 16666666\n",
            if shared { "Shared" } else { "Own" },
            if recorders { "installed" } else { "absent" },
        )
    }

    /// One frame's stamps: its first tick `late` after its deadline and its call half that,
    /// its submit, its present call and that call's driver call, each from the deadline, its
    /// display time, where it has one, and the `waiting` its call reported.
    struct Shot {
        late: u64,
        submit: (u64, u64),
        call: (u64, u64),
        driver: (u64, u64),
        shown: Option<u64>,
        waiting: u32,
    }

    fn shot(text: &mut String, n: u64, at: u64, shot: &Shot) {
        let (s0, s1) = shot.submit;
        let (c0, c1) = shot.call;
        let (d0, d1) = shot.driver;
        let _ = writeln!(
            text,
            "frame {at} {} {} {} queued - 0",
            at + shot.late,
            at + shot.late / 2,
            at + shot.late + 500_000
        );
        let _ = writeln!(text, "job 0 {}", at + 600_000);
        let _ = writeln!(text, "drew {}", at + 900_000);
        let _ = writeln!(text, "submit {} {} {} {n}", at + s0, at + s0, at + s1);
        let _ = writeln!(
            text,
            "present {} 0 1000 2000 {} {} {} {} 1 {n} {n} 1",
            at + c0,
            d1 - d0,
            at + d0,
            at + c1,
            shot.waiting,
        );
        if let Some(shown) = shot.shown {
            let _ = writeln!(text, "displayed {n} {shown}");
        }
    }

    /// A frame that keeps its submit and its present call apart, shown 33 ms after it.
    fn apart(at: u64) -> Shot {
        Shot {
            late: 10_000,
            submit: (MS, MS + 700_000),
            call: (2 * MS, 3 * MS),
            driver: (2 * MS + 200_000, 2 * MS + 800_000),
            shown: Some(at + 33 * MS),
            waiting: 1,
        }
    }

    #[test]
    fn each_kind_of_stamp_is_read_and_an_unknown_one_refused() {
        let text = "info frame_worker_count 3\n\
                    info queues QueueLayout { compute: Own, present: Shared }\n\
                    frame 1000000 2000000 2000000 3000000 - busy 1\n\
                    job 4 5000000\n\
                    drew 6000000\n\
                    closure recorder 1 5000000 6000000\n\
                    submit 7000000 8000000 9000000 12\n\
                    present 10000000 1 2 4 3 10500000 11000000 - 1 12 40 1\n\
                    present 12000000 1 2 4 3 - 13000000 1 1 13 - 0\n\
                    displayed 40 30000000\n\
                    images 3\n\
                    stats drawn=1 shown=1\n";
        let stamps = Stamps::parse(text).expect("the stamps");
        assert_eq!(stamps.info("queues"), Some("QueueLayout { compute: Own, present: Shared }"));
        assert_eq!(stamps.frames[0].first_tick, Some(2 * MS));
        assert_eq!(stamps.frames[0].deferred, "busy");
        assert_eq!(stamps.presents[0].record, 4);
        assert_eq!(stamps.presents[0].driver_at, Some(10_500_000));
        assert_eq!(stamps.presents[0].waiting, None);
        assert_eq!(stamps.presents[0].id, Some(40));
        assert_eq!(stamps.presents[1].driver_at, None);
        assert_eq!(stamps.display_ages(), [22 * MS]);
        assert_eq!(stamps.stats.as_deref(), Some("drawn=1 shown=1"));
        assert!(Stamps::parse("frames 1").is_err());
        assert!(Stamps::parse("drew x").is_err());
    }

    #[test]
    fn a_percentile_is_by_nearest_rank() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&values, 0.5), Some(50.0));
        assert_eq!(percentile(&values, 0.99), Some(99.0));
        assert_eq!(percentile(&values, 1.0), Some(100.0));
        assert_eq!(percentile(&[7.0], 0.99), Some(7.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    /// Display times on a 60.0012 Hz grid, every second refresh, some skipped, fitted from
    /// the reported 60 Hz: the fit lands on the grid's period, not the report's, and the
    /// bounded grid test holds them.
    #[test]
    fn the_free_fit_finds_the_grid_from_a_reported_refresh_off_by_a_little() {
        let period = 1e9 / 60.0012;
        let times: Vec<u64> = (0..1800u64)
            .filter(|n| n % 7 != 3)
            .map(|n| (5e6 + (2 * n) as f64 * period).round() as u64)
            .collect();
        let fitted = fit(&times, P60).expect("a fit");
        assert!((fitted.period - period).abs() < 1.0, "{} against {period}", fitted.period);
        let (_, p99, max, verdict) = grid_test(&times, P60, fitted).expect("the grid test");
        assert!(max < 1_000.0, "{p99} {max}");
        assert!(verdict.starts_with("held"), "{verdict}");
        assert_eq!(fit(&times[..2], period), None);
    }

    /// The reviewers' probe of a drifting refresh: a skipped vsync every 97 frames, a frame
    /// with no display time every 50, and a refresh that drifts by 50 parts in a million over
    /// the run. The fit lands between the first and the last period and the grid test
    /// holds.
    #[test]
    fn a_drifting_refresh_with_skips_and_gaps_is_fitted_and_held() {
        let mut times = Vec::new();
        let (mut t, mut period) = (5.0 * MS as f64, P60);
        for n in 0..1800u64 {
            if n % 97 == 13 {
                t += period;
            }
            if n % 50 != 7 {
                times.push(t.round() as u64);
            }
            t += 2.0 * period;
            period += P60 * 50e-6 / 1800.0;
        }
        let fitted = fit(&times, P60).expect("a fit");
        assert!(fitted.period > P60 && fitted.period < period, "{}", fitted.period);
        let (_, _, _, verdict) = grid_test(&times, P60, fitted).expect("the grid test");
        assert!(verdict.starts_with("held"), "{verdict}");
    }

    /// A display 8.3 parts in a million slower than the 60 Hz it reports, over ten minutes:
    /// the reported period alone leaves the times past a tenth of a period from one phase,
    /// and the bounded refinement holds them.
    #[test]
    fn the_bounded_grid_holds_a_true_display_the_reported_period_alone_refuses() {
        let true_period = 1e9 / 59.9995;
        let times: Vec<u64> =
            (0..18_000u64).map(|k| (1e8 + 2.0 * k as f64 * true_period) as u64).collect();
        let (_, max) = grid(&times, P60).expect("the grid");
        assert!(max > P60 / 10.0, "{max}");
        let fitted = fit(&times, P60).expect("a fit");
        let (_, p99, _, verdict) = grid_test(&times, P60, fitted).expect("the grid test");
        assert!(verdict.starts_with("held"), "{verdict}: {p99}");
    }

    /// The reviewers' probe of times that follow the frames, a present wait's completions
    /// 9 ms after each latch, in a floor run at 499: the free fit lands on the frames' half
    /// period, past the bound, so the grid test refuses them, and the latches do not sweep
    /// that fit.
    #[test]
    fn times_that_follow_the_frames_are_refused_by_the_bounded_grid() {
        let mut text = head(false, true);
        let period = 16_666_666.0 / 0.499;
        let mut seed = 12_345u64;
        for n in 0..1797u64 {
            let at = (n as f64 * period) as u64;
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let shown = Some(at + 10 * MS + (seed >> 40) % 100_000);
            shot(&mut text, n, at, &Shot { shown, ..apart(at) });
        }
        let stamps = Stamps::parse(&text).expect("the stamps");
        let floor = floor_figures("floor", &stamps, P60);
        assert!(get(&floor.figures, "floor.grid.test").starts_with("refused: the free fit"));
        assert!(get(&floor.figures, "floor.floor").starts_with("no floor: it swept"));
        assert_eq!(floor.ages, None);
    }

    /// The reviewers' probe of a run shorter than a sweep: no floor, recorded as that, and
    /// the fitted period kept for the run made again.
    #[test]
    fn a_floor_run_shorter_than_a_sweep_is_no_floor_and_keeps_its_fit() {
        let mut text = head(false, true);
        let period = 16_666_666.0 / 0.499;
        for n in 0..150u64 {
            let at = (n as f64 * period) as u64;
            // Displayed at the first refresh 8 ms or more after the latch.
            let shown = (((at + 9 * MS) as f64 / P60).ceil() * P60) as u64;
            shot(&mut text, n, at, &Shot { shown: Some(shown), ..apart(at) });
        }
        let stamps = Stamps::parse(&text).expect("the stamps");
        let floor = floor_figures("floor", &stamps, P60);
        assert!(get(&floor.figures, "floor.floor").starts_with("no floor: it swept"));
        assert_eq!(floor.ages, None);
        assert!(floor.period.is_some_and(|fitted| (fitted - P60).abs() < 10.0));
    }

    /// The reviewers' probe of a display of 120 Hz whose rate is unread, as off macOS: the
    /// real-time run's frames show every second refresh, and no floor is made from them.
    #[test]
    fn a_display_whose_rate_is_unread_has_no_floor() {
        assert_eq!(
            floor_period(None, true),
            Err("no floor: the display's rate is unread".to_owned())
        );
        assert!(floor_period(Some(P60), false).is_err());
        assert_eq!(floor_period(Some(1e9 / 120.0), true), Ok(1e9 / 120.0));
        let mut text = head(false, true);
        for n in 0..600u64 {
            let at = n * 16_666_666;
            let shown = (((at + 20 * MS) as f64 / (1e9 / 120.0)).ceil() * (1e9 / 120.0)) as u64;
            shot(&mut text, n, at, &Shot { shown: Some(shown), ..apart(at) });
        }
        let stamps = Stamps::parse(&text).expect("the stamps");
        let figures = figures("real", &stamps, Against::default());
        assert_eq!(get(&figures, "real.latch_phase"), "unread: the display's rate is unread");
        assert_eq!(get(&figures, "real.reading.latch_against_floor"), "unread: no floor");
    }

    /// The reviewers' probe of a skipped vsync and a present with no display time in a
    /// real-time run: the late one is kept, the missing one counted, and a report earlier than
    /// its present's return dropped and counted by that order.
    #[test]
    fn a_skipped_vsync_a_missing_display_time_and_one_before_its_return() {
        let mut text = head(false, true);
        for n in 0..600u64 {
            let at = n * 16_666_667;
            let shown = match n {
                0 => Some(at + 2 * MS + 500_000),
                100 => None,
                200 => Some(at + 33 * MS + 16_666_667),
                _ => Some(at + 33 * MS),
            };
            shot(&mut text, n, at, &Shot { shown, ..apart(at) });
        }
        let stamps = Stamps::parse(&text).expect("the stamps");
        let against =
            Against { refresh: Some(P60), floor: Some((32.0e6, 33.5e6)), ..Default::default() };
        let figures = figures("real", &stamps, against);
        assert_eq!(get(&figures, "real.display.kept"), "598");
        assert_eq!(get(&figures, "real.display.dropped_before_return"), "1");
        assert_eq!(get(&figures, "real.display.missing"), "2");
        assert_eq!(get(&figures, "real.age_display.dropped_below_half_floor"), "0");
        assert_eq!(get(&figures, "real.age_display.max_us"), "48666.7");
        assert!(get(&figures, "real.latch_phase").ends_with("of a refresh after a display"));
    }

    /// The reviewers' probes of reading 1: submits as long as the driver call that overlap
    /// none are recorded as their own, and a few that waited inside a driver call on the
    /// shared queue are the defect, though the P99 of all the submits stays at their cost.
    #[test]
    fn reading_one_reads_the_submits_that_overlapped_a_driver_call() {
        let mut text = head(false, true);
        for n in 0..600u64 {
            let at = n * 16_666_666;
            shot(&mut text, n, at, &apart(at));
        }
        let stamps = Stamps::parse(&text).expect("the stamps");
        let figures = figures("real", &stamps, Against::default());
        let reading = get(&figures, "real.reading.submit_against_driver");
        assert!(reading.starts_with("recorded: no submit overlapped"), "{reading}");
        assert_eq!(get(&figures, "real.submit.overlapping_driver"), "0 of 600");

        let mut text = head(true, true);
        for n in 0..1000u64 {
            let at = n * 16_666_666;
            let mut frame = apart(at);
            frame.submit = (MS, MS + 300_000);
            frame.driver = (2 * MS, 3 * MS);
            if n % 200 == 7 {
                // Entered inside the driver call, and returned 300 us after it.
                frame.submit = (2 * MS + 100_000, 3 * MS + 300_000);
            }
            shot(&mut text, n, at, &frame);
        }
        let stamps = Stamps::parse(&text).expect("the stamps");
        let figures = super::figures("real", &stamps, Against::default());
        let reading = get(&figures, "real.reading.submit_against_driver");
        assert!(reading.starts_with("a defect to report: 5 of 1000"), "{reading}");
        assert_eq!(get(&figures, "real.submit.p99_us"), "300.0");
    }

    /// The reviewers' probe of the lateness: one frame of 600 at 51 us is counted, and the
    /// reading is by the P99, within D156's 50 us; the wake and the start are split.
    #[test]
    fn the_lateness_is_judged_at_its_p99_and_split_into_the_wake_and_the_start() {
        let mut text = head(false, true);
        for n in 0..600u64 {
            let at = n * 16_666_666;
            let late = if n == 300 { 51_000 } else { 20_000 };
            shot(&mut text, n, at, &Shot { late, ..apart(at) });
        }
        let stamps = Stamps::parse(&text).expect("the stamps");
        let figures =
            figures("real", &stamps, Against { lateness: Some(50_000), ..Default::default() });
        assert_eq!(get(&figures, "real.lateness.p99_us"), "20.0");
        assert_eq!(get(&figures, "real.lateness.over_limit"), "1 of 600");
        assert_eq!(get(&figures, "real.lateness.over_limit_by_the_wake"), "0");
        assert_eq!(get(&figures, "real.lateness.wake.max_us"), "25.5");
        assert_eq!(get(&figures, "real.lateness.start.p50_us"), "10.0");
        let reading = get(&figures, "real.reading.lateness_beside_width");
        assert!(reading.starts_with("recorded: the P99 20.0 us is within"), "{reading}");
    }

    /// A frame before whose call returned at or after the next deadline less `JOIN_MARGIN`
    /// waited to its limit; one that returned sooner did not.
    #[test]
    fn the_frame_before_is_classed_by_its_return() {
        let frame = |deadline: u64, returned: u64| Frame {
            deadline,
            first_tick: Some(deadline + 10_000),
            called: deadline,
            returned,
            joined: "queued".to_owned(),
            deferred: "-".to_owned(),
            behind: false,
        };
        let period = 16_666_666;
        let frames = [frame(0, 15 * MS), frame(period, period + MS), frame(2 * period, 2 * period)];
        assert_eq!(before(&frames, 0, Some(2 * MS)), "first");
        assert_eq!(before(&frames, 1, Some(2 * MS)), "joined_at_limit");
        assert_eq!(before(&frames, 2, Some(2 * MS)), "joined");
        assert_eq!(before(&frames, 1, None), "joined");
    }

    /// A present displayed before the next call begins and reported only at it is no image
    /// waiting: `waiting` counts it, the count by display times does not.
    #[test]
    fn a_present_displayed_and_not_yet_reported_is_not_counted_as_waiting() {
        let mut text = head(false, true);
        for n in 0..100u64 {
            let at = n * 16_666_666;
            // Shown 15.7 ms after the latch, under a millisecond before the next call.
            let shown = Some(at + 16 * MS + 700_000);
            shot(&mut text, n, at, &Shot { shown, waiting: 2, ..apart(at) });
        }
        let stamps = Stamps::parse(&text).expect("the stamps");
        let figures = figures("real", &stamps, Against::default());
        assert_eq!(get(&figures, "real.waiting.max"), "2");
        assert_eq!(get(&figures, "real.undisplayed.max"), "0");
    }

    /// Without the recorders' layer the width and the longest job are unread.
    #[test]
    fn the_width_is_unread_without_the_recorders_layer() {
        let mut text = head(false, false);
        shot(&mut text, 0, 0, &apart(0));
        let stamps = Stamps::parse(&text).expect("the stamps");
        let figures = figures("real", &stamps, Against::default());
        assert!(get(&figures, "real.width").starts_with("unread"));
        assert_eq!(get(&figures, "real.width.max"), "absent");
    }

    /// Latches a little over two refreshes apart go round the refresh's phase once in each
    /// 250 frames; a run of 1,000 frames sweeps four times.
    #[test]
    fn the_sweeps_count_the_phase_going_round() {
        let fitted = Fit { period: 16.0 * MS as f64, origin: 0.0 };
        let latches: Vec<u64> = (0..=1000u64).map(|n| n * (32 * MS + 64_000) + 3 * MS).collect();
        let swept = sweeps(&latches, fitted);
        assert!((swept - 4.0).abs() < 1e-6, "{swept}");
        let still: Vec<u64> = (0..100u64).map(|n| n * 32 * MS).collect();
        assert!(sweeps(&still, fitted) < 1e-9);
    }

    #[test]
    fn the_grid_measures_the_spread_about_one_phase() {
        let period = 16.0 * MS as f64;
        // Alternating 200 us either side of one phase, across the wrap of the first.
        let times: Vec<u64> = (0..100u64)
            .map(|n| 1_000 * MS + n * 16 * MS + if n % 2 == 0 { 0 } else { 400_000 })
            .collect();
        let (_, max) = grid(&times, period).expect("the grid");
        assert!((max - 200_000.0).abs() < 1.0, "{max}");
    }

    #[test]
    fn the_floor_scale_is_a_thousandth_slower_than_half_the_rate_rounded_down() {
        // The tick period truncated to whole nanoseconds, as `TICK_PERIOD` is: its rate is a
        // little over 60 Hz, so 0.999 at 120 Hz rounds down to 998.
        let tick = 1_000_000_000 / 60;
        assert_eq!(floor_scale(60.0, tick), 499);
        assert_eq!(floor_scale(120.0, tick), 998);
        assert_eq!(floor_scale(59.9, tick), 498);
    }

    #[test]
    fn a_record_is_as_wide_as_the_threads_inside_it_at_once() {
        let job = Job { thread: 0, start: 0 };
        let closures = [
            // On the job's own thread: counted once.
            Closure { thread: 0, entered: 1, left: 2 },
            Closure { thread: 1, entered: 1, left: 5 },
            Closure { thread: 2, entered: 3, left: 6 },
            // Touching thread 2's end: no overlap.
            Closure { thread: 3, entered: 6, left: 7 },
            // Outside the record.
            Closure { thread: 4, entered: 20, left: 30 },
        ];
        assert_eq!(width(job, 10, &closures), (3, Some(4)));
        assert_eq!(width(job, 10, &[]), (1, None));
    }

    /// A run whose every reading fires: the submits inside the driver calls on a shared
    /// queue, the waiting low through blocking calls, the median far above the floor, and
    /// the lateness over the criterion at P99 beside records as wide as the workers.
    #[test]
    fn the_readings_name_a_defect_and_a_finding() {
        let mut stamps = Stamps::default();
        for (name, value) in [
            ("frame_worker_count", "2"),
            ("queues", "QueueLayout { compute: Own, present: Shared }"),
            ("recorders", "installed"),
            ("shed_depth", "3"),
        ] {
            stamps.info.push((name.to_owned(), value.to_owned()));
        }
        for n in 0..10u64 {
            let at = n * 20 * MS;
            stamps.frames.push(Frame {
                deadline: at,
                first_tick: Some(at + 200_000),
                called: at,
                returned: at + MS,
                joined: "queued".to_owned(),
                deferred: "-".to_owned(),
                behind: false,
            });
            stamps.jobs.push(Job { thread: 0, start: at + MS });
            stamps.closures.push(Closure { thread: 1, entered: at + MS, left: at + 3 * MS });
            stamps.drews.push(at + 4 * MS);
            stamps.submits.push(Submit {
                entered: at + 5 * MS,
                latched: at + 5 * MS,
                returned: at + 7 * MS,
                point: n,
            });
            stamps.presents.push(Present {
                began: at + 4 * MS,
                waits: 0,
                acquire: 2 * MS,
                record: 0,
                driver: MS,
                driver_at: Some(at + 6 * MS),
                returned: at + 11 * MS,
                waiting: Some(1),
                source: true,
                point: n,
                id: Some(n),
            });
            stamps.displayed.push((n, at + 40 * MS));
        }
        let against = Against {
            lateness: Some(50_000),
            floor: Some((10.0 * MS as f64, 12.0 * MS as f64)),
            refresh: Some(16.0 * MS as f64),
        };
        let figures = figures("real", &stamps, against);
        assert!(get(&figures, "real.reading.submit_against_driver").starts_with("a defect"));
        assert!(get(&figures, "real.flag.waiting_stayed_low").starts_with("FLAGGED"));
        assert!(get(&figures, "real.reading.latch_against_floor").starts_with("a finding"));
        let lateness = get(&figures, "real.reading.lateness_beside_width");
        assert!(lateness.starts_with("a finding for the planner"), "{lateness}");
        assert!(lateness.contains("9 beside a record as wide"), "{lateness}");
        assert_eq!(get(&figures, "real.width.max"), "2");
        assert_eq!(get(&figures, "real.lateness.p99_us"), "200.0");
        assert_eq!(get(&figures, "real.present.rest.p50_us"), "4000.0");
        assert_eq!(get(&figures, "real.age_display.p50_us"), "35000.0");
        assert!(get(&figures, "real.reading.hold_against_driver").starts_with("the hold's P50"));
    }
}
