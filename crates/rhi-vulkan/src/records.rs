//! The records: what the "Every backend" column of the contract's rules table needs a device
//! to keep (`lactose-rhi-api`'s README, "What a device backend keeps"). One lock guards them
//! (`Shared::records`); no call holds it across a driver call, a wait or the caller's
//! closure. Nothing here calls the driver: the values a check needs from it are read before
//! the lock and folded in.
//!
//! A device's pending is completion. Every submit to a queue signals the queue's own
//! timeline to the submit's stamp, at every command's end, so a queue whose known value is
//! at or above a stamp has run the stamped submit and every earlier one. A caller's point
//! known reached that a submit signals completes it and every earlier submit of its queue
//! too, and is folded into the queue's known value.

use std::collections::VecDeque;
use std::num::NonZeroU32;

use lactose_core::{Error, ErrorKind, Handle, Result};
use lactose_rhi_api::{
    CacheStart, ImageHandle, MemoryBlockHandle, MemoryClass, PipelineStats, QueueRole, Rule,
    TimelineHandle, TimelinePoint,
};

use crate::table::{CAPACITY, generation};

/// The queues: graphics, compute and present, by the role that names each.
pub(crate) const QUEUES: usize = 3;

/// The index of the queue a role names, once the layout has resolved it to the role that
/// serves it.
pub(crate) const fn queue_index(serving: QueueRole) -> usize {
    match serving {
        QueueRole::Graphics => 0,
        QueueRole::Compute => 1,
        QueueRole::Present => 2,
    }
}

/// The distance at or past which two values of one timeline are refused (`timeline`).
pub(crate) const DIFFERENCE_LIMIT: u64 = 1 << 31;

/// For each queue, the stamp of the last submit that named something; zero for none.
pub(crate) type Stamps = [u64; QUEUES];

/// Records of one kind, by handle index, each with the generation its handle carries.
pub(crate) struct Slots<R> {
    items: Vec<Option<(NonZeroU32, R)>>,
    free: Vec<u32>,
}

impl<R> Slots<R> {
    pub(crate) const fn new() -> Self {
        Self { items: Vec::new(), free: Vec::new() }
    }

    pub(crate) fn get<K>(&self, handle: Handle<K>) -> Option<&R> {
        match self.items.get(handle.index() as usize)? {
            Some((generation, record)) if *generation == handle.generation() => Some(record),
            _ => None,
        }
    }

    pub(crate) fn get_mut<K>(&mut self, handle: Handle<K>) -> Option<&mut R> {
        match self.items.get_mut(handle.index() as usize)? {
            Some((generation, record)) if *generation == handle.generation() => Some(record),
            _ => None,
        }
    }

    /// The live record of `handle`, or the refusal of a stale or foreign one.
    pub(crate) fn live<K>(&self, handle: Handle<K>, what: &'static str) -> Result<&R> {
        self.get(handle).ok_or_else(|| stale(what))
    }

    pub(crate) fn live_mut<K>(&mut self, handle: Handle<K>, what: &'static str) -> Result<&mut R> {
        self.get_mut(handle).ok_or_else(|| stale(what))
    }

    /// Stores `record` under a new handle.
    ///
    /// # Errors
    ///
    /// `ErrorKind::ResourceExhausted` when the device holds as many as its table does.
    pub(crate) fn insert<K>(&mut self, record: R) -> Result<Handle<K>> {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                let index = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
                if index >= CAPACITY {
                    return Err(Error::new(
                        ErrorKind::ResourceExhausted,
                        "the device holds as many of these as it can",
                    ));
                }
                self.items.push(None);
                index
            }
        };
        let generation = generation();
        self.items[index as usize] = Some((generation, record));
        Ok(Handle::from_parts(index, generation))
    }

    pub(crate) fn remove<K>(&mut self, handle: Handle<K>) -> Option<R> {
        self.get(handle)?;
        let (_, record) = self.items[handle.index() as usize].take()?;
        self.free.push(handle.index());
        Some(record)
    }
}

/// The refusal of a handle whose resource is gone or was never this device's.
pub(crate) fn stale(what: &'static str) -> Error {
    Rule::Destroyed.refuse(format!("{what} that is destroyed or of another device"))
}

/// Byte ranges, half open.
#[derive(Clone, Default, Debug)]
pub(crate) struct Ranges(Vec<(u64, u64)>);

impl Ranges {
    pub(crate) fn add(&mut self, start: u64, end: u64) {
        if start < end {
            self.0.push((start, end));
        }
    }

    pub(crate) fn overlaps(&self, start: u64, end: u64) -> bool {
        self.0.iter().any(|&(a, b)| a < end && start < b)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Removes one range exactly as it was added.
    pub(crate) fn remove_one(&mut self, start: u64, end: u64) {
        if let Some(at) = self.0.iter().position(|&range| range == (start, end)) {
            self.0.swap_remove(at);
        }
    }
}

pub(crate) struct BufferRec {
    pub(crate) size: u64,
    pub(crate) memory: MemoryClass,
    pub(crate) stamps: Stamps,
    /// The last submit of each queue that writes it: what a host write waits for.
    pub(crate) writers: Stamps,
    /// The last submit of each queue that writes it or changes its state: what a host
    /// read and an invalidate wait for.
    pub(crate) changed: Stamps,
    /// Host accesses in progress.
    pub(crate) host: Ranges,
}

impl BufferRec {
    pub(crate) fn new(size: u64, memory: MemoryClass) -> Self {
        Self {
            size,
            memory,
            stamps: [0; QUEUES],
            writers: [0; QUEUES],
            changed: [0; QUEUES],
            host: Ranges::default(),
        }
    }
}

pub(crate) struct ImageRec {
    pub(crate) stamps: Stamps,
    /// Heap slots that hold it, the one a write in progress gives it included.
    pub(crate) slots: u32,
    /// The block it is placed in.
    pub(crate) placed: Option<MemoryBlockHandle>,
    /// One of the swapchain's images, which the swapchain alone destroys.
    pub(crate) swapchain: bool,
}

pub(crate) struct BlockRec {
    pub(crate) size: u64,
    /// The images placed in it.
    pub(crate) images: u32,
}

#[derive(Default)]
pub(crate) struct PipelineRec {
    pub(crate) stamps: Stamps,
}

pub(crate) struct TimelineRec {
    /// The greatest value the device knows the timeline has reached.
    pub(crate) known: u64,
    /// The value of a host signal in progress.
    pub(crate) mark: Option<u64>,
}

#[derive(Default)]
pub(crate) struct SlotRec {
    pub(crate) image: Option<ImageHandle>,
    /// A write of the slot is in progress.
    pub(crate) mark: bool,
    /// The last submit of each queue with a pass that declares the slot.
    pub(crate) stamps: Stamps,
}

/// A submit the records keep while it has not completed or a promise of it is pending.
#[derive(Clone, Debug)]
pub(crate) struct SubmitRec {
    pub(crate) seq: u64,
    pub(crate) waits: Vec<TimelinePoint>,
    pub(crate) signals: Vec<TimelinePoint>,
    /// Its driver call failed: its signals stay promises that nothing orders, refusing
    /// every submit's signal of their timelines and every host signal of them but one below
    /// them that meets one of its waits, and its waits order nothing, until its stamp and
    /// its waits are known reached. A submit already credited through either keeps the
    /// order it was credited with: what it waits for can be signalled only after.
    pub(crate) failed: bool,
}

impl SubmitRec {
    /// The waits a later submit is ordered after through this one: none for a failed
    /// submit, whose batch never reached the driver.
    fn ordering(&self) -> &[TimelinePoint] {
        if self.failed { &[] } else { &self.waits }
    }
}

pub(crate) struct QueueRec {
    /// The stamp the next submit takes, from one.
    pub(crate) next: u64,
    /// The greatest value the device knows the queue's own timeline has reached: every
    /// submit stamped at or below it has completed.
    pub(crate) known: u64,
    /// The stamp of the last submit whose driver call has returned.
    pub(crate) received: u64,
    /// The submits kept, in order.
    pub(crate) submits: VecDeque<SubmitRec>,
}

impl QueueRec {
    const fn new() -> Self {
        Self { next: 1, known: 0, received: 0, submits: VecDeque::new() }
    }
}

pub(crate) struct Records {
    pub(crate) lost: bool,
    pub(crate) buffers: Slots<BufferRec>,
    pub(crate) images: Slots<ImageRec>,
    pub(crate) blocks: Slots<BlockRec>,
    pub(crate) graphics: Slots<PipelineRec>,
    pub(crate) compute: Slots<PipelineRec>,
    pub(crate) timelines: Slots<TimelineRec>,
    pub(crate) slots: Vec<SlotRec>,
    /// Which sampler entries are live.
    pub(crate) samplers: Vec<bool>,
    pub(crate) queues: [QueueRec; QUEUES],
    pub(crate) stats: PipelineStats,
    /// The device's swapchain lives.
    pub(crate) swapchain: bool,
}

/// What a present's walk of the records decided of its submit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Release {
    /// Every point it depends on is known reached, or promised by a released submit.
    Released,
    /// The walk could not decide this point: no released promise meets it, or the walk
    /// came back to a submit it was still deciding, and this is the point it came back
    /// from. The present waits for it with no lock held and walks again.
    Undecided(TimelinePoint),
}

