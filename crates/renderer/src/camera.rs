//! The latched orbit camera (architecture section 5.3, D48): the one buffer a frame's
//! shaders read that is written after every pass is recorded, from the snapshot's pivot,
//! the newest yaw and pitch and the boom length (`README.md`, "The camera and the latch").

use lactose_input_host::LatestSample;
use lactose_math::render::RenderPosition;
use lactose_rhi_api::Extent;

/// A perspective projection, without its aspect ratio, which is the target's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Projection {
    /// The vertical field of view in radians, between zero and pi.
    pub fov_y: f32,
    /// The distance of the near plane in metres, above zero. It maps to depth zero.
    pub near: f32,
    /// The distance of the far plane in metres, above `near`. It maps to depth one.
    pub far: f32,
}

/// What [`Renderer::latch`](crate::Renderer::latch) is given.
#[derive(Clone, Copy, Debug)]
pub struct CameraLatch<'a> {
    /// The point the camera orbits: the camera rig's pivot from the presentation
    /// snapshot, interpolated to the frame's time by the presentation schedule.
    pub pivot: RenderPosition,
    /// Where the newest input is published. The latch reads it once, inside the call,
    /// and takes the yaw and the pitch of what it read.
    pub input: &'a LatestSample,
    /// The distance from the pivot back to the eye in metres, zero or more: the boom
    /// length the presentation schedule's collision found. The latch does not check it
    /// again.
    pub boom: f32,
    /// The projection.
    pub projection: Projection,
    /// The extent of the frame's target, for the aspect ratio.
    pub extent: Extent,
}

/// The camera buffer's contents: what a vertex shader of the frame reads through the
/// camera's address.
///
/// 80 bytes, little-endian: `clip_from_cell` as sixteen `f32` in column-major order (0),
/// the cell as three `i32` (64), and four reserved bytes written as zero (76).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CameraData {
    /// The matrix that takes a position in metres from the centre of [`cell`](Self::cell)
    /// to clip space, as a column vector: column `c`, row `r` at index `4 * c + r`. Clip
    /// space is the device's: `x` right, `y` up, depth from zero at the near plane to one
    /// at the far plane.
    pub clip_from_cell: [f32; 16],
    /// The camera's cell: the pivot's. A shader subtracts it from an instance's cell, as
    /// integers, before anything becomes a float (architecture section 12.4).
    pub cell: [i32; 3],
}

impl CameraData {
    /// The bytes of the camera buffer.
    pub const BYTES: u32 = 80;

    /// The orbit camera of architecture section 5.3, in the world frame that section
    /// states and `lactose-input` derives its directions in: right-handed, `+Y` up, yaw
    /// zero facing `-Z`, positive yaw turning left, positive pitch looking up.
    ///
    /// - The forward direction is
    ///   `(-sin(yaw) * cos(pitch), sin(pitch), -cos(yaw) * cos(pitch))`.
    /// - The eye is `pivot - forward * boom`, the camera looks along `forward`, and its
    ///   up direction is the one in the plane of `forward` and `+Y` with a positive `Y`.
    ///   At a pitch of a quarter turn either way, up is
    ///   `(-sin(yaw), 0, -cos(yaw)) * sign(pitch)`, the limit of that direction.
    /// - The cell is `pivot.cell`, and positions are taken relative to its centre, so the
    ///   result does not change when the pivot and every instance move by the same whole
    ///   number of cells.
    /// - The projection is `projection` at the aspect ratio `extent.width /
    ///   extent.height`.
    ///
    /// Computed in `lactose_math`'s scalars and narrowed to 32-bit floats at the end.
    /// Equal arguments give equal bits on one machine; nothing rests on two machines
    /// agreeing.
    #[must_use]
    pub fn orbit(
        pivot: RenderPosition,
        yaw: f32,
        pitch: f32,
        boom: f32,
        projection: Projection,
        extent: Extent,
    ) -> Self {
        todo!()
    }

    /// The camera's bytes, as the buffer holds them.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 80] {
        todo!()
    }

    /// A position relative to the cell's centre, in clip space: `clip_from_cell * (x, y,
    /// z, 1)`, in 32-bit floats, each row summed left to right. What the vertex shader
    /// writes as its position.
    #[must_use]
    pub fn clip(&self, position: [f32; 3]) -> [f32; 4] {
        todo!()
    }
}

/// What a latch wrote.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LatchedCamera {
    /// The camera buffer's contents for the frame.
    pub data: CameraData,
    /// The yaw the latch read, in radians.
    pub yaw: f32,
    /// The pitch the latch read, in radians.
    pub pitch: f32,
}
