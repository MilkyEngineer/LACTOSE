//! The triple buffer (`README.md`, "The triple buffer" and "Events and publication"; D157): a
//! property test over sequences of write, publish and acquire against a model of the README's
//! rules, under the five cadences of D157's row and at random.
//!
//! Every write is a real pass of a world with one presented row whose `Shown` holds the
//! write's number, so an acquired snapshot says which write it holds: a pass after a tick, a
//! full pass at the same tick (a load's `set_tick`: a cut in a `Server` world, a rollback that
//! takes the place of the snapshot published last in a `Prediction` world), a full pass at an
//! earlier tick, and an idle pass after an edit. The writer's `Arc::get_mut` asserts in the
//! crate that the buffer a pass writes is shared with nobody, and so that no held snapshot is
//! ever written; the model says which pair each acquire returns, and the checks after every
//! acquire are the README's sentences, on the pair the channel returned. Across threads, the
//! render side acquires on its own thread, and events reach one frame with their snapshot.

#![forbid(unsafe_code)]

use crate::common;

use common::*;
use std::collections::BTreeMap;

use lactose_core::{FixedHashMap, Tick};
use lactose_ecs::{LayoutConfig, PresentEventKey, WorldKind};
use lactose_present_api::{PresentSnapshot, snapshot_channel};

const ROW: u64 = 1;
const START: u32 = 10;

/// What one write is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Write {
    /// A pass after this many ticks.
    Ticks(u32),
    /// A full pass at the tick of the previous one.
    Full,
    /// A full pass this many ticks back.
    Back(u32),
    /// An idle pass after an edit of the presented row.
    Idle,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Write(Write),
    Publish,
    Acquire,
}

/// A snapshot as the model sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Snap {
    /// The write it holds: the number in its `Shown`.
    id: u32,
    tick: u32,
    full: bool,
    cut: bool,
    number: u64,
    follows: Option<u64>,
}

/// The README's exchange, by its sentences: which buffer a pass writes, what a snapshot
/// follows, when one takes another's place, the mark, and the pair an acquire returns.
struct Model {
    kind: WorldKind,
    published: Vec<Snap>,
    held: Option<[Snap; 2]>,
    pending: Option<Snap>,
    publications: u64,
    last: Option<Snap>,
    mark: Option<u64>,
    acquired_at: Option<u64>,
    /// The publication numbers of the snapshots published and not replaced, in order.
    surviving: Vec<u64>,
    /// How many snapshots took another's place, and how many writes took a buffer back.
    replaced: u32,
    taken_back: u32,
}

/// What an acquire returned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pair {
    /// Nothing published yet.
    None,
    /// Nothing published since the previous acquire: the same pair.
    Same([Snap; 2]),
    /// The reader's newer snapshot twice, while a pass writes one it took back.
    Window([Snap; 2]),
    /// A new pair.
    New([Snap; 2]),
}

impl Model {
    fn new(kind: WorldKind) -> Self {
        Self {
            kind,
            published: Vec::new(),
            held: None,
            pending: None,
            publications: 0,
            last: None,
            mark: None,
            acquired_at: None,
            surviving: Vec::new(),
            replaced: 0,
            taken_back: 0,
        }
    }

    fn held_buffers(&self) -> usize {
        match self.held {
            None => 0,
            Some([older, newer]) if older.number == newer.number => 1,
            Some(_) => 2,
        }
    }

    fn write(&mut self, id: u32, tick: u32, full: bool, cut: bool) {
        if let Some(pending) = &mut self.pending {
            // A second pass before `publish` updates the same snapshot, which keeps a cut and
            // a full mark.
            pending.id = id;
            pending.tick = tick;
            pending.full |= full;
            pending.cut |= cut;
            return;
        }
        let free = 3 - self.published.len() - self.held_buffers();
        if free == 0 {
            // None free: the newest published, which the reader does not hold, is taken back.
            self.published.pop().expect("one buffer is free or published");
            self.taken_back += 1;
        }
        self.pending = Some(Snap { id, tick, full, cut, number: u64::MAX, follows: None });
    }

