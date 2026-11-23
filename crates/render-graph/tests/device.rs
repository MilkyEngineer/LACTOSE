//! The graph on a Vulkan device, by class of hazard (decision 5.17 of the milestone 2 plan):
//! the frames the null tests hold, run under the Khronos layer's core and synchronisation
//! validation and, under MoltenVK, Metal's API and shader validation, which `cargo xtask gpu`
//! sets and judges, each test in a process of its own. A dropped barrier is what the layer
//! reports; the tests here are the clean frames it reports on, and the builds that drop one
//! fail them by its message. A host access under pending work, a reset of a pending
//! encoder's pool and a frame point's lost wait are held with the GPU made to lag, and each
//! test that lags checks that the lag held: the point it lags is not reached when the test
//! looks. Where the device has one queue, a test of the several-queue path says it is
//! skipped and why.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lactose_render_graph::{
    BufferAccess, BufferImport, BufferUse, ComputePassDecl, CopyPassDecl, DepthUse, FrameDecl,
    GraphDesc, ImageImport, RenderGraph, RenderPassDecl, TransientImage, TransientMemory,
};
use lactose_rhi_api::{
    BufferCopy, BufferDesc, BufferHandle, BufferState, BufferTransition, BufferUsage,
    ColourAttachment, ColourTargetDesc, Compare, ComputePipelineDesc, ComputePipelineHandle,
    CookedShader, Cull, DepthDesc, Device, DeviceRequest, Encoder, Extent, Format,
    GraphicsPipelineDesc, GraphicsPipelineHandle, HEAP_SLOTS_DEFAULT, HeapSlot, ImageDesc,
    ImageHandle, ImageMemory, ImageRange, ImageState, ImageTransition, ImageUsage, Load,
    MemoryBlockDesc, MemoryClass, Message, MessageSink, PassReach, QueuePath, QueueRole,
    RenderPass, Rule, SAMPLERS_DEFAULT, Sharing, StageDesc, StateOn, Store, Submit, Target,
    TimelinePoint, WriteMask,
};
use lactose_rhi_vulkan::VulkanDevice;

use crate::common::{
    C, EXTENT, G, PATIENCE, WHOLE, bring_image, colour, draws, fresh_buffer, image, image_import,
    in_job, on, reached, reads, writes,
};

/// `store.slang`'s rounds for a lag of about a second on the M1, as rhi-vulkan's tests lag.
const LAG: u32 = 1 << 25;

/// The words a test writes and reads back.
const WORDS: u32 = 64;

/// The bytes of `WORDS` words.
const WORD_BYTES: u64 = WORDS as u64 * 4;

/// The bytes of one `EXTENT` image of `Rgba8Unorm`.
const PIXEL_BYTES: u64 = EXTENT.width as u64 * EXTENT.height as u64 * 4;

/// A sink that keeps what it is sent and prints each message in the form the run's judge
/// counts.
#[derive(Default)]
struct Kept(Mutex<Vec<String>>);

impl MessageSink for Kept {
    fn message(&self, message: &Message<'_>) {
        eprintln!("message: {message:?}");
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{message:?}"));
    }
}

/// A device on the adapter `LACTOSE_GPU_ADAPTER` names, or the loader's first, with the
/// default heap and sampler table and no surface.
fn vulkan(sink: &Arc<Kept>) -> Arc<dyn Device> {
    let request = DeviceRequest {
        adapter: std::env::var("LACTOSE_GPU_ADAPTER").ok(),
        surface: None,
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages: Arc::clone(sink) as _,
    };
    Arc::new(VulkanDevice::new(request).expect("a Vulkan device"))
}

/// Whether the device's compute role has a queue of its own.
fn own_compute(device: &dyn Device) -> bool {
    device.capabilities().queues.compute == QueuePath::Own
}

/// Says the test is skipped, in the form the run's judge reads.
fn skipped(test: &str, why: &str) {
    eprintln!("{test}: skipped, {why}");
}

/// Drops the last holder of the device, so that every message of its destroy is sent, and
/// fails the test on any message; then says it passed.
fn passed(test: &str, device: Arc<dyn Device>, sink: &Kept) {
    assert_eq!(Arc::strong_count(&device), 1, "the test holds the device's last reference");
    drop(device);
    let texts = sink.0.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(texts.is_empty(), "the layer or the driver spoke: {texts:?}");
    eprintln!("{test}: passed");
}

/// Whether `point` is not yet reached: the lag of the work that signals it held.
fn pending(device: &dyn Device, point: TimelinePoint) -> bool {
    device.timeline_value(point.timeline).expect("the timeline's value") < point.value
}

