//! The encoder: a list of commands, the scope recording is in, the first breach it found,
//! and what each handle it recorded resolved to. Recording takes no lock but one table
//! entry's, once for each handle between resets.

use core::ops::Range;
use std::collections::HashMap;
use std::sync::Arc;

use lactose_rhi_api::{
    BufferCopy, BufferHandle, BufferImageCopy, BufferReach, BufferState, BufferTransition,
    BufferUsage, ComputePass, ComputePipelineHandle, Encoder, Extent, Format,
    GraphicsPipelineHandle, HeapSlot, ImageHandle, ImageRange, ImageState, ImageTransition,
    ImageUsage, IndexedIndirectArgs, Load, PUSH_BYTES_MAX, PassReach, QueueRole, Rect, RenderPass,
    Rule, Store, Target, Viewport,
};

use crate::device::{BufferObj, ComputeObj, GraphicsObj, ImageObj, Shared, range};
use crate::lock;

/// An attachment as the submit replays it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Attach {
    pub(crate) target: Target,
    /// The state its use needs.
    pub(crate) state: ImageState,
    /// Loaded with `Load::Keep`: a read.
    pub(crate) keep: bool,
    /// Stored: its contents are defined after the pass.
    pub(crate) store: bool,
}

/// A recorded command, as the submit replays it against the records. Only what the
/// records check is kept.
pub(crate) enum Cmd {
    Images(Vec<ImageTransition>),
    Buffers(Vec<BufferTransition>),
    Alias(ImageHandle),
    CopyBuffer {
        source: BufferHandle,
        destination: BufferHandle,
        regions: Vec<BufferCopy>,
    },
    CopyToImage {
        source: BufferHandle,
        destination: ImageHandle,
        subresources: Vec<(u32, u32)>,
    },
    CopyToBuffer {
        source: ImageHandle,
        subresources: Vec<(u32, u32)>,
        destination: BufferHandle,
        ranges: Vec<(u64, u64)>,
    },
    BeginRender {
        colour: Vec<Attach>,
        depth: Option<Attach>,
        slots: Vec<SlotDecl>,
        buffers: Vec<BufferReach>,
    },
    BeginCompute {
        slots: Vec<SlotDecl>,
        buffers: Vec<BufferReach>,
    },
    EndPass,
    Graphics(GraphicsPipelineHandle),
    Compute(ComputePipelineHandle),
    Draw {
        index: Option<BufferHandle>,
        indirect: Vec<BufferHandle>,
    },
    Dispatch {
        indirect: Option<BufferHandle>,
    },
}

/// What a render pass being recorded holds.
struct RenderScope {
    extent: Extent,
    formats: Vec<Format>,
    depth: Option<Format>,
    /// Bit `i`: colour attachment `i` is marked for the local read.
    marked: u32,
    /// The pass has an attachment made with a memoryless request the device honoured.
    memoryless: bool,
    slots: Vec<SlotDecl>,
    buffers: Vec<BufferHandle>,
    pipeline: Option<Arc<GraphicsObj>>,
    /// The bound index buffer, its offset and size.
    index: Option<(BufferHandle, u64, u64)>,
    /// Bit `i`: a draw has written colour attachment `i` since the last local read.
    written: u32,
    local_read: bool,
}

/// What a compute pass being recorded holds.
struct ComputeScope {
    slots: Vec<SlotDecl>,
    buffers: Vec<BufferHandle>,
    pipeline: bool,
}

enum Scope {
    Outside,
    Render(Box<RenderScope>),
    Compute(ComputeScope),
}

/// A null encoder. `Send`: what it holds is counted and owned.
pub(crate) struct NullEncoder {
    pub(crate) shared: Arc<Shared>,
    pub(crate) role: QueueRole,
    pub(crate) commands: Vec<Cmd>,
    /// The first breach found while recording; nothing is recorded after it.
    pub(crate) breach: Option<(Rule, String)>,
    /// The queue and sequence number of the submit that took it, until its reset.
    pub(crate) submitted: Option<(usize, u64)>,
    scope: Scope,
    pub(crate) buffers: HashMap<BufferHandle, Arc<BufferObj>>,
    pub(crate) images: HashMap<ImageHandle, Arc<ImageObj>>,
    pub(crate) graphics: HashMap<GraphicsPipelineHandle, Arc<GraphicsObj>>,
    pub(crate) compute: HashMap<ComputePipelineHandle, Arc<ComputeObj>>,
}

impl NullEncoder {
    pub(crate) fn new(shared: Arc<Shared>, role: QueueRole) -> Self {
        Self {
            shared,
            role,
            commands: Vec::new(),
            breach: None,
            submitted: None,
            scope: Scope::Outside,
            buffers: HashMap::new(),
            images: HashMap::new(),
            graphics: HashMap::new(),
            compute: HashMap::new(),
        }
    }

