//! What one tick costs at the milestone 1 scale (architecture section 22: "100k entities
//! simulate a tick in at most 4 ms on 8 performance cores"): a server world of 100,000 rows
//! in four archetypes over sixteen regions, stepped through `Simulation::step` with a
//! representative load (systems that read and write a few columns, a transient column
//! written by an input applier and by a kernel, an event channel indexed by target, region
//! resources read by kernels and written at a sync point, a reduction, command traffic that
//! despawns and spawns rows every tick) and hashed after every step as SOUR does. The time
//! of each phase (sync points, each system, the hash) is collected from the `casein` spans
//! and printed; it is measured by an ignored test, run by hand, and reported, never
//! asserted. What the scene computes is asserted in a test that runs everywhere, and the
//! scene at a small scale records its hash under the runner's session.

#![forbid(unsafe_code)]

mod glue;

use std::collections::BTreeMap;

use glue::{Harness, protocol_error};
use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Event, EventWriter, Events, Executor, LayoutConfig, Mut,
    RegionConfig, RegionId, RegionMode, RegionRes, Res, ResMut, StableKeyOf, SystemSet, Without,
    World, WorldKind, finalize, system,
};
use lactose_math::{ExactSum, Scalar};
use lactose_reflect::{component, resource};
use lactose_sim::harness::{ChunkCapacity, layout_mode, run_recorded};
use lactose_sim::{
    InputApplier, PlayerSample, Registration, Simulation, SimulationBuilder, Stage, TickDelta,
    TickInput, TypeTable, sets,
};

// ---- The scene ----

#[component(key = "cost::position", persistent, replicated)]
struct Position {
    #[field(id = 1)]
    x: Scalar,
    #[field(id = 2)]
    y: Scalar,
}

#[component(key = "cost::velocity", persistent)]
struct Velocity {
    #[field(id = 1)]
    x: Scalar,
    #[field(id = 2)]
    y: Scalar,
}

