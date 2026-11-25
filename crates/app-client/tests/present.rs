//! `ClientPresent` alone on `lactose-rhi-null`, called as the present thread calls it, with
//! targets this test draws: the swapchain resized at the first call and at each change of the
//! frame's extent and at no other call, and a heap slot rewritten for each new image, so that
//! an image replaced in its slot is destroyed once the read of its successor is reached; the
//! present's wait for the frame that drew its target, and the slot's wait for the last read,
//! with what null would refuse kept pending; a present refused as pending, and a resize that
//! fails with a driver's error; and the encoders the side keeps, counted as the device makes
//! them: made only where none is reached, held to their bound, given back by a refused
//! present, and let go on a lost device.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use lactose_app::{Finished, PresentReport, PresentSide, TargetIndex};
use lactose_app_client::{ClientPresent, TARGET_FORMAT};
use lactose_core::ErrorKind;
use lactose_rhi_api::{
    Capabilities, ColourAttachment, Device, Encoder, Extent, ImageDesc, ImageHandle, ImageMemory,
    ImageRange, ImageState, ImageTransition, ImageUsage, Load, PassReach, QueueLayout, QueueRole,
    RenderPass, Rule, Sharing, StateOn, Store, Submit, SwapchainDesc, Target, TimelineHandle,
    TimelinePoint,
};
use lactose_rhi_null::Control;

use crate::support::{
    Said, WAIT_BOUND, Watch, Watched, WatchedDevice, lock, null_device, null_shaders,
};

const E1: Extent = Extent { width: 64, height: 48 };
const E2: Extent = Extent { width: 80, height: 40 };

/// A target as a frame leaves it: made at `extent` and drawn ([`draw`]).
fn drawn(
    device: &dyn Device,
    frames: TimelineHandle,
    value: &mut u64,
    extent: Extent,
) -> (ImageHandle, TimelinePoint) {
    let image = device
        .create_image(&ImageDesc {
            label: "target",
            format: TARGET_FORMAT,
            extent,
            mips: 1,
            layers: 1,
            usage: ImageUsage::COLOUR.union(ImageUsage::SAMPLED),
            memory: ImageMemory::Own,
            sharing: Sharing::Exclusive,
        })
        .expect("a target");
    let point = draw(device, frames, value, image, None);
    (image, point)
}

/// `image` drawn as a frame draws a target, behind `ready` where it is given: taken from
/// `Undefined` to a colour attachment on the graphics role by a submit that signals the next
/// value of `frames` and, where the present role has a queue of its own, releases the target
/// toward it as the frame's hand-off does. Nothing here observes that value, so the submit
/// stays pending until a wait sees it.
fn draw(
    device: &dyn Device,
    frames: TimelineHandle,
    value: &mut u64,
    image: ImageHandle,
    ready: Option<TimelinePoint>,
) -> TimelinePoint {
    let mut encoder: Box<dyn Encoder> =
        device.create_encoder(QueueRole::Graphics, "frame").expect("an encoder");
    encoder.transition_images(&[ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: StateOn { state: ImageState::Undefined, queue: QueueRole::Graphics },
        to: StateOn { state: ImageState::ColourAttachment, queue: QueueRole::Graphics },
    }]);
    // A clear and a store are the frame's draw: null refuses a read of undefined contents.
    let colour = [ColourAttachment {
        target: Target { image, mip: 0, layer: 0 },
        load: Load::Clear([0.0, 0.0, 0.0, 1.0]),
        store: Store::Store,
        local_read: false,
    }];
    encoder.begin_render_pass(&RenderPass {
        label: "frame",
        colour: &colour,
        depth: None,
        reach: PassReach { slots: &[], buffers: &[] },
    });
    encoder.end_pass();
    if device.capabilities().queues != QueueLayout::ONE {
        encoder.transition_images(&[ImageTransition {
            image,
            range: ImageRange::FIRST,
            from: StateOn { state: ImageState::ColourAttachment, queue: QueueRole::Graphics },
            to: StateOn { state: ImageState::SampledRead, queue: QueueRole::Present },
        }]);
    }
    *value += 1;
    let point = TimelinePoint { timeline: frames, value: *value };
    let mut encoders = [encoder];
    device
        .submit(
            QueueRole::Graphics,
            Submit { encoders: &mut encoders, waits: ready.as_slice(), signals: &[point] },
        )
        .expect("the frame's submit");
    point
}

