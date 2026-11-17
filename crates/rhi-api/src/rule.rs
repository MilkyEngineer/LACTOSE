//! The rules of the contract a backend can check, and the value a refused call returns.
//!
//! A call that breaks a rule a backend checks returns an error of kind
//! [`ErrorKind::InvalidData`] whose source is a [`Refusal`]; [`Rule::of`] reads the rule back.
//! `README.md`, "The rules", states each rule in full and says which backend checks it.

use std::borrow::Cow;

use lactose_core::{Error, ErrorKind};

/// A rule of the contract, by the name the result files and the plan give it.
///
/// The null backend checks every rule. A device backend checks each clause whose breach
/// on a device would be undefined behaviour on the host, a GPU fault or an abort inside
/// the driver; a clause whose breach is a wrong image it leaves to null's runs and the
/// validators. What a shader's code does, and what it reaches through data alone, no rule
/// covers (`README.md`, "The rules").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Rule {
    /// `state`: a transition's `from` is not the state the subresource is in, a use needs
    /// a state the subresource is not in, or a resource an earlier pass or copy wrote is
    /// used with no transition since, in submission order.
    State,
    /// `no-qfot`: a use on a queue other than the owning one with no release and acquire
    /// between, an acquire with no release submitted, a change of a resource's state or
    /// owner while a pending submit of another queue uses it and with no wait for that
    /// submit, or an alias call on another queue than the one the images it ends were
    /// used on.
    Queue,
    /// `destroyed`: a handle whose resource has been destroyed, at the call it is given to
    /// or at the submit of an encoder that recorded it, or, on null, a handle another
    /// device issued.
    Destroyed,
    /// `pending`: a destroy, an encoder reset or a swapchain resize while a submit that
    /// has not completed uses what it would free; a destroy of an image a heap slot still
    /// holds, or of a block an image is still placed in; a host read or write of a buffer
    /// a pending submit writes.
    Pending,
    /// `capability`: a call that needs a reported capability the device does not report,
    /// or a shader module that declares a capability the device did not enable.
    Capability,
    /// `usage`: a use of a resource that its descriptor's usage, memory class or format
    /// does not allow, or a descriptor no device could honour.
    Usage,
    /// `bounds`: a range outside its buffer, image, memory block, heap, push-constant
    /// range or attachments, an index or indirect offset or stride off its alignment, a
    /// viewport that is not finite, not of positive extent or whose depth range leaves zero
    /// to one, or a dispatch above the device's workgroup counts.
    Bounds,
    /// `undeclared`: a heap slot or a buffer handed to a pass through
    /// [`push_slot`](crate::Encoder::push_slot) or
    /// [`push_address`](crate::Encoder::push_address) that the pass did not declare.
    Undeclared,
    /// `unflushed`: at a submit, a written range of memory that is not coherent with no
    /// flush since; or a host read of such a range the device wrote with no invalidate
    /// since.
    Unflushed,
    /// `host-overlap`: a host access to a range of a buffer while another host access to
    /// an overlapping range is in progress, or a destroy of the buffer or a submit that
    /// writes it while any is.
    HostOverlap,
    /// `local`: a draw whose pipeline reads input `i` with no
    /// [`local_read`](crate::Encoder::local_read) since colour attachment `i` was last
    /// written in the pass.
    Local,
    /// `local-map`: a draw whose pipeline reads input `i` in a pass that has no colour
    /// attachment `i` marked for the local read.
    LocalMap,
    /// `split`: a use of a memoryless attachment after a local read in its pass, on a
    /// device that reports its contents do not survive one.
    Split,
    /// `memoryless-store`: an attachment made with a memoryless request that is stored, or
    /// loaded with [`Load::Keep`](crate::Load::Keep).
    MemorylessStore,
    /// `heap-layout`: a draw or dispatch in a pass that declares a heap slot which is empty
    /// or whose image is not in the sampled-read state.
    HeapLayout,
    /// `uuwp-used`: a write to a heap slot that a pass of a pending submit declared.
    HeapWrite,
    /// `no-fence`: a use of a placed image whose memory another image has used, with no
    /// alias call since.
    Alias,
    /// `undefined-read`: a read of contents that are undefined, before the next write: an
    /// image's after an alias call, a transition from undefined or a don't-care store; a
    /// buffer's that nothing has written, or that another queue took from undefined.
    UndefinedRead,
    /// `present-queue`: a submit to the present role that is not a present, or a
    /// swapchain image used by an encoder of another role.
    PresentQueue,
    /// `swapchain`: an acquire while an acquired image is not yet presented, a present or
    /// any other use of a swapchain image that is not the acquired one, or a second
    /// swapchain for a device that has one.
    Swapchain,
    /// `timeline`: a submit's signal of a value not greater than every value the timeline
    /// has reached or been promised, or while another queue's signal of it is pending with
    /// no wait for it; a host signal not between the timeline's value and every pending
    /// promise, or one a pending promise or another host signal could pass; or a value
    /// 2^31 or more from the timeline's.
    Timeline,
    /// `encoding`: a command outside the scope it belongs to, a draw or dispatch with
    /// nothing bound to it, a pass left open, an encoder of another device or role, or an
    /// encoder submitted again with no reset.
    Encoding,
}