    fn publish(&mut self) {
        let Some(mut snap) = self.pending.take() else { return };
        let number = self.publications;
        self.publications += 1;
        let mut own = snap.cut || self.last.is_some_and(|last| snap.tick < last.tick);
        let replaced = self.last.filter(|last| {
            self.kind == WorldKind::Prediction && snap.full && last.tick == snap.tick
        });
        snap.follows = match replaced {
            Some(last) => {
                own |= last.cut;
                last.follows
            }
            None => self
                .published
                .last()
                .map(|published| published.number)
                .or(self.held.map(|[_, newer]| newer.number)),
        };
        if own {
            self.mark = Some(number);
        }
        snap.cut = own || self.mark.is_some();
        snap.number = number;
        let place = replaced.and_then(|last| {
            self.published.iter().position(|published| published.number == last.number)
        });
        match place {
            Some(index) => self.published[index] = snap,
            None => self.published.push(snap),
        }
        if let Some(last) = replaced {
            self.surviving.retain(|&number| number != last.number);
            self.replaced += 1;
        }
        self.surviving.push(number);
        self.last = Some(snap);
    }

    fn acquire(&mut self) -> Pair {
        if self.publications == 0 {
            return Pair::None;
        }
        if self.acquired_at == Some(self.publications) {
            return Pair::Same(self.held.expect("acquired"));
        }
        self.acquired_at = Some(self.publications);
        let pair = match self.published.pop() {
            None => {
                let [_, newer] = self.held.expect("what was published and is no more was acquired");
                Pair::Window([newer, newer])
            }
            Some(mut newer) => {
                let candidates: Vec<Snap> =
                    self.published.iter().copied().chain(self.held.into_iter().flatten()).collect();
                let older = newer
                    .follows
                    .and_then(|number| candidates.iter().find(|snap| snap.number == number))
                    .copied()
                    .or(self.held.map(|[_, newer]| newer))
                    .unwrap_or(newer);
                self.published.clear();
                // Once acquired, a snapshot follows the older of its pair.
                newer.follows = Some(older.number);
                if let Some(last) = self.last.as_mut().filter(|last| last.number == newer.number) {
                    last.follows = Some(older.number);
                }
                Pair::New([older, newer])
            }
        };
        let (Pair::Window(held) | Pair::New(held)) = pair else { unreachable!() };
        if self.mark.is_some_and(|mark| held[1].number >= mark) {
            self.mark = None;
        }
        self.held = Some(held);
        pair
    }
}

/// A world of `kind` with the presented row, its channel and the model, and what the checks
/// keep between acquires.
struct Run {
    rig: Rig,
    model: Model,
    id: u32,
    /// The kind of the previous acquire that returned a pair.
    previous: Option<Pair>,
    /// Acquires that followed a publication since the mark was set, and the mark they count
    /// for.
    since_mark: (Option<u64>, u32),
    /// The most acquires after a mark it took to pass it.
    longest_mark: u32,
    /// Two-snapshot pairs from before the mark returned while it was set, and the most for
    /// one mark.
    stale: u32,
    most_stale: u32,
    windows: u32,
    pairs: u32,
}

impl Run {
    fn new(kind: WorldKind) -> Self {
        let simulation = simulation(kind, LayoutConfig::default(), Tick(START));
        let mut rig = Rig::new(simulation, executor(2));
        rig.world_mut().spawn(key(ROW), REGION, (at(0.0), Shown(0), claim())).unwrap();
        Self {
            rig,
            model: Model::new(kind),
            id: 0,
            previous: None,
            since_mark: (None, 0),
            longest_mark: 0,
            stale: 0,
            most_stale: 0,
            windows: 0,
            pairs: 0,
        }
    }