/// A present side over a [`WatchedDevice`] on null with `queues`, its swapchain
/// [`Watched`], every present's read kept pending until a wait sees it where `lag` is set.
struct Side {
    device: Arc<dyn Device>,
    control: Control,
    said: Arc<Said>,
    watch: Arc<Watch>,
    present: ClientPresent,
    frames: TimelineHandle,
    value: u64,
    /// Every target drawn, destroyed at the end.
    images: Vec<ImageHandle>,
}

impl Side {
    fn new(queues: QueueLayout, lag: bool) -> Self {
        let (inner, control, said) = null_device(Capabilities { queues, ..Capabilities::LEAST });
        let watch = Arc::new(Watch::default());
        watch.lag.store(lag, Ordering::Release);
        let swapchain = inner.create_swapchain(&SwapchainDesc { extent: E1 }).expect("a swapchain");
        let swapchain =
            Box::new(Watched { inner: swapchain, watch: Arc::clone(&watch), skip: false });
        let device: Arc<dyn Device> = Arc::new(WatchedDevice { inner, watch: Arc::clone(&watch) });
        let present =
            ClientPresent::new(Arc::clone(&device), swapchain, &null_shaders()).expect("the side");
        let frames = device.create_timeline(0).expect("a timeline");
        Self { device, control, said, watch, present, frames, value: 0, images: Vec::new() }
    }

    /// A target drawn at `extent` for `slot`.
    fn finished(&mut self, slot: usize, extent: Extent) -> Finished {
        let (image, point) = drawn(&*self.device, self.frames, &mut self.value, extent);
        self.images.push(image);
        let target = TargetIndex::new(slot).expect("a target index");
        Finished { target, image, extent, point }
    }

    /// `finished`'s target drawn again behind `ready`, the read of its last present.
    fn redrawn(&mut self, finished: Finished, ready: TimelinePoint) -> Finished {
        let point = draw(&*self.device, self.frames, &mut self.value, finished.image, Some(ready));
        Finished { point, ..finished }
    }

    fn call(&mut self, finished: Finished) -> lactose_core::Result<PresentReport> {
        self.present.present(finished)
    }

    /// Null said nothing and refused nothing the watch noted; the side closed, its sampler
    /// destroyed with it, every frame waited for and every target destroyed.
    fn close(mut self) {
        assert_eq!(*lock(&self.watch.refusals), Vec::<String>::new());
        assert_eq!(self.said.count.load(Ordering::Relaxed), 0, "{:?}", lock(&self.said.texts));
        assert_eq!(lock(&self.watch.samplers).len(), 1, "the side's sampler");
        self.present.close();
        assert_eq!(*lock(&self.watch.samplers), [], "the closed side's sampler");
        assert_eq!(*lock(&self.watch.refusals), Vec::<String>::new(), "the close");
        let last = TimelinePoint { timeline: self.frames, value: self.value };
        assert!(self.device.wait_timeline(last, WAIT_BOUND).expect("a wait"), "the frames");
        for image in self.images {
            self.device.destroy_image(image).expect("a target the closed side let go");
        }
        self.device.destroy_timeline(self.frames).expect("the frames' timeline");
    }
}