/// The conformance suite's cooked shader `<source>.<entry>`, which `cargo xtask gpu` cooks
/// into the directory `LACTOSE_GPU_SHADERS` names.
fn cooked(name: &str) -> CookedShader {
    let dir = std::env::var_os("LACTOSE_GPU_SHADERS").expect("LACTOSE_GPU_SHADERS is set");
    let path = std::path::Path::new(&dir).join(format!("conformance/{name}.spirv"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    CookedShader::decode(&bytes).expect("a cooked shader")
}

fn kernel(device: &dyn Device, source: &str) -> ComputePipelineHandle {
    let shader = cooked(&format!("{source}.cs_main"));
    let desc =
        ComputePipelineDesc { label: source, stage: StageDesc { shader: &shader, constants: &[] } };
    device.create_compute_pipeline(&desc).expect("a compute pipeline")
}

/// The pipeline of `<source>.vs_main` and `<source>.fs_main` into one `Rgba8Unorm` target.
fn drawing(device: &dyn Device, source: &str, depth: Option<DepthDesc>) -> GraphicsPipelineHandle {
    let vertex = cooked(&format!("{source}.vs_main"));
    let fragment = cooked(&format!("{source}.fs_main"));
    let colour = [ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::ALL }];
    let desc = GraphicsPipelineDesc {
        label: source,
        vertex: StageDesc { shader: &vertex, constants: &[] },
        fragment: Some(StageDesc { shader: &fragment, constants: &[] }),
        colour: &colour,
        depth,
        cull: Cull::None,
    };
    device.create_graphics_pipeline(&desc).expect("a graphics pipeline")
}

fn buffer(
    device: &dyn Device,
    label: &str,
    size: u64,
    usage: BufferUsage,
    memory: MemoryClass,
) -> BufferHandle {
    let desc = BufferDesc { label, size, usage, memory, sharing: Sharing::Exclusive };
    device.create_buffer(&desc).expect("a buffer")
}

/// A buffer of `WORDS` words a shader writes through its address, which a copy reads.
fn written(device: &dyn Device, label: &str) -> BufferHandle {
    let usage = BufferUsage::DEVICE_ADDRESS.union(BufferUsage::COPY_SOURCE);
    buffer(device, label, WORD_BYTES, usage, MemoryClass::Device)
}

/// A readback buffer of `size` bytes a copy fills.
fn readback(device: &dyn Device, label: &str, size: u64) -> BufferHandle {
    buffer(device, label, size, BufferUsage::COPY_DESTINATION, MemoryClass::Readback)
}

fn graph(device: &Arc<dyn Device>, jobs: u32) -> RenderGraph {
    RenderGraph::new(Arc::clone(device), &GraphDesc { jobs, plans: 4 }).expect("a graph")
}

/// Compiles, records and submits `decl`.
fn frame(
    graph: &mut RenderGraph,
    ctx: &mut lactose_task::JobContext<'_>,
    decl: FrameDecl<'_>,
) -> lactose_core::Result<TimelinePoint> {
    let compiled = graph.compile(decl)?;
    let recorded = graph.record(compiled, ctx)?;
    graph.submit(recorded)
}

/// Declares a compute pass that writes `count` words of `value + i` into `out` with
/// `store.slang`, after `lag` rounds.
fn store(
    decl: &mut FrameDecl<'_>,
    label: &str,
    pipeline: ComputePipelineHandle,
    out: lactose_render_graph::BufferId,
    (count, lag, value): (u32, u32, u32),
) -> lactose_render_graph::PassId {
    decl.compute_pass(
        &ComputePassDecl { label, sampled: &[], buffers: &writes(out), slots: &[] },
        move |pass| {
            pass.bind_pipeline(pipeline);
            pass.push_address(0, out, 0);
            pass.push_bytes(8, &count.to_le_bytes());
            pass.push_bytes(12, &lag.to_le_bytes());
            pass.push_bytes(16, &value.to_le_bytes());
            pass.dispatch([count.div_ceil(64), 1, 1]);
        },
    )
}

/// A buffer import from `initial` to `last`, each on its role.
fn moved<'a>(
    label: &'a str,
    buffer: BufferHandle,
    initial: StateOn<BufferState>,
    last: StateOn<BufferState>,
) -> BufferImport<'a> {
    BufferImport { label, buffer, sharing: Sharing::Exclusive, initial, last, ready: None }
}

/// The words of `buffer` from its start, once every frame that wrote it is reached.
fn read_words(device: &dyn Device, buffer: BufferHandle, words: u32) -> Vec<u32> {
    let len = u64::from(words) * 4;
    device.invalidate_buffer(buffer, 0, len).expect("the invalidate");
    let mut out = Vec::new();
    device
        .read_buffer(buffer, 0, len, &mut |bytes| {
            out.extend(bytes.as_chunks::<4>().0.iter().map(|word| u32::from_le_bytes(*word)));
        })
        .expect("the read");
    out
}

/// Writes `words` from the start of the `Upload` buffer `buffer`, and flushes them.
fn write_words(device: &dyn Device, buffer: BufferHandle, words: &[u32]) {
    let len = words.len() as u64 * 4;
    device
        .write_buffer(buffer, 0, len, &mut |bytes| {
            for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(words) {
                *chunk = word.to_le_bytes();
            }
        })
        .expect("the write");
    device.flush_buffer(buffer, 0, len).expect("the flush");
}

fn float_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

const MAGENTA: [f32; 4] = [1.0, 0.0, 1.0, 1.0];

