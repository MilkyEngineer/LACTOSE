//! Regressions from the pre-review of the implementation: each test asserts the contract a
//! finding showed broken, and fails if the defect returns.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, ComponentData, Overlay, Record};
use lactose_core::{EntityRef, NetId, PredictionKey, StableKey, Tick};
use lactose_ecs::{
    LayoutConfig, OwnedBy, PresentEvents, RegionConfig, RegionMode, Res, Schedule, World,
    WorldBuilder, WorldKind, system,
};
use lactose_reflect::{Reflect, Registry, component};

/// A per-tick target: transient, holding a reference.
#[component(key = "probe::target_this_tick", transient)]
#[derive(PartialEq, Debug)]
pub struct TargetThisTick {
    /// What the entity aims at this tick.
    #[field(id = 1)]
    pub target: EntityRef,
}

/// A per-tick input applied by an exclusive system at the input sync point.
#[component(key = "probe::tick_input", transient)]
#[derive(PartialEq, Debug)]
pub struct TickInput(#[field(id = 1)] pub u32);

fn probe_registry() -> Arc<Registry> {
    let mut registry = base_registry();
    registry.register::<TargetThisTick>().unwrap();
    registry.register::<TickInput>().unwrap();
    Arc::new(registry)
}

#[system(name = "probe::noop", worlds = [server, prediction])]
fn noop(_dt: Res<TickDelta>) {}

fn noop_schedule(kind: WorldKind) -> Schedule {
    let mut builder = Schedule::builder(kind);
    builder.stage("only", &[]).add(&NOOP);
    builder.build().unwrap()
}

// ---- P1: zeroing a transient reference column leaves the reverse index stale ----

#[test]
fn p1_transient_reference_zeroing_keeps_the_reverse_index_consistent() {
    let executor = executor();
    let mut world = world_with(WorldKind::Server, LayoutConfig::default(), probe_registry());
    let a = net(1);
    let b = net(2);
    let c = net(3);
    for key in [a, b, c] {
        world.spawn(key, REGION, (Health(1),)).unwrap();
    }
    world.insert(a, TargetThisTick { target: EntityRef::new(b) }).unwrap();
    assert_eq!(world.referrers(b).count(), 1);
    let mut schedule = noop_schedule(WorldKind::Server);
    let mut world = tick(&mut schedule, world, &executor);
    // The input sync point zeroed the field: `a` no longer refers to `b`.
    assert_eq!(world.get::<TargetThisTick>(a).unwrap().target.get(), None);
    let stale: Vec<_> = world.referrers(b).collect();
    // Writing a new target does not clean the old entry either.
    world.insert(a, TargetThisTick { target: EntityRef::new(c) }).unwrap();
    let still: Vec<_> = world.referrers(b).collect();
    assert!(
        stale.is_empty() && still.is_empty(),
        "referrers(b) after zeroing: {stale:?}; after a rewrite to c: {still:?}"
    );
}

// ---- P2: a pass's presentation events are cancelled by a re-simulation of their tick ----

#[system(name = "probe::pass_present", worlds = [prediction], exclusive)]
fn pass_present(_world: &mut World, present: PresentEvents) {
    present.emit(net(1), AudioRequest { sound: 7 });
}

#[test]
fn p2_a_re_simulated_tick_does_not_cancel_or_redeliver_a_pass_emission() {
    let executor = executor();
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    world.spawn(net(1), REGION, (Health(1),)).unwrap();
    let mut ticks = noop_schedule(WorldKind::Prediction);
    let mut pass = Schedule::builder(WorldKind::Prediction);
    pass.stage("extract", &[]).add(&PASS_PRESENT);
    let mut pass = pass.build().unwrap();

    let snapshot = world.snapshot();
    let mut world = tick(&mut ticks, world, &executor);
    world = executor.block_on(pass.run_pass(world, &executor)).unwrap();
    let first = world.take_present_events();
    assert_eq!(first.emitted.len(), 1);

    // A rollback that changes nothing: restore, re-simulate the tick, run the pass again.
    world.restore(&snapshot).unwrap();
    let mut world = tick(&mut ticks, world, &executor);
    let after_tick = world.take_present_events();
    world = executor.block_on(pass.run_pass(world, &executor)).unwrap();
    let after_pass = world.take_present_events();
    assert!(
        after_tick.cancelled.is_empty() && after_pass.emitted.is_empty(),
        "nothing mispredicted, yet cancelled {:?} and re-delivered {:?}",
        after_tick.cancelled,
        after_pass.emitted.iter().map(|e| e.key).collect::<Vec<_>>()
    );
}

// ---- P3/P4: ownership of a predicted owner through export and load ----

const A: lactose_ecs::RegionId = REGION;
const B: lactose_ecs::RegionId = OTHER_REGION;

fn owner_key() -> PredictionKey {
    PredictionKey { player: PLAYER, input_tick: Tick(4), ordinal: 0x1234 }
}

fn server_with_owner() -> World {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.create_region(RegionConfig { id: B, seed: 2, mode: RegionMode::Simulated }).unwrap();
    let owner = StableKey::Predicted(owner_key());
    world.spawn(owner, A, (Health(1),)).unwrap();
    world.bind(owner_key(), NetId::from_bits(500).unwrap()).unwrap();
    world.spawn(net(600), B, (Health(2),)).unwrap();
    world.insert(net(600), OwnedBy { owner: EntityRef::new(owner) }).unwrap();
    world
}

/// The overlay `lactose-persist` would write for an exported region over an empty
/// baseline: a `Spawn` per row and an `OwnershipChange` for its owner.
fn documents(registry: &Registry, rows: &lactose_ecs::RegionRows) -> (Baseline, Overlay) {
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(registry), []).unwrap();
    let mut records = Vec::new();
    for row in &rows.rows {
        records.push(Record::Spawn { id: row.id, components: row.components.clone() });
        if row.owner.is_some() {
            records.push(Record::OwnershipChange { id: row.id, owner: row.owner });
        }
    }
    let overlay = Overlay::new(baseline.content_hash(), schemas(registry), records).unwrap();
    (baseline, overlay)
}

