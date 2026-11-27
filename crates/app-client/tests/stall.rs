//! `record-stall` (the milestone 2 plan's verification row 21, decision 5.16, section 4's
//! table; D162): what the physics phase costs a frame's record beside it. Milestone 1's
//! composed tick, `lactose-sim`'s cost scene of 100,000 rows with 2,000 awake bodies on
//! Rapier beside it, steps on N frame workers (N the performance cores up to eight) while
//! the test thread records frame after frame of the binary's render side, `ClientRender`,
//! on `lactose-rhi-null`: each record one job from `Executor::run_job`, as the shown frame
//! runs its own, which opens a region, writes the binary's seven instances and calls
//! `RenderSide::draw` (the remote batch, `prepare`, the declaration, compile and record of
//! the minimal renderer's passes), timed from the job's start to `draw`'s return; the
//! submit follows on the test thread. It runs at a physics reservation of one frame worker
//! and of two, the solver pool sized as `lactose-app` sizes it, N less the reservation and
//! at least one, and records nothing: `RECORD_RESERVATION` stays as it is until the file is
//! read.
//!
//! The physics phase of each tick is the span from the first entry into its three systems
//! (`physics_api::sync`, `step` and `write_back`) to the last exit: `step` from the
//! executor's node spans, and the two exclusive systems, which run as no node, from
//! `lactose-ecs`'s system spans, read by name. Every tick asserts that all three were seen. A
//! record that overlaps the phase is *beside* the physics phase; one that overlaps the tick
//! and not the phase is *beside the tick*; and the same records made with no tick running
//! are *alone*. The file gives the median, the 99th percentile and the worst of each, with
//! the same of each group's start wait, from `run_job`'s call to the job's start; and the
//! physics phase's and the tick's medians, the tick timed inside its bounded future.
//!
//! The gate's wake (D165, D162) is read from `lactose-task`'s two debug spans: `open_gate`,
//! from the end of the step's `block_in_place` closure to the gate's return, and `park`,
//! around each wait a frame worker makes without a permit, in the idle protocol or on a
//! scope's latch. A worker is *parked across* an opening when its `park` began before the
//! opening's start and ended after it, whether or not it slept: the idle wait returns without
//! sleeping when a waker has already claimed the worker, and the latch's when its re-check
//! after the span's callbacks finds a reason to go on. Either way the worker was waiting
//! without a permit when the opening began, and its exit may have another cause than the
//! opening, such as a latch set meanwhile. At each reservation the file gives the time from
//! each opening's start to the `park` exit of each worker parked across it, with the count of
//! each kind of wait; the opening's own length, on the opener's side, which bounds no wake;
//! for the openings after which a worker parked across it took the record's
//! next job, the first to start after the opening's start, that job's start from the
//! opening's start and its start wait; and the count of the openings after which a worker
//! that was awake took it, the opener among them. The time from the opening to that job's
//! start is mostly the test thread's own: records run back to back, each a `run_job` call,
//! the bound's keeping thread and the submit, so the next call comes when the record in
//! progress at the opening ends. The gate's figures are the wake and the start wait. A
//! figure of fewer than 100 samples gives its median and its worst and no 99th percentile.
//! On Windows the wake and the start wait show what the `SetEvent` for each frame worker that
//! `Pool::open_gate` makes costs (row 164's unmeasured list). Every tick asserts that it saw
//! an `open_gate` span, and each reservation that it saw `park` spans, so a build that
//! compiles debug spans out fails.
//!
//! 360 ticks at each reservation (6 in a short run) after 10 of warm-up, so that the wakes
//! number over a thousand on six frame workers.

#![allow(dead_code, reason = "the scene as `lactose-sim`'s cost test has it")]