/// Targets 0 and 1 in turn at E1, E1, E2, E2, E1, E1, each a new image: the swapchain is
/// resized to E1 at the first call, to E2 at the third and to E1 at the fifth, and at no
/// other; and each image the call before last named in the same slot is destroyed, which
/// null refuses while a heap slot holds it, once this call's read is reached.
#[test]
fn the_present_side_resizes_at_each_change_of_extent_and_lets_each_replaced_image_go() {
    let (device, _control, said) =
        null_device(Capabilities { queues: QueueLayout::ONE, ..Capabilities::LEAST });
    let watch = Arc::new(Watch::default());
    let inner = device.create_swapchain(&SwapchainDesc { extent: E1 }).expect("a swapchain");
    let swapchain = Box::new(Watched { inner, watch: Arc::clone(&watch), skip: false });
    let mut present =
        ClientPresent::new(Arc::clone(&device), swapchain, &null_shaders()).expect("the side");
    let frames = device.create_timeline(0).expect("a timeline");
    let mut value = 0;
    let mut in_slots: [Option<ImageHandle>; 2] = [None, None];
    for (call, extent) in [E1, E1, E2, E2, E1, E1].into_iter().enumerate() {
        let slot = call % 2;
        let (image, point) = drawn(&*device, frames, &mut value, extent);
        let target = TargetIndex::new(slot).expect("a target index");
        let report = present.present(Finished { target, image, extent, point }).expect("a call");
        let read = report.read.unwrap_or_else(|| panic!("call {call}: no read: {report:?}"));
        assert!(device.wait_timeline(read, WAIT_BOUND).expect("a wait"), "call {call}'s read");
        if let Some(replaced) = in_slots[slot].replace(image) {
            device
                .destroy_image(replaced)
                .unwrap_or_else(|error| panic!("call {call}: the image it replaced: {error}"));
        }
    }
    assert_eq!(*lock(&watch.resizes), [E1, E2, E1]);
    assert_eq!(said.count.load(Ordering::Relaxed), 0, "{:?}", lock(&said.texts));
    present.close();
    for image in in_slots.into_iter().flatten() {
        device.destroy_image(image).expect("a target the closed side let go");
    }
    device.destroy_timeline(frames).expect("the frames' timeline");
}

/// On two queues, each target released toward the present role by a frame whose point
/// nothing has seen, so its submit is pending when the call takes the target over. The
/// present's submit waits for that point, so null refuses nothing; one that waited for
/// nothing would take the target over from a submit still pending, which null refuses
/// (`present-no-wait`). The render side's own call observes the frame's point right after
/// its submit (`Renderer::open`), so this is shown here and not on a whole run.
#[test]
fn the_present_waits_for_the_frame_that_drew_its_target() {
    let mut side = Side::new(QueueLayout::SEVERAL, false);
    for call in 0..4_u64 {
        let finished = side.finished(usize::try_from(call % 2).expect("an index"), E1);
        let report = side.call(finished).unwrap_or_else(|error| panic!("call {call}: {error}"));
        assert!(report.read.is_some(), "call {call}: {report:?}");
    }
    assert_eq!(side.watch.presents(), 4);
    side.close();
}

/// With every present's read kept pending until a wait sees it, at E1 with no resize after
/// the first call, each slot written anew while a present that declared it may be pending
/// unless a call waited for its read. Each slot's write waits for the last read first, so
/// null refuses nothing; a slot written with no wait, or behind an earlier read, is refused
/// (`uuwp-used`; `crates/app/README.md`, "The targets and the present thread").
///
/// Targets 0, 1 and 0, each a new image: the third call's write of slot 0 meets the first
/// present pending (`slot-no-wait`). Then targets 0, 1 and 0 again: the first two images
/// drawn again behind their reads, the second's present refused as pending, so that it makes
/// no read, and a new image in slot 0, whose write meets pending the present that declared
/// slot 0 last, which is now the last read and not the one before it (`slot-wait-stale`).
#[test]
fn a_heap_slot_is_written_anew_only_behind_the_last_read() {
    for queues in [QueueLayout::ONE, QueueLayout::SEVERAL] {
        let mut side = Side::new(queues, true);
        for (call, slot) in [0, 1, 0].into_iter().enumerate() {
            let finished = side.finished(slot, E1);
            let report = side.call(finished).unwrap_or_else(|error| panic!("call {call}: {error}"));
            assert!(report.read.is_some(), "call {call}: {report:?}");
        }
        side.close();

        let mut side = Side::new(queues, true);
        let a = side.finished(0, E1);
        let a_read = side.call(a).expect("call 0").read.expect("call 0's read");
        let b = side.finished(1, E1);
        let b_read = side.call(b).expect("call 1").read.expect("call 1's read");
        let again = side.redrawn(a, a_read);
        assert!(side.call(again).expect("call 2").read.is_some(), "call 2");
        let again = side.redrawn(b, b_read);
        *lock(&side.watch.refuse_present) = Some(Rule::Pending);
        assert_eq!(side.call(again).expect("call 3").read, None, "call 3, refused");
        let c = side.finished(0, E1);
        let report = side.call(c).unwrap_or_else(|error| panic!("{queues:?}: call 4: {error}"));
        assert!(report.read.is_some(), "call 4: {report:?}");
        assert_eq!(*lock(&side.watch.resizes), [E1]);
        side.close();
    }
}

