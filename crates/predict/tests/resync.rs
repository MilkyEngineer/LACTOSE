//! Resync: a prediction world that fell behind, or whose frames are older than its ring,
//! is told so; `resync` restarts the ring at the target tick, keeps what it knows of the
//! claimed entities until the next full frame replaces it, and that frame rebuilds. The
//! frames between a full frame's tick and the restart are reconciled at the restart.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{StableKey, Tick};
use lactose_net_api::{StagedFrame, Tombstone, TombstoneCause};
use lactose_predict::ResyncReason;
use lactose_sim::StagedState;

#[test]
fn a_queued_frame_after_the_present_is_behind() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    rig.push(StagedFrame::new(Tick(25)));
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(
        reconciled.resync,
        Some(ResyncReason::Behind { server: Tick(25), present: Tick(21) })
    );
    assert_eq!(reconciled.frames, 0);
    // Asked again with nothing popped: still behind.
    assert!(matches!(rig.reconcile().resync, Some(ResyncReason::Behind { .. })));
}

#[test]
fn a_queued_frame_at_or_before_the_present_is_not_behind() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    // Pushed after the frame's steps: it waits for the next step.
    rig.push(StagedFrame::new(Tick(20)));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.resync, None);
    assert_eq!(reconciled.confirmed, None);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!((reconciled.frames, reconciled.confirmed), (1, Some(Tick(20))));
    assert_eq!(reconciled.resync, None);
}

#[test]
fn a_frame_older_than_the_ring_is_too_old_and_nothing_changes() {
    let mut rig = Rig::joined();
    rig.p_mut().set_depth(4).unwrap();
    assert_eq!(rig.p().ring().len(), 1);
    rig.step_to(20, walking);
    assert_eq!(rig.p().ring().len(), 4);
    assert_eq!(rig.p().ring().oldest(), Some(Tick(17)));
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(100, 0));
    rig.push(frame);
    let mut later = StagedFrame::new(Tick(19));
    exact(&mut later, me(), &at(100, 0));
    rig.push(later);
    rig.step(walking(21));
    let hash = rig.hash();
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.resync, Some(ResyncReason::TooOld { frame: Tick(15), oldest: Tick(18) }));
    // This frame and every later one of the call are discarded.
    assert!(reconciled.corrections.is_empty());
    assert_eq!(reconciled.frames, 0);
    assert_eq!(rig.hash(), hash);
    assert_eq!(rig.p().ring().oldest(), Some(Tick(18)));
    assert_eq!(rig.p().ring().newest(), Some(Tick(21)));
}

#[test]
fn set_depth_drops_slots_and_moves_the_horizon() {
    let mut rig = Rig::joined();
    rig.step_to(30, walking);
    assert_eq!(rig.p().ring().len(), 20);
    assert_eq!(rig.p().ring().oldest(), Some(Tick(11)));
    rig.p_mut().set_depth(5).unwrap();
    assert_eq!(rig.p().ring().len(), 5);
    assert_eq!(rig.p().ring().oldest(), Some(Tick(26)));
    assert_eq!(rig.p().ring().newest(), Some(Tick(30)));
    // The restart's slot is gone, so is `rebuilt_at`.
    assert_eq!(rig.p().ring().rebuilt_at(), None);
    assert!(rig.p_mut().set_depth(1).is_err());
    assert_eq!(rig.p().ring().depth(), 5);
}

