//! What the Vulkan device holds beyond the suite's cases: its creation on the driver the
//! loader finds, a device-only buffer filled through a staging copy, the pipelines of the
//! engine's shader, a workgroup or an entry point the module does not declare, a heap past
//! the driver's support and a resource past the device's size, the pipeline cache's start
//! from bytes that fail each check and a cache directory that cannot be written, the drops
//! while a thread unwinds, and each validator's canary but thread safety's, which is a unit
//! test (`src/canary.rs`). `cargo xtask gpu` runs every canary in a process of its own and
//! checks that it said its known message and nothing else.

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use lactose_core::ErrorKind;
use lactose_rhi_api::{
    AddressMode, BufferCopy, BufferDesc, BufferHandle, BufferImageCopy, BufferReach, BufferState,
    BufferTransition, BufferUsage, CacheStart, ColourAttachment, ColourTargetDesc, ComputePass,
    ComputePipelineDesc, ComputePipelineHandle, Constant, ConstantValue, CookedShader, Cull,
    Device, DeviceRequest, Encoder, Extent, Filter, Format, GraphicsPipelineDesc, ImageDesc,
    ImageMemory, ImageRange, ImageState, ImageTransition, ImageUsage, Load, MemoryBlockDesc,
    MemoryClass, PassReach, QueuePath, QueueRole, Rect, RenderPass, SamplerDesc, Sharing,
    StageDesc, StateOn, Store, Submit, Target, TimelinePoint, WriteMask,
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

/// Synchronisation validation's canary across queues: a colour target stored by a graphics
/// submit that a lagging dispatch holds in flight, and taken from `Undefined` (a layout
/// transition, a write) by a compute submit with no semaphore between them, a race of the
/// "Null alone" column. The layer hears it only while it does not know the graphics submit
/// complete, which the device learns through waits alone (verification row 211). A device
/// whose compute role shares the graphics queue has no second queue to race on.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn canary_synchronisation_validation_reports_a_race_across_queues() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    if device.capabilities().queues.compute != QueuePath::Own {
        eprintln!(
            "canary_synchronisation_validation_reports_a_race_across_queues: skipped, compute \
             shares the graphics queue"
        );
        return;
    }
    let image = device
        .create_image(&ImageDesc {
            label: "canary",
            format: Format::Rgba8Unorm,
            extent: Extent { width: 4, height: 4 },
            mips: 1,
            layers: 1,
            usage: ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE),
            memory: ImageMemory::Own,
            sharing: Sharing::Exclusive,
        })
        .expect("an image");
    let mut graphics = lagging(&device);
    graphics.transition_images(&[ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: StateOn { state: ImageState::Undefined, queue: QueueRole::Graphics },
        to: StateOn { state: ImageState::ColourAttachment, queue: QueueRole::Graphics },
    }]);
    let target = Target { image, mip: 0, layer: 0 };
    let colour = [ColourAttachment {
        target,
        load: Load::Clear([0.0; 4]),
        store: Store::Store,
        local_read: false,
    }];
    graphics.begin_render_pass(&RenderPass {
        label: "canary",
        colour: &colour,
        depth: None,
        reach: PassReach::NOTHING,
    });
    graphics.end_pass();
    let mut compute = device.create_encoder(QueueRole::Compute, "canary").expect("an encoder");
    compute.transition_images(&[ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: StateOn { state: ImageState::Undefined, queue: QueueRole::Compute },
        to: StateOn { state: ImageState::CopySource, queue: QueueRole::Compute },
    }]);
    let timeline = device.create_timeline(0).expect("a timeline");
    let done = TimelinePoint { timeline, value: 1 };
    let mut first = [graphics];
    device
        .submit(QueueRole::Graphics, Submit { encoders: &mut first, waits: &[], signals: &[done] })
        .expect("the graphics submit");
    let mut second = [compute];
    device
        .submit(QueueRole::Compute, Submit { encoders: &mut second, waits: &[], signals: &[] })
        .expect("the compute submit");
    assert!(device.wait_timeline(done, PATIENCE).expect("the wait"), "the work did not complete");
    drop((first, second));
    drop(device);
    heard_alone(&sink, CROSS_QUEUE_CANARY);
}

