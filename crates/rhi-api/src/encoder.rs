//! The encoder: what one thread records, and the passes, draws and copies it records.
//!
//! Recording does not fail. A command that breaks a rule is found when the encoder is
//! submitted, in submission order, which is the only order states have
//! (`README.md`, "Recording and submitting").

use core::any::Any;
use core::ops::Range;

use lactose_core::Result;

use crate::caps::QueueRole;
use crate::pipeline::{ComputePipelineHandle, GraphicsPipelineHandle};
use crate::resource::{BufferHandle, Extent, HeapSlot, ImageHandle};
use crate::state::{BufferTransition, ImageTransition, PassReach};

/// One mip of one layer of an image, as an attachment or one side of a copy.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Target {
    /// The image.
    pub image: ImageHandle,
    /// The mip.
    pub mip: u32,
    /// The layer.
    pub layer: u32,
}

/// What an attachment holds when its pass begins.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Load<C> {
    /// The value given.
    Clear(C),
    /// Nothing the pass may read before it writes.
    DontCare,
    /// What the attachment held. Never the load of an attachment made with a memoryless
    /// request ([`Rule::MemorylessStore`](crate::Rule::MemorylessStore)).
    Keep,
}

/// What becomes of an attachment's contents when its pass ends.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Store {
    /// They are kept.
    Store,
    /// They are undefined. The only action a memoryless attachment takes.
    DontCare,
}

/// A colour attachment of a render pass.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColourAttachment {
    /// What is drawn into, in [`ImageState::ColourAttachment`], or in
    /// [`ImageState::LocalRead`] when `local_read` is set.
    ///
    /// [`ImageState::ColourAttachment`]: crate::ImageState::ColourAttachment
    /// [`ImageState::LocalRead`]: crate::ImageState::LocalRead
    pub target: Target,
    /// What it holds at the pass's start; the clear value is red, green, blue, alpha.
    pub load: Load<[f32; 4]>,
    /// What becomes of it at the pass's end.
    pub store: Store,
    /// Whether the pass reads it through the local read. The attachment at index `i` of
    /// the pass is input `i`: there is no other mapping.
    pub local_read: bool,
}

/// The depth attachment of a render pass.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DepthAttachment {
    /// What is drawn into, in [`ImageState::DepthAttachment`](crate::ImageState).
    pub target: Target,
    /// What it holds at the pass's start.
    pub load: Load<f32>,
    /// What becomes of it at the pass's end.
    pub store: Store,
}

/// A render pass: its attachments, all of one extent, and what its shaders reach.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RenderPass<'a> {
    /// The pass's name. A backend opens no span: the caller that records the pass does,
    /// under this name.
    pub label: &'a str,
    /// The colour attachments, in the order pipelines and inputs number them, at most the
    /// device's [`colour_attachments`](crate::Capabilities::colour_attachments).
    pub colour: &'a [ColourAttachment],
    /// The depth attachment, if the pass has one.
    pub depth: Option<DepthAttachment>,
    /// What the pass's shaders reach.
    pub reach: PassReach<'a>,
}

/// A compute pass: what its shaders reach.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ComputePass<'a> {
    /// The pass's name, as a render pass's.
    pub label: &'a str,
    /// What the pass's shaders reach.
    pub reach: PassReach<'a>,
}

/// A rectangle in texels, from the top left.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Rect {
    /// The left edge.
    pub x: u32,
    /// The top edge.
    pub y: u32,
    /// The size.
    pub extent: Extent,
}

/// A viewport: a rectangle in texels from the top left, and a depth range.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub width: f32,
    /// The height.
    pub height: f32,
    /// The depth the near plane maps to, in zero to one.
    pub near: f32,
    /// The depth the far plane maps to, in zero to one.
    pub far: f32,
}

/// A range copied between two buffers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferCopy {
    /// The first byte read.
    pub source_offset: u64,
    /// The first byte written.
    pub destination_offset: u64,
    /// The bytes copied, above zero.
    pub size: u64,
}

/// A rectangle of one mip of one layer copied between a buffer and an image. The buffer's
/// rows are packed: `extent.width` texels each, with nothing between them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferImageCopy {
    /// The first byte of the buffer's side.
    pub buffer_offset: u64,
    /// The image's mip.
    pub mip: u32,
    /// The image's layer.
    pub layer: u32,
    /// The rectangle of that mip.
    pub rect: Rect,
}