    fn write(&mut self, write: Write) {
        let kind = self.rig.world().kind();
        let tick = self.rig.tick().0;
        // The first pass is full.
        let first = self.id == 0;
        let (full, cut) = match write {
            Write::Ticks(ticks) => {
                for _ in 0..ticks {
                    self.rig.step();
                }
                (false, false)
            }
            Write::Full => {
                self.rig.world_mut().set_tick(Tick(tick)).unwrap();
                (true, kind == WorldKind::Server)
            }
            Write::Back(back) => {
                let to = tick.saturating_sub(back).max(START);
                self.rig.world_mut().set_tick(Tick(to)).unwrap();
                (true, kind == WorldKind::Server)
            }
            Write::Idle => (false, false),
        };
        self.id += 1;
        self.rig.world_mut().get_mut::<Shown>(key(ROW)).unwrap().0 = self.id;
        self.rig.passes();
        self.rig.recording.take();
        let cut = cut || (first && kind == WorldKind::Server);
        self.model.write(self.id, self.rig.tick().0, full || first, cut);
    }

    fn apply(&mut self, op: Op) {
        match op {
            Op::Write(write) => self.write(write),
            Op::Publish => {
                self.rig.publish();
                self.model.publish();
            }
            Op::Acquire => self.acquire(),
        }
    }

    fn acquire(&mut self) {
        let mark = self.model.mark;
        let expected = self.model.acquire();
        let frame = self.rig.reader.acquire();
        let held = match expected {
            Pair::None => {
                assert!(frame.is_none(), "nothing was published");
                return;
            }
            Pair::Same(held) | Pair::Window(held) | Pair::New(held) => held,
        };
        let frame = frame.expect("published");
        let seen = |snapshot: &PresentSnapshot| {
            let shown = snapshot.get::<Shown>(key(ROW)).expect("the presented row").0;
            (shown, snapshot.tick().0, snapshot.cut())
        };
        let (older, newer) = (seen(frame.older()), seen(frame.newer()));
        assert_eq!(
            (older, newer),
            ((held[0].id, held[0].tick, held[0].cut), (held[1].id, held[1].tick, held[1].cut)),
            "the channel's pair is the model's: {expected:?}"
        );
        if matches!(expected, Pair::Same(_)) {
            return;
        }
        self.pairs += 1;

        // After the first publication no pair holds one snapshot twice, except the reader's
        // newer while a pass writes a snapshot it took back, and never two such pairs in a
        // row.
        if held[0].number == held[1].number && self.model.surviving.len() > 1 {
            assert!(matches!(expected, Pair::Window(_)), "one snapshot twice: {held:?}");
            assert!(self.model.pending.is_some(), "a window pair while no pass writes");
            assert!(!matches!(self.previous, Some(Pair::Window(_))), "two window pairs in a row");
        }
        if matches!(expected, Pair::Window(_)) {
            self.windows += 1;
        }
        // No pair's older snapshot is of a later tick than its newer one, unless that one is
        // a cut.
        assert!(held[0].tick <= held[1].tick || held[1].cut, "a pair backward in time: {held:?}");
        // The first acquire whose newer snapshot was published since a cut has a cut as its
        // newer one.
        if let Some(mark) = mark.filter(|&mark| held[1].number >= mark) {
            assert!(held[1].cut, "the first newer snapshot published since the cut {mark}");
        }
        // The mark is passed by the third acquire after it that follows a publication.
        match mark {
            Some(mark) => {
                let same = self.since_mark.0 == Some(mark);
                let count = if same { self.since_mark.1 + 1 } else { 1 };
                self.since_mark = (Some(mark), count);
                if !same {
                    self.stale = 0;
                }
                if matches!(expected, Pair::New(_)) && self.model.mark == Some(mark) {
                    self.stale += 1;
                    self.most_stale = self.most_stale.max(self.stale);
                }
                if self.model.mark.is_none() {
                    self.longest_mark = self.longest_mark.max(count);
                }
                assert!(
                    count < 3 || self.model.mark != Some(mark),
                    "the mark outlived three acquires"
                );
            }
            None => self.since_mark = (None, 0),
        }
        self.previous = Some(expected);
    }
}

/// A small deterministic generator: the test needs no crate for it.
struct Draw(u64);

impl Draw {
    fn next(&mut self, below: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        u32::try_from(self.0 % u64::from(below)).expect("below a u32")
    }

    fn write(&mut self) -> Op {
        Op::Write(match self.next(8) {
            0..=3 => Write::Ticks(1 + self.next(3)),
            4 => Write::Full,
            5 => Write::Back(1 + self.next(3)),
            _ => Write::Idle,
        })
    }
}

