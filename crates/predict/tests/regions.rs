//! Facts that place a claimed entity in a region its frame loads: slot *T* lacks the
//! region and the tick the frame was popped at holds it, so the fact is compared there,
//! and kept there for later calls, since a re-simulation of that tick never re-applies a
//! claim change; a later fact about the entity for a tick before it follows it there,
//! unless it names a region that tick lacks and its own popped tick loads.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::Baseline;
use lactose_core::{PlayerId, StableKey, Tick};
use lactose_ecs::{RegionId, RegionMode};
use lactose_math::{Scalar, Vec3};
use lactose_net_api::{StagedFrame, Tombstone, TombstoneCause};
use lactose_physics_api::{Transform, Velocity};
use lactose_predict::{Absorbed, CorrectionCause, Divergence};
use lactose_reflect::Reflect;
use lactose_sim::{RegionBundle, TickInput};

/// The region the step that pops the frame for 15 loads, at 21.
const NEW: RegionId = RegionId(3);
/// A region a later step loads, at 22.
const NEW2: RegionId = RegionId(4);
/// A region no step loads: naming it is a sender defect.
const UNLOADED: RegionId = RegionId(5);

fn bundle() -> RegionBundle {
    RegionBundle {
        seed: 33,
        mode: RegionMode::Simulated,
        baseline: Arc::new(
            Baseline::from_rows(AssetSet::new(), [], []).expect("an empty baseline"),
        ),
        overlay: None,
        resources: Vec::new(),
        transient_resources: Vec::new(),
        assets: None,
    }
}

/// Steps one tick, walking, loading `NEW` in it if `load`.
fn step(rig: &mut Rig, load: bool) {
    step_loading(rig, if load { &[NEW] } else { &[] });
}

/// Steps one tick, walking, loading `regions` in it.
fn step_loading(rig: &mut Rig, regions: &[RegionId]) {
    let next = Tick(rig.tick().0 + 1);
    let mut input = TickInput::default();
    input.inputs.insert(PLAYER, walking(next.0).to_player_sample(next));
    for &region in regions {
        input.loaded.insert(region, bundle());
    }
    rig.try_step_with(input).expect("the step");
}

/// Steps and reconciles `ticks` times with no frame; the corrections made.
fn idle(rig: &mut Rig, ticks: u32) -> u64 {
    let before = rig.p().stats().corrections;
    for _ in 0..ticks {
        step(rig, false);
        let _ = rig.reconcile();
    }
    rig.p().stats().corrections - before
}

/// The server claims key 4 for the local player in `NEW` at 15, as `components`.
fn claim_into_new(components: Vec<lactose_bundle::ComponentData>) -> StagedFrame {
    let mut frame = StagedFrame::new(Tick(15));
    frame.spawns.insert(net(4), claimed(NEW, components));
    frame
}

fn new_cart() -> Vec<lactose_bundle::ComponentData> {
    vec![data(at(5, 5)), data(Health(4))]
}

/// The cart, whole, one unit off the prediction: a correction at `tick`.
fn cart_off(tick: u32) -> StagedFrame {
    let mut frame = StagedFrame::new(Tick(tick));
    exact(&mut frame, cart(), &at(51, 0));
    exact(&mut frame, cart(), &Health(10));
    frame
}

/// Joined, stepped to 20, then `first` popped and reconciled at 21, where `NEW` loads.
fn loaded_at_21(first: StagedFrame) -> (Rig, Vec<Summary>) {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    rig.push(first);
    step(&mut rig, true);
    let reconciled = rig.reconcile();
    (rig, summaries(&reconciled))
}

fn live_at(rig: &Rig, key: StableKey) -> Option<(RegionId, Position)> {
    let world = rig.world();
    assert!(!world.contains(key) || claim(world, key) == Some(PLAYER));
    Some((world.region_of(key)?, world.get::<Position>(key).copied()?))
}

