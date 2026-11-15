//! The hook: what a staged frame does to the prediction world inside the tick, in the
//! README's order, and the local player's claim and anchor on every path.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeSet;

use common::*;
use lactose_bundle::{BaselineRow, ComponentData};
use lactose_core::{ComponentKey, EntityRef, NetId, PlayerId, StableKey, Tick};
use lactose_ecs::{LayoutConfig, OwnedBy, RegionId, World};
use lactose_input::{PlayerInput, player_entity};
use lactose_net_api::{RemoteSlot, StagedFrame, Tombstone, TombstoneCause};
use lactose_predict::{CorrectionCause, LocalPlayer, StagedRegion};
use lactose_reflect::Reflect;
use lactose_sim::{StagedState, StreamingAnchor};

fn anchored(world: &World) -> Vec<StableKey> {
    let mut keys: Vec<StableKey> =
        world.columns::<StreamingAnchor>().flat_map(|column| column.keys().to_vec()).collect();
    keys.sort_unstable();
    keys
}

#[test]
fn the_join_frame_rebuilds_and_the_local_entity_carries_its_claim_and_anchor() {
    let mut rig = Rig::new(simulation(Default::default(), None), 1);
    assert_eq!(rig.p().ring().len(), 1);
    assert_eq!(rig.p().ring().newest(), Some(START));
    rig.push(join_frame(JOIN));
    rig.step(walking(11));
    let world = rig.world();
    assert_eq!(claim(world, me()), Some(PLAYER));
    assert_eq!(claim(world, cart()), Some(PLAYER));
    assert_eq!(claim(world, proxy()), None);
    assert_eq!(anchored(world), vec![me()]);
    assert_eq!(player_entity(world, PLAYER), Some(me()));
    // Never `RemoteSlot`: the prediction world presents its proxies through the replicated
    // world's slots.
    assert!(world.get::<RemoteSlot>(proxy()).is_none());
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.confirmed, Some(JOIN));
    assert_eq!(rig.p().ring().rebuilt_at(), Some(Tick(11)));
    assert_eq!(rig.p().ring().oldest(), Some(Tick(11)));
    // The applier ran before the hook spawned the row: the rebuild tick's own sample was
    // not applied, the next is.
    assert_eq!(rig.position(me()), Some(at(0, 0)));
    rig.step(walking(12));
    assert_eq!(rig.position(me()), Some(at(1, 0)));
}

#[test]
fn the_first_frame_at_or_after_the_rebuild_corrects_once() {
    let mut rig = Rig::joined();
    rig.step_to(14, walking);
    // The server's player had walked further than the rebuilt state knows.
    let mut frame = StagedFrame::new(Tick(12));
    exact(&mut frame, me(), &at(8, 0));
    rig.push(frame);
    rig.step(walking(15));
    let first = rig.reconcile();
    assert_eq!(first.corrections.len(), 1);
    assert_eq!(first.corrections[0].tick, Tick(12));
    let mut frame = StagedFrame::new(Tick(13));
    exact(&mut frame, me(), &at(9, 0));
    rig.push(frame);
    rig.step(walking(16));
    let second = rig.reconcile();
    assert!(second.corrections.is_empty());
    assert_eq!(rig.position(me()), Some(at(12, 0)));
}

#[test]
fn a_claimed_entitys_spawn_rows_and_tombstone_are_not_written_by_the_hook() {
    let mut rig = Rig::joined();
    rig.step_to(14, walking);
    let mut frame = StagedFrame::new(Tick(14));
    exact(&mut frame, me(), &at(40, 0));
    frame.spawns.insert(net(2), claimed(REGION, vec![data(at(60, 0)), data(Health(1))]));
    frame.despawns.insert(
        cart(),
        Tombstone { region: REGION, tick: Tick(14), cause: TombstoneCause::Destroyed },
    );
    frame.moves.insert(me(), OTHER_REGION);
    rig.push(frame);
    rig.step(walking(15));
    // After the step and before the reconciliation the world is the prediction's.
    let world = rig.world();
    assert_eq!(rig.position(me()), Some(at(4, 0)));
    assert_eq!(world.region_of(me()), Some(REGION));
    assert_eq!(world.get::<Health>(cart()), Some(&Health(10)));
    assert!(world.contains(cart()));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(reconciled.corrections[0].entities, vec![me(), cart()]);
    assert!(!rig.world().contains(cart()));
    assert_eq!(rig.world().region_of(me()), Some(OTHER_REGION));
}

/// A region-leave tombstone of a proxy residing in the region it names, while the world
/// has that region loaded, is the region's to settle and is skipped (review r1 of
/// `lactose-net-host`, B2: the region was reloaded after the tombstone's tick, and the
/// load holds the proxy's unreplicated rows); one naming another region despawns it.
#[test]
fn a_left_tombstone_of_a_proxy_its_loaded_region_holds_is_skipped() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let mut frame = StagedFrame::new(Tick(12));
    frame.spawns.insert(net(5), remote(REGION, 3, vec![data(at(1, 1)), data(Health(2))]));
    frame.spawns.insert(net(6), remote(REGION, 4, vec![data(at(2, 2)), data(Health(3))]));
    rig.push(frame);
    rig.step(walking(13));
    assert!(rig.world().contains(key(5)) && rig.world().contains(key(6)));
    let mut frame = StagedFrame::new(Tick(13));
    frame.despawns.insert(key(5), left(REGION, 13));
    frame.despawns.insert(key(6), left(OTHER_REGION, 13));
    rig.push(frame);
    rig.step(walking(14));
    assert!(rig.world().contains(key(5)), "settled by its region");
    assert!(!rig.world().contains(key(6)), "it resides elsewhere");
    assert!(rig.reconcile().corrections.is_empty());
}

