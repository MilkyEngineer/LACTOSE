//! `ClientPresent` alone on `lactose-rhi-null`, called as the present thread calls it, with
//! targets this test draws: the swapchain resized at the first call and at each change of the
//! frame's extent and at no other call, and a heap slot rewritten for each new image, so that
//! an image replaced in its slot is destroyed once the read of its successor is reached.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use lactose_app::{Finished, PresentSide, TargetIndex};
use lactose_app_client::{ClientPresent, TARGET_FORMAT};
use lactose_core::Result;
use lactose_rhi_api::{
    Acquired, Capabilities, ColourAttachment, Device, DisplayTime, Encoder, Extent, Format,
    ImageDesc, ImageHandle, ImageMemory, ImageRange, ImageState, ImageTransition, ImageUsage, Load,
    PassReach, PresentId, Presented, QueueLayout, QueueRole, RenderPass, Sharing, StateOn, Store,
    Submit, Swapchain, SwapchainDesc, Target, TimelineHandle, TimelinePoint,
};

use crate::support::{WAIT_BOUND, lock, null_device, null_shaders};

const E1: Extent = Extent { width: 64, height: 48 };
const E2: Extent = Extent { width: 80, height: 40 };

/// Null's swapchain, with each resize it is asked for written down.
struct Resizes {
    inner: Box<dyn Swapchain>,
    asked: Arc<Mutex<Vec<Extent>>>,
}

impl Swapchain for Resizes {
    fn format(&self) -> Format {
        self.inner.format()
    }

    fn extent(&self) -> Extent {
        self.inner.extent()
    }

    fn image_count(&self) -> u32 {
        self.inner.image_count()
    }

    fn acquire(&mut self) -> Result<Acquired> {
        self.inner.acquire()
    }

    fn present(&mut self, image: ImageHandle, submit: Submit<'_>) -> Result<Presented> {
        self.inner.present(image, submit)
    }

    fn resize(&mut self, extent: Extent) -> Result<()> {
        lock(&self.asked).push(extent);
        self.inner.resize(extent)
    }

    fn last_present(&self) -> Option<PresentId> {
        self.inner.last_present()
    }

    fn display_times(&mut self, into: &mut Vec<DisplayTime>) -> bool {
        self.inner.display_times(into)
    }
}

/// A target as a frame leaves it on one queue: made at `extent`, a colour attachment on
/// the graphics role from a submit that signals the next value of `frames`.
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
    *value += 1;
    let point = TimelinePoint { timeline: frames, value: *value };
    let mut encoders = [encoder];
    device
        .submit(
            QueueRole::Graphics,
            Submit { encoders: &mut encoders, waits: &[], signals: &[point] },
        )
        .expect("the frame's submit");
    (image, point)
}

/// Targets 0 and 1 in turn at E1, E1, E2, E2, E1, E1, each a new image: the swapchain is
/// resized to E1 at the first call, to E2 at the third and to E1 at the fifth, and at no
/// other; and each image the call before last named in the same slot is destroyed, which
/// null refuses while a heap slot holds it, once this call's read is reached.
#[test]
fn the_present_side_resizes_at_each_change_of_extent_and_lets_each_replaced_image_go() {
    let (device, _control, said) =
        null_device(Capabilities { queues: QueueLayout::ONE, ..Capabilities::LEAST });
    let asked = Arc::new(Mutex::new(Vec::new()));
    let inner = device.create_swapchain(&SwapchainDesc { extent: E1 }).expect("a swapchain");
    let swapchain = Box::new(Resizes { inner, asked: Arc::clone(&asked) });
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
    assert_eq!(*lock(&asked), [E1, E2, E1]);
    assert_eq!(said.count.load(Ordering::Relaxed), 0, "{:?}", lock(&said.texts));
    present.close();
    for image in in_slots.into_iter().flatten() {
        device.destroy_image(image).expect("a target the closed side let go");
    }
    device.destroy_timeline(frames).expect("the frames' timeline");
}