/// The record an indexed indirect draw reads: twenty bytes, five little-endian 32-bit
/// fields in this order, the same on every backend.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IndexedIndirectArgs {
    /// The number of indices drawn.
    pub index_count: u32,
    /// The number of instances drawn.
    pub instance_count: u32,
    /// The first index, counted from the bound index buffer's offset.
    pub first_index: u32,
    /// What is added to each index.
    pub base_vertex: i32,
    /// The first instance.
    pub first_instance: u32,
}

impl IndexedIndirectArgs {
    /// The record's size, and the least stride of an indirect draw.
    pub const BYTES: u32 = 20;

    /// The record as a device reads it.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 20] {
        let fields = [
            self.index_count.to_le_bytes(),
            self.instance_count.to_le_bytes(),
            self.first_index.to_le_bytes(),
            self.base_vertex.to_le_bytes(),
            self.first_instance.to_le_bytes(),
        ];
        let mut bytes = [0; 20];
        let mut i = 0;
        while i < 20 {
            bytes[i] = fields[i / 4][i % 4];
            i += 1;
        }
        bytes
    }
}

/// What one thread records, and the pool its commands come from: one for each recording
/// job and frame in flight. `Send` and not `Sync`.
///
/// An encoder is recording from its creation and after each [`reset`](Self::reset), and
/// is submitted once between resets. A submit that takes it makes it pending until that
/// submit completes. From that submit to the next reset a recording call records nothing
/// and changes nothing the submit runs, and a second submit with no reset between is
/// refused ([`Rule::Encoding`](crate::Rule::Encoding), every backend).
///
/// A draw whose vertex, index or instance range is empty, a dispatch with a count of zero,
/// and an indirect draw of no records record nothing, on every backend; a range whose
/// start passes its end is empty.
///
/// Commands belong to a scope: outside a pass (transitions, the alias call, copies,
/// beginning a pass), inside a render pass (draws, the local read, the viewport and the
/// scissor, binding a graphics pipeline or an index buffer), inside a compute pass
/// (dispatches, binding a compute pipeline), inside either (push constants, ending the
/// pass). A command outside its scope, a draw or dispatch with no pipeline of its kind
/// bound, an indexed draw with no index buffer bound, and a pass left open at the end are
/// refused at the submit ([`Rule::Encoding`](crate::Rule::Encoding), every backend). After
/// such a breach, or any other a device backend finds while recording, it records nothing
/// more into its driver's command buffer, and the submit refuses the encoder.
///
/// `Any` is a supertrait so that a backend recovers its own type from the object a submit
/// hands it, through the object the box holds: `&mut *encoders[i]` as a `&mut dyn Any`.
/// The box is `Any` too, so `&mut encoders[i]` coerces the box and matches no backend's
/// type. A caller has no use for it.
///
/// Dropping a pending encoder blocks until its submit completes, so an encoder is dropped
/// off the frame workers (`README.md`, "Handles and lifetimes").
pub trait Encoder: Any + Send {
    /// The role the encoder was created for. A submit to another role refuses it.
    fn queue(&self) -> QueueRole;

    /// Forgets what was recorded and returns the encoder's memory to its pool, so that it
    /// records again.
    ///
    /// # Errors
    ///
    /// Refused while the encoder is pending ([`Rule::Pending`](crate::Rule::Pending)): the
    /// caller waits for the timeline value its submit signals. `ErrorKind::DeviceLost`
    /// never: a lost device's encoders reset.
    fn reset(&mut self) -> Result<()>;

    /// Changes the state of images' subresources ([`ImageTransition`]).
    fn transition_images(&mut self, transitions: &[ImageTransition]);

    /// Changes the state of buffers ([`BufferTransition`]).
    fn transition_buffers(&mut self, transitions: &[BufferTransition]);

    /// The alias call: `image`, placed in a memory block, now begins, and every image
    /// whose placement overlaps its own has ended. It comes after the last use of those
    /// images and before the first use of `image`, in submission order. Afterwards every
    /// subresource of `image` is `Undefined` on this encoder's queue, and the overlapping
    /// images are unusable until an alias call of their own
    /// ([`Rule::Alias`](crate::Rule::Alias)).
    fn alias(&mut self, image: ImageHandle);

