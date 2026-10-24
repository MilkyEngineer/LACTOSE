//! Substitution: the server substituted the local player's sample for a tick it never
//! received; the ring's sample is replaced by the same substitute, the first delivered
//! sample after it is repaired, and the world re-simulated equals a run that applied them
//! from the start.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::Tick;
use lactose_input::{Action, Actions, PlayerInput};
use lactose_net_api::StagedFrame;
use lactose_predict::{CorrectionCause, Reconciled, ResyncReason};
use lactose_sim::TICK_RATE_HZ;

/// What the player sends: walking, with a jump pressed at 14 and held through 16.
fn sample(tick: u32) -> PlayerInput {
    match tick {
        14 => jumping(3, true),
        15 | 16 => jumping(1, false),
        _ => moving(1),
    }
}

/// [`sample`], with a jump pressed again at 18 and held at 19: the substitute of 18 and the
/// repair of 19 each change the sample.
fn jump_at_18(tick: u32) -> PlayerInput {
    match tick {
        18 => jumping(1, true),
        19 => jumping(1, false),
        _ => sample(tick),
    }
}

/// Walking, with a jump pressed at 12 and released at 13: the substitute of 13 holds the
/// jump and the repair of 14 releases it there, so each changes the sample.
fn tapping(tick: u32) -> PlayerInput {
    let jump = Actions::EMPTY.with(Action::Jump);
    match tick {
        12 => jumping(1, true),
        13 => PlayerInput { released: jump, ..moving(1) },
        _ => walking(tick),
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
    // from 17, which the ring no longer holds. The substitute equals the sample of 18, but
    // only the sample of 17 tells, and it left with its slot.
    assert_eq!(rig.p().ring().oldest(), Some(Tick(18)));
    let hash = rig.hash();
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.resync, Some(ResyncReason::TooOld { frame: Tick(19), oldest: Tick(18) }));
    assert!(reconciled.corrections.is_empty());
    assert_eq!(rig.hash(), hash);
    assert_eq!(rig.p().stats().substitutions, 0);
}

// ---- A replacement that changes no byte ----
//
// The world already ran the sample, so a rollback from the tick before would re-simulate
// to the state it holds: no correction point, nothing re-simulated, no resync on its
// account. The tick is still acted on: counted, remembered, its repair armed.

/// Reconciles and returns what it did with the ticks it re-simulated by the counter.
fn reconcile_counted(rig: &mut Rig) -> (Reconciled, u64) {
    let before = rig.p().stats().ticks_resimulated;
    let reconciled = rig.reconcile();
    let resimulated = rig.p().stats().ticks_resimulated - before;
    (reconciled, resimulated)
}

/// 13 continues 12 with no edge, so its substitute is its own sample; so is the repair of
/// 14 against it, which presses the jump 14 pressed.
#[test]
fn an_equal_substitution_rolls_nothing_back_and_is_still_acted_on() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, sample);
    reference.step_to(22, sample);
    assert_eq!(sample(12).substitute(), sample(13));
    assert_eq!(sample(14).after_substitute(&sample(12).substitute()), sample(14));
    rig.push(substituted(13, &[13]));
    rig.step(sample(21));
    let hash = rig.hash();
    let (reconciled, resimulated) = reconcile_counted(&mut rig);
    assert!(reconciled.corrections.is_empty());
    assert_eq!((reconciled.resimulated, resimulated), (0, 0));
    assert_eq!(reconciled.resync, None);
    assert_eq!(reconciled.confirmed, Some(Tick(13)));
    assert_eq!(rig.hash(), hash);
    assert_eq!(rig.p().stats().substitutions, 1);
    // The repair of 14 is due with this frame; the repeat of 13 is known, not counted again.
    rig.push(substituted(14, &[13]));
    rig.step(sample(22));
    let (repeated, resimulated) = reconcile_counted(&mut rig);
    assert!(repeated.corrections.is_empty());
    assert_eq!((repeated.resimulated, resimulated), (0, 0));
    assert_eq!(rig.p().stats().substitutions, 1);
    assert_eq!(rig.hash(), reference.hash());
}

