//! The present side (`crates/app/README.md`, "What `lactose-app-client` wires", step 5): the
//! swapchain, held by `lactose_platform::present_thread`'s lane, drawing each finished
//! target into an acquired image with a pass that samples it (rhi-api, "The present
//! thread"), and the count the frame sheds by. `tests/shown.rs`, section 2, of `lactose-app`
//! is its sketch.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
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
/// waited for that call's read, or, for a new image, which waits for no read, the side waits
/// for that read itself before the slot's write; and the renderer's two regions open a frame
/// only once the point of the frame two submits before it is reached. So where every call
/// presents, three reads at most are pending as a call begins, and the call's own encoder is
/// the fourth.
const ENCODERS: usize = 4;

/// The passes each present of a side made after it adds ([`set_extra_passes`]). It is the
/// process's: a side `wire` makes after a `run` in the same process takes that run's value.
static EXTRA_PASSES: AtomicU32 = AtomicU32::new(0);

/// Makes each present of a side made after this call draw `passes` more passes over its
/// image, each loading and storing the whole image behind a barrier, which leaves the image
/// as it was and each present's read later: `Options::extra_present_passes`, a test and
/// measurement knob. A side reads it once, as it is made.
pub(crate) fn set_extra_passes(passes: u32) {
    EXTRA_PASSES.store(passes, Ordering::Relaxed);
}

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
/// whose drop waits for its submit and so for the frame's GPU work. [`ENCODERS`] at most in
/// all: one is made only where none is free and fewer are submitted.
///
/// A kept encoder holds what it recorded until it is reset: the target and the swapchain
/// image it drew, and through that image its swapchain, a retired one after a resize among
/// them (`lactose-rhi-api`'s README, "Handles and lifetimes"). So every one whose read is
/// reached is reset as a call begins ([`settle`](Self::settle)).
#[derive(Default)]
struct Encoders {
    /// Those given to a submit, each with the value its read signals, oldest first.
    submitted: VecDeque<(u64, Box<dyn Encoder>)>,
    /// Those reset and not submitted since.
    free: Vec<Box<dyn Encoder>>,
    /// The resets of submitted encoders whose read was reached that the backend refused,
    /// which `close` logs: none where `reached` comes from the device's own reads, which a
    /// backend has folded before it answers.
    refused: u64,
}

impl Encoders {
    /// Whether the oldest submitted one's read is beyond `reached`.
    fn pending(&self, reached: u64) -> bool {
        self.submitted.front().is_some_and(|&(read, _)| read > reached)
    }

    /// Resets every submitted encoder whose read is `reached` or less, oldest first, and
    /// keeps it free. One whose reset is refused stays submitted, counted, and the next call's
    /// settle tries it again: a backend that has not yet seen its submit complete refuses it as
    /// pending.
    fn settle(&mut self, reached: u64) {
        while let Some((read, mut encoder)) =
            self.submitted.pop_front_if(|&mut (read, _)| read <= reached)
        {
            if encoder.reset().is_err() {
                self.refused += 1;
                self.submitted.push_front((read, encoder));
                return;
            }
            self.free.push(encoder);
        }
    }

    /// The read a call waits for before it takes an encoder: the oldest one's, where it is
    /// beyond `reached` and the side holds its bound, all of them submitted.
    fn behind(&self, reached: u64) -> Option<u64> {
        if self.submitted.len() < ENCODERS {
            return None;
        }
        self.submitted.front().map(|&(read, _)| read).filter(|&read| read > reached)
    }

