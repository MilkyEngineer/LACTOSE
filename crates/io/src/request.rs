//! The plumbing between a request's future and the backend that completes it.
//!
//! Hidden and changeable: nothing here is part of the contract. The public futures in
//! `file.rs` and `io.rs` each wrap a [`Pending`], and the contract they state is what
//! `README.md` says, not how this module keeps it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use lactose_core::{Error, Result};

use crate::backend::Backend;
use crate::file::{FileId, OpenFile};
use crate::types::{DirEntry, Encoding, ReplaceFailed};

/// One request as data, for the backend to run: what to do and the [`Completion`] to hand
/// the result to. Carries everything the work needs, so the backend owns it until it
/// completes and nothing of the caller's is borrowed.
pub(crate) enum Request {
    Open {
        path: PathBuf,
        id: FileId,
        complete: Arc<Completion<Arc<OpenFile>>>,
    },
    Read {
        file: Arc<OpenFile>,
        offset: u64,
        len: u64,
        complete: Arc<Completion<Vec<u8>>>,
    },
    /// The stored range of an encoded read: the backend reads and checks the frame and
    /// reserves the decoded buffer; the decode itself is the future's (see [`Encoded`]).
    ReadEncoded {
        file: Arc<OpenFile>,
        offset: u64,
        len: u64,
        encoding: Encoding,
        complete: Arc<Completion<Encoded>>,
    },
    ReadFile {
        path: PathBuf,
        complete: Arc<Completion<Vec<u8>>>,
    },
    Replace {
        path: PathBuf,
        bytes: Vec<u8>,
        complete: Arc<Completion<(), ReplaceFailed>>,
    },
    ReadDir {
        path: PathBuf,
        complete: Arc<Completion<Vec<DirEntry>>>,
    },
    CreateDirAll {
        path: PathBuf,
        complete: Arc<Completion<()>>,
    },
}

impl Request {
    /// The request's span, made from its own fields by the variant's constructor in
    /// `span.rs`. Called once, by the first poll, before the request is submitted.
    pub(crate) fn span(&self) -> tracing::Span {
        todo!("the variant's `span::*` constructor over its fields")
    }

    /// Whether the future that made this request has been dropped, read by the backend
    /// before it starts the work so a cancelled request never runs.
    pub(crate) fn is_cancelled(&self) -> bool {
        todo!("the variant's completion's flag")
    }

    /// The path whose replaces this request is ordered with, for a replace: the backend
    /// starts it only after every replace of an equal path submitted before it has finished.
    /// `None` for every other request.
    pub(crate) fn replace_path(&self) -> Option<&Path> {
        todo!("`Some(path)` for `Replace`")
    }

    /// Fails the request without running it: a stopping backend completes what it still
    /// holds with `Cancelled`, a replace handing its bytes back in its [`ReplaceFailed`].
    pub(crate) fn cancel(self) {
        todo!("complete the variant's completion with ErrorKind::Cancelled")
    }
}

/// What the backend hands an encoded read's future.
pub(crate) enum Encoded {
    /// The stored frame, read and checked (one frame, not skippable, ending at the range's
    /// end, its content size `decoded_len`), and an empty buffer reserved for exactly
    /// `decoded_len` bytes, both allocated on the pool thread under `AllocTag::Io`. The
    /// future decodes the one into the other in its own poll (architecture section 5.1:
    /// asset decode is background work).
    Stored { stored: Vec<u8>, decoded: Vec<u8> },
    /// Bytes a backend with a hardware decoder decoded itself, whole, as the request.
    Decoded(Vec<u8>),
}

/// What a failed result carries for its span and for cancellation: the error itself, or a
/// replace's error beside the bytes it hands back.
pub(crate) trait Failure: Send + 'static {
    fn error(&self) -> &Error;
}

impl Failure for Error {
    fn error(&self) -> &Error {
        self
    }
}

impl Failure for ReplaceFailed {
    fn error(&self) -> &Error {
        &self.error
    }
}

/// What a request's future and its backend share.
pub(crate) struct Completion<T, E = Error> {
    state: Mutex<State<T, E>>,
    /// The request's span: set by the first poll before the request is submitted, entered
    /// by the backend for the work, closed when the last of the future and the backend lets
    /// go of the completion. A future dropped unpolled leaves it unset, so no span exists.
    pub(crate) span: OnceLock<tracing::Span>,
}

/// The request's progress, under the completion's lock.
struct State<T, E> {
    /// The result, once the backend has it; taken by the one poll that returns it.
    result: Option<Result<T, E>>,
    /// The waker of the last poll, woken when the result is set.
    waker: Option<Waker>,
    /// Set when the future is dropped. A backend that has not started the request skips
    /// it; one that has finishes and discards the result.
    cancelled: bool,
}

impl<T, E: Failure> Completion<T, E> {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State { result: None, waker: None, cancelled: false }),
            span: OnceLock::new(),
        })
    }

    /// Called by the backend with the request's result; records `error` on the span for a
    /// failure and wakes the future.
    ///
    /// The waker is taken under the lock and woken after it is released, and a result
    /// discarded because the future was dropped is dropped after it too. A wake can run the
    /// task inline (the executor does when it races its own shutdown), and that may drop
    /// the future and re-enter [`Pending`]'s `drop` on this same lock.
    pub(crate) fn complete(&self, result: Result<T, E>) {
        todo!(
            "record the span's error; under the lock store the result unless cancelled and \
             take the waker; release the lock; then drop a discarded result and wake"
        )
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        todo!("read the flag under the lock")
    }
}

/// One request as its future sees it: the request until it is submitted, the completion
/// after.
///
/// The request is submitted to the backend at the first poll, which also creates its span,
/// so an unpolled future costs nothing beyond its own allocation. Dropping it marks the
/// completion cancelled and never blocks.
pub(crate) struct Pending<T, E = Error> {
    /// The request, until the first poll hands it to the backend.
    request: Option<Request>,
    /// Where the backend puts the result.
    completion: Arc<Completion<T, E>>,
    /// The backend the request goes to; `None` once submitted or for a request that
    /// failed before it was made.
    backend: Option<Arc<dyn Backend>>,
}

impl<T, E: Failure> Pending<T, E> {
    /// A request `backend` will run, completing `completion`, which `request` also holds.
    pub(crate) fn new(
        backend: Arc<dyn Backend>,
        completion: Arc<Completion<T, E>>,
        request: Request,
    ) -> Self {
        Self { request: Some(request), completion, backend: Some(backend) }
    }

    /// A request that failed before it was made (a range above the cap, an overflowing
    /// range, an empty encoded range): resolves to `error` at the first poll, which still
    /// creates the span from `request` and records `error` on it, and submits nothing.
    pub(crate) fn failed(completion: Arc<Completion<T, E>>, request: Request, error: E) -> Self {
        todo!("store `Err(error)` in the completion; keep `request` for its span; no backend")
    }

    /// The request's span once the first poll has created it, for an encoded read's
    /// decode slices to enter.
    pub(crate) fn span(&self) -> Option<&tracing::Span> {
        self.completion.span.get()
    }
}

impl<T, E: Failure> Future for Pending<T, E> {
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<T, E>> {
        todo!(
            "on the first poll set the completion's span from the request and submit it; \
             then return the result or park the waker"
        )
    }
}

impl<T, E> Drop for Pending<T, E> {
    fn drop(&mut self) {
        todo!("mark the completion cancelled; a request never submitted is dropped here")
    }
}