/// The substitution of 14 changes its sample and rolls back; the repair of 15, due in a
/// later call, is its own sample (neither holds an action) and re-simulates nothing.
#[test]
fn an_equal_repair_resimulates_nothing() {
    let fast_at_14 = |tick: u32| if tick == 14 { moving(2) } else { walking(tick) };
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, fast_at_14);
    // The server applied the substitute of 13 at 14, and 15 as sent.
    reference.step_to(22, walking);
    assert_eq!(walking(15).after_substitute(&walking(13).substitute()), walking(15));
    rig.push(substituted(14, &[14]));
    rig.step(walking(21));
    let (first, resimulated) = reconcile_counted(&mut rig);
    assert_eq!(points(&first), vec![(Tick(13), CorrectionCause::Substituted)]);
    assert_eq!((first.resimulated, resimulated), (8, 8));
    rig.push(StagedFrame::new(Tick(15)));
    rig.step(walking(22));
    let hash = rig.hash();
    let (repair, resimulated) = reconcile_counted(&mut rig);
    assert!(repair.corrections.is_empty());
    assert_eq!((repair.resimulated, resimulated), (0, 0));
    assert_eq!(repair.resync, None);
    assert_eq!(rig.hash(), hash);
    assert_eq!(rig.hash(), reference.hash());
}

/// Outside net-api's rule: 14 first, its substitute equal to its sample; then 13 first,
/// whose substitute changes 13. The repair of 14 that 13 arms substitutes 14 again,
/// continuing 13's new substitute, which now changes 14 and is acted on; a later repeat of
/// 14 is still skipped.
#[test]
fn an_equal_substitution_is_substituted_again_once_the_tick_before_it_changes() {
    let fast_at_12 = |tick: u32| if tick == 12 { moving(3) } else { walking(tick) };
    let lost_13_14 = |tick: u32| if (12..=14).contains(&tick) { moving(3) } else { walking(tick) };
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, fast_at_12);
    reference.step_to(23, lost_13_14);
    rig.push(substituted(14, &[14]));
    rig.step(walking(21));
    let (equal, resimulated) = reconcile_counted(&mut rig);
    assert!(equal.corrections.is_empty());
    assert_eq!(resimulated, 0);
    rig.push(substituted(15, &[13, 14]));
    rig.step(walking(22));
    let older = rig.reconcile();
    assert_eq!(points(&older), vec![(Tick(12), CorrectionCause::Substituted)]);
    rig.push(substituted(16, &[14]));
    rig.step(walking(23));
    let (repeated, resimulated) = reconcile_counted(&mut rig);
    assert!(repeated.corrections.is_empty());
    assert_eq!(resimulated, 0);
    assert_eq!(rig.p().stats().substitutions, 2);
    assert_eq!(rig.hash(), reference.hash());
}

/// The reviewer's shape (s3 of review r1 of the repeated report): the repair of 19 is its
/// own sample, but it is due only once slot 18 has left the ring, and only slot 18's
/// sample tells: `TooOld`, with nothing changed, as any repair whose previous slot is gone.
#[test]
fn an_equal_repair_past_the_ring_is_too_old() {
    let fast_at_18 = |tick: u32| if tick == 18 { moving(2) } else { walking(tick) };
    let mut rig = Rig::joined();
    rig.p_mut().set_depth(6).unwrap();
    rig.step_to(20, fast_at_18);
    rig.push(substituted(18, &[18]));
    rig.step(walking(21));
    assert_eq!(points(&rig.reconcile()), vec![(Tick(17), CorrectionCause::Substituted)]);
    assert_eq!(walking(19).after_substitute(&walking(17).substitute()), walking(19));
    rig.step_to(23, walking);
    rig.push(StagedFrame::new(Tick(19)));
    rig.step(walking(24));
    assert_eq!(rig.p().ring().oldest(), Some(Tick(19)));
    let hash = rig.hash();
    let (reconciled, resimulated) = reconcile_counted(&mut rig);
    assert_eq!(reconciled.resync, Some(ResyncReason::TooOld { frame: Tick(19), oldest: Tick(19) }));
    assert!(reconciled.corrections.is_empty());
    assert_eq!(resimulated, 0);
    assert_eq!(rig.hash(), hash);
}

