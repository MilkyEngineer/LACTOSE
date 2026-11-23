//! What a pass's recorder is lent: the commands of its kind of pass, over the ids of its
//! declaration.
//!
//! A recorder reaches an encoder through one of these and through nothing else, so a
//! transition, an alias call, a pass's beginning and end and an encoder's reset are the
//! graph's alone. Each call forwards to the `lactose_rhi_api::Encoder` call of the same
//! name with the handle the frame imported under the id, and keeps that call's contract:
//! what the device refuses at the submit, `RenderGraph::submit` returns.
//!
//! An id resolves only as its pass declared it. A call that names a buffer or an image its
//! pass did not declare for that use is a breach: `RenderGraph::record` returns
//! `ErrorKind::InvalidData`, naming the pass and the call, and no frame is recorded
//! (`README.md`, "Recording"). Heap slots are not resolved and not checked here: the
//! device refuses one the pass did not declare (`undeclared`, every backend).

use std::ops::Range;

use lactose_rhi_api::{
    BufferCopy, BufferHandle, BufferImageCopy, ComputePipelineHandle, Encoder,
    GraphicsPipelineHandle, HeapSlot, ImageHandle, Rect, Viewport,
};

use crate::decl::DeclaredKind;
use crate::id::{BufferId, ImageId};
use crate::pass::BufferAccess;
use crate::plan::Resource;

/// What every recorder type resolves its ids through: the pass's declaration, the frame's
/// handles, and the job's place for its first breach.
pub(crate) struct PassScope<'r> {
    /// The pass's label, for a breach's message.
    pub(crate) label: &'r str,
    /// What the pass declared.
    pub(crate) kind: &'r DeclaredKind,
    /// The frame's images by id: a transient's is the plan's, and `None` is a transient
    /// the plan did not make.
    pub(crate) images: &'r [Option<ImageHandle>],
    /// The frame's buffers by id.
    pub(crate) buffers: &'r [BufferHandle],
    /// The job's first breach. Once it is written, every call records nothing.
    pub(crate) breach: &'r mut Option<String>,
}

impl PassScope<'_> {
    /// Whether a breach was recorded, so the call records nothing.
    fn breached(&self) -> bool {
        self.breach.is_some()
    }

    fn refuse(&mut self, message: String) {
        *self.breach = Some(message);
    }

    /// The handle of a buffer the pass declared with one of `accesses`.
    fn buffer(
        &mut self,
        call: &str,
        id: BufferId,
        accesses: &[BufferAccess],
    ) -> Option<BufferHandle> {
        if self.breached() {
            return None;
        }
        let declared = self
            .kind
            .buffers()
            .iter()
            .any(|use_| use_.buffer == id && accesses.contains(&use_.access));
        if declared {
            return Some(self.buffers[id.index() as usize]);
        }
        let label = self.label;
        let index = id.index();
        let accesses: Vec<String> = accesses.iter().map(|access| format!("{access:?}")).collect();
        self.refuse(format!(
            "pass `{label}`: `{call}` names buffer {index}, which the pass does not declare as {}",
            accesses.join(" or ")
        ));
        None
    }

    /// The handle of a buffer the copy pass declared in `list`.
    fn copy_buffer(&mut self, call: &str, id: BufferId, list: CopySide) -> Option<BufferHandle> {
        if self.breached() {
            return None;
        }
        let DeclaredKind::Copy { buffer_sources, buffer_destinations, .. } = self.kind else {
            unreachable!("a copy recorder is lent for a copy pass alone");
        };
        let ids = match list {
            CopySide::Source => buffer_sources,
            CopySide::Destination => buffer_destinations,
        };
        if ids.contains(&id) {
            return Some(self.buffers[id.index() as usize]);
        }
        let (label, index) = (self.label, id.index());
        self.refuse(format!(
            "pass `{label}`: `{call}` names buffer {index}, which the pass does not declare among \
             its buffer {}",
            list.plural()
        ));
        None
    }

    /// The handle of an image the copy pass declared in `list`.
    fn copy_image(&mut self, call: &str, id: ImageId, list: CopySide) -> Option<ImageHandle> {
        if self.breached() {
            return None;
        }
        let DeclaredKind::Copy { image_sources, image_destinations, .. } = self.kind else {
            unreachable!("a copy recorder is lent for a copy pass alone");
        };
        let ids = match list {
            CopySide::Source => image_sources,
            CopySide::Destination => image_destinations,
        };
        if ids.contains(&id) {
            // A kept copy pass's images are all made: a transient it names is one it uses.
            return Some(self.images[id.index() as usize].expect("a kept pass's images are made"));
        }
        let (label, index) = (self.label, id.index());
        self.refuse(format!(
            "pass `{label}`: `{call}` names image {index}, which the pass does not declare among \
             its image {}",
            list.plural()
        ));
        None
    }
}

