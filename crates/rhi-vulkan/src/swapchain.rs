//! The swapchain: the driver's images of the device's surface, an acquire bounded at a
//! second, and a present that submits once its walk of the records finds its submit
//! released, then presents with a present fence (`VK_EXT_swapchain_maintenance1`) and, where
//! the device reports display times, its id (`VK_GOOGLE_display_timing`).
//!
//! Each acquire signals a semaphore of a pool, which is used again only once the submit that
//! waited for it is known complete; each image has its own present semaphore, which its
//! present's submit signals and which is signalled again only once that present's fence
//! has. A present is pending until its fence signals: `resize` and the drop wait for them.
//! Neither waits for a batch of the present queue, which a submit held by a host point ahead
//! of it on a shared queue may hold: the drop hands its semaphores to the device with the
//! queue's stamp they wait behind.
//!
//! The driver swapchain is destroyed with the last count of its owner, which the swapchain
//! holds while it presents to it and each of its images' objects holds: an encoder that
//! recorded an image, or a submit still pending that names one, keeps it past a `resize` or
//! the swapchain's drop.

use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard};
use std::time::{Duration, Instant};

use ash::vk;
use lactose_core::{Error, Result};
use lactose_rhi_api::{
    Acquired, DisplayTime, Extent, Format, ImageHandle, ImageUsage, PresentId, Presented,
    QueueRole, Rule, Submit, Swapchain, TimelinePoint,
};

use crate::device::{ImageObj, Shared, alive, lost};
use crate::lock;
use crate::raw::{Presents, Raw, driver, unsupported};
use crate::records::{ImageRec, QUEUES, Stamps};
use crate::submit::{self, Binary, Staged, Staging};

/// The bound of an acquire, of a present's wait for its submit's release, and of a
/// `resize`'s wait for the swapchain's presents.
const BOUND: Duration = Duration::from_secs(1);

/// The images a swapchain asks for, within what the surface allows.
const IMAGES: u32 = 3;

/// The entries of `VK_GOOGLE_display_timing`'s history one call reads: more than MoltenVK
/// 1.4.2 keeps, 60.
const HISTORY: usize = 64;

/// The most entries one drain reads: a driver that keeps answering `VK_INCOMPLETE` is read
/// no further.
const DRAIN_MOST: usize = 4 * HISTORY;

/// The widest bracket of two `Instant` reads that the host clock's read between them is
/// paired within, if one of `PAIRING_TRIES` is: the pairing's error is half the bracket.
#[cfg(target_os = "macos")]
const PAIRING: Duration = Duration::from_micros(10);
#[cfg(target_os = "macos")]
const PAIRING_TRIES: usize = 100;

/// The formats a swapchain takes, the first the surface offers in `SRGB_NONLINEAR`.
const FORMATS: [(vk::Format, Format); 4] = [
    (vk::Format::B8G8R8A8_UNORM, Format::Bgra8Unorm),
    (vk::Format::R8G8B8A8_UNORM, Format::Rgba8Unorm),
    (vk::Format::B8G8R8A8_SRGB, Format::Bgra8Srgb),
    (vk::Format::R8G8B8A8_SRGB, Format::Rgba8Srgb),
];

/// What the images are made for: colour attachments alone, which the contract names, and
/// where `copies`, for tests that read an image back, copies from and to as well where the
/// surface allows both (`Shared::copies`).
fn usage(supported: vk::ImageUsageFlags, copies: bool) -> (vk::ImageUsageFlags, ImageUsage) {
    let copy = vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
    if copies && supported.contains(copy) {
        let usage =
            ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE).union(ImageUsage::COPY_DESTINATION);
        (vk::ImageUsageFlags::COLOR_ATTACHMENT | copy, usage)
    } else {
        (vk::ImageUsageFlags::COLOR_ATTACHMENT, ImageUsage::COLOUR)
    }
}

/// Whether the surface is empty: a current extent with a zero dimension, which Windows
/// reports for a minimised window, or no extent it allows. No swapchain is made for it.
/// `0xFFFF_FFFF` is a current extent the swapchain decides, never an empty one.
fn empty(caps: &vk::SurfaceCapabilitiesKHR) -> bool {
    let (current, most) = (caps.current_extent, caps.max_image_extent);
    current.width == 0 || current.height == 0 || most.width == 0 || most.height == 0
}

/// A driver swapchain, destroyed at its last count: the swapchain holds one while it
/// presents to it, and each of its images' objects one, so an encoder that recorded an
/// image, or a pending submit that names one, keeps it. The swapchain lets go of its count
/// only once its presents' fences have signalled, or their presents were never enqueued.
pub(crate) struct SwapchainObj {
    raw: Arc<Raw>,
    swapchain: vk::SwapchainKHR,
}

impl Drop for SwapchainObj {
    fn drop(&mut self) {
        if self.raw.leaking() {
            return;
        }
        let presents = self.raw.presents.as_ref().expect("a swapchain's device presents");
        // SAFETY: the swapchain's presents are done (the type's promise); no object of its
        // images is left, so no encoder records one and no kept submit names one; the
        // surface lives as long as the `Raw` this holds.
        unsafe { presents.swapchain.destroy_swapchain(self.swapchain, None) };
    }
}

/// What a swapchain let go of that a submit or an encoder may still use, which the device
/// keeps: let go at the swapchain's calls once nothing pending names it, and at the device's
/// last drop once it is idle.
#[derive(Default)]
pub(crate) struct Left {
    /// The objects of a chain's images given up while a submit that names one is pending,
    /// with that image's stamps.
    images: Vec<(Stamps, Arc<ImageObj>)>,
    /// The driver swapchain of a dropped swapchain whose images or semaphores something
    /// still holds: not retired, so the next swapchain is made from it.
    unretired: Option<Arc<SwapchainObj>>,
    /// The semaphores of dropped swapchains, and the driver swapchains of dropped and
    /// resized ones, whose last batches may be pending.
    spent: Vec<Spent>,
}

/// A dropped swapchain's semaphores, its acquire semaphores and its images' present
/// semaphores, with the stamp of the present queue's last batch that waits for one:
/// destroyed once that stamp is reached, or at the device's last drop, once it is idle. It
/// holds a count of the driver swapchain whose acquires signalled them, which so outlives
/// those batches; a `resize`'s holds that count alone, for the batch after an acquire it
/// gave up.
pub(crate) struct Spent {
    raw: Arc<Raw>,
    queue: usize,
    seq: u64,
    semaphores: Vec<vk::Semaphore>,
    _owner: Option<Arc<SwapchainObj>>,
}

impl Drop for Spent {
    fn drop(&mut self) {
        if self.raw.leaking() {
            return;
        }
        for &semaphore in &self.semaphores {
            // SAFETY: the present queue has reached `seq`, or the device is idle (its last
            // drop) or lost: every batch that waited for one of these has completed. Each
            // present that waited for a present semaphore is done, by its fence, which the
            // swapchain's drop waited for before it handed these over; nothing signals one
            // again.
            unsafe { self.raw.device.destroy_semaphore(semaphore, None) };
        }
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
    /// The batch that was to wait for it and the empty batch after it both failed: it is
    /// never taken again, and is destroyed with the swapchain.
    Abandoned,
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
    /// `None` until a swapchain is made, and once one is given up.
    owner: Option<Arc<SwapchainObj>>,
    images: Vec<ImageHandle>,
    /// One for each image, which its present's submit signals and its present waits for.
    present_semaphores: Vec<vk::Semaphore>,
    /// The fence of each image's last present.
    last_fence: Vec<Option<vk::Fence>>,
    /// The stamp of the last empty batch that waited for a present semaphore a present the
    /// driver did not take left signalled, or zero: the semaphores are destroyed after it.
    unsignalled: u64,
}

impl Chain {
    fn swapchain(&self) -> vk::SwapchainKHR {
        self.owner.as_ref().map_or(vk::SwapchainKHR::null(), |owner| owner.swapchain)
    }
}

/// Where a swapchain's display times come from, and what it has not handed out yet.
struct Timing {
    /// An `Instant` and the host clock's nanoseconds read within `PAIRING` of it, at the
    /// swapchain's making: the times are moved onto `Instant` by their distance from it.
    base: (Instant, u64),
    /// The reports drained and not yet handed out, in increasing id.
    backlog: Vec<DisplayTime>,
    /// The greatest id handed out or in the backlog.
    reported: u64,
    /// A drain's entries as the driver wrote them, kept for their capacity.
    entries: Vec<vk::PastPresentationTimingGOOGLE>,
}

/// A failure the unit tests inject into the making of a swapchain.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fail {
    /// `vkCreateSwapchainKHR` fails, as if out of host memory.
    Create,
    /// The image of this index is not taken into the chain, as if the image table were full.
    Image(usize),
    /// `vkQueuePresentKHR` is not called and answers this error, as if the presentation
    /// engine refused the present with nothing enqueued: its fence never signals, and its
    /// semaphore stays signalled.
    Present(vk::Result),
}

