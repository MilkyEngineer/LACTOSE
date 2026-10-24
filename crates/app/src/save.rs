//! The save path from the sync point to the slot's bytes and the load path from the slot's
//! bytes to a server's store (architecture sections 15.2, 15.3 and 15.5; D129).
//! `lactose-persist` is pure over its arguments; this is where its calls are placed on
//! threads. The bytes meet a file through `lactose_io::Io::replace`, which the headless
//! app calls and nothing here wraps.

use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroU64;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use lactose_asset_api::{PinnedTable, VersionTable};
use lactose_bundle::Baseline;
use lactose_core::{Result, Tick};
use lactose_ecs::RegionId;
use lactose_persist::RegionRecords;

use crate::game::Game;

/// Where cooked content comes from until the asset store exists: the baseline a save or a
/// relevance message names by content hash, the resolved asset table of a region, and the
/// pinning of a table. A stand-in with the store's shape and nothing of its own: milestone
/// 3 replaces every `&dyn Cooked` and `Arc<dyn Cooked>` in this crate with the store's
/// traits (`lactose_asset_api::AssetCatalog`, and a region-baseline asset kind), a
/// signature change to list (`README.md`, "Not in this crate yet"). At milestone 1 the
/// headless binary and the spike implement it over the content they build in memory.
pub trait Cooked: Send + Sync + 'static {
    /// The cooked baseline whose `Baseline::content_hash` is `content_hash`. Fails with
    /// `NotFound` if there is none.
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>>;

    /// The exact-version table of `region`'s asset set, or `None` while it is not resolved:
    /// a region loaded without one waits `Frozen` for a table through `TickInput::assets`.
    fn table(&self, region: RegionId) -> Option<VersionTable>;

    /// `table` pinned: every asset it names loaded at the version it names. Fails as the
    /// store fails, with the asset's failure kind.
    fn pin(&self, table: &VersionTable) -> Result<PinnedTable>;
}

/// One region's saved state while it is not loaded: what the server's store holds for it
/// (`Server::kept`), what a save point yields for it, and what a load decodes for it.
#[derive(Clone, Debug)]
pub struct SavedRegion {
    /// The region's records, at the registry's schemas: a decoded section after
    /// `Loader::migrate`, or an export after `RegionRecords::from_export`. Reloaded through
    /// `Loader::bundle`, re-saved through `RegionRecords::encode_section`, with no diff in
    /// between: a region kept across many saves is diffed once, when it unloads.
    pub records: RegionRecords,
    /// The baseline `records.baseline_hash` names, resolved once: by `Cooked::baseline` at
    /// load, or the baseline the region was loaded with when it unloaded.
    pub baseline: Arc<Baseline>,
}

/// Every region's saved state at one tick: what a container decodes to ([`load`]) and what
/// a save point yields (`Server::save_point`), the one shape `Start::Loaded` and
/// `Server::encode_save` take, so a save point can start a server without a file between.
#[derive(Clone, Debug)]
pub struct SavedWorld {
    /// The saved tick, for `SimulationBuilder::tick`: the first step runs the tick after it.
    pub tick: Tick,
    /// The saved counter, for `SimulationBuilder::next_net_id`.
    pub next_net_id: NonZeroU64,
    /// One entry per region, the loaded regions of the save point diffed and its kept ones
    /// as kept, in ascending region order. Shared, since a save point hands the store's
    /// entries over without a copy.
    pub regions: BTreeMap<RegionId, Arc<SavedRegion>>,
}

/// Decodes a save for a fresh process: `game.types()` for the registry,
/// `SchemaTable::from_registry` over it, `game.migrations()`, `Container::decode` over the
/// whole file, then for every section in ascending region order the baseline from
/// `cooked.baseline(records.baseline_hash)`, `Loader::bundle(&records, baseline,
/// RegionMode::Frozen, None)` to validate the section (the bundle is dropped: a save that
/// cannot load fails here, before any world exists, not at the step that reloads a region
/// hours in) and `Loader::migrate(&records)` as the entry's records. Synchronous, on the
/// thread that starts the binary, before any world exists: the registry a load needs is the
/// world's, and the tick and counter it yields are the builder's. The first error ends the
/// load with nothing built; the policy's response to [`Source::Load`](crate::Source::Load)
/// is `Stop`.
pub fn load(game: &Game, container: &[u8], cooked: &dyn Cooked) -> Result<SavedWorld> {
    todo!("decode the container, validate and migrate every section")
}

/// A save being encoded: one background task per region, then the assembly. See
/// [`Server::encode_save`](crate::Server::encode_save).
#[must_use = "futures do nothing unless awaited"]
pub struct SaveEncode {
    run: Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>,
}

impl Future for SaveEncode {
    type Output = Result<Vec<u8>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}
