//! The swapchain on a headless surface (`VK_EXT_headless_surface`): frames presented across
//! a resize with frames in flight, each drawing an offscreen target on the graphics queue
//! that crosses to the present queue and is read back there, and a shutdown with frames in
//! flight. A driver that offers no headless surface skips each, naming why.

use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_core::ErrorKind;
use lactose_rhi_api::{
    Acquired, BufferDesc, BufferHandle, BufferImageCopy, BufferState, BufferTransition,
    BufferUsage, ColourAttachment, Device, Encoder, Extent, Format, ImageDesc, ImageHandle,
    ImageMemory, ImageRange, ImageState, ImageTransition, ImageUsage, Load, MemoryClass, PassReach,
    Presented, QueuePath, QueueRole, Rect, RenderPass, Sharing, StateOn, Store, Submit, Swapchain,
    SwapchainDesc, Target, TimelineHandle, TimelinePoint,
};
use lactose_rhi_vulkan::VulkanDevice;

use crate::{Counting, request};

const PATIENCE: Duration = Duration::from_secs(20);
const G: QueueRole = QueueRole::Graphics;
const P: QueueRole = QueueRole::Present;
/// The offscreen target's size: small, so that its readback is quick to compare.
const TARGET: Extent = Extent { width: 8, height: 8 };

/// A headless device, or `None` where the driver offers no headless surface.
fn headless(sink: &Arc<Counting>, test: &str) -> Option<VulkanDevice> {
    match VulkanDevice::headless(request(Arc::clone(sink) as _)) {
        Ok(device) => Some(device),
        Err(error) if error.kind() == ErrorKind::Unsupported => {
            eprintln!("{test}: skipped, {error}");
            None
        }
        Err(error) => panic!("a headless device: {error}"),
    }
}

/// The next image: an acquire repeated while the bound passes with none.
fn acquire(swapchain: &mut dyn Swapchain) -> Acquired {
    let start = Instant::now();
    loop {
        match swapchain.acquire().expect("an acquire") {
            Acquired::None if start.elapsed() < PATIENCE => {}
            other => return other,
        }
    }
}

/// The colour frame `frame` clears to, in steps of 1/255 so that `Rgba8Unorm` keeps it.
fn colour(frame: u64) -> [u8; 4] {
    [(frame * 37 % 256) as u8, (frame * 11 % 256) as u8, (255 - frame % 256) as u8, 255]
}

fn unorm(bytes: [u8; 4]) -> [f32; 4] {
    bytes.map(|byte| f32::from(byte) / 255.0)
}

fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
    StateOn { state, queue }
}

fn clear(encoder: &mut dyn Encoder, image: ImageHandle, rgba: [f32; 4]) {
    let target = Target { image, mip: 0, layer: 0 };
    let colour = [ColourAttachment {
        target,
        load: Load::Clear(rgba),
        store: Store::Store,
        local_read: false,
    }];
    encoder.begin_render_pass(&RenderPass {
        label: "clear",
        colour: &colour,
        depth: None,
        reach: PassReach::NOTHING,
    });
    encoder.end_pass();
}

fn image_to(
    image: ImageHandle,
    from: StateOn<ImageState>,
    to: StateOn<ImageState>,
) -> ImageTransition {
    ImageTransition { image, range: ImageRange::FIRST, from, to }
}

/// What one frame in flight owns: its offscreen target, the buffer the present queue reads
/// it back into, and the graphics submit's encoder and the present's, kept until the frame's
/// point is reached.
struct Slot {
    target: ImageHandle,
    readback: BufferHandle,
    encoders: Vec<Box<dyn Encoder>>,
    /// The frame and its present's point, once presented.
    shown: Option<(u64, TimelinePoint)>,
}

/// The frames' device objects, and how the target crosses to the present queue.
struct Frames<'a> {
    device: &'a VulkanDevice,
    sharing: Sharing,
    /// The present role has a queue of its own: an exclusive target crosses with a release
    /// on the graphics queue and an acquire on the present queue.
    own: bool,
    timeline: TimelineHandle,
}

