//! Corrections: a frame's facts compared at their tick, the world restored there, the
//! facts written stamped, and every later tick re-simulated, with the result equal to a
//! run that had the server's rows from that tick on.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::Tick;
use lactose_ecs::LayoutConfig;
use lactose_net_api::StagedFrame;
use lactose_predict::{CorrectionCause, Divergence};
use lactose_reflect::Reflect;
use lactose_sim::StagedState;
use lactose_sim::harness::{ChunkCapacity, layout_mode};

fn layouts() -> Vec<(LayoutConfig, u32)> {
    vec![
        (LayoutConfig::default(), 1),
        (layout_mode(ChunkCapacity::Rows(1), 3).expect("a layout"), 4),
        (layout_mode(ChunkCapacity::Rows(2), 7).expect("a layout"), 2),
    ]
}

#[test]
fn a_differing_row_is_corrected_at_its_tick_and_the_rest_resimulated() {
    for (layout, workers) in layouts() {
        let mut rig = Rig::joined_with(layout, workers);
        let mut reference = Rig::joined_with(layout, workers);
        rig.step_to(20, walking);
        // The player walked one a tick from tick 12: four at tick 15.
        reference.step_to(15, walking);
        assert_eq!(reference.position(me()), Some(at(4, 0)));
        reference.world_mut().insert(me(), at(100, 0)).unwrap();
        reference.step_to(21, walking);

        let mut frame = StagedFrame::new(Tick(15));
        exact(&mut frame, me(), &at(100, 0));
        rig.push(frame);
        rig.step(walking(21));
        let reconciled = rig.reconcile();

        assert_eq!(
            summaries(&reconciled),
            vec![(
                Tick(15),
                6,
                CorrectionCause::Mismatch,
                vec![me()],
                Divergence { entities: 1, fields: 1 }
            )]
        );
        assert_eq!((reconciled.confirmed, reconciled.frames), (Some(Tick(15)), 1));
        assert_eq!(reconciled.resimulated, 6);
        assert_eq!(reconciled.resync, None);
        assert_eq!(rig.tick(), Tick(21));
        assert_eq!(rig.position(me()), Some(at(106, 0)));
        assert_eq!(rig.hash(), reference.hash());
        // And they stay equal.
        rig.step_to(25, walking);
        reference.step_to(25, walking);
        assert_eq!(rig.hash(), reference.hash());

        let stats = rig.p().stats();
        assert_eq!(stats.corrections, 1);
        assert_eq!((stats.mismatched_entities, stats.mismatched_fields), (1, 1));
        assert_eq!(stats.deepest_correction, 6);
        assert_eq!(stats.ticks_resimulated, 6);
        assert_eq!(stats.ticks_predicted, 15);
        assert_eq!(stats.frames_reconciled, 2);
    }
}

#[test]
fn an_agreeing_frame_corrects_nothing() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let hash = rig.hash();
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(4, 0));
    exact(&mut frame, cart(), &at(50, 0));
    exact(&mut frame, cart(), &Health(10));
    rig.push(frame);
    rig.step(walking(21));
    let after = rig.hash();
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert_eq!(reconciled.resimulated, 0);
    assert_eq!(reconciled.confirmed, Some(Tick(15)));
    assert_ne!(hash, after);
    assert_eq!(rig.hash(), after);
    assert_eq!(rig.p().stats().ticks_resimulated, 0);
}

#[test]
fn a_fact_is_compared_at_its_own_tick_only() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    // At tick 16 the player was at five: a row saying five at tick 15 is a misprediction
    // there, whatever the other slots hold.
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(5, 0));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(reconciled.corrections[0].tick, Tick(15));
    assert_eq!(rig.position(me()), Some(at(11, 0)));
}

#[test]
fn a_correction_is_stamped_so_a_derived_cache_follows_it() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let before = *rig.world().get::<Trail>(cart()).unwrap();
    assert_eq!(before.x, 50);
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, cart(), &at(77, 0));
    exact(&mut frame, cart(), &Health(10));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections[0].entities, vec![cart()]);
    let after = *rig.world().get::<Trail>(cart()).unwrap();
    assert_eq!(after.x, 77);
    assert_eq!(after.updates, before.updates + 1);
}

