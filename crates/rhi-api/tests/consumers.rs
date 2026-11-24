//! Compile-only: each consumer's use of the surface, written as that consumer would write
//! it, so that a signature a consumer has to work around fails this file. Nothing here
//! runs: there is no backend yet. `README.md`, "What each consumer needs", is the list
//! these functions follow.

#![allow(dead_code, unused_variables, missing_docs, clippy::too_many_arguments)]

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use lactose_core::{ErrorKind, Result};
use lactose_rhi_api::{
    Acquired, BufferDesc, BufferHandle, BufferReach, BufferState, BufferTransition, BufferUsage,
    Capabilities, ColourAttachment, ComputePass, CookedShader, Device, DeviceInfo, DeviceRequest,
    DisplayTime, Encoder, Extent, GraphicsPipelineDesc, GraphicsPipelineHandle, HeapSlot,
    ImageDesc, ImageHandle, ImageMemory, ImageRange, ImageState, ImageTransition, ImageUsage,
    IndexedIndirectArgs, Load, MemoryClass, Message, MessageSink, PassReach, PresentId, Presented,
    QueueRole, RenderPass, Rule, Sharing, StateOn, Store, Submit, SurfaceSource, Swapchain, Target,
    TimelinePoint,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

/// `lactose-platform` and `lactose-app-client`: a window of any windowing crate becomes
/// the request's surface with no `unsafe` and no type of `lactose-platform` in a backend.
fn request_for<W>(window: Arc<W>, messages: Arc<dyn MessageSink>) -> DeviceRequest
where
    W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
{
    let surface: Arc<dyn SurfaceSource> = window;
    DeviceRequest {
        adapter: None,
        surface: Some(surface),
        heap_slots: lactose_rhi_api::HEAP_SLOTS_DEFAULT,
        samplers: lactose_rhi_api::SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages,
    }
}

/// `lactose-app` (decision 5.11): it holds the device as a `dyn` resource, names no
/// backend, and chooses policy from the error's kind.
struct RenderSide {
    device: Arc<dyn Device>,
}

impl RenderSide {
    fn frame_failed(&self, error: &lactose_core::Error) -> bool {
        // A loss is a value: the frame stops, and recovery is `app`'s policy.
        error.kind() == ErrorKind::DeviceLost || self.device.is_lost()
    }
}

/// `lactose-render-graph`: one recording job on a frame worker. The encoder moves to the
/// worker, records a pass under derived transitions, and comes back for the submit. The
/// pass's first pipeline writes colour attachment 0; after the local read a second one
/// reads it as input 0 and writes attachment 1, since no pipeline reads an input and
/// writes the attachment of the same index.
fn record_pass(
    mut encoder: Box<dyn Encoder>,
    colour: ImageHandle,
    transient: ImageHandle,
    g_buffer: GraphicsPipelineHandle,
    lighting: GraphicsPipelineHandle,
    reach: PassReach<'_>,
) -> Box<dyn Encoder> {
    let queue = encoder.queue();
    // A transient that aliases another begins here, undefined.
    encoder.alias(transient);
    encoder.transition_images(&[
        ImageTransition {
            image: colour,
            range: ImageRange::FIRST,
            from: StateOn { state: ImageState::Undefined, queue },
            to: StateOn { state: ImageState::LocalRead, queue },
        },
        ImageTransition {
            image: transient,
            range: ImageRange::FIRST,
            from: StateOn { state: ImageState::Undefined, queue },
            to: StateOn { state: ImageState::ColourAttachment, queue },
        },
    ]);
    encoder.begin_render_pass(&RenderPass {
        label: "g-buffer and lighting",
        colour: &[
            ColourAttachment {
                target: Target { image: colour, mip: 0, layer: 0 },
                load: Load::Clear([0.0; 4]),
                store: Store::DontCare,
                local_read: true,
            },
            ColourAttachment {
                target: Target { image: transient, mip: 0, layer: 0 },
                load: Load::DontCare,
                store: Store::Store,
                local_read: false,
            },
        ],
        depth: None,
        reach,
    });
    encoder.bind_graphics_pipeline(g_buffer);
    encoder.draw(0..3, 0..1);
    // Input 0 is colour attachment 0, and reads what the draw above wrote.
    encoder.local_read();
    encoder.bind_graphics_pipeline(lighting);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    encoder
}

/// `lactose-render-graph`: submission in graph order, the compute queue's work crossing
/// to the graphics queue by a timeline value and a transfer recorded on both sides.
fn submit_in_graph_order(
    device: &dyn Device,
    compute: &mut [Box<dyn Encoder>],
    graphics: &mut [Box<dyn Encoder>],
    instances: BufferHandle,
    crossed: TimelinePoint,
    frame_done: TimelinePoint,
) -> Result<()> {
    let transfer = BufferTransition {
        buffer: instances,
        from: StateOn { state: BufferState::ShaderWrite, queue: QueueRole::Compute },
        to: StateOn { state: BufferState::ShaderRead, queue: QueueRole::Graphics },
    };
    let layout = device.capabilities().queues;
    if layout.same_queue(QueueRole::Compute, QueueRole::Graphics) {
        // One queue: the change of state is recorded once, by the reader.
        graphics[0].transition_buffers(&[transfer]);
    } else {
        compute[0].transition_buffers(&[transfer]);
        graphics[0].transition_buffers(&[transfer]);
    }
    device.submit(
        QueueRole::Compute,
        Submit { encoders: compute, waits: &[], signals: &[crossed] },
    )?;
    device.submit(
        QueueRole::Graphics,
        Submit { encoders: graphics, waits: &[crossed], signals: &[frame_done] },
    )
}

/// `lactose-render-graph`: an encoder is reset only after its frame's timeline value.
fn recycle(
    device: &dyn Device,
    encoder: &mut dyn Encoder,
    frame_done: TimelinePoint,
) -> Result<()> {
    if device.wait_timeline(frame_done, Duration::ZERO)? {
        encoder.reset()?;
    }
    Ok(())
}

/// `lactose-renderer`: the staging ring's writer. One call lends a frame's region; the
/// rows are written in place, in pieces that do not overlap, and nothing is read.
fn write_ring(
    device: &dyn Device,
    ring: BufferHandle,
    offset: u64,
    rows: &[[u8; 48]],
) -> Result<()> {
    let len = (rows.len() * 48) as u64;
    device.write_buffer(ring, offset, len, &mut |bytes| {
        let (out, _) = bytes.as_chunks_mut::<48>();
        out.copy_from_slice(rows);
    })?;
    device.flush_buffer(ring, offset, len)
}

/// `lactose-renderer`: the scatter, the latched camera and the instanced draw. Addresses
/// and slots reach the shaders as typed push constants, each one declared by its pass.
fn scatter_and_draw(
    encoder: &mut dyn Encoder,
    ring: BufferHandle,
    instances: BufferHandle,
    camera: BufferHandle,
    arguments: BufferHandle,
    indices: BufferHandle,
    albedo: HeapSlot,
    target: ImageHandle,
) {
    encoder.begin_compute_pass(&ComputePass {
        label: "scatter",
        reach: PassReach {
            slots: &[],
            buffers: &[
                BufferReach { buffer: ring, state: BufferState::ShaderRead },
                BufferReach { buffer: instances, state: BufferState::ShaderWrite },
            ],
        },
    });
    encoder.push_address(0, ring, 0);
    encoder.push_address(8, instances, 0);
    encoder.push_bytes(16, &50_000_u32.to_le_bytes());
    encoder.dispatch([782, 1, 1]);
    encoder.end_pass();

    let queue = encoder.queue();
    encoder.transition_buffers(&[BufferTransition {
        buffer: instances,
        from: StateOn { state: BufferState::ShaderWrite, queue },
        to: StateOn { state: BufferState::ShaderRead, queue },
    }]);
    encoder.begin_render_pass(&RenderPass {
        label: "triangles",
        colour: &[ColourAttachment {
            target: Target { image: target, mip: 0, layer: 0 },
            load: Load::Clear([0.0, 0.0, 0.0, 1.0]),
            store: Store::Store,
            local_read: false,
        }],
        depth: None,
        reach: PassReach {
            slots: &[albedo],
            buffers: &[
                BufferReach { buffer: instances, state: BufferState::ShaderRead },
                BufferReach { buffer: camera, state: BufferState::ShaderRead },
            ],
        },
    });
    encoder.push_address(0, instances, 0);
    encoder.push_address(8, camera, 0);
    encoder.push_slot(16, albedo);
    encoder.bind_index_buffer(indices, 0);
    encoder.draw_indexed_indirect(arguments, 0, 1, IndexedIndirectArgs::BYTES);
    encoder.end_pass();
}

/// `lactose-renderer`: what it creates. A buffer the host never touches is filled by a
/// copy, and a memoryless request is made only where the device reports it survives.
fn create(device: &dyn Device, extent: Extent) -> Result<(BufferHandle, ImageHandle)> {
    let caps: &Capabilities = device.capabilities();
    let instances = device.create_buffer(&BufferDesc {
        label: "instances",
        size: 100_000 * 48,
        usage: BufferUsage::DEVICE_ADDRESS.union(BufferUsage::COPY_DESTINATION),
        memory: MemoryClass::Device,
        sharing: Sharing::Exclusive,
    })?;
    let memoryless = caps.memoryless_colour && caps.memoryless_survives_local_read;
    let g_buffer = device.create_image(&ImageDesc {
        label: "g-buffer",
        format: lactose_rhi_api::Format::Rgba8Unorm,
        extent,
        mips: 1,
        layers: 1,
        usage: ImageUsage::COLOUR.union(ImageUsage::INPUT).union(ImageUsage::TRANSIENT),
        memory: if memoryless { ImageMemory::Memoryless } else { ImageMemory::Own },
        sharing: Sharing::Exclusive,
    })?;
    let honoured: bool = device.image_memoryless(g_buffer)?;
    Ok((instances, g_buffer))
}

/// `lactose-platform`'s present thread and `swapchain` (decision 5.6): it acquires with
/// no timeout of its own, takes the newest finished target across from the frame's queue,
/// draws it into the swapchain image by the surface's format, presents with the submit,
/// and signals that its read is done so the frame may take the target back. On a stale
/// surface it waits for its last present before the resize, which is refused while one
/// is pending.
fn present_once(
    device: &dyn Device,
    swapchain: &mut dyn Swapchain,
    encoders: &mut [Box<dyn Encoder>],
    blit: GraphicsPipelineHandle,
    target: ImageHandle,
    target_slot: HeapSlot,
    finished: TimelinePoint,
    read_done: TimelinePoint,
    last_read_done: Option<TimelinePoint>,
) -> Result<bool> {
    let image = match swapchain.acquire()? {
        Acquired::Image(image) => image,
        Acquired::None => return Ok(true),
        Acquired::Stale => {
            if let Some(last) = last_read_done
                && !device.wait_timeline(last, Duration::from_secs(1))?
            {
                return Ok(true);
            }
            swapchain.resize(swapchain.extent())?;
            return Ok(true);
        }
    };
    let present = QueueRole::Present;
    let encoder = &mut encoders[0];
    encoder.transition_images(&[
        // The acquire half of the target's crossing; the frame recorded the release.
        ImageTransition {
            image: target,
            range: ImageRange::FIRST,
            from: StateOn { state: ImageState::ColourAttachment, queue: QueueRole::Graphics },
            to: StateOn { state: ImageState::SampledRead, queue: present },
        },
        ImageTransition {
            image,
            range: ImageRange::FIRST,
            from: StateOn { state: ImageState::Undefined, queue: present },
            to: StateOn { state: ImageState::ColourAttachment, queue: present },
        },
    ]);
    encoder.begin_render_pass(&RenderPass {
        label: "present",
        colour: &[ColourAttachment {
            target: Target { image, mip: 0, layer: 0 },
            load: Load::DontCare,
            store: Store::Store,
            local_read: false,
        }],
        depth: None,
        reach: PassReach { slots: &[target_slot], buffers: &[] },
    });
    encoder.bind_graphics_pipeline(blit);
    encoder.push_slot(0, target_slot);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    encoder.transition_images(&[ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: StateOn { state: ImageState::ColourAttachment, queue: present },
        to: StateOn { state: ImageState::Present, queue: present },
    }]);
    let shown =
        swapchain.present(image, Submit { encoders, waits: &[finished], signals: &[read_done] })?;
    Ok(shown == Presented::Shown)
}

