//! What the Vulkan device holds beyond the suite's cases: its creation on the driver the
//! loader finds, a device-only buffer filled through a staging copy, the pipelines of the
//! engine's shader, the pipeline cache's start from bytes that fail each check and a cache
//! directory that cannot be written, and each validator's canary. `cargo xtask gpu` runs every canary in a process of its own and
//! checks that it said its known message and nothing else.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use lactose_core::ErrorKind;
use lactose_rhi_api::{
    BufferCopy, BufferDesc, BufferHandle, BufferImageCopy, BufferReach, BufferState,
    BufferTransition, BufferUsage, CacheStart, ColourAttachment, ColourTargetDesc, ComputePass,
    ComputePipelineDesc, ComputePipelineHandle, Constant, ConstantValue, CookedShader, Cull,
    Device, DeviceRequest, Encoder, Extent, Format, GraphicsPipelineDesc, ImageDesc, ImageMemory,
    ImageRange, ImageState, ImageTransition, ImageUsage, Load, MemoryClass, PassReach, QueueRole,
    Rect, RenderPass, Sharing, StageDesc, StateOn, Store, Submit, Target, TimelinePoint, WriteMask,
};
use lactose_rhi_vulkan::VulkanDevice;

use crate::{Counting, engine_shader, request, suite_shader};

const PATIENCE: Duration = Duration::from_secs(20);

fn device(sink: &Arc<Counting>) -> VulkanDevice {
    VulkanDevice::new(request(Arc::clone(sink) as _)).expect("a Vulkan device")
}

