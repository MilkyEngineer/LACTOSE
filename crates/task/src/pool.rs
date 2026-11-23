//! Worker pools: deque entries, the worker loop, stealing, idling and the `block_in_place`
//! gate.

use core::cell::{Cell, OnceCell, RefCell};
use core::ops::Range;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::Thread;
use std::time::{Duration, Instant};

use crate::deque::{Injector, Steal, Stealer, Worker};
use lactose_core::Arena;

use crate::chaos::{Chaos, ChaosConfig, ChaosRng, Filter, Wake};
use crate::gate::{self, Gate, Look, Opener};
use crate::graph::{self, RunState};
use crate::guard::abort_on_unwind;
use crate::job::{self, ChildHeader, ScopeCore};
use crate::job_run::JobEntry;
use crate::latch::Latch;
use crate::platform::{Platform, ThreadHandle};
use crate::shutdown::ShutdownFlag;
use crate::sleep::Sleep;
use crate::task::RawTask;
use crate::time::Timers;
use crate::width::{Place, Queued, Width};
use crate::{WorkerClass, lock};

/// How long a worker that found nothing keeps rescanning the queues before it parks.
///
/// Long enough to catch work that follows at once, short enough that a dry worker costs
/// next to nothing. Scanning for longer while a graph run is in flight was measured (one
/// worker at a time, 300 µs) and bought no makespan on any benchmark shape while costing
/// up to a third more CPU on the narrow ones, so every dry worker parks after this.
const SCAN_IDLE: Duration = Duration::from_micros(50);

/// How long a replay may sit with pooled work, nothing running and the log's next job
/// unavailable before it is declared unsatisfiable.
const REPLAY_STALL: Duration = Duration::from_secs(2);

/// Under Miri time is simulated, so the scan phase is a fixed number of rounds instead.
const MIRI_SCAN_ROUNDS: u32 = 2;

/// How long the opener of a scope spins on its latch before parking.
pub(crate) const LATCH_SPIN: Duration = Duration::from_micros(if cfg!(miri) { 0 } else { 20 });

/// One unit of schedulable work. Entries are held by value in deques: nothing about a stolen
/// or split range lives in any worker's arena.
pub(crate) enum Entry {
    /// Units of a graph node. An empty range stands for "this node only needs finalizing":
    /// a node with no units, or in replay the finalize job of a node whose units are done.
    Range {
        run: Arc<RunState>,
        node: u32,
        units: Range<u32>,
        /// Whether this is the entry the node was released as, which gets the per-node
        /// span, or a part split off it, which gets the per-range span.
        root: bool,
    },
    /// Units of a scope child. `scope` is the tag an opener compares when it pops.
    Child { scope: NonNull<ScopeCore>, job: NonNull<ChildHeader>, units: Range<u32> },
    /// A stackless task that is ready to be polled.
    Task(Arc<dyn RawTask>),
    /// A job of [`crate::Executor::run_job`], not started.
    Job(JobEntry),
}

// SAFETY: the pointers of a `Child` entry refer to a scope and to job storage that stay alive
// until the scope's latch has counted the entry's units down (`job.rs`), and the job behind
// them is `Send` (and `Sync` when it can run on several workers at once), as `Scope::fork`
// and `Scope::fork_range` require. The other variants are `Send` on their own.
unsafe impl Send for Entry {}

impl Entry {
    /// The same work over a different range.
    pub(crate) fn with_units(&self, units: Range<u32>) -> Self {
        match self {
            Self::Range { run, node, .. } => {
                Self::Range { run: run.clone(), node: *node, units, root: false }
            }
            Self::Child { scope, job, .. } => Self::Child { scope: *scope, job: *job, units },
            Self::Task(_) | Self::Job(_) => unreachable!("a task or a job has no units"),
        }
    }

    /// This entry narrowed to `units`.
    pub(crate) fn into_units(self, units: Range<u32>) -> Self {
        match self {
            Self::Range { run, node, root, .. } => Self::Range { run, node, units, root },
            Self::Child { scope, job, .. } => Self::Child { scope, job, units },
            Self::Task(_) => unreachable!("a task has no units"),
            // One entry, never split: its one unit is the whole job.
            job @ Self::Job(_) => {
                debug_assert_eq!(units, 0..1, "a job is one unit");
                job
            }
        }
    }

    /// Splits the upper half off a range of more than one unit.
    fn split_upper(&mut self) -> Option<Self> {
        let upper = match self {
            Self::Range { units, .. } | Self::Child { units, .. } => {
                if units.end - units.start < 2 {
                    return None;
                }
                let middle = units.start + (units.end - units.start) / 2;
                let upper = middle..units.end;
                units.end = middle;
                upper
            }
            Self::Task(_) | Self::Job(_) => return None,
        };
        Some(self.with_units(upper))
    }

    /// Gives up on an entry that will never run. A task is cancelled and a job dropped
    /// unrun, which resolves its future to `Cancelled`; graph work is dropped, which leaves
    /// its run unfinished for good. Never called on a child, whose opener would wait forever.
    fn discard(self) {
        match self {
            Self::Task(task) => task.run(true),
            Self::Range { .. } | Self::Job(_) => {}
            Self::Child { .. } => unreachable!("scope children are always run"),
        }
    }
}

/// The per-worker bump arenas (README, "Scopes").
#[derive(Default)]
pub(crate) struct Arenas {
    /// Scratch of the graph job this worker is running. Reset after each job.
    pub(crate) job: Arena,
    /// Scratch of the scope child this worker is running, which may be a child the worker's
    /// own suspended job forked. Reset after each child.
    pub(crate) child: Arena,
    /// Storage for the children of the scope this worker's job has open. Reset when the
    /// scope closes.
    pub(crate) scope: Arena,
}

/// What a worker thread knows about itself. Lives in a thread-local so that code running on
/// the worker (a waker, `run_graph` called from a task) can reach its deque.
pub(crate) struct WorkerLocal {
    pub(crate) shared: Arc<Shared>,
    pub(crate) class: WorkerClass,
    pub(crate) index: usize,
    pub(crate) deque: Worker<Entry>,
    /// Whether this worker's job has a scope open. While it does, the worker's deque takes
    /// only that scope's children (see [`Shared::submit_range`]).
    pub(crate) scope_open: Cell<bool>,
    /// Whether this worker is running a job of [`crate::Executor::run_job`]. While it is, the
    /// children of the job's scopes wait on the pool's queue for such work ([`Wide`]).
    pub(crate) wide_job: Cell<bool>,
    /// Set while this worker fires due timers ([`fire_timers`]) and none of the tasks they
    /// wake has been kept yet.
    keep_woken_task: Cell<bool>,
    /// Seeded perturbation, when chaos is configured.
    chaos: Option<RefCell<ChaosRng>>,
    /// Round-robin start for victim selection when chaos does not shuffle it.
    next_victim: Cell<usize>,
}

