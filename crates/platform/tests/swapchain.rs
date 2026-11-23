//! The swapchain on a window, beside the pump (`README.md`, "Proof"): a device made in a
//! main-lane call, its swapchain made, resized and dropped on the present thread, frames
//! presented across a resize with the swapchain image read back, the display-time report,
//! and the drawable of a minimised window. Each case makes a `lactose-rhi-vulkan` device on
//! the driver the loader finds, and skips, naming why, where that driver makes no swapchain
//! on the window.

use core::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lactose_core::{ErrorKind, Result};
use lactose_platform::{
    ClientPlatform, MainThread, PixelSize, PresentThread, WindowReports, event_pump, present_thread,
};
use lactose_rhi_api::{
    Acquired, BufferDesc, BufferHandle, BufferImageCopy, BufferState, BufferTransition,
    BufferUsage, ColourAttachment, Device, DeviceRequest, Encoder, Extent, Format,
    HEAP_SLOTS_DEFAULT, ImageDesc, ImageHandle, ImageMemory, ImageRange, ImageState,
    ImageTransition, ImageUsage, Load, MemoryClass, Message, MessageSink, PassReach, PresentId,
    Presented, QueuePath, QueueRole, Rect, RenderPass, Rule, SAMPLERS_DEFAULT, Sharing, StateOn,
    Store, Submit, SurfaceSource, Swapchain, SwapchainDesc, Target, TimelineHandle, TimelinePoint,
};
use lactose_rhi_vulkan::VulkanDevice;
use lactose_task::{Executor, Lane, WorkerClass};

use crate::{ANSWER, desc, failed, poll_until, pump_with, wait, wait_within};

/// How long a case waits for what a driver under the validation layer may be slow over: a
/// device made, a point its submit signals, a present whose drawable blocks, which MoltenVK
/// bounds at about a second for each of the swapchain's images and each fetch.
const PATIENCE: Duration = Duration::from_secs(30);
const G: QueueRole = QueueRole::Graphics;
const P: QueueRole = QueueRole::Present;
/// The offscreen target's size, and the rectangle of the swapchain image it is copied into
/// and read back from: small, so that each readback is quick to compare.
const TARGET: Extent = Extent { width: 8, height: 8 };
/// The frames whose targets may be in flight at once, more than the swapchain's images, so
/// that each present waits for an image and not for a target.
const SLOTS: usize = 6;
/// The block of the drawable that row 182 asks the present thread to record.
const BLOCK: Duration = Duration::from_secs(1);
/// MoltenVK's message when no drawable came within its attempts: the minimised window's
/// case exists to provoke it.
const NO_DRAWABLE: &str = "could not be acquired";

/// The driver lines (MoltenVK's `[mvk-error]` and `[mvk-warn]`, Mesa's log, Metal's
/// validation) a case expects on its standard error: the parent fails a case on any other.
pub(crate) fn expected_driver_lines(case: &str) -> &'static [&'static str] {
    if case == "a_minimised_window_is_presented_to_and_the_frame_goes_on" {
        &[NO_DRAWABLE]
    } else {
        &[]
    }
}

// ---- What every case shares ----

/// A sink that prints every message, as `cargo xtask gpu`'s judge reads them, and keeps
/// those that are not the case's expected one.
struct Sink {
    expected: Option<&'static str>,
    heard: AtomicUsize,
    others: Mutex<Vec<String>>,
}

impl Sink {
    fn new(expected: Option<&'static str>) -> Arc<Self> {
        Arc::new(Self { expected, heard: AtomicUsize::new(0), others: Mutex::new(Vec::new()) })
    }

    /// The expected message's count.
    fn heard(&self) -> usize {
        // Relaxed: read after the calls that send it have returned.
        self.heard.load(Ordering::Relaxed)
    }