// ---- A repeated report ----
//
// net-api's rule: a substitution is in the first frame accepted at or after its tick, and
// the server may repeat it in later frames until it sees one of them acknowledged. Each
// case below keeps that rule. A repeat that rolled back again would re-simulate a claim or
// a release an earlier call reconciled, which a replay never re-applies and no later frame
// restates (review r2 of the proxy region, P1: q1, q2 and q4).

/// Pushes `frame`, steps once with the walking sample and reconciles.
fn reconcile_frame(rig: &mut Rig, frame: StagedFrame) -> Vec<Summary> {
    let present = rig.tick().0 + 1;
    rig.push(frame);
    rig.step(walking(present));
    summaries(&rig.reconcile())
}

/// `frame` repeating the substitution of 13, reconciled in a call of its own.
fn repeat_13(rig: &mut Rig, mut frame: StagedFrame) -> Vec<Summary> {
    frame.substituted.insert(Tick(13));
    reconcile_frame(rig, frame)
}

/// The cart's release into [`OTHER_REGION`].
fn release_cart(frame: &mut StagedFrame) {
    let components = vec![data(at(50, 0)), data(Health(10))];
    frame.spawns.insert(net(2), remote(OTHER_REGION, 1, components));
}

/// q1 under the rule: the substitution of 13 first in the frame of 13, the claim of key 7
/// in the frame of 14, a call of its own, the report repeated there and in the next three
/// frames.
#[test]
fn a_repeated_substitution_keeps_a_claim_an_earlier_call_reconciled() {
    let mut rig = Rig::joined();
    rig.step_to(13, tapping);
    let first = repeat_13(&mut rig, StagedFrame::new(Tick(13)));
    let points: Vec<_> = first.iter().map(|s| (s.0, s.2)).collect();
    assert_eq!(points, vec![(Tick(12), CorrectionCause::Substituted)]);
    let mut frame = StagedFrame::new(Tick(14));
    frame.spawns.insert(net(7), claimed(REGION, vec![data(at(60, 0)), data(Health(1))]));
    let claim_call = repeat_13(&mut rig, frame);
    assert_eq!(claim(rig.world(), key(7)), Some(PLAYER));
    // The repair of 14, due with the first frame at or after it, and the claim at 14: one
    // rollback, from 13. The repeat adds no point at 12.
    let points: Vec<_> = claim_call.iter().map(|s| (s.0, s.2, s.3.clone())).collect();
    assert_eq!(
        points,
        vec![
            (Tick(13), CorrectionCause::Repaired, vec![]),
            (Tick(14), CorrectionCause::Mismatch, vec![key(7)]),
        ]
    );
    for tick in 15..18 {
        let corrections = repeat_13(&mut rig, StagedFrame::new(Tick(tick)));
        assert!(rig.world().contains(key(7)), "frame {tick}: the claimed entity is live");
        assert_eq!(claim(rig.world(), key(7)), Some(PLAYER));
        assert!(corrections.is_empty(), "frame {tick}: a repeat is no rollback");
    }
    assert_eq!(rig.p().stats().substitutions, 1);
}

/// q2 under the rule: the frame of 13 lost, so the substitution of 13 is first in the
/// frame of 14, which releases the cart; the report repeated in the next three frames,
/// each a call of its own, which carry the cart's proxy rows.
#[test]
fn a_repeated_substitution_keeps_a_release_an_earlier_call_reconciled() {
    let mut rig = Rig::joined();
    rig.step_to(14, tapping);
    let mut frame = StagedFrame::new(Tick(14));
    release_cart(&mut frame);
    let release_call = repeat_13(&mut rig, frame);
    let points: Vec<_> = release_call.iter().map(|s| (s.0, s.2)).collect();
    assert_eq!(
        points,
        vec![(Tick(12), CorrectionCause::Substituted), (Tick(14), CorrectionCause::Mismatch),]
    );
    assert_eq!(claim(rig.world(), cart()), None);
    assert_eq!(rig.world().region_of(cart()), Some(OTHER_REGION));
    for tick in 15..18 {
        let x = 50 + i32::try_from(tick).expect("small");
        let mut frame = StagedFrame::new(Tick(tick));
        dequantised(&mut frame, cart(), &at(x, 0));
        let corrections = repeat_13(&mut rig, frame);
        assert_eq!(claim(rig.world(), cart()), None, "frame {tick}: the release holds");
        assert_eq!(rig.world().region_of(cart()), Some(OTHER_REGION));
        assert_eq!(rig.position(cart()), Some(at(x, 0)), "frame {tick}: the proxy row applies");
        assert!(corrections.is_empty(), "frame {tick}: a repeat is no rollback");
    }
    assert_eq!(rig.p().stats().substitutions, 1);
}

