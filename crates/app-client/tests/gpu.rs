//! `gpu-cost` (the milestone 2 plan's section 4 table; verification rows 15 and 34, and the
//! comparison of its decision 5.7): ten thousand indexed draws and 96 pipelines, through
//! `lactose-rhi-vulkan`. `cargo xtask measure gpu-cost` runs it under each driver the host has
//! with every layer off, writing one file a driver; `cargo xtask gpu` runs it short under the
//! validators, whose figures it discards.
//!
//! The draws are ten thousand triangles of a vertex pool, drawn into one colour target three
//! ways, each in a render pass of its own: as `draw_indexed` calls, three indices each; as one
//! multi-draw, a `draw_indexed_indirect` of ten thousand records; and as one indirect draw of
//! one record over the thirty thousand indices of a compacted index buffer, the architecture's
//! draw submission (section 2, "Draw submission"). Each way is recorded on a thread of the
//! test's own, named `record`, from the encoder's first command to its last, and submitted from
//! the test's thread, the two timed apart, since MoltenVK encodes Metal inside the submit. Its
//! GPU time is read from two of `lactose-rhi-vulkan`'s timestamps, each in an encoder
//! submitted alone on either side of the pass's submit and each submit waiting on the one
//! before: the layout that crate's device test holds to a lagging render pass under both M1
//! drivers. The pair reads the pass with the queue's handoffs between the three submits, which
//! `clear`, the render pass with its clear alone and no draw, reads by itself. The figures are
//! by the graphics queue's valid bits; a queue that keeps none is said so, and the run gives
//! host time alone.
//!
//! The pipelines are made three ways on a device whose cache starts empty, each pipeline's
//! creation timed with the driver's hit bits counted across the set (`PipelineStats`):
//! source-cold, from the 96 entry pairs of a source the run writes and cooks with a
//! link-time constant it salts, so that the SPIR-V and the MSL are new; specialisation-cold,
//! from `shaders/measure/cost.slang` under a specialisation constant the test salts with 32
//! bits of full entropy, used whole by both stages, which under MoltenVK leaves the MSL the
//! same; and from a reloaded cache, the source-cold set made again on a second device whose
//! cache starts from the first's bytes.
//!
//! *Cold* is cold in the Vulkan pipeline cache, which the hit bits show, and new to Metal's
//! per-user shader cache below it, which nothing here can see: both M1 drivers hand Metal
//! their code, and Metal keeps what it compiled across processes. Under MoltenVK the constant
//! reaches Metal as a function constant over the same MSL; under KosmicKrisp it is folded
//! into the code, so Metal's cache serves any value an earlier run compiled, and its hit bits
//! read as misses either way. A salt is new to Metal's cache only as its values are unlikely
//! to have been drawn before: 96 of 2^32 a run. The cooked salted
//! source is the directory `LACTOSE_MEASURE_SALTED` names, which `cargo xtask measure`
//! writes; under `cargo xtask gpu`, which sets none, the run is short, the source-cold set is
//! skipped and said so, and the reloaded set is the specialised one.

use std::path::{Path, PathBuf};

use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

use lactose_rhi_api::{
    BufferDesc, BufferHandle, BufferReach, BufferState, BufferTransition, BufferUsage,
    ColourAttachment, ColourTargetDesc, Constant, ConstantValue, CookedShader, Cull, Device,
    Encoder, Extent, Format, GraphicsPipelineDesc, GraphicsPipelineHandle, ImageDesc, ImageHandle,
    ImageMemory, ImageRange, ImageState, ImageTransition, ImageUsage, IndexedIndirectArgs, Load,
    MemoryClass, PassReach, PipelineStats, QueueRole, RenderPass, Sharing, StageDesc, StateOn,
    Store, Submit, Target, TimelinePoint, WriteMask,
};
use lactose_rhi_vulkan::{Timestamps, VulkanDevice};

use crate::support::WAIT_BOUND;
use crate::timing::{figure, identity, micros, short, spread, vulkan_device};

