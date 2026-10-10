//! Regressions from the second review of the implementation: each test asserts the
//! contract a finding showed broken, and fails if the defect returns.

#![forbid(unsafe_code)]

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, ComponentData, Overlay, Record};
use lactose_core::{ComponentKey, EntityRef, ErrorKind, NetId, PredictionKey, StableKey, Tick};
use lactose_ecs::{
    Commands, Ctx, LayoutConfig, Mut, PresentEvents, RegionConfig, RegionMode, RegionRes,
    RegionRows, Res, Schedule, ScheduleError, StableKeyOf, SystemDescriptor, World, WorldKind,
    system,
};
use lactose_reflect::{Persistence, Reflect, Registry, TypeRole, component, resource};

fn n(id: u64) -> NetId {
    NetId::from_bits(id).unwrap()
}

fn pk(ordinal: u32) -> PredictionKey {
    PredictionKey { player: PLAYER, input_tick: t(4), ordinal }
}

/// How many ticks before the world's current tick `tick` is, across the wrap.
fn age(world: &World, tick: Tick) -> u32 {
    world.tick().0.wrapping_sub(tick.0)
}

// ---- B1: a new chunk's column tick above tick 2^31 ----

/// A spawn into a new chunk sets its column ticks, tracked column and untracked alike, to
/// the spawn's tick, whatever the tick.
#[test]
fn b1_a_new_chunk_takes_the_column_ticks_of_its_first_row_above_2_31() {
    for at in [1000, (1 << 31) + 5, 3_000_000_000, u32::MAX] {
        let mut world = world(WorldKind::Server, LayoutConfig::default());
        world.set_tick(Tick(at)).unwrap();
        world.spawn(net(1), REGION, (pos(1, 1), vel(1, 1))).unwrap();
        let position: Vec<u32> =
            world.columns::<Position>().map(|c| age(&world, c.column_tick())).collect();
        let velocity: Vec<u32> =
            world.columns::<Velocity>().map(|c| age(&world, c.column_tick())).collect();
        assert_eq!((position, velocity), (vec![0], vec![0]), "at tick {at}");
    }
}

/// What a consumer that skips chunks by column tick, as `ColumnRef::column_tick`
/// documents, sees of changes after its mark: here a row whose `Counter` changed after the
/// mark and then moved to a new chunk. Also each chunk's column tick, as an age.
fn chunk_skipping_consumer(at: u32) -> (Vec<(StableKey, u64)>, Vec<u32>) {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.set_tick(Tick(at)).unwrap();
    world.spawn(net(1), REGION, (Counter(0),)).unwrap();
    world.spawn(net(2), REGION, (Counter(0),)).unwrap();
    let mark = world.mark();
    world.insert(net(2), Counter(5)).unwrap();
    world.insert(net(2), vel(1, 1)).unwrap();
    let mut seen = Vec::new();
    let mut column_ages = Vec::new();
    for column in world.columns::<Counter>() {
        column_ages.push(age(&world, column.column_tick()));
        if age(&world, column.column_tick()) > age(&world, mark.tick()) {
            continue;
        }
        let rows = column.keys().iter().zip(column.values()).zip(column.row_ticks().unwrap());
        for ((key, value), tick) in rows {
            if age(&world, *tick) <= age(&world, mark.tick()) && value.0 != 0 {
                seen.push((*key, value.0));
            }
        }
    }
    (seen, column_ages)
}

#[test]
fn b1_a_chunk_skipping_consumer_sees_a_moved_change_at_any_tick() {
    let reference = chunk_skipping_consumer(1000);
    assert_eq!(reference.0, vec![(net(2), 5)]);
    for at in [(1 << 31) - 1, 3_000_000_000, u32::MAX] {
        assert_eq!(chunk_skipping_consumer(at), reference, "at tick {at}");
    }
}

// ---- B2: presentation events across the wrap ----