#[test]
fn resync_restarts_the_ring_keeps_the_known_regions_and_skips_the_queued_frames() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    rig.push(StagedFrame::new(Tick(25)));
    rig.step(walking(21));
    assert!(matches!(rig.reconcile().resync, Some(ResyncReason::Behind { .. })));
    // A hook inserted over the predictor's is replaced by the resync.
    rig.world_mut().insert_service::<dyn StagedState>(Box::new(Inert));
    rig.p_mut().resync(Tick(30)).unwrap();
    let ring = rig.p().ring();
    assert_eq!((ring.len(), ring.oldest(), ring.rebuilt_at()), (1, Some(Tick(30)), Some(Tick(30))));
    assert_eq!(rig.tick(), Tick(30));
    assert_eq!(rig.p().stats().resyncs, 1);
    // The prediction world lost the cart while the full frame is on its way.
    rig.world_mut().despawn(cart()).unwrap();
    rig.step(walking(31));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.resync, None, "the queued frame is from before the restart");
    assert_eq!(reconciled.confirmed, Some(Tick(25)));
    assert!(reconciled.corrections.is_empty());
    // The cart's rows after the restart: the known region brings it back.
    rig.step(walking(32));
    let mut frame = StagedFrame::new(Tick(32));
    exact(&mut frame, cart(), &at(50, 0));
    exact(&mut frame, cart(), &Health(2));
    rig.push(frame);
    rig.step(walking(33));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(rig.world().region_of(cart()), Some(REGION));
    // And the next full frame rebuilds.
    rig.push(join_frame(Tick(33)));
    rig.step(walking(34));
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert_eq!(rig.p().ring().rebuilt_at(), Some(Tick(34)));
    assert_eq!(rig.position(cart()), Some(at(50, 0)));
    assert_eq!(rig.world().get::<Health>(cart()), Some(&Health(10)));
}

#[test]
fn a_rejoin_resets_the_inbox_beside_the_resync() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    rig.push(StagedFrame::new(Tick(25)));
    // A new session whose ticks lie before the old one's.
    rig.inbox().reset();
    rig.p_mut().resync(Tick(8)).unwrap();
    rig.push(join_frame(Tick(2)));
    rig.step(walking(9));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.resync, None);
    assert_eq!(reconciled.confirmed, Some(Tick(2)));
    assert_eq!(rig.p().ring().rebuilt_at(), Some(Tick(9)));
}

// ---- The band between a full frame's tick and the restart ----

/// Joined (the full frame for 4 applied at 11, so the restart is 11 and frames for 5 to 10
/// are the band), then `event` for server tick `at` pushed and stepped to 12, then a frame
/// per tick for 11 to 20, two ticks late, each with the player's row as predicted and
/// `later`'s. Returns the rig and every correction's tick and entities.
fn band(
    at: u32,
    event: impl FnOnce(&mut StagedFrame),
    later: impl Fn(&mut StagedFrame),
) -> (Rig, Vec<(Tick, Vec<StableKey>)>) {
    let mut rig = Rig::joined();
    assert_eq!(rig.p().ring().rebuilt_at(), Some(Tick(11)));
    let mut corrections = Vec::new();
    let mut reconcile = |rig: &mut Rig| {
        let reconciled = rig.reconcile();
        assert_eq!(reconciled.resync, None);
        corrections.extend(reconciled.corrections.into_iter().map(|c| (c.tick, c.entities)));
    };
    let mut frame = StagedFrame::new(Tick(at));
    event(&mut frame);
    rig.push(frame);
    rig.step(walking(12));
    reconcile(&mut rig);
    for present in 13..=22 {
        let tick = present - 2;
        if tick > at {
            let mut frame = StagedFrame::new(Tick(tick));
            exact(&mut frame, me(), &common::at(i32::try_from(tick).unwrap() - 11, 0));
            later(&mut frame);
            rig.push(frame);
        }
        rig.step(walking(present));
        reconcile(&mut rig);
    }
    (rig, corrections)
}

#[test]
fn a_band_tombstone_of_a_claimed_entity_is_applied_at_the_restart() {
    let (rig, corrections) = band(
        6,
        |frame| {
            frame.despawns.insert(
                cart(),
                Tombstone { region: REGION, tick: Tick(6), cause: TombstoneCause::Destroyed },
            );
        },
        |_| {},
    );
    assert_eq!(corrections, vec![(Tick(11), vec![cart()])]);
    assert!(!rig.world().contains(cart()), "destroyed at 6");
}

#[test]
fn a_tombstone_of_a_claimed_entity_after_the_band_is_applied_at_its_tick() {
    let (rig, corrections) = band(
        12,
        |frame| {
            frame.despawns.insert(
                cart(),
                Tombstone { region: REGION, tick: Tick(12), cause: TombstoneCause::Destroyed },
            );
        },
        |_| {},
    );
    assert_eq!(corrections, vec![(Tick(12), vec![cart()])]);
    assert!(!rig.world().contains(cart()), "destroyed at 12");
}

