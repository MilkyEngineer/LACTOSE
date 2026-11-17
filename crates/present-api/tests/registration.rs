//! What the facade runs before any body exists: the registration functions against a real
//! simulation build of every world kind, and the few bodies that decide nothing.

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::SystemKey;
use lactose_ecs::{PresentEvent, WorldKind};
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_predict::{EXCLUSIVE_SYSTEMS, check_prediction_reads};
use lactose_present_api::{
    AudioRequest, CameraRig, DebugDraw, DebugShape, ExtractState, MeshInstance, RenderTransform,
    VfxRequest, ViewModels,
};
use lactose_sim::{SimulationBuilder, TypeTable};

fn vec3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(Scalar::from_f64(x), Scalar::from_f64(y), Scalar::from_f64(z))
}

/// The registry takes the four components as hashed, replicated types (no raw float, every
/// field with an ID), the schedule of every kind takes extract as a pass system, and
/// `setup` leaves the state where `app` reaches it.
#[test]
fn every_world_kind_builds_with_the_crate_registered() {
    for kind in [WorldKind::Server, WorldKind::Client, WorldKind::Prediction] {
        let mut types = TypeTable::new();
        lactose_physics_api::register_types(&mut types).unwrap();
        lactose_present_api::register_types(&mut types).unwrap();
        let mut simulation = SimulationBuilder::new(kind, types)
            .setup(lactose_present_api::setup)
            .systems(lactose_present_api::register_systems)
            .build()
            .unwrap();
        assert!(simulation.world_mut().service_mut::<ExtractState>().is_some(), "{kind:?}");
    }
}

/// Extract is exclusive and runs in the prediction world, so that world's build check
/// (`lactose_predict::check_prediction_reads`, which `app` hands the builder) admits it
/// only by its key: the list there and the name declared here agree.
#[test]
fn the_prediction_world_builds_under_predicts_check() {
    assert!(EXCLUSIVE_SYSTEMS.contains(&SystemKey::from_name("present::extract")));
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types).unwrap();
    lactose_present_api::register_types(&mut types).unwrap();
    SimulationBuilder::new(WorldKind::Prediction, types)
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
