//! The records: the state the caller carries between calls, which every check reads. One
//! lock guards them (`Shared::records`); a call holds it once for its checks and the
//! updates they permit, and where it must run the caller's closure or let another call in
//! between, a second time, which decides nothing.
//!
//! A submit is pending, for every rule that names a pending submit, until the caller has
//! seen a point that it, a later submit to its queue, or a submit that waits for such a
//! point, signals (`lactose-rhi-api`'s README, "Queues by role"). Each queue keeps two
//! marks in its submission order: the submits through `ran` have run, and those through
//! `observed` are seen. A submit runs once its waits are reached and every earlier submit
//! of its queue has run, so a timeline's value here is both what the device knows and
//! what its driver has.

use std::collections::{HashSet, VecDeque};
use std::num::NonZeroU32;
use std::sync::Arc;

use lactose_core::{Error, ErrorKind, Handle, Result};
use lactose_rhi_api::{
    BufferCopy, BufferState, CacheStart, ImageHandle, ImageState, MemoryBlockHandle, MemoryClass,
    PipelineStats, QueueRole, Rule, SamplerDesc, Sharing, TimelineHandle, TimelinePoint,
};

use crate::device::BufferObj;
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

/// For each queue, the last submit that named something, by its sequence number; zero for
/// none.
pub(crate) type Stamps = [u64; QUEUES];

/// Records of one kind of resource, by handle index, each with the generation its handle
/// carries.
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
                        "the null device holds as many of these as it can",
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

    pub(crate) fn subtract(&mut self, start: u64, end: u64) {
        let mut kept = Vec::with_capacity(self.0.len());
        for &(a, b) in &self.0 {
            if b <= start || a >= end {
                kept.push((a, b));
                continue;
            }
            if a < start {
                kept.push((a, start));
            }
            if b > end {
                kept.push((end, b));
            }
        }
        self.0 = kept;
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

/// A release submitted, which an acquire of equal values completes: the states, and the
/// queues as the layout resolves them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Transfer<S> {
    pub(crate) from: S,
    pub(crate) from_queue: usize,
    pub(crate) to: S,
    pub(crate) to_queue: usize,
}

/// One subresource of an image, or a buffer: its state, the queue that owns it, a release
/// awaiting its acquire, whether its contents are defined, and whether a pass or copy has
/// written it since its last transition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sub<S> {
    pub(crate) state: S,
    pub(crate) owner: Option<usize>,
    pub(crate) transfer: Option<Transfer<S>>,
    pub(crate) defined: bool,
    pub(crate) written: bool,
}

impl<S> Sub<S> {
    pub(crate) const fn new(state: S) -> Self {
        Self { state, owner: None, transfer: None, defined: false, written: false }
    }
}

/// Where a placed image lies in its block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    pub(crate) block: MemoryBlockHandle,
    pub(crate) start: u64,
    pub(crate) end: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ImageRec {
    pub(crate) mips: u32,
    pub(crate) sharing: Sharing,
    pub(crate) subs: Vec<Sub<ImageState>>,
    pub(crate) stamps: Stamps,
    /// The last submit of each queue that writes it or changes its state or owner.
    pub(crate) changed: Stamps,
    /// Heap slots that hold it, the one a write in progress gives it included.
    pub(crate) slots: u32,
    pub(crate) placed: Option<Placed>,
    /// For a placed image: begun, by an alias call or by a first use where no image that
    /// overlaps it was used, and not ended since by another's alias call.
    pub(crate) active: bool,
    /// For a placed image: ended by another's alias call, and not begun by one of its own
    /// since. Kept when the image that ended it is destroyed.
    pub(crate) ended: bool,
    /// For a placed image: the queue of its last use, `None` before its first.
    pub(crate) used_on: Option<usize>,
    /// For a swapchain image: whether it is the acquired one.
    pub(crate) acquired: Option<bool>,
}

impl ImageRec {
    pub(crate) fn new(
        mips: u32,
        layers: u32,
        sharing: Sharing,
        placed: Option<Placed>,
        swapchain: bool,
    ) -> Self {
        Self {
            mips,
            sharing,
            subs: vec![Sub::new(ImageState::Undefined); (mips * layers) as usize],
            stamps: [0; QUEUES],
            changed: [0; QUEUES],
            slots: 0,
            placed,
            active: false,
            ended: false,
            used_on: None,
            acquired: swapchain.then_some(false),
        }
    }

