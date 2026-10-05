//! The work-stealing deque: `crossbeam-deque`, a vetted dependency.
//!
//! Under Miri a mutex-backed stand-in with the same interface takes its place. Miri's
//! default aliasing model (Stacked Borrows) rejects `crossbeam-epoch`'s intrusive list, which
//! is a known limitation upstream tests around with other Miri settings and says nothing
//! about this crate. The stand-in lets the plain `cargo miri test` check everything this
//! crate does with the entries it moves through a deque, under the strict model. The real
//! deque is exercised natively by every test and by the stress test.

#[cfg(not(miri))]
pub(crate) use crossbeam_deque::{Injector, Steal, Stealer, Worker};

#[cfg(miri)]
pub(crate) use stand_in::{Injector, Steal, Stealer, Worker};

#[cfg(miri)]
mod stand_in {
    use core::cell::Cell;
    use core::marker::PhantomData;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use crate::lock;

    pub(crate) enum Steal<T> {
        Empty,
        Success(T),
        /// Never produced here; kept so callers handle what the real deque returns.
        #[allow(dead_code, reason = "mirrors crossbeam_deque::Steal")]
        Retry,
    }

    /// The owner's end: last in, first out.
    pub(crate) struct Worker<T> {
        queue: Arc<Mutex<VecDeque<T>>>,
        /// `Send` but not `Sync`, like the real one.
        owner_only: PhantomData<Cell<()>>,
    }

    impl<T> Worker<T> {
        pub(crate) fn new_lifo() -> Self {
            Self { queue: Arc::default(), owner_only: PhantomData }
        }

        pub(crate) fn stealer(&self) -> Stealer<T> {
            Stealer { queue: self.queue.clone() }
        }

        pub(crate) fn push(&self, value: T) {
            lock(&self.queue).push_back(value);
        }

        pub(crate) fn pop(&self) -> Option<T> {
            lock(&self.queue).pop_back()
        }

        pub(crate) fn is_empty(&self) -> bool {
            lock(&self.queue).is_empty()
        }
    }

    /// A thief's end: first in, first out.
    pub(crate) struct Stealer<T> {
        queue: Arc<Mutex<VecDeque<T>>>,
    }

    impl<T> Stealer<T> {
        pub(crate) fn steal(&self) -> Steal<T> {
            lock(&self.queue).pop_front().map_or(Steal::Empty, Steal::Success)
        }

        pub(crate) fn is_empty(&self) -> bool {
            lock(&self.queue).is_empty()
        }
    }

    /// The shared queue: first in, first out.
    pub(crate) struct Injector<T> {
        queue: Mutex<VecDeque<T>>,
    }

    impl<T> Injector<T> {
        pub(crate) fn new() -> Self {
            Self { queue: Mutex::default() }
        }

        pub(crate) fn push(&self, value: T) {
            lock(&self.queue).push_back(value);
        }

        pub(crate) fn steal(&self) -> Steal<T> {
            lock(&self.queue).pop_front().map_or(Steal::Empty, Steal::Success)
        }

        pub(crate) fn is_empty(&self) -> bool {
            lock(&self.queue).is_empty()
        }
    }
}