#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn device_reports_its_driver_and_sends_nothing_when_clean() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let info = device.info();
    eprintln!("{info:?}\n{:?}", device.capabilities());
    assert_eq!(info.backend, "vulkan");
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn device_buffer_is_filled_through_a_staging_copy() {
    const SIZE: u64 = 4096;
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let buffer = |usage, memory| {
        let desc =
            BufferDesc { label: "staging", size: SIZE, usage, memory, sharing: Sharing::Exclusive };
        device.create_buffer(&desc).expect("a buffer")
    };
    let upload = buffer(BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let middle =
        buffer(BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION), MemoryClass::Device);
    let readback = buffer(BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let bytes: Vec<u8> = (0..SIZE).map(|at| (at * 7 % 251) as u8).collect();
    device
        .write_buffer(upload, 0, SIZE, &mut |out| out.copy_from_slice(&bytes))
        .expect("the host write");
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let mut encoder = device.create_encoder(QueueRole::Graphics, "staging").expect("an encoder");
    let all = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
    encoder.transition_buffers(&[
        BufferTransition {
            buffer: upload,
            from: on(BufferState::Undefined),
            to: on(BufferState::CopySource),
        },
        BufferTransition {
            buffer: middle,
            from: on(BufferState::Undefined),
            to: on(BufferState::CopyDestination),
        },
    ]);
    encoder.copy_buffer(upload, middle, &all);
    encoder.transition_buffers(&[
        BufferTransition {
            buffer: middle,
            from: on(BufferState::CopyDestination),
            to: on(BufferState::CopySource),
        },
        BufferTransition {
            buffer: readback,
            from: on(BufferState::Undefined),
            to: on(BufferState::CopyDestination),
        },
    ]);
    encoder.copy_buffer(middle, readback, &all);
    encoder.transition_buffers(&[BufferTransition {
        buffer: readback,
        from: on(BufferState::CopyDestination),
        to: on(BufferState::HostRead),
    }]);
    let timeline = device.create_timeline(0).expect("a timeline");
    let done = TimelinePoint { timeline, value: 1 };
    let mut encoders = [encoder];
    device
        .submit(
            QueueRole::Graphics,
            Submit { encoders: &mut encoders, waits: &[], signals: &[done] },
        )
        .expect("the submit");
    assert!(device.wait_timeline(done, PATIENCE).expect("the wait"), "the copies did not complete");
    let mut read = Vec::new();
    device
        .read_buffer(readback, 0, SIZE, &mut |got| read.extend_from_slice(got))
        .expect("the host read");
    assert_eq!(read, bytes);
    drop(encoders);
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// Submits `encoder` to the graphics queue and waits for it.
fn run(device: &VulkanDevice, encoder: Box<dyn Encoder>) {
    let timeline = device.create_timeline(0).expect("a timeline");
    let done = TimelinePoint { timeline, value: 1 };
    let mut encoders = [encoder];
    device
        .submit(
            QueueRole::Graphics,
            Submit { encoders: &mut encoders, waits: &[], signals: &[done] },
        )
        .expect("the submit");
    assert!(device.wait_timeline(done, PATIENCE).expect("the wait"), "the work did not complete");
}

fn buffer(device: &VulkanDevice, usage: BufferUsage, memory: MemoryClass) -> BufferHandle {
    let desc =
        BufferDesc { label: "canary", size: 4096, usage, memory, sharing: Sharing::Exclusive };
    device.create_buffer(&desc).expect("a buffer")
}

/// Whether `sink` heard `id`, and nothing without it.
fn heard_alone(sink: &Counting, id: &str) {
    let texts = sink.texts();
    eprintln!("the canary heard: {texts:?}");
    assert!(!texts.is_empty(), "the canary {id} was not heard: is the layer loaded?");
    assert!(texts.iter().all(|text| text.contains(id)), "{texts:?}");
}

/// Core validation's canary: an image declared in `SampledRead` that has never left
/// `Undefined`, a breach of the "Null alone" column the layer reports at the submit.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn canary_core_validation_reports_a_wrong_layout() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let source = buffer(&device, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let extent = Extent { width: 4, height: 4 };
    let image = device
        .create_image(&ImageDesc {
            label: "canary",
            format: Format::Rgba8Unorm,
            extent,
            mips: 1,
            layers: 1,
            usage: ImageUsage::SAMPLED.union(ImageUsage::COPY_DESTINATION),
            memory: ImageMemory::Own,
            sharing: Sharing::Exclusive,
        })
        .expect("an image");
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let mut encoder = device.create_encoder(QueueRole::Graphics, "canary").expect("an encoder");
    encoder.transition_buffers(&[BufferTransition {
        buffer: source,
        from: on(BufferState::Undefined),
        to: on(BufferState::CopySource),
    }]);
    encoder.transition_images(&[ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: StateOn { state: ImageState::SampledRead, queue: QueueRole::Graphics },
        to: StateOn { state: ImageState::CopyDestination, queue: QueueRole::Graphics },
    }]);
    let rect = Rect { x: 0, y: 0, extent };
    encoder.copy_buffer_to_image(
        source,
        image,
        &[BufferImageCopy { buffer_offset: 0, mip: 0, layer: 0, rect }],
    );
    run(&device, encoder);
    drop(device);
    heard_alone(&sink, "current layout is VK_IMAGE_LAYOUT_UNDEFINED");
}

/// Synchronisation validation's canary: two copies into one range with no barrier between
/// them, a race of the "Null alone" column.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn canary_synchronisation_validation_reports_a_race() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let source = buffer(&device, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let target = buffer(&device, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let mut encoder = device.create_encoder(QueueRole::Graphics, "canary").expect("an encoder");
    encoder.transition_buffers(&[
        BufferTransition {
            buffer: source,
            from: on(BufferState::Undefined),
            to: on(BufferState::CopySource),
        },
        BufferTransition {
            buffer: target,
            from: on(BufferState::Undefined),
            to: on(BufferState::CopyDestination),
        },
    ]);
    let all = [BufferCopy { source_offset: 0, destination_offset: 0, size: 4096 }];
    encoder.copy_buffer(source, target, &all);
    encoder.copy_buffer(source, target, &all);
    run(&device, encoder);
    drop(device);
    heard_alone(&sink, "SYNC-HAZARD-WRITE-AFTER-WRITE");
}

