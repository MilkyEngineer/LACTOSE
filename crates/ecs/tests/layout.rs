//! Layout independence (architecture section 8.3, SOUR's layout mode): one program under
//! every layout gives byte-identical exports, identical structural logs, identical
//! reductions and identical presentation events.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{
    Commands, Ctx, LayoutConfig, LogEntry, Mut, RegionConfig, RegionMode, RegionRows, StableKeyOf,
    WorldKind, system,
};
use lactose_math::Scalar;

/// Spawns, despawns and region moves drawn from the entity's RNG stream.
#[system(name = "test::churn", worlds = [server])]
fn churn(ctx: Ctx, cmd: Commands, key: StableKeyOf, counter: Mut<Counter>) {
    counter.0 += 1;
    let mut rng = ctx.rng(*key);
    match rng.next_below(6) {
        0 => {
            let child = cmd.spawn((Counter(0), pos(counter.0 as i32, 0), Mass(rng.next_scalar())));
            cmd.set(child, ThrownBy { thrower: lactose_core::EntityRef::new(*key) });
        }
        1 if counter.0 > 2 => cmd.despawn(*key),
        2 => cmd.move_to_region(*key, if ctx.region() == REGION { OTHER_REGION } else { REGION }),
        3 => cmd.set(*key, Health(rng.next_below(100) as i32)),
        _ => {}
    }
}

type Outcome = (Vec<RegionRows>, Vec<LogEntry>, Scalar, Vec<(StableKey, u32)>);

fn run(layout: LayoutConfig) -> Outcome {
    let executor = executor();
    let mut world = world(WorldKind::Server, layout);
    world
        .create_region(RegionConfig { id: OTHER_REGION, seed: 0xD1CE, mode: RegionMode::Simulated })
        .unwrap();
    populate(&mut world);
    for id in 10..40u64 {
        let region = if id % 3 == 0 { OTHER_REGION } else { REGION };
        let mass = Mass(Scalar::from_i32(id as i32) / Scalar::from_i32(7));
        match id % 4 {
            0 => world.spawn(net(id), region, (Counter(0), mass)).unwrap(),
            1 => world.spawn(net(id), region, (Counter(0), mass, pos(1, 2), vel(0, 1))).unwrap(),
            2 => world.spawn(net(id), region, (Counter(id), AnimState { phase: id as u32 })).unwrap(),
            _ => world.spawn(net(id), region, (Counter(0), Health(50), mass)).unwrap(),
        }
        world.allocate_net_id().unwrap();
    }
    let mut builder = lactose_ecs::Schedule::builder(WorldKind::Server);
    builder.stage("input", &[&sets::INPUT]).add(&READ_INPUT);
    builder
        .stage("simulation", &[&sets::MOVEMENT])
        .add(&APPLY_GRAVITY)
        .add(&INTEGRATE)
        .add(&TOTAL_MASS)
        .add(&THROW_BOMB)
        .add(&SWORD_HIT)
        .add(&APPLY_DAMAGE)
        .add(&FOOTSTEPS)
        .add(&ANIMATE);
    builder.stage("churn", &[]).add(&CHURN);
    let mut schedule = builder.build().unwrap();
    let mut presented = Vec::new();
    for _ in 0..6 {
        world = tick(&mut schedule, world, &executor);
        let batch = world.take_present_events();
        presented.extend(batch.emitted.iter().map(|e| (e.key.origin, e.key.tick.0)));
    }
    let exports = [REGION, OTHER_REGION].map(|region| world.export_region(region).unwrap()).to_vec();
    let mut cursor = world.structural_log().cursor_at_start();
    let log = world.structural_log().since(&mut cursor).unwrap().copied().collect();
    (exports, log, world.resource::<TotalMass>().unwrap().0, presented)
}

#[test]
fn every_layout_gives_the_same_bytes_and_the_same_log() {
    let reference = run(LayoutConfig::default());
    assert!(reference.1.len() > 40, "the program changes structure: {}", reference.1.len());
    assert!(reference.0.iter().all(|rows| !rows.rows.is_empty()));
    for layout in layouts() {
        let outcome = run(layout);
        assert_eq!(outcome.0, reference.0, "exports under {layout:?}");
        assert_eq!(outcome.1, reference.1, "structural log under {layout:?}");
        assert_eq!(outcome.2.to_canonical_bits(), reference.2.to_canonical_bits(), "reduction under {layout:?}");
        assert_eq!(outcome.3, reference.3, "presentation under {layout:?}");
    }
}