/// A present refused as pending leaves its image acquired, and the next call presents it
/// with no acquire of its own: a side that forgot it would acquire again, which null refuses
/// while an image is acquired (`pending-present-forgotten`).
#[test]
fn a_present_refused_as_pending_is_made_by_the_next_call_with_the_same_image() {
    let mut side = Side::new(QueueLayout::ONE, false);
    let finished = side.finished(0, E1);
    assert!(side.call(finished).expect("call 0").read.is_some());
    *lock(&side.watch.refuse_present) = Some(Rule::Pending);
    let finished = side.finished(1, E1);
    let report = side.call(finished).expect("call 1");
    assert_eq!(report.read, None, "the refused present made nothing");
    assert_eq!(side.watch.acquires.load(Ordering::Relaxed), 2);
    let finished = side.finished(0, E1);
    assert!(side.call(finished).expect("call 2").read.is_some());
    assert_eq!(side.watch.acquires.load(Ordering::Relaxed), 2, "no acquire of its own");
    assert_eq!(side.watch.presents(), 2);
    side.close();
}

/// A present refused by a rule other than pending also leaves its image acquired (rhi-api,
/// `Swapchain::present`): the call fails, and the next call presents that image with no
/// acquire of its own, as does every call after it. A side that forgot it would acquire
/// again, which null refuses while an image is acquired, at every call after
/// (`refused-present-forgotten`).
#[test]
fn a_present_refused_by_another_rule_is_made_by_the_next_call_with_the_same_image() {
    let mut side = Side::new(QueueLayout::ONE, false);
    let finished = side.finished(0, E1);
    assert!(side.call(finished).expect("call 0").read.is_some());
    *lock(&side.watch.refuse_present) = Some(Rule::State);
    let finished = side.finished(1, E1);
    let refused = side.call(finished).expect_err("call 1, refused");
    assert_eq!(Rule::of(&refused), Some(Rule::State), "{refused}");
    assert_eq!(side.watch.acquires.load(Ordering::Relaxed), 2);
    for call in 2..5 {
        let finished = side.finished(call % 2, E1);
        let report = side.call(finished).unwrap_or_else(|error| panic!("call {call}: {error}"));
        assert!(report.read.is_some(), "call {call}: {report:?}");
    }
    assert_eq!(side.watch.acquires.load(Ordering::Relaxed), 4, "call 2 acquired nothing");
    assert_eq!(side.watch.presents(), 4);
    side.close();
}

