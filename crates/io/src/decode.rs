//! The decode of an encoded read, run in the read's own future (architecture section 5.1:
//! asset decode is background work, and background CPU work yields every 0.5 ms), and the
//! frame checks the pool makes before it allocates the decoded buffer.
//!
//! Hidden and changeable. The pool reads the stored range and calls [`check_frame`] before
//! anything is decoded or the decoded buffer allocated (`backend/blocking.rs`,
//! `read_encoded_at`); [`Decode`] only turns checked stored bytes into decoded ones, in
//! slices, on whichever thread polls the future.

use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use lactose_core::{Error, ErrorKind, Result};
use zstd::zstd_safe::{self, DCtx, DParameter, InBuffer, OutBuffer};

use crate::request::record_error;

/// How long one poll decodes before it yields: section 5.1's slice for background CPU work.
pub(crate) const SLICE: Duration = Duration::from_micros(500);

/// The largest frame window a decode accepts, as a base-2 logarithm: 2^27 bytes, zstd's
/// own default made explicit. A frame with a larger window is `InvalidData`; the cooker
/// never compresses with a window log above it.
pub(crate) const WINDOW_LOG_MAX: u32 = 27;

/// The most output, and the most stored input, one step of a slice gives the stream: about
/// 64 KiB each, between two looks at the clock.
pub(crate) const STEP: usize = 64 << 10;

/// zstd's frame magic number, little-endian at the start of every frame that is not
/// skippable.
const FRAME_MAGIC: u32 = 0xFD2F_B528;

/// The code zstd returns when its own `malloc` fails: `ZSTD_error_memory_allocation`, 64,
/// returned as `0usize.wrapping_sub(64)`.
pub(crate) const MEMORY_ALLOCATION: usize = 0usize.wrapping_sub(64);

/// The pool's checks of a stored range before the decoded buffer is allocated, each failure
/// `InvalidData`: the first four bytes are zstd's frame magic, so the frame is not a
/// skippable one; `find_frame_compressed_size` is the whole range, so it is exactly one
/// frame, not truncated, with nothing after it (a second, concatenated frame included);
/// and `get_frame_content_size` is present and equal to `decoded_len`. Every zstd error
/// here is `InvalidData` explicitly, never through `From<std::io::Error>`.
pub(crate) fn check_frame(stored: &[u8], decoded_len: u64) -> Result<()> {
    let invalid = |message: String| Error::new(ErrorKind::InvalidData, message);
    let magic = stored.first_chunk::<4>().map(|bytes| u32::from_le_bytes(*bytes));
    if magic != Some(FRAME_MAGIC) {
        return Err(invalid("the range is not a zstd frame, or is a skippable one".into()));
    }
    match zstd_safe::find_frame_compressed_size(stored) {
        Ok(size) if size == stored.len() => {}
        Ok(size) => {
            return Err(invalid(format!(
                "the frame is {size} of the range's {} bytes",
                stored.len()
            )));
        }
        Err(code) => {
            return Err(invalid(format!(
                "the frame is malformed: {}",
                zstd_safe::get_error_name(code)
            )));
        }
    }
    match zstd_safe::get_frame_content_size(stored) {
        Ok(Some(size)) if size == decoded_len => Ok(()),
        Ok(Some(size)) => {
            Err(invalid(format!("the frame decodes to {size} bytes, not the {decoded_len} asked")))
        }
        Ok(None) => Err(invalid("the frame does not record its content size".into())),
        Err(_) => Err(invalid("the frame header is corrupt".into())),
    }
}

/// A zstd error as a request's, from its code: `ResourceExhausted` when zstd's own memory
/// allocation failed, `InvalidData` for every other.
pub(crate) fn zstd_error(code: usize) -> Error {
    let name = zstd_safe::get_error_name(code);
    if code == MEMORY_ALLOCATION {
        Error::new(ErrorKind::ResourceExhausted, format!("zstd could not allocate: {name}"))
    } else {
        Error::new(ErrorKind::InvalidData, format!("the frame does not decode: {name}"))
    }
}