/// q4, the reviewer's shape: the substitution of 13 first in the frame of 13, a call of
/// its own; the release in the frame of 14 with the report repeated, another call; the
/// report repeated once more in the frame of 15, then the cart's proxy rows alone.
#[test]
fn a_resent_substitution_after_a_release_in_its_own_call_keeps_the_release() {
    let mut rig = Rig::joined();
    rig.step_to(13, tapping);
    assert_eq!(repeat_13(&mut rig, StagedFrame::new(Tick(13))).len(), 1);
    let mut frame = StagedFrame::new(Tick(14));
    release_cart(&mut frame);
    let release_call = repeat_13(&mut rig, frame);
    let points: Vec<_> = release_call.iter().map(|s| (s.0, s.2)).collect();
    assert_eq!(
        points,
        vec![(Tick(13), CorrectionCause::Repaired), (Tick(14), CorrectionCause::Mismatch),]
    );
    assert_eq!(claim(rig.world(), cart()), None);
    for tick in 15..20 {
        let x = 50 + i32::try_from(tick).expect("small");
        let mut frame = StagedFrame::new(Tick(tick));
        dequantised(&mut frame, cart(), &at(x, 0));
        let corrections =
            if tick == 15 { repeat_13(&mut rig, frame) } else { reconcile_frame(&mut rig, frame) };
        assert_eq!(claim(rig.world(), cart()), None, "frame {tick}: the release holds");
        assert_eq!(rig.world().region_of(cart()), Some(OTHER_REGION));
        assert_eq!(rig.position(cart()), Some(at(x, 0)), "frame {tick}: the proxy row applies");
        assert!(corrections.is_empty(), "frame {tick}: nothing to correct");
    }
    assert_eq!(rig.p().stats().substitutions, 1);
}

/// A report repeated in three frames of one call and again in the next call is one
/// substitution, one rollback and one repair.
#[test]
fn a_substitution_repeated_within_and_across_calls_counts_once() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, sample);
    reference.step_to(22, applied);
    rig.push(substituted(14, &[14]));
    rig.push(substituted(15, &[14]));
    rig.push(substituted(16, &[14]));
    rig.step(sample(21));
    let reconciled = rig.reconcile();
    let points: Vec<_> = reconciled.corrections.iter().map(|c| (c.tick, c.cause)).collect();
    // The substitution of 14 at 13 and the repair of 15 at 14, from the frame of 15.
    assert_eq!(points, vec![(Tick(13), CorrectionCause::Substituted)]);
    assert_eq!(rig.p().stats().substitutions, 1);
    rig.push(substituted(17, &[14]));
    rig.step(sample(22));
    let repeated = rig.reconcile();
    assert!(repeated.corrections.is_empty(), "a repeat in a later call is no rollback");
    assert_eq!(repeated.resync, None);
    assert_eq!(rig.p().stats().substitutions, 1);
    assert_eq!(rig.hash(), reference.hash());
}

