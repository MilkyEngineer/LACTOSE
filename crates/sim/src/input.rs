//! What enters the simulation at the input sync point (architecture sections 6, 8.2, 9.2
//! and 14): one [`TickInput`] per tick, and the two hooks that write what `sim` cannot
//! type itself.

use std::sync::Arc;

use lactose_asset_api::{AssetFailure, PinnedTable};
use lactose_bundle::{Baseline, ComponentData, Overlay};
use lactose_core::{ComponentKey, PlayerId, Result, Tick};
use lactose_ecs::{RegionId, RegionMode, World};

/// Everything that enters the boundary for one tick, handed to
/// [`Simulation::step`](crate::Simulation::step) and applied at the input sync point of
/// that tick, field by field in the order declared here.
///
/// The input journal (architecture section 15.5) is the sequence of these, and
/// `lactose-predict`'s ring keeps one per tick for re-simulation. Whoever assembles one
/// (`app` from `net`, `input` and `asset`; a test directly) decides nothing by order: the
/// sync point sorts each list by its key and applies it in that order, so the same entries
/// in any order are one input. A list is empty by default, so `TickInput::default()` is a
/// tick with nothing new.
#[derive(Clone, Debug, Default)]
pub struct TickInput {
    /// The players' inputs for this tick, at most one per player; a second entry for a
    /// player is dropped and logged at target `curd`.
    pub inputs: Vec<PlayerInput>,
    /// Asset version tables resolved and pinned outside the boundary, at most one per
    /// region: the publish events of architecture section 9.3, tick-stamped by the frame
    /// they arrive in. A table for a region that is not loaded waits in the lifecycle for
    /// the region's bundle; a later table for the same region replaces it.
    pub assets: Vec<RegionAssets>,
    /// Region bundles that have arrived, from the asset store or a save.
    pub loaded: Vec<RegionBundle>,
    /// Regions to export and unload now. Honoured under
    /// [`RegionControl::Directed`](crate::RegionControl::Directed); under `Anchored` the
    /// policy decides and these are dropped and logged.
    pub unloaded: Vec<RegionId>,
    /// Mode changes. Honoured under `Directed`; dropped and logged under `Anchored`.
    pub modes: Vec<RegionModeChange>,
    /// Asset failures the store reported since the last tick. The sync point stamps each
    /// with the tick (architecture section 14) and keeps it in the
    /// [`RegionLifecycle`](crate::RegionLifecycle) for `app` and `net` to take.
    pub failures: Vec<AssetFailure>,
}

/// One player's input for one tick, quantised on the client so that the server simulates
/// exactly the value the client sent (architecture section 9.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlayerInput {
    /// The player.
    pub player: PlayerId,
    /// The input slot the sample is applied for: the player's input tick this tick, which
    /// [`World::set_input_tick`] receives before the sample is written and which a predicted
    /// spawn's key carries. Set to the slot of the substitute when the input was lost.
    pub input_tick: Tick,
    /// Whether the server substituted the previous input for a lost one (architecture
    /// section 9.2). Journaled, so a replay applies the same substitute; the client is told
    /// by `net`, not through the simulation.
    pub substituted: bool,
    /// The sample: the plain-data bytes of the quantised input component `lactose-input`
    /// defines, exactly as that component is laid out in memory. `sim` never interprets
    /// them; the [`InputApplier`] decodes them with the component's `CheckedBitPattern` and
    /// writes the value. The journal stores these bytes verbatim.
    pub sample: Vec<u8>,
}

/// A region's exact-version asset table, resolved by `AssetCatalog::resolve` and pinned
/// with `PinnedTable::pin` outside the boundary, entering at the input sync point through
/// [`World::set_region_assets`] (architecture sections 8.2 and 14). On the server the
/// frame it arrives in is the journaled publish time of architecture section 9.3.
#[derive(Clone, Debug)]
pub struct RegionAssets {
    /// The region.
    pub region: RegionId,
    /// The table, holding a handle to every version in it.
    pub table: PinnedTable,
}

