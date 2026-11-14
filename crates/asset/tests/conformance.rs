//! The conformance suite of `lactose-asset-api` (architecture section 20) against the
//! store: the harness that turns the suite's `provide` into a pack on disk and a mount,
//! and its `settle` into [`Store::settled`].

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lactose_asset::{PackBuilder, Storage, Store};
use lactose_asset_api::conformance::{self, Harness};
use lactose_asset_api::{AssetCatalog, AssetId, AssetVersion, CookedAsset, ErasedLoader};
use lactose_core::xxh64;
use lactose_io::{Io, IoConfig};
use lactose_task::Executor;

/// A store over a directory of its own, on the executor and the pool every check shares.
struct OnDisk {
    executor: Executor,
    io: Io,
    store: Store,
    dir: PathBuf,
    storage: Storage,
    /// Packs written so far: each `provide` is one pack, mounted after every earlier one.
    packs: AtomicU64,
}

impl OnDisk {
    fn new(
        executor: &Executor,
        io: &Io,
        dir: PathBuf,
        storage: Storage,
        loaders: Vec<ErasedLoader>,
    ) -> Self {
        executor.block_on(io.create_dir_all(dir.clone())).expect("the check's directory");
        let store = Store::new(executor.clone(), io.clone(), loaders).expect("one loader per kind");
        Self {
            executor: executor.clone(),
            io: io.clone(),
            store,
            dir,
            storage,
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
        let mut pack = PackBuilder::new();
        let actual = pack.add(id, &cooked, self.storage).expect("a pack of one asset");
        let mut bytes = pack.finish().expect("a pack of one asset");
        if actual != version {
            claim(&mut bytes, version);
        }
        let number = self.packs.fetch_add(1, Ordering::Relaxed);
        let path = self.dir.join(format!("{number:08}.pack"));
        self.executor.block_on(self.io.replace(path.clone(), bytes)).expect("the pack written");
        self.executor.block_on(self.store.mount(path)).expect("the pack mounted");
    }

    fn settle(&self) {
        self.executor.block_on(self.store.settled());
    }
}

/// Rewrites the version the index of a one-entry pack claims, and reseals the index: the
/// content on disk no longer hashes to the version the index promises, as after damage to
/// a payload. It follows `README.md`, "The pack format": the index is the bytes before the
/// 24-byte trailer, whose first two words are the index's length and its xxh64; an index is
/// a `u32` count and then, per entry, the asset ID and the version as `u64`s.
fn claim(pack: &mut [u8], version: AssetVersion) {
    const TRAILER_LEN: usize = 24;
    let trailer = pack.len() - TRAILER_LEN;
    let index_len = u64::from_le_bytes(pack[trailer..trailer + 8].try_into().expect("a word"));
    let index = trailer - usize::try_from(index_len).expect("an index in memory");
    assert_eq!(pack[index..index + 4], 1u32.to_le_bytes(), "a pack of one entry");
    pack[index + 12..index + 20].copy_from_slice(&version.to_bits().to_le_bytes());
    let hash = xxh64(&pack[index..trailer], 0);
    pack[trailer + 8..trailer + 16].copy_from_slice(&hash.to_le_bytes());
}

#[test]
fn the_store_conforms_over_raw_and_compressed_packs() {
    let executor = Executor::builder().build().expect("an executor");
    let platform = executor.platform().as_ref();
    let io = Io::start(platform, IoConfig::for_platform(platform)).expect("the pool");
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("conformance");

    for (name, storage) in [("raw", Storage::Raw), ("zstd", Storage::Zstd { level: 3 })] {
        let checks = AtomicU64::new(0);
        conformance::run(|loaders| {
            let check = checks.fetch_add(1, Ordering::Relaxed);
            OnDisk::new(&executor, &io, root.join(format!("{name}-{check}")), storage, loaders)
        });
    }

    io.shutdown();
    executor.shutdown();
}
