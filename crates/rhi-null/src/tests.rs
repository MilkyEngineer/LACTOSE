//! Fixed interleavings: a call made in full between another call's two holds of the
//! records, at the points the hook of [`Shared`](crate::device::Shared) names; and the
//! present's walk of the records, against queues that run nothing (`lag`) where a promise
//! must decide a point the timeline has not reached; and the drops that wait, of the
//! swapchain and of the device's last handle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
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
    *lock(&device.shared().between) = Some(Arc::new(Mutex::new(move |reached| {
        if reached == at && !done.swap(true, Ordering::Relaxed) {
            *lock(&kept) = Some(call(&twin));
        }
    })));
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
        workgroup: Some([1, 1, 1]),
        varyings: Vec::new(),
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

/// How long a drop that must wait is given before the test takes it as not waiting.
const HELD: Duration = Duration::from_millis(300);

/// How long a drop, once what it waits for has run, may take to return.
const WOKEN: Duration = Duration::from_secs(10);

/// Runs `release` on a thread of its own and reports, through the receiver, when it returns.
/// Not scoped: a drop that never returns fails the test rather than hanging its join.
fn dropped(release: impl FnOnce() + Send + 'static) -> mpsc::Receiver<()> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        release();
        sender.send(()).ok();
    });
    receiver
}

/// A second write of the heap, started between a write's first hold and its slot's write,
/// does not land before the first write ends ("two writes of the heap never interleave"):
/// the heap's lock holds it. The write is started from inside the hook and given 200 ms.
/// Without the lock, two writes of one slot can leave its entry on one image and its record
/// on the other.
#[test]
fn a_heap_write_started_inside_another_waits_for_it() {
    let device = device(QueueLayout::ONE, false);
    let (first, second) = (sampled(&device), sampled(&device));
    let (slot, other) = (HeapSlot(3), HeapSlot(4));
    let (sender, receiver) = mpsc::channel();
    let keep = Arc::new(Mutex::new(None));
    let kept = Arc::clone(&keep);
    let out = between(&device, Between::HeapWrite, move |twin| {
        let racer = twin.twin();
        let sender = sender.clone();
        *lock(&kept) = Some(std::thread::spawn(move || {
            let written = racer.write_heap_slot(other, second);
            sender.send(()).ok();
            written
        }));
        receiver.recv_timeout(Duration::from_millis(200)).is_ok()
    });
    device.write_heap_slot(slot, first).expect("the first write");
    let landed = taken(&device, &out);
    let racer = lock(&keep).take().expect("the second write was started");
    racer.join().expect("the second writer").expect("the second write");
    assert!(!landed, "a second heap write ran to its end inside the first's holds");
}

/// A hook cleared while it runs stays cleared, as `taken` needs of a call that runs it on
/// another thread: a hook put back after it ran would keep its twin of the device, and so
/// the device, alive (Miri's leak check under one seed of the test above).
#[test]
fn a_hook_cleared_while_it_runs_stays_cleared() {
    let device = device(QueueLayout::ONE, false);
    let image = sampled(&device);
    let out = between(&device, Between::HeapWrite, |twin| {
        *lock(&twin.shared().between) = None;
    });
    device.write_heap_slot(HeapSlot(3), image).expect("the write");
    assert!(lock(&device.shared().between).is_none(), "a cleared hook was put back");
    assert!(lock(&out).is_some(), "the hook ran");
}

/// With the queues running nothing, a present is shown with its submit pending: the
/// swapchain's drop waits until that submit runs, and returns once it has.
#[test]
fn a_swapchain_drop_with_its_present_pending_waits_for_its_submit() {
    let device = device(QueueLayout::SEVERAL, true);
    let (mut swapchain, image) = swapchain(&device);
    lag(&device, true);
    let mut encoders = [frame(&device, image)];
    let shown = present(&mut *swapchain, &mut encoders, image, &[]);
    assert_eq!(shown.expect("a present with its submit lagging"), Presented::Shown);
    let returned = dropped(move || drop(swapchain));
    assert!(
        returned.recv_timeout(HELD).is_err(),
        "the swapchain's drop returned with its present's submit pending"
    );
    lag(&device, false);
    returned.recv_timeout(WOKEN).expect("the swapchain's drop once the submit ran");
}

/// The last drop of a device waits until every submit it took has run: with the queues
/// running nothing, until they run again; with a submit held by a point no one signals,
/// until the device is lost.
#[test]
fn the_last_drop_of_a_device_waits_for_its_pending_submits() {
    let lagging = device(QueueLayout::ONE, false);
    let shared = Arc::clone(lagging.shared());
    lag(&lagging, true);
    let done = at(lagging.create_timeline(0).expect("a timeline"), 1);
    submit(&lagging, G, &[], &[done]).expect("a lagging submit");
    let returned = dropped(move || drop(lagging));
    assert!(returned.recv_timeout(HELD).is_err(), "the last drop returned with a submit lagging");
    let mut records = shared.records();
    records.lag = false;
    records.run();
    drop(records);
    shared.progressed();
    returned.recv_timeout(WOKEN).expect("the last drop once the submit ran");

    let held = device(QueueLayout::ONE, false);
    let control = held.control();
    let gate = at(held.create_timeline(0).expect("a timeline"), 1);
    submit(&held, G, &[gate], &[]).expect("a held submit");
    let returned = dropped(move || drop(held));
    assert!(returned.recv_timeout(HELD).is_err(), "the last drop returned with a submit held");
    assert!(control.lose(), "the device is still there");
    returned.recv_timeout(WOKEN).expect("the last drop once the device is lost");
}