/// The walks a present left undecided, counted for the threaded test that shows another
/// thread's submit deciding them.
#[cfg(test)]
static UNDECIDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
thread_local! {
    /// The stamp the last drop on this thread handed to the device with its semaphores, for
    /// the test that checks it covers a batch the queue may complete before it is looked at.
    static HANDED: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
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
    /// Unsignalled fences for later presents: signalled ones reset, and ones no present took.
    spare: Vec<vk::Fence>,
    /// The fences of `pending` whose present failed with an error that does not say whether
    /// the present was enqueued, with when: given up, and destroyed, once the bound has
    /// passed and the present queue is idle.
    doubtful: Vec<(vk::Fence, Instant)>,
    /// An acquire or a present was out of date, the surface was empty at the last make, or
    /// the last make failed.
    stale: bool,
    presented: Option<PresentId>,
    timing: Option<Timing>,
    #[cfg(test)]
    fail: Option<Fail>,
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
            doubtful: Vec::new(),
            stale: true,
            presented: None,
            timing: timing.and_then(|_| host_clock()).map(|base| Timing {
                base,
                backlog: Vec::new(),
                reported: 0,
                entries: Vec::new(),
            }),
            #[cfg(test)]
            fail: None,
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
    /// it makes nothing and keeps what it has, stale. Once the driver is asked, the one it
    /// had is retired whatever the answer: where the making fails, that one is given up all
    /// the same, and the swapchain is left with no images, stale, as at an empty surface.
    fn make(&mut self, extent: Extent) -> Result<()> {
        let presents = self.presents();
        let (loader, surface) = (&presents.surface.loader, presents.surface.surface);
        let physical = self.shared.raw.physical;
        // SAFETY: the surface and the physical device are the device's, and live.
        let caps = unsafe { loader.get_physical_device_surface_capabilities(physical, surface) }
            .map_err(|e| self.failed(e, "vkGetPhysicalDeviceSurfaceCapabilitiesKHR"))?;
        if empty(&caps) {
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
        // Relaxed: set before the device is shared, or on the thread that makes the
        // swapchain, by the one test switch.
        let copies = self.shared.copies.load(Ordering::Relaxed);
        let (vk_usage, usage) = usage(caps.supported_usage_flags, copies);
        // A dropped swapchain's driver swapchain that an image's user still holds is the
        // surface's, not retired: the new one is made from it.
        let unretired = match self.chain.owner {
            Some(_) => None,
            None => lock(&self.shared.left).unretired.take(),
        };
        let old = self.chain.owner.as_ref().or(unretired.as_ref());
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
            .old_swapchain(old.map_or(vk::SwapchainKHR::null(), |old| old.swapchain));
        let created = self.create(&info).map_err(|e| self.failed(e, "vkCreateSwapchainKHR"));
        // Retired from here, whatever the call returned.
        drop(unretired);
        let made_chain = created.and_then(|swapchain| {
            let owner = Arc::new(SwapchainObj { raw: Arc::clone(&self.shared.raw), swapchain });
            self.chain_of(&owner, format, usage, made)
        });
        let old = core::mem::take(&mut self.chain);
        self.retire(old);
        let chain = match made_chain {
            Ok(chain) => chain,
            Err(error) => {
                self.stale = true;
                return Err(error);
            }
        };
        self.chain = chain;
        self.format = format;
        self.vk_format = vk_format;
        self.extent = Extent { width: made.width, height: made.height };
        self.stale = false;
        Ok(())
    }

    /// `vkCreateSwapchainKHR`, or the failure a unit test injects.
    fn create(
        &self,
        info: &vk::SwapchainCreateInfoKHR<'_>,
    ) -> core::result::Result<vk::SwapchainKHR, vk::Result> {
        #[cfg(test)]
        if self.fail == Some(Fail::Create) {
            return Err(vk::Result::ERROR_OUT_OF_HOST_MEMORY);
        }
        // SAFETY: the surface is live, and the swapchain it replaces, if any, is the
        // surface's and not retired; the extent and the count are within the surface's
        // bounds, FIFO is always offered, and the format was among the surface's.
        unsafe { self.presents().swapchain.create_swapchain(info, None) }
    }

    /// `vkQueuePresentKHR` on the present queue, or the failure a unit test injects. The
    /// caller holds the queue's lock; the image is the one acquired, its submit left it in
    /// `PRESENT_SRC_KHR` and signals the present semaphore `info` waits for, a binary
    /// semaphore no other wait is pending on; the fence is unsignalled and no other present
    /// names it.
    fn queue_present(&self, info: &vk::PresentInfoKHR<'_>) -> ash::prelude::VkResult<bool> {
        #[cfg(test)]
        if let Some(Fail::Present(error)) = self.fail {
            return Err(error);
        }
        let target = self.shared.queue(self.queue);
        // SAFETY: as the caller promises.
        unsafe { self.presents().swapchain.queue_present(target.queue, info) }
    }

    /// The images of a new swapchain as handles, each object holding `owner`, and a present
    /// semaphore for each. Where it fails, what it made is let go, and the driver swapchain
    /// with the last count.
    fn chain_of(
        &self,
        owner: &Arc<SwapchainObj>,
        format: Format,
        usage: ImageUsage,
        made: vk::Extent2D,
    ) -> Result<Chain> {
        let extent = Extent { width: made.width, height: made.height };
        // SAFETY: the swapchain is live.
        let images = unsafe { self.presents().swapchain.get_swapchain_images(owner.swapchain) }
            .map_err(|e| self.failed(e, "vkGetSwapchainImagesKHR"))?;
        let raw = &self.shared.raw;
        let mut chain = Chain { owner: Some(Arc::clone(owner)), ..Chain::default() };
        let mut failure = None;
        for (at, &image) in images.iter().enumerate() {
            let made = self
                .taken(at)
                .and_then(|()| ImageObj::swapchain_image(raw, owner, image, format, usage, extent))
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
            // Nothing of the new chain has been used.
            self.forget(&chain);
            self.destroy_semaphores(&chain);
            return Err(error);
        }
        Ok(chain)
    }

    /// Whether the image at `at` of a new swapchain is taken into its chain: always, but
    /// where a unit test injects a failure.
    fn taken(&self, at: usize) -> Result<()> {
        #[cfg(test)]
        if self.fail == Some(Fail::Image(at)) {
            return Err(Error::new(
                lactose_core::ErrorKind::ResourceExhausted,
                "an injected failure of a swapchain image",
            ));
        }
        let _ = at;
        Ok(())
    }

    /// Lets go of a chain the swapchain no longer presents to, its presents done: its
    /// handles are stale from now on, and its reports are kept for the next call. Its driver
    /// swapchain is destroyed with the last count, here or when the last user of an image
    /// lets go.
    fn retire(&mut self, old: Chain) {
        if old.owner.is_none() {
            return;
        }
        self.drain_reports(old.swapchain());
        self.forget(&old);
        self.destroy_semaphores(&old);
    }

    /// Removes a chain's images from the records and the table: a handle to one is stale.
    /// The object of an image a pending submit names is kept by the device until the submit
    /// completes.
    fn forget(&self, chain: &Chain) {
        let read = self.shared.read_queues();
        let mut left = lock(&self.shared.left);
        let mut records = self.shared.records();
        records.fold(&read, &[]);
        let mut kept = Vec::new();
        for &image in &chain.images {
            let rec = records.images.remove(image);
            let pending = rec.filter(|rec| records.pending(&rec.stamps) && !records.lost);
            let object =
                pending.and_then(|rec| Some((rec.stamps, self.shared.images.resolve(image)?)));
            kept.extend(object);
        }
        drop(records);
        left.images.append(&mut kept);
        drop(left);
        for &image in &chain.images {
            self.shared.images.remove(image);
        }
    }

    /// Lets go of what the device keeps for its swapchains that no pending submit names any
    /// more, of semaphores whose last batch has completed, and then of a dropped swapchain's
    /// unretired driver swapchain that nothing else holds.
    fn reap_left(&self) {
        let read = self.shared.read_queues();
        let mut left = lock(&self.shared.left);
        if left.images.is_empty() && left.unretired.is_none() && left.spent.is_empty() {
            return;
        }
        let mut released = Vec::new();
        let mut spent = Vec::new();
        {
            let mut records = self.shared.records();
            records.fold(&read, &[]);
            let mut at = 0;
            while at < left.images.len() {
                if records.pending(&left.images[at].0) && !records.lost {
                    at += 1;
                } else {
                    released.push(left.images.swap_remove(at));
                }
            }
            let mut at = 0;
            while at < left.spent.len() {
                let batch = &left.spent[at];
                if batch.seq > records.queues[batch.queue].known && !records.lost {
                    at += 1;
                } else {
                    spent.push(left.spent.swap_remove(at));
                }
            }
        }
        drop(left);
        // The last counts are dropped once the locks are given up; the unretired driver
        // swapchain is looked at after, since those counts may be its others.
        drop(released);
        drop(spent);
        let unretired = lock(&self.shared.left).unretired.take_if(|o| Arc::strong_count(o) == 1);
        drop(unretired);
    }

    fn destroy_semaphores(&self, chain: &Chain) {
        if self.shared.raw.leaking() {
            return;
        }
        for &semaphore in &chain.present_semaphores {
            // SAFETY: each present's wait for it is done (its fence), the empty batch that
            // waited for one a present left signalled has completed (`unsignalled`), and no
            // submit to come signals it. One a given-up present left signalled has no wait
            // pending, since that present enqueued nothing.
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
                self.unname(fence);
            } else {
                at += 1;
            }
        }
        Ok(())
    }

    /// No image's last present is `fence` any more, nor is it doubtful.
    fn unname(&mut self, fence: vk::Fence) {
        for last in &mut self.chain.last_fence {
            if *last == Some(fence) {
                *last = None;
            }
        }
        self.doubtful.retain(|&(doubtful, _)| doubtful != fence);
    }

    /// Waits until `deadline` for `fences`, and reaps what signalled; whether they all did.
    /// A doubtful fence that has not signalled a bound after its present, with the present
    /// queue idle, is given up and destroyed: the bound with the queue idle is taken to show
    /// that the present enqueued nothing, so nothing pending names the fence or the image's
    /// present semaphore, which is left for `destroy_semaphores`. The one known source is
    /// MoltenVK 1.4.2's present with no command buffer, whose waits and fence go nowhere; its
    /// drawable timeout, the same error, still signals the fence, which is reaped as usual.
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
        match waited {
            Ok(()) | Err(vk::Result::TIMEOUT) => {}
            Err(error) => return Err(self.failed(error, "vkWaitForFences")),
        }
        self.reap()?;
        if !self.doubtful.is_empty() {
            let idle = {
                let records = self.shared.settled();
                let queue = &records.queues[self.queue];
                queue.known >= queue.next - 1
            };
            let now = Instant::now();
            let given_up: Vec<vk::Fence> = self
                .doubtful
                .iter()
                .filter(|&&(_, at)| idle && now.saturating_duration_since(at) >= BOUND)
                .map(|&(fence, _)| fence)
                .collect();
            for fence in given_up {
                self.pending.retain(|&pending| pending != fence);
                self.unname(fence);
                // SAFETY: the bound with the present queue idle is taken to show that the
                // present enqueued nothing, so no queue operation names the fence. MoltenVK
                // 1.4.2's present with no command buffer (`MVKQueue.mm` 739-750) is the one
                // known source; its drawable timeout (`MVKImage.mm` 1625), the same error,
                // signals the fence, which `reap` takes as any other. The image's present
                // semaphore, left signalled with no wait pending, is never signalled again:
                // the error left the swapchain stale until its chain is replaced, whose
                // retirement destroys it.
                unsafe { self.device().destroy_fence(fence, None) };
            }
        }
        Ok(fences.iter().all(|fence| !self.pending.contains(fence)))
    }

    /// Waits until `deadline` for the present queue's own timeline to reach `seq`; whether
    /// it did.
    fn wait_queue(&self, seq: u64, deadline: Instant) -> Result<bool> {
        if seq <= self.shared.settled().queues[self.queue].known {
            return Ok(true);
        }
        let semaphores = [self.shared.queue(self.queue).timeline];
        let values = [seq];
        let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
        let left = deadline.saturating_duration_since(Instant::now());
        let nanos = u64::try_from(left.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the queue's semaphore lives as long as the device, which the swapchain
        // holds; the wait is bounded.
        match unsafe { self.device().wait_semaphores(&info, nanos) } {
            Ok(()) => Ok(true),
            Err(vk::Result::TIMEOUT) => Ok(false),
            Err(error) => Err(self.failed(error, "vkWaitSemaphores")),
        }
    }

    /// The fence a present takes: a spare, or a new one.
    fn fence(&mut self) -> Result<vk::Fence> {
        if let Some(fence) = self.spare.pop() {
            return Ok(fence);
        }
        let info = vk::FenceCreateInfo::default();
        // SAFETY: the device is live.
        unsafe { self.device().create_fence(&info, None) }
            .map_err(|e| self.failed(e, "vkCreateFence"))
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

    /// An empty batch on the present queue that waits for `semaphore`, a binary semaphore
    /// signalled with no wait pending, and takes the queue's next stamp, which it returns
    /// where the driver took the batch. The caller holds the queue's lock.
    fn empty_batch(&self, semaphore: vk::Semaphore) -> Result<u64> {
        let queue = self.queue;
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
        let waits = [point(semaphore, 0)];
        let signals = [point(target.timeline, seq)];
        let info = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&waits)
            .signal_semaphore_infos(&signals);
        // SAFETY: the queue's lock is held (the caller's); the semaphore is signalled with no
        // wait pending; the queue's own timeline grows by one.
        let submitted =
            unsafe { self.device().queue_submit2(target.queue, &[info], vk::Fence::null()) };
        self.shared.records().received(queue, seq, submitted.is_err());
        submitted.map(|()| seq).map_err(|e| self.failed(e, "vkQueueSubmit2"))
    }

    /// Ends a held image's acquire: where no batch has waited for its semaphore, an empty
    /// batch on the present queue waits for it, so that the semaphore is used again only
    /// after that batch completes, and never where the batch failed. The image is forgotten,
    /// acquired by a swapchain that is to be replaced or destroyed. The batch's stamp, where
    /// one was made.
    fn spend(&mut self) -> Result<Option<u64>> {
        let Some(held) = self.held.take() else { return Ok(None) };
        let Some(at) = held.semaphore else { return Ok(None) };
        let _queue = lock(&self.shared.queues[self.queue]);
        let spent = self.empty_batch(self.pool[at].0);
        self.pool[at].1 = match spent {
            Ok(seq) => Use::Waited(seq),
            Err(_) => Use::Abandoned,
        };
        spent.map(Some)
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

    /// Stages a present's submit once its walk finds it released, under the present queue's
    /// lock, which it returns held; refused past `deadline`.
    fn release<'a>(
        &self,
        shared: &'a Shared,
        submit: &mut Submit<'_>,
        deadline: Instant,
    ) -> Result<(MutexGuard<'a, ()>, Staged)> {
        loop {
            let held_queue = lock(&shared.queues[self.queue]);
            match submit::stage(shared, self.queue, QueueRole::Present, submit, true)? {
                Staging::Staged(staged) => return Ok((held_queue, staged)),
                Staging::Undecided(point) => {
                    #[cfg(test)]
                    UNDECIDED.fetch_add(1, Ordering::Relaxed);
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
                }
            }
        }
    }

    /// Adds to the backlog the reports the driver keeps for `swapchain` past the last
    /// handed out.
    fn drain_reports(&mut self, swapchain: vk::SwapchainKHR) {
        let last = self.presented.map_or(0, |PresentId(id)| id);
        let Some(timing) = self.timing.as_mut() else { return };
        let Some(google) = self.shared.raw.presents.as_ref().and_then(|p| p.timing.as_ref()) else {
            return;
        };
        if swapchain == vk::SwapchainKHR::null() {
            return;
        }
        let mut entries = core::mem::take(&mut timing.entries);
        entries.clear();
        read_history(
            |count, into| {
                // SAFETY: the swapchain is live, and this thread's alone between its acquire
                // and its present (`Swapchain` is not `Sync`); `into` holds `count` entries.
                unsafe {
                    (google.fp().get_past_presentation_timing_google)(
                        google.device(),
                        swapchain,
                        count,
                        into,
                    )
                }
            },
            &mut entries,
        );
        timing.admit(&entries, last);
        timing.entries = entries;
    }
}