/// Review r2 of `lactose-net-host`, N1: a proxy kept for a `Left` tombstone naming the
/// region the frames placed it in is despawned by the server's next departure of the
/// entity, which the receiver stages under the key the `Left` tombstone released: a `Left`
/// naming another region, or a `Destroyed`.
#[test]
fn a_proxy_kept_for_a_left_tombstone_is_despawned_by_a_later_tombstone() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let mut frame = StagedFrame::new(Tick(12));
    frame.spawns.insert(net(5), remote(REGION, 3, vec![data(at(1, 1)), data(Health(2))]));
    frame.spawns.insert(net(6), remote(REGION, 4, vec![data(at(2, 2)), data(Health(3))]));
    rig.push(frame);
    rig.step(walking(13));
    let tombstone = |region, tick, cause| Tombstone { region, tick: Tick(tick), cause };
    let mut frame = StagedFrame::new(Tick(13));
    for n in [5, 6] {
        frame.despawns.insert(key(n), tombstone(REGION, 13, TombstoneCause::Left));
    }
    rig.push(frame);
    rig.step(walking(14));
    assert!(rig.world().contains(key(5)) && rig.world().contains(key(6)), "kept");
    let mut frame = StagedFrame::new(Tick(14));
    frame.despawns.insert(key(5), tombstone(RegionId(3), 14, TombstoneCause::Left));
    frame.despawns.insert(key(6), tombstone(REGION, 14, TombstoneCause::Destroyed));
    rig.push(frame);
    rig.step(walking(15));
    assert!(!rig.world().contains(key(5)), "the server has it in a region the client lacks");
    assert!(!rig.world().contains(key(6)), "destroyed");
    assert!(rig.reconcile().corrections.is_empty());
}

/// Where a proxy resided after the frames last placed it, as its `StagedRegion` records it.
fn placed(world: &World, key: StableKey) -> Option<RegionId> {
    world.get::<StagedRegion>(key).map(StagedRegion::region)
}

fn left(region: RegionId, tick: u32) -> Tombstone {
    Tombstone { region, tick: Tick(tick), cause: TombstoneCause::Left }
}

/// The gap of D121's addendum (review r2 of `lactose-net-host`, N1, prediction world): a
/// prediction system kicks a proxy into a region the server has not named; the server
/// then moves the entity into that region and, unable to name it, sends a `Left`
/// tombstone naming it, the last it sends of the entity once acknowledged. The hook finds
/// the proxy residing in the region the tombstone names, but the frames last placed it
/// elsewhere, so the tombstone is not the region's to settle and the proxy is despawned,
/// as the replicated world despawns its copy; one the frames placed there is kept. A
/// rollback through the kick and the tombstone ends the same, since the placement is a
/// column the ring restores.
#[test]
fn a_proxy_a_prediction_system_moved_is_despawned_by_a_left_tombstone_naming_its_region() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let mut frame = StagedFrame::new(Tick(12));
    frame.spawns.insert(net(5), remote(REGION, 3, vec![data(at(1, 1)), data(Health(2))]));
    frame.spawns.insert(net(6), remote(OTHER_REGION, 4, vec![data(at(2, 2)), data(Health(3))]));
    rig.push(frame);
    rig.step(looking(13));
    assert_eq!(placed(rig.world(), key(5)), Some(REGION));
    assert_eq!(placed(rig.world(), key(6)), Some(OTHER_REGION));
    // The kick's command applies in tick 14; the frames keep the placement.
    rig.step(kicking());
    let world = rig.world();
    assert_eq!(world.region_of(key(5)), Some(OTHER_REGION));
    assert_eq!(placed(world, key(5)), Some(REGION));
    let mut frame = StagedFrame::new(Tick(14));
    frame.despawns.insert(key(5), left(OTHER_REGION, 14));
    frame.despawns.insert(key(6), left(OTHER_REGION, 14));
    rig.push(frame);
    rig.step(walking(15));
    assert!(!rig.world().contains(key(5)), "the frames placed it elsewhere");
    assert!(rig.world().contains(key(6)), "settled by its region");
    assert!(rig.reconcile().corrections.is_empty());

    // A substitution of 13 by a sample that differs from the player's by its yaw alone
    // rolls back to 12 and re-simulates the kick and the tombstone's frame, to the same
    // world.
    let mut frame = StagedFrame::new(Tick(15));
    frame.substituted.insert(Tick(13));
    rig.push(frame);
    rig.step(walking(16));
    let hash = rig.hash();
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled)[0].0, Tick(12));
    assert_eq!(reconciled.resimulated, 4);
    assert_eq!(rig.hash(), hash, "the re-simulated world equals the straight run");
    assert!(!rig.world().contains(key(5)));
    assert_eq!(placed(rig.world(), key(6)), Some(OTHER_REGION));
}