#[test]
fn a_claimed_spawn_into_a_region_its_frame_loads_is_live_and_agrees() {
    let (rig, corrections) = loaded_at_21(claim_into_new(new_cart()));
    assert_eq!(corrections, vec![]);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
}

#[test]
fn it_survives_a_rollback_through_the_load_in_the_same_call() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    rig.push(claim_into_new(new_cart()));
    rig.push(cart_off(16));
    step(&mut rig, true);
    let reconciled = rig.reconcile();
    // The cart at 16; the spawn the re-simulation did not re-apply, at 21.
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(16), Tick(21)]);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
}

#[test]
fn it_survives_a_rollback_through_the_load_in_a_later_call() {
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    rig.push(cart_off(17));
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(17), Tick(21)]);
    assert_eq!(idle(&mut rig, 10), 0);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
}

#[test]
fn a_frame_for_a_tick_before_the_load_follows_the_spawn_and_corrects_once() {
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    // A frame for 18 carrying the entity: slot 18 lacks `NEW`, so its rows join the spawn
    // kept at 21, which they restate.
    let mut frame = StagedFrame::new(Tick(18));
    exact(&mut frame, key(4), &at(6, 5));
    exact(&mut frame, key(4), &Health(4));
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    assert_eq!(
        summaries(&reconciled),
        vec![(
            Tick(21),
            1,
            CorrectionCause::Mismatch,
            vec![key(4)],
            Divergence { entities: 1, fields: 1 }
        )]
    );
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(6, 5))));
    // A rollback through 21 re-applies what was kept, the rows included.
    rig.push(cart_off(19));
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(19), Tick(21)]);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(6, 5))));
    // The kept facts are one statement the slot agrees with: no correction repeats.
    assert_eq!(idle(&mut rig, 10), 0);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(6, 5))));
}

#[test]
fn a_claimed_entitys_move_into_a_region_its_frame_loads_is_applied() {
    let mut frame = StagedFrame::new(Tick(15));
    frame.moves.insert(cart(), NEW);
    let (mut rig, corrections) = loaded_at_21(frame);
    assert_eq!(
        corrections,
        vec![(
            Tick(21),
            0,
            CorrectionCause::Mismatch,
            vec![cart()],
            Divergence { entities: 1, fields: 0 }
        )]
    );
    assert_eq!(live_at(&rig, cart()), Some((NEW, at(50, 0))));
    // A rollback through 21 moves it again.
    rig.push(cart_off(17));
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 5), 0);
    assert_eq!(live_at(&rig, cart()), Some((NEW, at(51, 0))));
}

#[test]
fn a_swap_in_one_frame_destroys_the_old_entity_and_keeps_the_new() {
    let mut frame = claim_into_new(new_cart());
    frame.despawns.insert(
        cart(),
        Tombstone { region: REGION, tick: Tick(15), cause: TombstoneCause::Destroyed },
    );
    let (mut rig, _) = loaded_at_21(frame);
    assert!(!rig.world().contains(cart()));
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
    assert_eq!(idle(&mut rig, 5), 0);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
}

#[test]
fn a_claimed_spawn_destroyed_before_the_load_is_destroyed() {
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    assert!(rig.world().contains(key(4)));
    // The tombstone for 17, popped at 22: slot 17 lacks the entity, so it follows the
    // spawn to 21 and replaces it.
    let mut frame = StagedFrame::new(Tick(17));
    frame.despawns.insert(
        key(4),
        Tombstone { region: NEW, tick: Tick(17), cause: TombstoneCause::Destroyed },
    );
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(21)]);
    assert!(!rig.world().contains(key(4)));
    // Nor does a rollback through 21 bring it back.
    rig.push(cart_off(19));
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 10), 0);
    assert!(!rig.world().contains(key(4)));
}

#[test]
fn a_removal_before_the_load_is_folded_into_the_spawn() {
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    let mut frame = StagedFrame::new(Tick(17));
    frame.removed.insert((key(4), Health::KEY));
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(rig.world().get::<Health>(key(4)), None);
    rig.push(cart_off(19));
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 10), 0);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
    assert_eq!(rig.world().get::<Health>(key(4)), None);
}