/// The canary of synchronisation validation's load-after-store check
/// (`syncval_load_op_after_store_op_validation`, verification row 211): a colour target
/// stored by one render pass and kept by the next with no transition between them, a breach
/// of the "Null alone" column. The layer reports a load after a store between two render pass
/// instances only with the setting on.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn canary_load_after_store_validation_reports_a_kept_target() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let image = device
        .create_image(&ImageDesc {
            label: "canary",
            format: Format::Rgba8Unorm,
            extent: Extent { width: 4, height: 4 },
            mips: 1,
            layers: 1,
            usage: ImageUsage::COLOUR,
            memory: ImageMemory::Own,
            sharing: Sharing::Exclusive,
        })
        .expect("an image");
    let mut encoder = device.create_encoder(QueueRole::Graphics, "canary").expect("an encoder");
    encoder.transition_images(&[ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: StateOn { state: ImageState::Undefined, queue: QueueRole::Graphics },
        to: StateOn { state: ImageState::ColourAttachment, queue: QueueRole::Graphics },
    }]);
    let target = Target { image, mip: 0, layer: 0 };
    for load in [Load::Clear([0.0; 4]), Load::Keep] {
        let colour = [ColourAttachment { target, load, store: Store::Store, local_read: false }];
        encoder.begin_render_pass(&RenderPass {
            label: "canary",
            colour: &colour,
            depth: None,
            reach: PassReach::NOTHING,
        });
        encoder.end_pass();
    }
    run(&device, encoder);
    drop(device);
    heard_alone(&sink, LOAD_AFTER_STORE_CANARY);
}

/// What the load-after-store canary must say.
const LOAD_AFTER_STORE_CANARY: &str = "SYNC-HAZARD-READ-AFTER-WRITE";

/// A compute stage whose workgroup is not its module's `LocalSize` is refused before the
/// driver sees it, since Vulkan dispatches workgroups of the module's size alone; the same
/// stage of its module's size is made.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn workgroup_other_than_the_modules_is_refused() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let shader = suite_shader("workgroup.cs_main").expect("the suite's cooked shaders");
    assert_eq!(shader.workgroup, Some([8, 4, 2]), "workgroup.slang's size");
    let make = |shader: &CookedShader| {
        let stage = StageDesc { shader, constants: &[] };
        device.create_compute_pipeline(&ComputePipelineDesc { label: "device", stage })
    };
    for other in [[4, 4, 2], [8, 4, 1], [2, 8, 2]] {
        let shader = CookedShader { workgroup: Some(other), ..shader.clone() };
        let refused = make(&shader).expect_err("a workgroup other than the module's");
        assert_eq!(refused.kind(), ErrorKind::InvalidData, "{refused}");
    }
    make(&shader).expect("the module's own workgroup");
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// A device's cache start from `bytes`.
fn start(bytes: Vec<u8>) -> CacheStart {
    let sink = Arc::new(Counting::default());
    let request = DeviceRequest { pipeline_cache: bytes, ..request(Arc::clone(&sink) as _) };
    let device = VulkanDevice::new(request).expect("a device, whatever its cache's bytes");
    let start = device.pipeline_stats().cache;
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
    start
}

