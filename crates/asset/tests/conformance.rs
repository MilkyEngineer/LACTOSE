//! The conformance suite of `lactose-asset-api` (architecture section 20) against the
//! store: the harness that turns the suite's `provide` into a pack on disk and a mount,
//! and its `settle` into [`Store::settled`].

#![forbid(unsafe_code)]

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use common::format::{RawEntry, index, seal};
use common::{LIMIT, wait_on};
use lactose_asset::Store;
use lactose_asset_api::conformance::{self, Harness};
use lactose_asset_api::{AssetCatalog, AssetId, AssetVersion, CookedAsset, ErasedLoader};
use lactose_io::{Io, IoConfig};
use lactose_task::Executor;

/// A store over a directory of its own, on the executor and the pool every check shares.
struct OnDisk {
    executor: Executor,
    io: Io,
    store: Store,
    dir: PathBuf,
    zstd: bool,
    /// Packs written so far: each `provide` is one pack, mounted after every earlier one.
    packs: AtomicU64,
}

impl OnDisk {
    fn new(
        executor: &Executor,
        io: &Io,
        dir: PathBuf,
        zstd: bool,
        loaders: Vec<ErasedLoader>,
    ) -> Self {
        let created = wait_on(executor, io.create_dir_all(dir.clone()), LIMIT);
        created.expect("the check's directory");
        let store = Store::new(executor.clone(), io.clone(), loaders).expect("one loader per kind");
        Self {
            executor: executor.clone(),
            io: io.clone(),
            store,
            dir,
            zstd,
            packs: AtomicU64::new(0),
        }
    }
}

impl Harness for OnDisk {
    fn catalog(&self) -> &dyn AssetCatalog {
        &self.store
    }

    /// "As a cook that published it would": a pack of the one asset, written whole and
    /// mounted, which makes it the newest pack that holds `id`. The suite calls this from
    /// one thread at a time, so the mounts' places are in call order.
    fn provide(&self, id: AssetId, version: AssetVersion, cooked: CookedAsset) {
        let bytes = pack_of(id, version, &cooked, self.zstd);
        let number = self.packs.fetch_add(1, Ordering::Relaxed);
        let path = self.dir.join(format!("{number:08}.pack"));
        let written = wait_on(&self.executor, self.io.replace(path.clone(), bytes), LIMIT);
        written.expect("the pack written");
        wait_on(&self.executor, self.store.mount(path), LIMIT).expect("the pack mounted");
    }

    fn settle(&self) {
        wait_on(&self.executor, self.store.settled(), LIMIT);
    }
}

/// A pack of the one entry `id` at `version`, written by the tests' own writer from
/// `README.md`, "The pack format". The index claims `version` whether or not the content
/// hashes to it, as after damage to a payload. With `zstd` the payload is one zstd frame
/// that carries its content size, however short the bytes: `PackBuilder` stores bytes raw
/// where a frame would be no shorter, which every asset of the suite is.
fn pack_of(id: AssetId, version: AssetVersion, cooked: &CookedAsset, zstd: bool) -> Vec<u8> {
    let (payload, storage) = if zstd {
        let mut compressor = zstd::bulk::Compressor::new(3).expect("a compressor");
        compressor.include_contentsize(true).expect("a frame that carries its size");
        (compressor.compress(&cooked.bytes).expect("a frame"), 1)
    } else {
        (cooked.bytes.clone(), 0)
    };
    let header = &cooked.header;
    let entry = RawEntry {
        id: id.to_bits(),
        version: version.to_bits(),
        kind: header.kind().0,
        format: header.format_version(),
        dependencies: header.dependencies().iter().map(|id| id.to_bits()).collect(),
        storage,
        offset: 0,
        stored: payload.len() as u64,
        decoded: cooked.bytes.len() as u64,
    };
    seal(&payload, &index(1, &[entry]))
}

#[test]
fn the_store_conforms_over_raw_and_compressed_packs() {
    let executor = Executor::builder().build().expect("an executor");
    let platform = executor.platform().as_ref();
    let io = Io::start(platform, IoConfig::for_platform(platform)).expect("the pool");
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("conformance");

    for (name, zstd) in [("raw", false), ("zstd", true)] {
        let checks = AtomicU64::new(0);
        conformance::run(|loaders| {
            let check = checks.fetch_add(1, Ordering::Relaxed);
            OnDisk::new(&executor, &io, root.join(format!("{name}-{check}")), zstd, loaders)
        });
    }

    io.shutdown();
    executor.shutdown();
}