/// A proxy no frame has placed (here spawned directly, as a region's load places its
/// cooked rows before the server names the region) is settled by the region it resides in:
/// a `Left` tombstone naming it is skipped, one naming another region despawns it.
#[test]
fn a_proxy_no_frame_placed_is_settled_by_the_region_it_resides_in() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let world = rig.world_mut();
    world.spawn(key(5), REGION, (at(1, 1), Health(2))).unwrap();
    world.spawn(key(6), REGION, (at(2, 2), Health(3))).unwrap();
    world.move_to_region(key(5), OTHER_REGION).unwrap();
    assert_eq!(placed(world, key(5)), None);
    let mut frame = StagedFrame::new(Tick(12));
    frame.despawns.insert(key(5), left(OTHER_REGION, 12));
    frame.despawns.insert(key(6), left(OTHER_REGION, 12));
    rig.push(frame);
    rig.step(walking(13));
    assert!(rig.world().contains(key(5)), "it resides where the tombstone names");
    assert!(!rig.world().contains(key(6)), "it resides elsewhere");
    assert!(rig.reconcile().corrections.is_empty());
}

/// Review r1 of this change, S1: a frame's move into a region the world lacks (the step's
/// fold took the region's `Relevant` and `Irrelevant` together) leaves the proxy where it
/// was, and the placement records that region, not the move's. The server's owed `Left`
/// tombstone names the region the server has the entity in, so a later `Left` naming where
/// the proxy stayed means the server moved it back there while owing one, and the proxy is
/// that region's to settle (B2 of review r1 of `lactose-net-host`): a cooked row a load
/// placed keeps the unreplicated `Diary` a despawn followed by the re-entry spawn would
/// lose. A `Left` naming the region the move was refused into despawns it, since it resides
/// elsewhere.
#[test]
fn a_frame_move_the_world_refused_leaves_the_proxy_settled_by_the_region_it_stayed_in() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    step_regions(&mut rig, walking(13), &[], &[COOKED]);
    let world = rig.world();
    assert_eq!(world.region_of(cooked_key()), Some(COOKED));
    assert_eq!(placed(world, cooked_key()), None, "a load's row: no frame has placed it");
    // The server names the region: the row is upserted and placed.
    let named = || remote(COOKED, 9, vec![data(at(7, 7)), data(Health(7))]);
    let mut frame = StagedFrame::new(Tick(13));
    frame.spawns.insert(NetId::cooked(77), named());
    rig.push(frame);
    rig.step(walking(14));
    assert_eq!(placed(rig.world(), cooked_key()), Some(COOKED));
    // The move into a region the world lacks is refused: the placement is where it stayed.
    let mut frame = StagedFrame::new(Tick(14));
    frame.moves.insert(cooked_key(), UNLOADED);
    rig.push(frame);
    rig.step(walking(15));
    let world = rig.world();
    assert_eq!(world.region_of(cooked_key()), Some(COOKED));
    assert_eq!(placed(world, cooked_key()), Some(COOKED), "where the move left it");
    // The server moved the entity back while it owed the `Left` of the leave, which was
    // replaced before it was sent: the `Left` the client stages names where the row is.
    let mut frame = StagedFrame::new(Tick(15));
    frame.despawns.insert(cooked_key(), left(COOKED, 15));
    rig.push(frame);
    rig.step(walking(16));
    let world = rig.world();
    assert!(world.contains(cooked_key()), "settled by the region it stayed in");
    assert_eq!(world.get::<Diary>(cooked_key()), Some(&Diary(42)), "the cooked row kept");
    // The region named again: the re-entry is an upsert, and the `Diary` stays.
    let mut frame = StagedFrame::new(Tick(16));
    frame
        .spawns
        .insert(NetId::cooked(77), remote(COOKED, 9, vec![data(at(8, 7)), data(Health(6))]));
    rig.push(frame);
    rig.step(walking(17));
    let world = rig.world();
    assert_eq!(world.get::<Diary>(cooked_key()), Some(&Diary(42)));
    assert_eq!(world.get::<Health>(cooked_key()), Some(&Health(6)));
    // A `Left` naming the region the move was refused into: it resides elsewhere.
    let mut frame = StagedFrame::new(Tick(17));
    frame.despawns.insert(cooked_key(), left(UNLOADED, 17));
    rig.push(frame);
    rig.step(walking(18));
    assert!(!rig.world().contains(cooked_key()));
    assert!(rig.reconcile().corrections.is_empty());

    // A prediction system moved the proxy before the refused move (review r2, m1): the
    // placement is where it stayed, the kicked-into region, not the last placement, and a
    // `Left` naming that region keeps it.
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let mut frame = StagedFrame::new(Tick(12));
    frame.spawns.insert(net(5), remote(REGION, 3, vec![data(at(1, 1)), data(Health(2))]));
    rig.push(frame);
    rig.step(walking(13));
    rig.step(kicking());
    let world = rig.world();
    assert_eq!(world.region_of(key(5)), Some(OTHER_REGION));
    assert_eq!(placed(world, key(5)), Some(REGION));
    let mut frame = StagedFrame::new(Tick(14));
    frame.moves.insert(key(5), UNLOADED);
    rig.push(frame);
    rig.step(walking(15));
    let world = rig.world();
    assert_eq!(world.region_of(key(5)), Some(OTHER_REGION));
    assert_eq!(placed(world, key(5)), Some(OTHER_REGION), "where the move left it");
    let mut frame = StagedFrame::new(Tick(15));
    frame.despawns.insert(key(5), left(OTHER_REGION, 15));
    rig.push(frame);
    rig.step(walking(16));
    assert!(rig.world().contains(key(5)), "settled by the region it stayed in");
    assert!(rig.reconcile().corrections.is_empty());
}

