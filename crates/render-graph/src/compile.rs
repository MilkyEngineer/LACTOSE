//! Compiling a declaration: the rules it is held to, the key it is cached under, and the
//! plan derived from it (`README.md`, "Declaring" to "Submission").
//!
//! Everything here is a function of the declaration, the device's layout and
//! capabilities, and what `Device::image_placement` returns: no thread, address or hash
//! map order reaches a plan.

use core::hash::{Hash, Hasher};

use lactose_core::{Error, ErrorKind, FixedHasher, Result};
use lactose_rhi_api::{
    BufferState, Capabilities, Device, Extent, Format, ImageDesc, ImageMemory, ImageRange,
    ImageState, ImageUsage, Load, QueueLayout, QueuePath, QueueRole, Sharing, StateOn, Store,
};

use crate::decl::{Declared, DeclaredImage, DeclaredKind, DeclaredPass};
use crate::pass::{BufferAccess, ColourUse, DepthUse};
use crate::plan::{
    Begin, GraphKey, JobPlan, PassStep, PlanParts, PlannedBufferTransition, PlannedImageTransition,
    PlannedPass, Resource, Step, SubmitPlan, Transitions, count,
};
use crate::resource::TransientMemory;

/// A breach of a declaration's rules.
fn breach(message: String) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// Holds a declaration to the rules of `README.md`, "Declaring", and to no others,
/// returning the first breach.
pub(crate) fn validate(declared: &Declared, layout: QueueLayout) -> Result<()> {
    // Rule 1.
    if declared.views == 0 {
        return Err(breach("a declaration of no view".to_owned()));
    }
    let present_own = layout.present == QueuePath::Own;
    let present = |queue: QueueRole| present_own && queue == QueueRole::Present;
    for image in &declared.images {
        let DeclaredImage::Import(import) = image else { continue };
        let label = &import.label;
        // Rule 4, the import's half.
        if import.mips == 0 || import.layers == 0 {
            return Err(breach(format!("image `{label}` is imported with no mip or no layer")));
        }
        // Rule 8.
        if import.last.state == ImageState::Undefined
            || import.handoff.is_some_and(|handoff| handoff.state == ImageState::Undefined)
        {
            return Err(breach(format!("image `{label}` is left or handed off `Undefined`")));
        }
        // Rule 9.
        let initial_named = import.initial.state != ImageState::Undefined;
        if present(import.last.queue) || (initial_named && present(import.initial.queue)) {
            return Err(breach(format!(
                "image `{label}` is on the present role's own queue at an end of the frame"
            )));
        }
    }
    for buffer in &declared.buffers {
        let label = &buffer.label;
        if buffer.last.state == BufferState::Undefined {
            return Err(breach(format!("buffer `{label}` is left `Undefined`")));
        }
        if present(buffer.initial.queue) || present(buffer.last.queue) {
            return Err(breach(format!(
                "buffer `{label}` is on the present role's own queue at an end of the frame"
            )));
        }
    }
    for pass in &declared.passes {
        validate_pass(declared, pass)?;
    }
    Ok(())
}

/// Rules 2 to 7 for one pass.
fn validate_pass(declared: &Declared, pass: &DeclaredPass) -> Result<()> {
    let label = &pass.label;
    let images = declared.images.len();
    let buffers = declared.buffers.len();
    let (named_images, named_buffers) = named(&pass.kind);
    // Rule 2.
    if let Some(image) = named_images.iter().find(|&&image| image as usize >= images) {
        return Err(breach(format!("pass `{label}` names image {image}, which is not declared")));
    }
    if let Some(buffer) = named_buffers.iter().find(|&&buffer| buffer as usize >= buffers) {
        return Err(breach(format!("pass `{label}` names buffer {buffer}, which is not declared")));
    }
    // Rule 3.
    if let Some(image) = twice(&named_images) {
        let name = declared.images[image as usize].label();
        return Err(breach(format!("pass `{label}` names image `{name}` twice")));
    }
    if let Some(buffer) = twice(&named_buffers) {
        let name = &declared.buffers[buffer as usize].label;
        return Err(breach(format!("pass `{label}` names buffer `{name}` twice")));
    }
    let attachment = |image: u32, mip: u32, layer: u32, depth: bool| {
        let declared_image = &declared.images[image as usize];
        let name = declared_image.label();
        let (mips, layers) = declared_image.mips_and_layers();
        // Rule 4, the attachment's half.
        if mip >= mips || layer >= layers {
            return Err(breach(format!(
                "pass `{label}` attaches mip {mip} of layer {layer} of image `{name}`, which has \
                 {mips} mips and {layers} layers"
            )));
        }
        // Rule 5.
        if let DeclaredImage::Transient(transient) = declared_image
            && transient.format.is_depth() != depth
        {
            let wrong = if depth { "as depth" } else { "as colour" };
            return Err(breach(format!(
                "pass `{label}` attaches transient `{name}` {wrong}, against its format"
            )));
        }
        Ok(())
    };
    let sampled_imports = |sampled: &[crate::id::ImageId]| {
        // Rule 6.
        match sampled
            .iter()
            .find(|image| declared.images[image.index() as usize].import().is_none())
        {
            Some(image) => Err(breach(format!(
                "pass `{label}` samples transient `{}`; only an import is sampled",
                declared.images[image.index() as usize].label()
            ))),
            None => Ok(()),
        }
    };
    match &pass.kind {
        DeclaredKind::Render { colour, depth, sampled, .. } => {
            for use_ in colour {
                attachment(use_.image.index(), use_.mip, use_.layer, false)?;
            }
            if let Some(use_) = depth {
                attachment(use_.image.index(), use_.mip, use_.layer, true)?;
            }
            sampled_imports(sampled)?;
        }
        DeclaredKind::Compute { sampled, buffers, .. } => {
            sampled_imports(sampled)?;
            // Rule 7.
            if let Some(use_) = buffers.iter().find(|use_| use_.access == BufferAccess::Index) {
                let name = &declared.buffers[use_.buffer.index() as usize].label;
                return Err(breach(format!(
                    "compute pass `{label}` declares buffer `{name}` as an index buffer"
                )));
            }
        }
        DeclaredKind::Copy { .. } => {}
    }
    Ok(())
}

