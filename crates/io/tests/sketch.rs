//! Compile-only consumer sketches against the facade: what `app` writes to start the pool
//! and to save a slot, what SILO writes to read a pack range and hand it to a loader, what
//! the cooker writes to read sources and write outputs, and what the server writes to
//! publish a region. Type-checked and never run (every deciding body of the crate is
//! `todo!()`), except the one test, which runs the real `IoConfig::for_platform` against
//! the standard platform. They pin the surface the README promises: a change that breaks
//! one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use core::num::NonZero;
use std::path::{Path, PathBuf};

use lactose_asset_api::{AssetId, AssetVersion, CookedHeader, ErasedLoader, UntypedHandle};
use lactose_core::{Error, ErrorKind, Result};
use lactose_io::{DirEntry, Encoding, EntryKind, File, Io, IoConfig, ReadAt, ReplaceFailed};
use lactose_task::{Executor, Platform, StdPlatform, Task, WorkerClass};

// ---- 1. app: the pool, started once from the executor's platform ----

/// Half the cores, the rule of architecture section 9.7, on client and server alike.
fn app_start(executor: &Executor) -> Result<Io> {
    let platform = executor.platform().as_ref();
    Io::start(platform, IoConfig::for_platform(platform))
}

/// A count `app` chose itself, from a platform layer or a setting.
fn app_start_with(platform: &dyn Platform, pool_threads: NonZero<u32>) -> Result<Io> {
    Io::start(platform, IoConfig::new(pool_threads))
}

/// The save directory exists before the first save, so a `NotFound` from a replace is a
/// failure to report, not a step to retry.
async fn app_prepare_saves(io: &Io, save_dir: &Path) -> Result<()> {
    io.create_dir_all(save_dir).await
}

/// The pool stops before the executor does.
fn app_shutdown(io: Io, executor: Executor) {
    io.shutdown();
    executor.shutdown();
}

// ---- 2. app and persist: the save written and read back ----

/// What `app` does with a failed save, chosen from the kind and never from the message.
/// `DiskFull` keeps the bytes, so the save is written again once the player frees space,
/// without assembling it again.
enum SaveOutcome {
    Saved,
    DiskFull(Vec<u8>),
    NoSaveDirectory,
    ShuttingDown,
    Failed(Error),
}

/// The bytes `Container::assemble` produced, written as one request: temporary file
/// beside the slot, sync, rename. A failure hands the bytes back.
///
/// `StorageFull` and `NotFound` come only before the rename, so the slot still holds the
/// previous save. `Io` may come after it, from the directory sync, with the new save in
/// place and its durability unknown, or before it with the previous save kept; either
/// way writing the same bytes once more is the remedy, and replaces of one slot land in
/// submission order, so the retry is the last to land.
async fn app_write_save(io: &Io, slot: PathBuf, bytes: Vec<u8>) -> SaveOutcome {
    let failed = match io.replace(slot.clone(), bytes).await {
        Ok(()) => return SaveOutcome::Saved,
        Err(failed) => failed,
    };
    let failed = match failed.error.kind() {
        ErrorKind::StorageFull => return SaveOutcome::DiskFull(failed.bytes),
        ErrorKind::NotFound => return SaveOutcome::NoSaveDirectory,
        ErrorKind::Cancelled => return SaveOutcome::ShuttingDown,
        ErrorKind::Io => match io.replace(slot, failed.bytes).await {
            Ok(()) => return SaveOutcome::Saved,
            Err(failed) => failed,
        },
        _ => failed,
    };
    SaveOutcome::Failed(failed.error)
}

/// A caller that only reports the failure converts it into its error with `?`.
async fn app_write_or_report(io: &Io, slot: PathBuf, bytes: Vec<u8>) -> Result<()> {
    io.replace(slot, bytes).await?;
    Ok(())
}

/// The load: the preamble first, then the index it names, then one section per region,
/// each a positional read on the one open slot. Several sections in flight at once are
/// several tasks, since a future submits at its first poll.
async fn app_read_save_preamble(
    io: &Io,
    slot: &Path,
    preamble_bytes: u64,
) -> Result<(File, Vec<u8>)> {
    let file = io.open(slot).await?;
    let preamble = file.read_at(0, preamble_bytes).await?;
    Ok((file, preamble))
}

fn app_read_save_sections(
    executor: &Executor,
    slot: &File,
    sections: &[(u64, u64)],
) -> Vec<Task<Result<Vec<u8>>>> {
    sections
        .iter()
        .map(|&(offset, len)| executor.spawn(WorkerClass::Background, slot.read_at(offset, len)))
        .collect()
}

