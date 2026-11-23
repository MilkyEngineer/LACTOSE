//! One frame's declaration: its imports, its transients and its passes, in order.

use lactose_rhi_api::{
    BufferHandle, BufferState, Extent, Format, HeapSlot, ImageHandle, ImageState, Sharing, StateOn,
    TimelinePoint,
};

use crate::id::{BufferId, ImageId, PassId};
use crate::pass::{BufferUse, ColourUse, ComputePassDecl, CopyPassDecl, DepthUse, RenderPassDecl};
use crate::recorder::{ComputeRecorder, CopyRecorder, RenderRecorder};
use crate::resource::{BufferImport, ImageImport, TransientImage};
use crate::sync_view::SyncView;

/// A pass's recorder, boxed by the call that declared the pass. It is called at most once,
/// by the one recording job that owns it, so it is `FnOnce` and `Send` and need be neither
/// `Fn` nor `Sync`.
pub(crate) enum Recorder<'f> {
    /// A render pass's.
    Render(Box<dyn FnOnce(&mut RenderRecorder<'_>) + Send + 'f>),
    /// A compute pass's.
    Compute(Box<dyn FnOnce(&mut ComputeRecorder<'_>) + Send + 'f>),
    /// A copy pass's.
    Copy(Box<dyn FnOnce(&mut CopyRecorder<'_>) + Send + 'f>),
}

/// What one frame draws: built by the caller every frame, then given to
/// [`RenderGraph::compile`](crate::RenderGraph::compile).
///
/// A declaration is a value of its own. It borrows no graph, a call on it reaches no
/// device and checks nothing, and every breach of a declaration's rules is
/// [`compile`](crate::RenderGraph::compile)'s to report, naming the pass or the resource
/// by its label. It owns a copy of everything its calls are given and a box for each
/// recorder; `'f` is what the recorders borrow, shared or exclusively.
///
/// The order of the calls is the declaration: ids count in it, the key hashes in it, and
/// a pass sees what the passes declared before it wrote (`README.md`, "Uses and order").
/// A caller that wants one plan for its steady frames declares the same things in the
/// same order every frame.
///
/// `Send + Sync` whatever its recorders are, so that a job may build it for another to
/// compile and a `'static` one waits in a world's resource. What it copied is shared by
/// the recording jobs; each recorder is owned by the one job that calls it (`README.md`,
/// "Concurrency").
pub struct FrameDecl<'f> {
    /// What the calls declared, copied: read by `compile` and shared read-only by the
    /// recording jobs.
    pub(crate) declared: Declared,
    /// The recorders, by pass, reached through `&mut` or by value alone. The job that
    /// calls a recorder takes it out of its slot.
    pub(crate) recorders: SyncView<Vec<Option<Recorder<'f>>>>,
}

/// Everything a declaration's calls were given but the recorders, owned.
pub(crate) struct Declared {
    /// The view count.
    pub(crate) views: u32,
    /// The images, by id.
    pub(crate) images: Vec<DeclaredImage>,
    /// The buffers, by id.
    pub(crate) buffers: Vec<DeclaredBuffer>,
    /// The passes, by id.
    pub(crate) passes: Vec<DeclaredPass>,
}

/// An image of a declaration.
pub(crate) enum DeclaredImage {
    /// An [`ImageImport`], owned.
    Import(DeclaredImageImport),
    /// A [`TransientImage`], owned.
    Transient(DeclaredTransient),
}

impl DeclaredImage {
    /// The label it was declared under.
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Import(import) => &import.label,
            Self::Transient(transient) => &transient.label,
        }
    }

    /// The mips and layers it was declared with: what each of its transitions covers.
    pub(crate) const fn mips_and_layers(&self) -> (u32, u32) {
        match self {
            Self::Import(import) => (import.mips, import.layers),
            Self::Transient(transient) => (transient.mips, transient.layers),
        }
    }

    /// The import, for an image the caller made.
    pub(crate) const fn import(&self) -> Option<&DeclaredImageImport> {
        match self {
            Self::Import(import) => Some(import),
            Self::Transient(_) => None,
        }
    }
}

