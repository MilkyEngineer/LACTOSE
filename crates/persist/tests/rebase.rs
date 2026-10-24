//! A rebased section is the diff of its state (README, "Records and the relevance
//! message", `rebase`; D134): records carried over to a patched baseline equal the records
//! `from_export` gives the same state against it, digest included, whatever the patch did
//! to the rows the save touched, the rows it did not, ownership, transient and derived
//! columns; what the rebase drops is returned and nothing else is.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, Overlay, Record};
use lactose_core::{ErrorKind, FieldId, NetId};
use lactose_ecs::{OwnedBy, RegionConfig, RegionMode, World, WorldBuilder, WorldKind};
use lactose_math::Scalar;
use lactose_persist::{Loader, Migrations, RegionRecords, SchemaTable};
use lactose_reflect::Reflect;
use lactose_sim::RegionExport;

fn cook(rows: Vec<BaselineRow>) -> Baseline {
    Baseline::from_rows(AssetSet::new(), cooked_schemas(), rows).unwrap()
}

fn export(world: &World) -> RegionExport {
    RegionExport { region: A, rows: world.export_region(A).unwrap() }
}

/// A world of A, loaded from [`baseline_a`], and B, cooked with `b_rows`, every row of B
/// then moved into A.
fn moved_in(b_rows: Vec<BaselineRow>) -> (World, SchemaTable) {
    let registry = Arc::new(registry());
    let table = SchemaTable::from_registry(&registry).unwrap();
    let mut world = WorldBuilder::new(WorldKind::Server, registry).build().unwrap();
    let ids: Vec<NetId> = b_rows.iter().map(|row| row.id).collect();
    for (region, baseline) in [(A, baseline_a()), (B, cook(b_rows))] {
        let config =
            RegionConfig { id: region, seed: seed_of(region), mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
        world.load_region(region, &baseline, None).unwrap();
    }
    for id in ids {
        world.move_to_region(net(id), A).unwrap();
    }
    (world, table)
}

/// `records` carried over to `patched`, and what the rebase dropped.
fn rebase_onto(
    records: &RegionRecords,
    patched: &Baseline,
    table: &SchemaTable,
) -> (RegionRecords, Vec<Record>) {
    let mut rebased = records.clone();
    let dropped = rebased.rebase(patched, table).unwrap();
    rebased.check().unwrap();
    assert_eq!(rebased.baseline_hash, patched.content_hash());
    (rebased, dropped)
}

/// The records of the state `records` load to against `patched`: bundled, loaded into a
/// fresh world, exported and diffed again.
fn reloaded(
    records: &RegionRecords,
    patched: &Arc<Baseline>,
    table: &SchemaTable,
) -> RegionRecords {
    let migrations = Migrations::new();
    let loader = Loader::new(table, &migrations);
    let bundle = loader.bundle(records, Arc::clone(patched), RegionMode::Simulated, None).unwrap();
    let mut world = bare_world();
    load_into(&mut world, A, &bundle);
    RegionRecords::from_export(&export(&world), patched, table).unwrap()
}

fn digest(records: &RegionRecords) -> Option<u64> {
    records.overlay.as_ref().map(|overlay| overlay.digest())
}

fn of(records: &RegionRecords, id: NetId) -> Vec<Record> {
    let all = records.overlay.iter().flat_map(|overlay| overlay.records());
    all.filter(|record| record.id() == id).cloned().collect()
}

fn names_owned_by(record: &Record) -> bool {
    match record {
        Record::AddComponent { data, .. } => data.component == OwnedBy::KEY,
        Record::RemoveComponent { component, .. } | Record::SetField { component, .. } => {
            *component == OwnedBy::KEY
        }
        Record::Spawn { components, .. } | Record::SpawnFromBaseline { components, .. } => {
            components.iter().any(|data| data.component == OwnedBy::KEY)
        }
        Record::Destroy { .. } | Record::OwnershipChange { .. } => false,
    }
}

#[test]
fn rebased_records_equal_the_diff_of_the_same_state() {
    let Changed { mut world, table, .. } = changed();
    // The moved-in entity gains a transient, a derived value and an owner.
    world.insert(net(c(B, 1)), Contacts(5)).unwrap();
    world.insert(net(c(B, 1)), Cache(9)).unwrap();
    world.insert(net(c(B, 1)), owned_by(c(A, 3))).unwrap();
    let saved = RegionRecords::from_export(&export(&world), &baseline_a(), &table).unwrap();

    // The patch makes it a row of A: its Position equal, its Age another, a derived
    // column, the transient it has and the owner it has.
    let mut rows = rows_a();
    rows.push(BaselineRow {
        id: c(B, 1),
        components: vec![
            data(pos(20, 20)),
            data(Age(3)),
            data(Contacts(1)),
            data(Cache(1)),
            data(owned_by(c(A, 3))),
        ],
    });
    let patched = cook(rows);
    let (rebased, dropped) = rebase_onto(&saved, &patched, &table);
    assert!(dropped.is_empty());

    let diffed = RegionRecords::from_export(&export(&world), &patched, &table).unwrap();
    assert_eq!(rebased, diffed);
    assert_eq!(digest(&rebased), digest(&diffed));
    let age = Record::AddComponent { id: c(B, 1), data: data(Age(4)) };
    assert_eq!(of(&rebased, c(B, 1)), vec![age]);
    assert!(!rebased.overlay.iter().flat_map(|overlay| overlay.records()).any(names_owned_by));

    // The bundle's own rebase is a valid overlay of the same meaning but not this one.
    let (bundle_rebased, _) = saved.overlay.as_ref().unwrap().rebase(&patched).unwrap();
    let by_bundle: Vec<&Record> =
        bundle_rebased.records().iter().filter(|record| record.id() == c(B, 1)).collect();
    assert!(by_bundle.len() > 1);
    assert!(by_bundle.into_iter().any(names_owned_by));
}

#[test]
fn ownership_after_a_rebase_is_the_diffs() {
    let row = |n: u64| BaselineRow { id: c(B, n), components: vec![data(pos(20, n as i32))] };
    let (mut world, table) = moved_in(vec![row(1), row(2), row(3)]);
    world.insert(net(c(B, 2)), owned_by(c(A, 6))).unwrap();
    world.insert(net(c(B, 3)), owned_by(c(A, 3))).unwrap();
    let saved = RegionRecords::from_export(&export(&world), &baseline_a(), &table).unwrap();

    // The patch makes each a row of A owned by c(A, 3).
    let mut rows = rows_a();
    for n in 1..=3 {
        let components = vec![data(pos(20, n as i32)), data(owned_by(c(A, 3)))];
        rows.push(BaselineRow { id: c(B, n), components });
    }
    let patched = cook(rows);
    let (rebased, _) = rebase_onto(&saved, &patched, &table);
    let diffed = RegionRecords::from_export(&export(&world), &patched, &table).unwrap();
    assert_eq!(rebased, diffed);

    let none = Record::OwnershipChange { id: c(B, 1), owner: None };
    assert_eq!(of(&rebased, c(B, 1)), vec![none]);
    let other = Record::OwnershipChange { id: c(B, 2), owner: Some(c(A, 6)) };
    assert_eq!(of(&rebased, c(B, 2)), vec![other]);
    assert_eq!(of(&rebased, c(B, 3)), vec![]);
}

#[test]
fn a_patch_reaches_what_the_save_did_not_touch() {
    let (mut world, table) = moved_in(vec![]);
    world.insert(net(c(A, 1)), Health(7)).unwrap();
    let saved = RegionRecords::from_export(&export(&world), &baseline_a(), &table).unwrap();
    let health = Record::AddComponent { id: c(A, 1), data: data(Health(7)) };
    assert_eq!(of(&saved, c(A, 1)), vec![health.clone()]);

    let patch = |position: Position, hp: i32| {
        let mut rows = rows_a();
        rows[0] = BaselineRow { id: c(A, 1), components: vec![data(position), data(Health(hp))] };
        Arc::new(cook(rows))
    };

    // The patch moves Position, which the save never touched: the patched value loads.
    let patched = patch(pos(9, 9), 10);
    let (rebased, _) = rebase_onto(&saved, &patched, &table);
    assert_eq!(of(&rebased, c(A, 1)), vec![health.clone()]);
    assert_eq!(reloaded(&rebased, &patched, &table), rebased);
    let migrations = Migrations::new();
    let bundle = Loader::new(&table, &migrations)
        .bundle(&rebased, Arc::clone(&patched), RegionMode::Simulated, None)
        .unwrap();
    let mut loaded = bare_world();
    load_into(&mut loaded, A, &bundle);
    assert_eq!(loaded.get::<Position>(net(c(A, 1))), Some(&pos(9, 9)));
    assert_eq!(loaded.get::<Health>(net(c(A, 1))), Some(&Health(7)));

    // The patch writes the saved value: nothing is left to say.
    let patched = patch(pos(1, 1), 7);
    let (rebased, _) = rebase_onto(&saved, &patched, &table);
    assert!(rebased.overlay.is_none());
    assert_eq!(reloaded(&rebased, &patched, &table), rebased);

    // The patch writes another value: the saved one stays whole.
    let patched = patch(pos(1, 1), 12);
    let (rebased, _) = rebase_onto(&saved, &patched, &table);
    assert_eq!(of(&rebased, c(A, 1)), vec![health]);
    assert_eq!(reloaded(&rebased, &patched, &table), rebased);
}

#[test]
fn rebase_returns_what_it_drops_and_nothing_else() {
    let Changed { world, table, spawned, .. } = changed();
    let saved = RegionRecords::from_export(&export(&world), &baseline_a(), &table).unwrap();
    // A `SetField` beside the diff's records, as a save from another writer may hold.
    let velocity = data(Velocity { x: Scalar::ONE, y: Scalar::ZERO });
    let set = Record::SetField {
        id: c(A, 3),
        component: Velocity::KEY,
        field: FieldId(1),
        bytes: velocity.bytes[..Scalar::BYTES].to_vec(),
    };
    let overlay = saved.overlay.as_ref().unwrap();
    let records = overlay.records().iter().cloned().chain([set.clone()]);
    let overlay = Overlay::new(saved.baseline_hash, table.schemas().cloned(), records).unwrap();
    let saved = RegionRecords { overlay: Some(Arc::new(overlay)), ..saved };
    saved.check().unwrap();

    // The patch removes c(A, 1), which the save destroys, and c(A, 4), whose owner the
    // save changed, and cooks c(A, 3) without the Marker the save removes and the Velocity
    // it sets a field of.
    let mut rows = rows_a();
    rows.retain(|row| row.id != c(A, 1) && row.id != c(A, 4));
    for row in &mut rows {
        if row.id == c(A, 3) {
            row.components = vec![data(pos(3, 3)), data(Mass(Scalar::ONE))];
        }
    }
    let patched = cook(rows);
    let (rebased, dropped) = rebase_onto(&saved, &patched, &table);

    let (_, by_bundle) = saved.overlay.as_ref().unwrap().rebase(&patched).unwrap();
    assert_eq!(dropped, by_bundle);
    let expected = [
        Record::Destroy { id: c(A, 1) },
        Record::RemoveComponent { id: c(A, 3), component: Marker::KEY },
        set,
        Record::OwnershipChange { id: c(A, 4), owner: Some(spawned) },
    ];
    assert_eq!(dropped.len(), expected.len());
    for record in &expected {
        assert!(dropped.contains(record), "{record:?} is not dropped");
    }
    let kept: Vec<&Record> = rebased.overlay.iter().flat_map(|overlay| overlay.records()).collect();
    for record in &dropped {
        assert!(!kept.contains(&record), "{record:?} is dropped and kept");
    }
}

#[test]
fn a_noncanonical_scalar_in_the_patch_is_equal() {
    let patch = |mass: Scalar| {
        let mut rows = rows_a();
        rows[1] = BaselineRow { id: c(A, 2), components: vec![data(pos(2, 2)), data(Mass(mass))] };
        Arc::new(cook(rows))
    };

    // The save holds the canonical NaN, the patch another NaN: one value, no record.
    let (mut world, table) = moved_in(vec![]);
    world.insert(net(c(A, 2)), Mass(Scalar::NAN)).unwrap();
    let saved = RegionRecords::from_export(&export(&world), &baseline_a(), &table).unwrap();
    assert!(saved.overlay.is_some());
    let patched = patch(odd_nan());
    let (rebased, _) = rebase_onto(&saved, &patched, &table);
    assert!(rebased.overlay.is_none());
    assert_eq!(rebased, RegionRecords::from_export(&export(&world), &patched, &table).unwrap());
    assert_eq!(reloaded(&rebased, &patched, &table), rebased);

    // Negative zero is a value of its own, not a spelling of zero: the save's zero stays.
    let (mut world, table) = moved_in(vec![]);
    world.insert(net(c(A, 2)), Mass(Scalar::ZERO)).unwrap();
    let saved = RegionRecords::from_export(&export(&world), &baseline_a(), &table).unwrap();
    let patched = patch(Scalar::from_f64(-0.0));
    let (rebased, _) = rebase_onto(&saved, &patched, &table);
    let zero = Record::AddComponent { id: c(A, 2), data: data(Mass(Scalar::ZERO)) };
    assert_eq!(of(&rebased, c(A, 2)), vec![zero]);
    assert_eq!(rebased, RegionRecords::from_export(&export(&world), &patched, &table).unwrap());
}

#[test]
fn an_empty_result_is_none_and_a_region_without_an_overlay_takes_the_hash() {
    let (mut world, table) = moved_in(vec![]);
    world.insert(net(c(A, 1)), Health(7)).unwrap();
    world.insert_region_resource(A, Weather(5)).unwrap();
    let state = export(&world);
    let saved = RegionRecords::from_export(&state, &baseline_a(), &table).unwrap();
    assert!(saved.overlay.is_some());

    // The patch cooks the row as the save has it: no record is left, and only the
    // resource's digest stays.
    let mut rows = rows_a();
    rows[0] = BaselineRow { id: c(A, 1), components: vec![data(pos(1, 1)), data(Health(7))] };
    let patched = Arc::new(cook(rows));
    let (rebased, dropped) = rebase_onto(&saved, &patched, &table);
    assert!(rebased.overlay.is_none());
    assert!(dropped.is_empty());
    assert!(rebased.layouts.keys().copied().eq([Weather::KEY]));
    assert_eq!(rebased, RegionRecords::from_export(&state, &patched, &table).unwrap());
    assert_eq!(reloaded(&rebased, &patched, &table), rebased);

    // A region with no overlay only takes the new hash.
    let mut cooked = rebased.clone();
    let other = Arc::new(baseline_a());
    assert!(cooked.rebase(&other, &table).unwrap().is_empty());
    assert_eq!(cooked, RegionRecords { baseline_hash: other.content_hash(), ..rebased });
}

#[test]
fn rebase_then_bundle_then_load_equals_the_export() {
    let Changed { world, table, .. } = changed();
    let saved = RegionRecords::from_export(&export(&world), &baseline_a(), &table).unwrap();
    // The patch moves a row the save touched, drops a column the save removed, gives a row
    // the save never touched a component, and re-homes the entity the world moved in.
    let mut rows = rows_a();
    for row in &mut rows {
        if row.id == c(A, 6) {
            row.components = vec![data(pos(60, 60))];
        } else if row.id == c(A, 3) {
            row.components.retain(|data| data.component != Marker::KEY);
        } else if row.id == c(A, 7) {
            row.components.push(data(Age(70)));
        }
    }
    let components = vec![data(pos(20, 20)), data(Age(3)), data(Cache(1))];
    rows.push(BaselineRow { id: c(B, 1), components });
    let patched = Arc::new(cook(rows));
    let (rebased, _) = rebase_onto(&saved, &patched, &table);
    assert_eq!(reloaded(&rebased, &patched, &table), rebased);
}

#[test]
fn records_not_at_the_tables_schemas_are_refused_unchanged() {
    let Changed { world, table, .. } = changed();
    let mut saved = RegionRecords::from_export(&export(&world), &baseline_a(), &table).unwrap();
    saved.layouts.insert(Health::KEY, table.layout(Age::KEY).unwrap());
    let before = saved.clone();
    let patched = cook(rows_a());
    let error = saved.rebase(&patched, &table).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch);
    assert_eq!(saved, before);
}