#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn cache_that_fails_a_check_starts_empty() {
    let sink = Arc::new(Counting::default());
    let sealed = device(&sink).pipeline_cache().expect("the cache's bytes");
    assert_eq!(start(Vec::new()), CacheStart::Empty);
    assert_eq!(start(sealed.clone()), CacheStart::Loaded);
    let mut truncated = sealed.clone();
    truncated.truncate(sealed.len() / 2);
    assert_eq!(start(truncated), CacheStart::Rejected, "a truncated file");
    let mut checksum = sealed.clone();
    *checksum.last_mut().expect("bytes") ^= 1;
    assert_eq!(start(checksum), CacheStart::Rejected, "a wrong checksum");
    // A byte of the driver's payload changed after the seal: the checksum no longer holds.
    let mut corrupt = sealed.clone();
    let inside = sealed.len() - 8 - 16;
    corrupt[inside] ^= 0x5a;
    assert_eq!(start(corrupt), CacheStart::Rejected, "a corrupt payload");
    // The payload's own header is the 32 bytes before the trailing checksum's payload: a
    // sealed cache of a payload whose header names another device is stale.
    let payload = lactose_rhi_api::open_cache(&sealed, identity(&sealed)).expect("the payload");
    let mut stale = payload.to_vec();
    stale[8] ^= 0xff;
    let resealed = lactose_rhi_api::seal_cache(identity(&sealed), &stale).expect("a seal");
    assert_eq!(start(resealed), CacheStart::Rejected, "a stale header");
    // Garbage after the driver's own header, sealed as a good cache: what the driver makes of
    // it is the driver's, and recorded. The device is made and sends nothing either way.
    let mut garbage = payload.to_vec();
    garbage.truncate(32);
    garbage.extend_from_slice(&[0xa5; 64]);
    let resealed = lactose_rhi_api::seal_cache(identity(&sealed), &garbage).expect("a seal");
    let garbage_start = start(resealed);
    eprintln!("garbage sealed under the driver's header starts the cache {garbage_start:?}");
    assert_ne!(garbage_start, CacheStart::Empty);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// The identity a sealed cache carries: the bytes after its magic, version and length.
fn identity(sealed: &[u8]) -> lactose_rhi_api::CacheIdentity<'_> {
    let len = u32::from_le_bytes([sealed[8], sealed[9], sealed[10], sealed[11]]) as usize;
    lactose_rhi_api::CacheIdentity(&sealed[12..12 + len])
}

/// The engine's shader, `shaders/surface.slang`, as the cook wrote it, makes its two
/// pipelines on the device: its surface with its specialisation constant given, into one
/// target, and its resolve, which reads target 0 through the local read and writes target 1.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn engine_shader_makes_its_pipelines() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let shader = |name: &str| engine_shader(name).expect("the engine's cooked shaders");
    let (vertex, surface, resolve) =
        (shader("surface.vs_main"), shader("surface.fs_main"), shader("surface.fs_resolve"));
    let constants = [Constant { id: 0, value: ConstantValue::Uint(7) }];
    let target = ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::ALL };
    // The resolve reads target 0 as its input, so it may not write it.
    let read = ColourTargetDesc { write_mask: WriteMask::NONE, ..target };
    for (fragment, constants, colour) in
        [(&surface, &constants[..], &[target][..]), (&resolve, &[][..], &[read, target][..])]
    {
        let desc = GraphicsPipelineDesc {
            label: "engine",
            vertex: StageDesc { shader: &vertex, constants: &[] },
            fragment: Some(StageDesc { shader: fragment, constants }),
            colour,
            depth: None,
            cull: Cull::None,
        };
        device.create_graphics_pipeline(&desc).expect("a pipeline of the engine's shader");
    }
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// A compute pipeline of the suite's cooked `name`.
fn kernel(device: &VulkanDevice, name: &str) -> ComputePipelineHandle {
    let shader = suite_shader(name).expect("the suite's cooked shaders");
    let stage = StageDesc { shader: &shader, constants: &[] };
    device
        .create_compute_pipeline(&ComputePipelineDesc { label: "device", stage })
        .expect("a compute pipeline")
}

/// Writes a device's cache bytes into `dir` as a caller does at shutdown.
fn save_cache(dir: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("pipeline.cache"), bytes)
}

