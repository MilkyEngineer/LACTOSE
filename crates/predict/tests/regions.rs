//! Facts that place a claimed entity in a region its frame loads: slot *T* lacks the
//! region and the tick the frame was popped at holds it, so the fact is compared there,
//! and kept there for later calls, since a re-simulation of that tick never re-applies a
//! claim change; a later fact about the entity for a tick before it follows it there.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::Baseline;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{RegionId, RegionMode};
use lactose_math::{Scalar, Vec3};
use lactose_net_api::{StagedFrame, Tombstone};
use lactose_physics_api::{Transform, Velocity};
use lactose_predict::{Absorbed, CorrectionCause, Divergence};
use lactose_reflect::Reflect;
use lactose_sim::{RegionBundle, TickInput};

/// The region the step that pops the frame for 15 loads, at 21.
const NEW: RegionId = RegionId(3);

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
    let next = Tick(rig.tick().0 + 1);
    let mut input = TickInput::default();
    input.inputs.insert(PLAYER, walking(next.0).to_player_sample(next));
    if load {
        input.loaded.insert(NEW, bundle());
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
    frame.despawns.insert(cart(), Tombstone { region: REGION, tick: Tick(15) });
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
    frame.despawns.insert(key(4), Tombstone { region: NEW, tick: Tick(17) });
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
