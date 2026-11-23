//! The camera rig's orbit against `lactose-input`'s derivations (verification row 95): for the
//! six samples whose derived values `lactose-input` pins, and for a sweep of the angles, a raw
//! sample at the sample's angles quantises back to it, and the rig's forward and yaw turn are
//! `PlayerInput::aim` and `Yaw::rotation` up to the 32-bit rounding of the raw angles. The rig's
//! pose on a row: the pivot of the shown transform and the rest length.

#![forbid(unsafe_code)]

use core::f64::consts::{PI, TAU};

use crate::common;

use common::{Ignored, Scene, at, claim, key, scalar, view};
use lactose_core::Tick;
use lactose_ecs::WorldKind;
use lactose_input::{Pitch, PlayerInput, Yaw};
use lactose_input_host::{Quantiser, RawSample};
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_present_api::CameraRig;
use lactose_presentation::{Orbit, PresentView};

/// The (yaw, pitch) units of `crates/input/tests/sample.rs`'s golden table, in its order.
const PINNED: [(u16, i16); 6] =
    [(0, 0), (16384, Pitch::LIMIT), (1, -1), (32769, 9001), (51234, -16000), (u16::MAX, 4321)];

fn sample(yaw: u16, pitch: i16) -> PlayerInput {
    PlayerInput { yaw: Yaw(yaw), pitch: Pitch(pitch), ..PlayerInput::IDLE }
}

/// The raw sample a camera holds at `input`'s angles: the yaw in `[-PI, PI)`, as
/// `RawSample::turn` keeps it, each angle rounded to 32 bits.
fn raw(input: &PlayerInput) -> RawSample {
    let yaw = input.yaw.radians().to_f64();
    let yaw = if yaw >= PI { yaw - TAU } else { yaw };
    RawSample { yaw: yaw as f32, pitch: input.pitch.radians().to_f64() as f32, ..RawSample::IDLE }
}

/// How far the raw angles are from the quantised ones, summed: a direction turned through
/// angles that far apart moves by at most this much in each component.
fn rounding(raw: &RawSample, input: &PlayerInput) -> f64 {
    let yaw = f64::from(raw.yaw).rem_euclid(TAU) - input.yaw.radians().to_f64();
    // The yaw unit at a full turn wraps to zero, where its raw angle is just below `TAU`.
    let yaw = yaw.abs().min((yaw.abs() - TAU).abs());
    let pitch = f64::from(raw.pitch) - input.pitch.radians().to_f64();
    yaw + pitch.abs()
}

fn largest(differences: [Scalar; 4]) -> f64 {
    differences.into_iter().map(|d| d.abs().to_f64()).fold(0.0, f64::max)
}

fn vector_error(a: Vec3, b: Vec3) -> f64 {
    let d = a - b;
    largest([d.x, d.y, d.z, Scalar::ZERO])
}

/// How far apart two rotations are, componentwise: a quaternion and its negation are one
/// rotation, and `Yaw::rotation` past half a turn is the negation of the turn the other way
/// that a raw yaw in `[-PI, PI)` gives.
fn rotation_error(a: Quat, b: Quat) -> f64 {
    let [ax, ay, az, aw] = a.to_array();
    let [bx, by, bz, bw] = b.to_array();
    let same = largest([ax - bx, ay - by, az - bz, aw - bw]);
    let negated = largest([ax + bx, ay + by, az + bz, aw + bw]);
    same.min(negated)
}

/// Checks one sample: the raw angles quantise back to it, and the orbit's directions are its
/// own within the raw angles' rounding, plus `slack` for the scalars' own.
fn check(input: &PlayerInput) {
    let mut raw = raw(input);
    let quantised = Quantiser::new().next(&mut raw);
    assert_eq!((quantised.yaw, quantised.pitch), (input.yaw, input.pitch), "{input:?}");

    // The second-order terms of a turn through angles ~1e-7 apart, and the two derivations'
    // own roundings.
    let slack = 1e-12;
    let bound = rounding(&raw, input) + slack;
    // An `f32` angle in `[-PI, PI]` is within half its unit in the last place of the exact one.
    assert!(bound < 2.5e-7, "{input:?}: {bound}");
    let orbit = Orbit::of(&raw);
    let forward = vector_error(orbit.forward(), input.aim());
    assert!(forward <= bound, "{input:?}: the forward is {forward} from `aim`, bound {bound}");
    let turn = rotation_error(orbit.yaw_rotation(), input.yaw.rotation());
    assert!(turn <= bound, "{input:?}: the yaw turn is {turn} from `Yaw::rotation`, bound {bound}");
}