    /// An encoder to record into: a free one, once those whose read is `reached` or less are
    /// settled; else the oldest submitted, reset, where the side holds its bound; else a new
    /// one.
    ///
    /// # Errors
    ///
    /// The device's; the reset's, which keeps the encoder it refused: refused as pending
    /// (`Rule::Pending`) where the oldest one's read is not reached.
    fn take(&mut self, device: &dyn Device, reached: u64) -> Result<Box<dyn Encoder>> {
        self.settle(reached);
        if let Some(encoder) = self.free.pop() {
            return Ok(encoder);
        }
        if self.submitted.len() >= ENCODERS
            && let Some((read, mut encoder)) = self.submitted.pop_front()
        {
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

    /// `encoder`, back from a present that failed: reset at once and kept free. A present's
    /// errors are a refusal, which submitted nothing, and a lost device, whose encoders
    /// reset (rhi-api, `Swapchain::present` and `Encoder::reset`), so neither backend
    /// refuses the reset; were one refused, the encoder would be dropped, and its drop would
    /// wait for whatever it submitted.
    fn unsubmitted(&mut self, mut encoder: Box<dyn Encoder>) {
        if encoder.reset().is_ok() {
            self.free.push(encoder);
        }
    }

    /// Every encoder reset, which a lost device accepts, and dropped, so that no drop waits.
    fn lost(&mut self) {
        self.free.clear();
        for (_, mut encoder) in self.submitted.drain(..) {
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
    /// The read of the last present that declared each heap slot, which a new image's
    /// write of the slot waits for.
    declared: [Option<TimelinePoint>; TARGETS],
    waiting: Waiting,
    encoders: Encoders,
    /// The passes each present adds over its image ([`set_extra_passes`]).
    extra_passes: u32,
    /// The calls made, and those of them that began with their oldest encoder's read not
    /// reached, which `close` logs.
    calls: u64,
    pending_at_head: u64,
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
            declared: [None; TARGETS],
            waiting: Waiting::new(),
            encoders: Encoders::default(),
            extra_passes: EXTRA_PASSES.load(Ordering::Relaxed),
            calls: 0,
            pending_at_head: 0,
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
        self.calls += 1;
        if self.encoders.pending(reached) {
            self.pending_at_head += 1;
        }
        // The resets are the record's part of the call, as the reset behind the bound is.
        let encoders = &mut self.encoders;
        timed(&mut self.timed.record, || encoders.settle(reached));
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
        // A slot is written only once no pending pass declares it: only this side's present
        // passes declare it, and the read of the last that did covers every earlier one. Not
        // the last read: the other target's present renews that each call, so with the GPU
        // behind by more than the bound a slot that waited for it would never be written. A
        // new image can come with no resize: an extent that changed and came back, the frame
        // between dropped or torn.
        let slot = finished.target.index();
        if self.slots[slot] != Some(finished.image) {
            if let Some(declared) = self.declared[slot] {
                if !timed(&mut self.timed.waits, || device.wait_timeline(declared, PRESENT_WAIT))? {
                    return Ok(nothing);
                }
                reached = reached.max(declared.value);
            }
            device.write_heap_slot(HEAP[slot], finished.image)?;
            self.slots[slot] = Some(finished.image);
        }
        // At its bound the side waits for its oldest present's read before the acquire, so
        // that no image is held through the wait; the take then resets that encoder, as it
        // does the oldest at the bound.
        if let Some(value) = self.encoders.behind(reached) {
            let oldest = TimelinePoint { timeline: self.reads, value };
            if !timed(&mut self.timed.waits, || device.wait_timeline(oldest, PRESENT_WAIT))? {
                return Ok(nothing);
            }
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
        let (pipeline, sampler, extra) = (self.pipeline, self.sampler, self.extra_passes);
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
            draw_target(&mut *encoder, pipeline, HEAP[slot], sampler, image, extent, extra);
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
                self.declared[slot] = Some(read);
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
                self.encoders.unsubmitted(encoder);
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

/// The pass that samples the target in `slot` over the whole of the acquired `image`, then
/// `extra` more that load what it stored, each behind a barrier, which leave `image` as it
/// was ([`set_extra_passes`]); it leaves `image` in `Present`.
fn draw_target(
    encoder: &mut dyn Encoder,
    pipeline: GraphicsPipelineHandle,
    slot: HeapSlot,
    sampler: SamplerIndex,
    image: ImageHandle,
    extent: Extent,
    extra: u32,
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
    let pass = |encoder: &mut dyn Encoder, label, colour: &[ColourAttachment]| {
        encoder.begin_render_pass(&RenderPass {
            label,
            colour,
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
    };
    pass(encoder, "present", &colour);
    let kept = [ColourAttachment { load: Load::Keep, ..colour[0] }];
    for _ in 0..extra {
        encoder.transition_images(&[ImageTransition {
            image,
            range: ImageRange::FIRST,
            from: on(ImageState::ColourAttachment, present),
            to: on(ImageState::ColourAttachment, present),
        }]);
        pass(encoder, "present-extra", &kept);
    }
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
        // A bounded wait for the last read, then the encoders' drop: past the bound a pending
        // one's drop waits for its submit, as a call's did before it kept them (`README.md`,
        // `ClientPresent`), and on a lost device for nothing; on a lost device the wait
        // returns at once. Then the heap slots this side wrote are emptied, behind every
        // submit, so the render side's drop may destroy the targets; a clear the device
        // refuses all the same leaves its slot, and the device then refuses the destroy of
        // the target it holds, which is freed with the device. Nothing here retries; each
        // refusal and error is logged. The swapchain's drop is bounded (`lactose-rhi-vulkan`'s
        // README, "The swapchain").
        let Some(device) = self.device.take() else { return };
        tracing::info!(
            target: crate::LOG_TARGET,
            "the present side's close: {} calls, {} of them began with their oldest read pending, \
             {} resets of a reached encoder refused",
            self.calls,
            self.pending_at_head,
            self.encoders.refused,
        );
        if let Some(last) = self.last
            && let Err(error) = device.wait_timeline(last, PRESENT_WAIT)
        {
            warn("the wait for the last read", &error);
        }
        drop(core::mem::take(&mut self.encoders));
        for (slot, held) in HEAP.iter().zip(&mut self.slots) {
            if held.take().is_some()
                && let Err(error) = device.clear_heap_slot(*slot)
            {
                warn("a heap slot's clear", &error);
            }
        }
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

/// The present encoders on `lactose-rhi-null`, whose reset is refused until the caller has
/// seen the submit's point. What a kept encoder holds of a swapchain null does not show: its
/// images hold no swapchain, and its destroys are refused by pending submits, not by an
/// encoder that recorded the image. So this shows that each reached one is reset, and
/// `lactose-rhi-vulkan` alone what a reset lets go.
#[cfg(test)]
mod encoders {
    use std::sync::Arc;

    use lactose_rhi_api::{
        Acquired, Capabilities, Device, DeviceRequest, Extent, HEAP_SLOTS_DEFAULT, ImageRange,
        ImageState, ImageTransition, Message, MessageSink, QueueRole, Rule, SAMPLERS_DEFAULT,
        Submit, Swapchain, SwapchainDesc, TimelineHandle, TimelinePoint,
    };
    use lactose_rhi_null::{NullDevice, NullSurface};

    use super::{ENCODERS, Encoders, on};

    struct Silent;

    impl MessageSink for Silent {
        fn message(&self, message: &Message<'_>) {
            panic!("null said {message:?}");
        }
    }

    fn device() -> NullDevice {
        let request = DeviceRequest {
            adapter: None,
            surface: Some(Arc::new(NullSurface)),
            heap_slots: HEAP_SLOTS_DEFAULT,
            samplers: SAMPLERS_DEFAULT,
            pipeline_cache: Vec::new(),
            messages: Arc::new(Silent),
        };
        NullDevice::new(request, Capabilities::LEAST).expect("a null device")
    }

    /// Encoders taken and given to `n` presents in turn, which signal reads `1..=n`, none of
    /// them seen; with the swapchain, which must outlive them.
    fn submitted(device: &NullDevice, n: u64) -> (Encoders, TimelineHandle, Box<dyn Swapchain>) {
        let reads = device.create_timeline(0).expect("a timeline");
        let desc = SwapchainDesc { extent: Extent { width: 8, height: 8 } };
        let mut swapchain = device.create_swapchain(&desc).expect("a swapchain");
        let mut encoders = Encoders::default();
        for value in 1..=n {
            let Acquired::Image(image) = swapchain.acquire().expect("an acquire") else {
                panic!("no image to present {value}");
            };
            let mut encoder = encoders.take(device, 0).expect("an encoder");
            encoder.transition_images(&[ImageTransition {
                image,
                range: ImageRange::FIRST,
                from: on(ImageState::Undefined, QueueRole::Present),
                to: on(ImageState::Present, QueueRole::Present),
            }]);
            let mut taken = [encoder];
            let signals = [TimelinePoint { timeline: reads, value }];
            let submit = Submit { encoders: &mut taken, waits: &[], signals: &signals };
            swapchain.present(image, submit).expect("a present");
            let [encoder] = taken;
            encoders.submitted(value, encoder);
        }
        (encoders, reads, swapchain)
    }

    /// Of three submitted, the two whose reads are reached are reset and kept free at once,
    /// not the oldest alone (`settle-front-only`), and the next takes are theirs: none is
    /// made. The third stays submitted behind its read. Before null has seen the reads, the
    /// settle's resets are refused and every encoder stays submitted
    /// (`settle-keeps-refused-free`), each settle's refusal counted once, where it stops.
    #[test]
    fn every_encoder_whose_read_is_reached_is_reset_and_kept_free() {
        let device = device();
        let (mut encoders, reads, _swapchain) = submitted(&device, 3);
        assert_eq!(encoders.submitted.len(), 3);
        assert!(encoders.pending(0));
        encoders.settle(2);
        assert_eq!((encoders.free.len(), encoders.submitted.len()), (0, 3), "unseen");
        assert_eq!(encoders.refused, 1);
        // Null's reset is refused until the point is seen: `timeline_value` sees it.
        assert_eq!(device.timeline_value(reads).expect("a read"), 3);
        encoders.settle(2);
        assert_eq!((encoders.free.len(), encoders.submitted.len()), (2, 1));
        assert_eq!(encoders.refused, 1, "no refusal once seen");
        assert!(encoders.pending(2));
        assert!(!encoders.pending(3));
        for _ in 0..2 {
            drop(encoders.take(&device, 2).expect("a free encoder"));
        }
        assert_eq!((encoders.free.len(), encoders.submitted.len()), (0, 1));
    }

    /// Free and submitted together, no more than `ENCODERS` are held: at the bound with
    /// none free, the oldest is reset and taken rather than a new one made, and refused as
    /// pending where its read is not seen, so that the caller waits for it first. The refused
    /// one is kept, not dropped (`take-drops-on-failed-reset`), whose drop would wait for its
    /// submit.
    #[test]
    fn no_more_than_the_bound_is_held_in_all() {
        let device = device();
        let n = u64::try_from(ENCODERS).expect("four");
        let (mut encoders, reads, _swapchain) = submitted(&device, n);
        let refused = encoders.take(&device, 0).err().expect("the oldest, pending");
        assert_eq!(Rule::of(&refused), Some(Rule::Pending));
        assert_eq!(encoders.submitted.len(), ENCODERS, "the refused one is kept");
        assert_eq!(device.timeline_value(reads).expect("a read"), 4);
        let taken = encoders.take(&device, 1).expect("the oldest, reset");
        assert_eq!(encoders.submitted.len() + encoders.free.len() + 1, ENCODERS);
        drop(taken);
    }
}
