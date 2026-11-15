//! The binds of a saved world (README, "Proof for D142"; D142): `Loader::binds` over a
//! save's sections gives exactly the binds their loads make and the saving world held,
//! after every step of seeded histories; it refuses what no export holds, and reads records
//! at the registry's schemas only.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, ComponentSchema, Overlay, Record};
use lactose_core::{ErrorKind, NetId, PlayerId, PredictionKey, Result, StableKey, Tick};
use lactose_ecs::{PredictedOrigin, RegionConfig, RegionId, RegionMode, World, WorldKind};
use lactose_persist::{FieldValues, Loader, Migration, Migrations, RegionRecords, SchemaTable};
use lactose_reflect::Reflect;
use lactose_sim::{RegionExport, Simulation, TickInput};

fn key(ordinal: u32) -> PredictionKey {
    PredictionKey { player: PlayerId(4), input_tick: Tick(6), ordinal }
}

fn origin(key: PredictionKey) -> PredictedOrigin {
    PredictedOrigin { player: key.player, input_tick: key.input_tick, ordinal: key.ordinal }
}

fn binds_of<'r>(
    sections: impl IntoIterator<Item = &'r RegionRecords>,
) -> Result<BTreeMap<PredictionKey, NetId>> {
    let migrations = Migrations::new();
    Loader::new(schemas(), &migrations).binds(sections)
}