#[test]
fn a_move_before_the_load_is_folded_into_the_spawn() {
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    let mut frame = StagedFrame::new(Tick(17));
    frame.moves.insert(key(4), REGION);
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(live_at(&rig, key(4)), Some((REGION, at(5, 5))));
    rig.push(cart_off(19));
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 10), 0);
    assert_eq!(live_at(&rig, key(4)), Some((REGION, at(5, 5))));
}

#[test]
fn a_release_before_the_load_replaces_the_spawn() {
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    let mut frame = StagedFrame::new(Tick(17));
    frame.spawns.insert(net(4), remote(NEW, 2, vec![data(at(7, 5)), data(Health(4))]));
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(21)]);
    rig.push(cart_off(19));
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 10), 0);
    // A proxy now: the frame popped at 22 spawns it again in a re-simulation.
    let world = rig.world();
    assert_eq!(claim(world, key(4)), None);
    assert_eq!(world.region_of(key(4)), Some(NEW));
    assert_eq!(world.get::<Position>(key(4)), Some(&at(7, 5)));
}

#[test]
fn a_band_spawn_into_a_region_its_frame_loads_is_kept_too() {
    // Joined: the restart is 11 and a frame for 6 is from the band; it is popped at 12,
    // where `NEW` loads, which slot 11 lacks.
    let mut rig = Rig::joined();
    let mut frame = StagedFrame::new(Tick(6));
    frame.spawns.insert(net(4), claimed(NEW, new_cart()));
    rig.push(frame);
    step(&mut rig, true);
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty(), "{:?}", summaries(&reconciled));
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
    // A rollback from the restart through 12.
    rig.push(cart_off(11));
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(11), Tick(12)]);
    assert_eq!(idle(&mut rig, 5), 0);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
}

/// A transform at `(x, 0, 0)`.
fn placed(x: f64) -> Transform {
    Transform::at(Vec3::new(Scalar::from_f64(x), Scalar::ZERO, Scalar::ZERO))
}

#[test]
fn a_kept_fact_counts_what_the_tolerance_absorbed_once() {
    let body =
        |x: f64| vec![data(placed(x)), data(Velocity::ZERO), data(floating()), data(sphere())];
    let (mut rig, corrections) = loaded_at_21(claim_into_new(body(1.0)));
    assert_eq!(corrections, vec![]);
    // A dynamic body's rows for 17, within the tolerance of the kept spawn.
    let mut frame = StagedFrame::new(Tick(17));
    for data in body(1.001) {
        assert!(frame.push_row(key(4), data.component, Tick(17), true, &data.bytes));
    }
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty(), "{:?}", summaries(&reconciled));
    let absorbed = rig.p().stats().absorbed;
    assert_eq!(absorbed.fields, 1);
    // Compared again in every later call, and written by a rollback through 21, but
    // counted once.
    assert_eq!(idle(&mut rig, 5), 0);
    rig.push(cart_off(19));
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 5), 0);
    assert_eq!(rig.p().stats().absorbed, absorbed);
    assert_ne!(absorbed, Absorbed::default());
    assert_eq!(rig.world().get::<Transform>(key(4)), Some(&placed(1.001)));
}

#[test]
fn a_kept_fact_folded_with_a_move_counts_no_absorbed_again() {
    let body =
        |x: f64| vec![data(placed(x)), data(Velocity::ZERO), data(floating()), data(sphere())];
    let (mut rig, _) = loaded_at_21(claim_into_new(body(1.0)));
    let mut frame = StagedFrame::new(Tick(17));
    for data in body(1.001) {
        assert!(frame.push_row(key(4), data.component, Tick(17), true, &data.bytes));
    }
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    let absorbed = rig.p().stats().absorbed;
    assert_eq!(absorbed.fields, 1);
    // A move folded into the kept statement states no transform.
    let mut frame = StagedFrame::new(Tick(18));
    frame.moves.insert(key(4), REGION);
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 5), 0);
    assert_eq!(rig.p().stats().absorbed, absorbed);
    assert_eq!(rig.world().region_of(key(4)), Some(REGION));
}