use Op::{Acquire, Publish};

/// A cadence: the writes drawn, the publishes and acquires fixed.
fn cadence(pattern: &[Option<Op>], repeats: usize, draw: &mut Draw) -> Vec<Op> {
    (0..repeats)
        .flat_map(|_| {
            pattern.iter().map(|op| op.unwrap_or_else(|| draw.write())).collect::<Vec<_>>()
        })
        .collect()
}

const WRITE: Option<Op> = None;

fn drive(kind: WorldKind, ops: &[Op]) -> Run {
    let mut run = Run::new(kind);
    for &op in ops {
        run.apply(op);
    }
    run
}

const KINDS: [WorldKind; 2] = [WorldKind::Server, WorldKind::Prediction];

/// A reader that acquires every publication gets the two newest published (a replaced one is
/// not published), and never a window pair.
#[test]
fn a_reader_that_acquires_every_publication() {
    for kind in KINDS {
        for seed in 1..=8 {
            let mut draw = Draw(seed);
            let mut run = Run::new(kind);
            for op in cadence(&[WRITE, Some(Publish), Some(Acquire)], 30, &mut draw) {
                run.apply(op);
                if op == Acquire {
                    // The two newest published, a replaced one not counted; while one snapshot
                    // survives (the first, or its replacement) the pair is of its tick.
                    let held = run.model.held.unwrap();
                    let surviving = &run.model.surviving;
                    assert_eq!(held[1].number, *surviving.last().unwrap());
                    match surviving.len() {
                        1 => assert_eq!(held[0].tick, held[1].tick, "{kind:?} {seed}"),
                        n => assert_eq!(held[0].number, surviving[n - 2], "{kind:?} {seed}"),
                    }
                }
            }
            assert_eq!(run.windows, 0);
            assert!(run.longest_mark <= 1, "{kind:?}: passed at the first acquire");
        }
    }
}

/// A reader that acquires every second publication, and one that never does until the end:
/// passes never wait, and the pairs follow the model.
#[test]
fn a_reader_that_acquires_every_second_publication_or_never() {
    for kind in KINDS {
        for seed in 1..=8 {
            let mut draw = Draw(seed);
            let every_second = cadence(
                &[WRITE, Some(Publish), WRITE, Some(Publish), Some(Acquire)],
                20,
                &mut draw,
            );
            drive(kind, &every_second);
            let mut never = cadence(&[WRITE, Some(Publish)], 40, &mut draw);
            never.push(Acquire);
            drive(kind, &never);
        }
    }
}

/// Once per two publications, while the second pass writes the first's taken-back buffer:
/// every second acquire is a window pair, and the mark is passed after one pair.
#[test]
fn a_reader_that_acquires_while_a_pass_writes_a_taken_back_buffer() {
    for kind in KINDS {
        for seed in 1..=8 {
            let mut draw = Draw(seed);
            let ops = cadence(
                &[WRITE, Some(Publish), WRITE, Some(Acquire), Some(Publish)],
                24,
                &mut draw,
            );
            let run = drive(kind, &ops);
            assert!(run.windows >= run.pairs / 2 - 1, "{kind:?}: {} of {}", run.windows, run.pairs);
            assert!(run.longest_mark <= 3, "{kind:?} {seed}: {}", run.longest_mark);
        }
    }
}

/// In the slow cadence with one cut, wherever it falls, the mark is passed, after one pair
/// from before it at most: marking every snapshot until an acquire returned the latest one
/// published would keep marking them for good here.
#[test]
fn the_mark_in_the_slow_cadence() {
    use Write::{Back, Ticks};
    for kind in KINDS {
        for at in 4..14 {
            let mut ops = Vec::new();
            let mut writes = 0;
            for _ in 0..10 {
                for op in [None, Some(Publish), None, Some(Acquire), Some(Publish)] {
                    ops.push(op.unwrap_or_else(|| {
                        writes += 1;
                        Op::Write(if writes == at { Back(1) } else { Ticks(1) })
                    }));
                }
            }
            let run = drive(kind, &ops);
            // Passed at the latest by the third acquire, after one pair from before it at most.
            assert!(run.model.mark.is_none());
            assert!((1..=3).contains(&run.longest_mark), "{kind:?} {at}: {}", run.longest_mark);
            assert!(run.most_stale <= 1, "{kind:?} {at}: {}", run.most_stale);
        }
    }
}