/// Only the last of the device and the encoders and swapchain it made waits. With a submit
/// held by a point no one signals, the device's drop returns at once while a swapchain, or an
/// encoder, still holds the device; that holder's drop, now the last, waits until the device
/// is lost. The loss is made on a thread of its own: a last drop that keeps the records would
/// hold it for good.
#[test]
fn a_drop_that_is_not_the_last_returns_at_once() {
    for holder in ["swapchain", "encoder"] {
        let held = device(QueueLayout::SEVERAL, true);
        let control = held.control();
        let gate = at(held.create_timeline(0).expect("a timeline"), 1);
        submit(&held, G, &[gate], &[]).expect("a held submit");
        let keep: Box<dyn std::any::Any + Send> = if holder == "swapchain" {
            Box::new(swapchain(&held).0)
        } else {
            Box::new(held.create_encoder(G, "kept").expect("an encoder"))
        };
        let returned = dropped(move || drop(held));
        returned.recv_timeout(WOKEN).unwrap_or_else(|_| {
            panic!("{holder}: the device's drop waited while the {holder} still held the device")
        });
        let returned = dropped(move || drop(keep));
        assert!(
            returned.recv_timeout(HELD).is_err(),
            "{holder}: the last drop, the {holder}'s, returned with a submit held"
        );
        let (sender, lost) = mpsc::channel();
        std::thread::spawn(move || sender.send(control.lose()).ok());
        let lost = lost.recv_timeout(WOKEN).unwrap_or_else(|_| {
            panic!("{holder}: a loss could not take the records: the last drop holds them")
        });
        assert!(lost, "{holder}: the device is still there");
        returned
            .recv_timeout(WOKEN)
            .unwrap_or_else(|_| panic!("{holder}: the last drop once the device is lost"));
    }
}

/// A drop made while its thread unwinds a panic does not wait, whether it is the last of the
/// device and its encoders and swapchain, a pending encoder's, or a swapchain's with a present
/// pending. Each holder is dropped by a thread that panics with a submit held or lagging; a
/// guard declared first, and so dropped last, reports that the unwind got past the holder's
/// drop.
#[test]
fn a_drop_while_unwinding_does_not_wait() {
    struct Sent(mpsc::Sender<()>);
    impl Drop for Sent {
        fn drop(&mut self) {
            self.0.send(()).ok();
        }
    }
    for holder in ["device", "encoder", "swapchain", "pending encoder", "pending present"] {
        let held = device(QueueLayout::SEVERAL, true);
        let control = held.control();
        let gate = at(held.create_timeline(0).expect("a timeline"), 1);
        let mut kept_device = None;
        let mut kept_encoders = None;
        let unwound: Box<dyn std::any::Any + Send> = match holder {
            "device" => {
                submit(&held, G, &[gate], &[]).expect("a held submit");
                Box::new(held)
            }
            "encoder" | "swapchain" => {
                submit(&held, G, &[gate], &[]).expect("a held submit");
                let last: Box<dyn std::any::Any + Send> = if holder == "encoder" {
                    Box::new(held.create_encoder(G, "kept").expect("an encoder"))
                } else {
                    Box::new(swapchain(&held).0)
                };
                dropped(move || drop(held)).recv_timeout(WOKEN).unwrap_or_else(|_| {
                    panic!("{holder}: the device's drop, not the last, waited")
                });
                last
            }
            "pending present" => {
                let (mut swapchain, image) = swapchain(&held);
                lag(&held, true);
                let mut encoders = [frame(&held, image)];
                present(&mut *swapchain, &mut encoders, image, &[])
                    .expect("a present with its submit lagging");
                kept_encoders = Some(encoders);
                kept_device = Some(held);
                Box::new(swapchain)
            }
            _ => {
                let mut encoders = [held.create_encoder(G, "pending").expect("an encoder")];
                held.submit(G, Submit { encoders: &mut encoders, waits: &[gate], signals: &[] })
                    .expect("a held submit");
                let [encoder] = encoders;
                kept_device = Some(held);
                Box::new(encoder)
            }
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _sent = Sent(sender);
            let _unwound = unwound;
            panic!("unwinding past a {holder}'s drop");
        });
        let returned = receiver.recv_timeout(WOKEN).is_ok();
        if !returned {
            let _ = control.lose();
        }
        assert!(returned, "{holder}: a drop made while unwinding waited for a held submit");
        if let Some(device) = kept_device {
            device.signal_timeline(gate).expect("the gate");
            lag(&device, false);
            drop(kept_encoders);
        }
    }
}