/// A region's bundle as the input sync point loads it: the cooked baseline, the overlay
/// against it if the region has state beyond the baseline (a save, or what the server sent
/// on relevance), the seed and the region resources that travel beside the overlay
/// (architecture sections 9.3 and 15.2). The load path is
/// [`World::create_region`](lactose_ecs::World::create_region) with the seed,
/// [`World::load_region`](lactose_ecs::World::load_region), then
/// [`insert_region_resource_erased`](lactose_ecs::World::insert_region_resource_erased) for
/// each resource and [`insert_region_resource_zeroed`](lactose_ecs::World::insert_region_resource_zeroed)
/// for each transient one: the reverse of an export, so a load, a save and an unload are
/// one code path through [`RegionExport`](crate::RegionExport).
#[derive(Clone, Debug)]
pub struct RegionBundle {
    /// The region.
    pub region: RegionId,
    /// The region seed: the cooked seed for a fresh region, the saved one for a reload.
    pub seed: u64,
    /// The mode the region is created in. Honoured under
    /// [`RegionControl::Directed`](crate::RegionControl::Directed); under `Anchored` the
    /// policy sets the mode at the same sync point and this is ignored.
    pub mode: RegionMode,
    /// The cooked baseline. Shared, since the asset store keeps it loaded and the journal
    /// names it by its content hash.
    pub baseline: Arc<Baseline>,
    /// The overlay against `baseline`, or `None` for a region in its cooked state. An
    /// overlay made against another baseline is a load failure of kind `VersionMismatch`;
    /// `lactose-persist` rebases and migrates before it builds a bundle.
    pub overlay: Option<Arc<Overlay>>,
    /// The persistent region resources, as an export lists them.
    pub resources: Vec<ComponentData>,
    /// The transient region resources, by key, restored zeroed.
    pub transient_resources: Vec<ComponentKey>,
}

/// A mode change requested through the frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RegionModeChange {
    /// The region.
    pub region: RegionId,
    /// The mode it takes at the sync point.
    pub mode: RegionMode,
}

/// Writes a player's quantised input into the world: the typed half of input application,
/// implemented beside the input component's definition in `lactose-input` (or by `app` and
/// tests for a component of their own) and inserted by
/// [`SimulationBuilder::inputs`](crate::SimulationBuilder::inputs) as the service resource
/// `dyn InputApplier`.
///
/// [`systems::apply_inputs`](crate::systems) calls it once per [`PlayerInput`] of the frame,
/// in ascending `PlayerId` order, after [`World::set_input_tick`] has recorded the slot.
/// The implementation decodes `input.sample` with the component's own validation, finds the
/// player's entity (a session fact it keeps; `sim` holds no player table) and writes the
/// value through [`World::insert`] or [`World::get_mut`](lactose_ecs::World::get_mut). An
/// error is logged at target `curd` and the input is dropped; the tick runs. The component
/// is `derived`, or its crate adds an exclusive system of its own, ordered after
/// `apply_inputs`, that declares `writes = [Sample]` so transient readers have a writer.
///
/// The same implementation serves the server (samples from `net`), the prediction world
/// (the local player's own sample) and a replay (samples from the journal).
pub trait InputApplier: Send + Sync + 'static {
    /// Writes `input` for this tick.
    fn apply(&mut self, world: &mut World, input: &PlayerInput) -> Result<()>;
}

/// Applies staged state to the world at the input sync point: the hook `lactose-net` fills
/// on a client (received state into `Replicated` rows, architecture section 9.2) and
/// `lactose-predict` fills in the prediction world (the kinematic proxies' per-tick history,
/// section 9.4). Inserted by
/// [`SimulationBuilder::staged_state`](crate::SimulationBuilder::staged_state) as the service
/// resource `dyn StagedState`; the server leaves the default, which does nothing.
///
/// [`systems::apply_staged_state`](crate::systems) calls it once per tick after the inputs
/// have been applied. Everything a client receives is written here and nowhere else: spawns
/// with the server's key through [`World::spawn`](lactose_ecs::World::spawn), binds through
/// [`World::bind`](lactose_ecs::World::bind), whole components through
/// [`World::insert`], tracked columns through [`World::columns_mut`](lactose_ecs::World::columns_mut)
/// or [`Mut::set_if_neq`](lactose_ecs::Mut::set_if_neq) so an equal value does not dirty the
/// row, despawns from tombstones. An error is logged at target `curd`; the tick runs.
pub trait StagedState: Send + Sync + 'static {
    /// Writes what was staged for `tick`.
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()>;
}