/// Frames on the several-queue path: a compute pass writes words into one of two buffers,
/// which no graphics-role pass of its frame uses, so it is the compute role's where that
/// has a queue of its own; a render pass fills a target; on the graphics queue, a copy
/// stages the words the frame before wrote, which cross the queues at the frames' edges,
/// and a copy reads back them and the target. Four frames, each from where the ones before
/// left its imports, read back what each wrote. A build that drops the barrier between the
/// two copies or on the target fails it by the layer's message; the M1's four families of
/// one queue each give compute a family of its own (`rhi-vulkan`, "Queues"), and lavapipe's
/// one queue runs the same frames with every pass on the graphics queue.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn frames_on_both_queues_read_back_what_each_pass_wrote() {
    let test = "frames_on_both_queues_read_back_what_each_pass_wrote";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    let home = if own_compute(&*device) { C } else { G };
    let store_pipeline = kernel(&*device, "store");
    let fill = drawing(&*device, "fill", None);
    let words = [written(&*device, "words 0"), written(&*device, "words 1")];
    let target = image(&*device, "target", Sharing::Exclusive);
    let copied = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
    let middle = buffer(&*device, "middle", WORD_BYTES, copied, MemoryClass::Device);
    let words_back = readback(&*device, "words back", WORD_BYTES);
    let pixels_back = readback(&*device, "pixels back", PIXEL_BYTES);
    let shared = Arc::clone(&device);
    in_job(move |ctx| {
        let mut graph = graph(&shared, 2);
        for at in 0..4u32 {
            let (writing, reading) = (words[at as usize % 2], words[(at as usize + 1) % 2]);
            let mut decl = FrameDecl::new(1);
            let back = |state| if at == 0 { on(BufferState::Undefined, G) } else { on(state, G) };
            // Each buffer is written in one frame and read back on graphics in the next.
            let written = decl.import_buffer(&moved(
                "writing",
                writing,
                if at < 2 {
                    on(BufferState::Undefined, home)
                } else {
                    on(BufferState::CopySource, G)
                },
                on(BufferState::ShaderWrite, home),
            ));
            let read = (at > 0).then(|| {
                decl.import_buffer(&moved(
                    "reading",
                    reading,
                    on(BufferState::ShaderWrite, home),
                    on(BufferState::CopySource, G),
                ))
            });
            let words_out = decl.import_buffer(&moved(
                "words back",
                words_back,
                back(BufferState::HostRead),
                on(BufferState::HostRead, G),
            ));
            let pixels_out = decl.import_buffer(&moved(
                "pixels back",
                pixels_back,
                back(BufferState::HostRead),
                on(BufferState::HostRead, G),
            ));
            let drawn = decl.import_image(&image_import(
                "target",
                target,
                on(if at == 0 { ImageState::Undefined } else { ImageState::CopySource }, G),
                on(ImageState::CopySource, G),
            ));
            let write = store(&mut decl, "write", store_pipeline, written, (WORDS, 0, 100 * at));
            decl.render_pass(
                &draws("fill", &[colour(drawn, Load::Clear([0.0; 4]))]),
                move |pass| {
                    pass.bind_pipeline(fill);
                    pass.push_bytes(0, &float_bytes(&MAGENTA));
                    pass.draw(0..3, 0..1);
                },
            );
            // The words go through `middle` by two copies, the barrier between which the
            // layer sees: it tracks no access through a buffer's address, so not the store's.
            let all = BufferCopy { source_offset: 0, destination_offset: 0, size: WORD_BYTES };
            let staged = read.map(|read| {
                let staged = decl.import_buffer(&moved(
                    "middle",
                    middle,
                    on(BufferState::Undefined, G),
                    on(BufferState::CopySource, G),
                ));
                decl.copy_pass(
                    &CopyPassDecl {
                        label: "stage",
                        image_sources: &[],
                        image_destinations: &[],
                        buffer_sources: &[read],
                        buffer_destinations: &[staged],
                    },
                    move |pass| pass.copy_buffer(read, staged, &[all]),
                );
                staged
            });
            let sources: Vec<_> = staged.into_iter().collect();
            decl.copy_pass(
                &CopyPassDecl {
                    label: "read back",
                    image_sources: &[drawn],
                    image_destinations: &[],
                    buffer_sources: &sources,
                    buffer_destinations: &[words_out, pixels_out],
                },
                move |pass| {
                    if let Some(staged) = staged {
                        pass.copy_buffer(staged, words_out, &[all]);
                    }
                    pass.copy_image_to_buffer(drawn, pixels_out, &[WHOLE]);
                },
            );
            let compiled = graph.compile(decl).expect("the frame compiles");
            let planned = compiled.plan().passes().iter().find(|planned| planned.pass == write);
            assert_eq!(planned.expect("the write is kept").queue, home, "the write's queue");
            let recorded = graph.record(compiled, ctx).expect("the frame records");
            let point = graph.submit(recorded).expect("the frame submits");
            reached(&**graph.device(), point);
            let device = &**graph.device();
            if at > 0 {
                let expected: Vec<u32> = (0..WORDS).map(|i| 100 * (at - 1) + i).collect();
                assert_eq!(read_words(device, words_back, WORDS), expected, "frame {at}");
            }
            let pixels = read_words(device, pixels_back, (PIXEL_BYTES / 4) as u32);
            let magenta = u32::from_le_bytes([255, 0, 255, 255]);
            assert!(pixels.iter().all(|&pixel| pixel == magenta), "frame {at}: the fill");
        }
    });
    passed(test, device, &sink);
}

/// The five chains of two transitions a frame can give one resource at its end (the null
/// tests' `transitions::no_transition_call_names_a_resource_twice`), each a frame of its
/// own, clean under the layer. Each chain is two transition calls in order; a build that
/// puts both in one call is refused by every backend (`state`, rhi-api's "States"). A build
/// that drops a chain's closing transition is not heard here, since nothing in the frame
/// uses the resource after it: null's state rule fails that one.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn the_five_chains_of_two_transitions_are_clean() {
    let test = "the_five_chains_of_two_transitions_are_clean";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    let shared = Arc::clone(&device);
    in_job(move |ctx| {
        let device = &*shared;
        let present = QueueRole::Present;
        let sampled = on(ImageState::SampledRead, G);
        for chain in 0..5 {
            let target = image(device, "target", Sharing::Exclusive);
            let data = written(device, "data");
            let out = written(device, "out");
            if chain == 2 {
                bring_image(device, target, sampled);
            }
            let mut graph = graph(&shared, 1);
            let mut decl = FrameDecl::new(1);
            let handed = |initial, last| ImageImport {
                handoff: Some(on(ImageState::SampledRead, present)),
                ..image_import("target", target, initial, last)
            };
            let data_import =
                moved("data", data, on(BufferState::Undefined, G), on(BufferState::ShaderRead, C));
            match chain {
                // A target drawn, closed into `SampledRead`, then released toward present.
                0 => {
                    let image = decl.import_image(&handed(on(ImageState::Undefined, G), sampled));
                    decl.render_pass(
                        &draws("draw", &[colour(image, Load::Clear([0.0; 4]))]),
                        |_| {},
                    );
                }
                // A target no pass uses, taken from `Undefined` into its `last`, released.
                1 => {
                    decl.import_image(&handed(
                        on(ImageState::Undefined, G),
                        on(ImageState::ColourAttachment, G),
                    ));
                }
                // An image sampled on the compute queue, acquired back, then released.
                2 => {
                    let image = decl.import_image(&handed(sampled, sampled));
                    let out =
                        decl.import_buffer(&fresh_buffer("out", out, C, BufferState::ShaderWrite));
                    decl.compute_pass(
                        &ComputePassDecl {
                            label: "sample",
                            sampled: &[image],
                            buffers: &writes(out),
                            slots: &[],
                        },
                        |_| {},
                    );
                }
                // A buffer imported `Undefined` on graphics and read on compute.
                3 => {
                    let data = decl.import_buffer(&data_import);
                    decl.compute_pass(
                        &ComputePassDecl {
                            label: "read",
                            sampled: &[],
                            buffers: &reads(data),
                            slots: &[],
                        },
                        |_| {},
                    );
                }
                // The same buffer, which no pass uses, left on compute.
                _ => {
                    decl.import_buffer(&data_import);
                }
            }
            let point = frame(&mut graph, ctx, decl).expect("the chain's frame");
            reached(device, point);
        }
    });
    passed(test, device, &sink);
}