/// The correction of a release moves the entity where the releasing spawn has it, as the
/// hook's release did in the predicted tick, which holds it there through the
/// re-simulated ticks before the releasing frame is re-popped (whose upsert restates it
/// there; a replay never re-applies the claim change itself). Observed by an unload of
/// that region between the release's tick and the tick the frame was popped at: the
/// re-simulated unload takes the released entity with it, where the predicted tick, which
/// had the entity claimed in its old region until the frame arrived, could not move it
/// into the region any more; the result equals a run that had the release before the
/// unload.
#[test]
fn the_release_correction_holds_the_entity_where_it_was_released_through_the_replay() {
    let release = || {
        let mut frame = StagedFrame::new(Tick(15));
        let cart = vec![data(at(50, 0)), data(Health(10))];
        frame.spawns.insert(net(2), remote(OTHER_REGION, 1, cart));
        frame
    };
    // The release before the unload: applied at 16, corrected at 15; the unload at 18
    // takes the cart.
    let mut straight = Rig::joined();
    straight.step_to(15, walking);
    straight.push(release());
    straight.step(walking(16));
    assert_eq!(straight.reconcile().corrections.len(), 1);
    assert_eq!(straight.world().region_of(cart()), Some(OTHER_REGION));
    straight.step(walking(17));
    step_regions(&mut straight, walking(18), &[OTHER_REGION], &[]);
    assert!(!straight.world().contains(cart()), "unloaded with its region");
    straight.step_to(21, walking);
    assert!(straight.reconcile().corrections.is_empty());

    // The release after the unload: popped at 21, where the cart cannot be moved into the
    // unloaded region, so the predicted tick leaves it in REGION, released.
    let mut rig = Rig::joined();
    rig.step_to(17, walking);
    step_regions(&mut rig, walking(18), &[OTHER_REGION], &[]);
    rig.step_to(20, walking);
    rig.push(release());
    rig.step(walking(21));
    let world = rig.world();
    assert_eq!(claim(world, cart()), None);
    assert_eq!(world.region_of(cart()), Some(REGION));
    assert_eq!(placed(world, cart()), Some(REGION), "where the release left it");
    // The correction at 15 releases the cart into OTHER_REGION, loaded then; the replay
    // of 18 unloads the region and takes the cart; the replay of 21 finds nothing to
    // upsert and cannot spawn it there.
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled)[0].0, Tick(15));
    assert_eq!(reconciled.corrections[0].entities, vec![cart()]);
    assert_eq!(reconciled.resimulated, 6);
    assert!(!rig.world().contains(cart()), "the replayed unload took it");
    assert_eq!(rig.hash(), straight.hash(), "as the run that had the release in time");
}

/// The placement follows the frames: a proxy's spawn, upsert and move write it, and a
/// release writes where the releasing spawn left the entity, in the predicted tick and
/// through the correction a rollback makes of the release.
#[test]
fn the_placement_follows_the_frames_spawns_moves_and_releases() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    assert_eq!(placed(rig.world(), proxy()), Some(REGION), "the join frame's spawn");
    assert_eq!(placed(rig.world(), cart()), None, "claimed: not the frames' to place");
    let mut frame = StagedFrame::new(Tick(12));
    frame.moves.insert(proxy(), OTHER_REGION);
    // The server releases the cart into the other region.
    frame.spawns.insert(net(2), remote(OTHER_REGION, 7, vec![data(at(50, 0)), data(Health(10))]));
    rig.push(frame);
    rig.step(walking(13));
    let world = rig.world();
    assert_eq!(world.region_of(proxy()), Some(OTHER_REGION));
    assert_eq!(placed(world, proxy()), Some(OTHER_REGION));
    assert_eq!(claim(world, cart()), None);
    assert_eq!(world.region_of(cart()), Some(OTHER_REGION));
    assert_eq!(placed(world, cart()), Some(OTHER_REGION));
    // The release is a fact compared at 12, where the cart was claimed: the correction
    // writes the release at 12, holding the cart where the server released it for the
    // replayed ticks before 13, where the replay's upsert of the releasing frame restates
    // it (a replay never re-applies the claim change itself).
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(reconciled.corrections[0].entities, vec![cart()]);
    let world = rig.world();
    assert_eq!(claim(world, cart()), None);
    assert_eq!(world.region_of(cart()), Some(OTHER_REGION));
    assert_eq!(placed(world, cart()), Some(OTHER_REGION));
    assert_eq!(placed(world, proxy()), Some(OTHER_REGION));
    // An upsert moves the proxy back and places it again.
    let mut frame = StagedFrame::new(Tick(13));
    frame.spawns.insert(net(3), remote(REGION, 0, vec![data(at(-47, 0))]));
    rig.push(frame);
    rig.step(walking(14));
    assert_eq!(rig.world().region_of(proxy()), Some(REGION));
    assert_eq!(placed(rig.world(), proxy()), Some(REGION));
    assert!(rig.reconcile().corrections.is_empty());
}

