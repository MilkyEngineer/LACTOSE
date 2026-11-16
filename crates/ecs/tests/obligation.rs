//! Hand-written per-entity kernels whose `each` disagrees with the access table they were
//! built with, which only a broken caller of the `unsafe` `SystemDescriptor::new` can make.
//! Each parameter checks its table entry once per unit, so such a kernel panics before it
//! reads a row instead of taking what was prepared for one entry as another type. A panic
//! in a job aborts the process, so each case runs in a child process and the test reads how
//! it died, as `lactose-task`'s abort tests do.

// Miri cannot spawn processes.
#![cfg(not(miri))]

use crate::common;

use std::io::Read;
#[expect(clippy::disallowed_types, reason = "a test that runs its own binary as a child")]
use std::process::{Command, Stdio};
use std::time::Duration;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::__private::{Invocation, Kernel, Unit, column, next_row, row};
use lactose_ecs::{
    Access, AccessKind, AccessTarget, Channel, CommandChannel, Event, EventWriter, Events,
    LayoutConfig, Mut, RegionRes, Res, Schedule, SystemDescriptor, SystemParam, World, WorldKind,
    WorldSet,
};
use lactose_math::Scalar;
use lactose_reflect::Reflect;

fn never(_: &mut Invocation<'_, '_>) {
    unreachable!("a per-entity system runs through `each`")
}

fn nothing(_: &mut Invocation<'_, '_>) {}

/// A server world of twenty rows with `Health(100)` and `Mass(5)`, and `Weather(3)` in the
/// region.
fn rows() -> World {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    for id in 1..=20 {
        world.spawn(net(id), REGION, (Health(100), Mass(Scalar::from_i32(5)))).unwrap();
    }
    world.insert_region_resource(REGION, Weather(3)).unwrap();
    world
}

const CHILD_ENV: &str = "ECS_OBLIGATION_CASE";

/// How long a child may run before the test kills it: a regression that unwinds instead of
/// aborting could leave the run waiting for ever.
const LIMIT: Duration = Duration::from_secs(60);

/// In the child process `test` runs in, runs `system` for one tick. In the parent, runs
/// `test` again as a child and asserts that it aborted saying `expected`.
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a test that runs its own binary as a child, told the case by the environment, \
              and bounds its wait by the wall clock"
)]
fn aborts_running(test: &str, system: &'static SystemDescriptor, expected: &str) {
    if std::env::var(CHILD_ENV).is_ok_and(|case| case == test) {
        let executor = executor();
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("simulation", &[]).add(system);
        let mut schedule = builder.build().unwrap();
        tick(&mut schedule, rows(), &executor);
        panic!("the run finished, so the kernel read what its table did not declare");
    }
    // The harness names a test by its path below the crate root.
    let path = module_path!()
        .split_once("::")
        .map_or_else(|| test.to_owned(), |(_, module)| format!("{module}::{test}"));
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", path.as_str(), "--nocapture", "--test-threads", "1"])
        .env(CHILD_ENV, test)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut stderr = Vec::new();
        let _ = pipe.read_to_end(&mut stderr);
        stderr
    });
    let begun = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if begun.elapsed() >= LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{test}: the child neither aborted nor exited in {LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stderr = reader.join().unwrap();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains(expected), "{test}: the child did not say {expected:?}:\n{stderr}");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(6), "{test}: expected SIGABRT:\n{stderr}");
    }
    #[cfg(not(unix))]
    assert!(!status.success(), "{test}: the child survived");
}

/// A per-entity descriptor built from `table` and `each`, which disagree.
macro_rules! lying {
    ($descriptor:ident, $name:literal, $table:ident, $each:ident) => {
        // SAFETY: none; the kernel breaks the obligation on purpose, which the run turns
        // into a panic before any row is read.
        static $descriptor: SystemDescriptor = unsafe {
            SystemDescriptor::new(
                $name,
                WorldSet::SERVER,
                None,
                &[],
                &$table,
                &[],
                false,
                None,
                Kernel { run: never, each: $each, finalize: nothing, reducer: None },
            )
        };
    };
}

