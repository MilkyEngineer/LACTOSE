//! The cost scene of probe F (`README.md`, "The write-back"; `docs/verification.md`): 100k
//! entities, of which 2,000 bodies the step moves every tick, 50,000 fixed colliders and
//! 48,000 rows with a transform alone, through the real sync, step and write-back over the
//! test backend. Asserts on counts (the sync saw 2,000 changed rows and pushed none; the
//! write-back stamped exactly the moved rows) and prints the microseconds of the sync and
//! the write-back, bracketed by exclusive systems ordered around them. Also the physics
//! snapshot's size and time for 50 claimed bodies plus N statics. Timings are printed,
//! never asserted: run it in release on an idle machine for numbers that mean anything.

#![forbid(unsafe_code)]

mod common;

use std::time::Duration;

use common::*;
use lactose_core::{NetId, StableKey};
use lactose_ecs::{
    ClaimedBy, RegionConfig, RegionMode, ResMut, SystemSet, World, WorldKind, system,
};
use lactose_physics_api::{
    Collider, PhysicsConfig, Shape, Transform, Velocity, register_systems, register_types, sets,
    setup, snapshot,
};
use lactose_sim::{Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable};

const BODIES: u64 = 2_000;
const STATICS: u64 = 50_000;
const OTHERS: u64 = 48_000;

/// When each bracketing system ran, in order.
#[derive(Default)]
struct Clock {
    #[expect(
        clippy::disallowed_types,
        reason = "a measurement outside the step: printed, never asserted"
    )]
    marks: Vec<(&'static str, std::time::Instant)>,
}

impl Clock {
    #[expect(
        clippy::disallowed_types,
        reason = "a measurement outside the step: printed, never asserted"
    )]
    fn mark(&mut self, what: &'static str) {
        self.marks.push((what, std::time::Instant::now()));
    }

    /// The durations between each `begin` mark and the `end` mark after it.
    fn spans(&self, begin: &str, end: &str) -> Vec<Duration> {
        let mut spans = Vec::new();
        let mut start = None;
        for &(what, at) in &self.marks {
            if what == begin {
                start = Some(at);
            } else if what == end
                && let Some(start) = start.take()
            {
                spans.push(at - start);
            }
        }
        spans
    }
}

static BEFORE_SYNC_SET: SystemSet = SystemSet::new("cost::before_sync_set");
static AFTER_SYNC_SET: SystemSet = SystemSet::new("cost::after_sync_set").after(&[&sets::SYNC]);
static BEFORE_WRITE_BACK_SET: SystemSet = SystemSet::new("cost::before_write_back_set");
static AFTER_WRITE_BACK_SET: SystemSet =
    SystemSet::new("cost::after_write_back_set").after(&[&sets::WRITE_BACK]);

#[system(name = "cost::before_sync", worlds = [server], exclusive, in_set = BEFORE_SYNC_SET)]
fn before_sync(_world: &mut World, clock: ResMut<Clock>) {
    clock.mark("sync");
}

#[system(name = "cost::after_sync", worlds = [server], exclusive, in_set = AFTER_SYNC_SET)]
fn after_sync(_world: &mut World, clock: ResMut<Clock>) {
    clock.mark("synced");
}

#[system(name = "cost::before_write_back", worlds = [server], exclusive, in_set = BEFORE_WRITE_BACK_SET)]
fn before_write_back(_world: &mut World, clock: ResMut<Clock>) {
    clock.mark("write_back");
}

#[system(name = "cost::after_write_back", worlds = [server], exclusive, in_set = AFTER_WRITE_BACK_SET)]
fn after_write_back(_world: &mut World, clock: ResMut<Clock>) {
    clock.mark("written");
}