/// The image and buffer ids a pass names, across all its lists.
fn named(kind: &DeclaredKind) -> (Vec<u32>, Vec<u32>) {
    let buffers_of = |buffers: &[crate::pass::BufferUse]| {
        buffers.iter().map(|use_| use_.buffer.index()).collect::<Vec<_>>()
    };
    match kind {
        DeclaredKind::Render { colour, depth, sampled, buffers, .. } => {
            let images = colour
                .iter()
                .map(|use_| use_.image)
                .chain(depth.map(|use_| use_.image))
                .chain(sampled.iter().copied())
                .map(crate::id::ImageId::index)
                .collect();
            (images, buffers_of(buffers))
        }
        DeclaredKind::Compute { sampled, buffers, .. } => {
            (sampled.iter().map(|image| image.index()).collect(), buffers_of(buffers))
        }
        DeclaredKind::Copy {
            image_sources,
            image_destinations,
            buffer_sources,
            buffer_destinations,
        } => (
            image_sources.iter().chain(image_destinations).map(|image| image.index()).collect(),
            buffer_sources.iter().chain(buffer_destinations).map(|buffer| buffer.index()).collect(),
        ),
    }
}

/// The first id that occurs twice in `ids`.
fn twice(ids: &[u32]) -> Option<u32> {
    ids.iter().enumerate().find(|&(at, id)| ids[..at].contains(id)).map(|(_, &id)| id)
}

/// What the key hashes, and what a hit compares: the declaration but for its handles, its
/// `ready` points, its clear values, its slots and its recorders, with the device's
/// layout, in the order of `README.md`, "Compile and its cache".
#[derive(PartialEq, Eq, Hash)]
pub(crate) struct Shape {
    views: u32,
    queues: QueueLayout,
    images: Vec<ImageShape>,
    buffers: Vec<BufferShape>,
    passes: Vec<PassShape>,
}

#[derive(PartialEq, Eq, Hash)]
enum ImageShape {
    Import {
        label: String,
        mips: u32,
        layers: u32,
        sharing: Sharing,
        initial: StateOn<ImageState>,
        last: StateOn<ImageState>,
        handoff: Option<StateOn<ImageState>>,
    },
    Transient {
        label: String,
        format: Format,
        extent: Extent,
        mips: u32,
        layers: u32,
    },
}

#[derive(PartialEq, Eq, Hash)]
struct BufferShape {
    label: String,
    sharing: Sharing,
    initial: StateOn<BufferState>,
    last: StateOn<BufferState>,
}

/// A pass: its kind, its label and each use under the list it is declared in.
#[derive(PartialEq, Eq, Hash)]
enum PassShape {
    Render {
        label: String,
        colour: Vec<AttachmentShape>,
        depth: Option<AttachmentShape>,
        sampled: Vec<u32>,
        buffers: Vec<(u32, BufferAccess)>,
    },
    Compute {
        label: String,
        sampled: Vec<u32>,
        buffers: Vec<(u32, BufferAccess)>,
    },
    Copy {
        label: String,
        image_sources: Vec<u32>,
        image_destinations: Vec<u32>,
        buffer_sources: Vec<u32>,
        buffer_destinations: Vec<u32>,
    },
}

#[derive(PartialEq, Eq, Hash)]
struct AttachmentShape {
    image: u32,
    mip: u32,
    layer: u32,
    local_read: bool,
    load: LoadVariant,
}

/// A load's variant: a clear's value is not hashed.
#[derive(PartialEq, Eq, Hash)]
enum LoadVariant {
    Clear,
    DontCare,
    Keep,
}

impl<C> From<Load<C>> for LoadVariant {
    fn from(load: Load<C>) -> Self {
        match load {
            Load::Clear(_) => Self::Clear,
            Load::DontCare => Self::DontCare,
            Load::Keep => Self::Keep,
        }
    }
}

impl Shape {
    /// The shape of `declared` on a device of `queues`.
    pub(crate) fn of(declared: &Declared, queues: QueueLayout) -> Self {
        let images = declared
            .images
            .iter()
            .map(|image| match image {
                DeclaredImage::Import(import) => ImageShape::Import {
                    label: import.label.clone(),
                    mips: import.mips,
                    layers: import.layers,
                    sharing: import.sharing,
                    initial: import.initial,
                    last: import.last,
                    handoff: import.handoff,
                },
                DeclaredImage::Transient(transient) => ImageShape::Transient {
                    label: transient.label.clone(),
                    format: transient.format,
                    extent: transient.extent,
                    mips: transient.mips,
                    layers: transient.layers,
                },
            })
            .collect();
        let buffers = declared
            .buffers
            .iter()
            .map(|buffer| BufferShape {
                label: buffer.label.clone(),
                sharing: buffer.sharing,
                initial: buffer.initial,
                last: buffer.last,
            })
            .collect();
        let ids =
            |images: &[crate::id::ImageId]| images.iter().map(|image| image.index()).collect();
        let accesses = |buffers: &[crate::pass::BufferUse]| {
            buffers.iter().map(|use_| (use_.buffer.index(), use_.access)).collect()
        };
        let passes = declared
            .passes
            .iter()
            .map(|pass| {
                let label = pass.label.clone();
                match &pass.kind {
                    DeclaredKind::Render { colour, depth, sampled, buffers, .. } => {
                        PassShape::Render {
                            label,
                            colour: colour.iter().map(colour_shape).collect(),
                            depth: depth.as_ref().map(depth_shape),
                            sampled: ids(sampled),
                            buffers: accesses(buffers),
                        }
                    }
                    DeclaredKind::Compute { sampled, buffers, .. } => PassShape::Compute {
                        label,
                        sampled: ids(sampled),
                        buffers: accesses(buffers),
                    },
                    DeclaredKind::Copy {
                        image_sources,
                        image_destinations,
                        buffer_sources,
                        buffer_destinations,
                    } => PassShape::Copy {
                        label,
                        image_sources: ids(image_sources),
                        image_destinations: ids(image_destinations),
                        buffer_sources: buffer_sources.iter().map(|id| id.index()).collect(),
                        buffer_destinations: buffer_destinations
                            .iter()
                            .map(|id| id.index())
                            .collect(),
                    },
                }
            })
            .collect();
        Self { views: declared.views, queues, images, buffers, passes }
    }

