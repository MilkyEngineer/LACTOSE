//! Interpolation between the snapshot pair, and the camera rig: its pivot, its boom and its
//! orbit (architecture sections 5.3 and 13).

use lactose_core::StableKey;
use lactose_input_host::RawSample;
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_present_api::{CameraRig, PresentFrame};

/// The transform `frame` shows the presented row `key` at, or `None` where
/// [`PresentFrame::transform`] has none (no such row, or a `Client` world's snapshot).
///
/// `fraction` is the frame's, as `lactose-renderer`'s `FrameView::fraction`: from zero, the
/// older snapshot's transform, to one, the newer's, whatever the ticks between the two
/// snapshots; clamped to `[0, 1]`, and NaN counts as one. It is the weight: the renderer
/// draws an instance at `previous + (current - previous) * fraction` over the same pair, so a
/// row of the snapshot and the instance the renderer draws for it are shown at the same
/// instant, the camera's pivot with the mesh it follows.
///
/// Where the pair is flagged `no_interpolate` the newer transform is shown. Otherwise the
/// translation is `lerp` and the rotation `slerp` of `lactose_math`, and a fraction of zero or
/// one gives the older or the newer transform exactly.
#[must_use]
pub fn interpolate(frame: &PresentFrame<'_>, key: StableKey, fraction: f32) -> Option<Transform> {
    let pair = frame.transform(key)?;
    if pair.no_interpolate || fraction.is_nan() || fraction >= 1.0 {
        return Some(pair.current);
    }
    if fraction <= 0.0 {
        return Some(pair.previous);
    }
    let weight = Scalar::from_f32(fraction);
    let (previous, current) = (pair.previous, pair.current);
    Some(Transform::new(
        previous.translation.lerp(current.translation, weight),
        previous.rotation.slerp(current.rotation, weight),
    ))
}

/// Where a frame's camera orbits, and how far behind: what `lactose-renderer`'s
/// `CameraLatch` takes beside the newest input and the projection.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CameraPose {
    /// The rig's pivot, [`CameraRig::pivot_of`] the row's [`interpolate`]d transform, in
    /// render space.
    pub pivot: RenderPosition,
    /// The boom's length in metres, zero or more. At milestone 2 it is the rig's rest
    /// length: boom collision needs the static collision asset of milestone 3
    /// (`docs/plans/milestone-2.md`, decision 5.5).
    pub boom: f32,
}

impl CameraPose {
    /// The pose of the rig on the row `key` of `frame` at `fraction`, or `None` if the newer
    /// snapshot holds no transform or no [`CameraRig`] for it. The rig is the newer
    /// snapshot's; a negative or NaN rest length is zero.
    pub(crate) fn of(frame: &PresentFrame<'_>, key: StableKey, fraction: f32) -> Option<Self> {
        let rig = frame.newer().get::<CameraRig>(key)?;
        let shown = interpolate(frame, key, fraction)?;
        let boom = rig.boom.to_f32();
        Some(Self {
            pivot: RenderPosition::from_sim(rig.pivot_of(&shown)),
            boom: if boom > 0.0 { boom } else { 0.0 },
        })
    }
}

/// The camera rig's orbit: the yaw and pitch it turns about its pivot at, in the world frame
/// of architecture section 5.3 (right-handed, `+Y` up, yaw zero facing `-Z`, positive yaw
/// turning left, positive pitch looking up). The input the rig produces carries these
/// angles into the simulation (section 13), quantised by `lactose-input-host`'s
/// `Quantiser`, so its directions are `lactose-input`'s for the quantised sample:
/// [`forward`](Self::forward) is `PlayerInput::aim` and
/// [`yaw_rotation`](Self::yaw_rotation) is `Yaw::rotation`, up to the quantisation and the
/// angles' 32-bit rounding.
///
/// `lactose-renderer`'s `CameraData::orbit` is the one statement of the latched camera;
/// this is the rig's own, which boom collision will cast along.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Orbit {
    /// The yaw in radians, as the sample holds it.
    pub yaw: f32,
    /// The pitch in radians, as the sample holds it.
    pub pitch: f32,
}

impl Orbit {
    /// The orbit of a raw sample: its absolute yaw and pitch, which `RawSample::turn` keeps
    /// in `[-PI, PI)` and `[-PI/2, PI/2]`.
    #[must_use]
    pub const fn of(sample: &RawSample) -> Self {
        Self { yaw: sample.yaw, pitch: sample.pitch }
    }

    /// The direction the rig looks along, from the pivot away from the eye: `-Z` turned by
    /// the pitch about `+X`, then by the yaw about `+Y`, in `lactose_math`'s scalars. A unit
    /// vector up to rounding.
    #[must_use]
    pub fn forward(self) -> Vec3 {
        let pitch = Quat::from_rotation_x(Scalar::from_f32(self.pitch));
        self.yaw_rotation() * pitch * -Vec3::Z
    }

    /// The rig's turn about `+Y` alone: the facing the player's movement is taken in.
    #[must_use]
    pub fn yaw_rotation(self) -> Quat {
        Quat::from_rotation_y(Scalar::from_f32(self.yaw))
    }
}
