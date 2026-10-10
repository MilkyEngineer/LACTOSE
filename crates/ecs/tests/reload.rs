//! An unload and reload of a region with a transient component (review of the
//! implementation, S3): the export lists transient presence, the documents carry it as
//! `lactose-persist` may, and the reloaded entity keeps the archetype its systems run over.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, Overlay, Record};
use lactose_core::StableKey;
use lactose_ecs::{
    LayoutConfig, Mut, RegionConfig, RegionMode, Res, Schedule, StableKeyOf, WorldKind, system,
};
use lactose_reflect::Registry;

type Log = Mutex<Vec<StableKey>>;

#[system(name = "pr::fill", worlds = [server])]
fn fill(s: Mut<Scratch>, _p: &Position) {
    s.0 += 5;
}

#[system(name = "pr::read", worlds = [server], after = fill)]
fn read(s: &Scratch, key: StableKeyOf, log: Res<Log>) {
    if s.0 == 5 {
        log.lock().unwrap().push(*key);
    }
}

fn schedule() -> Schedule {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("only", &[]).add(&FILL).add(&READ);
    builder.build().unwrap()
}

fn documents(registry: &Registry, rows: &lactose_ecs::RegionRows) -> (Baseline, Overlay) {
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(registry), []).unwrap();
    // Presence of a transient component travels as the component with zeroed bytes.
    let records: Vec<Record> = rows
        .rows
        .iter()
        .map(|row| {
            let mut components = row.components.clone();
            for &component in &row.transient {
                let size = registry.get(component).unwrap().layout().size();
                components.push(lactose_bundle::ComponentData { component, bytes: vec![0; size] });
            }
            Record::Spawn { id: row.id, components }
        })
        .collect();
    let overlay = Overlay::new(baseline.content_hash(), schemas(registry), records).unwrap();
    (baseline, overlay)
}

#[test]
fn a_reload_keeps_the_transient_columns_the_systems_run_over() {
    let executor = executor();
    let mut schedule = schedule();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.insert_service::<Log>(Box::new(Mutex::new(Vec::new())));
    world.spawn(net(1), REGION, (pos(1, 1), Scratch(0))).unwrap();
    world = tick(&mut schedule, world, &executor);
    let before = std::mem::take(&mut *world.service::<Log>().unwrap().lock().unwrap());
    assert_eq!(before, vec![net(1)]);

    let rows = world.export_region(REGION).unwrap();
    assert_eq!(rows.rows[0].transient, [<Scratch as lactose_reflect::Reflect>::KEY]);
    assert_eq!(world.get::<Scratch>(net(1)), Some(&Scratch(5)));
    let (baseline, overlay) = documents(world.registry(), &rows);
    world.unload_region(REGION).unwrap();
    world
        .create_region(RegionConfig { id: REGION, seed: rows.seed, mode: RegionMode::Simulated })
        .unwrap();
    world.load_region(REGION, &baseline, Some(&overlay)).unwrap();
    assert_eq!(world.get::<Scratch>(net(1)), Some(&Scratch(0)), "present, and zeroed");
    world = tick(&mut schedule, world, &executor);
    let after = std::mem::take(&mut *world.service::<Log>().unwrap().lock().unwrap());
    assert_eq!(after, vec![net(1)], "the reloaded entity no longer runs the transient systems");
}
