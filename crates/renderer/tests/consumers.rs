//! Compile-only: what `renderer-impl`'s tests, the cost tests of extract and of the remote
//! upload, `lactose-presentation`, the client's frame and a pass provider write against
//! this crate, type-checked and never run (every deciding body is `todo!()`). They pin the
//! surface the README promises: a change that breaks one is a signature change to list.

#![allow(dead_code, unused_variables, missing_docs, clippy::too_many_arguments)]

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use lactose_core::{Error, ErrorKind, Result, StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_input_host::LatestSample;
use lactose_math::render::RenderPosition;
use lactose_present_api::{ExtractPass, ExtractState, InstanceRow, InstanceSink, extract_state};
use lactose_render_graph::{
    BufferImport, CopyPassDecl, FrameDecl, ImageImport, RecordedFrame, RenderGraph,
};
use lactose_renderer::{
    CameraData, CameraLatch, CellTransform, DeclaredFrame, ExtractSink, FrameStats, FrameView,
    InstanceRecord, LatchedCamera, Opened, Projection, RemoteWriter, Renderer, RendererDesc,
    ScatterBatch, ScatterEntry, Shaders, Slot, SlotHolder, SlotTable, StagingRow, shader,
};
use lactose_rhi_api::{
    BufferCopy, BufferHandle, BufferState, CookedShader, Device, Extent, Format, ImageHandle,
    ImageState, QueueRole, Sharing, StateOn, Submit, TimelineHandle, TimelinePoint,
};
use lactose_sim::Simulation;
use lactose_task::{Executor, JobContext};

const GRAPHICS: QueueRole = QueueRole::Graphics;

const PROJECTION: Projection = Projection { fov_y: 1.0, near: 0.1, far: 1000.0 };

fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
    StateOn { state, queue }
}

/// An offscreen target the frame draws into and leaves in `last`.
fn target_import(
    image: ImageHandle,
    last: ImageState,
    ready: Option<TimelinePoint>,
) -> ImageImport<'static> {
    ImageImport {
        label: "target",
        image,
        mips: 1,
        layers: 1,
        sharing: Sharing::Exclusive,
        initial: on(ImageState::Undefined, GRAPHICS),
        last: on(last, GRAPHICS),
        handoff: None,
        ready,
    }
}

/// A world that is shown: the renderer's sink in its extract state. A pass after it is
/// full, which is also how the frame brings a lost kind or a torn frame's instances back.
fn show(simulation: &mut Simulation, renderer: &Renderer) {
    let state: &mut ExtractState = extract_state(simulation).expect("`setup` inserted it");
    let sink: ExtractSink = renderer.sink();
    let replaced: Option<Box<dyn InstanceSink>> = state.set_sink(Some(Box::new(sink)));
}

/// The frame's passes, extract among them.
fn passes(executor: &Executor, simulation: Simulation) -> Simulation {
    executor.block_on(simulation.run_passes(executor)).expect("the passes")
}

/// The end of a frame that declares nothing: a submit with no encoder that signals a
/// point, which frees the frame's region when it is reached.
fn end_with_an_empty_submit(
    device: &dyn Device,
    renderer: &mut Renderer,
    timeline: TimelineHandle,
    value: &mut u64,
) -> Result<TimelinePoint> {
    *value += 1;
    let point = TimelinePoint { timeline, value: *value };
    device.submit(GRAPHICS, Submit { encoders: &mut [], waits: &[], signals: &[point] })?;
    renderer.submitted(point);
    Ok(point)
}

// ---- 1. `renderer-impl`, on null: the slot table and the batch, with no shader run ----