/// An [`ImageImport`], owned.
pub(crate) struct DeclaredImageImport {
    pub(crate) label: String,
    pub(crate) image: ImageHandle,
    pub(crate) mips: u32,
    pub(crate) layers: u32,
    pub(crate) sharing: Sharing,
    pub(crate) initial: StateOn<ImageState>,
    pub(crate) last: StateOn<ImageState>,
    pub(crate) handoff: Option<StateOn<ImageState>>,
    pub(crate) ready: Option<TimelinePoint>,
}

/// A [`TransientImage`], owned.
pub(crate) struct DeclaredTransient {
    pub(crate) label: String,
    pub(crate) format: Format,
    pub(crate) extent: Extent,
    pub(crate) mips: u32,
    pub(crate) layers: u32,
}

/// A [`BufferImport`], owned.
pub(crate) struct DeclaredBuffer {
    pub(crate) label: String,
    pub(crate) buffer: BufferHandle,
    pub(crate) sharing: Sharing,
    pub(crate) initial: StateOn<BufferState>,
    pub(crate) last: StateOn<BufferState>,
    pub(crate) ready: Option<TimelinePoint>,
}

/// A pass of a declaration, owned.
pub(crate) struct DeclaredPass {
    pub(crate) label: String,
    pub(crate) kind: DeclaredKind,
}

/// What a pass of each kind declared, owned.
pub(crate) enum DeclaredKind {
    /// A [`RenderPassDecl`]'s lists.
    Render {
        colour: Vec<ColourUse>,
        depth: Option<DepthUse>,
        sampled: Vec<ImageId>,
        buffers: Vec<BufferUse>,
        slots: Vec<HeapSlot>,
    },
    /// A [`ComputePassDecl`]'s lists.
    Compute { sampled: Vec<ImageId>, buffers: Vec<BufferUse>, slots: Vec<HeapSlot> },
    /// A [`CopyPassDecl`]'s lists.
    Copy {
        image_sources: Vec<ImageId>,
        image_destinations: Vec<ImageId>,
        buffer_sources: Vec<BufferId>,
        buffer_destinations: Vec<BufferId>,
    },
}

impl DeclaredKind {
    /// The buffers a render or compute pass declared, with their access; none for a copy
    /// pass, whose buffers are its sources and destinations.
    pub(crate) fn buffers(&self) -> &[BufferUse] {
        match self {
            Self::Render { buffers, .. } | Self::Compute { buffers, .. } => buffers,
            Self::Copy { .. } => &[],
        }
    }
}

impl<'f> FrameDecl<'f> {
    /// An empty declaration for a frame of `views` views, at least one.
    ///
    /// The view count is hashed into the key by itself and read back from the plan
    /// ([`CompiledPlan::views`](crate::CompiledPlan::views)). The graph instances nothing
    /// by it: the caller declares each view's passes (`README.md`, "Not in this crate
    /// yet").
    #[must_use]
    pub fn new(views: u32) -> Self {
        Self {
            declared: Declared {
                views,
                images: Vec::new(),
                buffers: Vec::new(),
                passes: Vec::new(),
            },
            recorders: SyncView::new(Vec::new()),
        }
    }

    /// The view count the declaration was made with.
    #[must_use]
    pub fn views(&self) -> u32 {
        self.declared.views
    }

    /// Declares an image the caller made.
    pub fn import_image(&mut self, import: &ImageImport<'_>) -> ImageId {
        self.image(DeclaredImage::Import(DeclaredImageImport {
            label: import.label.to_owned(),
            image: import.image,
            mips: import.mips,
            layers: import.layers,
            sharing: import.sharing,
            initial: import.initial,
            last: import.last,
            handoff: import.handoff,
            ready: import.ready,
        }))
    }