/// The binary's present side (D162): it keeps each present's id beside its call, and as
/// each call begins reads the display times on the thread that presents, with no wait. It
/// counts the presents not yet known displayed: a present is taken as displayed once its
/// own id or a later one is reported. `None` where the swapchain reports no display time.
/// The buffer is the caller's, cleared and reused at each call.
fn waiting(
    swapchain: &mut dyn Swapchain,
    times: &mut Vec<DisplayTime>,
    displayed: &mut Option<PresentId>,
) -> Option<u64> {
    times.clear();
    if !swapchain.display_times(times) {
        return None;
    }
    if let Some(last) = times.last() {
        *displayed = Some(last.present);
        let age_at_display: Duration = last.shown.elapsed();
    }
    let presented = swapchain.last_present().map_or(0, |PresentId(id)| id);
    Some(presented - displayed.map_or(0, |PresentId(id)| id))
}

/// `tools/cook` and a backend: the cooker writes the format, and a pipeline is made from
/// what `decode` reads back.
fn cook_then_load(device: &dyn Device, shader: &CookedShader) -> Result<GraphicsPipelineHandle> {
    let bytes: Vec<u8> = shader.encode()?;
    let vertex = CookedShader::decode(&bytes)?;
    device.create_graphics_pipeline(&GraphicsPipelineDesc {
        label: "triangle",
        vertex: lactose_rhi_api::StageDesc { shader: &vertex, constants: &[] },
        fragment: None,
        colour: &[],
        depth: None,
        cull: lactose_rhi_api::Cull::Back,
    })
}