/// Slots are allocated on the CPU from what extract's sink handed over, and freed from
/// its despawns.
fn renderer_impl_slots_follow_a_world(
    executor: &Executor,
    device: Arc<dyn Device>,
    mut simulation: Simulation,
    kind: WorldKind,
    spawned: StableKey,
    despawned: StableKey,
) -> Result<Simulation> {
    let desc = RendererDesc { instances: 1024, rows: 2048, regions: 2, shaders: None };
    let mut renderer = Renderer::new(device, &desc)?;
    show(&mut simulation, &renderer);
    // A new renderer has a region open: the first pass needs no `open`.
    let simulation = passes(executor, simulation);

    let stats: FrameStats = renderer.prepare()?;
    assert!(!stats.any_lost() && !stats.lost(kind));
    assert_eq!((stats.dropped, stats.duplicates, stats.unplaced), (0, 0, 0));
    assert_eq!(stats.live, stats.allocated - stats.freed);

    let slots: &SlotTable = renderer.slots();
    let slot: Slot = slots.get(kind, spawned).expect("the pass wrote it");
    let holder: SlotHolder = slots.holder(slot).expect("a held slot has a holder");
    assert_eq!((holder.kind, holder.key), (kind, spawned));
    assert_eq!(slots.get(kind, despawned), None);
    assert_eq!(slots.live(), stats.live);
    assert!(slots.live() <= slots.extent() && slots.extent() <= slots.capacity());
    // Ascending slot order, on every run.
    let held: Vec<(Slot, SlotHolder)> = slots.iter().collect();
    assert!(held.windows(2).all(|pair| pair[0].0 < pair[1].0));
    // The first instances take the lowest slots, in key order.
    let mut keys: Vec<StableKey> = held.iter().map(|(_, holder)| holder.key).collect();
    keys.sort_unstable();
    assert_eq!(slots.get(kind, keys[0]), Some(Slot::new(0)));
    Ok(simulation)
}

/// After a restore, which a prediction world's rollback is, extract's pass is full: the
/// table holds what the pass wrote and nothing else.
fn renderer_impl_reconciles_after_a_restore(
    executor: &Executor,
    renderer: &mut Renderer,
    restored: Simulation,
    gone: &[StableKey],
    kept: &[StableKey],
) -> Result<Simulation> {
    let simulation = passes(executor, restored);
    let stats = renderer.prepare()?;
    assert!(!stats.lost(WorldKind::Prediction));
    let slots = renderer.slots();
    assert!(gone.iter().all(|&key| slots.get(WorldKind::Prediction, key).is_none()));
    assert!(kept.iter().all(|&key| slots.get(WorldKind::Prediction, key).is_some()));
    assert_eq!(stats.freed as usize, gone.len());
    Ok(simulation)
}

/// The table does not depend on which kind's pass began first: `prepare` applies kind by
/// kind, `Server` before `Prediction`, so a `Prediction` pass begun first still finds the
/// `Server` pass's new instance in slot zero. Each pass writes one new instance.
fn renderer_impl_applies_kind_by_kind(
    device: Arc<dyn Device>,
    tick: Tick,
    server: InstanceRow,
    predicted: InstanceRow,
) -> Result<()> {
    let desc = RendererDesc { instances: 16, rows: 64, regions: 2, shaders: None };
    let mut renderer = Renderer::new(device, &desc)?;
    let (mut early, mut late): (ExtractSink, ExtractSink) = (renderer.sink(), renderer.sink());
    early.begin(ExtractPass { kind: WorldKind::Prediction, tick, full: false });
    late.begin(ExtractPass { kind: WorldKind::Server, tick, full: false });
    early.write(&[predicted]);
    late.write(&[server]);
    early.end();
    late.end();
    let stats = renderer.prepare()?;
    assert_eq!((stats.allocated, stats.unplaced), (2, 0));
    let slots = renderer.slots();
    assert_eq!(slots.get(WorldKind::Server, server.key), Some(Slot::new(0)));
    assert_eq!(slots.get(WorldKind::Prediction, predicted.key), Some(Slot::new(1)));
    Ok(())
}

/// A load that replaces a kind's whole key set, on a renderer sized to what is drawn: the
/// full pass frees the instances it did not write before it places the new ones, so every
/// new instance finds a slot. `old` and `new` are `instances` rows each, of disjoint keys.
fn renderer_impl_places_a_load_at_capacity(
    device: Arc<dyn Device>,
    tick: Tick,
    old: &[InstanceRow],
    new: &[InstanceRow],
) -> Result<()> {
    let instances = u32::try_from(old.len()).expect("a test's instances fit");
    let desc = RendererDesc { instances, rows: 2 * instances + 8, regions: 2, shaders: None };
    let mut renderer = Renderer::new(Arc::clone(&device), &desc)?;
    world_pass(&renderer, WorldKind::Server, tick, false, old);
    assert_eq!(renderer.prepare()?.live, instances);
    let timeline = device.create_timeline(0)?;
    let mut value = 0;
    end_with_an_empty_submit(&*device, &mut renderer, timeline, &mut value)?;
    open_waiting(&mut renderer)?;

    world_pass(&renderer, WorldKind::Server, Tick(tick.0.wrapping_add(1)), true, new);
    let stats = renderer.prepare()?;
    assert_eq!((stats.unplaced, stats.unplaced_in(WorldKind::Server)), (0, 0));
    assert_eq!((stats.freed, stats.allocated, stats.live), (instances, instances, instances));
    let slots = renderer.slots();
    assert!(new.iter().all(|row| slots.get(WorldKind::Server, row.key).is_some()));
    Ok(())
}

