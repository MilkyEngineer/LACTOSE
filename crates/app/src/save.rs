//! The save path from the sync point to the slot, and the load path from the slot to the
//! first frame (architecture sections 15.3 and 15.5; D129). `lactose-persist` is pure over
//! its arguments; this is where its calls are placed on threads and where the bytes meet a
//! file.

use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use lactose_asset_api::{PinnedTable, VersionTable};
use lactose_bundle::Baseline;
use lactose_core::{Result, Tick};
use lactose_ecs::RegionId;
use lactose_sim::RegionBundle;
use lactose_task::Executor;

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

/// A decoded save: what [`Start::Loaded`](crate::Start::Loaded) carries into a server world.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The saved tick, for `SimulationBuilder::tick`: the first step runs the tick after it.
    pub tick: Tick,
    /// The saved counter, for `SimulationBuilder::next_net_id`.
    pub next_net_id: NonZeroU64,
    /// One bundle per saved region, each through `Loader::bundle`, in ascending region
    /// order: the first frame's `loaded`.
    pub regions: BTreeMap<RegionId, RegionBundle>,
}

/// Decodes a save for a fresh process: `game.types()` for the registry,
/// `SchemaTable::from_registry` over it, `game.migrations()`, `Container::decode` over the
/// whole file, then for every section in ascending region order
/// `Loader::bundle(records, cooked.baseline(hash)?, RegionMode::Simulated, table)` with
/// `table` the region's `cooked.table(region)` pinned, or `None`. Synchronous, on the thread
/// that starts the binary, before any world exists: the registry a load needs is the
/// world's, and the tick and counter it yields are the builder's. The first error ends the
/// load with nothing built; the policy's response to [`Source::Load`](crate::Source::Load)
/// is `Stop`.
pub fn load(game: &Game, container: &[u8], cooked: &dyn Cooked) -> Result<Loaded> {
    todo!("decode the container and bundle every section")
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

/// Writes `bytes` over the save slot at `slot` atomically, on a background task: a
/// temporary file named `<slot file name>.<unique>.tmp` in the slot's directory, written in
/// full, synced, then renamed over the slot, so a reader never sees a partial save and a
/// failure at any point leaves the previous save in place and the temporary file removed.
/// `std::fs` on a background worker until `lactose-io` exists; then this is
/// `Io::replace(slot, bytes)` and the parameter list gains the `Io` (`README.md`, "Not in
/// this crate yet"). Fails with `StorageFull`, `NotFound` (the slot's directory does not
/// exist) or `Io`; the policy's response to [`Source::Save`](crate::Source::Save) is `Log`.
pub fn write_slot(executor: &Executor, slot: PathBuf, bytes: Vec<u8>) -> SlotWrite {
    todo!("spawn the write on a background worker")
}

/// A slot write in flight. See [`write_slot`].
#[must_use = "futures do nothing unless awaited"]
pub struct SlotWrite {
    run: Pin<Box<dyn Future<Output = Result<()>> + Send>>,
}

impl Future for SlotWrite {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}