/// A null-backend test and the graph's tests on null: a refusal is a value that names
/// its rule, so a test asserts which rule refused.
fn refused_by(result: Result<()>, rule: Rule) -> bool {
    result.is_err_and(|error| Rule::of(&error) == Some(rule))
}

/// A backend: the traits are implemented with no `unsafe`, as `lactose-rhi-null` must,
/// and a submit recovers the backend's own encoder from the object it is handed.
mod backend {
    use super::*;
    use lactose_rhi_api::{
        BufferCopy, BufferImageCopy, ComputePipelineDesc, ComputePipelineHandle, Format,
        MemoryBlockDesc, MemoryBlockHandle, PipelineStats, Placement, Rect, SamplerDesc,
        SamplerIndex, SwapchainDesc, TimelineHandle, Viewport,
    };

    struct Sketch {
        info: DeviceInfo,
        caps: Capabilities,
    }

    struct SketchEncoder {
        queue: QueueRole,
        commands: Vec<u8>,
    }

    struct SketchSwapchain;

    struct Counting;

    impl MessageSink for Counting {
        fn message(&self, message: &Message<'_>) {}
    }

    /// The backend's own encoders, or `None` when one is another backend's. Through the
    /// object each box holds: `&mut encoders[i]` would coerce the box itself to `Any`.
    fn own(encoders: &mut [Box<dyn Encoder>]) -> Option<Vec<&mut SketchEncoder>> {
        encoders
            .iter_mut()
            .map(|encoder| {
                let object: &mut dyn Any = &mut **encoder;
                object.downcast_mut::<SketchEncoder>()
            })
            .collect()
    }