#[test]
fn proxies_are_spawned_moved_written_stripped_and_tombstoned_in_the_frame_order() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let ball = key(5);
    let mut frame = StagedFrame::new(Tick(12));
    frame.spawns.insert(net(5), remote(REGION, 3, vec![data(at(1, 1)), data(Health(2))]));
    dequantised(&mut frame, proxy(), &at(-49, 0));
    rig.push(frame);
    rig.step(looking(13));
    let world = rig.world();
    assert_eq!(world.get::<Position>(ball), Some(&at(1, 1)));
    assert_eq!(claim(world, ball), None);
    assert_eq!(rig.position(proxy()), Some(at(-49, 0)));

    let mut frame = StagedFrame::new(Tick(13));
    frame.moves.insert(ball, OTHER_REGION);
    frame.removed.insert((ball, Health::KEY));
    // An upsert: a spawn of a live unclaimed entity writes every component.
    frame.spawns.insert(net(3), remote(REGION, 0, vec![data(at(-47, 0)), data(Health(6))]));
    rig.push(frame);
    rig.step(walking(14));
    let world = rig.world();
    assert_eq!(world.region_of(ball), Some(OTHER_REGION));
    assert_eq!(world.get::<Health>(ball), None);
    assert_eq!(world.get::<Health>(proxy()), Some(&Health(6)));
    assert_eq!(rig.position(proxy()), Some(at(-47, 0)));

    let mut frame = StagedFrame::new(Tick(14));
    frame.despawns.insert(
        ball,
        Tombstone { region: OTHER_REGION, tick: Tick(14), cause: TombstoneCause::Destroyed },
    );
    rig.push(frame);
    rig.step(walking(15));
    assert!(!rig.world().contains(ball));
    // None of it is a correction: the proxies are written, not compared.
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert_eq!(reconciled.frames, 3);

    // A rollback replays the proxies' history: a substitution of 13 by a sample that
    // differs from the player's by its yaw alone restores 12 and re-simulates 13 to 16
    // with the frames popped there, to the same world.
    let mut frame = StagedFrame::new(Tick(15));
    frame.substituted.insert(Tick(13));
    rig.push(frame);
    rig.step(walking(16));
    let hash = rig.hash();
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled)[0].0, Tick(12));
    assert_eq!(reconciled.resimulated, 4);
    assert_eq!(rig.hash(), hash);
    assert!(!rig.world().contains(ball));
    assert_eq!(rig.position(proxy()), Some(at(-47, 0)));
    assert_eq!(rig.world().get::<Health>(proxy()), Some(&Health(6)));
}

#[test]
fn an_exact_row_of_an_unclaimed_entity_is_a_defect_and_skipped() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let mut frame = StagedFrame::new(Tick(12));
    exact(&mut frame, proxy(), &at(0, 0));
    dequantised(&mut frame, me(), &at(30, 0));
    rig.push(frame);
    rig.step(walking(13));
    assert_eq!(rig.position(proxy()), Some(at(-50, 0)));
    assert_eq!(rig.position(me()), Some(at(2, 0)));
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
}

fn ball_of(world: &World) -> Option<StableKey> {
    world.claimed(PLAYER).find(|key| matches!(key, StableKey::Predicted(_)))
}

#[test]
fn an_unechoed_predicted_spawn_is_despawned_at_the_frame_after_its_input_tick() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    rig.step(throwing());
    let ball = ball_of(rig.world()).expect("the throw spawned a predicted ball");
    let StableKey::Predicted(predicted) = ball else { unreachable!() };
    assert_eq!((predicted.player, predicted.input_tick), (PLAYER, Tick(13)));
    // The frame of the spawn's own tick acknowledges it: not too late.
    let mut frame = StagedFrame::new(Tick(13));
    frame.acked_input = Some(Tick(13));
    rig.push(frame);
    rig.step(walking(14));
    assert!(rig.world().contains(ball));
    rig.reconcile();
    assert_eq!(rig.p().stats().unechoed_spawns, 0);
    // The next one passes it without an echo.
    let mut frame = StagedFrame::new(Tick(14));
    frame.acked_input = Some(Tick(14));
    rig.push(frame);
    rig.step(walking(15));
    assert!(!rig.world().contains(ball));
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert_eq!(rig.p().stats().unechoed_spawns, 1);

    // An echo that arrives anyway is a fact: the step that applies it spawns and claims
    // the ball as the server has it (a claim is applied at once), and the slot of its tick,
    // which lacks the ball, is corrected.
    let mut frame = StagedFrame::new(Tick(15));
    let echo = lactose_net_api::StagedSpawn {
        origin: Some(predicted),
        ..claimed(REGION, vec![data(Health(1))])
    };
    frame.spawns.insert(net(40), echo);
    rig.push(frame);
    rig.step(walking(16));
    assert!(rig.world().contains(ball));
    assert_eq!(claim(rig.world(), ball), Some(PLAYER));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(reconciled.corrections[0].cause, CorrectionCause::Mismatch);
    assert_eq!(reconciled.corrections[0].entities, vec![ball]);
    assert!(rig.world().contains(ball));
    assert_eq!(rig.world().alias(predicted), Some(net(40)));
}

#[test]
fn an_echoed_predicted_spawn_is_bound_and_kept() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    rig.step(throwing());
    let ball = ball_of(rig.world()).expect("a ball");
    let StableKey::Predicted(predicted) = ball else { unreachable!() };
    let mut frame = StagedFrame::new(Tick(13));
    frame.acked_input = Some(Tick(14));
    let echo = lactose_net_api::StagedSpawn {
        origin: Some(predicted),
        ..claimed(REGION, vec![data(Health(1))])
    };
    frame.spawns.insert(net(40), echo);
    rig.push(frame);
    rig.step(walking(14));
    assert!(rig.world().contains(ball));
    assert_eq!(rig.world().alias(predicted), Some(net(40)));
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert_eq!(rig.p().stats().unechoed_spawns, 0);
}

