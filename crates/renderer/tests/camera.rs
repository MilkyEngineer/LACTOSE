//! The orbit camera against `lactose-input`'s derivations (verification row 95), as
//! `lactose-presentation`'s `tests/camera.rs` holds its `Orbit`: for the six samples whose
//! derived values `lactose-input` pins, a raw sample at the sample's angles quantises back to
//! it, and `CameraData::orbit`'s forward and right are `PlayerInput::aim` and `Yaw::rotation`
//! applied to `+X`, up to the 32-bit rounding of the raw angles and of the matrix.
//!
//! The forward is the matrix's `w` row, which is the forward unscaled; the right is its `x`
//! row, the right scaled by the focal length over the aspect ratio, normalised here.

use core::f64::consts::{PI, TAU};

use lactose_input::{Pitch, PlayerInput, Yaw};
use lactose_input_host::{Quantiser, RawSample};
use lactose_math::Vec3;
use lactose_math::render::RenderPosition;
use lactose_renderer::{CameraData, Projection};
use lactose_rhi_api::Extent;

/// The (yaw, pitch) units of `crates/input/tests/sample.rs`'s golden table, in its order.
const PINNED: [(u16, i16); 6] =
    [(0, 0), (16384, Pitch::LIMIT), (1, -1), (32769, 9001), (51234, -16000), (u16::MAX, 4321)];

const PROJECTION: Projection = Projection { fov_y: 1.0, near: 0.1, far: 1000.0 };

/// A wide extent, so that the right row's scale is not one.
const EXTENT: Extent = Extent { width: 1920, height: 1080 };

/// Half a unit in the last place of an `f32` in `[0.5, 1)`: what the narrowing of one of
/// the matrix's components, each at most one in size, costs at most.
const NARROWING: f64 = 1.0 / (1u64 << 25) as f64;

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

fn of(vector: Vec3) -> [f64; 3] {
    [vector.x, vector.y, vector.z].map(|component| component.to_f64())
}

fn error(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|axis| (a[axis] - b[axis]).abs()).fold(0.0, f64::max)
}

fn check(input: &PlayerInput) {
    let raw = raw(input);
    let mut quantised = raw;
    let quantised = Quantiser::new().next(&mut quantised);
    assert_eq!((quantised.yaw, quantised.pitch), (input.yaw, input.pitch), "{input:?}");

    // The raw angles' rounding, the narrowing of each component, and the scalars' own
    // roundings, as `lactose-presentation`'s test allows them.
    let bound = rounding(&raw, input) + NARROWING + 1e-12;
    assert!(bound < 2.5e-7, "{input:?}: {bound}");
    let pivot = RenderPosition { cell: [3, -2, 7], offset: [0.25, 1.5, -4.0] };
    let camera = CameraData::orbit(pivot, raw.yaw, raw.pitch, 4.0, PROJECTION, EXTENT);
    let m = camera.clip_from_cell.map(f64::from);

    let forward = [m[3], m[7], m[11]];
    let aim = of(input.aim());
    let off = error(forward, aim);
    assert!(off <= bound, "{input:?}: the forward is {off} from `aim`, bound {bound}");

    // The right row is scaled by about one; its normalisation in `f64` adds the narrowing of
    // each component again, relative, at most.
    let row = [m[0], m[4], m[8]];
    let length = row.iter().map(|c| c * c).sum::<f64>().sqrt();
    let right = row.map(|c| c / length);
    let turned = of(input.yaw.rotation() * Vec3::X);
    let off = error(right, turned);
    let bound = bound + 2.0 * NARROWING;
    assert!(off <= bound, "{input:?}: the right is {off} from `Yaw::rotation`, bound {bound}");

    // The camera is the pivot's cell, so the cell is never in a float.
    assert_eq!(camera.cell, pivot.cell);
}

/// Row 95: the six pinned samples.
#[test]
fn the_orbit_is_the_aim_of_the_pinned_samples() {
    for (yaw, pitch) in PINNED {
        check(&sample(yaw, pitch));
    }
}

/// The world frame's cardinal directions (architecture section 5.3), read through the clip
/// transform: the pivot straight ahead projects to the centre of the screen at a positive
/// depth, and a point to the camera's right to a positive `x`.
#[test]
fn the_pivot_projects_to_the_centre_ahead_of_the_camera() {
    let pivot = RenderPosition { cell: [0; 3], offset: [0.0; 3] };
    for (yaw, pitch) in PINNED {
        let raw = raw(&sample(yaw, pitch));
        let camera = CameraData::orbit(pivot, raw.yaw, raw.pitch, 4.0, PROJECTION, EXTENT);
        let [x, y, z, w] = camera.clip([0.0; 3]);
        assert!((w - 4.0).abs() < 1e-5, "{yaw} {pitch}: the pivot is the boom ahead, w {w}");
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5, "{yaw} {pitch}: ({x}, {y})");
        assert!(z > 0.0 && z < w, "{yaw} {pitch}: the depth {z} lies between the planes");
        let right = (sample(yaw, pitch).yaw.rotation() * Vec3::X).to_array();
        let [x, ..] = camera.clip(right.map(|c| c.to_f64() as f32));
        assert!(x > 0.0, "{yaw} {pitch}: the right is on the right, x {x}");
    }
}