#[component(key = "cost::health", persistent, replicated)]
struct Health(#[field(id = 1)] i32);

#[component(key = "cost::timer", persistent)]
struct Timer(#[field(id = 1)] u32);

#[component(key = "cost::marker", persistent)]
struct Marker;

/// Contacts this tick: transient, zeroed at every input sync point.
#[component(key = "cost::contacts", transient)]
struct Contacts(#[field(id = 1)] u32);

/// A player's steering this tick: transient, written by the input applier.
#[component(key = "cost::steer", transient)]
struct Steer {
    #[field(id = 1)]
    dx: i32,
    #[field(id = 2)]
    dy: i32,
}

/// A region's wind, read by every mover's kernel and turned at the post sync point.
#[resource(key = "cost::wind", persistent)]
struct Wind(#[field(id = 1)] Scalar);

/// A region's gusts this tick: transient, zeroed at the input sync point.
#[resource(key = "cost::gusts", transient)]
struct Gusts(#[field(id = 1)] u32);

/// How many rows the scene was built with, from which a hit picks its target.
#[resource(key = "cost::population", derived)]
struct Population(#[field(id = 1)] u32);

/// What the reduction writes.
#[resource(key = "cost::energy", derived)]
struct Energy(#[field(id = 1)] Scalar);

/// One hit on one entity.
#[derive(Clone, Copy)]
struct Hit {
    target: StableKey,
    amount: i32,
}

impl Event for Hit {
    fn target(&self) -> StableKey {
        self.target
    }
}

const ROWS: u64 = 100_000;
const REGIONS: u64 = 16;
const PLAYERS: u32 = 64;
/// The first player's key, after every row's.
const FIRST_PLAYER: u64 = 1_000_000;

fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

fn region(n: u64) -> RegionId {
    RegionId(1 + n % REGIONS)
}

static MOVEMENT: SystemSet = SystemSet::new("cost::movement");
static INPUT: SystemSet = SystemSet::new("cost::input").after(&[&sets::INPUT]);

/// The declared writer of [`Steer`]: the applier writes it inside `sim`'s systems.
#[system(name = "cost::input_writer", worlds = [server], exclusive, in_set = INPUT, writes = [Steer])]
fn input_writer(_world: &mut World) {}

#[system(name = "cost::steer", worlds = [server], in_set = MOVEMENT)]
fn steer(input: &Steer, vel: Mut<Velocity>) {
    vel.x += Scalar::from_i32(input.dx);
    vel.y += Scalar::from_i32(input.dy);
}

#[system(name = "cost::integrate", worlds = [server], in_set = MOVEMENT, after = steer)]
fn integrate(dt: Res<TickDelta>, wind: RegionRes<Wind>, vel: &Velocity, pos: Mut<Position>) {
    let next = Position { x: pos.x + (vel.x + wind.0) * dt.0, y: pos.y + vel.y * dt.0 };
    pos.set_if_neq(next);
}

#[system(name = "cost::drag", worlds = [server], in_set = MOVEMENT, after = integrate)]
fn drag(vel: Mut<Velocity>, _plain: Without<Marker>) {
    let half = Scalar::from_i32(1) / Scalar::from_i32(2);
    vel.x *= half;
    vel.y *= half;
}

#[system(name = "cost::touch", worlds = [server], in_set = MOVEMENT, after = integrate)]
fn touch(pos: &Position, contacts: Mut<Contacts>) {
    contacts.0 = 1 + u32::from(pos.y < Scalar::ZERO);
}

/// One row in a hundred hits another, chosen by the RNG keyed by the striker.
#[system(name = "cost::strike", worlds = [server], in_set = MOVEMENT, after = touch)]
fn strike(
    ctx: Ctx,
    rows: Res<Population>,
    key: StableKeyOf,
    contacts: &Contacts,
    hits: EventWriter<Hit>,
) {
    let mut rng = ctx.rng(*key);
    if rng.next_below(100) == 0 {
        let target = self::key(1 + u64::from(rng.next_below(rows.0)));
        hits.emit(Hit { target, amount: contacts.0 as i32 });
    }
}

#[system(name = "cost::take_hits", worlds = [server], in_set = MOVEMENT, after = strike)]
fn take_hits(events: Events<Hit>, key: StableKeyOf, hp: Mut<Health>) {
    for hit in events.for_target(*key) {
        hp.0 -= hit.amount;
    }
}

/// One timed row in a thousand is replaced by a fresh one each tick.
#[system(name = "cost::timers", worlds = [server], in_set = MOVEMENT, after = integrate)]
fn timers(ctx: Ctx, key: StableKeyOf, timer: Mut<Timer>, pos: &Position, cmd: Commands) {
    timer.0 += 1;
    if ctx.rng(*key).next_below(1_000) == 0 {
        cmd.despawn(*key);
        let velocity = Velocity { x: Scalar::ONE, y: Scalar::ZERO };
        cmd.spawn((Position { x: pos.x, y: pos.y }, velocity, Timer(0)));
    }
}

#[system(name = "cost::energy", worlds = [server], in_set = MOVEMENT, after = drag, reduce = ExactSum<Scalar>)]
fn energy(vel: &Velocity) -> Scalar {
    vel.x * vel.x + vel.y * vel.y
}

#[finalize(energy)]
fn store_energy(sum: Scalar, out: ResMut<Energy>) {
    out.0 = sum;
}

/// Every simulated region's wind turns and its gusts count, at the post sync point.
#[system(name = "cost::weather", worlds = [server], exclusive)]
fn weather(world: &mut World) {
    let regions: Vec<RegionId> = world.regions().collect();
    for region in regions {
        world.update_region_resource::<Wind>(region, |wind| wind.0 = -wind.0).unwrap();
        world.update_region_resource::<Gusts>(region, |gusts| gusts.0 += 1).unwrap();
    }
}

fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Input).sets(&[&INPUT]).add(&INPUT_WRITER);
    registration
        .stage(Stage::Simulation)
        .sets(&[&MOVEMENT])
        .add(&STEER)
        .add(&INTEGRATE)
        .add(&DRAG)
        .add(&TOUCH)
        .add(&STRIKE)
        .add(&TAKE_HITS)
        .add(&TIMERS)
        .add(&ENERGY);
    registration.stage(Stage::Post).add(&WEATHER);
}

fn types() -> TypeTable {
    let mut types = TypeTable::new();
    types.register::<Position>().unwrap();
    types.register::<Velocity>().unwrap();
    types.register::<Health>().unwrap();
    types.register::<Timer>().unwrap();
    types.register::<Marker>().unwrap();
    types.register::<Contacts>().unwrap();
    types.register::<Steer>().unwrap();
    types.register::<Wind>().unwrap();
    types.register::<Gusts>().unwrap();
    types.register::<Energy>().unwrap();
    types.register::<Population>().unwrap();
    types
}

/// Writes each player's sample to the entity the player has claimed.
struct Steering;

impl InputApplier for Steering {
    fn apply(&mut self, world: &mut World, player: PlayerId, sample: &PlayerSample) -> Result<()> {
        let value: Steer = lactose_reflect::decode(&sample.sample)?;
        let key = world.claimed(player).next().expect("every player has an entity");
        world.insert(key, value)
    }
}

/// The scene: `rows` rows of four archetypes spread over the regions, and the players.
fn scene(rows: u64) -> Simulation {
    scene_in(rows, LayoutConfig::default())
}

/// The scene under `layout`.
fn scene_in(rows: u64, layout: LayoutConfig) -> Simulation {
    let mut simulation = SimulationBuilder::new(WorldKind::Server, types())
        .layout(layout)
        .setup(move |world| {
            world.insert_resource(Energy(Scalar::ZERO));
            world.insert_resource(Population(rows as u32));
            Ok(())
        })
        .systems(register_systems)
        .inputs(Box::new(Steering))
        .build()
        .unwrap();
    let world = simulation.world_mut();
    for n in 0..REGIONS {
        let id = RegionId(1 + n);
        world.create_region(RegionConfig { id, seed: n, mode: RegionMode::Simulated }).unwrap();
        world.insert_region_resource(id, Wind(Scalar::from_i32(n as i32 - 8))).unwrap();
        world.insert_region_resource(id, Gusts(0)).unwrap();
    }
    for n in 1..=rows {
        let at = Position { x: Scalar::from_i32(n as i32), y: Scalar::from_i32(-(n as i32 % 7)) };
        let moving = Velocity { x: Scalar::ONE, y: Scalar::from_i32(n as i32 % 3) };
        let result = match n % 10 {
            0..4 => world.spawn(key(n), region(n), (at, moving, Health(100), Contacts(0))),
            4..7 => world.spawn(key(n), region(n), (at, moving, Timer(n as u32 % 600))),
            7..9 => world.spawn(key(n), region(n), (at, Health(100))),
            _ => world.spawn(
                key(n),
                region(n),
                (at, moving, Health(200), Timer(0), Marker, Contacts(0)),
            ),
        };
        result.unwrap();
    }
    for player in 0..PLAYERS {
        let n = FIRST_PLAYER + u64::from(player);
        let row = (
            Position { x: Scalar::ZERO, y: Scalar::ZERO },
            Velocity { x: Scalar::ZERO, y: Scalar::ZERO },
            Health(1_000),
            Contacts(0),
            Steer { dx: 0, dy: 0 },
            ClaimedBy { player: PlayerId(player) },
        );
        world.spawn(key(n), region(n), row).unwrap();
    }
    simulation
}

/// Every player's input for `tick`.
fn frame(tick: u32) -> TickInput {
    let mut frame = TickInput::default();
    for player in 0..PLAYERS {
        let steer = Steer { dx: (tick % 3) as i32 - 1, dy: (player % 3) as i32 - 1 };
        let sample = PlayerSample {
            input_tick: Tick(tick),
            substituted: false,
            sample: lactose_reflect::bytemuck::bytes_of(&steer).to_vec(),
        };
        frame.inputs.insert(PlayerId(player), sample);
    }
    frame
}

fn step(executor: &Executor, simulation: Simulation) -> Simulation {
    let tick = simulation.tick().0 + 1;
    executor.block_on(simulation.step(executor, frame(tick))).unwrap()
}

/// The scene at a small scale runs, every system does its work, and the result does not
/// depend on the worker count: each per-entity system's units fork over the workers
/// (D137 part B), so one worker and four run the same units in different orders and on
/// different threads.
#[test]
fn the_scene_runs_and_its_hash_is_the_same_on_one_and_four_workers() {
    let mut hashes = Vec::new();
    for workers in [1, 4] {
        let executor =
            Executor::builder().frame_workers(workers).background_workers(1).build().unwrap();
        let mut simulation = scene(2_000);
        let mut trace = Vec::new();
        for _ in 0..30 {
            simulation = step(&executor, simulation);
            trace.push(simulation.hash().unwrap());
        }
        let world = simulation.world_mut();
        assert_eq!(world.query::<&Position>().count(), 2_000 + PLAYERS as usize);
        assert!(world.query::<&Timer>().count() > 0);
        let mut hurt = 0;
        world.query::<&Health>().for_each(|_, hp| hurt += usize::from(hp.0 < 100));
        assert!(hurt > 0, "hits landed");
        assert_ne!(world.resource::<Energy>().unwrap().0, Scalar::ZERO);
        let wind = world.region_resource::<Wind>(RegionId(1)).unwrap().0;
        assert_eq!(wind, Scalar::from_i32(-8), "turned an even number of times");
        hashes.push(trace);
    }
    assert_eq!(hashes[0], hashes[1]);
}

/// Every kernel of the scene runs per chunk, or under `split_batches` per random range of a
/// chunk's rows, so the scene under SOUR's layout mode at every chunk capacity is the same
/// computation cut into different units: one hash every tick, and one merged output of the
/// reduction, which is derived and so outside the hash.
#[test]
fn the_scene_gives_one_hash_and_one_reduction_every_tick_under_every_layout() {
    let executor = Executor::builder().frame_workers(1).background_workers(1).build().unwrap();
    let trace = |layout: LayoutConfig| {
        let mut simulation = scene_in(2_000, layout);
        let mut trace = Vec::new();
        for _ in 0..20 {
            simulation = step(&executor, simulation);
            let energy = simulation.world_mut().resource::<Energy>().unwrap().0;
            trace.push((simulation.hash().unwrap(), energy));
        }
        trace
    };
    let reference = trace(LayoutConfig::default());
    assert!(reference.windows(2).all(|pair| pair[0].0 != pair[1].0), "every tick changes");
    for capacity in [
        ChunkCapacity::Rows(1),
        ChunkCapacity::Rows(7),
        ChunkCapacity::Rows(64),
        ChunkCapacity::Bytes(16384),
    ] {
        for seed in [1, 0x5eed_1a70] {
            let layout = layout_mode(capacity, seed).unwrap();
            let first = trace(layout).iter().zip(&reference).position(|(a, b)| a != b);
            assert_eq!(first, None, "{capacity:?} seed {seed}: first differing tick");
        }
    }
}

/// The scene at a small scale, under the runner's session: its worker count, its layout
/// and, in chaos mode, every perturbation with each split forced, recording the hash after
/// every step, so every SOUR mode compares the scene's ticks with the baseline's.
#[test]
fn the_scene_records_its_hash_every_tick_under_the_session() {
    const TEST: &str = "lactose_sim::cost::scene";
    let harness = Harness::from_env().unwrap();
    let mut simulation = scene_in(2_000, harness.layout);
    let mut recorder = harness.recorder(TEST, &simulation).unwrap();
    let mut record = |tick: Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    let frames = (1..=30).map(frame);
    simulation = run_recorded(&harness.executor, simulation, frames, &mut record).unwrap();
    harness.finish(TEST).unwrap();
    let mut hurt = 0;
    simulation.world_mut().query::<&Health>().for_each(|_, hp| hurt += usize::from(hp.0 < 100));
    assert!(hurt > 0, "hits landed");
}

// ---- The measurement ----

/// Times the `casein` spans: each sync point, stage and system, summed by name until taken.
mod timing {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::subscriber::Interest;
    use tracing::{Event, Metadata, Subscriber};

    /// The time since the first call: the one place the subscriber reads the clock.
    #[expect(clippy::disallowed_types, reason = "test code timing the step from outside")]
    fn clock() -> Duration {
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed()
    }

    struct Open {
        label: String,
        entered: Option<Duration>,
    }

    #[derive(Default)]
    pub struct Spans {
        next: AtomicU64,
        open: Mutex<BTreeMap<u64, Open>>,
        totals: Mutex<BTreeMap<String, Duration>>,
    }

    impl Spans {
        /// The time spent in each span since the last take, by label.
        pub fn take(&self) -> BTreeMap<String, Duration> {
            core::mem::take(&mut *self.totals.lock().unwrap())
        }
    }

    struct Name(String);

    impl Visit for Name {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "name" || field.name() == "stage" {
                self.0 = value.to_owned();
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
            if field.name() == "name" || field.name() == "stage" {
                self.0 = format!("{value:?}");
            }
        }
    }

    impl Subscriber for &'static Spans {
        fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
            if self.enabled(metadata) { Interest::always() } else { Interest::never() }
        }

        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.is_span() && metadata.target() == "casein"
        }

        fn new_span(&self, span: &Attributes<'_>) -> Id {
            let mut name = Name(String::new());
            span.record(&mut name);
            let label = format!("{} {}", span.metadata().name(), name.0);
            let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
            self.open.lock().unwrap().insert(id, Open { label, entered: None });
            Id::from_u64(id)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}

        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, _: &Event<'_>) {}

        fn enter(&self, span: &Id) {
            if let Some(open) = self.open.lock().unwrap().get_mut(&span.into_u64()) {
                open.entered = Some(clock());
            }
        }

        fn exit(&self, span: &Id) {
            let now = clock();
            let Some(open) = self.open.lock().unwrap().remove(&span.into_u64()) else { return };
            if let Some(entered) = open.entered {
                *self.totals.lock().unwrap().entry(open.label).or_default() += now - entered;
            }
        }
    }
}

/// The median and the 90th percentile of `samples`, in microseconds.
fn quantiles(samples: &mut [core::time::Duration]) -> (f64, f64) {
    samples.sort();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
    (at(0.5).as_secs_f64() * 1e6, at(0.9).as_secs_f64() * 1e6)
}

/// The time of a step at 100,000 rows, phase by phase, and of the hash after it, printed
/// for the report. Run with `cargo nextest run -p lactose-sim --run-ignored only
/// --cargo-profile release --no-capture -E 'test(cost_)'`.
#[test]
#[ignore = "a measurement, run by hand: timings are reported, not asserted"]
#[expect(clippy::disallowed_types, reason = "test code measuring the step, outside it")]
fn cost_of_a_tick_at_100k_rows() {
    use std::time::{Duration, Instant};
    const WARM: u32 = 10;
    const TICKS: u32 = 60;
    let spans: &'static timing::Spans = Box::leak(Box::default());
    tracing::subscriber::set_global_default(spans).unwrap();
    for workers in [1, 4] {
        let executor =
            Executor::builder().frame_workers(workers).background_workers(1).build().unwrap();
        let mut simulation = scene(ROWS);
        let mut phases: BTreeMap<String, Vec<Duration>> = BTreeMap::new();
        let mut steps = Vec::new();
        let mut hashes = Vec::new();
        for tick in 0..WARM + TICKS {
            spans.take();
            let start = Instant::now();
            simulation = step(&executor, simulation);
            let stepped = start.elapsed();
            let start = Instant::now();
            let hash = simulation.hash().unwrap();
            let hashed = start.elapsed();
            assert_ne!(hash, 0);
            if tick < WARM {
                continue;
            }
            steps.push(stepped);
            hashes.push(hashed);
            for (label, time) in spans.take() {
                phases.entry(label).or_default().push(time);
            }
        }
        let rows = simulation.world_mut().query::<&Position>().count();
        println!("{workers} frame worker(s), {rows} rows, {TICKS} ticks: median / p90 in µs");
        let (median, p90) = quantiles(&mut steps);
        println!("  {:<40} {median:>9.1} {p90:>9.1}", "step (whole)");
        for (label, samples) in &mut phases {
            let (median, p90) = quantiles(samples);
            println!("  {label:<40} {median:>9.1} {p90:>9.1}");
        }
        let (median, p90) = quantiles(&mut hashes);
        println!("  {:<40} {median:>9.1} {p90:>9.1}", "hash (all regions)");
        let start = Instant::now();
        let one = simulation.world_hash(RegionId(1)).unwrap();
        println!("  {:<40} {:>9.1}", "hash (one region)", start.elapsed().as_secs_f64() * 1e6);
        assert_ne!(one, 0);
    }
}