thread_local! {
    static LOCAL: OnceCell<WorkerLocal> = const { OnceCell::new() };
}

/// Calls `f` with the calling thread's worker state, or `None` on any other thread.
fn with_local<R>(f: impl FnOnce(Option<&WorkerLocal>) -> R) -> R {
    let mut f = Some(f);
    let on_worker = LOCAL.try_with(|cell| (f.take().expect("called once"))(cell.get()));
    match on_worker {
        Ok(result) => result,
        // The thread is being torn down and its local is gone.
        Err(_) => (f.take().expect("called once"))(None),
    }
}

/// Whether the calling thread is a worker of any executor.
pub(crate) fn on_worker_thread() -> bool {
    with_local(|local| local.is_some())
}

/// Whether the calling thread is one of `shared`'s own workers, which cannot join them.
pub(crate) fn on_worker_of(shared: &Shared) -> bool {
    with_local(|local| local.is_some_and(|local| core::ptr::eq(Arc::as_ptr(&local.shared), shared)))
}

impl WorkerLocal {
    pub(crate) fn pool(&self) -> &Pool {
        self.shared.pool(self.class)
    }

    pub(crate) fn chaos_config(&self) -> Option<&ChaosConfig> {
        self.shared.chaos.as_ref().map(|chaos| &chaos.config)
    }

    /// Pushes work this worker produced onto its own deque and tells a sleeper.
    pub(crate) fn push(&self, entry: Entry) {
        self.deque.push(entry);
        self.pool().notify_one();
    }
}

/// One class of workers.
pub(crate) struct Pool {
    injector: Injector<Entry>,
    stealers: Box<[Stealer<Entry>]>,
    threads: Box<[Thread]>,
    sleep: Sleep,
    /// Run permits and the `block_in_place` gate.
    gate: Gate,
    /// How many openers are parked on their scope's latch, for the tests that need one to
    /// be.
    #[cfg(all(test, not(feature = "loom")))]
    parked_openers: core::sync::atomic::AtomicU32,
}

impl Pool {
    /// A pool of one worker per stealer. Workers that `keep_timers`, the frame-critical
    /// pool's, get a timed wait each ([`Sleep::with_timed_parks`]); it is closed when the
    /// pool is dropped.
    fn new(stealers: Vec<Stealer<Entry>>, threads: Vec<Thread>, keep_timers: bool) -> Self {
        let workers = stealers.len();
        Self {
            injector: Injector::new(),
            sleep: if keep_timers { Sleep::with_timed_parks(workers) } else { Sleep::new(workers) },
            stealers: stealers.into(),
            threads: threads.into(),
            gate: Gate::new(workers),
            #[cfg(all(test, not(feature = "loom")))]
            parked_openers: core::sync::atomic::AtomicU32::new(0),
        }
    }

    pub(crate) fn workers(&self) -> usize {
        self.stealers.len()
    }

    /// How many of this pool's workers are asleep in the idle protocol, or about to be,
    /// and not yet claimed by a wake-up.
    #[cfg(all(test, not(feature = "loom")))]
    pub(crate) fn sleepers(&self) -> u32 {
        self.sleep.announced()
    }

    /// How many openers are parked on their scope's latch.
    #[cfg(all(test, not(feature = "loom")))]
    pub(crate) fn parked_openers(&self) -> u32 {
        self.parked_openers.load(Ordering::SeqCst)
    }

    /// Parks an opener on its scope's latch ([`Opener::park`]).
    pub(crate) fn park_opener(&self, latch: &Latch) {
        #[cfg(all(test, not(feature = "loom")))]
        self.parked_openers.fetch_add(1, Ordering::SeqCst);
        latch.park();
        #[cfg(all(test, not(feature = "loom")))]
        self.parked_openers.fetch_sub(1, Ordering::SeqCst);
    }

    /// Wakes one sleeping worker, if any. Call after making work visible.
    pub(crate) fn notify_one(&self) {
        self.announce_entry(false);
    }

    /// Wakes one sleeping worker for an entry that was just queued, unless the caller
    /// `kept` it (see [`Sleep::announce_entry`]).
    fn announce_entry(&self, kept: bool) {
        if let Some(worker) = self.sleep.announce_entry(kept) {
            self.wake_worker(worker);
        }
    }

    /// Wakes `worker` whether it sleeps in the idle protocol or on a scope latch.
    ///
    /// A worker this does not claim is awake, parked on a latch, which waits untimed, or
    /// claimed by another waker. It is woken all the same, timed wait included, so that it
    /// does not wait for the other waker to get round to it.
    pub(crate) fn wake(&self, worker: usize) {
        let _ = self.sleep.notify(worker);
        self.wake_worker(worker);
    }

    /// Wakes `worker`, timed wait included ([`Sleep::wake_worker`]). Every wake of a worker
    /// that may be in the idle protocol goes through this, once per wake: after a claim of
    /// that one worker, or, in `open_gate` and `Shared::shut_down`, for every worker after
    /// a claim of every sleeper. A spare one costs a trip round [`Sleep::park`].
    fn wake_worker(&self, worker: usize) {
        self.sleep.wake_worker(worker, || self.threads[worker].unpark());
    }

    /// Unparks `worker`, which waits on a scope latch that has just been set.
    pub(crate) fn unpark(&self, worker: usize) {
        self.threads[worker].unpark();
    }

    fn work_visible(&self) -> bool {
        !self.injector.is_empty() || self.stealers.iter().any(|stealer| !stealer.is_empty())
    }

    fn try_acquire(&self) -> bool {
        self.gate.try_acquire()
    }

    fn release(&self) {
        self.gate.release(self);
    }

    fn release_if_over(&self) -> bool {
        self.gate.release_if_over()
    }

    /// Whether a `block_in_place` has lowered the limit below the number of permits out,
    /// so that some worker has to stop taking units.
    pub(crate) fn over_limit(&self) -> bool {
        self.gate.over_limit()
    }

    /// Starts a `block_in_place`: at most `reserved` workers other than the caller may hold
    /// permits. Workers over the limit give theirs up at their next unit boundary.
    pub(crate) fn close_gate(&self, reserved: u32) {
        self.gate.close(reserved as usize, self);
    }