/// Once per pass inside its span (between the pass's taking a buffer and its publication):
/// a window pair every second acquire, and never a pair backward in time.
#[test]
fn a_reader_that_acquires_once_per_pass_inside_its_span() {
    for kind in KINDS {
        for seed in 1..=8 {
            let mut draw = Draw(seed);
            let mut run = Run::new(kind);
            let mut kinds = Vec::new();
            for op in cadence(&[WRITE, Some(Acquire), Some(Publish)], 30, &mut draw) {
                run.apply(op);
                if op == Acquire {
                    kinds.push(matches!(run.previous, Some(Pair::Window(_))));
                }
            }
            // From the third acquire on, window pairs alternate with new ones.
            assert!(kinds[2..].windows(2).all(|pair| pair[0] != pair[1]), "{kind:?}: {kinds:?}");
            assert!(run.longest_mark <= 3);
        }
    }
}

/// Random sequences of every operation, a second pass before a publication and a second
/// acquire before one included.
#[test]
fn random_sequences_hold_every_rule() {
    for kind in KINDS {
        let mut seen = [0; 4];
        for seed in 1..=60 {
            let mut draw = Draw(seed * 7919);
            let ops: Vec<Op> = (0..80)
                .map(|_| match draw.next(3) {
                    0 => draw.write(),
                    1 => Publish,
                    _ => Acquire,
                })
                .collect();
            let run = drive(kind, &ops);
            assert!(run.longest_mark <= 3);
            let counts = [run.windows, run.model.taken_back, run.model.replaced, run.longest_mark];
            seen.iter_mut().zip(counts).for_each(|(seen, count)| *seen += count);
        }
        // The runs went through window pairs, take-backs, marks and, in a prediction world,
        // replacements.
        assert!(seen[0] > 0 && seen[1] > 0 && seen[3] > 0, "{kind:?}: {seen:?}");
        assert_eq!(seen[2] > 0, kind == WorldKind::Prediction, "{kind:?}: {seen:?}");
    }
}

/// The sentences a random run may miss, each in one sequence: a cut taken back before any
/// acquire saw it still reaches the reader; a prediction world's full pass at the tick
/// published last takes that snapshot's place, keeps its cut and pairs from what it
/// followed; a snapshot a second pass updates before `publish` keeps a cut and a full mark.
#[test]
fn a_cut_reaches_the_reader_and_a_replacement_keeps_it() {
    use Write::{Back, Full, Ticks};
    let w = Op::Write;
    // The cut (a pass back) is published, then taken back by the next pass, which publishes
    // over it: the reader's next newer snapshot is a cut.
    for kind in KINDS {
        let mut run = drive(
            kind,
            &[w(Ticks(3)), Publish, Acquire, w(Ticks(1)), Publish, w(Back(2)), Publish],
        );
        run.apply(w(Ticks(1)));
        run.apply(Publish);
        run.apply(Acquire);
        let held = run.model.held.unwrap();
        assert!(held[1].cut, "{kind:?}");
    }
    // A rollback at the tick published last takes its place: it pairs from the tick before.
    let mut run =
        drive(WorldKind::Prediction, &[w(Ticks(1)), Publish, w(Ticks(1)), Publish, Acquire]);
    let before = run.model.held.unwrap();
    run.apply(w(Full));
    run.apply(Publish);
    run.apply(Acquire);
    let after = run.model.held.unwrap();
    assert_eq!(after[0].number, before[0].number, "the pair goes on from the tick before");
    assert_eq!(after[1].tick, before[1].tick);
    assert!(!after[1].cut);
    // A resync back, then a rollback at its tick: the replacement is a cut as well.
    run.apply(w(Back(1)));
    run.apply(Publish);
    run.apply(w(Full));
    run.apply(Publish);
    run.apply(Acquire);
    assert!(run.model.held.unwrap()[1].cut, "a snapshot that takes the place of a cut is one");
    // Two passes before a publish: a full one, then one after a tick, keep the full mark (a
    // `Server` cut) and the cut.
    let mut run = drive(WorldKind::Server, &[w(Ticks(1)), Publish, Acquire]);
    run.apply(w(Full));
    run.apply(w(Ticks(1)));
    run.apply(Publish);
    run.apply(Acquire);
    assert!(run.model.held.unwrap()[1].cut);
}