impl Rule {
    /// Every rule, in declaration order.
    pub const ALL: [Self; 22] = [
        Self::State,
        Self::Queue,
        Self::Destroyed,
        Self::Pending,
        Self::Capability,
        Self::Usage,
        Self::Bounds,
        Self::Undeclared,
        Self::Unflushed,
        Self::HostOverlap,
        Self::Local,
        Self::LocalMap,
        Self::Split,
        Self::MemorylessStore,
        Self::HeapLayout,
        Self::HeapWrite,
        Self::Alias,
        Self::UndefinedRead,
        Self::PresentQueue,
        Self::Swapchain,
        Self::Timeline,
        Self::Encoding,
    ];

    /// The rule's stable name: the one the spikes' result files use where they name it.
    /// A refusal's message begins with it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Queue => "no-qfot",
            Self::Destroyed => "destroyed",
            Self::Pending => "pending",
            Self::Capability => "capability",
            Self::Usage => "usage",
            Self::Bounds => "bounds",
            Self::Undeclared => "undeclared",
            Self::Unflushed => "unflushed",
            Self::HostOverlap => "host-overlap",
            Self::Local => "local",
            Self::LocalMap => "local-map",
            Self::Split => "split",
            Self::MemorylessStore => "memoryless-store",
            Self::HeapLayout => "heap-layout",
            Self::HeapWrite => "uuwp-used",
            Self::Alias => "no-fence",
            Self::UndefinedRead => "undefined-read",
            Self::PresentQueue => "present-queue",
            Self::Swapchain => "swapchain",
            Self::Timeline => "timeline",
            Self::Encoding => "encoding",
        }
    }

    /// The error a backend returns when it finds this rule broken: kind
    /// [`ErrorKind::InvalidData`], a message of the rule's name, a colon and `detail`, and
    /// a [`Refusal`] as its source. Every backend builds its refusals here, so
    /// [`of`](Self::of) reads them all.
    #[must_use]
    pub fn refuse(self, detail: impl Into<Cow<'static, str>>) -> Error {
        let detail = detail.into();
        Error::new(ErrorKind::InvalidData, format!("{}: {detail}", self.name()))
            .with_source(Refusal { rule: self })
    }

    /// The rule `error` refuses a call by, or `None` when it is not a refusal: a device
    /// loss, an exhausted budget, a missing capability at creation, or bytes that failed a
    /// check are errors of their own kinds and carry no rule. The first [`Refusal`] in
    /// `error`'s chain of sources is read, so a caller that wraps a refusal in an error of
    /// its own keeps the rule.
    #[must_use]
    pub fn of(error: &Error) -> Option<Self> {
        let mut source = std::error::Error::source(error);
        while let Some(error) = source {
            if let Some(refusal) = error.downcast_ref::<Refusal>() {
                return Some(refusal.rule);
            }
            source = error.source();
        }
        None
    }
}

/// The source of the error a refused call returns. It names the rule and nothing else:
/// what was wrong is the error's message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Refusal {
    /// The rule the call broke.
    pub rule: Rule,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "the rule `{}` of lactose-rhi-api's contract", self.rule.name())
    }
}

impl std::error::Error for Refusal {}
