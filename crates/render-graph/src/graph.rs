//! The graph: the plan cache, the transients and the encoders, and the three steps a frame
//! takes through them.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    Capabilities, Device, Encoder, ImageDesc, ImageHandle, ImageMemory, MemoryBlockDesc,
    MemoryBlockHandle, QueueRole, Rule, Sharing, Submit, TimelineHandle, TimelinePoint,
};
use lactose_task::JobContext;

use crate::compile::{Shape, derive, validate};
use crate::decl::{DeclaredImage, FrameDecl, Recorder};
use crate::job::{self, Owned, Shared};
use crate::plan::{CompiledFrame, CompiledPlan, GraphKey, Owner, Resource, Transient};
use crate::resource::TransientMemory;
use crate::sync_view::SyncView;

/// The target of the graph's spans and events. Every kept pass is recorded inside one
/// span of this target named `pass`, at trace level, with the pass's label as its `label`
/// field.
pub const LOG_TARGET: &str = "gouda";

/// What a graph is made with.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GraphDesc {
    /// The most recording jobs the passes of one submit are split into, at least one. A
    /// submit that carries `n` passes is recorded by the smaller of `n` and `jobs` jobs,
    /// each a run of consecutive passes with an encoder of its own.
    pub jobs: u32,
    /// The most plans the cache keeps, at least one. A plan compiled into a full cache
    /// evicts the plan least recently returned by
    /// [`compile`](RenderGraph::compile).
    pub plans: u32,
}

/// What a graph has done so far: counts a test or a trace reads. None is a timing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GraphStats {
    /// The plans compiled: the calls to [`compile`](RenderGraph::compile) that missed the
    /// cache and succeeded.
    pub compiles: u64,
    /// The calls to [`compile`](RenderGraph::compile) that the cache answered.
    pub hits: u64,
    /// The plans evicted from the cache.
    pub evictions: u64,
    /// The plans the cache holds now.
    pub plans: u32,
    /// The encoders the graph holds: free, and pending in a submitted frame. A recorded
    /// frame's encoders are its own from `record`'s return to `submit`, and are not
    /// counted between the two.
    pub encoders: u32,
}

/// The render graph of one device (architecture section 12.3).
///
/// A frame goes through it in three calls, each consuming what the one before returned:
/// [`compile`](Self::compile), [`record`](Self::record) and [`submit`](Self::submit).
/// Between `record` and `submit` the caller latches: it writes the buffers the frame reads
/// last, such as the camera's, and nothing recorded depends on their bytes.
///
/// `Send + Sync + 'static`, so that it sits in a world's resource or behind a `dyn`
/// object, and every call that changes it takes `&mut self`: one thread at a time by the
/// borrow, with no lock (`README.md`, "Concurrency").
///
/// Its drop waits, with no bound, until every submit it made has completed, then
/// destroys the transients, the memory blocks, the timelines and the encoders it made. It
/// is dropped off the frame-critical workers, as a device's objects are, and after the
/// host has signalled every point a pending frame waits for. A drop made while the thread
/// unwinds a panic does not wait: it destroys what no pending submit uses and leaks the
/// rest, as the device's own drops do (`README.md`, "Errors and drops").
pub struct RenderGraph {
    /// The device every call reaches.
    device: Arc<dyn Device>,
    /// The device's capabilities, which are fixed.
    caps: Capabilities,
    desc: GraphDesc,
    /// The mark of the plans this graph compiles.
    owner: Arc<Owner>,
    /// The plans the cache holds, at most `desc.plans`.
    cache: Vec<Entry>,
    /// Evicted plans whose transients are not yet destroyed.
    retired: Vec<Entry>,
    /// The count of `compile`'s returns, which orders the cache's entries by use.
    tick: u64,
    compiles: u64,
    hits: u64,
    evictions: u64,
    timelines: Timelines,
    /// The last frame point made and where it was signalled from: the last returned, or
    /// the one signalled in a torn frame's place since.
    previous: Option<(TimelinePoint, QueueRole)>,
    /// The point the last call to `submit` returned.
    last_frame: Option<TimelinePoint>,
    /// The frame that a later submit failed in, whose point is owed.
    torn: Option<Torn>,
    /// The encoders no frame holds.
    encoders: Encoders,
}