/// The slot listing: every `.carton` in the save directory, the `.tmp` a crashed replace
/// may have left skipped.
async fn app_list_slots(io: &Io, save_dir: &Path) -> Result<Vec<PathBuf>> {
    let entries: Vec<DirEntry> = io.read_dir(save_dir).await?;
    Ok(entries
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .map(|entry| save_dir.join(entry.name))
        .filter(|path| path.extension().is_some_and(|ext| ext == "carton"))
        .collect())
}

// ---- 3. SILO: a pack opened once, ranges read by the pool and decoded by their futures ----

/// Where one asset's cooked bytes lie in a pack and how they are stored, as the store's
/// catalog will hold it from the pack index.
struct PackEntry {
    offset: u64,
    stored_len: u64,
    decoded_len: u64,
    compressed: bool,
}

/// The pack is opened once for the session and its index read from a fixed-size trailer;
/// the handle is cloned into every load.
async fn silo_open_pack(io: &Io, path: &Path, trailer_len: u64) -> Result<(File, Vec<u8>)> {
    let pack = io.open(path).await?;
    let trailer = pack.read_at(pack.size().saturating_sub(trailer_len), trailer_len).await?;
    Ok((pack, trailer))
}

/// One load: the range, decoded as part of the same request when it is stored compressed,
/// in this task's own polls on the background worker that runs it, then the loader over
/// the decoded bytes on the same worker. The buffer is the store's and is dropped here
/// once the loader has built the asset.
async fn silo_load(
    pack: File,
    entry: PackEntry,
    loader: &ErasedLoader,
    id: AssetId,
    version: AssetVersion,
    header: &CookedHeader,
) -> Result<UntypedHandle> {
    let bytes = if entry.compressed {
        let encoding = Encoding::Zstd { decoded_len: entry.decoded_len };
        pack.read_encoded_at(entry.offset, entry.stored_len, encoding).await?
    } else {
        pack.read_at(entry.offset, entry.stored_len).await?
    };
    loader.load(id, version, header, &bytes)
}

/// The store spawns a load as a background task; the task owns its clone of the handle,
/// so the descriptor stays open for as long as a read on it is in flight.
fn silo_spawn_read(executor: &Executor, pack: &File, entry: &PackEntry) -> Task<Result<Vec<u8>>> {
    let read: ReadAt = pack.read_at(entry.offset, entry.stored_len);
    executor.spawn(WorkerClass::Background, read)
}

/// A load no longer wanted drops its task: the read inside is dropped, one not yet
/// started never runs, one started finishes on the pool and frees its buffer there.
fn silo_cancel(load: Task<Result<Vec<u8>>>) {
    drop(load);
}

/// A corrupt index entry fails as a value before anything is allocated.
async fn silo_bad_index(pack: &File) {
    let error = pack.read_at(0, lactose_io::MAX_READ_BYTES + 1).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceExhausted);
    let error = pack.read_at(pack.size(), 1).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

// ---- 4. The cooker: the same futures, driven from the main thread ----

fn cook(source: &[u8]) -> Vec<u8> {
    source.to_vec()
}

/// Sources listed and read, outputs written whole, each request completed before the
/// next through `Executor::block_on`; there is no second, blocking API.
fn cook_directory(executor: &Executor, io: &Io, sources: &Path, outputs: &Path) -> Result<()> {
    executor.block_on(io.create_dir_all(outputs))?;
    let entries = executor.block_on(io.read_dir(sources))?;
    for entry in entries.iter().filter(|entry| entry.kind == EntryKind::File) {
        let source = executor.block_on(io.read_file(sources.join(&entry.name)))?;
        let cooked = cook(&source);
        let output = outputs.join(&entry.name).with_extension("cooked");
        executor.block_on(io.replace(output, cooked))?;
    }
    Ok(())
}

// ---- 5. The server: a region publish that never waits behind another ----

/// The container bytes a background worker assembled, written by the pool while the
/// worker goes on to the next region; `pool_threads` of these run at once. A failed
/// publish keeps its bytes for the retry.
fn server_publish_region(
    executor: &Executor,
    io: &Io,
    path: PathBuf,
    bytes: Vec<u8>,
) -> Task<Result<(), ReplaceFailed>> {
    executor.spawn(WorkerClass::Background, io.replace(path, bytes))
}

/// The rule is real: at least one thread, and never more than the cores the platform
/// reports.
#[test]
#[cfg_attr(miri, ignore = "queries the operating system for the topology")]
fn the_default_pool_fits_the_platform() {
    let topology = StdPlatform.topology();
    let threads = IoConfig::for_platform(&StdPlatform).pool_threads.get();
    assert!(threads >= 1);
    assert!(threads <= (topology.performance + topology.efficiency).max(1));
}
