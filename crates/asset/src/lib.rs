//! The asset store (architecture section 4, `asset`; section 14): cooked packs mounted and
//! read through `lactose-io`, loads on the executor's background workers, versioned
//! publication and hot reload behind the two store traits of `lactose-asset-api`, the pack
//! format the cooker writes, and the region baseline as an asset. SILO is the store's name
//! on the things that run. `README.md` is the contract: it states what the signatures
//! cannot, and what a caller may rely on.
//!
//! Backend tier, outside the determinism boundary. What a boundary world reaches through
//! it is the `PublishedAssets` trait object and the `PinnedTable` pinned from it, and
//! nothing of this crate by name.
//!
//! # Status
//!
//! Implemented (D136): the store, the pack format and the region baseline do what
//! `README.md` says, proved by the tests its "Proof for milestone 1" lists.
//!
//! # Where to start
//!
//! - [`Store`] is the store: built once by `app` from the executor, the IO pool and every
//!   kind's loader; [`Store::mount`] makes a cooked pack's versions requestable and is how
//!   a newly cooked pack reaches a running store; the store is a
//!   `lactose_asset_api::AssetCatalog` and so a `PublishedAssets`; [`Store::settled`] is
//!   the one wait, for tools and tests.
//! - [`pack_set`] lists a cooked directory's packs in the order a fresh process mounts
//!   them, as a [`PackSet`] future.
//! - [`PackBuilder`] writes a pack, for the cooker and for tests; [`Storage`] says how one
//!   payload is stored.
//! - [`RegionBaseline`] is a region's cooked baseline as an asset, named by its content
//!   hash.
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use lactose_asset::Store;
//! use lactose_asset_api::{
//!     AssetCatalog, AssetId, AssetSet, LoadPriority, PinnedTable, PublishedAssets,
//! };
//! use lactose_io::{Io, IoConfig};
//! use lactose_task::Executor;
//!
//! # fn main() -> lactose_core::Result<()> {
//! let executor = Executor::builder().build()?;
//! let platform = executor.platform().as_ref();
//! let io = Io::start(platform, IoConfig::for_platform(platform))?;
//!
//! // One loader per asset kind, from the crates that define the kinds.
//! let store = Store::new(executor.clone(), io.clone(), Vec::new())?;
//! // Mount order is call order: a later pack's version of an asset is the newest.
//! executor.block_on(store.mount("cooked/base.pack"))?;
//! executor.block_on(store.mount("cooked/0001.pack"))?;
//!
//! // Outside the boundary: the catalogue. Request a region's set, then resolve it.
//! let catalog: Arc<dyn AssetCatalog> = Arc::new(store.clone());
//! let set = AssetSet::from_iter([AssetId::from_name("props/barrel.mesh")]);
//! for id in set.iter() {
//!     catalog.request_newest(id, LoadPriority::default());
//! }
//! executor.block_on(store.settled()); // a tool waits; a frame loop watches `generation`
//! let table = catalog.resolve(&set).ready().expect("published");
//!
//! // Inside the boundary: exact versions only, pinned from the table.
//! let published: Arc<dyn PublishedAssets> = catalog.clone();
//! let pinned = PinnedTable::pin(published.as_ref(), &table).ready().expect("published");
//! # let _ = pinned;
//!
//! drop((catalog, published, store));
//! io.shutdown();
//! executor.shutdown();
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

mod baseline;
mod pack;
mod state;
mod store;
mod trace;

pub use baseline::{RegionBaseline, RegionBaselineLoader};
pub use pack::{PACK_FORMAT_VERSION, PackBuilder, Storage};
pub use store::{Mount, PackSet, Settled, Store, pack_set};

/// The `tracing` target of everything this crate emits: `silo`, the store's codename
/// (architecture sections 19 and 23).
pub const LOG_TARGET: &str = "silo";

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the store is a `Clone` handle shared across threads; a
/// shared store is the catalogue trait object, and that is the published-assets trait
/// object a boundary world receives; the three futures are `Send + 'static` so
/// `Executor::spawn` takes them and `Unpin` so a caller polls them without pinning, and
/// each resolves to what its method documents; a pack is built on any thread; the region
/// baseline is an asset its loader produces; and the format version is the number the
/// README states.
const _: () = {
    use std::future::Future;
    use std::sync::Arc;

    use lactose_asset_api::{Asset, AssetCatalog, Loader, PublishedAssets};

    const fn handle<T: Clone + Send + Sync + 'static>() {}
    const fn send_sync<T: Send + Sync + 'static>() {}
    const fn future<F: Future<Output = T> + Send + Unpin + 'static, T>() {}
    const fn copy<T: Copy + Send + Sync + 'static>() {}
    const fn asset<T: Asset>() {}
    const fn loader_of<L: Loader<Asset = T> + Copy, T: Asset>() {}

    handle::<Store>();
    future::<Mount, lactose_core::Result<()>>();
    future::<Settled, ()>();
    future::<PackSet, lactose_core::Result<Vec<std::path::PathBuf>>>();

    send_sync::<PackBuilder>();
    copy::<Storage>();

    asset::<RegionBaseline>();
    handle::<RegionBaseline>();
    loader_of::<RegionBaselineLoader, RegionBaseline>();

    // The two views `app` hands on: a shared store is the catalogue, and the catalogue is
    // the published-assets view.
    const _: fn(Arc<Store>) -> Arc<dyn AssetCatalog> = |store| store;
    const _: fn(Arc<dyn AssetCatalog>) -> Arc<dyn PublishedAssets> = |catalog| catalog;

    assert!(PACK_FORMAT_VERSION == 1);
};