    /// The key: the shape hashed with a fixed hasher, so equal in every process of one
    /// build.
    pub(crate) fn key(&self) -> GraphKey {
        let mut hasher = FixedHasher::default();
        self.hash(&mut hasher);
        GraphKey::new(hasher.finish())
    }
}

fn colour_shape(use_: &ColourUse) -> AttachmentShape {
    AttachmentShape {
        image: use_.image.index(),
        mip: use_.mip,
        layer: use_.layer,
        local_read: use_.local_read,
        load: use_.load.into(),
    }
}

fn depth_shape(use_: &DepthUse) -> AttachmentShape {
    AttachmentShape {
        image: use_.image.index(),
        mip: use_.mip,
        layer: use_.layer,
        local_read: false,
        load: use_.load.into(),
    }
}

/// The state a use needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Need {
    Image(ImageState),
    Buffer(BufferState),
}

/// One use of a resource by a pass (`README.md`, "Uses and order").
#[derive(Clone, Copy, Debug)]
struct Use {
    resource: Resource,
    need: Need,
    writes: bool,
}

/// A pass's uses, in the order its lists declare them.
fn uses_of(kind: &DeclaredKind) -> Vec<Use> {
    let image = |image: crate::id::ImageId, state, writes| Use {
        resource: Resource::Image(image.index()),
        need: Need::Image(state),
        writes,
    };
    let buffer = |buffer: crate::id::BufferId, state, writes| Use {
        resource: Resource::Buffer(buffer.index()),
        need: Need::Buffer(state),
        writes,
    };
    let declared = |buffers: &[crate::pass::BufferUse]| {
        buffers
            .iter()
            .map(|use_| match use_.access {
                BufferAccess::Read => buffer(use_.buffer, BufferState::ShaderRead, false),
                BufferAccess::Write => buffer(use_.buffer, BufferState::ShaderWrite, true),
                BufferAccess::Index => buffer(use_.buffer, BufferState::Index, false),
                BufferAccess::Indirect => buffer(use_.buffer, BufferState::Indirect, false),
            })
            .collect::<Vec<_>>()
    };
    match kind {
        DeclaredKind::Render { colour, depth, sampled, buffers, .. } => {
            let mut uses: Vec<Use> = colour
                .iter()
                .map(|use_| {
                    let state = if use_.local_read {
                        ImageState::LocalRead
                    } else {
                        ImageState::ColourAttachment
                    };
                    image(use_.image, state, true)
                })
                .collect();
            uses.extend(depth.map(|use_| image(use_.image, ImageState::DepthAttachment, true)));
            uses.extend(sampled.iter().map(|&id| image(id, ImageState::SampledRead, false)));
            uses.extend(declared(buffers));
            uses
        }
        DeclaredKind::Compute { sampled, buffers, .. } => {
            let mut uses: Vec<Use> =
                sampled.iter().map(|&id| image(id, ImageState::SampledRead, false)).collect();
            uses.extend(declared(buffers));
            uses
        }
        DeclaredKind::Copy {
            image_sources,
            image_destinations,
            buffer_sources,
            buffer_destinations,
        } => image_sources
            .iter()
            .map(|&id| image(id, ImageState::CopySource, false))
            .chain(
                image_destinations.iter().map(|&id| image(id, ImageState::CopyDestination, true)),
            )
            .chain(buffer_sources.iter().map(|&id| buffer(id, BufferState::CopySource, false)))
            .chain(
                buffer_destinations
                    .iter()
                    .map(|&id| buffer(id, BufferState::CopyDestination, true)),
            )
            .collect(),
    }
}

/// Whether a resource in `state` is one a pass writes, which rhi-api orders against the
/// next use by a transition even into the same state ("States").
const fn image_writes(state: ImageState) -> bool {
    matches!(
        state,
        ImageState::ColourAttachment
            | ImageState::LocalRead
            | ImageState::DepthAttachment
            | ImageState::CopyDestination
    )
}

const fn buffer_writes(state: BufferState) -> bool {
    matches!(state, BufferState::ShaderWrite | BufferState::CopyDestination)
}

/// The two queues a graph records for, as indices.
const fn queue_index(role: QueueRole) -> usize {
    match role {
        QueueRole::Compute => 1,
        QueueRole::Graphics | QueueRole::Present => 0,
    }
}

/// What a derived plan needs made: the transients, by image, and the blocks' sizes.
pub(crate) struct Derived {
    pub(crate) parts: PlanParts,
    /// For each declared image, the usage and memory of the transient to make.
    pub(crate) transients: Vec<Option<(ImageUsage, TransientMemory)>>,
    /// The size of each memory block to make.
    pub(crate) blocks: Vec<u64>,
}

/// A run of the frame's submits, on one queue: the kept passes it carries, by graph
/// position, and the transitions at its start and its end.
struct Segment {
    queue: QueueRole,
    passes: Vec<usize>,
    start: Transitions,
    end: Transitions,
}

impl Segment {
    fn new(queue: QueueRole) -> Self {
        Self {
            queue,
            passes: Vec::new(),
            start: Transitions::default(),
            end: Transitions::default(),
        }
    }
}

/// Where the frame so far left a resource.
#[derive(Clone, Copy)]
struct Track<S> {
    at: StateOn<S>,
    /// The graph position of its last use so far, `None` before its first.
    last: Option<usize>,
}

