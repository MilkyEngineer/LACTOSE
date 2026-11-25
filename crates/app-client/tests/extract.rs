//! `extract-50k` (the milestone 2 plan's criterion 4 and verification row 16, section 4's
//! table): extract's pass alone, writing the renderer's ring on a Vulkan device and again
//! on null. `cargo xtask measure extract-50k` runs it under the driver it names with every
//! layer off, and writes the device's identity and the ring's memory type into the file's
//! header; `cargo xtask gpu` runs it short under the validators, whose figures it discards.
//!
//! The world is a server's: 100,000 rows with a `Transform` and a `MeshInstance` in four
//! archetypes and sixteen regions, as milestone 1's cost scene is laid out: half carry a
//! walker that a kernel moves one unit along x every tick, and one row in ten of each half
//! carries `Shown`, an `extracted` component, so 10,000 presented rows go to the snapshot
//! beside the ring (D157). Each frame is one tick, then `Simulation::run_passes` with
//! extract alone in the pass stage, timed, then the publication, `Renderer::prepare` and an
//! empty submit that signals the frame's timeline value, so the ring's regions come free; a
//! wait for a free region, before the pass, is outside the time and reported.
//!
//! The pass is timed three ways on N frame workers (N the performance cores up to eight):
//! writing the ring and the snapshot, the figure criterion 4 holds to 1 ms on the device;
//! the ring alone, with no snapshot installed; and the snapshot alone, beside a sink that
//! counts the rows and keeps nothing. The snapshot's part is the first less the second, and
//! the ring's the first less the third, signed. The first again on one worker gives the cost per
//! dirty row. Each is the median of 60 passes (6 in a short run) after a full pass.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use lactose_app::{Workers, start_executor};
use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, StableKey};
use lactose_ecs::{Executor, Mut, RegionConfig, RegionId, RegionMode, WorldKind, system};
use lactose_math::Scalar;
use lactose_physics_api::Transform;
use lactose_present_api::{
    ExtractPass, InstanceRow, InstanceSink, MeshInstance, SnapshotReader, extract_state,
    snapshot_channel,
};
use lactose_reflect::component;
use lactose_renderer::{Renderer, RendererDesc};
use lactose_rhi_api::{Capabilities, Device};
use lactose_sim::{Simulation, SimulationBuilder, Stage, TickInput, TypeTable};

use crate::support::{bounded, null_device};
use crate::timing::{Ender, Waits, figure, measured_workers, median, micros, short, vulkan};

const ROWS: u64 = 100_000;
const MOVED: u64 = 50_000;
const PRESENTED: u64 = 10_000;
const REGIONS: u64 = 16;
/// The ring's regions: the frame being written, and two submitted behind it.
const RING_REGIONS: u32 = 3;