/// The triangles drawn, each way.
const DRAWS: u32 = 10_000;
/// The pipelines made, each way.
const PIPELINES: u32 = 96;
/// The colour target's side.
const SIDE: u32 = 512;
/// The variable naming the directory of the cooked salted source (`salted.vs_NN` and
/// `salted.fs_NN`, as `ricotta --shaders` writes them) and its salt, the file `salt`.
const SALTED_VAR: &str = "LACTOSE_MEASURE_SALTED";
/// The variable naming the directory of the engine's cooked shaders, as `cargo xtask gpu` and
/// `cargo xtask measure gpu-cost` set it.
const SHADERS_VAR: &str = "LACTOSE_GPU_SHADERS";

/// Draws and pipelines, timed. Asserts on counts alone: the draws' and the pipelines' number,
/// the reloaded cache's start, and that the devices said nothing.
#[test]
#[ignore = "needs a Vulkan device: cargo xtask measure gpu-cost"]
fn cost_of_draws_and_pipelines() {
    let shaders = directory(SHADERS_VAR).unwrap_or_else(|| {
        panic!("{SHADERS_VAR} is not set: the engine's shaders were not cooked")
    });
    let salted = directory(SALTED_VAR);
    assert!(
        salted.is_some() || short(),
        "{SALTED_VAR} is not set: a full run is `cargo xtask measure gpu-cost`'s, which cooks \
         the salted source"
    );
    let (device, messages) = vulkan_device(Vec::new());
    identity(&device);
    draws(&device, &shaders);
    let reloaded = pipelines(&device, &shaders, salted.as_deref());
    let cache = device.pipeline_cache().expect("the pipeline cache's bytes");
    drop(device);
    messages.assert_none();
    figure("reloaded.cache_bytes", cache.len());
    let (device, again) = vulkan_device(cache);
    let started = device.pipeline_stats();
    figure("reloaded.cache_start", format!("{:?}", started.cache));
    let target = ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::ALL };
    let (times, _) = make(&device, &reloaded.1, &target);
    report("reloaded", &started, &device.pipeline_stats(), times);
    figure("reloaded.of", reloaded.0);
    drop(device);
    again.assert_none();
}