    /// Copies ranges of `source`, in `CopySource`, to `destination`, in
    /// `CopyDestination`. The two are different buffers. A region outside either is
    /// refused at the submit ([`Rule::Bounds`](crate::Rule::Bounds), every backend), as
    /// is one of the two copies below, and so are two regions of one call that write one
    /// byte of the destination buffer or one texel of the destination image. Vulkan
    /// forbids them (`VUID-vkCmdCopyBuffer-pRegions-12482`,
    /// `VUID-vkCmdCopyBufferToImage-pRegions-12483`,
    /// `VUID-vkCmdCopyImageToBuffer-pRegions-12485`, and their `VkCopy*Info2` forms
    /// `VUID-VkCopyBufferInfo2-pRegions-12482`,
    /// `VUID-VkCopyBufferToImageInfo2-pRegions-12483` and
    /// `VUID-VkCopyImageToBufferInfo2-pRegions-12485`), and each backend checks them from
    /// the regions and the image's format, with no driver call: an image-to-buffer
    /// region's bytes are its rows packed at the format's texel size.
    fn copy_buffer(
        &mut self,
        source: BufferHandle,
        destination: BufferHandle,
        regions: &[BufferCopy],
    );

    /// Copies packed rows of `source`, in `CopySource`, into rectangles of `destination`,
    /// whose subresources are in `CopyDestination`.
    fn copy_buffer_to_image(
        &mut self,
        source: BufferHandle,
        destination: ImageHandle,
        regions: &[BufferImageCopy],
    );

    /// Copies rectangles of `source`, whose subresources are in `CopySource`, into packed
    /// rows of `destination`, in `CopyDestination`.
    fn copy_image_to_buffer(
        &mut self,
        source: ImageHandle,
        destination: BufferHandle,
        regions: &[BufferImageCopy],
    );

    /// Begins a render pass. The viewport and the scissor are the attachments' extent
    /// until set. Nothing is bound: a pass starts with no pipeline, no index buffer and a
    /// push-constant block of zeros.
    fn begin_render_pass(&mut self, pass: &RenderPass<'_>);

    /// Begins a compute pass, with no pipeline and a push-constant block of zeros.
    fn begin_compute_pass(&mut self, pass: &ComputePass<'_>);

    /// Ends the pass that is open.
    fn end_pass(&mut self);

    /// Sets the viewport of the draws that follow in the pass. Its fields are finite, its
    /// width and height above zero and at most the device's
    /// [`viewport_extent`](crate::Capabilities::viewport_extent), its edges within its
    /// [`viewport_bounds`](crate::Capabilities::viewport_bounds), and `near` and `far` in
    /// zero to one ([`Rule::Bounds`](crate::Rule::Bounds), every backend); within those it
    /// may reach past the attachments.
    fn set_viewport(&mut self, viewport: Viewport);

    /// Sets the scissor of the draws that follow in the pass, inside the attachments'
    /// extent ([`Rule::Bounds`](crate::Rule::Bounds), every backend).
    fn set_scissor(&mut self, scissor: Rect);

    /// Binds the pipeline of the draws that follow. Its colour and depth formats are the
    /// pass's attachments'.
    fn bind_graphics_pipeline(&mut self, pipeline: GraphicsPipelineHandle);

    /// Binds the pipeline of the dispatches that follow.
    fn bind_compute_pipeline(&mut self, pipeline: ComputePipelineHandle);

    /// Binds the index buffer of the indexed draws that follow: 32-bit indices from
    /// `offset`, a multiple of four inside the buffer, in a buffer in `Index`
    /// ([`Rule::Bounds`](crate::Rule::Bounds), every backend).
    fn bind_index_buffer(&mut self, buffer: BufferHandle, offset: u64);

    /// Writes `bytes` into the pass's push-constant block at `offset`. The block is what
    /// every draw or dispatch after it sees, whichever pipeline is bound. A write that
    /// ends past [`PUSH_BYTES_MAX`](crate::PUSH_BYTES_MAX), by this call or the two typed
    /// ones below, is refused at the submit ([`Rule::Bounds`](crate::Rule::Bounds)).
    fn push_bytes(&mut self, offset: u32, bytes: &[u8]);

    /// Writes the device address of byte `byte_offset` of `buffer` into the push-constant
    /// block at `offset`, as eight little-endian bytes. The pass must declare `buffer`
    /// ([`Rule::Undeclared`](crate::Rule::Undeclared), every backend).
    fn push_address(&mut self, offset: u32, buffer: BufferHandle, byte_offset: u64);

    /// Writes `slot`'s number into the push-constant block at `offset`, as four
    /// little-endian bytes. The pass must declare `slot`
    /// ([`Rule::Undeclared`](crate::Rule::Undeclared), every backend), which is then not
    /// empty at a draw or dispatch ([`Rule::HeapLayout`](crate::Rule::HeapLayout), every
    /// backend).
    fn push_slot(&mut self, offset: u32, slot: HeapSlot);

    /// The local read: the draws that follow may read, as input `i`, what the pass has
    /// written to colour attachment `i` so far. It is not a barrier and takes no
    /// argument. A draw that reads an attachment written since the last call is refused
    /// ([`Rule::Local`](crate::Rule::Local)).
    fn local_read(&mut self);

    /// Draws `vertices` for each of `instances`, with no index buffer. A vertex shader's
    /// vertex id is `vertices.start` plus the vertex's number, and its instance id
    /// `instances.start` plus the instance's number, on every backend.
    fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>);

    /// Draws `indices` of the bound index buffer, adding `base_vertex` to each, for each
    /// of `instances`. A vertex shader's vertex id is the index plus `base_vertex`, and
    /// its instance id `instances.start` plus the instance's number, on every backend.
    /// The indices read lie inside the buffer ([`Rule::Bounds`](crate::Rule::Bounds),
    /// every backend).
    fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>);