/// Which of a copy pass's lists an id is looked up in.
#[derive(Clone, Copy)]
enum CopySide {
    Source,
    Destination,
}

impl CopySide {
    const fn plural(self) -> &'static str {
        match self {
            Self::Source => "sources",
            Self::Destination => "destinations",
        }
    }
}

/// The commands of a render pass, lent to its recorder between the pass's beginning and
/// its end.
///
/// The pass begins as `Encoder::begin_render_pass` leaves it: no pipeline, no index
/// buffer, a push-constant block of zeros, and the viewport and the scissor at the
/// attachments' extent.
pub struct RenderRecorder<'r> {
    /// The recording job's encoder, with the pass open.
    encoder: &'r mut dyn Encoder,
    /// The pass's ids and the job's breach.
    scope: PassScope<'r>,
}

impl<'r> RenderRecorder<'r> {
    /// The recorder of a render pass begun on `encoder`.
    pub(crate) fn new(encoder: &'r mut dyn Encoder, scope: PassScope<'r>) -> Self {
        Self { encoder, scope }
    }

    /// Sets the viewport of the draws that follow (`Encoder::set_viewport`).
    pub fn set_viewport(&mut self, viewport: Viewport) {
        if !self.scope.breached() {
            self.encoder.set_viewport(viewport);
        }
    }

    /// Sets the scissor of the draws that follow (`Encoder::set_scissor`).
    pub fn set_scissor(&mut self, scissor: Rect) {
        if !self.scope.breached() {
            self.encoder.set_scissor(scissor);
        }
    }

    /// Binds the pipeline of the draws that follow (`Encoder::bind_graphics_pipeline`).
    pub fn bind_pipeline(&mut self, pipeline: GraphicsPipelineHandle) {
        if !self.scope.breached() {
            self.encoder.bind_graphics_pipeline(pipeline);
        }
    }

    /// Binds the index buffer of the indexed draws that follow
    /// (`Encoder::bind_index_buffer`). The pass declared `buffer` with
    /// [`BufferAccess::Index`](crate::BufferAccess::Index).
    pub fn bind_index_buffer(&mut self, buffer: BufferId, offset: u64) {
        if let Some(buffer) = self.scope.buffer("bind_index_buffer", buffer, &[BufferAccess::Index])
        {
            self.encoder.bind_index_buffer(buffer, offset);
        }
    }

    /// Writes `bytes` into the pass's push-constant block (`Encoder::push_bytes`).
    pub fn push_bytes(&mut self, offset: u32, bytes: &[u8]) {
        if !self.scope.breached() {
            self.encoder.push_bytes(offset, bytes);
        }
    }

    /// Writes the device address of byte `byte_offset` of `buffer` into the push-constant
    /// block (`Encoder::push_address`). The pass declared `buffer` with
    /// [`BufferAccess::Read`](crate::BufferAccess::Read) or
    /// [`BufferAccess::Write`](crate::BufferAccess::Write).
    pub fn push_address(&mut self, offset: u32, buffer: BufferId, byte_offset: u64) {
        let reached = &[BufferAccess::Read, BufferAccess::Write];
        if let Some(buffer) = self.scope.buffer("push_address", buffer, reached) {
            self.encoder.push_address(offset, buffer, byte_offset);
        }
    }

    /// Writes `slot`'s number into the push-constant block (`Encoder::push_slot`). The
    /// pass declared `slot`, which the device checks.
    pub fn push_slot(&mut self, offset: u32, slot: HeapSlot) {
        if !self.scope.breached() {
            self.encoder.push_slot(offset, slot);
        }
    }

    /// The local read (`Encoder::local_read`): the draws that follow may read what the
    /// pass has written so far to each colour attachment declared with `local_read`.
    pub fn local_read(&mut self) {
        if !self.scope.breached() {
            self.encoder.local_read();
        }
    }