    /// Ends a `block_in_place` started with the same `reserved`.
    pub(crate) fn open_gate(&self, reserved: u32) {
        self.gate.open(reserved as usize, self);
        // Claims every sleeper, then wakes every worker once: the sleepers just claimed,
        // openers that parked on their latch with nothing on their deque and announced
        // nothing, and workers another waker has claimed but may not have woken yet.
        self.sleep.notify_all(|_| {});
        (0..self.threads.len()).for_each(|worker| self.wake_worker(worker));
    }

    /// The wait of a scope's opener on this pool. See [`gate::wait_for_children`].
    pub(crate) fn wait_for_children(&self, latch: &Latch, worker: usize, opener: &mut impl Opener) {
        gate::wait_for_children(latch, &self.gate, worker, self, opener);
    }
}

impl gate::Wake for Pool {
    fn idle_one(&self) {
        self.notify_one();
    }

    fn unpark(&self, worker: usize) {
        self.threads[worker].unpark();
    }
}

/// Finishes a steal: if the stolen entry is a range of more than one unit, its upper half
/// goes onto the thief's own deque, where other thieves find it.
fn split_stolen(mut entry: Entry, thief: &Worker<Entry>, pool: &Pool) -> Entry {
    if let Some(upper) = entry.split_upper() {
        thief.push(upper);
        pool.notify_one();
    }
    entry
}

/// One steal from `victim`, including the thief-side split.
fn steal_from(victim: &Stealer<Entry>, thief: &Worker<Entry>, pool: &Pool) -> Steal<Entry> {
    match victim.steal() {
        Steal::Success(entry) => Steal::Success(split_stolen(entry, thief, pool)),
        other => other,
    }
}

/// Times `samples` steals, each from a deque another thread filled, through
/// [`steal_from`]: the function a worker steals with. For the benchmark only.
pub(crate) fn time_steals(
    platform: &dyn Platform,
    samples: usize,
) -> lactose_core::Result<Vec<u64>> {
    const BATCH: usize = 1024;
    let run = graph::placeholder_run();
    let victim = Worker::<Entry>::new_lifo();
    let thief = Worker::<Entry>::new_lifo();
    let stealer = victim.stealer();
    let pool = Pool::new(
        vec![victim.stealer(), thief.stealer()],
        vec![std::thread::current(), std::thread::current()],
        false,
    );
    let (filled, wait_filled) = std::sync::mpsc::channel::<usize>();
    let (stolen, wait_stolen) = std::sync::mpsc::channel::<Vec<u64>>();
    let spec = crate::ThreadSpec {
        name: "whisk-bench-thief".to_owned(),
        class: lactose_core::ThreadClass::FrameCritical,
        index_in_class: 1,
    };
    let body = Box::new(move || {
        while let Ok(count) = wait_filled.recv() {
            let mut nanos = Vec::with_capacity(count);
            for _ in 0..count {
                let start = Instant::now();
                let entry = loop {
                    match steal_from(&stealer, &thief, &pool) {
                        Steal::Success(entry) => break entry,
                        Steal::Retry => {}
                        Steal::Empty => unreachable!("the victim holds a full batch"),
                    }
                };
                nanos.push(start.elapsed().as_nanos() as u64);
                drop(entry);
            }
            while thief.pop().is_some() {}
            if stolen.send(nanos).is_err() {
                break;
            }
        }
    });
    let thread = platform.spawn_thread(spec, body)?;
    let mut nanos = Vec::with_capacity(samples);
    while nanos.len() < samples {
        let count = BATCH.min(samples - nanos.len());
        for index in 0..count as u32 {
            let units = index * 64..index * 64 + 64;
            victim.push(Entry::Range { run: run.clone(), node: 0, units, root: false });
        }
        filled.send(count).expect("the thief is running");
        nanos.extend(wait_stolen.recv().expect("the thief reports"));
    }
    drop(filled);
    thread.join();
    Ok(nanos)
}

/// The frame-critical pool's queues for the jobs of [`crate::Executor::run_job`] and for the
/// children of their scopes, and the width that bounds the workers such work occupies
/// (`width.rs`). Nothing else waits here, and nothing of such work waits anywhere else
/// outside a replay.
pub(crate) struct Wide {
    pub(crate) width: Width,
    jobs: Injector<Entry>,
    /// Children of the one scope of such a job that is open, whole, as halves split off or
    /// as the rest a worker put back.
    children: Injector<Entry>,
}

impl Wide {
    fn new(workers: usize) -> Self {
        Self { width: Width::new(workers), jobs: Injector::new(), children: Injector::new() }
    }

    fn take(queue: &Injector<Entry>) -> Option<Entry> {
        loop {
            match queue.steal() {
                Steal::Success(entry) => return Some(entry),
                Steal::Empty => return None,
                Steal::Retry => {}
            }
        }
    }

    /// Takes a child under the opener's place.
    pub(crate) fn take_child(&self) -> Option<Entry> {
        Self::take(&self.children)
    }

    pub(crate) fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

impl Queued for Wide {
    fn jobs(&self) -> bool {
        !self.jobs.is_empty()
    }

    fn children(&self) -> bool {
        !self.children.is_empty()
    }
}

/// Everything the workers of one executor share.
pub(crate) struct Shared {
    frame: Pool,
    background: Pool,
    pub(crate) platform: Arc<dyn Platform>,
    pub(crate) shutdown: ShutdownFlag,
    pub(crate) timers: Timers,
    pub(crate) chaos: Option<Chaos>,
    /// The jobs of `run_job` and their scopes' children.
    pub(crate) wide: Wide,
    /// Graph runs and `run_job` jobs started so far; a run's ordinal identifies it in an
    /// execution log.
    runs: AtomicU64,
    /// Graph runs that have started and not finished, so shutdown can resolve their
    /// futures.
    live_runs: Mutex<HashMap<u64, Weak<RunState>>>,
    /// Every unfinished task, so shutdown can cancel the ones no queue holds.
    tasks: Mutex<HashMap<u64, Weak<dyn RawTask>>>,
    next_task: AtomicU64,
    handles: Mutex<Vec<ThreadHandle>>,
}

/// What a worker thread needs before it can enter its loop.
pub(crate) struct WorkerSeed {
    pub(crate) class: WorkerClass,
    pub(crate) index: usize,
    pub(crate) deque: Worker<Entry>,
    /// Sends the worker's thread handle to the builder.
    pub(crate) thread: Sender<Thread>,
    /// Receives the shared state once every worker has reported in. A closed channel means
    /// the build failed and the worker exits.
    pub(crate) shared: Receiver<Arc<Shared>>,
}

impl Shared {
    pub(crate) fn new(
        frame: (Vec<Stealer<Entry>>, Vec<Thread>),
        background: (Vec<Stealer<Entry>>, Vec<Thread>),
        platform: Arc<dyn Platform>,
        chaos: Option<Chaos>,
        handles: Vec<ThreadHandle>,
    ) -> Self {
        Self {
            wide: Wide::new(frame.0.len()),
            // Only the frame-critical pool keeps the timers (`idle`).
            frame: Pool::new(frame.0, frame.1, true),
            background: Pool::new(background.0, background.1, false),
            platform,
            shutdown: ShutdownFlag::new(),
            timers: Timers::new(),
            chaos,
            runs: AtomicU64::new(0),
            live_runs: Mutex::new(HashMap::new()),
            tasks: Mutex::new(HashMap::new()),
            next_task: AtomicU64::new(0),
            handles: Mutex::new(handles),
        }
    }

