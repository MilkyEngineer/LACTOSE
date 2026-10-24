//! The plumbing between a request's future and the backend that completes it.
//!
//! Hidden and changeable: nothing here is part of the contract. The public futures in
//! `file.rs` and `io.rs` each wrap a [`Pending`], and the contract they state is what
//! `README.md` says, not how this module keeps it.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use lactose_core::Result;

use crate::backend::Backend;
use crate::file::{FileId, OpenFile};
use crate::types::{DirEntry, Encoding};

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
        encoding: Option<Encoding>,
        complete: Arc<Completion<Vec<u8>>>,
    },
    ReadFile {
        path: PathBuf,
        complete: Arc<Completion<Vec<u8>>>,
    },
    Replace {
        path: PathBuf,
        bytes: Vec<u8>,
        complete: Arc<Completion<()>>,
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
    /// Whether the future that made this request has been dropped, read by the backend
    /// before it starts the work so a cancelled request never runs.
    pub(crate) fn is_cancelled(&self) -> bool {
        todo!("the variant's completion's flag")
    }

    /// Fails the request without running it: a stopping backend completes what it still
    /// holds with `Cancelled`.
    pub(crate) fn cancel(self) {
        todo!("complete the variant's completion with ErrorKind::Cancelled")
    }
}

/// What a request's future and its backend share.
pub(crate) struct Completion<T> {
    state: Mutex<State<T>>,
    /// The request's span: created with the future, entered by the backend for the work,
    /// closed when the last of the future and the backend lets go of the completion.
    pub(crate) span: tracing::Span,
}

/// The request's progress, under the completion's lock.
struct State<T> {
    /// The result, once the backend has it; taken by the one poll that returns it.
    result: Option<Result<T>>,
    /// The waker of the last poll, woken when the result is set.
    waker: Option<Waker>,
    /// Set when the future is dropped. A backend that has not started the request skips
    /// it; one that has finishes and discards the result.
    cancelled: bool,
}

impl<T> Completion<T> {
    pub(crate) fn new(span: tracing::Span) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State { result: None, waker: None, cancelled: false }),
            span,
        })
    }

    /// Called by the backend with the request's result; records `error` on the span for a
    /// failure and wakes the future.
    pub(crate) fn complete(&self, result: Result<T>) {
        todo!("store the result unless cancelled, record the span's error field, wake")
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        todo!("read the flag under the lock")
    }
}

/// One request as its future sees it: the request until it is submitted, the completion
/// after.
///
/// The request is submitted to the backend at the first poll, so an unpolled future
/// costs nothing beyond its own allocation. Dropping it marks the completion cancelled
/// and never blocks.
pub(crate) struct Pending<T> {
    /// The request, until the first poll hands it to the backend.
    request: Option<Request>,
    /// Where the backend puts the result.
    completion: Arc<Completion<T>>,
    /// The backend the request goes to; `None` once submitted or for a request that
    /// failed before it was made.
    backend: Option<Arc<dyn Backend>>,
}

impl<T> Pending<T> {
    /// A request `backend` will run, completing `completion`, which `request` also holds.
    pub(crate) fn new(
        backend: Arc<dyn Backend>,
        completion: Arc<Completion<T>>,
        request: Request,
    ) -> Self {
        Self { request: Some(request), completion, backend: Some(backend) }
    }

    /// A request that failed before it was made (a range above the cap, a backend that
    /// has stopped): resolves to `error` at the first poll and submits nothing.
    pub(crate) fn failed(completion: Arc<Completion<T>>, error: lactose_core::Error) -> Self {
        todo!("store `Err(error)` in the completion; no request, no backend")
    }
}

impl<T> Future for Pending<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<T>> {
        todo!("submit the request on the first poll, then return the result or park the waker")
    }
}

impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        todo!("mark the completion cancelled; a request never submitted is dropped here")
    }
}
