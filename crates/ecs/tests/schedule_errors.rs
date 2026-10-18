//! Every `ScheduleError` variant, triggered, and the two ways `run_tick` fails.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::ErrorKind;
use lactose_ecs::{
    AccessTarget, Changed, Channel, ChannelTake, ChannelWriter, Commands, Ctx, EventWriter, Events,
    Job, LayoutConfig, Mut, OwnedBy, Res, ResMut, Schedule, ScheduleError, StableKeyOf,
    SystemDescriptor, SystemSet, Without, World, WorldKind, system,
};
use lactose_math::Vec2;
use lactose_reflect::{Reflect, component};

/// A component no registry of these tests holds.
#[component(key = "test::unregistered", persistent)]
struct Unregistered(#[field(id = 1)] u32);

#[system(name = "test::plain")]
fn plain(_counter: &Counter) {}

#[system(name = "test::dup")]
fn dup_a(_counter: &Counter) {}

#[system(name = "test::dup")]
fn dup_b(_health: &Health) {}

static ELSEWHERE: SystemSet = SystemSet::new("test::elsewhere");

static FIRST_ALONE: SystemSet = SystemSet::new("test::first_alone");

#[system(name = "test::in_elsewhere", in_set = ELSEWHERE)]
fn in_elsewhere(_counter: &Counter) {}

#[system(name = "test::after_plain", after = plain)]
fn after_plain(_counter: &Counter) {}

#[system(name = "test::exclusive_entity", exclusive)]
fn exclusive_entity(_world: &mut World, _counter: &Counter) {}

#[system(name = "test::key_alone")]
fn key_alone(_key: StableKeyOf) {}

#[system(name = "test::exclusive_ctx", exclusive)]
fn exclusive_ctx(_world: &mut World, _ctx: Ctx) {}

#[system(name = "test::entity_writes_resource")]
fn entity_writes_resource(_gravity: ResMut<Gravity>, _counter: &Counter) {}

#[system(name = "test::entity_job")]
fn entity_job(_job: Job, _counter: &Counter) {}

#[system(name = "test::aliased")]
fn aliased(_read: &Counter, _write: Mut<Counter>) {}

#[system(name = "test::without_beside")]
fn without_beside(_read: &Counter, _without: Without<Counter>) {}

#[system(name = "test::two_commands")]
fn two_commands(_a: Commands, _b: Commands) {}

#[system(name = "test::resource_twice")]
fn resource_twice(_a: ResMut<Gravity>, _b: Res<Gravity>) {}

#[system(name = "test::cycle_a", after = cycle_b)]
fn cycle_a(_counter: &Counter) {}

#[system(name = "test::cycle_b", after = cycle_a)]
fn cycle_b(_counter: &Counter) {}

#[system(name = "test::write_a")]
fn write_a(_counter: Mut<Counter>) {}

#[system(name = "test::write_b")]
fn write_b(_counter: Mut<Counter>) {}

/// Writes `Counter` through commands, declared.
#[system(name = "test::sets_counter", writes = [Counter])]
fn sets_counter(cmd: Commands) {
    cmd.set(net(1), Counter(1));
}

/// Reads `Counter`; no edge to `sets_counter`.
#[system(name = "test::reads_counter")]
fn reads_counter(_counter: &Counter) {}

#[test]
fn a_declared_write_is_a_write_for_conflicts() {
    let conflict = build_error(&[&SETS_COUNTER, &READS_COUNTER]);
    assert!(
        matches!(conflict, ScheduleError::UnorderedConflict { target: AccessTarget::Component(key), .. } if key == Counter::KEY),
        "{conflict:?}"
    );
    // A `writes` entry fetches nothing and does not make the system run per entity: it
    // runs once per tick, and its command applies.
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(1), REGION, (Counter(0),)).unwrap();
    let mut schedule = build(&[&SETS_COUNTER]).unwrap();
    let world = tick(&mut schedule, world, &executor);
    assert_eq!(world.get::<Counter>(net(1)), Some(&Counter(1)));
}

#[system(name = "test::write_b_ordered", after = write_a)]
fn write_b_ordered(_counter: Mut<Counter>) {}

#[system(name = "test::exclusive_a", exclusive)]
fn exclusive_a(_world: &mut World) {}

#[system(name = "test::exclusive_b", exclusive)]
fn exclusive_b(_world: &mut World) {}

