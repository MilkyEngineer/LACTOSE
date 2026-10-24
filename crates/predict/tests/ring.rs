//! The ring: one slot per predicted tick, the depth it keeps, what a slot weighs, and a
//! rollback to any slot re-simulating to the straight run under every layout.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::Tick;
use lactose_ecs::LayoutConfig;
use lactose_net_api::StagedFrame;
use lactose_predict::{CorrectionCause, DEFAULT_DEPTH};
use lactose_reflect::{Persistence, TypeRole};
use lactose_sim::harness::{ChunkCapacity, layout_mode};

#[test]
fn a_rollback_to_every_slot_resimulates_to_the_straight_run() {
    let layouts = [
        (LayoutConfig::default(), 1),
        (layout_mode(ChunkCapacity::Rows(1), 9).unwrap(), 3),
        (layout_mode(ChunkCapacity::Rows(3), 4).unwrap(), 4),
    ];
    let mut finals = Vec::new();
    for (layout, workers) in layouts {
        let mut rig = Rig::joined_with(layout, workers);
        let mut straight = Rig::joined_with(layout, workers);
        rig.step_to(20, looking);
        straight.step_to(20, looking);
        assert_eq!(rig.hash(), straight.hash());
        // Each frame reports a substitution of a tick by a sample that differs from the
        // player's by its yaw alone, which no system reads (from 13: the rebuild tick 11
        // applied no sample, so substituting 12 would stop the walk): a rollback to the tick
        // before it, with nothing in the world to change.
        for i in 0..9 {
            let present = 21 + i;
            let mut frame = StagedFrame::new(Tick(present));
            frame.substituted.insert(Tick(13 + i));
            rig.push(frame);
            rig.step(looking(present));
            straight.step(looking(present));
            let reconciled = rig.reconcile();
            assert_eq!(reconciled.corrections.len(), 1);
            let correction = &reconciled.corrections[0];
            assert_eq!(correction.tick, Tick(12 + i));
            assert_eq!(correction.cause, CorrectionCause::Substituted);
            assert_eq!(reconciled.resimulated, 9);
            assert_eq!(rig.hash(), straight.hash(), "after the rollback to {}", 12 + i);
        }
        finals.push(rig.hash());
    }
    assert!(finals.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn the_ring_keeps_the_newest_depth_slots_one_per_tick() {
    let mut rig = Rig::joined();
    let ring = rig.p().ring();
    assert_eq!((ring.depth(), ring.len()), (DEFAULT_DEPTH, 1));
    assert_eq!((ring.oldest(), ring.newest()), (Some(Tick(11)), Some(Tick(11))));
    rig.step_to(80, walking);
    let ring = rig.p().ring();
    assert_eq!(ring.len(), 60);
    assert_eq!((ring.oldest(), ring.newest()), (Some(Tick(21)), Some(Tick(80))));
    assert_eq!(ring.rebuilt_at(), None);
    assert!(ring.slot_bytes(Tick(20)).is_none());
    assert!(ring.slot_bytes(Tick(81)).is_none());
    assert!(!ring.is_empty());
}

#[test]
fn a_slot_weighs_its_snapshot_and_its_record_of_the_claimed_rows() {
    let mut rig = Rig::joined();
    rig.step_to(14, walking);
    // The record holds every replicated persistent component of every claimed entity, as
    // read back through the erased reads.
    let world = rig.world();
    let registry = world.registry();
    let mut expected = 0;
    for key in world.claimed(PLAYER) {
        for info in registry.iter() {
            let recorded = info.role() == TypeRole::Component
                && info.persistence() == Persistence::Persistent
                && info.tracking().replicated;
            if recorded && let Some(bytes) = world.get_erased(key, info.key()) {
                expected += bytes.len();
            }
        }
    }
    assert!(expected >= 8 + 8 + 4, "the player's and the cart's positions, the cart's health");
    let ring = rig.p().ring();
    let slot = ring.slot_bytes(Tick(14)).unwrap();
    assert_eq!(slot.record, expected);
    assert!(slot.world > 0);
    assert_eq!(slot.physics, 0, "no physics in this world");
    // A slot that applied a frame holds its payload too.
    let mut frame = StagedFrame::new(Tick(14));
    dequantised(&mut frame, proxy(), &at(-40, 0));
    rig.push(frame);
    rig.step(walking(15));
    let slot = rig.p().ring().slot_bytes(Tick(15)).unwrap();
    assert_eq!(slot.record, expected + 8);
    let total = rig.p().ring().bytes();
    let ticks = 11..=15;
    let sum =
        ticks.map(|t| rig.p().ring().slot_bytes(Tick(t)).unwrap()).fold(0, |s, b| s + b.world);
    assert_eq!(total.world, sum);
}