impl Frames<'_> {
    fn slot(&self) -> Slot {
        let target = self
            .device
            .create_image(&ImageDesc {
                label: "offscreen",
                format: Format::Rgba8Unorm,
                extent: TARGET,
                mips: 1,
                layers: 1,
                usage: ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE),
                memory: ImageMemory::Own,
                sharing: self.sharing,
            })
            .expect("a target");
        let readback = self
            .device
            .create_buffer(&BufferDesc {
                label: "readback",
                size: u64::from(TARGET.width * TARGET.height * 4),
                usage: BufferUsage::COPY_DESTINATION,
                memory: MemoryClass::Readback,
                sharing: Sharing::Exclusive,
            })
            .expect("a readback buffer");
        Slot { target, readback, encoders: Vec::new(), shown: None }
    }

    /// Frame `frame`: the target cleared on the graphics queue and handed to the present
    /// queue, then the present's submit, which reads it back and clears the swapchain's
    /// image. Returns what the present gave, or `None` for a stale acquire.
    fn frame(
        &self,
        swapchain: &mut dyn Swapchain,
        slot: &mut Slot,
        frame: u64,
    ) -> Option<Presented> {
        let image = match acquire(swapchain) {
            Acquired::Image(image) => image,
            Acquired::Stale => return None,
            Acquired::None => panic!("no image within {PATIENCE:?}"),
        };
        let rgba = unorm(colour(frame));
        let drawn = TimelinePoint { timeline: self.timeline, value: 2 * frame - 1 };
        let shown = TimelinePoint { timeline: self.timeline, value: 2 * frame };
        let mut draw = self.device.create_encoder(G, "frame").expect("an encoder");
        draw.transition_images(&[image_to(
            slot.target,
            on(ImageState::Undefined, G),
            on(ImageState::ColourAttachment, G),
        )]);
        clear(&mut *draw, slot.target, rgba);
        let crossing = image_to(
            slot.target,
            on(ImageState::ColourAttachment, G),
            on(ImageState::CopySource, P),
        );
        draw.transition_images(&[crossing]);
        let mut draws = [draw];
        self.device
            .submit(G, Submit { encoders: &mut draws, waits: &[], signals: &[drawn] })
            .expect("the frame's submit");
        let mut present = self.device.create_encoder(P, "present").expect("an encoder");
        if self.own {
            present.transition_images(&[crossing]);
        }
        present.transition_buffers(&[BufferTransition {
            buffer: slot.readback,
            from: on(BufferState::Undefined, P),
            to: on(BufferState::CopyDestination, P),
        }]);
        let rect = Rect { x: 0, y: 0, extent: TARGET };
        present.copy_image_to_buffer(
            slot.target,
            slot.readback,
            &[BufferImageCopy { buffer_offset: 0, mip: 0, layer: 0, rect }],
        );
        present.transition_buffers(&[BufferTransition {
            buffer: slot.readback,
            from: on(BufferState::CopyDestination, P),
            to: on(BufferState::HostRead, P),
        }]);
        present.transition_images(&[image_to(
            image,
            on(ImageState::Undefined, P),
            on(ImageState::ColourAttachment, P),
        )]);
        clear(&mut *present, image, rgba);
        present.transition_images(&[image_to(
            image,
            on(ImageState::ColourAttachment, P),
            on(ImageState::Present, P),
        )]);
        let mut presents = [present];
        let presented = swapchain
            .present(image, Submit { encoders: &mut presents, waits: &[drawn], signals: &[shown] })
            .expect("the present");
        let [draw] = draws;
        let [present] = presents;
        slot.encoders = vec![draw, present];
        slot.shown = Some((frame, shown));
        Some(presented)
    }

    /// Waits for a slot's last frame and compares its readback with the colour it cleared.
    fn check(&self, slot: &mut Slot) {
        let Some((frame, shown)) = slot.shown.take() else { return };
        assert!(
            self.device.wait_timeline(shown, PATIENCE).expect("the wait"),
            "frame {frame}'s present submit did not complete"
        );
        let mut read = Vec::new();
        let size = u64::from(TARGET.width * TARGET.height * 4);
        self.device
            .read_buffer(slot.readback, 0, size, &mut |got| read.extend_from_slice(got))
            .expect("the host read");
        let expected: Vec<u8> =
            (0..TARGET.width * TARGET.height).flat_map(|_| colour(frame)).collect();
        assert_eq!(read, expected, "frame {frame}'s target as the present queue read it");
        slot.encoders.clear();
    }
}

