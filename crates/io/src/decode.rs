//! The decode of an encoded read, run in the read's own future (architecture section 5.1:
//! asset decode is background work, and background CPU work yields every 0.5 ms).
//!
//! Hidden and changeable. The pool reads the stored range and checks the frame before
//! anything is decoded or the decoded buffer allocated (`backend/blocking.rs`,
//! `read_encoded_at`); this module only turns checked stored bytes into decoded ones, in
//! slices, on whichever thread polls the future.

use std::task::{Context, Poll};
use std::time::Duration;

use lactose_core::Result;

/// How long one poll decodes before it yields: section 5.1's slice for background CPU work.
pub(crate) const SLICE: Duration = Duration::from_micros(500);

/// A streaming decode of one checked zstd frame into a buffer reserved for exactly its
/// `decoded_len`.
///
/// With `zstd` as a dependency it also holds the stream's context
/// (`zstd::zstd_safe::DCtx<'static>`, `Send + Sync`), created on the first slice.
pub(crate) struct Decode {
    /// The stored frame, freed when the decode ends or the future is dropped.
    stored: Vec<u8>,
    /// The output, empty with capacity `decoded_len`; never grown.
    decoded: Vec<u8>,
    /// Stored bytes the stream has consumed.
    consumed: usize,
    /// The read's span, entered for each slice.
    span: tracing::Span,
}

impl Decode {
    pub(crate) fn new(stored: Vec<u8>, decoded: Vec<u8>, span: tracing::Span) -> Self {
        Self { stored, decoded, consumed: 0, span }
    }

    /// Decodes until the frame ends or [`SLICE`] has passed, inside the span and under an
    /// `AllocTag::Io` scope so the stream's own memory is charged to `Io`.
    ///
    /// `Ready(Ok)` with exactly `decoded_len` bytes once the stream reports the frame
    /// complete with every stored byte consumed and the output full. `Ready(Err)` of kind
    /// `InvalidData` for every error the stream reports (mapped explicitly from its code,
    /// never through `From<std::io::Error>`), for output that would pass `decoded_len`,
    /// and for a frame that ends with stored bytes left over or the output short. Otherwise
    /// it wakes `cx`'s waker and returns `Pending`, which puts a task behind the other ready
    /// tasks of its class.
    pub(crate) fn poll_slice(&mut self, cx: &mut Context<'_>) -> Poll<Result<Vec<u8>>> {
        todo!(
            "decompress_stream over `stored[consumed..]` into the spare capacity of \
             `decoded`, in output steps of about 64 KiB, checking the clock between steps"
        )
    }
}