/// A batch names each slot once: the builder resolves a duplicate on the host. The test's
/// rows name `twice` two times, the second time in the batch's last row.
fn renderer_impl_resolves_duplicates_on_null(
    renderer: &mut Renderer,
    rows: &[InstanceRow],
    twice: StableKey,
) -> Result<()> {
    {
        let writer: RemoteWriter<'_> = renderer.remote();
        writer.write(rows);
        // Dropping the writer closes the batch; `prepare` cannot be called before.
    }
    let stats = renderer.prepare()?;
    assert_eq!(stats.duplicates, 1);
    assert_eq!(stats.remote_rows as usize, rows.len());

    let batch: ScatterBatch<'_> = renderer.batch();
    assert_eq!(batch.entries().len(), stats.entries as usize);
    for &range in batch.dispatches() {
        let run: &[ScatterEntry] = batch.run(range);
        let mut slots: Vec<Slot> = run.iter().map(|entry| entry.slot).collect();
        slots.sort_unstable();
        assert!(slots.windows(2).all(|pair| pair[0] != pair[1]), "a dispatch names a slot twice");
    }
    // The row kept is the later of the two, which is the last row any entry reads.
    let slot = renderer.slots().get(WorldKind::Client, twice).expect("the batch wrote it");
    let entry = batch.entries().iter().find(|entry| entry.slot == slot).expect("an entry");
    assert!(batch.entries().iter().all(|other| other.row <= entry.row));
    // A remote entry carries neither base flag.
    assert_eq!(entry.flags, 0);
    Ok(())
}

/// The CPU reference of the scatter, on a device that runs no shader: the batch run
/// through the scatter's rule. The ring is never read, so the test learns which row an
/// entry reads from the key of its slot's holder and the rows it wrote, here one pass's.
fn reference_scatter(
    records: &mut [InstanceRecord],
    batch: ScatterBatch<'_>,
    slots: &SlotTable,
    written: &BTreeMap<StableKey, InstanceRow>,
) {
    for &range in batch.dispatches() {
        for entry in batch.run(range) {
            let record = &mut records[entry.slot.index() as usize];
            if entry.flags & ScatterEntry::FREE != 0 {
                record.free();
            } else {
                let holder = slots.holder(entry.slot).expect("the slot was written, so held");
                let row: StagingRow = StagingRow::of(&written[&holder.key]);
                record.scatter(entry.flags, &row);
                assert_eq!(record.flags, InstanceRecord::LIVE);
            }
        }
    }
}

// ---- 2. `renderer-impl`, on a device: the readbacks its plan row asks for ----

/// A renderer with its pipelines, from shaders the cooker wrote for the device.
fn renderer_with_shaders(
    device: Arc<dyn Device>,
    scatter: &CookedShader,
    vertex: &CookedShader,
    fragment: &CookedShader,
) -> Result<Renderer> {
    let shaders = Shaders { scatter, vertex, fragment, target: Format::Rgba8Unorm };
    let desc =
        RendererDesc { instances: 65_536, rows: 65_536 + 8, regions: 3, shaders: Some(shaders) };
    // Every pipeline is made here: the cache the device reports afterwards is complete.
    let renderer = Renderer::new(device, &desc)?;
    let cache: Vec<u8> = renderer.device().pipeline_cache()?;
    Ok(renderer)
}

