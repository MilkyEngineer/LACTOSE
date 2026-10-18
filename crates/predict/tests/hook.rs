//! The hook: what a staged frame does to the prediction world inside the tick, in the
//! README's order, and the local player's claim and anchor on every path.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{PlayerId, StableKey, Tick};
use lactose_ecs::World;
use lactose_input::player_entity;
use lactose_net_api::{RemoteSlot, StagedFrame, Tombstone, TombstoneCause};
use lactose_predict::{CorrectionCause, LocalPlayer};
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
    let left = |region| Tombstone { region, tick: Tick(13), cause: TombstoneCause::Left };
    let mut frame = StagedFrame::new(Tick(13));
    frame.despawns.insert(key(5), left(REGION));
    frame.despawns.insert(key(6), left(OTHER_REGION));
    rig.push(frame);
    rig.step(walking(14));
    assert!(rig.world().contains(key(5)), "settled by its region");
    assert!(!rig.world().contains(key(6)), "it resides elsewhere");
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
    rig.step(walking(13));
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

    // A rollback replays the proxies' history: a substitution of 13 by the sample the
    // player sent anyway restores 12 and re-simulates 13 to 16 with the frames popped
    // there, to the same world.
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