    /// Declares a buffer the caller made.
    pub fn import_buffer(&mut self, import: &BufferImport<'_>) -> BufferId {
        let id = BufferId::new(next_id(self.declared.buffers.len()));
        self.declared.buffers.push(DeclaredBuffer {
            label: import.label.to_owned(),
            buffer: import.buffer,
            sharing: import.sharing,
            initial: import.initial,
            last: import.last,
            ready: import.ready,
        });
        id
    }

    /// Declares an image the graph makes, which lives from its first to its last kept use
    /// in this frame and is undefined when the frame begins.
    pub fn transient_image(&mut self, image: &TransientImage<'_>) -> ImageId {
        self.image(DeclaredImage::Transient(DeclaredTransient {
            label: image.label.to_owned(),
            format: image.format,
            extent: image.extent,
            mips: image.mips,
            layers: image.layers,
        }))
    }

    /// Declares a render pass, after every pass declared so far.
    ///
    /// `record` is called by [`RenderGraph::record`](crate::RenderGraph::record), once if
    /// the pass is kept and never if it is culled, on a frame-critical worker, with the
    /// pass begun; the graph ends the pass when it returns. It may run while other
    /// passes' recorders run on other workers, in no promised order with them. Called or
    /// not, it is dropped before `RenderGraph::record` returns, or with the declaration.
    pub fn render_pass(
        &mut self,
        pass: &RenderPassDecl<'_>,
        record: impl FnOnce(&mut RenderRecorder<'_>) + Send + 'f,
    ) -> PassId {
        let kind = DeclaredKind::Render {
            colour: pass.colour.to_vec(),
            depth: pass.depth,
            sampled: pass.sampled.to_vec(),
            buffers: pass.buffers.to_vec(),
            slots: pass.slots.to_vec(),
        };
        self.pass(pass.label, kind, Recorder::Render(Box::new(record)))
    }

    /// Declares a compute pass, after every pass declared so far. `record` is called as a
    /// render pass's is ([`render_pass`](Self::render_pass)).
    pub fn compute_pass(
        &mut self,
        pass: &ComputePassDecl<'_>,
        record: impl FnOnce(&mut ComputeRecorder<'_>) + Send + 'f,
    ) -> PassId {
        let kind = DeclaredKind::Compute {
            sampled: pass.sampled.to_vec(),
            buffers: pass.buffers.to_vec(),
            slots: pass.slots.to_vec(),
        };
        self.pass(pass.label, kind, Recorder::Compute(Box::new(record)))
    }

    /// Declares a copy pass, after every pass declared so far. `record` is called as a
    /// render pass's is ([`render_pass`](Self::render_pass)), with no pass open on the
    /// encoder.
    pub fn copy_pass(
        &mut self,
        pass: &CopyPassDecl<'_>,
        record: impl FnOnce(&mut CopyRecorder<'_>) + Send + 'f,
    ) -> PassId {
        let kind = DeclaredKind::Copy {
            image_sources: pass.image_sources.to_vec(),
            image_destinations: pass.image_destinations.to_vec(),
            buffer_sources: pass.buffer_sources.to_vec(),
            buffer_destinations: pass.buffer_destinations.to_vec(),
        };
        self.pass(pass.label, kind, Recorder::Copy(Box::new(record)))
    }

    fn image(&mut self, image: DeclaredImage) -> ImageId {
        let id = ImageId::new(next_id(self.declared.images.len()));
        self.declared.images.push(image);
        id
    }

    fn pass(&mut self, label: &str, kind: DeclaredKind, recorder: Recorder<'f>) -> PassId {
        let id = PassId::new(next_id(self.declared.passes.len()));
        self.declared.passes.push(DeclaredPass { label: label.to_owned(), kind });
        self.recorders.get_mut().push(Some(recorder));
        id
    }
}

/// The id the next entry of a list of `len` takes.
fn next_id(len: usize) -> u32 {
    u32::try_from(len).expect("a declaration holds fewer than 2^32 of each kind")
}