fn fresh_server() -> World {
    let mut world = WorldBuilder::new(WorldKind::Server, registry()).build().unwrap();
    for (id, seed) in [(A, 0x5EED), (B, 2)] {
        world.create_region(RegionConfig { id, seed, mode: RegionMode::Simulated }).unwrap();
    }
    world
}

fn owned_by_bytes(world: &World) -> Vec<u8> {
    bytemuck::bytes_of(world.get::<OwnedBy>(net(600)).unwrap()).to_vec()
}

#[test]
fn p3_a_load_stores_the_same_owner_bytes_and_index_in_either_region_order() {
    let source = server_with_owner();
    let registry = registry();
    let (base_a, over_a) = documents(&registry, &source.export_region(A).unwrap());
    let (base_b, over_b) = documents(&registry, &source.export_region(B).unwrap());

    let mut a_first = fresh_server();
    a_first.load_region(A, &base_a, Some(&over_a)).unwrap();
    a_first.load_region(B, &base_b, Some(&over_b)).unwrap();

    let mut b_first = fresh_server();
    b_first.load_region(B, &base_b, Some(&over_b)).unwrap();
    b_first.load_region(A, &base_a, Some(&over_a)).unwrap();

    let owner = StableKey::Predicted(owner_key());
    let owned_a: Vec<_> = a_first.owned_by(owner).collect();
    let owned_b: Vec<_> = b_first.owned_by(owner).collect();
    assert_eq!(owned_by_bytes(&a_first), owned_by_bytes(&source), "A then B");
    assert_eq!(
        (owned_by_bytes(&b_first), owned_b.clone()),
        (owned_by_bytes(&a_first), owned_a.clone()),
        "B then A stores {:?} and owned_by(P) is {owned_b:?}; A then B stores {:?}, {owned_a:?}",
        b_first.get::<OwnedBy>(net(600)).unwrap().owner.get(),
        a_first.get::<OwnedBy>(net(600)).unwrap().owner.get(),
    );
}