/// One frame of a device test: what the writers left is prepared, declared, recorded,
/// latched and submitted, with the instance buffer copied out behind the triangle pass.
/// Returns the frame's point, the records below the table's extent and the camera.
fn frame_with_readback(
    graph: &mut RenderGraph,
    ctx: &mut JobContext<'_>,
    renderer: &mut Renderer,
    target: ImageHandle,
    held: Option<TimelinePoint>,
    readback: BufferHandle,
    latch: &CameraLatch<'_>,
) -> Result<(TimelinePoint, u64, LatchedCamera)> {
    let stats = renderer.prepare()?;
    let size = u64::from(renderer.slots().extent()) * u64::from(InstanceRecord::BYTES);
    let recorded = {
        let mut decl = FrameDecl::new(1);
        let target = decl.import_image(&target_import(target, ImageState::CopySource, held));
        let out = decl.import_buffer(&BufferImport {
            label: "instances, read back",
            buffer: readback,
            sharing: Sharing::Exclusive,
            initial: on(BufferState::Undefined, GRAPHICS),
            last: on(BufferState::HostRead, GRAPHICS),
            ready: None,
        });
        // The frame's one extent: `latch` refuses a `CameraLatch::extent` other than this.
        let view = FrameView { target, extent: latch.extent, fraction: 1.0 };
        let declared: DeclaredFrame = renderer.declare(&mut decl, &view)?;
        assert_eq!(declared.scatter.is_some(), stats.entries > 0);
        let instances = declared.instances;
        if size > 0 {
            decl.copy_pass(
                &CopyPassDecl {
                    label: "instances",
                    image_sources: &[],
                    image_destinations: &[],
                    buffer_sources: &[instances],
                    buffer_destinations: &[out],
                },
                move |copy| {
                    let all = BufferCopy { source_offset: 0, destination_offset: 0, size };
                    copy.copy_buffer(instances, out, &[all]);
                },
            );
        }
        let compiled = graph.compile(decl)?;
        graph.record(compiled, ctx)?
        // The declaration's borrow of the renderer ends here.
    };
    let camera = renderer.latch(latch)?;
    match graph.submit(recorded) {
        Ok(point) => {
            renderer.submitted(point);
            Ok((point, size, camera))
        }
        Err(error) => {
            renderer.torn();
            Err(error)
        }
    }
}

/// The records a frame's readback holds, once its point is reached.
fn read_records(
    device: &dyn Device,
    done: TimelinePoint,
    readback: BufferHandle,
    size: u64,
) -> Result<Vec<InstanceRecord>> {
    assert!(device.wait_timeline(done, Duration::from_secs(5))?);
    device.invalidate_buffer(readback, 0, size)?;
    let mut bytes = Vec::new();
    device.read_buffer(readback, 0, size, &mut |lent| bytes.extend_from_slice(lent))?;
    let (records, rest) = bytes.as_chunks::<128>();
    assert!(rest.is_empty());
    Ok(records.iter().map(InstanceRecord::from_bytes).collect())
}

/// One pass of a world, written through a sink of the test's own.
fn world_pass(renderer: &Renderer, kind: WorldKind, tick: Tick, full: bool, rows: &[InstanceRow]) {
    let mut sink: ExtractSink = renderer.sink();
    sink.begin(ExtractPass { kind, tick, full });
    sink.write(rows);
    sink.end();
}

/// Opens a region for the next frame's writers, waiting here for the point that frees
/// one: a test's thread is no worker.
fn open_waiting(renderer: &mut Renderer) -> Result<()> {
    loop {
        match renderer.open()? {
            Opened::Ready => return Ok(()),
            Opened::Wait(point) => {
                renderer.device().wait_timeline(point, Duration::from_secs(5))?;
            }
            Opened::Unsubmitted => unreachable!("no frame of this test is torn"),
        }
    }
}

/// Many duplicates in different workgroups of a large batch. Null's assertion is what
/// proves the resolution; on a device this shows that a resolved batch scatters right.
/// So the entries read their records: in frame 1 a world pass places every instance at
/// `first`, and in frame 2 a pass at the next tick writes each instance twice, workgroups
/// apart, as `TAKE_BASE | HAS_BASE` entries on slots already `LIVE`. Kept, the later row
/// gives `base == previous == first`; unresolved, the second duplicate takes its base
/// from the first one's current placement, which an in-order device such as the M1 shows.
/// An entry with neither flag, as a remote row on a fresh slot, reads nothing of its
/// record and would pass either build.
fn renderer_impl_duplicates_across_workgroups(
    graph: &mut RenderGraph,
    ctx: &mut JobContext<'_>,
    renderer: &mut Renderer,
    targets: [ImageHandle; 2],
    readback: BufferHandle,
    tick: Tick,
    first: &BTreeMap<StableKey, InstanceRow>,
    duplicated: &[InstanceRow],
    kept: &BTreeMap<StableKey, InstanceRow>,
    input: &LatestSample,
) -> Result<()> {
    // The duplicates lie further apart than a workgroup, and no row is a jump.
    assert!(duplicated.len() > 4 * shader::SCATTER_GROUP as usize);
    assert!(duplicated.iter().all(|row| !row.no_interpolate));
    let latch = CameraLatch {
        pivot: RenderPosition::default(),
        input,
        boom: 4.0,
        projection: PROJECTION,
        extent: Extent { width: 256, height: 256 },
    };
    let placed: Vec<InstanceRow> = first.values().copied().collect();
    world_pass(renderer, WorldKind::Server, tick, false, &placed);
    let (done, _, _) =
        frame_with_readback(graph, ctx, renderer, targets[0], None, readback, &latch)?;
    assert!(graph.device().wait_timeline(done, Duration::from_secs(5))?);

    open_waiting(renderer)?;
    world_pass(renderer, WorldKind::Server, Tick(tick.0.wrapping_add(1)), false, duplicated);
    let (done, size, _) =
        frame_with_readback(graph, ctx, renderer, targets[1], None, readback, &latch)?;
    let both = ScatterEntry::TAKE_BASE | ScatterEntry::HAS_BASE;
    assert!(renderer.batch().entries().iter().all(|entry| entry.flags == both));
    let records = read_records(&**graph.device(), done, readback, size)?;
    for (slot, holder) in renderer.slots().iter() {
        let record = records[slot.index() as usize];
        let earlier = StagingRow::of(&first[&holder.key]).transform;
        let later = StagingRow::of(&kept[&holder.key]).transform;
        assert_eq!(record.flags, InstanceRecord::LIVE);
        assert_eq!((record.base, record.previous, record.current), (earlier, earlier, later));
    }
    Ok(())
}