/// A plan in the cache, or evicted.
struct Entry {
    key: GraphKey,
    /// What the key hashed, compared on a hit.
    shape: Shape,
    plan: Arc<CompiledPlan>,
    /// The tick of the last `compile` that returned it.
    used: u64,
    /// The point of the last frame submitted from it.
    last: Option<u64>,
}

/// The timelines the graph's submits signal.
struct Timelines {
    /// The frame points'.
    frame: TimelineHandle,
    /// The last frame value assigned.
    frame_value: u64,
    /// On a device whose compute role has a queue of its own, one for each queue the graph
    /// submits to, graphics then compute, which its submits signal so that a submit to
    /// the other waits for them.
    queues: Option<[QueueTimeline; 2]>,
}

struct QueueTimeline {
    timeline: TimelineHandle,
    /// The last value a made submit signalled.
    value: u64,
}

/// A frame torn by a refused submit after its first.
struct Torn {
    /// Its point, which the submit in its place signals.
    point: TimelinePoint,
    /// The queue its last submit was planned for.
    queue: QueueRole,
}

/// The encoders the graph holds, and how many: the count is read through `&self`, which
/// never reaches the encoders.
struct Encoders {
    held: SyncView<Held>,
    count: usize,
}

struct Held {
    /// Reset and free, graphics then compute.
    free: [Vec<Box<dyn Encoder>>; 2],
    /// Submitted, by the frame value whose point frees them, in growing order.
    pending: VecDeque<(u64, Vec<Box<dyn Encoder>>)>,
}

impl Encoders {
    const fn new() -> Self {
        Self {
            held: SyncView::new(Held { free: [Vec::new(), Vec::new()], pending: VecDeque::new() }),
            count: 0,
        }
    }

    /// A free encoder of `role`'s.
    fn take(&mut self, role: QueueRole) -> Option<Box<dyn Encoder>> {
        let encoder = self.held.get_mut().free[role_index(role)].pop();
        self.count -= usize::from(encoder.is_some());
        encoder
    }

    /// Resets `encoders`, none of which is pending, and frees them.
    ///
    /// A reset the device refuses as `pending` is the graph's own breach of that: a debug
    /// build stops at it. A release build holds such an encoder as pending in the frame of
    /// value `value`, for a later reclaim to reset, rather than drop it: a pending
    /// encoder's drop may wait for its submit, which would block a frame-critical worker.
    fn free_all(&mut self, value: u64, encoders: impl IntoIterator<Item = Box<dyn Encoder>>) {
        let held = self.held.get_mut();
        let mut kept = Vec::new();
        for mut encoder in encoders {
            match encoder.reset() {
                Ok(()) => {
                    held.free[role_index(encoder.queue())].push(encoder);
                    self.count += 1;
                }
                Err(error) => {
                    let pending = Rule::of(&error) == Some(Rule::Pending);
                    debug_assert!(!pending, "the graph freed a pending encoder: {error}");
                    log("an encoder's reset", &error);
                    if pending {
                        kept.push(encoder);
                    }
                }
            }
        }
        if !kept.is_empty() {
            self.pend(value, kept);
        }
    }

    /// Holds `encoders`, submitted in the frame of value `value`, until its point.
    fn pend(&mut self, value: u64, encoders: Vec<Box<dyn Encoder>>) {
        self.count += encoders.len();
        let pending = &mut self.held.get_mut().pending;
        let at = pending.partition_point(|&(held, _)| held <= value);
        pending.insert(at, (value, encoders));
    }

    /// Resets and frees the encoders of every frame whose value is at most `reached`.
    fn reclaim(&mut self, reached: u64) {
        let pending = &mut self.held.get_mut().pending;
        let mut done = Vec::new();
        while pending.front().is_some_and(|&(value, _)| value <= reached) {
            done.push(pending.pop_front().expect("checked above"));
        }
        // Freed after the loop, so that one `free_all` holds again goes to the next call.
        for (value, encoders) in done {
            self.count -= encoders.len();
            self.free_all(value, encoders);
        }
    }
}