/// What a release walk has decided: for each queue, every submit through `decided` is
/// released, and the submits from `deciding` on are still being decided.
struct Walk {
    decided: Stamps,
    deciding: [Option<u64>; QUEUES],
}

impl Records {
    pub(crate) fn new(heap_slots: u32, samplers: u32, cache: CacheStart) -> Self {
        Self {
            lost: false,
            buffers: Slots::new(),
            images: Slots::new(),
            blocks: Slots::new(),
            graphics: Slots::new(),
            compute: Slots::new(),
            timelines: Slots::new(),
            slots: (0..heap_slots).map(|_| SlotRec::default()).collect(),
            samplers: vec![false; samplers as usize],
            queues: [QueueRec::new(), QueueRec::new(), QueueRec::new()],
            stats: PipelineStats {
                cache,
                pipelines: 0,
                stage_hits: 0,
                stage_misses: 0,
                creation: core::time::Duration::ZERO,
            },
            swapchain: false,
        }
    }

    /// Folds what a call read from the driver into the known values: each queue's own
    /// timeline, and the caller's timelines it read; then every submit a caller's point it
    /// signals completes, and the submits nothing keeps leave.
    pub(crate) fn fold(
        &mut self,
        queues: &[Option<u64>; QUEUES],
        timelines: &[(TimelineHandle, u64)],
    ) {
        for (rec, value) in self.queues.iter_mut().zip(queues) {
            if let Some(value) = value {
                rec.known = rec.known.max(*value);
            }
        }
        for &(timeline, value) in timelines {
            self.fold_timeline(timeline, value);
        }
        self.settle();
    }

    pub(crate) fn fold_timeline(&mut self, timeline: TimelineHandle, value: u64) {
        if let Some(rec) = self.timelines.get_mut(timeline) {
            rec.known = rec.known.max(value);
        }
    }

    /// Completes every submit that signals a point known reached, with the earlier ones of
    /// its queue, and drops the submits whose completion and promises are all known: a
    /// failed submit once its stamp and its waits are, its promises not folded in.
    pub(crate) fn settle(&mut self) {
        let timelines = &self.timelines;
        let reached = |point: &TimelinePoint| {
            timelines.get(point.timeline).is_none_or(|rec| rec.known >= point.value)
        };
        for rec in &mut self.queues {
            let done = rec
                .submits
                .iter()
                // A failed submit's signal counts as reached once its timeline is gone, which
                // `destroy_timeline` refuses while it is kept; it must complete nothing then.
                .filter(|submit| {
                    !submit.failed && submit.seq > rec.known && submit.signals.iter().any(reached)
                })
                .map(|submit| submit.seq)
                .max();
            if let Some(done) = done {
                rec.known = done;
            }
            let known = rec.known;
            rec.submits.retain(|submit| {
                let points = if submit.failed { &submit.waits } else { &submit.signals };
                submit.seq > known || !points.iter().all(reached)
            });
        }
    }

    /// Whether a submit stamped in `stamps` is pending: one that has not completed, as far
    /// as the device knows.
    pub(crate) fn pending(&self, stamps: &Stamps) -> bool {
        (0..QUEUES).any(|queue| stamps[queue] > self.queues[queue].known)
    }

    pub(crate) fn timeline(&self, timeline: TimelineHandle) -> Result<&TimelineRec> {
        self.timelines.live(timeline, "a timeline")
    }

    /// The values above a timeline's known value that a kept submit signals or a host signal
    /// in progress gives it, ascending and once each: the values a read may learn it has
    /// reached. None for a handle that does not resolve.
    pub(crate) fn awaited(&self, timeline: TimelineHandle) -> Vec<u64> {
        let Some(rec) = self.timelines.get(timeline) else {
            return Vec::new();
        };
        let signals = self.queues.iter().flat_map(|queue| &queue.submits).flat_map(|submit| {
            submit.signals.iter().filter(|p| p.timeline == timeline).map(|p| p.value)
        });
        let mut values: Vec<u64> =
            signals.chain(rec.mark).filter(|&value| value > rec.known).collect();
        values.sort_unstable();
        values.dedup();
        values
    }

    /// Whether a promise of `point`'s timeline is pending: its value not yet known reached.
    fn promise_pending(&self, promise: &TimelinePoint) -> bool {
        self.timelines.get(promise.timeline).is_some_and(|rec| rec.known < promise.value)
    }

    /// The submits a new submit to `queue` that waits for `waits` is ordered after, as a
    /// mark for each queue: every earlier submit of its own queue; for each point it waits
    /// for, each submit that signals that timeline at or below the point and every earlier
    /// submit of that one's queue; and in turn what each of those waits for, a failed one's
    /// waits aside. Completed submits are below every mark.
    pub(crate) fn cover(&self, queue: usize, waits: &[TimelinePoint]) -> Stamps {
        let mut cover = self.queues.each_ref().map(|rec| rec.known);
        let mut work = waits.to_vec();
        self.raise(&mut cover, queue, self.queues[queue].next - 1, &mut work);
        while let Some(point) = work.pop() {
            for at in 0..QUEUES {
                let top = self.queues[at]
                    .submits
                    .iter()
                    .filter(|submit| submit.seq > cover[at])
                    .filter(|submit| {
                        submit
                            .signals
                            .iter()
                            .any(|s| s.timeline == point.timeline && s.value <= point.value)
                    })
                    .map(|submit| submit.seq)
                    .max();
                if let Some(top) = top {
                    self.raise(&mut cover, at, top, &mut work);
                }
            }
        }
        cover
    }

    /// Raises `cover[queue]` to `top`, adding the waits of the submits it takes in.
    fn raise(&self, cover: &mut Stamps, queue: usize, top: u64, work: &mut Vec<TimelinePoint>) {
        if top <= cover[queue] {
            return;
        }
        let taken = self.queues[queue]
            .submits
            .iter()
            .filter(|submit| submit.seq > cover[queue] && submit.seq <= top);
        for submit in taken {
            work.extend_from_slice(submit.ordering());
        }
        cover[queue] = top;
    }