/// A present encoder that cannot be made, a driver's error after the acquire, also leaves
/// the image acquired: the call fails, and the next call presents that image with no acquire
/// of its own, as does every call after it. A side that forgot it would acquire again, which
/// null refuses while an image is acquired, at every call after (`encoder-fail`). The first
/// call's encoder is refused, since every later call reuses the encoder it made.
#[test]
fn an_encoder_that_fails_after_the_acquire_leaves_the_image_for_the_next_call() {
    let mut side = Side::new(QueueLayout::ONE, false);
    side.watch.fail_encoder.store(true, Ordering::Release);
    let finished = side.finished(0, E1);
    let failed = side.call(finished).expect_err("call 0, its encoder refused");
    assert_eq!(failed.kind(), ErrorKind::ResourceExhausted, "{failed}");
    assert_eq!(side.watch.acquires.load(Ordering::Relaxed), 1);
    for call in 1..5 {
        let finished = side.finished(call % 2, E1);
        let report = side.call(finished).unwrap_or_else(|error| panic!("call {call}: {error}"));
        assert!(report.read.is_some(), "call {call}: {report:?}");
    }
    assert_eq!(side.watch.acquires.load(Ordering::Relaxed), 4, "call 1 acquired nothing");
    assert_eq!(side.watch.presents(), 4);
    side.close();
}

/// The bound on the encoders the side holds (`ENCODERS` in `src/present.rs`).
const ENCODERS: usize = 4;

/// Targets 0 and 1 drawn once each, then `calls - 2` times again, each behind the read of
/// its last present, so that no call after the second writes a slot or resizes: every call
/// presents. Returns the side.
fn redrawn_in_turn(queues: QueueLayout, lag: bool, calls: usize) -> Side {
    let mut side = Side::new(queues, lag);
    let mut last: [Option<(Finished, TimelinePoint)>; 2] = [None, None];
    for call in 0..calls {
        let slot = call % 2;
        let finished = match last[slot] {
            None => side.finished(slot, E1),
            Some((finished, read)) => side.redrawn(finished, read),
        };
        let report = side
            .call(finished)
            .unwrap_or_else(|error| panic!("{queues:?}, lag {lag}: call {call}: {error}"));
        let read = report.read.unwrap_or_else(|| panic!("call {call}: no read: {report:?}"));
        last[slot] = Some((finished, read));
    }
    assert_eq!(side.watch.presents(), calls);
    side
}

/// The side keeps each present's encoder and records into it again once its read is
/// reached, and makes one only where none is: with every read reached as the next call
/// begins, one encoder for 40 calls. A side that drops each call's encoder makes 40, and its
/// drop of a pending one would wait for the frame's GPU work (`G6`).
#[test]
fn the_side_keeps_its_encoders_and_makes_one_only_where_none_is_reached() {
    for queues in [QueueLayout::ONE, QueueLayout::SEVERAL] {
        let side = redrawn_in_turn(queues, false, 40);
        assert_eq!(side.watch.encoders.load(Ordering::Relaxed), 1, "{queues:?}");
        side.close();
    }
}

/// With every present's read kept pending until a wait sees it, the side holds its bound of
/// encoders and no more over 40 calls: each call past the bound waits for the oldest one's
/// read and then records into it again, and every call presents. An encoder reset before
/// its read is reached is refused as pending (`reset-before-reached`), which fails the call;
/// a side that made one past its bound would make 39 (`bound-ignored`).
#[test]
fn the_side_holds_its_bound_of_encoders_and_resets_each_only_behind_its_read() {
    for queues in [QueueLayout::ONE, QueueLayout::SEVERAL] {
        let side = redrawn_in_turn(queues, true, 40);
        assert_eq!(side.watch.encoders.load(Ordering::Relaxed), ENCODERS, "{queues:?}");
        side.close();
    }
}

