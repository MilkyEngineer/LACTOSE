//! The build check of D121: what a prediction-world system may name.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{ErrorKind, SystemKey};
use lactose_ecs::{Mut, SystemDescriptor, SystemSet, World, WorldKind, finalize, system};
use lactose_input::{PlayerInput, PlayerInputApplier};
use lactose_math::IntSum;
use lactose_predict::{
    DEFAULT_DEPTH, EXCLUSIVE_SYSTEMS, Predictor, check_prediction_reads, prediction_may_name,
};
use lactose_reflect::Reflect;
use lactose_sim::{SimulationBuilder, Stage, TypeTable};

#[system(name = "check::reminisce", worlds = [server, prediction])]
fn reminisce(diary: &Diary, health: Mut<Health>) {
    health.0 += i32::try_from(diary.0).unwrap_or(0);
}

#[system(name = "check::write_diary", worlds = [server])]
fn write_diary(diary: Mut<Diary>) {
    diary.0 += 1;
}

/// `Without<Diary>` is a name too.
#[system(name = "check::without", worlds = [prediction])]
fn without(_none: lactose_ecs::Without<Diary>, health: Mut<Health>) {
    health.0 += 1;
}

/// Names a transient (`PlayerInput`) and a derived (`Trail`) component: both may be named.
#[system(name = "check::follow", worlds = [server, prediction])]
fn follow(input: &PlayerInput, trail: Mut<Trail>) {
    trail.x += i32::from(input.movement.x);
}

/// A reduction whose finalize names the unreplicated component.
#[system(name = "check::count", worlds = [prediction], reduce = IntSum<u32>)]
fn count(_health: &Health) -> u32 {
    1
}

#[finalize(count)]
fn keep(_total: i128, _diary: Option<&Diary>) {}

#[system(name = "check::sweep", worlds = [server, prediction], exclusive)]
fn sweep(world: &mut World) {
    let _ = world.tick();
}

static ENGINE_LOOKALIKE: SystemSet = SystemSet::new("physics_api::sync");

/// An exclusive system in a set named like an engine's, under its own name.
#[system(name = "check::lookalike", worlds = [prediction], exclusive, in_set = ENGINE_LOOKALIKE)]
fn lookalike(world: &mut World) {
    let _ = world.tick();
}

/// Declared under an engine system's name: passes by key.
#[system(name = "physics_api::sync", worlds = [prediction], exclusive)]
fn sync_namesake(world: &mut World) {
    let _ = world.tick();
}

fn check(systems: &[&'static SystemDescriptor]) -> lactose_core::Result<()> {
    check_prediction_reads(types().registry(), systems)
}

fn refused(systems: &[&'static SystemDescriptor]) -> String {
    let error = check(systems).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    error.to_string()
}

#[test]
fn a_persistent_unreplicated_component_is_refused_by_name() {
    let message = refused(&[&REMINISCE]);
    assert!(message.contains("check::reminisce") && message.contains("test::diary"), "{message}");
    let message = refused(&[&WITHOUT]);
    assert!(message.contains("check::without"), "{message}");
}

#[test]
fn a_system_outside_the_prediction_world_is_not_judged() {
    check(&[&WRITE_DIARY]).unwrap();
}

#[test]
fn transient_derived_and_replicated_components_pass() {
    check(&[&FOLLOW]).unwrap();
    let types = types();
    let registry = types.registry();
    for key in [PlayerInput::KEY, Trail::KEY, Position::KEY, Health::KEY] {
        assert!(prediction_may_name(registry.get(key).unwrap()));
    }
    assert!(!prediction_may_name(registry.get(Diary::KEY).unwrap()));
}

#[test]
fn the_finalize_table_is_judged() {
    let message = refused(&[&COUNT]);
    assert!(message.contains("check::count") && message.contains("test::diary"), "{message}");
}

#[test]
fn an_exclusive_system_must_be_an_engines_by_key() {
    let message = refused(&[&SWEEP]);
    assert!(message.contains("check::sweep"), "{message}");
    let message = refused(&[&LOOKALIKE]);
    assert!(message.contains("check::lookalike"), "{message}");
    check(&[&SYNC_NAMESAKE]).unwrap();
    assert!(EXCLUSIVE_SYSTEMS.contains(&SystemKey::from_name("physics_api::sync")));
}

#[test]
fn a_component_the_registry_lacks_is_refused() {
    let empty = TypeTable::new();
    let error = check_prediction_reads(empty.registry(), &[&FOLLOW]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains("check::follow"));
}

/// `Predictor::new` requires the registry to hold `StagedRegion`, which the hook writes on
/// every proxy a frame places: a prediction world built without
/// `lactose_predict::register_types` is refused, naming the function.
#[test]
fn a_registry_without_the_placement_is_refused_by_the_predictor() {
    let mut types = TypeTable::new();
    lactose_input::register_types(&mut types).unwrap();
    lactose_net_api::register_types(&mut types).unwrap();
    let simulation = SimulationBuilder::new(WorldKind::Prediction, types)
        .tick(START)
        .setup(lactose_net_api::setup)
        .systems(lactose_input::register_systems)
        .systems(lactose_net_api::register_systems)
        .inputs(Box::new(PlayerInputApplier::new()))
        .build()
        .unwrap();
    let Err(error) = Predictor::new(simulation, local(), DEFAULT_DEPTH) else {
        panic!("a predictor built over a registry without `StagedRegion`")
    };
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.to_string().contains("lactose_predict::register_types"), "{error}");
}

#[test]
fn the_engine_systems_of_a_prediction_world_pass_at_its_build() {
    // The tests' prediction world, with physics, input and net-api, builds with the
    // check: every exclusive system it runs is listed.
    let calls = fake::CallLog::default();
    let backend = Box::new(fake::Fake::new(calls));
    let _ = simulation(Default::default(), Some(backend));
    // And a refused system fails the build with the check's error.
    let error = SimulationBuilder::new(WorldKind::Prediction, types())
        .systems(|registration| {
            registration.stage(Stage::Simulation).add(&REMINISCE);
        })
        .check(check_prediction_reads)
        .build()
        .map(|_| ())
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains("check::reminisce"));
}
