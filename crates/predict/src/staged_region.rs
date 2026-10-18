//! Where the server has each proxy: the region the staged frames last placed it in,
//! recorded on the row so that a rollback restores it with the rest of the world
//! (architecture section 10, D126), which is what the hook's tombstone rule reads instead
//! of where the proxy happens to reside.

use lactose_core::{Result, StableKey};
use lactose_ecs::{RegionId, World};
use lactose_reflect::component;
use lactose_sim::TypeTable;

/// The region the staged frames last placed a proxy in: where the server has the entity
/// as of the last frame that named it, which the frames alone write. A proxy's residence
/// (`World::region_of`) is this region unless a prediction system moved the proxy with
/// `Commands::move_to_region`, which the frames never confirm; the hook's rule for a
/// `TombstoneCause::Left` tombstone reads this component so that such a proxy is not kept
/// as one the server has in the region the tombstone names.
///
/// Written by [`PredictionHook`](crate::PredictionHook) alone, in a predicted tick and a
/// replay alike: at a proxy's spawn and upsert (the spawn's region), at a frame's move of
/// a proxy (the move's region) and at a release (the releasing spawn's region); a
/// correction that writes a release writes it too. Never written to an entity the server
/// has claimed for the local player, whose region is predicted by its own kernels and
/// reconciled by the server's facts; one that carries it from before its claim keeps a
/// stale value nothing reads until the release restates it. A proxy no frame has placed
/// (a region load's cooked rows until the server names the region, about a round trip)
/// has none, and the hook treats where such a proxy resides as the server's region.
///
/// Prediction-world only, `derived`: never replicated, extracted, hashed or saved. A
/// column of the world, so it is in every ring slot and a rollback restores it
/// (architecture section 10); not in the record, so section 9.4's comparison never reads
/// it. A prediction system may name it (it is derived) and reads the server's last
/// placement; one that writes it corrupts the hook's record, as one writing `ClaimedBy`
/// would, and the build check does not judge writes. `app` registers it for the
/// prediction world through [`register_types`], which
/// [`Predictor::new`](crate::Predictor::new) requires.
#[component(key = "predict::staged_region", derived)]
#[derive(PartialEq, Eq, Debug)]
pub struct StagedRegion {
    /// The region the frames last placed the proxy in.
    #[field(id = 1)]
    pub region: RegionId,
}

/// Registers this crate's one component, [`StagedRegion`], which the hook writes on every
/// proxy a frame places. `app` calls it for the prediction world beside every other
/// crate's `register_types`; [`Predictor::new`](crate::Predictor::new) refuses a world
/// whose registry lacks it. Registering it for a server or client world is harmless and
/// unnecessary: nothing there writes it.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<StagedRegion>()
}

/// Records `region` as where the frames last placed the live proxy `key`: a write only
/// when the recorded region differs, so an unchanged placement touches nothing.
pub(crate) fn place(world: &mut World, key: StableKey, region: RegionId) -> Result<()> {
    if world.get::<StagedRegion>(key).is_some_and(|placed| placed.region == region) {
        return Ok(());
    }
    world.insert(key, StagedRegion { region })
}

/// The region the frames last placed `key` in, or, for an entity no frame has placed,
/// the region it resides in; `None` for an entity that is not live.
pub(crate) fn placed(world: &World, key: StableKey) -> Option<RegionId> {
    match world.get::<StagedRegion>(key) {
        Some(placed) => Some(placed.region),
        None => world.region_of(key),
    }
}