/// The directory `var` names, where it is set and not empty.
fn directory(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// The cooked shader `<dir>/<name>.spirv`.
fn cooked(dir: &Path, name: &str) -> CookedShader {
    let path = dir.join(format!("{name}.spirv"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    CookedShader::decode(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A pipeline's stages: its cooked vertex and fragment shaders and the fragment's constants.
struct Stages {
    vertex: CookedShader,
    fragment: CookedShader,
    constants: Vec<Constant>,
}

/// Makes a pipeline of each of `sets`, timing each creation alone.
fn make(
    device: &VulkanDevice,
    sets: &[Stages],
    target: &ColourTargetDesc,
) -> (Vec<Duration>, Vec<GraphicsPipelineHandle>) {
    let (mut times, mut made) = (Vec::with_capacity(sets.len()), Vec::with_capacity(sets.len()));
    for stages in sets {
        let desc = GraphicsPipelineDesc {
            label: "gpu-cost",
            vertex: StageDesc { shader: &stages.vertex, constants: &[] },
            fragment: Some(StageDesc { shader: &stages.fragment, constants: &stages.constants }),
            colour: std::slice::from_ref(target),
            depth: None,
            cull: Cull::None,
        };
        let start = Instant::now();
        let pipeline = device.create_graphics_pipeline(&desc).expect("a pipeline");
        times.push(start.elapsed());
        made.push(pipeline);
    }
    (times, made)
}

/// A set's figures: each creation's spread and their sum, and the stages the driver's cache
/// had and had not across it, by its hit bits.
fn report(prefix: &str, before: &PipelineStats, after: &PipelineStats, times: Vec<Duration>) {
    let made = after.pipelines - before.pipelines;
    assert_eq!(made, u64::from(PIPELINES), "{prefix}: the pipelines made");
    figure(&format!("{prefix}.pipelines"), made);
    figure(&format!("{prefix}.stage_hits"), after.stage_hits - before.stage_hits);
    figure(&format!("{prefix}.stage_misses"), after.stage_misses - before.stage_misses);
    figure(&format!("{prefix}.total_us"), micros(times.iter().sum()));
    spread(&format!("{prefix}.each"), times);
}

/// The 96 pipelines source-cold and specialisation-cold on `device`, whose cache started
/// empty; returns the set the reloaded cache makes again, and its name.
fn pipelines(
    device: &VulkanDevice,
    shaders: &Path,
    salted: Option<&Path>,
) -> (&'static str, Vec<Stages>) {
    let target = ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::ALL };
    let source = salted.map(|dir| {
        let salt = std::fs::read_to_string(dir.join("salt")).expect("the salted source's salt");
        figure("source_cold.salt", salt.trim());
        (0..PIPELINES)
            .map(|at| Stages {
                vertex: cooked(dir, &format!("salted.vs_{at:02}")),
                fragment: cooked(dir, &format!("salted.fs_{at:02}")),
                constants: Vec::new(),
            })
            .collect::<Vec<_>>()
    });
    if let Some(sets) = &source {
        let before = device.pipeline_stats();
        let (times, _) = make(device, sets, &target);
        report("source_cold", &before, &device.pipeline_stats(), times);
    } else {
        figure("source_cold.skipped", "no salted source: a run under `cargo xtask gpu`");
    }
    // A salt of 32 bits of full entropy, so that no earlier run is likely to have made these
    // values: the empty hash of a `RandomState`, whose keys std draws from the system's
    // random source. A clock's nanoseconds are microseconds on macOS, all multiples of 8.
    let salt = {
        let hash = std::collections::hash_map::RandomState::new().build_hasher().finish();
        (hash ^ (hash >> 32)) as u32
    };
    figure("specialisation_cold.salt", salt);
    figure("specialisation_cold.salt_source", "the empty hash of a std `RandomState`");
    figure(
        "cold.means",
        "missed by the Vulkan pipeline cache; Metal's per-user shader cache below it is not \
         seen, and under KosmicKrisp its hits count as misses",
    );
    let specialised = (0..PIPELINES)
        .map(|at| Stages {
            vertex: cooked(shaders, "measure/cost.vs_main"),
            fragment: cooked(shaders, "measure/cost.fs_main"),
            constants: vec![Constant { id: 0, value: ConstantValue::Uint(salt.wrapping_add(at)) }],
        })
        .collect::<Vec<_>>();
    let before = device.pipeline_stats();
    let (times, _) = make(device, &specialised, &target);
    report("specialisation_cold", &before, &device.pipeline_stats(), times);
    match source {
        Some(sets) => ("source_cold", sets),
        None => ("specialisation_cold", specialised),
    }
}

/// What every way draws with: the pipeline, the target, the vertex pool, its indices and the
/// two sets of records, all of them in their states before the first way.
struct Scene {
    pipeline: GraphicsPipelineHandle,
    target: ImageHandle,
    positions: BufferHandle,
    indices: BufferHandle,
    multi: BufferHandle,
    compacted: BufferHandle,
}

/// One way of drawing the ten thousand triangles, or the render pass with its clear alone,
/// what the timestamps' layout reads around a pass that draws nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Draws,
    MultiDraw,
    Compacted,
    Clear,
}

impl Way {
    const fn name(self) -> &'static str {
        match self {
            Self::Draws => "draws",
            Self::MultiDraw => "multi_draw",
            Self::Compacted => "compacted",
            Self::Clear => "clear",
        }
    }
}

/// An upload buffer of `usage` holding `bytes`.
fn upload(device: &VulkanDevice, usage: BufferUsage, bytes: &[u8]) -> BufferHandle {
    let size = bytes.len() as u64;
    let desc = BufferDesc {
        label: "gpu-cost",
        size,
        usage,
        memory: MemoryClass::Upload,
        sharing: Sharing::Exclusive,
    };
    let buffer = device.create_buffer(&desc).expect("a buffer");
    let fill = &mut |lent: &mut [u8]| lent.copy_from_slice(bytes);
    device.write_buffer(buffer, 0, size, fill).expect("a host write");
    device.flush_buffer(buffer, 0, size).expect("a flush");
    buffer
}

fn scene(device: &VulkanDevice, shaders: &Path) -> Scene {
    let target = ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::ALL };
    let vertex = cooked(shaders, "measure/cost.vs_main");
    let fragment = cooked(shaders, "measure/cost.fs_main");
    let pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            label: "gpu-cost",
            vertex: StageDesc { shader: &vertex, constants: &[] },
            fragment: Some(StageDesc { shader: &fragment, constants: &[] }),
            colour: &[target],
            depth: None,
            cull: Cull::None,
        })
        .expect("the draws' pipeline");
    let image = device
        .create_image(&ImageDesc {
            label: "gpu-cost",
            format: Format::Rgba8Unorm,
            extent: Extent { width: SIDE, height: SIDE },
            mips: 1,
            layers: 1,
            usage: ImageUsage::COLOUR,
            memory: ImageMemory::Own,
            sharing: Sharing::Exclusive,
        })
        .expect("the target");
    // A grid of 100 by 100 cells across clip space, a small triangle in each.
    let side = 100;
    let mut positions = Vec::with_capacity(DRAWS as usize * 3 * 16);
    for cell in 0..DRAWS {
        let x = -1.0 + 2.0 * (cell % side) as f32 / side as f32;
        let y = -1.0 + 2.0 * (cell / side) as f32 / side as f32;
        let step = 1.5 / side as f32;
        for [dx, dy] in [[0.0, 0.0], [step, 0.0], [0.0, step]] {
            for value in [x + dx, y + dy, 0.5, 1.0_f32] {
                positions.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    let indices: Vec<u8> = (0..DRAWS * 3).flat_map(u32::to_le_bytes).collect();
    let record = |first_index, index_count| IndexedIndirectArgs {
        index_count,
        instance_count: 1,
        first_index,
        base_vertex: 0,
        first_instance: 0,
    };
    let multi: Vec<u8> = (0..DRAWS).flat_map(|at| record(at * 3, 3).to_bytes()).collect();
    let scene = Scene {
        pipeline,
        target: image,
        positions: upload(device, BufferUsage::DEVICE_ADDRESS, &positions),
        indices: upload(device, BufferUsage::INDEX, &indices),
        multi: upload(device, BufferUsage::INDIRECT, &multi),
        compacted: upload(device, BufferUsage::INDIRECT, &record(0, DRAWS * 3).to_bytes()),
    };
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let to = |buffer, state| BufferTransition {
        buffer,
        from: on(BufferState::Undefined),
        to: on(state),
    };
    let mut encoder = device.create_encoder(QueueRole::Graphics, "gpu-cost").expect("an encoder");
    encoder.transition_buffers(&[
        to(scene.positions, BufferState::ShaderRead),
        to(scene.indices, BufferState::Index),
        to(scene.multi, BufferState::Indirect),
        to(scene.compacted, BufferState::Indirect),
    ]);
    encoder.transition_images(&[ImageTransition {
        image: scene.target,
        range: ImageRange::FIRST,
        from: StateOn { state: ImageState::Undefined, queue: QueueRole::Graphics },
        to: StateOn { state: ImageState::ColourAttachment, queue: QueueRole::Graphics },
    }]);
    submit_and_wait(device, vec![encoder]);
    scene
}

/// Submits `encoders` to the graphics queue and waits for them, within [`WAIT_BOUND`];
/// returns how long the submit call took.
fn submit_and_wait(device: &VulkanDevice, mut encoders: Vec<Box<dyn Encoder>>) -> Duration {
    let timeline = device.create_timeline(0).expect("a timeline");
    let done = TimelinePoint { timeline, value: 1 };
    let start = Instant::now();
    device
        .submit(
            QueueRole::Graphics,
            Submit { encoders: &mut encoders, waits: &[], signals: &[done] },
        )
        .expect("the submit");
    let submitted = start.elapsed();
    let reached = device.wait_timeline(done, WAIT_BOUND).expect("the wait");
    assert!(reached, "the submit did not complete within {WAIT_BOUND:?}");
    device.destroy_timeline(timeline).expect("the timeline's destruction");
    submitted
}

/// Submits the encoder of the first timestamp, `pass`'s and the second's, each alone and
/// each waiting on the one before, the layout `lactose-rhi-vulkan`'s device test holds to a
/// lagging render pass under both M1 drivers, and waits for the last within [`WAIT_BOUND`];
/// returns how long the pass's submit call took, and the host's time from the first submit to
/// the wait's end.
fn bracketed(
    device: &VulkanDevice,
    first: Box<dyn Encoder>,
    pass: Box<dyn Encoder>,
    last: Box<dyn Encoder>,
) -> (Duration, Duration) {
    let timeline = device.create_timeline(0).expect("a timeline");
    let mut before = None;
    let mut submitted = Duration::ZERO;
    // Each encoder is held until the wait has seen the last signal.
    let mut encoders = [[first], [pass], [last]];
    let began = Instant::now();
    for (value, encoder) in (1..).zip(&mut encoders) {
        let point = TimelinePoint { timeline, value };
        let waits: Vec<TimelinePoint> = before.into_iter().collect();
        let start = Instant::now();
        device
            .submit(
                QueueRole::Graphics,
                Submit { encoders: encoder, waits: &waits, signals: &[point] },
            )
            .expect("the submit");
        if value == 2 {
            submitted = start.elapsed();
        }
        before = Some(point);
    }
    let last = before.expect("three submits");
    let reached = device.wait_timeline(last, WAIT_BOUND).expect("the wait");
    assert!(reached, "the submits did not complete within {WAIT_BOUND:?}");
    let round_trip = began.elapsed();
    device.destroy_timeline(timeline).expect("the timeline's destruction");
    (submitted, round_trip)
}

/// Records one way's render pass into a new encoder.
fn record(device: &VulkanDevice, scene: &Scene, way: Way) -> (Box<dyn Encoder>, Duration) {
    let mut encoder = device.create_encoder(QueueRole::Graphics, "gpu-cost").expect("an encoder");
    let start = Instant::now();
    let colour = [ColourAttachment {
        target: Target { image: scene.target, mip: 0, layer: 0 },
        load: Load::Clear([0.0; 4]),
        store: Store::Store,
        local_read: false,
    }];
    let reach = [BufferReach { buffer: scene.positions, state: BufferState::ShaderRead }];
    encoder.begin_render_pass(&RenderPass {
        label: way.name(),
        colour: &colour,
        depth: None,
        reach: PassReach { slots: &[], buffers: &reach },
    });
    encoder.bind_graphics_pipeline(scene.pipeline);
    encoder.bind_index_buffer(scene.indices, 0);
    encoder.push_address(0, scene.positions, 0);
    match way {
        Way::Draws => {
            for at in 0..DRAWS {
                encoder.draw_indexed(at * 3..at * 3 + 3, 0, 0..1);
            }
        }
        Way::MultiDraw => {
            encoder.draw_indexed_indirect(scene.multi, 0, DRAWS, IndexedIndirectArgs::BYTES);
        }
        Way::Compacted => {
            encoder.draw_indexed_indirect(scene.compacted, 0, 1, IndexedIndirectArgs::BYTES);
        }
        Way::Clear => {}
    }
    encoder.end_pass();
    let recorded = start.elapsed();
    (encoder, recorded)
}

/// The two timestamps once both are written, read until they are within [`WAIT_BOUND`]: a
/// driver may make a result available a moment after the submit's signal is seen (MoltenVK
/// makes it in the command buffer's completion handler). Both `None`, at once, on a queue
/// with no valid bits.
fn written(stamps: &Timestamps) -> Vec<Option<u64>> {
    let started = Instant::now();
    loop {
        let read = stamps.read().expect("the timestamps' read");
        if stamps.valid_bits() == 0 || read.iter().all(Option::is_some) {
            return read;
        }
        assert!(
            started.elapsed() < WAIT_BOUND,
            "the timestamps of a completed submit were not available within {WAIT_BOUND:?}: \
             {read:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The three ways and the clear alone, each recorded on the `record` thread and submitted
/// from this one, with the GPU time between timestamps on either side of its render pass.
fn draws(device: &VulkanDevice, shaders: &Path) {
    let scene = scene(device, shaders);
    let stamps = device.create_timestamps(QueueRole::Graphics, 2).expect("the timestamps");
    let bits = stamps.valid_bits();
    figure("draws.count", DRAWS);
    figure("timestamps.valid_bits", bits);
    if bits == 0 {
        figure(
            "timestamps.gpu_time",
            "none: the graphics queue keeps no valid bits, host time alone",
        );
    }
    let here = std::thread::current();
    let submit_thread = here.name().unwrap_or("unnamed").to_owned();
    let rounds = if short() { 3 } else { 50 };
    let multi = device.capabilities().multi_draw_indirect;
    for way in [Way::Draws, Way::MultiDraw, Way::Compacted, Way::Clear] {
        let prefix = way.name();
        if way == Way::MultiDraw && !multi {
            figure(&format!("{prefix}.skipped"), "the device reports no multi_draw_indirect");
            continue;
        }
        let (mut recorded, mut submitted, mut gpu) = (Vec::new(), Vec::new(), Vec::new());
        let mut record_thread = None;
        let mut past_host = 0_u32;
        // One round more than counted: the first is the pipeline's first use.
        for round in 0..=rounds {
            let ((encoder, record_time), thread) = std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .name("record".to_owned())
                    .spawn_scoped(scope, || {
                        let thread = std::thread::current().name().map(str::to_owned);
                        (record(device, &scene, way), thread)
                    })
                    .expect("the record thread")
                    .join()
                    .expect("the record thread's result")
            });
            let mut first =
                device.create_encoder(QueueRole::Graphics, "gpu-cost").expect("an encoder");
            stamps.write(&mut *first, 0).expect("the first timestamp");
            let mut last =
                device.create_encoder(QueueRole::Graphics, "gpu-cost").expect("an encoder");
            stamps.write(&mut *last, 1).expect("the second timestamp");
            let (submit_time, round_trip) = bracketed(device, first, encoder, last);
            let read = written(&stamps);
            record_thread = thread;
            if round == 0 {
                continue;
            }
            recorded.push(record_time);
            submitted.push(submit_time);
            if bits > 0 {
                let (Some(start), Some(end)) = (read[0], read[1]) else {
                    unreachable!("`written` returned with a timestamp unwritten");
                };
                let between = stamps.elapsed(start, end);
                past_host += u32::from(between > round_trip);
                gpu.push(between);
            }
        }
        figure(&format!("{prefix}.record_thread"), record_thread.as_deref().unwrap_or("unnamed"));
        figure(&format!("{prefix}.submit_thread"), &submit_thread);
        spread(&format!("{prefix}.record"), recorded);
        spread(&format!("{prefix}.submit"), submitted);
        if !gpu.is_empty() {
            spread(&format!("{prefix}.gpu"), gpu);
            // A pair that read more than the host's time from the first submit to the wait's
            // end did not read this pass: the one bound a run can check of itself.
            figure(&format!("{prefix}.gpu_past_host_rounds"), past_host);
        }
    }
    figure("draws.rounds", rounds);
}