/// With the device made to lag, the ring never overwrites a region a frame in flight
/// reads, and frame N's submit reads N's camera after N + 1's has been written. The
/// renderer has two regions; `held` is a point the host owes, which the first frame's
/// target waits for.
fn renderer_impl_holds_frames_in_flight(
    graph: &mut RenderGraph,
    ctx: &mut JobContext<'_>,
    renderer: &mut Renderer,
    targets: [ImageHandle; 2],
    readbacks: [BufferHandle; 2],
    held: TimelinePoint,
    rows: [&[InstanceRow]; 3],
    latches: [&CameraLatch<'_>; 2],
) -> Result<()> {
    renderer.remote().write(rows[0]);
    let (first, size, first_camera) = frame_with_readback(
        graph,
        ctx,
        renderer,
        targets[0],
        Some(held),
        readbacks[0],
        latches[0],
    )?;
    // The other region is free, so the next frame's writers run while the first is held.
    assert_eq!(renderer.open()?, Opened::Ready);
    renderer.remote().write(rows[1]);
    let (second, _, second_camera) =
        frame_with_readback(graph, ctx, renderer, targets[1], None, readbacks[1], latches[1])?;
    assert_ne!(first_camera.data, second_camera.data);

    // Both regions are in flight. No region is opened, and a writer that runs anyway
    // drops its rows instead of writing under the first frame.
    assert_eq!(renderer.open()?, Opened::Wait(first));
    renderer.remote().write(rows[2]);

    // The test lets the device go. The first frame drew its own rows under its own camera.
    let device = Arc::clone(graph.device());
    device.signal_timeline(held)?;
    let records = read_records(&*device, first, readbacks[0], size)?;
    assert!(device.wait_timeline(second, Duration::from_secs(5))?);
    assert_eq!(renderer.open()?, Opened::Ready);
    let stats = renderer.prepare()?;
    assert_eq!(stats.dropped as usize, rows[2].len());
    Ok(())
}

/// The CPU reference a device test compares its target with: where instance `slot`'s
/// triangle lands in clip space, and its colour.
fn reference_triangle(
    slot: Slot,
    record: &InstanceRecord,
    camera: &CameraData,
    fraction: f32,
) -> Option<([[f32; 4]; 3], [u8; 4])> {
    let placed: CellTransform = record.drawn(fraction, camera.cell)?;
    let corners = shader::TRIANGLE.map(|vertex| camera.clip(placed.apply(vertex)));
    Some((corners, shader::PALETTE[slot.index() as usize % shader::PALETTE.len()]))
}

/// The camera a test expects of a latch: the one statement of the orbit, from the sample
/// the test published.
fn reference_camera(latch: &CameraLatch<'_>, latched: &LatchedCamera) {
    let sample = latch.input.read();
    assert_eq!((latched.yaw, latched.pitch), (sample.yaw, sample.pitch));
    let expected = CameraData::orbit(
        latch.pivot,
        sample.yaw,
        sample.pitch,
        latch.boom,
        latch.projection,
        latch.extent,
    );
    assert_eq!(latched.data, expected);
    assert_eq!(latched.data.cell, latch.pivot.cell);
    let bytes: [u8; 80] = latched.data.to_bytes();
}

// ---- 3. The cost tests of decisions 5.2 and 5.3: the ring on a device, no shader ----

/// One measured pass of `extract-50k`: the wait for a free region is outside it and
/// counted, the pass writes the device's ring through the sink, and the frame ends with
/// an empty submit that frees the region.
fn extract_cost_pass(
    executor: &Executor,
    renderer: &mut Renderer,
    simulation: Simulation,
    timeline: TimelineHandle,
    value: &mut u64,
    waits: &mut u32,
    mut measure: impl FnMut(&mut dyn FnMut()),
) -> Result<Simulation> {
    let device = Arc::clone(renderer.device());
    loop {
        match renderer.open()? {
            Opened::Ready => break,
            Opened::Wait(point) => {
                *waits += 1;
                device.wait_timeline(point, Duration::from_secs(5))?;
            }
            Opened::Unsubmitted => unreachable!("no frame of this test is torn"),
        }
    }
    let mut world = Some(simulation);
    measure(&mut || world = world.take().map(|simulation| passes(executor, simulation)));
    let stats = renderer.prepare()?;
    assert_eq!(stats.dropped, 0, "a dropped row makes the pass cheaper than it is");
    end_with_an_empty_submit(&*device, renderer, timeline, value)?;
    Ok(world.expect("the pass ran"))
}

// ---- 4. `lactose-presentation`: the remote upload, from the schedule's jobs ----

/// What the presentation schedule does with the placements it sampled this frame: writes
/// them through the same ring as extract, from forked jobs, each remote slot once.
fn presentation_uploads_remote(
    writer: &RemoteWriter<'_>,
    ctx: &mut JobContext<'_>,
    sampled: &[InstanceRow],
) {
    ctx.scope(|scope| {
        for rows in sampled.chunks(256) {
            scope.fork(move |_| writer.write(rows));
        }
    });
}

/// One measured upload of `remote-upload-5k`, and the counts that show each remote slot
/// was written once.
fn remote_upload_cost(
    renderer: &mut Renderer,
    ctx: &mut JobContext<'_>,
    sampled: &[InstanceRow],
    timeline: TimelineHandle,
    value: &mut u64,
) -> Result<()> {
    {
        let writer = renderer.remote();
        presentation_uploads_remote(&writer, ctx, sampled);
    }
    let stats = renderer.prepare()?;
    assert_eq!(stats.remote_rows as usize, sampled.len());
    assert_eq!((stats.duplicates, stats.dropped, stats.unplaced), (0, 0, 0));
    assert_eq!(stats.live as usize, sampled.len());
    let device = Arc::clone(renderer.device());
    end_with_an_empty_submit(&*device, renderer, timeline, value)?;
    Ok(())
}

// ---- 5. The client's frame (`frame-design`, `frame-impl`) ----

/// What `lactose-app-client` wires. `lactose-app` never names this crate (decision 5.11):
/// it drives the render side through a trait of `frame-design`'s, whose shape is below.
struct ClientRender {
    graph: RenderGraph,
    renderer: Renderer,
    recorded: Option<RecordedFrame>,
    extent: Extent,
    counts: FrameStats,
}

trait RenderSide: Send + Sync {
    /// Before the frame's writers: whether a region is open, or the point that frees one.
    fn open(&mut self, ctx: &mut JobContext<'_>) -> Result<Option<TimelinePoint>>;

    /// Waits for the point `open` returned, on the frame's own thread: never in a job.
    fn wait(&self, point: TimelinePoint) -> Result<()>;

    /// The counts of the frame last prepared, zero where its `prepare` failed.
    fn counts(&self) -> FrameStats;

    /// A sink for a world that is shown, or shown again after a lost kind or a torn frame.
    fn sink(&self) -> Box<dyn InstanceSink>;

    /// The remote upload, lent to the presentation schedule.
    fn upload_remote(&mut self, ctx: &mut JobContext<'_>, sampled: &[InstanceRow]);

    /// Prepare, declare, compile and record, in a job of the frame.
    fn record(
        &mut self,
        ctx: &mut JobContext<'_>,
        target: ImageHandle,
        read_done: Option<TimelinePoint>,
        fraction: f32,
    ) -> Result<()>;

    /// Latch and submit: the frame's point, which the present thread's submit waits for.
    fn latch_and_submit(
        &mut self,
        pivot: RenderPosition,
        input: &LatestSample,
        boom: f32,
    ) -> Result<TimelinePoint>;
}

impl ClientRender {
    /// Declare, compile and record against a shared borrow of the renderer, which ends
    /// when the recorded frame is returned.
    fn recorded_frame(
        renderer: &Renderer,
        graph: &mut RenderGraph,
        ctx: &mut JobContext<'_>,
        target: &ImageImport<'_>,
        extent: Extent,
        fraction: f32,
    ) -> Result<RecordedFrame> {
        let mut decl = FrameDecl::new(1);
        let target = decl.import_image(target);
        renderer.declare(&mut decl, &FrameView { target, extent, fraction })?;
        let compiled = graph.compile(decl)?;
        graph.record(compiled, ctx)
    }
}

impl RenderSide for ClientRender {
    fn open(&mut self, ctx: &mut JobContext<'_>) -> Result<Option<TimelinePoint>> {
        loop {
            match self.renderer.open()? {
                Opened::Ready => return Ok(None),
                // Where the frame waits is `frame-design`'s; this crate never does.
                Opened::Wait(point) => return Ok(Some(point)),
                // After torn frames alone: a frame with no pass still signals a point.
                Opened::Unsubmitted => {
                    let empty = self.graph.compile(FrameDecl::new(1))?;
                    let recorded = self.graph.record(empty, ctx)?;
                    let point = self.graph.submit(recorded)?;
                    self.renderer.submitted(point);
                }
            }
        }
    }

    fn wait(&self, point: TimelinePoint) -> Result<()> {
        // `false` is a timeout: the frame calls `open` again either way.
        self.graph.device().wait_timeline(point, Duration::from_millis(100)).map(|_| ())
    }

    fn counts(&self) -> FrameStats {
        self.counts
    }

    fn sink(&self) -> Box<dyn InstanceSink> {
        Box::new(self.renderer.sink())
    }

    fn upload_remote(&mut self, ctx: &mut JobContext<'_>, sampled: &[InstanceRow]) {
        let writer = self.renderer.remote();
        presentation_uploads_remote(&writer, ctx, sampled);
    }

    fn record(
        &mut self,
        ctx: &mut JobContext<'_>,
        target: ImageHandle,
        read_done: Option<TimelinePoint>,
        fraction: f32,
    ) -> Result<()> {
        self.counts = FrameStats::default();
        self.counts = match self.renderer.prepare() {
            Ok(counts) => counts,
            Err(error) => {
                // A device's error left the frame prepared, to be ended here. After a
                // refusal `torn` ends the earlier frame that was never ended, if that
                // was the reason, and does nothing otherwise.
                self.renderer.torn();
                return Err(error);
            }
        };
        // One of the offscreen targets of decision 5.6, handed on to the present role.
        let target = ImageImport {
            handoff: Some(on(ImageState::SampledRead, QueueRole::Present)),
            ..target_import(target, ImageState::ColourAttachment, read_done)
        };
        let extent = self.extent;
        match Self::recorded_frame(&self.renderer, &mut self.graph, ctx, &target, extent, fraction)
        {
            Ok(recorded) => {
                self.recorded = Some(recorded);
                Ok(())
            }
            Err(error) => {
                // The frame was prepared and will not be submitted.
                self.renderer.torn();
                Err(error)
            }
        }
    }

    fn latch_and_submit(
        &mut self,
        pivot: RenderPosition,
        input: &LatestSample,
        boom: f32,
    ) -> Result<TimelinePoint> {
        let recorded = self
            .recorded
            .take()
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "no frame is recorded"))?;
        let latch = CameraLatch { pivot, input, boom, projection: PROJECTION, extent: self.extent };
        if let Err(error) = self.renderer.latch(&latch) {
            self.renderer.torn();
            return Err(error);
        }
        match self.graph.submit(recorded) {
            Ok(point) => {
                self.renderer.submitted(point);
                Ok(point)
            }
            Err(error) => {
                self.renderer.torn();
                Err(error)
            }
        }
    }
}

/// A world that is no longer shown: nothing tells the renderer, so the frame clears the
/// world's kind with a full pass that writes nothing, through a sink of its own.
fn frame_clears_a_kind(renderer: &Renderer, kind: WorldKind, tick: Tick) {
    let mut sink: ExtractSink = renderer.sink();
    sink.begin(ExtractPass { kind, tick, full: true });
    sink.end();
}

/// The render side waits in a world's resource between the frame's steps.
fn as_resource(render: ClientRender) -> Box<dyn Any + Send + Sync> {
    Box::new(render)
}

/// Runs `step` in one job of the frame's and returns once it has run, so that the frame's
/// own thread blocks and no worker does. How the frame makes the job is `frame-design`'s.
type InJob<'a> = &'a mut dyn FnMut(&mut dyn FnMut(&mut JobContext<'_>));

/// The frame drives the render side through the `dyn` object from its own thread, in the
/// order of decision 5.1: open, waiting off the workers; the passes; the presentation
/// schedule's upload and record, in a job; latch and submit. It begins with `open` after
/// a frame that failed as after any other. The world of `kind` is given a sink again,
/// so that its next pass is full, after a frame that failed and wherever the counts say
/// it lost instances: a lost kind, dropped rows or unplaced rows.
fn frame(
    side: &mut dyn RenderSide,
    executor: &Executor,
    in_job: InJob<'_>,
    simulation: Simulation,
    kind: WorldKind,
    sampled: &[InstanceRow],
    target: ImageHandle,
    pivot: RenderPosition,
    input: &LatestSample,
) -> Result<Simulation> {
    loop {
        let mut opened = None;
        in_job(&mut |ctx| opened = Some(side.open(ctx)));
        match opened.expect("the job ran")? {
            None => break,
            Some(point) => side.wait(point)?,
        }
    }
    // On the frame's own thread, which is no worker: `block_on` is allowed here.
    let mut simulation = passes(executor, simulation);
    let mut recorded = None;
    in_job(&mut |ctx| {
        side.upload_remote(ctx, sampled);
        recorded = Some(side.record(ctx, target, None, 0.5));
    });
    let submitted =
        recorded.expect("the job ran").and_then(|()| side.latch_and_submit(pivot, input, 4.0));
    let counts = side.counts();
    let missing = counts.lost(kind) || counts.dropped_in(kind) > 0 || counts.unplaced_in(kind) > 0;
    if submitted.is_err() || missing {
        let state = extract_state(&mut simulation).expect("`setup` inserted it");
        state.set_sink(Some(side.sink()));
    }
    submitted.map(|_| simulation)
}

// ---- 6. A pass provider (`lactose-terrain`, `lactose-debug-ui`) ----

/// A provider that names this crate and `lactose-rhi-api` and not `lactose-render-graph`,
/// as `crate-tiers.toml` holds `lactose-terrain` and `lactose-debug-ui` to.
mod provider {
    use lactose_renderer::graph::{
        BufferAccess, BufferId, BufferUse, ColourUse, DepthUse, FrameDecl, ImageId, RenderPassDecl,
    };
    use lactose_renderer::{FeatureView, RenderFeature, Renderer};
    use lactose_rhi_api::{Extent, GraphicsPipelineHandle, Load, Rect, Viewport};

    /// Made with its pipeline, when the client starts.
    struct Overlay {
        pipeline: GraphicsPipelineHandle,
    }

    impl RenderFeature for Overlay {
        fn label(&self) -> &str {
            "overlay"
        }

        fn declare<'f>(&'f self, frame: &mut FrameDecl<'f>, view: &FeatureView) {
            let camera = view.camera;
            let extent = view.extent;
            let colour = ColourUse {
                image: view.target,
                mip: 0,
                layer: 0,
                load: Load::Keep,
                local_read: false,
            };
            let depth = DepthUse { image: view.depth, mip: 0, layer: 0, load: Load::Keep };
            frame.render_pass(
                &RenderPassDecl {
                    label: "overlay",
                    colour: &[colour],
                    depth: Some(depth),
                    sampled: &[],
                    buffers: &[BufferUse { buffer: camera, access: BufferAccess::Read }],
                    slots: &[],
                },
                // The recorder borrows the provider until the frame is recorded.
                move |pass| {
                    pass.set_viewport(Viewport {
                        x: 0.0,
                        y: 0.0,
                        width: extent.width as f32,
                        height: extent.height as f32,
                        near: 0.0,
                        far: 1.0,
                    });
                    pass.set_scissor(Rect { x: 0, y: 0, extent });
                    pass.bind_pipeline(self.pipeline);
                    pass.push_address(0, camera, 0);
                    pass.draw(0..3, 0..1);
                },
            );
        }
    }

    /// The client binary registers it once.
    fn register(renderer: &mut Renderer, pipeline: GraphicsPipelineHandle) {
        renderer.add_feature(Box::new(Overlay { pipeline }));
    }

    /// The provider's own test, on null with no renderer and no cooked shader of the
    /// renderer's: its passes declared into a declaration of the test's, over the ids the
    /// test imported there.
    fn provider_declares_alone<'f>(
        overlay: &'f Overlay,
        frame: &mut FrameDecl<'f>,
        target: ImageId,
        depth: ImageId,
        camera: BufferId,
        extent: Extent,
    ) {
        overlay.declare(frame, &FeatureView::new(target, depth, camera, extent));
    }
}
