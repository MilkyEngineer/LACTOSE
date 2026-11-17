//! The render side's sink and the presentation frame agree (D157; `README.md`, "What the sink
//! keeps"): one claimed row in a prediction world, moved one unit a tick, four frames a tick,
//! through every sequence D157's row lists. Each frame runs the passes, publishes and
//! acquires, as `app` does; a sink keeps placements by the README's base rule, once taking
//! the base at the tick's first `begin` and once lazily at a key's first write, and both must
//! hold the frame's pair: the sink's previous and current placements are
//! `PresentFrame::transform`'s, and equal exactly when the frame's `no_interpolate` is set.
//! Neither may draw from a placement of a later tick than the one it draws to.
//!
//! A rollback is the world restored to its own state at the tick, corrected, and its
//! `Teleported` set: a full pass at the tick of the previous one. A resync is the world
//! restored to an earlier tick; a hold is a tick in which the row does not move, after which
//! a pass calls `begin` and `end` alone. Without the base drop, or without the lazy sink's
//! count of drops, exactly the sequences the README's proof row names fail, which the last
//! two tests show.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::common;

use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{LayoutConfig, WorldKind, WorldSnapshot};
use lactose_physics_api::Teleported;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, RenderTransform};

/// What a frame of a sequence does before its passes.
#[derive(Clone, Copy, Debug)]
enum Event {
    /// This many ticks, the last one a teleport to the position if `Some`.
    Step(u32, Option<f64>),
    /// The current tick corrected to the position, with `Teleported` at the tick.
    Rollback(f64, u32),
    /// The world back at the earlier tick, at the position.
    Resync(u32, f64),
    /// This many ticks in which the row does not move.
    Hold(u32),
}

use Event::{Hold, Resync, Rollback, Step};

struct Sequence {
    name: &'static str,
    events: &'static [(u32, Event)],
    frames: u32,
}

/// The sequences of D157's row, as the review's model ran them; a frame a multiple of four
/// with no event of its own steps once.
const SEQUENCES: &[Sequence] = &[
    Sequence {
        name: "S2: corrected by 0.1 in the frame after a tick",
        events: &[(5, Rollback(1.1, 0))],
        frames: 13,
    },
    Sequence {
        name: "S2B: the same, Teleported at the tick",
        events: &[(5, Rollback(1.1, 1))],
        frames: 13,
    },
    Sequence {
        name: "T: a teleport predicted at tick 1, undone by a rollback in the next frame",
        events: &[(4, Step(1, Some(10.0))), (5, Rollback(1.0, 0))],
        frames: 13,
    },
    Sequence {
        name: "T2: a teleport predicted at tick 5, undone by a correction of depth four",
        events: &[(20, Step(1, Some(50.0))), (21, Rollback(5.0, 2))],
        frames: 29,
    },
    Sequence {
        name: "R2: two rollbacks in tick 1's window, the first flagged and the second not",
        events: &[(5, Rollback(1.1, 1)), (6, Rollback(1.2, 0))],
        frames: 13,
    },
    Sequence {
        name: "R2b: two rollbacks, neither flagged",
        events: &[(5, Rollback(1.1, 0)), (6, Rollback(1.2, 0))],
        frames: 13,
    },
    Sequence {
        name: "CU: a catch-up of three ticks, then a rollback",
        events: &[(4, Step(3, None)), (5, Rollback(3.1, 0))],
        frames: 13,
    },
    Sequence {
        name: "CUT: a catch-up of three with a teleport at its last tick, then a rollback that keeps it",
        events: &[(4, Step(3, Some(20.0))), (5, Rollback(20.1, 3))],
        frames: 13,
    },
    Sequence {
        name: "F: rollbacks before the first tick is shown",
        events: &[(1, Rollback(0.1, 0)), (2, Rollback(0.2, 0))],
        frames: 9,
    },
    Sequence {
        name: "RS: a resync to an earlier tick",
        events: &[(9, Resync(1, 1.05))],
        frames: 17,
    },
    Sequence {
        name: "RSR: the resync, then a rollback at its tick in the next frame",
        events: &[(9, Resync(1, 1.05)), (10, Rollback(1.15, 0))],
        frames: 17,
    },
    Sequence {
        name: "LZ: a resync to an earlier tick that changes no row, then a step",
        events: &[(9, Resync(1, 2.0))],
        frames: 17,
    },
    Sequence {
        name: "RS2: two resyncs to earlier ticks, then the ticks between",
        events: &[(17, Resync(3, 3.05)), (21, Resync(1, 1.05))],
        frames: 33,
    },
    Sequence {
        name: "RSS: the row still for two ticks, then a resync to the tick before the newest",
        events: &[(12, Hold(1)), (16, Hold(1)), (17, Resync(3, 2.5))],
        frames: 25,
    },
    Sequence {
        name: "RSS3: the row still for three ticks, then a resync to the tick before the newest",
        events: &[(12, Hold(1)), (16, Hold(1)), (20, Hold(1)), (21, Resync(4, 2.5))],
        frames: 29,
    },
    Sequence {
        name: "RSSR: RSS, then a rollback at the resync's tick in the next frame",
        events: &[(12, Hold(1)), (16, Hold(1)), (17, Resync(3, 2.5)), (18, Rollback(2.6, 0))],
        frames: 25,
    },
];