#[system(name = "test::emitter")]
fn emitter(_damage: EventWriter<Damage>, _counter: &Counter) {}

#[system(name = "test::reader")]
fn reader(_damage: Events<Damage>) {}

#[system(name = "test::unregistered_reader")]
fn unregistered_reader(_value: &Unregistered) {}

#[system(name = "test::changed_untracked")]
fn changed_untracked(_changed: Changed<Velocity>, _velocity: &Velocity) {}

#[system(name = "test::writes_owner")]
fn writes_owner(_owner: Mut<OwnedBy>) {}

#[system(name = "test::writes_reference")]
fn writes_reference(_thrown: Mut<ThrownBy>) {}

#[system(name = "test::reads_scratch")]
fn reads_scratch(_scratch: &Scratch) {}

#[system(name = "test::writes_scratch")]
fn writes_scratch(_scratch: Mut<Scratch>) {}

#[system(name = "test::reads_scratch_after", after = writes_scratch)]
fn reads_scratch_after(_scratch: &Scratch) {}

#[system(name = "test::needs_weather")]
fn needs_weather(_weather: Res<Weather>) {}

fn build(systems: &[&'static SystemDescriptor]) -> Result<Schedule, ScheduleError> {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("only", &[]);
    for system in systems {
        builder.add(system);
    }
    builder.build()
}

fn build_error(systems: &[&'static SystemDescriptor]) -> ScheduleError {
    build(systems).map(|_| ()).unwrap_err()
}

fn prepare_error(systems: &[&'static SystemDescriptor]) -> ScheduleError {
    let world = world(WorldKind::Server, LayoutConfig::default());
    build(systems).unwrap().prepare(&world).unwrap_err()
}

#[test]
fn structural_errors() {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.add(&PLAIN);
    builder.stage("late", &[]);
    assert_eq!(
        builder.build().map(|_| ()).unwrap_err(),
        ScheduleError::NoStage { system: "test::plain" }
    );
    // A system whose worlds exclude the kind is skipped, so it cannot be an orphan.
    let mut builder = Schedule::builder(WorldKind::Client);
    builder.add(&PLAIN);
    assert!(builder.build().is_ok());

    assert_eq!(
        build_error(&[&DUP_A, &DUP_B]),
        ScheduleError::DuplicateKey { first: "test::dup", second: "test::dup", key: DUP_A.key() }
    );
    assert_eq!(
        build_error(&[&IN_ELSEWHERE]),
        ScheduleError::UnknownSet { system: "test::in_elsewhere", set: "test::elsewhere" }
    );
    assert_eq!(
        build_error(&[&AFTER_PLAIN]),
        ScheduleError::UnknownSystem { system: "test::after_plain", after: "test::plain" }
    );
}

static ORDER_A: SystemSet = SystemSet::new("test::order_a");
static ORDER_B: SystemSet = SystemSet::new("test::order_b");

#[system(name = "test::ordered_a", exclusive, in_set = ORDER_A)]
fn ordered_a(_world: &mut World) {}

#[system(name = "test::ordered_b", exclusive, in_set = ORDER_B)]
fn ordered_b(_world: &mut World) {}

/// An edge the assembler adds between two sets that do not name each other orders their
/// systems, in either direction; across stages it must agree with the stage order; a set no
/// stage declares is an error.
#[test]
fn an_assembler_orders_sets_that_do_not_name_each_other() {
    let names = |schedule: &Schedule| -> Vec<&'static str> {
        schedule.order().iter().map(|system| system.name()).collect()
    };
    let unordered = {
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("only", &[&ORDER_A, &ORDER_B]).add(&ORDERED_A).add(&ORDERED_B);
        builder.build().map(|_| ()).unwrap_err()
    };
    assert!(matches!(unordered, ScheduleError::UnorderedConflict { .. }), "{unordered:?}");

    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("only", &[&ORDER_A, &ORDER_B]).add(&ORDERED_A).add(&ORDERED_B);
    builder.order(&ORDER_A, &ORDER_B);
    assert_eq!(names(&builder.build().unwrap()), ["test::ordered_a", "test::ordered_b"]);

    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("only", &[&ORDER_A, &ORDER_B]).add(&ORDERED_A).add(&ORDERED_B);
    builder.order(&ORDER_B, &ORDER_A);
    assert_eq!(names(&builder.build().unwrap()), ["test::ordered_b", "test::ordered_a"]);

    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("only", &[&ORDER_A, &ORDER_B]).add(&ORDERED_A).add(&ORDERED_B);
    builder.order(&ORDER_A, &ORDER_B).order(&ORDER_B, &ORDER_A);
    assert!(matches!(builder.build().map(|_| ()).unwrap_err(), ScheduleError::Cycle { .. }));

    // Across stages: agreeing with the stage order is accepted, disagreeing is a cycle.
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("first", &[&ORDER_A]).add(&ORDERED_A);
    builder.stage("second", &[&ORDER_B]).add(&ORDERED_B);
    builder.order(&ORDER_A, &ORDER_B);
    assert_eq!(names(&builder.build().unwrap()), ["test::ordered_a", "test::ordered_b"]);
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("first", &[&ORDER_A]).add(&ORDERED_A);
    builder.stage("second", &[&ORDER_B]).add(&ORDERED_B);
    builder.order(&ORDER_B, &ORDER_A);
    assert_eq!(
        builder.build().map(|_| ()).unwrap_err(),
        ScheduleError::Cycle { members: vec!["test::order_b", "test::order_a"] }
    );

    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("only", &[&ORDER_A]).add(&ORDERED_A);
    builder.order(&ELSEWHERE, &ORDER_A);
    assert_eq!(
        builder.build().map(|_| ()).unwrap_err(),
        ScheduleError::UnknownOrderedSet {
            set: "test::elsewhere",
            before: "test::elsewhere",
            after: "test::order_a"
        }
    );
}

#[test]
fn parameter_errors() {
    let cases: [(&'static SystemDescriptor, ScheduleError); 10] = [
        (
            &EXCLUSIVE_ENTITY,
            ScheduleError::ExclusiveWithEntityAccess { system: "test::exclusive_entity" },
        ),
        (&KEY_ALONE, ScheduleError::EntityParamInWorldSystem { system: "test::key_alone" }),
        (&EXCLUSIVE_CTX, ScheduleError::RegionParamInWorldSystem { system: "test::exclusive_ctx" }),
        (
            &ENTITY_WRITES_RESOURCE,
            ScheduleError::ResourceWriteInKernel {
                system: "test::entity_writes_resource",
                resource: core::any::type_name::<Gravity>(),
            },
        ),
        (&ENTITY_JOB, ScheduleError::JobInKernel { system: "test::entity_job" }),
        (
            &ALIASED,
            ScheduleError::AliasedParameter { system: "test::aliased", component: Counter::KEY },
        ),
        (
            &WITHOUT_BESIDE,
            ScheduleError::AliasedParameter {
                system: "test::without_beside",
                component: Counter::KEY,
            },
        ),
        (
            &TWO_COMMANDS,
            ScheduleError::InvalidParameter { system: "test::two_commands", parameter: "Commands" },
        ),
        (
            &RESOURCE_TWICE,
            ScheduleError::InvalidParameter {
                system: "test::resource_twice",
                parameter: core::any::type_name::<Gravity>(),
            },
        ),
        (&STORE_ONLY, ScheduleError::RegionParamInWorldSystem { system: "test::finalize_ctx" }),
    ];
    for (system, expected) in cases {
        assert_eq!(build_error(&[system]), expected);
    }
}

/// A reduction whose finalize takes `Ctx`, which a finalize cannot.
#[system(name = "test::finalize_ctx", reduce = lactose_math::IntSum<u32>)]
fn store_only(_counter: &Counter) -> u32 {
    1
}

#[lactose_ecs::finalize(store_only)]
fn store_with_ctx(_sum: i128, _ctx: Ctx) {}

#[test]
fn order_errors() {
    assert!(matches!(
        build_error(&[&CYCLE_A, &CYCLE_B]),
        ScheduleError::Cycle { members } if members.len() == 2
    ));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("first", &[]).add(&AFTER_PLAIN);
    builder.stage("second", &[]).add(&PLAIN);
    assert_eq!(
        builder.build().map(|_| ()).unwrap_err(),
        ScheduleError::Cycle { members: vec!["test::after_plain", "test::plain"] }
    );
    static FIRST: SystemSet = SystemSet::new("test::first").after(&[&SECOND]);
    static SECOND: SystemSet = SystemSet::new("test::second").after(&[&FIRST]);
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("sets", &[&FIRST, &SECOND]);
    assert!(matches!(builder.build(), Err(ScheduleError::Cycle { .. })));

    let conflict = build_error(&[&WRITE_A, &WRITE_B]);
    let ScheduleError::UnorderedConflict { target, .. } = conflict else { panic!("{conflict:?}") };
    assert_eq!(target, AccessTarget::Component(Counter::KEY));
    assert!(build(&[&WRITE_A, &WRITE_B_ORDERED]).is_ok(), "an edge orders the pair");
    let exclusive = build_error(&[&EXCLUSIVE_A, &EXCLUSIVE_B]);
    assert!(matches!(
        exclusive,
        ScheduleError::UnorderedConflict { target: AccessTarget::World, .. }
    ));
    assert!(exclusive.to_string().contains("conflict on the world"), "{exclusive}");

    // A set belongs to one stage.
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("one", &[&FIRST_ALONE]);
    builder.stage("two", &[&FIRST_ALONE]);
    assert_eq!(
        builder.build().map(|_| ()).unwrap_err(),
        ScheduleError::SetInTwoStages { set: "test::first_alone", first: "one", second: "two" }
    );
    assert_eq!(
        build_error(&[&EMITTER, &READER]),
        ScheduleError::EventsBeforeEmitter {
            reader: "test::reader",
            emitter: "test::emitter",
            event: core::any::type_name::<Damage>(),
        }
    );
}

// ---- Channels (D126) ----

/// A per-entity writer of the physics channel.
#[system(name = "test::writes_physics")]
fn writes_physics(_counter: &Counter, physics: ChannelWriter<Physics>) {
    physics.apply_impulse(net(1), Vec2::ZERO);
}

/// The taker of the physics channel.
#[system(name = "test::takes_physics", exclusive)]
fn takes_physics(_world: &mut World, _taken: ChannelTake<Physics>) {}

/// A second taker of the physics channel.
#[system(name = "test::takes_physics_again", exclusive)]
fn takes_physics_again(_world: &mut World, _taken: ChannelTake<Physics>) {}

/// A once-per-tick reader of the physics channel.
#[system(name = "test::reads_physics")]
fn reads_physics(_impulses: Channel<Physics>) {}

/// An exclusive reader of the physics channel ordered before the taker in its stage.
#[system(name = "test::reads_physics_first", exclusive)]
fn reads_physics_first(_world: &mut World, _impulses: Channel<Physics>) {}

#[system(name = "test::takes_after_reader", exclusive, after = reads_physics_first)]
fn takes_after_reader(_world: &mut World, _taken: ChannelTake<Physics>) {}

/// An exclusive writer, and a taker ordered after it in its stage.
#[system(name = "test::exclusive_writer", exclusive)]
fn exclusive_writer(_world: &mut World, _physics: ChannelWriter<Physics>) {}

#[system(name = "test::takes_after_writer", exclusive, after = exclusive_writer)]
fn takes_after_writer(_world: &mut World, _taken: ChannelTake<Physics>) {}

#[system(name = "test::entity_takes")]
fn entity_takes(_counter: &Counter, _taken: ChannelTake<Physics>) {}

#[system(name = "test::once_takes")]
fn once_takes(_taken: ChannelTake<Physics>) {}

#[system(name = "test::finalize_takes", reduce = lactose_math::IntSum<u32>)]
fn finalize_takes(_counter: &Counter) -> u32 {
    1
}

#[lactose_ecs::finalize(finalize_takes)]
fn store_taking(_sum: i128, _taken: ChannelTake<Physics>) {}

#[system(name = "test::takes_twice", exclusive)]
fn takes_twice(_world: &mut World, _a: ChannelTake<Physics>, _b: ChannelTake<Physics>) {}

/// Review probe r6a: a take beside a reader of the same channel, which would read what the
/// take removed.
#[system(name = "test::takes_and_reads", exclusive)]
fn takes_and_reads(_world: &mut World, _read: Channel<Physics>, _taken: ChannelTake<Physics>) {}

#[system(name = "test::takes_and_writes", exclusive)]
fn takes_and_writes(
    _world: &mut World,
    _taken: ChannelTake<Physics>,
    _physics: ChannelWriter<Physics>,
) {
}

/// Builds one stage per entry, in order.
fn staged(stages: &[&[&'static SystemDescriptor]]) -> Result<Schedule, ScheduleError> {
    const NAMES: [&str; 4] = ["first", "second", "third", "fourth"];
    let mut builder = Schedule::builder(WorldKind::Server);
    for (stage, systems) in stages.iter().enumerate() {
        builder.stage(NAMES[stage], &[]);
        for system in *systems {
            builder.add(system);
        }
    }
    builder.build()
}

fn staged_error(stages: &[&[&'static SystemDescriptor]]) -> ScheduleError {
    staged(stages).map(|_| ()).unwrap_err()
}

fn after_taker(system: &'static str, taker: &'static str) -> ScheduleError {
    ScheduleError::ChannelAfterTaker { system, taker, channel: core::any::type_name::<Physics>() }
}

#[test]
fn a_channel_take_is_an_exclusive_systems_alone_once_and_beside_nothing_on_its_channel() {
    let refused =
        |system: &'static str| ScheduleError::InvalidParameter { system, parameter: "ChannelTake" };
    assert_eq!(build_error(&[&ENTITY_TAKES]), refused("test::entity_takes"));
    assert_eq!(build_error(&[&ONCE_TAKES]), refused("test::once_takes"));
    assert_eq!(build_error(&[&FINALIZE_TAKES]), refused("test::finalize_takes"));
    assert_eq!(build_error(&[&TAKES_TWICE]), refused("test::takes_twice"));
    assert_eq!(build_error(&[&TAKES_AND_READS]), refused("test::takes_and_reads"));
    assert_eq!(build_error(&[&TAKES_AND_WRITES]), refused("test::takes_and_writes"));
}

#[test]
fn a_writer_in_the_takers_stage_or_a_later_one_is_refused() {
    // Review probe r6b's writer: one stage with the taker.
    assert_eq!(
        staged_error(&[&[&TAKES_PHYSICS, &WRITES_PHYSICS], &[&READS_PHYSICS]]),
        after_taker("test::writes_physics", "test::takes_physics")
    );
    assert_eq!(
        staged_error(&[&[&TAKES_PHYSICS], &[&WRITES_PHYSICS], &[&READS_PHYSICS]]),
        after_taker("test::writes_physics", "test::takes_physics")
    );
    // Ordered before the taker, an exclusive writer's entries still apply at the sync point
    // that ends the stage, after the take.
    assert_eq!(
        staged_error(&[&[&EXCLUSIVE_WRITER, &TAKES_AFTER_WRITER], &[&READS_PHYSICS]]),
        after_taker("test::exclusive_writer", "test::takes_after_writer")
    );
}

#[test]
fn a_reader_or_a_second_taker_after_the_taker_is_refused() {
    // Review probe r6b's reader: a later stage.
    assert_eq!(
        staged_error(&[&[&WRITES_PHYSICS], &[&TAKES_PHYSICS], &[&READS_PHYSICS]]),
        after_taker("test::reads_physics", "test::takes_physics")
    );
    // The taker's stage: a system that is not exclusive runs after every exclusive one.
    assert_eq!(
        staged_error(&[&[&WRITES_PHYSICS], &[&TAKES_PHYSICS, &READS_PHYSICS]]),
        after_taker("test::reads_physics", "test::takes_physics")
    );
    assert_eq!(
        staged_error(&[&[&WRITES_PHYSICS], &[&TAKES_PHYSICS], &[&TAKES_PHYSICS_AGAIN]]),
        after_taker("test::takes_physics_again", "test::takes_physics")
    );
    let message = after_taker("test::reads_physics", "test::takes_physics").to_string();
    assert_eq!(
        message,
        format!(
            "`test::reads_physics` uses channel `{}` after its taker `test::takes_physics`: a \
             writer's entries would land after the take, and a reader or a second taker would \
             find it empty; place it before the taker",
            core::any::type_name::<Physics>()
        )
    );
}

#[test]
fn a_writer_needs_a_taker_or_a_reader_in_a_later_stage() {
    let without = ScheduleError::ChannelWithoutConsumer {
        writer: "test::writes_physics",
        channel: core::any::type_name::<Physics>(),
    };
    assert_eq!(build_error(&[&WRITES_PHYSICS]), without);
    // A reader in the writer's stage sees earlier stages' entries alone.
    assert_eq!(build_error(&[&WRITES_PHYSICS, &READS_PHYSICS]), without);
    assert_eq!(staged_error(&[&[&READS_PHYSICS], &[&WRITES_PHYSICS]]), without);
    assert_eq!(
        without.to_string(),
        format!(
            "`test::writes_physics` writes channel `{}`, which no system of a later stage takes \
             or reads",
            core::any::type_name::<Physics>()
        )
    );
}

#[test]
fn writers_and_readers_before_the_taker_build() {
    staged(&[&[&WRITES_PHYSICS], &[&TAKES_PHYSICS]]).unwrap();
    staged(&[&[&WRITES_PHYSICS], &[&READS_PHYSICS], &[&TAKES_PHYSICS]]).unwrap();
    // An exclusive reader ordered before the taker in its stage reads before the take.
    staged(&[&[&WRITES_PHYSICS], &[&READS_PHYSICS_FIRST, &TAKES_AFTER_READER]]).unwrap();
    staged(&[&[&WRITES_PHYSICS], &[&READS_PHYSICS]]).unwrap();
}

#[test]
fn errors_that_need_a_world() {
    let world = world(WorldKind::Prediction, LayoutConfig::default());
    assert_eq!(
        build(&[&PLAIN]).unwrap().prepare(&world),
        Err(ScheduleError::WrongWorld {
            schedule: WorldKind::Server,
            world: WorldKind::Prediction
        })
    );
    assert_eq!(
        prepare_error(&[&UNREGISTERED_READER]),
        ScheduleError::UnregisteredComponent {
            system: "test::unregistered_reader",
            component: Unregistered::KEY,
        }
    );
    assert_eq!(
        prepare_error(&[&CHANGED_UNTRACKED]),
        ScheduleError::ChangedOnUntracked {
            system: "test::changed_untracked",
            component: Velocity::KEY
        }
    );
    assert_eq!(
        prepare_error(&[&WRITES_OWNER]),
        ScheduleError::WritesMaintainedComponent {
            system: "test::writes_owner",
            component: OwnedBy::KEY
        }
    );
    assert_eq!(
        prepare_error(&[&WRITES_REFERENCE]),
        ScheduleError::WritesMaintainedComponent {
            system: "test::writes_reference",
            component: ThrownBy::KEY,
        }
    );
    assert_eq!(
        prepare_error(&[&READS_SCRATCH]),
        ScheduleError::TransientBeforeWriter {
            reader: "test::reads_scratch",
            component: Scratch::KEY
        }
    );
    let world = self::world(WorldKind::Server, LayoutConfig::default());
    build(&[&WRITES_SCRATCH, &READS_SCRATCH_AFTER]).unwrap().prepare(&world).unwrap();
    assert_eq!(
        prepare_error(&[&NEEDS_WEATHER]),
        ScheduleError::MissingResource {
            system: "test::needs_weather",
            resource: core::any::type_name::<Weather>(),
        }
    );
}

#[test]
fn every_error_has_a_message() {
    let errors = [
        build_error(&[&DUP_A, &DUP_B]),
        build_error(&[&CYCLE_A, &CYCLE_B]),
        build_error(&[&WRITE_A, &WRITE_B]),
        prepare_error(&[&NEEDS_WEATHER]),
    ];
    for error in errors {
        assert!(error.to_string().contains("test::"), "{error}");
    }
}

#[test]
fn run_tick_fails_on_prepare_and_on_shutdown() {
    let executor = executor();
    let world = world(WorldKind::Server, LayoutConfig::default());
    let mut schedule = build(&[&NEEDS_WEATHER]).unwrap();
    let error = executor.block_on(schedule.run_tick(world, &executor)).map(|_| ()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.message().contains("test::needs_weather"));

    let world = self::world(WorldKind::Server, LayoutConfig::default());
    let mut schedule = build(&[&PLAIN]).unwrap();
    let other = executor.clone();
    other.shutdown();
    let error = executor.block_on(schedule.run_tick(world, &executor)).map(|_| ()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled);
}

/// A schedule serves one world (review of the implementation, M4): its pass windows are
/// kept per schedule, so a second world would lose changes to the first world's marks.
#[test]
fn a_schedule_serves_the_one_world_it_first_ran_with() {
    let executor = executor();
    let mut pass = build(&[&PLAIN]).unwrap();
    let first = world(WorldKind::Server, LayoutConfig::default());
    let second = world(WorldKind::Server, LayoutConfig::default());
    let first = executor.block_on(pass.run_pass(first, &executor)).unwrap();
    assert_eq!(
        pass.prepare(&second),
        Err(ScheduleError::WrongWorld { schedule: WorldKind::Server, world: WorldKind::Server })
    );
    let error = executor.block_on(pass.run_pass(second, &executor)).map(|_| ()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(pass.prepare(&first).is_ok(), "the first world is still served");
}
