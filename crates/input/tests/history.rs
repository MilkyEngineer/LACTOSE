//! The redundancy window (README, "Redundancy and substitution"): the client's `insert`,
//! the server's `insert_bounded`, and both across the `u32` wrap of the tick.

#![forbid(unsafe_code)]

mod common;

use common::Rng;
use lactose_core::Tick;
use lactose_input::{Axis2, PlayerInput, REDUNDANT_SAMPLES, SampleHistory, Yaw};

const CAPACITY: u32 = SampleHistory::CAPACITY as u32;

/// A sample that names its tick, so the window's contents can be read back.
fn sample(tick: u32) -> PlayerInput {
    PlayerInput {
        yaw: Yaw(tick as u16),
        movement: Axis2 { x: 1, y: (tick >> 16) as i16 },
        ..PlayerInput::IDLE
    }
}

fn ticks(history: &SampleHistory) -> Vec<u32> {
    history.iter().map(|(tick, _)| tick.0).collect()
}

/// Every yielded sample is the one inserted for its tick.
fn consistent(history: &SampleHistory) -> bool {
    history.iter().all(|(tick, input)| *input == sample(tick.0) && history.get(tick) == Some(input))
}

/// The README's rows, from a starting tick: run at zero and across the wrap.
fn the_rows_from(base: u32) {
    let t = |offset: u32| base.wrapping_add(offset);
    let mut history = SampleHistory::new();
    assert!(history.is_empty() && history.newest().is_none() && history.iter().next().is_none());
    // Eight ticks in order, then a ninth: the first is gone.
    for offset in 0..CAPACITY {
        assert!(history.insert(Tick(t(offset)), sample(t(offset))));
    }
    assert_eq!(history.len(), REDUNDANT_SAMPLES);
    assert_eq!(ticks(&history), (0..CAPACITY).map(t).collect::<Vec<_>>());
    assert!(history.insert(Tick(t(CAPACITY)), sample(t(CAPACITY))));
    assert_eq!(history.get(Tick(t(0))), None);
    assert_eq!(ticks(&history), (1..=CAPACITY).map(t).collect::<Vec<_>>());
    assert_eq!(history.newest(), Some((Tick(t(CAPACITY)), &sample(t(CAPACITY)))));
    assert!(consistent(&history));
    // A duplicate returns `false` and keeps the first.
    assert!(!history.insert(Tick(t(5)), PlayerInput::IDLE));
    assert_eq!(history.get(Tick(t(5))), Some(&sample(t(5))));
    // A tick older than the window is ignored.
    assert!(!history.insert(Tick(t(0)), sample(t(0))));
    assert!(!history.insert(Tick(t(0).wrapping_sub(1000)), sample(0)));
    assert_eq!(ticks(&history), (1..=CAPACITY).map(t).collect::<Vec<_>>());

    // Out-of-order inserts: the window moves to the newest, the gaps fill later, and `iter`
    // is ascending throughout.
    let mut history = SampleHistory::new();
    for offset in [20, 17, 19, 14, 13, 18, 12, 16, 15] {
        let fresh = history.insert(Tick(t(offset)), sample(t(offset)));
        // 12 is older than the window `13..=20` once 20 is the newest.
        assert_eq!(fresh, offset != 12, "offset {offset}");
        let held = ticks(&history);
        assert!(held.windows(2).all(|pair| pair[1].wrapping_sub(pair[0]) as i32 > 0), "{held:?}");
        assert!(consistent(&history));
    }
    assert_eq!(ticks(&history), (13..=20).map(t).collect::<Vec<_>>());
    // A jump of more than the window empties it but the newest.
    assert!(history.insert(Tick(t(100)), sample(t(100))));
    assert_eq!(ticks(&history), vec![t(100)]);
    assert_eq!(history.len(), 1);

    // The server's merge: only `(consumed, consumed + CAPACITY]`.
    let consumed = Tick(t(50));
    let mut history = SampleHistory::new();
    assert!(!history.insert_bounded(consumed, consumed, sample(t(50))));
    assert!(!history.insert_bounded(consumed, Tick(t(50 + CAPACITY + 1)), sample(t(59))));
    assert!(!history.insert_bounded(consumed, Tick(t(49)), sample(t(49))));
    assert!(history.is_empty());
    assert!(history.insert_bounded(consumed, Tick(t(51)), sample(t(51))));
    assert!(history.insert_bounded(consumed, Tick(t(50 + CAPACITY)), sample(t(50 + CAPACITY))));
    assert!(!history.insert_bounded(consumed, Tick(t(51)), PlayerInput::IDLE));
    // A forged tick far ahead of the newest is refused and moves nothing.
    let newest = history.newest().map(|(tick, _)| tick).unwrap();
    assert!(!history.insert_bounded(consumed, Tick(newest.0.wrapping_add(1 << 30)), sample(0)));
    assert!(!history.insert_bounded(consumed, Tick(newest.0.wrapping_add(1 << 31)), sample(0)));
    assert_eq!(ticks(&history), vec![t(51), t(50 + CAPACITY)]);
    assert!(consistent(&history));
    // Every tick still to come can be filled.
    for offset in 52..50 + CAPACITY {
        assert!(history.insert_bounded(consumed, Tick(t(offset)), sample(t(offset))));
    }
    assert_eq!(ticks(&history), (51..=50 + CAPACITY).map(t).collect::<Vec<_>>());
}

#[test]
fn the_window_from_tick_zero() {
    the_rows_from(0);
}

#[test]
fn the_window_across_the_wrap() {
    for base in [u32::MAX - 3, u32::MAX - 50, u32::MAX - 8, u32::MAX] {
        the_rows_from(base);
    }
}

/// The window against a model kept as a sorted list, over random inserts around a moving
/// newest tick, crossing the wrap.
#[test]
fn the_window_matches_a_model() {
    let mut rng = Rng::new(0x41_57);
    let mut history = SampleHistory::new();
    let mut model: Vec<u32> = Vec::new();
    let mut head = u32::MAX - 2000;
    for _ in 0..20_000 {
        head = head.wrapping_add(rng.next_u32() % 3);
        let tick = head.wrapping_sub(rng.next_u32() % 12);
        let newest = model.last().copied();
        let ahead = newest.is_none_or(|newest| tick.wrapping_sub(newest) as i32 > 0);
        let expected = if ahead {
            model.retain(|&held| tick.wrapping_sub(held) < CAPACITY);
            model.push(tick);
            true
        } else {
            let age = newest.unwrap().wrapping_sub(tick);
            if age < CAPACITY && !model.contains(&tick) {
                let at = model.iter().position(|&held| (tick.wrapping_sub(held) as i32) < 0);
                model.insert(at.unwrap_or(model.len()), tick);
                true
            } else {
                false
            }
        };
        assert_eq!(history.insert(Tick(tick), sample(tick)), expected, "tick {tick}");
        assert_eq!(ticks(&history), model, "after tick {tick}");
        assert_eq!(history.len(), model.len());
        assert!(consistent(&history));
    }
}