    /// The timeline checks of a submit to `queue` (`lactose-rhi-api`'s README,
    /// "Timelines"): each wait's and signal's timeline lives; no value is 2^31 or more from
    /// those it is compared with; each signal grows past the timeline's known value, its
    /// host signal in progress, every pending promise and the submit's own waits and
    /// earlier signals on it; every pending promise of it is ordered before the signal, and
    /// none is a failed submit's; and while a host signal of it is in progress, the submit
    /// waits for that signal's value.
    pub(crate) fn check_points(
        &self,
        queue: usize,
        waits: &[TimelinePoint],
        signals: &[TimelinePoint],
    ) -> Result<()> {
        for &point in waits.iter().chain(signals) {
            self.within_limit(point)?;
        }
        let cover = if signals.is_empty() { [0; QUEUES] } else { self.cover(queue, waits) };
        let mut through = waits.to_vec();
        for (at, rec) in self.queues.iter().enumerate() {
            let taken = rec.submits.iter().filter(|submit| submit.seq <= cover[at]);
            through.extend(taken.flat_map(|submit| submit.ordering().iter().copied()));
        }
        for (at, &signal) in signals.iter().enumerate() {
            let timeline = self.timeline(signal.timeline)?;
            let refuse =
                |what: &str| Rule::Timeline.refuse(format!("a signal of {} {what}", signal.value));
            if signal.value <= timeline.known {
                return Err(refuse("not above the timeline's value"));
            }
            let own_waits = || waits.iter().filter(|wait| wait.timeline == signal.timeline);
            if own_waits().any(|wait| wait.value >= signal.value) {
                return Err(refuse("not above a value the same submit waits for"));
            }
            let mut earlier =
                signals[..at].iter().filter(|other| other.timeline == signal.timeline);
            if earlier.any(|other| other.value >= signal.value) {
                return Err(refuse("not above the submit's own earlier signal"));
            }
            if let Some(mark) = timeline.mark {
                if signal.value <= mark {
                    return Err(refuse("not above a host signal in progress"));
                }
                if !own_waits().any(|wait| wait.value > timeline.known && wait.value <= mark) {
                    return Err(refuse("while a host signal it does not wait for is in progress"));
                }
            }
            for (other, rec) in self.queues.iter().enumerate() {
                for submit in &rec.submits {
                    let promises = submit
                        .signals
                        .iter()
                        .filter(|p| p.timeline == signal.timeline && self.promise_pending(p));
                    for promise in promises {
                        if promise.value >= signal.value {
                            return Err(refuse("not above a pending promise"));
                        }
                        if submit.failed {
                            return Err(refuse("while a failed submit's signal of it is pending"));
                        }
                        // Ordered: an earlier batch of the submit's own queue; an earlier
                        // batch of a queue than a submit whose signal it waits for or the
                        // host has seen reached; or that submit, where it waits for this
                        // timeline.
                        let ordered = other == queue
                            || submit.seq < cover[other]
                            || submit.seq == cover[other] && waited_through(promise, &through);
                        if !ordered {
                            return Err(refuse(
                                "while another queue's signal of it is pending, with no wait that \
                                 orders it",
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Refuses a value 2^31 or more from the timeline's known value, from its host signal
    /// in progress, or from the least or the greatest pending wait or promise on it.
    pub(crate) fn within_limit(&self, point: TimelinePoint) -> Result<()> {
        let timeline = self.timeline(point.timeline)?;
        let mut listed: Vec<&TimelinePoint> = Vec::new();
        for rec in &self.queues {
            for submit in &rec.submits {
                if submit.seq > rec.known {
                    listed.extend(submit.waits.iter().filter(|p| p.timeline == point.timeline));
                }
                listed.extend(
                    submit
                        .signals
                        .iter()
                        .filter(|p| p.timeline == point.timeline && self.promise_pending(p)),
                );
            }
        }
        let least = listed.iter().map(|p| p.value).min();
        let greatest = listed.iter().map(|p| p.value).max();
        let compared = [Some(timeline.known), timeline.mark, least, greatest];
        if compared.into_iter().flatten().any(|v| v.abs_diff(point.value) >= DIFFERENCE_LIMIT) {
            return Err(Rule::Timeline.refuse(format!(
                "{} is 2^31 or more from the timeline's value, a host signal's in progress, or a \
                 pending wait or signal",
                point.value
            )));
        }
        Ok(())
    }

    /// The first hold of a host signal: its checks, and its mark.
    pub(crate) fn begin_host_signal(&mut self, point: TimelinePoint) -> Result<()> {
        let timeline = self.timeline(point.timeline)?;
        let refuse =
            |what: &str| Rule::Timeline.refuse(format!("a host signal of {} {what}", point.value));
        if point.value <= timeline.known {
            return Err(refuse("not above the timeline's value"));
        }
        if timeline.mark.is_some() {
            return Err(refuse("while another host signal of the timeline is in progress"));
        }
        self.within_limit(point)?;
        let known = timeline.known;
        for rec in &self.queues {
            for submit in &rec.submits {
                let promises = submit
                    .signals
                    .iter()
                    .filter(|p| p.timeline == point.timeline && self.promise_pending(p));
                for promise in promises {
                    if promise.value <= point.value {
                        return Err(refuse("not below a pending promise"));
                    }
                    let released_by_this = submit.waits.iter().any(|wait| {
                        wait.timeline == point.timeline
                            && wait.value > known
                            && wait.value <= point.value
                    });
                    if !released_by_this {
                        return Err(refuse(
                            "below a promise whose submit it does not alone release",
                        ));
                    }
                }
            }
        }
        self.timelines.live_mut(point.timeline, "a timeline")?.mark = Some(point.value);
        Ok(())
    }

    /// The second hold of a host signal: the mark cleared and, where its driver call
    /// succeeded, the value folded in.
    pub(crate) fn end_host_signal(&mut self, point: TimelinePoint, signalled: bool) {
        if let Some(timeline) = self.timelines.get_mut(point.timeline) {
            timeline.mark = None;
            if signalled {
                timeline.known = timeline.known.max(point.value);
            }
        }
        self.settle();
    }

    /// Takes the next stamp of `queue` for a submit that passed its checks, and keeps it
    /// with its points.
    pub(crate) fn stamp(
        &mut self,
        queue: usize,
        waits: &[TimelinePoint],
        signals: &[TimelinePoint],
    ) -> u64 {
        let rec = &mut self.queues[queue];
        let seq = rec.next;
        rec.next += 1;
        let (waits, signals) = (waits.to_vec(), signals.to_vec());
        rec.submits.push_back(SubmitRec { seq, waits, signals, failed: false });
        seq
    }

    /// The hold after a submit's driver call: its stamp received; a failed submit marked,
    /// so that its points are retried only once its stamp and its waits are known reached.
    pub(crate) fn received(&mut self, queue: usize, seq: u64, failed: bool) {
        let rec = &mut self.queues[queue];
        rec.received = rec.received.max(seq);
        if failed && let Some(submit) = rec.submits.iter_mut().find(|submit| submit.seq == seq) {
            submit.failed = true;
        }
        self.settle();
    }

    /// The first hold of a heap slot's write: refused while a pass of a pending submit
    /// declares the slot; then the slot is marked, and the image it is given counted from
    /// now.
    pub(crate) fn begin_slot_write(
        &mut self,
        index: usize,
        image: Option<ImageHandle>,
    ) -> Result<()> {
        if self.pending(&self.slots[index].stamps) {
            return Err(Rule::HeapWrite.refuse(format!(
                "slot {index} written while a pass of a pending submit declares it"
            )));
        }
        self.slots[index].mark = true;
        if let Some(rec) = image.and_then(|image| self.images.get_mut(image)) {
            rec.slots += 1;
        }
        Ok(())
    }

    /// The second hold of a heap slot's write: the mark cleared, the slot's image
    /// recorded, and the image it held no longer counted.
    pub(crate) fn end_slot_write(&mut self, index: usize, image: Option<ImageHandle>) {
        let rec = &mut self.slots[index];
        rec.mark = false;
        let old = core::mem::replace(&mut rec.image, image);
        if let Some(rec) = old.and_then(|old| self.images.get_mut(old)) {
            rec.slots -= 1;
        }
    }

    /// A submit's check of a slot its pass declared, holding `image` when the pass began:
    /// refused while a write of the slot is in progress, and once a write has given the
    /// slot another image.
    pub(crate) fn check_slot(&self, index: usize, image: Option<ImageHandle>) -> Result<()> {
        let rec = &self.slots[index];
        if rec.mark {
            return Err(Rule::HeapWrite
                .refuse(format!("a pass that declares slot {index}, whose write is in progress")));
        }
        if rec.image != image {
            return Err(Rule::HeapLayout.refuse(format!(
                "a pass that declares slot {index}, written since the pass began"
            )));
        }
        Ok(())
    }

    /// Whether any submit has not completed.
    pub(crate) fn any_pending(&self) -> bool {
        self.queues.iter().any(|queue| queue.next - 1 > queue.known)
    }

    /// The stamp each queue's next submit takes.
    pub(crate) fn next_stamps(&self) -> Stamps {
        self.queues.each_ref().map(|rec| rec.next)
    }

    /// Decides whether a submit to `queue` made now, waiting for `waits`, is released: each
    /// of its waits known reached or promised by a released submit, and every earlier
    /// submit of its queue released (`lactose-rhi-api`'s README, "The swapchain"). A
    /// submit is released once its driver call has returned and did not fail, and its
    /// waits and the submits before it on its queue are; the caller holds `queue`'s lock,
    /// so every submit stamped on it has been received.
    pub(crate) fn release(&self, queue: usize, waits: &[TimelinePoint]) -> Result<Release> {
        let mut walk =
            Walk { decided: self.queues.each_ref().map(|rec| rec.known), deciding: [None; QUEUES] };
        for &wait in waits {
            if let Some(point) = self.point(wait, &mut walk)? {
                return Ok(Release::Undecided(point));
            }
        }
        let last = self.queues[queue].next - 1;
        if let Some(point) = self.submits(queue, last, None, &mut walk)? {
            return Ok(Release::Undecided(point));
        }
        Ok(Release::Released)
    }

    /// Decides one point: `None` when it is known reached or its least promise at or above
    /// it is released, or else the point the walk could not decide.
    fn point(&self, point: TimelinePoint, walk: &mut Walk) -> Result<Option<TimelinePoint>> {
        let timeline = self.timeline(point.timeline)?;
        if timeline.known >= point.value {
            return Ok(None);
        }
        // A greater promise never decides a point: every greater one depends on the least.
        let mut least: Option<(u64, usize, &SubmitRec)> = None;
        for (queue, rec) in self.queues.iter().enumerate() {
            for submit in rec.submits.iter().filter(|submit| submit.seq > rec.known) {
                for promise in &submit.signals {
                    let meets = promise.timeline == point.timeline && promise.value >= point.value;
                    if meets && least.is_none_or(|(value, ..)| promise.value < value) {
                        least = Some((promise.value, queue, submit));
                    }
                }
            }
        }
        let Some((_, queue, submit)) = least else { return Ok(Some(point)) };
        // A failed submit's promise never reached the driver, and one whose call has not
        // returned may yet fail.
        if submit.failed || submit.seq > self.queues[queue].received {
            return Ok(Some(point));
        }
        self.submits(queue, submit.seq, Some(point), walk)
    }

    /// Decides the submits of `queue` through `through`, in order: each is released once
    /// its driver call has returned, its waits are and the one before it is. `via` is the
    /// point whose promise led here; the first call of a walk, from the present's own
    /// queue, has none and finds nothing in progress.
    fn submits(
        &self,
        queue: usize,
        through: u64,
        via: Option<TimelinePoint>,
        walk: &mut Walk,
    ) -> Result<Option<TimelinePoint>> {
        if through <= walk.decided[queue] {
            return Ok(None);
        }
        // A return to a submit still being decided: its own promise, or one of a submit
        // that depends on it, never meets its wait.
        if walk.deciding[queue].is_some_and(|from| through >= from) {
            return Ok(via);
        }
        if through > self.queues[queue].received {
            return Ok(via);
        }
        for seq in walk.decided[queue] + 1..=through {
            // The submits from `seq` on are in progress; those before it are decided.
            walk.deciding[queue] = Some(seq);
            let kept = self.queues[queue].submits.iter().find(|submit| submit.seq == seq);
            // A failed submit's stamp is reached by an empty batch that waits for nothing.
            for &wait in kept.map_or(&[][..], SubmitRec::ordering) {
                if let Some(point) = self.point(wait, walk)? {
                    return Ok(Some(point));
                }
            }
            walk.decided[queue] = seq;
        }
        walk.deciding[queue] = None;
        Ok(None)
    }
}

/// Whether a point of `through` is on `promise`'s timeline at or above it: a wait, of a new
/// submit or of one it is ordered after, that orders the promise before the new submit's
/// signal of that timeline although the promise's submit is the latest of its queue the new
/// one is ordered after. A wait orders the one signal it waits for and every signal of an
/// earlier batch of that signal's queue, and not the other signals of its batch:
/// "Semaphore signal operations defined by a single VkSubmitInfo or VkSubmitInfo2 or
/// VkBindSparseInfo structure are unordered with respect to other semaphore signal
/// operations defined within the same structure" (Vulkan, "Implicit Synchronization
/// Guarantees"). `lactose-rhi-null`'s records hold the same rule, and the suite's timeline
/// cases hold both to it.
fn waited_through(promise: &TimelinePoint, through: &[TimelinePoint]) -> bool {
    through.iter().any(|point| point.timeline == promise.timeline && promise.value <= point.value)
}

#[cfg(test)]
mod tests {
    use lactose_rhi_api::{CacheStart, ImageHandle, Rule, TimelineHandle, TimelinePoint};

    use super::{DIFFERENCE_LIMIT, ImageRec, QUEUES, Records, Release, TimelineRec};

    fn records() -> Records {
        Records::new(4, 4, CacheStart::Empty)
    }

    fn rule(error: &lactose_core::Error) -> Option<Rule> {
        Rule::of(error)
    }

    #[test]
    fn a_reached_point_completes_its_submit_and_the_earlier_ones_of_its_queue() {
        let mut records = records();
        let timeline = records.timelines.insert(TimelineRec { known: 0, mark: None }).unwrap();
        let first = records.stamp(0, &[], &[]);
        let second = records.stamp(0, &[], &[TimelinePoint { timeline, value: 1 }]);
        let third = records.stamp(0, &[], &[]);
        assert!(records.pending(&[first, 0, 0]));
        records.fold(&[None, None, None], &[(timeline, 1)]);
        assert!(!records.pending(&[first, 0, 0]));
        assert!(!records.pending(&[second, 0, 0]));
        assert!(records.pending(&[third, 0, 0]));
        records.fold(&[Some(third), None, None], &[]);
        assert!(!records.any_pending());
        assert!(records.queues[0].submits.is_empty());
    }

    /// A failed submit's promise refuses every submit's signal of its timeline, a later one
    /// of its own queue's among them, and a host signal of it, until its stamp and its wait are known reached; then it
    /// leaves, not folded in, and its point is retried.
    #[test]
    fn a_failed_submits_promise_is_kept_until_its_stamp_and_waits_are_reached() {
        let mut records = records();
        let mut timeline = || records.timelines.insert(TimelineRec { known: 0, mark: None });
        let (t, gate) = (timeline().unwrap(), timeline().unwrap());
        let at = |timeline, value| TimelinePoint { timeline, value };
        records.check_points(0, &[at(gate, 1)], &[at(t, 1)]).unwrap();
        let seq = records.stamp(0, &[at(gate, 1)], &[at(t, 1)]);
        let again = records.check_points(0, &[], &[at(t, 1)]).unwrap_err();
        assert_eq!(rule(&again), Some(Rule::Timeline));
        records.received(0, seq, true);
        for value in [1, 2] {
            let refused = records.check_points(0, &[], &[at(t, value)]).unwrap_err();
            assert_eq!(rule(&refused), Some(Rule::Timeline), "{value}: {refused:?}");
        }
        let host = records.begin_host_signal(at(t, 1)).unwrap_err();
        assert_eq!(rule(&host), Some(Rule::Timeline), "{host:?}");
        // The batch with no work reached the stamp; the failed submit's wait is not.
        records.fold(&[Some(seq), None, None], &[]);
        let refused = records.check_points(0, &[], &[at(t, 1)]).unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline), "{refused:?}");
        records.fold(&[None; QUEUES], &[(gate, 1)]);
        assert!(records.queues[0].submits.is_empty());
        assert_eq!(records.timeline(t).unwrap().known, 0, "the promise not folded in");
        records.check_points(0, &[], &[at(t, 1)]).expect("the point retried");
    }

    #[test]
    fn another_queues_pending_signal_needs_a_wait_for_it() {
        let mut records = records();
        let timeline = records.timelines.insert(TimelineRec { known: 0, mark: None }).unwrap();
        let one = TimelinePoint { timeline, value: 1 };
        let two = TimelinePoint { timeline, value: 2 };
        records.stamp(1, &[], &[one]);
        let refused = records.check_points(0, &[], &[two]).unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline));
        records.check_points(0, &[one], &[two]).unwrap();
    }

    /// The shapes `lactose-rhi-null`'s `a_wait_orders_only_the_signal_it_waits_for` holds
    /// null to, on queue 1 for compute and 0 for graphics, a wait below the promise among
    /// them; and one of a device alone, a submit the host has seen complete by one of its
    /// signals and not the other.
    #[test]
    fn a_wait_orders_only_the_signal_it_waits_for() {
        let two_timelines = || {
            let mut records = records();
            let mut timeline = || records.timelines.insert(TimelineRec { known: 0, mark: None });
            let (c, f) = (timeline().unwrap(), timeline().unwrap());
            (records, c, f)
        };
        let at = |timeline, value| TimelinePoint { timeline, value };

        let (mut records, c, f) = two_timelines();
        records.stamp(1, &[], &[at(c, 1), at(f, 1)]);
        let refused = records.check_points(0, &[at(c, 1)], &[at(f, 2)]).unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline), "a wait for another signal of the batch");
        records.check_points(0, &[at(f, 1)], &[at(f, 2)]).expect("a wait for the signal itself");
        records.stamp(0, &[at(f, 1)], &[]);
        records.check_points(0, &[], &[at(f, 2)]).expect("an earlier submit's wait for it");

        let (mut records, c, f) = two_timelines();
        records.stamp(1, &[], &[at(f, 1)]);
        records.stamp(1, &[], &[at(c, 1)]);
        records.check_points(0, &[at(c, 1)], &[at(f, 2)]).expect("an earlier batch's signal");

        for host in [true, false] {
            let (mut records, c, f) = two_timelines();
            if host {
                records.begin_host_signal(at(f, 1)).expect("the host's f = 1");
                records.end_host_signal(at(f, 1), true);
            } else {
                records.stamp(1, &[], &[at(f, 1)]);
            }
            records.stamp(1, &[], &[at(c, 1), at(f, 2)]);
            let refused = records.check_points(0, &[at(c, 1), at(f, 1)], &[at(f, 3)]).unwrap_err();
            assert_eq!(rule(&refused), Some(Rule::Timeline), "a wait below the promise, {host}");
        }

        let (mut records, c, f) = two_timelines();
        let seq = records.stamp(1, &[], &[at(c, 1), at(f, 1)]);
        records.fold(&[None, Some(seq), None], &[(c, 1)]);
        let refused = records.check_points(0, &[], &[at(f, 2)]).unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline), "a batch seen complete by `c` alone");
    }

