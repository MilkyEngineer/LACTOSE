//! What a frame presents from (`README.md`, "Events and publication" and "A frame's
//! transforms"): each emission in exactly one frame's batch across skipped and taken-back
//! snapshots, folded between acquires, a re-simulated tick's re-emission not delivered again,
//! nothing kept once the reader is gone; and `PresentFrame::transform` for a moved, a
//! teleported, a new and a `Client`-world row, with one-tick pairs and cuts not interpolated.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use crate::common;

use common::*;
use lactose_core::Tick;
use lactose_ecs::{LayoutConfig, PresentEventBatch, PresentEventKey, WorldKind};
use lactose_math::Scalar;
use lactose_present_api::{AudioRequest, extract_state, snapshot_channel};
use lactose_sim::TickInput;

/// A prediction world with three chirping rows, each a presented row.
fn chirping() -> Rig {
    let simulation = simulation(WorldKind::Prediction, LayoutConfig::default(), Tick(10));
    let mut rig = Rig::new(simulation, executor(2));
    for n in 1..=3 {
        let row = (at(0.0), Chirper(0), Shown(0), claim(), Walker(Scalar::ONE));
        rig.world_mut().spawn(key(n), REGION, row).unwrap();
    }
    rig
}

/// A count per key; the key is not `Ord` and the test's maps are small.
#[derive(Default)]
struct Counts(Vec<(PresentEventKey, i32)>);

impl Counts {
    fn add(&mut self, key: PresentEventKey, by: i32) {
        match self.0.iter_mut().find(|(seen, _)| *seen == key) {
            Some((_, count)) => *count += by,
            None => self.0.push((key, by)),
        }
    }

    /// The keys whose count is not zero.
    fn net(&self) -> Vec<(PresentEventKey, i32)> {
        self.0.iter().copied().filter(|&(_, count)| count != 0).collect()
    }
}

/// What the test published and what the reader got, by key: emissions minus cancellations.
#[derive(Default)]
struct Ledger {
    published: Counts,
    delivered: Counts,
    /// Keys a frame's batch emitted, to find one emitted in two frames.
    emitted: Vec<PresentEventKey>,
    cancelled: Vec<PresentEventKey>,
}

impl Ledger {
    fn publish(&mut self, rig: &mut Rig) {
        let events = rig.world_mut().take_present_events();
        for emission in &events.emitted {
            self.published.add(emission.key, 1);
        }
        for key in &events.cancelled {
            self.published.add(*key, -1);
        }
        rig.state().publish(events);
    }

    fn acquire(&mut self, rig: &mut Rig) {
        let Some(frame) = rig.reader.acquire() else { return };
        for emission in &frame.events().emitted {
            assert!(emission.get::<AudioRequest>().is_some());
            self.delivered.add(emission.key, 1);
            assert!(!self.emitted.contains(&emission.key), "{:?} in two frames", emission.key);
            self.emitted.push(emission.key);
        }
        for key in &frame.events().cancelled {
            self.delivered.add(*key, -1);
            assert!(!self.cancelled.contains(key), "{key:?} cancelled in two frames");
            self.cancelled.push(*key);
        }
    }
}