use std::cell::Cell;
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lactose_app::{
    Draw, RECORD_RESERVATION, Region, RenderSide, TargetIndex, Workers, start_executor,
};
use lactose_app_client::{ClientRender, FIXED_EXTENT};
use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, PlayerId, Result, StableKey, ThreadClass, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Event, EventWriter, Events, Executor, LayoutConfig, Mut,
    RegionConfig, RegionId, RegionMode, RegionRes, Res, ResMut, StableKeyOf, SystemSet, Without,
    World, WorldKind, finalize, system,
};
use lactose_input_host::LatestSample;
use lactose_math::{ExactSum, Scalar, Vec3};
use lactose_physics_api::{Collider, PhysicsConfig, RigidBody, Shape, Transform};
use lactose_present_api::{ExtractPass, InstanceRow, RenderTransform};
use lactose_reflect::{component, resource};
use lactose_rhi_api::Capabilities;
use lactose_sim::{
    InputApplier, PlayerSample, Registration, Simulation, SimulationBuilder, Stage, TickDelta,
    TickInput, TypeTable, sets,
};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Metadata, Subscriber};

use crate::support::{bounded, lock, null_device, null_shaders};
use crate::timing::{enter, figure, measured_workers, median, micros, percentile, short, spread};

include!("../../sim/tests/cost/scene.rs");

const BODIES: u64 = 2_000;
/// The first key of the bodies: after the scene's rows and players.
const FIRST_BODY: u64 = 2_000_000;
/// The instances the binary's scene shows.
const INSTANCES: u64 = 7;
const WARM_UP: u32 = 10;
/// Under this many samples a figure gives its median and its worst, and no 99th percentile.
const PERCENTILE_SAMPLES: usize = 100;

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

/// A floor 80 metres square with its top at zero, and on it [`BODIES`] dynamic bodies that
/// never sleep, as `lactose-app`'s composed tick places them.
fn floor_and_bodies(world: &mut World, region: RegionId, first: u64) {
    let floor = Collider::new(Shape::cuboid(Vec3::new(s(40.0), s(1.0), s(40.0))))
        .with_offset(Vec3::new(Scalar::ZERO, s(-1.0), Scalar::ZERO));
    world.spawn(key(first), region, (Transform::IDENTITY, floor)).unwrap();
    let mut body = RigidBody::dynamic();
    body.can_sleep = false;
    for n in 0..BODIES {
        let (layer, cell) = (n / 1_000, n % 1_000);
        let at = Vec3::new(
            s((cell % 50) as f64 - 25.0),
            s(0.5 + layer as f64),
            s((cell / 50) as f64 - 10.0),
        );
        let shape = match n % 3 {
            0 => Shape::sphere(s(0.4)),
            1 => Shape::cuboid(Vec3::new(s(0.4), s(0.4), s(0.4))),
            _ => Shape::capsule(s(0.2), s(0.3)),
        };
        let row = (Transform::at(at), body, Collider::new(shape));
        world.spawn(key(first + 1 + n), region, row).unwrap();
    }
}

/// The composed tick's world on `workers` frame workers at a physics reservation of
/// `reserved`, its solver pool as `lactose-app` sizes it.
fn composed(workers: u32, reserved: u32) -> Simulation {
    let threads = workers.saturating_sub(reserved).max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads as usize)
        .thread_name(|index| format!("solver-{index}"))
        .start_handler(|_| enter(ThreadClass::PhysicsSolver))
        .build()
        .unwrap();
    let backend = lactose_physics_rapier::backend(pool);
    let config = PhysicsConfig { reserved_workers: reserved, ..PhysicsConfig::default() };
    let mut types = types();
    lactose_physics_api::register_types(&mut types).unwrap();
    let mut simulation = builder(ROWS, types)
        .setup(move |world| lactose_physics_api::setup(world, backend, config))
        .systems(lactose_physics_api::register_systems)
        .systems(|registration| {
            registration.order(&lactose_physics_api::sets::WRITE_BACK, &POST);
        })
        .build()
        .unwrap();
    let world = simulation.world_mut();
    populate(world, ROWS);
    floor_and_bodies(world, RegionId(1), FIRST_BODY);
    simulation
}

