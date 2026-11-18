//! The wakes null alone makes, each bounded by how soon its waiter returns rather than by
//! the result a timed recheck reaches anyway: a timeline's destroy under a wait, and the
//! present's wait, woken by a loss, a destroy, a frame submit on its shared queue and an
//! unstall, and itself waking a wait. The wakes every backend makes are the suite's cases.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use lactose_core::ErrorKind;
use lactose_rhi_api::{
    Acquired, Capabilities, ColourAttachment, Device, Encoder, Extent, ImageHandle, ImageRange,
    ImageState, ImageTransition, Load, PassReach, Presented, QueueLayout, QueuePath, QueueRole,
    RenderPass, Rule, StateOn, Store, Submit, Swapchain, SwapchainDesc, Target, TimelinePoint,
};
use lactose_rhi_null::NullDevice;

use crate::{Silent, request};

/// A wait long enough that a missed wake shows as a wait to its end.
const LONG: Duration = Duration::from_secs(60);
/// What a woken `wait_timeline` must return within: far under `LONG`, however loaded the
/// machine.
const WOKEN: Duration = Duration::from_secs(10);
/// What a present or an acquire, whose own bound is a second, must return within once woken:
/// under the bound, so that the recheck at the bound cannot pass the test.
const WOKEN_IN_BOUND: Duration = Duration::from_millis(600);
/// The head start a waiter is given before the call that should wake it.
const HEAD: Duration = Duration::from_millis(100);

/// A present that shares the graphics queue, and compute on a queue of its own.
const SHARED_PRESENT: QueueLayout =
    QueueLayout { compute: QueuePath::Own, present: QueuePath::Shared };

fn device(queues: QueueLayout, presents: bool) -> NullDevice {
    let set = Capabilities { queues, ..Capabilities::LEAST };
    NullDevice::new(request(&set, presents, Arc::new(Silent)), set).expect("a null device")
}

fn point(device: &dyn Device, value: u64) -> TimelinePoint {
    TimelinePoint { timeline: device.create_timeline(0).expect("a timeline"), value }
}

fn rule<T>(result: &lactose_core::Result<T>) -> Option<Rule> {
    result.as_ref().err().and_then(Rule::of)
}

/// Runs `wait` on a thread, gives it `HEAD`, runs `wake`, and returns what `wait` returned
/// and how long after `wake` began it returned.
fn woken<T: Send>(wait: impl FnOnce() -> T + Send, wake: impl FnOnce()) -> (T, Duration) {
    thread::scope(|scope| {
        let waiter = scope.spawn(move || {
            let out = wait();
            (out, Instant::now())
        });
        thread::sleep(HEAD);
        let woke = Instant::now();
        wake();
        let (out, at) = waiter.join().expect("the waiter");
        (out, at.saturating_duration_since(woke))
    })
}

/// A present encoder that clears `image` and leaves it in `Present`.
fn frame(device: &dyn Device, image: ImageHandle) -> Box<dyn Encoder> {
    let queue = QueueRole::Present;
    let mut encoder = device.create_encoder(queue, "null").expect("an encoder");
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
        label: "null",
        colour: &colour,
        depth: None,
        reach: PassReach::NOTHING,
    });
    encoder.end_pass();
    encoder.transition_images(&[to(ImageState::ColourAttachment, ImageState::Present)]);
    encoder
}

fn swapchain(device: &dyn Device) -> Box<dyn Swapchain> {
    device
        .create_swapchain(&SwapchainDesc { extent: Extent { width: 8, height: 8 } })
        .expect("a swapchain")
}

fn acquire(swapchain: &mut dyn Swapchain) -> ImageHandle {
    for _ in 0..20 {
        if let Acquired::Image(image) = swapchain.acquire().expect("an acquire") {
            return image;
        }
    }
    panic!("no image in twenty bounds")
}

/// A timeline's destroy wakes a wait for it, which is refused `destroyed` at once. Null's
/// alone: on a device a timeline is not destroyed while the host waits on it.
#[test]
fn a_destroy_wakes_a_wait() {
    let device = device(QueueLayout::ONE, false);
    let never = point(&device, 1);
    let device: &dyn Device = &device;
    let (refused, after) = woken(
        || device.wait_timeline(never, LONG),
        || device.destroy_timeline(never.timeline).expect("a destroy"),
    );
    assert_eq!(rule(&refused), Some(Rule::Destroyed), "{refused:?}");
    assert!(after < WOKEN, "the wait returned {after:?} after its timeline's destroy");
}