    /// Whether a recording call records: the encoder is not taken by a submit and has
    /// found no breach.
    const fn recording(&self) -> bool {
        self.submitted.is_none() && self.breach.is_none()
    }

    /// Whether a pass is left open.
    pub(crate) const fn open(&self) -> bool {
        !matches!(self.scope, Scope::Outside)
    }

    fn fail(&mut self, rule: Rule, detail: impl Into<String>) {
        if self.breach.is_none() {
            self.breach = Some((rule, detail.into()));
        }
    }

    /// Whether the encoder may record `what`, which the graphics role alone takes: the
    /// compute role's queue may be of a family that does not draw, so a `Compute` encoder
    /// records none on any layout (`lactose-rhi-api`, "Queues by role"). A render pass and a
    /// depth image's copy ask; a graphics bind and a draw need a render pass, which none
    /// begins on a `Compute` encoder, so the scope rule refuses them there.
    fn draws(&mut self, what: &str) -> bool {
        if self.role == QueueRole::Compute {
            self.fail(Rule::Encoding, format!("{what} on a compute encoder"));
            return false;
        }
        true
    }

    fn outside(&mut self, what: &str) -> bool {
        if self.open() {
            self.fail(Rule::Encoding, format!("{what} inside a pass"));
            return false;
        }
        true
    }

    fn buffer(&mut self, handle: BufferHandle) -> Option<Arc<BufferObj>> {
        if let Some(object) = self.buffers.get(&handle) {
            return Some(Arc::clone(object));
        }
        let Some(object) = self.shared.buffers.resolve(handle) else {
            self.fail(Rule::Destroyed, "a buffer that is destroyed or of another device");
            return None;
        };
        self.buffers.insert(handle, Arc::clone(&object));
        Some(object)
    }

    fn image(&mut self, handle: ImageHandle) -> Option<Arc<ImageObj>> {
        if let Some(object) = self.images.get(&handle) {
            return Some(Arc::clone(object));
        }
        let Some(object) = self.shared.images.resolve(handle) else {
            self.fail(Rule::Destroyed, "an image that is destroyed or of another device");
            return None;
        };
        if object.swapchain && self.role != QueueRole::Present {
            self.fail(Rule::PresentQueue, "a swapchain image on an encoder of another role");
            return None;
        }
        self.images.insert(handle, Arc::clone(&object));
        Some(object)
    }

    /// A buffer with every flag of `usage`, or the breach.
    fn buffer_for(&mut self, handle: BufferHandle, usage: BufferUsage) -> Option<Arc<BufferObj>> {
        let object = self.buffer(handle)?;
        if !object.usage.contains(usage) {
            self.fail(Rule::Usage, format!("a buffer used outside its usage, {usage:?}"));
            return None;
        }
        Some(object)
    }

    fn image_for(&mut self, handle: ImageHandle, usage: ImageUsage) -> Option<Arc<ImageObj>> {
        let object = self.image(handle)?;
        if !object.usage.contains(usage) {
            self.fail(Rule::Usage, format!("an image used outside its usage, {usage:?}"));
            return None;
        }
        Some(object)
    }

    fn bounded(&mut self, end: lactose_core::Result<u64>) -> bool {
        match end {
            Ok(_) => true,
            Err(error) => {
                self.fail(Rule::Bounds, error.to_string());
                false
            }
        }
    }

    /// Resolves the slots a pass declares, through each slot's entry lock.
    fn declare_slots(&mut self, slots: &[HeapSlot]) -> Option<Vec<SlotDecl>> {
        let mut slots = slots.to_vec();
        slots.sort_unstable();
        slots.dedup();
        let mut resolved = Vec::with_capacity(slots.len());
        for slot in slots {
            let Some(entry) = self.shared.slots.get(slot.0 as usize) else {
                self.fail(Rule::Bounds, format!("a declared slot {} outside the heap", slot.0));
                return None;
            };
            let entry = lock(entry).clone();
            let image = entry.map(|(handle, object)| {
                self.images.entry(handle).or_insert(object);
                handle
            });
            resolved.push((slot, image));
        }
        Some(resolved)
    }

    /// Resolves the buffers a pass declares.
    fn declare_buffers(&mut self, buffers: &[BufferReach]) -> Option<Vec<BufferHandle>> {
        let mut declared = Vec::with_capacity(buffers.len());
        for reach in buffers {
            self.buffer_for(reach.buffer, BufferUsage::DEVICE_ADDRESS)?;
            if !matches!(reach.state, BufferState::ShaderRead | BufferState::ShaderWrite) {
                self.fail(Rule::Usage, "a buffer declared in a state no shader reaches it in");
                return None;
            }
            declared.push(reach.buffer);
        }
        Some(declared)
    }