// ---- The physics phase, from the executor's spans ----

/// The physics systems, as the executor names their nodes and `lactose-ecs` their system
/// spans.
const PHYSICS: [&str; 3] = ["physics_api::sync", "physics_api::step", "physics_api::write_back"];

/// The id of a span of no physics system: every system span is seen, to read its name.
const OTHER: u64 = 1 + PHYSICS.len() as u64;
/// The ids of `lactose-task`'s `open_gate` span and of its `park` span at each wait.
const OPENING: u64 = OTHER + 1;
const PARK_IDLE: u64 = OTHER + 2;
const PARK_LATCH: u64 = OTHER + 3;

/// What the spans of one tick's physics systems showed.
#[derive(Default)]
struct Phase {
    /// The first entry and the last exit.
    span: Option<(Instant, Instant)>,
    /// Which of [`PHYSICS`] were entered.
    seen: [bool; PHYSICS.len()],
}

/// A frame worker's wait without a permit, from a `park` span.
#[derive(Clone, Copy)]
struct Park {
    worker: usize,
    latch: bool,
    began: Instant,
    /// `None` while the worker waits.
    ended: Option<Instant>,
}

/// What the `open_gate` and `park` spans of the frame workers showed since the last take.
#[derive(Default)]
struct Gate {
    /// Each opening's start and end, the end `None` until the gate has returned.
    openings: Vec<(Instant, Option<Instant>)>,
    /// The waits an opening began within, the rest dropped as they end: the workers park
    /// between records far more often than the gate opens, and a callback that grew a long
    /// vector would stall the worker it runs on.
    parks: Vec<Park>,
    /// Each frame worker's wait in progress.
    waiting: Vec<Option<Park>>,
    /// Every wait begun, kept or not.
    began: usize,
}

impl Gate {
    /// Whether an opening began after `park` did, and so within it while it waits.
    fn spans_an_opening(&self, park: &Park) -> bool {
        self.openings.last().is_some_and(|&(start, _)| start > park.began)
    }
}

/// The first entry into a physics system's span and the last exit since the last take: the
/// physics phase of one tick. A subscriber of its own: it sees the executor's node and range
/// spans of the three systems, and `lactose-ecs`'s system span of every system, whose name
/// it reads to keep the three. `step` runs as a node of the executor's; `sync` and
/// `write_back` are exclusive and run as no node, so only their system spans show them.
///
/// It also sees `lactose-task`'s `open_gate` and `park` spans and keeps the frame workers'
/// ([`Gate`]). A `park` callback runs on a worker that is going to sleep or has just been
/// woken, where it must return promptly and never call into the executor (`lactose-task`'s
/// README, "Tracing"), and an `open_gate` one on the opener while the gate is still closed:
/// each reads the clock, takes a lock that only these callbacks take and the test thread
/// between ticks, and allocates nothing, since [`PhysicsPhase::take_gate`] leaves room for
/// every opening and wait of a reservation.
#[derive(Default)]
struct PhysicsPhase {
    phase: Mutex<Phase>,
    gate: Mutex<Gate>,
}

impl PhysicsPhase {
    /// The phase since the last take, and which of the three systems it saw.
    fn take(&self) -> Phase {
        std::mem::take(&mut *lock(&self.phase))
    }

    /// How many openings were seen since the last take of the gate's spans.
    fn openings(&self) -> usize {
        lock(&self.gate).openings.len()
    }