/// The plan of a valid declaration (`README.md`, "Uses and order" to "Submission").
pub(crate) fn derive(
    declared: &Declared,
    device: &dyn Device,
    caps: &Capabilities,
    jobs_cap: u32,
) -> Result<Derived> {
    let layout = caps.queues;
    let uses: Vec<Vec<Use>> = declared.passes.iter().map(|pass| uses_of(&pass.kind)).collect();
    let is_import = |resource: Resource| match resource {
        Resource::Image(image) => declared.images[image as usize].import().is_some(),
        Resource::Buffer(_) => true,
    };

    // Culling: from the last pass back, a pass is kept when it writes an import or a
    // transient that a kept pass after it uses.
    let mut culled = vec![true; declared.passes.len()];
    let mut needed = vec![false; declared.images.len()];
    for pass in (0..declared.passes.len()).rev() {
        let keeps = uses[pass].iter().any(|use_| {
            use_.writes
                && (is_import(use_.resource)
                    || matches!(use_.resource, Resource::Image(image) if needed[image as usize]))
        });
        if keeps {
            culled[pass] = false;
            for use_ in &uses[pass] {
                if let Resource::Image(image) = use_.resource {
                    needed[image as usize] = true;
                }
            }
        }
    }
    let order: Vec<usize> = (0..declared.passes.len()).filter(|&pass| !culled[pass]).collect();

    let roles = queues(declared, &uses, &order, layout);

    // The runs of graph order by queue.
    let mut segments: Vec<Segment> = Vec::new();
    let mut segment_of = vec![0; order.len()];
    for (at, &pass) in order.iter().enumerate() {
        if segments.last().is_none_or(|segment| segment.queue != roles[pass]) {
            segments.push(Segment::new(roles[pass]));
        }
        segment_of[at] = segments.len() - 1;
        segments.last_mut().expect("pushed above").passes.push(at);
    }

    // The leading submits of transitions alone: a release at the frame's edge on a queue
    // with no run before the acquiring one.
    let first_use = first_uses(declared, &uses, &order);
    let mut leading = [false; 2];
    for &(resource, at) in &first_use {
        let Some(at) = at else { continue };
        let Some(from) = edge_release(declared, resource, roles[order[at]], layout) else {
            continue;
        };
        let before = &segments[..segment_of[at]];
        if !before.iter().any(|segment| segment.queue == from) {
            leading[queue_index(from)] = true;
        }
    }
    let lead: Vec<QueueRole> = [QueueRole::Graphics, QueueRole::Compute]
        .into_iter()
        .filter(|&role| leading[queue_index(role)])
        .collect();
    let shift = lead.len();
    segments.splice(0..0, lead.into_iter().map(Segment::new));
    for segment in &mut segment_of {
        *segment += shift;
    }

    // Transients: lifetimes in graph order, usage, memory, and placement.
    let transients = plan_transients(declared, device, caps, &uses, &order)?;

    // The frame, pass by pass: the transitions before each pass and after its use.
    let mut images: Vec<Track<ImageState>> = declared
        .images
        .iter()
        .map(|image| Track {
            at: image.import().map_or(
                StateOn { state: ImageState::Undefined, queue: QueueRole::Graphics },
                |import| import.initial,
            ),
            last: None,
        })
        .collect();
    let mut buffers: Vec<Track<BufferState>> =
        declared.buffers.iter().map(|buffer| Track { at: buffer.initial, last: None }).collect();
    let mut after: Vec<Transitions> = order.iter().map(|_| Transitions::default()).collect();
    let mut steps: Vec<PassStep> = Vec::with_capacity(order.len());
    for (at, &pass) in order.iter().enumerate() {
        let queue = roles[pass];
        let segment = segment_of[at];
        let mut before = Transitions::default();
        let mut alias = Vec::new();
        for use_ in &uses[pass] {
            match (use_.resource, use_.need) {
                (Resource::Image(image), Need::Image(state)) => {
                    let index = image as usize;
                    let (mips, layers) = declared.images[index].mips_and_layers();
                    let range = ImageRange { mip: 0, mips, layer: 0, layers };
                    let to = StateOn { state, queue };
                    let track = &mut images[index];
                    let (sharing, undefined_first) = match &declared.images[index] {
                        DeclaredImage::Import(import) => {
                            (import.sharing, track.at.state == ImageState::Undefined)
                        }
                        DeclaredImage::Transient(_) => {
                            let first = track.last.is_none();
                            if first && transients.aliased[index] {
                                alias.push(image);
                            }
                            (Sharing::Exclusive, first)
                        }
                    };
                    let transition = |from| PlannedImageTransition { image, range, from, to };
                    if undefined_first {
                        // From `Undefined`, for the queue that uses it, with no transfer.
                        let from = StateOn { state: ImageState::Undefined, ..track.at };
                        before.images.push(transition(from));
                    } else if sharing == Sharing::Concurrent
                        || layout.same_queue(track.at.queue, queue)
                    {
                        if track.at.state != state || image_writes(state) {
                            before.images.push(transition(track.at));
                        } else {
                            *track = Track { at: StateOn { queue, ..track.at }, last: Some(at) };
                            continue;
                        }
                    } else {
                        let release = transition(track.at);
                        match track.last {
                            Some(last) => after[last].images.push(release),
                            None => {
                                let from = layout.serving(track.at.queue);
                                let edge = edge_segment(&segments, from, segment);
                                segments[edge].start.images.push(release);
                            }
                        }
                        before.images.push(release);
                    }
                    *track = Track { at: to, last: Some(at) };
                }
                (Resource::Buffer(buffer), Need::Buffer(state)) => {
                    let index = buffer as usize;
                    let sharing = declared.buffers[index].sharing;
                    let to = StateOn { state, queue };
                    let track = &mut buffers[index];
                    let transition = |from| PlannedBufferTransition { buffer, from, to };
                    let same =
                        sharing == Sharing::Concurrent || layout.same_queue(track.at.queue, queue);
                    if same {
                        if track.at.state != state || buffer_writes(state) {
                            before.buffers.push(transition(track.at));
                        } else {
                            *track = Track { at: StateOn { queue, ..track.at }, last: Some(at) };
                            continue;
                        }
                    } else {
                        let from = track.at;
                        let mut releasing = Vec::new();
                        if from.state == BufferState::Undefined {
                            // Taken on the queue the caller names first, which keeps its
                            // bytes, then moved.
                            let held = StateOn { state, queue: from.queue };
                            releasing.push(PlannedBufferTransition { buffer, from, to: held });
                            releasing.push(transition(held));
                        } else {
                            releasing.push(transition(from));
                        }
                        let acquire = *releasing.last().expect("pushed above");
                        match track.last {
                            Some(last) => after[last].buffers.extend(releasing),
                            None => {
                                let edge =
                                    edge_segment(&segments, layout.serving(from.queue), segment);
                                segments[edge].start.buffers.extend(releasing);
                            }
                        }
                        before.buffers.push(acquire);
                    }
                    *track = Track { at: to, last: Some(at) };
                }
                _ => unreachable!("a use's need is of its resource's kind"),
            }
        }
        steps.push(PassStep { pass, alias, before, begin: begin(declared, pass, at, &transients) });
    }

    // The frame's end: every import to its `last`, then its hand-off.
    for (index, image) in declared.images.iter().enumerate() {
        let DeclaredImage::Import(import) = image else { continue };
        let image = count(index);
        let (mips, layers) = (import.mips, import.layers);
        let range = ImageRange { mip: 0, mips, layer: 0, layers };
        let transition = |from, to| PlannedImageTransition { image, range, from, to };
        let track = images[index];
        let last = import.last;
        let exclusive = import.sharing == Sharing::Exclusive;
        let closing = Closing {
            exclusive,
            used: track.last,
            at: track.at,
            initial: import.initial,
            last,
            undefined_initial: import.initial.state == ImageState::Undefined,
            take_first: false,
            changes: track.at.state != last.state,
            unused_changes: import.initial.state != last.state,
        };
        let placed =
            close(&mut segments, &segment_of, &mut after, layout, &closing, |list, from, to| {
                list.images.push(transition(from, to));
            });
        if let Some(handoff) = import.handoff
            && exclusive
            && !layout.same_queue(last.queue, handoff.queue)
        {
            let release = transition(last, handoff);
            match placed {
                Placed::After(at) => after[at].images.push(release),
                Placed::End(segment) => segments[segment].end.images.push(release),
                Placed::Nothing => {
                    let segment = place_last(&mut segments, layout.serving(last.queue));
                    segments[segment].end.images.push(release);
                }
            }
        }
    }
    for (index, buffer) in declared.buffers.iter().enumerate() {
        let id = count(index);
        let track = buffers[index];
        let closing = Closing {
            exclusive: buffer.sharing == Sharing::Exclusive,
            used: track.last,
            at: track.at,
            initial: buffer.initial,
            last: buffer.last,
            undefined_initial: false,
            take_first: buffer.initial.state == BufferState::Undefined,
            changes: track.at.state != buffer.last.state,
            unused_changes: buffer.initial.state != buffer.last.state,
        };
        close(&mut segments, &segment_of, &mut after, layout, &closing, |list, from, to| {
            list.buffers.push(PlannedBufferTransition { buffer: id, from, to });
        });
    }
    if segments.is_empty() {
        // A frame with nothing to record still makes the submit that signals its point.
        segments.push(Segment::new(QueueRole::Graphics));
    }

    // Jobs: each run split into at most `jobs_cap` runs of consecutive passes.
    let mut steps: Vec<Option<PassStep>> = steps.into_iter().map(Some).collect();
    let mut jobs: Vec<JobPlan> = Vec::new();
    let mut submits: Vec<SubmitPlan> = Vec::new();
    let mut passes: Vec<PlannedPass> = Vec::with_capacity(order.len());
    for (number, segment) in segments.into_iter().enumerate() {
        let Segment { queue, passes: carried, mut start, mut end } = segment;
        let first_job = jobs.len();
        if carried.is_empty() {
            start.append(&mut end);
            let steps = if start.is_empty() { Vec::new() } else { vec![Step::Transitions(start)] };
            jobs.push(JobPlan { queue, passes: 0..0, steps });
        } else {
            let split = (jobs_cap as usize).min(carried.len());
            let (base, extra) = (carried.len() / split, carried.len() % split);
            let mut rest = &carried[..];
            for job in 0..split {
                let (mine, tail) = rest.split_at(base + usize::from(job < extra));
                rest = tail;
                let mut job_steps = Vec::new();
                if job == 0 && !start.is_empty() {
                    job_steps.push(Step::Transitions(core::mem::take(&mut start)));
                }
                for &at in mine {
                    let step = steps[at].take().expect("each pass is in one job");
                    passes.push(PlannedPass {
                        pass: crate::id::PassId::new(count(step.pass)),
                        queue,
                        submit: count(number),
                    });
                    job_steps.push(Step::Pass(step));
                    push_transitions(&mut job_steps, core::mem::take(&mut after[at]));
                }
                if job + 1 == split {
                    push_transitions(&mut job_steps, core::mem::take(&mut end));
                }
                let first = order[mine[0]];
                let last = order[mine[mine.len() - 1]];
                jobs.push(JobPlan { queue, passes: first..last + 1, steps: job_steps });
            }
        }
        submits.push(SubmitPlan {
            queue,
            jobs: first_job..jobs.len(),
            waits: Vec::new(),
            names: Vec::new(),
        });
    }
    order_queues(declared, &uses, &jobs, &mut submits);

    Ok(Derived {
        parts: PlanParts { views: declared.views, queues: layout, passes, culled, submits, jobs },
        transients: transients.made,
        blocks: transients.blocks,
    })
}