const MISMATCH_0: &str = "parameter 0 does not match the system's access table";
const MISMATCH_1: &str = "parameter 1 does not match the system's access table";

fn mass_target() -> AccessTarget {
    AccessTarget::Component(<Mass as Reflect>::KEY)
}

fn health_target() -> AccessTarget {
    AccessTarget::Component(<Health as Reflect>::KEY)
}

/// Every case but the first reads `Mass` as parameter 0, so the system is per-entity.
const READS_MASS: Access = Access::new(AccessKind::Read, mass_target);

/// Lends parameter 0 as a written `Mass` column; the kernel takes it as `Mut<Health>`, a
/// type whose every bit pattern is valid. The length assert would stop it too, `Health`
/// being half the size; without both, the cast would write the first half of the `Mass`
/// column.
static WRITES_MASS: [Access; 1] = [Access::new(AccessKind::Write, mass_target)];

fn each_health(unit: &mut Unit<'_>) {
    let mut health = column(unit, 0);
    while let Some(mut r) = next_row(unit) {
        let mut health: Mut<Health> = row(&mut health, &mut r);
        health.0 = 1;
    }
}

lying!(COLUMN_AS_ANOTHER, "obligation::column_as_another", WRITES_MASS, each_health);

#[test]
fn a_kernel_that_takes_a_lent_column_as_another_component_panics() {
    aborts_running(
        "a_kernel_that_takes_a_lent_column_as_another_component_panics",
        &COLUMN_AS_ANOTHER,
        MISMATCH_0,
    );
}

/// Lends parameter 0 as an optionally written `Health` column; the kernel takes it as
/// `Option<Mut<Scratch>>`, a `u32` the size of `Health`'s `i32`, so only the entry's
/// component stops it.
static MAY_WRITE_HEALTH: [Access; 1] = [Access::new(AccessKind::OptionalWrite, health_target)];

fn each_scratch(unit: &mut Unit<'_>) {
    let mut scratch = column(unit, 0);
    while let Some(mut r) = next_row(unit) {
        let scratch: Option<Mut<Scratch>> = row(&mut scratch, &mut r);
        if let Some(mut scratch) = scratch {
            scratch.0 = u32::MAX;
        }
    }
}

lying!(OPTION_AS_ANOTHER, "obligation::option_as_another", MAY_WRITE_HEALTH, each_scratch);

#[test]
fn a_kernel_that_takes_an_optional_column_as_another_component_of_its_size_panics() {
    aborts_running(
        "a_kernel_that_takes_an_optional_column_as_another_component_of_its_size_panics",
        &OPTION_AS_ANOTHER,
        MISMATCH_0,
    );
}

fn weather_target() -> AccessTarget {
    AccessTarget::RegionResource(<Weather as Reflect>::KEY)
}

/// Lends parameter 1 as the region's `Weather`; the kernel takes it as `RegionScratch`, a
/// region resource of the same size.
static READS_WEATHER: [Access; 2] = [READS_MASS, Access::new(AccessKind::Read, weather_target)];

fn each_region_scratch(unit: &mut Unit<'_>) {
    let mut mass = column(unit, 0);
    let mut scratch = column(unit, 1);
    while let Some(mut r) = next_row(unit) {
        let _: &Mass = row(&mut mass, &mut r);
        let _: RegionRes<'_, RegionScratch> = row(&mut scratch, &mut r);
    }
}

lying!(
    REGION_RESOURCE_AS_ANOTHER,
    "obligation::region_resource_as_another",
    READS_WEATHER,
    each_region_scratch
);

#[test]
fn a_kernel_that_takes_a_region_resource_as_another_panics() {
    aborts_running(
        "a_kernel_that_takes_a_region_resource_as_another_panics",
        &REGION_RESOURCE_AS_ANOTHER,
        MISMATCH_1,
    );
}