/// A repeat is known by the ticks the predictor acted on, so one whose previous tick has
/// left the ring is skipped, not `TooOld`.
#[test]
fn a_repeat_whose_previous_tick_left_the_ring_is_skipped() {
    let mut rig = Rig::joined();
    rig.p_mut().set_depth(6).unwrap();
    rig.step_to(20, jump_at_18);
    // The frame of 18 lost: the substitution of 18 first in the frame of 19, with the
    // repair of 19.
    rig.push(substituted(19, &[18]));
    rig.step(sample(21));
    let first = rig.reconcile();
    let points: Vec<_> = first.corrections.iter().map(|c| (c.tick, c.cause)).collect();
    assert_eq!(points, vec![(Tick(17), CorrectionCause::Substituted)]);
    rig.step(sample(22));
    rig.push(substituted(22, &[18]));
    rig.step(sample(23));
    assert_eq!(rig.p().ring().oldest(), Some(Tick(18)));
    let hash = rig.hash();
    let repeated = rig.reconcile();
    assert_eq!(repeated.resync, None);
    assert!(repeated.corrections.is_empty());
    assert_eq!(rig.hash(), hash);
    assert_eq!(rig.p().stats().substitutions, 1);
}

/// What the server applied when it lost 14 and 15: 15's substitute continues 14's.
fn lost_14_15(tick: u32) -> PlayerInput {
    match tick {
        14 | 15 => sample(13).substitute(),
        16 => sample(16).after_substitute(&sample(13).substitute()),
        _ => sample(tick),
    }
}

fn points(reconciled: &Reconciled) -> Vec<(Tick, CorrectionCause)> {
    reconciled.corrections.iter().map(|c| (c.tick, c.cause)).collect()
}

/// The receiver's split under the rule: the frame of 14 reports 14; the frame of 15 holds
/// region changes alone (an empty frame here: predict reads none of them) and makes the
/// repair of 15 due; the frame of 16, the first that may carry 15, reports 14 again and 15
/// for the first time. A repaired tick is no repeat. Each frame a call of its own.
#[test]
fn a_repaired_tick_first_reported_after_a_split_frame_is_substituted_across_calls() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, sample);
    reference.step_to(23, lost_14_15);
    rig.push(substituted(14, &[14]));
    rig.step(sample(21));
    let first = rig.reconcile();
    rig.push(StagedFrame::new(Tick(15)));
    rig.step(sample(22));
    let split = rig.reconcile();
    rig.push(substituted(16, &[14, 15]));
    rig.step(sample(23));
    let second = rig.reconcile();
    assert_eq!(points(&first), vec![(Tick(13), CorrectionCause::Substituted)]);
    assert_eq!(points(&split), vec![(Tick(14), CorrectionCause::Repaired)]);
    assert_eq!(points(&second), vec![(Tick(14), CorrectionCause::Substituted)]);
    assert_eq!(second.resync, None);
    assert_eq!(rig.p().stats().substitutions, 2);
    assert_eq!(rig.hash(), reference.hash());
}

/// The same three frames in one call.
#[test]
fn a_repaired_tick_first_reported_after_a_split_frame_is_substituted_in_one_call() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, sample);
    reference.step_to(21, lost_14_15);
    rig.push(substituted(14, &[14]));
    rig.push(StagedFrame::new(Tick(15)));
    rig.push(substituted(16, &[14, 15]));
    rig.step(sample(21));
    let reconciled = rig.reconcile();
    assert_eq!(points(&reconciled), vec![(Tick(13), CorrectionCause::Substituted)]);
    assert_eq!(rig.p().stats().substitutions, 2);
    assert_eq!(rig.hash(), reference.hash());
}

/// A repeat after the slot of its own tick left the ring is skipped, not `TooOld`. The
/// repair of 19 is due with the frame of 19, in a call of its own: a repair due only once
/// its previous slot has left the ring is `TooOld`, as a first report is.
#[test]
fn a_repeat_after_its_own_slot_left_the_ring_is_skipped() {
    let mut rig = Rig::joined();
    rig.p_mut().set_depth(6).unwrap();
    rig.step_to(20, jump_at_18);
    rig.push(substituted(18, &[18]));
    rig.step(sample(21));
    assert_eq!(points(&rig.reconcile()), vec![(Tick(17), CorrectionCause::Substituted)]);
    rig.push(substituted(19, &[18]));
    rig.step(sample(22));
    assert_eq!(points(&rig.reconcile()), vec![(Tick(18), CorrectionCause::Repaired)]);
    rig.step(sample(23));
    rig.push(substituted(23, &[18]));
    rig.step(sample(24));
    assert_eq!(rig.p().ring().oldest(), Some(Tick(19)));
    let hash = rig.hash();
    let repeated = rig.reconcile();
    assert_eq!(repeated.resync, None);
    assert!(repeated.corrections.is_empty());
    assert_eq!(rig.hash(), hash);
    assert_eq!(rig.p().stats().substitutions, 1);
}