/// Appends `transitions` to a job's steps, into the step before where that is a step of
/// transitions too.
fn push_transitions(steps: &mut Vec<Step>, mut transitions: Transitions) {
    if transitions.is_empty() {
        return;
    }
    if let Some(Step::Transitions(previous)) = steps.last_mut() {
        previous.append(&mut transitions);
    } else {
        steps.push(Step::Transitions(transitions));
    }
}

/// Each kept pass's role (`README.md`, "Queues"): a compute pass is the compute role's
/// where that role has a queue of its own and the pass shares no resource with a
/// graphics-role pass of the frame, applied until nothing changes.
fn queues(
    declared: &Declared,
    uses: &[Vec<Use>],
    order: &[usize],
    layout: QueueLayout,
) -> Vec<QueueRole> {
    let mut roles: Vec<QueueRole> = declared
        .passes
        .iter()
        .map(|pass| match pass.kind {
            DeclaredKind::Compute { .. } if layout.compute == QueuePath::Own => QueueRole::Compute,
            _ => QueueRole::Graphics,
        })
        .collect();
    let independent = |a: &Use, b: &Use| {
        let initial = match a.resource {
            Resource::Image(image) => match &declared.images[image as usize] {
                DeclaredImage::Import(import) if import.sharing == Sharing::Concurrent => {
                    Some(Need::Image(import.initial.state))
                }
                _ => None,
            },
            Resource::Buffer(buffer) => {
                let buffer = &declared.buffers[buffer as usize];
                (buffer.sharing == Sharing::Concurrent)
                    .then_some(Need::Buffer(buffer.initial.state))
            }
        };
        !a.writes && !b.writes && a.need == b.need && initial == Some(a.need)
    };
    let depends = |a: usize, b: usize| {
        uses[a].iter().any(|ours| {
            uses[b]
                .iter()
                .any(|theirs| ours.resource == theirs.resource && !independent(ours, theirs))
        })
    };
    loop {
        let mut changed = false;
        for &pass in order {
            if roles[pass] != QueueRole::Compute {
                continue;
            }
            let graphics = order.iter().any(|&other| {
                other != pass && roles[other] == QueueRole::Graphics && depends(pass, other)
            });
            if graphics {
                roles[pass] = QueueRole::Graphics;
                changed = true;
            }
        }
        if !changed {
            return roles;
        }
    }
}

