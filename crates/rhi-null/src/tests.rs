//! Fixed interleavings: a call made in full between another call's two holds of the
//! records, at the points the hook of [`Shared`](crate::device::Shared) names; and the
//! present's walk of the records, against queues that run nothing (`lag`) where a promise
//! must decide a point the timeline has not reached.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lactose_core::Result;
use lactose_rhi_api::{
    Acquired, BufferCopy, BufferDesc, BufferHandle, BufferImageCopy, BufferState, BufferTransition,
    BufferUsage, Capabilities, ColourAttachment, ComputePass, ComputePipelineDesc, CookedShader,
    Device, DeviceRequest, Encoder, Extent, HeapSlot, ImageDesc, ImageHandle, ImageMemory,
    ImageRange, ImageState, ImageTransition, ImageUsage, Load, MemoryClass, Message, MessageSink,
    PassReach, Presented, QueueLayout, QueueRole, Rect, RenderPass, Rule, ShaderStage,
    ShaderTarget, Sharing, StageDesc, StateOn, Store, Submit, Swapchain, SwapchainDesc, Target,
    TimelineHandle, TimelinePoint,
};

use crate::device::Between;
use crate::{NullDevice, NullSurface, lock};

struct Silent;

impl MessageSink for Silent {
    fn message(&self, _: &Message<'_>) {}
}

const G: QueueRole = QueueRole::Graphics;
const C: QueueRole = QueueRole::Compute;
const LIMIT: u64 = 1 << 31;
const PATIENCE: Duration = Duration::from_secs(20);

fn device(queues: QueueLayout, presents: bool) -> NullDevice {
    let set = Capabilities { queues, ..Capabilities::LEAST };
    let request = DeviceRequest {
        adapter: None,
        surface: presents.then(|| Arc::new(NullSurface) as _),
        heap_slots: set.heap_slots,
        samplers: set.samplers,
        pipeline_cache: Vec::new(),
        messages: Arc::new(Silent),
    };
    NullDevice::new(request, set).expect("a null device")
}

/// Runs `call` once, on a second handle of `device`, at the first time a call of `device`
/// reaches `at`, and keeps what it returned.
fn between<T: Send + 'static>(
    device: &NullDevice,
    at: Between,
    mut call: impl FnMut(&NullDevice) -> T + Send + 'static,
) -> Arc<Mutex<Option<T>>> {
    let twin = device.twin();
    let out = Arc::new(Mutex::new(None));
    let kept = Arc::clone(&out);
    let done = AtomicBool::new(false);
    *lock(&device.shared().between) = Some(Box::new(move |reached| {
        if reached == at && !done.swap(true, Ordering::Relaxed) {
            *lock(&kept) = Some(call(&twin));
        }
    }));
    out
}

/// What the hook's call returned, once; and the hook cleared, so the twin it holds goes.
fn taken<T>(device: &NullDevice, out: &Arc<Mutex<Option<T>>>) -> T {
    *lock(&device.shared().between) = None;
    lock(out).take().expect("the hook ran")
}

fn rule<T>(result: &Result<T>) -> Option<Rule> {
    result.as_ref().err().and_then(Rule::of)
}

const fn at(timeline: TimelineHandle, value: u64) -> TimelinePoint {
    TimelinePoint { timeline, value }
}

fn submit(
    device: &dyn Device,
    queue: QueueRole,
    waits: &[TimelinePoint],
    signals: &[TimelinePoint],
) -> Result<()> {
    device.submit(queue, Submit { encoders: &mut [], waits, signals })
}

fn lag(device: &NullDevice, on: bool) {
    let mut records = device.shared().records();
    records.lag = on;
    records.run();
    drop(records);
    device.shared().progressed();
}

#[test]
fn a_host_signal_between_another_host_signals_holds_is_refused() {
    let device = device(QueueLayout::ONE, false);
    let timeline = device.create_timeline(0).expect("a timeline");
    let out = between(&device, Between::HostSignal, move |twin| {
        (
            twin.signal_timeline(at(timeline, 7)),
            submit(twin, G, &[], &[at(timeline, 9)]),
            submit(twin, G, &[at(timeline, 5)], &[at(timeline, 9)]),
        )
    });
    device.signal_timeline(at(timeline, 5)).expect("the host signal");
    let (signal, unwaited, waited) = taken(&device, &out);
    assert_eq!(rule(&signal), Some(Rule::Timeline), "a second host signal in progress: {signal:?}");
    assert_eq!(
        rule(&unwaited),
        Some(Rule::Timeline),
        "a signal that does not wait for it: {unwaited:?}"
    );
    waited.expect("a signal that waits for the host signal's value");
    assert!(device.wait_timeline(at(timeline, 9), PATIENCE).expect("a wait"));
}

