//! Substitution: the server substituted the local player's sample for a tick it never
//! received; the ring's sample is replaced by the same substitute, the first delivered
//! sample after it is repaired, and the world re-simulated equals a run that applied them
//! from the start.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::Tick;
use lactose_input::PlayerInput;
use lactose_net_api::StagedFrame;
use lactose_predict::{CorrectionCause, ResyncReason};

/// What the player sends: walking, with a jump pressed at 14 and held through 16.
fn sample(tick: u32) -> PlayerInput {
    match tick {
        14 => jumping(3, true),
        15 | 16 => jumping(1, false),
        _ => moving(1),
    }
}

/// What the server applied when it lost 14: the substitute of 13, then 15 with its edges
/// recomputed against it.
fn applied(tick: u32) -> PlayerInput {
    match tick {
        14 => sample(13).substitute(),
        15 => sample(15).after_substitute(&sample(13).substitute()),
        _ => sample(tick),
    }
}

fn substituted(tick: u32, ticks: &[u32]) -> StagedFrame {
    let mut frame = StagedFrame::new(Tick(tick));
    frame.substituted.extend(ticks.iter().map(|&t| Tick(t)));
    frame
}

#[test]
fn a_substitution_and_its_repair_resimulate_to_the_run_that_had_them() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, sample);
    reference.step_to(21, applied);
    rig.push(substituted(16, &[14]));
    rig.step(sample(21));
    let reconciled = rig.reconcile();
    let summary: Vec<_> =
        reconciled.corrections.iter().map(|c| (c.tick, c.depth, c.cause)).collect();
    // The substitution of 14 is a point at 13, the repair of 15 at 14: one rollback, from
    // the older.
    assert_eq!(summary, vec![(Tick(13), 8, CorrectionCause::Substituted)]);
    assert!(reconciled.corrections[0].entities.is_empty());
    assert_eq!(reconciled.resimulated, 8);
    assert_eq!(rig.hash(), reference.hash());
    // The repair fired the press at 15 that the lost 14 carried.
    assert_eq!(rig.position(me()), reference.position(me()));
    assert_eq!(rig.p().stats().substitutions, 1);
    assert_ne!(sample(14), applied(14));
    assert_ne!(sample(15), applied(15));
}

#[test]
fn the_repair_is_found_by_tick_in_a_later_call() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, sample);
    reference.step_to(23, applied);
    // The frame of tick 14 reports the substitution; 15's repair waits for a frame at or
    // after 15, which arrives in the next call and spans 15 to 17.
    rig.push(substituted(14, &[14]));
    rig.step(sample(21));
    let first = rig.reconcile();
    let summary: Vec<_> = first.corrections.iter().map(|c| (c.tick, c.cause)).collect();
    assert_eq!(summary, vec![(Tick(13), CorrectionCause::Substituted)]);
    rig.step(sample(22));
    let quiet = rig.reconcile();
    assert!(quiet.corrections.is_empty(), "no frame, no repair");
    rig.push(StagedFrame::new(Tick(17)));
    rig.step(sample(23));
    let second = rig.reconcile();
    let summary: Vec<_> = second.corrections.iter().map(|c| (c.tick, c.depth, c.cause)).collect();
    assert_eq!(summary, vec![(Tick(14), 9, CorrectionCause::Repaired)]);
    assert_eq!(rig.hash(), reference.hash());
    assert_eq!(rig.p().stats().substitutions, 1);
}

#[test]
fn consecutive_substitutions_continue_the_replaced_sample() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    let lost = |tick: u32| match tick {
        // 14 and 15 lost: 15's substitute continues 14's, which continued 13.
        14 | 15 => sample(13).substitute(),
        16 => sample(16).after_substitute(&sample(13).substitute()),
        _ => sample(tick),
    };
    rig.step_to(20, sample);
    reference.step_to(21, lost);
    rig.push(substituted(16, &[14, 15]));
    rig.step(sample(21));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(reconciled.corrections[0].tick, Tick(13));
    assert_eq!(rig.hash(), reference.hash());
    assert_eq!(rig.p().stats().substitutions, 2);
}

#[test]
fn a_substitution_at_or_before_the_restart_is_skipped_with_its_repair() {
    let mut rig = Rig::joined();
    rig.step_to(20, sample);
    let hash = rig.hash();
    // The ring restarted at 11, where the join frame was applied.
    assert_eq!(rig.p().ring().rebuilt_at(), Some(Tick(11)));
    rig.push(substituted(12, &[11]));
    rig.step(sample(21));
    let after = rig.hash();
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert_eq!(rig.hash(), after);
    assert_ne!(hash, after);
    assert_eq!(rig.p().stats().substitutions, 0);
}

#[test]
fn a_substitution_in_a_frame_before_the_newest_full_frame_is_skipped_with_its_repair() {
    let mut rig = Rig::joined();
    rig.step_to(20, sample);
    rig.push(substituted(15, &[15]));
    rig.push(join_frame(Tick(16)));
    rig.push(StagedFrame::new(Tick(17)));
    rig.step(sample(21));
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert_eq!(reconciled.frames, 3);
    assert_eq!(reconciled.confirmed, Some(Tick(17)));
    assert_eq!(rig.p().ring().rebuilt_at(), Some(Tick(21)));
    assert_eq!(rig.p().ring().oldest(), Some(Tick(21)));
    assert_eq!(rig.p().stats().substitutions, 0);
}

#[test]
fn a_substitution_before_the_horizon_is_too_old() {
    let mut rig = Rig::joined();
    rig.p_mut().set_depth(4).unwrap();
    rig.step_to(20, sample);
    rig.push(substituted(19, &[18]));
    rig.step(sample(21));
    // 21, 20, 19, 18: the substitute for 18 replaces the sample of 18 and re-simulates
    // from 17, which the ring no longer holds.
    assert_eq!(rig.p().ring().oldest(), Some(Tick(18)));
    let hash = rig.hash();
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.resync, Some(ResyncReason::TooOld { frame: Tick(19), oldest: Tick(18) }));
    assert!(reconciled.corrections.is_empty());
    assert_eq!(rig.hash(), hash);
    assert_eq!(rig.p().stats().substitutions, 0);
}
