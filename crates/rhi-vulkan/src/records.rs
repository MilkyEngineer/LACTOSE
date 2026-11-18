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
    /// its queue, and drops the submits whose completion and promises are all known.
    pub(crate) fn settle(&mut self) {
        let timelines = &self.timelines;
        let reached = |point: &TimelinePoint| {
            timelines.get(point.timeline).is_none_or(|rec| rec.known >= point.value)
        };
        for rec in &mut self.queues {
            let done = rec
                .submits
                .iter()
                .filter(|submit| submit.seq > rec.known && submit.signals.iter().any(reached))
                .map(|submit| submit.seq)
                .max();
            if let Some(done) = done {
                rec.known = done;
            }
            let known = rec.known;
            rec.submits.retain(|submit| submit.seq > known || !submit.signals.iter().all(reached));
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

    /// Whether a promise of `point`'s timeline is pending: its value not yet known reached.
    fn promise_pending(&self, promise: &TimelinePoint) -> bool {
        self.timelines.get(promise.timeline).is_some_and(|rec| rec.known < promise.value)
    }

    /// The submits a new submit to `queue` that waits for `waits` is ordered after, as a
    /// mark for each queue: every earlier submit of its own queue; for each point it waits
    /// for, each submit that signals that timeline at or below the point and every earlier
    /// submit of that one's queue; and in turn what each of those waits for. Completed
    /// submits are below every mark.
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
            work.extend_from_slice(&submit.waits);
        }
        cover[queue] = top;
    }

    /// The timeline checks of a submit to `queue` (`lactose-rhi-api`'s README,
    /// "Timelines"): each wait's and signal's timeline lives; no value is 2^31 or more from
    /// those it is compared with; each signal grows past the timeline's known value, its
    /// host signal in progress, every pending promise and the submit's own waits and
    /// earlier signals on it; every pending promise of it is from a submit the new one is
    /// ordered after; and while a host signal of it is in progress, the submit waits for
    /// that signal's value.
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
                        if submit.seq > cover[other] {
                            return Err(refuse(
                                "while another queue's signal of it is pending, with no wait for it",
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
        rec.submits.push_back(SubmitRec { seq, waits: waits.to_vec(), signals: signals.to_vec() });
        seq
    }

    /// The hold after a submit's driver call: its stamp received; a failed submit's
    /// promises and points dropped, so a retry of the same points is accepted.
    pub(crate) fn received(&mut self, queue: usize, seq: u64, failed: bool) {
        let rec = &mut self.queues[queue];
        rec.received = rec.received.max(seq);
        if failed && let Some(submit) = rec.submits.iter_mut().find(|submit| submit.seq == seq) {
            submit.waits.clear();
            submit.signals.clear();
        }
        self.settle();
    }

    /// Whether any submit has not completed.
    pub(crate) fn any_pending(&self) -> bool {
        self.queues.iter().any(|queue| queue.next - 1 > queue.known)
    }

    /// The stamp each queue's next submit takes.
    pub(crate) fn next_stamps(&self) -> Stamps {
        self.queues.each_ref().map(|rec| rec.next)
    }
}

#[cfg(test)]
mod tests {
    use lactose_rhi_api::{CacheStart, Rule, TimelinePoint};

    use super::{Records, TimelineRec};

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

    #[test]
    fn a_failed_submits_promise_is_dropped_and_its_point_retried() {
        let mut records = records();
        let timeline = records.timelines.insert(TimelineRec { known: 0, mark: None }).unwrap();
        let point = TimelinePoint { timeline, value: 1 };
        records.check_points(0, &[], &[point]).unwrap();
        let seq = records.stamp(0, &[], &[point]);
        let again = records.check_points(0, &[], &[point]).unwrap_err();
        assert_eq!(rule(&again), Some(Rule::Timeline));
        records.received(0, seq, true);
        records.check_points(0, &[], &[point]).unwrap();
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
}
