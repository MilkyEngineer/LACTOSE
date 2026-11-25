//! The present side (`crates/app/README.md`, "What `lactose-app-client` wires", step 5): the
//! swapchain, held by `lactose_platform::present_thread`'s lane, drawing each finished
//! target into an acquired image with a pass that samples it (rhi-api, "The present
//! thread"), and the count the frame sheds by. `tests/shown.rs`, section 2, of `lactose-app`
//! is its sketch.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use lactose_app::{Finished, PresentReport, PresentSide, TARGETS};
use lactose_core::Result;
use lactose_rhi_api::{
    Acquired, AddressMode, ColourAttachment, ColourTargetDesc, Cull, Device, DisplayTime, Encoder,
    Extent, Filter, GraphicsPipelineDesc, GraphicsPipelineHandle, HeapSlot, ImageHandle,
    ImageRange, ImageState, ImageTransition, Load, PassReach, PresentId, Presented, QueueRole,
    RenderPass, Rule, SamplerDesc, SamplerIndex, StageDesc, StateOn, Store, Submit, Swapchain,
    Target, TimelineHandle, TimelinePoint, WriteMask,
};

use crate::shaders::CookedShaders;

/// The bound the present side gives its own waits, on the present thread alone.
const PRESENT_WAIT: Duration = Duration::from_millis(100);

/// The heap slots the present pass samples the targets through, one a target.
pub const HEAP: [HeapSlot; TARGETS] = [HeapSlot(0), HeapSlot(1)];

const NOTHING: PresentReport = PresentReport { read: None, shown: false, waiting: None };

const LINEAR: SamplerDesc = SamplerDesc {
    magnify: Filter::Linear,
    minify: Filter::Linear,
    mip: Filter::Nearest,
    address_u: AddressMode::ClampToEdge,
    address_v: AddressMode::ClampToEdge,
};

fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
    StateOn { state, queue }
}

/// The present side's count, `PresentReport::waiting` (`crates/app/README.md`, "The shedding
/// rule"): of the presents made before a call, those finished on the device and not named
/// displayed, neither they nor a later one, by the swapchain's display-time report.
#[derive(Default, Debug)]
pub struct Waiting {
    /// The presents made and not yet known displayed, each with the value its `read`
    /// signals, oldest first.
    made: VecDeque<(PresentId, u64)>,
}

impl Waiting {
    /// No present made.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The present `present` was made, its read signalling `read` on the side's timeline.
    pub fn presented(&mut self, present: PresentId, read: u64) {
        self.made.push_back((present, read));
    }

    /// The count as a call begins: `None` where the swapchain has no source of display
    /// times (`source`), and every present made is forgotten; otherwise the presents whose
    /// read value is `reached` or less, the reads' timeline as one read of it gave it, of
    /// those that `displayed`, the reports since the last call, has not named and that no
    /// later named present follows.
    pub fn take(&mut self, source: bool, displayed: &[DisplayTime], reached: u64) -> Option<u32> {
        if !source {
            self.made.clear();
            return None;
        }
        if let Some(newest) = displayed.iter().map(|time| time.present).max() {
            while self.made.front().is_some_and(|&(present, _)| present <= newest) {
                self.made.pop_front();
            }
        }
        let finished = self.made.iter().filter(|&&(_, read)| read <= reached).count();
        Some(u32::try_from(finished).unwrap_or(u32::MAX))
    }

    /// How many presents are kept.
    #[must_use]
    pub fn kept(&self) -> usize {
        self.made.len()
    }
}

/// The state of the present thread's lane: `Send` and not `Sync`, as the swapchain is.
pub struct ClientPresent {
    device: Option<Arc<dyn Device>>,
    swapchain: Option<Box<dyn Swapchain>>,
    /// The present pass, made for the swapchain's format.
    pipeline: GraphicsPipelineHandle,
    sampler: SamplerIndex,
    /// A timeline of the present side's own: each present that reads a target signals its
    /// next value, which is the report's `read`.
    reads: TimelineHandle,
    next: u64,
    /// The last point a present signalled: what a resize waits for first.
    last: Option<TimelinePoint>,
    /// Whether the swapchain said it no longer matches its surface.
    stale: bool,
    /// The extent the swapchain was last resized to, `None` before the first call, which
    /// resizes it to the frame's. The swapchain's own may differ: it makes images at about
    /// the extent asked for (rhi-api, "The swapchain").
    asked: Option<Extent>,
    /// A swapchain image whose present was refused as pending, presented by the next call.
    acquired: Option<ImageHandle>,
    /// The image this side last put in each heap slot.
    slots: [Option<ImageHandle>; TARGETS],
    waiting: Waiting,
    times: Vec<DisplayTime>,
}