/// A host write under a pending reader: a frame copies an upload buffer behind a lagging
/// pass, and the host writes the buffer while the frame is pending. The copy reads the
/// host's later write, a wrong readback, which is the hazard; a caller that waits for the
/// frame's point before it writes reads back what the frame was submitted over. Neither is
/// refused, since the device reads the buffer and does not write it (rhi-api's
/// `write_buffer`), so the graph's point is the caller's one guard.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_host_write_under_a_pending_reader_is_read_and_one_after_the_frame_point_is_not() {
    let test = "a_host_write_under_a_pending_reader_is_read_and_one_after_the_frame_point_is_not";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    let store_pipeline = kernel(&*device, "store");
    let upload = buffer(&*device, "upload", 16, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let lagged = written(&*device, "lagged");
    let upload_back = readback(&*device, "upload back", 16);
    let lagged_back = readback(&*device, "lagged back", 4);
    write_words(&*device, upload, &[1, 2, 3, 4]);
    let shared = Arc::clone(&device);
    in_job(move |ctx| {
        let mut graph = graph(&shared, 1);
        let mut points = Vec::new();
        for at in 0..2 {
            let first = at == 0;
            let state = |state| if first { on(BufferState::Undefined, G) } else { on(state, G) };
            let mut decl = FrameDecl::new(1);
            let source = decl.import_buffer(&moved(
                "upload",
                upload,
                state(BufferState::CopySource),
                on(BufferState::CopySource, G),
            ));
            let lag = decl.import_buffer(&moved(
                "lagged",
                lagged,
                state(BufferState::CopySource),
                on(BufferState::CopySource, G),
            ));
            let outs = [upload_back, lagged_back].map(|out| {
                decl.import_buffer(&moved(
                    "back",
                    out,
                    state(BufferState::HostRead),
                    on(BufferState::HostRead, G),
                ))
            });
            store(&mut decl, "lag", store_pipeline, lag, (1, LAG, 7));
            decl.copy_pass(
                &CopyPassDecl {
                    label: "read",
                    image_sources: &[],
                    image_destinations: &[],
                    buffer_sources: &[source, lag],
                    buffer_destinations: &outs,
                },
                move |pass| {
                    let copy = |size| BufferCopy { source_offset: 0, destination_offset: 0, size };
                    pass.copy_buffer(source, outs[0], &[copy(16)]);
                    pass.copy_buffer(lag, outs[1], &[copy(4)]);
                },
            );
            let point = frame(&mut graph, ctx, decl).expect("the frame");
            let device = &**graph.device();
            assert!(pending(device, point), "the lag held: frame {at} is pending at its submit");
            if first {
                write_words(device, upload, &[9, 9, 9, 9]);
                assert!(pending(device, point), "the lag held: the write is under the reader");
            } else {
                reached(device, point);
                write_words(device, upload, &[5, 5, 5, 5]);
            }
            reached(device, point);
            points.push(point);
            let got = read_words(device, upload_back, 4);
            let want = [9, 9, 9, 9];
            assert_eq!(got, want, "frame {at}: the copy read the write made before it ran");
            assert_eq!(read_words(device, lagged_back, 1), [7], "frame {at}: the lag ran");
        }
        assert!(points[1].value > points[0].value, "the frame point grows");
    });
    passed(test, device, &sink);
}

/// A host access under a pending writer is refused on every backend (`pending`): a frame
/// writes a readback buffer with a lagging pass, and the host's read of it while the frame
/// is pending is refused; after the frame's point it reads what the pass wrote.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_host_read_under_a_pending_writer_is_refused() {
    let test = "a_host_read_under_a_pending_writer_is_refused";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    let home = if own_compute(&*device) { C } else { G };
    let store_pipeline = kernel(&*device, "store");
    let usage = BufferUsage::DEVICE_ADDRESS;
    let out = buffer(&*device, "out", 4, usage, MemoryClass::Readback);
    let shared = Arc::clone(&device);
    in_job(move |ctx| {
        let mut graph = graph(&shared, 1);
        let mut decl = FrameDecl::new(1);
        // Left in `HostRead` on the writer's queue: a transfer into `HostRead` across families
        // is a barrier with the host stage between two families, which Vulkan forbids
        // (`VUID-VkBufferMemoryBarrier2-srcStageMask-03851`).
        let id = decl.import_buffer(&moved(
            "out",
            out,
            on(BufferState::Undefined, home),
            on(BufferState::HostRead, home),
        ));
        store(&mut decl, "lag", store_pipeline, id, (1, LAG, 7));
        let point = frame(&mut graph, ctx, decl).expect("the frame");
        let device = &**graph.device();
        assert!(pending(device, point), "the lag held: the frame is pending at its submit");
        let refused = device.read_buffer(out, 0, 4, &mut |_| {}).expect_err("a read under it");
        assert_eq!(Rule::of(&refused), Some(Rule::Pending), "{refused}");
        let refused = device.invalidate_buffer(out, 0, 4).expect_err("an invalidate under it");
        assert_eq!(Rule::of(&refused), Some(Rule::Pending), "{refused}");
        assert!(pending(device, point), "the lag held: the refusals were under the writer");
        reached(device, point);
        assert_eq!(read_words(device, out, 1), [7], "the pass's write, after the point");
    });
    passed(test, device, &sink);
}

