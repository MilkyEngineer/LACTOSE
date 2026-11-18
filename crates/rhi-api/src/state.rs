//! Resource states, the transitions between them, and what a pass declares it reaches.
//!
//! A subresource is in one state, on one queue, at each point of the submission order.
//! The caller says every change ([`Encoder::transition_images`],
//! [`Encoder::transition_buffers`]) and every backend derives what its driver needs from
//! it: barriers and ownership transfers on Vulkan, nothing on Metal but the usage a
//! declared buffer is made resident with. The null backend keeps the state and refuses a
//! use that does not match it (`README.md`, "States").
//!
//! [`Encoder::transition_images`]: crate::Encoder::transition_images
//! [`Encoder::transition_buffers`]: crate::Encoder::transition_buffers

use crate::caps::QueueRole;
use crate::resource::{BufferHandle, HeapSlot, ImageHandle, ImageRange};

/// The state of an image's subresource.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ImageState {
    /// The contents mean nothing. Every image starts here, a transition from here discards
    /// what the image held, and a read before the next write is refused
    /// ([`Rule::UndefinedRead`](crate::Rule::UndefinedRead)).
    Undefined,
    /// A colour attachment its pass writes and does not read.
    ColourAttachment,
    /// A colour attachment its pass writes and reads through the local read.
    LocalRead,
    /// The depth attachment.
    DepthAttachment,
    /// Sampled by a shader through a heap slot.
    SampledRead,
    /// The source of a copy.
    CopySource,
    /// The destination of a copy.
    CopyDestination,
    /// A swapchain image as [`Swapchain::present`](crate::Swapchain::present) takes it.
    Present,
}

/// The state of a buffer. A buffer has one state for all its bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BufferState {
    /// No use by the device yet. Every buffer starts here, and a host write needs no
    /// state. A transition from here keeps the buffer's bytes unless it moves the buffer
    /// to another queue, which discards them (`README.md`, "States").
    Undefined,
    /// Read by shaders through its device address.
    ShaderRead,
    /// Written by shaders through its device address, and perhaps read.
    ShaderWrite,
    /// The index buffer of draws.
    Index,
    /// The argument or count buffer of indirect draws and dispatches.
    Indirect,
    /// The source of a copy.
    CopySource,
    /// The destination of a copy.
    CopyDestination,
    /// Read by the host, after the submit that wrote it has completed.
    HostRead,
}

/// A state and the role whose queue the resource is on while in it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StateOn<S> {
    /// The state.
    pub state: S,
    /// The role. Two roles one queue serves are the same owner
    /// ([`QueueLayout::same_queue`](crate::QueueLayout::same_queue)).
    pub queue: QueueRole,
}

/// One change of an image's state.
///
/// Where the two queues differ, as the device's layout resolves them, the change is an
/// ownership transfer and is recorded twice, with equal values: on an encoder of `from`'s
/// role, which releases, and on an encoder of `to`'s role, which acquires; the acquiring
/// submit waits for a timeline value the releasing submit signals. A resource made
/// [`Sharing::Concurrent`](crate::Sharing::Concurrent), and a change from `Undefined`, are
/// recorded once, on the encoder that uses the resource next.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageTransition {
    /// The image.
    pub image: ImageHandle,
    /// The mips and layers that change. Each must be in `from`.
    pub range: ImageRange,
    /// Where the subresources are.
    pub from: StateOn<ImageState>,
    /// Where they are after it.
    pub to: StateOn<ImageState>,
}

/// One change of a buffer's state. Ownership transfers as an image's does
/// ([`ImageTransition`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferTransition {
    /// The buffer.
    pub buffer: BufferHandle,
    /// Where it is.
    pub from: StateOn<BufferState>,
    /// Where it is after it.
    pub to: StateOn<BufferState>,
}

/// A buffer a pass reaches through its device address, and the state it is in for the
/// whole pass.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferReach {
    /// The buffer.
    pub buffer: BufferHandle,
    /// [`BufferState::ShaderRead`] or [`BufferState::ShaderWrite`], and exactly the one
    /// the pass's shaders need: a buffer a shader writes is declared `ShaderWrite`, and
    /// one no shader writes is declared `ShaderRead`. Metal's hazard tracking orders the
    /// pass by it.
    pub state: BufferState,
}

/// What a pass declares it reaches: every heap slot a shader of the pass may index, and
/// every buffer a shader of the pass may reach through a device address, whether the
/// address came through [`push_address`](crate::Encoder::push_address), through
/// push-constant bytes or out of another buffer.
///
/// The declaration is required on every backend and checked where a backend can see it
/// (`README.md`, "What a pass declares"). A slot listed twice is listed once; a buffer
/// listed twice is listed in one state, or the pass is refused on every backend
/// ([`Rule::State`](crate::Rule::State)).
/// The index and indirect buffers a draw names are not declared here: the call that names
/// them is their declaration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PassReach<'a> {
    /// The heap slots. Each must hold an image in [`ImageState::SampledRead`] at every
    /// draw or dispatch of the pass.
    pub slots: &'a [HeapSlot],
    /// The buffers, each in the state it names for the whole pass.
    pub buffers: &'a [BufferReach],
}

impl PassReach<'_> {
    /// A pass that reaches nothing through the heap or an address.
    pub const NOTHING: Self = Self { slots: &[], buffers: &[] };
}
