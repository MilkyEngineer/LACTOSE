//! `SimulationBuilder::check` (README, "Runs" and the builder): called once at build, after
//! every registration function and before any schedule is built or prepared, with the
//! registry and every system of the builder's kind in registration order; its error fails
//! the build as returned; without one, or with one that passes, the build is unchanged.
//!
//! `lactose_predict::check_prediction_reads` is still `todo!()` in the predict facade, so a
//! local check function stands in for it here.

#![forbid(unsafe_code)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::*;
use lactose_core::{Error, ErrorKind, Result};
use lactose_ecs::{Res, SystemDescriptor, World, WorldKind, system};
use lactose_reflect::Registry;
use lactose_sim::{Pass, Registration, SimulationBuilder, Stage, TickInput};

#[system(name = "test::post_one", worlds = [server, prediction])]
fn post_one(_health: &Health) {}

#[system(name = "test::post_two", worlds = [server])]
fn post_two(_health: &Health) {}

#[system(name = "test::sim_one", worlds = [server, prediction])]
fn sim_one(_position: &Position) {}

#[system(name = "test::sim_two", worlds = [prediction])]
fn sim_two(_position: &Position) {}

#[system(name = "test::client_only", worlds = [client])]
fn client_only(_position: &Position) {}

#[system(name = "test::extract_one", worlds = [server, prediction], exclusive)]
fn extract_one(_world: &mut World) {}

/// Names a resource no setup function inserts, so preparing the tick schedule fails.
#[system(name = "test::needs_gravity", worlds = [server, prediction])]
fn needs_gravity(_gravity: Res<Gravity>, _position: &Position) {}

/// Adds across stages and a pass, interleaved, so registration order is not stage order.
fn register_first(registration: &mut Registration) {
    registration.stage(Stage::Post).add(&POST_ONE);
    registration.stage(Stage::Simulation).add(&SIM_ONE).add(&CLIENT_ONLY);
    registration.pass(Pass::Extract).add(&EXTRACT_ONE);
}

fn register_second(registration: &mut Registration) {
    registration.stage(Stage::Post).add(&POST_TWO);
    registration.stage(Stage::Simulation).add(&SIM_TWO);
}

const INPUT_SYSTEMS: [&str; 6] = [
    "sim::begin_tick",
    "sim::apply_frame_regions",
    "sim::apply_sessions",
    "sim::apply_inputs",
    "sim::apply_staged_state",
    "sim::apply_region_policy",
];

/// A builder with the two registration functions and a check that records what it saw.
fn recording(kind: WorldKind, seen: &Arc<Mutex<Vec<Vec<&'static str>>>>) -> SimulationBuilder {
    let seen = Arc::clone(seen);
    SimulationBuilder::new(kind, types())
        .check(move |registry: &Registry, systems: &[&'static SystemDescriptor]| {
            assert!(registry.contains::<Health>(), "the world's registry");
            seen.lock().unwrap().push(systems.iter().map(|system| system.name()).collect());
            Ok(())
        })
        .systems(register_first)
        .systems(register_second)
}

#[test]
fn a_check_sees_every_system_of_the_kind_once_in_registration_order() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    recording(WorldKind::Server, &seen).build().unwrap();
    let mut server: Vec<&str> = INPUT_SYSTEMS.to_vec();
    server.extend([
        "test::post_one",
        "test::sim_one",
        "test::extract_one",
        "test::post_two",
        "sim::save_copy",
    ]);
    assert_eq!(*seen.lock().unwrap(), [server]);

    let seen = Arc::new(Mutex::new(Vec::new()));
    recording(WorldKind::Prediction, &seen).build().unwrap();
    let mut prediction: Vec<&str> = INPUT_SYSTEMS.to_vec();
    prediction.extend(["test::post_one", "test::sim_one", "test::extract_one", "test::sim_two"]);
    assert_eq!(*seen.lock().unwrap(), [prediction]);

    let seen = Arc::new(Mutex::new(Vec::new()));
    recording(WorldKind::Client, &seen).build().unwrap();
    let mut client: Vec<&str> = INPUT_SYSTEMS.to_vec();
    client.push("test::client_only");
    assert_eq!(*seen.lock().unwrap(), [client]);
}

#[test]
fn checks_run_after_every_registration_function_in_the_order_added() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let record = |entry: &'static str| {
        let log = Arc::clone(&log);
        move || log.lock().unwrap().push(entry)
    };
    let (first, second, check_a, check_b) =
        (record("first"), record("second"), record("check a"), record("check b"));
    SimulationBuilder::new(WorldKind::Server, types())
        .check(move |_: &Registry, _: &[&'static SystemDescriptor]| {
            check_a();
            Ok(())
        })
        .systems(move |registration| {
            first();
            register_first(registration);
        })
        .check(move |_: &Registry, _: &[&'static SystemDescriptor]| {
            check_b();
            Ok(())
        })
        .systems(move |registration| {
            second();
            register_second(registration);
        })
        .build()
        .unwrap();
    assert_eq!(*log.lock().unwrap(), ["first", "second", "check a", "check b"]);
}

#[test]
fn a_failing_check_fails_the_build_with_its_error_before_anything_is_prepared() {
    // Without a check, the missing resource fails the build when the schedule prepares.
    let unprepared = || {
        SimulationBuilder::new(WorldKind::Server, types()).systems(|registration| {
            registration.stage(Stage::Simulation).add(&NEEDS_GRAVITY);
        })
    };
    let error = unprepared().build().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains("which the world does not hold"), "{error}");

    // A failing check fails it first, with its own error, so nothing was prepared.
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let error = unprepared()
        .check(move |_: &Registry, systems: &[&'static SystemDescriptor]| -> Result<()> {
            counted.fetch_add(1, Ordering::Relaxed);
            let refused = systems.iter().find(|system| system.name() == "test::needs_gravity");
            match refused {
                Some(system) => Err(Error::new(
                    ErrorKind::Unsupported,
                    format!("`{}` is refused by the check", system.name()),
                )),
                None => Ok(()),
            }
        })
        .check(|_: &Registry, _: &[&'static SystemDescriptor]| -> Result<()> {
            panic!("a check after a failing one ran")
        })
        .build()
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.to_string().contains("`test::needs_gravity` is refused by the check"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    // A check that passes leaves the prepare to fail as before.
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let error = unprepared()
        .check(move |_: &Registry, _: &[&'static SystemDescriptor]| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .build()
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn a_passing_check_leaves_the_build_unchanged() {
    let executor = executor();
    let region = region_at(0, 0);
    let mut load = TickInput::default();
    load.loaded.insert(region, fresh(region));
    let passing = |_: &Registry, _: &[&'static SystemDescriptor]| Ok(());
    let mut plain = game(WorldKind::Server).build().unwrap();
    let mut checked = game(WorldKind::Server).check(passing).build().unwrap();
    for tick in 0..5 {
        let frame = if tick == 0 { load.clone() } else { TickInput::default() };
        plain = step(&executor, plain, frame.clone());
        checked = step(&executor, checked, frame);
        assert_eq!(plain.hash().unwrap(), checked.hash().unwrap());
    }
}
