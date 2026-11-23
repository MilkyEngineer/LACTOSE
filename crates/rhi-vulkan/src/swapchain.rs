//! The swapchain: the driver's images of the device's surface, an acquire bounded at a
//! second, and a present that submits once its walk of the records finds its submit
//! released, then presents with a present fence (`VK_EXT_swapchain_maintenance1`) and, where
//! the device reports display times, its id (`VK_GOOGLE_display_timing`).
//!
//! Each acquire signals a semaphore of a pool, which is used again only once the submit that
//! waited for it is known complete; each image has its own present semaphore, which its
//! present's submit signals and which is signalled again only once that present's fence
//! has. A present is pending until its fence signals: `resize` and the drop wait for them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ash::vk;
use lactose_core::{Error, Result};
use lactose_rhi_api::{
    Acquired, DisplayTime, Extent, Format, ImageHandle, ImageUsage, PresentId, Presented,
    QueueRole, Rule, Submit, Swapchain, TimelinePoint,
};

use crate::device::{ImageObj, Shared, alive, lost};
use crate::lock;
use crate::raw::{Presents, driver, unsupported};
use crate::records::{ImageRec, QUEUES};
use crate::submit::{self, Binary, Staging};

/// The bound of an acquire, of a present's wait for its submit's release, and of a
/// `resize`'s wait for the swapchain's presents.
const BOUND: Duration = Duration::from_secs(1);

/// The images a swapchain asks for, within what the surface allows.
const IMAGES: u32 = 3;

/// The entries of `VK_GOOGLE_display_timing`'s history one call reads: more than MoltenVK
/// 1.4.2 keeps, 60.
const HISTORY: usize = 64;

/// The formats a swapchain takes, the first the surface offers in `SRGB_NONLINEAR`.
const FORMATS: [(vk::Format, Format); 4] = [
    (vk::Format::B8G8R8A8_UNORM, Format::Bgra8Unorm),
    (vk::Format::R8G8B8A8_UNORM, Format::Rgba8Unorm),
    (vk::Format::B8G8R8A8_SRGB, Format::Bgra8Srgb),
    (vk::Format::R8G8B8A8_SRGB, Format::Rgba8Srgb),
];

/// What the images are made for: colour attachments alone, which the contract names, and
/// with the `swapchain-copies` feature, for tests that read an image back, copies from and to
/// as well where the surface allows both.
fn usage(supported: vk::ImageUsageFlags) -> (vk::ImageUsageFlags, ImageUsage) {
    let copies = vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
    if cfg!(feature = "swapchain-copies") && supported.contains(copies) {
        let usage =
            ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE).union(ImageUsage::COPY_DESTINATION);
        (vk::ImageUsageFlags::COLOR_ATTACHMENT | copies, usage)
    } else {
        (vk::ImageUsageFlags::COLOR_ATTACHMENT, ImageUsage::COLOUR)
    }
}

/// Where an acquire semaphore of the pool is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Use {
    /// Unsignalled, with no operation pending: an acquire may take it.
    Free,
    /// An acquire signalled it, and no batch has waited for it yet.
    Signalled,
    /// The present queue's submit of this stamp waited for it.
    Waited(u64),
}

/// The image an acquire returned, until its present is taken.
#[derive(Clone, Copy, Debug)]
struct Held {
    index: u32,
    image: ImageHandle,
    /// The pool's semaphore the acquire signalled, until a batch waits for it.
    semaphore: Option<usize>,
}

/// The driver's swapchain and what is made for each of its images.
#[derive(Default)]
struct Chain {
    swapchain: vk::SwapchainKHR,
    images: Vec<ImageHandle>,
    /// One for each image, which its present's submit signals and its present waits for.
    present_semaphores: Vec<vk::Semaphore>,
    /// The fence of each image's last present.
    last_fence: Vec<Option<vk::Fence>>,
}

/// Where a swapchain's display times come from, and what it has not handed out yet.
struct Timing {
    /// An `Instant` and the host clock's nanoseconds read beside it, at the swapchain's
    /// making: the times are moved onto `Instant` by their distance from it.
    base: (Instant, u64),
    /// The reports drained from a replaced swapchain, handed out at the next call.
    backlog: Vec<DisplayTime>,
    /// The greatest id handed out.
    reported: u64,
}

pub(crate) struct VulkanSwapchain {
    shared: Arc<Shared>,
    /// The records queue the present role is served by.
    queue: usize,
    format: Format,
    vk_format: vk::Format,
    extent: Extent,
    chain: Chain,
    /// The acquire semaphores and where each is.
    pool: Vec<(vk::Semaphore, Use)>,
    held: Option<Held>,
    /// The fences of presents not known done.
    pending: Vec<vk::Fence>,
    /// Signalled fences, reset, for later presents.
    spare: Vec<vk::Fence>,
    /// An acquire or a present was out of date, or the surface was empty at the last make.
    stale: bool,
    presented: Option<PresentId>,
    timing: Option<Timing>,
}