#[system(name = "regression2::chirp", worlds = [server])]
fn chirp(present: PresentEvents, key: StableKeyOf, _bomb: &Bomb) {
    present.emit(*key, AudioRequest::FOOTSTEP);
}

#[test]
fn b2_a_batch_across_the_wrap_is_in_tick_order() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.set_tick(Tick(u32::MAX - 2)).unwrap();
    world.spawn(net(1), REGION, (Bomb { fuse: 1 },)).unwrap();
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("sim", &[]).add(&CHIRP);
    let mut schedule = builder.build().unwrap();
    for _ in 0..4 {
        world = tick(&mut schedule, world, &executor);
    }
    let ticks: Vec<u32> =
        world.take_present_events().emitted.iter().map(|e| e.key.tick.0).collect();
    assert_eq!(ticks, [u32::MAX - 1, u32::MAX, 0, 1]);
}

// ---- S1: a declared commands writer in the reader's stage ----

type Seen = Mutex<Vec<u32>>;

#[system(name = "regression2::scratch_cmd", worlds = [server], writes = [Scratch])]
fn scratch_cmd(cmd: Commands, key: StableKeyOf, _hp: &Health) {
    cmd.set(*key, Scratch(42));
}

#[system(name = "regression2::scratch_cmd_too", worlds = [server], writes = [Scratch])]
fn scratch_cmd_too(cmd: Commands, key: StableKeyOf, _mass: &Mass) {
    cmd.set(*key, Scratch(43));
}

/// Writes the transient through `&mut World` at its stage's sync point.
#[system(
    name = "regression2::scratch_exclusive",
    worlds = [server],
    exclusive,
    writes = [Scratch]
)]
fn scratch_exclusive(world: &mut World) {
    world.insert(net(1), Scratch(9)).unwrap();
}

/// Declares a write it never performs.
#[system(name = "regression2::declares_only", worlds = [server], writes = [Scratch])]
fn declares_only(_hp: &Health) {}

#[system(name = "regression2::scratch_reader", worlds = [server])]
fn scratch_reader(scratch: &Scratch, seen: Res<Seen>) {
    seen.lock().unwrap().push(scratch.0);
}

#[system(name = "regression2::scratch_reader_after_cmd", worlds = [server], after = scratch_cmd)]
fn scratch_reader_after_cmd(scratch: &Scratch, seen: Res<Seen>) {
    seen.lock().unwrap().push(scratch.0);
}

#[system(
    name = "regression2::scratch_reader_after_exclusive",
    worlds = [server],
    after = scratch_exclusive
)]
fn scratch_reader_after_exclusive(scratch: &Scratch, seen: Res<Seen>) {
    seen.lock().unwrap().push(scratch.0);
}

fn scratch_world() -> World {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    world.spawn(net(1), REGION, (Health(1), Scratch(0))).unwrap();
    world
}