/// Claimed into `NEW` at 15, popped at 21; the frame for `first`, popped at 22, moves it
/// into `NEW2`, which that step loads.
fn moved_into_new2(first: StagedFrame, mover: StableKey, tick: u32) -> Rig {
    let (mut rig, _) = loaded_at_21(first);
    let mut frame = StagedFrame::new(Tick(tick));
    frame.moves.insert(mover, NEW2);
    rig.push(frame);
    step_loading(&mut rig, &[NEW2]);
    let _ = rig.reconcile();
    rig
}

#[test]
fn a_move_that_follows_a_kept_spawn_into_a_region_its_slot_lacks_is_kept_at_its_own_load() {
    // The move for 17 follows the spawn kept at 21, which lacks `NEW2`: kept at 22.
    let mut rig = moved_into_new2(claim_into_new(new_cart()), key(4), 17);
    assert_eq!(live_at(&rig, key(4)), Some((NEW2, at(5, 5))));
    assert_eq!(idle(&mut rig, 20), 0);
    // A rollback from before both: the spawn written at 21, the move at 22.
    rig.push(cart_off(18));
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(18), Tick(21), Tick(22)]);
    assert_eq!(idle(&mut rig, 10), 0);
    assert_eq!(live_at(&rig, key(4)), Some((NEW2, at(5, 5))));
}

#[test]
fn the_same_move_for_a_tick_after_the_kept_one_is_compared_at_its_own_slot() {
    let mut rig = moved_into_new2(claim_into_new(new_cart()), key(4), 22);
    assert_eq!(live_at(&rig, key(4)), Some((NEW2, at(5, 5))));
    assert_eq!(idle(&mut rig, 20), 0);
}

#[test]
fn two_moves_into_regions_each_frame_loads_end_in_the_second() {
    // The cart moved into `NEW` at 15 (popped at 21, kept), then into `NEW2` at 17.
    let mut first = StagedFrame::new(Tick(15));
    first.moves.insert(cart(), NEW);
    let mut rig = moved_into_new2(first, cart(), 17);
    assert_eq!(idle(&mut rig, 20), 0);
    assert_eq!(rig.world().region_of(cart()), Some(NEW2));
}

#[test]
fn a_move_into_a_region_neither_slot_holds_that_follows_a_kept_spawn_is_skipped() {
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    let mut frame = StagedFrame::new(Tick(17));
    frame.moves.insert(key(4), UNLOADED);
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty(), "{:?}", summaries(&reconciled));
    assert_eq!(idle(&mut rig, 20), 0);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
}

#[test]
fn a_move_into_a_region_neither_slot_holds_is_skipped() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(17));
    frame.moves.insert(cart(), UNLOADED);
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty(), "{:?}", summaries(&reconciled));
    assert_eq!(idle(&mut rig, 20), 0);
    assert_eq!(rig.world().region_of(cart()), Some(REGION));
}

#[test]
fn a_claimed_spawn_naming_a_region_neither_slot_holds_still_corrects_its_components() {
    // The cart, claimed again for 17 in a region no step loads, with another `Health`:
    // the components are written, the region is not.
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(17));
    frame.spawns.insert(net(2), claimed(UNLOADED, vec![data(at(50, 0)), data(Health(7))]));
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(17)]);
    assert_eq!(idle(&mut rig, 10), 0);
    assert_eq!(rig.world().get::<Health>(cart()), Some(&Health(7)));
    assert_eq!(rig.world().region_of(cart()), Some(REGION));
}

/// Joined, `claim_into_new` kept at 21, then `frame` reconciled at 22, with the ticks it
/// corrected, and the cart off at 19: a rollback through 21.
fn kept_then_rolled_back(frame: StagedFrame, corrected: &[Tick]) -> Rig {
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, corrected);
    rig.push(cart_off(19));
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks, vec![Tick(19), Tick(21)]);
    assert_eq!(idle(&mut rig, 10), 0);
    rig
}