    /// Draws `count` records ([`IndexedIndirectArgs`]) read from `arguments`, in
    /// `Indirect`, from `offset`, `stride` bytes apart, each as
    /// [`draw_indexed`](Self::draw_indexed) with the record's `first_instance` as the
    /// instances' start. A count above one needs
    /// [`multi_draw_indirect`](crate::Capabilities::multi_draw_indirect)
    /// ([`Rule::Capability`](crate::Rule::Capability)). `offset` is a multiple of four,
    /// `stride` a multiple of four and at least [`IndexedIndirectArgs::BYTES`] when
    /// `count` is above one, and every record lies inside the buffer
    /// ([`Rule::Bounds`](crate::Rule::Bounds), every backend). The indices a record
    /// names are read on the device, and no backend checks them.
    fn draw_indexed_indirect(
        &mut self,
        arguments: BufferHandle,
        offset: u64,
        count: u32,
        stride: u32,
    );

    /// As [`draw_indexed_indirect`](Self::draw_indexed_indirect), with the count read on
    /// the device: a 32-bit value at `count_offset` of `count`, in `Indirect`, capped at
    /// `max_count`. Needs [`indirect_count`](crate::Capabilities::indirect_count)
    /// ([`Rule::Capability`](crate::Rule::Capability)). `count_offset` is a multiple of
    /// four inside `count`, `stride` a multiple of four and at least
    /// [`IndexedIndirectArgs::BYTES`] whatever `max_count` is, and `max_count` records lie
    /// inside `arguments` from an `offset` that is a multiple of four
    /// ([`Rule::Bounds`](crate::Rule::Bounds), every backend).
    fn draw_indexed_indirect_count(
        &mut self,
        arguments: BufferHandle,
        offset: u64,
        count: BufferHandle,
        count_offset: u64,
        max_count: u32,
        stride: u32,
    );

    /// Runs the bound compute pipeline over `groups` workgroups, each count at most the
    /// device's [`compute_groups`](crate::Capabilities::compute_groups) in its dimension
    /// ([`Rule::Bounds`](crate::Rule::Bounds), every backend). The dispatches of one
    /// compute pass run in recording order, each seeing the buffer writes of those before
    /// it.
    fn dispatch(&mut self, groups: [u32; 3]);

    /// Runs the bound compute pipeline over the workgroups read from `arguments`, in
    /// `Indirect`: three little-endian 32-bit counts at `offset`, a multiple of four with
    /// the twelve bytes inside the buffer ([`Rule::Bounds`](crate::Rule::Bounds), every
    /// backend). The counts are device data: no backend checks them, and their writer keeps
    /// each within [`compute_groups`](crate::Capabilities::compute_groups).
    fn dispatch_indirect(&mut self, arguments: BufferHandle, offset: u64);
}