/// Two host signals of one timeline, each landed between the other's two holds round after
/// round: the one that lands is refused (`timeline`), the other is accepted, and the
/// timeline ends at its value. The conformance case races the two on threads, where null's
/// holds are too close for a landing inside them to happen; this fixes it in both orders.
#[test]
fn a_host_signal_landed_between_another_host_signals_holds_keeps_the_timeline_growing() {
    const ROUNDS: u64 = 20;
    let device = device(QueueLayout::ONE, false);
    let mut landings = [0; 2];
    for round in 0..ROUNDS {
        for (order, landed) in landings.iter_mut().enumerate() {
            let timeline = device.create_timeline(round).expect("a timeline");
            let (low, high) = (at(timeline, round + 3), at(timeline, round + 5));
            let (first, inside) = if order == 0 { (low, high) } else { (high, low) };
            let out =
                between(&device, Between::HostSignal, move |twin| twin.signal_timeline(inside));
            device.signal_timeline(first).expect("the signal whose holds the other lands in");
            let refused = taken(&device, &out);
            assert_eq!(rule(&refused), Some(Rule::Timeline), "round {round}: {refused:?}");
            *landed += 1;
            let value = device.timeline_value(timeline).expect("a read");
            assert_eq!(value, first.value, "round {round}: the timeline ends at the accepted");
            device.destroy_timeline(timeline).expect("a destroy");
        }
    }
    assert_eq!(landings, [ROUNDS; 2], "a landing in every round of each order");
}

/// A wait orders the one signal it waits for, every signal of an earlier submit of that
/// signal's queue, and what an earlier submit of the new one's own queue waits for; not the
/// other signals of the waited submit, and not a signal above the value it waits for.
/// `lactose-rhi-vulkan`'s records test of the same name holds its records to the same
/// shapes.
#[test]
fn a_wait_orders_only_the_signal_it_waits_for() {
    let two_timelines = || {
        let device = device(QueueLayout::SEVERAL, false);
        let c = device.create_timeline(0).expect("a timeline");
        let f = device.create_timeline(0).expect("a timeline");
        lag(&device, true);
        (device, c, f)
    };

    let (device, c, f) = two_timelines();
    submit(&device, C, &[], &[at(c, 1), at(f, 1)]).expect("the pair's first submit");
    let refused = submit(&device, G, &[at(c, 1)], &[at(f, 2)]);
    assert_eq!(rule(&refused), Some(Rule::Timeline), "a wait for another signal of the batch");
    submit(&device, G, &[at(f, 1)], &[]).expect("a wait for the signal itself");
    submit(&device, G, &[], &[at(f, 2)]).expect("an earlier submit's wait for it");
    lag(&device, false);
    assert!(device.wait_timeline(at(f, 2), PATIENCE).expect("a wait"));

    let (device, c, f) = two_timelines();
    submit(&device, C, &[], &[at(f, 1)]).expect("the earlier batch");
    submit(&device, C, &[], &[at(c, 1)]).expect("the later batch");
    submit(&device, G, &[at(c, 1)], &[at(f, 2)]).expect("an earlier batch's signal");
    lag(&device, false);
    assert!(device.wait_timeline(at(f, 2), PATIENCE).expect("a wait"));

    for host in [true, false] {
        let (device, c, f) = two_timelines();
        if host {
            device.signal_timeline(at(f, 1)).expect("the host's f = 1");
        } else {
            submit(&device, C, &[], &[at(f, 1)]).expect("an earlier batch's f = 1");
        }
        submit(&device, C, &[], &[at(c, 1), at(f, 2)]).expect("the pair's first submit");
        let refused = submit(&device, G, &[at(c, 1), at(f, 1)], &[at(f, 3)]);
        assert_eq!(rule(&refused), Some(Rule::Timeline), "a wait below the promise, host {host}");
        lag(&device, false);
        assert!(device.wait_timeline(at(f, 2), PATIENCE).expect("a wait"));
    }
}

/// Null's second leniency against a device ("Queues by role"): it takes the 2^31 limit from
/// the value it has run a timeline to, not from one the caller has seen. A compute submit
/// with no gate signals f = 1, which null runs at once and the caller has not seen; a later
/// compute submit of f = 2^31 is accepted, where a device with f = 1 in flight, its value
/// 0, refuses it.
#[test]
fn a_signal_2_31_above_a_value_the_caller_has_not_seen_is_accepted() {
    let device = device(QueueLayout::SEVERAL, false);
    let f = device.create_timeline(0).expect("a timeline");
    submit(&device, C, &[], &[at(f, 1)]).expect("f = 1, run and unseen");
    submit(&device, C, &[], &[at(f, LIMIT)]).expect("2^31 from 0, within it from 1");
}
