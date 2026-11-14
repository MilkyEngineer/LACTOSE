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
use lactose_bundle::{Baseline, Record};
use lactose_core::{Error, ErrorKind, NetId, Result, Tick};
use lactose_ecs::{RegionId, RegionMode};
use lactose_persist::{Container, Loader, RegionRecords, SchemaTable};

use crate::game::Game;
use crate::policy::{ErrorPolicy, Source};

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
/// whole file (which also refuses a counter not above a runtime `NetId` a section spawns
/// and one entity held by two sections: `Container::check_sections`), then every section's
/// baseline from `cooked.baseline(records.baseline_hash)`, then the check that needs them:
/// a section's `SpawnFromBaseline` of a cooked `NetId` that another section's baseline holds
/// as a row, where that section has no `Destroy` of it, is `InvalidData` naming both
/// regions, since the second region's load would find the entity live. Then for every
/// section in ascending region order `Loader::bundle(&records, baseline, RegionMode::Frozen,
/// None)` to validate the section (the bundle is dropped: a save that cannot load fails
/// here, before any world exists, not at the step that reloads a region hours in) and
/// `Loader::migrate(&records)` as the entry's records. From the lane `binds-impl` on (D142;
/// a facade until then), `Loader::binds` then runs over the migrated records of every
/// section and its result is dropped: a save whose sections bind one prediction key to two
/// `NetId`s fails here with `InvalidData`, and `build_server` reads the binds again from
/// the `SavedWorld`, as it does for one a save point produced. Synchronous, on the thread that
/// starts the binary, before any world exists: the registry a load needs is the world's,
/// and the tick and counter it yields are the builder's. The first error ends the load
/// with nothing built; the policy's response to [`Source::Load`](crate::Source::Load) is
/// `Stop`.
///
/// What no section's baseline shows is not checked: a cooked entity that a section spawns
/// from the baseline while its home region has no section in the file is found live when
/// that region first loads fresh, since a region's baseline is known only when the store
/// can name it (milestone 3, `README.md`, "Not in this crate yet").
pub fn load(game: &Game, container: &[u8], cooked: &dyn Cooked) -> Result<SavedWorld> {
    decode(game, container, cooked).inspect_err(|error| {
        ErrorPolicy::MILESTONE_1.decide(Source::Load, error);
    })
}

/// [`load`] before the policy sees its error.
fn decode(game: &Game, container: &[u8], cooked: &dyn Cooked) -> Result<SavedWorld> {
    let types = game.types()?;
    let schemas = SchemaTable::from_registry(types.registry())?;
    let migrations = game.migrations()?;
    let decoded = Container::decode(container)?;
    let sections = decoded
        .regions()
        .map(|records| Ok((records, cooked.baseline(records.baseline_hash)?)))
        .collect::<Result<Vec<_>>>()?;
    check_cooked_homes(&sections)?;
    let loader = Loader::new(&schemas, &migrations);
    let mut regions = BTreeMap::new();
    for (records, baseline) in sections {
        loader.bundle(records, Arc::clone(&baseline), RegionMode::Frozen, None)?;
        let records = loader.migrate(records)?;
        regions.insert(records.region, Arc::new(SavedRegion { records, baseline }));
    }
    let header = decoded.header();
    Ok(SavedWorld { tick: header.tick, next_net_id: header.next_net_id, regions })
}

/// Refuses a section that spawns from the baseline a cooked entity which another section's
/// baseline holds as a row that section does not destroy, as [`load`] states. A home
/// section that changes the row was refused by `Container::check_sections`; one that
/// leaves it unchanged has no record of it, so only its baseline shows it.
fn check_cooked_homes(sections: &[(&RegionRecords, Arc<Baseline>)]) -> Result<()> {
    for (records, _) in sections {
        let Some(overlay) = &records.overlay else { continue };
        // Records ascend by `NetId` and cooked IDs order above runtime ones.
        let list = overlay.records();
        let cooked = list.partition_point(|record| !record.id().is_cooked());
        for record in &list[cooked..] {
            let Record::SpawnFromBaseline { id, .. } = *record else { continue };
            let home = sections.iter().find(|(home, baseline)| {
                home.region != records.region && baseline.contains(id) && !destroys(home, id)
            });
            if let Some((home, _)) = home {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "the section of region {} spawns NetId {:#x} from the baseline, which \
                         region {}'s baseline holds as a row its section does not destroy",
                        records.region.0,
                        id.to_bits(),
                        home.region.0
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Whether `records` hold a `Destroy` of `id`.
fn destroys(records: &RegionRecords, id: NetId) -> bool {
    let Some(overlay) = &records.overlay else { return false };
    let list = overlay.records();
    let at = list.partition_point(|record| record.id() < id);
    matches!(list.get(at), Some(&Record::Destroy { id: destroyed }) if destroyed == id)
}

/// A save being encoded: one background task per region, then the assembly. See
/// [`Server::encode_save`](crate::Server::encode_save).
#[must_use = "futures do nothing unless awaited"]
pub struct SaveEncode {
    run: Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>,
}

impl SaveEncode {
    /// The encode in flight.
    pub(crate) fn new(run: Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>) -> Self {
        Self { run }
    }
}

impl Future for SaveEncode {
    type Output = Result<Vec<u8>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}
