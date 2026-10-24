//! Where the frames left each proxy: the region a proxy resided in after the staged frames
//! last placed it, recorded on the row so that a rollback restores it with the rest of the
//! world (architecture section 10, D126), which is what the hook's tombstone rule reads
//! beside where the proxy happens to reside now.

use lactose_bundle::ComponentData;
use lactose_core::{Result, StableKey};
use lactose_ecs::{RegionId, World};
use lactose_reflect::{Reflect, component};
use lactose_sim::TypeTable;

/// The region a proxy resided in after the staged frames last placed it: the frame's
/// region when the world could put the proxy there, else the region it stayed in, which
/// is where the server's next `Left` tombstone names it if the server moves the entity
/// back while owing one. A proxy's residence (`World::region_of`) is this region unless
/// a prediction system moved the proxy since with `Commands::move_to_region`, which the
/// frames never confirm; the hook's rule for a `TombstoneCause::Left` tombstone reads this
/// component so that such a proxy is not kept as one the server has in the region the
/// tombstone names.
///
/// Written by [`PredictionHook`](crate::PredictionHook) alone, in a predicted tick and a
/// replay alike: a proxy is spawned with it (the spawn's region), and it is rewritten
/// after an upsert, a frame's move and a release with where the entry left the proxy; a
/// correction that writes a release rewrites it the same way. Never written to an entity
/// the server has claimed for the local player, whose region is predicted by its own
/// kernels and reconciled by the server's facts; one that carries it from before its claim
/// keeps a stale value nothing reads until the release restates it. A proxy no frame has
/// placed (a region load's cooked rows until the server names the region, about a round
/// trip) has none, and the hook treats where such a proxy resides as the server's region.
///
/// Prediction-world only, `derived`: never replicated, extracted, hashed or saved. A
/// column of the world, so it is in every ring slot and a rollback restores it
/// (architecture section 10); not in the record, so section 9.4's comparison never reads
/// it. The field is private, so no crate but this one builds a value: a prediction system
/// may name the component (it is derived) and reads the server's last placement through
/// [`region`](Self::region); one that removes it through `Commands` degrades that proxy
/// to the residence rule, as one removing `ClaimedBy` would corrupt a claim, and the build
/// check does not judge writes. `app` registers it for the prediction world through
/// [`register_types`], which [`Predictor::new`](crate::Predictor::new) requires.
#[component(key = "predict::staged_region", derived)]
#[derive(PartialEq, Eq, Debug)]
pub struct StagedRegion {
    #[field(id = 1)]
    region: RegionId,
}

impl StagedRegion {
    /// The region the proxy resided in after the frames last placed it.
    #[must_use]
    pub const fn region(&self) -> RegionId {
        self.region
    }
}

/// Registers this crate's one component, [`StagedRegion`], which the hook writes on every
/// proxy a frame places. `app` calls it for the prediction world beside every other
/// crate's `register_types`; [`Predictor::new`](crate::Predictor::new) refuses a world
/// whose registry lacks it. Registering it for a server or client world is harmless and
/// unnecessary: nothing there writes it.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<StagedRegion>()
}

/// The placement `region` as a spawn's component, so a proxy is born with it in its final
/// archetype rather than moved to one after the spawn.
pub(crate) fn placement_data(region: RegionId) -> ComponentData {
    let placed = StagedRegion { region };
    ComponentData {
        component: StagedRegion::KEY,
        bytes: lactose_reflect::bytemuck::bytes_of(&placed).to_vec(),
    }
}

/// Whether `components`, a spawn's, already name the placement.
pub(crate) fn names_placement(components: &[ComponentData]) -> bool {
    components.iter().any(|data| data.component == StagedRegion::KEY)
}

/// Records `region` as where the frames last left the live proxy `key`: a write only when
/// the recorded region differs, so an unchanged placement touches nothing.
pub(crate) fn place(world: &mut World, key: StableKey, region: RegionId) -> Result<()> {
    if world.get::<StagedRegion>(key).is_some_and(|placed| placed.region == region) {
        return Ok(());
    }
    world.insert(key, StagedRegion { region })
}

/// Records where the live proxy `key` resides as where the frames last left it: what the
/// hook does after a frame's entry has moved the proxy, or failed to.
pub(crate) fn place_residence(world: &mut World, key: StableKey) -> Result<()> {
    match world.region_of(key) {
        Some(resides) => place(world, key, resides),
        None => Ok(()),
    }
}

/// The region the frames last left `key` in, or, for an entity no frame has placed, the
/// region it resides in; `None` for an entity that is not live.
pub(crate) fn placed(world: &World, key: StableKey) -> Option<RegionId> {
    match world.get::<StagedRegion>(key) {
        Some(placed) => Some(placed.region),
        None => world.region_of(key),
    }
}