    pub(crate) fn pool(&self, class: WorkerClass) -> &Pool {
        match class {
            WorkerClass::FrameCritical => &self.frame,
            WorkerClass::Background => &self.background,
        }
    }

    pub(crate) fn frame(&self) -> &Pool {
        &self.frame
    }

    pub(crate) fn is_shut_down(&self) -> bool {
        self.shutdown.is_set()
    }

    /// Notes the start of a graph run and returns its ordinal.
    pub(crate) fn begin_run(&self, run: &Arc<RunState>) -> u64 {
        let ordinal = self.next_run_ordinal();
        lock(&self.live_runs).insert(ordinal, Arc::downgrade(run));
        ordinal
    }

    /// The ordinal of the next graph run or `run_job` job, in the order the program starts
    /// them.
    pub(crate) fn next_run_ordinal(&self) -> u64 {
        self.runs.fetch_add(1, Ordering::Relaxed)
    }

    /// Queues a job of `run_job`. One that finds the executor shut down is dropped here,
    /// which resolves its future to `Cancelled`.
    pub(crate) fn submit_job(&self, job: JobEntry) {
        self.shutdown.queue(
            job,
            drop,
            |job| {
                let entry = Entry::Job(job);
                if let Some(replay) = self.replay() {
                    self.replay_wake(replay.push(entry));
                } else {
                    self.wide.jobs.push(entry);
                    self.wide.width.queued(Place::Job, None, &self.frame);
                }
            },
            || self.discard_queued(),
        );
    }

    /// Puts back a job a worker took while a `block_in_place` wanted it parked. The worker
    /// gives its place up next, which wakes a worker that may take it.
    fn requeue_job(&self, job: JobEntry) {
        self.wide.jobs.push(Entry::Job(job));
        self.discard_if_shut_down();
    }

    /// Queues a child of a scope of a `run_job` job. `opener` is the scope's opener when the
    /// caller is another worker: it takes its children from this queue and is told.
    pub(crate) fn submit_wide_child(&self, entry: Entry, opener: Option<usize>) {
        self.wide.children.push(entry);
        self.wide.width.queued(Place::Child, opener, &self.frame);
    }

    /// Takes a child or a job of `run_job` for a worker that holds no place, with the place
    /// it took. Children come first: they belong to the one job that has started.
    fn take_wide(&self) -> Option<(Entry, Place)> {
        let wide = &self.wide;
        for (queue, place) in [(&wide.children, Place::Child), (&wide.jobs, Place::Job)] {
            if queue.is_empty() || !wide.width.try_enter(place) {
                continue;
            }
            if let Some(entry) = Wide::take(queue) {
                return Some((entry, place));
            }
            wide.width.leave(place, wide, &self.frame);
        }
        None
    }

    /// How many graph runs are registered as unfinished.
    #[cfg(all(test, not(feature = "loom")))]
    pub(crate) fn live_run_count(&self) -> usize {
        lock(&self.live_runs).len()
    }

    pub(crate) fn end_run(&self, ordinal: u64) {
        lock(&self.live_runs).remove(&ordinal);
    }

    /// The replay state, when this executor replays a log.
    pub(crate) fn replay(&self) -> Option<&Chaos> {
        self.chaos.as_ref().filter(|chaos| chaos.replaying)
    }

    pub(crate) fn replay_wake(&self, wake: Wake) {
        match wake {
            Wake::Nobody => {}
            Wake::Worker(worker) => self.frame.wake(worker),
            Wake::Anyone => self.frame.notify_one(),
        }
    }

    /// Queues graph work: on the calling worker's deque when the caller is one of this
    /// executor's frame-critical workers and has no scope open, on the injector otherwise.
    ///
    /// The scope rule is what lets an opener find its children: while its scope is open,
    /// nothing but that scope's children is pushed onto its deque, so they are always the
    /// topmost entries. A graph started from a scope body or from a child the opener runs
    /// itself therefore goes to the injector.
    ///
    /// `keeps_one` says the caller is a worker at the end of its job that is releasing
    /// nodes and will take one of them itself as its next entry. An entry that lands on its
    /// own deque then wakes nobody: a sleeper woken for it would find it gone.
    pub(crate) fn submit_range(&self, entry: Entry, keeps_one: bool) {
        self.shutdown.queue(
            entry,
            drop,
            |entry| self.push_range(entry, keeps_one),
            || {
                self.discard_queued();
            },
        );
    }

    fn push_range(&self, entry: Entry, keeps_one: bool) {
        if let Some(replay) = self.replay() {
            self.replay_wake(replay.push(entry));
            return;
        }
        let entry = with_local(|local| match local {
            Some(local)
                if core::ptr::eq(Arc::as_ptr(&local.shared), self)
                    && local.class == WorkerClass::FrameCritical
                    && !local.scope_open.get() =>
            {
                local.deque.push(entry);
                None
            }
            _ => Some(entry),
        });
        let on_own_deque = entry.is_none();
        if let Some(entry) = entry {
            self.frame.injector.push(entry);
        }
        self.frame.announce_entry(keeps_one && on_own_deque);
    }

    /// Closes the race between a submission and shutdown for an entry queued without
    /// [`ShutdownFlag::queue`]: either shutdown's own sweep comes after the entry and finds
    /// it, or this sees the flag and sweeps.
    fn discard_if_shut_down(&self) {
        if self.shutdown.seen_after_queueing() {
            self.discard_queued();
        }
    }

    /// Cancels every queued task and job and drops queued graph work. Safe while workers
    /// still run: scope children are never touched.
    fn discard_queued(&self) {
        self.shutdown.before_sweep();
        if let Some(chaos) = &self.chaos {
            chaos.discard_graph_work();
        }
        for queue in [&self.frame.injector, &self.background.injector, &self.wide.jobs] {
            while let Some(entry) = Wide::take(queue) {
                entry.discard();
            }
        }
    }

