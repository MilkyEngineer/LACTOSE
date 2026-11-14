//! Hand-written per-entity kernels whose `each` disagrees with the access table they were
//! built with, which only a broken caller of the `unsafe` `SystemDescriptor::new` can make.
//! Each parameter checks its table entry once per unit, so such a kernel panics before it
//! reads a row instead of taking what was lent for one entry as another type. A panic in a
//! job aborts the process, so each case runs in a child process and the test reads how it
//! died, as `lactose-task`'s abort tests do.

// Miri cannot spawn processes.
#![cfg(not(miri))]

mod common;

#[expect(clippy::disallowed_types, reason = "a test that runs its own binary as a child")]
use std::process::Command;

use common::*;
use lactose_ecs::__private::{Invocation, Kernel, Unit, column, next_row, row};
use lactose_ecs::{
    Access, AccessKind, AccessTarget, Event, EventWriter, LayoutConfig, Mut, RegionRes, Schedule,
    SystemDescriptor, World, WorldKind, WorldSet,
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

/// In the child process `test` runs in, runs `system` for one tick. In the parent, runs
/// `test` again as a child and asserts that it aborted saying `expected`.
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a test that runs its own binary as a child, told the case by the environment"
)]
fn aborts_running(test: &str, system: &'static SystemDescriptor, expected: &str) {
    if std::env::var(CHILD_ENV).is_ok_and(|case| case == test) {
        let executor = executor();
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("simulation", &[]).add(system);
        let mut schedule = builder.build().unwrap();
        tick(&mut schedule, rows(), &executor);
        panic!("the run finished, so the kernel read what its table did not lend it");
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture", "--test-threads", "1"])
        .env(CHILD_ENV, test)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(expected), "{test}: the child did not say {expected:?}:\n{stderr}");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(6), "{test}: expected SIGABRT:\n{stderr}");
    }
    #[cfg(not(unix))]
    assert!(!output.status.success(), "{test}: the child survived");
}

fn mass_target() -> AccessTarget {
    AccessTarget::Component(<Mass as Reflect>::KEY)
}

/// Lends parameter 0 as a written `Mass` column; the kernel takes it as `Mut<Health>`, a
/// type whose every bit pattern is valid. Without the check the cast would succeed and
/// the kernel would write the first half of the `Mass` column.
static WRITES_MASS: [Access; 1] = [Access::new(AccessKind::Write, mass_target)];

fn each_health(unit: &mut Unit<'_>) {
    let mut health = column(unit, 0);
    while let Some(mut r) = next_row(unit) {
        let mut health: Mut<Health> = row(&mut health, &mut r);
        health.0 = 1;
    }
}

// SAFETY: none; the kernel breaks the obligation on purpose, which the run turns into a
// panic before any row is read.
static COLUMN_AS_ANOTHER: SystemDescriptor = unsafe {
    SystemDescriptor::new(
        "obligation::column_as_another",
        WorldSet::SERVER,
        None,
        &[],
        &WRITES_MASS,
        &[],
        false,
        None,
        Kernel { run: never, each: each_health, finalize: nothing, reducer: None },
    )
};

#[test]
fn a_kernel_that_takes_a_lent_column_as_another_component_panics() {
    aborts_running(
        "a_kernel_that_takes_a_lent_column_as_another_component_panics",
        &COLUMN_AS_ANOTHER,
        "parameter 0 does not match the system's access table",
    );
}

fn weather_target() -> AccessTarget {
    AccessTarget::RegionResource(<Weather as Reflect>::KEY)
}

/// Lends parameter 1 as the region's `Weather`; the kernel takes it as `RegionScratch`, a
/// region resource of the same size.
static READS_WEATHER: [Access; 2] =
    [Access::new(AccessKind::Read, mass_target), Access::new(AccessKind::Read, weather_target)];

fn each_scratch(unit: &mut Unit<'_>) {
    let mut mass = column(unit, 0);
    let mut scratch = column(unit, 1);
    while let Some(mut r) = next_row(unit) {
        let _: &Mass = row(&mut mass, &mut r);
        let _: RegionRes<'_, RegionScratch> = row(&mut scratch, &mut r);
    }
}

// SAFETY: none, as above.
static REGION_RESOURCE_AS_ANOTHER: SystemDescriptor = unsafe {
    SystemDescriptor::new(
        "obligation::region_resource_as_another",
        WorldSet::SERVER,
        None,
        &[],
        &READS_WEATHER,
        &[],
        false,
        None,
        Kernel { run: never, each: each_scratch, finalize: nothing, reducer: None },
    )
};

#[test]
fn a_kernel_that_takes_a_region_resource_as_another_panics() {
    aborts_running(
        "a_kernel_that_takes_a_region_resource_as_another_panics",
        &REGION_RESOURCE_AS_ANOTHER,
        "parameter 1 does not match the system's access table",
    );
}

/// An event no table below declares.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Heal {
    target: lactose_core::StableKey,
}

impl Event for Heal {
    fn target(&self) -> lactose_core::StableKey {
        self.target
    }
}

fn damage_target() -> AccessTarget {
    <EventWriter<'static, Damage> as lactose_ecs::SystemParam>::ACCESS.target()
}

/// Lends parameter 1 as the sink of `Damage`; the kernel emits `Heal` into it.
static EMITS_DAMAGE: [Access; 2] = [
    Access::new(AccessKind::Read, mass_target),
    Access::new(AccessKind::EmitEvents, damage_target),
];

fn each_heal(unit: &mut Unit<'_>) {
    let mut mass = column(unit, 0);
    let mut heals = column(unit, 1);
    while let Some(mut r) = next_row(unit) {
        let _: &Mass = row(&mut mass, &mut r);
        let mut heals: EventWriter<'_, Heal> = row(&mut heals, &mut r);
        heals.emit(Heal { target: net(1) });
    }
}

// SAFETY: none, as above.
static EVENT_AS_ANOTHER: SystemDescriptor = unsafe {
    SystemDescriptor::new(
        "obligation::event_as_another",
        WorldSet::SERVER,
        None,
        &[],
        &EMITS_DAMAGE,
        &[],
        false,
        None,
        Kernel { run: never, each: each_heal, finalize: nothing, reducer: None },
    )
};

#[test]
fn a_kernel_that_emits_another_event_into_a_lent_sink_panics() {
    aborts_running(
        "a_kernel_that_emits_another_event_into_a_lent_sink_panics",
        &EVENT_AS_ANOTHER,
        "parameter 1 does not match the system's access table",
    );
}