/// A streaming decode of one checked zstd frame into a buffer reserved for exactly its
/// `decoded_len`.
///
/// It also holds the stream's context (`DCtx<'static>`, `Send + Sync`), created on the
/// first slice and given `WindowLogMax(WINDOW_LOG_MAX)`, so the bound on a frame's window
/// is this crate's and not zstd's default.
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
    /// The exact length the frame decodes to.
    decoded_len: usize,
    /// Stored bytes the stream has consumed.
    consumed: usize,
    /// Decoded bytes the stream has written, at most `decoded.len()`.
    written: usize,
    /// The stream, from the first slice until the decode ends.
    context: Option<DCtx<'static>>,
    /// The read's span, entered for each slice.
    span: tracing::Span,
}

impl Decode {
    pub(crate) fn new(
        stored: Vec<u8>,
        decoded: Vec<u8>,
        decoded_len: usize,
        span: tracing::Span,
    ) -> Self {
        debug_assert!(decoded.capacity() >= decoded_len && decoded.is_empty());
        Self { stored, decoded, decoded_len, consumed: 0, written: 0, context: None, span }
    }

    /// Decodes until the frame ends or [`SLICE`] has passed, inside the span.
    ///
    /// Each step zero-fills `decoded` up to `end = min(written + STEP, decoded_len)` (the
    /// capacity is already reserved, so this never allocates) and calls `decompress_stream`
    /// with `OutBuffer::around_pos(&mut decoded[..end], written)` and an `InBuffer` over
    /// `stored[..min(consumed + STEP, len)]` at `consumed`, then checks the clock. Both
    /// sides are bounded, because one call's time grows with either. With output room for
    /// the frame's whole content and the whole frame as input, zstd takes its single-pass
    /// shortcut and decodes the frame in one call however long it is. With the whole frame
    /// as input and only the output bounded, one call consumes every block that writes
    /// nothing (a frame of millions of empty blocks) or little (blocks of one byte each),
    /// so its time grows with the stored size. Either is the very wait this slicing exists
    /// to cut. Writing into the spare capacity without zero-filling needs zstd-safe's
    /// `unsafe` `WriteBuf`, which this crate forbids.
    ///
    /// `Ready(Ok)` with exactly `decoded_len` bytes once the stream reports the frame
    /// complete with every stored byte consumed and the output full. `Ready(Err)` with
    /// [`zstd_error`]'s mapping of the stream's errors (a window above `2^WINDOW_LOG_MAX`
    /// among them), and `InvalidData` for a frame that ends with stored bytes left over or
    /// the output short, or that would write past `decoded_len`. Otherwise it wakes `cx`'s
    /// waker and returns `Pending`, which puts a task behind the other ready tasks of its
    /// class.
    pub(crate) fn poll_slice(&mut self, cx: &mut Context<'_>) -> Poll<Result<Vec<u8>>> {
        self.poll_observed(cx, &mut |_, _| {})
    }

    /// [`Decode::poll_slice`], telling `observe` the bytes each `decompress_stream` call
    /// wrote and the stored bytes it consumed, in that order.
    fn poll_observed(
        &mut self,
        cx: &mut Context<'_>,
        observe: &mut dyn FnMut(usize, usize),
    ) -> Poll<Result<Vec<u8>>> {
        let _entered = self.span.clone().entered();
        let result = self.slice(observe);
        match result {
            None => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Some(result) => {
                if let Err(error) = &result {
                    record_error(&self.span, error);
                }
                // Both buffers but the result, and zstd's context, are freed here.
                self.stored = Vec::new();
                self.context = None;
                Poll::Ready(result)
            }
        }
    }