    fn reach(&mut self, reach: &PassReach<'_>) -> Option<Reached> {
        let slots = self.declare_slots(reach.slots)?;
        let declared = self.declare_buffers(reach.buffers)?;
        let mut buffers = reach.buffers.to_vec();
        buffers.sort_unstable_by_key(|reach| (reach.buffer.index(), reach.buffer.generation()));
        if buffers
            .windows(2)
            .any(|two| two[0].buffer == two[1].buffer && two[0].state != two[1].state)
        {
            self.fail(Rule::State, "a pass that declares one buffer in two states");
            return None;
        }
        buffers.dedup_by_key(|reach| reach.buffer);
        Some((slots, buffers, declared))
    }

    /// The checks a draw makes of what its pass holds, then the command.
    fn draw_command(&mut self, indexed: bool, indirect: Vec<BufferHandle>) {
        let survives = self.shared.caps.memoryless_survives_local_read;
        let checked = match &mut self.scope {
            Scope::Render(pass) => draw_checks(pass, indexed, survives),
            _ => Err((Rule::Encoding, "a draw outside a render pass".to_owned())),
        };
        match checked {
            Ok(index) => self.commands.push(Cmd::Draw { index, indirect }),
            Err((rule, detail)) => self.fail(rule, detail),
        }
    }

    fn dispatch_command(&mut self, indirect: Option<BufferHandle>) {
        let checked = match &self.scope {
            Scope::Compute(pass) if !pass.pipeline => {
                Err((Rule::Encoding, "a dispatch with no compute pipeline bound".to_owned()))
            }
            Scope::Compute(pass) => empty_slot(&pass.slots, "a dispatch"),
            _ => Err((Rule::Encoding, "a dispatch outside a compute pass".to_owned())),
        };
        match checked {
            Ok(()) => self.commands.push(Cmd::Dispatch { indirect }),
            Err((rule, detail)) => self.fail(rule, detail),
        }
    }

    /// The declarations of the open pass, if one is open.
    fn declared(&self) -> Option<(&[SlotDecl], &[BufferHandle])> {
        match &self.scope {
            Scope::Render(pass) => Some((&pass.slots, &pass.buffers)),
            Scope::Compute(pass) => Some((&pass.slots, &pass.buffers)),
            Scope::Outside => None,
        }
    }

    fn push_range(&mut self, offset: u32, len: usize) -> bool {
        let end = u64::from(offset) + len as u64;
        if end > u64::from(PUSH_BYTES_MAX) {
            self.fail(
                Rule::Bounds,
                format!("a push of {len} bytes at {offset}, past {PUSH_BYTES_MAX}"),
            );
            return false;
        }
        true
    }

    fn attachment(
        &mut self,
        target: Target,
        usage: ImageUsage,
        memoryless_store: bool,
        seen: &mut Vec<ImageHandle>,
        extent: &mut Option<Extent>,
    ) -> Option<Arc<ImageObj>> {
        let object = self.image_for(target.image, usage)?;
        if target.mip >= object.mips || target.layer >= object.layers {
            self.fail(Rule::Bounds, "an attachment of a mip or layer the image does not have");
            return None;
        }
        if seen.contains(&target.image) {
            self.fail(Rule::Usage, "an image named twice among a pass's attachments");
            return None;
        }
        seen.push(target.image);
        let size = object.mip_extent(target.mip);
        if extent.is_some_and(|extent| extent != size) {
            self.fail(Rule::Usage, "attachments of unequal extent");
            return None;
        }
        *extent = Some(size);
        if object.memoryless_request && memoryless_store {
            self.fail(
                Rule::MemorylessStore,
                "an attachment made with a memoryless request, stored or kept",
            );
            return None;
        }
        Some(object)
    }
}

impl Drop for NullEncoder {
    fn drop(&mut self) {
        if let Some((queue, seq)) = self.submitted
            && !std::thread::panicking()
        {
            let mut records = self.shared.records();
            while !records.lost && records.queues[queue].ran < seq {
                records = self.shared.wait(records, None);
            }
        }
        self.shared.let_go();
    }
}

impl Encoder for NullEncoder {
    fn queue(&self) -> QueueRole {
        self.role
    }

    fn reset(&mut self) -> lactose_core::Result<()> {
        if let Some((queue, seq)) = self.submitted {
            let records = self.shared.records();
            if !records.lost && seq > records.queues[queue].observed {
                return Err(Rule::Pending.refuse(
                    "a reset of an encoder whose submit the caller has not seen complete",
                ));
            }
        }
        self.commands.clear();
        self.breach = None;
        self.submitted = None;
        self.scope = Scope::Outside;
        self.buffers.clear();
        self.images.clear();
        self.graphics.clear();
        self.compute.clear();
        Ok(())
    }