/// A fixed number of frames, two in flight, presented across a resize, each drawing a
/// target on the graphics queue that the present queue reads back: every present is shown
/// with the next id, every readback is the frame's colour, the resize gives the extent asked
/// for within the surface's bounds, and the layer says nothing. Run with the target shared
/// exclusively, crossing by a release and an acquire where the present role has its own
/// queue, and concurrently.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn frames_in_flight_are_presented_across_a_resize() {
    const FRAMES: u64 = 48;
    const IN_FLIGHT: usize = 2;
    const RESIZE: u64 = 24;
    let sink = Arc::new(Counting::default());
    let Some(device) = headless(&sink, "frames_in_flight_are_presented_across_a_resize") else {
        return;
    };
    let own = device.capabilities().queues.present == QueuePath::Own;
    eprintln!("the present role's path: {:?}", device.capabilities().queues.present);
    for sharing in [Sharing::Exclusive, Sharing::Concurrent] {
        let frames = Frames {
            device: &device,
            sharing,
            own: own && sharing == Sharing::Exclusive,
            timeline: device.create_timeline(0).expect("a timeline"),
        };
        let first = Extent { width: 64, height: 64 };
        let mut swapchain =
            device.create_swapchain(&SwapchainDesc { extent: first }).expect("a swapchain");
        assert_eq!(swapchain.extent(), first);
        assert!(swapchain.image_count() >= 2, "{} images", swapchain.image_count());
        assert!(swapchain.display_times().is_none(), "a headless surface reports no times");
        let mut slots: Vec<Slot> = (0..IN_FLIGHT).map(|_| frames.slot()).collect();
        let mut shown = 0;
        let mut stale = 0;
        let mut frame = 1;
        while frame <= FRAMES {
            let at = (frame as usize) % IN_FLIGHT;
            frames.check(&mut slots[at]);
            if frame == RESIZE + 1 {
                // The caller's own points first, then the resize.
                for slot in &mut slots {
                    frames.check(slot);
                }
                let wider = Extent { width: 96, height: 48 };
                swapchain.resize(wider).expect("the resize");
                assert_eq!(swapchain.extent(), wider);
            }
            // A stale acquire takes no frame, and a stale present takes its frame: either
            // way the caller waits for its points and rebuilds.
            let presented = frames.frame(&mut *swapchain, &mut slots[at], frame);
            match presented {
                Some(Presented::Shown) => shown += 1,
                Some(Presented::Stale) => stale += 1,
                None => {}
            }
            if presented.is_some() {
                assert_eq!(swapchain.last_present().map(|id| id.0), Some(shown + stale));
                frame += 1;
            }
            if presented != Some(Presented::Shown) {
                for slot in &mut slots {
                    frames.check(slot);
                }
                swapchain.resize(swapchain.extent()).expect("the rebuild");
            }
        }
        for slot in &mut slots {
            frames.check(slot);
        }
        eprintln!("{sharing:?}: {shown} shown, {stale} stale of {FRAMES} frames");
        assert_eq!(shown + stale, FRAMES, "every frame was presented");
        assert!(swapchain.display_times().is_none());
        drop(swapchain);
    }
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}

/// Frames in flight at a shutdown, with nothing waited for: the swapchain waits for its
/// presents and the acquires' submits before it destroys what they use, the device waits
/// for the rest, and the layer says nothing. Again with an image acquired and never
/// presented, whose acquire's semaphore the swapchain's drop waits for.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn shutdown_with_frames_in_flight_ends_cleanly() {
    let sink = Arc::new(Counting::default());
    let Some(device) = headless(&sink, "shutdown_with_frames_in_flight_ends_cleanly") else {
        return;
    };
    let own = device.capabilities().queues.present == QueuePath::Own;
    for held in [false, true] {
        let frames = Frames {
            device: &device,
            sharing: Sharing::Exclusive,
            own,
            timeline: device.create_timeline(0).expect("a timeline"),
        };
        let mut swapchain = device
            .create_swapchain(&SwapchainDesc { extent: Extent { width: 32, height: 32 } })
            .expect("a swapchain");
        let mut slots: Vec<Slot> = (0..3).map(|_| frames.slot()).collect();
        for (at, slot) in slots.iter_mut().enumerate() {
            let presented = frames.frame(&mut *swapchain, slot, at as u64 + 1);
            assert_eq!(presented, Some(Presented::Shown));
        }
        if held {
            assert!(matches!(acquire(&mut *swapchain), Acquired::Image(_)));
        }
        drop(swapchain);
        // The encoders' drops wait for their submits.
        drop(slots);
    }
    drop(device);
    assert_eq!(sink.count(), 0, "{:?}", sink.texts());
}