#[test]
fn two_frames_with_facts_correct_at_the_older_and_again_at_the_newer() {
    for (layout, workers) in layouts() {
        let mut rig = Rig::joined_with(layout, workers);
        let mut reference = Rig::joined_with(layout, workers);
        rig.step_to(20, walking);
        reference.step_to(15, walking);
        reference.world_mut().insert(me(), at(100, 0)).unwrap();
        reference.world_mut().insert(cart(), Health(3)).unwrap();
        reference.step_to(17, walking);
        reference.world_mut().insert(me(), at(200, 0)).unwrap();
        reference.world_mut().insert(cart(), at(9, 9)).unwrap();
        reference.step_to(21, walking);

        let mut first = StagedFrame::new(Tick(15));
        exact(&mut first, me(), &at(100, 0));
        // Exact rows state the whole entity: the cart's position as predicted beside it.
        exact(&mut first, cart(), &at(50, 0));
        exact(&mut first, cart(), &Health(3));
        rig.push(first);
        let mut second = StagedFrame::new(Tick(17));
        // The player's x agrees with the re-simulation from 15 (102) only by chance in
        // neither field: both differ.
        exact(&mut second, me(), &at(200, 0));
        exact(&mut second, cart(), &at(9, 9));
        exact(&mut second, cart(), &Health(3));
        rig.push(second);
        rig.step(walking(21));
        let reconciled = rig.reconcile();

        assert_eq!(
            summaries(&reconciled),
            vec![
                (
                    Tick(15),
                    6,
                    CorrectionCause::Mismatch,
                    vec![me(), cart()],
                    Divergence { entities: 2, fields: 2 }
                ),
                (
                    Tick(17),
                    4,
                    CorrectionCause::Mismatch,
                    vec![me(), cart()],
                    Divergence { entities: 2, fields: 3 }
                ),
            ]
        );
        assert_eq!(reconciled.resimulated, 6);
        assert_eq!(reconciled.frames, 2);
        assert_eq!(reconciled.confirmed, Some(Tick(17)));
        assert_eq!(rig.hash(), reference.hash());
        assert_eq!(rig.p().stats().ticks_resimulated, 6);
        assert_eq!(rig.p().stats().deepest_correction, 6);
    }
}

#[test]
fn an_entity_despawned_too_early_is_respawned_in_its_last_known_region() {
    let mut rig = Rig::joined();
    rig.step_to(13, walking);
    // The prediction world loses the cart (a prediction system's mistake, here a direct
    // write): every slot from 13 lacks it.
    rig.world_mut().despawn(cart()).unwrap();
    rig.step_to(20, walking);
    assert!(!rig.world().contains(cart()));
    let mut frame = StagedFrame::new(Tick(16));
    exact(&mut frame, cart(), &at(50, 0));
    exact(&mut frame, cart(), &Health(4));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(
        summaries(&reconciled),
        vec![(
            Tick(16),
            5,
            CorrectionCause::Mismatch,
            vec![cart()],
            Divergence { entities: 1, fields: 0 }
        )]
    );
    let world = rig.world();
    assert_eq!(world.region_of(cart()), Some(REGION));
    assert_eq!(world.get::<Health>(cart()), Some(&Health(4)));
    assert_eq!(claim(world, cart()), Some(PLAYER));
}

#[test]
fn a_fact_in_a_region_neither_its_slot_nor_the_popped_tick_holds_is_skipped() {
    // No step loads `UNLOADED`: the replicated world's hook refuses the same spawn.
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    frame.spawns.insert(net(4), claimed(UNLOADED, vec![data(at(1, 1))]));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert!(!rig.world().contains(key(4)));
}