/// A world of [`registry`]'s types holding `regions`, each loaded from its baseline.
fn world_of(regions: &[(RegionId, &Baseline)]) -> World {
    let mut world = bare_world();
    for &(region, baseline) in regions {
        let config =
            RegionConfig { id: region, seed: seed_of(region), mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
        world.load_region(region, baseline, None).unwrap();
    }
    world
}

/// `region`'s records in `world`, through `export_region` and `from_export`.
fn saved(world: &World, region: RegionId, baseline: &Baseline) -> RegionRecords {
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let export = RegionExport { region, rows: world.export_region(region).unwrap() };
    RegionRecords::from_export(&export, baseline, &table).unwrap()
}

/// The records of `id` in `records`.
fn records_for(records: &RegionRecords, id: NetId) -> Vec<&Record> {
    let all = records.overlay.iter().flat_map(|overlay| overlay.records());
    all.filter(|record| record.id() == id).collect()
}

/// Whether `record` carries a predicted origin.
fn carries_origin(record: &Record) -> bool {
    match record {
        Record::Spawn { components, .. } | Record::SpawnFromBaseline { components, .. } => {
            components.iter().any(|data| data.component == PredictedOrigin::KEY)
        }
        Record::AddComponent { data, .. } => data.component == PredictedOrigin::KEY,
        _ => false,
    }
}

#[test]
fn the_binds_of_a_saved_world_are_what_its_loads_bind() {
    let (a, b) = (baseline_a(), baseline_b());
    let mut world = world_of(&[(A, &a), (B, &b)]);
    // A predicted spawn bound to a runtime `NetId`: a `Spawn` that carries the origin.
    let runtime = world.allocate_net_id().unwrap();
    world.spawn(StableKey::Predicted(key(1)), A, (pos(1, 1),)).unwrap();
    world.bind(key(1), runtime).unwrap();
    // One bound by a direct bind to a cooked `NetId` no row of A's baseline holds: a
    // `SpawnFromBaseline` that carries it.
    let cooked_free = NetId::cooked(900_001);
    world.spawn(StableKey::Predicted(key(2)), A, (pos(2, 2),)).unwrap();
    world.bind(key(2), cooked_free).unwrap();
    // One bound to the `NetId` of a cooked row of the baseline, despawned first: an
    // `AddComponent` of the origin.
    world.despawn(net(c(A, 1))).unwrap();
    world.spawn(StableKey::Predicted(key(3)), A, (pos(3, 3), Health(10))).unwrap();
    world.bind(key(3), c(A, 1)).unwrap();
    // Rows without an origin; B stays in its cooked state.
    let plain = world.allocate_net_id().unwrap();
    world.spawn(net(plain), A, (pos(4, 4),)).unwrap();
    world.insert(net(c(A, 2)), Health(1)).unwrap();

    let sections = [saved(&world, A, &a), saved(&world, B, &b)];
    assert!(sections[1].overlay.is_none(), "B is cooked");
    let shape = |id: NetId| -> Vec<u8> {
        let carrying = records_for(&sections[0], id).into_iter().filter(|r| carries_origin(r));
        carrying
            .map(|record| match record {
                Record::Spawn { .. } => 1,
                Record::SpawnFromBaseline { .. } => 2,
                Record::AddComponent { .. } => 4,
                _ => 0,
            })
            .collect()
    };
    assert_eq!((shape(runtime), shape(cooked_free), shape(c(A, 1))), (vec![1], vec![2], vec![4]));
    assert!(shape(plain).is_empty() && shape(c(A, 2)).is_empty());

    let expected = BTreeMap::from([(key(1), runtime), (key(2), cooked_free), (key(3), c(A, 1))]);
    assert_eq!(binds_of(&sections).unwrap(), expected);
    assert_eq!(binds_of(sections.iter().rev()).unwrap(), expected);

    let migrations = Migrations::new();
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let loader = Loader::new(&table, &migrations);
    let baseline_of = |region| Arc::new(if region == A { a.clone() } else { b.clone() });
    let bundle = |records: &RegionRecords| {
        let baseline = baseline_of(records.region);
        loader.bundle(records, baseline, RegionMode::Simulated, None).unwrap()
    };
    // A world that loads every section, in either order, holds exactly those binds.
    for order in [[0, 1], [1, 0]] {
        let mut fresh = bare_world();
        for index in order {
            load_into(&mut fresh, sections[index].region, &bundle(&sections[index]));
        }
        for (&predicted, &id) in &expected {
            assert_eq!(fresh.alias(predicted), Some(id), "order {order:?}");
            assert!(fresh.contains(StableKey::Predicted(predicted)));
        }
        assert_eq!(fresh.alias(key(4)), None);
    }
    // A world built with them loads each section alone to the export the save was made
    // from, each of the three under its predicted origin.
    for records in &sections {
        let mut fresh = bare_world();
        for (&predicted, &id) in &expected {
            fresh.bind(predicted, id).unwrap();
        }
        load_into(&mut fresh, records.region, &bundle(records));
        let region = records.region;
        assert_eq!(fresh.export_region(region).unwrap(), world.export_region(region).unwrap());
        assert_eq!(&saved(&fresh, region, &baseline_of(region)), records);
    }
    let mut alone = bare_world();
    for (&predicted, &id) in &expected {
        alone.bind(predicted, id).unwrap();
    }
    load_into(&mut alone, A, &bundle(&sections[0]));
    for predicted in expected.keys() {
        assert!(alone.contains(StableKey::Predicted(*predicted)));
        assert_eq!(alone.region_of(StableKey::Predicted(*predicted)), Some(A));
    }
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// What the histories of [`a_save_points_binds_are_the_worlds`] reached, summed.
#[derive(Default, Debug)]
struct Reached {
    /// Binds read from a kept region's records at a check.
    kept: usize,
    /// Predicted entities despawned by the game's `age` system, a command.
    aged: usize,
    /// Predicted entities despawned through `world_mut()` between steps.
    despawned: usize,
    /// Predicted entities bound to a cooked `NetId`.
    cooked: usize,
}

/// The live predicted entities of `made`.
fn live(world: &World, made: &[PredictionKey]) -> Vec<StableKey> {
    let keys = made.iter().map(|&predicted| StableKey::Predicted(predicted));
    keys.filter(|&key| world.contains(key)).collect()
}

/// One seeded history of predicted spawns (some aging out through the game's `age`, some
/// bound to a cooked `NetId`), despawns through `world_mut()`, moves, unloads and reloads
/// over A and B, driven through `Simulation::step`; after every step, `binds` over the
/// records of the save point's regions and the kept ones holds a key exactly when the world
/// holds a bind for it, with the same `NetId`.
fn history(seed: u64, reached: &mut Reached) {
    const STEPS: u32 = 30;
    let executor = executor();
    let mut rng = Rng(seed);
    let mut simulation = game(WorldKind::Server).build().unwrap();
    let mut frame = TickInput::default();
    frame.loaded.insert(A, fresh(A));
    frame.loaded.insert(B, fresh(B));
    simulation = step(&executor, simulation, frame);
    let mut kept: BTreeMap<RegionId, RegionRecords> = BTreeMap::new();
    let mut made: Vec<PredictionKey> = Vec::new();
    let mut log = Vec::new();
    for tick in 2..=STEPS {
        let mut frame = TickInput::default();
        let loaded: Vec<RegionId> = simulation.world().regions().collect();
        let world = simulation.world_mut();
        for _ in 0..3 {
            let live = live(world, &made);
            match rng.below(9) {
                0..=2 if !loaded.is_empty() => {
                    let region = loaded[rng.below(loaded.len())];
                    let ordinal = u32::try_from(made.len()).unwrap();
                    let predicted = PredictionKey { input_tick: Tick(tick), ..key(ordinal) };
                    let aging = rng.below(3) == 0;
                    let age = Age(if aging { 89 } else { 0 });
                    world.spawn(StableKey::Predicted(predicted), region, (pos(1, 1), age)).unwrap();
                    let id = if rng.below(4) == 0 {
                        reached.cooked += 1;
                        NetId::cooked(800_000 + u64::from(ordinal))
                    } else {
                        world.allocate_net_id().unwrap()
                    };
                    world.bind(predicted, id).unwrap();
                    log.push(format!("spawn {predicted:?} as {id:?} in {region:?}, aging {aging}"));
                    made.push(predicted);
                }
                3 if !live.is_empty() => {
                    let key = live[rng.below(live.len())];
                    world.despawn(key).unwrap();
                    reached.despawned += 1;
                    log.push(format!("despawn {key:?}"));
                }
                4 if !live.is_empty() && loaded.len() == 2 => {
                    let key = live[rng.below(live.len())];
                    let to = if world.region_of(key) == Some(A) { B } else { A };
                    world.move_to_region(key, to).unwrap();
                    log.push(format!("move {key:?} to {to:?}"));
                }
                5 if !loaded.is_empty() => {
                    let region = loaded[rng.below(loaded.len())];
                    frame.unloaded.insert(region);
                    log.push(format!("unload {region:?}"));
                }
                6 if !kept.is_empty() => {
                    let regions: Vec<RegionId> = kept.keys().copied().collect();
                    let region = regions[rng.below(regions.len())];
                    let records = kept.remove(&region).unwrap();
                    frame.loaded.insert(region, bundle_of(&records, Some(first_table(region))));
                    log.push(format!("reload {region:?}"));
                }
                _ => {}
            }
        }
        let before = live(simulation.world(), &made).len();
        simulation = step(&executor, simulation, frame);
        let context = format!("seed {seed:#x} tick {tick}: {log:#?}");
        for export in simulation.lifecycle_mut().take_exports() {
            kept.insert(export.region, records_of(&export));
        }
        let unloaded: usize = kept
            .values()
            .map(|records| records_for_all(records).filter(|r| carries_origin(r)).count())
            .sum();
        let still = live(simulation.world(), &made).len();
        let aged = before.saturating_sub(still + unloaded_now(&simulation, &made, &kept));
        reached.aged += aged;
        reached.kept += unloaded;

        let (back, save) = save_of(&executor, simulation);
        simulation = back;
        let sections: Vec<RegionRecords> =
            save.regions.iter().map(records_of).chain(kept.values().cloned()).collect();
        let binds = binds_of(&sections).unwrap_or_else(|error| panic!("{error}: {context}"));
        for &predicted in &made {
            let bound = simulation.world().alias(predicted);
            assert_eq!(binds.get(&predicted).copied(), bound, "{predicted:?}: {context}");
        }
        assert!(binds.keys().all(|predicted| made.contains(predicted)), "{context}");
    }
}

/// Every record of `records`.
fn records_for_all(records: &RegionRecords) -> impl Iterator<Item = &Record> {
    records.overlay.iter().flat_map(|overlay| overlay.records())
}

/// The predicted entities of `made` that are neither live nor despawned: held in a kept
/// region's records.
fn unloaded_now(
    simulation: &Simulation,
    made: &[PredictionKey],
    kept: &BTreeMap<RegionId, RegionRecords>,
) -> usize {
    let world = simulation.world();
    let in_kept = |predicted: &PredictionKey| {
        world.alias(*predicted).is_some_and(|id| {
            kept.values().any(|records| records_for_all(records).any(|r| r.id() == id))
        })
    };
    made.iter()
        .filter(|predicted| !world.contains(StableKey::Predicted(**predicted)))
        .filter(|predicted| in_kept(predicted))
        .count()
}

#[test]
fn a_save_points_binds_are_the_worlds() {
    let mut reached = Reached::default();
    for seed in 0..200 {
        history(0xB1D5_0000 + seed, &mut reached);
    }
    // The histories reach every kind of step the check is about.
    let Reached { kept, aged, despawned, cooked } = reached;
    assert!(kept > 0 && aged > 0 && despawned > 0 && cooked > 0, "{reached:?}");
}

/// A section of `region` over an empty baseline whose overlay is `records`, with the
/// schemas and layout digests of a world's own export that carries an origin and a
/// position, so it passes `check`.
fn section(region: RegionId, records: Vec<Record>) -> RegionRecords {
    let empty = Baseline::from_rows(AssetSet::new(), cooked_schemas(), []).unwrap();
    let mut world = world_of(&[(region, &empty)]);
    world.spawn(StableKey::Predicted(key(99)), region, (pos(0, 0),)).unwrap();
    world.bind(key(99), NetId::cooked(999_999)).unwrap();
    let mut template = saved(&world, region, &empty);
    let overlay = template.overlay.as_deref().unwrap();
    let overlay = Overlay::new(overlay.baseline_hash(), overlay.schemas().to_vec(), records);
    template.overlay = Some(Arc::new(overlay.unwrap()));
    template.check().unwrap();
    template
}

/// A runtime spawn of `id` that carries `predicted`'s origin.
fn spawn(id: u64, predicted: PredictionKey) -> Record {
    let id = NetId::from_bits(id).unwrap();
    Record::Spawn { id, components: vec![data(origin(predicted)), data(pos(1, 1))] }
}

/// `result` is `InvalidData` naming every one of `named`.
fn refused(result: Result<BTreeMap<PredictionKey, NetId>>, named: &[String]) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    for name in named {
        assert!(error.message().contains(name.as_str()), "`{error}` names `{name}`");
    }
}

#[test]
fn binds_refuse_what_no_export_holds() {
    let id = |bits| NetId::from_bits(bits).unwrap();
    let (k, other) = (key(1), key(2));
    let region = |region: RegionId| format!("region {}", region.0);

    // One prediction key under two `NetId`s in one section, and across two.
    let one = section(A, vec![spawn(5, k), spawn(6, k)]);
    let named = [format!("{k:?}"), format!("{:?}", id(5)), format!("{:?}", id(6)), region(A)];
    refused(binds_of([&one]), &named);
    let (in_a, in_b) = (section(A, vec![spawn(5, k)]), section(B, vec![spawn(6, k)]));
    let named = [format!("{k:?}"), format!("{:?} in {}", id(5), region(A))];
    refused(binds_of([&in_a, &in_b]), &named);
    let named = [format!("{:?} in {}", id(6), region(B)), format!("{:?} in {}", id(5), region(A))];
    refused(binds_of([&in_b, &in_a]), &named);

    // One `NetId` under two prediction keys across two sections.
    let (first, second) = (section(A, vec![spawn(5, k)]), section(B, vec![spawn(5, other)]));
    let named = [
        format!("{:?}", id(5)),
        format!("{k:?} in {}", region(A)),
        format!("{other:?} in {}", region(B)),
    ];
    refused(binds_of([&first, &second]), &named);

    // Records that fail `check`.
    let mut broken = section(A, vec![spawn(5, k)]);
    broken.layouts.clear();
    assert!(broken.check().is_err());
    assert_eq!(binds_of([&broken]).unwrap_err().kind(), ErrorKind::InvalidData);

    // The same pairs once each are read.
    let (in_a, in_b) = (section(A, vec![spawn(5, k)]), section(B, vec![spawn(6, other)]));
    let binds = binds_of([&in_a, &in_b]).unwrap();
    assert_eq!(binds, BTreeMap::from([(k, id(5)), (other, id(6))]));
}

/// A step from version 0 that changes nothing.
fn unchanged(_: &mut FieldValues) -> Result<()> {
    Ok(())
}

#[test]
fn binds_read_records_at_the_registrys_schema() {
    let current = schemas().schema(PredictedOrigin::KEY).unwrap().clone();
    assert_eq!(current.schema_version(), 1);
    let section_under = |schema: ComponentSchema| {
        let mut records = section(A, vec![spawn(5, key(1))]);
        let overlay = records.overlay.as_deref().unwrap();
        let schemas = overlay.schemas().iter().map(|carried| {
            if carried.component() == PredictedOrigin::KEY {
                schema.clone()
            } else {
                carried.clone()
            }
        });
        let rebuilt = Overlay::new(overlay.baseline_hash(), schemas, overlay.records().to_vec());
        records.overlay = Some(Arc::new(rebuilt.unwrap()));
        records.check().unwrap();
        records
    };

    // Under another layout digest than the table's, at the table's version.
    let mut other_layout = section_under(current.clone());
    let digest = other_layout.layouts.get_mut(&PredictedOrigin::KEY).unwrap();
    *digest = schemas().layout(lactose_core::ComponentKey(Position::KEY.0)).unwrap();
    let error = binds_of([&other_layout]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{error}");
    assert!(error.message().contains("ecs::predicted_origin"), "{error}");

    // At an older version: refused as it is, read once `Loader::migrate` has run its step.
    let fields = current.fields().iter().copied();
    let older = ComponentSchema::new(PredictedOrigin::KEY, 0, current.element_size(), fields);
    let older = section_under(older.unwrap());
    let error = binds_of([&older]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{error}");
    let mut migrations = Migrations::new();
    let step = Migration { component: PredictedOrigin::KEY, from: 0, run: unchanged };
    migrations.register(step).unwrap();
    let loader = Loader::new(schemas(), &migrations);
    let migrated = loader.migrate(&older).unwrap();
    let id = NetId::from_bits(5).unwrap();
    assert_eq!(loader.binds([&migrated]).unwrap(), BTreeMap::from([(key(1), id)]));
}
