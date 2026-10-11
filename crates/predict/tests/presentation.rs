//! Presentation across a rollback: the events of a re-simulated tick are delivered once,
//! the ones the re-simulation no longer emits are cancelled, and extract's next pass is a
//! full one.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::Tick;
use lactose_ecs::{PresentEvent, PresentEventKey};
use lactose_input::PlayerInput;
use lactose_net_api::StagedFrame;

/// A jump pressed at 14, held to 16; 14 is lost on the way to the server.
fn sample(tick: u32) -> PlayerInput {
    match tick {
        14 => jumping(0, true),
        15 | 16 => jumping(0, false),
        _ => moving(1),
    }
}

fn sound(tick: u32, seq: u32) -> PresentEventKey {
    PresentEventKey { origin: me(), tick: Tick(tick), kind: Sound::KIND, seq }
}

#[test]
fn a_rollback_delivers_new_events_once_and_cancels_the_ones_not_emitted_again() {
    let mut rig = Rig::joined();
    let _ = rig.world_mut().take_present_events();
    rig.step_to(20, sample);
    let batch = rig.world_mut().take_present_events();
    let keys: Vec<PresentEventKey> = batch.emitted.iter().map(|emission| emission.key).collect();
    // A footstep a tick from 12, and the jump at 14.
    assert_eq!(keys.len(), 9 + 1);
    assert!(keys.contains(&sound(14, 1)));
    assert!(batch.cancelled.is_empty());

    let mut frame = StagedFrame::new(Tick(16));
    frame.substituted.insert(Tick(14));
    rig.push(frame);
    rig.step(sample(21));
    assert_eq!(rig.reconcile().corrections[0].tick, Tick(13));
    let batch = rig.world_mut().take_present_events();
    let keys: Vec<PresentEventKey> = batch.emitted.iter().map(|emission| emission.key).collect();
    // The jump moved to 15, where the repaired sample presses it; the footsteps of 13 to 20
    // re-emitted are not delivered again, 21's is new.
    assert_eq!(keys, vec![sound(15, 1), sound(21, 0)]);
    assert_eq!(batch.cancelled, vec![sound(14, 1)]);
}

#[test]
fn extracts_pass_after_a_rollback_is_full() {
    let mut rig = Rig::joined();
    rig.step_to(14, walking);
    rig.passes();
    rig.step(walking(15));
    rig.passes();
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(30, 0));
    rig.push(frame);
    rig.step(walking(16));
    assert_eq!(rig.reconcile().corrections.len(), 1);
    rig.passes();
    let seen = rig.world().service::<Seen>().unwrap().0.lock().unwrap().clone();
    // Every claimed row (the prediction world's systems run over those, so the proxy is
    // not seen) the first time; the player's alone after a predicted tick; every claimed
    // row after the restore's new epoch.
    assert_eq!(seen, vec![2, 1, 2]);
}
