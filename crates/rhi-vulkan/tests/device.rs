//! What the Vulkan device holds beyond the suite's cases: its creation on the driver the
//! loader finds, a device-only buffer filled through a staging copy, and the pipeline
//! cache's start from bytes that fail each check.

use std::sync::Arc;
use std::time::Duration;

use lactose_rhi_api::{
    BufferCopy, BufferDesc, BufferHandle, BufferImageCopy, BufferState, BufferTransition,
    BufferUsage, CacheStart, Device, DeviceRequest, Encoder, Extent, Format, ImageDesc,
    ImageMemory, ImageRange, ImageState, ImageTransition, ImageUsage, MemoryClass, QueueRole, Rect,
    Sharing, StateOn, Submit, TimelinePoint,
};
use lactose_rhi_vulkan::VulkanDevice;

use crate::{Counting, request};

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
    // The payload's own header is the 32 bytes before the trailing checksum's payload: a
    // sealed cache of a payload whose header names another device is stale.
    let payload = lactose_rhi_api::open_cache(&sealed, identity(&sealed)).expect("the payload");
    let mut stale = payload.to_vec();
    stale[8] ^= 0xff;
    let resealed = lactose_rhi_api::seal_cache(identity(&sealed), &stale).expect("a seal");
    assert_eq!(start(resealed), CacheStart::Rejected, "a stale header");
    let mut corrupt = payload.to_vec();
    corrupt.truncate(32);
    corrupt.extend_from_slice(&[0xa5; 64]);
    let resealed = lactose_rhi_api::seal_cache(identity(&sealed), &corrupt).expect("a seal");
    let corrupt_start = start(resealed);
    eprintln!("a corrupt payload under a good header starts the cache {corrupt_start:?}");
    assert_ne!(corrupt_start, CacheStart::Empty);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// The identity a sealed cache carries: the bytes after its magic, version and length.
fn identity(sealed: &[u8]) -> lactose_rhi_api::CacheIdentity<'_> {
    let len = u32::from_le_bytes([sealed[8], sealed[9], sealed[10], sealed[11]]) as usize;
    lactose_rhi_api::CacheIdentity(&sealed[12..12 + len])
}