/// A present refused, by a rule other than pending and as pending, submitted nothing: its
/// encoder is reset at once and the next call records into it. With every read kept
/// pending, the third call's encoder is a new one, since the first is behind an unseen read,
/// and its present is refused; the fourth records into it, so two are made. A side that
/// dropped it would make a third (`unsubmitted-dropped`), and one that kept it as submitted
/// would too, behind a read no submit signals.
#[test]
fn a_refused_presents_encoder_is_reset_at_once_and_recorded_into_by_the_next_call() {
    for rule in [Rule::State, Rule::Pending] {
        let mut side = Side::new(QueueLayout::ONE, true);
        let a = side.finished(0, E1);
        let a_read = side.call(a).expect("call 0").read.expect("call 0's read");
        let b = side.finished(1, E1);
        let b_read = side.call(b).expect("call 1").read.expect("call 1's read");
        *lock(&side.watch.refuse_present) = Some(rule);
        let again = side.redrawn(a, a_read);
        match side.call(again) {
            Ok(report) => assert_eq!((rule, report.read), (Rule::Pending, None), "call 2"),
            Err(error) => assert_eq!(Rule::of(&error), Some(rule), "call 2: {error}"),
        }
        let again = side.redrawn(b, b_read);
        assert!(side.call(again).expect("call 3").read.is_some(), "{rule:?}: call 3");
        assert_eq!(side.watch.encoders.load(Ordering::Relaxed), 2, "{rule:?}");
        assert_eq!(side.watch.presents(), 3, "{rule:?}");
        side.close();
    }
}

/// A device lost with encoders held, their reads pending: the next call fails with
/// `DeviceLost` and makes no encoder, the side lets its encoders go, and its `close`
/// returns. Null's drops never wait, so the reset before each drop, which a lost device
/// accepts, is shown here only as accepted; `lactose-rhi-vulkan`'s drop waits for nothing on
/// a lost device either way.
#[test]
fn a_lost_device_ends_the_call_and_the_side_closes_with_its_encoders_let_go() {
    for queues in [QueueLayout::ONE, QueueLayout::SEVERAL] {
        let mut side = redrawn_in_turn(queues, true, 6);
        let made = side.watch.encoders.load(Ordering::Relaxed);
        assert_eq!(made, ENCODERS, "{queues:?}");
        let finished = side.finished(0, E1);
        assert!(side.control.lose(), "{queues:?}: the device is gone");
        let lost = side.call(finished).expect_err("a call on a lost device");
        assert_eq!(lost.kind(), ErrorKind::DeviceLost, "{queues:?}: {lost}");
        assert_eq!(side.watch.encoders.load(Ordering::Relaxed), made, "{queues:?}");
        side.present.close();
    }
}

/// A resize that fails with a driver's error gives up the acquired image (rhi-api,
/// `Swapchain::resize`), and the side forgets it. The call after one whose present was
/// refused as pending holds the acquired image; its resize to E2 fails, null's image table
/// full; the call after that, back at E1 and so with no resize, acquires anew and presents.
/// A side that kept the given-up image would present it, which null refuses
/// (`Rule::Swapchain`).
#[test]
fn a_resize_that_fails_gives_up_the_acquired_image() {
    let mut side = Side::new(QueueLayout::ONE, false);
    let finished = side.finished(0, E1);
    assert!(side.call(finished).expect("call 0").read.is_some());
    *lock(&side.watch.refuse_present) = Some(Rule::Pending);
    let finished = side.finished(1, E1);
    assert_eq!(side.call(finished).expect("call 1").read, None);
    let at_e2 = side.finished(0, E2);
    // Null's image table filled, so that the resize can make no image.
    let filler = ImageDesc {
        label: "filler",
        format: TARGET_FORMAT,
        extent: Extent { width: 1, height: 1 },
        mips: 1,
        layers: 1,
        usage: ImageUsage::SAMPLED,
        memory: ImageMemory::Own,
        sharing: Sharing::Exclusive,
    };
    let mut fillers = Vec::new();
    let full = loop {
        match side.device.create_image(&filler) {
            Ok(image) => fillers.push(image),
            Err(error) => break error,
        }
    };
    assert_eq!(full.kind(), ErrorKind::ResourceExhausted, "{full}");
    let failed = side.call(at_e2).expect_err("a resize into a full table");
    assert_eq!(failed.kind(), ErrorKind::ResourceExhausted, "{failed}");
    for image in fillers {
        side.device.destroy_image(image).expect("a filler");
    }
    let finished = side.finished(1, E1);
    let report = side.call(finished).expect("call 3");
    assert!(report.read.is_some(), "{report:?}");
    assert_eq!(*lock(&side.watch.resizes), [E1, E2]);
    side.close();
}
