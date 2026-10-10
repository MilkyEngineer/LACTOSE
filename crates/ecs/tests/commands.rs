//! Command apply order: (execution order, source origin key, sequence), whatever the
//! layout (architecture section 5.2).

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{Commands, LayoutConfig, Schedule, StableKeyOf, World, WorldKind, system};
use lactose_reflect::component;

/// Which invocation spawned an entity, and its place in the invocation.
#[component(key = "test::origin", persistent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Origin {
    #[field(id = 1)]
    source: u64,
    #[field(id = 2)]
    system: u32,
    #[field(id = 3)]
    seq: u32,
}

/// The entity every invocation overwrites: the last write is the last command applied.
const TARGET: u64 = 1;

fn record(cmd: &mut Commands, key: StableKey, system: u32) {
    let StableKey::Net(source) = key else { panic!("server keys") };
    for seq in 0..3 {
        cmd.spawn((Origin { source: source.to_bits(), system, seq },));
        cmd.set(
            net(TARGET),
            Counter(u64::from(system) << 32 | source.to_bits() << 8 | u64::from(seq)),
        );
    }
}

#[system(name = "test::order_a", worlds = [server])]
fn order_a(cmd: Commands, key: StableKeyOf, _counter: &Counter) {
    record(&mut cmd, *key, 0);
}

#[system(name = "test::order_b", worlds = [server])]
fn order_b(cmd: Commands, key: StableKeyOf, _counter: &Counter) {
    record(&mut cmd, *key, 1);
}

/// Spawned entities by key, with what spawned them.
fn run(layout: LayoutConfig) -> (Vec<(StableKey, Origin)>, Counter, Vec<&'static str>) {
    let executor = executor();
    let mut registry = base_registry();
    registry.register::<Origin>().unwrap();
    let mut world = world_with(WorldKind::Server, layout, std::sync::Arc::new(registry));
    // Sources in an order that is neither ascending nor descending.
    for id in [17, 3, 29, 11, 5, 23, 2, 13, 31, 7, 19] {
        world.spawn(net(id), REGION, (Counter(0),)).unwrap();
    }
    world.spawn(net(TARGET), REGION, (pos(0, 0),)).unwrap();
    let mut schedule = Schedule::builder(WorldKind::Server);
    schedule.stage("commands", &[]).add(&ORDER_B).add(&ORDER_A);
    let mut schedule = schedule.build().unwrap();
    let order = schedule.order().iter().map(|system| system.name()).collect();
    world = tick(&mut schedule, world, &executor);
    let spawned = spawned(&world);
    (spawned, *world.get::<Counter>(net(TARGET)).unwrap(), order)
}

fn spawned(world: &World) -> Vec<(StableKey, Origin)> {
    world.entities_in(REGION).filter_map(|key| Some((key, *world.get::<Origin>(key)?))).collect()
}

#[test]
fn commands_apply_in_execution_then_source_then_sequence_order() {
    let (spawned, target, order) = run(LayoutConfig::default());
    assert_eq!(spawned.len(), 2 * 11 * 3);
    // Execution order breaks the tie between unordered systems by key, not by registration.
    let a_first = order == ["test::order_a", "test::order_b"];
    assert!(a_first || order == ["test::order_b", "test::order_a"]);
    let exec = |system: u32| if a_first { system } else { 1 - system };
    // NetIds are allocated at apply, so ascending keys are the apply order.
    let applied: Vec<(u32, u64, u32)> =
        spawned.iter().map(|(_, o)| (exec(o.system), o.source, o.seq)).collect();
    let mut sorted = applied.clone();
    sorted.sort_unstable();
    assert_eq!(applied, sorted, "spawns applied out of order");
    let last = sorted.last().unwrap();
    let last_system = if exec(0) == last.0 { 0u64 } else { 1 };
    assert_eq!(
        target,
        Counter(last_system << 32 | last.1 << 8 | u64::from(last.2)),
        "the last set wins"
    );
}

#[test]
fn command_order_is_the_same_under_every_layout() {
    let reference = run(LayoutConfig::default());
    for layout in layouts() {
        assert_eq!(run(layout), reference, "{layout:?}");
    }
}
