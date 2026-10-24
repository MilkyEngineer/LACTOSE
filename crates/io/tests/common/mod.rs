//! What the integration tests share: a scratch directory, compressible content, and a poll
//! loop that turns a hang into a failure. Each test asserts on counts and orderings; the
//! bounded waits never decide an outcome.

#![allow(dead_code, reason = "each test binary uses part of the shared support")]

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use core::num::NonZero;

use lactose_io::{Io, IoConfig};
use lactose_task::StdPlatform;

/// How long a test waits for something that must happen before it calls it a hang.
pub const LIMIT: Duration = Duration::from_secs(60);

/// A directory of its own under the system's temporary directory, removed when dropped.
pub struct Scratch {
    pub root: PathBuf,
}

impl Scratch {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let name = format!(
            "lactose-io-test-{}-{unique}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// The names in the directory, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// `len` bytes of words from a small vocabulary, picked by a fixed generator: content zstd
/// compresses about as well as source text.
pub fn text(len: usize, seed: u64) -> Vec<u8> {
    const WORDS: &[&[u8]] = &[
        b"curd ",
        b"whey ",
        b"rennet ",
        b"brine ",
        b"rind ",
        b"culture ",
        b"press ",
        b"mould ",
        b"aged ",
        b"salt ",
        b"vat ",
        b"wheel ",
        b"\n",
        b"0x3f, ",
        b"{ ",
        b"} ",
        b"fn ",
        b"let ",
    ];
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(len + 16);
    while out.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(WORDS[(state % WORDS.len() as u64) as usize]);
    }
    out.truncate(len);
    out
}

/// A pool of `count` threads on the standard platform.
pub fn start(count: u32) -> Io {
    Io::start(&StdPlatform, IoConfig::new(NonZero::new(count).unwrap())).unwrap()
}

struct Unpark(std::thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Drives `future` to its end on this thread, failing the test if it takes [`LIMIT`].
pub fn wait<F: Future + Unpin>(mut future: F) -> F::Output {
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let deadline = Instant::now() + LIMIT;
    loop {
        if let Poll::Ready(output) = Pin::new(&mut future).poll(&mut cx) {
            return output;
        }
        let now = Instant::now();
        assert!(now < deadline, "a request did not resolve within {LIMIT:?}");
        std::thread::park_timeout(deadline - now);
    }
}

/// Polls `future` once with `waker`.
pub fn poll_with<F: Future + Unpin>(future: &mut F, waker: &Waker) -> Poll<F::Output> {
    Pin::new(future).poll(&mut Context::from_waker(waker))
}

/// Polls `future` once, with a waker that does nothing: a first poll submits the request.
pub fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    poll_with(future, Waker::noop())
}

/// One zstd frame of `content` with its content size, as the cooker writes it.
pub fn frame(content: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(content, 3).unwrap()
}
