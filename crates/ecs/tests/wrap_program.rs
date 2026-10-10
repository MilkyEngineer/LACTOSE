//! One program run from several start ticks: far from any boundary, across the 2^30
//! rebase, across 2^31, in the upper half and across the `u32` wrap. It has a mark and a
//! log cursor kept by a consumer, `Changed` readers in two stages, a pass, column and row
//! ticks, presentation events, and a snapshot taken before the boundary and restored after
//! it. Every observation is relative to the start tick, so every run must agree with the
//! reference run, except the raw mark and cursor results, which a rebase makes stale by
//! design (the consumer then does a full pass and its mirror still agrees).

#![forbid(unsafe_code)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{
    ChangeMark, Changed, LayoutConfig, LogCursor, Mut, PresentEvents, Res, Schedule, StableKeyOf,
    StructuralChange, With, World, WorldKind, system,
};
use lactose_math::Scalar;
use lactose_task::Executor;

type Log = Mutex<Vec<(u8, StableKey)>>;

/// The start tick, as a service so it survives a restore.
struct Start(u32);

/// Ticks since the start.
fn rel(world: &World) -> u32 {
    world.tick().0.wrapping_sub(world.service::<Start>().unwrap().0)
}

#[system(name = "wrap_program::input", worlds = [server], exclusive, writes = [Scratch])]
fn input(world: &mut World) {
    let r = rel(world);
    if r.is_multiple_of(3) {
        let row = (pos(r as i32, 0), vel(1, 1), Mass(Scalar::ONE), Scratch(0));
        world.spawn(net(100 + u64::from(r)), REGION, row).unwrap();
    }
    if r % 4 == 1 {
        let key = net(100 + (u64::from(r) / 3) * 3);
        if world.contains(key) {
            world.insert(key, Health(r as i32)).unwrap();
        }
    }
    if r % 5 == 2 {
        let key = net(100 + (u64::from(r).saturating_sub(6) / 3) * 3);
        if world.contains(key) {
            world.despawn(key).unwrap();
        }
    }
    if r.is_multiple_of(2) && world.contains(net(2)) {
        world.get_mut::<Scratch>(net(2)).unwrap().0 = r;
    }
}

#[system(name = "wrap_program::w1", worlds = [server])]
fn w1(p: Mut<Position>, _m: With<Mass>) {
    p.x += Scalar::ONE;
}

#[system(name = "wrap_program::r1", worlds = [server], after = w1)]
fn r1(_c: Changed<Position>, key: StableKeyOf, log: Res<Log>) {
    log.lock().unwrap().push((1, *key));
}

#[system(name = "wrap_program::r2", worlds = [server])]
fn r2(_c: Changed<Position>, key: StableKeyOf, log: Res<Log>) {
    log.lock().unwrap().push((2, *key));
}

#[system(name = "wrap_program::w2", worlds = [server], after = r2)]
fn w2(p: Mut<Position>, _b: With<Bomb>) {
    p.y += Scalar::ONE;
}

#[system(name = "wrap_program::r3", worlds = [server])]
fn r3(_c: Changed<Health>, key: StableKeyOf, log: Res<Log>) {
    log.lock().unwrap().push((3, *key));
}

#[system(name = "wrap_program::sound", worlds = [server])]
fn sound(present: PresentEvents, key: StableKeyOf, _b: &Bomb) {
    present.emit(*key, AudioRequest::FOOTSTEP);
}

#[system(name = "wrap_program::pass_reader", worlds = [server])]
fn pass_reader(_c: Changed<Position>, key: StableKeyOf, log: Res<Log>) {
    log.lock().unwrap().push((9, *key));
}

fn tick_schedule() -> Schedule {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("a", &[]).add(&INPUT).add(&W1).add(&R1);
    builder.stage("b", &[]).add(&R2).add(&W2).add(&R3).add(&SOUND);
    builder.build().unwrap()
}

fn pass_schedule() -> Schedule {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("p", &[]).add(&PASS_READER);
    builder.build().unwrap()
}

/// What one step shows, every tick relative to the start or to now.
#[derive(Clone, PartialEq, Debug)]
struct Observation {
    rel: u32,
    readers: Vec<(u8, StableKey)>,
    pass: Option<Vec<(u8, StableKey)>>,
    /// The keys `changed_since` the consumer's mark yields, or `None` for a stale mark.
    since_raw: Option<Vec<StableKey>>,
    /// The consumer's mirror of every `Position`.
    mirror: BTreeMap<StableKey, (i64, i64)>,
    /// The log entries since the consumer's cursor, or `None` for a stale cursor.
    log_raw: Option<Vec<(u32, String)>>,
    /// The consumer's mirror of the live keys.
    log_mirror: BTreeSet<StableKey>,
    /// Per chunk of `Position` (tracked) and `Velocity` (untracked): keys, and the column
    /// tick's age.
    column_ticks: Vec<(char, Vec<StableKey>, u32)>,
    /// Per key: the age of the row's `Position` tick.
    row_ticks: BTreeMap<StableKey, u32>,
    present: Vec<(u32, StableKey, u32)>,
}