/// The index of a queue the graph records for.
const fn role_index(role: QueueRole) -> usize {
    match role {
        QueueRole::Compute => 1,
        QueueRole::Graphics | QueueRole::Present => 0,
    }
}

fn log(what: &str, error: &Error) {
    tracing::warn!(target: LOG_TARGET, "{what} failed: {error}");
}

fn foreign(what: &str) -> Error {
    Error::new(ErrorKind::InvalidData, format!("a {what} another graph made"))
}

/// The label of every encoder and block the graph makes.
const LABEL: &str = "render-graph";

impl RenderGraph {
    /// A graph for `device`, with an empty cache and no encoder. Reads the device's
    /// capabilities, which are fixed, and makes the timelines its submits signal.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` where `desc.jobs` or `desc.plans` is zero; what
    /// `Device::create_timeline` returns.
    pub fn new(device: Arc<dyn Device>, desc: &GraphDesc) -> Result<Self> {
        if desc.jobs == 0 || desc.plans == 0 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "a render graph of no recording job or no plan",
            ));
        }
        let caps = *device.capabilities();
        let frame = device.create_timeline(0)?;
        let queues = if caps.queues.same_queue(QueueRole::Graphics, QueueRole::Compute) {
            None
        } else {
            let made = device.create_timeline(0).and_then(|graphics| {
                device.create_timeline(0).map(|compute| [graphics, compute]).inspect_err(|_| {
                    if let Err(error) = device.destroy_timeline(graphics) {
                        log("a timeline's destroy", &error);
                    }
                })
            });
            match made {
                Ok(made) => Some(made.map(|timeline| QueueTimeline { timeline, value: 0 })),
                Err(error) => {
                    if let Err(error) = device.destroy_timeline(frame) {
                        log("a timeline's destroy", &error);
                    }
                    return Err(error);
                }
            }
        };
        Ok(Self {
            device,
            caps,
            desc: *desc,
            owner: Arc::new(Owner),
            cache: Vec::new(),
            retired: Vec::new(),
            tick: 0,
            compiles: 0,
            hits: 0,
            evictions: 0,
            timelines: Timelines { frame, frame_value: 0, queues },
            previous: None,
            last_frame: None,
            torn: None,
            encoders: Encoders::new(),
        })
    }

    /// The device the graph was made for.
    #[must_use]
    pub fn device(&self) -> &Arc<dyn Device> {
        &self.device
    }

    /// Compiles `decl`, or finds the plan an equal declaration compiled to.
    ///
    /// On a miss it derives the plan and makes the transients a kept pass uses and the
    /// blocks they are placed in; on a hit it makes nothing. Either way it may destroy
    /// the transients of evicted plans that no frame holds and whose last submitted
    /// frame has completed, which it learns by reading a timeline's value and never by
    /// waiting.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData`, naming the pass or the resource by its label, for a
    /// declaration that breaks a rule of `README.md`, "Declaring"; what the device
    /// returns for a transient it cannot make (`Unsupported`, `ResourceExhausted`,
    /// `DeviceLost`). After an error the cache holds no plan for the declaration and the
    /// graph is as it was.
    pub fn compile<'f>(&mut self, decl: FrameDecl<'f>) -> Result<CompiledFrame<'f>> {
        validate(&decl.declared, self.caps.queues)?;
        let shape = Shape::of(&decl.declared, self.caps.queues);
        let key = shape.key();
        let hit = self.cache.iter_mut().find(|entry| entry.key == key && entry.shape == shape);
        let (plan, fresh) = if let Some(entry) = hit {
            self.tick += 1;
            entry.used = self.tick;
            self.hits += 1;
            (Arc::clone(&entry.plan), false)
        } else {
            let plan = Arc::new(self.make_plan(&decl, key)?);
            if self.cache.len() >= self.desc.plans as usize {
                let oldest = (0..self.cache.len())
                    .min_by_key(|&at| self.cache[at].used)
                    .expect("a full cache holds a plan");
                self.retired.push(self.cache.remove(oldest));
                self.evictions += 1;
            }
            self.tick += 1;
            self.cache.push(Entry {
                key,
                shape,
                plan: Arc::clone(&plan),
                used: self.tick,
                last: None,
            });
            self.compiles += 1;
            (plan, true)
        };
        self.reclaim();
        Ok(CompiledFrame { decl, plan, fresh })
    }

    /// Derives the plan of a valid declaration and makes its blocks and transients,
    /// destroying what it made where a call fails.
    fn make_plan(&self, decl: &FrameDecl<'_>, key: GraphKey) -> Result<CompiledPlan> {
        let device = &*self.device;
        let derived = derive(&decl.declared, device, &self.caps, self.desc.jobs)?;
        let mut blocks: Vec<MemoryBlockHandle> = Vec::new();
        let mut transients: Vec<Option<Transient>> = vec![None; derived.transients.len()];
        let made: Result<()> = (|| {
            for &size in &derived.blocks {
                blocks.push(device.create_memory_block(&MemoryBlockDesc { label: LABEL, size })?);
            }
            for (index, transient) in derived.transients.iter().enumerate() {
                let Some((usage, memory)) = *transient else { continue };
                let DeclaredImage::Transient(declared) = &decl.declared.images[index] else {
                    unreachable!("the plan makes transients alone");
                };
                let image = device.create_image(&ImageDesc {
                    label: &declared.label,
                    format: declared.format,
                    extent: declared.extent,
                    mips: declared.mips,
                    layers: declared.layers,
                    usage,
                    memory: match memory {
                        TransientMemory::Own => ImageMemory::Own,
                        TransientMemory::Memoryless => ImageMemory::Memoryless,
                        TransientMemory::Placed { block, offset, .. } => {
                            ImageMemory::Placed { block: blocks[block as usize], offset }
                        }
                    },
                    sharing: Sharing::Exclusive,
                })?;
                transients[index] = Some(Transient { image, memory });
            }
            Ok(())
        })();
        if let Err(error) = made {
            destroy(device, &transients, &blocks);
            return Err(error);
        }
        Ok(CompiledPlan::new(key, Arc::clone(&self.owner), derived.parts, transients, blocks))
    }

    /// Frees the encoders of every frame whose point is seen reached, and destroys the
    /// transients of every evicted plan that no frame holds and whose last frame's point
    /// is: by reading the frame timeline's value, never by waiting.
    fn reclaim(&mut self) {
        let reached = match self.device.timeline_value(self.timelines.frame) {
            Ok(value) => value,
            Err(error) => {
                log("reading the frame timeline", &error);
                return;
            }
        };
        self.encoders.reclaim(reached);
        let device = &*self.device;
        self.retired.retain(|entry| {
            let done = Arc::strong_count(&entry.plan) == 1
                && entry.last.is_none_or(|last| last <= reached);
            if done {
                destroy(device, &entry.plan.transients, &entry.plan.blocks);
            }
            !done
        });
    }

    /// Records `frame`: forks its recording jobs in one scope of `ctx` and returns when
    /// every one has finished.
    ///
    /// Each job owns one encoder and records a run of consecutive passes into it, in
    /// graph order: for each pass, inside one span, the alias calls and transitions the
    /// plan derived, the pass's beginning, its recorder, and its end. The jobs run on the
    /// frame-critical workers, the calling worker among them, and share nothing they
    /// write: each owns its encoder and the recorders of its passes (`README.md`,
    /// "Concurrency"). A recorder does not block or wait: not for another recorder, which
    /// on one worker runs after it, not for a lock and not for a timeline point. A panic
    /// in a recorder aborts the process, as any scope child's does.
    ///
    /// An encoder is taken from the graph's free ones, or made. A pending encoder becomes
    /// free, and is reset, once the point of the frame it was submitted in is seen
    /// reached, which the graph learns by reading the timeline's value and never by
    /// waiting.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` for a frame another graph compiled, for a recorder that
    /// named an id its pass did not declare for that use, and for a copy pass's recorder
    /// that wrote one destination in two calls, naming the pass and the call, the first
    /// such pass in graph order where there are several; what
    /// `Device::create_encoder` returns. After an error nothing of the frame is
    /// submitted or can be, and its encoders are free again.
    pub fn record(
        &mut self,
        frame: CompiledFrame<'_>,
        ctx: &mut JobContext<'_>,
    ) -> Result<RecordedFrame> {
        if !Arc::ptr_eq(&frame.plan.owner, &self.owner) {
            return Err(foreign("frame compiled by"));
        }
        self.reclaim();
        let CompiledFrame { decl, plan, .. } = frame;
        let FrameDecl { declared, recorders } = decl;
        let mut recorders = recorders.into_inner();

        let mut encoders: Vec<Box<dyn Encoder>> = Vec::with_capacity(plan.jobs.len());
        for job in &plan.jobs {
            let free = self.encoders.take(job.queue);
            match free.map_or_else(|| self.device.create_encoder(job.queue, LABEL), Ok) {
                Ok(encoder) => encoders.push(encoder),
                Err(error) => {
                    // Those taken are free ones, untouched since their reset.
                    self.encoders.free_all(self.timelines.frame_value, encoders);
                    return Err(error);
                }
            }
        }

        let images: Vec<Option<ImageHandle>> = declared
            .images
            .iter()
            .zip(&plan.transients)
            .map(|(image, transient)| match image {
                DeclaredImage::Import(import) => Some(import.image),
                DeclaredImage::Transient(_) => transient.map(|transient| transient.image),
            })
            .collect();
        let buffers: Vec<_> = declared.buffers.iter().map(|buffer| buffer.buffer).collect();
        let shared = Shared { declared: &declared, images: &images, buffers: &buffers };
        let mut breaches: Vec<Option<String>> = vec![None; plan.jobs.len()];
        {
            // What each job owns, split by job: its encoder, its breach, and the recorders
            // of its passes, which are consecutive in the declaration.
            let shared = &shared;
            let mut rest: &mut [Option<Recorder<'_>>] = &mut recorders;
            let mut start = 0;
            let mut owned = Vec::with_capacity(plan.jobs.len());
            for ((job, encoder), breach) in plan.jobs.iter().zip(&mut encoders).zip(&mut breaches) {
                let mine: &mut [Option<Recorder<'_>>] = if job.passes.is_empty() {
                    &mut []
                } else {
                    let skipped = job.passes.start - start;
                    let (_, tail) = core::mem::take(&mut rest).split_at_mut(skipped);
                    let (mine, tail) = tail.split_at_mut(job.passes.len());
                    rest = tail;
                    start = job.passes.end;
                    mine
                };
                owned.push((job, Owned { encoder: &mut **encoder, recorders: mine, breach }));
            }
            ctx.scope(|scope| {
                for (job, owned) in owned {
                    scope.fork(move |_| job::run(shared, job, owned));
                }
            });
        }
        drop(recorders);

        if let Some(breach) = breaches.into_iter().flatten().next() {
            self.encoders.free_all(self.timelines.frame_value, encoders);
            return Err(Error::new(ErrorKind::InvalidData, breach));
        }
        let ready = plan
            .submits
            .iter()
            .map(|submit| {
                submit
                    .names
                    .iter()
                    .filter_map(|&resource| match resource {
                        Resource::Image(image) => {
                            declared.images[image as usize].import().and_then(|import| import.ready)
                        }
                        Resource::Buffer(buffer) => declared.buffers[buffer as usize].ready,
                    })
                    .collect()
            })
            .collect();
        Ok(RecordedFrame { plan, encoders: SyncView::new(encoders), ready })
    }

    /// Submits `frame`: one `Device::submit` for each submit of its plan, in order, and
    /// returns the frame's point, which is reached when every one of them has completed.
    ///
    /// Frame points are values of one timeline of the graph's and grow with every frame
    /// submitted, so a point reached means that frame's submits and every earlier
    /// frame's have completed. The call waits for nothing.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` for a frame another graph recorded; otherwise what
    /// `Device::submit` returned, unchanged, so that `Rule::of` names the rule a refusal
    /// broke. An error from the frame's first submit leaves every import as the
    /// declaration found it. An error from a later one leaves the frame torn: the
    /// submits before it were made, the frame's point is not yet signalled, and the
    /// states of the frame's imports are no longer the graph's word. The error does not
    /// say which submit failed, so the caller takes every error as a torn frame: it
    /// imports the frame's imports next from `Undefined`, relies on no buffer's bytes, and
    /// calls [`History::reset`](crate::History::reset) on every history the frame
    /// imported.
    ///
    /// After a torn frame, the next call first makes, in the torn frame's place, its
    /// planned last submit with no encoders: it waits as that submit would have and
    /// signals the torn frame's point. Where that submit is refused, `frame` is not
    /// submitted, its encoders are free, the graph still owes that submit, and the error
    /// is returned unchanged (`README.md`, "Submission").
    pub fn submit(&mut self, frame: RecordedFrame) -> Result<TimelinePoint> {
        if !Arc::ptr_eq(&frame.plan.owner, &self.owner) {
            return Err(foreign("frame recorded by"));
        }
        self.reclaim();
        let RecordedFrame { plan, encoders, ready } = frame;
        let mut encoders = encoders.into_inner();
        if let Some(torn) = self.torn.take()
            && let Err(error) = self.replace(&torn)
        {
            self.torn = Some(torn);
            self.encoders.free_all(self.timelines.frame_value, encoders);
            return Err(error);
        }

        let point =
            TimelinePoint { timeline: self.timelines.frame, value: self.timelines.frame_value + 1 };
        let last = plan.submits.len() - 1;
        let mut points: Vec<Option<TimelinePoint>> = Vec::with_capacity(plan.submits.len());
        let mut made: Vec<Box<dyn Encoder>> = Vec::with_capacity(encoders.len());
        // The last submit made to each queue before this frame. Each submit waits for the
        // other queue's, which covers every earlier frame's use there, and the frame's last
        // also for the frame point before where that was signalled from the other queue,
        // so that the frame timeline's values grow across a wait.
        let before: Option<[TimelinePoint; 2]> = self.timelines.queues.as_ref().map(|queues| {
            queues
                .each_ref()
                .map(|queue| TimelinePoint { timeline: queue.timeline, value: queue.value })
        });
        for (number, submit) in plan.submits.iter().enumerate() {
            let mut waits: Vec<TimelinePoint> = Vec::new();
            if let Some(before) = before {
                let at = role_index(submit.queue);
                waits.extend(Some(before[1 - at]).filter(|other| other.value > 0));
                // Not implied by the wait above, though one submit signalled both points:
                // the signals of one submit are unordered with one another on Vulkan, so
                // only a wait for the frame point itself orders this frame's after it
                // (VUID-VkSubmitInfo2-semaphore-03882). Null refuses the signal without it
                // while the frame before is pending (`timeline`, rhi-api's "Timelines").
                if number == last
                    && let Some((previous, queue)) = self.previous
                    && role_index(queue) != at
                {
                    waits.push(previous);
                }
            }
            waits.extend(submit.waits.iter().filter_map(|&earlier| points[earlier as usize]));
            waits.extend(ready[number].iter().copied());
            let waits = latest(waits);
            let queue_point = self.timelines.queues.as_ref().map(|queues| {
                let queue = &queues[role_index(submit.queue)];
                TimelinePoint { timeline: queue.timeline, value: queue.value + 1 }
            });
            let signals: Vec<TimelinePoint> =
                queue_point.into_iter().chain((number == last).then_some(point)).collect();
            let mut chunk: Vec<Box<dyn Encoder>> = encoders.drain(..submit.jobs.len()).collect();
            let result = self.device.submit(
                submit.queue,
                Submit { encoders: &mut chunk, waits: &waits, signals: &signals },
            );
            if let Err(error) = result {
                // The refused submit's encoders are not pending, nor are those of the
                // submits not made.
                self.encoders
                    .free_all(self.timelines.frame_value, chunk.into_iter().chain(encoders));
                if number > 0 {
                    // Torn: the made submits' encoders and the plan are held until the
                    // point the next call signals in the frame's place.
                    self.timelines.frame_value = point.value;
                    self.torn = Some(Torn { point, queue: plan.submits[last].queue });
                    self.encoders.pend(point.value, made);
                    self.note_submitted(&plan, point.value);
                }
                return Err(error);
            }
            if let (Some(queues), Some(signalled)) = (self.timelines.queues.as_mut(), queue_point) {
                queues[role_index(submit.queue)].value = signalled.value;
            }
            points.push(queue_point);
            made.append(&mut chunk);
        }
        self.timelines.frame_value = point.value;
        self.previous = Some((point, plan.submits[last].queue));
        self.last_frame = Some(point);
        self.encoders.pend(point.value, made);
        self.note_submitted(&plan, point.value);
        Ok(point)
    }

    /// Makes the submit in a torn frame's place: no encoders, the waits the torn frame's
    /// last submit would have made, and its point.
    fn replace(&mut self, torn: &Torn) -> Result<()> {
        let waits = self.replacement_waits(torn.queue);
        let signals = [torn.point];
        self.device
            .submit(torn.queue, Submit { encoders: &mut [], waits: &waits, signals: &signals })?;
        self.previous = Some((torn.point, torn.queue));
        Ok(())
    }

    /// What a frame's last submit to `queue` waits for beside its frame's own submits:
    /// the last submit made to the other queue and the frame point before, the second for
    /// the reason `submit` gives at its own wait for it.
    fn replacement_waits(&self, queue: QueueRole) -> Vec<TimelinePoint> {
        let Some(queues) = &self.timelines.queues else { return Vec::new() };
        let other = &queues[1 - role_index(queue)];
        let made = (other.value > 0)
            .then_some(TimelinePoint { timeline: other.timeline, value: other.value });
        latest(made.into_iter().chain(self.previous.map(|(previous, _)| previous)).collect())
    }

    /// Notes that a frame of `plan` was submitted under the frame value `value`.
    fn note_submitted(&mut self, plan: &Arc<CompiledPlan>, value: u64) {
        let entry = self
            .cache
            .iter_mut()
            .chain(&mut self.retired)
            .find(|entry| Arc::ptr_eq(&entry.plan, plan))
            .expect("a frame's plan is cached or retired while the frame holds it");
        entry.last = Some(value);
    }

    /// The point of the last frame [`submit`](Self::submit) returned, or `None` before
    /// the first.
    #[must_use]
    pub fn last_frame(&self) -> Option<TimelinePoint> {
        self.last_frame
    }

    /// The graph's counts.
    #[must_use]
    pub fn stats(&self) -> GraphStats {
        GraphStats {
            compiles: self.compiles,
            hits: self.hits,
            evictions: self.evictions,
            plans: crate::plan::count(self.cache.len()),
            encoders: u32::try_from(self.encoders.count).unwrap_or(u32::MAX),
        }
    }

    /// The drop of a thread that is not unwinding: every submit waited for, then every
    /// object destroyed.
    fn drop_waiting(&mut self) {
        let mut waits: Vec<TimelinePoint> = Vec::new();
        if let Some(torn) = self.torn.take()
            && let Err(error) = self.replace(&torn)
        {
            log("the submit in a torn frame's place", &error);
            // Every other point the graph's made submits signal.
            if let Some(queues) = &self.timelines.queues {
                waits.extend(
                    queues.iter().filter(|queue| queue.value > 0).map(|queue| TimelinePoint {
                        timeline: queue.timeline,
                        value: queue.value,
                    }),
                );
            }
        }
        // The last frame point, or the one just signalled in a torn frame's place.
        waits.extend(self.previous.map(|(previous, _)| previous));
        for point in waits {
            if let Err(error) = self.device.wait_timeline(point, Duration::MAX) {
                log("the wait for the graph's submits", &error);
            }
        }
        drop(core::mem::replace(&mut self.encoders, Encoders::new()));
        for entry in self.cache.drain(..).chain(self.retired.drain(..)) {
            destroy(&*self.device, &entry.plan.transients, &entry.plan.blocks);
        }
        for timeline in self.timeline_handles() {
            if let Err(error) = self.device.destroy_timeline(timeline) {
                log("a timeline's destroy", &error);
            }
        }
    }

    /// The drop of a thread that is unwinding: no submit, no wait, and no destroy a
    /// pending submit would refuse.
    fn drop_unwinding(&mut self) {
        let device = &*self.device;
        let Ok(reached) = device.timeline_value(self.timelines.frame) else { return };
        for entry in self.cache.drain(..).chain(self.retired.drain(..)) {
            if entry.last.is_none_or(|last| last <= reached) {
                destroy(device, &entry.plan.transients, &entry.plan.blocks);
            }
        }
        let queues_done = self.timelines.queues.as_ref().is_none_or(|queues| {
            queues.iter().all(|queue| {
                device.timeline_value(queue.timeline).is_ok_and(|value| value >= queue.value)
            })
        });
        if self.torn.is_none() && reached >= self.timelines.frame_value && queues_done {
            for timeline in self.timeline_handles() {
                if let Err(error) = device.destroy_timeline(timeline) {
                    log("a timeline's destroy", &error);
                }
            }
        }
    }

    fn timeline_handles(&self) -> Vec<TimelineHandle> {
        let queues = self.timelines.queues.iter().flatten().map(|queue| queue.timeline);
        core::iter::once(self.timelines.frame).chain(queues).collect()
    }
}