/// Under every cadence, with rollbacks that re-simulate two ticks, once re-emitting and
/// once with a row no longer chirping: no key is emitted, or cancelled, in two frames, and
/// what the reader got of each key nets to what was published. A cancellation in the batch
/// after its emission, before an acquire, leaves neither.
#[test]
fn each_emission_reaches_one_frame() {
    // Per cycle of four: whether the reader acquires after the publication, inside the pass,
    // or not at all.
    let cadences: [&[u8]; 4] = [&[1, 1, 1, 1], &[1, 0, 1, 0], &[2, 2, 2, 2], &[0, 0, 0, 0]];
    for (index, cadence) in cadences.iter().enumerate() {
        let mut rig = chirping();
        let mut ledger = Ledger::default();
        let mut saved = BTreeMap::new();
        let mut rollbacks = 0;
        for frame in 0..48u32 {
            if frame % 8 == 7 {
                // A rollback of two ticks; the second time, row 1 stops chirping.
                let tick = rig.tick().0;
                rig.world_mut().restore(&saved[&(tick - 2)]).unwrap();
                if rollbacks % 2 == 1 {
                    rig.world_mut().remove::<Chirper>(key(1)).unwrap();
                }
                rollbacks += 1;
                rig.step();
                rig.step();
            } else {
                rig.step();
            }
            saved.insert(rig.tick().0, rig.world().snapshot());
            let when = cadence[frame as usize % cadence.len()];
            rig.passes();
            rig.recording.take();
            if when == 2 {
                ledger.acquire(&mut rig);
            }
            ledger.publish(&mut rig);
            if when == 1 {
                ledger.acquire(&mut rig);
            }
        }
        ledger.acquire(&mut rig);
        let (delivered, published) = (ledger.delivered.net(), ledger.published.net());
        assert_eq!(delivered.len(), published.len(), "cadence {index}");
        assert!(published.iter().all(|entry| delivered.contains(entry)), "cadence {index}");
        assert!(ledger.published.0.iter().all(|&(_, count)| count == 0 || count == 1));
        assert!(ledger.emitted.len() > 80, "cadence {index}: {}", ledger.emitted.len());
        assert!(!ledger.cancelled.is_empty() || index == 3, "cadence {index}: cancellations");
    }
}

/// An emission and its cancellation published between two acquires are in neither list; the
/// re-emission of a re-simulated tick is not delivered again.
#[test]
fn a_cancellation_folds_into_its_emission() {
    let mut rig = chirping();
    rig.step();
    let saved = rig.world().snapshot();
    rig.step();
    rig.frame();
    let first = rig.reader.acquire().unwrap().events().emitted.len();
    assert_eq!(first, 6, "two ticks of three chirpers");

    // Ticks 12 and 13 again without row 1's chirp. Tick 12's emission of row 1 was delivered:
    // its cancellation is in this frame. Tick 13's was published and cancelled since the last
    // acquire: in neither list. Rows 2 and 3 re-emit tick 12, delivered before and not again,
    // and tick 13, published once and delivered once, here.
    rig.step();
    rig.frame();
    rig.world_mut().restore(&saved).unwrap();
    rig.world_mut().remove::<Chirper>(key(1)).unwrap();
    rig.step();
    rig.step();
    rig.frame();
    let frame = rig.reader.acquire().unwrap();
    let events: &PresentEventBatch = frame.events();
    let cancelled: Vec<_> = events.cancelled.iter().map(|key| (key.origin, key.tick)).collect();
    assert_eq!(cancelled, [(key(1), Tick(12))], "{events:?}");
    let mut emitted: Vec<_> =
        events.emitted.iter().map(|emission| (emission.key.origin, emission.key.tick)).collect();
    emitted.sort_unstable();
    assert_eq!(emitted, [(key(2), Tick(13)), (key(3), Tick(13))], "{events:?}");
    // Nothing published since: an empty batch.
    assert!(rig.reader.acquire().unwrap().events().emitted.is_empty());
}

/// With the reader dropped, `publish` keeps no event: what the writer holds says so.
#[test]
fn with_the_reader_gone_publish_keeps_no_event() {
    let mut rig = chirping();
    let (writer, reader) = snapshot_channel();
    drop(rig.state().set_snapshot(Some(writer)));
    rig.step();
    rig.frame();
    let shown = |rig: &mut Rig| {
        let writer = rig.state().set_snapshot(None).expect("installed");
        let shown = format!("{writer:?}");
        drop(rig.state().set_snapshot(Some(writer)));
        shown
    };
    assert!(!shown(&mut rig).contains("emitted: []"), "kept for the reader");
    drop(reader);
    rig.step();
    rig.frame();
    rig.step();
    rig.frame();
    assert!(shown(&mut rig).contains("events: [], publications"));
}