    fn transition_images(&mut self, transitions: &[ImageTransition]) {
        if !self.recording() || !self.outside("a transition") {
            return;
        }
        for (at, transition) in transitions.iter().enumerate() {
            let Some(object) = self.image(transition.image) else { return };
            let range = transition.range;
            let inside = range.mips > 0
                && range.layers > 0
                && range.mip.checked_add(range.mips).is_some_and(|end| end <= object.mips)
                && range.layer.checked_add(range.layers).is_some_and(|end| end <= object.layers);
            if !inside {
                self.fail(Rule::Bounds, "a transition of mips or layers the image does not have");
                return;
            }
            let needs = state_usage(transition.to.state);
            if !object.usage.contains(needs) {
                self.fail(
                    Rule::Usage,
                    format!(
                        "a transition to {:?} of an image without {needs:?}",
                        transition.to.state
                    ),
                );
                return;
            }
            let repeats = transitions[..at]
                .iter()
                .any(|earlier| earlier.image == transition.image && share(earlier.range, range));
            if repeats {
                self.fail(Rule::State, "a transition call that names a subresource twice");
                return;
            }
        }
        self.commands.push(Cmd::Images(transitions.to_vec()));
    }

    fn transition_buffers(&mut self, transitions: &[BufferTransition]) {
        if !self.recording() || !self.outside("a transition") {
            return;
        }
        for (at, transition) in transitions.iter().enumerate() {
            if self.buffer(transition.buffer).is_none() {
                return;
            }
            if transitions[..at].iter().any(|earlier| earlier.buffer == transition.buffer) {
                self.fail(Rule::State, "a transition call that names a buffer twice");
                return;
            }
        }
        self.commands.push(Cmd::Buffers(transitions.to_vec()));
    }

    fn alias(&mut self, image: ImageHandle) {
        if !self.recording() || !self.outside("an alias call") {
            return;
        }
        let Some(object) = self.image(image) else { return };
        if !object.placed {
            self.fail(Rule::Usage, "an alias call of an image not placed in a block");
            return;
        }
        self.commands.push(Cmd::Alias(image));
    }

    fn copy_buffer(
        &mut self,
        source: BufferHandle,
        destination: BufferHandle,
        regions: &[BufferCopy],
    ) {
        if !self.recording() || !self.outside("a copy") {
            return;
        }
        let Some(from) = self.buffer_for(source, BufferUsage::COPY_SOURCE) else { return };
        let Some(to) = self.buffer_for(destination, BufferUsage::COPY_DESTINATION) else { return };
        if source == destination {
            self.fail(Rule::Usage, "a copy from a buffer to itself");
            return;
        }
        for region in regions {
            if region.size == 0 {
                self.fail(Rule::Bounds, "a copy region of no bytes");
                return;
            }
            let ok = self.bounded(range(
                region.source_offset,
                region.size,
                from.size,
                "a copy's source",
            )) && self.bounded(range(
                region.destination_offset,
                region.size,
                to.size,
                "a copy's destination",
            ));
            if !ok {
                return;
            }
        }
        let written: Vec<_> = regions
            .iter()
            .map(|region| (region.destination_offset, region.destination_offset + region.size))
            .collect();
        if overlap(&written) {
            self.fail(Rule::Bounds, "two regions of one copy that write one destination byte");
            return;
        }
        self.commands.push(Cmd::CopyBuffer { source, destination, regions: regions.to_vec() });
    }

    fn copy_buffer_to_image(
        &mut self,
        source: BufferHandle,
        destination: ImageHandle,
        regions: &[BufferImageCopy],
    ) {
        if !self.recording() || !self.outside("a copy") {
            return;
        }
        let Some(buffer) = self.buffer_for(source, BufferUsage::COPY_SOURCE) else { return };
        let Some(image) = self.image_for(destination, ImageUsage::COPY_DESTINATION) else { return };
        if image.format.is_depth() && !self.draws("a depth image's copy") {
            return;
        }
        let Some((subresources, _)) = self.image_regions(&buffer, &image, regions) else { return };
        let shared = regions.iter().enumerate().any(|(at, one)| {
            regions[at + 1..].iter().any(|other| {
                (one.mip, one.layer) == (other.mip, other.layer) && meet(one.rect, other.rect)
            })
        });
        if shared {
            self.fail(Rule::Bounds, "two regions of one copy that write one destination texel");
            return;
        }
        self.commands.push(Cmd::CopyToImage { source, destination, subresources });
    }

    fn copy_image_to_buffer(
        &mut self,
        source: ImageHandle,
        destination: BufferHandle,
        regions: &[BufferImageCopy],
    ) {
        if !self.recording() || !self.outside("a copy") {
            return;
        }
        let Some(image) = self.image_for(source, ImageUsage::COPY_SOURCE) else { return };
        if image.format.is_depth() && !self.draws("a depth image's copy") {
            return;
        }
        let Some(buffer) = self.buffer_for(destination, BufferUsage::COPY_DESTINATION) else {
            return;
        };
        let Some((subresources, ranges)) = self.image_regions(&buffer, &image, regions) else {
            return;
        };
        if overlap(&ranges) {
            self.fail(Rule::Bounds, "two regions of one copy that write one destination byte");
            return;
        }
        self.commands.push(Cmd::CopyToBuffer { source, subresources, destination, ranges });
    }