/// Reads a driver's history through `read`, a `vkGetPastPresentationTimingGOOGLE` that
/// writes at most the count it is given and sets it to what it wrote, into `into`. The
/// history is taken by each call that writes it (MoltenVK takes off every entry it writes),
/// so a call's entries are kept as it wrote them, and the call is made again while more are
/// held: ash's wrapper counts first and throws a fill away on `VK_INCOMPLETE`, which would
/// lose the entries that fill took. A driver that keeps its history would give the same
/// entries again: a call that writes none, or `DRAIN_MOST` entries in all, ends the drain.
fn read_history(
    mut read: impl FnMut(&mut u32, *mut vk::PastPresentationTimingGOOGLE) -> vk::Result,
    into: &mut Vec<vk::PastPresentationTimingGOOGLE>,
) {
    let mut buffer = [vk::PastPresentationTimingGOOGLE::default(); HISTORY];
    loop {
        let mut count = HISTORY as u32;
        let result = read(&mut count, buffer.as_mut_ptr());
        if result != vk::Result::SUCCESS && result != vk::Result::INCOMPLETE {
            return;
        }
        let written = (count as usize).min(HISTORY);
        into.extend_from_slice(&buffer[..written]);
        if result == vk::Result::SUCCESS || written == 0 || into.len() >= DRAIN_MOST {
            return;
        }
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

    /// Adds to the backlog the reports of `entries`, in any order and any repeated, whose
    /// ids, widened against `last`, the newest present's, are past the greatest reported:
    /// each once, in increasing id. No allocation once the backlog has room.
    fn admit(&mut self, entries: &[vk::PastPresentationTimingGOOGLE], last: u64) {
        let start = self.backlog.len();
        for entry in entries {
            let id = widen(entry.present_id, last);
            if let Some(shown) = self.at(entry.actual_present_time)
                && id > self.reported
            {
                self.backlog.push(DisplayTime { present: PresentId(id), shown });
            }
        }
        self.backlog[start..].sort_unstable_by_key(|time| time.present);
        // Every id before `start` is at or below `reported`, below every new one.
        self.backlog.dedup_by_key(|time| time.present);
        if let Some(newest) = self.backlog.last() {
            self.reported = self.reported.max(newest.present.0);
        }
    }
}

/// An `Instant` and the host clock `VK_GOOGLE_display_timing` reports on under MoltenVK,
/// `CLOCK_UPTIME_RAW`: the clock of `mach_absolute_time` and of `CACurrentMediaTime`, whose
/// seconds MoltenVK multiplies into `actualPresentTime`, and `Instant`'s on macOS. The clock
/// is read between two `Instant`s, again until they are within `PAIRING` or for
/// `PAIRING_TRIES` reads, and the narrowest read is paired with its bracket's midpoint.
#[cfg(target_os = "macos")]
fn host_clock() -> Option<(Instant, u64)> {
    let mut best: Option<(Duration, Instant, u64)> = None;
    for _ in 0..PAIRING_TRIES {
        let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        let before = Instant::now();
        // SAFETY: a read of a clock into a local.
        let read = unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut time) };
        let after = Instant::now();
        if read != 0 {
            return None;
        }
        let seconds = u64::try_from(time.tv_sec).ok()?;
        let nanos = u64::try_from(time.tv_nsec).ok()?;
        let clock = seconds.checked_mul(1_000_000_000)?.checked_add(nanos)?;
        let bracket = after.saturating_duration_since(before);
        if best.is_none_or(|(narrowest, ..)| bracket < narrowest) {
            best = Some((bracket, before + bracket / 2, clock));
        }
        if bracket <= PAIRING {
            break;
        }
    }
    best.map(|(_, instant, clock)| (instant, clock))
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
        if self.chain.owner.is_none() { Extent { width: 0, height: 0 } } else { self.extent }
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
        self.reap_left();
        if self.stale || self.chain.owner.is_none() {
            return Ok(Acquired::Stale);
        }
        let at = self.acquire_semaphore()?;
        let nanos = u64::try_from(BOUND.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the swapchain is this one's, not retired (`stale` is set wherever it may
        // be), and reached by this thread alone (`Swapchain` is not `Sync`); the semaphore
        // is unsignalled with no operation pending (`Use::Free`).
        let acquired = unsafe {
            self.presents().swapchain.acquire_next_image(
                self.chain.swapchain(),
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
        // Taken before the submit: once the submit is taken, a failure is a loss or a
        // present taken stale (6(i)).
        let fence = self.fence()?;
        let shared = Arc::clone(&self.shared);
        let (held_queue, staged) = match self.release(&shared, &mut submit, deadline) {
            Ok(released) => released,
            Err(error) => {
                self.spare.push(fence);
                return Err(error);
            }
        };
        let seq = staged.seq;
        let binary = Binary {
            wait: held.semaphore.map(|at| self.pool[at].0),
            signal: Some(self.chain.present_semaphores[index]),
        };
        let (sent, spent) = submit::send(&shared, staged, &submit, binary);
        if let Some(at) = held.semaphore {
            self.pool[at].1 = if spent { Use::Waited(seq) } else { Use::Abandoned };
        }
        // The acquire's semaphore is spent, by the submit or the empty batch after its
        // failure, or abandoned: the image stays acquired, and its next present waits for
        // nothing.
        self.held = Some(Held { semaphore: None, ..held });
        if let Err(error) = sent {
            self.spare.push(fence);
            return Err(error);
        }
        let id = self.presented.map_or(1, |PresentId(last)| last + 1);
        let fences = [fence];
        let mut fence_info = vk::SwapchainPresentFenceInfoEXT::default().fences(&fences);
        let times = [vk::PresentTimeGOOGLE {
            // The driver carries 32 bits; `widen` gives the reports theirs back.
            present_id: id as u32,
            desired_present_time: 0,
        }];
        let mut time_info = vk::PresentTimesInfoGOOGLE::default().times(&times);
        let swapchains = [self.chain.swapchain()];
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
        let shown = self.queue_present(&info);
        self.held = None;
        let presented = match shown {
            // `VK_SUBOPTIMAL_KHR` is shown, never stale.
            Ok(_) => {
                self.pending.push(fence);
                self.chain.last_fence[index] = Some(fence);
                Presented::Shown
            }
            Err(vk::Result::ERROR_DEVICE_LOST) => {
                drop(held_queue);
                self.pending.push(fence);
                self.shared.lost_on(vk::Result::ERROR_DEVICE_LOST);
                return Err(lost());
            }
            // Not enqueued: the fence and the present semaphore are as they were. The
            // semaphore the submit signalled is waited for by an empty batch, so that the
            // image's next present may signal it again, and the fence is another's.
            Err(vk::Result::ERROR_OUT_OF_HOST_MEMORY | vk::Result::ERROR_OUT_OF_DEVICE_MEMORY) => {
                self.spare.push(fence);
                if let Ok(seq) = self.empty_batch(self.chain.present_semaphores[index]) {
                    self.chain.unsignalled = seq;
                }
                self.stale = true;
                Presented::Stale
            }
            // Enqueued, by the spec: the fence signals.
            Err(
                vk::Result::ERROR_OUT_OF_DATE_KHR
                | vk::Result::ERROR_SURFACE_LOST_KHR
                | vk::Result::ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT,
            ) => {
                self.pending.push(fence);
                self.chain.last_fence[index] = Some(fence);
                self.stale = true;
                Presented::Stale
            }
            // An error the spec does not list, which says nothing of the fence: MoltenVK's
            // `VK_ERROR_OUT_OF_POOL_MEMORY`, after which the fence signals where the drawable
            // timed out and never where the present had no command buffer.
            Err(_) => {
                self.pending.push(fence);
                self.chain.last_fence[index] = Some(fence);
                self.doubtful.push((fence, Instant::now()));
                self.stale = true;
                Presented::Stale
            }
        };
        drop(held_queue);
        self.presented = Some(PresentId(id));
        Ok(presented)
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
        if !self.wait_queue(self.chain.unsignalled, deadline)? {
            return Err(Rule::Pending.refuse(
                "a resize while the batch after a present the driver did not take is pending \
                 past the bound",
            ));
        }
        // The batch that waits for a given-up acquire's semaphore holds the driver swapchain,
        // which is retired below, as the drop's batches hold it; the semaphore stays the
        // pool's.
        if let Some(seq) = self.spend()? {
            let spent = Spent {
                raw: Arc::clone(&self.shared.raw),
                queue: self.queue,
                seq,
                semaphores: Vec::new(),
                _owner: self.chain.owner.clone(),
            };
            lock(&self.shared.left).spent.push(spent);
        }
        self.reap_left();
        self.make(extent)
    }

    fn last_present(&self) -> Option<PresentId> {
        self.presented
    }

    fn display_times(&mut self, into: &mut Vec<DisplayTime>) -> bool {
        if self.timing.is_none() {
            return false;
        }
        self.drain_reports(self.chain.swapchain());
        if let Some(timing) = self.timing.as_mut() {
            into.append(&mut timing.backlog);
        }
        true
    }
}

impl VulkanSwapchain {
    /// The drop's wait, three bounds, for the presents' fences, which the presentation
    /// engine signals; then a held image's acquire semaphore is spent by an empty batch,
    /// which is not waited for. Whether the presents ended, or the device is lost.
    fn finish(&mut self) -> bool {
        // A present is pending until its fence signals, which the presentation engine
        // gives within a few refreshes. No batch of the present queue is waited for: where
        // the present role shares a queue, a submit held there by a host point holds every
        // batch after it, for as long as its caller keeps the point unsignalled.
        let deadline = Instant::now() + BOUND * 3;
        let pending = self.pending.clone();
        let shown = self.wait_fences(&pending, deadline).unwrap_or(false);
        let _ = self.spend();
        shown || self.shared.records().lost
    }

    /// The drop's check while its thread unwinds a panic, which waits for nothing: whether
    /// no present is pending and no acquire semaphore is left signalled with no batch to
    /// wait for it.
    fn quiet(&mut self) -> bool {
        if self.reap().is_err() {
            return self.shared.records().lost;
        }
        let spent = self.pool.iter().all(|entry| entry.1 != Use::Signalled);
        self.pending.is_empty() && spent
    }

    /// The stamp of the present queue's last batch that waits for a semaphore of the
    /// swapchain: the empty batches after its acquires, and after a present the driver did
    /// not take.
    fn last_batch(&self) -> u64 {
        let waited = self.pool.iter().filter_map(|entry| match entry.1 {
            Use::Waited(seq) => Some(seq),
            Use::Free | Use::Signalled | Use::Abandoned => None,
        });
        waited.max().unwrap_or(0).max(self.chain.unsignalled)
    }
}

impl Drop for VulkanSwapchain {
    fn drop(&mut self) {
        let lost = self.shared.records().lost;
        let done = lost || if std::thread::panicking() { self.quiet() } else { self.finish() };
        // Read before the chain is taken: the batch after a present the driver did not take
        // is the chain's (`unsignalled`).
        let seq = self.last_batch();
        let chain = core::mem::take(&mut self.chain);
        self.forget(&chain);
        if !done {
            // Something of the swapchain may still be in the presentation engine's or a
            // queue's use: the device leaks with it, nothing of either destroyed under that
            // use, and the records keep the swapchain's mark, so that a later
            // `create_swapchain` is refused and never reaches the driver with the surface
            // in use.
            self.shared.raw.leak();
            self.shared.let_go();
            return;
        }
        if !self.shared.raw.leaking() {
            // SAFETY: every present's fence has signalled, or its present was never
            // enqueued, or the device is lost. A fence given up as doubtful, a bound after
            // its present with the present queue idle, which is taken to show that the
            // present enqueued nothing (MoltenVK 1.4.2's present with no command buffer is
            // the one known source), was destroyed then and is in neither list.
            unsafe {
                for &fence in self.pending.iter().chain(&self.spare) {
                    self.device().destroy_fence(fence, None);
                }
            }
        }
        // The semaphores go to the device with the stamp of the last batch that waits for
        // one and a count of the driver swapchain; `reap_left` below destroys them at once
        // where that batch has completed.
        let mut semaphores: Vec<vk::Semaphore> = self.pool.iter().map(|entry| entry.0).collect();
        semaphores.extend_from_slice(&chain.present_semaphores);
        let spent = Spent {
            raw: Arc::clone(&self.shared.raw),
            queue: self.queue,
            seq,
            semaphores,
            _owner: chain.owner.clone(),
        };
        #[cfg(test)]
        HANDED.set(spent.seq);
        let mut left = lock(&self.shared.left);
        left.spent.push(spent);
        // The driver swapchain is the surface's still while an image's object or the
        // semaphores' batches hold it: it is kept for the next swapchain, which is made from
        // it, and destroyed with its last count, at the latest at the device's last drop.
        if let Some(owner) = chain.owner {
            left.unretired = Some(owner);
        }
        drop(left);
        self.reap_left();
        self.shared.records().swapchain = false;
        self.shared.let_go();
    }
}

#[cfg(test)]
mod tests {
    //! The pure parts with no device: the empty surface, the drain of the display-time
    //! history and its ids. Then, on a driver, what no safe call can reach: an acquire
    //! semaphore reused under a pending wait, a drop that leaks, an injected failure of a
    //! resize, and the walk a frame thread's submit decides, which a counter shows.

    use std::panic::AssertUnwindSafe;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex, PoisonError, mpsc};
    use std::time::{Duration, Instant};

    use ash::vk;
    use lactose_core::ErrorKind;
    use lactose_rhi_api::{
        Acquired, BufferCopy, BufferDesc, BufferState, BufferTransition, BufferUsage, Device,
        DeviceRequest, DisplayTime, Encoder, Extent, HEAP_SLOTS_DEFAULT, ImageHandle, ImageRange,
        ImageState, ImageTransition, ImageUsage, MemoryClass, Message, MessageSink, PresentId,
        Presented, QueueRole, Rule, SAMPLERS_DEFAULT, Sharing, StateOn, Submit, Swapchain,
        TimelineHandle, TimelinePoint,
    };

    use super::{
        BOUND, DRAIN_MOST, Fail, HANDED, HISTORY, Timing, UNDECIDED, Use, VulkanSwapchain, empty,
        read_history, usage, widen,
    };
    use crate::{VulkanDevice, lock};

    fn caps(current: (u32, u32), most: (u32, u32)) -> vk::SurfaceCapabilitiesKHR {
        vk::SurfaceCapabilitiesKHR {
            current_extent: vk::Extent2D { width: current.0, height: current.1 },
            max_image_extent: vk::Extent2D { width: most.0, height: most.1 },
            ..Default::default()
        }
    }

    /// A current extent with a zero dimension, as Windows reports for a minimised window,
    /// makes no swapchain, nor does a surface that allows no extent; the special extent
    /// `0xFFFF_FFFF`, which the swapchain decides, is not empty.
    #[test]
    fn a_surface_with_a_zero_dimension_is_empty() {
        assert!(empty(&caps((0, 0), (4096, 4096))));
        assert!(empty(&caps((0, 600), (4096, 4096))));
        assert!(empty(&caps((800, 0), (4096, 4096))));
        assert!(empty(&caps((800, 600), (0, 4096))));
        assert!(!empty(&caps((800, 600), (4096, 4096))));
        assert!(!empty(&caps((u32::MAX, u32::MAX), (4096, 4096))));
    }

    /// The images are colour attachments alone without copies, or where the surface does
    /// not allow both copies; with them, copies from and to as well.
    #[test]
    fn the_images_take_copies_only_where_asked_and_allowed() {
        let colour = vk::ImageUsageFlags::COLOR_ATTACHMENT;
        let copy = vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
        let all = colour | copy | vk::ImageUsageFlags::SAMPLED;
        assert_eq!(usage(all, false), (colour, ImageUsage::COLOUR));
        let source = colour | vk::ImageUsageFlags::TRANSFER_SRC;
        assert_eq!(usage(source, true), (colour, ImageUsage::COLOUR));
        let copies =
            ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE).union(ImageUsage::COPY_DESTINATION);
        assert_eq!(usage(all, true), (colour | copy, copies));
    }

    /// The driver carries a present id's low 32 bits; it is widened against the newest id
    /// taken, across the 2^32 wrap.
    #[test]
    fn a_reported_id_is_widened_across_the_wrap() {
        assert_eq!(widen(5, 7), 5);
        assert_eq!(widen(u32::MAX, 1 << 32 | 1), 0xFFFF_FFFF);
        assert_eq!(widen(0, 1 << 32), 1 << 32);
        assert_eq!(widen(1, 1 << 32 | 1), 1 << 32 | 1);
        // An id not yet taken reads as the one 2^32 before it; at the start there is none.
        assert_eq!(widen(9, 3 << 32 | 2), 2 << 32 | 9);
        assert_eq!(widen(9, 2), 0);
    }

    fn timing() -> Timing {
        Timing {
            base: (Instant::now(), 1_000_000_000),
            backlog: Vec::new(),
            reported: 0,
            entries: Vec::new(),
        }
    }

    fn entry(id: u32, nanos: u64) -> vk::PastPresentationTimingGOOGLE {
        vk::PastPresentationTimingGOOGLE {
            present_id: id,
            actual_present_time: nanos,
            ..Default::default()
        }
    }

    fn ids(times: &[DisplayTime]) -> Vec<u64> {
        times.iter().map(|time| time.present.0).collect()
    }

    /// Entries in any order and repeated are admitted once each, in increasing id, at the
    /// `Instant` their host time is; an id at or below the greatest reported is never
    /// admitted again; a backlog from a replaced swapchain is followed by the next batch.
    #[test]
    fn reports_are_admitted_once_each_in_increasing_id() {
        let mut timing = timing();
        let (base, nanos) = timing.base;
        timing.admit(&[entry(3, nanos + 3), entry(1, nanos + 1), entry(3, nanos + 3)], 3);
        assert_eq!(ids(&timing.backlog), [1, 3]);
        assert_eq!(timing.backlog[1].shown, base + Duration::from_nanos(3));
        assert_eq!(timing.reported, 3);
        // A replaced swapchain's backlog is kept; the next batch follows it, and what the
        // backlog holds or what was reported is never admitted twice.
        timing.admit(&[entry(5, nanos + 5), entry(2, nanos + 2), entry(4, nanos - 4)], 5);
        assert_eq!(ids(&timing.backlog), [1, 3, 4, 5]);
        assert_eq!(timing.backlog[2].shown, base - Duration::from_nanos(4));
        let mut handed = Vec::new();
        handed.append(&mut timing.backlog);
        timing.admit(&[entry(5, nanos + 5), entry(1, nanos + 1)], 5);
        assert!(timing.backlog.is_empty(), "{:?}", timing.backlog);
        // Across the wrap: the newest is 2^32 + 1, and the driver says 0xFFFF_FFFF and 1.
        timing.reported = 0xFFFF_FFFE;
        timing.admit(&[entry(1, nanos + 7), entry(u32::MAX, nanos + 6)], 1 << 32 | 1);
        assert_eq!(ids(&timing.backlog), [0xFFFF_FFFF, 1 << 32 | 1]);
        assert_eq!(timing.reported, 1 << 32 | 1);
        assert_eq!(ids(&handed), [1, 3, 4, 5]);
    }

    /// The history is read in calls of `HISTORY` entries while the driver answers
    /// `VK_INCOMPLETE`, each call's entries kept; a call that writes none ends it, and a
    /// driver that never stops answering `VK_INCOMPLETE` is read no further than
    /// `DRAIN_MOST` entries.
    #[test]
    fn the_history_is_read_until_complete_or_the_guard() {
        // 150 entries, taken off as they are written.
        let mut held: Vec<u32> = (1..=150).collect();
        let mut calls = 0;
        let mut into = Vec::new();
        read_history(
            |count, buffer| {
                calls += 1;
                let written = (*count as usize).min(held.len());
                for (at, id) in held.drain(..written).enumerate() {
                    // SAFETY: the buffer holds `count` entries, and `at` is below it.
                    unsafe { buffer.add(at).write(entry(id, 0)) };
                }
                *count = written as u32;
                if held.is_empty() { vk::Result::SUCCESS } else { vk::Result::INCOMPLETE }
            },
            &mut into,
        );
        assert_eq!(calls, 3);
        assert_eq!(
            into.iter().map(|entry| entry.present_id).collect::<Vec<_>>(),
            (1..=150).collect::<Vec<_>>()
        );
        // A driver that keeps its history answers `VK_INCOMPLETE` with the same entries.
        let mut calls = 0;
        let mut into = Vec::new();
        read_history(
            |count, buffer| {
                calls += 1;
                for at in 0..*count as usize {
                    // SAFETY: as above.
                    unsafe { buffer.add(at).write(entry(at as u32, 0)) };
                }
                vk::Result::INCOMPLETE
            },
            &mut into,
        );
        assert_eq!(into.len(), DRAIN_MOST);
        assert_eq!(calls, DRAIN_MOST / HISTORY);
        // One that writes none, or fails, ends the read with what it wrote.
        let mut into = Vec::new();
        read_history(
            |count, _| {
                *count = 0;
                vk::Result::INCOMPLETE
            },
            &mut into,
        );
        assert!(into.is_empty());
        read_history(|_, _| vk::Result::ERROR_OUT_OF_HOST_MEMORY, &mut into);
        assert!(into.is_empty());
    }

    /// The messages a device sends, kept and not printed: `cargo xtask gpu` fails a device
    /// test that prints one, and these tests assert on what they provoke or on silence.
    #[derive(Default)]
    struct Kept(Mutex<Vec<String>>);

    impl MessageSink for Kept {
        fn message(&self, message: &Message<'_>) {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{message:?}"));
        }
    }

    impl Kept {
        fn heard(&self) -> Vec<String> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
        }
    }

    /// A headless device sending to `kept`, or `None` where the driver offers no headless
    /// surface.
    fn device(kept: &Arc<Kept>, test: &str) -> Option<VulkanDevice> {
        let request = DeviceRequest {
            adapter: std::env::var("LACTOSE_GPU_ADAPTER").ok(),
            surface: None,
            heap_slots: HEAP_SLOTS_DEFAULT,
            samplers: SAMPLERS_DEFAULT,
            pipeline_cache: Vec::new(),
            messages: Arc::clone(kept) as _,
        };
        match VulkanDevice::headless(request) {
            Ok(device) => Some(device),
            Err(error) if error.kind() == ErrorKind::Unsupported => {
                eprintln!("{test}: skipped, {error}");
                None
            }
            Err(error) => panic!("a headless device: {error}"),
        }
    }

    const SMALL: Extent = Extent { width: 64, height: 64 };
    const P: QueueRole = QueueRole::Present;
    const G: QueueRole = QueueRole::Graphics;

    fn at(timeline: TimelineHandle, value: u64) -> TimelinePoint {
        TimelinePoint { timeline, value }
    }

    /// The next image, within a few bounds.
    fn image(swapchain: &mut VulkanSwapchain) -> ImageHandle {
        let start = Instant::now();
        loop {
            match swapchain.acquire().expect("an acquire") {
                Acquired::Image(image) => return image,
                Acquired::None if start.elapsed() < BOUND * 10 => {}
                other => panic!("the acquire gave {other:?}"),
            }
        }
    }

    /// An encoder of the present role that leaves `image` ready to present.
    fn presentable(device: &VulkanDevice, image: ImageHandle) -> Box<dyn Encoder> {
        let mut encoder = device.create_encoder(P, "present").expect("an encoder");
        encoder.transition_images(&[ImageTransition {
            image,
            range: ImageRange::FIRST,
            from: StateOn { state: ImageState::Undefined, queue: P },
            to: StateOn { state: ImageState::Present, queue: P },
        }]);
        encoder
    }

    /// A graphics submit that keeps the GPU busy for a while and waits for nothing, so that it
    /// is released at once: copies between two device buffers, a barrier between each. It
    /// signals `signal`; its encoders are returned, to be dropped once that is reached.
    fn slow_submit(device: &VulkanDevice, signal: TimelinePoint) -> Vec<Box<dyn Encoder>> {
        const SIZE: u64 = 128 << 20;
        const COPIES: usize = 32;
        let usage = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
        let buffer = |label| {
            let memory = MemoryClass::Device;
            let desc = BufferDesc { label, size: SIZE, usage, memory, sharing: Sharing::Exclusive };
            device.create_buffer(&desc).expect("a buffer")
        };
        let (mut from, mut to) = (buffer("slow-a"), buffer("slow-b"));
        let on = |state| StateOn { state, queue: G };
        let flip = |from, to| {
            [
                BufferTransition {
                    buffer: from,
                    from: on(BufferState::CopySource),
                    to: on(BufferState::CopyDestination),
                },
                BufferTransition {
                    buffer: to,
                    from: on(BufferState::CopyDestination),
                    to: on(BufferState::CopySource),
                },
            ]
        };
        let mut encoder = device.create_encoder(G, "slow").expect("an encoder");
        encoder.transition_buffers(&[
            BufferTransition {
                buffer: from,
                from: on(BufferState::Undefined),
                to: on(BufferState::CopySource),
            },
            BufferTransition {
                buffer: to,
                from: on(BufferState::Undefined),
                to: on(BufferState::CopyDestination),
            },
        ]);
        let all = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
        for _ in 0..COPIES {
            encoder.copy_buffer(from, to, &all);
            encoder.transition_buffers(&flip(from, to));
            core::mem::swap(&mut from, &mut to);
        }
        let mut encoders = [encoder];
        let submit = Submit { encoders: &mut encoders, waits: &[], signals: &[signal] };
        device.submit(G, submit).expect("the slow submit");
        encoders.into_iter().collect()
    }

    /// What the layer said that reports a breach, of the spec or of synchronisation.
    fn breaches(kept: &Kept) -> Vec<String> {
        let breach = ["VUID-", "UNASSIGNED-", "SYNC-"];
        kept.heard().into_iter().filter(|text| breach.iter().any(|b| text.contains(b))).collect()
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
        const REUSE: &str = "VUID-vkAcquireNextImageKHR-semaphore-01779";
        let kept = Arc::new(Kept::default());
        let Some(device) =
            device(&kept, "an_acquire_semaphore_reused_under_a_pending_wait_is_reported")
        else {
            return;
        };
        let shared = device.shared();
        let mut swapchain = VulkanSwapchain::new(shared, SMALL).expect("a swapchain");
        let Acquired::Image(_) = swapchain.acquire().expect("the first acquire") else {
            panic!("the first acquire gave no image");
        };
        let held = swapchain.held.and_then(|held| held.semaphore).expect("the acquire's semaphore");
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
            let waits = [point(swapchain.pool[held].0, 0), point(host.semaphore, 1)];
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
            swapchain.pool[held].1 = Use::Waited(seq);
        }
        // The breach: the semaphore taken as free while its wait is pending, and the image
        // as given up.
        swapchain.pool[held].1 = Use::Free;
        swapchain.held = None;
        let again = swapchain.acquire().expect("the second acquire");
        assert!(matches!(again, Acquired::Image(_)), "the second acquire gave {again:?}");
        assert_eq!(swapchain.held.and_then(|held| held.semaphore), Some(held), "the reuse");
        device.signal_timeline(at(gate, 1)).expect("the gate");
        drop(swapchain);
        drop(device);
        let heard = kept.heard();
        assert!(
            heard.iter().any(|text| text.contains(REUSE)),
            "the layer did not report the reuse: is it loaded? {heard:?}"
        );
    }

    /// A drop whose present never ends within its wait leaks the swapchain and the device
    /// with it: the device's drop then destroys nothing under the presentation engine, so the
    /// layer reports no child left alive, and a second swapchain is refused by the records
    /// and never reaches the driver.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn a_drop_that_leaks_leaks_the_device_and_refuses_a_second_swapchain() {
        let kept = Arc::new(Kept::default());
        let Some(device) =
            device(&kept, "a_drop_that_leaks_leaks_the_device_and_refuses_a_second_swapchain")
        else {
            return;
        };
        let shared = Arc::clone(device.shared());
        let mut swapchain = VulkanSwapchain::new(&shared, SMALL).expect("a swapchain");
        // A present's fence that never signals.
        let info = vk::FenceCreateInfo::default();
        // SAFETY: the device is live.
        let fence = unsafe { shared.raw.device.create_fence(&info, None) }.expect("a fence");
        swapchain.pending.push(fence);
        drop(swapchain);
        assert!(shared.raw.leaking(), "the drop did not leak the device");
        let again = VulkanSwapchain::new(&shared, SMALL).err().expect("a second swapchain");
        assert_eq!(Rule::of(&again), Some(Rule::Swapchain), "{again}");
        drop(shared);
        drop(device);
        assert_eq!(breaches(&kept), Vec::<String>::new());
    }

    /// The present role served by the graphics queue, as on a driver of one queue: a
    /// graphics submit that writes the acquired image, held by a host point, is pending on
    /// that queue while the swapchain is dropped. The drop waits for nothing behind it: the
    /// device does not leak, a swapchain is made again at once, and once the point is
    /// signalled it presents, with the layer silent to the device's drop.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn a_drop_behind_a_held_submit_on_the_shared_queue_lets_a_swapchain_be_made_again() {
        let test = "a_drop_behind_a_held_submit_on_the_shared_queue_lets_a_swapchain_be_made_again";
        let kept = Arc::new(Kept::default());
        crate::raw::PRESENT_ON_GRAPHICS.set(true);
        let made = device(&kept, test);
        crate::raw::PRESENT_ON_GRAPHICS.set(false);
        let Some(device) = made else { return };
        let shared = Arc::clone(device.shared());
        assert_eq!(shared.queue_of(P), shared.queue_of(G), "the present role has its own queue");
        let mut swapchain = VulkanSwapchain::new(&shared, SMALL).expect("a swapchain");
        let held = image(&mut swapchain);
        let gate = device.create_timeline(0).expect("a timeline");
        let drawn = device.create_timeline(0).expect("a timeline");
        let mut draw = device.create_encoder(G, "frame").expect("an encoder");
        draw.transition_images(&[ImageTransition {
            image: held,
            range: ImageRange::FIRST,
            from: StateOn { state: ImageState::Undefined, queue: G },
            to: StateOn { state: ImageState::ColourAttachment, queue: G },
        }]);
        let mut draws = [draw];
        let waits = [at(gate, 1)];
        let signals = [at(drawn, 1)];
        device
            .submit(G, Submit { encoders: &mut draws, waits: &waits, signals: &signals })
            .expect("the frame's submit");
        let start = Instant::now();
        drop(swapchain);
        let took = start.elapsed();
        eprintln!("{test}: the drop took {took:?}");
        let mut swapchain = VulkanSwapchain::new(&shared, SMALL).expect("a swapchain again");
        assert!(!shared.raw.leaking(), "the drop leaked the device");
        device.signal_timeline(waits[0]).expect("the gate");
        assert!(device.wait_timeline(signals[0], BOUND * 5).expect("the wait"), "the submit");
        drop(draws);
        let image = image(&mut swapchain);
        let mut encoders = [presentable(&device, image)];
        let submit = Submit { encoders: &mut encoders, waits: &[], signals: &[] };
        assert_eq!(swapchain.present(image, submit).expect("a present"), Presented::Shown);
        drop(swapchain);
        drop(encoders);
        drop(shared);
        drop(device);
        assert_eq!(breaches(&kept), Vec::<String>::new());
    }

    /// A present the driver did not take (out of host memory, injected) leaves the image's
    /// present semaphore waited for by an empty batch (`unsignalled`), here behind released
    /// slow work. The drop hands its semaphores to the device with a stamp at or past that
    /// batch, whether or not the batch has completed by then; the next swapchain's reaps
    /// race its completion, and the layer says nothing.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn a_drop_after_a_present_the_driver_did_not_take_keeps_its_semaphores_past_that_batch() {
        let test =
            "a_drop_after_a_present_the_driver_did_not_take_keeps_its_semaphores_past_that_batch";
        let kept = Arc::new(Kept::default());
        let Some(device) = device(&kept, test) else { return };
        let shared = Arc::clone(device.shared());
        let mut swapchain = VulkanSwapchain::new(&shared, SMALL).expect("a swapchain");
        let first = image(&mut swapchain);
        let slow = device.create_timeline(0).expect("a timeline");
        let busy = slow_submit(&device, at(slow, 1));
        let mut encoders = [presentable(&device, first)];
        swapchain.fail = Some(Fail::Present(vk::Result::ERROR_OUT_OF_HOST_MEMORY));
        let submit = Submit { encoders: &mut encoders, waits: &[at(slow, 1)], signals: &[] };
        assert_eq!(swapchain.present(first, submit).expect("the present"), Presented::Stale);
        swapchain.fail = None;
        let unsignalled = swapchain.chain.unsignalled;
        assert!(unsignalled > 0, "no empty batch after the present");
        HANDED.set(0);
        drop(swapchain);
        let handed = HANDED.get();
        let left: Vec<u64> = lock(&shared.left).spent.iter().map(|spent| spent.seq).collect();
        eprintln!("{test}: the empty batch {unsignalled}, handed {handed}, pending {left:?}");
        assert!(handed >= unsignalled, "the stamp {handed} is short of the batch {unsignalled}");
        assert!(left.iter().all(|&seq| seq >= unsignalled), "{left:?} short of {unsignalled}");
        let next = VulkanSwapchain::new(&shared, SMALL).expect("a swapchain again");
        let deadline = Instant::now() + BOUND * 10;
        while !lock(&shared.left).spent.is_empty() && Instant::now() < deadline {
            next.reap_left();
        }
        assert!(device.wait_timeline(at(slow, 1), BOUND * 10).expect("the wait"), "slow work");
        drop(next);
        drop(encoders);
        drop(busy);
        drop(shared);
        drop(device);
        assert_eq!(breaches(&kept), Vec::<String>::new());
    }

    /// The present role served by the graphics queue, behind a graphics submit held by a host
    /// point that names no image of the swapchain: an image acquired and not presented is
    /// given up by a `resize` or by the drop, whose empty batch waits for its acquire's
    /// semaphore behind the held submit. No image object holds the driver swapchain, so the
    /// count handed to the device with that batch's stamp alone keeps it, until the point
    /// is signalled and the batch completes; then it goes. The layer says nothing.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn a_driver_swapchain_outlives_the_batch_that_waits_for_its_acquire() {
        let test = "a_driver_swapchain_outlives_the_batch_that_waits_for_its_acquire";
        for by_drop in [false, true] {
            let kept = Arc::new(Kept::default());
            crate::raw::PRESENT_ON_GRAPHICS.set(true);
            let made = device(&kept, test);
            crate::raw::PRESENT_ON_GRAPHICS.set(false);
            let Some(device) = made else { return };
            let shared = Arc::clone(device.shared());
            assert_eq!(
                shared.queue_of(P),
                shared.queue_of(G),
                "the present role has its own queue"
            );
            let mut swapchain = VulkanSwapchain::new(&shared, SMALL).expect("a swapchain");
            let _held = image(&mut swapchain);
            let owner = swapchain.chain.owner.as_ref().expect("a driver swapchain");
            let owner = Arc::downgrade(owner);
            let gate = device.create_timeline(0).expect("a timeline");
            let done = device.create_timeline(0).expect("a timeline");
            let mut held = [device.create_encoder(G, "held").expect("an encoder")];
            let (waits, signals) = ([at(gate, 1)], [at(done, 1)]);
            let submit = Submit { encoders: &mut held, waits: &waits, signals: &signals };
            device.submit(G, submit).expect("the held submit");
            let next = if by_drop {
                drop(swapchain);
                None
            } else {
                swapchain.resize(Extent { width: 96, height: 48 }).expect("the resize");
                Some(swapchain)
            };
            let pending = !lock(&shared.left).spent.is_empty();
            assert!(pending, "by_drop {by_drop}: no batch handed over");
            assert!(owner.upgrade().is_some(), "by_drop {by_drop}: the driver swapchain went");
            let next = next.unwrap_or_else(|| {
                VulkanSwapchain::new(&shared, SMALL).expect("a swapchain again")
            });
            assert!(owner.upgrade().is_some(), "by_drop {by_drop}: the driver swapchain went");
            device.signal_timeline(waits[0]).expect("the gate");
            assert!(device.wait_timeline(signals[0], BOUND * 5).expect("the wait"), "held");
            let deadline = Instant::now() + BOUND * 10;
            while !lock(&shared.left).spent.is_empty() && Instant::now() < deadline {
                next.reap_left();
            }
            assert!(lock(&shared.left).spent.is_empty(), "by_drop {by_drop}: still handed");
            assert!(owner.upgrade().is_none(), "by_drop {by_drop}: the driver swapchain stayed");
            drop(next);
            drop(held);
            drop(shared);
            drop(device);
            assert_eq!(breaches(&kept), Vec::<String>::new(), "by_drop {by_drop}");
        }
    }

    /// A `resize` whose making fails, at `vkCreateSwapchainKHR` or at an image after it, has
    /// retired the swapchain it had: that one is given up, the swapchain has no images and
    /// is stale, so `acquire` answers `Stale` and never reaches the retired one, and a later
    /// `resize` makes a swapchain from nothing that presents. The layer says nothing.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn a_failed_resize_is_stale_until_a_later_resize() {
        let kept = Arc::new(Kept::default());
        let Some(device) = device(&kept, "a_failed_resize_is_stale_until_a_later_resize") else {
            return;
        };
        let wider = Extent { width: 96, height: 48 };
        for fail in [Fail::Create, Fail::Image(1)] {
            let mut swapchain = VulkanSwapchain::new(device.shared(), SMALL).expect("a swapchain");
            let held = image(&mut swapchain);
            swapchain.fail = Some(fail);
            let refused = swapchain.resize(wider).expect_err("the injected failure");
            assert_eq!(refused.kind(), ErrorKind::ResourceExhausted, "{fail:?}: {refused}");
            assert_eq!(swapchain.image_count(), 0, "{fail:?}");
            assert_eq!(swapchain.extent(), Extent { width: 0, height: 0 }, "{fail:?}");
            assert!(matches!(swapchain.acquire(), Ok(Acquired::Stale)), "{fail:?}");
            let none = Submit { encoders: &mut [], waits: &[], signals: &[] };
            assert!(swapchain.present(held, none).is_err(), "{fail:?}");
            swapchain.fail = None;
            swapchain.resize(wider).expect("the later resize");
            assert_eq!(swapchain.extent(), wider);
            let image = image(&mut swapchain);
            let mut encoders = [presentable(&device, image)];
            let submit = Submit { encoders: &mut encoders, waits: &[], signals: &[] };
            assert_eq!(swapchain.present(image, submit).expect("a present"), Presented::Shown);
            drop(swapchain);
            drop(encoders);
        }
        drop(device);
        assert_eq!(breaches(&kept), Vec::<String>::new());
    }

    /// The present thread presents each frame before a frame thread's graphics submit is
    /// released: the submit waits for a gate the frame thread opens only once the present's
    /// walk has left the frame's point undecided. Each present's walk is undecided at least
    /// once, waits with no lock held, and is decided after the frame thread's submit and
    /// signal; every present is shown and the layer says nothing.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn a_present_beside_a_frame_thread_is_decided_by_its_submit() {
        const FRAMES: u64 = 16;
        let kept = Arc::new(Kept::default());
        let Some(device) =
            device(&kept, "a_present_beside_a_frame_thread_is_decided_by_its_submit")
        else {
            return;
        };
        let mut swapchain = VulkanSwapchain::new(device.shared(), SMALL).expect("a swapchain");
        let gate = device.create_timeline(0).expect("a timeline");
        let drawn = device.create_timeline(0).expect("a timeline");
        let shown = device.create_timeline(0).expect("a timeline");
        let start = UNDECIDED.load(Ordering::Relaxed);
        let (frames, next) = mpsc::channel::<usize>();
        std::thread::scope(|scope| {
            let device = &device;
            scope.spawn(move || {
                let mut done = Vec::new();
                for frame in 1..=FRAMES {
                    let Ok(before) = next.recv() else { return };
                    let encoder = device.create_encoder(QueueRole::Graphics, "frame");
                    let mut encoders = [encoder.expect("an encoder")];
                    let submit = Submit {
                        encoders: &mut encoders,
                        waits: &[at(gate, frame)],
                        signals: &[at(drawn, frame)],
                    };
                    device.submit(QueueRole::Graphics, submit).expect("the frame's submit");
                    done.extend(encoders);
                    // The gate opens once the present's walk has found the frame undecided.
                    let patience = Instant::now() + BOUND * 5;
                    while UNDECIDED.load(Ordering::Relaxed) <= before && Instant::now() < patience {
                        std::thread::yield_now();
                    }
                    device.signal_timeline(at(gate, frame)).expect("the gate");
                }
            });
            let mut presents = Vec::new();
            for frame in 1..=FRAMES {
                let image = image(&mut swapchain);
                frames.send(UNDECIDED.load(Ordering::Relaxed)).expect("the frame thread");
                let mut encoders = [presentable(device, image)];
                let submit = Submit {
                    encoders: &mut encoders,
                    waits: &[at(drawn, frame)],
                    signals: &[at(shown, frame)],
                };
                let presented = swapchain.present(image, submit).expect("a present");
                assert_eq!(presented, Presented::Shown, "frame {frame}");
                assert_eq!(swapchain.last_present(), Some(PresentId(frame)));
                presents.extend(encoders);
            }
            let reached = device.wait_timeline(at(shown, FRAMES), BOUND * 20);
            assert!(reached.expect("the wait"), "the last present's submit did not complete");
        });
        let undecided = UNDECIDED.load(Ordering::Relaxed) - start;
        eprintln!("{undecided} walks undecided over {FRAMES} presents");
        assert!(undecided >= FRAMES as usize, "{undecided} walks undecided over {FRAMES}");
        drop(swapchain);
        drop(device);
        assert_eq!(breaches(&kept), Vec::<String>::new());
    }

    /// A present the presentation engine refused with nothing enqueued, injected, is
    /// `Stale` with an id. Out of memory, host or device, gives its fence back and unsignals
    /// the image's present semaphore with an empty batch, which the `resize` after waits
    /// for. An error the spec does not list waits for its fence a bound, with the present
    /// queue idle, then destroys it, and the `resize` the image's present semaphore. A later
    /// `resize` makes a swapchain that presents, and the layer says nothing, to the device's
    /// destroy.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn a_present_refused_with_nothing_enqueued_is_stale_until_a_resize() {
        let errors = [
            vk::Result::ERROR_OUT_OF_HOST_MEMORY,
            vk::Result::ERROR_OUT_OF_DEVICE_MEMORY,
            vk::Result::ERROR_OUT_OF_POOL_MEMORY,
        ];
        for error in errors {
            let kept = Arc::new(Kept::default());
            let test = "a_present_refused_with_nothing_enqueued_is_stale_until_a_resize";
            let Some(device) = device(&kept, test) else { return };
            let mut swapchain = VulkanSwapchain::new(device.shared(), SMALL).expect("a swapchain");
            let first = image(&mut swapchain);
            let mut encoders = [presentable(&device, first)];
            swapchain.fail = Some(Fail::Present(error));
            let submit = Submit { encoders: &mut encoders, waits: &[], signals: &[] };
            let refused = swapchain.present(first, submit).expect("the present");
            assert_eq!(refused, Presented::Stale, "{error}");
            assert_eq!(swapchain.last_present(), Some(PresentId(1)), "{error}");
            swapchain.fail = None;
            assert!(matches!(swapchain.acquire(), Ok(Acquired::Stale)), "{error}");
            let unlisted = error == vk::Result::ERROR_OUT_OF_POOL_MEMORY;
            assert_eq!(swapchain.pending.len(), usize::from(unlisted), "{error}");
            assert_eq!(swapchain.chain.unsignalled > 0, !unlisted, "{error}");
            swapchain.resize(Extent { width: 96, height: 48 }).expect("the resize");
            assert!(swapchain.pending.is_empty() && swapchain.doubtful.is_empty(), "{error}");
            let next = image(&mut swapchain);
            let mut more = [presentable(&device, next)];
            let submit = Submit { encoders: &mut more, waits: &[], signals: &[] };
            assert_eq!(swapchain.present(next, submit).expect("a present"), Presented::Shown);
            assert_eq!(swapchain.last_present(), Some(PresentId(2)), "{error}");
            drop(swapchain);
            drop(encoders);
            drop(more);
            drop(device);
            assert_eq!(breaches(&kept), Vec::<String>::new(), "{error}");
        }
    }

    /// A frame submit that writes the acquired image, held by a host point, is pending at a
    /// `resize`, and its encoder is dropped while a panic unwinds, which waits for nothing:
    /// the device keeps the image's object, and so the replaced driver swapchain, until the
    /// point is signalled and the submit is known complete, and the layer says nothing.
    #[test]
    #[ignore = "needs a Vulkan driver: cargo xtask gpu"]
    fn an_encoder_dropped_unwinding_leaves_its_pending_image_to_the_device() {
        let kept = Arc::new(Kept::default());
        let test = "an_encoder_dropped_unwinding_leaves_its_pending_image_to_the_device";
        let Some(device) = device(&kept, test) else { return };
        let mut swapchain = VulkanSwapchain::new(device.shared(), SMALL).expect("a swapchain");
        let first = image(&mut swapchain);
        let replaced = Arc::downgrade(swapchain.chain.owner.as_ref().expect("a driver swapchain"));
        let gate = device.create_timeline(0).expect("a timeline");
        let done = device.create_timeline(0).expect("a timeline");
        let mut draw = device.create_encoder(G, "frame").expect("an encoder");
        draw.transition_images(&[ImageTransition {
            image: first,
            range: ImageRange::FIRST,
            from: StateOn { state: ImageState::Undefined, queue: G },
            to: StateOn { state: ImageState::ColourAttachment, queue: G },
        }]);
        let mut draws = [draw];
        let submit =
            Submit { encoders: &mut draws, waits: &[at(gate, 1)], signals: &[at(done, 1)] };
        device.submit(G, submit).expect("the frame's submit");
        swapchain.resize(Extent { width: 96, height: 48 }).expect("the resize");
        let [draw] = draws;
        let unwound = std::panic::catch_unwind(AssertUnwindSafe(move || {
            let _draw = draw;
            // No hook runs: the test's output holds no panic.
            std::panic::resume_unwind(Box::new("the encoder is dropped unwinding"));
        }));
        assert!(unwound.is_err());
        swapchain.reap_left();
        assert!(replaced.upgrade().is_some(), "the driver swapchain went with the submit pending");
        device.signal_timeline(at(gate, 1)).expect("the gate");
        assert!(device.wait_timeline(at(done, 1), BOUND * 20).expect("the wait"), "the submit");
        swapchain.reap_left();
        assert!(replaced.upgrade().is_none(), "the driver swapchain outlived the submit");
        drop(swapchain);
        drop(device);
        assert_eq!(breaches(&kept), Vec::<String>::new());
    }
}
