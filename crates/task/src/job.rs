//! What a running job can do: fork-join over borrowed data, scratch memory, and the one
//! sanctioned blocking wait.

use core::alloc::Layout;
use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::Range;
use core::ptr::NonNull;

use lactose_core::{AllocTag, Arena};

use crate::chaos::{Filter, JobKind, LogRecord};
use crate::gate::Opener;
use crate::guard::abort_on_unwind;
use crate::latch::Latch;
use crate::pool::{Arenas, Entry, LATCH_SPIN, Rest, WorkerLocal, drain_range, spin_until};

/// Handed to every job.
pub struct JobContext<'job> {
    local: &'job WorkerLocal,
    scratch: &'job Arena,
    child_scratch: &'job mut Arena,
    scope_jobs: &'job mut Arena,
    /// This job's position in the execution log. Zero unless chaos records or replays.
    position: u64,
    /// Children forked by this job so far, across all its scopes.
    forked: u32,
}

impl<'job> JobContext<'job> {
    pub(crate) fn new(local: &'job WorkerLocal, arenas: &'job mut Arenas, position: u64) -> Self {
        Self {
            local,
            scratch: &arenas.job,
            child_scratch: &mut arenas.child,
            scope_jobs: &mut arenas.scope,
            position,
            forked: 0,
        }
    }

    /// Runs `f`, which may fork child jobs that borrow from the caller's stack, and returns
    /// once every child has finished.
    ///
    /// While waiting, the calling worker runs only jobs of this scope, then spins briefly,
    /// then parks on the scope's latch. It never runs unrelated graph jobs or background
    /// work.
    ///
    /// Taking `&mut self` makes nesting a compile error: a job opens scopes one after
    /// another, never one inside another. Children receive a [`ChildContext`] that cannot
    /// open scopes, and [`Scope`] is neither `Send` nor `Sync`, so a child cannot fork into
    /// its parent's scope either. Every fork happens on the opening worker.
    ///
    /// A panic in `f` or in a child aborts the process.
    pub fn scope<'env, F, R>(&mut self, f: F) -> R
    where
        F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> R,
    {
        // The guard covers the body as well as the wait: unwinding out of here with
        // children outstanding would free the stack they borrow.
        let result = abort_on_unwind(|| {
            let scope = Scope {
                core: ScopeCore {
                    latch: Latch::new(),
                    opener: self.local.index,
                    tag: AllocTag::current(),
                    position: self.position,
                },
                local: self.local,
                jobs: &*self.scope_jobs,
                drops: Cell::new(None),
                forked: Cell::new(self.forked),
                scope: PhantomData,
                env: PhantomData,
                opener_only: PhantomData,
            };
            // From here until the wait is over, this worker's deque takes nothing but this
            // scope's children (`Shared::submit_range`).
            self.local.scope_open.set(true);
            let result = f(&scope);
            scope.wait(self.child_scratch);
            self.local.scope_open.set(false);
            scope.drop_range_jobs();
            self.forked = scope.forked.get();
            result
        });
        self.scope_jobs.reset();
        result
    }

    /// Scratch memory for this job, valid until the job returns. Reset, and poisoned in
    /// debug builds, between jobs.
    ///
    /// The borrow is independent of the context, so a job can allocate per-child buffers
    /// here and hand them to children forked in a later [`JobContext::scope`].
    #[must_use]
    pub fn scratch(&self) -> &'job Arena {
        self.scratch
    }

    /// A dense index below [`crate::Executor::slot_count`]. At most one job or child job
    /// runs under a given slot at any moment.
    ///
    /// The slot is **not a semantic key**. It depends on scheduling and differs between
    /// runs. Its one legitimate use is addressing per-slot partial storage that a
    /// [`crate::GraphNode`] merges in `finalize` with an order-independent combiner.
    /// `lactose-ecs` uses it that way and never passes it to a system.
    ///
    /// A job that opens a scope lends its slot to the children it runs itself while it
    /// waits, so it must not hold a borrow of its slot's storage across
    /// [`JobContext::scope`].
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.local.index as u32
    }

    /// Runs `f`, which blocks this worker on threads the executor does not own, and returns
    /// its result. This is the one sanctioned blocking wait on a worker; the physics step
    /// uses it to call into the solver's pool (architecture section 16).
    ///
    /// While `f` runs, at most `reserved_workers` other frame-critical workers take jobs
    /// and the rest stay parked, so the foreign pool and the executor together do not
    /// oversubscribe the performance cores. The caller sizes the foreign pool as
    /// [`crate::Executor::frame_worker_count`] minus `reserved_workers`.
    ///
    /// A worker that is in the middle of a job when `f` starts finishes that job first; the
    /// limit applies to which workers take another one.
    pub fn block_in_place<R>(&mut self, reserved_workers: u32, f: impl FnOnce() -> R) -> R {
        let pool = self.local.pool();
        pool.close_gate(reserved_workers);
        let result = f();
        pool.open_gate(reserved_workers);
        result
    }
}