#[test]
fn p4_an_owned_row_exports_after_its_predicted_owner_region_unloads() {
    let mut world = server_with_owner();
    world.unload_region(A).unwrap();
    // The server, which never rolls back, sets the horizon to the current tick.
    let now = world.tick();
    world.set_rollback_horizon(Tick(now.0 + 1));
    let exported = world.export_region(B);
    assert!(exported.is_ok(), "export of B fails: {:?}", exported.err());
}

// ---- P5: setting the tick back reports old changes again and again ----

#[test]
fn p5_after_set_tick_backwards_a_change_is_reported_once() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(1), REGION, (pos(0, 0),)).unwrap();
    let mut schedule = noop_schedule(WorldKind::Server);
    for _ in 0..10 {
        world = tick(&mut schedule, world, &executor);
    }
    // A change at tick 10, then the client re-syncs its clock backwards.
    world.insert(net(1), pos(1, 1)).unwrap();
    world.set_tick(Tick(5)).unwrap();
    // A consumer after the new epoch: full pass, then a fresh mark.
    let mark = world.mark();
    world = tick(&mut schedule, world, &executor);
    let again: Vec<StableKey> =
        world.changed_since::<Position>(mark).unwrap().map(|(key, _)| key).collect();
    let mark = world.mark();
    world = tick(&mut schedule, world, &executor);
    let and_again: Vec<StableKey> =
        world.changed_since::<Position>(mark).unwrap().map(|(key, _)| key).collect();
    assert!(
        again.is_empty() && and_again.is_empty(),
        "unchanged row reported after the full pass: {again:?}, then {and_again:?}"
    );
}

// ---- P6: a transient column written only by an exclusive input system ----

#[system(name = "probe::apply_inputs", worlds = [server, prediction], exclusive)]
fn apply_inputs(world: &mut World) {
    let _ = world.insert(net(1), TickInput(3));
}

#[system(name = "probe::read_tick_input", worlds = [server, prediction])]
fn read_tick_input(input: &TickInput, _hp: &Health) {
    let _ = input.0;
}

#[test]
fn p6_a_transient_written_at_the_input_sync_point_can_be_read() {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("input", &[]).add(&APPLY_INPUTS);
    builder.stage("sim", &[]).add(&READ_TICK_INPUT);
    let mut schedule = builder.build().unwrap();
    let world = world_with(WorldKind::Server, LayoutConfig::default(), probe_registry());
    let prepared = schedule.prepare(&world);
    assert!(prepared.is_ok(), "{prepared:?}");
}

#[system(name = "probe::aim", worlds = [server, prediction])]
fn aim(cmd: lactose_ecs::Commands, key: lactose_ecs::StableKeyOf, _hp: &Health) {
    cmd.set(*key, TargetThisTick { target: EntityRef::new(*key) });
}

#[system(name = "probe::read_target", worlds = [server, prediction])]
fn read_target(target: &TargetThisTick) {
    let _ = target.target.get();
}

/// A known gap, pinned so a change to it is deliberate: a write through commands is not a
/// declared write, so a transient column written only that way has no reader. An exclusive
/// system ordered before the reader is a declared writer (`p6`).
#[test]
fn p6b_a_transient_written_only_by_commands_still_needs_a_declared_writer() {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("aim", &[]).add(&AIM);
    builder.stage("read", &[]).add(&READ_TARGET);
    let mut schedule = builder.build().unwrap();
    let world = world_with(WorldKind::Server, LayoutConfig::default(), probe_registry());
    assert!(matches!(
        schedule.prepare(&world),
        Err(lactose_ecs::ScheduleError::TransientBeforeWriter { reader: "probe::read_target", .. })
    ));
}

// ---- P9: two rollbacks between takes put one key in both lists of a batch ----

