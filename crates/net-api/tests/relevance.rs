//! The relevance message (README, "The relevance message"; proof row "The relevance
//! message"): built from a region's export, turned on the client into the bundle the sim
//! loads, and refused as `VersionMismatch` for the wrong baseline, a digest without a cache
//! or with another digest, and an overlay made against another baseline.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;

use common::*;
use lactose_bundle::{BaselineRow, Overlay, Record};
use lactose_core::{ErrorKind, NetId, Tick};
use lactose_ecs::{RegionConfig, RegionMode};
use lactose_net_api::{OverlayPayload, RegionRelevance};
use lactose_reflect::Reflect;
use lactose_sim::{RegionExport, TickInput};

fn overlay_against(baseline_hash: u64, records: Vec<Record>) -> Arc<Overlay> {
    Arc::new(Overlay::new(baseline_hash, schemas(types().registry()), records).expect("an overlay"))
}

/// A region exported from a server world, with an entity spawned at runtime.
fn export() -> RegionExport {
    let mut simulation = server(lactose_ecs::LayoutConfig::default());
    let world = simulation.world_mut();
    let config = RegionConfig { id: REGION, seed: seed_of(REGION), mode: RegionMode::Simulated };
    world.create_region(config).expect("a region");
    world.spawn(key(1), REGION, (Position { x: 1, y: 2 }, Health(3))).expect("spawn");
    let mut rows = world.export_region(REGION).expect("an export");
    // The export as `persist` hands it over, with a persistent resource and a transient key
    // standing in for the region resources a game registers.
    rows.resources.push(data(Health(77)));
    rows.transient_resources.push(Status::KEY);
    RegionExport { region: REGION, rows }
}

#[test]
fn from_export_copies_the_seed_the_resources_and_the_transient_keys() {
    let export = export();
    let base = baseline(vec![]);
    let overlay = overlay_against(base.content_hash(), vec![]);
    let message = RegionRelevance::from_export(
        &export,
        base.content_hash(),
        Some(Arc::clone(&overlay)),
        None,
    );
    assert_eq!(message.region, REGION);
    assert_eq!(message.seed, seed_of(REGION));
    assert_eq!(message.baseline_hash, base.content_hash());
    assert_eq!(message.resources, export.rows.resources);
    assert_eq!(message.transient_resources, vec![Status::KEY]);
    assert_eq!(message.overlay, OverlayPayload::Full(Arc::clone(&overlay)));
    assert_eq!(message.assets, None);
    let cooked = RegionRelevance::from_export(&export, base.content_hash(), None, None);
    assert_eq!(cooked.overlay, OverlayPayload::Cooked);
    assert_eq!(cooked.with_digest(), cooked);
    let digested = message.with_digest();
    assert_eq!(digested.overlay, OverlayPayload::Digest(overlay.digest()));
    assert_eq!(RegionRelevance { overlay: message.overlay.clone(), ..digested }, message);
}

#[test]
fn to_bundle_builds_the_bundle_the_sim_loads() {
    let cooked = NetId::cooked(REGION.0 * 1000 + 1);
    let base =
        baseline(vec![BaselineRow { id: cooked, components: vec![data(Position { x: 1, y: 1 })] }]);
    let records = vec![
        Record::Spawn {
            id: net(9),
            components: vec![data(Position { x: 9, y: 9 }), data(Health(9))],
        },
        Record::AddComponent { id: cooked, data: data(Health(4)) },
    ];
    let overlay = overlay_against(base.content_hash(), records);
    let message = RegionRelevance {
        region: REGION,
        seed: seed_of(REGION),
        baseline_hash: base.content_hash(),
        overlay: OverlayPayload::Full(Arc::clone(&overlay)),
        resources: Vec::new(),
        transient_resources: Vec::new(),
        assets: None,
    };
    let bundle = message.to_bundle(Arc::clone(&base), None, None).expect("a bundle");
    assert_eq!(bundle.seed, seed_of(REGION));
    assert_eq!(bundle.mode, RegionMode::Replicated);
    assert!(Arc::ptr_eq(&bundle.baseline, &base));
    assert!(bundle.overlay.as_ref().is_some_and(|carried| Arc::ptr_eq(carried, &overlay)));
    assert!(bundle.assets.is_none());

    // The same through a digest and the client's cached copy.
    let digested = message.with_digest();
    let from_cache = digested.to_bundle(Arc::clone(&base), Some(&overlay), None).expect("cached");
    assert!(from_cache.overlay.as_ref().is_some_and(|carried| Arc::ptr_eq(carried, &overlay)));
    let cooked_only = RegionRelevance { overlay: OverlayPayload::Cooked, ..message.clone() };
    assert!(
        cooked_only
            .to_bundle(Arc::clone(&base), Some(&overlay), None)
            .expect("cooked")
            .overlay
            .is_none()
    );

    // The sim loads it as a replicated region.
    let (mut simulation, _) = client(Tick(10));
    let mut input = TickInput::default();
    input.loaded.insert(REGION, bundle);
    let executor = executor(2);
    simulation = step(&executor, simulation, input);
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
    let world = simulation.world();
    assert_eq!(world.region(REGION).map(|info| info.mode), Some(RegionMode::Replicated));
    assert_eq!(world.get::<Position>(key(9)), Some(&Position { x: 9, y: 9 }));
    let cooked_key = lactose_core::StableKey::Net(cooked);
    assert_eq!(world.get::<Position>(cooked_key), Some(&Position { x: 1, y: 1 }));
    assert_eq!(world.get::<Health>(cooked_key), Some(&Health(4)));
}

#[test]
fn to_bundle_refuses_each_mismatch_as_version_mismatch() {
    let base = baseline(vec![]);
    let other = baseline(vec![BaselineRow {
        id: NetId::cooked(5),
        components: vec![data(Position { x: 5, y: 5 })],
    }]);
    assert_ne!(base.content_hash(), other.content_hash());
    let overlay = overlay_against(
        base.content_hash(),
        vec![Record::Spawn { id: net(9), components: vec![data(Position { x: 9, y: 9 })] }],
    );
    let message = RegionRelevance {
        region: REGION,
        seed: 1,
        baseline_hash: base.content_hash(),
        overlay: OverlayPayload::Full(Arc::clone(&overlay)),
        resources: Vec::new(),
        transient_resources: Vec::new(),
        assets: None,
    };
    // The wrong baseline.
    assert_kind(message.to_bundle(Arc::clone(&other), None, None), ErrorKind::VersionMismatch);
    // A digest with no cached copy, or a copy with another digest.
    let digested = message.with_digest();
    assert_kind(digested.to_bundle(Arc::clone(&base), None, None), ErrorKind::VersionMismatch);
    let another = overlay_against(base.content_hash(), vec![]);
    assert_ne!(another.digest(), overlay.digest());
    assert_kind(
        digested.to_bundle(Arc::clone(&base), Some(&another), None),
        ErrorKind::VersionMismatch,
    );
    // An overlay made against another baseline, under a message naming this one.
    let foreign = overlay_against(other.content_hash(), vec![]);
    let mislabelled = RegionRelevance { overlay: OverlayPayload::Full(foreign), ..message.clone() };
    assert_kind(mislabelled.to_bundle(Arc::clone(&base), None, None), ErrorKind::VersionMismatch);
    // And the matching cases succeed.
    assert!(message.to_bundle(Arc::clone(&base), None, None).is_ok());
    assert!(digested.to_bundle(Arc::clone(&base), Some(&overlay), None).is_ok());
}