    /// One slice: `None` if it ran out of time before the frame ended.
    fn slice(&mut self, observe: &mut dyn FnMut(usize, usize)) -> Option<Result<Vec<u8>>> {
        let start = Instant::now();
        let context = match &mut self.context {
            Some(context) => context,
            empty => match new_context() {
                Ok(context) => empty.insert(context),
                Err(error) => return Some(Err(error)),
            },
        };
        let invalid =
            |message: &'static str| Some(Err(Error::new(ErrorKind::InvalidData, message)));
        loop {
            let end = self.decoded_len.min(self.written + STEP);
            if self.decoded.len() < end {
                self.decoded.resize(end, 0);
            }
            let before = (self.written, self.consumed);
            let mut output = OutBuffer::around_pos(&mut self.decoded[..end], self.written);
            let mut input =
                InBuffer::around(&self.stored[..self.stored.len().min(self.consumed + STEP)]);
            input.set_pos(self.consumed);
            let remaining = match context.decompress_stream(&mut output, &mut input) {
                Ok(remaining) => remaining,
                Err(code) => return Some(Err(zstd_error(code))),
            };
            self.written = output.pos();
            self.consumed = input.pos();
            observe(self.written - before.0, self.consumed - before.1);
            if remaining == 0 {
                if self.consumed != self.stored.len() {
                    return invalid("the frame ends before the stored range does");
                }
                if self.written != self.decoded_len {
                    return invalid("the frame decodes to fewer bytes than its content size");
                }
                return Some(Ok(std::mem::take(&mut self.decoded)));
            }
            if (self.written, self.consumed) == before {
                // No progress with room on both sides is a stream that wants more than the
                // range holds; with the output full, a frame that decodes past its length.
                return if self.written == self.decoded_len {
                    invalid("the frame decodes to more bytes than its content size")
                } else {
                    invalid("the frame is truncated")
                };
            }
            if start.elapsed() >= SLICE {
                return None;
            }
        }
    }
}