/// On a queue the present shares with graphics, a present waits for a point that a frame
/// submit made after it signals: the present waits with no lock held, so the frame submit
/// gets in at once, and the present is shown inside its bound.
#[test]
fn a_present_waiting_on_a_shared_queue_lets_the_frame_submit_in() {
    let device = device(SHARED_PRESENT, true);
    let mut swapchain = swapchain(&device);
    let image = acquire(&mut *swapchain);
    let frame_point = point(&device, 1);
    let mut encoders = [frame(&device, image)];
    let device: &dyn Device = &device;
    let (shown, submit_took) = thread::scope(|scope| {
        let submitter = scope.spawn(move || {
            thread::sleep(HEAD);
            let start = Instant::now();
            let signals = [frame_point];
            let submit = Submit { encoders: &mut [], waits: &[], signals: &signals };
            device.submit(QueueRole::Graphics, submit).expect("the frame submit");
            start.elapsed()
        });
        let submit = Submit { encoders: &mut encoders, waits: &[frame_point], signals: &[] };
        let shown = swapchain.present(image, submit);
        (shown, submitter.join().expect("the submitter"))
    });
    assert_eq!(shown.expect("the present"), Presented::Shown);
    assert!(submit_took < WOKEN_IN_BOUND, "the frame submit took {submit_took:?}");
}

/// A loss while a present waits: the present returns `DeviceLost` at once, not `pending` at
/// its bound.
#[test]
fn a_loss_during_a_present_wait_returns_lost() {
    for queues in [QueueLayout::SEVERAL, SHARED_PRESENT] {
        let device = device(queues, true);
        let control = device.control();
        let mut swapchain = swapchain(&device);
        let image = acquire(&mut *swapchain);
        let never = point(&device, 1);
        let mut encoders = [frame(&device, image)];
        let (lost, after) = woken(
            || {
                let submit = Submit { encoders: &mut encoders, waits: &[never], signals: &[] };
                swapchain.present(image, submit)
            },
            || assert!(control.lose(), "the device is still there"),
        );
        assert_eq!(lost.map_err(|error| error.kind()), Err(ErrorKind::DeviceLost), "{queues:?}");
        assert!(after < WOKEN_IN_BOUND, "the present returned {after:?} after the loss");
    }
}

/// A destroy of the timeline a present waits for: refused `destroyed`, and at once.
#[test]
fn a_destroy_during_a_present_wait_refuses_it_at_once() {
    let device = device(QueueLayout::SEVERAL, true);
    let mut swapchain = swapchain(&device);
    let image = acquire(&mut *swapchain);
    let never = point(&device, 1);
    let mut encoders = [frame(&device, image)];
    let device: &dyn Device = &device;
    let (refused, after) = woken(
        || {
            let submit = Submit { encoders: &mut encoders, waits: &[never], signals: &[] };
            swapchain.present(image, submit)
        },
        || device.destroy_timeline(never.timeline).expect("a destroy"),
    );
    assert_eq!(rule(&refused), Some(Rule::Destroyed), "{refused:?}");
    assert!(after < WOKEN_IN_BOUND, "the present returned {after:?} after the destroy");
}

/// A present's submit wakes a wait for the point it signals.
#[test]
fn a_present_wakes_a_wait() {
    let device = device(QueueLayout::SEVERAL, true);
    let mut swapchain = swapchain(&device);
    let image = acquire(&mut *swapchain);
    let presented = point(&device, 1);
    let mut encoders = [frame(&device, image)];
    let device: &dyn Device = &device;
    let (reached, after) = woken(
        || device.wait_timeline(presented, LONG),
        || {
            let signals = [presented];
            let submit = Submit { encoders: &mut encoders, waits: &[], signals: &signals };
            assert_eq!(swapchain.present(image, submit).expect("a present"), Presented::Shown);
        },
    );
    assert!(reached.expect("a wait"), "the presented point is reached");
    assert!(after < WOKEN, "the wait returned {after:?} after the present");
}

/// Every image presented under a stall: an acquire waits, and an unstall gives it an image
/// at once rather than at its bound.
#[test]
fn an_unstall_wakes_an_acquire() {
    let device = device(QueueLayout::SEVERAL, true);
    let control = device.control();
    let mut swapchain = swapchain(&device);
    assert!(control.stall(true), "the device is still there");
    let mut kept = Vec::new();
    for _ in 0..3 {
        let image = acquire(&mut *swapchain);
        let mut encoders = [frame(&device, image)];
        let submit = Submit { encoders: &mut encoders, waits: &[], signals: &[] };
        assert_eq!(swapchain.present(image, submit).expect("a present"), Presented::Shown);
        kept.push(encoders);
    }
    let (acquired, after) =
        woken(|| swapchain.acquire(), || assert!(control.stall(false), "the device is there"));
    assert!(matches!(acquired, Ok(Acquired::Image(_))), "{acquired:?}");
    assert!(after < WOKEN_IN_BOUND, "the acquire returned {after:?} after the unstall");
}