/// Each import's first kept use, by graph position.
fn first_uses(
    declared: &Declared,
    uses: &[Vec<Use>],
    order: &[usize],
) -> Vec<(Resource, Option<usize>)> {
    let images = (0..declared.images.len())
        .filter(|&image| declared.images[image].import().is_some())
        .map(|image| Resource::Image(count(image)));
    let buffers = (0..declared.buffers.len()).map(|buffer| Resource::Buffer(count(buffer)));
    images
        .chain(buffers)
        .map(|resource| {
            let first = order
                .iter()
                .position(|&pass| uses[pass].iter().any(|use_| use_.resource == resource));
            (resource, first)
        })
        .collect()
}

/// The role whose queue releases `resource` at the frame's edge, where its first use, on
/// `queue`, is on another queue than its `initial`'s and it is not taken from `Undefined`
/// with no transfer.
fn edge_release(
    declared: &Declared,
    resource: Resource,
    queue: QueueRole,
    layout: QueueLayout,
) -> Option<QueueRole> {
    let (sharing, initial, discarded) = match resource {
        Resource::Image(image) => {
            let import = declared.images[image as usize].import()?;
            (import.sharing, import.initial.queue, import.initial.state == ImageState::Undefined)
        }
        Resource::Buffer(buffer) => {
            let buffer = &declared.buffers[buffer as usize];
            (buffer.sharing, buffer.initial.queue, false)
        }
    };
    (sharing == Sharing::Exclusive && !discarded && !layout.same_queue(initial, queue))
        .then(|| layout.serving(initial))
}

/// The first segment of `queue` before `segment`, which the leading submits guarantee.
fn edge_segment(segments: &[Segment], queue: QueueRole, segment: usize) -> usize {
    segments[..segment]
        .iter()
        .position(|candidate| candidate.queue == queue)
        .expect("a leading submit is made for every release at the frame's edge")
}

/// The first segment of `queue` after `after`, made at the frame's end where there is none.
fn place_after(segments: &mut Vec<Segment>, queue: QueueRole, after: Option<usize>) -> usize {
    let from = after.map_or(0, |after| after + 1);
    match segments.iter().skip(from).position(|segment| segment.queue == queue) {
        Some(at) => from + at,
        None => {
            segments.push(Segment::new(queue));
            segments.len() - 1
        }
    }
}

/// The last segment of `queue`, made at the frame's end where there is none.
fn place_last(segments: &mut Vec<Segment>, queue: QueueRole) -> usize {
    match segments.iter().rposition(|segment| segment.queue == queue) {
        Some(at) => at,
        None => {
            segments.push(Segment::new(queue));
            segments.len() - 1
        }
    }
}

/// An import at the frame's end.
struct Closing<S> {
    exclusive: bool,
    /// Its last kept use, by graph position.
    used: Option<usize>,
    /// Where the frame left it.
    at: StateOn<S>,
    initial: StateOn<S>,
    last: StateOn<S>,
    /// An image taken from `Undefined` with no transfer.
    undefined_initial: bool,
    /// A buffer imported from `Undefined`, taken on its `initial` queue before a transfer.
    take_first: bool,
    /// Whether `at` and `last` differ in state.
    changes: bool,
    /// Whether `initial` and `last` differ in state.
    unused_changes: bool,
}

/// Where an import's last command at the frame's end was placed, for its hand-off.
enum Placed {
    /// In the step after the pass at this graph position.
    After(usize),
    /// At the end of this segment.
    End(usize),
    /// Nowhere: the frame left it in its `last`.
    Nothing,
}