    pub(crate) const fn sub(&self, mip: u32, layer: u32) -> usize {
        (layer * self.mips + mip) as usize
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BufferRec {
    pub(crate) size: u64,
    pub(crate) memory: MemoryClass,
    pub(crate) sharing: Sharing,
    pub(crate) sub: Sub<BufferState>,
    pub(crate) stamps: Stamps,
    /// The last submit of each queue that writes it: what a host write waits for.
    pub(crate) writers: Stamps,
    /// The last submit of each queue that writes it or changes its state or owner: what a
    /// host read and an invalidate wait for, and a use on another queue.
    pub(crate) changed: Stamps,
    /// Host accesses in progress.
    pub(crate) host: Ranges,
    /// Host-written ranges with no flush since, where memory is not coherent.
    pub(crate) unflushed: Ranges,
    /// Device-written ranges with no invalidate since, where memory is not coherent.
    pub(crate) uninvalidated: Ranges,
}

pub(crate) struct BlockRec {
    pub(crate) size: u64,
    /// The images placed in it.
    pub(crate) images: Vec<ImageHandle>,
}

#[derive(Default)]
pub(crate) struct PipelineRec {
    pub(crate) stamps: Stamps,
}

pub(crate) struct TimelineRec {
    /// The timeline's value: the greatest the device knows it has reached, which on null
    /// is its driver's too.
    pub(crate) value: u64,
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

/// Bytes a buffer-to-buffer copy moves when its submit runs.
pub(crate) struct CopyBytes {
    pub(crate) source: Arc<BufferObj>,
    pub(crate) destination: Arc<BufferObj>,
    pub(crate) regions: Vec<BufferCopy>,
}

/// A submit the records still keep: from its acceptance until it is seen.
pub(crate) struct SubmitRec {
    pub(crate) seq: u64,
    pub(crate) waits: Vec<TimelinePoint>,
    pub(crate) signals: Vec<TimelinePoint>,
    pub(crate) copies: Vec<CopyBytes>,
}

pub(crate) struct QueueRec {
    /// The sequence number the next submit takes, from one.
    pub(crate) next: u64,
    /// Every submit through this one has run.
    pub(crate) ran: u64,
    /// Every submit through this one the caller has seen.
    pub(crate) observed: u64,
    /// The submits after `observed`, in order.
    pub(crate) submits: VecDeque<SubmitRec>,
}

impl QueueRec {
    const fn new() -> Self {
        Self { next: 1, ran: 0, observed: 0, submits: VecDeque::new() }
    }

    fn get(&self, seq: u64) -> Option<&SubmitRec> {
        let front = self.submits.front()?.seq;
        self.submits.get(usize::try_from(seq.checked_sub(front)?).ok()?)
    }

    /// The submits after `after`, in order.
    pub(crate) fn after(&self, after: u64) -> impl Iterator<Item = &SubmitRec> {
        self.submits.iter().filter(move |submit| submit.seq > after)
    }

    /// The submits that have not run.
    pub(crate) fn unrun(&self) -> impl Iterator<Item = &SubmitRec> {
        self.after(self.ran)
    }
}

/// A present whose image the driver may still read: its swapchain, its image, and the
/// submit it made.
pub(crate) struct PresentRec {
    pub(crate) swapchain: u64,
    pub(crate) image: ImageHandle,
    pub(crate) queue: usize,
    pub(crate) seq: u64,
}

/// What a release walk decided of a present's submit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Release {
    /// Every point it depends on is reached, or promised by a released submit.
    Released,
    /// The walk could not decide this point: no released promise meets it, or the walk
    /// came back to a submit it was still deciding, and this is the point it came back
    /// from. The present waits for it with no lock held and walks again.
    Undecided(TimelinePoint),
}

/// The pipeline cache's stages and the statistics a device keeps.
pub(crate) struct PipelineState {
    pub(crate) stats: PipelineStats,
    /// The stages the cache holds, by the hash of their cooked bytes.
    pub(crate) stages: HashSet<u64>,
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
    pub(crate) samplers: Vec<Option<SamplerDesc>>,
    pub(crate) queues: [QueueRec; QUEUES],
    /// Presents whose image the driver may still read.
    pub(crate) presents: Vec<PresentRec>,
    /// The live swapchain's number, if one lives.
    pub(crate) swapchain: Option<u64>,
    /// The number the next swapchain takes.
    pub(crate) next_swapchain: u64,
    /// Presents are held from completing (`Control::stall`).
    pub(crate) stalled: bool,
    /// Where the next buffer's address begins.
    pub(crate) next_address: u64,
    pub(crate) pipelines: PipelineState,
    /// Under test: queues run nothing while it is set, as a lagging GPU's would not.
    #[cfg(test)]
    pub(crate) lag: bool,
}

impl Records {
    pub(crate) fn new(
        heap_slots: u32,
        samplers: u32,
        cache: CacheStart,
        stages: HashSet<u64>,
    ) -> Self {
        Self {
            lost: false,
            buffers: Slots::new(),
            images: Slots::new(),
            blocks: Slots::new(),
            graphics: Slots::new(),
            compute: Slots::new(),
            timelines: Slots::new(),
            slots: (0..heap_slots).map(|_| SlotRec::default()).collect(),
            samplers: (0..samplers).map(|_| None).collect(),
            queues: [QueueRec::new(), QueueRec::new(), QueueRec::new()],
            presents: Vec::new(),
            swapchain: None,
            next_swapchain: 1,
            stalled: false,
            next_address: 1 << 32,
            pipelines: PipelineState {
                stats: PipelineStats {
                    cache,
                    pipelines: 0,
                    stage_hits: 0,
                    stage_misses: 0,
                    creation: core::time::Duration::ZERO,
                },
                stages,
            },
            #[cfg(test)]
            lag: false,
        }
    }

    /// Whether a submit stamped in `stamps` is pending: one the caller has not seen.
    pub(crate) fn pending(&self, stamps: &Stamps) -> bool {
        (0..QUEUES).any(|queue| stamps[queue] > self.queues[queue].observed)
    }

    /// Whether any submit is pending.
    pub(crate) fn any_pending(&self) -> bool {
        self.queues.iter().any(|queue| queue.next - 1 > queue.observed)
    }

    pub(crate) fn timeline(&self, timeline: TimelineHandle) -> Result<&TimelineRec> {
        self.timelines.live(timeline, "a timeline")
    }

    /// The submits a new submit to `queue` that waits for `waits` is ordered after, as a
    /// mark for each queue: every earlier submit of its own queue; for each point it waits
    /// for, each submit that signals that timeline at or below the point and every earlier
    /// submit of that one's queue; and in turn what each of those waits for. Submits the
    /// caller has seen are below every mark.
    pub(crate) fn cover(&self, queue: usize, waits: &[TimelinePoint]) -> Stamps {
        let mut cover = self.queues.each_ref().map(|rec| rec.observed);
        let mut work = waits.to_vec();
        self.raise(&mut cover, queue, self.queues[queue].next - 1, &mut work);
        self.close(&mut cover, work);
        cover
    }

    /// The submits a new submit is ordered after by its waits alone: for each point it
    /// waits for, each submit that signals that timeline at or below the point and every
    /// earlier submit of that one's queue, and in turn what each of those waits for. The
    /// earlier submits of its own queue add nothing, nor do their waits, unlike
    /// [`cover`](Self::cover)'s.
    pub(crate) fn waited(&self, waits: &[TimelinePoint]) -> Stamps {
        let mut cover = self.queues.each_ref().map(|rec| rec.observed);
        self.close(&mut cover, waits.to_vec());
        cover
    }

    /// Raises `cover` through every submit that signals a point of `work`, as `cover`
    /// says.
    fn close(&self, cover: &mut Stamps, mut work: Vec<TimelinePoint>) {
        while let Some(point) = work.pop() {
            for at in 0..QUEUES {
                let top = self.queues[at]
                    .after(cover[at])
                    .filter(|submit| signals_at_or_below(submit, point))
                    .map(|submit| submit.seq)
                    .max();
                if let Some(top) = top {
                    self.raise(cover, at, top, &mut work);
                }
            }
        }
    }

    /// Raises `cover[queue]` to `top`, adding the waits of the submits it takes in.
    fn raise(&self, cover: &mut Stamps, queue: usize, top: u64, work: &mut Vec<TimelinePoint>) {
        if top <= cover[queue] {
            return;
        }
        let taken = self.queues[queue].after(cover[queue]).take_while(|submit| submit.seq <= top);
        for submit in taken {
            work.extend_from_slice(&submit.waits);
        }
        cover[queue] = top;
    }

    /// Records that the caller has seen `point` reached: every submit that signals the
    /// timeline at or below it, the earlier submits of its queue and what those waited for
    /// are seen, and leave the records.
    pub(crate) fn observe(&mut self, point: TimelinePoint) {
        let mut cover = self.queues.each_ref().map(|rec| rec.observed);
        self.close(&mut cover, vec![point]);
        for (at, rec) in self.queues.iter_mut().enumerate() {
            rec.observed = rec.observed.max(cover[at].min(rec.ran));
            while rec.submits.front().is_some_and(|submit| submit.seq <= rec.observed) {
                rec.submits.pop_front();
            }
        }
    }

    /// Whether `point` is reached; `false` for a stale timeline.
    pub(crate) fn reached(&self, point: TimelinePoint) -> bool {
        self.timelines.get(point.timeline).is_some_and(|timeline| timeline.value >= point.value)
    }

    /// Runs every submit whose waits are reached and whose queue's earlier submits have
    /// run, moving its bytes and signalling its points, until none can run.
    pub(crate) fn run(&mut self) {
        #[cfg(test)]
        if self.lag {
            return;
        }
        if self.lost {
            return;
        }
        loop {
            let mut progress = false;
            for queue in 0..QUEUES {
                loop {
                    let rec = &self.queues[queue];
                    let Some(next) = rec.unrun().next() else { break };
                    if !next.waits.iter().all(|&wait| self.reached(wait)) {
                        break;
                    }
                    for copy in &next.copies {
                        move_bytes(copy);
                    }
                    let (seq, signals) = (next.seq, next.signals.clone());
                    self.queues[queue].ran = seq;
                    for signal in signals {
                        if let Some(timeline) = self.timelines.get_mut(signal.timeline) {
                            timeline.value = timeline.value.max(signal.value);
                        }
                    }
                    progress = true;
                }
            }
            if !progress {
                return;
            }
        }
    }

    /// The timeline checks of a submit ordered after the submits `cover` names: each
    /// wait's and signal's timeline lives; no value is 2^31 or more from those it is
    /// compared with; each signal grows past the timeline's value, its host signal in
    /// progress, every pending promise and the submit's own waits and earlier signals on
    /// it; every pending promise of it is from a submit `cover` names; and while a host
    /// signal of it is in progress, the submit waits for that signal's value.
    pub(crate) fn check_points(
        &self,
        waits: &[TimelinePoint],
        signals: &[TimelinePoint],
        cover: &Stamps,
    ) -> Result<()> {
        for &point in waits.iter().chain(signals) {
            self.within_limit(point)?;
        }
        for (at, &signal) in signals.iter().enumerate() {
            let timeline = self.timeline(signal.timeline)?;
            let refuse =
                |what: &str| Rule::Timeline.refuse(format!("a signal of {} {what}", signal.value));
            if signal.value <= timeline.value {
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
                if !own_waits().any(|wait| wait.value > timeline.value && wait.value <= mark) {
                    return Err(refuse("while a host signal it does not wait for is in progress"));
                }
            }
            for (other, rec) in self.queues.iter().enumerate() {
                for submit in rec.unrun() {
                    let promises = submit.signals.iter().filter(|p| p.timeline == signal.timeline);
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

    /// Refuses a value 2^31 or more from the timeline's value, from its host signal in
    /// progress, or from the least or the greatest value a pending submit waits for or
    /// signals on it.
    pub(crate) fn within_limit(&self, point: TimelinePoint) -> Result<()> {
        let timeline = self.timeline(point.timeline)?;
        let listed = self.queues.iter().flat_map(QueueRec::unrun).flat_map(|submit| {
            submit.waits.iter().chain(&submit.signals).filter(|p| p.timeline == point.timeline)
        });
        let (least, greatest) = listed.fold((None::<u64>, None::<u64>), |(least, greatest), p| {
            (
                Some(least.map_or(p.value, |v| v.min(p.value))),
                Some(greatest.map_or(p.value, |v| v.max(p.value))),
            )
        });
        let compared = [Some(timeline.value), timeline.mark, least, greatest];
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
        if point.value <= timeline.value {
            return Err(refuse("not above the timeline's value"));
        }
        if timeline.mark.is_some() {
            return Err(refuse("while another host signal of the timeline is in progress"));
        }
        self.within_limit(point)?;
        let known = timeline.value;
        for submit in self.queues.iter().flat_map(QueueRec::unrun) {
            for promise in submit.signals.iter().filter(|p| p.timeline == point.timeline) {
                if promise.value <= point.value {
                    return Err(refuse("not below a pending promise"));
                }
                let released_by_this = submit.waits.iter().any(|wait| {
                    wait.timeline == point.timeline
                        && wait.value > known
                        && wait.value <= point.value
                });
                if !released_by_this {
                    return Err(refuse("below a promise whose submit it does not alone release"));
                }
            }
        }
        self.timelines.live_mut(point.timeline, "a timeline")?.mark = Some(point.value);
        Ok(())
    }

    /// The second hold of a host signal: the mark cleared and the value folded in, while
    /// the timeline lives; then whatever it released runs.
    pub(crate) fn end_host_signal(&mut self, point: TimelinePoint) {
        if let Some(timeline) = self.timelines.get_mut(point.timeline) {
            timeline.mark = None;
            timeline.value = timeline.value.max(point.value);
        }
        self.run();
    }

    /// Decides whether a submit to `queue` made now, waiting for `waits`, is released: each
    /// of its waits reached or promised by a released submit, and every earlier submit of
    /// its queue released (`lactose-rhi-api`'s README, "The swapchain").
    pub(crate) fn release(&self, queue: usize, waits: &[TimelinePoint]) -> Result<Release> {
        let mut walk =
            Walk { decided: self.queues.each_ref().map(|rec| rec.ran), deciding: [None; QUEUES] };
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

    /// Decides one point: `None` when it is reached or its least promise at or above it is
    /// released, or else the point the walk could not decide.
    fn point(&self, point: TimelinePoint, walk: &mut Walk) -> Result<Option<TimelinePoint>> {
        let timeline = self.timeline(point.timeline)?;
        if timeline.value >= point.value {
            return Ok(None);
        }
        // A greater promise never decides a point: every greater one depends on the least.
        let mut least: Option<(u64, usize, u64)> = None;
        for (queue, rec) in self.queues.iter().enumerate() {
            for submit in rec.unrun() {
                for promise in &submit.signals {
                    let meets = promise.timeline == point.timeline && promise.value >= point.value;
                    if meets && least.is_none_or(|(value, ..)| promise.value < value) {
                        least = Some((promise.value, queue, submit.seq));
                    }
                }
            }
        }
        let Some((_, queue, seq)) = least else { return Ok(Some(point)) };
        self.submits(queue, seq, Some(point), walk)
    }

    /// Decides the submits of `queue` through `through`, in order: each is released once
    /// its waits are and the one before it is. `via` is the point whose promise led here;
    /// the first call of a walk, from the present's own queue, has none and finds nothing
    /// in progress.
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
        for seq in walk.decided[queue] + 1..=through {
            // The submits from `seq` on are in progress; those before it are decided.
            walk.deciding[queue] = Some(seq);
            let Some(submit) = self.queues[queue].get(seq) else { continue };
            for &wait in &submit.waits {
                if let Some(point) = self.point(wait, walk)? {
                    return Ok(Some(point));
                }
            }
            walk.decided[queue] = seq;
        }
        walk.deciding[queue] = None;
        Ok(None)
    }

    /// Whether the driver no longer reads a present's image: its submit has run and
    /// presents are not held.
    pub(crate) fn present_done(&self, present: &PresentRec) -> bool {
        !self.stalled && present.seq <= self.queues[present.queue].ran
    }

    /// Forgets the presents that are done.
    pub(crate) fn settle_presents(&mut self) {
        if self.stalled {
            return;
        }
        let ran = self.queues.each_ref().map(|rec| rec.ran);
        self.presents.retain(|present| present.seq > ran[present.queue]);
    }
}

/// What a release walk has decided: for each queue, every submit through `decided` is
/// released, and the submits from `deciding` on are still being decided. The two are kept
/// apart, so a return to a submit in progress is never taken for a decision.
struct Walk {
    decided: Stamps,
    deciding: [Option<u64>; QUEUES],
}

fn signals_at_or_below(submit: &SubmitRec, point: TimelinePoint) -> bool {
    submit
        .signals
        .iter()
        .any(|signal| signal.timeline == point.timeline && signal.value <= point.value)
}

/// Moves a copy's bytes from one store to the other, holding one store's lock at a time.
fn move_bytes(copy: &CopyBytes) {
    for region in &copy.regions {
        let (Ok(from), Ok(to), Ok(len)) = (
            usize::try_from(region.source_offset),
            usize::try_from(region.destination_offset),
            usize::try_from(region.size),
        ) else {
            continue;
        };
        let bytes = copy.source.read(from, len);
        copy.destination.write(to, &bytes);
    }
}