#[test]
fn a_band_claim_of_a_proxy_is_applied_at_the_restart() {
    let (rig, corrections) = band(
        6,
        |frame| {
            frame.spawns.insert(net(3), claimed(REGION, vec![data(at(-50, 0))]));
        },
        // The server, which has the proxy claimed now, sends its exact rows once, at 14.
        |frame| {
            if frame.tick == Tick(14) {
                exact(frame, proxy(), &at(-40, 0));
            }
        },
    );
    assert_eq!(corrections, vec![(Tick(11), vec![proxy()]), (Tick(14), vec![proxy()])]);
    assert_eq!(claim(rig.world(), proxy()), Some(PLAYER));
    assert_eq!(rig.position(proxy()), Some(at(-40, 0)));
}

#[test]
fn a_band_release_of_a_claimed_entity_is_applied_at_the_restart() {
    let (rig, corrections) = band(
        6,
        |frame| {
            let parts = vec![data(at(50, 0)), data(Health(10))];
            frame.spawns.insert(net(2), remote(REGION, 1, parts));
        },
        // The server, which has the cart as a proxy now, sends its dequantised rows.
        |frame| {
            dequantised(frame, cart(), &at(60 + i32::try_from(frame.tick.0).unwrap(), 0));
        },
    );
    assert_eq!(corrections, vec![(Tick(11), vec![cart()])]);
    assert_eq!(claim(rig.world(), cart()), None);
    assert_eq!(rig.position(cart()), Some(at(80, 0)));
}

#[test]
fn a_band_spawn_of_a_claimed_entity_is_applied_at_the_restart() {
    let (rig, corrections) = band(
        6,
        |frame| {
            frame.spawns.insert(net(4), claimed(REGION, vec![data(at(5, 5)), data(Health(1))]));
        },
        |frame| {
            if frame.tick == Tick(14) {
                exact(frame, key(4), &at(6, 6));
                exact(frame, key(4), &Health(1));
            }
        },
    );
    assert_eq!(corrections, vec![(Tick(11), vec![key(4)]), (Tick(14), vec![key(4)])]);
    assert_eq!(claim(rig.world(), key(4)), Some(PLAYER));
    assert_eq!(rig.position(key(4)), Some(at(6, 6)));
}

#[test]
fn a_band_row_alone_makes_no_correction_and_the_first_frame_after_corrects_once() {
    let cart_at_70 = |frame: &mut StagedFrame| {
        exact(frame, cart(), &at(70, 0));
        exact(frame, cart(), &Health(10));
    };
    let (rig, corrections) = band(6, cart_at_70, cart_at_70);
    assert_eq!(corrections, vec![(Tick(11), vec![cart()])]);
    assert_eq!(rig.position(cart()), Some(at(70, 0)));
}

#[test]
fn a_full_frame_replaces_the_known_regions() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    // The server destroyed the cart and its tombstone was lost: the next full frame lacks
    // it, and the prediction world, which lost it too, must not bring it back.
    rig.world_mut().despawn(cart()).unwrap();
    let mut full = StagedFrame::resync(Tick(18));
    full.spawns.insert(net(1), claimed(REGION, vec![data(at(7, 0))]));
    full.spawns.insert(net(3), remote(REGION, 0, vec![data(at(-50, 0))]));
    rig.push(full);
    rig.step(walking(21));
    assert!(rig.reconcile().corrections.is_empty());
    let mut frame = StagedFrame::new(Tick(21));
    frame.moves.insert(cart(), OTHER_REGION);
    rig.push(frame);
    rig.step(walking(22));
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty(), "{:?}", summaries(&reconciled));
    assert!(!rig.world().contains(cart()));
}

/// A hook that does nothing.
struct Inert;

impl StagedState for Inert {
    fn apply(&mut self, _: &mut lactose_ecs::World, _: Tick) -> lactose_core::Result<()> {
        Ok(())
    }
}