/// A decompression context with this crate's window bound.
fn new_context() -> Result<DCtx<'static>> {
    let mut context = DCtx::try_create().ok_or_else(|| zstd_error(MEMORY_ALLOCATION))?;
    context.set_parameter(DParameter::WindowLogMax(WINDOW_LOG_MAX)).map_err(zstd_error)?;
    Ok(context)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::task::{Wake, Waker};

    use super::*;

    struct Count(std::sync::atomic::AtomicUsize);

    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[test]
    fn zstd_allocation_failure_is_resource_exhausted_and_every_other_code_invalid_data() {
        assert_eq!(
            zstd_safe::get_error_name(MEMORY_ALLOCATION),
            "Allocation error : not enough memory"
        );
        assert_eq!(zstd_error(MEMORY_ALLOCATION).kind(), ErrorKind::ResourceExhausted);
        for code in 1..=120usize {
            let code = 0usize.wrapping_sub(code);
            if code != MEMORY_ALLOCATION {
                assert_eq!(zstd_error(code).kind(), ErrorKind::InvalidData, "{code}");
            }
        }
    }

    /// Every call a whole decode of `stored` makes, as (written, consumed), and the
    /// decoded bytes.
    struct Observed {
        decoded: Vec<u8>,
        calls: Vec<(usize, usize)>,
        polls: usize,
        wakes: usize,
    }

    fn decode_observed(stored: Vec<u8>, decoded_len: usize) -> Observed {
        check_frame(&stored, decoded_len as u64).unwrap();
        let mut decoded = Vec::new();
        decoded.try_reserve_exact(decoded_len).unwrap();
        let mut decode = Decode::new(stored, decoded, decoded_len, tracing::Span::none());
        let wakes = Arc::new(Count(Default::default()));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut cx = Context::from_waker(&waker);
        let (mut calls, mut polls) = (Vec::new(), 0usize);
        let decoded = loop {
            polls += 1;
            let mut observe = |written: usize, consumed: usize| calls.push((written, consumed));
            if let Poll::Ready(result) = decode.poll_observed(&mut cx, &mut observe) {
                break result.unwrap();
            }
        };
        let wakes = wakes.0.load(std::sync::atomic::Ordering::Relaxed);
        Observed { decoded, calls, polls, wakes }
    }

    /// A single-segment frame by hand with a four-byte content size, made of `blocks`, each
    /// a block type (0 raw, 2 compressed) and its body; the last is flagged last here.
    fn frame_of_blocks(content_size: u32, blocks: &[(u32, &[u8])]) -> Vec<u8> {
        let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, 0xA0];
        frame.extend_from_slice(&content_size.to_le_bytes());
        for (i, &(kind, body)) in blocks.iter().enumerate() {
            let last = u32::from(i + 1 == blocks.len());
            let header = last | (kind << 1) | ((body.len() as u32) << 3);
            frame.extend_from_slice(&header.to_le_bytes()[..3]);
            frame.extend_from_slice(body);
        }
        frame
    }

    #[test]
    fn a_step_never_writes_more_than_its_share_of_output() {
        let content = crate::tests::text(6 << 20, 7);
        let stored = zstd::bulk::compress(&content, 3).unwrap();
        let observed = decode_observed(stored, content.len());
        assert!(observed.decoded == content, "the decode differs from what was compressed");
        assert_eq!(observed.decoded.capacity(), content.len());
        let largest = observed.calls.iter().map(|call| call.0).max().unwrap();
        assert!(largest <= STEP, "one call wrote {largest} bytes");
        let calls = observed.calls.len();
        assert!(calls >= content.len() / STEP, "{calls} calls for {} bytes", content.len());
        assert!(observed.polls > 1, "the whole frame decoded in one poll");
        assert_eq!(observed.wakes, observed.polls - 1);
    }

    /// Blocks that write nothing cost a call time without filling its output, so only a
    /// bound on the input keeps one call short: 200 000 empty raw blocks are about 600 KB
    /// that one unbounded call would take whole.
    #[test]
    fn a_step_never_consumes_more_than_its_share_of_a_frame_of_empty_blocks() {
        let content = crate::tests::text(1000, 3);
        let mut blocks = vec![(0, &[][..]); 200_000];
        blocks.push((0, &content[..]));
        let stored = frame_of_blocks(1000, &blocks);
        let stored_len = stored.len();
        let observed = decode_observed(stored, content.len());
        assert!(observed.decoded == content);
        let largest = observed.calls.iter().map(|call| call.1).max().unwrap();
        assert!(largest <= STEP, "one call consumed {largest} of {stored_len} stored bytes");
        let consumed: usize = observed.calls.iter().map(|call| call.1).sum();
        assert_eq!(consumed, stored_len);
        let calls = observed.calls.len();
        assert!(calls >= stored_len / STEP, "{calls} calls for {stored_len} stored bytes");
    }

    /// Blocks of one decoded byte each, raw (four stored bytes per decoded byte) and
    /// compressed (one raw literal and no sequences, six): a 64 KiB output step alone would
    /// take a quarter or a sixth of a MiB of blocks in one call.
    #[test]
    fn a_step_never_consumes_more_than_its_share_of_a_frame_of_one_byte_blocks() {
        let content = crate::tests::text(200_000, 9);
        let literals: Vec<[u8; 3]> = content.iter().map(|&byte| [1 << 3, byte, 0]).collect();
        let raw: Vec<_> = content.chunks(1).map(|byte| (0, byte)).collect();
        let compressed: Vec<_> = literals.iter().map(|body| (2, &body[..])).collect();
        let raw = frame_of_blocks(200_000, &raw);
        let compressed = frame_of_blocks(200_000, &compressed);
        for (name, stored) in [("raw", raw), ("compressed", compressed)] {
            let stored_len = stored.len();
            let observed = decode_observed(stored, content.len());
            assert!(observed.decoded == content, "{name}");
            let written = observed.calls.iter().map(|call| call.0).max().unwrap();
            let consumed = observed.calls.iter().map(|call| call.1).max().unwrap();
            assert!(written <= STEP, "{name}: one call wrote {written} bytes");
            assert!(consumed <= STEP, "{name}: one call consumed {consumed} of {stored_len}");
            let calls = observed.calls.len();
            assert!(calls >= stored_len / STEP, "{name}: {calls} calls for {stored_len} bytes");
        }
    }
}