fn take_log(world: &World) -> Vec<(u8, StableKey)> {
    let mut log = core::mem::take(&mut *world.service::<Log>().unwrap().lock().unwrap());
    log.sort();
    log
}

fn full_mirror(world: &World) -> BTreeMap<StableKey, (i64, i64)> {
    let mut mirror = BTreeMap::new();
    for column in world.columns::<Position>() {
        for (key, value) in column.keys().iter().zip(column.values()) {
            mirror.insert(*key, (value.x.to_i64(), value.y.to_i64()));
        }
    }
    mirror
}

fn start_world(at: u32) -> World {
    let layout = LayoutConfig { chunk_rows: Some(2), ..LayoutConfig::default() };
    let mut world = world(WorldKind::Server, layout);
    world.set_tick(Tick(at)).unwrap();
    world.insert_service::<Log>(Box::new(Mutex::new(Vec::new())));
    world.insert_service::<Start>(Box::new(Start(at)));
    world.spawn(net(1), REGION, (pos(0, 0), Mass(Scalar::ONE), vel(0, 0))).unwrap();
    world.spawn(net(2), REGION, (pos(0, 0), Bomb { fuse: 1 }, Scratch(0), Health(5))).unwrap();
    world.spawn(net(3), REGION, (pos(0, 0), Bomb { fuse: 1 }, Mass(Scalar::ONE))).unwrap();
    world
}

/// A world, its schedules, and a consumer that follows it with a mark and a log cursor.
struct Runner {
    world: Option<World>,
    ticks: Schedule,
    pass: Schedule,
    mark: ChangeMark,
    mirror: BTreeMap<StableKey, (i64, i64)>,
    cursor: LogCursor,
    live: BTreeSet<StableKey>,
}

impl Runner {
    fn new(at: u32) -> Self {
        let world = start_world(at);
        Self {
            mark: world.mark(),
            cursor: world.structural_log().cursor(),
            mirror: full_mirror(&world),
            live: world.entities_in(REGION).collect(),
            world: Some(world),
            ticks: tick_schedule(),
            pass: pass_schedule(),
        }
    }

    /// A tick, a direct write, a pass if `with_pass`, and what the consumer sees.
    fn step(&mut self, executor: &Executor, with_pass: bool) -> Observation {
        let world = self.world.take().unwrap();
        let mut world = executor.block_on(self.ticks.run_tick(world, executor)).unwrap();
        let readers = take_log(&world);
        // A direct write between ticks, seen by every reader in the next tick.
        if let Some(mut p) = world.get_mut::<Position>(net(1)) {
            p.y += Scalar::ONE;
        }
        let pass = if with_pass {
            world = executor.block_on(self.pass.run_pass(world, executor)).unwrap();
            Some(take_log(&world))
        } else {
            None
        };
        let since_raw = world.changed_since::<Position>(self.mark).map(|rows| {
            let mut keys = Vec::new();
            for (key, value) in rows {
                self.mirror.insert(key, (value.x.to_i64(), value.y.to_i64()));
                keys.push(key);
            }
            keys.sort();
            keys
        });
        if since_raw.is_none() {
            self.mirror = full_mirror(&world);
        }
        self.mark = world.mark();
        let now = world.tick();
        let start = world.service::<Start>().unwrap().0;
        let log_raw = world.structural_log().since(&mut self.cursor).map(|entries| {
            entries
                .map(|entry| {
                    match entry.change {
                        StructuralChange::Spawned { key, .. } => {
                            self.live.insert(key);
                        }
                        StructuralChange::Despawned { key, .. } => {
                            self.live.remove(&key);
                            self.mirror.remove(&key);
                        }
                        _ => {}
                    }
                    (entry.tick.0.wrapping_sub(start), format!("{:?}", entry.change))
                })
                .collect::<Vec<_>>()
        });
        if log_raw.is_none() {
            self.live = world.entities_in(REGION).collect();
            self.cursor = world.structural_log().cursor();
            self.mirror.retain(|key, _| self.live.contains(key));
        }
        let mut column_ticks = Vec::new();
        for column in world.columns::<Position>() {
            let mut keys = column.keys().to_vec();
            keys.sort();
            column_ticks.push(('p', keys, now.0.wrapping_sub(column.column_tick().0)));
        }
        for column in world.columns::<Velocity>() {
            let mut keys = column.keys().to_vec();
            keys.sort();
            column_ticks.push(('v', keys, now.0.wrapping_sub(column.column_tick().0)));
        }
        column_ticks.sort();
        let mut row_ticks = BTreeMap::new();
        for column in world.columns::<Position>() {
            for (key, tick) in column.keys().iter().zip(column.row_ticks().unwrap()) {
                row_ticks.insert(*key, now.0.wrapping_sub(tick.0));
            }
        }
        let rel = rel(&world);
        let present = if rel % 3 == 2 {
            let batch = world.take_present_events();
            batch
                .emitted
                .iter()
                .map(|e| (e.key.tick.0.wrapping_sub(start), e.key.origin, e.key.seq))
                .collect()
        } else {
            Vec::new()
        };
        self.world = Some(world);
        Observation {
            rel,
            readers,
            pass,
            since_raw,
            mirror: self.mirror.clone(),
            log_raw,
            log_mirror: self.live.clone(),
            column_ticks,
            row_ticks,
            present,
        }
    }
}