#[test]
fn a_move_into_a_region_neither_slot_holds_is_not_folded_into_the_kept_spawn() {
    let mut frame = StagedFrame::new(Tick(17));
    frame.moves.insert(key(4), UNLOADED);
    let rig = kept_then_rolled_back(frame, &[]);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(5, 5))));
}

#[test]
fn a_claimed_spawn_naming_a_region_neither_slot_holds_folds_its_components_alone() {
    let mut frame = StagedFrame::new(Tick(18));
    frame.spawns.insert(net(4), claimed(UNLOADED, vec![data(at(6, 5)), data(Health(3))]));
    // Its components correct the kept spawn at 21; its region is skipped.
    let rig = kept_then_rolled_back(frame, &[Tick(21)]);
    assert_eq!(live_at(&rig, key(4)), Some((NEW, at(6, 5))));
    assert_eq!(rig.world().get::<Health>(key(4)), Some(&Health(3)));
}

#[test]
fn a_claimed_spawn_naming_a_region_neither_slot_holds_after_a_kept_tombstone_is_skipped() {
    // The cart moved into `NEW` at 15 (kept at 21), destroyed at 17 (folded there), then
    // claimed again at 18 in a region no step loads: the claim does not replace the
    // tombstone, so a rollback from before 21 still destroys the cart there.
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    frame.moves.insert(cart(), NEW);
    rig.push(frame);
    step(&mut rig, true);
    let _ = rig.reconcile();
    let mut frame = StagedFrame::new(Tick(17));
    frame.despawns.insert(
        cart(),
        Tombstone { region: NEW, tick: Tick(17), cause: TombstoneCause::Destroyed },
    );
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert!(!rig.world().contains(cart()));
    let mut frame = StagedFrame::new(Tick(18));
    frame.spawns.insert(net(2), claimed(UNLOADED, vec![data(at(50, 0)), data(Health(10))]));
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    // A rollback from 19, where the cart is live.
    let mut frame = StagedFrame::new(Tick(19));
    exact(&mut frame, me(), &at(1000, 0));
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).first().map(|s| s.0), Some(Tick(19)));
    assert_eq!(idle(&mut rig, 10), 0);
    assert!(!rig.world().contains(cart()));
}

#[test]
fn a_live_entitys_move_into_a_region_neither_slot_holds_survives_a_rollback() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(17));
    frame.moves.insert(cart(), UNLOADED);
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    rig.push(cart_off(19));
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 10), 0);
    assert_eq!(rig.world().region_of(cart()), Some(REGION));
}

/// The spawn kept at 21 in `NEW`, its move kept at 22 in `NEW2`, then `last` for 18, which
/// follows the newer, and a rollback from 19 through both.
fn kept_at_two_ticks_then(last: StagedFrame) -> Rig {
    let mut rig = moved_into_new2(claim_into_new(new_cart()), key(4), 17);
    rig.push(last);
    step(&mut rig, false);
    let _ = rig.reconcile();
    rig.push(cart_off(19));
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    let ticks: Vec<Tick> = summaries(&reconciled).iter().map(|s| s.0).collect();
    assert_eq!(ticks.first(), Some(&Tick(19)));
    assert_eq!(idle(&mut rig, 10), 0);
    rig
}

#[test]
fn a_tombstone_that_follows_facts_kept_at_two_ticks_ends_absent() {
    let mut frame = StagedFrame::new(Tick(18));
    frame.despawns.insert(
        key(4),
        Tombstone { region: NEW2, tick: Tick(18), cause: TombstoneCause::Destroyed },
    );
    let rig = kept_at_two_ticks_then(frame);
    assert!(!rig.world().contains(key(4)));
}

