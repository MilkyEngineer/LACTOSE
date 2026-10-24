//! How a request's work is done, behind the futures.
//!
//! Hidden and changeable. Architecture section 20: "`io` has no `-api`; its backends sit
//! behind a trait inside the crate." [`Backend`] is that trait, and [`blocking`] its one
//! implementation: a pool of platform threads of class `ThreadClass::Io`, each taking one
//! request at a time from a FIFO queue and running it as ordinary blocking `std::fs`
//! calls, with the replaces of one path held back in submission order. No backend
//! decodes on its own threads except through a hardware decoder: an encoded read's
//! decode runs in its future (`decode.rs`). Section 4 names io_uring (Linux) and IOCP
//! (Windows) as the later backends; each would be its own module here, chosen at
//! `Io::start`, submitting a request to the kernel and completing it from a completion
//! queue instead of a pool thread. The futures, their contract and the `Pending` plumbing
//! do not change for that; `SAFETY.md` says where that `unsafe` lives and what it relies
//! on.

pub(crate) mod blocking;

use crate::request::Request;

/// What runs requests. One per [`Io`](crate::Io), shared by its handles and its open
/// files.
pub(crate) trait Backend: Send + Sync + 'static {
    /// Takes `request` and completes it, now or later, on some thread of the backend's.
    /// A replace starts only after every replace of an equal path submitted before it has
    /// finished, without a thread of the backend's waiting for it. A stopped backend
    /// completes the request with `Cancelled` at once.
    fn submit(&self, request: Request);

    /// Stops taking requests: everything queued and not started completes `Cancelled`,
    /// everything started finishes. Returns without waiting. Idempotent.
    fn stop(&self);

    /// Waits for every thread of the backend to exit; [`Backend::stop`] first. Idempotent.
    fn join(&self);
}