    /// Queues a ready task behind the other ready tasks of its class and wakes a sleeper for
    /// it. The exception is the first frame-critical task woken by the timers a worker fires
    /// on its way to its next job ([`fire_timers`]) while nothing waits in the frame-critical
    /// queue: it goes onto that worker's own deque, to be its next job, and wakes nobody.
    pub(crate) fn submit_task(&self, class: WorkerClass, task: Arc<dyn RawTask>) {
        self.shutdown.queue(
            task,
            |task| task.run(true),
            |task| {
                let pool = self.pool(class);
                let entry = Entry::Task(task);
                let entry = match class {
                    WorkerClass::FrameCritical => self.keep_woken_task(entry),
                    WorkerClass::Background => Some(entry),
                };
                let kept = entry.is_none();
                if let Some(entry) = entry {
                    pool.injector.push(entry);
                }
                pool.announce_entry(kept);
            },
            || self.discard_queued(),
        );
    }

    /// Pushes a frame-critical task onto the calling worker's own deque if that worker is
    /// firing timers, has not kept a task yet and finds the frame-critical queue empty;
    /// otherwise hands it back.
    ///
    /// The empty queue is what keeps the hand-over from cutting in. A worker that was woken
    /// for a queued task, and then fires a timer, would otherwise run the timer's task first
    /// while the queued task's wake-up was spent; and a task whose timer is due again
    /// whenever its poll returns would be kept on every pass and starve the queue. A worker
    /// with a scope open never keeps one, since its deque takes nothing but that scope's
    /// children; no timer is fired inside a scope, so that is a guard, not a path.
    fn keep_woken_task(&self, entry: Entry) -> Option<Entry> {
        with_local(|local| match local {
            Some(local)
                if core::ptr::eq(Arc::as_ptr(&local.shared), self)
                    && local.class == WorkerClass::FrameCritical
                    && !local.scope_open.get()
                    && self.frame.injector.is_empty()
                    && local.keep_woken_task.replace(false) =>
            {
                local.deque.push(entry);
                None
            }
            _ => Some(entry),
        })
    }

    pub(crate) fn register_task(&self, task: Weak<dyn RawTask>) -> u64 {
        let id = self.next_task.fetch_add(1, Ordering::Relaxed);
        lock(&self.tasks).insert(id, task);
        id
    }

    pub(crate) fn forget_task(&self, id: u64) {
        lock(&self.tasks).remove(&id);
    }

    /// Stops the workers, cancels every unfinished task, drops queued graph work and
    /// resolves every future that was waiting on any of it to `Cancelled`.
    ///
    /// With `join` it returns once every thread has exited. Without, for a caller that is
    /// itself a worker, it only tells them to exit.
    pub(crate) fn shut_down(&self, join: bool) {
        self.shutdown.set();
        if let Some(replay) = self.replay() {
            replay.abandon();
        }
        for pool in [&self.frame, &self.background] {
            // Claims every sleeper, then wakes every worker once: the sleepers just claimed,
            // workers parked on a scope latch, which have no sleeper bit, and workers
            // another waker has claimed but may not have woken yet.
            pool.sleep.notify_all(|_| {});
            (0..pool.threads.len()).for_each(|worker| pool.wake_worker(worker));
        }
        // Once now, so nothing that waits on cancelled work has to wait for the joins,
        // which last as long as the longest running job; and once after them, for what
        // those jobs submitted on their way out.
        self.cancel_everything();
        if join {
            // Held across the joins, so a second caller also returns only once every
            // thread has exited.
            let mut handles = lock(&self.handles);
            for handle in handles.drain(..) {
                handle.join();
            }
            drop(handles);
            self.cancel_everything();
        }
    }

