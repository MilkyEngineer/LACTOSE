//! The game the client shows: `game-sim`'s registration with the scene's session applier, the
//! cooked content of its one region in memory, and the frames that load the region and join
//! one player. As in the headless binary, the registration values are the binary's, since a
//! game crate may name neither `lactose-app` nor `lactose-persist`, and they pass no setup
//! and no migration function (architecture section 8.1). The asset store replaces the
//! in-memory content at milestone 3.

use std::sync::Arc;

use lactose_app::{Cooked, CrateRegistration, Game};
use lactose_asset_api::{
    Asset, AssetId, AssetKind, AssetSet, AssetVersion, LoadPriority, PinnedTable, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, PlayerId, Result, Tick};
use lactose_ecs::{RegionId, RegionMode};
use lactose_sim::{RegionBundle, SessionEvent, TickInput};

static GAME_SIM: CrateRegistration = CrateRegistration {
    name: "game-sim",
    types: game_sim::register_types,
    setup: None,
    systems: game_sim::register_systems,
    migrations: None,
};

/// `game-sim` with [`game_sim::scene`] as its session applier: the join of the first frame
/// spawns the seven rows that orbit.
pub static GAME: Game =
    Game { name: "scene", crates: &[GAME_SIM], order: None, sessions: Some(game_sim::scene) };

/// The region the scene runs in.
pub const REGION: RegionId = RegionId(1);

/// A store that has published every asset at every version.
struct Store;

struct Blob;

impl Asset for Blob {
    const KIND: AssetKind = AssetKind::from_name("scene::blob");
    const FORMAT_VERSION: u32 = 1;
}

impl PublishedAssets for Store {
    fn request(&self, _: AssetId, _: AssetVersion, _: LoadPriority) {}

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        Readiness::Ready(UntypedHandle::new(id, version, Blob))
    }
}

fn table_of(region: RegionId) -> VersionTable {
    [(AssetId::from_bits(region.0), AssetVersion::from_bits(1).expect("non-zero"))]
        .into_iter()
        .collect()
}

/// The cooked content of [`REGION`]: an empty baseline and one asset.
pub struct InMemory {
    baseline: Arc<Baseline>,
}

impl InMemory {
    /// The content.
    ///
    /// # Errors
    ///
    /// The baseline's, which an empty one does not have.
    pub fn new() -> Result<Self> {
        Ok(Self { baseline: Arc::new(Baseline::from_rows(AssetSet::new(), [], [])?) })
    }

    fn fresh(&self) -> Result<RegionBundle> {
        Ok(RegionBundle {
            seed: REGION.0,
            mode: RegionMode::Simulated,
            baseline: Arc::clone(&self.baseline),
            overlay: None,
            resources: Vec::new(),
            transient_resources: Vec::new(),
            assets: Some(self.pin(&table_of(REGION))?),
        })
    }
}

impl Cooked for InMemory {
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>> {
        if self.baseline.content_hash() == content_hash {
            Ok(Arc::clone(&self.baseline))
        } else {
            Err(Error::new(ErrorKind::NotFound, "no cooked baseline has that hash"))
        }
    }

    fn table(&self, region: RegionId) -> Option<VersionTable> {
        (region == REGION).then(|| table_of(region))
    }

    fn pin(&self, table: &VersionTable) -> Result<PinnedTable> {
        PinnedTable::pin(&Store, table)
            .ready()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "the store has not published it"))
    }
}

/// The input of each tick: the region's bundle and player 1's join in the first, nothing
/// after.
pub struct Frames {
    first: Option<RegionBundle>,
}

impl Frames {
    /// The frames of a run over `cooked`.
    ///
    /// # Errors
    ///
    /// The pin of the region's table.
    pub fn new(cooked: &InMemory) -> Result<Self> {
        Ok(Self { first: Some(cooked.fresh()?) })
    }

    /// The next tick's input.
    pub fn next(&mut self, _tick: Tick) -> TickInput {
        let mut input = TickInput::default();
        if let Some(bundle) = self.first.take() {
            input.loaded.insert(REGION, bundle);
            input.sessions.insert(PlayerId(1), SessionEvent::Join { region: REGION });
        }
        input
    }
}