/// The README's rule with one part taken out, to show that a sequence needs it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mutation {
    None,
    /// The eager sink keeps its bases at a `begin` of an earlier tick.
    NoDrop,
    /// The lazy sink's base counts at the tick it was taken whatever was dropped since.
    LazyTick,
}

/// The sequences that fail without the base drop, as `README.md`'s proof row names them.
const BASE_DROP_FAILS: &[&str] = &["RSR", "RSSR"];
/// The sequences that fail without the lazy sink's count of drops, as the row names them.
const LAZY_COUNT_FAILS: &[&str] = &["RS", "RSR", "LZ", "RS2"];

/// A placement and the tick of the state it shows.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Placement {
    at: RenderTransform,
    tick: Tick,
}

#[derive(Clone, Copy, Debug)]
struct Held {
    previous: Placement,
    current: Placement,
    base: Option<Placement>,
    /// The lazy sink's: the tick the base was taken at, and the drops counted then.
    taken: (Tick, u32),
}

impl Held {
    fn new(placement: Placement, taken: (Tick, u32)) -> Self {
        Self { previous: placement, current: placement, base: None, taken }
    }

    /// A write: from the base to the row, or both at the row.
    fn write(&mut self, placement: Placement, base: Option<Placement>, flag: bool) {
        self.previous = if flag { placement } else { base.unwrap_or(placement) };
        self.current = placement;
    }
}

/// Both sinks of the README, fed the same calls.
#[derive(Default)]
struct Keepers {
    mutation: Option<Mutation>,
    /// The tick of the open pass, and of the `begin` before it.
    tick: Option<Tick>,
    eager: BTreeMap<StableKey, Held>,
    lazy: BTreeMap<StableKey, Held>,
    /// The lazy sink's count of `begin`s that dropped the bases, and the tick of the last.
    drops: u32,
    drop_tick: Option<Tick>,
    written: Vec<StableKey>,
}

fn before(tick: Tick, than: Tick) -> bool {
    tick.0.wrapping_sub(than.0) >= 1 << 31
}

impl Keepers {
    fn begin(&mut self, pass: ExtractPass) {
        let mutation = self.mutation.unwrap_or(Mutation::None);
        let earlier = self.tick.is_some_and(|last| before(pass.tick, last));
        // The eager sink: drop every base at an earlier tick, take each at a new one.
        if earlier && mutation != Mutation::NoDrop {
            self.eager.values_mut().for_each(|held| held.base = None);
        } else if self.tick != Some(pass.tick) {
            self.eager.values_mut().for_each(|held| held.base = Some(held.current));
        }
        // The lazy sink: count the drop.
        if earlier {
            self.drops += 1;
            self.drop_tick = Some(pass.tick);
        }
        self.tick = Some(pass.tick);
    }