/// Passes on one thread and acquires on another: nothing waits for the other but for the
/// exchange, nothing panics (every buffer a pass writes is unshared), and no pair goes back
/// in time unless its newer snapshot is a cut.
#[test]
#[expect(clippy::disallowed_methods, reason = "the render side's thread, in a test")]
fn the_render_side_on_its_own_thread() {
    for kind in KINDS {
        let mut run = Run::new(kind);
        let mut reader =
            core::mem::replace(&mut run.rig.reader, lactose_present_api::snapshot_channel().1);
        let done = std::sync::Arc::new(std::sync::Mutex::new(false));
        let finished = std::sync::Arc::clone(&done);
        let render = std::thread::spawn(move || {
            let mut pairs = 0u32;
            loop {
                let last = *finished.lock().expect("unpoisoned");
                if let Some(frame) = reader.acquire() {
                    let (older, newer) = (frame.older(), frame.newer());
                    assert!(older.tick() <= newer.tick() || newer.cut());
                    pairs += 1;
                }
                if last {
                    return pairs;
                }
            }
        });
        let mut draw = Draw(99);
        for _ in 0..300 {
            let op = draw.write();
            if let Op::Write(write) = op {
                run.write(write);
            }
            run.rig.publish();
            run.model.publish();
        }
        *done.lock().expect("unpoisoned") = true;
        assert!(render.join().expect("the render side does not panic") > 0);
    }
}

/// A prediction world whose four presented rows walk and chirp every tick, for the tests
/// across threads.
fn chirping_rig() -> Rig {
    let simulation = simulation(WorldKind::Prediction, LayoutConfig::default(), Tick(100));
    let mut rig = Rig::new(simulation, executor(2));
    for n in 1..=4u32 {
        let row = (at(0.0), mesh(n.into()), Walker(scalar(1.0)), claim(), Chirper(1), Shown(n));
        rig.world_mut().spawn(key(n.into()), REGION, row).unwrap();
    }
    rig
}

/// The exchange is one: passes, publications and acquires at drawn cadences on two threads,
/// and each emission reaches exactly one frame, and a frame whose newer snapshot is of tick
/// T has every emission published with or before T's first publication. Publishing the
/// snapshot and its events, or taking the pair and the events, in two exchanges fails here
/// (review round 1, C3). The reader acquires without pause, which is what lets a split with
/// nothing between its two exchanges show (review round 2, C8). It stops once the writer is
/// done and it has acquired after that, so every wait ends.
#[test]
#[expect(clippy::disallowed_methods, reason = "the render side's thread, in a test")]
fn events_reach_one_frame_with_their_snapshot_across_threads() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    for seed in [7_u64, 11, 13] {
        let mut rig = chirping_rig();
        let mut reader = core::mem::replace(&mut rig.reader, snapshot_channel().1);
        // Per tick: how many emissions were published with or before the first publication
        // of a snapshot of it. Written before that publication, so the reader finds it.
        let published: Arc<Mutex<BTreeMap<u32, usize>>> = Arc::default();
        let done = Arc::new(AtomicBool::new(false));
        let (by_tick, finished) = (Arc::clone(&published), Arc::clone(&done));
        let render = std::thread::spawn(move || {
            let mut seen: FixedHashMap<PresentEventKey, ()> = FixedHashMap::new();
            let mut acquires = 0_u32;
            loop {
                let last = finished.load(Ordering::Acquire);
                if let Some(frame) = reader.acquire() {
                    acquires += 1;
                    for emission in &frame.events().emitted {
                        let again = seen.insert(emission.key, ()).is_some();
                        assert!(!again, "an emission in two frames: {:?}", emission.key);
                    }
                    let newer = frame.newer().tick();
                    let needed = *by_tick
                        .lock()
                        .expect("unpoisoned")
                        .get(&newer.0)
                        .expect("noted before it was published");
                    assert!(
                        seen.len() >= needed,
                        "a frame holds a snapshot of {newer:?} with {} emissions seen, {needed} \
                         published with or before it",
                        seen.len()
                    );
                }
                if last {
                    return (seen.len(), acquires);
                }
            }
        });
        let mut draw = Draw(seed.wrapping_mul(31) | 1);
        let mut emitted = 0;
        for _ in 0..1000 {
            for _ in 0..draw.next(3) {
                rig.step();
            }
            rig.passes();
            if draw.next(6) == 0 {
                // A second pass before the publication: the writer keeps its buffer.
                rig.passes();
            }
            let events = rig.world_mut().take_present_events();
            emitted += events.emitted.len();
            let tick = rig.tick().0;
            published.lock().expect("unpoisoned").entry(tick).or_insert(emitted);
            rig.state().publish(events);
            if draw.next(3) == 0 {
                std::thread::yield_now();
            }
        }
        done.store(true, Ordering::Release);
        let (seen, acquires) = render.join().expect("the render side does not panic");
        assert!(emitted > 0 && acquires > 1, "seed {seed}: {emitted} emitted, {acquires} acquires");
        assert_eq!(seen, emitted, "seed {seed}: every emission reaches exactly one frame");
    }
}

