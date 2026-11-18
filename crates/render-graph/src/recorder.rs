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
    BufferCopy, BufferImageCopy, ComputePipelineHandle, Encoder, GraphicsPipelineHandle, HeapSlot,
    Rect, Viewport,
};

use crate::id::{BufferId, ImageId};

/// The commands of a render pass, lent to its recorder between the pass's beginning and
/// its end.
///
/// The pass begins as `Encoder::begin_render_pass` leaves it: no pipeline, no index
/// buffer, a push-constant block of zeros, and the viewport and the scissor at the
/// attachments' extent.
pub struct RenderRecorder<'r> {
    /// The recording job's encoder, with the pass open.
    encoder: &'r mut dyn Encoder,
}

impl RenderRecorder<'_> {
    /// Sets the viewport of the draws that follow (`Encoder::set_viewport`).
    pub fn set_viewport(&mut self, viewport: Viewport) {
        todo!()
    }

    /// Sets the scissor of the draws that follow (`Encoder::set_scissor`).
    pub fn set_scissor(&mut self, scissor: Rect) {
        todo!()
    }

    /// Binds the pipeline of the draws that follow (`Encoder::bind_graphics_pipeline`).
    pub fn bind_pipeline(&mut self, pipeline: GraphicsPipelineHandle) {
        todo!()
    }

    /// Binds the index buffer of the indexed draws that follow
    /// (`Encoder::bind_index_buffer`). The pass declared `buffer` with
    /// [`BufferAccess::Index`](crate::BufferAccess::Index).
    pub fn bind_index_buffer(&mut self, buffer: BufferId, offset: u64) {
        todo!()
    }

    /// Writes `bytes` into the pass's push-constant block (`Encoder::push_bytes`).
    pub fn push_bytes(&mut self, offset: u32, bytes: &[u8]) {
        todo!()
    }

    /// Writes the device address of byte `byte_offset` of `buffer` into the push-constant
    /// block (`Encoder::push_address`). The pass declared `buffer` with
    /// [`BufferAccess::Read`](crate::BufferAccess::Read) or
    /// [`BufferAccess::Write`](crate::BufferAccess::Write).
    pub fn push_address(&mut self, offset: u32, buffer: BufferId, byte_offset: u64) {
        todo!()
    }

    /// Writes `slot`'s number into the push-constant block (`Encoder::push_slot`). The
    /// pass declared `slot`, which the device checks.
    pub fn push_slot(&mut self, offset: u32, slot: HeapSlot) {
        todo!()
    }

    /// The local read (`Encoder::local_read`): the draws that follow may read what the
    /// pass has written so far to each colour attachment declared with `local_read`.
    pub fn local_read(&mut self) {
        todo!()
    }

    /// Draws `vertices` for each of `instances`, with no index buffer (`Encoder::draw`).
    pub fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        todo!()
    }

    /// Draws `indices` of the bound index buffer (`Encoder::draw_indexed`).
    pub fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        todo!()
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
        todo!()
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
        todo!()
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
}

impl ComputeRecorder<'_> {
    /// Binds the pipeline of the dispatches that follow
    /// (`Encoder::bind_compute_pipeline`).
    pub fn bind_pipeline(&mut self, pipeline: ComputePipelineHandle) {
        todo!()
    }

    /// Writes `bytes` into the pass's push-constant block (`Encoder::push_bytes`).
    pub fn push_bytes(&mut self, offset: u32, bytes: &[u8]) {
        todo!()
    }

    /// Writes the device address of byte `byte_offset` of `buffer` into the push-constant
    /// block (`Encoder::push_address`). The pass declared `buffer` with
    /// [`BufferAccess::Read`](crate::BufferAccess::Read) or
    /// [`BufferAccess::Write`](crate::BufferAccess::Write).
    pub fn push_address(&mut self, offset: u32, buffer: BufferId, byte_offset: u64) {
        todo!()
    }

    /// Writes `slot`'s number into the push-constant block (`Encoder::push_slot`). The
    /// pass declared `slot`, which the device checks.
    pub fn push_slot(&mut self, offset: u32, slot: HeapSlot) {
        todo!()
    }

    /// Runs the bound pipeline over `groups` workgroups (`Encoder::dispatch`).
    pub fn dispatch(&mut self, groups: [u32; 3]) {
        todo!()
    }

    /// Runs the bound pipeline over the workgroups read from `arguments`
    /// (`Encoder::dispatch_indirect`). The pass declared `arguments` with
    /// [`BufferAccess::Indirect`](crate::BufferAccess::Indirect).
    pub fn dispatch_indirect(&mut self, arguments: BufferId, offset: u64) {
        todo!()
    }
}

/// The commands of a copy pass, lent to its recorder with no pass open on the encoder.
pub struct CopyRecorder<'r> {
    /// The recording job's encoder, outside any pass.
    encoder: &'r mut dyn Encoder,
}

impl CopyRecorder<'_> {
    /// Copies ranges of `source` to `destination` (`Encoder::copy_buffer`). The pass
    /// declared `source` among its buffer sources and `destination` among its buffer
    /// destinations.
    pub fn copy_buffer(&mut self, source: BufferId, destination: BufferId, regions: &[BufferCopy]) {
        todo!()
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
        todo!()
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
        todo!()
    }
}