    fn write(&mut self, row: &InstanceRow) {
        let tick = self.tick.expect("a pass is open");
        let placement = Placement { at: row.transform, tick };
        let taken = (tick, self.drops);
        match self.eager.get_mut(&row.key) {
            Some(held) => {
                let base = held.base;
                held.write(placement, base, row.no_interpolate);
            }
            None => drop(self.eager.insert(row.key, Held::new(placement, taken))),
        }
        let lazy_tick = self.mutation == Some(Mutation::LazyTick);
        match self.lazy.get_mut(&row.key) {
            Some(held) => {
                let base = if self.drop_tick == Some(tick) {
                    None
                } else if held.taken.0 == tick && (held.taken.1 == self.drops || lazy_tick) {
                    held.base
                } else {
                    held.base = Some(held.current);
                    held.taken = taken;
                    held.base
                };
                held.write(placement, base, row.no_interpolate);
            }
            None => drop(self.lazy.insert(row.key, Held::new(placement, taken))),
        }
        self.written.push(row.key);
    }

    fn end(&mut self, full: bool) {
        // A full pass: what it did not write is gone.
        let written = core::mem::take(&mut self.written);
        if full {
            self.eager.retain(|key, _| written.contains(key));
            self.lazy.retain(|key, _| written.contains(key));
        }
    }
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Keepers>>);

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Keepers> {
        self.0.lock().expect("no test panics while holding the sink")
    }
}

struct KeepingSink {
    keepers: Shared,
    full: bool,
}

impl InstanceSink for KeepingSink {
    fn begin(&mut self, pass: ExtractPass) {
        self.full = pass.full;
        self.keepers.lock().begin(pass);
    }

    fn removed(&mut self, keys: &[StableKey]) {
        let mut keepers = self.keepers.lock();
        for key in keys {
            keepers.eager.remove(key);
            keepers.lazy.remove(key);
        }
    }

    fn write(&self, rows: &[InstanceRow]) {
        let mut keepers = self.keepers.lock();
        rows.iter().for_each(|row| keepers.write(row));
    }

    fn end(&mut self) {
        self.keepers.lock().end(self.full);
    }
}