#[system(name = "probe::chirp_if", worlds = [prediction])]
fn chirp_if(present: PresentEvents, gravity: Res<Gravity>) {
    if gravity.0 == lactose_math::Scalar::TWO {
        present.emit(net(1), AudioRequest { sound: 1 });
    }
}

#[test]
fn p9_a_batch_never_holds_one_key_as_both_emitted_and_cancelled() {
    let executor = executor();
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    let mut schedule = Schedule::builder(WorldKind::Prediction);
    schedule.stage("only", &[]).add(&CHIRP_IF);
    let mut schedule = schedule.build().unwrap();
    let snapshot = world.snapshot();
    world = tick(&mut schedule, world, &executor);
    assert_eq!(world.take_present_events().emitted.len(), 1);
    // Rollback one: the correction removes the event.
    world.restore(&snapshot).unwrap();
    world.insert_resource(Gravity(lactose_math::Scalar::ONE));
    world = tick(&mut schedule, world, &executor);
    // Rollback two in the same frame: a later correction brings it back.
    world.restore(&snapshot).unwrap();
    world = tick(&mut schedule, world, &executor);
    let batch = world.take_present_events();
    let emitted: Vec<_> = batch.emitted.iter().map(|e| e.key).collect();
    assert!(
        emitted.iter().all(|key| !batch.cancelled.contains(key)),
        "emitted {emitted:?} and cancelled {:?}",
        batch.cancelled
    );
}

// ---- P7: a load that fails validation leaves half a region behind ----

#[test]
fn p7_a_failed_load_changes_nothing() {
    let registry = registry();
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(&registry), []).unwrap();
    let held = HeldItem::KEY;
    let records = [
        Record::Spawn {
            id: NetId::from_bits(10).unwrap(),
            components: vec![ComponentData {
                component: Health::KEY,
                bytes: bytemuck::bytes_of(&Health(5)).to_vec(),
            }],
        },
        // An invalid enum discriminant: rejected only when this row is spawned.
        Record::Spawn {
            id: NetId::from_bits(11).unwrap(),
            components: vec![ComponentData { component: held, bytes: vec![9] }],
        },
    ];
    let overlay = Overlay::new(baseline.content_hash(), schemas(&registry), records).unwrap();
    let mut world = fresh_server();
    let mut cursor = world.structural_log().cursor();
    let result = world.load_region(A, &baseline, Some(&overlay));
    let left: Vec<StableKey> = world.entities_in(A).collect();
    let logged = world.structural_log().since(&mut cursor).unwrap().count();
    assert!(result.is_err());
    assert!(
        left.is_empty() && logged == 0,
        "after {:?}: {left:?} live, {logged} log entries; a retry fails with {:?}",
        result.err(),
        world.load_region(A, &baseline, Some(&overlay)).err()
    );
}

// ---- P8: an exclusive system removes a resource a later system names ----

#[system(name = "probe::drop_gravity", worlds = [server], exclusive)]
fn drop_gravity(world: &mut World) {
    world.remove_resource::<Gravity>();
}

#[system(name = "probe::read_gravity", worlds = [server])]
fn read_gravity(_g: Res<Gravity>, cmd: lactose_ecs::Commands) {
    cmd.spawn_in(REGION, (Health(9),));
}

/// The system that names the removed resource is skipped and logged; the run goes on.
#[test]
fn p8_a_resource_removed_mid_run_skips_the_system_without_a_panic() {
    let executor = executor();
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("input", &[]).add(&DROP_GRAVITY);
    builder.stage("sim", &[]).add(&READ_GRAVITY);
    let mut schedule = builder.build().unwrap();
    let world = world(WorldKind::Server, LayoutConfig::default());
    assert!(schedule.prepare(&world).is_ok());
    let world = executor.block_on(schedule.run_tick(world, &executor)).unwrap();
    assert_eq!(world.entities_in(REGION).count(), 0, "the skipped system spawned nothing");
    assert!(world.resource::<Gravity>().is_none());
}