/// The command pools of a frame are reset only after its point: a lagging frame, then two
/// frames submitted while it is pending, each with an encoder of its own, so the graph
/// holds three and resets none (a reset of a pending encoder is refused by `rhi-vulkan`,
/// which the graph takes for its own breach, and is
/// `VUID-vkResetCommandPool-commandPool-00040` under the layer). Once the last point is
/// reached, the next frames reset and reuse them and the graph makes no more.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_frames_encoders_are_reset_only_after_its_point() {
    let test = "a_frames_encoders_are_reset_only_after_its_point";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    let home = if own_compute(&*device) { C } else { G };
    let store_pipeline = kernel(&*device, "store");
    let lagged = written(&*device, "lagged");
    let target = image(&*device, "target", Sharing::Exclusive);
    let shared = Arc::clone(&device);
    in_job(move |ctx| {
        let mut graph = graph(&shared, 1);
        let lagging = |lag| {
            let mut decl = FrameDecl::new(1);
            let id =
                decl.import_buffer(&fresh_buffer("lagged", lagged, home, BufferState::ShaderWrite));
            store(&mut decl, "lag", store_pipeline, id, (1, lag, 7));
            decl
        };
        let clear = || {
            let mut decl = FrameDecl::new(1);
            let drawn = decl.import_image(&image_import(
                "target",
                target,
                on(ImageState::Undefined, G),
                on(ImageState::ColourAttachment, G),
            ));
            decl.render_pass(&draws("clear", &[colour(drawn, Load::Clear([0.0; 4]))]), |_| {});
            decl
        };
        let first = frame(&mut graph, ctx, lagging(LAG)).expect("the lagging frame");
        let mut last = first;
        for _ in 0..2 {
            last = frame(&mut graph, ctx, clear()).expect("a frame behind it");
        }
        let device = &**graph.device();
        assert!(pending(device, first), "the lag held: the lagging frame is pending");
        assert_eq!(graph.stats().encoders, 3, "every frame's encoder held, none freed");
        reached(device, last);
        assert!(!pending(device, first), "the lagging frame's point, before the last");
        let again = frame(&mut graph, ctx, clear()).expect("a frame after");
        let after = frame(&mut graph, ctx, lagging(0)).expect("a lagging frame after");
        assert_eq!(graph.stats().encoders, 3, "the reached frames' encoders reused");
        reached(&**graph.device(), again);
        reached(&**graph.device(), after);
    });
    passed(test, device, &sink);
}

/// A frame of one compute submit, lagging, then an all-graphics frame submitted while it is
/// pending (G5): the second frame's submit waits for the frame point before itself, since
/// the signals of the first frame's one submit are unordered with one another on Vulkan
/// (`VUID-VkSubmitInfo2-semaphore-03882`). `rhi-vulkan` refuses a signal beside another
/// queue's pending signal of the timeline that no wait orders (`timeline`), so a build
/// without that wait fails here as on null; under the layer the pair is clean, and the
/// frame timeline's values grow.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_frame_after_one_that_ended_on_the_compute_queue_is_ordered_after_its_point() {
    let test = "a_frame_after_one_that_ended_on_the_compute_queue_is_ordered_after_its_point";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    if !own_compute(&*device) {
        skipped(test, "compute shares the graphics queue");
        return;
    }
    let store_pipeline = kernel(&*device, "store");
    let lagged = written(&*device, "lagged");
    let target = image(&*device, "target", Sharing::Exclusive);
    let shared = Arc::clone(&device);
    in_job(move |ctx| {
        let mut graph = graph(&shared, 1);
        let mut decl = FrameDecl::new(1);
        let id = decl.import_buffer(&fresh_buffer("lagged", lagged, C, BufferState::ShaderWrite));
        store(&mut decl, "lag", store_pipeline, id, (1, LAG, 7));
        let first = frame(&mut graph, ctx, decl).expect("the compute frame");
        let mut decl = FrameDecl::new(1);
        let drawn = decl.import_image(&image_import(
            "target",
            target,
            on(ImageState::Undefined, G),
            on(ImageState::ColourAttachment, G),
        ));
        decl.render_pass(&draws("clear", &[colour(drawn, Load::Clear([0.0; 4]))]), |_| {});
        let second = frame(&mut graph, ctx, decl).expect("the graphics frame");
        let device = &**graph.device();
        assert!(pending(device, first), "the lag held: the second frame came under the first");
        assert_eq!(second.timeline, first.timeline, "one frame timeline");
        assert_eq!(second.value, first.value + 1, "the frame timeline's values grow");
        reached(device, second);
        assert!(!pending(device, first), "the first frame's point, before the second's");
    });
    passed(test, device, &sink);
}