/// A writer installed between runs, after the passes and before `publish`, as `app` does for
/// a world it starts to show: the events of that publication have no snapshot with them, an
/// acquire then returns `None`, and they reach the first frame (review round 1, MINOR-2).
#[test]
fn events_published_before_the_first_snapshot_reach_the_first_frame() {
    let executor = executor(1);
    let mut simulation = simulation(WorldKind::Prediction, LayoutConfig::default(), Tick(10));
    let row = (at(0.0), mesh(1), Chirper(1), Shown(1), claim());
    simulation.world_mut().spawn(key(1), REGION, row).unwrap();
    let run = simulation.step(&executor, TickInput::default());
    let simulation = executor.block_on(run).unwrap();
    let run = simulation.run_passes(&executor);
    let mut simulation = executor.block_on(run).unwrap();

    let (writer, mut reader) = snapshot_channel();
    drop(extract_state(&mut simulation).unwrap().set_snapshot(Some(writer)));
    let events = simulation.world_mut().take_present_events();
    let emitted = events.emitted.len();
    assert!(emitted > 0, "the chirper emitted");
    extract_state(&mut simulation).unwrap().publish(events);
    assert!(reader.acquire().is_none(), "no snapshot was published");

    let run = simulation.run_passes(&executor);
    let mut simulation = executor.block_on(run).unwrap();
    let events = simulation.world_mut().take_present_events();
    extract_state(&mut simulation).unwrap().publish(events);
    let frame = reader.acquire().expect("published");
    assert_eq!(frame.events().emitted.len(), emitted, "every emission reaches the first frame");
}

/// `transform`: a moved row interpolates from the older snapshot; a row teleported after the
/// older snapshot's tick, a row the older snapshot does not hold, a pair of one tick and a cut
/// do not; a `Client` world's snapshot holds no transform.
#[test]
fn a_frames_transforms() {
    let mut rig =
        Rig::new(simulation(WorldKind::Server, LayoutConfig::default(), Tick(10)), executor(2));
    for n in 1..=2 {
        rig.world_mut().spawn(key(n), REGION, (at(0.0), Shown(0), Walker(Scalar::ONE))).unwrap();
    }
    rig.frame();
    let frame = rig.reader.acquire().unwrap();
    let pair = frame.transform(key(1)).unwrap();
    assert!(pair.no_interpolate, "the first publication is published alone");
    assert_eq!(pair.previous, pair.current);

    teleport(rig.world_mut(), key(2), Tick(11));
    rig.world_mut().spawn(key(3), REGION, (at(5.0), Shown(0))).unwrap();
    rig.tick_frame();
    let frame = rig.reader.acquire().unwrap();
    let moved = frame.transform(key(1)).unwrap();
    assert_eq!((moved.previous, moved.current, moved.no_interpolate), (at(0.0), at(1.0), false));
    let teleported = frame.transform(key(2)).unwrap();
    assert_eq!((teleported.previous, teleported.current), (at(0.0), at(1.0)));
    assert!(teleported.no_interpolate, "teleported after the older snapshot's tick");
    let new = frame.transform(key(3)).unwrap();
    assert_eq!((new.previous, new.current, new.no_interpolate), (at(5.0), at(5.0), true));
    assert_eq!(frame.transform(key(9)), None);

    rig.tick_frame();
    let frame = rig.reader.acquire().unwrap();
    assert!(!frame.transform(key(2)).unwrap().no_interpolate, "the teleport is in the past");

    // An edit between ticks: a pair of one tick.
    rig.world_mut().get_mut::<Shown>(key(1)).unwrap().0 = 1;
    rig.frame();
    let frame = rig.reader.acquire().unwrap();
    assert_eq!(frame.older().tick(), frame.newer().tick());
    assert!(frame.transform(key(1)).unwrap().no_interpolate);

    // A cut: a full pass in a `Server` world.
    rig.tick_frame();
    let recording = Recording::default();
    drop(rig.state().set_sink(Some(recording.sink())));
    rig.tick_frame();
    let frame = rig.reader.acquire().unwrap();
    assert!(frame.newer().cut());
    assert_ne!(frame.older().tick(), frame.newer().tick());
    assert!(frame.transform(key(1)).unwrap().no_interpolate);

    let mut client =
        Rig::new(simulation(WorldKind::Client, LayoutConfig::default(), Tick(10)), executor(2));
    client.world_mut().spawn(key(1), REGION, (at(0.0), Shown(0))).unwrap();
    client.frame();
    let frame = client.reader.acquire().unwrap();
    assert_eq!(frame.newer().get::<Shown>(key(1)), Some(Shown(0)));
    assert_eq!(frame.transform(key(1)), None, "a client world's snapshot holds no transform");
}
