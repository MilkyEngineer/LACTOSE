//! Refusals (README, "Regions" and the proof table's "Refusals" row): a bundle against
//! another baseline, a second bundle for a loaded region, an unwanted bundle, a region the
//! atlas lacks, bad resources, a document whose schema is not the registry's, a table for
//! an unloaded region, a frame's unloads and modes under `Anchored`: each refused as
//! stated, nothing changed, the tick runs, the failure reported.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, ComponentSchema, Overlay};
use lactose_core::{ComponentKey, ErrorKind};
use lactose_ecs::{Executor, LogCursor, RegionId, RegionMode, WorldKind};
use lactose_reflect::Reflect;
use lactose_sim::{RegionBundle, Simulation, SimulationBuilder, TickInput};

const HELD: RegionId = region_at(0, 0);
const OTHER: RegionId = region_at(1, 0);

/// A server world without the game's systems, so that nothing but the frame changes it:
/// under `Directed` holding [`HELD`], under `Anchored` holding the pinned region.
fn holding(anchored_control: bool) -> (Executor, Simulation) {
    let executor = executor();
    let mut builder =
        SimulationBuilder::new(WorldKind::Server, types()).sessions(Box::new(Sessions::default()));
    let held = if anchored_control {
        builder = builder.regions(anchored(true));
        PINNED
    } else {
        HELD
    };
    let mut frame = TickInput::default();
    frame.loaded.insert(held, fresh(held));
    let mut simulation = step(&executor, builder.build().unwrap(), frame);
    assert_eq!(loaded(&simulation), BTreeMap::from([(held, RegionMode::Simulated)]));
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
    let _ = simulation.lifecycle_mut().take_requests();
    (executor, simulation)
}

type State = (BTreeMap<RegionId, (RegionMode, u64)>, LogCursor);

fn state(simulation: &Simulation) -> State {
    let world = simulation.world();
    let regions = world
        .regions()
        .map(|region| {
            (region, (world.region(region).unwrap().mode, simulation.world_hash(region).unwrap()))
        })
        .collect();
    (regions, world.structural_log().cursor())
}

/// Runs `frame` and checks that nothing changed, the tick ran, and exactly the failure of
/// `kind` was reported (none for `None`).
fn refused(anchored_control: bool, frame: TickInput, kind: Option<ErrorKind>) {
    let (executor, simulation) = holding(anchored_control);
    let before = state(&simulation);
    let tick = simulation.tick();
    let mut simulation = step(&executor, simulation, frame);
    assert_eq!(state(&simulation), before, "a refusal changed the world");
    assert_eq!(simulation.tick().0, tick.0 + 1, "the tick ran");
    let failures = simulation.lifecycle_mut().take_load_failures();
    match kind {
        Some(kind) => {
            assert_eq!(failures.len(), 1, "{failures:?}");
            assert_eq!(failures[0].error.kind(), kind, "{:?}", failures[0].error);
            assert_eq!(failures[0].tick.0, tick.0 + 1);
        }
        None => assert!(failures.is_empty(), "{failures:?}"),
    }
}

fn with_bundle(region: RegionId, bundle: RegionBundle) -> TickInput {
    let mut frame = TickInput::default();
    frame.loaded.insert(region, bundle);
    frame
}

#[test]
fn a_second_bundle_for_a_loaded_region() {
    refused(false, with_bundle(HELD, fresh(HELD)), Some(ErrorKind::InvalidData));
}

#[test]
fn an_overlay_against_another_baseline() {
    let mut bundle = fresh(OTHER);
    let elsewhere = baseline(region_at(4, 4));
    let overlay = Overlay::new(elsewhere.content_hash(), [], []).unwrap();
    bundle.overlay = Some(Arc::new(overlay));
    refused(false, with_bundle(OTHER, bundle), Some(ErrorKind::VersionMismatch));
}

#[test]
fn a_document_whose_schema_is_not_the_registrys() {
    let mut schema_v2 = schemas(types().registry());
    let marker = schema_v2.iter().position(|schema| schema.component() == Marker::KEY).unwrap();
    schema_v2[marker] = ComponentSchema::new(Marker::KEY, 2, 0, []).unwrap();
    let rows = [BaselineRow { id: cooked(OTHER, 1), components: vec![data(Marker)] }];
    let mut bundle = fresh(OTHER);
    bundle.baseline = Arc::new(Baseline::from_rows(AssetSet::new(), schema_v2, rows).unwrap());
    refused(false, with_bundle(OTHER, bundle), Some(ErrorKind::VersionMismatch));
}

#[test]
fn resources_the_registry_refuses() {
    let resource = |data: ComponentData| {
        let mut bundle = fresh(OTHER);
        bundle.resources.push(data);
        with_bundle(OTHER, bundle)
    };
    let unknown = ComponentData { component: ComponentKey(12345), bytes: vec![0; 4] };
    refused(false, resource(unknown), Some(ErrorKind::InvalidData));
    let short = ComponentData { component: Score::KEY, bytes: vec![0; 3] };
    refused(false, resource(short), Some(ErrorKind::VersionMismatch));
    let invalid = ComponentData { component: Switch::KEY, bytes: vec![2] };
    refused(false, resource(invalid), Some(ErrorKind::InvalidData));
    // A world resource's type, a transient one among the persistent, and a duplicate.
    refused(
        false,
        resource(data(Gravity(lactose_math::Scalar::ONE))),
        Some(ErrorKind::InvalidData),
    );
    refused(false, resource(data(Rain(0))), Some(ErrorKind::InvalidData));
    refused(false, resource(data(Weather(9))), Some(ErrorKind::InvalidData));
    // A transient key that is not a transient resource.
    let mut bundle = fresh(OTHER);
    bundle.transient_resources.push(Weather::KEY);
    refused(false, with_bundle(OTHER, bundle), Some(ErrorKind::InvalidData));
}

#[test]
fn a_table_for_an_unloaded_region_and_modes_for_one() {
    let mut frame = TickInput::default();
    frame.assets.insert(OTHER, table(&[(1, 1)]));
    frame.modes.insert(OTHER, RegionMode::Frozen);
    frame.unloaded.insert(OTHER);
    refused(false, frame, None);
}

#[test]
fn an_unwanted_bundle_under_anchored() {
    refused(true, with_bundle(OTHER, fresh(OTHER)), Some(ErrorKind::InvalidData));
}

#[test]
fn a_bundle_for_a_region_the_atlas_lacks() {
    let outside = RegionId(5);
    refused(true, with_bundle(outside, fresh(outside)), Some(ErrorKind::NotFound));
}

#[test]
fn the_frames_unloads_and_modes_under_anchored() {
    let mut frame = TickInput::default();
    frame.unloaded.insert(PINNED);
    frame.modes.insert(PINNED, RegionMode::Frozen);
    refused(true, frame, None);
}

#[test]
fn a_bundles_mode_under_anchored_is_the_policys() {
    let (executor, simulation) = holding(true);
    // A join makes the region wanted; the bundle asks for `Replicated`.
    let region = region_at(2, 2);
    let mut frame = join(1, region);
    let mut bundle = fresh(region);
    bundle.mode = RegionMode::Replicated;
    bundle.assets = None;
    frame.loaded.insert(region, bundle);
    let mut simulation = step(&executor, simulation, frame);
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
    assert_eq!(loaded(&simulation)[&region], RegionMode::Frozen, "no table yet");
}