    fn begin_render_pass(&mut self, pass: &RenderPass<'_>) {
        if !self.recording() || !self.outside("a pass begun") || !self.draws("a render pass") {
            return;
        }
        let limit = self.shared.caps.colour_attachments;
        if pass.colour.len() > limit as usize {
            self.fail(Rule::Usage, format!("a pass of more than {limit} colour attachments"));
            return;
        }
        if pass.colour.is_empty() && pass.depth.is_none() {
            self.fail(Rule::Usage, "a render pass with no attachment");
            return;
        }
        let mut seen = Vec::new();
        let mut extent = None;
        let mut colour = Vec::with_capacity(pass.colour.len());
        let mut formats = Vec::with_capacity(pass.colour.len());
        let mut marked = 0;
        let mut memoryless = false;
        for (at, attachment) in pass.colour.iter().enumerate() {
            let usage = if attachment.local_read {
                ImageUsage::COLOUR.union(ImageUsage::INPUT)
            } else {
                ImageUsage::COLOUR
            };
            let keep = attachment.load == Load::Keep;
            let store = attachment.store == Store::Store;
            let Some(object) =
                self.attachment(attachment.target, usage, keep || store, &mut seen, &mut extent)
            else {
                return;
            };
            memoryless |= object.memoryless;
            formats.push(object.format);
            if attachment.local_read {
                marked |= 1 << at;
            }
            let state = if attachment.local_read {
                ImageState::LocalRead
            } else {
                ImageState::ColourAttachment
            };
            colour.push(Attach { target: attachment.target, state, keep, store });
        }
        let mut depth = None;
        if let Some(attachment) = pass.depth {
            let keep = attachment.load == Load::Keep;
            let store = attachment.store == Store::Store;
            let Some(object) = self.attachment(
                attachment.target,
                ImageUsage::DEPTH,
                keep || store,
                &mut seen,
                &mut extent,
            ) else {
                return;
            };
            memoryless |= object.memoryless;
            depth = Some((
                object.format,
                Attach {
                    target: attachment.target,
                    state: ImageState::DepthAttachment,
                    keep,
                    store,
                },
            ));
        }
        let Some((slots, buffers, declared)) = self.reach(&pass.reach) else { return };
        let Some(extent) = extent else { return };
        self.scope = Scope::Render(Box::new(RenderScope {
            extent,
            formats,
            depth: depth.map(|(format, _)| format),
            marked,
            memoryless,
            slots: slots.clone(),
            buffers: declared,
            pipeline: None,
            index: None,
            written: 0,
            local_read: false,
        }));
        self.commands.push(Cmd::BeginRender {
            colour,
            depth: depth.map(|(_, attach)| attach),
            slots,
            buffers,
        });
    }

    fn begin_compute_pass(&mut self, pass: &ComputePass<'_>) {
        if !self.recording() || !self.outside("a pass begun") {
            return;
        }
        let Some((slots, buffers, declared)) = self.reach(&pass.reach) else { return };
        self.scope = Scope::Compute(ComputeScope {
            slots: slots.clone(),
            buffers: declared,
            pipeline: false,
        });
        self.commands.push(Cmd::BeginCompute { slots, buffers });
    }

    fn end_pass(&mut self) {
        if !self.recording() {
            return;
        }
        if !self.open() {
            self.fail(Rule::Encoding, "a pass ended with none open");
            return;
        }
        self.scope = Scope::Outside;
        self.commands.push(Cmd::EndPass);
    }