    /// The gate's spans since the last take, a wait in progress kept with `ended` `None`
    /// where an opening began within it. Taken before a reservation, it drops every wait in
    /// progress: each began before any opening the next take holds.
    ///
    /// Leaves room for `openings` openings on `workers` frame workers, made here on the test
    /// thread so that no callback grows a vector: a worker's waits do not overlap, and each
    /// one kept spans an opening's start, so no more than `workers` are kept per opening.
    fn take_gate(&self, workers: u32, openings: u32) -> Gate {
        let (workers, openings) = (workers as usize, openings as usize);
        let room = Gate {
            openings: Vec::with_capacity(openings),
            parks: Vec::with_capacity(openings * workers),
            waiting: vec![None; workers],
            began: 0,
        };
        let mut gate = std::mem::replace(&mut *lock(&self.gate), room);
        let waiting: Vec<Park> = gate.waiting.drain(..).flatten().collect();
        for park in waiting {
            if gate.spans_an_opening(&park) {
                gate.parks.push(park);
            }
        }
        gate
    }

    fn gate_entered(&self, id: u64, worker: usize, now: Instant) {
        let mut gate = lock(&self.gate);
        if id == OPENING {
            gate.openings.push((now, None));
            return;
        }
        gate.began += 1;
        if gate.waiting.len() <= worker {
            gate.waiting.resize(worker + 1, None);
        }
        gate.waiting[worker] =
            Some(Park { worker, latch: id == PARK_LATCH, began: now, ended: None });
    }

    fn gate_exited(&self, id: u64, worker: usize, now: Instant) {
        let mut gate = lock(&self.gate);
        if id == OPENING {
            if let Some(opening) = gate.openings.last_mut() {
                opening.1 = Some(now);
            }
            return;
        }
        let Some(mut park) = gate.waiting.get_mut(worker).and_then(Option::take) else { return };
        if gate.spans_an_opening(&park) {
            park.ended = Some(now);
            gate.parks.push(park);
        }
    }
}

/// The calling thread's index among the executor's frame-critical workers, which
/// `lactose-task` names `whisk-frame-<index>`, else `None`. Read from the name once a thread.
fn frame_worker() -> Option<usize> {
    thread_local! {
        static INDEX: Cell<Option<Option<usize>>> = const { Cell::new(None) };
    }
    INDEX.with(|index| {
        if let Some(known) = index.get() {
            return known;
        }
        let known = std::thread::current()
            .name()
            .and_then(|name| name.strip_prefix("whisk-frame-"))
            .and_then(|at| at.parse().ok());
        index.set(Some(known));
        known
    })
}

/// Reads a `park` span's `wait` field as the span's id, with no allocation: it runs on the
/// worker that parks.
#[derive(Default)]
struct Wait(Option<u64>);

impl tracing::field::Visit for Wait {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "wait" {
            self.0 = Some(match value {
                "idle" => PARK_IDLE,
                "latch" => PARK_LATCH,
                other => panic!("a `park` span whose wait is {other:?}"),
            });
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        assert!(field.name() != "wait", "a `park` span whose wait is not a string: {value:?}");
    }
}

/// The id of the physics system `name`, else [`OTHER`].
fn physics_id(name: &str) -> Id {
    let index = PHYSICS.iter().position(|&physics| physics == name);
    Id::from_u64(index.map_or(OTHER, |index| index as u64 + 1))
}

/// Reads a system span's `name` field.
#[derive(Default)]
struct SystemName(Option<String>);

impl tracing::field::Visit for SystemName {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "name" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "name" {
            self.0 = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }
}