#[test]
fn set_local_moves_the_claim_and_the_anchor() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    rig.p_mut().set_local(LocalPlayer { player: PLAYER, entity: cart() }).unwrap();
    assert_eq!(rig.p().local().entity, cart());
    rig.step(walking(13));
    let world = rig.world();
    assert_eq!(anchored(world), vec![cart()]);
    assert_eq!(player_entity(world, PLAYER), Some(cart()));
    // The player's input now drives the cart's row.
    rig.step(walking(14));
    assert_eq!(rig.position(cart()), Some(at(51, 0)));

    // A proxy named as the player's entity is claimed and anchored too.
    rig.p_mut().set_local(LocalPlayer { player: PLAYER, entity: proxy() }).unwrap();
    rig.step(walking(15));
    let world = rig.world();
    assert_eq!(claim(world, proxy()), Some(PLAYER));
    assert_eq!(anchored(world), vec![proxy()]);
    // Another player's anchor is not this world's to keep.
    assert_eq!(player_entity(world, PlayerId(2)), None);
}

#[test]
fn the_anchor_is_restored_on_the_local_entity_whatever_removed_it() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let world = rig.world_mut();
    world.remove::<StreamingAnchor>(me()).unwrap();
    world.insert(cart(), StreamingAnchor { priority: 0 }).unwrap();
    rig.step(walking(13));
    assert_eq!(anchored(rig.world()), vec![me()]);
    assert_eq!(player_entity(rig.world(), PLAYER), Some(me()));
    // The anchor holds through a rollback.
    let mut frame = StagedFrame::new(Tick(12));
    exact(&mut frame, me(), &at(9, 0));
    rig.push(frame);
    rig.step(walking(14));
    assert_eq!(rig.reconcile().corrections.len(), 1);
    assert_eq!(anchored(rig.world()), vec![me()]);
    assert_eq!(rig.position(me()), Some(at(11, 0)));
}

/// A hook that does nothing, inserted over the predictor's.
struct Inert;

impl StagedState for Inert {
    fn apply(&mut self, _world: &mut World, _tick: Tick) -> lactose_core::Result<()> {
        Ok(())
    }
}

#[test]
fn a_hook_replaced_through_simulation_mut_fails_the_next_step_with_the_predictor_kept() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    rig.world_mut().insert_service::<dyn StagedState>(Box::new(Inert));
    let error = rig.try_step(walking(13)).unwrap_err();
    assert_eq!(error.kind(), lactose_core::ErrorKind::InvalidData);
    // Kept, with the slot taken.
    assert_eq!(rig.tick(), Tick(13));
    assert_eq!(rig.p().ring().newest(), Some(Tick(13)));
    // `resync` reinstalls the hook.
    rig.p_mut().resync(Tick(13)).unwrap();
    rig.step(walking(14));
}

// ---- One entity per entry, and what a row built from the stream has (D141) ----

/// A region no other test here uses: a document's.
const DOCUMENT: RegionId = RegionId(6);

fn owner() -> StableKey {
    StableKey::Net(NetId::cooked(80))
}

fn item() -> StableKey {
    StableKey::Net(NetId::cooked(81))
}

fn owner_of(world: &World, key: StableKey) -> Option<StableKey> {
    world.get::<OwnedBy>(key).and_then(|owned| owned.owner.get())
}

/// A region document placing, in [`DOCUMENT`], an owner and an item it owns, both proxies:
/// the prediction world's edge is the document's, older than anything the server did since.
fn owned_document(rig: &mut Rig) {
    let owner_row =
        BaselineRow { id: NetId::cooked(80), components: vec![data(at(1, 1)), data(Health(1))] };
    let owned = OwnedBy { owner: EntityRef::new(owner()) };
    let components = vec![data(at(2, 2)), data(Health(2)), data(Diary(5)), data(owned)];
    let item_row = BaselineRow { id: NetId::cooked(81), components };
    place_document(rig.world_mut(), DOCUMENT, vec![owner_row, item_row]);
    assert_eq!(owner_of(rig.world(), item()), Some(owner()));
}

/// The item as the document gave it: its region, position, health and unreplicated diary.
fn item_state(world: &World) -> Option<(RegionId, Position, Health, Diary)> {
    Some((
        world.region_of(item())?,
        *world.get::<Position>(item())?,
        *world.get::<Health>(item())?,
        *world.get::<Diary>(item())?,
    ))
}