fn register_clock(registration: &mut Registration) {
    registration
        .stage(Stage::Physics)
        .sets(&[&BEFORE_SYNC_SET, &AFTER_SYNC_SET])
        .add(&BEFORE_SYNC)
        .add(&AFTER_SYNC);
    registration
        .stage(Stage::Post)
        .sets(&[&BEFORE_WRITE_BACK_SET, &AFTER_WRITE_BACK_SET])
        .add(&BEFORE_WRITE_BACK)
        .add(&AFTER_WRITE_BACK);
    registration.order(&BEFORE_SYNC_SET, &sets::SYNC);
    registration.order(&BEFORE_WRITE_BACK_SET, &sets::WRITE_BACK);
}

fn net(n: u64) -> StableKey {
    StableKey::Net(NetId::from_bits(n).unwrap())
}

fn median(mut spans: Vec<Duration>) -> Duration {
    spans.sort();
    spans[spans.len() / 2]
}

#[test]
fn the_cost_scene_pushes_nothing_back_and_stamps_only_the_moved_rows() {
    let calls = CallLog::default();
    let backend = Box::new(Fake::new(calls.clone()).without_contacts());
    let mut types = TypeTable::new();
    register_types(&mut types).unwrap();
    let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
        .setup(move |world| setup(world, backend, PhysicsConfig::default()))
        .setup(|world| {
            world.insert_service::<Clock>(Box::new(Clock::default()));
            Ok(())
        })
        .systems(register_systems)
        .systems(register_clock)
        .build()
        .unwrap();
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated }).unwrap();
    let moving = Velocity { linear: v(1, 0, 0), angular: v(0, 0, 0) };
    for i in 1..=BODIES {
        world.spawn(net(i), REGION, (Transform::IDENTITY, moving, floating(), sphere())).unwrap();
    }
    let cube = Collider::new(Shape::cuboid(v(1, 1, 1)));
    for i in 0..STATICS {
        world.spawn(net(100_000 + i), REGION, (Transform::IDENTITY, cube)).unwrap();
    }
    for i in 0..OTHERS {
        world.spawn(net(200_000 + i), REGION, (Transform::IDENTITY,)).unwrap();
    }
    let executor = executor(1);
    let step = |simulation: Simulation| {
        executor.block_on(simulation.step(&executor, TickInput::default())).unwrap()
    };
    // The first sync creates every body; the next settles.
    simulation = step(step(simulation));
    let created =
        calls.take().iter().filter(|call| matches!(call, Call::CreateBody { .. })).count();
    assert_eq!(created, (BODIES + STATICS) as usize);
    let mut total = Vec::new();
    for _ in 0..21 {
        let mark = simulation.world().mark();
        #[expect(
            clippy::disallowed_types,
            reason = "a measurement outside the step: printed, never asserted"
        )]
        let start = std::time::Instant::now();
        simulation = step(simulation);
        total.push(start.elapsed());
        // The write-back stamped exactly the moved rows; their velocities did not change.
        let world = simulation.world();
        assert_eq!(world.changed_since::<Transform>(mark).unwrap().count(), BODIES as usize);
        assert_eq!(world.changed_since::<Velocity>(mark).unwrap().count(), 0);
        // The sync saw those 2,000 rows and pushed none of them: a step and nothing else.
        let calls = calls.take();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0], Call::Step { .. }));
    }
    let clock = simulation.world().service::<Clock>().unwrap();
    let sync = clock.spans("sync", "synced");
    let write_back = clock.spans("write_back", "written");
    assert_eq!((sync.len(), write_back.len()), (23, 23));
    println!(
        "COST 100k entities, {BODIES} moved bodies, {STATICS} statics: median of 21 ticks: tick {:?}, \
         sync {:?}, write-back {:?}",
        median(total),
        median(sync[2..].to_vec()),
        median(write_back[2..].to_vec())
    );
}

/// How the bodies of the resting scene are keyed.
#[derive(Clone, Copy, Debug)]
enum Keys {
    Net,
    BoundPredicted,
    UnboundPredicted,
}