    /// Cancels queued and suspended tasks, drops queued graph work and resolves the
    /// futures of unfinished runs and pending timers. The shutdown flag is set. Safe while
    /// workers are still finishing their jobs: scope children are never touched.
    fn cancel_everything(&self) {
        self.discard_queued();
        // Taken out first: cancelling a task removes it from the registry.
        let tasks = core::mem::take(&mut *lock(&self.tasks));
        for task in tasks.into_values().filter_map(|task| task.upgrade()) {
            task.cancel();
        }
        let runs = core::mem::take(&mut *lock(&self.live_runs));
        for run in runs.into_values().filter_map(|run| run.upgrade()) {
            run.cancel(self);
        }
        self.timers.cancel_all();
    }
}

/// The body of a worker thread.
pub(crate) fn worker_thread(seed: WorkerSeed) {
    let WorkerSeed { class, index, deque, thread, shared } = seed;
    // The platform applied the thread's class. The floating-point state is asserted here
    // as well, so it holds under a platform that sets scheduling its own way.
    lactose_core::thread_class::fp::set_deterministic();
    if thread.send(std::thread::current()).is_err() {
        return;
    }
    drop(thread);
    let Ok(shared) = shared.recv() else { return };
    LOCAL.with(|cell| {
        let chaos = shared.chaos.as_ref().map(|chaos| {
            let stream = (index as u64) << 1 | u64::from(class == WorkerClass::Background);
            RefCell::new(ChaosRng::new(chaos.config.seed, stream))
        });
        let local = WorkerLocal {
            shared,
            class,
            index,
            deque,
            scope_open: Cell::new(false),
            wide_job: Cell::new(false),
            keep_woken_task: Cell::new(false),
            chaos,
            next_victim: Cell::new(index + 1),
        };
        assert!(cell.set(local).is_ok(), "a thread becomes a worker once");
        // Read back through the cell, the way `with_local` reaches it from code running on
        // this thread, so the loop and that code share one kind of borrow of the state.
        worker_loop(cell.get().expect("set above"));
    });
}

fn worker_loop(local: &WorkerLocal) {
    let shared = &*local.shared;
    let pool = local.pool();
    let mut arenas = Arenas::default();
    let frame = local.class == WorkerClass::FrameCritical;
    let mut permit = false;
    // When this worker last had work, and how often it has looked since.
    let mut scanning_since = None;
    let mut scans = 0u32;

    while !shared.is_shut_down() {
        if !permit {
            permit = pool.try_acquire();
        } else if pool.release_if_over() {
            permit = false;
            if !local.deque.is_empty() {
                // What this worker holds, which may include a node it released without
                // waking anyone because it meant to run it, is now for the others.
                pool.notify_one();
            }
        }
        if permit {
            if let Some(found) = find_work(local) {
                scanning_since = None;
                scans = 0;
                execute(local, &mut arenas, found.entry, found.position);
                if let Some(place) = found.place {
                    shared.wide.width.leave(place, &shared.wide, pool);
                }
                if found.replayed
                    && let Some(replay) = shared.replay()
                {
                    replay.finished();
                }
                perturb(local, |config| config.inject_yields, 4);
                continue;
            }
            let keep_scanning = if cfg!(miri) {
                scans < MIRI_SCAN_ROUNDS
            } else {
                scanning_since.get_or_insert_with(Instant::now).elapsed() < SCAN_IDLE
            };
            if keep_scanning {
                scans += 1;
                for _ in 0..1u32 << scans.min(6) {
                    core::hint::spin_loop();
                }
                continue;
            }
            // Found nothing: no idle worker is woken for this permit (`Gate::release_idle`).
            pool.gate.release_idle(pool);
            permit = false;
        }
        scanning_since = None;
        scans = 0;
        idle(local);
    }
    if permit {
        pool.release();
    }

    // Children still have an opener waiting for them, on a worker that is finishing its
    // job. Everything else is dropped.
    while let Some(entry) = local.deque.pop() {
        match entry {
            Entry::Child { .. } => execute(local, &mut arenas, entry, 0),
            other => other.discard(),
        }
    }
    if let Some(replay) = shared.replay().filter(|_| frame) {
        while let (Some((entry, position)), _) = replay.take(local.index, Filter::Children) {
            execute(local, &mut arenas, entry, position);
            replay.finished();
        }
    }
}

/// Yields with probability one in `one_in` when the chaos option `enabled` selects is on.
fn perturb(local: &WorkerLocal, enabled: impl FnOnce(&ChaosConfig) -> bool, one_in: usize) {
    if let Some(rng) = &local.chaos
        && local.chaos_config().is_some_and(enabled)
        && rng.borrow_mut().below(one_in) == 0
    {
        std::thread::yield_now();
    }
}

/// What [`find_work`] found.
struct Found {
    entry: Entry,
    /// The entry's position in the log, in replay.
    position: u64,
    /// Whether the entry came from the replay pool, which wants to hear when it is done.
    replayed: bool,
    /// The place in the width the worker took for the entry, which it gives up when the
    /// entry is done.
    place: Option<Place>,
}

/// The next entry for this worker.
fn find_work(local: &WorkerLocal) -> Option<Found> {
    let shared = &*local.shared;
    let pool = local.pool();
    let frame = local.class == WorkerClass::FrameCritical;
    let found = |entry| Some(Found { entry, position: 0, replayed: false, place: None });

    if frame && let Some(replay) = shared.replay() {
        let (taken, wake) = replay.take(local.index, Filter::Any);
        shared.replay_wake(wake);
        if let Some((entry, position)) = taken {
            return Some(Found { entry, position, replayed: true, place: None });
        }
    }
    if let Some(entry) = local.deque.pop() {
        return found(entry);
    }
    if frame
        && fire_timers(local)
        && let Some(entry) = local.deque.pop()
    {
        return found(entry);
    }
    // In replay the log places such work like any other.
    if frame
        && shared.replay().is_none()
        && let Some((entry, place)) = shared.take_wide()
    {
        return Some(Found { entry, position: 0, replayed: false, place: Some(place) });
    }
    perturb(local, |config| config.shuffle_steals, 8);

    let victims = pool.stealers.len();
    let first = match &local.chaos {
        Some(rng) if local.chaos_config().is_some_and(|config| config.shuffle_steals) => {
            rng.borrow_mut().below(victims)
        }
        _ => local.next_victim.replace(local.next_victim.get().wrapping_add(1)) % victims,
    };
    loop {
        let mut retry = false;
        let sources = core::iter::once(None).chain((0..victims).map(Some));
        for source in sources {
            let stolen = match source {
                None => match pool.injector.steal() {
                    Steal::Success(entry) => {
                        Steal::Success(split_stolen(entry, &local.deque, pool))
                    }
                    other => other,
                },
                Some(offset) => {
                    let victim = (first + offset) % victims;
                    if victim == local.index {
                        continue;
                    }
                    steal_from(&pool.stealers[victim], &local.deque, pool)
                }
            };
            match stolen {
                Steal::Success(entry) => return found(entry),
                Steal::Retry => retry = true,
                Steal::Empty => {}
            }
        }
        if !retry {
            return None;
        }
    }
}

/// Runs one entry under the abort-on-unwind guard.
fn execute(local: &WorkerLocal, arenas: &mut Arenas, entry: Entry, position: u64) {
    let shared = &*local.shared;
    abort_on_unwind(|| match entry {
        Entry::Range { run, node, units, root } => {
            graph::run_range(local, arenas, &run, node, units, root, position);
        }
        Entry::Child { scope, job, units } => {
            job::run_child(local, scope, job, units, &mut arenas.child);
        }
        Entry::Task(task) => task.run(false),
        // A `block_in_place` wants this worker parked: the job goes back unstarted, as the
        // rest of a range does. A replay runs what the log names.
        Entry::Job(job) if shared.replay().is_none() && local.pool().over_limit() => {
            shared.requeue_job(job);
        }
        Entry::Job(job) => job.run(local, arenas, position),
    });
}

/// Fires the due timers on a frame-critical worker's way to its next job. Returns whether
/// the worker kept one of the tasks they woke on its own deque.
///
/// The worker is awake, so when nothing waits in the frame-critical queue it runs the first
/// frame-critical task the timers wake itself and wakes nobody for it, as a worker that
/// releases nodes keeps one: a sleeper woken for the task would leave deep idle to find it
/// gone, and waking it would delay the task. Every other task they wake is queued with a
/// wake-up of its own ([`Shared::submit_task`]). A worker that holds no permit keeps the
/// task all the same: it is visible on its deque, and the worker's own re-check before it
/// sleeps sees it (`gate::idle_park`).
fn fire_timers(local: &WorkerLocal) -> bool {
    local.keep_woken_task.set(true);
    local.shared.timers.fire_due();
    !local.keep_woken_task.replace(false)
}

/// Parks the worker until there is something for it to do. The worker holds no permit.
fn idle(local: &WorkerLocal) {
    let shared = &*local.shared;
    let pool = local.pool();
    let frame = local.class == WorkerClass::FrameCritical;

    let runs_started = || shared.runs.load(Ordering::SeqCst);
    let mut stalled = None;
    let asleep = Instant::now();
    gate::idle_park(&pool.gate, &pool.sleep, local.index, || {
        // Timers are served by whichever frame-critical worker is idle: there is no timer
        // thread. One sleeper at a time is the keeper and parks only until the next
        // deadline.
        let mut deadline = if frame {
            // Not `fire_timers`: this worker has announced itself, and nothing would order
            // a kept task's push before the permit check below, which a worker giving its
            // permit up relies on. A task woken here is queued with a wake-up.
            shared.timers.fire_due();
            shared.timers.keep(local.index)
        } else {
            None
        };
        // A replay that looks stuck gets a deadline too, after which it is declared so.
        stalled =
            shared.replay().filter(|_| frame).and_then(|replay| replay.stalled(runs_started()));
        if stalled.is_some() {
            let limit = Instant::now() + REPLAY_STALL;
            deadline = Some(deadline.map_or(limit, |timer| timer.min(limit)));
        }
        // Such work of `run_job` counts only while a worker without a place could take it.
        let work = pool.work_visible()
            || frame && shared.wide.width.admits(&shared.wide)
            || frame && shared.replay().is_some_and(|replay| replay.ready(local.index));
        let seen = if shared.is_shut_down() {
            Look::Stop
        } else if work {
            Look::Work
        } else {
            Look::Nothing
        };
        (seen, deadline)
    });
    if frame {
        // If the park timed out, this is the deadline it was for, and the task it wakes is
        // this worker's next job.
        fire_timers(local);
        if shared.timers.release(local.index) {
            // Later deadlines remain and this worker may be about to take a job; hand the
            // watch to another sleeper.
            pool.notify_one();
        }
    }
    if let (Some(replay), Some(cursor)) = (shared.replay(), stalled)
        && asleep.elapsed() >= REPLAY_STALL
        && replay.stalled(runs_started()) == Some(cursor)
        && !shared.is_shut_down()
    {
        // The log names a job this program never makes available. Nothing can change
        // that, so stop with the evidence instead of hanging until a test times out.
        abort_on_unwind(|| panic!("{}", replay.stall_report()));
    }
}

/// What a worker running a range does about the units it has not started yet.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rest {
    /// Keep going.
    Run,
    /// Push them back as an entry and stop: a `block_in_place` wants this worker parked.
    Requeue,
    /// Drop them: the executor is shutting down and the work will never complete.
    Abandon,
}