    /// Draws `vertices` for each of `instances`, with no index buffer (`Encoder::draw`).
    pub fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        if !self.scope.breached() {
            self.encoder.draw(vertices, instances);
        }
    }

    /// Draws `indices` of the bound index buffer (`Encoder::draw_indexed`).
    pub fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        if !self.scope.breached() {
            self.encoder.draw_indexed(indices, base_vertex, instances);
        }
    }

    /// Draws `count` records read from `arguments` (`Encoder::draw_indexed_indirect`).
    /// The pass declared `arguments` with
    /// [`BufferAccess::Indirect`](crate::BufferAccess::Indirect).
    pub fn draw_indexed_indirect(
        &mut self,
        arguments: BufferId,
        offset: u64,
        count: u32,
        stride: u32,
    ) {
        let call = "draw_indexed_indirect";
        if let Some(arguments) = self.scope.buffer(call, arguments, &[BufferAccess::Indirect]) {
            self.encoder.draw_indexed_indirect(arguments, offset, count, stride);
        }
    }

    /// As [`draw_indexed_indirect`](Self::draw_indexed_indirect), with the count read on
    /// the device from `count` (`Encoder::draw_indexed_indirect_count`). The pass declared
    /// both buffers with [`BufferAccess::Indirect`](crate::BufferAccess::Indirect); they
    /// may be one buffer.
    pub fn draw_indexed_indirect_count(
        &mut self,
        arguments: BufferId,
        offset: u64,
        count: BufferId,
        count_offset: u64,
        max_count: u32,
        stride: u32,
    ) {
        let call = "draw_indexed_indirect_count";
        let indirect = &[BufferAccess::Indirect];
        let Some(arguments) = self.scope.buffer(call, arguments, indirect) else { return };
        let Some(count) = self.scope.buffer(call, count, indirect) else { return };
        self.encoder.draw_indexed_indirect_count(
            arguments,
            offset,
            count,
            count_offset,
            max_count,
            stride,
        );
    }
}

/// The commands of a compute pass, lent to its recorder between the pass's beginning and
/// its end.
///
/// The pass begins as `Encoder::begin_compute_pass` leaves it: no pipeline and a
/// push-constant block of zeros.
pub struct ComputeRecorder<'r> {
    /// The recording job's encoder, with the pass open.
    encoder: &'r mut dyn Encoder,
    /// The pass's ids and the job's breach.
    scope: PassScope<'r>,
}

impl<'r> ComputeRecorder<'r> {
    /// The recorder of a compute pass begun on `encoder`.
    pub(crate) fn new(encoder: &'r mut dyn Encoder, scope: PassScope<'r>) -> Self {
        Self { encoder, scope }
    }

    /// Binds the pipeline of the dispatches that follow
    /// (`Encoder::bind_compute_pipeline`).
    pub fn bind_pipeline(&mut self, pipeline: ComputePipelineHandle) {
        if !self.scope.breached() {
            self.encoder.bind_compute_pipeline(pipeline);
        }
    }

    /// Writes `bytes` into the pass's push-constant block (`Encoder::push_bytes`).
    pub fn push_bytes(&mut self, offset: u32, bytes: &[u8]) {
        if !self.scope.breached() {
            self.encoder.push_bytes(offset, bytes);
        }
    }

    /// Writes the device address of byte `byte_offset` of `buffer` into the push-constant
    /// block (`Encoder::push_address`). The pass declared `buffer` with
    /// [`BufferAccess::Read`](crate::BufferAccess::Read) or
    /// [`BufferAccess::Write`](crate::BufferAccess::Write).
    pub fn push_address(&mut self, offset: u32, buffer: BufferId, byte_offset: u64) {
        let reached = &[BufferAccess::Read, BufferAccess::Write];
        if let Some(buffer) = self.scope.buffer("push_address", buffer, reached) {
            self.encoder.push_address(offset, buffer, byte_offset);
        }
    }

    /// Writes `slot`'s number into the push-constant block (`Encoder::push_slot`). The
    /// pass declared `slot`, which the device checks.
    pub fn push_slot(&mut self, offset: u32, slot: HeapSlot) {
        if !self.scope.breached() {
            self.encoder.push_slot(offset, slot);
        }
    }

    /// Runs the bound pipeline over `groups` workgroups (`Encoder::dispatch`).
    pub fn dispatch(&mut self, groups: [u32; 3]) {
        if !self.scope.breached() {
            self.encoder.dispatch(groups);
        }
    }

