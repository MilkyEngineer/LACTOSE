//! The plumbing between a request's future and the backend that completes it.
//!
//! Hidden and changeable: nothing here is part of the contract. The public futures in
//! `file.rs` and `io.rs` each wrap a [`Pending`], and the contract they state is what
//! `README.md` says, not how this module keeps it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::task::{Context, Poll, Waker};

use lactose_core::{Error, ErrorKind, Result};

use crate::backend::Backend;
use crate::file::{FileId, OpenFile};
use crate::span;
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
        match self {
            Self::Open { path, id, .. } => span::open(path, *id),
            Self::Read { file, offset, len, .. } => span::read(file.id, *offset, *len, None),
            Self::ReadEncoded { file, offset, len, encoding, .. } => {
                span::read(file.id, *offset, *len, Some(*encoding))
            }
            Self::ReadFile { path, .. } => span::read_file(path),
            Self::Replace { path, bytes, .. } => span::replace(path, bytes.len() as u64),
            Self::ReadDir { path, .. } => span::read_dir(path),
            Self::CreateDirAll { path, .. } => span::create_dir_all(path),
        }
    }

    /// The span the first poll created, for the backend to enter while it runs the request;
    /// a disabled span if there is none.
    pub(crate) fn current_span(&self) -> tracing::Span {
        let span = match self {
            Self::Open { complete, .. } => complete.span.get(),
            Self::Read { complete, .. } | Self::ReadFile { complete, .. } => complete.span.get(),
            Self::ReadEncoded { complete, .. } => complete.span.get(),
            Self::Replace { complete, .. } => complete.span.get(),
            Self::ReadDir { complete, .. } => complete.span.get(),
            Self::CreateDirAll { complete, .. } => complete.span.get(),
        };
        span.cloned().unwrap_or_else(tracing::Span::none)
    }

    /// Whether the future that made this request has been dropped, read by the backend
    /// before it starts the work so a cancelled request never runs.
    pub(crate) fn is_cancelled(&self) -> bool {
        match self {
            Self::Open { complete, .. } => complete.is_cancelled(),
            Self::Read { complete, .. } | Self::ReadFile { complete, .. } => {
                complete.is_cancelled()
            }
            Self::ReadEncoded { complete, .. } => complete.is_cancelled(),
            Self::Replace { complete, .. } => complete.is_cancelled(),
            Self::ReadDir { complete, .. } => complete.is_cancelled(),
            Self::CreateDirAll { complete, .. } => complete.is_cancelled(),
        }
    }

    /// The path whose replaces this request is ordered with, for a replace: the backend
    /// starts it only after every replace of an equal path submitted before it has finished.
    /// `None` for every other request.
    pub(crate) fn replace_path(&self) -> Option<&Path> {
        match self {
            Self::Replace { path, .. } => Some(path),
            _ => None,
        }
    }

    /// Fails the request without running it: a stopping backend completes what it still
    /// holds with `Cancelled`, a replace handing its bytes back in its [`ReplaceFailed`].
    pub(crate) fn cancel(self) {
        let stopped = || Error::new(ErrorKind::Cancelled, "the IO pool has stopped");
        match self {
            Self::Open { complete, .. } => complete.complete(Err(stopped())),
            Self::Read { complete, .. } | Self::ReadFile { complete, .. } => {
                complete.complete(Err(stopped()));
            }
            Self::ReadEncoded { complete, .. } => complete.complete(Err(stopped())),
            Self::Replace { bytes, complete, .. } => {
                complete.complete(Err(ReplaceFailed { error: stopped(), bytes }));
            }
            Self::ReadDir { complete, .. } => complete.complete(Err(stopped())),
            Self::CreateDirAll { complete, .. } => complete.complete(Err(stopped())),
        }
    }
}