impl VulkanSwapchain {
    /// Makes the device's swapchain: refused at a zero extent and while another lives, and
    /// `Unsupported` on a device made with no surface.
    pub(crate) fn new(shared: &Arc<Shared>, extent: Extent) -> Result<Self> {
        if shared.raw.presents.is_none() {
            alive(&shared.records())?;
            return Err(unsupported("a swapchain of a device made with no surface"));
        }
        {
            let mut records = shared.records();
            alive(&records)?;
            if extent.width == 0 || extent.height == 0 {
                return Err(Rule::Bounds.refuse("a swapchain extent with a zero dimension"));
            }
            if records.swapchain {
                return Err(Rule::Swapchain.refuse("a second swapchain for a device that has one"));
            }
            records.swapchain = true;
        }
        let timing = shared.raw.presents.as_ref().and_then(|presents| presents.timing.as_ref());
        // From here the drop gives the records' mark and the device's hold back.
        let mut swapchain = Self {
            shared: shared.hold(),
            queue: shared.queue_of(QueueRole::Present),
            format: Format::Bgra8Unorm,
            vk_format: vk::Format::UNDEFINED,
            extent: Extent { width: 0, height: 0 },
            chain: Chain::default(),
            pool: Vec::new(),
            held: None,
            pending: Vec::new(),
            spare: Vec::new(),
            stale: true,
            presented: None,
            timing: timing.and_then(|_| host_clock()).map(|base| Timing {
                base,
                backlog: Vec::new(),
                reported: 0,
            }),
        };
        swapchain.make(extent)?;
        Ok(swapchain)
    }

    fn presents(&self) -> &Presents {
        // A swapchain is made only on a device with a surface.
        self.shared.raw.presents.as_ref().expect("a swapchain's device presents")
    }

    fn device(&self) -> &ash::Device {
        &self.shared.raw.device
    }

    /// A driver failure, the device marked lost where it is.
    fn failed(&self, result: vk::Result, call: &str) -> Error {
        self.shared.lost_on(result);
        if result == vk::Result::ERROR_DEVICE_LOST { lost() } else { driver(result, call) }
    }

