//! Interpolation between the snapshot pair (verification row 99): a body re-spawned under its
//! key without `Teleported` is interpolated between the two snapshots, and one re-spawned with
//! it is shown where it now is; and the weight a frame's fraction gives over a pair one and two
//! ticks apart, at its ends, and for a pair the snapshot flags.

#![forbid(unsafe_code)]

use crate::common;

use common::{Ignored, Scene, at, claim, key, scalar, shown_x, view};
use lactose_core::Tick;
use lactose_ecs::WorldKind;
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{Teleported, Transform};
use lactose_present_api::CameraRig;
use lactose_presentation::{PresentView, interpolate};

/// A prediction world with the body `key(1)` at x 0, carrying the camera's rig, its first
/// snapshot acquired.
fn body() -> Scene {
    let mut scene = Scene::new(WorldKind::Prediction, Tick(10), 2);
    let rig = CameraRig::new(Vec3::ZERO, scalar(3.0));
    scene.world_mut().spawn(key(1), common::REGION, (at(0.0), rig, claim())).unwrap();
    scene.publish();
    scene.frame(&view(0.0), &mut Ignored).unwrap();
    scene
}

/// The body despawned and spawned again under its key at x 10, between ticks, with a
/// `Teleported` of `teleported` if `Some`; then a tick and its publication.
fn respawn(scene: &mut Scene, teleported: Option<Tick>) {
    scene.world_mut().despawn(key(1)).unwrap();
    let rig = CameraRig::new(Vec3::ZERO, scalar(3.0));
    let row = (at(10.0), rig, claim());
    match teleported {
        None => scene.world_mut().spawn(key(1), common::REGION, row).unwrap(),
        Some(tick) => {
            let row = (at(10.0), rig, claim(), Teleported(tick));
            scene.world_mut().spawn(key(1), common::REGION, row).unwrap();
        }
    }
    scene.tick_publish();
}

/// Row 99, without `Teleported`: the pair holds the body in both snapshots, so the frame
/// blends from where it was to where it is, and the camera's pivot with it.
#[test]
fn a_respawn_without_teleported_interpolates() {
    let mut scene = body();
    respawn(&mut scene, None);
    let focus = PresentView { focus: Some(key(1)), ..view(0.5) };
    let shown = scene.frame(&focus, &mut Ignored).unwrap();
    let pair = shown.frame.transform(key(1)).unwrap();
    assert_eq!((pair.previous, pair.current, pair.no_interpolate), (at(0.0), at(10.0), false));
    assert_eq!(shown_x(&shown.frame, key(1), 0.5), 5.0);
    assert_eq!(shown_x(&shown.frame, key(1), 0.0), 0.0);
    assert_eq!(shown_x(&shown.frame, key(1), 1.0), 10.0);
    let pivot = shown.camera.unwrap().pivot;
    assert_eq!(pivot, RenderPosition::from_sim(at(5.0).translation));
}

/// Row 99, with `Teleported` at the newer snapshot's tick: the pair is flagged, and every
/// fraction shows the body where it is; the camera's pivot jumps with it.
#[test]
fn a_respawn_with_teleported_does_not_interpolate() {
    let mut scene = body();
    let newer = Tick(scene.tick().0 + 1);
    respawn(&mut scene, Some(newer));
    let focus = PresentView { focus: Some(key(1)), ..view(0.5) };
    let shown = scene.frame(&focus, &mut Ignored).unwrap();
    assert_eq!(shown.frame.newer().tick(), newer);
    let pair = shown.frame.transform(key(1)).unwrap();
    assert_eq!((pair.previous, pair.current, pair.no_interpolate), (at(0.0), at(10.0), true));
    for fraction in [0.0, 0.5, 1.0] {
        assert_eq!(shown_x(&shown.frame, key(1), fraction), 10.0, "{fraction}");
    }
    let pivot = shown.camera.unwrap().pivot;
    assert_eq!(pivot, RenderPosition::from_sim(at(10.0).translation));
}

/// A `Teleported` at or before the older snapshot's tick is in the past: the re-spawn blends.
#[test]
fn a_teleport_before_the_pair_interpolates() {
    let mut scene = body();
    let older = scene.tick();
    respawn(&mut scene, Some(older));
    let shown = scene.frame(&view(0.5), &mut Ignored).unwrap();
    assert!(!shown.frame.transform(key(1)).unwrap().no_interpolate);
    assert_eq!(shown_x(&shown.frame, key(1), 0.5), 5.0);
}

/// The weight over a pair two ticks apart, as a frame gets when two ticks ran between its
/// publications: the frame's time is the fraction past the tick before the newer one. The
/// ends are exact, a fraction out of `[0, 1]` is clamped and NaN shows the newer transform;
/// the rotation is `slerp`ed.
#[test]
fn the_weight_spans_the_pair() {
    let mut scene = body();
    let quarter = Quat::from_rotation_y(Scalar::FRAC_PI_2);
    scene.world_mut().get_mut::<Transform>(key(1)).unwrap().translation.x = scalar(4.0);
    scene.step();
    scene.world_mut().get_mut::<Transform>(key(1)).unwrap().translation.x = scalar(8.0);
    scene.world_mut().get_mut::<Transform>(key(1)).unwrap().rotation = quarter;
    scene.tick_publish();
    let shown = scene.frame(&view(0.0), &mut Ignored).unwrap();
    let frame = shown.frame;
    assert_eq!(frame.newer().tick().0 - frame.older().tick().0, 2, "two ticks in one pair");
    assert_eq!(shown_x(&frame, key(1), 0.0), 4.0);
    assert_eq!(shown_x(&frame, key(1), 0.5), 6.0);
    assert_eq!(shown_x(&frame, key(1), 1.0), 8.0);
    assert_eq!(shown_x(&frame, key(1), -3.0), 4.0);
    assert_eq!(shown_x(&frame, key(1), 7.0), 8.0);
    assert_eq!(shown_x(&frame, key(1), f32::NAN), 8.0);
    let half = interpolate(&frame, key(1), 0.0).unwrap().rotation.to_array();
    let eighth = Quat::from_rotation_y(Scalar::FRAC_PI_2 * Scalar::HALF).to_array();
    for (a, b) in half.into_iter().zip(eighth) {
        assert!((a - b).abs() < scalar(1e-12), "{half:?}");
    }
    assert_eq!(interpolate(&frame, key(1), 1.0).unwrap().rotation, quarter);
    assert!(interpolate(&frame, key(9), 0.5).is_none());
}

/// The first snapshot is published alone, so its pair is one tick: shown as it is.
#[test]
fn a_pair_of_one_tick_is_not_interpolated() {
    let mut scene = Scene::new(WorldKind::Prediction, Tick(10), 1);
    let row = (at(2.0), CameraRig::new(Vec3::ZERO, Scalar::ONE), claim());
    scene.world_mut().spawn(key(1), common::REGION, row).unwrap();
    scene.publish();
    let shown = scene.frame(&view(0.5), &mut Ignored).unwrap();
    assert_eq!(shown.frame.older().tick(), shown.frame.newer().tick());
    assert_eq!(shown_x(&shown.frame, key(1), 0.5), 2.0);
}
