//! Claims and whole-entity statements: a claim that starts or ends inside one
//! reconciliation window, with a later frame of the same window about the same entity,
//! ends as it does across two windows (the hook applies the claim change at once); a
//! claimed spawn and a frame's exact rows of a claimed entity state its whole recorded
//! component set, so a component the server's entity lacks is removed; and a transient
//! replicated component is never reconciled, nor written to a claimed entity, nor is
//! `Teleported`.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::Tick;
use lactose_net_api::{StagedFrame, Tombstone, TombstoneCause};
use lactose_physics_api::Teleported;
use lactose_predict::Reconciled;
use lactose_reflect::Reflect;

/// The server claims the proxy for the local player at 15.
fn claim_frame() -> StagedFrame {
    let mut frame = StagedFrame::new(Tick(15));
    frame.spawns.insert(net(3), claimed(REGION, vec![data(at(-50, 0))]));
    frame
}

/// The server releases the cart at 15.
fn release_frame() -> StagedFrame {
    let mut frame = StagedFrame::new(Tick(15));
    frame.spawns.insert(net(2), remote(REGION, 1, vec![data(at(50, 0)), data(Health(10))]));
    frame
}

/// The claim or release frame then a frame for 17 that `then` fills, reconciled in one
/// call (`separate` false) or one call each, and both runs stepped to 22. Returns the rig
/// and every correction made.
fn run(first: StagedFrame, then: &dyn Fn(&mut StagedFrame), separate: bool) -> (Rig, usize) {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut second = StagedFrame::new(Tick(17));
    then(&mut second);
    let mut corrections = 0;
    let mut count = |reconciled: Reconciled| corrections += reconciled.corrections.len();
    rig.push(first);
    if separate {
        rig.step(walking(21));
        count(rig.reconcile());
        rig.push(second);
    } else {
        rig.push(second);
        rig.step(walking(21));
        count(rig.reconcile());
    }
    rig.step(walking(22));
    count(rig.reconcile());
    (rig, corrections)
}

/// Runs the frames in one window and in two: both make `corrections` corrections (a
/// release's later entries are a proxy's, written by the hook) and end with equal worlds.
/// Returns the one-window rig.
fn in_one_window_as_in_two(
    first: fn() -> StagedFrame,
    then: &dyn Fn(&mut StagedFrame),
    corrections: usize,
) -> Rig {
    let (two, separately) = run(first(), then, true);
    let (one, together) = run(first(), then, false);
    assert_eq!((together, separately), (corrections, corrections));
    assert_eq!(one.hash(), two.hash());
    one
}

#[test]
fn a_claim_then_a_tombstone_in_one_window_leaves_the_entity_destroyed() {
    let tombstone = |frame: &mut StagedFrame| {
        frame.despawns.insert(
            proxy(),
            Tombstone { region: REGION, tick: Tick(17), cause: TombstoneCause::Destroyed },
        );
    };
    let rig = in_one_window_as_in_two(claim_frame, &tombstone, 2);
    assert!(!rig.world().contains(proxy()), "the server destroyed it at 17");
}

#[test]
fn a_claim_then_a_move_in_one_window_moves_the_entity() {
    let rig = in_one_window_as_in_two(
        claim_frame,
        &|frame| {
            frame.moves.insert(proxy(), OTHER_REGION);
        },
        2,
    );
    assert_eq!(rig.world().region_of(proxy()), Some(OTHER_REGION));
    assert_eq!(claim(rig.world(), proxy()), Some(PLAYER));
}

#[test]
fn a_claim_then_a_row_in_one_window_writes_the_row() {
    let row = |frame: &mut StagedFrame| exact(frame, proxy(), &at(7, 7));
    let rig = in_one_window_as_in_two(claim_frame, &row, 2);
    assert_eq!(rig.position(proxy()), Some(at(7, 7)));
    assert_eq!(claim(rig.world(), proxy()), Some(PLAYER));
}

#[test]
fn a_release_then_a_move_in_one_window_leaves_the_entity_released_and_moved() {
    let rig = in_one_window_as_in_two(
        release_frame,
        &|frame| {
            frame.moves.insert(cart(), OTHER_REGION);
        },
        1,
    );
    assert_eq!(claim(rig.world(), cart()), None, "released at 15");
    assert_eq!(rig.world().region_of(cart()), Some(OTHER_REGION));
}

#[test]
fn a_release_then_a_removal_in_one_window_leaves_the_entity_released_without_it() {
    let rig = in_one_window_as_in_two(
        release_frame,
        &|frame| {
            frame.removed.insert((cart(), Health::KEY));
        },
        1,
    );
    assert_eq!(claim(rig.world(), cart()), None, "released at 15");
    assert_eq!(rig.world().get::<Health>(cart()), None);
}