/// Brings an import to its `last` after its last use, or from its `initial` where no kept
/// pass used it (`README.md`, "Imports", "Transitions and store actions").
fn close<S: Copy + PartialEq>(
    segments: &mut Vec<Segment>,
    segment_of: &[usize],
    after: &mut [Transitions],
    layout: QueueLayout,
    closing: &Closing<S>,
    mut push: impl FnMut(&mut Transitions, StateOn<S>, StateOn<S>),
) -> Placed {
    let last = closing.last;
    let target = layout.serving(last.queue);
    match closing.used {
        Some(used) => {
            let from = closing.at;
            if closing.exclusive && !layout.same_queue(from.queue, last.queue) {
                push(&mut after[used], from, last);
                let acquire = place_after(segments, target, Some(segment_of[used]));
                push(&mut segments[acquire].end, from, last);
                Placed::End(acquire)
            } else if closing.changes {
                push(&mut after[used], from, last);
                Placed::After(used)
            } else {
                Placed::After(used)
            }
        }
        None => {
            let initial = closing.initial;
            let crosses = closing.exclusive && !layout.same_queue(initial.queue, last.queue);
            if closing.undefined_initial || (!crosses && closing.unused_changes) {
                let segment = place_last(segments, target);
                push(&mut segments[segment].end, initial, last);
                Placed::End(segment)
            } else if crosses {
                let source = layout.serving(initial.queue);
                let release = place_after(segments, source, None);
                // A buffer imported from `Undefined` is taken first on the queue the caller
                // names, which keeps its bytes (`README.md`, "Imports"), and released from
                // there.
                let from = if closing.take_first {
                    let held = StateOn { state: last.state, queue: initial.queue };
                    push(&mut segments[release].end, initial, held);
                    held
                } else {
                    initial
                };
                push(&mut segments[release].end, from, last);
                let acquire = place_after(segments, target, Some(release));
                push(&mut segments[acquire].end, from, last);
                Placed::End(acquire)
            } else {
                Placed::Nothing
            }
        }
    }
}

/// What the plan decided for the transients.
struct TransientPlans {
    /// For each image, the usage and memory of the transient to make.
    made: Vec<Option<(ImageUsage, TransientMemory)>>,
    /// For each image, whether it is placed over bytes another transient of the plan takes.
    aliased: Vec<bool>,
    /// For each image, the graph position of its last kept use.
    last: Vec<Option<usize>>,
    blocks: Vec<u64>,
}

/// Each transient's usage, memory and placement (`README.md`, "Transients").
fn plan_transients(
    declared: &Declared,
    device: &dyn Device,
    caps: &Capabilities,
    uses: &[Vec<Use>],
    order: &[usize],
) -> Result<TransientPlans> {
    let images = declared.images.len();
    let mut made = vec![None; images];
    let mut last = vec![None; images];
    // Placed candidates: (image, first use, last use, size, alignment).
    let mut placed: Vec<(usize, usize, usize, u64, u64)> = Vec::new();
    for (index, image) in declared.images.iter().enumerate() {
        let DeclaredImage::Transient(transient) = image else { continue };
        let resource = Resource::Image(count(index));
        let kept: Vec<(usize, Use)> = order
            .iter()
            .enumerate()
            .flat_map(|(at, &pass)| {
                uses[pass]
                    .iter()
                    .filter(|use_| use_.resource == resource)
                    .map(move |&use_| (at, use_))
            })
            .collect();
        let (Some(&(first, _)), Some(&(final_use, _))) = (kept.first(), kept.last()) else {
            continue;
        };
        last[index] = Some(final_use);
        let mut usage = ImageUsage::EMPTY;
        for (_, use_) in &kept {
            usage = usage.union(match use_.need {
                Need::Image(ImageState::ColourAttachment) => ImageUsage::COLOUR,
                Need::Image(ImageState::LocalRead) => ImageUsage::COLOUR.union(ImageUsage::INPUT),
                Need::Image(ImageState::DepthAttachment) => ImageUsage::DEPTH,
                Need::Image(ImageState::CopySource) => ImageUsage::COPY_SOURCE,
                Need::Image(ImageState::CopyDestination) => ImageUsage::COPY_DESTINATION,
                _ => unreachable!("a transient is attached or copied (rule 6)"),
            });
        }
        let memoryless = transient.mips == 1
            && transient.layers == 1
            && kept.len() == 1
            && memoryless_attachment(declared, caps, order[first], index);
        if memoryless {
            made[index] = Some((usage.union(ImageUsage::TRANSIENT), TransientMemory::Memoryless));
            continue;
        }
        let desc = ImageDesc {
            label: &transient.label,
            format: transient.format,
            extent: transient.extent,
            mips: transient.mips,
            layers: transient.layers,
            usage,
            memory: ImageMemory::Own,
            sharing: Sharing::Exclusive,
        };
        match device.image_placement(&desc) {
            Ok(placement) => {
                made[index] = Some((usage, TransientMemory::Own));
                placed.push((index, first, final_use, placement.size, placement.alignment.max(1)));
            }
            Err(error) if error.kind() == ErrorKind::Unsupported => {
                made[index] = Some((usage, TransientMemory::Own));
            }
            Err(error) => return Err(error),
        }
    }

    // First fit in the order of first use, declaration order breaking a tie, in one block
    // the plan sizes to what it holds.
    placed.sort_by_key(|&(image, first, ..)| (first, image));
    let mut done: Vec<(usize, usize, usize, u64, u64)> = Vec::new();
    for &(image, first, final_use, size, alignment) in &placed {
        let live: Vec<(u64, u64)> = done
            .iter()
            .filter(|&&(_, other_first, other_last, ..)| {
                first <= other_last && other_first <= final_use
            })
            .map(|&(.., offset, other_size)| (offset, offset + other_size))
            .collect();
        let offset = core::iter::once(0)
            .chain(live.iter().map(|&(_, end)| end.next_multiple_of(alignment)))
            .filter(|&offset| {
                live.iter().all(|&(start, end)| offset + size <= start || end <= offset)
            })
            .min()
            .expect("the end of the highest live range is free");
        done.push((image, first, final_use, offset, size));
        made[image] = Some((
            made[image].expect("placed above").0,
            TransientMemory::Placed { block: 0, offset, size },
        ));
    }
    let mut aliased = vec![false; images];
    for &(image, _, _, offset, size) in &done {
        aliased[image] = done.iter().any(|&(other, _, _, other_offset, other_size)| {
            other != image && offset < other_offset + other_size && other_offset < offset + size
        });
    }
    let blocks = done.iter().map(|&(.., offset, size)| offset + size).max().into_iter().collect();
    Ok(TransientPlans { made, aliased, last, blocks })
}