fn scratch_schedule(
    stages: &[&[&'static SystemDescriptor]],
    world: &World,
) -> Result<Schedule, ScheduleError> {
    let mut builder = Schedule::builder(WorldKind::Server);
    for (index, systems) in stages.iter().enumerate() {
        let stage = builder.stage(["s0", "s1", "s2"][index], &[]);
        for system in systems.iter() {
            stage.add(system);
        }
    }
    let mut schedule = builder.build()?;
    schedule.prepare(world)?;
    Ok(schedule)
}

/// Three ticks of `stages`, and what the reader saw.
fn scratch_seen(stages: &[&[&'static SystemDescriptor]]) -> Vec<u32> {
    let executor = executor();
    let mut world = scratch_world();
    let mut schedule = scratch_schedule(stages, &world).unwrap();
    for _ in 0..3 {
        world = tick(&mut schedule, world, &executor);
    }
    world.service::<Seen>().unwrap().lock().unwrap().clone()
}

#[test]
fn s1_a_commands_writer_in_the_readers_stage_is_no_writer_for_it() {
    let world = scratch_world();
    let refused = scratch_schedule(&[&[&SCRATCH_CMD, &SCRATCH_READER_AFTER_CMD]], &world);
    assert!(matches!(
        refused,
        Err(ScheduleError::TransientBeforeWriter {
            reader: "regression2::scratch_reader_after_cmd",
            ..
        })
    ));
}

#[test]
fn s1_a_commands_writer_in_an_earlier_stage_is_read_every_tick() {
    assert_eq!(scratch_seen(&[&[&SCRATCH_CMD], &[&SCRATCH_READER]]), [42, 42, 42]);
}

#[test]
fn s1_an_exclusive_writer_in_the_readers_stage_is_read_every_tick() {
    assert_eq!(scratch_seen(&[&[&SCRATCH_EXCLUSIVE, &SCRATCH_READER_AFTER_EXCLUSIVE]]), [9, 9, 9]);
}

#[test]
fn s1_two_declarations_do_not_conflict_and_an_unperformed_one_is_accepted() {
    let mut world = scratch_world();
    world.spawn(net(2), REGION, (Mass(lactose_math::Scalar::ONE), Scratch(0))).unwrap();
    assert!(scratch_schedule(&[&[&SCRATCH_CMD, &SCRATCH_CMD_TOO]], &world).is_ok());
    assert!(scratch_schedule(&[&[&DECLARES_ONLY], &[&SCRATCH_READER]], &world).is_ok());
}

// ---- S2: a restore that revives a bound `Net` entity ----

fn thrower(world: &World, key: StableKey) -> Option<StableKey> {
    world.get::<ThrownBy>(key).and_then(|thrown| thrown.thrower.get())
}

/// A `Net` entity live at the snapshot, despawned and then bound: the restore revives it,
/// and the references that named it stay on it rather than move to a predicted key that
/// is not live.
#[test]
fn s2_a_revived_net_entity_keeps_the_references_to_it() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    world.spawn(net(77), REGION, (pos(7, 7),)).unwrap();
    world.spawn(net(10), REGION, (ThrownBy { thrower: EntityRef::new(net(77)) },)).unwrap();
    let snapshot = world.snapshot();
    world.despawn(net(77)).unwrap();
    world.bind(pk(1), n(77)).unwrap();
    world.restore(&snapshot).unwrap();
    assert_eq!(world.origin(net(77)), Some(net(77)), "the restore revives the entity");
    assert_eq!(thrower(&world, net(10)), Some(net(77)));
    let referrers: Vec<StableKey> = world.referrers(net(77)).map(|r| r.source).collect();
    assert_eq!(referrers, [net(10)]);
    assert_eq!(world.referrers(StableKey::Predicted(pk(1))).count(), 0);
    // `lactose-predict` despawns the stale entity; the bind is kept.
    world.despawn(net(77)).unwrap();
    assert_eq!(world.alias(pk(1)), Some(n(77)));
}

// ---- S3: region resource updates are validated ----

/// A region resource with a reference.
#[resource(key = "regression2::boss", persistent)]
#[derive(PartialEq, Debug)]
pub struct Boss {
    /// Whom the boss hunts.
    #[field(id = 1)]
    pub target: EntityRef,
}

#[system(name = "regression2::hunt", worlds = [server, prediction])]
fn hunt(_ctx: Ctx, boss: RegionRes<Boss>) {
    let _ = boss.target.get();
}

fn boss_registry() -> Arc<Registry> {
    let mut registry = base_registry();
    registry.register::<Boss>().unwrap();
    Arc::new(registry)
}

#[test]
fn s3_an_invalid_update_is_refused_and_the_world_still_saves_loads_and_ticks() {
    let executor = executor();
    let mut world = world_with(WorldKind::Server, LayoutConfig::default(), boss_registry());
    world.insert_region_resource(REGION, Boss { target: EntityRef::new(net(5)) }).unwrap();
    let placeholder = EntityRef::new(StableKey::Placeholder(1));
    let error =
        world.update_region_resource::<Boss>(REGION, |boss| boss.target = placeholder).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(world.region_resource::<Boss>(REGION).unwrap().target, EntityRef::new(net(5)));

    let rows = world.export_region(REGION).unwrap();
    let mut fresh = world_with(WorldKind::Server, LayoutConfig::default(), boss_registry());
    for resource in rows.resources {
        fresh.insert_region_resource_erased(REGION, resource).unwrap();
    }
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("sim", &[]).add(&HUNT);
    let mut schedule = builder.build().unwrap();
    assert!(executor.block_on(schedule.run_tick(world, &executor)).is_ok());
}

// ---- S4: transient region resources through a reload ----

/// A per-tick region accumulator: transient, so hashed, zeroed every tick and never saved.
#[resource(key = "regression2::tally", transient)]
#[derive(PartialEq, Debug)]
pub struct Tally(#[field(id = 1)] pub u32);

/// A transient component beside `Scratch`.
#[component(key = "regression2::flag", transient)]
#[derive(PartialEq, Debug)]
pub struct Flag {
    /// Set every tick.
    #[field(id = 1)]
    pub on: bool,
}

type Log = Mutex<Vec<(u8, StableKey)>>;

#[system(name = "regression2::fill", worlds = [server])]
fn fill(scratch: Mut<Scratch>, flag: Mut<Flag>, _pos: &Position) {
    scratch.0 += 5;
    flag.on = true;
}

#[system(name = "regression2::read_scratch", worlds = [server], after = fill)]
fn read_scratch(_scratch: &Scratch, key: StableKeyOf, log: Res<Log>) {
    log.lock().unwrap().push((1, *key));
}

/// Logs the region and the tally it read, which the input sync point has zeroed.
#[system(name = "regression2::per_region", worlds = [server])]
fn per_region(ctx: Ctx, tally: RegionRes<Tally>, log: Res<Log>) {
    log.lock().unwrap().push((2, StableKey::Placeholder(ctx.region().0 as u32 + tally.0)));
}

fn tally_registry() -> Arc<Registry> {
    let mut registry = base_registry();
    registry.register::<Flag>().unwrap();
    registry.register::<Tally>().unwrap();
    Arc::new(registry)
}

fn tally_schedule() -> Schedule {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("sim", &[]).add(&FILL).add(&READ_SCRATCH).add(&PER_REGION);
    builder.build().unwrap()
}

fn tally_world() -> World {
    let mut world = world_with(WorldKind::Server, LayoutConfig::default(), tally_registry());
    world.insert_service::<Log>(Box::new(Mutex::new(Vec::new())));
    world.insert_region_resource(REGION, Weather(3)).unwrap();
    world.insert_region_resource(REGION, Tally(7)).unwrap();
    world.spawn(net(1), REGION, (pos(1, 1), Scratch(0), Flag { on: false })).unwrap();
    world.spawn(net(2), REGION, (pos(2, 2), Health(3))).unwrap();
    world
}

/// What a world hasher reads: every non-derived component's rows by key, and every
/// non-derived region resource.
fn hash(world: &World) -> Vec<(u64, String, Vec<u8>)> {
    let mut out = Vec::new();
    for info in world.registry().iter() {
        if info.role() != TypeRole::Component || info.persistence() == Persistence::Derived {
            continue;
        }
        for column in world.columns_erased(info.key()) {
            let size = column.element_size();
            for (row, key) in column.keys().iter().enumerate() {
                let bytes = column.bytes()[row * size..(row + 1) * size].to_vec();
                out.push((info.key().0, format!("{key:?}"), bytes));
            }
        }
    }
    for (key, bytes) in world.region_resources_erased(REGION) {
        if world.registry().get(key).unwrap().persistence() != Persistence::Derived {
            out.push((key.0, "region resource".to_owned(), bytes.to_vec()));
        }
    }
    out.sort();
    out
}

/// The baseline and overlay `lactose-persist` would write for `rows`, transient presence
/// encoded as zeroed components.
fn documents(registry: &Registry, rows: &RegionRows) -> (Baseline, Overlay) {
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(registry), []).unwrap();
    let records: Vec<Record> = rows
        .rows
        .iter()
        .map(|row| {
            let mut components = row.components.clone();
            for &component in &row.transient {
                let size = registry.get(component).unwrap().layout().size();
                components.push(ComponentData { component, bytes: vec![0; size] });
            }
            Record::Spawn { id: row.id, components }
        })
        .collect();
    let overlay = Overlay::new(baseline.content_hash(), schemas(registry), records).unwrap();
    (baseline, overlay)
}

fn reload(world: &mut World) {
    let rows = world.export_region(REGION).unwrap();
    let (baseline, overlay) = documents(world.registry(), &rows);
    world.unload_region(REGION).unwrap();
    let config = RegionConfig { id: REGION, seed: rows.seed, mode: RegionMode::Simulated };
    world.create_region(config).unwrap();
    world.load_region(REGION, &baseline, Some(&overlay)).unwrap();
    for resource in rows.resources {
        world.insert_region_resource_erased(REGION, resource).unwrap();
    }
    for resource in rows.transient_resources {
        world.insert_region_resource_zeroed(REGION, resource).unwrap();
    }
}

fn take_log(world: &World) -> Vec<(u8, StableKey)> {
    let mut log = core::mem::take(&mut *world.service::<Log>().unwrap().lock().unwrap());
    log.sort();
    log
}

#[test]
fn s4_an_export_lists_transient_resources_and_the_input_sync_point_zeroes_them() {
    let executor = executor();
    let mut world = tally_world();
    let rows = world.export_region(REGION).unwrap();
    let resources: Vec<ComponentKey> = rows.resources.iter().map(|r| r.component).collect();
    assert_eq!(resources, [<Weather as Reflect>::KEY]);
    assert_eq!(rows.transient_resources, [<Tally as Reflect>::KEY]);
    let mut schedule = tally_schedule();
    world = tick(&mut schedule, world, &executor);
    assert_eq!(world.region_resource::<Tally>(REGION), Some(&Tally(0)));
    assert_eq!(world.region_resource::<Weather>(REGION), Some(&Weather(3)));
    assert_eq!(take_log(&world), [(1, net(1)), (2, StableKey::Placeholder(REGION.0 as u32))]);
}

#[test]
fn s4_insert_zeroed_takes_only_a_transient_resource_of_a_loaded_region() {
    let mut world = tally_world();
    let error = world.insert_region_resource_zeroed(REGION, <Weather as Reflect>::KEY);
    assert_eq!(error.unwrap_err().kind(), ErrorKind::InvalidData);
    let error = world.insert_region_resource_zeroed(OTHER_REGION, <Tally as Reflect>::KEY);
    assert_eq!(error.unwrap_err().kind(), ErrorKind::NotFound);
    world.insert_region_resource_zeroed(REGION, <Tally as Reflect>::KEY).unwrap();
    assert_eq!(world.region_resource::<Tally>(REGION), Some(&Tally(0)));
}

/// A region reloaded from its export hashes as a twin that never reloaded, and runs the
/// same systems, from the first tick after the reload.
#[test]
fn s4_a_reloaded_region_hashes_as_its_twin() {
    let executor = executor();
    let (mut twin, mut reloaded) = (tally_world(), tally_world());
    let (mut twin_schedule, mut reloaded_schedule) = (tally_schedule(), tally_schedule());
    twin = tick(&mut twin_schedule, twin, &executor);
    reloaded = tick(&mut reloaded_schedule, reloaded, &executor);
    take_log(&twin);
    take_log(&reloaded);
    reload(&mut reloaded);
    for _ in 0..3 {
        twin = tick(&mut twin_schedule, twin, &executor);
        reloaded = tick(&mut reloaded_schedule, reloaded, &executor);
        let log = take_log(&twin);
        assert_eq!(log.len(), 2, "both readers ran in the twin: {log:?}");
        assert_eq!(take_log(&reloaded), log);
        assert_eq!(hash(&reloaded), hash(&twin));
    }
}

// ---- m1: one world per schedule ----

#[test]
fn m1_a_failed_prepare_does_not_tie_the_schedule() {
    let without_service = world(WorldKind::Server, LayoutConfig::default());
    let with_service = scratch_world();
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("s0", &[]).add(&DECLARES_ONLY);
    builder.stage("s1", &[]).add(&SCRATCH_READER);
    let mut schedule = builder.build().unwrap();
    assert!(matches!(
        schedule.prepare(&without_service),
        Err(ScheduleError::MissingResource { .. })
    ));
    assert_eq!(schedule.prepare(&with_service), Ok(()));
}

#[test]
fn m1_another_world_of_the_same_kind_is_named_as_such() {
    let first = scratch_world();
    let second = scratch_world();
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("s0", &[]).add(&DECLARES_ONLY);
    let mut schedule = builder.build().unwrap();
    schedule.prepare(&first).unwrap();
    let error = schedule.prepare(&second).unwrap_err();
    assert!(matches!(error, ScheduleError::WrongWorld { .. }));
    assert!(error.to_string().contains("another world of the same kind"), "{error}");
}

// ---- m2: references in region resources through bind and restore ----

fn boss_bytes(world: &World) -> Vec<Vec<u8>> {
    world.region_resources_erased(REGION).map(|(_, bytes)| bytes.to_vec()).collect()
}

fn boss_world() -> World {
    world_with(WorldKind::Prediction, LayoutConfig::default(), boss_registry())
}

#[test]
fn m2_resource_bytes_do_not_depend_on_the_order_of_insert_and_bind() {
    let boss = Boss { target: EntityRef::new(StableKey::Net(n(40))) };
    let mut insert_first = boss_world();
    insert_first.insert_region_resource(REGION, boss).unwrap();
    insert_first.bind(pk(5), n(40)).unwrap();
    let mut bind_first = boss_world();
    bind_first.bind(pk(5), n(40)).unwrap();
    bind_first.insert_region_resource(REGION, boss).unwrap();
    assert_eq!(boss_bytes(&insert_first), boss_bytes(&bind_first));
    let target = insert_first.region_resource::<Boss>(REGION).unwrap().target;
    assert_eq!(target, EntityRef::new(StableKey::Predicted(pk(5))));
}

#[test]
fn m2_a_restore_rewrites_a_resource_reference_bound_since_the_snapshot() {
    let mut world = boss_world();
    let boss = Boss { target: EntityRef::new(StableKey::Net(n(40))) };
    world.insert_region_resource(REGION, boss).unwrap();
    let snapshot = world.snapshot();
    world.bind(pk(5), n(40)).unwrap();
    world.restore(&snapshot).unwrap();
    let target = world.region_resource::<Boss>(REGION).unwrap().target;
    assert_eq!(target, EntityRef::new(StableKey::Predicted(pk(5))));
}

// ---- m3: the alias of a predicted entity a restore removed ----

#[test]
fn m3_the_horizon_prunes_the_alias_of_an_entity_the_restore_removed() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    world.set_tick(Tick(10)).unwrap();
    let snapshot = world.snapshot();
    world.spawn(StableKey::Predicted(pk(1)), REGION, (pos(1, 1),)).unwrap();
    world.bind(pk(1), n(500)).unwrap();
    world.restore(&snapshot).unwrap();
    assert!(!world.contains(StableKey::Predicted(pk(1))));
    world.set_rollback_horizon(Tick(10));
    assert_eq!(world.alias(pk(1)), Some(n(500)), "a restore can still reach the snapshot");
    world.set_rollback_horizon(Tick(11));
    assert_eq!(world.alias(pk(1)), None);
    // The server's entity now arrives under its own key.
    world.spawn(net(500), REGION, (pos(5, 5),)).unwrap();
    assert_eq!(world.origin(net(500)), Some(net(500)));
}
