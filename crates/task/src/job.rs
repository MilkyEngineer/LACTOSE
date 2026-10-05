//! What a running job can do: fork-join over borrowed data, and scratch memory.

use core::marker::PhantomData;
use core::ops::Range;

use lactose_core::Arena;

/// Handed to every job. It exposes no worker, chunk or job index: nothing a job can observe
/// through it depends on scheduling.
pub struct JobContext<'job> {
    marker: PhantomData<&'job mut ()>,
}

impl JobContext<'_> {
    /// Runs `f`, which may fork child jobs that borrow from the caller's stack, and returns
    /// once every child has finished.
    ///
    /// While waiting, the calling worker runs only jobs of this scope, then spins briefly,
    /// then parks on the scope's latch. It never runs unrelated graph jobs or background
    /// work. Children are stored in the worker's bump arena, which is reset when the scope
    /// closes.
    ///
    /// Taking `&mut self` makes nesting a compile error: a job opens scopes one after
    /// another, never one inside another, and children receive a [`ChildContext`] that
    /// cannot open scopes at all.
    pub fn scope<'env, F, R>(&mut self, f: F) -> R
    where
        F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> R,
    {
        todo!()
    }

    /// Scratch memory for this job. Reset, and poisoned in debug builds, between jobs.
    #[must_use]
    pub fn scratch(&self) -> &Arena {
        todo!()
    }
}

/// A fork-join scope. See [`JobContext::scope`].
///
/// The lifetimes follow `std::thread::scope`: `'scope` is invariant and is the lifetime of
/// the scope itself, and `'env` is what forked closures may borrow.
pub struct Scope<'scope, 'env: 'scope> {
    scope: PhantomData<&'scope mut &'scope ()>,
    env: PhantomData<&'env mut &'env ()>,
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

impl ChildContext<'_> {
    /// Scratch memory for this child job. Reset, and poisoned in debug builds, between jobs.
    #[must_use]
    pub fn scratch(&self) -> &Arena {
        todo!()
    }
}