/// A window pair across threads, driven step by step: the reader holds two snapshots, the
/// writer publishes a third and takes it back for its next pass, and the reader acquires on
/// its own thread between that pass and its publication. The pair is the reader's newer
/// snapshot twice, and the events published with the taken-back snapshot arrive in that
/// frame (review round 1, C3). Every wait is bounded.
#[test]
#[expect(clippy::disallowed_methods, reason = "the render side's thread, in a test")]
fn a_window_pair_across_threads() {
    use std::sync::mpsc;
    use std::time::Duration;

    /// What the reader saw: one buffer twice, the pair's ticks, the emissions.
    type Seen = (bool, Tick, Tick, usize);
    let mut rig = chirping_rig();
    let mut reader = core::mem::replace(&mut rig.reader, snapshot_channel().1);
    let (go, go_rx) = mpsc::channel::<()>();
    let (seen_tx, seen) = mpsc::channel::<Seen>();
    let render = std::thread::spawn(move || {
        while go_rx.recv_timeout(Duration::from_secs(60)).is_ok() {
            let frame = reader.acquire().expect("published");
            let (older, newer) = (frame.older(), frame.newer());
            let events = frame.events().emitted.len();
            let seen = (core::ptr::eq(older, newer), older.tick(), newer.tick(), events);
            if seen_tx.send(seen).is_err() {
                return;
            }
        }
    });
    let acquired = |go: &mpsc::Sender<()>| {
        go.send(()).expect("the reader waits");
        seen.recv_timeout(Duration::from_secs(60)).expect("the reader answers within a minute")
    };
    let publish = |rig: &mut Rig| {
        let events = rig.world_mut().take_present_events();
        let count = events.emitted.len();
        rig.state().publish(events);
        count
    };
    // Two publications, each acquired: the reader holds two buffers.
    for _ in 0..2 {
        rig.step();
        rig.passes();
        publish(&mut rig);
        acquired(&go);
    }
    let held = rig.tick();
    // A third publication, not acquired: the third buffer is published.
    rig.step();
    rig.passes();
    let third = publish(&mut rig);
    assert!(third > 0);
    // The next pass finds no free buffer and takes the third back; the reader acquires
    // between that pass and its publication.
    rig.step();
    rig.passes();
    let (same, older, newer, events) = acquired(&go);
    assert!(same, "the acquire during the take-back's pass returns one buffer twice");
    assert_eq!((older, newer), (held, held), "the reader's newer snapshot");
    assert_eq!(events, third, "the taken-back snapshot's events arrive in the window frame");
    let fourth = publish(&mut rig);
    let (same, older, newer, events) = acquired(&go);
    assert!(!same);
    assert_eq!((older, newer, events), (held, rig.tick(), fourth));
    drop(go);
    render.join().expect("the render side does not panic");
}