    #[test]
    fn host_signal_below_a_promise_is_accepted_only_where_it_releases_the_promise() {
        let mut records = records();
        let timeline = records.timelines.insert(TimelineRec { known: 0, mark: None }).unwrap();
        let one = TimelinePoint { timeline, value: 1 };
        let three = TimelinePoint { timeline, value: 3 };
        records.stamp(0, &[one], &[three]);
        records.begin_host_signal(one).unwrap();
        let second = records.begin_host_signal(TimelinePoint { timeline, value: 2 }).unwrap_err();
        assert_eq!(rule(&second), Some(Rule::Timeline));
        records.end_host_signal(one, true);
        assert_eq!(records.timeline(timeline).unwrap().known, 1);
        let above = records.begin_host_signal(TimelinePoint { timeline, value: 4 }).unwrap_err();
        assert_eq!(rule(&above), Some(Rule::Timeline));
    }

    // The fixed interleavings `lactose-rhi-api`'s README asks of a device backend's records
    // ("Threads"), each a device call's read, holds and driver call in the order the
    // device makes them, against a model of the driver's values.

    /// The driver's values, as a device's calls would read and change them.
    struct Driver {
        queues: [u64; QUEUES],
        timelines: Vec<(TimelineHandle, u64)>,
    }

    impl Driver {
        fn new() -> Self {
            Self { queues: [0; QUEUES], timelines: Vec::new() }
        }