    fn set_viewport(&mut self, viewport: Viewport) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a viewport set outside a render pass");
            return;
        }
        let caps = &self.shared.caps;
        let Viewport { x, y, width, height, near, far } = viewport;
        let fields = [x, y, width, height, near, far];
        let (low, high) = (f64::from(caps.viewport_bounds[0]), f64::from(caps.viewport_bounds[1]));
        let ok = fields.iter().all(|field| field.is_finite())
            && width > 0.0
            && height > 0.0
            && f64::from(width) <= f64::from(caps.viewport_extent[0])
            && f64::from(height) <= f64::from(caps.viewport_extent[1])
            && f64::from(x) >= low
            && f64::from(y) >= low
            && f64::from(x) + f64::from(width) <= high
            && f64::from(y) + f64::from(height) <= high
            && (0.0..=1.0).contains(&near)
            && (0.0..=1.0).contains(&far);
        if !ok {
            self.fail(Rule::Bounds, format!("a viewport outside its rules: {viewport:?}"));
        }
    }

    fn set_scissor(&mut self, scissor: Rect) {
        if !self.recording() {
            return;
        }
        let Scope::Render(pass) = &self.scope else {
            self.fail(Rule::Encoding, "a scissor set outside a render pass");
            return;
        };
        let extent = pass.extent;
        let inside = scissor
            .x
            .checked_add(scissor.extent.width)
            .is_some_and(|end| end <= extent.width)
            && scissor.y.checked_add(scissor.extent.height).is_some_and(|end| end <= extent.height);
        if !inside {
            self.fail(Rule::Bounds, "a scissor outside the attachments");
        }
    }

    fn bind_graphics_pipeline(&mut self, pipeline: GraphicsPipelineHandle) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a graphics pipeline bound outside a render pass");
            return;
        }
        let object = match self.graphics.get(&pipeline) {
            Some(object) => Arc::clone(object),
            None => {
                let Some(object) = self.shared.graphics.resolve(pipeline) else {
                    self.fail(
                        Rule::Destroyed,
                        "a graphics pipeline that is destroyed or of another device",
                    );
                    return;
                };
                self.graphics.insert(pipeline, Arc::clone(&object));
                object
            }
        };
        let Scope::Render(pass) = &mut self.scope else { return };
        if object.colour != pass.formats || object.depth != pass.depth {
            self.fail(Rule::Usage, "a pipeline of other formats than its pass's attachments");
            return;
        }
        pass.pipeline = Some(object);
        self.commands.push(Cmd::Graphics(pipeline));
    }

    fn bind_compute_pipeline(&mut self, pipeline: ComputePipelineHandle) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Compute(_)) {
            self.fail(Rule::Encoding, "a compute pipeline bound outside a compute pass");
            return;
        }
        if !self.compute.contains_key(&pipeline) {
            let Some(object) = self.shared.compute.resolve(pipeline) else {
                self.fail(
                    Rule::Destroyed,
                    "a compute pipeline that is destroyed or of another device",
                );
                return;
            };
            self.compute.insert(pipeline, object);
        }
        if let Scope::Compute(pass) = &mut self.scope {
            pass.pipeline = true;
        }
        self.commands.push(Cmd::Compute(pipeline));
    }

    fn bind_index_buffer(&mut self, buffer: BufferHandle, offset: u64) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "an index buffer bound outside a render pass");
            return;
        }
        let Some(object) = self.buffer_for(buffer, BufferUsage::INDEX) else { return };
        if !offset.is_multiple_of(4) || offset >= object.size {
            self.fail(
                Rule::Bounds,
                format!("an index buffer bound at {offset}, unaligned or outside"),
            );
            return;
        }
        if let Scope::Render(pass) = &mut self.scope {
            pass.index = Some((buffer, offset, object.size));
        }
    }

    fn push_bytes(&mut self, offset: u32, bytes: &[u8]) {
        if !self.recording() {
            return;
        }
        if self.declared().is_none() {
            self.fail(Rule::Encoding, "a push outside a pass");
            return;
        }
        self.push_range(offset, bytes.len());
    }

    fn push_address(&mut self, offset: u32, buffer: BufferHandle, byte_offset: u64) {
        if !self.recording() {
            return;
        }
        let Some(declared) = self.declared().map(|(_, buffers)| buffers.contains(&buffer)) else {
            self.fail(Rule::Encoding, "a push outside a pass");
            return;
        };
        let Some(object) = self.buffer(buffer) else { return };
        if !declared {
            self.fail(Rule::Undeclared, "an address of a buffer the pass did not declare");
            return;
        }
        if byte_offset >= object.size {
            self.fail(Rule::Bounds, "an address past the end of its buffer");
            return;
        }
        self.push_range(offset, 8);
    }

    fn push_slot(&mut self, offset: u32, slot: HeapSlot) {
        if !self.recording() {
            return;
        }
        let Some(declared) =
            self.declared().map(|(slots, _)| slots.iter().any(|(declared, _)| *declared == slot))
        else {
            self.fail(Rule::Encoding, "a push outside a pass");
            return;
        };
        if !declared {
            self.fail(
                Rule::Undeclared,
                format!("slot {} pushed and not declared by the pass", slot.0),
            );
            return;
        }
        self.push_range(offset, 4);
    }

    fn local_read(&mut self) {
        if !self.recording() {
            return;
        }
        let Scope::Render(pass) = &mut self.scope else {
            self.fail(Rule::Encoding, "a local read outside a render pass");
            return;
        };
        pass.written = 0;
        pass.local_read = true;
    }

    fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        if !self.recording() || vertices.is_empty() || instances.is_empty() {
            return;
        }
        self.draw_command(false, Vec::new());
    }

    fn draw_indexed(&mut self, indices: Range<u32>, _base_vertex: i32, instances: Range<u32>) {
        if !self.recording() || indices.is_empty() || instances.is_empty() {
            return;
        }
        if let Scope::Render(pass) = &self.scope
            && let Some((_, offset, size)) = pass.index
            && offset + u64::from(indices.end) * 4 > size
        {
            self.fail(Rule::Bounds, "an indexed draw that reads past its index buffer");
            return;
        }
        self.draw_command(true, Vec::new());
    }

    fn draw_indexed_indirect(
        &mut self,
        arguments: BufferHandle,
        offset: u64,
        count: u32,
        stride: u32,
    ) {
        if !self.recording() || count == 0 {
            return;
        }
        if count > 1 && !self.shared.caps.multi_draw_indirect {
            self.fail(Rule::Capability, "a count above one with no multi_draw_indirect");
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a draw outside a render pass");
            return;
        }
        let Some(object) = self.buffer_for(arguments, BufferUsage::INDIRECT) else { return };
        if !self.records_inside(offset, count, stride, count > 1, object.size) {
            return;
        }
        self.draw_command(true, vec![arguments]);
    }

    fn draw_indexed_indirect_count(
        &mut self,
        arguments: BufferHandle,
        offset: u64,
        count: BufferHandle,
        count_offset: u64,
        max_count: u32,
        stride: u32,
    ) {
        if !self.recording() {
            return;
        }
        if !self.shared.caps.indirect_count {
            self.fail(Rule::Capability, "an indirect count draw with no indirect_count");
            return;
        }
        if max_count == 0 {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a draw outside a render pass");
            return;
        }
        let Some(object) = self.buffer_for(arguments, BufferUsage::INDIRECT) else { return };
        let Some(counter) = self.buffer_for(count, BufferUsage::INDIRECT) else { return };
        if !count_offset.is_multiple_of(4)
            || !self.bounded(range(count_offset, 4, counter.size, "a draw's count"))
        {
            self.fail(Rule::Bounds, "a count offset unaligned or outside its buffer");
            return;
        }
        if !self.records_inside(offset, max_count, stride, true, object.size) {
            return;
        }
        self.draw_command(true, vec![arguments, count]);
    }

    fn dispatch(&mut self, groups: [u32; 3]) {
        if !self.recording() || groups.contains(&0) {
            return;
        }
        let limit = self.shared.caps.compute_groups;
        if (0..3).any(|at| groups[at] > limit[at]) {
            self.fail(Rule::Bounds, format!("a dispatch of {groups:?} groups, above {limit:?}"));
            return;
        }
        self.dispatch_command(None);
    }

    fn dispatch_indirect(&mut self, arguments: BufferHandle, offset: u64) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Compute(_)) {
            self.fail(Rule::Encoding, "a dispatch outside a compute pass");
            return;
        }
        let Some(object) = self.buffer_for(arguments, BufferUsage::INDIRECT) else { return };
        if !offset.is_multiple_of(4)
            || !self.bounded(range(offset, 12, object.size, "a dispatch's counts"))
        {
            self.fail(Rule::Bounds, "a dispatch's counts unaligned or outside their buffer");
            return;
        }
        self.dispatch_command(Some(arguments));
    }
}