impl ClientPresent {
    /// The present side on `device`, which was made with the window as its surface, with
    /// the heap and sampler table the cooked shaders are compiled for, and `swapchain`, its
    /// swapchain, made in the main-lane call that made the device (`crates/app/README.md`,
    /// "What `lactose-app-client` wires", step 2). Made on the thread that makes the present
    /// thread, and moved to it.
    ///
    /// # Errors
    ///
    /// The device's.
    pub fn new(
        device: Arc<dyn Device>,
        swapchain: Box<dyn Swapchain>,
        shaders: &CookedShaders,
    ) -> Result<Self> {
        let desc = GraphicsPipelineDesc {
            label: "present",
            vertex: StageDesc { shader: &shaders.present_vertex, constants: &[] },
            fragment: Some(StageDesc { shader: &shaders.present_fragment, constants: &[] }),
            colour: &[ColourTargetDesc { format: swapchain.format(), write_mask: WriteMask::ALL }],
            depth: None,
            cull: Cull::None,
        };
        let pipeline = device.create_graphics_pipeline(&desc)?;
        let sampler = device.create_sampler(&LINEAR)?;
        let reads = device.create_timeline(0)?;
        Ok(Self {
            device: Some(device),
            swapchain: Some(swapchain),
            pipeline,
            sampler,
            reads,
            next: 1,
            last: None,
            stale: false,
            asked: None,
            acquired: None,
            slots: [None; TARGETS],
            waiting: Waiting::new(),
            times: Vec::new(),
        })
    }

    fn call(
        &mut self,
        device: &Arc<dyn Device>,
        swapchain: &mut dyn Swapchain,
        finished: Finished,
    ) -> Result<PresentReport> {
        // The count is taken as the call begins, before its waits and its acquire.
        let source = swapchain.display_times(&mut self.times);
        let reached = device.timeline_value(self.reads)?;
        let waiting = self.waiting.take(source, &self.times, reached);
        self.times.clear();
        let nothing = PresentReport { waiting, ..NOTHING };
        if self.stale || self.asked != Some(finished.extent) {
            // The caller's wait before a resize is for its own points (rhi-api, "The
            // swapchain"). A timeout or a refusal as pending is the next call's to retry.
            if let Some(last) = self.last
                && !device.wait_timeline(last, PRESENT_WAIT)?
            {
                return Ok(nothing);
            }
            match swapchain.resize(finished.extent) {
                Ok(()) => {}
                Err(error) if Rule::of(&error) == Some(Rule::Pending) => return Ok(nothing),
                Err(error) => return Err(error),
            }
            // A resize destroys every handle the swapchain gave before it. At an empty
            // surface it keeps its images and stays stale, which the acquire below says.
            self.acquired = None;
            self.stale = false;
            self.asked = Some(finished.extent);
        }
        // A slot is written only once no pending pass declares it, and the pass that last
        // declared this one is covered by the last read this side reported, since calls
        // name the targets in turn. A new image can come with no resize: an extent that
        // changed and came back, the frame between dropped or torn.
        let slot = finished.target.index();
        if self.slots[slot] != Some(finished.image) {
            if let Some(last) = self.last
                && !device.wait_timeline(last, PRESENT_WAIT)?
            {
                return Ok(nothing);
            }
            device.write_heap_slot(HEAP[slot], finished.image)?;
            self.slots[slot] = Some(finished.image);
        }
        let image = match self.acquired.take() {
            Some(image) => image,
            None => match swapchain.acquire()? {
                Acquired::Image(image) => image,
                Acquired::None => return Ok(nothing),
                Acquired::Stale => {
                    self.stale = true;
                    return Ok(nothing);
                }
            },
        };
        let mut encoder = device.create_encoder(QueueRole::Present, "present")?;
        // The acquire half of the hand-off the frame's submit released, with equal values;
        // where one queue serves both roles it is the whole change (graph, "Imports").
        encoder.transition_images(&[ImageTransition {
            image: finished.image,
            range: ImageRange::FIRST,
            from: on(ImageState::ColourAttachment, QueueRole::Graphics),
            to: on(ImageState::SampledRead, QueueRole::Present),
        }]);
        let extent = swapchain.extent();
        draw_target(&mut *encoder, self.pipeline, HEAP[slot], self.sampler, image, extent);
        let read = TimelinePoint { timeline: self.reads, value: self.next };
        let mut encoders = [encoder];
        let submit = Submit { encoders: &mut encoders, waits: &[finished.point], signals: &[read] };
        match swapchain.present(image, submit) {
            Ok(presented) => {
                self.next += 1;
                self.last = Some(read);
                self.stale = presented == Presented::Stale;
                if let Some(present) = swapchain.last_present() {
                    self.waiting.presented(present, read.value);
                }
                Ok(PresentReport {
                    read: Some(read),
                    shown: presented == Presented::Shown,
                    waiting,
                })
            }
            // Nothing was submitted and the image is still the acquired one: the frame's
            // release of the target stays unmet, and the frame takes the target back from
            // `Undefined` behind the read it already knows.
            Err(error) if Rule::of(&error) == Some(Rule::Pending) => {
                self.acquired = Some(image);
                Ok(nothing)
            }
            Err(error) => Err(error),
        }
    }
}