impl Subscriber for &'static PhysicsPhase {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) { Interest::always() } else { Interest::never() }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        let node = metadata.target() == "whisk"
            && metadata
                .module_path()
                .is_some_and(|path| path == "lactose_task::node" || path == "lactose_task::range")
            && PHYSICS.contains(&metadata.name());
        let system = metadata.target() == "casein" && metadata.name() == "system";
        let gate = metadata.target() == "whisk"
            && matches!(metadata.name(), "open_gate" | "park")
            && *metadata.level() == tracing::Level::DEBUG;
        metadata.is_span() && (node || system || gate)
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let metadata = attributes.metadata();
        if metadata.target() == "whisk" {
            return match metadata.name() {
                "open_gate" => Id::from_u64(OPENING),
                "park" => {
                    let mut wait = Wait::default();
                    attributes.record(&mut wait);
                    Id::from_u64(wait.0.expect("a `park` span has a `wait` field"))
                }
                name => physics_id(name),
            };
        }
        let mut name = SystemName::default();
        attributes.record(&mut name);
        name.0.as_deref().map_or(Id::from_u64(OTHER), physics_id)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &tracing::Event<'_>) {}

    fn enter(&self, id: &Id) {
        if id.into_u64() >= OPENING {
            let now = Instant::now();
            if let Some(worker) = frame_worker() {
                self.gate_entered(id.into_u64(), worker, now);
            }
            return;
        }
        let Some(index) = (id.into_u64() as usize).checked_sub(1).filter(|&i| i < PHYSICS.len())
        else {
            return;
        };
        let now = Instant::now();
        let mut phase = lock(&self.phase);
        phase.seen[index] = true;
        phase.span = Some(phase.span.map_or((now, now), |(first, last)| (first.min(now), last)));
    }

    fn exit(&self, id: &Id) {
        if id.into_u64() >= OPENING {
            let now = Instant::now();
            if let Some(worker) = frame_worker() {
                self.gate_exited(id.into_u64(), worker, now);
            }
            return;
        }
        if !(1..OTHER).contains(&id.into_u64()) {
            return;
        }
        let now = Instant::now();
        let mut phase = lock(&self.phase);
        phase.span = Some(phase.span.map_or((now, now), |(first, last)| (first, last.max(now))));
    }
}

// ---- The record ----

/// The binary's render side on null, recorded one frame at a time.
struct Recorder {
    render: Arc<Mutex<ClientRender>>,
    rows: Arc<Vec<InstanceRow>>,
    frames: u32,
}

impl Recorder {
    fn new() -> Self {
        let (device, _, _) = null_device(Capabilities::LEAST);
        let input = Arc::new(LatestSample::new());
        let render =
            ClientRender::new(&device, &null_shaders(), input, None).expect("a render side");
        let rows = (1..=INSTANCES)
            .map(|n| InstanceRow {
                key: key(3_000_000 + n),
                mesh: AssetRef::new(AssetId::from_bits(n)),
                transform: RenderTransform::from_sim(&Transform::IDENTITY),
                no_interpolate: false,
            })
            .collect();
        Self { render: Arc::new(Mutex::new(render)), rows: Arc::new(rows), frames: 0 }
    }

    /// One frame: the job that records it, timed from its start to `draw`'s return, then
    /// its submit. Returns when the job was asked for, and when the record began and ended.
    fn record(&mut self, executor: &Executor) -> Timed {
        let target = TargetIndex::new((self.frames % 2) as usize).expect("a target");
        let pass = ExtractPass {
            kind: WorldKind::Server,
            tick: Tick(self.frames),
            full: self.frames == 0,
        };
        self.frames += 1;
        let asked = Instant::now();
        let job = {
            let (render, rows) = (Arc::clone(&self.render), Arc::clone(&self.rows));
            executor.run_job(move |ctx| {
                let start = Instant::now();
                let worker = frame_worker();
                let mut render = lock(&render);
                assert_eq!(render.open(ctx).expect("a region"), Region::Ready, "null frees each");
                let mut sink = render.sink();
                sink.begin(pass);
                sink.write(&rows);
                sink.end();
                drop(sink);
                let draw = Draw { target, extent: FIXED_EXTENT, ready: None };
                render.draw(ctx, draw).expect("the frame records");
                (start, Instant::now(), worker)
            })
        };
        let (start, end, worker) =
            bounded(executor, job, "a frame's record").expect("the job runs");
        lock(&self.render).submit().expect("the frame submits");
        Timed { asked, start, end, worker }
    }
}

/// One record's times: `run_job` called, the job begun, `draw` returned; and the frame
/// worker that ran it.
#[derive(Clone, Copy)]
struct Timed {
    asked: Instant,
    start: Instant,
    end: Instant,
    worker: Option<usize>,
}

impl Timed {
    fn time(&self) -> Duration {
        self.end - self.start
    }