/// What the cross-queue canary must say.
const CROSS_QUEUE_CANARY: &str = "SYNC-HAZARD-WRITE-RACING-WRITE";

/// Synchronisation validation's canary across two submits of one queue, which a device with
/// one queue runs too: a lagging submit copies into a buffer, and a second submit, made while
/// the first is in flight, takes the buffer from `CopySource` (it is in `CopyDestination`) and
/// copies into it again, a race of the "Null alone" column. As across queues, the layer hears
/// it only while it does not know the first submit complete (verification row 211).
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn canary_synchronisation_validation_reports_a_race_across_submits() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let source = buffer(&device, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let target = buffer(&device, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    let all = [BufferCopy { source_offset: 0, destination_offset: 0, size: 4096 }];
    let mut first = lagging(&device);
    first.transition_buffers(&[
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
    first.copy_buffer(source, target, &all);
    let mut second = device.create_encoder(QueueRole::Graphics, "canary").expect("an encoder");
    second.transition_buffers(&[BufferTransition {
        buffer: target,
        from: on(BufferState::CopySource),
        to: on(BufferState::CopyDestination),
    }]);
    second.copy_buffer(source, target, &all);
    let timeline = device.create_timeline(0).expect("a timeline");
    let done = TimelinePoint { timeline, value: 1 };
    let mut first = [first];
    device
        .submit(QueueRole::Graphics, Submit { encoders: &mut first, waits: &[], signals: &[] })
        .expect("the first submit");
    let mut second = [second];
    device
        .submit(QueueRole::Graphics, Submit { encoders: &mut second, waits: &[], signals: &[done] })
        .expect("the second submit");
    assert!(device.wait_timeline(done, PATIENCE).expect("the wait"), "the work did not complete");
    drop((first, second));
    drop(device);
    heard_alone(&sink, "SYNC-HAZARD-WRITE-AFTER-WRITE");
}

/// One transition call that names one subresource or one buffer twice, which this crate
/// sends as one `vkCmdPipelineBarrier2` whose elements are unordered and leaves to rhi-null
/// (`lactose-rhi-api`, "States"), reaches the driver and no validator hears it. On one
/// queue, a take from `Undefined` and then a change of a new image. Where compute has a
/// queue of its own, a change and then a release, a take and then a release, of an image and
/// of a buffer, each acquired by a compute submit that waits for the release's. Core
/// validation reads a call's elements in order; synchronisation validation reads them
/// together, and hears a change and then another of an image a copy wrote, the second
/// element's layout change racing the copy (`SYNC-HAZARD-WRITE-AFTER-WRITE`), a chain this
/// test leaves out since the suite hears canaries alone.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn transition_call_naming_a_resource_twice_is_sent_and_not_heard() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let extent = Extent { width: 4, height: 4 };
    let image = || {
        device
            .create_image(&ImageDesc {
                label: "repeat",
                format: Format::Rgba8Unorm,
                extent,
                mips: 1,
                layers: 1,
                usage: ImageUsage::SAMPLED
                    .union(ImageUsage::COPY_DESTINATION)
                    .union(ImageUsage::COPY_SOURCE),
                memory: ImageMemory::Own,
                sharing: Sharing::Exclusive,
            })
            .expect("an image")
    };
    const fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
        StateOn { state, queue }
    }
    let across = |image, from, to| ImageTransition { image, range: ImageRange::FIRST, from, to };
    let (g, c) = (QueueRole::Graphics, QueueRole::Compute);
    let rect = Rect { x: 0, y: 0, extent };
    let whole = [BufferImageCopy { buffer_offset: 0, mip: 0, layer: 0, rect }];
    let source = buffer(&device, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    device.write_buffer(source, 0, 4096, &mut |out| out.fill(3)).expect("the host write");
    let filled = |encoder: &mut dyn Encoder, image| {
        encoder.transition_images(&[across(
            image,
            on(ImageState::Undefined, g),
            on(ImageState::CopyDestination, g),
        )]);
        encoder.copy_buffer_to_image(source, image, &whole);
    };
    let chain = image();
    let mut encoder = device.create_encoder(g, "repeat").expect("an encoder");
    encoder.transition_buffers(&[BufferTransition {
        buffer: source,
        from: on(BufferState::Undefined, g),
        to: on(BufferState::CopySource, g),
    }]);
    encoder.transition_images(&[
        across(chain, on(ImageState::Undefined, g), on(ImageState::CopyDestination, g)),
        across(chain, on(ImageState::CopyDestination, g), on(ImageState::SampledRead, g)),
    ]);
    run(&device, encoder);
    if device.capabilities().queues.compute != QueuePath::Own {
        eprintln!("no release is sent: compute shares the graphics queue");
    } else {
        let changed = image();
        let taken = image();
        let data = buffer(&device, BufferUsage::DEVICE_ADDRESS, MemoryClass::Device);
        let mut graphics = device.create_encoder(g, "repeat").expect("an encoder");
        filled(&mut *graphics, changed);
        graphics.transition_images(&[
            across(changed, on(ImageState::CopyDestination, g), on(ImageState::SampledRead, g)),
            across(changed, on(ImageState::SampledRead, g), on(ImageState::SampledRead, c)),
        ]);
        graphics.transition_images(&[
            across(taken, on(ImageState::Undefined, g), on(ImageState::SampledRead, g)),
            across(taken, on(ImageState::SampledRead, g), on(ImageState::SampledRead, c)),
        ]);
        graphics.transition_buffers(&[
            BufferTransition {
                buffer: data,
                from: on(BufferState::Undefined, g),
                to: on(BufferState::ShaderRead, g),
            },
            BufferTransition {
                buffer: data,
                from: on(BufferState::ShaderRead, g),
                to: on(BufferState::ShaderRead, c),
            },
        ]);
        let mut compute = device.create_encoder(c, "repeat").expect("an encoder");
        compute.transition_images(&[
            across(changed, on(ImageState::SampledRead, g), on(ImageState::SampledRead, c)),
            across(taken, on(ImageState::SampledRead, g), on(ImageState::SampledRead, c)),
        ]);
        compute.transition_buffers(&[BufferTransition {
            buffer: data,
            from: on(BufferState::ShaderRead, g),
            to: on(BufferState::ShaderRead, c),
        }]);
        let timeline = device.create_timeline(0).expect("a timeline");
        let released = TimelinePoint { timeline, value: 1 };
        let acquired = TimelinePoint { timeline, value: 2 };
        let mut first = [graphics];
        device
            .submit(g, Submit { encoders: &mut first, waits: &[], signals: &[released] })
            .expect("the releasing submit");
        let mut second = [compute];
        device
            .submit(c, Submit { encoders: &mut second, waits: &[released], signals: &[acquired] })
            .expect("the acquiring submit");
        assert!(
            device.wait_timeline(acquired, PATIENCE).expect("the wait"),
            "the work did not complete"
        );
        drop((first, second));
    }
    drop(device);
    assert_eq!(sink.count(), 0, "the validators heard: {:?}", sink.texts());
}

/// A graphics encoder whose first command is a dispatch that runs for a while (`store.slang`
/// with a lag), so that what it records after is in flight when its submit returns.
fn lagging(device: &VulkanDevice) -> Box<dyn Encoder> {
    let pipeline = kernel(device, "store.cs_main");
    let scratch = buffer(device, BufferUsage::DEVICE_ADDRESS, MemoryClass::Device);
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let mut encoder = device.create_encoder(QueueRole::Graphics, "lagging").expect("an encoder");
    encoder.transition_buffers(&[BufferTransition {
        buffer: scratch,
        from: on(BufferState::Undefined),
        to: on(BufferState::ShaderWrite),
    }]);
    let declared = [BufferReach { buffer: scratch, state: BufferState::ShaderWrite }];
    encoder.begin_compute_pass(&ComputePass {
        label: "lagging",
        reach: PassReach { slots: &[], buffers: &declared },
    });
    encoder.bind_compute_pipeline(pipeline);
    // `store.slang`'s block: one word of 7, after 2^25 rounds (about a second on the M1).
    encoder.push_address(0, scratch, 0);
    encoder.push_bytes(8, &1u32.to_le_bytes());
    encoder.push_bytes(12, &(1u32 << 25).to_le_bytes());
    encoder.push_bytes(16, &7u32.to_le_bytes());
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    encoder
}

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

#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn entry_point_the_module_does_not_declare_is_refused() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let shader = |name: &str| suite_shader(name).expect("the suite's cooked shaders");
    let (vertex, fragment) = (shader("fill.vs_main"), shader("fill.fs_main"));
    let targets = [ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::ALL }];
    let make = |vertex: &CookedShader, fragment: &CookedShader| {
        device.create_graphics_pipeline(&GraphicsPipelineDesc {
            label: "device",
            vertex: StageDesc { shader: vertex, constants: &[] },
            fragment: Some(StageDesc { shader: fragment, constants: &[] }),
            colour: &targets,
            depth: None,
            cull: Cull::None,
        })
    };
    let renamed = |shader: &CookedShader, entry: &str| CookedShader {
        entry: entry.to_owned(),
        ..shader.clone()
    };
    // A name the module lacks, and the other stage's name, which the module declares of the
    // other execution model.
    let cases = [
        (renamed(&vertex, "no_such_entry"), fragment.clone()),
        (renamed(&vertex, &fragment.entry), fragment.clone()),
        (vertex.clone(), renamed(&fragment, "no_such_entry")),
        (vertex.clone(), renamed(&fragment, &vertex.entry)),
    ];
    for (vertex, fragment) in &cases {
        let refused = make(vertex, fragment).expect_err("an entry the module does not declare");
        assert_eq!(refused.kind(), ErrorKind::InvalidData, "{refused}");
    }
    make(&vertex, &fragment).expect("the module's own entries");
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn heap_past_the_drivers_support_is_refused() {
    // MoltenVK 1.4.2 supports at most 1211 slots and samplers in one set, below its
    // `maxPerSetDescriptors` of 1212, and 1024 samplers live: so it makes the first and
    // refuses the rest. KosmicKrisp makes all seven.
    let sink = Arc::new(Counting::default());
    let driver = device(&sink).info().driver.clone();
    let mut made = Vec::new();
    for (slots, samplers) in
        [(1147, 64), (1148, 64), (1149, 64), (4096, 64), (1_000_000, 64), (1024, 256), (1024, 1024)]
    {
        let sink = Arc::new(Counting::default());
        let device = VulkanDevice::new(DeviceRequest {
            heap_slots: slots,
            samplers,
            ..request(Arc::clone(&sink) as _)
        });
        match &device {
            Ok(_) => {
                eprintln!("{slots} slots and {samplers} samplers: made");
                made.push((slots, samplers));
            }
            Err(refused) => {
                eprintln!("{slots} slots and {samplers} samplers: {refused}");
                assert_eq!(refused.kind(), ErrorKind::Unsupported, "{refused}");
            }
        }
        drop(device);
        // A device made past its support is one the layer reports.
        assert_eq!(sink.count(), 0, "{slots} slots, {samplers} samplers: {:?}", sink.texts());
    }
    // The boundary the crate's README states: a driver that moves it moves the README.
    if driver.starts_with("MoltenVK") {
        assert_eq!(made, [(1147, 64)], "{driver}");
    }
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn resource_past_the_devices_size_is_refused() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    for size in [1 << 40, 1 << 62] {
        let desc = BufferDesc {
            label: "device",
            size,
            usage: BufferUsage::COPY_DESTINATION,
            memory: MemoryClass::Device,
            sharing: Sharing::Exclusive,
        };
        let refused = device.create_buffer(&desc).expect_err("a buffer past the device's size");
        assert_eq!(refused.kind(), ErrorKind::ResourceExhausted, "{refused}");
        let block = MemoryBlockDesc { label: "device", size };
        let refused =
            device.create_memory_block(&block).expect_err("a block past the device's size");
        assert_eq!(refused.kind(), ErrorKind::ResourceExhausted, "{refused}");
    }
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
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    read_past_a_buffer(&device);
    drop(device);
    heard_alone(&sink, GPU_ASSISTED_CANARY);
}

/// Metal's shader validation's canary, in a run under MoltenVK: the same read past a buffer,
/// which Metal reports on standard error, where `cargo xtask gpu` must hear it
/// (`METAL_CANARY`). The layer, without GPU-assisted validation, hears nothing of it.
#[test]
#[ignore = "needs Metal's shader validation: cargo xtask gpu --driver moltenvk"]
fn canary_metal_shader_validation_reports_a_read_past_a_buffer() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    read_past_a_buffer(&device);
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// Runs a dispatch of `copy.slang` that reads one word past the end of a buffer of 64 words.
fn read_past_a_buffer(device: &VulkanDevice) {
    const WORDS: u32 = 64;
    let pipeline = kernel(device, "copy.cs_main");
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
    run(device, encoder);
}

/// What GPU-assisted validation's canary must say.
const GPU_ASSISTED_CANARY: &str = "VUID-RuntimeSpirv-PhysicalStorageBuffer64";

/// GPU-assisted validation's canary of the heap: a draw that samples slot `heap_slots`, one
/// past the heap's end, by an index in push bytes, which no check before the driver can see.
/// Run by `cargo xtask gpu --gpu-assisted` alone, as the canary of a read past a buffer is.
#[test]
#[ignore = "needs GPU-assisted validation: cargo xtask gpu --gpu-assisted"]
fn canary_gpu_assisted_validation_reports_a_slot_past_the_heap() {
    let sink = Arc::new(Counting::default());
    let device = device(&sink);
    let shader = |name: &str| suite_shader(name).expect("the suite's cooked shaders");
    let (vertex, fragment) = (shader("sample.vs_main"), shader("sample.fs_main"));
    let colour = [ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::ALL }];
    let pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            label: "canary",
            vertex: StageDesc { shader: &vertex, constants: &[] },
            fragment: Some(StageDesc { shader: &fragment, constants: &[] }),
            colour: &colour,
            depth: None,
            cull: Cull::None,
        })
        .expect("a pipeline");
    let sampler = device
        .create_sampler(&SamplerDesc {
            magnify: Filter::Nearest,
            minify: Filter::Nearest,
            mip: Filter::Nearest,
            address_u: AddressMode::ClampToEdge,
            address_v: AddressMode::ClampToEdge,
        })
        .expect("a sampler");
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
    let load = Load::Clear([0.0; 4]);
    encoder.begin_render_pass(&RenderPass {
        label: "canary",
        colour: &[ColourAttachment { target, load, store: Store::Store, local_read: false }],
        depth: None,
        reach: PassReach::NOTHING,
    });
    encoder.bind_graphics_pipeline(pipeline);
    // `sample.slang`'s block: the slot, the left and the right half's sampler, the side.
    let past = device.capabilities().heap_slots;
    encoder.push_bytes(0, &past.to_le_bytes());
    encoder.push_bytes(4, &sampler.0.to_le_bytes());
    encoder.push_bytes(8, &sampler.0.to_le_bytes());
    encoder.push_bytes(12, &4u32.to_le_bytes());
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    run(&device, encoder);
    drop(device);
    heard_alone(&sink, GPU_ASSISTED_HEAP_CANARY);
}