/// The part of a scope its children reach from other workers. Lives in the opener's stack
/// frame for the duration of [`JobContext::scope`].
pub(crate) struct ScopeCore {
    pub(crate) latch: Latch,
    /// The frame-critical worker that opened the scope.
    pub(crate) opener: usize,
    /// The opener's allocation tag, re-entered around every child.
    pub(crate) tag: AllocTag,
    /// The opening job's position in the execution log.
    pub(crate) position: u64,
}

/// The type-erased head of a child job in scope job storage.
#[repr(C)]
pub(crate) struct ChildHeader {
    /// Runs the job over `units`.
    ///
    /// # Safety
    ///
    /// `job` must point to the live job this header belongs to, and `units` must be units
    /// of it that no other call receives.
    call: unsafe fn(job: NonNull<ChildHeader>, units: Range<u32>, ctx: &mut ChildContext<'_>),
    /// The job's fork ordinal within the opening job, which identifies it in a log.
    pub(crate) ordinal: u32,
}

/// A job from [`Scope::fork`]: one unit, run once, consuming the closure.
#[repr(C)]
struct OnceJob<F> {
    header: ChildHeader,
    f: ManuallyDrop<F>,
}

/// A job from [`Scope::fork_range`]: shared by every worker that runs part of the range.
#[repr(C)]
struct RangeJob<F> {
    header: ChildHeader,
    f: F,
}

/// A range job whose closure needs dropping when the scope closes.
#[derive(Clone, Copy)]
struct DropEntry {
    job: NonNull<u8>,
    /// # Safety
    ///
    /// `job` must point to the live `RangeJob` this function was instantiated for, and
    /// nothing may use that job afterwards.
    drop: unsafe fn(job: NonNull<u8>),
    next: Option<NonNull<DropEntry>>,
}

/// A fork-join scope. See [`JobContext::scope`].
///
/// The lifetimes follow `std::thread::scope`: `'scope` is invariant and is the lifetime of
/// the scope itself, and `'env` is what forked closures may borrow. Unlike
/// `std::thread::Scope` this type is neither `Send` nor `Sync`.
pub struct Scope<'scope, 'env: 'scope> {
    core: ScopeCore,
    local: &'scope WorkerLocal,
    /// Scope job storage. Only the opener allocates from it; other workers read the jobs.
    jobs: &'scope Arena,
    /// Range jobs to drop at close, newest first.
    drops: Cell<Option<NonNull<DropEntry>>>,
    forked: Cell<u32>,
    scope: PhantomData<&'scope mut &'scope ()>,
    env: PhantomData<&'env mut &'env ()>,
    opener_only: PhantomData<*const ()>,
}

