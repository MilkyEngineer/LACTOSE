//! The images and buffers a frame declares.
//!
//! An import is the caller's: made, kept and destroyed outside the graph, and in a state
//! at each end of the frame that is the caller's word. A transient is the graph's: an
//! image that lives inside one frame, made and placed by the plan that first needed it and
//! undefined when each frame begins (`README.md`, "Imports" and "Transients").

use lactose_rhi_api::{
    BufferHandle, BufferState, Extent, Format, ImageHandle, ImageState, Sharing, StateOn,
    TimelinePoint,
};

/// An image the caller made, for one frame.
///
/// The graph tracks one state for the whole image: every transition it records covers
/// `mips` mips and `layers` layers from zero. The format, the extent and the usage the
/// image was made with are the device's knowledge and are not repeated here.
///
/// One handle is imported at most once in a declaration. Nothing here can see a second
/// import of it, and the two ids would each be given transitions of their own; null
/// refuses the submit (`state`). An import is never an image made with a memoryless
/// request: the graph stores every import's attachments, which is refused for such an
/// image (`memoryless-store`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageImport<'a> {
    /// A name for an error that names the import. Part of the key.
    pub label: &'a str,
    /// The image. Not part of the key: a frame that imports another image under the same
    /// declaration compiles to the same plan.
    pub image: ImageHandle,
    /// How many mips the image was made with.
    pub mips: u32,
    /// How many layers the image was made with.
    pub layers: u32,
    /// What the image was made with: whether a use on another queue needs a release and
    /// an acquire.
    pub sharing: Sharing,
    /// The state and queue every subresource is in before the frame's first submit: the
    /// caller's word, which the graph does not check and null does (`state`). `Undefined`
    /// discards the contents and takes the image from whichever queue it was on, with no
    /// transfer, for the queue that uses it first; its `queue` is then ignored. A state
    /// that writes, where the frame's first use is in it, gets the same-state transition
    /// that orders this frame's write after the last frame's (`README.md`, "Transitions
    /// and store actions").
    pub initial: StateOn<ImageState>,
    /// The state and queue every subresource is in once the frame's submits have
    /// completed, whether or not a kept pass used the image, and before any release
    /// `handoff` asks for. Never `Undefined`.
    pub last: StateOn<ImageState>,
    /// Where the image's next user, which is not this graph, takes it from `last`: the
    /// present thread's read of a finished target. Where that is a transfer between two
    /// queues the graph records its release half on `last`'s queue, after every use of
    /// the image in the frame, and the next user records the acquire with equal values,
    /// over every mip and layer declared here. Where it is not, because one queue serves
    /// both roles or the image is `Concurrent`, the graph records nothing and the next
    /// user records the whole change. `None` where the next user is a later frame of a
    /// graph, which records both halves itself.
    pub handoff: Option<StateOn<ImageState>>,
    /// A point every submit of the frame that names the image waits for, for a use the
    /// graph did not make: the point of the last submit outside this graph that used the
    /// image, such as the present thread's read, where that submit may still be pending
    /// and the image is imported from `Undefined` or is `Concurrent`. A wait for a point
    /// already reached costs nothing on a timeline. `None` otherwise:
    /// the graph orders its own frames, and an `Exclusive` image imported in the state
    /// and on the queue it is in is ordered by that queue or by its transfer.
    pub ready: Option<TimelinePoint>,
}

/// A buffer the caller made, for one frame. Every buffer of a frame is an import.
///
/// One handle is imported at most once in a declaration, as an image's is
/// ([`ImageImport`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BufferImport<'a> {
    /// A name for an error that names the import. Part of the key.
    pub label: &'a str,
    /// The buffer. Not part of the key.
    pub buffer: BufferHandle,
    /// What the buffer was made with.
    pub sharing: Sharing,
    /// The state and queue the buffer is in before the frame's first submit: the caller's
    /// word. With `Undefined` the state is not given and `queue` is the queue the buffer
    /// was last used on, or any where it never was: the graph takes the buffer there
    /// first, which keeps its bytes, and moves it by a transfer where the frame first
    /// uses it on another queue. So a buffer may be imported from `Undefined` in every
    /// frame. Where the word is wrong the bytes are discarded, and null refuses the next
    /// read (`undefined-read`). A state that writes is ordered as an image's is
    /// ([`ImageImport::initial`]).
    pub initial: StateOn<BufferState>,
    /// The state and queue the buffer is in once the frame's submits have completed,
    /// whether or not a kept pass used it. Never `Undefined`. `HostRead` for a readback,
    /// which the host reads after the frame's point.
    pub last: StateOn<BufferState>,
    /// A point every submit of the frame that names the buffer waits for, as an image's
    /// ([`ImageImport::ready`]).
    pub ready: Option<TimelinePoint>,
}

/// An image the graph makes: what it is made from, but for what the graph derives.
///
/// The usage is the union of the uses the kept passes make of it, the memory is the
/// plan's choice ([`TransientMemory`]) and the sharing is `Exclusive`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TransientImage<'a> {
    /// The name the image is made under, and the name of an error that names it. Part of
    /// the key.
    pub label: &'a str,
    /// The format. A depth format is attached as depth and never as colour, and a colour
    /// format the other way round.
    pub format: Format,
    /// The extent of mip 0.
    pub extent: Extent,
    /// The number of mips, at least one.
    pub mips: u32,
    /// The number of layers, at least one.
    pub layers: u32,
}

/// Where a plan put a transient image.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TransientMemory {
    /// Memory of its own: the device accepts no placement for such an image
    /// (`Device::image_placement` returned `Unsupported`).
    Own,
    /// A memoryless request, made only where the device reports it honours it. The image
    /// is an attachment of one pass and nothing else.
    Memoryless,
    /// Placed in a memory block of the plan's. Two transients of one plan alias when
    /// their blocks are equal and their byte ranges overlap.
    Placed {
        /// The block, counted from zero among the plan's blocks.
        block: u32,
        /// The offset in bytes.
        offset: u64,
        /// The bytes the image takes from its offset.
        size: u64,
    },
}