/// A cache directory that cannot be written is reported, and costs the cache alone: the
/// device still makes a pipeline and runs work, and still gives its cache's bytes.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn unwritable_cache_directory_is_reported_and_the_run_goes_on() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    // A directory below a regular file cannot be made on any host, whoever runs the test.
    let file = std::env::temp_dir().join(format!("lactose-cache-{}", std::process::id()));
    std::fs::write(&file, b"not a directory").expect("a file in the temporary directory");
    let dir = file.join("cache");
    let bytes = device.pipeline_cache().expect("the cache's bytes");
    let saved = save_cache(&dir, &bytes);
    std::fs::remove_file(&file).expect("the file removed");
    let Err(error) = saved else { panic!("{} was written", dir.display()) };
    eprintln!("the pipeline cache was not saved to {}: {error}; the run goes on", dir.display());
    let pipeline = kernel(&device, "store.cs_main");
    let target = buffer(&device, BufferUsage::DEVICE_ADDRESS, MemoryClass::Device);
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let mut encoder = device.create_encoder(QueueRole::Graphics, "after").expect("an encoder");
    encoder.transition_buffers(&[BufferTransition {
        buffer: target,
        from: on(BufferState::Undefined),
        to: on(BufferState::ShaderWrite),
    }]);
    let declared = [BufferReach { buffer: target, state: BufferState::ShaderWrite }];
    encoder.begin_compute_pass(&ComputePass {
        label: "after",
        reach: PassReach { slots: &[], buffers: &declared },
    });
    encoder.bind_compute_pipeline(pipeline);
    // `store.slang`'s block: 16 words of 7 upward, with no lag.
    encoder.push_address(0, target, 0);
    encoder.push_bytes(8, &[16, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0]);
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    run(&device, encoder);
    assert!(!device.pipeline_cache().expect("the cache's bytes").is_empty());
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// GPU-assisted validation's canary: a shader's read one word past the end of a buffer it
/// reaches through its address, which no check before the driver can see. Run by
/// `cargo xtask gpu --gpu-assisted` alone: without that validator the read is the driver's
/// to make, of bytes past the buffer.
#[test]
#[ignore = "needs GPU-assisted validation: cargo xtask gpu --gpu-assisted"]
fn canary_gpu_assisted_validation_reports_a_read_past_a_buffer() {
    const WORDS: u32 = 64;
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let pipeline = kernel(&device, "copy.cs_main");
    let desc = |size| BufferDesc {
        label: "canary",
        size,
        usage: BufferUsage::DEVICE_ADDRESS,
        memory: MemoryClass::Device,
        sharing: Sharing::Exclusive,
    };
    let from = device.create_buffer(&desc(u64::from(WORDS * 4))).expect("a buffer");
    let to = device.create_buffer(&desc(u64::from(WORDS * 8))).expect("a buffer");
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let mut encoder = device.create_encoder(QueueRole::Graphics, "canary").expect("an encoder");
    encoder.transition_buffers(&[
        BufferTransition {
            buffer: from,
            from: on(BufferState::Undefined),
            to: on(BufferState::ShaderRead),
        },
        BufferTransition {
            buffer: to,
            from: on(BufferState::Undefined),
            to: on(BufferState::ShaderWrite),
        },
    ]);
    let declared = [
        BufferReach { buffer: from, state: BufferState::ShaderRead },
        BufferReach { buffer: to, state: BufferState::ShaderWrite },
    ];
    encoder.begin_compute_pass(&ComputePass {
        label: "canary",
        reach: PassReach { slots: &[], buffers: &declared },
    });
    encoder.bind_compute_pipeline(pipeline);
    // `copy.slang`'s block: one word more than `from` holds, each copied with nothing added.
    encoder.push_address(0, from, 0);
    encoder.push_address(8, to, 0);
    encoder.push_bytes(16, &(WORDS + 1).to_le_bytes());
    encoder.push_bytes(20, &0u32.to_le_bytes());
    encoder.dispatch([2, 1, 1]);
    encoder.end_pass();
    run(&device, encoder);
    drop(device);
    heard_alone(&sink, GPU_ASSISTED_CANARY);
}

/// What GPU-assisted validation's canary must say.
const GPU_ASSISTED_CANARY: &str = "VUID-RuntimeSpirv-PhysicalStorageBuffer64";
