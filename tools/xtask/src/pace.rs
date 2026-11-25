//! `frame-pace`'s reading (`README.md`, "frame-pace"): the stamps file the client binary
//! writes under `--stamps` (`crates/app-client/README.md`, "Stamps") read into the figures of
//! the milestone 2 plan's row for `frame-pace`, with the refresh fitted freely to the display
//! times, the floor run's sweeps and the grid test. Every time is in nanoseconds since the
//! file's epoch; every figure is printed in microseconds.

use std::collections::HashMap;
use std::f64::consts::TAU;

/// The present side's count at which a frame is shed, `lactose-app`'s `SHED_DEPTH`.
pub const SHED_DEPTH: u32 = 3;

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
    driver: u64,
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
                    driver: number(next(), line)?,
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

    /// Whether the swapchain had a source of display times at any present call.
    #[must_use]
    pub fn reports_display_times(&self) -> bool {
        self.presents.iter().any(|call| call.source)
    }

    /// The display times reported, in order.
    #[must_use]
    pub fn display_times(&self) -> Vec<u64> {
        let mut times: Vec<u64> = self.displayed.iter().map(|&(_, at)| at).collect();
        times.sort_unstable();
        times
    }

    /// The tick period the run was paced by, at a time scale of one.
    #[must_use]
    pub fn tick_period(&self) -> Option<u64> {
        self.info("tick_period_ns")?.parse().ok()
    }

    /// The latches of the frames submitted, in order.
    fn latches(&self) -> Vec<u64> {
        self.submits.iter().map(|submit| submit.latched).collect()
    }

    /// The latch's age at the display for each present displayed, in order of the reports.
    fn display_ages(&self) -> Vec<u64> {
        let latched: HashMap<u64, u64> =
            self.submits.iter().map(|submit| (submit.point, submit.latched)).collect();
        let point: HashMap<u64, u64> =
            self.presents.iter().filter_map(|call| Some((call.id?, call.point))).collect();
        self.displayed
            .iter()
            .filter_map(|(id, at)| Some(at.saturating_sub(*latched.get(point.get(id)?)?)))
            .collect()
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
pub struct Fit {
    /// The period, in nanoseconds.
    pub period: f64,
    /// One refresh, in nanoseconds since the epoch.
    pub origin: f64,
}

/// A period to start a fit from where the display mode's is unread: the 5th percentile of
/// the gaps between display times longer than a millisecond, which on a run at the display's
/// rate is one refresh.
#[must_use]
pub fn period_guess(times: &[u64]) -> Option<f64> {
    let gaps: Vec<f64> = times
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]) as f64)
        .filter(|&gap| gap > 1e6)
        .collect();
    percentile(&gaps, 0.05)
}

