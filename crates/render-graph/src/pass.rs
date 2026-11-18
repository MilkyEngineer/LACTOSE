//! What a pass declares: every image and buffer it uses, and how.
//!
//! A use is all the graph knows of a pass. From the uses it derives which passes are kept,
//! how long a transient lives, the state each resource is brought to before the pass, each
//! attachment's store action, what the pass's `PassReach` holds and which queue the pass
//! runs on (`README.md`, "Uses and order"). The table of uses, whether each writes and the
//! state it needs is in `README.md`.

use lactose_rhi_api::{HeapSlot, Load};

use crate::id::{BufferId, ImageId};

/// A colour attachment of a render pass: one mip of one layer of an image, written by the
/// pass's draws. Its store action is the graph's to derive.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColourUse {
    /// The image.
    pub image: ImageId,
    /// The mip attached.
    pub mip: u32,
    /// The layer attached.
    pub layer: u32,
    /// What the attachment holds when the pass begins. The variant is part of the key and
    /// a clear's value is not.
    pub load: Load<[f32; 4]>,
    /// Whether the pass reads the attachment through the local read, as input `i` for
    /// colour attachment `i`. The image is then in `ImageState::LocalRead` for the pass
    /// and, where it is a transient, made with `ImageUsage::INPUT`.
    pub local_read: bool,
}

/// The depth attachment of a render pass: one mip of one layer of an image in a depth
/// format, tested and written by the pass's draws.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DepthUse {
    /// The image.
    pub image: ImageId,
    /// The mip attached.
    pub mip: u32,
    /// The layer attached.
    pub layer: u32,
    /// What the attachment holds when the pass begins. The variant is part of the key and
    /// a clear's value is not.
    pub load: Load<f32>,
}

/// How the shaders and draws of a pass use a buffer. A pass names a buffer once, so a
/// buffer has one access in a pass, as it has one state at a time on a device.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BufferAccess {
    /// Shaders read it through its device address and none writes it: the buffer is in
    /// `BufferState::ShaderRead` for the pass and in the pass's reach with that state.
    Read,
    /// A shader writes it through its device address, and shaders may read it: the buffer
    /// is in `BufferState::ShaderWrite` for the pass and in the pass's reach with that
    /// state.
    Write,
    /// The pass binds it as the index buffer of its draws: `BufferState::Index`. A render
    /// pass's alone.
    Index,
    /// The pass names it as the argument or count buffer of an indirect draw or dispatch:
    /// `BufferState::Indirect`.
    Indirect,
}

/// One buffer a pass uses.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferUse {
    /// The buffer.
    pub buffer: BufferId,
    /// How.
    pub access: BufferAccess,
}

/// A render pass's declaration. The slices are copied by the call that takes it.
///
/// No image and no buffer is named twice in one pass.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RenderPassDecl<'a> {
    /// The pass's name: its span's `label`, the label its `RenderPass` is begun under,
    /// and the name of an error that names it. Part of the key.
    pub label: &'a str,
    /// The colour attachments, in the order the pipelines of the pass number them.
    pub colour: &'a [ColourUse],
    /// The depth attachment.
    pub depth: Option<DepthUse>,
    /// The imports the pass's shaders sample through heap slots: each is in
    /// `ImageState::SampledRead` for the pass. The slot that holds each is the caller's
    /// and is among `slots`.
    pub sampled: &'a [ImageId],
    /// The buffers the pass uses.
    pub buffers: &'a [BufferUse],
    /// Every heap slot a shader of the pass may index: the slots of its `PassReach`,
    /// passed on as they are. Not part of the key. A slot that holds an image the frame
    /// does not declare is the caller's to keep in `SampledRead` for the pass.
    pub slots: &'a [HeapSlot],
}

/// A compute pass's declaration. The slices are copied by the call that takes it.
///
/// No image and no buffer is named twice in one pass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ComputePassDecl<'a> {
    /// The pass's name, as a render pass's ([`RenderPassDecl::label`]).
    pub label: &'a str,
    /// The imports the pass's shaders sample through heap slots.
    pub sampled: &'a [ImageId],
    /// The buffers the pass uses. `BufferAccess::Index` is refused.
    pub buffers: &'a [BufferUse],
    /// Every heap slot a shader of the pass may index. Not part of the key.
    pub slots: &'a [HeapSlot],
}

/// A copy pass's declaration: the sources and destinations of the copies its recorder
/// makes. The slices are copied by the call that takes it.
///
/// No image and no buffer is named twice in one pass, so none is both a source and a
/// destination.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CopyPassDecl<'a> {
    /// The pass's name: its span's `label` and the name of an error that names it. Part
    /// of the key.
    pub label: &'a str,
    /// The images copied from: each in `ImageState::CopySource`.
    pub image_sources: &'a [ImageId],
    /// The images copied to: each in `ImageState::CopyDestination`.
    pub image_destinations: &'a [ImageId],
    /// The buffers copied from: each in `BufferState::CopySource`.
    pub buffer_sources: &'a [BufferId],
    /// The buffers copied to: each in `BufferState::CopyDestination`.
    pub buffer_destinations: &'a [BufferId],
}