        fn value(&self, timeline: TimelineHandle) -> u64 {
            self.timelines.iter().find(|(t, _)| *t == timeline).map_or(0, |&(_, v)| v)
        }

        /// A signal, by the host or a submit; a value not above the semaphore's is a breach
        /// of the driver's rules the records exist to prevent.
        fn signal(&mut self, point: TimelinePoint) {
            let now = self.value(point.timeline);
            assert!(point.value > now, "a signal of {} to a semaphore at {now}", point.value);
            self.timelines.retain(|(t, _)| *t != point.timeline);
            self.timelines.push((point.timeline, point.value));
        }

        /// What `Shared::read_timeline` learns, with no lock held: the greatest value the
        /// timeline is awaited to reach that the driver's has reached.
        fn read_timeline(
            &self,
            records: &Records,
            timeline: TimelineHandle,
        ) -> Option<(TimelineHandle, u64)> {
            let value = self.value(timeline);
            let reached = records.awaited(timeline).into_iter().filter(|&v| v <= value).max();
            reached.map(|reached| (timeline, reached))
        }

        /// What `Shared::read_queues` learns: each queue's greatest received stamp the
        /// driver has reached.
        fn read_queues(&self, records: &Records) -> [Option<u64>; QUEUES] {
            core::array::from_fn(|queue| {
                let rec = &records.queues[queue];
                (rec.known + 1..=rec.received).filter(|&seq| seq <= self.queues[queue]).max()
            })
        }

        /// A submit's completion on the device: its queue's own value and its signals.
        fn complete(&mut self, queue: usize, seq: u64, signals: &[TimelinePoint]) {
            self.queues[queue] = seq;
            for &point in signals {
                self.signal(point);
            }
        }
    }

    /// The check after every hold: no value the records know a caller's timeline has
    /// reached is above the driver's. A queue's own value may be known ahead of its
    /// semaphore, from a caller's point its submit signals.
    fn behind(records: &Records, driver: &Driver) {
        for &(timeline, value) in &driver.timelines {
            if let Some(rec) = records.timelines.get(timeline) {
                assert!(rec.known <= value, "known {} above the driver's {value}", rec.known);
            }
        }
    }

    /// A host signal's first hold, after a read made at `read`.
    fn begin(
        records: &mut Records,
        driver: &Driver,
        read: Option<(TimelineHandle, u64)>,
        point: TimelinePoint,
    ) -> lactose_core::Result<()> {
        records.fold(&[None; QUEUES], read.as_slice());
        let begun = records.begin_host_signal(point);
        behind(records, driver);
        begun
    }

    /// A host signal's driver call and second hold.
    fn end(records: &mut Records, driver: &mut Driver, point: TimelinePoint) {
        driver.signal(point);
        records.end_host_signal(point, true);
        behind(records, driver);
    }

    /// A host signal made in full.
    fn host_signal(records: &mut Records, driver: &mut Driver, point: TimelinePoint) {
        let read = driver.read_timeline(records, point.timeline);
        begin(records, driver, read, point).expect("a host signal");
        end(records, driver, point);
    }

    /// A submit's read of the driver, before its first hold.
    struct Read {
        queues: [Option<u64>; QUEUES],
        points: Vec<(TimelineHandle, u64)>,
    }

    fn read(records: &Records, driver: &Driver, points: &[TimelinePoint]) -> Read {
        Read {
            queues: driver.read_queues(records),
            points: points
                .iter()
                .filter_map(|p| driver.read_timeline(records, p.timeline))
                .collect(),
        }
    }

    /// A submit's first hold, the driver call and the second hold, after `read`: its stamp,
    /// or its refusal.
    fn submit(
        records: &mut Records,
        driver: &Driver,
        read: &Read,
        queue: usize,
        waits: &[TimelinePoint],
        signals: &[TimelinePoint],
        failed: bool,
    ) -> lactose_core::Result<u64> {
        records.fold(&read.queues, &read.points);
        let checked = records.check_points(queue, waits, signals);
        behind(records, driver);
        checked?;
        let seq = records.stamp(queue, waits, signals);
        records.received(queue, seq, failed);
        behind(records, driver);
        Ok(seq)
    }

    fn timeline(records: &mut Records, driver: &mut Driver, known: u64) -> TimelineHandle {
        let timeline =
            records.timelines.insert(TimelineRec { known, mark: None }).expect("a timeline");
        driver.timelines.push((timeline, known));
        timeline
    }

    fn at(timeline: TimelineHandle, value: u64) -> TimelinePoint {
        TimelinePoint { timeline, value }
    }