#[test]
fn a_missing_component_entity_region_or_claim_is_a_mismatch_with_no_field() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    // A component the player's row lacks, beside its position as predicted.
    exact(&mut frame, me(), &at(4, 0));
    exact(&mut frame, me(), &Health(7));
    // An entity the slot lacks, in a loaded region.
    frame.spawns.insert(net(4), claimed(OTHER_REGION, vec![data(at(1, 1))]));
    // A region.
    frame.moves.insert(cart(), OTHER_REGION);
    // A claim: the server claims the proxy for the local player.
    frame.spawns.insert(net(3), claimed(REGION, vec![data(at(-50, 0))]));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(
        summaries(&reconciled),
        vec![(
            Tick(15),
            6,
            CorrectionCause::Mismatch,
            vec![me(), cart(), proxy(), key(4)],
            Divergence { entities: 4, fields: 0 }
        )]
    );
    let world = rig.world();
    assert_eq!(world.get::<Health>(me()), Some(&Health(7)));
    assert_eq!(world.region_of(key(4)), Some(OTHER_REGION));
    assert_eq!(claim(world, key(4)), Some(PLAYER));
    assert_eq!(world.region_of(cart()), Some(OTHER_REGION));
    assert_eq!(claim(world, proxy()), Some(PLAYER));
}

#[test]
fn a_release_a_removal_and_a_tombstone_are_facts_that_correct() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    // The server releases the cart: a spawn with a slot of a live claimed entity.
    frame.spawns.insert(net(2), remote(REGION, 1, vec![data(at(50, 0)), data(Health(10))]));
    // And removes the player's position.
    frame.removed.insert((me(), Position::KEY));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(reconciled.corrections[0].entities, vec![me(), cart()]);
    assert_eq!(reconciled.corrections[0].divergence, Divergence { entities: 2, fields: 0 });
    let world = rig.world();
    assert_eq!(claim(world, cart()), None);
    assert_eq!(world.get::<Position>(me()), None);

    // A tombstone of a claimed entity: despawned at its tick and kept so.
    let mut frame = StagedFrame::new(Tick(18));
    frame.despawns.insert(
        me(),
        lactose_net_api::Tombstone {
            region: REGION,
            tick: Tick(18),
            cause: lactose_net_api::TombstoneCause::Destroyed,
        },
    );
    rig.push(frame);
    rig.step(walking(22));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(reconciled.corrections[0].tick, Tick(18));
    assert!(!rig.world().contains(me()));
}

#[test]
fn facts_are_written_in_frame_order_under_every_layout() {
    // Two frames popped in one step with facts about the same row at the same tick
    // cannot exist (a frame's tick ascends), but within a frame the hook records spawns,
    // moves, removals, rows and tombstones in that order: a removal then a row of the same
    // component leaves the row.
    let mut hashes = Vec::new();
    for (layout, workers) in layouts() {
        let mut rig = Rig::joined_with(layout, workers);
        rig.step_to(20, walking);
        let mut frame = StagedFrame::new(Tick(15));
        frame.removed.insert((cart(), Health::KEY));
        exact(&mut frame, cart(), &at(50, 0));
        exact(&mut frame, cart(), &Health(5));
        frame.moves.insert(cart(), OTHER_REGION);
        rig.push(frame);
        rig.step(walking(21));
        let reconciled = rig.reconcile();
        assert_eq!(reconciled.corrections.len(), 1);
        assert_eq!(rig.world().get::<Health>(cart()), Some(&Health(5)));
        assert_eq!(rig.world().region_of(cart()), Some(OTHER_REGION));
        hashes.push(rig.hash());
    }
    assert!(hashes.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn a_rollback_that_fails_part_way_counts_the_corrections_it_made() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(100, 0));
    rig.push(frame);
    rig.step(walking(21));
    // A hook inserted over the predictor's: the first re-simulated tick fails after the
    // correction at 15 was written.
    rig.world_mut().insert_service::<dyn StagedState>(Box::new(Inert));
    assert!(rig.try_reconcile().is_err());
    let stats = rig.p().stats();
    // The join's frame and this one.
    assert_eq!((stats.corrections, stats.frames_reconciled), (1, 2));
    assert_eq!((stats.mismatched_entities, stats.mismatched_fields), (1, 1));
    assert_eq!(stats.deepest_correction, 6);
}

/// A hook that does nothing.
struct Inert;

impl StagedState for Inert {
    fn apply(&mut self, _: &mut lactose_ecs::World, _: Tick) -> lactose_core::Result<()> {
        Ok(())
    }
}
