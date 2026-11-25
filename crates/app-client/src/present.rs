//! The present side (`crates/app/README.md`, "What `lactose-app-client` wires", step 5): the
//! swapchain, held by `lactose_platform::present_thread`'s lane, drawing each finished
//! target into an acquired image with a pass that samples it (rhi-api, "The present
//! thread"), and the count the frame sheds by. `tests/shown.rs`, section 2, of `lactose-app`
//! is its sketch.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_app::{Finished, PresentReport, PresentSide, TARGETS};
use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    Acquired, AddressMode, ColourAttachment, ColourTargetDesc, Cull, Device, DisplayTime, Encoder,
    Extent, Filter, GraphicsPipelineDesc, GraphicsPipelineHandle, HeapSlot, ImageHandle,
    ImageRange, ImageState, ImageTransition, Load, PassReach, PresentId, Presented, QueueRole,
    RenderPass, Rule, SamplerDesc, SamplerIndex, StageDesc, StateOn, Store, Submit, Swapchain,
    Target, TimelineHandle, TimelinePoint, WriteMask,
};

use crate::shaders::CookedShaders;
use crate::stamps::{self, PresentStamp, Stamp};

/// The bound the present side gives its own waits, on the present thread alone.
const PRESENT_WAIT: Duration = Duration::from_millis(100);

/// The heap slots the present pass samples the targets through, one a target: the device's
/// first two, which the client reserves for the present side. The heap's slots are the
/// device's to give out to every user of one device, and milestone 2 has no allocator for
/// them; the renderer writes no slot at milestone 2, and a user of the heap beside this side
/// takes slots from the third on (`README.md`, "Rules a caller should know").
pub const HEAP: [HeapSlot; TARGETS] = [HeapSlot(0), HeapSlot(1)];

/// How many present encoders the side holds, the bound at which a call waits, within
/// `PRESENT_WAIT`, for the oldest one's read before it records. A present waits on the device
/// for its frame's point; that frame, which drew the target the call before last named,
/// waited for that call's read; and the renderer's two regions open a frame only once the
/// point of the frame two submits before it is reached. So where every call presents, three
/// reads at most are pending as a call begins, and the call's own encoder is the fourth.
const ENCODERS: usize = 4;

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
struct Waiting {
    /// The presents made and not yet known displayed, each with the value its `read`
    /// signals, oldest first.
    made: VecDeque<(PresentId, u64)>,
}

impl Waiting {
    /// No present made.
    fn new() -> Self {
        Self::default()
    }

    /// The present `present` was made, its read signalling `read` on the side's timeline.
    fn presented(&mut self, present: PresentId, read: u64) {
        self.made.push_back((present, read));
    }

    /// The count as a call begins: `None` where the swapchain has no source of display
    /// times (`source`), and every present made is forgotten; otherwise the presents whose
    /// read value is `reached` or less, the reads' timeline as one read of it gave it, of
    /// those that `displayed`, the reports since the last call, has not named and that no
    /// later named present follows.
    fn take(&mut self, source: bool, displayed: &[DisplayTime], reached: u64) -> Option<u32> {
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
    #[cfg(test)]
    fn kept(&self) -> usize {
        self.made.len()
    }
}

/// The present encoders the side holds, each its own pool (`lactose-rhi-api`'s README,
/// "Recording and submitting"), so that a call neither makes one nor drops one that is pending,
/// whose drop waits for its submit and so for the frame's GPU work. [`ENCODERS`] at most.
#[derive(Default)]
struct Encoders {
    /// Those given to a submit, each with the value its read signals, oldest first.
    submitted: VecDeque<(u64, Box<dyn Encoder>)>,
    /// One reset and not submitted since.
    spare: Option<Box<dyn Encoder>>,
}

impl Encoders {
    /// The read a call waits for before it takes an encoder: the oldest one's, where the
    /// side holds its bound, none spare and none whose read is `reached` or less.
    fn behind(&self, reached: u64) -> Option<u64> {
        if self.spare.is_some() || self.submitted.len() < ENCODERS {
            return None;
        }
        self.submitted.front().map(|&(read, _)| read).filter(|&read| read > reached)
    }

    /// An encoder to record into: the spare; else the oldest submitted, reset, where its
    /// read is `reached` or less or the side holds its bound; else a new one.
    ///
    /// # Errors
    ///
    /// The device's; the reset's, which keeps the encoder it refused: refused as pending
    /// (`Rule::Pending`) where the oldest one's read is not reached.
    fn take(&mut self, device: &dyn Device, reached: u64) -> Result<Box<dyn Encoder>> {
        if let Some(encoder) = self.spare.take() {
            return Ok(encoder);
        }
        let reuse = self.submitted.len() >= ENCODERS
            || self.submitted.front().is_some_and(|&(read, _)| read <= reached);
        if reuse && let Some((read, mut encoder)) = self.submitted.pop_front() {
            return match encoder.reset() {
                Ok(()) => Ok(encoder),
                Err(error) => {
                    self.submitted.push_front((read, encoder));
                    Err(error)
                }
            };
        }
        device.create_encoder(QueueRole::Present, "present")
    }

