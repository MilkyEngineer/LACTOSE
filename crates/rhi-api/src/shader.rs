//! The cooked shader format: what the cooker writes for one stage and one target, and
//! what a backend makes a pipeline from. It names the heap's shader form.
//!
//! The bytes are [`CookedShader::encode`]'s and nothing else writes them, so the layout
//! is this crate's alone; what it carries is the struct. The cooker checks what the format
//! cannot (`README.md`, "The cooked shader format").

use lactose_core::Result;

/// The version [`CookedShader::encode`] writes. A change to the struct or to what a field
/// means is a new version, and [`CookedShader::decode`] reads no other.
pub const COOKED_SHADER_VERSION: u32 = 1;

/// The descriptor set (SPIR-V) the heap's arrays are bound in, on every pipeline. The
/// input attachments of a fragment stage are set 0, input `i` at binding `i`.
pub const HEAP_SET: u32 = 1;

/// The bytes of a pipeline's push-constant range at most.
pub const PUSH_BYTES_MAX: u32 = 128;

/// A pipeline stage.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ShaderStage {
    /// The vertex stage.
    Vertex,
    /// The fragment stage.
    Fragment,
    /// The compute stage.
    Compute,
}

/// What a cooked shader's code is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ShaderTarget {
    /// A SPIR-V module.
    SpirV,
    /// Metal Shading Language 3.1 source, which the backend compiles.
    Msl,
    /// A Metal library, compiled on macOS for macOS 14.
    MetalLibrary,
}

/// The form the heap takes in a shader. One form exists: a later one is a new name and a
/// new [`COOKED_SHADER_VERSION`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HeapForm {
    /// Fixed-size arrays in one parameter block, indexed by a slot number that is the same
    /// on every target (`docs/results/spike-metal-apple-m1-2026-11-17.md`, "The heap's
    /// shader form"). On SPIR-V the block is descriptor set [`HEAP_SET`]; on Metal it is
    /// one argument buffer.
    FixedArray,
}

/// What a heap array holds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HeapArrayKind {
    /// Two-dimensional sampled images: the heap's slots
    /// ([`HeapSlot`](crate::HeapSlot)).
    SampledImage2d,
    /// Samplers: the sampler table ([`SamplerIndex`](crate::SamplerIndex)).
    Sampler,
}

/// One array of the heap as a shader declares it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HeapArray {
    /// What it holds.
    pub kind: HeapArrayKind,
    /// Its binding in the heap's block: 0 for the images, 1 for the samplers.
    pub binding: u32,
    /// Its length: the device's [`heap_slots`](crate::Capabilities::heap_slots) for the
    /// images and its [`samplers`](crate::Capabilities::samplers) for the samplers. A
    /// pipeline is not made from a shader compiled for another length.
    pub capacity: u32,
}

/// How a shader reaches the heap.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct HeapUse {
    /// The form.
    pub form: HeapForm,
    /// The set the block is bound in: [`HEAP_SET`].
    pub set: u32,
    /// The arrays the shader declares, each kind at most once.
    pub arrays: Vec<HeapArray>,
}

/// A specialisation constant a shader declares.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ConstantSlot {
    /// Its id: the SPIR-V constant id, and the Metal function-constant index.
    pub id: u32,
    /// The bytes of its value: 1 for a boolean, otherwise 4.
    pub bytes: u32,
}

/// One cooked shader: one stage, for one target.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CookedShader {
    /// The stage.
    pub stage: ShaderStage,
    /// What `code` is.
    pub target: ShaderTarget,
    /// The entry point's name in `code`.
    pub entry: String,
    /// The module, the source or the library.
    pub code: Vec<u8>,
    /// How the shader reaches the heap, or `None` when it does not.
    pub heap: Option<HeapUse>,
    /// The specialisation constants it declares, in ascending id order.
    pub constants: Vec<ConstantSlot>,
    /// The bytes of its push-constant block, at most [`PUSH_BYTES_MAX`].
    pub push_bytes: u32,
    /// Bit `i` is set when the stage reads input attachment `i`, which is colour
    /// attachment `i` of its pass and nothing else. Zero outside the fragment stage.
    pub inputs: u32,
    /// Bit `i` is set when the stage writes colour attachment `i`. Zero outside the
    /// fragment stage.
    pub outputs: u32,
}

impl CookedShader {
    /// The shader's bytes, for the cooker to write: a magic number,
    /// [`COOKED_SHADER_VERSION`], every field, and a checksum over all of it. Equal
    /// shaders give equal bytes on every host.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        todo!()
    }

    /// Reads what [`encode`](Self::encode) wrote.
    ///
    /// # Errors
    ///
    /// `ErrorKind::VersionMismatch` for another version's bytes; `ErrorKind::InvalidData`
    /// for bytes that are not a cooked shader, that are truncated or fail the checksum, or
    /// whose fields break what their documentation states (a heap set other than
    /// [`HEAP_SET`], a kind twice, constants out of order, a push block above
    /// [`PUSH_BYTES_MAX`], inputs or outputs outside the fragment stage).
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        todo!()
    }
}