/// The server released the item, which no frame carries (a release of an edge is no entry
/// of the log), and then destroyed the owner: the one tombstone is the owner's. The hook
/// despawns the owner alone, so the item stays with what the document gave it and the
/// server's next row of it is written; a rollback through the tombstone's tick re-applies
/// the same, hash for hash. The cascading despawn took the item, and its row was skipped.
#[test]
fn a_released_item_survives_its_former_owners_tombstone() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    owned_document(&mut rig);
    rig.step(walking(13));
    rig.step(looking(14));
    let mut frame = StagedFrame::new(Tick(14));
    let destroyed =
        Tombstone { region: DOCUMENT, tick: Tick(14), cause: TombstoneCause::Destroyed };
    frame.despawns.insert(owner(), destroyed);
    rig.push(frame);
    rig.step(walking(15));
    assert!(!rig.world().contains(owner()));
    assert_eq!(item_state(rig.world()), Some((DOCUMENT, at(2, 2), Health(2), Diary(5))));
    assert!(rig.reconcile().corrections.is_empty());
    let mut frame = StagedFrame::new(Tick(15));
    dequantised(&mut frame, item(), &at(9, 9));
    rig.push(frame);
    rig.step(walking(16));
    assert_eq!(item_state(rig.world()), Some((DOCUMENT, at(9, 9), Health(2), Diary(5))));
    assert!(rig.reconcile().corrections.is_empty());

    // A substitution of 14 by a sample that differs by its yaw alone rolls back to 13 and
    // re-simulates the tombstone's frame and the row's, to the same world.
    let mut frame = StagedFrame::new(Tick(16));
    frame.substituted.insert(Tick(14));
    rig.push(frame);
    rig.step(walking(17));
    let hash = rig.hash();
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled)[0].0, Tick(13));
    assert_eq!(reconciled.resimulated, 4);
    assert_eq!(rig.hash(), hash, "the re-simulated world equals the straight run");
    assert!(!rig.world().contains(owner()));
    assert_eq!(item_state(rig.world()), Some((DOCUMENT, at(9, 9), Health(2), Diary(5))));
}

/// The same document; the server moves the owner into another loaded region, as a frame's
/// move and, in a second run, as an upsert naming that region. The owner resides there,
/// placed there; the item stays where the document put it, placed by no frame.
#[test]
fn a_released_item_stays_when_its_former_owner_moves() {
    for upsert in [false, true] {
        let mut rig = Rig::joined();
        rig.step_to(12, walking);
        owned_document(&mut rig);
        rig.step(walking(13));
        let mut frame = StagedFrame::new(Tick(13));
        if upsert {
            let spawn = remote(OTHER_REGION, 8, vec![data(at(1, 1)), data(Health(1))]);
            frame.spawns.insert(NetId::cooked(80), spawn);
        } else {
            frame.moves.insert(owner(), OTHER_REGION);
        }
        rig.push(frame);
        rig.step(walking(14));
        let world = rig.world();
        assert_eq!(world.region_of(owner()), Some(OTHER_REGION), "upsert {upsert}");
        assert_eq!(placed(world, owner()), Some(OTHER_REGION), "upsert {upsert}");
        assert_eq!(item_state(world), Some((DOCUMENT, at(2, 2), Health(2), Diary(5))));
        assert_eq!(placed(world, item()), None, "upsert {upsert}");
        assert!(rig.reconcile().corrections.is_empty());
    }
}

/// Every unechoed predicted spawn of the local player, by input tick.
fn balls_of(world: &World) -> Vec<StableKey> {
    let mut balls: Vec<StableKey> =
        world.claimed(PLAYER).filter(|key| matches!(key, StableKey::Predicted(_))).collect();
    balls.sort_by_key(|key| match key {
        StableKey::Predicted(predicted) => predicted.input_tick.0,
        _ => unreachable!(),
    });
    balls
}

/// A predicted spawn the server never echoes, owning a proxy (an edge a predicted command
/// wrote, written here as its apply would) and a second predicted spawn: the first goes
/// alone when the acknowledgement passes its input tick, counted, the proxy stays, and the
/// second goes at its own input tick.
#[test]
fn an_unechoed_spawn_is_despawned_alone() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    rig.step(throwing());
    rig.step(walking(14));
    rig.step(throwing());
    let balls = balls_of(rig.world());
    let [first, second] = balls[..] else { panic!("two balls: {balls:?}") };
    let owned = OwnedBy { owner: EntityRef::new(first) };
    rig.world_mut().insert(proxy(), owned).unwrap();
    rig.world_mut().insert(second, owned).unwrap();
    let mut frame = StagedFrame::new(Tick(14));
    frame.acked_input = Some(Tick(14));
    rig.push(frame);
    rig.step(walking(16));
    let world = rig.world();
    assert!(!world.contains(first));
    assert!(world.contains(proxy()), "the server's proxy is not the client's to destroy");
    assert!(world.contains(second), "its own input tick is not passed");
    rig.reconcile();
    assert_eq!(rig.p().stats().unechoed_spawns, 1);
    let mut frame = StagedFrame::new(Tick(16));
    frame.acked_input = Some(Tick(16));
    rig.push(frame);
    rig.step(walking(17));
    assert!(!rig.world().contains(second));
    assert!(rig.world().contains(proxy()));
    rig.reconcile();
    assert_eq!(rig.p().stats().unechoed_spawns, 2);
}

/// The stated transient components of `key`: `Aim`, with its value, which a hook never
/// writes to a claimed entity, and whether it has `Stance`.
fn transients(world: &World, key: StableKey) -> (Option<Aim>, bool) {
    (world.get::<Aim>(key).copied(), world.get::<Stance>(key).is_some())
}

