//! The swapchain: images no window shows, an acquire that rotates through those whose
//! presents are done, and a present that submits once its walk of the records finds its
//! submit released.

use std::sync::Arc;
use std::time::Instant;

use lactose_core::Result;
use lactose_rhi_api::{
    Acquired, DisplayTime, Extent, Format, ImageHandle, ImageState, ImageUsage, PresentId,
    Presented, QueueRole, Rule, Sharing, Submit, Swapchain,
};

use crate::device::{BOUND, ImageObj, Shared, alive};
use crate::lock;
use crate::records::{ImageRec, PresentRec, Records, Release, Sub, queue_index};
use crate::submit;

/// How many images a swapchain cycles through.
const IMAGES: usize = 3;

/// The format of every null swapchain's images.
const FORMAT: Format = Format::Bgra8Unorm;

pub(crate) struct NullSwapchain {
    shared: Arc<Shared>,
    /// Its number in the records, which its presents carry.
    number: u64,
    extent: Extent,
    images: Vec<ImageHandle>,
    /// Where the next acquire starts looking.
    next: usize,
    acquired: Option<ImageHandle>,
    /// The id of the last present taken.
    presented: Option<PresentId>,
}

impl NullSwapchain {
    /// Makes the device's swapchain, under the records' hold of `create_swapchain`.
    pub(crate) fn new(shared: &Arc<Shared>, records: &mut Records, extent: Extent) -> Result<Self> {
        if extent.width == 0 || extent.height == 0 {
            return Err(Rule::Bounds.refuse("a swapchain extent with a zero dimension"));
        }
        if records.swapchain.is_some() {
            return Err(Rule::Swapchain.refuse("a second swapchain for a device that has one"));
        }
        let number = records.next_swapchain;
        let images = make_images(shared, records, extent)?;
        records.next_swapchain += 1;
        records.swapchain = Some(number);
        Ok(Self {
            shared: shared.hold(),
            number,
            extent,
            images,
            next: 0,
            acquired: None,
            presented: None,
        })
    }

    /// The queue the present role is served by.
    fn queue(&self) -> usize {
        queue_index(self.shared.caps.queues.serving(QueueRole::Present))
    }

    /// Whether the driver may still read `image` for one of this swapchain's presents.
    fn presenting(&self, records: &Records, image: ImageHandle) -> bool {
        records.presents.iter().any(|present| {
            present.swapchain == self.number
                && present.image == image
                && !records.present_done(present)
        })
    }

    fn any_presenting(&self, records: &Records) -> bool {
        records
            .presents
            .iter()
            .any(|present| present.swapchain == self.number && !records.present_done(present))
    }
}

/// Makes a swapchain's images: colour attachments and nothing else. Where the table fills
/// part-way, the images already made are forgotten.
fn make_images(shared: &Shared, records: &mut Records, extent: Extent) -> Result<Vec<ImageHandle>> {
    let mut images = Vec::with_capacity(IMAGES);
    for _ in 0..IMAGES {
        let rec = ImageRec::new(1, 1, Sharing::Exclusive, None, true);
        let handle = match records.images.insert(rec) {
            Ok(handle) => handle,
            Err(error) => {
                forget_images(shared, records, &images);
                return Err(error);
            }
        };
        let object = ImageObj {
            format: FORMAT,
            extent,
            mips: 1,
            layers: 1,
            usage: ImageUsage::COLOUR,
            memoryless_request: false,
            memoryless: false,
            placed: false,
            swapchain: true,
        };
        shared.images.insert(handle, Arc::new(object));
        images.push(handle);
    }
    Ok(images)
}

/// Forgets a swapchain's images: their handles are stale from now on.
fn forget_images(shared: &Shared, records: &mut Records, images: &[ImageHandle]) {
    for &image in images {
        records.images.remove(image);
        shared.images.remove(image);
    }
}

impl Swapchain for NullSwapchain {
    fn format(&self) -> Format {
        FORMAT
    }

    fn extent(&self) -> Extent {
        self.extent
    }

    fn image_count(&self) -> u32 {
        IMAGES as u32
    }

    fn acquire(&mut self) -> Result<Acquired> {
        let deadline = Instant::now() + BOUND;
        let queue = self.queue();
        let mut records = self.shared.records();
        alive(&records)?;
        if self.acquired.is_some() {
            return Err(
                Rule::Swapchain.refuse("an acquire while the acquired image is not yet presented")
            );
        }
        loop {
            alive(&records)?;
            let free = (0..IMAGES)
                .map(|at| (self.next + at) % IMAGES)
                .find(|&at| !self.presenting(&records, self.images[at]));
            if let Some(at) = free {
                let image = self.images[at];
                if let Some(rec) = records.images.get_mut(image) {
                    rec.acquired = Some(true);
                    rec.subs.fill(Sub { owner: Some(queue), ..Sub::new(ImageState::Undefined) });
                }
                self.next = (at + 1) % IMAGES;
                self.acquired = Some(image);
                return Ok(Acquired::Image(image));
            }
            if Instant::now() >= deadline {
                return Ok(Acquired::None);
            }
            records = self.shared.wait(records, Some(deadline));
        }
    }