/// Runs `sequence` and returns every frame at which the sinks and the frame disagree, or one
/// of them draws backward, and how many frames drew a movement.
fn run(sequence: &Sequence, mutation: Mutation) -> (Vec<String>, u32) {
    let row = key(1);
    let simulation = simulation(WorldKind::Prediction, LayoutConfig::default(), Tick(0));
    let mut rig = Rig::new(simulation, executor(2));
    let keepers = Shared::default();
    keepers.lock().mutation = Some(mutation);
    drop(
        rig.state().set_sink(Some(Box::new(KeepingSink { keepers: keepers.clone(), full: false }))),
    );
    let spawned = (at(0.0), mesh(1), Teleported(Tick(0)), Shown(1), claim());
    rig.world_mut().spawn(row, REGION, spawned).unwrap();

    let mut position: BTreeMap<u32, f64> = BTreeMap::from([(0, 0.0)]);
    let mut history: BTreeMap<u32, WorldSnapshot> = BTreeMap::from([(0, rig.world().snapshot())]);
    let mut failures = Vec::new();
    let mut moving = 0;
    for frame in 0..sequence.frames {
        let scripted =
            sequence.events.iter().find(|&&(at, _)| at == frame).map(|&(_, event)| event);
        let event = scripted.or((frame > 0 && frame.is_multiple_of(4)).then_some(Step(1, None)));
        let mut tick = rig.tick().0;
        match event {
            Some(Step(ticks, teleport_to)) => {
                for n in 1..=ticks {
                    tick += 1;
                    let mut x = position[&(tick - 1)] + 1.0;
                    if let Some(to) = teleport_to.filter(|_| n == ticks) {
                        x = to;
                        teleport(rig.world_mut(), row, Tick(tick));
                    }
                    position.insert(tick, x);
                    place(rig.world_mut(), row, x);
                    rig.step();
                    history.insert(tick, rig.world().snapshot());
                }
            }
            Some(Rollback(x, teleported)) => {
                rig.world_mut().restore(&history[&tick]).unwrap();
                position.insert(tick, x);
                place(rig.world_mut(), row, x);
                teleport(rig.world_mut(), row, Tick(teleported));
                history.insert(tick, rig.world().snapshot());
            }
            Some(Hold(ticks)) => {
                for _ in 1..=ticks {
                    tick += 1;
                    let x = position[&(tick - 1)];
                    position.insert(tick, x);
                    rig.step();
                    history.insert(tick, rig.world().snapshot());
                }
            }
            Some(Resync(to, x)) => {
                rig.world_mut().restore(&history[&to]).unwrap();
                history.retain(|&at, _| at <= to);
                position.retain(|&at, _| at <= to);
                position.insert(to, x);
                place(rig.world_mut(), row, x);
                history.insert(to, rig.world().snapshot());
            }
            None => {}
        }
        rig.frame();

        let acquired = rig.reader.acquire().expect("published");
        let pair = acquired.transform(row).expect("a presented row with a transform");
        let current = RenderTransform::from_sim(&pair.current);
        let shown = if pair.no_interpolate {
            (current, current)
        } else {
            (RenderTransform::from_sim(&pair.previous), current)
        };
        moving += u32::from(shown.0 != shown.1);
        let (older, newer) = (acquired.older().tick(), acquired.newer().tick());
        let keepers = keepers.lock();
        let mut problems = Vec::new();
        if !pair.no_interpolate && before(newer, older) && shown.0 != shown.1 {
            problems.push(format!("the frame draws back from {older:?} to {newer:?}"));
        }
        for (name, held) in [("eager", keepers.eager[&row]), ("lazy", keepers.lazy[&row])] {
            if (held.previous.at, held.current.at) != shown {
                problems.push(format!(
                    "the {name} sink holds ({:?}, {:?}), the frame ({:?}, {:?})",
                    held.previous.at.position,
                    held.current.at.position,
                    shown.0.position,
                    shown.1.position
                ));
            }
            if before(held.current.tick, held.previous.tick) && held.previous.at != held.current.at
            {
                problems.push(format!("the {name} sink draws back"));
            }
        }
        if !problems.is_empty() {
            failures.push(format!(
                "frame {frame} (tick {}): {}",
                rig.tick().0,
                problems.join("; ")
            ));
        }
    }
    (failures, moving)
}

#[test]
fn the_sink_and_the_frame_agree_in_every_sequence() {
    for sequence in SEQUENCES {
        let (failures, moving) = run(sequence, Mutation::None);
        assert!(failures.is_empty(), "{}:\n{}", sequence.name, failures.join("\n"));
        // Every sequence draws movements, so the agreement is not of still placements alone.
        assert!(moving >= 4, "{}: {moving} frames moved", sequence.name);
    }
}

/// The short names of the sequences that fail with a part of the rule taken out.
fn failing(mutation: Mutation) -> Vec<&'static str> {
    SEQUENCES
        .iter()
        .filter(|sequence| !run(sequence, mutation).0.is_empty())
        .map(|sequence| sequence.name.split(':').next().expect("a name"))
        .collect()
}

/// Exactly the sequences `README.md`'s proof row names as needing the base drop.
#[test]
fn without_the_base_drop_a_sequence_fails() {
    assert_eq!(failing(Mutation::NoDrop), BASE_DROP_FAILS);
}

/// Exactly the sequences `README.md`'s proof row names as needing the lazy sink's count of
/// drops.
#[test]
fn without_the_lazy_sinks_count_of_drops_a_sequence_fails() {
    assert_eq!(failing(Mutation::LazyTick), LAZY_COUNT_FAILS);
}