/// Peels `units` one at a time through `run` while keeping the rest stealable, and returns
/// how many units it ran itself.
///
/// Before each unit, if `dry` says the place a split half goes holds nothing and more than
/// one unit is left, the upper half goes to `split`, which pushes it. For graph work and the
/// children of other scopes that place is this worker's own deque: a range that was just
/// stolen arrives with an empty deque, so this is also the thief-side split-on-steal. For
/// the children of a `run_job` job's scope it is the pool's queue for such work, which every
/// half and every worker's put-back goes to; the worker's own deque would say nothing about
/// that work. With `force_split` (chaos) the range is halved down to single units regardless.
///
/// `rest` is asked before every unit what to do with the units not yet started.
pub(crate) fn drain_range(
    local: &WorkerLocal,
    mut units: Range<u32>,
    mut split: impl FnMut(Range<u32>),
    dry: impl Fn() -> bool,
    mut run: impl FnMut(Range<u32>),
    rest: impl Fn() -> Rest,
) -> u32 {
    let force_split = local.chaos_config().is_some_and(|config| config.force_split);
    let mut done = 0;
    while units.start < units.end {
        match rest() {
            Rest::Run => {}
            Rest::Requeue => {
                split(units);
                break;
            }
            Rest::Abandon => break,
        }
        while units.end - units.start > 1 && (force_split || dry()) {
            let middle = units.start + (units.end - units.start) / 2;
            split(middle..units.end);
            units.end = middle;
            if !force_split {
                break;
            }
        }
        run(units.start..units.start + 1);
        units.start += 1;
        done += 1;
    }
    done
}