/// A full frame between the first report and its repeats: the repeats are at or before
/// the restart, which is remembered after its slot has left the ring.
#[test]
fn a_repeat_after_a_full_frame_whose_restart_left_the_ring_is_skipped() {
    let mut rig = Rig::joined();
    rig.p_mut().set_depth(6).unwrap();
    rig.step_to(20, jump_at_18);
    rig.push(substituted(18, &[18]));
    rig.step(sample(21));
    assert_eq!(points(&rig.reconcile()), vec![(Tick(17), CorrectionCause::Substituted)]);
    rig.push(join_frame(Tick(19)));
    rig.step(sample(22));
    assert!(rig.reconcile().corrections.is_empty());
    assert_eq!(rig.p().ring().rebuilt_at(), Some(Tick(22)));
    rig.push(substituted(20, &[18]));
    rig.step(sample(23));
    let held = rig.reconcile();
    assert_eq!(held.resync, None);
    assert!(held.corrections.is_empty());
    rig.step_to(28, sample);
    rig.push(substituted(27, &[18]));
    rig.step(sample(29));
    assert_eq!(rig.p().ring().oldest(), Some(Tick(24)));
    assert_eq!(rig.p().ring().rebuilt_at(), None);
    let late = rig.reconcile();
    assert_eq!(late.resync, None);
    assert!(late.corrections.is_empty());
    assert_eq!(rig.p().stats().substitutions, 1);
}

/// Outside the rule: 15 first, then 14 first in a later frame. The repair of 15 that 14
/// arms falls on a tick already substituted, so it substitutes 15 again, continuing 14's
/// substitute, and arms the repair of 16; the next repeat of 15 is skipped.
#[test]
fn an_older_substitution_first_reported_after_a_newer_one_substitutes_the_newer_again() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, sample);
    reference.step_to(23, lost_14_15);
    rig.push(substituted(15, &[15]));
    rig.step(sample(21));
    let first = rig.reconcile();
    rig.push(substituted(16, &[14, 15]));
    rig.step(sample(22));
    let older = rig.reconcile();
    rig.push(substituted(17, &[15]));
    rig.step(sample(23));
    let repeated = rig.reconcile();
    assert_eq!(points(&first), vec![(Tick(14), CorrectionCause::Substituted)]);
    assert_eq!(points(&older), vec![(Tick(13), CorrectionCause::Substituted)]);
    assert!(repeated.corrections.is_empty());
    assert_eq!(repeated.resync, None);
    assert_eq!(rig.p().stats().substitutions, 2);
    assert_eq!(rig.hash(), reference.hash());
}

/// The same order with the repair of 16 made in a call between, where it changes no byte
/// (16 holds the jump that 15's first substitute holds) and rolls nothing back: the
/// re-substitution of 15 arms the repair of 16 again, which now continues the new
/// substitute, presses the jump and is acted on.
#[test]
fn a_resubstitution_repairs_the_tick_after_it_again() {
    let mut rig = Rig::joined();
    let mut reference = Rig::joined();
    rig.step_to(20, sample);
    reference.step_to(23, lost_14_15);
    rig.push(substituted(15, &[15]));
    rig.step(sample(21));
    let first = rig.reconcile();
    rig.push(StagedFrame::new(Tick(16)));
    rig.step(sample(22));
    let repair = rig.reconcile();
    rig.push(substituted(17, &[14, 15]));
    rig.step(sample(23));
    let older = rig.reconcile();
    assert_eq!(points(&first), vec![(Tick(14), CorrectionCause::Substituted)]);
    assert!(points(&repair).is_empty());
    assert_eq!(repair.resimulated, 0);
    assert_eq!(points(&older), vec![(Tick(13), CorrectionCause::Substituted)]);
    assert_eq!(rig.p().stats().substitutions, 2);
    assert_eq!(rig.hash(), reference.hash());
}

