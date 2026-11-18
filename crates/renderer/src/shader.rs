//! The interface the renderer's three cooked shaders are written against: the scatter's
//! compute stage and the triangle pass's vertex and fragment stages. `README.md`, "The
//! shader interface", is the full statement; the numbers both sides must agree on are the
//! constants here, and [`Renderer::new`](crate::Renderer::new) holds each cooked shader's
//! reflection to them.
//!
//! Every buffer is read through a device address in the pass's push-constant block, and
//! a shader reads `SV_VulkanVertexID` and `SV_VulkanInstanceID`
//! (`crates/rhi-api/README.md`, "Pipelines and the cooked shader format").

use lactose_rhi_api::{CookedShader, Format};

/// The scatter's workgroup size: a compute stage of `local_size (64, 1, 1)`. A dispatch
/// of `n` entries runs `ceil(n / 64)` groups, and an invocation whose index is `n` or more
/// does nothing.
pub const SCATTER_GROUP: u32 = 64;

/// The bytes of the scatter's push-constant block: the address of the region's first row
/// (0), the address of the dispatch's first entry (8), the address of the instance
/// buffer's first record (16), the dispatch's entry count as a `u32` (24), and four bytes
/// of zero (28).
pub const SCATTER_PUSH_BYTES: u32 = 32;

/// The bytes of the triangle pass's push-constant block, which both of its stages
/// declare: the address of the instance buffer's first record (0), the address of the
/// camera buffer (8), the address of the triangle's first vertex (16), the frame's
/// fraction as an `f32` (24), and four bytes of zero (28).
pub const TRIANGLE_PUSH_BYTES: u32 = 32;

/// The id of the vertex stage's one specialisation constant: the cell size in metres, an
/// `f32` of four bytes, which the renderer sets to `RenderPosition::CELL_SIZE` when it
/// makes the pipeline, so that the shader's cell is the host's by construction.
pub const CELL_SIZE_CONSTANT: u32 = 0;

/// The built-in triangle every instance is drawn as at milestone 2, in the instance's own
/// space, in metres: three vertices of three `f32` each, twelve bytes a vertex, in a
/// buffer the vertex stage reads through its address by `SV_VulkanVertexID`. Anticlockwise
/// seen from `+Z`; the pipeline culls neither face.
pub const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.5, 0.0], [-0.5, -0.5, 0.0], [0.5, -0.5, 0.0]];

/// The colours of the triangle pass: the fragment stage writes `PALETTE[slot % 7]`, each
/// channel divided by 255, to colour attachment zero, where `slot` is
/// `SV_VulkanInstanceID` handed on from the vertex stage without interpolation. No entry
/// is the clear colour, so a readback tells a drawn pixel from an empty one, and seven
/// instances in the first seven slots are seven colours.
pub const PALETTE: [[u8; 4]; 7] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 0, 255],
    [255, 0, 255, 255],
    [0, 255, 255, 255],
    [255, 255, 255, 255],
];

/// What the triangle pass clears its colour target to: opaque black.
pub const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// The cooked shaders the renderer makes its pipelines from, and the format of the
/// target they draw into. Each is one the cooker wrote for the device's backend, decoded
/// by the caller with `CookedShader::decode`; the renderer opens no file.
#[derive(Clone, Copy, Debug)]
pub struct Shaders<'a> {
    /// The scatter: a compute stage with a push block of [`SCATTER_PUSH_BYTES`], no heap
    /// and no specialisation constant.
    pub scatter: &'a CookedShader,
    /// The triangle pass's vertex stage: a push block of [`TRIANGLE_PUSH_BYTES`], no
    /// heap, and the one constant [`CELL_SIZE_CONSTANT`].
    pub vertex: &'a CookedShader,
    /// The triangle pass's fragment stage: the same push block, no heap, no constant, no
    /// input attachment, and colour attachment zero as its one output.
    pub fragment: &'a CookedShader,
    /// The format of the colour target every frame's [`FrameView`](crate::FrameView)
    /// names. The fragment stage writes values a `Rgba8Unorm` target holds exactly.
    pub target: Format,
}