impl NullEncoder {
    /// Checks an indirect draw's offset, stride and the records it spans.
    fn records_inside(
        &mut self,
        offset: u64,
        count: u32,
        stride: u32,
        strided: bool,
        size: u64,
    ) -> bool {
        let record = u64::from(IndexedIndirectArgs::BYTES);
        if !offset.is_multiple_of(4)
            || (strided && (!stride.is_multiple_of(4) || stride < IndexedIndirectArgs::BYTES))
        {
            self.fail(Rule::Bounds, "indirect arguments at an unaligned offset or stride");
            return false;
        }
        let span = u64::from(count - 1) * u64::from(stride) + record;
        self.bounded(range(offset, span, size, "indirect records"))
    }

    /// Checks the regions of a copy between a buffer and an image, and returns the
    /// subresources and buffer ranges they name.
    fn image_regions(
        &mut self,
        buffer: &BufferObj,
        image: &ImageObj,
        regions: &[BufferImageCopy],
    ) -> Option<Regions> {
        let mut subresources = Vec::with_capacity(regions.len());
        let mut ranges = Vec::with_capacity(regions.len());
        for region in regions {
            let rect = region.rect;
            if region.mip >= image.mips || region.layer >= image.layers {
                self.fail(Rule::Bounds, "a copy of a mip or layer the image does not have");
                return None;
            }
            let extent = image.mip_extent(region.mip);
            let inside = rect.extent.width > 0
                && rect.extent.height > 0
                && rect.x.checked_add(rect.extent.width).is_some_and(|end| end <= extent.width)
                && rect.y.checked_add(rect.extent.height).is_some_and(|end| end <= extent.height);
            if !inside {
                self.fail(Rule::Bounds, "a copy's rectangle outside its mip");
                return None;
            }
            let len = u64::from(rect.extent.width)
                * u64::from(rect.extent.height)
                * u64::from(image.format.texel_bytes());
            // Vulkan's VUIDs 07975 and 07978: a multiple of the texel, which is 4 bytes of a
            // depth format too.
            if !region.buffer_offset.is_multiple_of(u64::from(image.format.texel_bytes())) {
                self.fail(Rule::Bounds, "a copy's buffer offset not a multiple of its texel");
                return None;
            }
            let Ok(end) = range(region.buffer_offset, len, buffer.size, "a copy's buffer side")
            else {
                self.fail(Rule::Bounds, "a copy's rows outside its buffer");
                return None;
            };
            subresources.push((region.mip, region.layer));
            ranges.push((region.buffer_offset, end));
        }
        Some((subresources, ranges))
    }
}