/// Runs 14 ticks with a pass every other tick, snapshots after tick 3, restores after the
/// last and re-runs ticks 4 to 13. Returns both runs' observations.
fn program(at: u32) -> (Vec<Observation>, Vec<Observation>) {
    let executor = executor();
    let mut runner = Runner::new(at);
    let mut first = Vec::new();
    let mut snapshot = None;
    for step in 0..14 {
        first.push(runner.step(&executor, step % 2 == 1));
        if step == 3 {
            snapshot = Some(runner.world.as_ref().unwrap().snapshot());
        }
    }
    runner.world.as_mut().unwrap().restore(&snapshot.unwrap()).unwrap();
    let second = (4..14).map(|step| runner.step(&executor, step % 2 == 1)).collect();
    (first, second)
}

/// Every field of `b` that differs from the reference `a`, where all must match whatever
/// the start tick: all but the raw mark and cursor results.
fn differences(run: &str, a: &Observation, b: &Observation) -> Vec<String> {
    let mut out = Vec::new();
    let rel = a.rel;
    let mut push = |name: &str, same: bool, a: String, b: String| {
        if !same {
            out.push(format!("{run} rel {rel} {name}:\n  reference {a}\n  this run  {b}"));
        }
    };
    push("rel", a.rel == b.rel, format!("{}", a.rel), format!("{}", b.rel));
    push("readers", a.readers == b.readers, format!("{:?}", a.readers), format!("{:?}", b.readers));
    push("pass", a.pass == b.pass, format!("{:?}", a.pass), format!("{:?}", b.pass));
    push("mirror", a.mirror == b.mirror, format!("{:?}", a.mirror), format!("{:?}", b.mirror));
    push(
        "log mirror",
        a.log_mirror == b.log_mirror,
        format!("{:?}", a.log_mirror),
        format!("{:?}", b.log_mirror),
    );
    push(
        "column ticks",
        a.column_ticks == b.column_ticks,
        format!("{:?}", a.column_ticks),
        format!("{:?}", b.column_ticks),
    );
    push(
        "row ticks",
        a.row_ticks == b.row_ticks,
        format!("{:?}", a.row_ticks),
        format!("{:?}", b.row_ticks),
    );
    push("present", a.present == b.present, format!("{:?}", a.present), format!("{:?}", b.present));
    out
}

/// The program from `at` against the reference run from tick 1000.
fn compare(at: u32) {
    let (reference_first, reference_second) = program(1000);
    let (first, second) = program(at);
    let mut out = Vec::new();
    for (a, b) in reference_first.iter().zip(&first) {
        out.extend(differences("first run", a, b));
    }
    for (a, b) in reference_second.iter().zip(&second) {
        out.extend(differences("re-simulation", a, b));
    }
    for b in &second {
        let a = first.iter().find(|a| a.rel == b.rel).unwrap();
        if a.readers != b.readers {
            out.push(format!("re-simulation rel {} differs from the first run", b.rel));
        }
    }
    assert!(out.is_empty(), "from tick {at}:\n{}", out.join("\n"));
}

#[test]
fn far_from_any_boundary() {
    compare(5000);
}

#[test]
fn across_the_2_30_rebase() {
    compare((1 << 30) - 6);
}

#[test]
fn across_2_31() {
    compare((1 << 31) - 6);
}

#[test]
fn in_the_upper_half() {
    compare(3_000_000_000);
}

#[test]
fn across_the_wrap() {
    compare(u32::MAX - 6);
}