/// The sync's median over 21 ticks with `n` resting bodies: nothing changed, nothing
/// reported, so what is left is the sync's own per-tick work.
fn resting(keys: Keys, n: u32) -> (Duration, Duration) {
    let calls = CallLog::default();
    let backend = Box::new(Fake::new(calls.clone()).without_contacts());
    let mut types = TypeTable::new();
    register_types(&mut types).unwrap();
    let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
        .setup(move |world| setup(world, backend, PhysicsConfig::default()))
        .setup(|world| {
            world.insert_service::<Clock>(Box::new(Clock::default()));
            Ok(())
        })
        .systems(register_systems)
        .systems(register_clock)
        .build()
        .unwrap();
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated }).unwrap();
    for i in 0..n {
        let net = NetId::from_bits(1_000_000 + u64::from(i)).unwrap();
        let predicted = lactose_core::PredictionKey {
            player: PLAYER,
            input_tick: lactose_core::Tick(1),
            ordinal: i,
        };
        let key = match keys {
            Keys::Net => StableKey::Net(net),
            Keys::BoundPredicted => {
                world.bind(predicted, net).unwrap();
                StableKey::Predicted(predicted)
            }
            Keys::UnboundPredicted => StableKey::Predicted(predicted),
        };
        world
            .spawn(key, REGION, (at(i as i32, 0, 0), Velocity::ZERO, floating(), sphere()))
            .unwrap();
    }
    let executor = executor(1);
    let step = |simulation: Simulation| {
        executor.block_on(simulation.step(&executor, TickInput::default())).unwrap()
    };
    for _ in 0..3 {
        simulation = step(simulation);
    }
    calls.take();
    let mut total = Vec::new();
    for _ in 0..21 {
        #[expect(
            clippy::disallowed_types,
            reason = "a measurement outside the step: printed, never asserted"
        )]
        let start = std::time::Instant::now();
        simulation = step(simulation);
        total.push(start.elapsed());
        // At rest: the step is all the backend is told.
        assert_eq!(calls.take().len(), 1);
    }
    let sync = simulation.world().service::<Clock>().unwrap().spans("sync", "synced");
    (median(total), median(sync[3..].to_vec()))
}

/// Review of the implementation, S1 (probe C3): the sync of resting bodies keyed by bound
/// predicted keys costs what the same bodies keyed by `Net` keys cost, since a bound body's
/// alias is never looked up again; unbound predicted bodies are looked up every tick.
#[test]
fn the_sync_of_resting_bodies_by_key_kind() {
    for n in [10_000, 50_000] {
        for keys in [Keys::Net, Keys::BoundPredicted, Keys::UnboundPredicted] {
            let (tick, sync) = resting(keys, n);
            println!("RESTING {n} bodies, {keys:?} keys: tick {tick:?}, sync {sync:?}");
        }
    }
}

#[test]
fn the_snapshot_of_fifty_claimed_bodies_and_n_statics() {
    for statics in [0, 100, 1_000, 10_000] {
        let mut options = Options::new(WorldKind::Prediction);
        options.contacts = false;
        let mut h = options.build();
        for i in 0..50 {
            let claim = ClaimedBy { player: PLAYER };
            h.spawn(key(i), (at(i as i32 * 2, 0, 0), Velocity::ZERO, floating(), sphere(), claim));
        }
        for i in 0..statics {
            h.spawn(
                key(1_000 + i),
                (at(0, -(i as i32) - 2, 0), Collider::new(Shape::cuboid(v(1, 1, 1)))),
            );
        }
        h = h.run(3);
        let mut spans = Vec::new();
        let mut bytes = 0;
        for _ in 0..11 {
            #[expect(
                clippy::disallowed_types,
                reason = "a measurement outside the step: printed, never asserted"
            )]
            let start = std::time::Instant::now();
            let taken = snapshot(h.world()).unwrap();
            spans.push(start.elapsed());
            bytes = taken.size_in_bytes();
        }
        assert!(bytes > 0);
        println!(
            "SNAPSHOT 50 claimed bodies + {statics} statics (the test backend's bytes): {bytes} bytes, \
             median {:?}",
            median(spans)
        );
    }
}