/// The refresh fitted freely to `times` from `guess`: least squares of each time against
/// its refresh's index, the index each time's rounding by the period fitted so far. The
/// window starts at 32 guessed periods and doubles, so a guess off by a part in a hundred
/// still gives each time its own refresh. `None` for fewer than three times, or for times
/// that fall on one refresh.
#[must_use]
pub fn fit(times: &[u64], guess: f64) -> Option<Fit> {
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

/// How many times `latches`' phase against `fit`'s refresh went round, unwrapped from one
/// latch to the next: each step taken as its distance to the nearest whole refresh.
#[must_use]
pub fn sweeps(latches: &[u64], fit: Fit) -> f64 {
    let phase = |t: u64| ((t as f64 - fit.origin) / fit.period).rem_euclid(1.0);
    let total: f64 = latches
        .windows(2)
        .map(|pair| {
            let step = phase(pair[1]) - phase(pair[0]);
            step - step.round()
        })
        .sum();
    total.abs()
}

/// The grid test: how far `times` lie from one phase of a refresh of `period`, as the P99
/// and the greatest distance of each from their circular mean phase, in nanoseconds. `None`
/// for no times.
#[must_use]
pub fn grid(times: &[u64], period: f64) -> Option<(f64, f64)> {
    let first = *times.first()? as f64;
    let angles: Vec<f64> =
        times.iter().map(|&t| ((t as f64 - first) / period).rem_euclid(1.0) * TAU).collect();
    let (sin, cos) = angles.iter().fold((0.0, 0.0), |(s, c), a| (s + a.sin(), c + a.cos()));
    let mean = sin.atan2(cos);
    let distances: Vec<f64> = angles
        .iter()
        .map(|a| {
            let d = (a - mean).rem_euclid(TAU);
            d.min(TAU - d) / TAU * period
        })
        .collect();
    Some((percentile(&distances, 0.99)?, distances.iter().copied().fold(0.0, f64::max)))
}

/// The time scale in thousandths whose frames are a thousandth slower than half `rate_hz`,
/// for a tick period of `tick_ns` at a scale of one, rounded down: the floor run's.
#[must_use]
pub fn floor_scale(rate_hz: f64, tick_ns: u64) -> u32 {
    let half = rate_hz / 2.0 * 0.999;
    let scale = half * tick_ns as f64 / 1e9;
    (scale * 1000.0).floor().max(1.0) as u32
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
    /// The lateness of a tick's start the host's criterion allows, in nanoseconds: D144's
    /// 1 ms on Windows, D156's 50 µs on macOS; `None` elsewhere.
    pub lateness: Option<u64>,
    /// The floor run's median and P99 of the latch's age at the display, where it swept.
    pub floor: Option<(f64, f64)>,
    /// The display's refresh period, in nanoseconds, where it is known.
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

/// The frame before's outcome a lateness is read by: what it deferred, or that it took a
/// frame back, or neither.
fn before(frame: &Frame) -> &'static str {
    match (frame.deferred.as_str(), frame.joined.as_str()) {
        ("busy", _) => "busy",
        ("waited", _) => "waited",
        ("torn", _) => "torn",
        (_, "-") => "nothing_joined",
        _ => "joined",
    }
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
    let frame_workers: Option<u32> = stamps.info("frame_worker_count").and_then(|n| n.parse().ok());
    for (key, value) in &stamps.info {
        put(&mut f, key, value.clone());
    }
    let shared = stamps.info("queues").is_some_and(|queues| queues.contains("present: Shared"));

    // Each paced frame.
    put(&mut f, "frames", stamps.frames.len().to_string());
    let lateness: Vec<(usize, f64)> = stamps
        .frames
        .iter()
        .enumerate()
        .filter_map(|(at, frame)| {
            Some((at, frame.first_tick?.saturating_sub(frame.deadline) as f64))
        })
        .collect();
    let late: Vec<f64> = lateness.iter().map(|&(_, l)| l).collect();
    spread(&mut f, &name("lateness"), &late, false);
    let calls: Vec<f64> = stamps
        .frames
        .iter()
        .map(|frame| frame.returned.saturating_sub(frame.called) as f64)
        .collect();
    spread(&mut f, &name("frame_call"), &calls, false);
    let mut outcomes: Vec<(String, usize)> = Vec::new();
    for frame in &stamps.frames {
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
    put(&mut f, "behind", stamps.frames.iter().filter(|frame| frame.behind).count().to_string());
    if let Some(stats) = &stamps.stats {
        put(&mut f, "stats", stats.clone());
    }

    // Each record: the latest job begun before each `draw`'s return.
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
    let widest = records.iter().map(|&(_, _, wide, _)| wide).max();
    put(&mut f, "width.max", widest.map_or_else(|| "none".to_owned(), |w| w.to_string()));
    let full = |wide: u32| frame_workers.is_some_and(|n| wide >= n);
    let at_full = records.iter().filter(|&&(_, _, wide, _)| full(wide)).count();
    put(&mut f, "width.at_frame_worker_count", at_full.to_string());
    let longest: Vec<f64> = records.iter().filter_map(|r| r.3).map(|ns| ns as f64).collect();
    spread(&mut f, &name("longest_job"), &longest, false);
    put(&mut f, "closures", stamps.closures.len().to_string());

    // The submit, and the hold on a shared queue.
    let submits: Vec<f64> =
        stamps.submits.iter().map(|s| s.returned.saturating_sub(s.entered) as f64).collect();
    spread(&mut f, &name("submit"), &submits, false);
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

    // Each present call.
    put(&mut f, "present.calls", stamps.presents.len().to_string());
    let presented: Vec<&Present> = stamps.presents.iter().filter(|c| c.id.is_some()).collect();
    put(&mut f, "present.presented", presented.len().to_string());
    let part = |get: fn(&Present) -> u64| -> Vec<f64> {
        stamps.presents.iter().map(|c| get(c) as f64).collect()
    };
    let (waits, acquire, driver) = (part(|c| c.waits), part(|c| c.acquire), part(|c| c.driver));
    spread(&mut f, &name("present.waits"), &waits, true);
    spread(&mut f, &name("present.acquire"), &acquire, true);
    spread(&mut f, &name("present.driver"), &driver, true);
    let call: Vec<f64> =
        stamps.presents.iter().map(|c| c.returned.saturating_sub(c.began) as f64).collect();
    spread(&mut f, &name("present.call"), &call, true);
    let reported: Vec<u32> = stamps.presents.iter().filter_map(|c| c.waiting).collect();
    let most_waiting = reported.iter().copied().max();
    put(&mut f, "waiting.reported", reported.len().to_string());
    put(&mut f, "waiting.max", most_waiting.map_or_else(|| "none".to_owned(), |w| w.to_string()));
    let two = reported.iter().filter(|&&w| w >= 2).count();
    let share = if reported.is_empty() { 0.0 } else { 100.0 * two as f64 / reported.len() as f64 };
    put(&mut f, "waiting.two_or_more_percent", format!("{share:.1}"));
    let images: Vec<String> = stamps.images.iter().map(u32::to_string).collect();
    put(
        &mut f,
        "images",
        if images.is_empty() { "none".to_owned() } else { images.join(" then ") },
    );
    let source = stamps.reports_display_times();
    put(&mut f, "display_times", if source { "reported" } else { "none" }.to_owned());

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
    let reading = match (p99(&submits), p99(&driver)) {
        (Some(submit), Some(call)) if submit >= call => format!(
            "a defect to report: the submit's P99 {} us is as long as the driver call's {} us",
            us(submit),
            us(call)
        ),
        (Some(submit), Some(call)) => {
            format!("the submit's P99 {} us is under the driver call's {} us", us(submit), us(call))
        }
        _ => "unread: no submit or no driver call".to_owned(),
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
            let over = most_waiting.is_some_and(|w| w > SHED_DEPTH);
            let refresh = against.refresh.unwrap_or(f64::INFINITY);
            let base =
                format!("the median {} us against the floor's P99 {} us", us(median), us(floor));
            if median > floor + refresh || over {
                format!(
                    "a finding: {base}{}{}",
                    if median > floor + refresh { ", more than a refresh above it" } else { "" },
                    if over { ", and the greatest `waiting` above SHED_DEPTH" } else { "" }
                )
            } else if median > floor {
                format!("recorded: {base}, above it: an image waits behind another")
            } else {
                format!("recorded: {base}, at or under it")
            }
        }
    };
    put(&mut f, "reading.latch_against_floor", reading);
    let busy = stamps.frames.iter().filter(|frame| frame.deferred == "busy").count();
    put(
        &mut f,
        "reading.busy_beside_record",
        format!(
            "{busy} busy beside the record's P99 {} us",
            p99(&durations).map_or_else(|| "none".to_owned(), us)
        ),
    );
    let reading = match against.lateness {
        None => "unread: the host has no criterion of a tick's start".to_owned(),
        Some(limit) => {
            // Each late frame beside the record last begun before its first tick, and by
            // what the frame before it did.
            let over: Vec<usize> =
                lateness.iter().filter(|&&(_, l)| l > limit as f64).map(|&(at, _)| at).collect();
            let beside_full = over
                .iter()
                .filter(|&&at| {
                    let tick = stamps.frames[at].first_tick.unwrap_or(0);
                    records
                        .iter()
                        .rev()
                        .find(|(job, ..)| job.start <= tick)
                        .is_some_and(|r| full(r.2))
                })
                .count();
            let mut by_before: Vec<(&str, usize)> = Vec::new();
            for &at in &over {
                let key = if at == 0 { "first" } else { before(&stamps.frames[at - 1]) };
                match by_before.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, n)) => *n += 1,
                    None => by_before.push((key, 1)),
                }
            }
            by_before.sort();
            let after: Vec<String> =
                by_before.iter().map(|(k, n)| format!("{n} after {k}")).collect();
            let base = format!(
                "{} of {} ticked frames late by over {} us ({})",
                over.len(),
                lateness.len(),
                us(limit as f64),
                if after.is_empty() { "none".to_owned() } else { after.join(", ") }
            );
            if beside_full > 0 {
                format!(
                    "a finding for the planner: {base}; {beside_full} beside a record as wide as the frame-critical workers"
                )
            } else if over.is_empty() {
                format!("recorded: {base}")
            } else {
                format!(
                    "a finding for the planner: {base}; none beside a record as wide as the frame-critical workers"
                )
            }
        }
    };
    put(&mut f, "reading.lateness_beside_width", reading);
    // The lateness by what the frame before did.
    let mut groups: Vec<(&str, Vec<f64>)> = Vec::new();
    for &(at, l) in &lateness {
        let key = if at == 0 { "first" } else { before(&stamps.frames[at - 1]) };
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
/// refresh fitted to its display times where one could be.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Floor {
    /// The figures, each name prefixed with the run's.
    pub figures: Vec<(String, String)>,
    /// The median and the P99 of the latch's age at the display, in nanoseconds, where the
    /// run swept twice or more.
    pub ages: Option<(f64, f64)>,
    /// The refresh fitted freely to the display times.
    pub fit: Option<Fit>,
}