/// Item (e) of the fix round: between a host signal's two holds, a submit that only waits,
/// 2^31 below the marked value, is refused, and one within it is not.
#[test]
fn a_wait_only_submit_2_31_below_a_marked_host_signal_is_refused() {
    let device = device(QueueLayout::ONE, false);
    let timeline = device.create_timeline(10).expect("a timeline");
    let marked = 10 + LIMIT - 1;
    let out = between(&device, Between::HostSignal, move |twin| {
        (
            submit(twin, G, &[at(timeline, marked - LIMIT)], &[]),
            submit(twin, G, &[at(timeline, marked - LIMIT + 1)], &[]),
        )
    });
    device.signal_timeline(at(timeline, marked)).expect("the host signal");
    let (beyond, within) = taken(&device, &out);
    assert_eq!(rule(&beyond), Some(Rule::Timeline), "a wait 2^31 below the mark: {beyond:?}");
    within.expect("a wait just within the limit");
    assert_eq!(device.timeline_value(timeline).expect("a read"), marked);
}

fn sampled(device: &NullDevice) -> ImageHandle {
    let desc = ImageDesc {
        label: "unit",
        format: lactose_rhi_api::Format::Rgba8Unorm,
        extent: Extent { width: 4, height: 4 },
        mips: 1,
        layers: 1,
        usage: ImageUsage::SAMPLED.union(ImageUsage::COPY_DESTINATION),
        memory: ImageMemory::Own,
        sharing: Sharing::Exclusive,
    };
    let image = device.create_image(&desc).expect("an image");
    let usage = BufferUsage::COPY_SOURCE;
    let source = BufferDesc {
        label: "unit",
        size: 64,
        usage,
        memory: MemoryClass::Upload,
        sharing: Sharing::Exclusive,
    };
    let source = device.create_buffer(&source).expect("a buffer");
    device.write_buffer(source, 0, 64, &mut |lent| lent.fill(7)).expect("a host write");
    device.flush_buffer(source, 0, 64).expect("a flush");
    let mut encoder = device.create_encoder(G, "unit").expect("an encoder");
    let on = |state| StateOn { state, queue: G };
    let range = ImageRange::FIRST;
    let to = |from, to| ImageTransition { image, range, from: on(from), to: on(to) };
    let buffer = |state| StateOn { state, queue: G };
    encoder.transition_buffers(&[BufferTransition {
        buffer: source,
        from: buffer(BufferState::Undefined),
        to: buffer(BufferState::CopySource),
    }]);
    encoder.transition_images(&[to(ImageState::Undefined, ImageState::CopyDestination)]);
    let rect = Rect { x: 0, y: 0, extent: desc.extent };
    encoder.copy_buffer_to_image(
        source,
        image,
        &[BufferImageCopy { buffer_offset: 0, mip: 0, layer: 0, rect }],
    );
    encoder.transition_images(&[to(ImageState::CopyDestination, ImageState::SampledRead)]);
    run(device, encoder);
    image
}

fn run(device: &NullDevice, encoder: Box<dyn Encoder>) {
    let done = at(device.create_timeline(0).expect("a timeline"), 1);
    device
        .submit(G, Submit { encoders: &mut [encoder], waits: &[], signals: &[done] })
        .expect("a submit");
    assert!(device.wait_timeline(done, PATIENCE).expect("a wait"));
}

/// A pass that declares a slot begins between a heap write's first hold and its slot's
/// write: its submit is refused (`uuwp-used`), the write is not, and a pass recorded after
/// the write sees the new image.
#[test]
fn a_pass_that_begins_during_a_heap_write_is_refused() {
    let device = device(QueueLayout::ONE, false);
    let (first, second) = (sampled(&device), sampled(&device));
    let slot = HeapSlot(3);
    device.write_heap_slot(slot, first).expect("a write");
    let shader = CookedShader {
        stage: ShaderStage::Compute,
        target: ShaderTarget::SpirV,
        entry: "main".to_owned(),
        code: [0x0723_0203_u32, 0x0001_0000, 0, 1, 0]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect(),
        heap: None,
        constants: Vec::new(),
        push_bytes: 0,
        inputs: 0,
        outputs: 0,
    };
    let desc =
        ComputePipelineDesc { label: "unit", stage: StageDesc { shader: &shader, constants: &[] } };
    let pipeline = device.create_compute_pipeline(&desc).expect("a pipeline");
    let record = move |device: &NullDevice| {
        let mut encoder = device.create_encoder(G, "unit").expect("an encoder");
        encoder.begin_compute_pass(&ComputePass {
            label: "unit",
            reach: PassReach { slots: &[slot], buffers: &[] },
        });
        encoder.bind_compute_pipeline(pipeline);
        encoder.push_slot(0, slot);
        encoder.dispatch([1, 1, 1]);
        encoder.end_pass();
        encoder
    };
    let out = between(&device, Between::HeapWrite, move |twin| {
        let encoder = record(twin);
        twin.submit(G, Submit { encoders: &mut [encoder], waits: &[], signals: &[] })
    });
    device.write_heap_slot(slot, second).expect("the write");
    let refused = taken(&device, &out);
    assert_eq!(rule(&refused), Some(Rule::HeapWrite), "{refused:?}");
    run(&device, record(&device));
}