/// Lends parameter 1 as the world's `Gravity`; the kernel takes it as `TickDelta`.
static READS_GRAVITY: [Access; 2] = [READS_MASS, <Res<'static, Gravity> as SystemParam>::ACCESS];

fn each_tick_delta(unit: &mut Unit<'_>) {
    let mut mass = column(unit, 0);
    let mut delta = column(unit, 1);
    while let Some(mut r) = next_row(unit) {
        let _: &Mass = row(&mut mass, &mut r);
        let _: Res<'_, TickDelta> = row(&mut delta, &mut r);
    }
}

lying!(RESOURCE_AS_ANOTHER, "obligation::resource_as_another", READS_GRAVITY, each_tick_delta);

#[test]
fn a_kernel_that_takes_a_resource_as_another_panics() {
    aborts_running(
        "a_kernel_that_takes_a_resource_as_another_panics",
        &RESOURCE_AS_ANOTHER,
        MISMATCH_1,
    );
}

/// An event only the tables below name.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Heal {
    target: StableKey,
}

impl Event for Heal {
    fn target(&self) -> StableKey {
        self.target
    }
}

/// Lends parameter 1 as the sink of `Damage`; the kernel emits `Heal` into it.
static EMITS_DAMAGE: [Access; 2] =
    [READS_MASS, <EventWriter<'static, Damage> as SystemParam>::ACCESS];

fn each_heal(unit: &mut Unit<'_>) {
    let mut mass = column(unit, 0);
    let mut heals = column(unit, 1);
    while let Some(mut r) = next_row(unit) {
        let _: &Mass = row(&mut mass, &mut r);
        let mut heals: EventWriter<'_, Heal> = row(&mut heals, &mut r);
        heals.emit(Heal { target: net(1) });
    }
}

lying!(EVENT_AS_ANOTHER, "obligation::event_as_another", EMITS_DAMAGE, each_heal);

#[test]
fn a_kernel_that_emits_another_event_into_a_lent_sink_panics() {
    aborts_running(
        "a_kernel_that_emits_another_event_into_a_lent_sink_panics",
        &EVENT_AS_ANOTHER,
        MISMATCH_1,
    );
}

/// Declares parameter 1 as the reader of `Heal`; the kernel reads `Damage`, a list of
/// another type that the declared entry would not have ordered it after.
static READS_HEALS: [Access; 2] = [READS_MASS, <Events<'static, Heal> as SystemParam>::ACCESS];

fn each_damage(unit: &mut Unit<'_>) {
    let mut mass = column(unit, 0);
    let mut damage = column(unit, 1);
    while let Some(mut r) = next_row(unit) {
        let _: &Mass = row(&mut mass, &mut r);
        let damage: Events<'_, Damage> = row(&mut damage, &mut r);
        let _ = damage.iter().count();
    }
}

lying!(EVENTS_AS_ANOTHER, "obligation::events_as_another", READS_HEALS, each_damage);

#[test]
fn a_kernel_that_reads_another_events_list_panics() {
    aborts_running(
        "a_kernel_that_reads_another_events_list_panics",
        &EVENTS_AS_ANOTHER,
        MISMATCH_1,
    );
}

/// A channel only the table below names.
struct Audio;

impl CommandChannel for Audio {
    type Entry = u32;

    fn remap(_: &mut u32, _: &mut dyn FnMut(&mut StableKey)) {}
}

/// Declares parameter 1 as the reader of `Audio`; the kernel reads `Physics`.
static READS_AUDIO: [Access; 2] = [READS_MASS, <Channel<'static, Audio> as SystemParam>::ACCESS];

fn each_physics(unit: &mut Unit<'_>) {
    let mut mass = column(unit, 0);
    let mut physics = column(unit, 1);
    while let Some(mut r) = next_row(unit) {
        let _: &Mass = row(&mut mass, &mut r);
        let physics: Channel<'_, Physics> = row(&mut physics, &mut r);
        let _ = physics.iter().count();
    }
}

lying!(CHANNEL_AS_ANOTHER, "obligation::channel_as_another", READS_AUDIO, each_physics);

#[test]
fn a_kernel_that_reads_another_channel_panics() {
    aborts_running("a_kernel_that_reads_another_channel_panics", &CHANNEL_AS_ANOTHER, MISMATCH_1);
}