/// What GPU-assisted validation's heap canary must say.
const GPU_ASSISTED_HEAP_CANARY: &str = "VUID-vkCmdDraw-None-10068";

/// Whether the run turns GPU-assisted validation on, whose teardown at the process's exit
/// aborts over a device leaked on purpose.
fn gpu_assisted() -> bool {
    std::env::var("VK_KHRONOS_VALIDATION_GPUAV_ENABLE").is_ok_and(|value| value == "true")
}

/// Submits an encoder with nothing in it to the graphics queue, held by a wait for `gate`.
fn held(device: &VulkanDevice, gate: TimelinePoint) -> [Box<dyn Encoder>; 1] {
    let encoder = device.create_encoder(QueueRole::Graphics, "held").expect("an encoder");
    let mut encoders = [encoder];
    device
        .submit(
            QueueRole::Graphics,
            Submit { encoders: &mut encoders, waits: &[gate], signals: &[] },
        )
        .expect("the submit, held by its wait");
    encoders
}

/// A last holder dropped while its thread unwinds, with a submit pending, leaks the device
/// whole and returns: nothing the leaked messengers name is freed, so the box their user
/// data points at keeps its count of the sink.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_drop_while_unwinding_with_a_submit_pending_keeps_the_messengers_sink() {
    if gpu_assisted() {
        eprintln!(
            "a_drop_while_unwinding_with_a_submit_pending_keeps_the_messengers_sink: skipped, \
             GPU-assisted validation aborts at exit over a leaked device"
        );
        return;
    }
    let sink = Arc::new(Counting::default());
    let unwound = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let device = device(&sink);
        let timeline = device.create_timeline(0).expect("a timeline");
        let _encoders = held(&device, TimelinePoint { timeline, value: 1 });
        assert_eq!(Arc::strong_count(&sink), 2, "the device's count of the sink");
        panic!("the test's panic, with a submit pending");
    }));
    assert!(unwound.is_err());
    assert_eq!(Arc::strong_count(&sink), 2, "the leaked messengers' sink was freed");
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// A last holder dropped while its thread unwinds, with nothing pending, tears the device
/// down as a drop that does not unwind does, its sampler included, which validation would
/// report left alive.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_drop_while_unwinding_with_nothing_pending_destroys_everything() {
    let sink = Arc::new(Counting::default());
    let unwound = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let device = device(&sink);
        let desc = SamplerDesc {
            magnify: Filter::Linear,
            minify: Filter::Linear,
            mip: Filter::Nearest,
            address_u: AddressMode::ClampToEdge,
            address_v: AddressMode::Repeat,
        };
        device.create_sampler(&desc).expect("a sampler");
        run(&device, lagging(&device));
        panic!("the test's panic, with nothing pending");
    }));
    assert!(unwound.is_err());
    assert_eq!(Arc::strong_count(&sink), 1, "the device's count of the sink was kept");
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// An encoder dropped while its thread unwinds, with its submit pending, hands its pools to
/// the device; the last holder, on another thread, destroys them once the device is idle.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn an_encoder_dropped_while_unwinding_hands_its_pools_to_the_last_holder() {
    let sink = Arc::new(Counting::default());
    let device = Arc::new(device(&sink));
    let timeline = device.create_timeline(0).expect("a timeline");
    let gate = TimelinePoint { timeline, value: 1 };
    let worker = Arc::clone(&device);
    let unwound = std::thread::spawn(move || {
        let _encoders = held(&worker, gate);
        panic!("the test's panic, with its encoder's submit held");
    })
    .join();
    assert!(unwound.is_err());
    device.signal_timeline(gate).expect("the gate's signal");
    drop(Arc::into_inner(device).expect("the last count of the device"));
    assert_eq!(Arc::strong_count(&sink), 1, "the device's count of the sink was kept");
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}