/// What the backend hands an encoded read's future.
pub(crate) enum Encoded {
    /// The stored frame, read and checked (one frame, not skippable, ending at the range's
    /// end, its content size `decoded_len`), and an empty buffer reserved for exactly
    /// `decoded_len` bytes, both allocated on the pool thread under `AllocTag::Io`. The
    /// future decodes the one into the other in its own poll (architecture section 5.1:
    /// asset decode is background work).
    Stored { stored: Vec<u8>, decoded: Vec<u8>, decoded_len: usize },
    /// Bytes a backend with a hardware decoder decoded itself, whole, as the request. The
    /// blocking pool has none.
    #[expect(dead_code, reason = "the variant a hardware-decoding backend completes with")]
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

/// Records a failure's kind on its request's span, the one field every span carries.
pub(crate) fn record_error(span: &tracing::Span, error: &Error) {
    span.record("error", tracing::field::debug(error.kind()));
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

impl<T, E> Completion<T, E> {
    /// The state. A poisoned lock is taken as it is: panics abort outside tests, and every
    /// update under the lock is a single assignment that leaves the state consistent.
    fn lock(&self) -> MutexGuard<'_, State<T, E>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
        if let (Err(failure), Some(span)) = (&result, self.span.get()) {
            record_error(span, failure.error());
        }
        let (discarded, waker) = {
            let mut state = self.lock();
            if state.cancelled {
                (Some(result), None)
            } else {
                state.result = Some(result);
                (None, state.waker.take())
            }
        };
        drop(discarded);
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.lock().cancelled
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
    /// The backend the request goes to; `None` once submitted or for a request decided
    /// before it was made.
    backend: Option<Arc<dyn Backend>>,
    /// Whether a poll has returned the result; polling again is a broken invariant.
    resolved: bool,
}

impl<T, E: Failure> Pending<T, E> {
    /// A request `backend` will run, completing `completion`, which `request` also holds.
    pub(crate) fn new(
        backend: Arc<dyn Backend>,
        completion: Arc<Completion<T, E>>,
        request: Request,
    ) -> Self {
        Self { request: Some(request), completion, backend: Some(backend), resolved: false }
    }

    /// A request decided before it was made (a zero-length read): resolves to `result` at
    /// the first poll, which still creates the span from `request` and records an error on
    /// it, and submits nothing.
    pub(crate) fn decided(
        completion: Arc<Completion<T, E>>,
        request: Request,
        result: Result<T, E>,
    ) -> Self {
        completion.lock().result = Some(result);
        Self { request: Some(request), completion, backend: None, resolved: false }
    }

    /// A request that failed before it was made (a range above the cap, an overflowing
    /// range, an empty encoded range): resolves to `error` at the first poll, which still
    /// creates the span from `request` and records `error` on it, and submits nothing.
    pub(crate) fn failed(completion: Arc<Completion<T, E>>, request: Request, error: E) -> Self {
        Self::decided(completion, request, Err(error))
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
        let this = self.get_mut();
        assert!(!this.resolved, "an IO request's future was polled after it resolved");
        if let Some(request) = this.request.take() {
            let span = this.completion.span.get_or_init(|| request.span());
            match this.backend.take() {
                Some(backend) => backend.submit(request),
                None => {
                    if let Some(Err(failure)) = &this.completion.lock().result {
                        record_error(span, failure.error());
                    }
                    drop(request);
                }
            }
        }
        let mut state = this.completion.lock();
        if let Some(result) = state.result.take() {
            drop(state);
            this.resolved = true;
            return Poll::Ready(result);
        }
        match &mut state.waker {
            Some(waker) if waker.will_wake(cx.waker()) => {}
            slot => *slot = Some(cx.waker().clone()),
        }
        Poll::Pending
    }
}

impl<T, E> Drop for Pending<T, E> {
    fn drop(&mut self) {
        // Taken under the lock and dropped after it: a result the backend left here is freed
        // on this thread, and the request, if never submitted, after this body.
        let (result, waker) = {
            let mut state = self.completion.lock();
            state.cancelled = true;
            (state.result.take(), state.waker.take())
        };
        drop((result, waker));
    }
}