    /// `encoder`, taken by a present whose submit signals `read`.
    fn submitted(&mut self, read: u64, encoder: Box<dyn Encoder>) {
        self.submitted.push_back((read, encoder));
    }

    /// `encoder`, back from a present that failed, which submitted nothing unless the device
    /// is lost (`Swapchain::present`): reset at once and kept spare. Kept behind `read` where
    /// the reset is refused as pending, as only a submit made would be; dropped where the
    /// reset fails otherwise, which leaves nothing of it pending.
    fn unsubmitted(&mut self, read: u64, mut encoder: Box<dyn Encoder>) {
        match encoder.reset() {
            Ok(()) => self.spare = Some(encoder),
            Err(error) if Rule::of(&error) == Some(Rule::Pending) => self.submitted(read, encoder),
            Err(_) => {}
        }
    }

    /// Every encoder reset, which a lost device accepts, and dropped, so that no drop waits.
    fn lost(&mut self) {
        let spare = self.spare.take();
        for (_, mut encoder) in self.submitted.drain(..).chain(spare.map(|spare| (0, spare))) {
            // A lost device's encoders reset (`Encoder::reset`): nothing is left to fail.
            let _ = encoder.reset();
        }
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
    encoders: Encoders,
    times: Vec<DisplayTime>,
    /// Where a run takes stamps, the call's timings so far, and the image count last stamped.
    timed: Timed,
    images: Option<u32>,
}

/// A present call's timings (`stamps::PresentStamp`), each zero where it did not happen.
#[derive(Clone, Copy, Default)]
struct Timed {
    waits: Duration,
    acquire: Duration,
    record: Duration,
    driver: Duration,
    /// The driver call's start.
    driver_at: Option<Instant>,
    id: Option<u64>,
    /// `Swapchain::display_times`'s answer as the call began.
    times: bool,
}

/// Runs `f`, its time added to `into` where a run takes stamps.
fn timed<R>(into: &mut Duration, f: impl FnOnce() -> R) -> R {
    let Some(start) = stamps::now() else { return f() };
    let returned = f();
    *into += start.elapsed();
    returned
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
            encoders: Encoders::default(),
            times: Vec::new(),
            timed: Timed::default(),
            images: None,
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
        let mut reached = device.timeline_value(self.reads)?;
        let waiting = self.waiting.take(source, &self.times, reached);
        if stamps::on() {
            self.timed.times = source;
            for time in &self.times {
                stamps::stamp(Stamp::Displayed { id: time.present.0, at: time.shown });
            }
        }
        self.times.clear();
        let nothing = PresentReport { waiting, ..NOTHING };
        if self.stale || self.asked != Some(finished.extent) {
            // The caller's wait before a resize is for its own points (rhi-api, "The
            // swapchain"). A timeout or a refusal as pending is the next call's to retry.
            if let Some(last) = self.last {
                if !timed(&mut self.timed.waits, || device.wait_timeline(last, PRESENT_WAIT))? {
                    return Ok(nothing);
                }
                reached = last.value;
            }
            match timed(&mut self.timed.waits, || swapchain.resize(finished.extent)) {
                Ok(()) => {}
                Err(error) if Rule::of(&error) == Some(Rule::Pending) => return Ok(nothing),
                Err(error) => {
                    // A driver's error has given up the acquired image (rhi-api, `resize`):
                    // the next call acquires anew.
                    self.acquired = None;
                    return Err(error);
                }
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
            if let Some(last) = self.last {
                if !timed(&mut self.timed.waits, || device.wait_timeline(last, PRESENT_WAIT))? {
                    return Ok(nothing);
                }
                reached = last.value;
            }
            device.write_heap_slot(HEAP[slot], finished.image)?;
            self.slots[slot] = Some(finished.image);
        }
        // At its bound the side waits for its oldest present's read before the acquire, so
        // that no image is held through the wait.
        if let Some(value) = self.encoders.behind(reached) {
            let oldest = TimelinePoint { timeline: self.reads, value };
            if !timed(&mut self.timed.waits, || device.wait_timeline(oldest, PRESENT_WAIT))? {
                return Ok(nothing);
            }
            reached = value;
        }
        let image = match self.acquired.take() {
            Some(image) => image,
            None => match timed(&mut self.timed.acquire, || swapchain.acquire())? {
                Acquired::Image(image) => image,
                Acquired::None => return Ok(nothing),
                Acquired::Stale => {
                    self.stale = true;
                    return Ok(nothing);
                }
            },
        };
        let extent = swapchain.extent();
        let (pipeline, sampler) = (self.pipeline, self.sampler);
        let encoders = &mut self.encoders;
        let recorded = timed(&mut self.timed.record, || {
            let mut encoder = encoders.take(&**device, reached)?;
            // The acquire half of the hand-off the frame's submit released, with equal
            // values; where one queue serves both roles it is the whole change (graph,
            // "Imports").
            encoder.transition_images(&[ImageTransition {
                image: finished.image,
                range: ImageRange::FIRST,
                from: on(ImageState::ColourAttachment, QueueRole::Graphics),
                to: on(ImageState::SampledRead, QueueRole::Present),
            }]);
            draw_target(&mut *encoder, pipeline, HEAP[slot], sampler, image, extent);
            Ok(encoder)
        });
        let encoder = match recorded {
            Ok(encoder) => encoder,
            Err(error) => {
                self.keep_unless_lost(image, &error);
                return Err(error);
            }
        };
        let read = TimelinePoint { timeline: self.reads, value: self.next };
        let mut encoders = [encoder];
        let submit = Submit { encoders: &mut encoders, waits: &[finished.point], signals: &[read] };
        self.timed.driver_at = stamps::now();
        let presented = swapchain.present(image, submit);
        if let Some(at) = self.timed.driver_at {
            self.timed.driver = at.elapsed();
        }
        let [encoder] = encoders;
        let report = match presented {
            Ok(presented) => {
                self.encoders.submitted(read.value, encoder);
                self.next += 1;
                self.last = Some(read);
                self.stale = presented == Presented::Stale;
                if let Some(present) = swapchain.last_present() {
                    if stamps::on() {
                        self.timed.id = Some(present.0);
                    }
                    self.waiting.presented(present, read.value);
                }
                Ok(PresentReport {
                    read: Some(read),
                    shown: presented == Presented::Shown,
                    waiting,
                })
            }
            // After a refusal nothing was submitted and the image is still the acquired one
            // (rhi-api, `Swapchain::present`): the frame's release of the target stays
            // unmet, and the frame takes the target back from `Undefined` behind the read it
            // already knows. A refusal as pending is the next call's to retry; any other
            // fails the frame.
            Err(error) => {
                self.encoders.unsubmitted(read.value, encoder);
                self.keep_unless_lost(image, &error);
                if Rule::of(&error) == Some(Rule::Pending) { Ok(nothing) } else { Err(error) }
            }
        };
        // The encoder is kept, not dropped: a pending encoder's drop waits for its submit,
        // which waits for the frame's point (`lactose-rhi-api`'s README, "Handles and
        // lifetimes"). So no wait of the call's is for this frame's GPU work or the blit; the
        // one at the bound is for the read of the present `ENCODERS` calls before.
        report
    }

    /// Keeps `image`, the acquired one, for the next call after `error` ended this one with
    /// nothing presented, unless the device is lost: every error between the acquire and a
    /// present taken leaves the image acquired.
    fn keep_unless_lost(&mut self, image: ImageHandle, error: &Error) {
        if error.kind() != ErrorKind::DeviceLost {
            self.acquired = Some(image);
        }
    }

    /// Lets the encoders go where `report` says the device is lost.
    fn forget_if_lost(&mut self, report: &Result<PresentReport>) {
        if report.as_ref().is_err_and(|error| error.kind() == ErrorKind::DeviceLost) {
            self.encoders.lost();
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
        let Some(began) = stamps::now() else {
            let report = self.call(&device, &mut *swapchain, finished);
            self.forget_if_lost(&report);
            self.swapchain = Some(swapchain);
            return report;
        };
        self.timed = Timed::default();
        let point = finished.point.value;
        let report = self.call(&device, &mut *swapchain, finished);
        let returned = Instant::now();
        self.forget_if_lost(&report);
        let Timed { waits, acquire, record, driver, driver_at, id, times } = self.timed;
        let (waiting, shown) = report.as_ref().map_or((None, false), |r| (r.waiting, r.shown));
        stamps::stamp(Stamp::Present(PresentStamp {
            began,
            waits,
            acquire,
            record,
            driver,
            driver_at,
            returned,
            waiting,
            source: times,
            point,
            id,
            shown,
        }));
        let images = swapchain.image_count();
        if self.images != Some(images) {
            self.images = Some(images);
            stamps::stamp(Stamp::Images(images));
        }
        self.swapchain = Some(swapchain);
        report
    }

    fn close(&mut self) {
        // The heap slots this side wrote are emptied first, behind a bounded wait for its
        // last read, so the render side's drop may destroy the targets. Past the bound the
        // device refuses the clear (`uuwp-used`) and the slot is left: the device then
        // refuses the destroy of the target it holds, and that image is freed with the
        // device. Nothing here retries; each refusal and error is logged. On a lost device
        // the wait returns at once.
        let Some(device) = self.device.take() else { return };
        if let Some(last) = self.last
            && let Err(error) = device.wait_timeline(last, PRESENT_WAIT)
        {
            warn("the wait for the last read", &error);
        }
        for (slot, held) in HEAP.iter().zip(&mut self.slots) {
            if held.take().is_some()
                && let Err(error) = device.clear_heap_slot(*slot)
            {
                warn("a heap slot's clear", &error);
            }
        }
        // Then the encoders: past the bound a pending one's drop waits for its submit, as a
        // call's did before it kept them (`README.md`, `ClientPresent`), and on a lost device
        // for nothing. The swapchain's drop waits for the submits that use its images.
        drop(core::mem::take(&mut self.encoders));
        drop(self.swapchain.take());
        if let Err(error) = device.destroy_graphics_pipeline(self.pipeline) {
            warn("the present pipeline's destroy", &error);
        }
        if let Err(error) = device.destroy_sampler(self.sampler) {
            warn("the sampler's destroy", &error);
        }
        if let Err(error) = device.destroy_timeline(self.reads) {
            warn("the reads' timeline's destroy", &error);
        }
    }
}

fn warn(what: &str, error: &lactose_core::Error) {
    tracing::warn!(target: crate::LOG_TARGET, "the present side's close: {what} failed: {error}");
}

/// The present side's count, `PresentReport::waiting`, over scripted display-time reports
/// and timeline values (`crates/app/README.md`, "The shedding rule", *The signal*).
#[cfg(test)]
mod tests {
    use std::time::Instant;

    use lactose_rhi_api::{DisplayTime, PresentId};

    use super::Waiting;

    fn shown(present: u64) -> DisplayTime {
        DisplayTime { present: PresentId(present), shown: Instant::now() }
    }

    /// Presents 1 to `n`, the read of present `k` signalling `k`.
    fn made(n: u64) -> Waiting {
        let mut waiting = Waiting::new();
        for k in 1..=n {
            waiting.presented(PresentId(k), k);
        }
        waiting
    }

    #[test]
    fn no_source_of_display_times_counts_nothing_and_keeps_nothing() {
        let mut waiting = made(5);
        assert_eq!(waiting.take(false, &[], 5), None);
        assert_eq!(waiting.kept(), 0);
        // Reports that arrive with no source are not read.
        let mut waiting = made(5);
        assert_eq!(waiting.take(false, &[shown(5)], 5), None);
    }

    #[test]
    fn a_present_still_behind_the_gpu_is_not_counted() {
        let mut waiting = made(4);
        // Reads 1 and 2 reached, 3 and 4 not: two finished, none displayed.
        assert_eq!(waiting.take(true, &[], 2), Some(2));
        assert_eq!(waiting.take(true, &[], 4), Some(4));
        assert_eq!(waiting.kept(), 4);
    }

    #[test]
    fn a_displayed_present_covers_itself_and_every_earlier_one() {
        let mut waiting = made(5);
        // Present 3 named displayed: 1 to 3 are gone, whether or not 1 and 2 were named.
        assert_eq!(waiting.take(true, &[shown(3)], 5), Some(2));
        assert_eq!(waiting.kept(), 2);
        // Reports since the last call only: none, and the two remain.
        assert_eq!(waiting.take(true, &[], 5), Some(2));
        // Several reports in one call, oldest first: the newest decides.
        assert_eq!(waiting.take(true, &[shown(4), shown(5)], 5), Some(0));
        assert_eq!(waiting.kept(), 0);
    }

    #[test]
    fn the_count_rises_one_a_frame_while_the_display_takes_nothing() {
        let mut waiting = Waiting::new();
        let mut counts = Vec::new();
        for k in 1..=4_u64 {
            // A call begins: every earlier read reached, nothing displayed.
            counts.push(waiting.take(true, &[], k - 1));
            waiting.presented(PresentId(k), k);
        }
        assert_eq!(counts, [Some(0), Some(1), Some(2), Some(3)]);
        // The display takes present 4: nothing is left waiting.
        assert_eq!(waiting.take(true, &[shown(4)], 4), Some(0));
    }

    #[test]
    fn a_report_of_a_present_never_made_here_drops_nothing_later() {
        let mut waiting = Waiting::new();
        waiting.presented(PresentId(7), 1);
        waiting.presented(PresentId(8), 2);
        // An id below every kept one changes nothing.
        assert_eq!(waiting.take(true, &[shown(6)], 2), Some(2));
        assert_eq!(waiting.kept(), 2);
    }
}
