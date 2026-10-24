//! Canonical records (README, "Records and the relevance message"): `check` refuses each
//! way a `RegionRecords` can leave the canonical form, naming the failure, and every
//! consumer that takes records refuses the same and changes nothing.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;

use common::*;
use lactose_bundle::Overlay;
use lactose_core::{ComponentKey, ErrorKind};
use lactose_ecs::RegionMode;
use lactose_persist::{
    Container, ContainerHeader, LayoutDigest, Loader, Migrations, RegionRecords,
};
use lactose_reflect::Reflect;

/// The pinned region's records at tick 30: an overlay (its spawner has spawned), two
/// persistent resources and a transient one.
fn pinned() -> RegionRecords {
    let records = records_in(&saved_at(30), PINNED);
    assert!(records.overlay.is_some());
    assert_eq!(records.resources.len(), 2);
    assert_eq!(records.transient_resources, vec![Rain::KEY]);
    records
}

type Case = (&'static str, fn(&mut RegionRecords), &'static str);

/// Every way out of the canonical form, with what the refusal names.
fn cases() -> Vec<Case> {
    vec![
        (
            "an empty overlay",
            |r| {
                r.overlay = Some(Arc::new(Overlay::new(r.baseline_hash, [], []).unwrap()));
            },
            "empty overlay",
        ),
        ("another baseline hash", |r| r.baseline_hash ^= 1, "name baseline"),
        (
            "unsorted resources",
            |r| {
                r.resources.swap(0, 1);
                r.resource_schemas.swap(0, 1);
            },
            "the resources are not strictly ascending",
        ),
        (
            "a duplicate resource",
            |r| {
                r.resources[1] = r.resources[0].clone();
                r.resource_schemas[1] = r.resource_schemas[0].clone();
            },
            "the resources are not strictly ascending",
        ),
        (
            "a schema missing",
            |r| {
                r.resource_schemas.pop();
            },
            "2 resources have 1 schemas",
        ),
        (
            "a schema of another resource",
            |r| r.resource_schemas.swap(0, 1),
            "the schema of resource",
        ),
        (
            "a value of another size than its schema's",
            |r| r.resources[0].bytes.push(0),
            "its schema states",
        ),
        (
            "unsorted transient keys",
            |r| r.transient_resources.push(ComponentKey(1)),
            "the transient resources are not strictly ascending",
        ),
        (
            "a duplicate transient key",
            |r| r.transient_resources.push(Rain::KEY),
            "the transient resources are not strictly ascending",
        ),
        (
            "a transient key that is a saved resource's",
            |r| {
                r.transient_resources.push(r.resources[0].component);
                r.transient_resources.sort_unstable();
            },
            "both saved and transient",
        ),
        (
            "a layout digest missing",
            |r| {
                r.layouts.remove(&Weather::KEY);
            },
            "no layout digest",
        ),
        (
            "a layout digest extra",
            |r| {
                r.layouts.insert(ComponentKey(12345), LayoutDigest(1));
            },
            "which the records do not carry",
        ),
    ]
}

#[test]
fn the_canonical_records_pass() {
    let records = pinned();
    records.check().unwrap();
    // The cooked state is `None`, and passes too.
    let mut cooked = records.clone();
    cooked.overlay = None;
    cooked.layouts.retain(|key, _| records.resources.iter().any(|data| data.component == *key));
    cooked.check().unwrap();
}

#[test]
fn check_refuses_each_failure_and_names_it() {
    let records = pinned();
    for (what, damage, named) in cases() {
        let mut damaged = records.clone();
        damage(&mut damaged);
        assert_ne!(damaged, records, "{what}: the case changes the records");
        let error = damaged.check().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{what}: {error}");
        assert!(error.to_string().contains(named), "{what}: `{error}` names `{named}`");
    }
}

#[test]
fn insert_refuses_and_changes_nothing() {
    let save = saved_at(30);
    let records = records_in(&save, PINNED);
    let mut container = Container::new(ContainerHeader::of(&save, schemas()));
    container.insert(records.clone()).unwrap();
    let before = container.clone();
    for (what, damage, named) in cases() {
        let mut damaged = records.clone();
        damage(&mut damaged);
        let error = container.insert(damaged).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{what}: {error}");
        assert!(error.to_string().contains(named), "{what}: {error}");
        assert_eq!(container, before, "{what}: the container is unchanged");
    }
}

#[test]
fn encode_section_refuses() {
    let records = pinned();
    for (what, damage, named) in cases() {
        let mut damaged = records.clone();
        damage(&mut damaged);
        let error = damaged.encode_section().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{what}: {error}");
        assert!(error.to_string().contains(named), "{what}: {error}");
    }
}

#[test]
fn the_loader_refuses() {
    let records = pinned();
    let migrations = Migrations::new();
    let loader = Loader::new(schemas(), &migrations);
    loader.bundle(&records, baseline(PINNED), RegionMode::Simulated, None).unwrap();
    for (what, damage, named) in cases() {
        let mut damaged = records.clone();
        damage(&mut damaged);
        let error =
            loader.bundle(&damaged, baseline(PINNED), RegionMode::Simulated, None).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{what}: {error}");
        assert!(error.to_string().contains(named), "{what}: {error}");
        let error = loader.migrate(&damaged).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{what}: {error}");
    }
}