/// The pass that samples the target in `slot` over the whole of the acquired `image`, which
/// it leaves in `Present`.
fn draw_target(
    encoder: &mut dyn Encoder,
    pipeline: GraphicsPipelineHandle,
    slot: HeapSlot,
    sampler: SamplerIndex,
    image: ImageHandle,
    extent: Extent,
) {
    let present = QueueRole::Present;
    encoder.transition_images(&[ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: on(ImageState::Undefined, present),
        to: on(ImageState::ColourAttachment, present),
    }]);
    let colour = [ColourAttachment {
        target: Target { image, mip: 0, layer: 0 },
        load: Load::Clear([0.0, 0.0, 0.0, 1.0]),
        store: Store::Store,
        local_read: false,
    }];
    let slots = [slot];
    encoder.begin_render_pass(&RenderPass {
        label: "present",
        colour: &colour,
        depth: None,
        reach: PassReach { slots: &slots, buffers: &[] },
    });
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_slot(0, slot);
    encoder.push_bytes(4, &sampler.0.to_le_bytes());
    encoder.push_bytes(8, &extent.width.to_le_bytes());
    encoder.push_bytes(12, &extent.height.to_le_bytes());
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    encoder.transition_images(&[ImageTransition {
        image,
        range: ImageRange::FIRST,
        from: on(ImageState::ColourAttachment, present),
        to: on(ImageState::Present, present),
    }]);
}

impl PresentSide for ClientPresent {
    fn present(&mut self, finished: Finished) -> Result<PresentReport> {
        let (Some(device), Some(mut swapchain)) = (self.device.clone(), self.swapchain.take())
        else {
            return Ok(NOTHING);
        };
        let report = self.call(&device, &mut *swapchain, finished);
        self.swapchain = Some(swapchain);
        report
    }

    fn close(&mut self) {
        // The heap slots this side wrote are emptied first, behind a bounded wait for its
        // last read, so the render side's drop may destroy the targets. Past the bound the
        // device refuses the clear (`uuwp-used`) and the slot is left: the device then
        // refuses the destroy of the target it holds, and that image is freed with the
        // device. Nothing here retries. On a lost device the wait returns at once.
        let Some(device) = self.device.take() else { return };
        if let Some(last) = self.last {
            let _ = device.wait_timeline(last, PRESENT_WAIT);
        }
        for (slot, held) in HEAP.iter().zip(&mut self.slots) {
            if held.take().is_some() {
                let _ = device.clear_heap_slot(*slot);
            }
        }
        // The swapchain's drop waits for the submits that use its images.
        drop(self.swapchain.take());
        let _ = device.destroy_graphics_pipeline(self.pipeline);
        let _ = device.destroy_timeline(self.reads);
    }
}