/// The greatest point of each timeline among `points`, in the order the timelines first
/// occur.
fn latest(points: Vec<TimelinePoint>) -> Vec<TimelinePoint> {
    let mut kept: Vec<TimelinePoint> = Vec::with_capacity(points.len());
    for point in points {
        match kept.iter_mut().find(|kept| kept.timeline == point.timeline) {
            Some(kept) => kept.value = kept.value.max(point.value),
            None => kept.push(point),
        }
    }
    kept
}

/// Destroys a plan's transients, then its blocks, logging what fails.
fn destroy(device: &dyn Device, transients: &[Option<Transient>], blocks: &[MemoryBlockHandle]) {
    for transient in transients.iter().flatten() {
        if let Err(error) = device.destroy_image(transient.image) {
            log("a transient's destroy", &error);
        }
    }
    for &block in blocks {
        if let Err(error) = device.destroy_memory_block(block) {
            log("a memory block's destroy", &error);
        }
    }
}

impl Drop for RenderGraph {
    /// Waits until every submit the graph made has completed, then destroys what the
    /// graph made. An error from a destroy is logged under [`LOG_TARGET`] and dropped. On
    /// a lost device the wait returns at once. After a torn frame with no `submit`
    /// since, it first makes the submit in the torn frame's place, as `submit` would.
    /// Where that is refused, the error is logged; the drop then waits for every point
    /// it can see signalled, drops its encoders, which waits for their submits unless
    /// the device is lost, and destroys what no pending submit uses, which on null
    /// leaks what the torn frame used (`README.md`, "Errors and drops"). While the
    /// thread unwinds a panic it makes no submit and waits for nothing:
    /// it makes no destroy a pending submit would refuse, and its encoders' drops leak
    /// what is pending.
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.drop_unwinding();
        } else {
            self.drop_waiting();
        }
    }
}

/// A frame recorded and not yet submitted: its encoders, each holding the commands of one
/// recording job, and its plan.
///
/// `Send + Sync + 'static` and borrowing nothing, so that it waits in a world's resource
/// while the frame latches. It holds no recorder: each was called, or dropped, before
/// `record` returned. Dropped unsubmitted, it drops its encoders, none of which is
/// pending, and the graph makes others.
pub struct RecordedFrame {
    /// The plan the frame was recorded from.
    plan: Arc<CompiledPlan>,
    /// The frame's encoders, in the order its submits take them.
    encoders: SyncView<Vec<Box<dyn Encoder>>>,
    /// For each submit, the `ready` points of the imports it names.
    ready: Vec<Vec<TimelinePoint>>,
}