    impl Device for Sketch {
        fn info(&self) -> &DeviceInfo {
            &self.info
        }
        fn capabilities(&self) -> &Capabilities {
            &self.caps
        }
        fn is_lost(&self) -> bool {
            false
        }
        fn create_buffer(&self, desc: &BufferDesc<'_>) -> Result<BufferHandle> {
            unimplemented!()
        }
        fn destroy_buffer(&self, buffer: BufferHandle) -> Result<()> {
            unimplemented!()
        }
        fn buffer_address(&self, buffer: BufferHandle) -> Result<u64> {
            unimplemented!()
        }
        fn write_buffer(
            &self,
            buffer: BufferHandle,
            offset: u64,
            len: u64,
            fill: &mut dyn FnMut(&mut [u8]),
        ) -> Result<()> {
            // Null's store is a vector behind a lock: the range is lent for the call.
            let mut store = vec![0_u8; usize::try_from(len).unwrap_or(0)];
            fill(&mut store);
            Ok(())
        }
        fn read_buffer(
            &self,
            buffer: BufferHandle,
            offset: u64,
            len: u64,
            read: &mut dyn FnMut(&[u8]),
        ) -> Result<()> {
            unimplemented!()
        }
        fn flush_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()> {
            unimplemented!()
        }
        fn invalidate_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()> {
            unimplemented!()
        }
        fn create_image(&self, desc: &ImageDesc<'_>) -> Result<ImageHandle> {
            unimplemented!()
        }
        fn destroy_image(&self, image: ImageHandle) -> Result<()> {
            unimplemented!()
        }
        fn image_memoryless(&self, image: ImageHandle) -> Result<bool> {
            unimplemented!()
        }
        fn image_placement(&self, desc: &ImageDesc<'_>) -> Result<Placement> {
            unimplemented!()
        }
        fn create_memory_block(&self, desc: &MemoryBlockDesc<'_>) -> Result<MemoryBlockHandle> {
            unimplemented!()
        }
        fn destroy_memory_block(&self, block: MemoryBlockHandle) -> Result<()> {
            unimplemented!()
        }
        fn write_heap_slot(&self, slot: HeapSlot, image: ImageHandle) -> Result<()> {
            unimplemented!()
        }
        fn clear_heap_slot(&self, slot: HeapSlot) -> Result<()> {
            unimplemented!()
        }
        fn create_sampler(&self, desc: &SamplerDesc) -> Result<SamplerIndex> {
            unimplemented!()
        }
        fn destroy_sampler(&self, sampler: SamplerIndex) -> Result<()> {
            unimplemented!()
        }
        fn create_graphics_pipeline(
            &self,
            desc: &GraphicsPipelineDesc<'_>,
        ) -> Result<GraphicsPipelineHandle> {
            unimplemented!()
        }
        fn create_compute_pipeline(
            &self,
            desc: &ComputePipelineDesc<'_>,
        ) -> Result<ComputePipelineHandle> {
            unimplemented!()
        }
        fn destroy_graphics_pipeline(&self, pipeline: GraphicsPipelineHandle) -> Result<()> {
            unimplemented!()
        }
        fn destroy_compute_pipeline(&self, pipeline: ComputePipelineHandle) -> Result<()> {
            unimplemented!()
        }
        fn pipeline_cache(&self) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn pipeline_stats(&self) -> PipelineStats {
            unimplemented!()
        }
        fn create_timeline(&self, initial: u64) -> Result<TimelineHandle> {
            unimplemented!()
        }
        fn destroy_timeline(&self, timeline: TimelineHandle) -> Result<()> {
            unimplemented!()
        }
        fn timeline_value(&self, timeline: TimelineHandle) -> Result<u64> {
            unimplemented!()
        }
        fn signal_timeline(&self, point: TimelinePoint) -> Result<()> {
            unimplemented!()
        }
        fn wait_timeline(&self, point: TimelinePoint, timeout: Duration) -> Result<bool> {
            unimplemented!()
        }
        fn create_encoder(&self, queue: QueueRole, label: &str) -> Result<Box<dyn Encoder>> {
            Ok(Box::new(SketchEncoder { queue, commands: Vec::new() }))
        }
        fn submit(&self, queue: QueueRole, submit: Submit<'_>) -> Result<()> {
            // A submit of no encoders waits and signals alone.
            match own(submit.encoders) {
                Some(encoders) if encoders.iter().all(|encoder| encoder.queue == queue) => Ok(()),
                _ => Err(Rule::Encoding.refuse("an encoder of another device or role")),
            }
        }
        fn create_swapchain(&self, desc: &SwapchainDesc) -> Result<Box<dyn Swapchain>> {
            Ok(Box::new(SketchSwapchain))
        }
    }