    fn others(&self) -> Vec<String> {
        self.others.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl MessageSink for Sink {
    fn message(&self, message: &Message<'_>) {
        eprintln!("message: {message:?}");
        if self.expected.is_some_and(|expected| message.text.contains(expected)) {
            self.heard.fetch_add(1, Ordering::Relaxed);
        } else {
            self.others.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{message:?}"));
        }
    }
}

fn request(sink: &Arc<Sink>, window: Arc<dyn SurfaceSource>) -> DeviceRequest {
    DeviceRequest {
        adapter: std::env::var("LACTOSE_GPU_ADAPTER").ok(),
        surface: Some(window),
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages: Arc::clone(sink) as _,
    }
}

/// The device on the pump's window, made in a main-lane call, or `None` where the driver
/// makes none on it, which the case prints as its skip. A device made with the window off
/// the main thread is refused first: the handles are lent there alone, and on macOS the
/// surface's layer is made there alone (row 210).
fn device_on_the_window(
    main: &Lane<MainThread>,
    sink: &Arc<Sink>,
    case: &str,
) -> Result<Option<Arc<VulkanDevice>>> {
    let window = wait(main.run(|main| main.window()))?;
    let off_main = VulkanDevice::new(request(sink, window.clone()));
    let refused = off_main.err().ok_or_else(|| failed("a device was made off the main thread"))?;
    println!("stage: off the main thread the device is refused: {:?}, {refused}", refused.kind());
    drop(window);
    let made = wait_within(
        PATIENCE,
        main.run({
            let sink = Arc::clone(sink);
            move |main| {
                let on = std::thread::current().name().map(str::to_owned);
                (VulkanDevice::new(request(&sink, main.window())), on)
            }
        }),
    )?;
    match made {
        (Ok(device), on) => {
            let info = device.info();
            println!(
                "stage: the device was made in a main-lane call, on {on:?}: {} {}, present {:?}",
                info.adapter,
                info.driver,
                device.capabilities().queues.present
            );
            Ok(Some(Arc::new(device)))
        }
        (Err(error), _) if error.kind() == ErrorKind::Unsupported => {
            println!("{case}: skipped, {error}");
            Ok(None)
        }
        (Err(error), _) => Err(error),
    }
}

/// Whether the device's driver is MoltenVK, whose drawable row 182 is about.
fn moltenvk(device: &VulkanDevice) -> bool {
    device.info().driver.contains("MoltenVK")
}

/// Drops the last of the device in a main-lane call, the shutdown's step 5 (`README.md`,
/// "Shutdown"), where its surface, and on macOS the layer, are dropped: a call made when the
/// swapchain and every encoder are gone, so that this drop is the last holder's.
fn drop_on_the_main_thread(main: &Lane<MainThread>, device: Arc<VulkanDevice>) -> Result<()> {
    let device = Arc::into_inner(device).ok_or_else(|| failed("the device is still shared"))?;
    let on = wait_within(
        PATIENCE,
        main.run(move |_| {
            drop(device);
            std::thread::current().name().map(str::to_owned)
        }),
    )?;
    println!("stage: the device, its surface and its layer were dropped on {on:?}");
    Ok(())
}

/// The colour frame `frame` clears to, in steps of 1/255 so that an 8-bit format keeps it.
fn colour(frame: u64) -> [u8; 4] {
    [(frame * 37 % 256) as u8, (frame * 11 % 256) as u8, (255 - frame % 256) as u8, 255]
}

/// The bytes a target of `format` holds after a clear to `rgba`, or `None` for an sRGB
/// format, whose encoding the case does not reproduce.
fn texel(format: Format, rgba: [u8; 4]) -> Option<[u8; 4]> {
    match format {
        Format::Rgba8Unorm => Some(rgba),
        Format::Bgra8Unorm => Some([rgba[2], rgba[1], rgba[0], rgba[3]]),
        _ => None,
    }
}

fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
    StateOn { state, queue }
}

fn image_to(
    image: ImageHandle,
    from: StateOn<ImageState>,
    to: StateOn<ImageState>,
) -> ImageTransition {
    ImageTransition { image, range: ImageRange::FIRST, from, to }
}

fn buffer_to(
    buffer: BufferHandle,
    from: StateOn<BufferState>,
    to: StateOn<BufferState>,
) -> BufferTransition {
    BufferTransition { buffer, from, to }
}

fn clear(encoder: &mut dyn Encoder, image: ImageHandle, rgba: [u8; 4]) {
    let colour = [ColourAttachment {
        target: Target { image, mip: 0, layer: 0 },
        load: Load::Clear(rgba.map(|byte| f32::from(byte) / 255.0)),
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

fn extent_of(size: PixelSize) -> Extent {
    Extent { width: size.width, height: size.height }
}

fn readback(device: &VulkanDevice, label: &str) -> Result<BufferHandle> {
    device.create_buffer(&BufferDesc {
        label,
        size: u64::from(TARGET.width * TARGET.height * 4),
        usage: BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION),
        memory: MemoryClass::Readback,
        sharing: Sharing::Exclusive,
    })
}

fn read(device: &VulkanDevice, buffer: BufferHandle) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let size = u64::from(TARGET.width * TARGET.height * 4);
    device.read_buffer(buffer, 0, size, &mut |got| bytes.extend_from_slice(got))?;
    Ok(bytes)
}

// ---- The present thread's side ----

/// One frame for the present thread: its target, drawn on the graphics queue to the point
/// `number` of the frames' `drawn` timeline, which its present's submit signals on `shown`.
#[derive(Clone, Copy, Debug)]
struct Job {
    frame: u64,
    /// The submit's number, which only grows: a frame whose acquire gave nothing is drawn
    /// again under the next number.
    number: u64,
    slot: usize,
    target: ImageHandle,
    drawn: TimelineHandle,
    /// The target is exclusive and crosses by a release on the graphics queue and this
    /// acquire on the present queue.
    crossing: bool,
    /// The swapchain is made again as soon as this frame's present returns, with its
    /// presents still in flight: whether the reports of the swapchain it replaces are lost.
    rebuild_after: bool,
}

/// What one present call did.
#[derive(Clone, Copy, Debug)]
struct Call {
    frame: u64,
    started: Instant,
    /// How long the acquires took, the empty ones included, and how many gave nothing.
    acquire: Duration,
    nones: u32,
    /// How long `present` took, and what it gave, where the call presented.
    present: Option<(Duration, Presented, PresentId)>,
    ended: Instant,
}

impl Call {
    /// The longest single block of the present thread in this call.
    fn block(&self) -> Duration {
        self.acquire.max(self.present.map_or(Duration::ZERO, |(took, ..)| took))
    }
}

/// A rebuild of the swapchain the present thread made.
#[derive(Clone, Copy, Debug)]
struct Rebuild {
    frame: u64,
    why: &'static str,
    from: Extent,
    to: Extent,
    /// How many times `resize` was refused for a pending present first: each present
    /// fence it waits for, a failed present's included, signalled within its bound where
    /// this is zero.
    refused: u32,
}

impl Rebuild {
    fn describe(rebuilds: &[Self]) -> String {
        let each = rebuilds.iter().map(|rebuild| {
            let (from, to) = (rebuild.from, rebuild.to);
            format!(
                "before frame {} for {}, {}x{} to {}x{}, refused {} times first",
                rebuild.frame,
                rebuild.why,
                from.width,
                from.height,
                to.width,
                to.height,
                rebuild.refused
            )
        });
        format!("[{}]", each.collect::<Vec<_>>().join("; "))
    }
}

/// One display time as `display_times` gave it.
#[derive(Clone, Copy, Debug)]
struct Report {
    present: PresentId,
    shown: Instant,
    /// When the call that gave it returned.
    received: Instant,
}

/// A present taken, with the swapchain it was taken on, counted from one at each rebuild.
#[derive(Clone, Copy, Debug)]
struct Taken {
    present: PresentId,
    swapchain: u32,
    /// When its present call began and when `present` returned.
    started: Instant,
    returned: Instant,
}

/// What the present thread saw, handed back when it lets the swapchain go.
#[derive(Default, Debug)]
struct Record {
    calls: Vec<Call>,
    rebuilds: Vec<Rebuild>,
    taken: Vec<Taken>,
    reports: Vec<Report>,
    /// How long each `display_times` call took; `None` where it answered `None`.
    report_calls: Vec<Option<Duration>>,
    /// Readbacks compared: the swapchain image's against the target's.
    compared: u32,
}

/// One slot's buffers: the target read on the present queue, and the swapchain image's
/// rectangle the target was copied into, read back; the encoder of its last present, kept
/// until that present's point; and that present's job.
struct Readback {
    target: BufferHandle,
    image: BufferHandle,
    encoder: Option<Box<dyn Encoder>>,
    last: Option<Job>,
}

/// What the present thread holds.
struct Presenter {
    device: Arc<VulkanDevice>,
    window: WindowReports,
    swapchain: Box<dyn Swapchain>,
    format: Format,
    shown: TimelineHandle,
    slots: Vec<Readback>,
    /// The point of the last present taken, which a rebuild waits for first.
    last_shown: u64,
    stale: bool,
    generation: u32,
    record: Record,
}

/// What the present thread says of the swapchain it made.
#[derive(Debug)]
struct Made {
    on: Option<String>,
    format: Format,
    extent: Extent,
    images: u32,
    reports: bool,
}

impl Presenter {
    /// Makes the swapchain at the window's size, on the present thread.
    fn make(
        device: Arc<VulkanDevice>,
        window: WindowReports,
        shown: TimelineHandle,
    ) -> Result<(Self, Made)> {
        let extent = extent_of(window.size());
        let mut swapchain = device.create_swapchain(&SwapchainDesc { extent })?;
        let made = Made {
            on: std::thread::current().name().map(str::to_owned),
            format: swapchain.format(),
            extent: swapchain.extent(),
            images: swapchain.image_count(),
            reports: swapchain.display_times().is_some(),
        };
        let slots = (0..SLOTS)
            .map(|_| {
                Ok(Readback {
                    target: readback(&device, "target readback")?,
                    image: readback(&device, "image readback")?,
                    encoder: None,
                    last: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let presenter = Self {
            device,
            window,
            format: swapchain.format(),
            swapchain,
            shown,
            slots,
            last_shown: 0,
            stale: false,
            generation: 1,
            record: Record::default(),
        };
        Ok((presenter, made))
    }

    /// Compares the slot's last readback, once its present's point is reached: the
    /// swapchain image's rectangle equals the target, and the target its frame's colour.
    fn compare(&mut self, slot: usize) -> Result<()> {
        let Some(job) = self.slots[slot].last.take() else { return Ok(()) };
        let point = TimelinePoint { timeline: self.shown, value: job.number };
        if !self.device.wait_timeline(point, PATIENCE)? {
            return Err(failed(format!("frame {}'s present did not complete", job.frame)));
        }
        self.slots[slot].encoder = None;
        let target = read(&self.device, self.slots[slot].target)?;
        let image = read(&self.device, self.slots[slot].image)?;
        assert_eq!(image, target, "frame {}: the swapchain image is not the target", job.frame);
        if let Some(texel) = texel(self.format, colour(job.frame)) {
            let expected: Vec<u8> = (0..TARGET.width * TARGET.height).flat_map(|_| texel).collect();
            assert_eq!(target, expected, "frame {}: the target's readback", job.frame);
        }
        self.record.compared += 1;
        Ok(())
    }

    /// Makes the swapchain again at the window's size, once the presents' points are
    /// reached; a refusal while a present is pending is retried.
    fn rebuild(&mut self, frame: u64, why: &'static str) -> Result<()> {
        let point = TimelinePoint { timeline: self.shown, value: self.last_shown };
        if !self.device.wait_timeline(point, PATIENCE)? {
            return Err(failed(format!("a rebuild: the present of {point:?} did not complete")));
        }
        // Every present's submit is complete: their encoders, which name the images the
        // rebuild destroys, are let go first, so that none outlives its image.
        for slot in &mut self.slots {
            slot.encoder = None;
        }
        let from = self.swapchain.extent();
        let size = self.window.size();
        let asked = if size.is_empty() { from } else { extent_of(size) };
        let asked = if asked.width == 0 || asked.height == 0 { TARGET } else { asked };
        let until = Instant::now() + PATIENCE;
        let mut refused = 0;
        loop {
            match self.swapchain.resize(asked) {
                Ok(()) => break,
                Err(error) if Rule::of(&error) == Some(Rule::Pending) && Instant::now() < until => {
                    eprintln!("a rebuild was refused, and is retried: {error}");
                    refused += 1;
                }
                Err(error) => return Err(error),
            }
        }
        self.stale = false;
        self.generation += 1;
        self.record.rebuilds.push(Rebuild {
            frame,
            why,
            from,
            to: self.swapchain.extent(),
            refused,
        });
        Ok(())
    }

    /// The next image, rebuilding once where the swapchain is stale; `None` where it is
    /// stale after the rebuild too.
    fn acquire(&mut self, frame: u64, nones: &mut u32) -> Result<Option<ImageHandle>> {
        let mut rebuilt = false;
        let until = Instant::now() + PATIENCE;
        loop {
            match self.swapchain.acquire()? {
                Acquired::Image(image) => return Ok(Some(image)),
                Acquired::None if Instant::now() < until => *nones += 1,
                Acquired::None => return Err(failed(format!("no image within {PATIENCE:?}"))),
                Acquired::Stale if rebuilt => return Ok(None),
                Acquired::Stale => {
                    self.rebuild(frame, "a stale acquire")?;
                    rebuilt = true;
                }
            }
        }
    }

    /// One frame: the slot's last readback compared, the swapchain rebuilt where it is stale
    /// or the window's size is not its extent, then the acquire and the present, whose
    /// submit copies the target into the image and reads both back, and the display times
    /// that arrived.
    fn frame(&mut self, job: Job) -> Result<Call> {
        self.compare(job.slot)?;
        let size = self.window.size();
        if self.stale {
            self.rebuild(job.frame, "a stale present")?;
        } else if !size.is_empty() && extent_of(size) != self.swapchain.extent() {
            self.rebuild(job.frame, "the window's size")?;
        }
        let started = Instant::now();
        let mut nones = 0;
        let image = self.acquire(job.frame, &mut nones)?;
        let acquire = started.elapsed();
        let Some(image) = image else {
            self.stale = true;
            let ended = Instant::now();
            let call = Call { frame: job.frame, started, acquire, nones, present: None, ended };
            self.record.calls.push(call);
            return Ok(call);
        };
        let slot = &self.slots[job.slot];
        let mut encoder = self.device.create_encoder(P, "present")?;
        encode(&mut *encoder, &job, slot, image, self.swapchain.extent());
        let drawn = TimelinePoint { timeline: job.drawn, value: job.number };
        let shown = TimelinePoint { timeline: self.shown, value: job.number };
        let mut encoders = [encoder];
        let before = Instant::now();
        let presented = self.swapchain.present(
            image,
            Submit { encoders: &mut encoders, waits: &[drawn], signals: &[shown] },
        )?;
        let returned = Instant::now();
        let id = self.swapchain.last_present().ok_or_else(|| failed("a present took no id"))?;
        let [encoder] = encoders;
        self.slots[job.slot].encoder = Some(encoder);
        self.slots[job.slot].last = Some(job);
        self.last_shown = job.number;
        self.stale = presented == Presented::Stale;
        self.record.taken.push(Taken {
            present: id,
            swapchain: self.generation,
            started,
            returned,
        });
        self.reports();
        if job.rebuild_after && !self.stale {
            // A swapchain made again with no wait for the display: its submits' points
            // first, as any rebuild, and no more.
            self.rebuild(job.frame + 1, "a rebuild right after a present")?;
        }
        let present = Some((returned - before, presented, id));
        let call =
            Call { frame: job.frame, started, acquire, nones, present, ended: Instant::now() };
        self.record.calls.push(call);
        Ok(call)
    }

    /// The display times that arrived, and how long the call took.
    fn reports(&mut self) {
        let before = Instant::now();
        let times = self.swapchain.display_times();
        let received = Instant::now();
        self.record.report_calls.push(times.as_ref().map(|_| received - before));
        for time in times.into_iter().flatten() {
            self.record.reports.push(Report { present: time.present, shown: time.shown, received });
        }
    }

    /// Waits for the last present, compares every readback left, and drops the swapchain
    /// on the present thread.
    fn finish(mut self) -> Result<Record> {
        for slot in 0..SLOTS {
            self.compare(slot)?;
        }
        self.reports();
        // The last presents reach the display a few refreshes after their submits: their
        // reports are waited for, within a bound, where the device has a source.
        let last = self.record.taken.last().map(|taken| taken.present);
        let until = Instant::now() + Duration::from_secs(1);
        while self.record.report_calls.last().is_some_and(Option::is_some)
            && last.is_some_and(|last| !self.record.reports.iter().any(|r| r.present == last))
            && Instant::now() < until
        {
            std::thread::sleep(Duration::from_millis(5));
            self.reports();
        }
        // No encoder that names an image outlives the swapchain.
        for slot in &mut self.slots {
            slot.encoder = None;
        }
        drop(self.swapchain);
        Ok(self.record)
    }
}

/// The present's commands: the target crossed and read into the slot's first buffer, the
/// image cleared and the target's bytes copied into its corner, that corner read into the
/// slot's second buffer, and the image left in `Present`.
fn encode(encoder: &mut dyn Encoder, job: &Job, slot: &Readback, image: ImageHandle, at: Extent) {
    let rect = Rect { x: 0, y: 0, extent: TARGET };
    let region = [BufferImageCopy { buffer_offset: 0, mip: 0, layer: 0, rect }];
    if job.crossing {
        encoder.transition_images(&[image_to(
            job.target,
            on(ImageState::ColourAttachment, G),
            on(ImageState::CopySource, P),
        )]);
    }
    encoder.transition_buffers(&[
        buffer_to(slot.target, on(BufferState::Undefined, P), on(BufferState::CopyDestination, P)),
        buffer_to(slot.image, on(BufferState::Undefined, P), on(BufferState::CopyDestination, P)),
    ]);
    encoder.copy_image_to_buffer(job.target, slot.target, &region);
    encoder.transition_buffers(&[buffer_to(
        slot.target,
        on(BufferState::CopyDestination, P),
        on(BufferState::CopySource, P),
    )]);
    encoder.transition_images(&[image_to(
        image,
        on(ImageState::Undefined, P),
        on(ImageState::ColourAttachment, P),
    )]);
    // The rest of the image: the frame's colour as well, so that a whole frame is shown.
    clear(encoder, image, colour(job.frame));
    encoder.transition_images(&[image_to(
        image,
        on(ImageState::ColourAttachment, P),
        on(ImageState::CopyDestination, P),
    )]);
    debug_assert!(at.width >= TARGET.width && at.height >= TARGET.height);
    encoder.copy_buffer_to_image(slot.target, image, &region);
    encoder.transition_images(&[image_to(
        image,
        on(ImageState::CopyDestination, P),
        on(ImageState::CopySource, P),
    )]);
    encoder.copy_image_to_buffer(image, slot.image, &region);
    encoder.transition_images(&[image_to(
        image,
        on(ImageState::CopySource, P),
        on(ImageState::Present, P),
    )]);
    encoder.transition_buffers(&[
        buffer_to(slot.target, on(BufferState::CopySource, P), on(BufferState::HostRead, P)),
        buffer_to(slot.image, on(BufferState::CopyDestination, P), on(BufferState::HostRead, P)),
    ]);
}

// ---- The frame's side ----

/// One target, and the encoder of its last graphics submit and what that submit's frame
/// waits for before the target is written again.
struct Slot {
    target: ImageHandle,
    encoder: Option<Box<dyn Encoder>>,
    /// The point after which the target is free: its present's, or where its frame was not
    /// presented, its graphics submit's.
    free: Option<TimelinePoint>,
}

/// The frames, drawn on the graphics queue by the thread that stands where the frame's
/// workers do, and handed to the present thread.
struct Frames {
    device: Arc<VulkanDevice>,
    present: Lane<Option<Presenter>>,
    drawn: TimelineHandle,
    shown: TimelineHandle,
    crossing: bool,
    slots: Vec<Slot>,
    number: u64,
}

impl Frames {
    /// The frames' objects, the swapchain made on the present thread, and what it says.
    fn new(
        device: &Arc<VulkanDevice>,
        present: &Lane<Option<Presenter>>,
        window: WindowReports,
    ) -> Result<(Self, Made)> {
        let drawn = device.create_timeline(0)?;
        let shown = device.create_timeline(0)?;
        let made = wait_within(
            PATIENCE,
            present.run({
                let device = Arc::clone(device);
                move |state: &mut Option<Presenter>| {
                    let (presenter, made) = Presenter::make(device, window, shown)?;
                    *state = Some(presenter);
                    Ok::<_, lactose_core::Error>(made)
                }
            }),
        )??;
        // An exclusive target, which crosses by a release and an acquire where present has
        // a queue of its own: the shape the frame takes (`lactose-rhi-api`'s README).
        let sharing = Sharing::Exclusive;
        let crossing = device.capabilities().queues.present == QueuePath::Own;
        let slots = (0..SLOTS)
            .map(|_| {
                let target = device.create_image(&ImageDesc {
                    label: "offscreen",
                    format: made.format,
                    extent: TARGET,
                    mips: 1,
                    layers: 1,
                    usage: ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE),
                    memory: ImageMemory::Own,
                    sharing,
                })?;
                Ok(Slot { target, encoder: None, free: None })
            })
            .collect::<Result<Vec<_>>>()?;
        let frames = Self {
            device: Arc::clone(device),
            present: present.clone(),
            drawn,
            shown,
            crossing,
            slots,
            number: 0,
        };
        Ok((frames, made))
    }

    /// Draws frame `frame` into its slot's target on the graphics queue and returns its
    /// job, not yet handed to the present thread.
    fn draw(&mut self, frame: u64) -> Result<Job> {
        self.draw_then(frame, false)
    }

    /// As `draw`, the job asking for a rebuild right after its present where `rebuild_after`.
    fn draw_then(&mut self, frame: u64, rebuild_after: bool) -> Result<Job> {
        let at = (frame as usize) % SLOTS;
        if let Some(free) = self.slots[at].free.take()
            && !self.device.wait_timeline(free, PATIENCE)?
        {
            return Err(failed(format!("frame {frame}'s target was not freed by {free:?}")));
        }
        self.slots[at].encoder = None;
        self.number += 1;
        let target = self.slots[at].target;
        let mut encoder = self.device.create_encoder(G, "frame")?;
        encoder.transition_images(&[image_to(
            target,
            on(ImageState::Undefined, G),
            on(ImageState::ColourAttachment, G),
        )]);
        clear(&mut *encoder, target, colour(frame));
        encoder.transition_images(&[image_to(
            target,
            on(ImageState::ColourAttachment, G),
            on(ImageState::CopySource, P),
        )]);
        let drawn = TimelinePoint { timeline: self.drawn, value: self.number };
        let mut encoders = [encoder];
        self.device.submit(G, Submit { encoders: &mut encoders, waits: &[], signals: &[drawn] })?;
        let [encoder] = encoders;
        self.slots[at].encoder = Some(encoder);
        self.slots[at].free = Some(drawn);
        Ok(Job {
            frame,
            number: self.number,
            slot: at,
            target,
            drawn: self.drawn,
            crossing: self.crossing,
            rebuild_after,
        })
    }

    /// Hands a drawn frame to the present thread, and once its call returns, sets what the
    /// target waits for.
    fn present(&mut self, job: Job) -> Result<Call> {
        let call = wait_within(
            PATIENCE * 2,
            self.present.run(move |state: &mut Option<Presenter>| {
                state.as_mut().expect("the swapchain is made").frame(job)
            }),
        )??;
        self.settle(job, &call);
        Ok(call)
    }

    fn settle(&mut self, job: Job, call: &Call) {
        if call.present.is_some() {
            self.slots[job.slot].free =
                Some(TimelinePoint { timeline: self.shown, value: job.number });
        }
    }

    /// Frames until `count` are presented, from frame `first`; returns the next frame.
    fn run(&mut self, first: u64, count: u64) -> Result<u64> {
        self.run_then(first, count, false)
    }

    /// As `run`, each present followed by a rebuild where `rebuild_after`.
    fn run_then(&mut self, first: u64, count: u64, rebuild_after: bool) -> Result<u64> {
        let mut frame = first;
        let mut presented = 0;
        while presented < count {
            let job = self.draw_then(frame, rebuild_after)?;
            if self.present(job)?.present.is_some() {
                presented += 1;
                frame += 1;
            }
        }
        Ok(frame)
    }

    /// The present thread's record, its swapchain dropped there; then every target's
    /// last submit waited for and the frames' objects let go.
    fn finish(mut self) -> Result<Record> {
        let record = wait_within(
            PATIENCE,
            self.present.run(|state: &mut Option<Presenter>| {
                state.take().expect("the swapchain is made").finish()
            }),
        )??;
        for slot in &mut self.slots {
            if let Some(free) = slot.free.take()
                && !self.device.wait_timeline(free, PATIENCE)?
            {
                return Err(failed(format!("a target's last submit, {free:?}, never ended")));
            }
            slot.encoder = None;
        }
        Ok(record)
    }
}

/// Stops the present thread, its state already let go.
fn stop(thread: PresentThread<Option<Presenter>>) -> Result<()> {
    let state = wait(thread.stop())?;
    if state.is_some() {
        return Err(failed("the present thread still held a swapchain"));
    }
    Ok(())
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// The `at`th of `values` in increasing order, `at` in [0, 1].
fn quantile(values: &mut [Duration], at: f64) -> Duration {
    values.sort_unstable();
    let index = ((values.len() - 1) as f64 * at).round() as usize;
    values[index]
}

// ---- The cases ----

/// Row 210 and the resize: the device is made in a main-lane call; the swapchain is made,
/// rebuilt at a resize and dropped on the present thread while the pump runs. A fixed number
/// of frames is presented across the resize, each target drawn on the graphics queue,
/// crossing to the present queue, copied into the swapchain image and read back from it,
/// equal; then one more frame whose swapchain is made again as soon as its present returns,
/// and a few after it. The display-time report is read after each present, and where the
/// device has a source each id is reported once and in order, at a time between its present
/// and its report, by a call within its bound, in a run whose presents are paced by the
/// swapchain's images; the reports of each replaced swapchain that never came are printed.
/// The device is dropped in a main-lane call, and nothing is sent to the sink.
pub(crate) fn a_swapchain_on_the_window_is_made_resized_and_dropped_on_the_present_thread()
-> Result<()> {
    const CASE: &str =
        "a_swapchain_on_the_window_is_made_resized_and_dropped_on_the_present_thread";
    /// Frames on each side of the resize.
    const FRAMES: u64 = 60;
    /// Frames after the rebuild made right after a present.
    const AFTER: u64 = 10;
    const PRESENTS: u64 = 2 * FRAMES + 1 + AFTER;
    /// The bound a `display_times` call returns within (`lactose-rhi-vulkan`'s README).
    const REPORT_BOUND: Duration = Duration::from_millis(5);
    let (main, pump) = event_pump(desc())?;
    let (present, thread) = present_thread(&ClientPlatform::new(), None::<Presenter>)?;
    pump_with(&main, pump, move |main| {
        let sink = Sink::new(None);
        let Some(device) = device_on_the_window(&main, &sink, CASE)? else {
            return stop(thread);
        };
        let reports = wait(main.run(|main| main.window().reports()))?;
        let (mut frames, made) = Frames::new(&device, &present, reports.clone())?;
        println!("stage: the swapchain was made on the present thread: {made:?}");
        assert_eq!(made.on.as_deref(), Some("present"));
        assert_eq!(made.extent, extent_of(reports.size()), "the swapchain is not the window's");
        assert!(made.images >= 2, "{} images", made.images);
        let next = frames.run(1, FRAMES)?;
        println!("stage: {FRAMES} frames presented at {:?}", made.extent);
        let before = reports.size();
        let asked = PixelSize { width: before.width / 4 * 2, height: before.height / 4 * 2 };
        wait(main.run(move |main| main.request_size(asked)))?;
        poll_until(ANSWER, "the asked size reported", || reports.size() == asked);
        let next = frames.run(next, FRAMES)?;
        let next = frames.run_then(next, 1, true)?;
        let last = frames.run(next, AFTER)?;
        let record = frames.finish()?;
        println!("stage: the swapchain was dropped on the present thread");
        stop(thread)?;
        drop_on_the_main_thread(&main, device)?;

        // The frames: a fixed number presented, every readback equal, a rebuild at the
        // resize to the window's new size and one right after a present.
        let taken = record.taken.len() as u64;
        let stale = record.calls.iter().filter(|call| {
            call.present.is_some_and(|(_, presented, _)| presented == Presented::Stale)
        });
        let stale = stale.count();
        println!(
            "stage: {taken} presents ({stale} stale) of frames 1 to {}, {} readbacks equal, \
             rebuilds {}",
            last - 1,
            record.compared,
            Rebuild::describe(&record.rebuilds)
        );
        assert_eq!(taken, PRESENTS, "every frame is presented once");
        assert_eq!(u64::from(record.compared), PRESENTS, "every frame is read back");
        let resized = record.rebuilds.iter().find(|rebuild| rebuild.to == extent_of(asked));
        assert!(resized.is_some(), "no rebuild to the window's new size");
        let right_after = record.rebuilds.iter().filter(|rebuild| rebuild.frame == 2 * FRAMES + 2);
        assert_eq!(right_after.count(), 1, "no rebuild right after a present");
        let ids: Vec<u64> = record.taken.iter().map(|taken| taken.present.0).collect();
        assert_eq!(ids, (1..=PRESENTS).collect::<Vec<_>>(), "the ids count up from one");

        // The presents wait for images. The frames are drawn and presented as fast as the
        // targets come free, and a target is free at its present's point, so the run goes
        // at the swapchain's pace: FIFO shows one image a refresh. An acquire may return at
        // once, as MoltenVK's does, whose semaphore it signals when an image is shown; the
        // wait is then the present's submit's, on the present queue.
        let mut apart: Vec<Duration> = record
            .taken
            .windows(2)
            .filter(|pair| pair[0].swapchain == pair[1].swapchain)
            .map(|pair| pair[1].returned - pair[0].returned)
            .collect();
        let mut acquires: Vec<Duration> = record.calls.iter().map(|call| call.acquire).collect();
        let apart_median = quantile(&mut apart, 0.5);
        println!(
            "stage: presents returned a median {:.2} ms apart; acquires took a median {:.2} ms, \
             the longest {:.2} ms",
            milliseconds(apart_median),
            milliseconds(quantile(&mut acquires, 0.5)),
            milliseconds(quantile(&mut acquires, 1.0)),
        );

        // The report.
        let answered = record.report_calls.iter().any(Option::is_some);
        assert_eq!(answered, made.reports, "the report's source changed after the creation");
        if !answered {
            println!("stage: the device has no display-time source: every call answered None");
        } else {
            report(&record, REPORT_BOUND, apart_median)?;
        }
        let others = sink.others();
        assert!(others.is_empty(), "messages: {others:?}");
        Ok(())
    })
}

/// The display-time report's checks and figures.
fn report(record: &Record, bound: Duration, apart: Duration) -> Result<()> {
    assert!(record.report_calls.iter().all(Option::is_some), "a call answered None after Some");
    let mut calls: Vec<Duration> = record.report_calls.iter().flatten().copied().collect();
    let longest = *calls.iter().max().expect("a call");
    let median = quantile(&mut calls, 0.5);
    println!(
        "stage: {} display_times calls on the present thread, median {:.3} ms, longest {:.3} \
         ms, in a run whose presents were a median {:.2} ms apart",
        calls.len(),
        milliseconds(median),
        milliseconds(longest),
        milliseconds(apart)
    );
    assert!(longest <= bound, "a display_times call took {longest:?}, past {bound:?}");
    let ids: Vec<u64> = record.reports.iter().map(|report| report.present.0).collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "reports out of order: {ids:?}");
    let mut delays = Vec::new();
    let mut lags = Vec::new();
    for report in &record.reports {
        let taken = record
            .taken
            .iter()
            .find(|taken| taken.present == report.present)
            .ok_or_else(|| failed(format!("a report of {:?}, never taken", report.present)))?;
        // The pairing of the two clocks is read within microseconds; a millisecond is far
        // past its error.
        let slack = Duration::from_millis(1);
        assert!(
            report.shown + slack >= taken.started && report.shown <= report.received + slack,
            "{:?} shown outside its present call and its report",
            report.present
        );
        delays.push(report.shown.saturating_duration_since(taken.returned));
        lags.push(report.received.saturating_duration_since(report.shown));
    }
    let reported: Vec<PresentId> = record.reports.iter().map(|report| report.present).collect();
    let missing: Vec<u64> = record
        .taken
        .iter()
        .filter(|taken| !reported.contains(&taken.present))
        .map(|taken| taken.present.0)
        .collect();
    println!(
        "stage: {} of {} presents reported once each and in order; not reported: {missing:?}",
        reported.len(),
        record.taken.len()
    );
    if !delays.is_empty() {
        println!(
            "stage: present's return to the time shown: median {:.2} ms, longest {:.2} ms; the \
             time shown to its report: median {:.2} ms, longest {:.2} ms",
            milliseconds(quantile(&mut delays, 0.5)),
            milliseconds(quantile(&mut delays, 1.0)),
            milliseconds(quantile(&mut lags, 0.5)),
            milliseconds(quantile(&mut lags, 1.0)),
        );
    }
    // The replaced swapchains: which of their presents' reports are lost.
    let replaced = record.taken.iter().map(|taken| taken.swapchain).max().unwrap_or(1);
    for swapchain in 1..replaced {
        let on: Vec<u64> = record
            .taken
            .iter()
            .filter(|taken| taken.swapchain == swapchain)
            .map(|taken| taken.present.0)
            .collect();
        let lost: Vec<u64> = on.iter().copied().filter(|id| missing.contains(id)).collect();
        println!(
            "stage: the replaced swapchain {swapchain}: presents {:?} to {:?}, not reported: \
             {lost:?}",
            on.first(),
            on.last()
        );
    }
    assert!(!record.reports.is_empty(), "no present was reported");
    Ok(())
}

/// Row 182: frames are presented to a minimised window, through the platform's own call,
/// for a few seconds, while the frame's workers draw targets the present thread does not
/// hold on the graphics queue, each job ending at its submit and each submit completing;
/// once the window is restored, a present is shown. Where the present thread recorded a
/// block of a second or more, which is what row 182 counts, MoltenVK has said that no
/// drawable could be acquired, a worker frame completed during the block, the present or the
/// next acquire was stale, and the swapchain was rebuilt before the present shown; under
/// another driver a block is printed with what followed it. Where no call blocked, the case
/// prints that minimising did not block the drawable, which leaves the row open: on the M1
/// under macOS 27.2.1 MoltenVK's drawable does not block.
pub(crate) fn a_minimised_window_is_presented_to_and_the_frame_goes_on() -> Result<()> {
    const CASE: &str = "a_minimised_window_is_presented_to_and_the_frame_goes_on";
    /// How long frames are presented while the window is minimised, at most: past this
    /// with no block, minimising does not block the drawable.
    const MINIMISED: Duration = Duration::from_secs(8);
    let (main, pump) = event_pump(desc())?;
    let (present, thread) = present_thread(&ClientPlatform::new(), None::<Presenter>)?;
    pump_with(&main, pump, move |main| {
        let sink = Sink::new(Some(NO_DRAWABLE));
        let Some(device) = device_on_the_window(&main, &sink, CASE)? else {
            return stop(thread);
        };
        let moltenvk = moltenvk(&device);
        let reports = wait(main.run(|main| main.window().reports()))?;
        let (mut frames, made) = Frames::new(&device, &present, reports.clone())?;
        println!("stage: the swapchain was made on the present thread: {made:?}");
        let mut frame = frames.run(1, 6)?;
        println!("stage: frames presented before the minimise");

        let executor = Executor::builder().platform(Arc::new(ClientPlatform::new())).build()?;
        let workers = Workers::new(&device, made.format)?;
        wait(main.run(|main| main.set_minimised(true)))?;
        poll_until(ANSWER, "a minimise reported", || {
            wait(main.run(|main| main.is_minimised())).ok().flatten() == Some(true)
        });
        println!("stage: the window is minimised, at {:?}", reports.size());
        let mut blocked = None;
        let mut stale = false;
        let minimised = Instant::now();
        let mut calls = 0;
        let mut worker_frames = 0;
        while minimised.elapsed() < MINIMISED {
            calls += 1;
            let job = frames.draw(frame)?;
            let pending = frames.present.run(move |state: &mut Option<Presenter>| {
                state.as_mut().expect("the swapchain is made").frame(job)
            });
            // The frame's workers draw while the present call is out.
            let (call, drawn) = workers.draw_while(&executor, pending)?;
            worker_frames += drawn.len();
            frames.settle(job, &call);
            let during: Vec<&Instant> =
                drawn.iter().filter(|at| **at > call.started && **at < call.ended).collect();
            // Every call that blocked, and one call in twenty, is printed.
            if call.block() >= BLOCK || calls % 20 == 1 || call.present.is_none() {
                println!(
                    "stage: frame {}'s call: acquire {:?} ({} empty), present {:?}, longest \
                 block {:?}; {} worker frames completed during it",
                    call.frame,
                    call.acquire,
                    call.nones,
                    call.present.map(|(took, presented, id)| (took, presented, id.0)),
                    call.block(),
                    during.len()
                );
            }
            if call.present.is_some() {
                frame += 1;
            }
            if call.block() >= BLOCK && blocked.is_none() {
                blocked = Some((call, during.len()));
            }
            stale |= call.present.is_none_or(|(_, presented, _)| presented == Presented::Stale);
            if stale {
                break;
            }
        }
        println!(
            "stage: while minimised, {calls} present calls and {worker_frames} worker frames in \
             {:?}: a block of a second or more {}, stale {stale}, MoltenVK's message heard {}",
            minimised.elapsed(),
            if blocked.is_some() { "recorded" } else { "not recorded" },
            sink.heard()
        );
        wait(main.run(|main| main.set_minimised(false)))?;
        poll_until(ANSWER, "a restore reported", || {
            wait(main.run(|main| main.is_minimised())).ok().flatten() == Some(false)
        });
        poll_until(ANSWER, "the size back", || !reports.size().is_empty());
        println!("stage: the window is restored, at {:?}", reports.size());
        let rebuilt_from = frames_rebuilds(&frames)?;
        let after = frames.run(frame, 3)?;
        let record = frames.finish()?;
        workers.finish()?;
        executor.shutdown();
        stop(thread)?;
        drop_on_the_main_thread(&main, device)?;

        let rebuilds = &record.rebuilds[rebuilt_from..];
        let shown_after = record.calls.iter().rev().take(3).all(|call| {
            call.present.is_some_and(|(_, presented, _)| presented == Presented::Shown)
        });
        println!(
            "stage: after the restore, frames {frame} to {}: rebuilds {}, the last three \
             presents shown: {shown_after}",
            after - 1,
            Rebuild::describe(rebuilds)
        );
        let others = sink.others();
        assert!(others.is_empty(), "messages: {others:?}");
        assert!(worker_frames >= 1, "no worker frame completed while the window was minimised");
        assert!(shown_after, "no present shown after the restore");
        let Some((call, during)) = blocked else {
            println!(
                "stage: minimising did not block the drawable{}: no present call took a second",
                if moltenvk { " under MoltenVK" } else { "" }
            );
            return Ok(());
        };
        assert!(during >= 1, "no worker frame completed while the present thread was blocked");
        println!(
            "stage: the present thread was blocked {:?}, and {during} worker frames completed",
            call.block()
        );
        if !moltenvk {
            return Ok(());
        }
        assert!(stale, "no stale present or acquire while minimised");
        assert!(sink.heard() >= 1, "MoltenVK did not say {NO_DRAWABLE:?}");
        assert!(
            rebuilds.iter().any(|rebuild| rebuild.why != "the window's size"),
            "no rebuild of the stale swapchain"
        );
        Ok(())
    })
}

/// How many rebuilds the present thread has recorded so far.
fn frames_rebuilds(frames: &Frames) -> Result<usize> {
    wait(frames.present.run(|state: &mut Option<Presenter>| {
        state.as_ref().map_or(0, |presenter| presenter.record.rebuilds.len())
    }))
}

/// The frame's workers' targets, which the present thread never holds: each is cleared on
/// the graphics queue by a worker's job and its point waited for by the driver.
struct Workers {
    device: Arc<VulkanDevice>,
    targets: Vec<ImageHandle>,
    timeline: TimelineHandle,
    number: Arc<AtomicUsize>,
}

impl Workers {
    fn new(device: &Arc<VulkanDevice>, format: Format) -> Result<Self> {
        let targets = (0..2)
            .map(|_| {
                device.create_image(&ImageDesc {
                    label: "worker target",
                    format,
                    extent: TARGET,
                    mips: 1,
                    layers: 1,
                    usage: ImageUsage::COLOUR,
                    memory: ImageMemory::Own,
                    sharing: Sharing::Exclusive,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let timeline = device.create_timeline(0)?;
        Ok(Self { device: Arc::clone(device), targets, timeline, number: Arc::default() })
    }

    /// Runs `pending`, a present call, to its end while the workers draw a target every 50
    /// ms, each job submitted by a worker and its point waited for here; returns the call
    /// and the instant each worker frame's point was reached.
    fn draw_while(
        &self,
        executor: &Executor,
        pending: impl Future<Output = Result<Result<Call>>>,
    ) -> Result<(Call, Vec<Instant>)> {
        use core::pin::pin;
        use core::task::{Context, Poll, Waker};
        let mut pending = pin!(pending);
        let mut context = Context::from_waker(Waker::noop());
        let mut drawn = Vec::new();
        let until = Instant::now() + PATIENCE * 2;
        loop {
            if let Poll::Ready(call) = pending.as_mut().poll(&mut context) {
                return Ok((call??, drawn));
            }
            if Instant::now() >= until {
                return Err(failed(format!("a present call passed {:?}", PATIENCE * 2)));
            }
            let number = self.number.fetch_add(1, Ordering::Relaxed) as u64 + 1;
            let target = self.targets[number as usize % self.targets.len()];
            let device = Arc::clone(&self.device);
            let point = TimelinePoint { timeline: self.timeline, value: number };
            let job = executor.spawn(WorkerClass::FrameCritical, async move {
                let mut encoder = device.create_encoder(G, "worker frame")?;
                encoder.transition_images(&[image_to(
                    target,
                    on(ImageState::Undefined, G),
                    on(ImageState::ColourAttachment, G),
                )]);
                clear(&mut *encoder, target, colour(number));
                let mut encoders = [encoder];
                device
                    .submit(G, Submit { encoders: &mut encoders, waits: &[], signals: &[point] })?;
                let [encoder] = encoders;
                Ok::<_, lactose_core::Error>(encoder)
            });
            // The job ends at its submit: a worker never waits for the GPU or the present.
            let encoder = wait(job)??;
            if !self.device.wait_timeline(point, PATIENCE)? {
                return Err(failed(format!("a worker frame's submit, {point:?}, never ended")));
            }
            drawn.push(Instant::now());
            drop(encoder);
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn finish(self) -> Result<()> {
        let number = self.number.load(Ordering::Relaxed) as u64;
        let point = TimelinePoint { timeline: self.timeline, value: number };
        if number > 0 && !self.device.wait_timeline(point, PATIENCE)? {
            return Err(failed("the workers' last submit never ended"));
        }
        Ok(())
    }
}