#[test]
fn an_entity_released_in_one_window_follows_the_servers_proxy_rows() {
    let (mut rig, _) = run(
        release_frame(),
        &|frame| {
            frame.moves.insert(cart(), OTHER_REGION);
        },
        false,
    );
    for present in 23..=30 {
        let mut frame = StagedFrame::new(Tick(present - 3));
        dequantised(&mut frame, cart(), &at(1000 + i32::try_from(present).unwrap(), 0));
        rig.push(frame);
        rig.step(walking(present));
        assert!(rig.reconcile().corrections.is_empty());
    }
    assert_eq!(claim(rig.world(), cart()), None);
    assert_eq!(rig.position(cart()), Some(at(1030, 0)));
}

/// Joined, the player's entity given `Health` at 13 (a prediction system's mistake, written
/// directly), then a frame per tick from 17 to 26 four ticks late, each stating the player
/// as the server has it through `state`. Returns the rig and every correction's tick.
fn extra_health(state: impl Fn(&mut StagedFrame, u32)) -> (Rig, Vec<Tick>) {
    let mut rig = Rig::joined();
    rig.step_to(13, walking);
    rig.world_mut().insert(me(), Health(99)).unwrap();
    rig.step_to(20, walking);
    let mut ticks = Vec::new();
    for present in 21..=30 {
        let mut frame = StagedFrame::new(Tick(present - 4));
        state(&mut frame, present - 4);
        rig.push(frame);
        rig.step(walking(present));
        ticks.extend(rig.reconcile().corrections.iter().map(|correction| correction.tick));
    }
    (rig, ticks)
}

/// The player's position at `tick` as predicted: one a tick from 12.
fn walked(tick: u32) -> Position {
    at(i32::try_from(tick).unwrap() - 11, 0)
}

#[test]
fn a_recorded_component_a_frames_rows_do_not_name_is_removed_once() {
    let (rig, ticks) = extra_health(|frame, tick| exact(frame, me(), &walked(tick)));
    assert_eq!(ticks, vec![Tick(17)]);
    assert_eq!(rig.world().get::<Health>(me()), None, "the server's entity has no Health");
}

#[test]
fn a_recorded_component_a_claimed_spawn_does_not_name_is_removed_once() {
    let (rig, ticks) = extra_health(|frame, tick| {
        if tick == 17 {
            frame.spawns.insert(net(1), claimed(REGION, vec![data(walked(tick))]));
        }
    });
    assert_eq!(ticks, vec![Tick(17)]);
    assert_eq!(rig.world().get::<Health>(me()), None);
    assert_eq!(claim(rig.world(), me()), Some(PLAYER));
}

#[test]
fn a_frame_with_no_row_of_the_entity_removes_nothing() {
    let (rig, ticks) = extra_health(|frame, _| {
        exact(frame, cart(), &at(50, 0));
        exact(frame, cart(), &Health(10));
    });
    assert_eq!(ticks, vec![]);
    assert_eq!(rig.world().get::<Health>(me()), Some(&Health(99)));
}

#[test]
fn a_transient_replicated_component_is_never_reconciled() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(4, 0));
    exact(&mut frame, me(), &Aim(3));
    // A proxy's is written by the hook as any row.
    dequantised(&mut frame, proxy(), &Aim(5));
    rig.push(frame);
    rig.step(walking(21));
    assert_eq!(rig.world().get::<Aim>(proxy()), Some(&Aim(5)));
    assert_eq!(rig.world().get::<Aim>(me()), None);
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty(), "{:?}", summaries(&reconciled));
    assert_eq!(rig.world().get::<Aim>(me()), None);
}

#[test]
fn a_claimed_spawn_the_hook_applies_carries_neither_teleported_nor_a_transient_component() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    let components = vec![data(at(5, 5)), data(Teleported(Tick(3))), data(Aim(7))];
    frame.spawns.insert(net(4), claimed(REGION, components));
    rig.push(frame);
    rig.step(walking(21));
    let world = rig.world();
    assert_eq!(world.get::<Position>(key(4)), Some(&at(5, 5)));
    assert_eq!(claim(world, key(4)), Some(PLAYER));
    assert_eq!(world.get::<Teleported>(key(4)), None);
    assert_eq!(world.get::<Aim>(key(4)), None);
    // The correction at 15, whose slot lacks the entity, spawns it the same way.
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1, "{:?}", summaries(&reconciled));
    let world = rig.world();
    assert_eq!(world.get::<Position>(key(4)), Some(&at(5, 5)));
    assert_eq!(world.get::<Teleported>(key(4)), None);
    assert_eq!(world.get::<Aim>(key(4)), None);
}
