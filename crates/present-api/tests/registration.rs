//! What the facade runs before any body exists: the registration functions against a real
//! simulation build of every world kind, and the few bodies that decide nothing.

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{ErrorKind, SystemKey};
use lactose_ecs::{Job, PresentEvent, Res, ResMut, ScheduleError, World, WorldKind, system};
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_predict::{EXCLUSIVE_SYSTEMS, check_prediction_reads};
use lactose_present_api::{
    AudioRequest, CameraRig, DebugDraw, DebugShape, ExtractState, MeshInstance, RenderTransform,
    VfxRequest, ViewModels, extract_state,
};
use lactose_sim::{Registration, SimulationBuilder, Stage, TypeTable};

fn vec3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(Scalar::from_f64(x), Scalar::from_f64(y), Scalar::from_f64(z))
}

fn types() -> TypeTable {
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types).unwrap();
    lactose_present_api::register_types(&mut types).unwrap();
    types
}

/// The registry takes the four components as hashed, replicated types (no raw float, every
/// field with an ID), the schedule of every kind takes extract as a pass system, and
/// `setup` leaves the state where `app` reaches it.
#[test]
fn every_world_kind_builds_with_the_crate_registered() {
    for kind in [WorldKind::Server, WorldKind::Client, WorldKind::Prediction] {
        let mut simulation = SimulationBuilder::new(kind, types())
            .setup(lactose_present_api::setup)
            .systems(lactose_present_api::register_systems)
            .build()
            .unwrap();
        assert!(extract_state(&mut simulation).is_some(), "{kind:?}");
    }
    // Without `setup` there is no state to reach.
    let mut bare = SimulationBuilder::new(WorldKind::Server, types()).build().unwrap();
    assert!(extract_state(&mut bare).is_none());
}

/// A tick system that would read what a pass left in the state.
#[system(name = "test::read_extract_state", worlds = [server, client, prediction])]
fn read_extract_state(state: Res<ExtractState>) {
    let _ = &*state;
}

/// An exclusive tick system that would run a pass, or take an output away, inside a tick.
#[system(name = "test::drive_extract_state", worlds = [server, client, prediction], exclusive)]
fn drive_extract_state(world: &mut World, job: Job, state: ResMut<ExtractState>) {
    let world: &World = world;
    state.extract(world, &mut job);
    drop(state.set_snapshot(None));
}

/// The state is held under a type private to the crate, so a system that names
/// `ExtractState` finds no resource, though `setup` ran: the build refuses it, in every kind,
/// as it refuses any resource no setup function inserted.
#[test]
fn no_tick_system_reaches_the_state() {
    let systems = [
        (&READ_EXTRACT_STATE, "test::read_extract_state"),
        (&DRIVE_EXTRACT_STATE, "test::drive_extract_state"),
    ];
    for kind in [WorldKind::Server, WorldKind::Client, WorldKind::Prediction] {
        for (system, name) in systems {
            let error = SimulationBuilder::new(kind, types())
                .setup(lactose_present_api::setup)
                .systems(lactose_present_api::register_systems)
                .systems(move |registration: &mut Registration| {
                    registration.stage(Stage::Post).add(system);
                })
                .build()
                .err()
                .unwrap_or_else(|| panic!("{kind:?}: `{name}` was admitted"));
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{kind:?} {name}: {error}");
            let source = std::error::Error::source(&error).expect("a schedule error");
            let refused = source.downcast_ref::<ScheduleError>().expect("a schedule error");
            assert!(
                matches!(refused, ScheduleError::MissingResource { system, resource }
                    if *system == name && resource.ends_with("ExtractState")),
                "{kind:?}: {refused}"
            );
        }
    }
}

/// Extract is exclusive and runs in the prediction world, so that world's build check
/// (`lactose_predict::check_prediction_reads`, which `app` hands the builder) admits it
/// only by its key: the list there and the name declared here agree.
#[test]
fn the_prediction_world_builds_under_predicts_check() {
    assert!(EXCLUSIVE_SYSTEMS.contains(&SystemKey::from_name("present::extract")));
    SimulationBuilder::new(WorldKind::Prediction, types())
        .setup(lactose_present_api::setup)
        .systems(lactose_present_api::register_systems)
        .check(check_prediction_reads)
        .build()
        .unwrap();
}

#[test]
fn the_request_kinds_are_distinct() {
    assert_ne!(AudioRequest::KIND, VfxRequest::KIND);
    assert_ne!(AudioRequest::KIND, DebugShape::KIND);
    assert_ne!(VfxRequest::KIND, DebugShape::KIND);
}

#[test]
fn the_pivot_is_the_offset_in_the_entitys_frame() {
    let rig = CameraRig::new(vec3(0.0, 1.5, 0.0), Scalar::from_f64(4.0));
    let unturned = Transform::at(vec3(1.0, 2.0, 3.0));
    assert_eq!(rig.pivot_of(&unturned), vec3(1.0, 3.5, 3.0));

    // Half a turn about the vertical axis leaves a vertical offset where it was and
    // mirrors a sideways one.
    let half_turn = Quat::new(Scalar::ZERO, Scalar::ONE, Scalar::ZERO, Scalar::ZERO);
    let turned = Transform::new(vec3(1.0, 2.0, 3.0), half_turn);
    assert_eq!(rig.pivot_of(&turned), vec3(1.0, 3.5, 3.0));
    let sideways = CameraRig::new(vec3(0.5, 0.0, 0.0), Scalar::ZERO);
    assert_eq!(sideways.pivot_of(&turned), vec3(0.5, 2.0, 3.0));
}

#[test]
fn a_render_transform_is_the_math_crates_conversion() {
    let transform = Transform::at(vec3(100.0, -3.0, 40.25));
    let render = RenderTransform::from_sim(&transform);
    assert_eq!(render.position, RenderPosition::from_sim(transform.translation));
    assert_eq!(render.rotation, [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn the_buffers_start_empty_and_the_debug_buffer_keeps_its_order() {
    assert!(ViewModels::new().is_empty());
    let mut debug = DebugDraw::new();
    assert!(debug.is_empty());
    let a = RenderPosition::from_sim(vec3(0.0, 0.0, 0.0));
    let b = RenderPosition::from_sim(vec3(1.0, 0.0, 0.0));
    debug.line(a, b, 0xff00_00ff);
    debug.line(b, a, 0x00ff_00ff);
    assert_eq!(debug.lines().len(), 2);
    assert_eq!((debug.lines()[0].from, debug.lines()[1].from), (a, b));
    debug.clear();
    assert!(debug.is_empty());
}

#[test]
fn a_mesh_instance_is_its_reference() {
    let mesh = AssetRef::new(AssetId::from_bits(7));
    assert_eq!(MeshInstance::new(mesh).mesh, mesh);
}
