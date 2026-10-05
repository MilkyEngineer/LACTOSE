//! Worker pools: deque entries, the worker loop, stealing, idling and the `block_in_place`
//! gate.

use core::cell::{Cell, OnceCell, RefCell};
use core::ops::Range;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::Thread;
use std::time::{Duration, Instant};

use crate::deque::{Injector, Steal, Stealer, Worker};
use lactose_core::Arena;

use crate::chaos::{Chaos, ChaosConfig, ChaosRng, Filter, Wake};
use crate::graph::{self, RunState};
use crate::guard::abort_on_unwind;
use crate::job::{self, ChildHeader, ScopeCore};
use crate::platform::{Platform, ThreadHandle};
use crate::sleep::Sleep;
use crate::task::RawTask;
use crate::time::Timers;
use crate::{WorkerClass, lock};

/// How long a worker that found nothing keeps rescanning the queues before it parks.
///
/// While a graph run is in flight the next node is usually one finalize away, and a park
/// and unpark round trip costs more than most jobs take, so a frame-critical worker keeps
/// looking for longer. Between runs it gives up quickly and leaves the core alone.
const SCAN_IDLE: Duration = Duration::from_micros(50);
const SCAN_DURING_RUN: Duration = Duration::from_micros(300);

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
            Self::Task(_) => unreachable!("a task has no units"),
        }
    }

    /// This entry narrowed to `units`.
    pub(crate) fn into_units(self, units: Range<u32>) -> Self {
        match self {
            Self::Range { run, node, root, .. } => Self::Range { run, node, units, root },
            Self::Child { scope, job, .. } => Self::Child { scope, job, units },
            Self::Task(_) => unreachable!("a task has no units"),
        }
    }

    /// Gives up on an entry that will never run. A task is cancelled; graph work is dropped,
    /// which leaves its run unfinished for good. Never called on a child, whose opener
    /// would wait forever.
    fn discard(self) {
        match self {
            Self::Task(task) => task.run(true),
            Self::Range { .. } => {}
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
    /// Workers currently holding a permit to take jobs. A sleeping worker holds none.
    running: AtomicU32,
    /// The most permits that may be out. `u32::MAX` except while a `block_in_place` runs.
    allowed: AtomicU32,
    /// The reservations of the `block_in_place` calls in progress.
    gate: Mutex<Vec<u32>>,
}

impl Pool {
    fn new(stealers: Vec<Stealer<Entry>>, threads: Vec<Thread>) -> Self {
        Self {
            injector: Injector::new(),
            sleep: Sleep::new(stealers.len()),
            stealers: stealers.into(),
            threads: threads.into(),
            running: AtomicU32::new(0),
            allowed: AtomicU32::new(u32::MAX),
            gate: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn workers(&self) -> usize {
        self.stealers.len()
    }

    /// Wakes one sleeping worker, if any. Call after making work visible.
    pub(crate) fn notify_one(&self) {
        if let Some(worker) = self.sleep.notify_one() {
            self.threads[worker].unpark();
        }
    }

    /// Wakes `worker` whether it sleeps in the idle protocol or on a scope latch.
    pub(crate) fn wake(&self, worker: usize) {
        let _claimed = self.sleep.notify(worker);
        self.threads[worker].unpark();
    }

    pub(crate) fn wake_all(&self) {
        self.sleep.notify_all(|worker| self.threads[worker].unpark());
    }

    /// Unparks `worker`, which waits on a scope latch that has just been set.
    pub(crate) fn unpark(&self, worker: usize) {
        self.threads[worker].unpark();
    }

    fn work_visible(&self) -> bool {
        !self.injector.is_empty() || self.stealers.iter().any(|stealer| !stealer.is_empty())
    }

    fn permit_available(&self) -> bool {
        self.running.load(Ordering::SeqCst) < self.allowed.load(Ordering::SeqCst)
    }

    fn try_acquire(&self) -> bool {
        self.running
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |running| {
                (running < self.allowed.load(Ordering::SeqCst)).then_some(running + 1)
            })
            .is_ok()
    }

    fn release(&self) {
        self.running.fetch_sub(1, Ordering::SeqCst);
    }

    /// Gives the calling worker's permit up if a `block_in_place` has lowered the limit
    /// below the number of permits out. Returns whether it did.
    fn release_if_over(&self) -> bool {
        let allowed = self.allowed.load(Ordering::Relaxed);
        allowed != u32::MAX
            && self
                .running
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |running| {
                    (running > allowed).then(|| running - 1)
                })
                .is_ok()
    }

    /// Whether a `block_in_place` has lowered the limit below the number of permits out,
    /// so that some worker has to stop taking units.
    pub(crate) fn over_limit(&self) -> bool {
        let allowed = self.allowed.load(Ordering::Relaxed);
        allowed != u32::MAX && self.running.load(Ordering::Relaxed) > allowed
    }

    /// Lends the calling worker's permit out while it is parked mid-job on a scope latch, so
    /// a closed gate does not count a worker that is running nothing. Returns whether it
    /// did, for [`Pool::resume_permit`].
    pub(crate) fn suspend_permit(&self) -> bool {
        let gated = self.allowed.load(Ordering::Relaxed) != u32::MAX;
        if gated {
            self.release();
            self.notify_one();
        }
        gated
    }

    /// Takes the permit back after [`Pool::suspend_permit`]. The worker is mid-job and
    /// continues regardless of the limit.
    pub(crate) fn resume_permit(&self, suspended: bool) {
        if suspended {
            self.running.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Starts a `block_in_place`: at most `reserved` workers other than the caller may hold
    /// permits. Workers over the limit give theirs up at their next job boundary.
    pub(crate) fn close_gate(&self, reserved: u32) {
        let mut gate = lock(&self.gate);
        gate.push(reserved);
        let allowed = gate.iter().copied().min().unwrap_or(u32::MAX);
        self.allowed.store(allowed, Ordering::SeqCst);
        drop(gate);
        // The caller blocks and stops counting.
        self.release();
    }

    /// Ends a `block_in_place` started with the same `reserved`.
    pub(crate) fn open_gate(&self, reserved: u32) {
        let mut gate = lock(&self.gate);
        let at = gate.iter().position(|&held| held == reserved).expect("gate was closed");
        gate.swap_remove(at);
        let allowed = gate.iter().copied().min().unwrap_or(u32::MAX);
        self.allowed.store(allowed, Ordering::SeqCst);
        drop(gate);
        self.running.fetch_add(1, Ordering::SeqCst);
        self.wake_all();
    }
}

/// Everything the workers of one executor share.
pub(crate) struct Shared {
    frame: Pool,
    background: Pool,
    pub(crate) platform: Arc<dyn Platform>,
    pub(crate) shutdown: AtomicBool,
    pub(crate) timers: Timers,
    pub(crate) chaos: Option<Chaos>,
    /// Graph runs started so far; a run's ordinal identifies it in an execution log.
    runs: AtomicU64,
    /// Graph runs that have started and not finished. Only steers how long idle workers
    /// scan before parking.
    runs_in_flight: AtomicU32,
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
            frame: Pool::new(frame.0, frame.1),
            background: Pool::new(background.0, background.1),
            platform,
            shutdown: AtomicBool::new(false),
            timers: Timers::new(),
            chaos,
            runs: AtomicU64::new(0),
            runs_in_flight: AtomicU32::new(0),
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
        self.shutdown.load(Ordering::Acquire)
    }

    /// Notes the start of a graph run and returns its ordinal.
    pub(crate) fn begin_run(&self) -> u64 {
        self.runs_in_flight.fetch_add(1, Ordering::Relaxed);
        self.runs.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn end_run(&self) {
        self.runs_in_flight.fetch_sub(1, Ordering::Relaxed);
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
    /// executor's frame-critical workers, on the injector otherwise.
    pub(crate) fn submit_range(&self, entry: Entry) {
        if self.is_shut_down() {
            return;
        }
        if let Some(replay) = self.replay() {
            self.replay_wake(replay.push(entry));
            self.discard_if_shut_down();
            return;
        }
        let entry = with_local(|local| match local {
            Some(local)
                if core::ptr::eq(Arc::as_ptr(&local.shared), self)
                    && local.class == WorkerClass::FrameCritical =>
            {
                local.deque.push(entry);
                None
            }
            _ => Some(entry),
        });
        if let Some(entry) = entry {
            self.frame.injector.push(entry);
        }
        self.frame.notify_one();
        self.discard_if_shut_down();
    }

    /// Closes the race between a submission and shutdown. Call after the entry is queued
    /// and after the notification, whose fence orders the queueing before the load here:
    /// either shutdown's own sweep comes after the entry and finds it, or this load sees
    /// the flag and sweeps.
    fn discard_if_shut_down(&self) {
        if self.shutdown.load(Ordering::SeqCst) {
            self.discard_queued();
        }
    }

    /// Cancels every queued task and drops queued graph work. Safe while workers still
    /// run: scope children are never touched.
    fn discard_queued(&self) {
        fence(Ordering::SeqCst);
        if let Some(chaos) = &self.chaos {
            chaos.discard_graph_work();
        }
        for pool in [&self.frame, &self.background] {
            loop {
                match pool.injector.steal() {
                    Steal::Success(entry) => entry.discard(),
                    Steal::Empty => break,
                    Steal::Retry => {}
                }
            }
        }
    }

    /// Queues a ready task behind the other ready tasks of its class.
    pub(crate) fn submit_task(&self, class: WorkerClass, task: Arc<dyn RawTask>) {
        if self.is_shut_down() {
            task.run(true);
            return;
        }
        let pool = self.pool(class);
        pool.injector.push(Entry::Task(task));
        pool.notify_one();
        self.discard_if_shut_down();
    }

    pub(crate) fn register_task(&self, task: Weak<dyn RawTask>) -> u64 {
        let id = self.next_task.fetch_add(1, Ordering::Relaxed);
        lock(&self.tasks).insert(id, task);
        id
    }

    pub(crate) fn forget_task(&self, id: u64) {
        lock(&self.tasks).remove(&id);
    }

    /// Stops the workers, cancels every unfinished task and drops queued graph work.
    pub(crate) fn shut_down(&self) {
        debug_assert!(!on_worker_thread(), "shutdown called on a worker would wait for itself");
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(replay) = self.replay() {
            replay.abandon();
        }
        for pool in [&self.frame, &self.background] {
            pool.wake_all();
            // A worker parked on a scope latch in replay is waiting for its turn, not for a
            // sleeper bit.
            pool.threads.iter().for_each(Thread::unpark);
        }
        {
            // Held across the joins, so a second caller also returns only once every
            // thread has exited.
            let mut handles = lock(&self.handles);
            for handle in handles.drain(..) {
                handle.join();
            }
        }

        self.timers.clear();
        self.discard_queued();
        // Taken out first: cancelling a task removes it from the registry.
        let tasks = core::mem::take(&mut *lock(&self.tasks));
        for task in tasks.into_values().filter_map(|task| task.upgrade()) {
            task.cancel();
        }
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
        let local = cell.get_or_init(|| WorkerLocal {
            shared,
            class,
            index,
            deque,
            chaos,
            next_victim: Cell::new(index + 1),
        });
        worker_loop(local);
    });
}

fn worker_loop(local: &WorkerLocal) {
    let shared = &*local.shared;
    let pool = local.pool();
    let mut arenas = Arenas::default();
    let mut permit = false;
    // When this worker last had work, and how often it has looked since.
    let mut scanning_since = None;
    let mut scans = 0u32;

    while !shared.is_shut_down() {
        if !permit {
            permit = pool.try_acquire();
        } else if pool.release_if_over() {
            permit = false;
        }
        if permit {
            if let Some((entry, position)) = find_work(local) {
                scanning_since = None;
                scans = 0;
                execute(local, &mut arenas, entry, position);
                perturb(local, |config| config.inject_yields, 4);
                continue;
            }
            let keep_scanning = if cfg!(miri) {
                scans < MIRI_SCAN_ROUNDS
            } else {
                let limit = if local.class == WorkerClass::FrameCritical
                    && shared.runs_in_flight.load(Ordering::Relaxed) != 0
                {
                    SCAN_DURING_RUN
                } else {
                    SCAN_IDLE
                };
                scanning_since.get_or_insert_with(Instant::now).elapsed() < limit
            };
            if keep_scanning {
                scans += 1;
                for _ in 0..1u32 << scans.min(6) {
                    core::hint::spin_loop();
                }
                continue;
            }
            pool.release();
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
    if let Some(replay) = shared.replay().filter(|_| local.class == WorkerClass::FrameCritical) {
        while let (Some((entry, position)), _) = replay.take(local.index, Filter::Children) {
            execute(local, &mut arenas, entry, position);
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

/// The next entry for this worker and, in replay, its position in the log.
fn find_work(local: &WorkerLocal) -> Option<(Entry, u64)> {
    let shared = &*local.shared;
    let pool = local.pool();
    let frame = local.class == WorkerClass::FrameCritical;

    if frame && let Some(replay) = shared.replay() {
        let (taken, wake) = replay.take(local.index, Filter::Any);
        shared.replay_wake(wake);
        if taken.is_some() {
            return taken;
        }
    }
    if let Some(entry) = local.deque.pop() {
        return Some((entry, 0));
    }
    if frame {
        shared.timers.fire_due();
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
                None => pool.injector.steal(),
                Some(offset) => {
                    let victim = (first + offset) % victims;
                    if victim == local.index {
                        continue;
                    }
                    pool.stealers[victim].steal()
                }
            };
            match stolen {
                Steal::Success(entry) => return Some((entry, 0)),
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
    abort_on_unwind(|| match entry {
        Entry::Range { run, node, units, root } => {
            graph::run_range(local, arenas, &run, node, units, root, position);
        }
        Entry::Child { scope, job, units } => {
            job::run_child(local, scope, job, units, &mut arenas.child, false);
        }
        Entry::Task(task) => task.run(false),
    });
}

/// Parks the worker until there is something for it to do. The worker holds no permit.
fn idle(local: &WorkerLocal) {
    let shared = &*local.shared;
    let pool = local.pool();
    let frame = local.class == WorkerClass::FrameCritical;

    pool.sleep.announce(local.index);
    // Timers are served by whichever frame-critical worker is idle: there is no timer
    // thread. One sleeper at a time is the keeper and parks only until the next deadline.
    let deadline = if frame {
        shared.timers.fire_due();
        shared.timers.keep(local.index)
    } else {
        None
    };
    let work =
        pool.work_visible() || frame && shared.replay().is_some_and(|r| r.ready(local.index));
    if shared.is_shut_down() || work && pool.permit_available() {
        pool.sleep.cancel(local.index);
    } else {
        pool.sleep.park(local.index, deadline);
    }
    if frame {
        // If the park timed out, this is the deadline it was for.
        shared.timers.fire_due();
        if shared.timers.release(local.index) {
            // Later deadlines remain and this worker may be about to take a job; hand the
            // watch to another sleeper.
            pool.notify_one();
        }
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
/// Before each unit, if this worker's deque is empty and more than one unit is left, the
/// upper half goes to `split`, which pushes it. A range that was just stolen arrives with an
/// empty deque, so this is also the thief-side split-on-steal. With `force_split` (chaos)
/// the range is halved down to single units regardless.
///
/// `rest` is asked before every unit what to do with the units not yet started.
pub(crate) fn drain_range(
    local: &WorkerLocal,
    mut units: Range<u32>,
    mut split: impl FnMut(Range<u32>),
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
        while units.end - units.start > 1 && (force_split || local.deque.is_empty()) {
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