/// Moves its row one unit along x every tick.
#[component(key = "extract_cost::walker", persistent)]
struct Walker(#[field(id = 1)] Scalar);

/// A component the UI would show: `extracted`, so its row is a presented row.
#[component(key = "extract_cost::shown", persistent, extracted)]
struct Shown(#[field(id = 1)] u32);

/// The kernel that dirties half the rows.
#[system(name = "extract_cost::walk", worlds = [server])]
fn walk(walker: &Walker, transform: Mut<Transform>) {
    transform.translation.x += walker.0;
}

fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

/// The world, populated: rows `1..=ROWS`, the even ones walking, those at 1 and 2 modulo 20
/// presented (half of them walking), in the regions by `n` modulo sixteen.
fn world() -> Simulation {
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types).unwrap();
    lactose_present_api::register_types(&mut types).unwrap();
    types.register::<Walker>().unwrap();
    types.register::<Shown>().unwrap();
    let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
        .setup(lactose_present_api::setup)
        .systems(lactose_present_api::register_systems)
        .systems(|registration| {
            registration.stage(Stage::Simulation).add(&WALK);
        })
        .build()
        .unwrap();
    let world = simulation.world_mut();
    for n in 0..REGIONS {
        let config = RegionConfig { id: RegionId(1 + n), seed: n, mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
    }
    for n in 1..=ROWS {
        let region = RegionId(1 + n % REGIONS);
        let mesh = MeshInstance::new(AssetRef::new(AssetId::from_bits(n)));
        let at = Transform::IDENTITY;
        let shown = matches!(n % 20, 1 | 2);
        let result = match (n % 2 == 0, shown) {
            (true, false) => world.spawn(key(n), region, (at, mesh, Walker(Scalar::ONE))),
            (true, true) => world.spawn(key(n), region, (at, mesh, Walker(Scalar::ONE), Shown(1))),
            (false, false) => world.spawn(key(n), region, (at, mesh)),
            (false, true) => world.spawn(key(n), region, (at, mesh, Shown(1))),
        };
        result.unwrap();
    }
    simulation
}

/// A sink that keeps nothing and counts the rows: a relaxed add per batch.
#[derive(Clone, Default)]
struct Counting(Arc<AtomicUsize>);

impl InstanceSink for Counting {
    fn begin(&mut self, _: ExtractPass) {}

    fn removed(&mut self, _: &[StableKey]) {}

    fn write(&self, rows: &[InstanceRow]) {
        self.0.fetch_add(rows.len(), Ordering::Relaxed);
    }

    fn end(&mut self) {}
}

/// What the pass writes into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Into {
    /// The ring and the snapshot.
    Both,
    /// The ring, with no snapshot installed.
    Ring,
    /// The snapshot, beside a counting sink.
    Snapshot,
}

/// A world and the renderer its passes write, frame by frame.
struct Rig {
    simulation: Option<Simulation>,
    renderer: Renderer,
    ender: Ender,
    waits: Waits,
    counted: Counting,
    reader: Option<SnapshotReader>,
    into: Into,
}

impl Rig {
    fn new(device: &Arc<dyn Device>) -> Self {
        // The first pass after a sink is installed is full: every row, and the notes.
        let desc = RendererDesc {
            instances: ROWS as u32,
            rows: ROWS as u32 + 4_096,
            regions: RING_REGIONS,
            shaders: None,
        };
        let renderer = Renderer::new(Arc::clone(device), &desc).expect("a renderer");
        let ender = Ender::new(&**device);
        Self {
            simulation: Some(world()),
            renderer,
            ender,
            waits: Waits::default(),
            counted: Counting::default(),
            reader: None,
            into: Into::Both,
        }
    }

    /// Installs the sink and the snapshot `into` names. The next pass is full.
    fn install(&mut self, into: Into) {
        let simulation = self.simulation.as_mut().expect("between frames");
        let state = extract_state(simulation).expect("setup ran");
        let sink: Box<dyn InstanceSink> = match into {
            Into::Both | Into::Ring => Box::new(self.renderer.sink()),
            Into::Snapshot => Box::new(self.counted.clone()),
        };
        drop(state.set_sink(Some(sink)));
        self.reader = match into {
            Into::Both | Into::Snapshot => {
                let (writer, reader) = snapshot_channel();
                drop(state.set_snapshot(Some(writer)));
                Some(reader)
            }
            Into::Ring => {
                drop(state.set_snapshot(None));
                None
            }
        };
        self.into = into;
    }

    /// One frame on `executor`: the tick, the timed passes, the publication and the frame's
    /// end. Returns the passes' time and the rows they wrote.
    fn frame(&mut self, executor: &Executor) -> (Duration, usize) {
        let simulation = self.simulation.take().expect("between frames");
        let simulation = bounded(
            executor,
            simulation.step(executor, TickInput::default()),
            "the tick before a pass",
        )
        .expect("the tick runs");
        let ring = self.into != Into::Snapshot;
        if ring {
            self.waits.open(&mut self.renderer);
        }
        let start = Instant::now();
        let run = simulation.run_passes(executor);
        let mut simulation = bounded(executor, run, "extract's pass").expect("the passes run");
        let time = start.elapsed();
        let events = simulation.world_mut().take_present_events();
        extract_state(&mut simulation).expect("setup ran").publish(events);
        self.simulation = Some(simulation);
        let rows = if ring {
            let stats = self.renderer.prepare().expect("the frame prepares");
            assert_eq!(stats.dropped, 0, "every row fits the region: {stats:?}");
            self.ender.end(&mut self.renderer);
            stats.pass_rows as usize
        } else {
            self.counted.0.swap(0, Ordering::Relaxed)
        };
        if let Some(reader) = &mut self.reader {
            let frame = reader.acquire().expect("a published snapshot");
            assert_eq!(frame.newer().rows::<Shown>().len(), PRESENTED as usize);
        }
        (time, rows)
    }

    /// The median of `passes` passes writing `into` on `executor`, after the full pass.
    fn median(&mut self, executor: &Executor, into: Into, passes: usize) -> Duration {
        self.install(into);
        let (_, full) = self.frame(executor);
        assert!(full >= ROWS as usize, "the first pass after a sink is full: {full} rows");
        let times: Vec<Duration> = (0..passes)
            .map(|_| {
                let (time, rows) = self.frame(executor);
                assert_eq!(rows, MOVED as usize, "a pass writes the rows that moved");
                time
            })
            .collect();
        median(times)
    }
}

/// Every figure of one ring, under `prefix`.
fn measure(prefix: &str, device: &Arc<dyn Device>, n: u32, passes: usize) {
    let mut rig = Rig::new(device);
    let executor = |workers| {
        start_executor(Workers { frame_workers: Some(workers), ..Workers::default() })
            .expect("an executor")
    };
    let wide = executor(n);
    let both = rig.median(&wide, Into::Both, passes);
    let ring = rig.median(&wide, Into::Ring, passes);
    let snapshot = rig.median(&wide, Into::Snapshot, passes);
    wide.shutdown();
    let one = executor(1);
    let single = rig.median(&one, Into::Both, passes);
    one.shutdown();
    figure(&format!("{prefix}.pass_us"), micros(both));
    figure(&format!("{prefix}.ring_alone_us"), micros(ring));
    figure(&format!("{prefix}.snapshot_alone_us"), micros(snapshot));
    // Signed: a part below zero is the noise between two medians, larger than the part.
    let part = |less: Duration| format!("{:.1}", (both.as_secs_f64() - less.as_secs_f64()) * 1e6);
    figure(&format!("{prefix}.ring_part_us"), part(snapshot));
    figure(&format!("{prefix}.snapshot_part_us"), part(ring));
    figure(&format!("{prefix}.w1.pass_us"), micros(single));
    let per_row = single.as_secs_f64() * 1e9 / MOVED as f64;
    figure(&format!("{prefix}.w1.dirty_row_ns"), format!("{per_row:.1}"));
    rig.waits.report(prefix);
}

/// `cargo xtask measure extract-50k` runs it.
#[test]
#[ignore = "a measurement on a Vulkan device, run by `cargo xtask measure`: times are reported"]
fn cost_of_extract_at_50k_dirty_rows() {
    let passes = if short() { 6 } else { 60 };
    let n = measured_workers();
    let (device, messages) = vulkan();
    measure("device", &device, n, passes);
    drop(device);
    messages.assert_none();
    let (null, _, said) = null_device(Capabilities::LEAST);
    measure("null", &null, n, passes);
    assert_eq!(said.count.load(Ordering::Relaxed), 0, "null refused nothing");
    figure("workers", format!("1 and {n}"));
    figure("rows", ROWS);
    figure("moved", MOVED);
    figure("presented", PRESENTED);
    figure("archetypes", 4);
    figure("regions", REGIONS);
    figure("passes", passes);
    figure("ring.memory_class", "Upload");
    figure("ring.regions", RING_REGIONS);
}
