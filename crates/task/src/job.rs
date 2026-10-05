//! What a running job can do: fork-join over borrowed data, scratch memory, and the one
//! sanctioned blocking wait.

use core::marker::PhantomData;
use core::ops::Range;

use lactose_core::Arena;

/// Handed to every job.
pub struct JobContext<'job> {
    marker: PhantomData<&'job mut ()>,
}

impl<'job> JobContext<'job> {
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
    pub fn scope<'env, F, R>(&mut self, f: F) -> R
    where
        F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> R,
    {
        todo!()
    }

    /// Scratch memory for this job, valid until the job returns. Reset, and poisoned in
    /// debug builds, between jobs.
    ///
    /// The borrow is independent of the context, so a job can allocate per-child buffers
    /// here and hand them to children forked in a later [`JobContext::scope`].
    #[must_use]
    pub fn scratch(&self) -> &'job Arena {
        todo!()
    }

    /// A dense index below [`crate::Executor::slot_count`]. At most one job or child job
    /// runs under a given slot at any moment.
    ///
    /// The slot is **not a semantic key**. It depends on scheduling and differs between
    /// runs. Its one legitimate use is addressing per-slot partial storage that a
    /// [`crate::GraphNode`] merges in `finalize` with an order-independent combiner.
    /// `lactose-ecs` uses it that way and never passes it to a system.
    #[must_use]
    pub fn slot(&self) -> u32 {
        todo!()
    }

    /// Runs `f`, which blocks this worker on threads the executor does not own, and returns
    /// its result. This is the one sanctioned blocking wait on a worker; the physics step
    /// uses it to call into the solver's pool (architecture section 16).
    ///
    /// While `f` runs, at most `reserved_workers` other frame-critical workers take jobs
    /// and the rest stay parked, so the foreign pool and the executor together do not
    /// oversubscribe the performance cores. The caller sizes the foreign pool as
    /// [`crate::Executor::frame_worker_count`] minus `reserved_workers`.
    pub fn block_in_place<R>(&mut self, reserved_workers: u32, f: impl FnOnce() -> R) -> R {
        todo!()
    }
}

/// A fork-join scope. See [`JobContext::scope`].
///
/// The lifetimes follow `std::thread::scope`: `'scope` is invariant and is the lifetime of
/// the scope itself, and `'env` is what forked closures may borrow. Unlike
/// `std::thread::Scope` this type is neither `Send` nor `Sync`.
pub struct Scope<'scope, 'env: 'scope> {
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
        todo!()
    }

    /// Queues `f` over `units` as one child job that is halved each time it is stolen. `f`
    /// is called on disjoint ranges that together cover `units` exactly once.
    pub fn fork_range<F>(&'scope self, units: Range<u32>, f: F)
    where
        F: Fn(Range<u32>, &mut ChildContext<'_>) + Send + Sync + 'scope,
    {
        todo!()
    }
}

/// Handed to every child job of a scope.
pub struct ChildContext<'job> {
    marker: PhantomData<&'job mut ()>,
}

impl<'job> ChildContext<'job> {
    /// Scratch memory for this child job, valid until the child returns. Reset, and
    /// poisoned in debug builds, between jobs. Distinct from the opening job's scratch even
    /// when the opener runs the child itself.
    #[must_use]
    pub fn scratch(&self) -> &'job Arena {
        todo!()
    }

    /// The slot this child runs under. See [`JobContext::slot`]; the same warning applies.
    #[must_use]
    pub fn slot(&self) -> u32 {
        todo!()
    }
}