fn upload(device: &NullDevice) -> BufferHandle {
    let usage = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
    let desc = BufferDesc {
        label: "unit",
        size: 16,
        usage,
        memory: MemoryClass::Upload,
        sharing: Sharing::Exclusive,
    };
    device.create_buffer(&desc).expect("a buffer")
}

/// Between a host write's lend and its bytes, a submit that copies into the buffer is
/// refused (`host-overlap`); once the write returns, the same submit is accepted.
#[test]
fn a_submit_that_writes_during_a_host_write_is_refused() {
    let device = device(QueueLayout::ONE, false);
    let (source, written) = (upload(&device), upload(&device));
    device.write_buffer(source, 0, 16, &mut |lent| lent.fill(1)).expect("a host write");
    device.flush_buffer(source, 0, 16).expect("a flush");
    let copy = move |device: &NullDevice| {
        let mut encoder = device.create_encoder(G, "unit").expect("an encoder");
        let on = |state| StateOn { state, queue: G };
        encoder.transition_buffers(&[
            BufferTransition {
                buffer: source,
                from: on(BufferState::Undefined),
                to: on(BufferState::CopySource),
            },
            BufferTransition {
                buffer: written,
                from: on(BufferState::Undefined),
                to: on(BufferState::CopyDestination),
            },
        ]);
        encoder.copy_buffer(
            source,
            written,
            &[BufferCopy { source_offset: 0, destination_offset: 0, size: 16 }],
        );
        encoder
    };
    let out = between(&device, Between::HostLend, move |twin| {
        twin.submit(G, Submit { encoders: &mut [copy(twin)], waits: &[], signals: &[] })
    });
    device.write_buffer(written, 0, 16, &mut |lent| lent.fill(2)).expect("the host write");
    let refused = taken(&device, &out);
    assert_eq!(rule(&refused), Some(Rule::HostOverlap), "{refused:?}");
    device.flush_buffer(written, 0, 16).expect("a flush");
    run(&device, copy(&device));
}

/// A present encoder that clears `image` and leaves it in `Present`.
fn frame(device: &NullDevice, image: ImageHandle) -> Box<dyn Encoder> {
    let queue = QueueRole::Present;
    let mut encoder = device.create_encoder(queue, "unit").expect("an encoder");
    let on = |state| StateOn { state, queue };
    let range = ImageRange::FIRST;
    let to = |from, to| ImageTransition { image, range, from: on(from), to: on(to) };
    encoder.transition_images(&[to(ImageState::Undefined, ImageState::ColourAttachment)]);
    let colour = [ColourAttachment {
        target: Target { image, mip: 0, layer: 0 },
        load: Load::Clear([0.0; 4]),
        store: Store::Store,
        local_read: false,
    }];
    encoder.begin_render_pass(&RenderPass {
        label: "unit",
        colour: &colour,
        depth: None,
        reach: PassReach::NOTHING,
    });
    encoder.end_pass();
    encoder.transition_images(&[to(ImageState::ColourAttachment, ImageState::Present)]);
    encoder
}

fn swapchain(device: &NullDevice) -> (Box<dyn Swapchain>, ImageHandle) {
    let mut swapchain = device
        .create_swapchain(&SwapchainDesc { extent: Extent { width: 4, height: 4 } })
        .expect("a swapchain");
    let Acquired::Image(image) = swapchain.acquire().expect("an acquire") else {
        panic!("no image")
    };
    (swapchain, image)
}

fn present(
    swapchain: &mut dyn Swapchain,
    encoder: &mut [Box<dyn Encoder>; 1],
    image: ImageHandle,
    waits: &[TimelinePoint],
) -> Result<Presented> {
    swapchain.present(image, Submit { encoders: encoder, waits, signals: &[] })
}

/// With the queues running nothing, a present waits for a point below two promises of
/// one queue: the least, of a submit with no wait, is released and decides it; the
/// greater, of a later submit held by a host point, would not.
#[test]
fn the_least_promise_at_or_above_a_point_decides_it() {
    let device = device(QueueLayout::SEVERAL, true);
    let (mut swapchain, image) = swapchain(&device);
    let (timeline, host) = (
        device.create_timeline(0).expect("a timeline"),
        device.create_timeline(0).expect("a timeline"),
    );
    lag(&device, true);
    submit(&device, G, &[], &[at(timeline, 5)]).expect("the least promise");
    submit(&device, G, &[at(host, 1)], &[at(timeline, 8)]).expect("a greater promise, held");
    let mut encoders = [frame(&device, image)];
    let shown = present(&mut *swapchain, &mut encoders, image, &[at(timeline, 3)]);
    assert_eq!(shown.expect("a present the least promise releases"), Presented::Shown);
    lag(&device, false);
    device.signal_timeline(at(host, 1)).expect("the host point");
    assert!(device.wait_timeline(at(timeline, 8), PATIENCE).expect("a wait"));
}

