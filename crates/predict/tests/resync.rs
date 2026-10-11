//! Resync: a prediction world that fell behind, or whose frames are older than its ring,
//! is told so; `resync` restarts the ring at the target tick, keeps what it knows of the
//! claimed entities, and the next full frame rebuilds.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::Tick;
use lactose_net_api::StagedFrame;
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
    // A row of the cart after the restart: the known region brings it back.
    rig.step(walking(32));
    let mut frame = StagedFrame::new(Tick(32));
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

/// A hook that does nothing.
struct Inert;

impl StagedState for Inert {
    fn apply(&mut self, _: &mut lactose_ecs::World, _: Tick) -> lactose_core::Result<()> {
        Ok(())
    }
}