/// Whether the one kept use of transient `image`, an attachment of `pass` or not, may be
/// memoryless (`README.md`, "Transients").
fn memoryless_attachment(
    declared: &Declared,
    caps: &Capabilities,
    pass: usize,
    image: usize,
) -> bool {
    let DeclaredKind::Render { colour, depth, .. } = &declared.passes[pass].kind else {
        return false;
    };
    let reads_locally = colour.iter().any(|use_| use_.local_read);
    if reads_locally && !caps.memoryless_survives_local_read {
        return false;
    }
    if let Some(use_) = depth.filter(|use_| use_.image.index() as usize == image) {
        return caps.memoryless_depth && use_.load != Load::Keep;
    }
    colour
        .iter()
        .find(|use_| use_.image.index() as usize == image)
        .is_some_and(|use_| use_.local_read && caps.memoryless_colour && use_.load != Load::Keep)
}

/// What a pass is begun with: its attachments' store actions and its reach.
fn begin(declared: &Declared, pass: usize, at: usize, transients: &TransientPlans) -> Begin {
    let store = |image: crate::id::ImageId| {
        let index = image.index() as usize;
        match transients.made[index] {
            None => Store::Store,
            Some((_, TransientMemory::Memoryless)) => Store::DontCare,
            Some(_) if transients.last[index].is_some_and(|last| last > at) => Store::Store,
            Some(_) => Store::DontCare,
        }
    };
    let reach = |buffers: &[crate::pass::BufferUse]| {
        buffers
            .iter()
            .filter_map(|use_| match use_.access {
                BufferAccess::Read => Some((use_.buffer.index(), BufferState::ShaderRead)),
                BufferAccess::Write => Some((use_.buffer.index(), BufferState::ShaderWrite)),
                BufferAccess::Index | BufferAccess::Indirect => None,
            })
            .collect()
    };
    match &declared.passes[pass].kind {
        DeclaredKind::Render { colour, depth, buffers, .. } => Begin::Render {
            colour: colour.iter().map(|use_| store(use_.image)).collect(),
            depth: depth.map(|use_| store(use_.image)),
            reach: reach(buffers),
        },
        DeclaredKind::Compute { buffers, .. } => Begin::Compute { reach: reach(buffers) },
        DeclaredKind::Copy { .. } => Begin::Copy,
    }
}

/// What one submit's commands do to one resource.
#[derive(Clone, Copy, Default)]
struct Touch {
    /// A transition or an alias call of it.
    changes: bool,
    /// A write of it by a pass.
    writes: bool,
}

/// The waits that order the frame's queues against each other, and the imports each
/// submit names (`README.md`, "Imports", "Submission"): a submit that uses a resource
/// another queue's earlier submit of the frame wrote or changed waits for it; one that
/// changes a resource another queue's earlier submit used waits for it; and the frame's
/// last submit waits for each other queue's last.
fn order_queues(
    declared: &Declared,
    uses: &[Vec<Use>],
    jobs: &[JobPlan],
    submits: &mut [SubmitPlan],
) {
    let images = declared.images.len();
    let slot = |resource: Resource| match resource {
        Resource::Image(image) => image as usize,
        Resource::Buffer(buffer) => images + buffer as usize,
    };
    let resources = images + declared.buffers.len();
    let mut used: Vec<[Option<u32>; 2]> = vec![[None; 2]; resources];
    let mut changed: Vec<[Option<u32>; 2]> = vec![[None; 2]; resources];
    let mut last_on: [Option<u32>; 2] = [None; 2];
    let final_submit = submits.len() - 1;
    for (number, submit) in submits.iter_mut().enumerate() {
        let mut touched: Vec<Touch> = vec![Touch::default(); resources];
        let mut named = vec![false; resources];
        let mut touch = |resource: Resource, change: bool, write: bool| {
            let at = slot(resource);
            named[at] = true;
            touched[at].changes |= change;
            touched[at].writes |= write;
        };
        let transitions = |list: &Transitions, touch: &mut dyn FnMut(Resource, bool, bool)| {
            for transition in &list.images {
                touch(Resource::Image(transition.image), true, false);
            }
            for transition in &list.buffers {
                touch(Resource::Buffer(transition.buffer), true, false);
            }
        };
        for job in &jobs[submit.jobs.clone()] {
            for step in &job.steps {
                match step {
                    Step::Transitions(list) => transitions(list, &mut touch),
                    Step::Pass(step) => {
                        for &image in &step.alias {
                            touch(Resource::Image(image), true, false);
                        }
                        transitions(&step.before, &mut touch);
                        for use_ in &uses[step.pass] {
                            touch(use_.resource, false, use_.writes);
                        }
                    }
                }
            }
        }
        let queue = queue_index(submit.queue);
        let other = 1 - queue;
        let mut wait: Option<u32> = None;
        for at in (0..resources).filter(|&at| named[at]) {
            wait = wait.max(changed[at][other]);
            if touched[at].changes {
                wait = wait.max(used[at][other]);
            }
        }
        if number == final_submit {
            wait = wait.max(last_on[other]);
        }
        let number = count(number);
        for at in (0..resources).filter(|&at| named[at]) {
            used[at][queue] = Some(number);
            if touched[at].changes || touched[at].writes {
                changed[at][queue] = Some(number);
            }
        }
        last_on[queue] = Some(number);
        submit.waits = wait.into_iter().collect();
        submit.names = (0..resources)
            .filter(|&at| named[at])
            .filter_map(|at| {
                if at < images {
                    declared.images[at].import().map(|_| Resource::Image(count(at)))
                } else {
                    Some(Resource::Buffer(count(at - images)))
                }
            })
            .collect();
    }
}