/// A torn frame on the several-queue path: a lagging compute frame, then a frame
/// `[draw, compute, draw]` whose compute submit `rhi-vulkan` refuses for a slot its pass did
/// not declare (`undeclared`), torn with its first graphics submit made while the first
/// frame is pending. The next frame makes the torn frame's last submit in its place: the
/// frame timeline's values grow by one a frame, the torn frame's point is reached, and the
/// torn frame's encoders are held until it, as the layer and `rhi-vulkan`'s refusal of a
/// pending reset check.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_torn_frame_is_finished_by_the_next_and_its_point_is_reached() {
    let test = "a_torn_frame_is_finished_by_the_next_and_its_point_is_reached";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    if !own_compute(&*device) {
        skipped(test, "compute shares the graphics queue, and one queue never tears a frame");
        return;
    }
    let store_pipeline = kernel(&*device, "store");
    let lagged = written(&*device, "lagged");
    let out = written(&*device, "out");
    let targets =
        [image(&*device, "a", Sharing::Exclusive), image(&*device, "b", Sharing::Exclusive)];
    let shared = Arc::clone(&device);
    in_job(move |ctx| {
        let mut graph = graph(&shared, 1);
        let mut decl = FrameDecl::new(1);
        let id = decl.import_buffer(&fresh_buffer("lagged", lagged, C, BufferState::ShaderWrite));
        store(&mut decl, "lag", store_pipeline, id, (1, LAG, 7));
        let first = frame(&mut graph, ctx, decl).expect("the lagging frame");
        let declare = |breach: bool| {
            let mut decl = FrameDecl::new(1);
            let draw = |decl: &mut FrameDecl<'_>, label: &'static str, at: usize| {
                let target = decl.import_image(&image_import(
                    label,
                    targets[at],
                    on(ImageState::Undefined, G),
                    on(ImageState::ColourAttachment, G),
                ));
                decl.render_pass(&draws(label, &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
            };
            draw(&mut decl, "a", 0);
            let id = decl.import_buffer(&fresh_buffer("out", out, C, BufferState::ShaderWrite));
            decl.compute_pass(
                &ComputePassDecl { label: "write", sampled: &[], buffers: &writes(id), slots: &[] },
                move |pass| {
                    pass.bind_pipeline(store_pipeline);
                    pass.push_address(0, id, 0);
                    pass.push_bytes(8, &1u32.to_le_bytes());
                    pass.push_bytes(12, &0u32.to_le_bytes());
                    pass.push_bytes(16, &7u32.to_le_bytes());
                    if breach {
                        pass.push_slot(20, HeapSlot(7));
                    }
                    pass.dispatch([1, 1, 1]);
                },
            );
            draw(&mut decl, "b", 1);
            decl
        };
        let refused = frame(&mut graph, ctx, declare(true)).expect_err("the slot is refused");
        assert_eq!(Rule::of(&refused), Some(Rule::Undeclared), "{refused}");
        let device = &**graph.device();
        assert!(pending(device, first), "the lag held: the frame tore under the first");
        let torn = TimelinePoint { timeline: first.timeline, value: first.value + 1 };
        // The lagging frame's encoder and the torn frame's made one, pending, and the two of
        // its submits never made, free.
        let held = graph.stats().encoders;
        let third = frame(&mut graph, ctx, declare(false)).expect("the frame after");
        assert!(pending(&**graph.device(), first), "the lag held: no encoder was reclaimed");
        assert_eq!(graph.stats().encoders, held + 1, "the two free ones taken, one more made");
        assert_eq!(third.timeline, first.timeline, "one frame timeline");
        assert_eq!(third.value, first.value + 2, "the torn frame's point, then the next's");
        reached(&**graph.device(), torn);
        reached(&**graph.device(), third);
        let fourth = frame(&mut graph, ctx, declare(false)).expect("a frame after the torn one");
        assert_eq!(graph.stats().encoders, held + 1, "reused once the torn point is reached");
        reached(&**graph.device(), fourth);
    });
    passed(test, device, &sink);
}

/// What the waits across the queues cost, on the M1's several queues: one queue's work of a
/// frame does not overlap the other's, within the frame (its last submit waits for the
/// other queue's) or across its end (each submit waits for the other queue's last submit of
/// the frame before). The figures are wall time on the host from the first submit to the
/// last point reached, with each queue's work a lagging pass and two frames in flight, set
/// against the same work on two graphs, one for each queue, which wait for nothing across
/// queues; the test prints them, as `measured:` lines, and asserts nothing of them. rhi-api
/// has no timestamp query, so they are not GPU timestamps, and on the M1 they are
/// MoltenVK's figures, not native Metal's.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn the_waits_across_the_queues_are_measured() {
    const FRAMES: u32 = 6;
    const COST_LAG: u32 = 1 << 23;
    let test = "the_waits_across_the_queues_are_measured";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    if !own_compute(&*device) {
        skipped(test, "compute shares the graphics queue, so no wait crosses queues");
        return;
    }
    let store_pipeline = kernel(&*device, "store");
    let on_compute = written(&*device, "on compute");
    let on_graphics = written(&*device, "on graphics");
    let target = image(&*device, "target", Sharing::Exclusive);
    let shared = Arc::clone(&device);
    in_job(move |ctx| {
        // `apart` runs each queue's work on a graph of its own, which has used no other
        // queue and so makes no wait across queues: what the device overlaps unwaited.
        let (mut computing, mut drawing) = (graph(&shared, 1), graph(&shared, 1));
        let mut graph = graph(&shared, 1);
        // `compute`: a lagging pass with no graphics dependency, the compute role's.
        // `graphics`: a lagging pass that writes a buffer a render pass reads, so it is the
        // graphics role's.
        let declare = |compute: bool, graphics: bool| {
            let mut decl = FrameDecl::new(1);
            if compute {
                let id = decl.import_buffer(&fresh_buffer(
                    "on compute",
                    on_compute,
                    C,
                    BufferState::ShaderWrite,
                ));
                store(&mut decl, "compute lag", store_pipeline, id, (1, COST_LAG, 7));
            }
            if graphics {
                let id = decl.import_buffer(&fresh_buffer(
                    "on graphics",
                    on_graphics,
                    G,
                    BufferState::ShaderRead,
                ));
                store(&mut decl, "graphics lag", store_pipeline, id, (1, COST_LAG, 7));
                let drawn = decl.import_image(&image_import(
                    "target",
                    target,
                    on(ImageState::Undefined, G),
                    on(ImageState::ColourAttachment, G),
                ));
                decl.render_pass(
                    &RenderPassDecl {
                        label: "read",
                        colour: &[colour(drawn, Load::Clear([0.0; 4]))],
                        depth: None,
                        sampled: &[],
                        buffers: &[BufferUse { buffer: id, access: BufferAccess::Read }],
                        slots: &[],
                    },
                    |_| {},
                );
            }
            decl
        };
        // Each step submits one frame of each graph it names, then reaches the step before
        // last's points: two frames in flight on each graph.
        let timed = |ctx: &mut lactose_task::JobContext<'_>,
                     graphs: &mut [(&mut RenderGraph, bool, bool)],
                     frames: u32|
         -> Duration {
            let start = Instant::now();
            let mut steps: Vec<Vec<TimelinePoint>> = Vec::new();
            for at in 0..frames as usize {
                let mut points = Vec::new();
                for (graph, compute, graphics) in graphs.iter_mut() {
                    let point = frame(graph, ctx, declare(*compute, *graphics)).expect("a frame");
                    if at == 0 {
                        assert!(pending(&**graph.device(), point), "the lag held at the first");
                    }
                    points.push(point);
                }
                steps.push(points);
                if at >= 1 {
                    for &point in &steps[at - 1] {
                        reached(&*shared, point);
                    }
                }
            }
            for &point in steps.last().expect("a step") {
                reached(&*shared, point);
            }
            start.elapsed()
        };
        // One frame of each first, so that every plan is compiled before what is timed.
        timed(ctx, &mut [(&mut graph, true, true)], 1);
        timed(ctx, &mut [(&mut computing, true, false), (&mut drawing, false, true)], 1);
        let compute = timed(ctx, &mut [(&mut computing, true, false)], FRAMES);
        let graphics = timed(ctx, &mut [(&mut drawing, false, true)], FRAMES);
        let apart =
            timed(ctx, &mut [(&mut computing, true, false), (&mut drawing, false, true)], FRAMES);
        let both = timed(ctx, &mut [(&mut graph, true, true)], FRAMES);
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        println!(
            "measured: {FRAMES} frames, two in flight: compute alone {:.1} ms, graphics alone \
             {:.1} ms, both on two graphs with no wait across queues {:.1} ms, both on one graph \
             {:.1} ms; serialised would be {:.1} ms",
            ms(compute),
            ms(graphics),
            ms(apart),
            ms(both),
            ms(compute + graphics),
        );
        println!(
            "measured: one graph's frames took {:.2} of two graphs' unwaited time",
            both.as_secs_f64() / apart.as_secs_f64()
        );
    });
    passed(test, device, &sink);
}