    impl Encoder for SketchEncoder {
        fn queue(&self) -> QueueRole {
            self.queue
        }
        fn reset(&mut self) -> Result<()> {
            self.commands.clear();
            Ok(())
        }
        fn transition_images(&mut self, transitions: &[ImageTransition]) {}
        fn transition_buffers(&mut self, transitions: &[BufferTransition]) {}
        fn alias(&mut self, image: ImageHandle) {}
        fn copy_buffer(
            &mut self,
            source: BufferHandle,
            destination: BufferHandle,
            regions: &[BufferCopy],
        ) {
        }
        fn copy_buffer_to_image(
            &mut self,
            source: BufferHandle,
            destination: ImageHandle,
            regions: &[BufferImageCopy],
        ) {
        }
        fn copy_image_to_buffer(
            &mut self,
            source: ImageHandle,
            destination: BufferHandle,
            regions: &[BufferImageCopy],
        ) {
        }
        fn begin_render_pass(&mut self, pass: &RenderPass<'_>) {}
        fn begin_compute_pass(&mut self, pass: &ComputePass<'_>) {}
        fn end_pass(&mut self) {}
        fn set_viewport(&mut self, viewport: Viewport) {}
        fn set_scissor(&mut self, scissor: Rect) {}
        fn bind_graphics_pipeline(&mut self, pipeline: GraphicsPipelineHandle) {}
        fn bind_compute_pipeline(&mut self, pipeline: ComputePipelineHandle) {}
        fn bind_index_buffer(&mut self, buffer: BufferHandle, offset: u64) {}
        fn push_bytes(&mut self, offset: u32, bytes: &[u8]) {}
        fn push_address(&mut self, offset: u32, buffer: BufferHandle, byte_offset: u64) {}
        fn push_slot(&mut self, offset: u32, slot: HeapSlot) {}
        fn local_read(&mut self) {}
        fn draw(&mut self, vertices: core::ops::Range<u32>, instances: core::ops::Range<u32>) {}
        fn draw_indexed(
            &mut self,
            indices: core::ops::Range<u32>,
            base_vertex: i32,
            instances: core::ops::Range<u32>,
        ) {
        }
        fn draw_indexed_indirect(
            &mut self,
            arguments: BufferHandle,
            offset: u64,
            count: u32,
            stride: u32,
        ) {
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
        }
        fn dispatch(&mut self, groups: [u32; 3]) {}
        fn dispatch_indirect(&mut self, arguments: BufferHandle, offset: u64) {}
    }

    impl Swapchain for SketchSwapchain {
        fn format(&self) -> Format {
            Format::Bgra8Srgb
        }
        fn extent(&self) -> Extent {
            Extent { width: 1, height: 1 }
        }
        fn image_count(&self) -> u32 {
            3
        }
        fn acquire(&mut self) -> Result<Acquired> {
            Ok(Acquired::None)
        }
        fn present(&mut self, image: ImageHandle, submit: Submit<'_>) -> Result<Presented> {
            unimplemented!()
        }
        fn resize(&mut self, extent: Extent) -> Result<()> {
            unimplemented!()
        }
        fn last_present(&self) -> Option<PresentId> {
            None
        }
        fn display_times(&mut self, into: &mut Vec<DisplayTime>) -> bool {
            false
        }
    }

    /// The device is shared between threads as `lactose-app` holds it, an encoder moves
    /// to the thread that records it, and the swapchain to the thread that presents.
    fn wiring(sketch: Sketch) -> Result<()> {
        let device: Arc<dyn Device> = Arc::new(sketch);
        let side = RenderSide { device: Arc::clone(&device) };
        let encoder = device.create_encoder(QueueRole::Graphics, "job")?;
        let recorder = std::thread::spawn(move || encoder);
        let mut swapchain = device
            .create_swapchain(&SwapchainDesc { extent: Extent { width: 1280, height: 720 } })?;
        let presenter = std::thread::spawn(move || swapchain.acquire());
        let sink: Arc<dyn MessageSink> = Arc::new(Counting);
        Ok(())
    }
}