/// A submit waits for a point and promises a greater one on the same timeline: a present
/// that waits for the point is not released by that promise, and is refused past its
/// bound; once the host signals the point, it is shown.
#[test]
fn a_walk_back_to_a_submit_in_progress_leaves_the_point_undecided() {
    let device = device(QueueLayout::SEVERAL, true);
    let (mut swapchain, image) = swapchain(&device);
    let timeline = device.create_timeline(0).expect("a timeline");
    submit(&device, G, &[at(timeline, 1)], &[at(timeline, 2)]).expect("a host release");
    let mut encoders = [frame(&device, image)];
    let held = present(&mut *swapchain, &mut encoders, image, &[at(timeline, 1)]);
    assert_eq!(rule(&held), Some(Rule::Pending), "{held:?}");
    device.signal_timeline(at(timeline, 1)).expect("the host point");
    assert_eq!(
        present(&mut *swapchain, &mut encoders, image, &[at(timeline, 1)])
            .expect("the present again"),
        Presented::Shown
    );
}

/// Two queues each wait for the other's promise: the walk comes back to the first submit
/// while it is still deciding it, and the present is refused past its bound.
#[test]
fn a_cycle_across_two_queues_is_undecided() {
    let device = device(QueueLayout::SEVERAL, true);
    let (mut swapchain, image) = swapchain(&device);
    let (t, u) = (
        device.create_timeline(0).expect("a timeline"),
        device.create_timeline(0).expect("a timeline"),
    );
    submit(&device, G, &[at(u, 1)], &[at(t, 1)]).expect("a submit");
    submit(&device, C, &[at(t, 1)], &[at(u, 2)]).expect("a submit");
    let mut encoders = [frame(&device, image)];
    let held = present(&mut *swapchain, &mut encoders, image, &[at(t, 1)]);
    assert_eq!(rule(&held), Some(Rule::Pending), "{held:?}");
    // Nothing the host may signal releases either: the device is lost to end them.
    assert!(device.control().lose());
}

/// A submit decided in one branch of the walk is decided in the next: with the queues
/// running nothing, a present waits for a point an earlier submit promises and one a
/// later submit, waiting for that promise, promises; both are released.
#[test]
fn a_decided_submit_is_not_taken_for_one_in_progress() {
    let device = device(QueueLayout::SEVERAL, true);
    let (mut swapchain, image) = swapchain(&device);
    let (t, u) = (
        device.create_timeline(0).expect("a timeline"),
        device.create_timeline(0).expect("a timeline"),
    );
    lag(&device, true);
    submit(&device, G, &[], &[at(t, 1)]).expect("a submit");
    submit(&device, G, &[at(t, 1)], &[at(u, 1)]).expect("a submit after it");
    submit(&device, C, &[at(u, 1)], &[at(u, 2)]).expect("a submit on another queue");
    let mut encoders = [frame(&device, image)];
    let shown = present(&mut *swapchain, &mut encoders, image, &[at(t, 1), at(u, 2)]);
    assert_eq!(shown.expect("a present every promise of which is released"), Presented::Shown);
    lag(&device, false);
    assert!(device.wait_timeline(at(u, 2), PATIENCE).expect("a wait"));
}

/// The timeline a present waits for is destroyed while it waits with no lock held: its
/// next walk refuses it (`destroyed`), it submits nothing, and the image stays acquired.
#[test]
fn a_timeline_destroyed_during_a_present_wait_refuses_the_present() {
    for queues in [QueueLayout::SEVERAL, QueueLayout::ONE] {
        let device = device(queues, true);
        let (mut swapchain, image) = swapchain(&device);
        let timeline = device.create_timeline(0).expect("a timeline");
        let out =
            between(&device, Between::PresentWait, move |twin| twin.destroy_timeline(timeline));
        let mut encoders = [frame(&device, image)];
        let refused = present(&mut *swapchain, &mut encoders, image, &[at(timeline, 1)]);
        taken(&device, &out).expect("the destroy of a timeline no submit names");
        assert_eq!(rule(&refused), Some(Rule::Destroyed), "{refused:?}");
        assert_eq!(
            rule(&swapchain.acquire()),
            Some(Rule::Swapchain),
            "the image is still acquired"
        );
        assert_eq!(
            present(&mut *swapchain, &mut encoders, image, &[]).expect("the present again"),
            Presented::Shown
        );
    }
}