/// Whether pixel `(x, y)`'s centre lies inside the triangle `corners`, given in clip space
/// over the whole `EXTENT` with the image's top at `y = 1`: `Some(inside)` more than half a
/// pixel from every edge's line, `None` nearer, where the rasteriser's rule decides
/// (rhi-api's conformance suite's own reference).
fn covers(corners: [[f32; 2]; 3], x: u32, y: u32) -> Option<bool> {
    let (w, h) = (EXTENT.width as f32, EXTENT.height as f32);
    let point = [(x as f32 + 0.5) / w * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / h * 2.0];
    let half = 1.0 / w;
    let mut inside = true;
    for at in 0..3 {
        let (a, b) = (corners[at], corners[(at + 1) % 3]);
        let edge = [b[0] - a[0], b[1] - a[1]];
        let cross = edge[0] * (point[1] - a[1]) - edge[1] * (point[0] - a[0]);
        let distance = cross / (edge[0] * edge[0] + edge[1] * edge[1]).sqrt();
        if distance.abs() < half {
            return None;
        }
        inside &= distance > 0.0;
    }
    Some(inside)
}

fn positions(corners: &[[f32; 2]], z: f32) -> Vec<f32> {
    corners.iter().flat_map(|corner| [corner[0], corner[1], z, 1.0]).collect()
}

/// A depth transient of one pass: memoryless where the device reports memoryless depth
/// honoured (MoltenVK on the M1), and otherwise placed in the plan's block where the device
/// places images (lavapipe, KosmicKrisp) or given memory of its own. A near triangle drawn
/// first keeps a far one out wherever it covers, against a CPU reference of both, so a
/// depth attachment that held nothing would fail it.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_depth_transient_of_one_pass_orders_two_triangles() {
    let test = "a_depth_transient_of_one_pass_orders_two_triangles";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    let depth = DepthDesc { format: Format::Depth32Float, compare: Compare::Less, write: true };
    let pipeline = drawing(&*device, "triangles", Some(depth));
    let near = [[-0.5, -0.5], [0.5, -0.5], [0.0, 0.5]];
    let far = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
    let mut floats = positions(&near, 0.25);
    floats.extend(positions(&far, 0.75));
    let usage = BufferUsage::DEVICE_ADDRESS;
    let source = buffer(&*device, "positions", 96, usage, MemoryClass::Upload);
    let bytes = float_bytes(&floats);
    device
        .write_buffer(source, 0, 96, &mut |lent| lent.copy_from_slice(&bytes))
        .expect("the write");
    device.flush_buffer(source, 0, 96).expect("the flush");
    let target = image(&*device, "target", Sharing::Exclusive);
    let pixels_back = readback(&*device, "pixels back", PIXEL_BYTES);
    let caps = device.capabilities();
    let depth_desc = ImageDesc {
        label: "depth",
        format: Format::Depth32Float,
        extent: EXTENT,
        mips: 1,
        layers: 1,
        usage: ImageUsage::DEPTH,
        memory: ImageMemory::Own,
        sharing: Sharing::Exclusive,
    };
    let places = device.image_placement(&depth_desc).is_ok();
    let shared = Arc::clone(&device);
    let memory = in_job(move |ctx| {
        let mut graph = graph(&shared, 1);
        let mut decl = FrameDecl::new(1);
        let positions = decl.import_buffer(&moved(
            "positions",
            source,
            on(BufferState::Undefined, G),
            on(BufferState::ShaderRead, G),
        ));
        let drawn = decl.import_image(&image_import(
            "target",
            target,
            on(ImageState::Undefined, G),
            on(ImageState::CopySource, G),
        ));
        let out = decl.import_buffer(&moved(
            "pixels back",
            pixels_back,
            on(BufferState::Undefined, G),
            on(BufferState::HostRead, G),
        ));
        let depth = decl.transient_image(&TransientImage {
            label: "depth",
            format: Format::Depth32Float,
            extent: EXTENT,
            mips: 1,
            layers: 1,
        });
        decl.render_pass(
            &RenderPassDecl {
                label: "triangles",
                colour: &[colour(drawn, Load::Clear([0.0; 4]))],
                depth: Some(DepthUse { image: depth, mip: 0, layer: 0, load: Load::Clear(1.0) }),
                sampled: &[],
                buffers: &reads(positions),
                slots: &[],
            },
            move |pass| {
                pass.bind_pipeline(pipeline);
                pass.push_address(0, positions, 0);
                pass.push_bytes(16, &float_bytes(&[1.0, 0.0, 0.0, 1.0]));
                pass.draw(0..3, 0..1);
                pass.push_bytes(16, &float_bytes(&[0.0, 1.0, 0.0, 1.0]));
                pass.draw(3..6, 0..1);
            },
        );
        decl.copy_pass(
            &CopyPassDecl {
                label: "read back",
                image_sources: &[drawn],
                image_destinations: &[],
                buffer_sources: &[],
                buffer_destinations: &[out],
            },
            move |pass| pass.copy_image_to_buffer(drawn, out, &[WHOLE]),
        );
        let compiled = graph.compile(decl).expect("the frame compiles");
        let memory = compiled.plan().transient(depth).expect("the depth transient is made");
        let recorded = graph.record(compiled, ctx).expect("the frame records");
        let point = graph.submit(recorded).expect("the frame submits");
        reached(&**graph.device(), point);
        memory
    });
    eprintln!("{test}: the depth transient's memory: {memory:?}");
    match memory {
        TransientMemory::Memoryless => assert!(caps.memoryless_depth, "memoryless if honoured"),
        TransientMemory::Placed { .. } => assert!(!caps.memoryless_depth && places, "{memory:?}"),
        TransientMemory::Own => assert!(!caps.memoryless_depth && !places, "{memory:?}"),
    }
    // Milestone 2's frames declare one transient, this depth at the view's extent, so where
    // it is placed the plan's one block is its placement: recorded at 3840 by 2160, and made,
    // against the 2^30 bytes below which no device's largest allocation lies.
    let largest = ImageDesc { extent: Extent { width: 3840, height: 2160 }, ..depth_desc };
    if let Ok(placement) = device.image_placement(&largest) {
        let desc = MemoryBlockDesc { label: "largest", size: placement.size };
        let block = device.create_memory_block(&desc).expect("the largest plan's block");
        device.destroy_memory_block(block).expect("the block's destroy");
        println!(
            "measured: a 3840 by 2160 depth transient's block is {} bytes, made; memoryless \
             depth honoured: {}",
            placement.size, caps.memoryless_depth
        );
    }
    let pixels = read_words(&*device, pixels_back, (PIXEL_BYTES / 4) as u32);
    let (red, green) = (u32::from_le_bytes([255, 0, 0, 255]), u32::from_le_bytes([0, 255, 0, 255]));
    let mut compared = 0;
    for y in 0..EXTENT.height {
        for x in 0..EXTENT.width {
            let Some(inside) = covers(near, x, y) else { continue };
            let got = pixels[(y * EXTENT.width + x) as usize];
            let want = if inside { red } else { green };
            assert_eq!(got.to_le_bytes(), want.to_le_bytes(), "pixel ({x}, {y})");
            compared += 1;
        }
    }
    assert!(compared > 3000, "most pixels are compared: {compared}");
    passed(test, device, &sink);
}