    /// From `run_job`'s call to the job's start: the wait for a worker to take it.
    fn start_wait(&self) -> Duration {
        self.start - self.asked
    }
}

/// A group of records' figures under `prefix`: the record's spread, and its start wait's.
fn records(prefix: &str, records: &[Timed]) {
    figure(&format!("{prefix}.records"), records.len());
    if records.is_empty() {
        return;
    }
    spread(&format!("{prefix}.record"), records.iter().map(Timed::time).collect());
    spread(&format!("{prefix}.start_wait"), records.iter().map(Timed::start_wait).collect());
}

/// The median and the worst of `spans`, as figures `<name>.p50_us` and `<name>.max_us`;
/// nothing for none.
fn median_and_worst(name: &str, mut spans: Vec<Duration>) {
    if spans.is_empty() {
        return;
    }
    spans.sort_unstable();
    figure(&format!("{name}.p50_us"), micros(percentile(&spans, 0.5)));
    figure(&format!("{name}.max_us"), micros(spans[spans.len() - 1]));
}

/// [`spread`] from [`PERCENTILE_SAMPLES`] on, else [`median_and_worst`].
fn spread_of_enough(name: &str, spans: Vec<Duration>) {
    if spans.len() >= PERCENTILE_SAMPLES {
        spread(name, spans);
    } else {
        median_and_worst(name, spans);
    }
}

/// The gate's figures of one reservation under `<prefix>.gate`, over the openings of the
/// measured ticks: each tick's range of `gate.openings` and the records it made.
fn gate_figures(prefix: &str, gate: &Gate, ticks: &[(Range<usize>, Vec<Timed>)]) {
    let (mut lengths, mut wakes, mut idle, mut latch) = (Vec::new(), Vec::new(), 0, 0);
    let (mut from_opening, mut start_wait, mut awake_took) = (Vec::new(), Vec::new(), 0);
    for (openings, made) in ticks {
        for &(start, end) in &gate.openings[openings.clone()] {
            lengths.push(end.expect("the gate returns before its tick ends") - start);
            let mut parked = Vec::new();
            for park in &gate.parks {
                if park.began >= start || park.ended.is_some_and(|ended| ended <= start) {
                    continue;
                }
                parked.push(park.worker);
                // A wait still in progress when the reservation ends is parked but not timed.
                if let Some(ended) = park.ended {
                    wakes.push(ended - start);
                    if park.latch {
                        latch += 1;
                    } else {
                        idle += 1;
                    }
                }
            }
            let Some(next) = made.iter().find(|record| record.start >= start) else {
                continue;
            };
            if next.worker.is_some_and(|worker| parked.contains(&worker)) {
                from_opening.push(next.start - start);
                start_wait.push(next.start_wait());
            } else {
                awake_took += 1;
            }
        }
    }
    let prefix = format!("{prefix}.gate");
    figure(&format!("{prefix}.openings"), lengths.len());
    spread_of_enough(&format!("{prefix}.opening"), lengths);
    figure(&format!("{prefix}.wakes_idle"), idle);
    figure(&format!("{prefix}.wakes_latch"), latch);
    spread_of_enough(&format!("{prefix}.wake"), wakes);
    figure(&format!("{prefix}.parked_took"), from_opening.len());
    median_and_worst(&format!("{prefix}.parked_took.from_opening"), from_opening);
    median_and_worst(&format!("{prefix}.parked_took.start_wait"), start_wait);
    figure(&format!("{prefix}.awake_took"), awake_took);
}