/// An unechoed predicted spawn despawned at the acknowledgement, then its echo after all,
/// with a replicated transient component among its components and a statement naming an
/// unreplicated one: spawned at once with both, zeroed, which the kernel that names them
/// runs on; and again by the correction at the echo's tick.
#[test]
fn a_late_echo_carries_its_transient_components() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    rig.step(throwing());
    let ball = ball_of(rig.world()).expect("the throw spawned a predicted ball");
    let StableKey::Predicted(predicted) = ball else { unreachable!() };
    let mut frame = StagedFrame::new(Tick(14));
    frame.acked_input = Some(Tick(14));
    rig.push(frame);
    rig.step(walking(14));
    assert!(!rig.world().contains(ball));
    rig.reconcile();
    rig.step(walking(15));

    // Popped at 16: the slot of its tick lacks the ball.
    let mut frame = StagedFrame::new(Tick(15));
    let echo = lactose_net_api::StagedSpawn {
        origin: Some(predicted),
        ..claimed(REGION, vec![data(Health(1)), data(Aim(7))])
    };
    frame.spawns.insert(net(40), echo);
    frame.transient.insert(ball, [Stance::KEY].into());
    rig.push(frame);
    rig.step(walking(16));
    assert_eq!(transients(rig.world(), ball), (Some(Aim(0)), true));
    assert_eq!(braced(rig.world(), ball), 1, "the kernel ran on it in the tick");
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(reconciled.corrections[0].tick, Tick(15));
    assert_eq!(transients(rig.world(), ball), (Some(Aim(0)), true));
    assert_eq!(braced(rig.world(), ball), 2, "and in the re-simulated tick");
}

/// A frame stating the transient components of proxies, which it does not state claimed,
/// in a delta frame and in a full frame: the world is as it is without the statements,
/// hash for hash and byte for byte in the slot.
#[test]
fn a_proxy_gets_no_unreplicated_transient_component() {
    let both = || -> BTreeSet<ComponentKey> { [Stance::KEY, Crouch::KEY].into() };
    let lacks = |world: &World, of: StableKey| {
        world.contains(of) && world.get::<Stance>(of).is_none() && world.get::<Crouch>(of).is_none()
    };
    let run = |stated: bool| {
        let mut rig = Rig::joined();
        rig.step_to(14, walking);
        let mut frame = StagedFrame::new(Tick(14));
        frame.spawns.insert(net(5), remote(REGION, 3, vec![data(at(1, 1)), data(Health(2))]));
        dequantised(&mut frame, proxy(), &at(-40, 0));
        if stated {
            frame.transient.insert(key(5), both());
            frame.transient.insert(proxy(), both());
        }
        rig.push(frame);
        rig.step(walking(15));
        assert!(lacks(rig.world(), key(5)) && lacks(rig.world(), proxy()));
        let delta = (rig.hash(), rig.p().ring().slot_bytes(Tick(15)));
        rig.step_to(20, walking);
        let mut frame = join_frame(Tick(18));
        if stated {
            frame.transient.insert(proxy(), both());
        }
        rig.push(frame);
        rig.step(walking(21));
        assert!(lacks(rig.world(), proxy()));
        let full = (rig.hash(), rig.p().ring().slot_bytes(Tick(21)));
        (delta, full)
    };
    assert_eq!(run(true), run(false));
}

/// A full frame's claimed spawn and a delta frame's proxy spawn carrying a derived
/// component and an unreplicated transient one as values, the proxy's with a placement of
/// its own too: the row has none of them as the frame sent them, the rest of the spawn
/// applied (`tests/logs.rs` holds what is logged).
#[test]
fn a_spawn_writes_replicated_components_alone() {
    let sent = Trail { x: 999, updates: 77 };
    let mut rig = Rig::new(simulation(LayoutConfig::default(), None), 1);
    let mut frame = join_frame(JOIN);
    let cart_spawn = frame.spawns.get_mut(&net(2)).expect("the cart");
    cart_spawn.components.extend([data(sent), data(Stance(5))]);
    cart_spawn.components.sort_by_key(|data| data.component);
    rig.push(frame);
    rig.step(PlayerInput::IDLE);
    rig.reconcile();
    let world = rig.world();
    assert_eq!(world.get::<Health>(cart()), Some(&Health(10)));
    assert_eq!(rig.position(cart()), Some(at(50, 0)));
    assert_ne!(world.get::<Trail>(cart()), Some(&sent), "its own kernel's, not the frame's");
    assert_eq!(world.get::<Stance>(cart()), None);

    rig.step_to(14, walking);
    // The placement of the proxy, which resides in `REGION`, sent with a spawn into
    // `OTHER_REGION`.
    let stale = world_placement(rig.world(), proxy());
    let mut frame = StagedFrame::new(Tick(14));
    let components = vec![data(at(1, 1)), data(Health(2)), data(sent), data(Stance(5)), stale];
    frame.spawns.insert(net(5), remote(OTHER_REGION, 3, components));
    rig.push(frame);
    rig.step(walking(15));
    let world = rig.world();
    assert_eq!(world.get::<Health>(key(5)), Some(&Health(2)));
    assert_eq!(rig.position(key(5)), Some(at(1, 1)));
    assert_eq!(world.get::<Trail>(key(5)), None);
    assert_eq!(world.get::<Stance>(key(5)), None);
    assert_eq!(placed(world, key(5)), Some(OTHER_REGION), "the hook's placement, not the frame's");
}

/// `key`'s placement as a spawn's component.
fn world_placement(world: &World, key: StableKey) -> ComponentData {
    let bytes = world.get_erased(key, StagedRegion::KEY).expect("a placed proxy").to_vec();
    ComponentData { component: StagedRegion::KEY, bytes }
}