    fn present(&mut self, image: ImageHandle, mut submit: Submit<'_>) -> Result<Presented> {
        let deadline = Instant::now() + BOUND;
        let queue = self.queue();
        let shared = Arc::clone(&self.shared);
        loop {
            let held = lock(&shared.queues[queue]);
            let mut records = shared.records();
            alive(&records)?;
            if self.acquired != Some(image) {
                return Err(
                    Rule::Swapchain.refuse("a present of an image that is not the acquired one")
                );
            }
            let plan = submit::check(&shared, &records, QueueRole::Present, &submit, Some(image))?;
            let point = match records.release(queue, submit.waits)? {
                Release::Released => {
                    let seq = submit::commit(&mut records, plan, &mut submit);
                    records.presents.push(PresentRec { swapchain: self.number, image, queue, seq });
                    if let Some(rec) = records.images.get_mut(image) {
                        rec.acquired = Some(false);
                    }
                    self.acquired = None;
                    let id = self.presented.map_or(1, |PresentId(last)| last + 1);
                    self.presented = Some(PresentId(id));
                    drop(records);
                    #[cfg(test)]
                    shared.between(crate::device::Between::PresentHeld);
                    drop(held);
                    shared.progressed();
                    return Ok(Presented::Shown);
                }
                Release::Undecided(point) => point,
            };
            // The wait is made with no lock held: on a shared queue the submit that
            // signals the point may need the queue's.
            drop(records);
            drop(held);
            #[cfg(test)]
            shared.between(crate::device::Between::PresentWait);
            let mut records = shared.records();
            loop {
                let settled = records.lost
                    || records
                        .timelines
                        .get(point.timeline)
                        .is_none_or(|timeline| timeline.value >= point.value);
                if settled {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(Rule::Pending.refuse(format!(
                        "a present whose submit is not released within the bound: it waits for {} \
                         on a timeline nothing has signalled or promised by a released submit",
                        point.value
                    )));
                }
                records = shared.wait(records, Some(deadline));
            }
        }
    }

    fn resize(&mut self, extent: Extent) -> Result<()> {
        let deadline = Instant::now() + BOUND;
        let mut records = self.shared.records();
        alive(&records)?;
        if extent.width == 0 || extent.height == 0 {
            return Err(Rule::Bounds.refuse("a swapchain extent with a zero dimension"));
        }
        while self.any_presenting(&records) {
            alive(&records)?;
            if Instant::now() >= deadline {
                return Err(
                    Rule::Pending.refuse("a resize while a present is pending past the bound")
                );
            }
            records = self.shared.wait(records, Some(deadline));
        }
        // The acquired image is given up before the images are made, so that a failure
        // leaves it given up as a driver's does on Vulkan (`lactose-rhi-api`'s `resize`).
        if let Some(image) = self.acquired.take()
            && let Some(rec) = records.images.get_mut(image)
        {
            rec.acquired = Some(false);
        }
        let images = make_images(&self.shared, &mut records, extent)?;
        let old = core::mem::replace(&mut self.images, images);
        forget_images(&self.shared, &mut records, &old);
        records.presents.retain(|present| present.swapchain != self.number);
        self.extent = extent;
        self.next = 0;
        Ok(())
    }

    fn last_present(&self) -> Option<PresentId> {
        self.presented
    }

    /// Nothing shows null's images, so it has no display time to report.
    fn display_times(&mut self, _: &mut Vec<DisplayTime>) -> bool {
        false
    }
}

impl Drop for NullSwapchain {
    fn drop(&mut self) {
        let mut records = self.shared.records();
        if !std::thread::panicking() {
            // A present's submit is waited for, as an encoder's is; the driver's read of
            // its image is the device's to end, and a stall is not waited out.
            loop {
                let running = records.presents.iter().any(|present| {
                    present.swapchain == self.number
                        && records.queues[present.queue].ran < present.seq
                });
                if records.lost || !running {
                    break;
                }
                records = self.shared.wait(records, None);
            }
        }
        forget_images(&self.shared, &mut records, &self.images);
        records.presents.retain(|present| present.swapchain != self.number);
        if records.swapchain == Some(self.number) {
            records.swapchain = None;
        }
        drop(records);
        self.shared.let_go();
    }
}
