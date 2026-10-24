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

/// The largest frame window a decode accepts, as a base-2 logarithm: 2^27 bytes, zstd's
/// own default made explicit. A frame with a larger window is `InvalidData`; the cooker
/// never compresses with a window log above it.
pub(crate) const WINDOW_LOG_MAX: u32 = 27;

/// The output one step of a slice gives the stream, about 64 KiB, between two looks at the
/// clock.
pub(crate) const STEP: usize = 64 << 10;

/// A streaming decode of one checked zstd frame into a buffer reserved for exactly its
/// `decoded_len`.
///
/// With `zstd` as a dependency it also holds the stream's context
/// (`zstd::zstd_safe::DCtx<'static>`, `Send + Sync`), created on the first slice and given
/// `set_parameter(DParameter::WindowLogMax(WINDOW_LOG_MAX))`, so the bound on a frame's
/// window is this crate's and not zstd's default.
///
/// Streaming in output steps makes zstd decode through a window buffer of its own: up to
/// the smaller of the frame's window and its content size, and at most
/// `2^WINDOW_LOG_MAX` bytes. zstd allocates it, and the context, with its own `malloc`,
/// outside the global allocator, so neither is charged to `AllocTag::Io` and neither is
/// one of this crate's fallible allocations; both are freed when the decode ends or the
/// future is dropped.
pub(crate) struct Decode {
    /// The stored frame, freed when the decode ends or the future is dropped.
    stored: Vec<u8>,
    /// The output: capacity `decoded_len`, reserved by the pool; its length is how far it
    /// has been zero-filled for the stream to write into, never past the capacity.
    decoded: Vec<u8>,
    /// Stored bytes the stream has consumed.
    consumed: usize,
    /// Decoded bytes the stream has written, at most `decoded.len()`.
    written: usize,
    /// The read's span, entered for each slice.
    span: tracing::Span,
}

impl Decode {
    pub(crate) fn new(stored: Vec<u8>, decoded: Vec<u8>, span: tracing::Span) -> Self {
        Self { stored, decoded, consumed: 0, written: 0, span }
    }

    /// Decodes until the frame ends or [`SLICE`] has passed, inside the span.
    ///
    /// Each step zero-fills `decoded` up to `end = min(written + STEP, decoded_len)` (the
    /// capacity is already reserved, so this never allocates) and calls `decompress_stream`
    /// with `OutBuffer::around_pos(&mut decoded[..end], written)` and an `InBuffer` over
    /// `stored` at `consumed`, then checks the clock. The output given to one call is never
    /// the whole spare capacity: with output room for the frame's whole content and the
    /// whole frame as input, zstd takes its single-pass shortcut and decodes the frame in
    /// one call however long it is, the very wait this slicing exists to cut. Writing into
    /// the spare capacity without zero-filling needs zstd-safe's `unsafe` `WriteBuf`, which
    /// this crate forbids.
    ///
    /// `Ready(Ok)` with exactly `decoded_len` bytes once the stream reports the frame
    /// complete (`decompress_stream` returns zero) with every stored byte consumed and the
    /// output full. `Ready(Err)` with zstd's errors mapped explicitly from its code, never
    /// through `From<std::io::Error>`: `ResourceExhausted` for zstd's memory-allocation
    /// error (`ZSTD_error_memory_allocation`, 64, which zstd returns as the code
    /// `0usize.wrapping_sub(64)`), since zstd's own `malloc` failed; `InvalidData` for every
    /// other error the stream reports (a window above `2^WINDOW_LOG_MAX` among them), for
    /// output that would pass `decoded_len`, and for a frame that ends with stored bytes
    /// left over or the output short. Otherwise it wakes `cx`'s waker and returns
    /// `Pending`, which puts a task behind the other ready tasks of its class.
    pub(crate) fn poll_slice(&mut self, cx: &mut Context<'_>) -> Poll<Result<Vec<u8>>> {
        todo!(
            "zero-fill `decoded` by STEP, decompress_stream into \
             `OutBuffer::around_pos(&mut decoded[..end], written)`, check the clock, repeat"
        )
    }
}
