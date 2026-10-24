//! Section 15.5's first and fourth tests with `lactose-physics-api`'s components (README,
//! "Proof for milestone 1"; D124), recorded under SOUR: bodies, a fixed collider and a
//! joint saved through this crate's records and container, reloaded in place and loaded by
//! a fresh process, which are equal at every tick (the hash, the backend's bytes and the
//! handle rows), and equal to an untouched twin. The backend is `lactose-physics-api`'s test
//! backend, included by path from its tests: it holds no solver state the rows do not
//! carry, so the twin is bit-equal here; with Rapier the twin is D124's tolerance away,
//! which `lactose-physics-rapier`'s own reload tests measure.

#![forbid(unsafe_code)]

#[path = "../../physics-api/tests/common/mod.rs"]
mod physics;
mod sour;

use std::sync::Arc;

use lactose_asset_api::AssetSet;
use lactose_bundle::Baseline;
use lactose_core::Tick;
use lactose_ecs::{LayoutConfig, RegionMode, WorldKind};
use lactose_persist::{Container, ContainerHeader, Loader, Migrations, RegionRecords, SchemaTable};
use lactose_physics_api::{BodyHandle, Joint, JointHandle, JointKind, Velocity, snapshot};
use lactose_sim::TickInput;
use lactose_sim::harness::run_recorded;
use physics::{Harness, Options, REGION, at, floating, frame, key, sphere, v};
use sour::{Sour, record};

/// A fixed collider, a body falling onto it, a body sliding past, and a joint between the
/// two bodies.
const FIXED: u64 = 1;
const FALLING: u64 = 2;
const SLIDING: u64 = 3;
const JOINT: u64 = 4;
const T0: u32 = 12;
const K: u32 = 40;

fn scene(layout: LayoutConfig, tick: u32) -> Harness {
    let mut options = Options::new(WorldKind::Server);
    options.layout = layout;
    options.tick = Tick(tick);
    options.build()
}

fn populate(h: &mut Harness) {
    let fixed = lactose_physics_api::Collider::new(lactose_physics_api::Shape::sphere(
        lactose_math::Scalar::ONE,
    ));
    h.spawn(key(FIXED), (at(0, -2, 0), fixed));
    let falling = Velocity { linear: v(0, -1, 0), angular: lactose_math::Vec3::ZERO };
    h.spawn(
        key(FALLING),
        (at(0, 0, 0), falling, floating(), sphere().with_events(lactose_math::Scalar::ZERO)),
    );
    let sliding = Velocity { linear: v(0, 0, 1), angular: lactose_math::Vec3::ZERO };
    h.spawn(key(SLIDING), (at(2, 0, 0), sliding, floating(), sphere()));
    h.spawn(key(JOINT), (Joint::new(JointKind::Spherical, key(FALLING), key(SLIDING)),));
}

fn step(sour: &Sour, mut h: Harness, input: TickInput) -> Harness {
    h.simulation = sour.executor.block_on(h.simulation.step(&sour.executor, input)).unwrap();
    h
}

/// Every body's handle row and the joint's.
type Handles = (Vec<Option<BodyHandle>>, Option<JointHandle>);

fn handles(h: &Harness) -> Handles {
    let bodies = [FIXED, FALLING, SLIDING]
        .iter()
        .map(|&n| h.world().get::<BodyHandle>(key(n)).copied())
        .collect();
    (bodies, h.world().get::<JointHandle>(key(JOINT)).copied())
}

/// The state two runs must agree on: the world hash, the backend's bytes and the handles.
fn state(h: &Harness) -> (u64, u64, Handles) {
    let bytes = snapshot(h.world()).unwrap().hash();
    (h.simulation.hash().unwrap(), bytes, handles(h))
}

#[test]
fn a_physics_scene_reloads_equal_to_a_fresh_process_and_a_twin() {
    let test = "lactose_persist::physics::reload_at_12";
    let sour = Sour::from_env().unwrap();
    let mut h = scene(sour.layout, 0);
    populate(&mut h);
    let mut recorder = sour.recorder(test, h.world().layout()).unwrap();
    let mut hashed = |tick: Tick, hash: u64| record(&mut recorder, tick, hash);
    let frames = (0..T0).map(|_| TickInput::default());
    h.simulation = run_recorded(&sour.executor, h.simulation, frames, &mut hashed).unwrap();
    assert!(handles(&h).0.iter().all(Option::is_some) && handles(&h).1.is_some());

    // The save: the region's records against an empty cooked baseline (the rows are all
    // moved in, as `SpawnFromBaseline`), in a container.
    let (simulation, save) =
        sour.executor.block_on(h.simulation.save_point(&sour.executor)).unwrap();
    h.simulation = simulation;
    let table = SchemaTable::from_registry(h.world().registry()).unwrap();
    let baseline = Arc::new(Baseline::from_rows(AssetSet::new(), [], []).unwrap());
    let export = &save.regions[0];
    assert_eq!(export.region, REGION);
    let records = RegionRecords::from_export(export, &baseline, &table).unwrap();
    let bytes = Container::assemble(
        &ContainerHeader::of(&save, &table),
        vec![records.encode_section().unwrap()],
    )
    .unwrap();
    let container = Container::decode(&bytes).unwrap();
    let migrations = Migrations::new();
    let loader = Loader::new(&table, &migrations);
    let bundle = || {
        let records = container.region(REGION).unwrap();
        loader.bundle(records, Arc::clone(&baseline), RegionMode::Simulated, None).unwrap()
    };

    // In place, one frame; recorded.
    let reload = frame([REGION], [(REGION, bundle())]);
    h.simulation = run_recorded(&sour.executor, h.simulation, [reload], &mut hashed).unwrap();
    assert_eq!(h.world().region_mode(REGION), Some(RegionMode::Simulated));

    // A fresh process at the save's tick, its own empty region gone so the bundle brings
    // it.
    let mut fresh = scene(sour.layout, save.tick.0);
    fresh.world_mut().unload_region(REGION).unwrap();
    fresh = step(&sour, fresh, frame([], [(REGION, bundle())]));

    // The twin never reloads.
    let mut twin = scene(sour.layout, 0);
    populate(&mut twin);
    for _ in 0..=T0 {
        twin = step(&sour, twin, TickInput::default());
    }

    for _ in 0..K {
        assert_eq!(fresh.tick(), h.tick());
        assert_eq!(twin.tick(), h.tick());
        let reloaded = state(&h);
        assert_eq!(state(&fresh), reloaded, "fresh process at tick {}", h.tick());
        assert_eq!(state(&twin), reloaded, "untouched twin at tick {}", h.tick());
        h.simulation =
            run_recorded(&sour.executor, h.simulation, [TickInput::default()], &mut hashed)
                .unwrap();
        fresh = step(&sour, fresh, TickInput::default());
        twin = step(&sour, twin, TickInput::default());
    }
    // The scene did something: the falling body touched the fixed one, and both moved.
    assert!(!h.recorded().contacts.is_empty());
    let moved = |n: u64| h.world().get::<lactose_physics_api::Transform>(key(n)).copied();
    assert_ne!(moved(FALLING), Some(at(0, 0, 0)));
    assert_ne!(moved(SLIDING), Some(at(2, 0, 0)));
    drop(hashed);
    sour.finish(test).unwrap();
}