impl<'scope> Scope<'scope, '_> {
    /// Queues `f` as a child job of this scope.
    pub fn fork<F>(&'scope self, f: F)
    where
        F: FnOnce(&mut ChildContext<'_>) + Send + 'scope,
    {
        /// # Safety
        ///
        /// See [`ChildHeader::call`]. The job has one unit, so this is called at most once.
        unsafe fn call<F: FnOnce(&mut ChildContext<'_>)>(
            job: NonNull<ChildHeader>,
            _units: Range<u32>,
            ctx: &mut ChildContext<'_>,
        ) {
            let job = job.cast::<OnceJob<F>>();
            // SAFETY: the caller passes the job this function was stored in, so the cast
            // restores its type. This is the only call, so the closure is read out once and
            // never touched again; the storage is plain arena memory that drops nothing.
            let f = unsafe { (&raw const (*job.as_ptr()).f).read() };
            ManuallyDrop::into_inner(f)(ctx);
        }

        let header = ChildHeader { call: call::<F>, ordinal: self.next_ordinal() };
        let job = self.store(OnceJob { header, f: ManuallyDrop::new(f) });
        self.submit(job.cast(), 0..1);
    }

    /// Queues `f` over `units` as one child job that is halved each time it is stolen. `f`
    /// is called on disjoint ranges that together cover `units` exactly once.
    pub fn fork_range<F>(&'scope self, units: Range<u32>, f: F)
    where
        F: Fn(Range<u32>, &mut ChildContext<'_>) + Send + Sync + 'scope,
    {
        /// # Safety
        ///
        /// See [`ChildHeader::call`].
        unsafe fn call<F: Fn(Range<u32>, &mut ChildContext<'_>)>(
            job: NonNull<ChildHeader>,
            units: Range<u32>,
            ctx: &mut ChildContext<'_>,
        ) {
            let job = job.cast::<RangeJob<F>>();
            // SAFETY: the caller passes the job this function was stored in, so the cast
            // restores its type, and the job is alive. Other workers hold only shared
            // references to the closure, as this one does.
            let f = unsafe { &(*job.as_ptr()).f };
            f(units, ctx);
        }

        /// # Safety
        ///
        /// See [`DropEntry::drop`].
        unsafe fn drop_job<F>(job: NonNull<u8>) {
            // SAFETY: the caller passes the `RangeJob<F>` this function was registered
            // with, alive and no longer used by anyone.
            unsafe { (&raw mut (*job.cast::<RangeJob<F>>().as_ptr()).f).drop_in_place() };
        }

        if units.is_empty() {
            return;
        }
        let header = ChildHeader { call: call::<F>, ordinal: self.next_ordinal() };
        let job = self.store(RangeJob { header, f });
        if core::mem::needs_drop::<F>() {
            let entry = DropEntry { job: job.cast(), drop: drop_job::<F>, next: self.drops.take() };
            self.drops.set(Some(NonNull::from(self.jobs.alloc(entry))));
        }
        self.submit(job.cast(), units);
    }

    fn next_ordinal(&self) -> u32 {
        self.forked.replace(self.forked.get() + 1)
    }

    /// Moves `job` into scope job storage.
    fn store<T>(&self, job: T) -> NonNull<T> {
        let slot = self.jobs.alloc_layout(Layout::new::<T>()).cast::<T>();
        // SAFETY: the arena returned memory that is aligned and large enough for `T`,
        // unaliased, and valid until the arena is reset when this scope closes.
        unsafe { slot.write(job) };
        slot
    }

    /// Counts `units` on the latch and makes them available to this worker and to thieves.
    fn submit(&self, job: NonNull<ChildHeader>, units: Range<u32>) {
        self.core.latch.add(units.len());
        let entry = Entry::Child { scope: NonNull::from(&self.core), job, units };
        match self.local.shared.replay() {
            Some(replay) => self.local.shared.replay_wake(replay.push(entry)),
            None => self.local.push(entry),
        }
    }

    /// Runs this scope's children until none is left on this worker, then waits for the
    /// ones other workers took.
    ///
    /// The opener finds its children by popping: while the scope is open nothing else is
    /// pushed onto its deque, so they are the topmost entries, and the first entry with
    /// another tag (older work, from before the scope opened) means none is left here.
    fn wait(&self, child_scratch: &mut Arena) {
        let local = self.local;
        let shared = &*local.shared;
        let pool = local.pool();
        let own = NonNull::from(&self.core);
        let latch = &self.core.latch;

        if let Some(replay) = shared.replay() {
            // The log decides when this worker runs each of its children.
            while !latch.is_set() {
                let (taken, wake) = replay.take(local.index, Filter::Scope(own));
                shared.replay_wake(wake);
                match taken {
                    Some((Entry::Child { scope, job, units }, _)) => {
                        run_child(local, scope, job, units, child_scratch);
                        replay.finished();
                    }
                    Some(_) => unreachable!("the filter admits only this scope's children"),
                    None => latch.park(),
                }
            }
            return;
        }

        /// The opener's side of [`gate::wait_for_children`].
        struct Children<'a> {
            local: &'a WorkerLocal,
            own: NonNull<ScopeCore>,
            scratch: &'a mut Arena,
        }

        impl Opener for Children<'_> {
            fn run_children(&mut self) -> bool {
                let (local, pool) = (self.local, self.local.pool());
                // A `block_in_place` may want this worker parked like any other: then its
                // children stay on the deque for the reserved workers. `run_child` puts
                // the rest of a range back under the same condition.
                while !pool.over_limit() || self.shutting_down() {
                    match local.deque.pop() {
                        Some(Entry::Child { scope, job, units }) if scope == self.own => {
                            run_child(local, scope, job, units, self.scratch);
                        }
                        Some(older) => {
                            // It was off the deque for a moment; a worker that looked
                            // then may have gone to sleep.
                            local.push(older);
                            return false;
                        }
                        None => return false,
                    }
                }
                !local.deque.is_empty()
            }

            fn shutting_down(&self) -> bool {
                self.local.shared.is_shut_down()
            }

            fn spin(&mut self, latch: &Latch) -> bool {
                spin_until(LATCH_SPIN, || latch.is_set())
            }
        }

        let mut children = Children { local, own, scratch: child_scratch };
        pool.wait_for_children(latch, local.index, &mut children);
    }

    /// Drops the closures of this scope's range jobs. Every child has finished.
    fn drop_range_jobs(&self) {
        let mut next = self.drops.take();
        while let Some(entry) = next {
            // SAFETY: entries are written by `fork_range` into scope job storage, which is
            // not reset until after this call.
            let entry = unsafe { entry.read() };
            // SAFETY: the entry names a `RangeJob` in the same storage and the function
            // instantiated for its type. The latch is set, so no worker uses the job any
            // more, and each entry is visited once.
            unsafe { (entry.drop)(entry.job) };
            next = entry.next;
        }
    }
}

/// Runs `units` of a scope child on this worker, which is the opener waiting on the scope or
/// a worker that took the entry from a deque, then counts them down on the scope's latch.
pub(crate) fn run_child(
    local: &WorkerLocal,
    scope: NonNull<ScopeCore>,
    job: NonNull<ChildHeader>,
    units: Range<u32>,
    scratch: &mut Arena,
) {
    // SAFETY: the entry carrying these pointers exists, so the latch still counts `units`
    // and the opener has not returned from `JobContext::scope`: the scope and the job
    // storage are alive. These fields are written before any entry exists and never change.
    let (opener, tag, position, call, ordinal) = unsafe {
        let core = scope.as_ptr();
        (
            (*core).opener,
            (*core).tag,
            (*core).position,
            (*job.as_ptr()).call,
            (*job.as_ptr()).ordinal,
        )
    };
    let shared = &*local.shared;
    let _tag = tag.scope();
    let slot = local.index as u32;

    let mut run = |units: Range<u32>| {
        let mut ctx = ChildContext { scratch: &*scratch, slot };
        // SAFETY: `job` is the live job `call` belongs to, and `units` come from an entry
        // this worker owns, so no other call receives them.
        unsafe { call(job, units, &mut ctx) };
        scratch.reset();
    };
    let done = if shared.replay().is_some() {
        // The log named exactly these units as one call.
        let done = units.len() as u32;
        run(units);
        done
    } else {
        let recorder = shared.chaos.as_ref().filter(|chaos| chaos.recording());
        drain_range(
            local,
            units,
            |upper| local.push(Entry::Child { scope, job, units: upper }),
            |units| {
                if let Some(chaos) = recorder {
                    chaos.record(LogRecord {
                        worker: slot,
                        kind: JobKind::Child,
                        key: position,
                        index: ordinal,
                        units: units.clone(),
                    });
                }
                run(units);
            },
            // A `block_in_place` wants this worker parked: the rest goes back on the
            // deque for the reserved workers. Not at shutdown, when every child left is run
            // by whoever holds it.
            || {
                let yields = local.pool().over_limit() && !shared.is_shut_down();
                if yields { Rest::Requeue } else { Rest::Run }
            },
        )
    };

    // SAFETY: the units counted down here are still outstanding, so the scope is alive for
    // this call. The reference covers only the latch, which is all atomics, and if this is
    // the decrement that reaches zero the latch makes no access after it.
    let last = unsafe { (*scope.as_ptr()).latch.count_down(done as usize) };
    // From here the scope may be gone. The opener is woken through the pool, which is not.
    if last && opener != local.index {
        local.pool().unpark(opener);
    }
}

/// Handed to every child job of a scope.
pub struct ChildContext<'job> {
    scratch: &'job Arena,
    slot: u32,
}

impl<'job> ChildContext<'job> {
    /// Scratch memory for this child job, valid until the child returns. Reset, and
    /// poisoned in debug builds, between jobs. Distinct from the opening job's scratch even
    /// when the opener runs the child itself.
    #[must_use]
    pub fn scratch(&self) -> &'job Arena {
        self.scratch
    }

    /// The slot this child runs under. See [`JobContext::slot`]; the same warning applies.
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.slot
    }
}