    /// Makes the driver's swapchain at `extent`, held to the surface's bounds, from the one
    /// it has, which it replaces; the replaced one's presents are done. At an empty surface
    /// it makes nothing and keeps what it has, stale.
    fn make(&mut self, extent: Extent) -> Result<()> {
        let presents = self.presents();
        let (loader, surface) = (&presents.surface.loader, presents.surface.surface);
        let physical = self.shared.raw.physical;
        // SAFETY: the surface and the physical device are the device's, and live.
        let caps = unsafe { loader.get_physical_device_surface_capabilities(physical, surface) }
            .map_err(|e| self.failed(e, "vkGetPhysicalDeviceSurfaceCapabilitiesKHR"))?;
        let current = caps.current_extent;
        let empty = (current.width == 0 || current.height == 0)
            || caps.max_image_extent.width == 0
            || caps.max_image_extent.height == 0;
        if empty {
            self.stale = true;
            return Ok(());
        }
        // SAFETY: as above.
        let offered = unsafe { loader.get_physical_device_surface_formats(physical, surface) }
            .map_err(|e| self.failed(e, "vkGetPhysicalDeviceSurfaceFormatsKHR"))?;
        let (vk_format, format) = FORMATS
            .into_iter()
            .find(|(vk_format, _)| {
                offered.iter().any(|offer| {
                    offer.format == *vk_format
                        && offer.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                })
            })
            .ok_or_else(|| unsupported(format!("a surface of none of {FORMATS:?}: {offered:?}")))?;
        let clamp = |wanted: u32, low: u32, high: u32| wanted.clamp(low, high.max(low));
        let made = vk::Extent2D {
            width: clamp(extent.width, caps.min_image_extent.width, caps.max_image_extent.width),
            height: clamp(
                extent.height,
                caps.min_image_extent.height,
                caps.max_image_extent.height,
            ),
        };
        let most = if caps.max_image_count == 0 { u32::MAX } else { caps.max_image_count };
        let count = IMAGES.clamp(caps.min_image_count, most.max(caps.min_image_count));
        let alpha = [
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::INHERIT,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        ]
        .into_iter()
        .find(|alpha| caps.supported_composite_alpha.contains(*alpha))
        .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE);
        let family = [self.shared.queue(self.queue).family];
        let (vk_usage, usage) = usage(caps.supported_usage_flags);
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(count)
            .image_format(vk_format)
            .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
            .image_extent(made)
            .image_array_layers(1)
            .image_usage(vk_usage)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .queue_family_indices(&family)
            .pre_transform(caps.current_transform)
            .composite_alpha(alpha)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true)
            .old_swapchain(self.chain.swapchain);
        let swapchain_fns = &presents.swapchain;
        // SAFETY: the surface is live, and the swapchain it replaces, if any, is this
        // swapchain's; the extent and the count are within the surface's bounds, FIFO is
        // always offered, and the format was among the surface's.
        let swapchain = unsafe { swapchain_fns.create_swapchain(&info, None) }
            .map_err(|e| self.failed(e, "vkCreateSwapchainKHR"))?;
        let made_chain = self.chain_of(swapchain, format, usage, made);
        let chain = match made_chain {
            Ok(chain) => chain,
            Err(error) => {
                // SAFETY: the swapchain was made above and nothing has used it.
                unsafe { self.presents().swapchain.destroy_swapchain(swapchain, None) };
                return Err(error);
            }
        };
        let old = core::mem::replace(&mut self.chain, chain);
        self.retire(old);
        self.format = format;
        self.vk_format = vk_format;
        self.extent = Extent { width: made.width, height: made.height };
        self.stale = false;
        Ok(())
    }

    /// The images of a new swapchain as handles, and a present semaphore for each.
    fn chain_of(
        &self,
        swapchain: vk::SwapchainKHR,
        format: Format,
        usage: ImageUsage,
        made: vk::Extent2D,
    ) -> Result<Chain> {
        let extent = Extent { width: made.width, height: made.height };
        // SAFETY: the swapchain is live.
        let images = unsafe { self.presents().swapchain.get_swapchain_images(swapchain) }
            .map_err(|e| self.failed(e, "vkGetSwapchainImagesKHR"))?;
        let mut chain = Chain { swapchain, ..Chain::default() };
        let mut failure = None;
        for &image in &images {
            let made = ImageObj::swapchain_image(&self.shared.raw, image, format, usage, extent)
                .and_then(|object| {
                    let rec =
                        ImageRec { stamps: [0; QUEUES], slots: 0, placed: None, swapchain: true };
                    let handle = self.shared.records().images.insert(rec)?;
                    self.shared.images.insert(handle, Arc::new(object));
                    Ok(handle)
                })
                .and_then(|handle| {
                    chain.images.push(handle);
                    let info = vk::SemaphoreCreateInfo::default();
                    // SAFETY: the device is live.
                    unsafe { self.device().create_semaphore(&info, None) }
                        .map_err(|e| self.failed(e, "vkCreateSemaphore"))
                });
            match made {
                Ok(semaphore) => {
                    chain.present_semaphores.push(semaphore);
                    chain.last_fence.push(None);
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        if let Some(error) = failure {
            // Nothing of the new chain has been used; its swapchain is the caller's to
            // destroy.
            let swapchain = core::mem::take(&mut chain.swapchain);
            self.forget(&chain);
            self.destroy_semaphores(&chain);
            let _ = swapchain;
            return Err(error);
        }
        Ok(chain)
    }

    /// Lets go of a chain the swapchain no longer presents to, its presents done: its
    /// handles are stale from now on, and its reports are kept for the next call.
    fn retire(&mut self, old: Chain) {
        if old.swapchain == vk::SwapchainKHR::null() {
            return;
        }
        self.drain_reports(old.swapchain);
        self.forget(&old);
        self.destroy_semaphores(&old);
        // SAFETY: every present of the old swapchain has its fence signalled (`resize`), so
        // the presentation engine is done with it; its images' handles are stale, so no
        // later command names them.
        unsafe { self.presents().swapchain.destroy_swapchain(old.swapchain, None) };
    }

    /// Removes a chain's images from the records and the table: a handle to one is stale.
    fn forget(&self, chain: &Chain) {
        let mut records = self.shared.records();
        for &image in &chain.images {
            records.images.remove(image);
        }
        drop(records);
        for &image in &chain.images {
            self.shared.images.remove(image);
        }
    }

    fn destroy_semaphores(&self, chain: &Chain) {
        if self.shared.raw.leaking() {
            return;
        }
        for &semaphore in &chain.present_semaphores {
            // SAFETY: each present's wait for it is done (its fence), and no submit to come
            // signals it.
            unsafe { self.device().destroy_semaphore(semaphore, None) };
        }
    }

    /// Moves the fences known signalled to the spares, reset.
    fn reap(&mut self) -> Result<()> {
        let mut at = 0;
        while at < self.pending.len() {
            let fence = self.pending[at];
            // SAFETY: the fence is this swapchain's.
            let done = unsafe { self.device().get_fence_status(fence) }
                .map_err(|e| self.failed(e, "vkGetFenceStatus"))?;
            if done {
                // SAFETY: the fence is signalled and no present names it any more.
                unsafe { self.device().reset_fences(&[fence]) }
                    .map_err(|e| self.failed(e, "vkResetFences"))?;
                self.pending.swap_remove(at);
                self.spare.push(fence);
                // A spare is another present's next: no image's last present is it now.
                for last in &mut self.chain.last_fence {
                    if *last == Some(fence) {
                        *last = None;
                    }
                }
            } else {
                at += 1;
            }
        }
        Ok(())
    }

    /// Waits until `deadline` for `fences`, and reaps what signalled; whether they all did.
    fn wait_fences(&mut self, fences: &[vk::Fence], deadline: Instant) -> Result<bool> {
        let fences: Vec<vk::Fence> =
            fences.iter().copied().filter(|fence| self.pending.contains(fence)).collect();
        if fences.is_empty() {
            return Ok(true);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        let nanos = u64::try_from(left.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the fences are this swapchain's; a wait is bounded by `deadline`.
        let waited = unsafe { self.device().wait_for_fences(&fences, true, nanos) };
        let all = match waited {
            Ok(()) => true,
            Err(vk::Result::TIMEOUT) => false,
            Err(error) => return Err(self.failed(error, "vkWaitForFences")),
        };
        self.reap()?;
        Ok(all)
    }

    /// The acquire semaphore a new acquire signals: one whose waiting submit is known
    /// complete, or a new one.
    fn acquire_semaphore(&mut self) -> Result<usize> {
        let known = self.shared.settled().queues[self.queue].known;
        for entry in &mut self.pool {
            if let Use::Waited(seq) = entry.1
                && seq <= known
            {
                entry.1 = Use::Free;
            }
        }
        if let Some(at) = self.pool.iter().position(|entry| entry.1 == Use::Free) {
            return Ok(at);
        }
        let info = vk::SemaphoreCreateInfo::default();
        // SAFETY: the device is live.
        let semaphore = unsafe { self.device().create_semaphore(&info, None) }
            .map_err(|e| self.failed(e, "vkCreateSemaphore"))?;
        self.pool.push((semaphore, Use::Free));
        Ok(self.pool.len() - 1)
    }

    /// Ends a held image's acquire: where no batch has waited for its semaphore, an empty
    /// batch on the present queue waits for it, so that the semaphore is used again only
    /// after that batch completes. The image is forgotten, acquired by a swapchain that is
    /// to be replaced or destroyed.
    fn spend(&mut self) -> Result<()> {
        let Some(held) = self.held.take() else { return Ok(()) };
        let Some(at) = held.semaphore else { return Ok(()) };
        let queue = self.queue;
        let _queue = lock(&self.shared.queues[queue]);
        let seq = {
            let mut records = self.shared.records();
            alive(&records)?;
            records.stamp(queue, &[], &[])
        };
        let target = self.shared.queue(queue);
        let point = |semaphore: vk::Semaphore, value: u64| {
            vk::SemaphoreSubmitInfo::default()
                .semaphore(semaphore)
                .value(value)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
        };
        let waits = [point(self.pool[at].0, 0)];
        let signals = [point(target.timeline, seq)];
        let info = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&waits)
            .signal_semaphore_infos(&signals);
        // SAFETY: the queue's lock is held; the acquire signalled the semaphore and no batch
        // has waited for it; the queue's own timeline grows by one.
        let submitted =
            unsafe { self.device().queue_submit2(target.queue, &[info], vk::Fence::null()) };
        self.pool[at].1 = Use::Waited(seq);
        let mut records = self.shared.records();
        records.received(queue, seq, submitted.is_err());
        drop(records);
        submitted.map_err(|e| self.failed(e, "vkQueueSubmit2"))
    }

    /// Waits, with no lock held, until `deadline` for a point a present's walk left
    /// undecided; whether to walk again.
    fn wait_point(&self, point: TimelinePoint, deadline: Instant) -> Result<bool> {
        let Some(object) = self.shared.timelines.resolve(point.timeline) else {
            // Destroyed meanwhile: the next walk refuses it.
            return Ok(true);
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        let semaphores = [object.semaphore];
        let values = [point.value];
        let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
        let nanos = u64::try_from(left.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the count keeps the semaphore alive through the wait, which is bounded.
        match unsafe { self.device().wait_semaphores(&info, nanos) } {
            Ok(()) | Err(vk::Result::TIMEOUT) => Ok(true),
            Err(error) => Err(self.failed(error, "vkWaitSemaphores")),
        }
    }

    /// The reports a swapchain's driver keeps, in increasing id, past the last handed out.
    fn drain_reports(&mut self, swapchain: vk::SwapchainKHR) {
        let last = self.presented.map_or(0, |PresentId(id)| id);
        let Some(timing) = self.timing.as_mut() else { return };
        let Some(google) = self.shared.raw.presents.as_ref().and_then(|p| p.timing.as_ref()) else {
            return;
        };
        if swapchain == vk::SwapchainKHR::null() {
            return;
        }
        // The history is taken by each call that writes it (MoltenVK takes off every entry
        // it writes), so a call's entries are kept as it wrote them, and the call is made
        // again while more are held: ash's wrapper counts first and throws a fill away on
        // `VK_INCOMPLETE`, which would lose the entries that fill took.
        let mut reports = Vec::new();
        let mut buffer = [vk::PastPresentationTimingGOOGLE::default(); HISTORY];
        loop {
            let mut count = HISTORY as u32;
            // SAFETY: the swapchain is live, and this thread's alone between its acquire and
            // its present (`Swapchain` is not `Sync`); the buffer holds `count` entries.
            let result = unsafe {
                (google.fp().get_past_presentation_timing_google)(
                    google.device(),
                    swapchain,
                    &mut count,
                    buffer.as_mut_ptr(),
                )
            };
            if result != vk::Result::SUCCESS && result != vk::Result::INCOMPLETE {
                break;
            }
            let written = (count as usize).min(HISTORY);
            reports.extend_from_slice(&buffer[..written]);
            // A driver that keeps its history would give the same entries again: one that
            // wrote none, or a history longer than any driver keeps, ends the drain.
            if result == vk::Result::SUCCESS || written == 0 || reports.len() >= 4 * HISTORY {
                break;
            }
        }
        let mut drained: Vec<DisplayTime> = reports
            .iter()
            .filter_map(|report| {
                let id = widen(report.present_id, last);
                let shown = timing.at(report.actual_present_time)?;
                (id > timing.reported).then_some(DisplayTime { present: PresentId(id), shown })
            })
            .collect();
        drained.sort_by_key(|time| time.present);
        drained.dedup_by_key(|time| time.present);
        if let Some(newest) = drained.last() {
            timing.reported = newest.present.0;
        }
        timing.backlog.extend(drained);
    }
}

/// The 64-bit id whose low half `low` is, at or below `last`: the driver carries 32 bits.
fn widen(low: u32, last: u64) -> u64 {
    let id = (last & !u64::from(u32::MAX)) | u64::from(low);
    if id > last { id.saturating_sub(1 << 32) } else { id }
}

impl Timing {
    /// `nanos` of the host clock as an `Instant`, if it is one.
    fn at(&self, nanos: u64) -> Option<Instant> {
        let (instant, base) = self.base;
        if nanos >= base {
            instant.checked_add(Duration::from_nanos(nanos - base))
        } else {
            instant.checked_sub(Duration::from_nanos(base - nanos))
        }
    }
}

/// An `Instant` and the host clock `VK_GOOGLE_display_timing` reports on under MoltenVK,
/// read one after the other: `CLOCK_UPTIME_RAW`, the clock of `mach_absolute_time` and of
/// `CACurrentMediaTime`, whose seconds MoltenVK multiplies into `actualPresentTime`.
#[cfg(target_os = "macos")]
fn host_clock() -> Option<(Instant, u64)> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a read of a clock into a local.
    let read = unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut time) };
    let instant = Instant::now();
    if read != 0 {
        return None;
    }
    let seconds = u64::try_from(time.tv_sec).ok()?;
    let nanos = u64::try_from(time.tv_nsec).ok()?;
    Some((instant, seconds.checked_mul(1_000_000_000)?.checked_add(nanos)?))
}

/// No device away from macOS has a display-time source here.
#[cfg(not(target_os = "macos"))]
fn host_clock() -> Option<(Instant, u64)> {
    None
}

impl Swapchain for VulkanSwapchain {
    fn format(&self) -> Format {
        self.format
    }

    fn extent(&self) -> Extent {
        if self.chain.swapchain == vk::SwapchainKHR::null() {
            Extent { width: 0, height: 0 }
        } else {
            self.extent
        }
    }

    fn image_count(&self) -> u32 {
        self.chain.images.len() as u32
    }

    fn acquire(&mut self) -> Result<Acquired> {
        alive(&self.shared.records())?;
        if self.held.is_some() {
            return Err(
                Rule::Swapchain.refuse("an acquire while the acquired image is not yet presented")
            );
        }
        if self.stale || self.chain.swapchain == vk::SwapchainKHR::null() {
            return Ok(Acquired::Stale);
        }
        let at = self.acquire_semaphore()?;
        let nanos = u64::try_from(BOUND.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the swapchain is this one's, reached by this thread alone (`Swapchain` is
        // not `Sync`); the semaphore is unsignalled with no operation pending (`Use::Free`).
        let acquired = unsafe {
            self.presents().swapchain.acquire_next_image(
                self.chain.swapchain,
                nanos,
                self.pool[at].0,
                vk::Fence::null(),
            )
        };
        match acquired {
            // `VK_SUBOPTIMAL_KHR` is an image, never stale.
            Ok((index, _)) => {
                self.pool[at].1 = Use::Signalled;
                let image = self.chain.images[index as usize];
                self.held = Some(Held { index, image, semaphore: Some(at) });
                Ok(Acquired::Image(image))
            }
            Err(vk::Result::TIMEOUT | vk::Result::NOT_READY) => Ok(Acquired::None),
            // MoltenVK's drawable timeout surfaces as an exhausted pool (decision 5.6).
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::ERROR_OUT_OF_POOL_MEMORY) => {
                self.stale = true;
                Ok(Acquired::Stale)
            }
            Err(error) => Err(self.failed(error, "vkAcquireNextImageKHR")),
        }
    }

    fn present(&mut self, image: ImageHandle, mut submit: Submit<'_>) -> Result<Presented> {
        let deadline = Instant::now() + BOUND;
        alive(&self.shared.records())?;
        let Some(held) = self.held.filter(|held| held.image == image) else {
            return Err(
                Rule::Swapchain.refuse("a present of an image that is not the acquired one")
            );
        };
        let index = held.index as usize;
        // The image's present semaphore is signalled again only once its last present's
        // wait for it is done. The image was acquired again, so that is all but certain.
        if let Some(fence) = self.chain.last_fence[index]
            && !self.wait_fences(&[fence], deadline)?
        {
            return Err(Rule::Pending.refuse("a present whose image's last present is pending"));
        }
        let queue = self.queue;
        let shared = Arc::clone(&self.shared);
        loop {
            let held_queue = lock(&shared.queues[queue]);
            let staged = match submit::stage(&shared, queue, QueueRole::Present, &mut submit, true)?
            {
                Staging::Staged(staged) => staged,
                Staging::Undecided(point) => {
                    // The wait is made with no lock held: on a shared queue the submit
                    // that signals the point may need the queue's.
                    drop(held_queue);
                    if Instant::now() >= deadline || !self.wait_point(point, deadline)? {
                        return Err(Rule::Pending.refuse(format!(
                            "a present whose submit is not released within the bound: it waits \
                             for {} on a timeline nothing has signalled or promised by a \
                             released submit",
                            point.value
                        )));
                    }
                    continue;
                }
            };
            let seq = staged.seq;
            let binary = Binary {
                wait: held.semaphore.map(|at| self.pool[at].0),
                signal: Some(self.chain.present_semaphores[index]),
            };
            let sent = submit::send(&shared, staged, &submit, binary);
            if let Some(at) = held.semaphore {
                self.pool[at].1 = Use::Waited(seq);
            }
            // The acquire's semaphore is spent, by the submit or the empty batch after its
            // failure: the image stays acquired, and its next present waits for nothing.
            self.held = Some(Held { semaphore: None, ..held });
            sent?;
            let fence = match self.spare.pop() {
                Some(fence) => fence,
                None => {
                    let info = vk::FenceCreateInfo::default();
                    // SAFETY: the device is live.
                    unsafe { self.device().create_fence(&info, None) }
                        .map_err(|e| self.failed(e, "vkCreateFence"))?
                }
            };
            let id = self.presented.map_or(1, |PresentId(last)| last + 1);
            let fences = [fence];
            let mut fence_info = vk::SwapchainPresentFenceInfoEXT::default().fences(&fences);
            let times = [vk::PresentTimeGOOGLE {
                // The driver carries 32 bits; `widen` gives the reports theirs back.
                present_id: id as u32,
                desired_present_time: 0,
            }];
            let mut time_info = vk::PresentTimesInfoGOOGLE::default().times(&times);
            let swapchains = [self.chain.swapchain];
            let indices = [held.index];
            let waits = [self.chain.present_semaphores[index]];
            let mut info = vk::PresentInfoKHR::default()
                .wait_semaphores(&waits)
                .swapchains(&swapchains)
                .image_indices(&indices)
                .push_next(&mut fence_info);
            if self.timing.is_some() {
                info = info.push_next(&mut time_info);
            }
            let target = self.shared.queue(queue);
            // SAFETY: the queue's lock is held; the image is the one acquired, its submit
            // left it in `PRESENT_SRC_KHR` and signals the present semaphore the present
            // waits for, a binary semaphore no other wait is pending on; the fence is
            // unsignalled and no other present names it.
            let shown = unsafe { self.presents().swapchain.queue_present(target.queue, &info) };
            drop(held_queue);
            self.pending.push(fence);
            self.chain.last_fence[index] = Some(fence);
            self.held = None;
            return match shown {
                // `VK_SUBOPTIMAL_KHR` is shown, never stale.
                Ok(_) => {
                    self.presented = Some(PresentId(id));
                    Ok(Presented::Shown)
                }
                Err(vk::Result::ERROR_DEVICE_LOST) => {
                    self.shared.lost_on(vk::Result::ERROR_DEVICE_LOST);
                    Err(lost())
                }
                // Every other failure after the submit was taken is a taken present that
                // is stale (6(i)): `VK_ERROR_OUT_OF_DATE_KHR`, and MoltenVK's
                // `VK_ERROR_OUT_OF_POOL_MEMORY` after its drawable timeout among them.
                Err(_) => {
                    self.presented = Some(PresentId(id));
                    self.stale = true;
                    Ok(Presented::Stale)
                }
            };
        }
    }

    fn resize(&mut self, extent: Extent) -> Result<()> {
        let deadline = Instant::now() + BOUND;
        alive(&self.shared.records())?;
        if extent.width == 0 || extent.height == 0 {
            return Err(Rule::Bounds.refuse("a swapchain extent with a zero dimension"));
        }
        let pending = self.pending.clone();
        if !self.wait_fences(&pending, deadline)? {
            return Err(Rule::Pending.refuse("a resize while a present is pending past the bound"));
        }
        self.spend()?;
        self.make(extent)
    }

    fn last_present(&self) -> Option<PresentId> {
        self.presented
    }

    fn display_times(&mut self) -> Option<Vec<DisplayTime>> {
        self.timing.as_ref()?;
        self.drain_reports(self.chain.swapchain);
        self.timing.as_mut().map(|timing| core::mem::take(&mut timing.backlog))
    }
}

impl Drop for VulkanSwapchain {
    fn drop(&mut self) {
        // Where the presents or the acquires' submits do not end within the wait, nothing of
        // the swapchain is destroyed under the presentation engine's or the queue's use.
        let mut leak = false;
        let lost = self.shared.records().lost;
        if !std::thread::panicking() && !lost {
            // A present is pending until its fence signals, which the presentation engine
            // gives within a few refreshes; one that does not within this wait leaks the
            // swapchain rather than destroy it under the engine's use.
            let deadline = Instant::now() + BOUND * 3;
            let pending = self.pending.clone();
            let shown = self.wait_fences(&pending, deadline).unwrap_or(true);
            let _ = self.spend();
            // The acquire semaphores are destroyed once the submits that waited for them
            // have completed, within the same wait.
            let waited = self.pool.iter().filter_map(|entry| match entry.1 {
                Use::Waited(seq) => Some(seq),
                Use::Free | Use::Signalled => None,
            });
            let mut spent = true;
            if let Some(last) = waited.max() {
                let semaphores = [self.shared.queue(self.queue).timeline];
                let values = [last];
                let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
                let left = deadline.saturating_duration_since(Instant::now());
                let nanos = u64::try_from(left.as_nanos()).unwrap_or(u64::MAX);
                // SAFETY: the queue's semaphore lives as long as the device, which the
                // swapchain holds; the wait is bounded.
                match unsafe { self.device().wait_semaphores(&info, nanos) } {
                    Ok(()) => {}
                    Err(vk::Result::TIMEOUT) => spent = false,
                    Err(error) => self.shared.lost_on(error),
                }
            }
            leak = !(shown && spent) && !self.shared.records().lost;
        }
        let chain = core::mem::take(&mut self.chain);
        self.forget(&chain);
        let destroy = !leak && !self.shared.raw.leaking() && !std::thread::panicking();
        if destroy {
            self.destroy_semaphores(&chain);
            // SAFETY: every present's fence has signalled, or the device is lost; nothing
            // names the swapchain's images any more (`forget`), and the semaphores' last
            // waits are done.
            unsafe {
                if chain.swapchain != vk::SwapchainKHR::null() {
                    self.presents().swapchain.destroy_swapchain(chain.swapchain, None);
                }
                for &(semaphore, _) in &self.pool {
                    self.device().destroy_semaphore(semaphore, None);
                }
                for &fence in self.pending.iter().chain(&self.spare) {
                    self.device().destroy_fence(fence, None);
                }
            }
        }
        self.shared.records().swapchain = false;
        self.shared.let_go();
    }
}

#[cfg(test)]
mod tests {
    //! The acquire semaphores' reuse rule, broken on purpose: a unit test, because no safe
    //! call can take a semaphore as free while a wait for it is pending.

    use std::sync::{Arc, Mutex, PoisonError};

    use ash::vk;
    use lactose_core::ErrorKind;
    use lactose_rhi_api::{
        Acquired, Device, DeviceRequest, Extent, HEAP_SLOTS_DEFAULT, Message, MessageSink,
        SAMPLERS_DEFAULT, Swapchain, TimelinePoint,
    };

    use super::{Use, VulkanSwapchain};
    use crate::{VulkanDevice, lock};

    /// What the layer says of an acquire that signals a semaphore a pending wait names.
    const REUSE: &str = "VUID-vkAcquireNextImageKHR-semaphore-01779";

    /// The messages a device sends, kept and not printed: `cargo xtask gpu` fails a device
    /// test that prints one, and this test asserts on the one it provokes.
    #[derive(Default)]
    struct Kept(Mutex<Vec<String>>);

    impl MessageSink for Kept {
        fn message(&self, message: &Message<'_>) {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{message:?}"));
        }
    }

    /// An acquire that reuses the last acquire's semaphore while the batch that waits for it
    /// is held on the present queue, by a host point not yet signalled, is reported by the
    /// layer; the swapchain's own pool never does so, and the device tests' runs are silent.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn an_acquire_semaphore_reused_under_a_pending_wait_is_reported() {
        // The check is core validation's, which the GPU-assisted run turns off.
        if std::env::var("VK_KHRONOS_VALIDATION_VALIDATE_CORE").is_ok_and(|value| value == "false")
        {
            eprintln!(
                "an_acquire_semaphore_reused_under_a_pending_wait_is_reported: skipped, core \
                 validation is off"
            );
            return;
        }
        let kept = Arc::new(Kept::default());
        let request = DeviceRequest {
            adapter: std::env::var("LACTOSE_GPU_ADAPTER").ok(),
            surface: None,
            heap_slots: HEAP_SLOTS_DEFAULT,
            samplers: SAMPLERS_DEFAULT,
            pipeline_cache: Vec::new(),
            messages: Arc::clone(&kept) as _,
        };
        let device = match VulkanDevice::headless(request) {
            Ok(device) => device,
            Err(error) if error.kind() == ErrorKind::Unsupported => {
                eprintln!(
                    "an_acquire_semaphore_reused_under_a_pending_wait_is_reported: skipped, \
                     {error}"
                );
                return;
            }
            Err(error) => panic!("a headless device: {error}"),
        };
        let shared = device.shared();
        let mut swapchain =
            VulkanSwapchain::new(shared, Extent { width: 64, height: 64 }).expect("a swapchain");
        let Acquired::Image(_) = swapchain.acquire().expect("the first acquire") else {
            panic!("the first acquire gave no image");
        };
        let at = swapchain.held.and_then(|held| held.semaphore).expect("the acquire's semaphore");
        // A batch on the present queue that waits for the acquire's semaphore and for a host
        // point, so that its wait is pending until the host signals.
        let gate = device.create_timeline(0).expect("a timeline");
        let host = shared.timelines.resolve(gate).expect("the timeline's object");
        let queue = swapchain.queue;
        {
            let _queue = lock(&shared.queues[queue]);
            let seq = shared.records().stamp(queue, &[], &[]);
            let point = |semaphore: vk::Semaphore, value: u64| {
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(semaphore)
                    .value(value)
                    .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            };
            let waits = [point(swapchain.pool[at].0, 0), point(host.semaphore, 1)];
            let signals = [point(shared.queue(queue).timeline, seq)];
            let info = vk::SubmitInfo2::default()
                .wait_semaphore_infos(&waits)
                .signal_semaphore_infos(&signals);
            let target = shared.queue(queue).queue;
            // SAFETY: the queue's lock is held; the acquire signalled the semaphore and no
            // batch has waited for it; the queue's own timeline grows by one.
            let submitted =
                unsafe { shared.raw.device.queue_submit2(target, &[info], vk::Fence::null()) };
            shared.records().received(queue, seq, submitted.is_err());
            submitted.expect("the held batch");
            swapchain.pool[at].1 = Use::Waited(seq);
        }
        // The breach: the semaphore taken as free while its wait is pending, and the image
        // as given up.
        swapchain.pool[at].1 = Use::Free;
        swapchain.held = None;
        let again = swapchain.acquire().expect("the second acquire");
        assert!(matches!(again, Acquired::Image(_)), "the second acquire gave {again:?}");
        assert_eq!(swapchain.held.and_then(|held| held.semaphore), Some(at), "the reuse");
        device.signal_timeline(TimelinePoint { timeline: gate, value: 1 }).expect("the gate");
        drop(swapchain);
        drop(device);
        let heard = kept.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
        assert!(
            heard.iter().any(|text| text.contains(REUSE)),
            "the layer did not report the reuse: is it loaded? {heard:?}"
        );
    }
}