/// A substitution at or before a resync's restart is skipped, a repeat of one acted on
/// before the resync included, and still after the restart's slot has left the ring.
#[test]
fn a_substitution_at_or_before_a_resync_is_skipped() {
    let mut rig = Rig::joined();
    rig.p_mut().set_depth(6).unwrap();
    rig.step_to(20, jump_at_18);
    rig.push(substituted(19, &[18]));
    rig.step(sample(21));
    assert_eq!(points(&rig.reconcile()), vec![(Tick(17), CorrectionCause::Substituted)]);
    rig.p_mut().resync(Tick(30)).unwrap();
    rig.step(sample(31));
    rig.push(substituted(31, &[18, 30]));
    rig.step(sample(32));
    let held = rig.reconcile();
    assert_eq!(held.resync, None);
    assert!(held.corrections.is_empty());
    rig.step_to(39, sample);
    assert_eq!(rig.p().ring().oldest(), Some(Tick(34)));
    rig.push(substituted(38, &[18, 30]));
    rig.step(sample(40));
    let late = rig.reconcile();
    assert_eq!(late.resync, None);
    assert!(late.corrections.is_empty());
    assert_eq!(rig.p().stats().substitutions, 1);
}

/// How late a repeat may come at the default depth: while the ring holds its tick's slot
/// or for ten seconds of ticks after it, whichever is longer, it is skipped; later, once
/// its slot has left the ring, it reads as a first report and is `TooOld`.
#[test]
fn a_repeat_is_skipped_for_ten_seconds_after_its_tick() {
    let window = 10 * TICK_RATE_HZ;
    for lag in [30, 58, 59, 60, 61, 62, 70, window - 1, window, window + 1] {
        let mut rig = Rig::joined();
        rig.step_to(20, jump_at_18);
        rig.push(substituted(18, &[18]));
        rig.step(sample(21));
        rig.reconcile();
        rig.push(StagedFrame::new(Tick(19)));
        rig.step(sample(22));
        rig.reconcile();
        // The repeat in the frame of 16 + lag, popped at 18 + lag.
        rig.step_to(18 + lag - 1, sample);
        rig.push(substituted(16 + lag, &[18]));
        rig.step(sample(18 + lag));
        let repeated = rig.reconcile();
        if lag <= window {
            assert_eq!(repeated.resync, None, "a repeat popped {lag} ticks after its tick");
            assert!(repeated.corrections.is_empty());
        } else {
            let oldest = rig.p().ring().oldest().expect("a ring");
            assert_eq!(
                repeated.resync,
                Some(ResyncReason::TooOld { frame: Tick(16 + lag), oldest }),
                "a repeat popped {lag} ticks after its tick"
            );
        }
        assert_eq!(rig.p().stats().substitutions, 1);
    }
}

/// A depth above the window: a repeat whose slot the ring still holds, popped later than
/// the window after its tick, is skipped, not rolled back again.
#[test]
fn a_repeat_is_skipped_while_the_ring_holds_its_slot_past_the_window() {
    let window = 10 * TICK_RATE_HZ;
    let mut rig = Rig::joined();
    rig.p_mut().set_depth(window + 100).unwrap();
    rig.step_to(20, jump_at_18);
    rig.push(substituted(18, &[18]));
    rig.step(sample(21));
    rig.reconcile();
    rig.push(StagedFrame::new(Tick(19)));
    rig.step(sample(22));
    rig.reconcile();
    let lag = window + 50;
    rig.step_to(18 + lag - 1, sample);
    rig.push(substituted(16 + lag, &[18]));
    rig.step(sample(18 + lag));
    assert_eq!(rig.p().ring().oldest(), Some(Tick(11)));
    let repeated = rig.reconcile();
    assert_eq!(repeated.resync, None);
    assert!(repeated.corrections.is_empty(), "a repeat is no rollback");
    assert_eq!(rig.p().stats().substitutions, 1);
}