#[test]
fn rows_that_follow_facts_kept_at_two_ticks_end_in_the_newer_region() {
    let mut frame = StagedFrame::new(Tick(18));
    exact(&mut frame, key(4), &at(6, 5));
    exact(&mut frame, key(4), &Health(3));
    let rig = kept_at_two_ticks_then(frame);
    assert_eq!(live_at(&rig, key(4)), Some((NEW2, at(6, 5))));
    assert_eq!(rig.world().get::<Health>(key(4)), Some(&Health(3)));
}

#[test]
fn a_release_that_follows_facts_kept_at_two_ticks_ends_a_proxy_in_the_newer_region() {
    let mut frame = StagedFrame::new(Tick(18));
    frame.spawns.insert(net(4), remote(NEW2, 2, vec![data(at(7, 5)), data(Health(4))]));
    let rig = kept_at_two_ticks_then(frame);
    let world = rig.world();
    assert_eq!(claim(world, key(4)), None);
    assert_eq!(world.region_of(key(4)), Some(NEW2));
}

/// Whether a key is live, who claims it, its region and its `Health`.
type State = (bool, Option<PlayerId>, Option<RegionId>, Option<Health>);

/// The `State` of `key`.
fn state(rig: &Rig, key: StableKey) -> State {
    let world = rig.world();
    (
        world.contains(key),
        claim(world, key),
        world.region_of(key),
        world.get::<Health>(key).copied(),
    )
}

#[test]
fn a_reclaim_of_a_kept_release_into_a_region_neither_slot_holds_leaves_a_proxy() {
    // Key 4 spawned and kept at 21 in `NEW`, released for 17, claimed again for 18 in a
    // region no step loads: a proxy, before and after a rollback through 21.
    let (mut rig, _) = loaded_at_21(claim_into_new(new_cart()));
    let mut frame = StagedFrame::new(Tick(17));
    frame.spawns.insert(net(4), remote(NEW, 2, vec![data(at(7, 5)), data(Health(4))]));
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    let mut frame = StagedFrame::new(Tick(18));
    frame.spawns.insert(net(4), claimed(UNLOADED, vec![data(at(6, 5)), data(Health(3))]));
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    let proxy = (true, None, Some(NEW), Some(Health(4)));
    assert_eq!(state(&rig, key(4)), proxy);
    rig.push(cart_off(19));
    step(&mut rig, false);
    let _ = rig.reconcile();
    assert_eq!(idle(&mut rig, 10), 0);
    assert_eq!(state(&rig, key(4)), proxy);
}

/// The cart released for 17, then claimed again for 18 in `region` with `Health(7)`, then
/// a rollback from 19; its state after the claim and after the rollback.
fn reclaimed_then_rolled_back(region: RegionId) -> [State; 2] {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(17));
    frame.spawns.insert(net(2), remote(REGION, 2, vec![data(at(50, 0)), data(Health(10))]));
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    let mut frame = StagedFrame::new(Tick(18));
    frame.spawns.insert(net(2), claimed(region, vec![data(at(50, 0)), data(Health(7))]));
    rig.push(frame);
    step(&mut rig, false);
    let _ = rig.reconcile();
    let claimed = state(&rig, cart());
    let mut frame = StagedFrame::new(Tick(19));
    exact(&mut frame, me(), &at(1000, 0));
    rig.push(frame);
    step(&mut rig, false);
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).first().map(|s| s.0), Some(Tick(19)));
    assert_eq!(idle(&mut rig, 10), 0);
    [claimed, state(&rig, cart())]
}

#[test]
fn a_reclaim_of_a_proxy_into_a_region_neither_slot_holds_is_not_applied() {
    let proxy = (true, None, Some(REGION), Some(Health(10)));
    assert_eq!(reclaimed_then_rolled_back(UNLOADED), [proxy, proxy]);
}

#[test]
fn a_reclaim_of_a_proxy_into_a_loaded_region_survives_a_rollback() {
    let claimed = (true, Some(PLAYER), Some(REGION), Some(Health(7)));
    assert_eq!(reclaimed_then_rolled_back(REGION), [claimed, claimed]);
}