    /// A host signal made in full between another host signal's driver read and its hold,
    /// and between a submit's: each hold folds a read older than what it then knows.
    #[test]
    fn a_host_signal_made_between_a_read_and_its_hold_is_seen_by_the_hold() {
        let (mut records, mut driver) = (records(), Driver::new());
        let t = timeline(&mut records, &mut driver, 0);
        let stale = driver.read_timeline(&records, t);
        host_signal(&mut records, &mut driver, at(t, 2));
        let below = begin(&mut records, &driver, stale, at(t, 1)).unwrap_err();
        assert_eq!(rule(&below), Some(Rule::Timeline), "{below:?}");
        begin(&mut records, &driver, stale, at(t, 3)).expect("a signal above the landed one");
        end(&mut records, &mut driver, at(t, 3));

        let stale = read(&records, &driver, &[at(t, 4)]);
        host_signal(&mut records, &mut driver, at(t, 4));
        let below = submit(&mut records, &driver, &stale, 0, &[], &[at(t, 4)], false).unwrap_err();
        assert_eq!(rule(&below), Some(Rule::Timeline), "{below:?}");
        submit(&mut records, &driver, &stale, 0, &[], &[at(t, 5)], false).expect("a signal above");
        assert_eq!(records.timeline(t).unwrap().known, 4);
    }

    /// A promise reached, and its submit dropped by a third call's fold, in the same two
    /// windows: the late hold's read names a value the records have folded in already.
    #[test]
    fn a_promise_dropped_between_a_read_and_its_hold_is_seen_by_the_hold() {
        let (mut records, mut driver) = (records(), Driver::new());
        let t = timeline(&mut records, &mut driver, 0);
        let none = read(&records, &driver, &[]);
        let promised = submit(&mut records, &driver, &none, 0, &[], &[at(t, 2)], false).unwrap();
        let stale = driver.read_timeline(&records, t);
        let stale_submit = read(&records, &driver, &[at(t, 3)]);
        assert_eq!((stale, stale_submit.queues[0]), (None, None), "read before the completion");
        driver.complete(0, promised, &[at(t, 2)]);
        // The third call: `timeline_value`'s read and hold.
        let third = driver.read_timeline(&records, t).expect("the promise reached");
        records.fold_timeline(t, third.1);
        records.settle();
        behind(&records, &driver);
        assert!(records.queues[0].submits.is_empty(), "the promise's submit dropped");
        let below = begin(&mut records, &driver, stale, at(t, 2)).unwrap_err();
        assert_eq!(rule(&below), Some(Rule::Timeline), "{below:?}");
        let seq = submit(&mut records, &driver, &stale_submit, 0, &[], &[at(t, 3)], false)
            .expect("a signal above the dropped promise");
        assert!(!records.pending(&[promised, 0, 0]) && records.pending(&[seq, 0, 0]));
    }

    /// A pass that begins between a heap write's first hold and its descriptor write
    /// declares the slot's old image, and its submit's walk before the write's second hold
    /// is refused; after it, the walk sees the image changed under the pass.
    #[test]
    fn a_pass_begun_during_a_slot_write_is_refused_at_its_walk() {
        let (mut records, driver) = (records(), Driver::new());
        let image: ImageHandle = records
            .images
            .insert(ImageRec { stamps: [0; QUEUES], slots: 0, placed: None, swapchain: false })
            .unwrap();
        records.begin_slot_write(0, Some(image)).expect("the first hold");
        let declared = None;
        let walked = records.check_slot(0, declared).unwrap_err();
        assert_eq!(rule(&walked), Some(Rule::HeapWrite), "{walked:?}");
        records.end_slot_write(0, Some(image));
        let walked = records.check_slot(0, declared).unwrap_err();
        assert_eq!(rule(&walked), Some(Rule::HeapLayout), "{walked:?}");
        assert_eq!(records.images.get(image).unwrap().slots, 1);
        // A pass begun after the write: its submit holds the slot from another write.
        records.check_slot(0, Some(image)).expect("a walk of the written image");
        let none = read(&records, &driver, &[]);
        let seq = submit(&mut records, &driver, &none, 0, &[], &[], false).unwrap();
        records.slots[0].stamps[0] = seq;
        let again = records.begin_slot_write(0, None).unwrap_err();
        assert_eq!(rule(&again), Some(Rule::HeapWrite), "{again:?}");
    }

    /// A destroy made at once after a wait that returned `true`: the wait's point is
    /// folded in, which completes its submit with the queue's own value still behind it,
    /// and the destroy's check is accepted.
    #[test]
    fn a_destroy_at_once_after_a_wait_with_the_queue_behind_is_accepted() {
        let (mut records, mut driver) = (records(), Driver::new());
        let t = timeline(&mut records, &mut driver, 0);
        let none = read(&records, &driver, &[]);
        let seq = submit(&mut records, &driver, &none, 0, &[], &[at(t, 1)], false).unwrap();
        let stamps = [seq, 0, 0];
        assert!(records.pending(&stamps));
        // The caller's semaphore signalled; the queue's own, at the batch's end, not yet.
        driver.signal(at(t, 1));
        // `wait_timeline` returned `true`: its hold.
        records.fold_timeline(t, 1);
        records.settle();
        behind(&records, &driver);
        assert_eq!(driver.queues[0], 0, "the queue's semaphore behind the stamp");
        assert!(!records.pending(&stamps), "the destroy's check, accepted");
        assert_eq!(driver.read_queues(&records), [None; QUEUES], "nothing left to read");
    }