/// Whether two images placed at one offset of one memory block share bytes on this device:
/// one is cleared and copied out, the other then begins by the alias call and is copied out
/// before any write. The second copy reads the first image's clear only where the two
/// share memory. Under MoltenVK's defaults a placed image may be given memory of its own,
/// which makes no aliasing result on the M1 evidence of aliasing; the test prints which
/// holds, as a `measured:` line, and asserts nothing of it.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn whether_placed_images_share_bytes_is_measured() {
    let test = "whether_placed_images_share_bytes_is_measured";
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    let desc = |label| ImageDesc {
        label,
        format: Format::Rgba8Unorm,
        extent: EXTENT,
        mips: 1,
        layers: 1,
        usage: ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE),
        memory: ImageMemory::Own,
        sharing: Sharing::Exclusive,
    };
    let Ok(placement) = device.image_placement(&desc("first")) else {
        skipped(test, "the device places no image");
        return;
    };
    let block = device
        .create_memory_block(&MemoryBlockDesc { label: "block", size: placement.size })
        .expect("a block");
    let placed = |label| {
        let desc = ImageDesc { memory: ImageMemory::Placed { block, offset: 0 }, ..desc(label) };
        device.create_image(&desc).expect("a placed image")
    };
    let images = [placed("first"), placed("second")];
    let outs = [
        readback(&*device, "first back", PIXEL_BYTES),
        readback(&*device, "second back", PIXEL_BYTES),
    ];
    let mut encoder = device.create_encoder(G, "alias probe").expect("an encoder");
    let image_to = |image, from, to| ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: on(from, G),
        to: on(to, G),
    };
    let copy_out = |encoder: &mut dyn Encoder, image: ImageHandle, out: BufferHandle| {
        encoder.transition_buffers(&[BufferTransition {
            buffer: out,
            from: on(BufferState::Undefined, G),
            to: on(BufferState::CopyDestination, G),
        }]);
        encoder.copy_image_to_buffer(image, out, &[WHOLE]);
        encoder.transition_buffers(&[BufferTransition {
            buffer: out,
            from: on(BufferState::CopyDestination, G),
            to: on(BufferState::HostRead, G),
        }]);
    };
    encoder.alias(images[0]);
    encoder.transition_images(&[image_to(
        images[0],
        ImageState::Undefined,
        ImageState::ColourAttachment,
    )]);
    encoder.begin_render_pass(&RenderPass {
        label: "clear",
        colour: &[ColourAttachment {
            target: Target { image: images[0], mip: 0, layer: 0 },
            load: Load::Clear(MAGENTA),
            store: Store::Store,
            local_read: false,
        }],
        depth: None,
        reach: PassReach::NOTHING,
    });
    encoder.end_pass();
    encoder.transition_images(&[image_to(
        images[0],
        ImageState::ColourAttachment,
        ImageState::CopySource,
    )]);
    copy_out(&mut *encoder, images[0], outs[0]);
    encoder.alias(images[1]);
    encoder.transition_images(&[image_to(
        images[1],
        ImageState::Undefined,
        ImageState::CopySource,
    )]);
    copy_out(&mut *encoder, images[1], outs[1]);
    let timeline = device.create_timeline(0).expect("a timeline");
    let done = TimelinePoint { timeline, value: 1 };
    let mut encoders = [encoder];
    device
        .submit(G, Submit { encoders: &mut encoders, waits: &[], signals: &[done] })
        .expect("the submit");
    assert!(device.wait_timeline(done, PATIENCE).expect("the wait"), "the probe ran");
    drop(encoders);
    let magenta = u32::from_le_bytes([255, 0, 255, 255]);
    let words = (PIXEL_BYTES / 4) as u32;
    let first = read_words(&*device, outs[0], words);
    assert!(first.iter().all(|&pixel| pixel == magenta), "the first image's clear");
    let second = read_words(&*device, outs[1], words);
    let shared = second.iter().filter(|&&pixel| pixel == magenta).count();
    println!(
        "measured: two images placed at one offset of one block share bytes: {} ({shared} of \
         {words} pixels of the second read the first's clear; placement {} bytes, alignment {})",
        if shared == words as usize { "yes" } else { "no" },
        placement.size,
        placement.alignment
    );
    passed(test, device, &sink);
}