/// The usage an image needs for a transition to `state`: Vulkan's layout of each is valid
/// only for an image with it.
const fn state_usage(state: ImageState) -> ImageUsage {
    match state {
        ImageState::Undefined | ImageState::Present => ImageUsage::EMPTY,
        ImageState::ColourAttachment => ImageUsage::COLOUR,
        ImageState::LocalRead => ImageUsage::COLOUR.union(ImageUsage::INPUT),
        ImageState::DepthAttachment => ImageUsage::DEPTH,
        ImageState::SampledRead => ImageUsage::SAMPLED,
        ImageState::CopySource => ImageUsage::COPY_SOURCE,
        ImageState::CopyDestination => ImageUsage::COPY_DESTINATION,
    }
}

/// Whether two ranges of one image share a subresource. Both are inside the image, so
/// neither end overflows.
const fn share(one: ImageRange, other: ImageRange) -> bool {
    one.mip < other.mip + other.mips
        && other.mip < one.mip + one.mips
        && one.layer < other.layer + other.layers
        && other.layer < one.layer + one.layers
}

/// Whether two of `ranges`, each a start and an end past it, share a byte.
fn overlap(ranges: &[(u64, u64)]) -> bool {
    let mut sorted = ranges.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).any(|pair| pair[1].0 < pair[0].1)
}

/// Whether two rectangles, each inside its mip, share a texel.
fn meet(one: Rect, other: Rect) -> bool {
    one.x < other.x + other.extent.width
        && other.x < one.x + one.extent.width
        && one.y < other.y + other.extent.height
        && other.y < one.y + one.extent.height
}

/// A heap slot a pass declares, and the image it resolved to as the pass began.
pub(crate) type SlotDecl = (HeapSlot, Option<ImageHandle>);

/// What a pass's reach resolves to: its slots, its buffers' uses in order, and the buffers
/// it declares.
type Reached = (Vec<SlotDecl>, Vec<BufferReach>, Vec<BufferHandle>);

/// A copy's image subresources, as (mip, layer), and its buffer ranges.
type Regions = (Vec<(u32, u32)>, Vec<(u64, u64)>);

/// A breach found by a check of what a pass holds, or of a recorded command.
type Breach = (Rule, String);

/// Refuses a draw or dispatch in a pass that declares an empty slot.
fn empty_slot(slots: &[SlotDecl], what: &str) -> Result<(), Breach> {
    match slots.iter().find(|(_, image)| image.is_none()) {
        Some((slot, _)) => Err((
            Rule::HeapLayout,
            format!("{what} in a pass that declares slot {}, empty", slot.0),
        )),
        None => Ok(()),
    }
}

/// The checks a draw makes of its render pass, and the attachments it writes noted;
/// the index buffer it reads.
fn draw_checks(
    pass: &mut RenderScope,
    indexed: bool,
    survives: bool,
) -> Result<Option<BufferHandle>, Breach> {
    let Some(pipeline) = pass.pipeline.clone() else {
        return Err((Rule::Encoding, "a draw with no graphics pipeline bound".to_owned()));
    };
    let index = match (indexed, pass.index) {
        (false, _) => None,
        (true, Some((buffer, ..))) => Some(buffer),
        (true, None) => {
            return Err((Rule::Encoding, "an indexed draw with no index buffer bound".to_owned()));
        }
    };
    empty_slot(&pass.slots, "a draw")?;
    let unmarked = pipeline.inputs & !pass.marked;
    if unmarked != 0 {
        return Err((
            Rule::LocalMap,
            format!(
                "a draw that reads input {}, with no attachment of its index marked for the local read",
                unmarked.trailing_zeros()
            ),
        ));
    }
    let unread = pipeline.inputs & pass.written;
    if unread != 0 {
        return Err((
            Rule::Local,
            format!(
                "a draw that reads input {}, written with no local read since",
                unread.trailing_zeros()
            ),
        ));
    }
    if pass.local_read && pass.memoryless && !survives {
        return Err((
            Rule::Split,
            "a memoryless attachment used after a local read, where its contents are lost"
                .to_owned(),
        ));
    }
    pass.written |= pipeline.writes;
    Ok(index)
}