/// Row 95: the six pinned samples.
#[test]
fn the_orbit_is_the_aim_of_the_pinned_samples() {
    for (yaw, pitch) in PINNED {
        check(&sample(yaw, pitch));
    }
}

/// The same over the angles: every 97th yaw unit at every 129th pitch unit, both limits
/// included.
#[test]
fn the_orbit_is_the_aim_across_the_angles() {
    let mut count = 0;
    let pitches = (-i32::from(Pitch::LIMIT)..=i32::from(Pitch::LIMIT)).step_by(129);
    for pitch in pitches.chain([i32::from(Pitch::LIMIT)]) {
        for yaw in (0..=u32::from(u16::MAX)).step_by(97).chain([u32::from(u16::MAX)]) {
            check(&sample(yaw as u16, pitch as i16));
            count += 1;
        }
    }
    assert_eq!(count, 256 * 677);
}

/// The world frame's cardinal directions (architecture section 5.3): yaw zero faces `-Z`, a
/// quarter turn left faces `-X`, pitch up looks along `+Y`.
#[test]
fn the_orbit_faces_the_world_frame() {
    let close = |a: Vec3, b: Vec3| vector_error(a, b) < 1e-7;
    let quarter = core::f32::consts::FRAC_PI_2;
    assert!(close(Orbit { yaw: 0.0, pitch: 0.0 }.forward(), -Vec3::Z));
    assert!(close(Orbit { yaw: quarter, pitch: 0.0 }.forward(), -Vec3::X));
    assert!(close(Orbit { yaw: 1.0, pitch: quarter }.forward(), Vec3::Y));
    assert!(close(Orbit { yaw: 0.0, pitch: -quarter }.forward(), -Vec3::Y));
}

/// The camera follows the focus: the pivot is the rig's offset, turned with the row, from the
/// shown transform, and the boom the rig's rest length; with no focus, or a focus the snapshot
/// holds no rig for, there is no camera.
#[test]
fn the_pose_is_the_rigs_pivot_of_the_shown_transform() {
    let mut scene = Scene::new(WorldKind::Prediction, Tick(10), 1);
    let rig = CameraRig::new(Vec3::new(scalar(1.0), scalar(1.5), Scalar::ZERO), scalar(4.0));
    let turned = Transform::new(Vec3::ZERO, Quat::from_rotation_y(Scalar::PI));
    scene.world_mut().spawn(key(1), common::REGION, (turned, rig, claim())).unwrap();
    scene.publish();
    scene.world_mut().get_mut::<Transform>(key(1)).unwrap().translation.x = scalar(64.0 * 3.0);
    scene.tick_publish();

    let focus = |key| PresentView { focus: Some(key), ..view(0.25) };
    let shown = scene.frame(&focus(key(1)), &mut Ignored).unwrap();
    let camera = shown.camera.unwrap();
    // A quarter of the way from x 0 to x 192, and the offset's x turned half a turn.
    let expected = RenderPosition::from_sim(Vec3::new(scalar(47.0), scalar(1.5), Scalar::ZERO));
    assert_eq!(camera.pivot.cell, expected.cell);
    for axis in 0..3 {
        assert!((camera.pivot.offset[axis] - expected.offset[axis]).abs() < 1e-6, "{camera:?}");
    }
    assert_eq!(camera.boom, 4.0);
    assert!(scene.frame(&focus(key(9)), &mut Ignored).unwrap().camera.is_none());
    assert!(scene.frame(&view(0.25), &mut Ignored).unwrap().camera.is_none());
}

/// A rest length below zero, or NaN, is a boom of zero.
#[test]
fn a_negative_rest_length_is_no_boom() {
    for rest in [-2.0, f64::NAN] {
        let mut scene = Scene::new(WorldKind::Prediction, Tick(10), 1);
        let rig = CameraRig::new(Vec3::ZERO, scalar(rest));
        scene.world_mut().spawn(key(1), common::REGION, (at(1.0), rig, claim())).unwrap();
        scene.publish();
        let view = PresentView { focus: Some(key(1)), ..view(1.0) };
        let camera = scene.frame(&view, &mut Ignored).unwrap().camera.unwrap();
        assert_eq!(camera.boom, 0.0, "{rest}");
        assert_eq!(camera.pivot, RenderPosition::from_sim(at(1.0).translation));
    }
}
