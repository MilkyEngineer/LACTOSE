//! File IO (architecture section 4, `io`): whole files and positional ranges read into
//! caller-owned buffers, a range decoded in the same request, the atomic replace of
//! section 15.3, and directory listing, each as a `Send + 'static` future completed by a
//! pool of platform threads of class `ThreadClass::Io` (section 9.7). SILO reads packs
//! through it, `app` writes saves through it, the cooker reads sources and writes outputs
//! through it. `README.md` is the contract: it states what the signatures cannot, and
//! what a caller may rely on.
//!
//! Foundation tier, outside the determinism boundary: the simulation never calls it, and
//! nothing it returns reaches the simulation except as bytes a loader or the save path
//! validated.
//!
//! # Status
//!
//! A facade: every public signature is final and every body that decides anything is
//! `todo!()`. [`IoConfig::for_topology`] and the span helpers are real. The
//! implementation lane replaces the bodies behind this surface and removes the
//! crate-level `allow` with the last `todo!()`.
//!
//! # Where to start
//!
//! - [`Io`] is the pool, started once by `app` from the executor's platform with an
//!   [`IoConfig`] and cloned into whatever submits requests. [`Io::open`],
//!   [`Io::read_file`], [`Io::replace`], [`Io::read_dir`] and [`Io::create_dir_all`] are
//!   the requests that take a path.
//! - [`File`] is an open file; [`File::read_at`] and [`File::read_encoded_at`] are the
//!   positional reads a pack is served by, the second decoding its range as one request.
//! - [`Open`], [`Read`], [`ReadFile`], [`Replace`], [`ReadDir`] and [`CreateDirAll`] are
//!   the futures. Each submits at its first poll, resolves to all of what was asked or an
//!   error, and cancels when dropped.
//!
//! ```no_run
//! use lactose_io::{Encoding, Io, IoConfig};
//! use lactose_task::{Executor, WorkerClass};
//!
//! # fn main() -> lactose_core::Result<()> {
//! let executor = Executor::builder().build()?;
//! let platform = executor.platform().as_ref();
//! let io = Io::start(platform, IoConfig::for_platform(platform))?;
//!
//! // SILO: a pack opened once, then a range read and decoded in one request on the pool.
//! let pack = executor.block_on(io.open("cooked/world.pack"))?;
//! let range = pack.read_encoded_at(4096, 1500, Encoding::Zstd { decoded_len: 8192 });
//! let bytes = executor.block_on(range)?;
//! assert_eq!(bytes.len(), 8192);
//!
//! // The save: temporary file beside the slot, sync, rename, as one request, from a task.
//! let save: Vec<u8> = Vec::new();
//! let written = executor.spawn(WorkerClass::Background, io.replace("saves/slot0.carton", save));
//! executor.block_on(written)??;
//!
//! io.shutdown();
//! executor.shutdown();
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every body is `todo!()` until the implementation lane replaces it, and \
              the fields and parameters it will use are declared now"
)]

mod backend;
mod config;
mod file;
mod io;
mod request;
mod span;
mod types;

pub use config::IoConfig;
pub use file::{File, FileId, Read};
pub use io::{CreateDirAll, Io, Open, ReadDir, ReadFile, Replace};
pub use types::{DirEntry, Encoding, EntryKind, MAX_READ_BYTES};

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the pool handle and an open file are shared across threads
/// by every consumer; every future is `Send + 'static` so `Executor::spawn` takes it and
/// `Unpin` so a caller polls it without pinning; each resolves to exactly what its method
/// documents; the plain data is `Copy`; and the cap is the number the README states.
const _: () = {
    const fn send_sync<T: Send + Sync + 'static>() {}
    const fn future<F: Future<Output = T> + Send + Sync + Unpin + 'static, T>() {}
    const fn copy<T: Copy>() {}

    send_sync::<Io>();
    send_sync::<File>();
    send_sync::<IoConfig>();
    send_sync::<DirEntry>();

    future::<Open, lactose_core::Result<File>>();
    future::<Read, lactose_core::Result<Vec<u8>>>();
    future::<ReadFile, lactose_core::Result<Vec<u8>>>();
    future::<Replace, lactose_core::Result<()>>();
    future::<ReadDir, lactose_core::Result<Vec<DirEntry>>>();
    future::<CreateDirAll, lactose_core::Result<()>>();

    copy::<IoConfig>();
    copy::<FileId>();
    copy::<Encoding>();
    copy::<EntryKind>();

    assert!(MAX_READ_BYTES == 1 << 30);
};