/// Every figure of one reservation, under `r<reserved>`.
fn beside(executor: &Executor, recorder: &mut Recorder, workers: u32, reserved: u32, ticks: u32) {
    let phase: &'static PhysicsPhase = physics_phase();
    let mut simulation = composed(workers, reserved);
    let (mut physics, mut steps) = (Vec::new(), Vec::new());
    let (mut by_physics, mut by_tick) = (Vec::new(), Vec::new());
    let mut gated = Vec::new();
    phase.take_gate(workers, WARM_UP + ticks);
    for tick in 0..WARM_UP + ticks {
        phase.take();
        let opened = phase.openings();
        let stepping = {
            let executor = executor.clone();
            std::thread::spawn(move || {
                let input = frame(simulation.tick().0 + 1);
                // Timed inside the bounded future: from its first poll to the tick's end,
                // not the bound's keeping thread, which `bounded` starts before and joins
                // after.
                let step = async {
                    let start = Instant::now();
                    let stepped = simulation.step(&executor, input).await;
                    (stepped, (start, Instant::now()))
                };
                let (stepped, span) = bounded(&executor, step, "a composed tick");
                (stepped.expect("the tick runs"), span)
            })
        };
        // Until the stepping thread ends, however it ends: a tick that panics ends the
        // loop, and the join below fails with it.
        let mut made = Vec::new();
        while !stepping.is_finished() {
            made.push(recorder.record(executor));
        }
        let (stepped, (began, ended)) = stepping.join().expect("the stepping thread");
        simulation = stepped;
        let Phase { span, seen } = phase.take();
        let (first, last) = span.expect("the tick ran its physics systems");
        assert!(seen.iter().all(|&seen| seen), "the phase saw {seen:?} of {PHYSICS:?}");
        let openings = opened..phase.openings();
        assert!(!openings.is_empty(), "tick {tick} at {reserved} saw no `open_gate` span");
        if tick < WARM_UP {
            continue;
        }
        steps.push(ended - began);
        physics.push(last - first);
        gated.push((openings, made.clone()));
        for record in made {
            if record.start < last && record.end > first {
                by_physics.push(record);
            } else if record.start < ended && record.end > began {
                by_tick.push(record);
            }
        }
    }
    let prefix = format!("r{reserved}");
    figure(&format!("{prefix}.physics_us"), micros(median(physics)));
    figure(&format!("{prefix}.step_us"), micros(median(steps)));
    records(&format!("{prefix}.beside_physics"), &by_physics);
    records(&format!("{prefix}.beside_tick"), &by_tick);
    let gate = phase.take_gate(workers, WARM_UP + ticks);
    assert!(gate.began > 0, "the reservation of {reserved} saw no `park` span");
    gate_figures(&prefix, &gate, &gated);
}

/// The subscriber, installed once per process (nextest runs each test in its own).
fn physics_phase() -> &'static PhysicsPhase {
    static PHASE: std::sync::OnceLock<&'static PhysicsPhase> = std::sync::OnceLock::new();
    PHASE.get_or_init(|| {
        let phase: &'static PhysicsPhase = Box::leak(Box::default());
        tracing::subscriber::set_global_default(phase).expect("no other subscriber");
        phase
    })
}

/// `cargo xtask measure record-stall` runs it.
#[test]
#[ignore = "a measurement, run by `cargo xtask measure`: times are reported, not asserted"]
fn the_record_beside_the_physics_phase() {
    let ticks = if short() { 6 } else { 360 };
    let workers = measured_workers();
    let executor = start_executor(Workers { frame_workers: Some(workers), ..Workers::default() })
        .expect("an executor");
    let mut recorder = Recorder::new();
    for reserved in [1, 2] {
        beside(&executor, &mut recorder, workers, reserved, ticks);
    }
    let alone: Vec<Timed> = (0..600).map(|_| recorder.record(&executor)).collect();
    records("alone", &alone);
    figure("workers", workers);
    figure("run_job_width", executor.run_job_width());
    figure("record_reservation", RECORD_RESERVATION);
    figure("reservations", "1 and 2");
    figure("rows", ROWS + u64::from(PLAYERS));
    figure("bodies", BODIES);
    figure("instances", INSTANCES);
    figure("ticks", ticks);
    figure("warm_up_ticks", WARM_UP);
    drop(recorder);
    executor.shutdown();
}