    /// Runs the bound pipeline over the workgroups read from `arguments`
    /// (`Encoder::dispatch_indirect`). The pass declared `arguments` with
    /// [`BufferAccess::Indirect`](crate::BufferAccess::Indirect).
    pub fn dispatch_indirect(&mut self, arguments: BufferId, offset: u64) {
        let call = "dispatch_indirect";
        if let Some(arguments) = self.scope.buffer(call, arguments, &[BufferAccess::Indirect]) {
            self.encoder.dispatch_indirect(arguments, offset);
        }
    }
}

/// The commands of a copy pass, lent to its recorder with no pass open on the encoder.
///
/// Each destination is written by one call, which takes as many regions as it needs. A
/// second call that writes a destination the pass already wrote is a breach, as a call
/// that names an undeclared id is: two copies into one resource need a transition between
/// them, which a recorder cannot record (`README.md`, "Recording").
pub struct CopyRecorder<'r> {
    /// The recording job's encoder, outside any pass.
    encoder: &'r mut dyn Encoder,
    /// The pass's ids and the job's breach.
    scope: PassScope<'r>,
    /// The destinations the pass's calls wrote so far.
    written: Vec<Resource>,
}

impl<'r> CopyRecorder<'r> {
    /// The recorder of a copy pass on `encoder`.
    pub(crate) fn new(encoder: &'r mut dyn Encoder, scope: PassScope<'r>) -> Self {
        Self { encoder, scope, written: Vec::new() }
    }

    /// Notes that `call` writes `destination`, refusing a second write of it.
    fn write(&mut self, call: &str, destination: Resource) -> bool {
        if self.scope.breached() {
            return false;
        }
        if !self.written.contains(&destination) {
            self.written.push(destination);
            return true;
        }
        let (label, name) = match destination {
            Resource::Image(image) => (self.scope.label, format!("image {image}")),
            Resource::Buffer(buffer) => (self.scope.label, format!("buffer {buffer}")),
        };
        self.scope.refuse(format!(
            "pass `{label}`: `{call}` writes {name}, which an earlier call of the pass wrote; one \
             call writes each destination"
        ));
        false
    }

    /// Copies ranges of `source` to `destination` (`Encoder::copy_buffer`). The pass
    /// declared `source` among its buffer sources and `destination` among its buffer
    /// destinations.
    pub fn copy_buffer(&mut self, source: BufferId, destination: BufferId, regions: &[BufferCopy]) {
        let call = "copy_buffer";
        let Some(from) = self.scope.copy_buffer(call, source, CopySide::Source) else { return };
        let Some(to) = self.scope.copy_buffer(call, destination, CopySide::Destination) else {
            return;
        };
        if self.write(call, Resource::Buffer(destination.index())) {
            self.encoder.copy_buffer(from, to, regions);
        }
    }

    /// Copies packed rows of `source` into rectangles of `destination`
    /// (`Encoder::copy_buffer_to_image`). The pass declared `source` among its buffer
    /// sources and `destination` among its image destinations.
    pub fn copy_buffer_to_image(
        &mut self,
        source: BufferId,
        destination: ImageId,
        regions: &[BufferImageCopy],
    ) {
        let call = "copy_buffer_to_image";
        let Some(from) = self.scope.copy_buffer(call, source, CopySide::Source) else { return };
        let Some(to) = self.scope.copy_image(call, destination, CopySide::Destination) else {
            return;
        };
        if self.write(call, Resource::Image(destination.index())) {
            self.encoder.copy_buffer_to_image(from, to, regions);
        }
    }

    /// Copies rectangles of `source` into packed rows of `destination`
    /// (`Encoder::copy_image_to_buffer`). The pass declared `source` among its image
    /// sources and `destination` among its buffer destinations.
    pub fn copy_image_to_buffer(
        &mut self,
        source: ImageId,
        destination: BufferId,
        regions: &[BufferImageCopy],
    ) {
        let call = "copy_image_to_buffer";
        let Some(from) = self.scope.copy_image(call, source, CopySide::Source) else { return };
        let Some(to) = self.scope.copy_buffer(call, destination, CopySide::Destination) else {
            return;
        };
        if self.write(call, Resource::Buffer(destination.index())) {
            self.encoder.copy_image_to_buffer(from, to, regions);
        }
    }
}
