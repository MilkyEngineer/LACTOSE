//! `Changed<T>` seen exactly once per reader across the tick counter's boundaries, the
//! `u32` wrap and the 2^30 clamp, and across `set_tick` (review of the implementation, M1
//! and M2).

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{
    Changed, Mut, RegionConfig, RegionMode, Res, Schedule, StableKeyOf, With, World, WorldBuilder,
    WorldKind, system,
};
use lactose_math::Scalar;

type Log = Mutex<Vec<(u32, StableKey)>>;

#[system(name = "pw::w_before", worlds = [server])]
fn w_before(p: Mut<Position>, _m: With<Mass>) {
    p.x += Scalar::ONE;
}

#[system(name = "pw::reader", worlds = [server], after = w_before)]
fn reader(_c: Changed<Position>, key: StableKeyOf, log: Res<Log>, dt: Res<TickDelta>) {
    let _ = dt;
    log.lock().unwrap().push((0, *key));
}

#[system(name = "pw::w_after", worlds = [server], after = reader)]
fn w_after(p: Mut<Position>, _b: With<Bomb>) {
    p.x += Scalar::ONE;
}

fn schedule() -> Schedule {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("only", &[]).add(&W_BEFORE).add(&READER).add(&W_AFTER);
    builder.build().unwrap()
}

const A: u64 = 1;
const B: u64 = 2;

fn start(at: u32) -> World {
    let mut world =
        WorldBuilder::new(WorldKind::Server, registry()).tick(Tick(at)).build().unwrap();
    world.create_region(RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated }).unwrap();
    world.insert_resource(TickDelta(Scalar::ONE));
    world.insert_service::<Log>(Box::new(Mutex::new(Vec::new())));
    world.spawn(net(A), REGION, (pos(0, 0), Mass(Scalar::ONE))).unwrap();
    world.spawn(net(B), REGION, (pos(0, 0), Bomb { fuse: 1 })).unwrap();
    world
}

/// Per tick run, what the reader saw: (tick after the run, sorted keys).
fn run(at: u32, ticks: u32) -> Vec<(u32, Vec<StableKey>)> {
    let executor = executor();
    let mut schedule = schedule();
    let mut world = start(at);
    let mut out = Vec::new();
    for _ in 0..ticks {
        world = tick(&mut schedule, world, &executor);
        let mut seen: Vec<StableKey> =
            std::mem::take(&mut *world.service::<Log>().unwrap().lock().unwrap())
                .into_iter()
                .map(|(_, k)| k)
                .collect();
        seen.sort();
        out.push((world.tick().0, seen));
    }
    out
}

#[test]
fn reference_far_from_any_boundary() {
    let got = run(100, 6);
    for (t, seen) in &got {
        assert_eq!(seen, &vec![net(A), net(B)], "tick {t}: {got:?}");
    }
}

#[test]
fn across_the_two_to_the_thirty_clamp() {
    let got = run((1 << 30) - 3, 6);
    for (t, seen) in &got {
        assert_eq!(seen, &vec![net(A), net(B)], "tick {t}: {got:?}");
    }
}

#[test]
fn across_the_u32_wrap() {
    let got = run(u32::MAX - 2, 6);
    assert_eq!(got[2].0, 0, "the run crosses the wrap");
    for (t, seen) in &got {
        assert_eq!(seen, &vec![net(A), net(B)], "tick {t}: {got:?}");
    }
}

#[test]
fn changed_since_across_the_u32_wrap() {
    let executor = executor();
    let mut schedule = schedule();
    let mut world = start(u32::MAX - 2);
    let mut current = Vec::new();
    for _ in 0..5 {
        let mark = world.mark();
        world = tick(&mut schedule, world, &executor);
        if world.is_current(mark) {
            let rows: Vec<StableKey> =
                world.changed_since::<Position>(mark).unwrap().map(|(key, _)| key).collect();
            assert_eq!(rows.len(), 2, "tick {}: a current mark misses changes", world.tick().0);
            current.push(world.tick().0);
        } else {
            // Tick zero is a multiple of 2^30: the clamp starts an epoch there.
            assert_eq!(world.tick().0, 0);
        }
    }
    assert_eq!(current, [u32::MAX - 1, u32::MAX, 1, 2], "marks on both sides of the wrap work");
}

#[test]
fn set_tick_forward_keeps_pending_changes() {
    let executor = executor();
    let mut schedule = schedule();
    let mut world = start(100);
    world = tick(&mut schedule, world, &executor);
    world.service::<Log>().unwrap().lock().unwrap().clear();
    // Between ticks, a direct write that the reader has not seen yet.
    world.spawn(net(3), REGION, (pos(9, 9),)).unwrap();
    let now = world.tick().0;
    world.set_tick(Tick(now + 10)).unwrap();
    world = tick(&mut schedule, world, &executor);
    let mut seen: Vec<StableKey> =
        world.service::<Log>().unwrap().lock().unwrap().iter().map(|(_, k)| *k).collect();
    seen.sort();
    // A: written before the reader this tick; B: written after the reader last tick;
    // net(3): spawned between ticks.
    assert_eq!(seen, vec![net(A), net(B), net(3)], "after set_tick forward");
}