/// The floor run's reading, its figures prefixed with `run.`: the refresh fitted freely to
/// its display times from `guess`, how many times its latches swept it, and the grid test
/// against the display mode's period, where it is read, and against the fitted one.
#[must_use]
pub fn floor_figures(run: &str, stamps: &Stamps, guess: f64, mode_period: Option<f64>) -> Floor {
    let mut floor = Floor::default();
    let f = &mut floor.figures;
    let put = |f: &mut Vec<(String, String)>, name: &str, value: String| {
        f.push((format!("{run}.{name}"), value));
    };
    let times = stamps.display_times();
    put(f, "display_times", times.len().to_string());
    let Some(fitted) = fit(&times, guess) else {
        put(f, "floor", "no floor: no refresh could be fitted to the display times".to_owned());
        return floor;
    };
    put(f, "fit.period_us", format!("{:.3}", fitted.period / 1000.0));
    put(f, "fit.hz", format!("{:.4}", 1e9 / fitted.period));
    let swept = sweeps(&stamps.latches(), fitted);
    put(f, "sweeps", format!("{swept:.2}"));
    match mode_period.and_then(|period| grid(&times, period)) {
        Some((p99, max)) => {
            put(f, "grid.p99_us", us(p99));
            put(f, "grid.max_us", us(max));
        }
        None => put(f, "grid", "unread: the display mode's rate is not read".to_owned()),
    }
    if let Some((p99, max)) = grid(&times, fitted.period) {
        put(f, "grid_fitted.p99_us", us(p99));
        put(f, "grid_fitted.max_us", us(max));
    }
    if swept < 2.0 {
        put(f, "floor", format!("no floor: it swept {swept:.2} times, fewer than two"));
        floor.fit = Some(fitted);
        return floor;
    }
    let ages: Vec<f64> = stamps.display_ages().iter().map(|&age| age as f64).collect();
    let median = percentile(&ages, 0.5).unwrap_or(0.0);
    let kept: Vec<f64> = ages.iter().copied().filter(|&age| age >= median / 2.0).collect();
    put(f, "floor", "a floor".to_owned());
    put(f, "age_display.dropped_below_half_median", (ages.len() - kept.len()).to_string());
    spread(f, &format!("{run}.age_display"), &kept, false);
    floor.ages = percentile(&kept, 0.5).zip(percentile(&kept, 0.99));
    floor.fit = Some(fitted);
    floor
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    #[test]
    fn each_kind_of_stamp_is_read_and_an_unknown_one_refused() {
        let text = "info frame_worker_count 3\n\
                    info queues QueueLayout { compute: Own, present: Shared }\n\
                    frame 1000000 2000000 2000000 3000000 - busy 1\n\
                    job 4 5000000\n\
                    drew 6000000\n\
                    closure recorder 1 5000000 6000000\n\
                    submit 7000000 8000000 9000000 12\n\
                    present 10000000 1 2 3 11000000 - 1 12 40 1\n\
                    displayed 40 30000000\n\
                    images 3\n\
                    stats drawn=1 shown=1\n";
        let stamps = Stamps::parse(text).expect("the stamps");
        assert_eq!(stamps.info("queues"), Some("QueueLayout { compute: Own, present: Shared }"));
        assert_eq!(stamps.frames[0].first_tick, Some(2 * MS));
        assert_eq!(stamps.frames[0].deferred, "busy");
        assert_eq!(stamps.presents[0].waiting, None);
        assert_eq!(stamps.presents[0].id, Some(40));
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

    /// Display times on a 60.0012 Hz grid, every second refresh, some skipped, started from a
    /// guess of exactly 60 Hz: the fit lands on the grid's period, not the guess's.
    #[test]
    fn the_free_fit_finds_the_grid_from_a_guess_off_by_a_little() {
        let period = 1e9 / 60.0012;
        let times: Vec<u64> = (0..1800u64)
            .filter(|n| n % 7 != 3)
            .map(|n| (5e6 + (2 * n) as f64 * period).round() as u64)
            .collect();
        let fitted = fit(&times, 1e9 / 60.0).expect("a fit");
        assert!((fitted.period - period).abs() < 1.0, "{} against {period}", fitted.period);
        let (p99, max) = grid(&times, period).expect("the grid test");
        assert!(max < 1_000.0, "{p99} {max}");
        assert!(period_guess(&times).is_some_and(|guess| (guess - 2.0 * period).abs() < 2.0));
        assert_eq!(fit(&times[..2], period), None);
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
    fn the_grid_test_measures_the_spread_about_one_phase() {
        let period = 16.0 * MS as f64;
        // Alternating 200 us either side of one phase, across the wrap of the first.
        let times: Vec<u64> = (0..100u64)
            .map(|n| 1_000 * MS + n * 16 * MS + if n % 2 == 0 { 0 } else { 400_000 })
            .collect();
        let (_, max) = grid(&times, period).expect("the grid test");
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

    #[test]
    fn the_readings_name_a_defect_and_a_finding() {
        let mut stamps = Stamps::default();
        stamps.info.push(("frame_worker_count".to_owned(), "2".to_owned()));
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
                began: at + 8 * MS,
                waits: 0,
                acquire: 2 * MS,
                driver: MS,
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
        let get = |name: &str| {
            figures.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()).unwrap_or("absent")
        };
        assert!(get("real.reading.submit_against_driver").starts_with("a defect"));
        assert!(get("real.flag.waiting_stayed_low").starts_with("FLAGGED"));
        assert!(get("real.reading.latch_against_floor").starts_with("a finding"));
        assert!(get("real.reading.lateness_beside_width").contains("9 beside a record as wide"));
        assert_eq!(get("real.width.max"), "2");
        assert_eq!(get("real.lateness.p99_us"), "200.0");
        assert_eq!(get("real.age_display.p50_us"), "35000.0");
        assert_eq!(get("real.reading.hold_against_driver").split(':').next(), Some("not read"));
    }
}