/// Spins until `done` returns `true` or `limit` has passed, and returns what `done` last
/// said. Used where a wait is expected to be shorter than a park and unpark round trip.
pub(crate) fn spin_until(limit: Duration, done: impl Fn() -> bool) -> bool {
    if limit.is_zero() {
        return done();
    }
    let start = Instant::now();
    loop {
        for _ in 0..32 {
            if done() {
                return true;
            }
            core::hint::spin_loop();
        }
        if start.elapsed() >= limit {
            return done();
        }
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use core::ops::Range;
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::task::{Context, Poll, Waker};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use lactose_core::ErrorKind;

    use super::{Arenas, Entry, Shared, Wide, execute, with_local};
    use crate::job_run::{self, JobRun};
    use crate::{Executor, GraphBuilder, GraphNode, JobContext, WorkerClass};

    /// On the one frame-critical worker, while a `block_in_place` would want that worker
    /// parked, takes the job it queues out of the pool's queue, as a worker does that found
    /// the job just before the gate closed, and executes it.
    struct TakeAtAClosedGate {
        started: Arc<AtomicBool>,
        run: Mutex<Option<JobRun<()>>>,
        put_back: AtomicBool,
    }

    impl GraphNode for TakeAtAClosedGate {
        fn name(&self) -> &'static str {
            "pool::tests::take_at_a_closed_gate"
        }

        fn units(&self) -> u32 {
            1
        }

        fn run(&self, _units: Range<u32>, _ctx: &mut JobContext<'_>) {
            with_local(|local| {
                let local = local.expect("a graph job runs on a worker");
                let (shared, pool) = (&*local.shared, local.pool());
                // The gate closes with no permit reserved, and this worker holds one all the
                // same: it is over the limit.
                pool.gate.close(0, pool);
                pool.gate.retake();
                assert!(pool.over_limit());
                let started = self.started.clone();
                let run = job_run::start(shared, move |_| started.store(true, Ordering::SeqCst));
                let entry = Wide::take(&shared.wide.jobs).expect("the job is queued");
                assert!(matches!(entry, Entry::Job(_)));
                execute(local, &mut Arenas::default(), entry, 0);
                // Back in the queue, unstarted.
                let back = !self.started.load(Ordering::SeqCst) && !shared.wide.jobs.is_empty();
                self.put_back.store(back, Ordering::SeqCst);
                // What `block_in_place` does when its closure returns.
                pool.gate.release(pool);
                pool.gate.open(0, pool);
                *self.run.lock().unwrap() = Some(run);
            });
        }
    }

    /// A worker that takes a job after shutdown began, before the sweep reached it, drops it
    /// unstarted: inside a job on the one frame-critical worker, a job is queued, taken out
    /// of the pool's queue and executed once the shutdown flag is set.
    #[test]
    fn a_job_taken_after_shutdown_began_never_starts() {
        let executor = Executor::builder().frame_workers(1).background_workers(1).build().unwrap();
        let (started, run) = executor
            .block_on(executor.run_job(|_ctx| {
                with_local(|local| {
                    let local = local.expect("a job runs on a worker");
                    let shared = &*local.shared;
                    let started = Arc::new(AtomicBool::new(false));
                    let flag = started.clone();
                    let run = job_run::start(shared, move |_| flag.store(true, Ordering::SeqCst));
                    let entry = Wide::take(&shared.wide.jobs).expect("the job is queued");
                    // Shutdown has begun; its sweep has not reached this entry.
                    shared.shutdown.set();
                    execute(local, &mut Arenas::default(), entry, 0);
                    (started.load(Ordering::SeqCst), run)
                })
            }))
            .unwrap();
        assert!(!started, "a job taken after shutdown began was started");
        let cancelled = executor.block_on(run).expect_err("the job never started");
        assert_eq!(cancelled.kind(), lactose_core::ErrorKind::Cancelled);
        executor.shutdown();
    }

    /// A worker that takes a job of `run_job` while a `block_in_place` wants it parked puts
    /// it back unstarted, and the job runs once the gate opens.
    #[test]
    fn a_job_taken_at_a_closed_gate_is_put_back_unstarted() {
        let executor = Executor::builder().frame_workers(1).background_workers(1).build().unwrap();
        let node = Arc::new(TakeAtAClosedGate {
            started: Arc::new(AtomicBool::new(false)),
            run: Mutex::new(None),
            put_back: AtomicBool::new(false),
        });
        let mut builder = GraphBuilder::new();
        builder.add_node(node.clone());
        executor.block_on(executor.run_graph(builder.build().unwrap())).unwrap();
        assert!(node.put_back.load(Ordering::SeqCst), "the job was not put back unstarted");
        let run = node.run.lock().unwrap().take().unwrap();
        executor.block_on(run).unwrap();
        assert!(node.started.load(Ordering::SeqCst));
        executor.shutdown();
    }

    /// How long a test waits for the held worker's job to begin.
    const LIMIT: Duration = Duration::from_secs(60);

    /// Polls `future` once, with a waker that does nothing.
    fn poll_now<F: Future>(future: F) -> Poll<F::Output> {
        core::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop()))
    }

    /// An executor whose one frame-critical worker is held inside a job of `run_job`, with
    /// shutdown set between the push and the read of every queueing from then on: whatever
    /// is queued next sees the flag after its push and must sweep itself, and no worker can
    /// take it first. The worker is released when this is dropped.
    struct ShutdownInTheGap {
        executor: Executor,
        shared: Arc<Shared>,
        release: Arc<AtomicBool>,
    }

    impl ShutdownInTheGap {
        fn new() -> Self {
            let executor =
                Executor::builder().frame_workers(1).background_workers(1).build().unwrap();
            let release = Arc::new(AtomicBool::new(false));
            let (began, shared) = std::sync::mpsc::channel();
            let held = release.clone();
            drop(executor.run_job(move |_| {
                began.send(with_local(|local| local.expect("on a worker").shared.clone())).unwrap();
                while !held.load(Ordering::SeqCst) {
                    std::thread::yield_now();
                }
            }));
            let shared: Arc<Shared> = shared.recv_timeout(LIMIT).expect("the holding job began");
            shared.shutdown.sets_after_queueing.store(true, Ordering::SeqCst);
            Self { executor, shared, release }
        }
    }

    impl Drop for ShutdownInTheGap {
        fn drop(&mut self) {
            self.release.store(true, Ordering::SeqCst);
            self.executor.clone().shutdown();
        }
    }

    /// A job of `run_job` queued as shutdown begins, which sees the flag after its push, is
    /// swept by its own submission: its future has resolved `Cancelled` by the time
    /// `run_job` returns.
    #[test]
    fn a_job_queued_as_shutdown_begins_is_cancelled() {
        let gap = ShutdownInTheGap::new();
        let ran = Arc::new(AtomicBool::new(false));
        let run = {
            let ran = ran.clone();
            gap.executor.run_job(move |_| ran.store(true, Ordering::SeqCst))
        };
        assert!(gap.shared.is_shut_down());
        match poll_now(run) {
            Poll::Ready(Err(error)) => assert_eq!(error.kind(), ErrorKind::Cancelled),
            Poll::Ready(Ok(())) => panic!("a job queued as shutdown began ran"),
            Poll::Pending => panic!("a job queued as shutdown began stayed queued"),
        }
        drop(gap);
        assert!(!ran.load(Ordering::SeqCst));
    }

    /// A task queued as shutdown begins, which sees the flag after its push, is swept by its
    /// own submission: it has been cancelled by the time `spawn` returns.
    #[test]
    fn a_task_queued_as_shutdown_begins_is_cancelled() {
        let gap = ShutdownInTheGap::new();
        let ran = Arc::new(AtomicBool::new(false));
        let task = {
            let ran = ran.clone();
            gap.executor.spawn(WorkerClass::FrameCritical, async move {
                ran.store(true, Ordering::SeqCst);
            })
        };
        assert!(gap.shared.is_shut_down());
        match poll_now(task) {
            Poll::Ready(Err(error)) => assert_eq!(error.kind(), ErrorKind::Cancelled),
            Poll::Ready(Ok(())) => panic!("a task queued as shutdown began ran"),
            Poll::Pending => panic!("a task queued as shutdown began stayed queued"),
        }
        drop(gap);
        assert!(!ran.load(Ordering::SeqCst));
    }

    /// A graph node that records whether it ran.
    #[derive(Default)]
    struct Runs {
        ran: AtomicBool,
    }

    impl GraphNode for Runs {
        fn name(&self) -> &'static str {
            "pool::tests::runs"
        }

        fn units(&self) -> u32 {
            1
        }

        fn run(&self, _units: Range<u32>, _ctx: &mut JobContext<'_>) {
            self.ran.store(true, Ordering::SeqCst);
        }
    }

    /// A graph's range queued as shutdown begins, which sees the flag after its push, is
    /// swept by its own submission: the queue is empty by the time `run_graph` returns, the
    /// node never runs, and the run resolves `Cancelled` once shutdown has run.
    #[test]
    fn a_range_queued_as_shutdown_begins_is_dropped() {
        let gap = ShutdownInTheGap::new();
        let node = Arc::new(Runs::default());
        let mut builder = GraphBuilder::new();
        builder.add_node(node.clone());
        let run = gap.executor.run_graph(builder.build().unwrap());
        assert!(gap.shared.is_shut_down());
        assert!(gap.shared.frame.injector.is_empty(), "a range queued as shutdown began stayed");
        let executor = gap.executor.clone();
        drop(gap);
        match executor.block_on(run) {
            Err(error) => assert_eq!(error.kind(), ErrorKind::Cancelled),
            Ok(_) => panic!("a run whose range was swept completed"),
        }
        assert!(!node.ran.load(Ordering::SeqCst));
    }
}
