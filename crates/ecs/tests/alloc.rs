//! What a kernel invocation allocates: nothing, for a row that emits nothing, whatever its
//! parameters. Counted by the tagged global allocator this test binary installs, across
//! every tag and thread, as the difference between two world sizes, so what a tick costs
//! whatever its size (the graph, the per-chunk buffers) cancels out.

#![forbid(unsafe_code)]
// A binary that installs `TaggedAlloc` runs under Miri only with Tree Borrows
// (`crates/core/SAFETY.md`), and the crate's Miri run uses Stacked Borrows.
#![cfg(not(miri))]

mod common;

use common::*;
use lactose_core::{AllocTag, StableKey, TaggedAlloc};
use lactose_ecs::{
    Commands, Ctx, EventWriter, LayoutConfig, Mut, Res, Schedule, StableKeyOf, WorldKind, system,
};

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

/// A key no row has.
fn nobody() -> StableKey {
    net(1_000_000)
}

#[system(name = "test::drift", worlds = [server])]
fn drift(
    ctx: Ctx,
    key: StableKeyOf,
    position: Mut<Position>,
    velocity: &Velocity,
    delta: Res<TickDelta>,
) {
    if *key != nobody() && ctx.tick().0 > 0 {
        position.set_if_neq(Position { x: velocity.x, y: delta.0 });
    }
}

#[system(name = "test::quiet", worlds = [server], after = drift)]
fn quiet(damage: EventWriter<Damage>, cmd: Commands, key: StableKeyOf, _velocity: &Velocity) {
    if *key == nobody() {
        damage.emit(Damage { target: *key, source: *key, amount: 1 });
        cmd.despawn(*key);
    }
}

fn allocations() -> u64 {
    AllocTag::ALL.iter().map(|&tag| ALLOCATOR.stats(tag).total_allocations).sum()
}

/// The allocations of the third tick over `rows` rows.
fn allocations_of_a_tick(rows: u64) -> u64 {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    for id in 1..=rows {
        world.spawn(net(id), REGION, (pos(0, 0), vel(1, 1))).unwrap();
    }
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("move", &[]).add(&QUIET).add(&DRIFT);
    let mut schedule = builder.build().unwrap();
    for _ in 0..2 {
        world = tick(&mut schedule, world, &executor);
    }
    let before = allocations();
    world = tick(&mut schedule, world, &executor);
    let allocated = allocations() - before;
    assert_eq!(world.get::<Position>(net(rows)), Some(&pos(1, 1)));
    allocated
}

#[test]
fn a_row_that_emits_nothing_allocates_nothing() {
    let (small, large) = (1_000, 5_000);
    let (small_tick, large_tick) = (allocations_of_a_tick(small), allocations_of_a_tick(large));
    // 4,000 more rows of two systems are 8,000 invocations; the bound admits a few
    // allocations per added chunk, none per row.
    let extra = large_tick.saturating_sub(small_tick);
    assert!(
        extra < 100,
        "{extra} more allocations for {} more rows ({small_tick} at {small}, {large_tick} at {large})",
        large - small
    );
}