    /// A submit on graphics credited through a compute submit's promise, whose driver call
    /// then fails after the credit: W on compute waits for a gate and signals g = 1; X on
    /// compute signals c = 1, its first hold made; N on graphics waits for c = 1 and
    /// signals g = 2, accepted; X fails. The host signal of c = 1 is refused while W is
    /// pending, since it would let N's g = 2 land before W's g = 1, and a destroy of what W
    /// uses is refused; once the gate opens, W completes and the batch with no work reaches
    /// X's stamp, it is accepted, and N's signal lands after W's.
    #[test]
    fn a_failed_submits_point_is_signalled_from_the_host_only_after_its_queue_reaches_it() {
        let (mut records, mut driver) = (records(), Driver::new());
        let (gate, g, c) = (
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
        );
        let none = read(&records, &driver, &[]);
        let w =
            submit(&mut records, &driver, &none, 1, &[at(gate, 1)], &[at(g, 1)], false).expect("W");
        records.fold(&none.queues, &none.points);
        records.check_points(1, &[], &[at(c, 1)]).expect("X's checks");
        let x = records.stamp(1, &[], &[at(c, 1)]);
        let now = read(&records, &driver, &[at(c, 1)]);
        let n = submit(&mut records, &driver, &now, 0, &[at(c, 1)], &[at(g, 2)], false)
            .expect("N, credited through X's promise");
        records.received(1, x, true);
        behind(&records, &driver);
        let read_c = driver.read_timeline(&records, c);
        let refused = begin(&mut records, &driver, read_c, at(c, 1)).unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline), "{refused:?}");
        assert!(records.pending(&[0, w, 0]), "the destroy's check, refused");
        host_signal(&mut records, &mut driver, at(gate, 1));
        driver.complete(1, w, &[at(g, 1)]);
        driver.complete(1, x, &[]);
        // The read of a call that reads every queue: a submit, a destroy, a heap write.
        let read = driver.read_queues(&records);
        records.fold(&read, &[]);
        behind(&records, &driver);
        assert!(!records.pending(&[0, x, 0]));
        host_signal(&mut records, &mut driver, at(c, 1));
        driver.complete(0, n, &[at(g, 2)]);
    }

    /// The same through a failed submit's wait: Q on graphics waits for a gate and signals
    /// f = 1; X on compute waits for f = 1 and signals c = 1, its first hold made; N on the
    /// present queue waits for c = 1 and signals f = 2, credited through X's wait; X fails,
    /// and its batch with no work reaches its stamp. A failed submit's waits order nothing
    /// once it has failed: a submit that waits for a later compute submit's signal is not
    /// credited through them. The host signal of c = 1 is refused until f = 1 is known
    /// reached.
    #[test]
    fn a_failed_submits_point_is_signalled_from_the_host_only_after_its_waits_are_reached() {
        let (mut records, mut driver) = (records(), Driver::new());
        let (gate, f, c, d) = (
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
        );
        let none = read(&records, &driver, &[]);
        let q =
            submit(&mut records, &driver, &none, 0, &[at(gate, 1)], &[at(f, 1)], false).expect("Q");
        records.fold(&none.queues, &none.points);
        records.check_points(1, &[at(f, 1)], &[at(c, 1)]).expect("X's checks");
        let x = records.stamp(1, &[at(f, 1)], &[at(c, 1)]);
        let now = read(&records, &driver, &[at(c, 1)]);
        let n = submit(&mut records, &driver, &now, 2, &[at(c, 1)], &[at(f, 2)], false)
            .expect("N, credited through X's wait");
        records.received(1, x, true);
        behind(&records, &driver);
        let none = read(&records, &driver, &[]);
        submit(&mut records, &driver, &none, 1, &[], &[at(d, 1)], false).expect("Y");
        let refused =
            submit(&mut records, &driver, &none, 2, &[at(d, 1)], &[at(f, 3)], false).unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline), "{refused:?}");
        driver.complete(1, x, &[]);
        // The read of a call that reads every queue: a submit, a destroy, a heap write.
        let read = driver.read_queues(&records);
        records.fold(&read, &[]);
        behind(&records, &driver);
        assert!(!records.pending(&[0, x, 0]), "X's stamp reached");
        let read_c = driver.read_timeline(&records, c);
        let refused = begin(&mut records, &driver, read_c, at(c, 1)).unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline), "{refused:?}");
        host_signal(&mut records, &mut driver, at(gate, 1));
        driver.complete(0, q, &[at(f, 1)]);
        // The caller sees f = 1, as a `timeline_value` of f or a `wait_timeline` for it does.
        let read_f = driver.read_timeline(&records, f);
        records.fold(&[None; QUEUES], read_f.as_slice());
        behind(&records, &driver);
        host_signal(&mut records, &mut driver, at(c, 1));
        driver.complete(2, n, &[at(f, 2)]);
    }

    /// A submit that only waits, made between a host signal's two holds, 2^31 below the
    /// marked value, is refused; one just within it is not.
    #[test]
    fn a_wait_only_submit_2_31_below_a_marked_host_signal_is_refused() {
        let (mut records, mut driver) = (records(), Driver::new());
        let t = timeline(&mut records, &mut driver, 0);
        host_signal(&mut records, &mut driver, at(t, 10));
        let marked = 10 + DIFFERENCE_LIMIT - 1;
        let read_first = driver.read_timeline(&records, t);
        begin(&mut records, &driver, read_first, at(t, marked)).expect("the first hold");
        let waits = [at(t, marked - DIFFERENCE_LIMIT)];
        let now = read(&records, &driver, &waits);
        let beyond = submit(&mut records, &driver, &now, 0, &waits, &[], false).unwrap_err();
        assert_eq!(rule(&beyond), Some(Rule::Timeline), "{beyond:?}");
        let waits = [at(t, marked - DIFFERENCE_LIMIT + 1)];
        submit(&mut records, &driver, &now, 0, &waits, &[], false).expect("a wait within");
        end(&mut records, &mut driver, at(t, marked));
        assert_eq!(records.timeline(t).unwrap().known, marked);
    }

    /// Host signals of one timeline from several threads, and reads of it, each making its
    /// driver read with no lock held, as the device does: after every hold the known value
    /// is at most the driver's, and the timeline ends at the greatest accepted signal. Run
    /// under Miri with many seeds by the nightly job.
    #[test]
    fn host_signals_racing_on_one_timeline_keep_it_behind_the_driver() {
        use std::sync::{Arc, Mutex};
        const ROUNDS: u64 = if cfg!(miri) { 12 } else { 2_000 };
        let (mut records, mut driver) = (records(), Driver::new());
        let t = timeline(&mut records, &mut driver, 0);
        let shared = Arc::new((Mutex::new(records), Mutex::new(driver)));
        let signaller = |offset: u64| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let (records, driver) = &*shared;
                let mut accepted = 0;
                for round in 0..ROUNDS {
                    let point = at(t, round * 2 + offset);
                    let read = {
                        let records = records.lock().unwrap();
                        let awaited = records.awaited(t);
                        drop(records);
                        let value = driver.lock().unwrap().value(t);
                        awaited.into_iter().filter(|&v| v <= value).max().map(|v| (t, v))
                    };
                    {
                        let mut records = records.lock().unwrap();
                        let driver = driver.lock().unwrap();
                        if begin(&mut records, &driver, read, point).is_err() {
                            continue;
                        }
                    }
                    driver.lock().unwrap().signal(point);
                    let mut records = records.lock().unwrap();
                    records.end_host_signal(point, true);
                    behind(&records, &driver.lock().unwrap());
                    accepted = point.value;
                }
                accepted
            })
        };
        let (one, two) = (signaller(1), signaller(2));
        let reader = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let (records, driver) = &*shared;
                for _ in 0..ROUNDS {
                    let value = driver.lock().unwrap().value(t);
                    let mut records = records.lock().unwrap();
                    records.fold_timeline(t, value);
                    records.settle();
                    behind(&records, &driver.lock().unwrap());
                }
            })
        };
        let greatest = one.join().unwrap().max(two.join().unwrap());
        reader.join().unwrap();
        let (records, driver) = &*shared;
        let known = records.lock().unwrap().timeline(t).unwrap().known;
        assert_eq!((known, driver.lock().unwrap().value(t)), (greatest, greatest));
    }

    /// A read made before a host signal, folded at a hold after it, names a value below the
    /// one the signal made known: the fold keeps the greater, so a signal or a submit's
    /// signal below the timeline's value is still refused (`fold_timeline`'s `max`).
    #[test]
    fn a_stale_read_folded_at_a_late_hold_lowers_no_timeline() {
        let (mut records, mut driver) = (records(), Driver::new());
        let t = timeline(&mut records, &mut driver, 0);
        let none = read(&records, &driver, &[]);
        let promised = submit(&mut records, &driver, &none, 0, &[], &[at(t, 2)], false).unwrap();
        driver.complete(0, promised, &[at(t, 2)]);
        // Two reads made now, each of the promise reached, held back past the signal below.
        let stale = driver.read_timeline(&records, t);
        let stale_submit = read(&records, &driver, &[at(t, 3)]);
        assert_eq!(stale, Some((t, 2)), "the reads carry the reached promise");
        let reached = driver.read_timeline(&records, t).expect("the promise reached");
        records.fold_timeline(t, reached.1);
        records.settle();
        host_signal(&mut records, &mut driver, at(t, 4));
        let below = begin(&mut records, &driver, stale, at(t, 3)).unwrap_err();
        assert_eq!(rule(&below), Some(Rule::Timeline), "{below:?}");
        let below =
            submit(&mut records, &driver, &stale_submit, 0, &[], &[at(t, 3)], false).unwrap_err();
        assert_eq!(rule(&below), Some(Rule::Timeline), "{below:?}");
    }

    /// Two submits of one queue whose signals one read finds reached: the settle completes
    /// the later, and the earlier with it, so a destroy at once after the wait that found them
    /// of what the later one uses is accepted (`settle`'s `max`; the rhi-api README's fourth
    /// interleaving).
    #[test]
    fn a_wait_that_reaches_two_submits_completes_the_later() {
        let (mut records, mut driver) = (records(), Driver::new());
        let t = timeline(&mut records, &mut driver, 0);
        let none = read(&records, &driver, &[]);
        let first = submit(&mut records, &driver, &none, 0, &[], &[at(t, 1)], false).unwrap();
        let none = read(&records, &driver, &[]);
        let later = submit(&mut records, &driver, &none, 0, &[], &[at(t, 2)], false).unwrap();
        assert!(later > first);
        driver.signal(at(t, 2));
        records.fold_timeline(t, 2);
        records.settle();
        behind(&records, &driver);
        assert!(!records.pending(&[later, 0, 0]), "the destroy's check, accepted");
    }

    /// How the caller lets a failed submit's promise go: X on compute waits for f = 1, which
    /// Q on graphics signals behind a gate, and signals c = 1, and fails; Y after it on
    /// compute signals s = 1. Once the caller has seen s = 1 and f = 1, X has left and the
    /// host signal of c = 1, which reads c alone, is accepted with no read of the queues.
    #[test]
    fn a_failed_submit_leaves_once_its_waits_and_a_later_point_of_its_queue_are_seen() {
        let (mut records, mut driver) = (records(), Driver::new());
        let (gate, f, c, s) = (
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
        );
        let none = read(&records, &driver, &[]);
        let q =
            submit(&mut records, &driver, &none, 0, &[at(gate, 1)], &[at(f, 1)], false).expect("Q");
        let x = submit(&mut records, &driver, &none, 1, &[at(f, 1)], &[at(c, 1)], true)
            .expect("X, failed");
        let y = submit(&mut records, &driver, &none, 1, &[], &[at(s, 1)], false).expect("Y");
        host_signal(&mut records, &mut driver, at(gate, 1));
        driver.complete(0, q, &[at(f, 1)]);
        driver.complete(1, x, &[]);
        driver.complete(1, y, &[at(s, 1)]);
        // What the hold of a `wait_timeline` that returned `true` does, for s = 1 and f = 1.
        for point in [at(s, 1), at(f, 1)] {
            records.fold_timeline(point.timeline, point.value);
            records.settle();
        }
        behind(&records, &driver);
        assert!(records.queues[1].submits.is_empty(), "X and Y have left");
        host_signal(&mut records, &mut driver, at(c, 1));
    }

    /// A failed submit's promise of a timeline that is gone completes nothing: W on compute
    /// waits for a gate no one signals; X after it signals c = 1 and fails; c is then
    /// removed, as `destroy_timeline` would if it did not refuse while X is kept. X's
    /// promise counts as reached, and W stays pending.
    #[test]
    fn a_failed_promise_of_a_removed_timeline_completes_nothing() {
        let (mut records, mut driver) = (records(), Driver::new());
        let (gate, g, c) = (
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
        );
        let none = read(&records, &driver, &[]);
        let w =
            submit(&mut records, &driver, &none, 1, &[at(gate, 1)], &[at(g, 1)], false).expect("W");
        submit(&mut records, &driver, &none, 1, &[], &[at(c, 1)], true).expect("X, failed");
        records.timelines.remove(c);
        records.settle();
        assert!(records.pending(&[0, w, 0]), "W counted complete");
    }

    /// The cover takes no failed submit's wait: P and Q on graphics wait for a gate and
    /// signal h = 1 and f = 1; X on compute waits for f = 1 and fails; Y after X on compute
    /// signals h = 2, which nothing orders after P's h = 1.
    #[test]
    fn a_failed_submits_wait_orders_no_later_submit_of_its_queue() {
        let (mut records, mut driver) = (records(), Driver::new());
        let (gate, f, c, h) = (
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
        );
        let none = read(&records, &driver, &[]);
        submit(&mut records, &driver, &none, 0, &[at(gate, 1)], &[at(h, 1)], false).expect("P");
        submit(&mut records, &driver, &none, 0, &[at(gate, 1)], &[at(f, 1)], false).expect("Q");
        submit(&mut records, &driver, &none, 1, &[at(f, 1)], &[at(c, 1)], true).expect("X, failed");
        let refused = submit(&mut records, &driver, &none, 1, &[], &[at(h, 2)], false).unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline), "{refused:?}");
    }

    /// The credit at the top of a submit takes no failed submit's wait: Q on graphics waits
    /// for a gate and signals g = 1 and f = 2; X on compute waits for f = 2 and fails; Y
    /// after it signals d = 1; M on the present queue waits for d = 1 and g = 1 and signals
    /// f = 3, which nothing orders after Q's f = 2.
    #[test]
    fn a_failed_submits_wait_gives_no_credit_at_the_top() {
        let (mut records, mut driver) = (records(), Driver::new());
        let (gate, f, c, d, g) = (
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
            timeline(&mut records, &mut driver, 0),
        );
        let none = read(&records, &driver, &[]);
        submit(&mut records, &driver, &none, 0, &[at(gate, 1)], &[at(g, 1), at(f, 2)], false)
            .expect("Q");
        submit(&mut records, &driver, &none, 1, &[at(f, 2)], &[at(c, 1)], true).expect("X, failed");
        submit(&mut records, &driver, &none, 1, &[], &[at(d, 1)], false).expect("Y");
        let refused =
            submit(&mut records, &driver, &none, 2, &[at(d, 1), at(g, 1)], &[at(f, 3)], false)
                .unwrap_err();
        assert_eq!(rule(&refused), Some(Rule::Timeline), "{refused:?}");
    }

    /// A failed submit that waits for its own timeline below its signal is released by the
    /// host's signal of that wait: X on compute waits for c = 1, signals c = 2 and fails.
    /// Nothing else can reach X's wait, so were the signal refused X would be kept for good.
    #[test]
    fn a_failed_submit_waiting_for_its_own_timeline_is_released_by_a_host_signal() {
        let (mut records, mut driver) = (records(), Driver::new());
        let c = timeline(&mut records, &mut driver, 0);
        let none = read(&records, &driver, &[]);
        let x = submit(&mut records, &driver, &none, 1, &[at(c, 1)], &[at(c, 2)], true)
            .expect("X, failed");
        driver.complete(1, x, &[]);
        let queues = driver.read_queues(&records);
        records.fold(&queues, &[]);
        host_signal(&mut records, &mut driver, at(c, 1));
        assert!(records.queues[1].submits.is_empty(), "X has left");
        let now = read(&records, &driver, &[at(c, 2)]);
        submit(&mut records, &driver, &now, 1, &[], &[at(c, 2)], false).expect("the retry");
    }

    /// A present's walk of a point another queue's submit promises: undecided while that
    /// submit is stamped and its driver call has not returned, which may yet fail; released
    /// once the call has returned; undecided for a submit whose call failed, whose promise
    /// never reached the driver.
    #[test]
    fn a_walk_releases_a_promise_once_its_submit_is_received_and_not_failed() {
        let (graphics, present) = (0, 2);
        for failed in [false, true] {
            let mut records = records();
            let t = records.timelines.insert(TimelineRec { known: 0, mark: None }).unwrap();
            let point = TimelinePoint { timeline: t, value: 1 };
            let seq = records.stamp(graphics, &[], &[point]);
            let walked = records.release(present, &[point]).unwrap();
            assert_eq!(walked, Release::Undecided(point), "stamped and not received");
            records.received(graphics, seq, failed);
            let walked = records.release(present, &[point]).unwrap();
            let expected = if failed { Release::Undecided(point) } else { Release::Released };
            assert_eq!(walked, expected, "received, failed: {failed}");
        }
    }

    /// A point already reached is released whatever promises it, a submit stamped and not
    /// received among them; a promise's own waits are walked in turn.
    #[test]
    fn a_walk_releases_a_reached_point_and_walks_a_promises_waits() {
        let (graphics, compute, present) = (0, 1, 2);
        let mut records = records();
        let mut timeline = || records.timelines.insert(TimelineRec { known: 0, mark: None });
        let (t, gate) = (timeline().unwrap(), timeline().unwrap());
        let at = |timeline, value| TimelinePoint { timeline, value };
        let unreceived = records.stamp(compute, &[], &[at(t, 2)]);
        records.fold(&[None, None, None], &[(t, 1)]);
        assert_eq!(records.release(present, &[at(t, 1)]).unwrap(), Release::Released);
        assert_eq!(records.release(present, &[at(t, 2)]).unwrap(), Release::Undecided(at(t, 2)));
        records.received(compute, unreceived, false);
        assert_eq!(records.release(present, &[at(t, 2)]).unwrap(), Release::Released);
        // A received promise whose submit waits for a gate no one has signalled or promised.
        let held = records.stamp(graphics, &[at(gate, 1)], &[at(t, 3)]);
        records.received(graphics, held, false);
        let walked = records.release(present, &[at(t, 3)]).unwrap();
        assert_eq!(walked, Release::Undecided(at(gate, 1)));
        records.fold(&[None, None, None], &[(gate, 1)]);
        assert_eq!(records.release(present, &[at(t, 3)]).unwrap(), Release::Released);
    }
}
