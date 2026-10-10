//! What enters the simulation at the input sync point (architecture sections 6, 8.2, 9.2
//! and 14): one [`TickInput`] per tick, and the three hooks that write what `sim` cannot
//! type itself.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lactose_asset_api::{AssetFailure, PinnedTable};
use lactose_bundle::{Baseline, ComponentData, Overlay};
use lactose_core::{ComponentKey, PlayerId, Result, Tick};
use lactose_ecs::{RegionId, RegionMode, World};

use crate::time::TimeScale;

/// Everything that enters the boundary for one tick, handed to
/// [`Simulation::step`](crate::Simulation::step) and applied at the input sync point of
/// that tick, field by field in the order declared here, each map in ascending key order.
///
/// The input journal (architecture section 15.5) is the sequence of these, and
/// `lactose-predict`'s ring keeps one per tick for re-simulation. Every keyed field is a
/// map, so a frame cannot hold two entries for one key and the way a frame was assembled
/// (by `app` from `net`, `input` and `asset`; by a test directly) decides nothing. A frame
/// is empty by default: `TickInput::default()` is a tick with nothing new.
#[derive(Clone, Debug, Default)]
pub struct TickInput {
    /// The time scale from this tick on, or `None` to leave the world's mirror as it is.
    /// Written by the sync point into the [`TimeScale`] resource.
    pub time_scale: Option<TimeScale>,
    /// Asset failures the store reported since the last tick, in the order it reported
    /// them. The sync point stamps each with the tick (architecture section 14) and keeps
    /// it in the [`RegionLifecycle`](crate::RegionLifecycle) for `app` and `net` to take.
    /// Events, not state: nothing in the world depends on their order.
    pub failures: Vec<AssetFailure>,
    /// Region bundles that have arrived, from the asset store or a save, by region.
    pub loaded: BTreeMap<RegionId, RegionBundle>,
    /// Asset version tables resolved and pinned outside the boundary, by region: the
    /// publish events of architecture section 9.3, tick-stamped by the frame they arrive
    /// in. A table for a region that is not loaded after this frame's bundles is dropped
    /// and logged; a bundle carries its first table in [`RegionBundle::assets`].
    pub assets: BTreeMap<RegionId, PinnedTable>,
    /// Regions to unload now. Honoured under
    /// [`RegionControl::Directed`](crate::RegionControl::Directed); under `Anchored` the
    /// policy decides and these are dropped and logged.
    pub unloaded: BTreeSet<RegionId>,
    /// Mode changes, by region. Honoured under `Directed`; dropped and logged under
    /// `Anchored`.
    pub modes: BTreeMap<RegionId, RegionMode>,
    /// Players joining and leaving, by player: the server's session inputs, applied through
    /// the [`SessionApplier`]. Dropped and logged on a world that is not `Server`, which
    /// learns of players through staged state.
    pub sessions: BTreeMap<PlayerId, SessionEvent>,
    /// The players' inputs for this tick, by player, applied through the [`InputApplier`].
    pub inputs: BTreeMap<PlayerId, PlayerSample>,
}

/// One player's input for one tick, quantised on the client so that the server simulates
/// exactly the value the client sent (architecture section 9.2). Keyed by the player in
/// [`TickInput::inputs`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlayerSample {
    /// The input slot this sample is applied for: always the slot being filled, also for a
    /// substitute, so a predicted spawn's key names this tick's slot and never the previous
    /// one's. [`World::set_input_tick`] receives it before the sample is written.
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

/// A region's bundle as the input sync point loads it: the cooked baseline, the overlay
/// against it if the region has state beyond the baseline (a save, or what the server sent
/// on relevance), the seed and the region resources that travel beside the overlay
/// (architecture sections 9.3 and 15.2), and the region's first asset table if it has
/// been resolved. Keyed by the region in [`TickInput::loaded`].
///
/// The load path is [`World::create_region`](lactose_ecs::World::create_region) with the
/// seed, [`World::load_region`](lactose_ecs::World::load_region), then
/// [`insert_region_resource_erased`](lactose_ecs::World::insert_region_resource_erased)
/// for each resource,
/// [`insert_region_resource_zeroed`](lactose_ecs::World::insert_region_resource_zeroed)
/// for each transient one and
/// [`set_region_assets`](lactose_ecs::World::set_region_assets) for the table: the reverse
/// of an export, so a load, a save and an unload are one code path through
/// [`RegionExport`](crate::RegionExport). Everything is validated before the region is
/// created, so a refused bundle changes nothing.
#[derive(Clone, Debug)]
pub struct RegionBundle {
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
    /// The persistent region resources, as an export lists them: from the save for a
    /// reload, and empty for a cooked region until the bundle format carries them
    /// (`docs/verification.md`, the seed-and-resources-beside-the-overlay item).
    pub resources: Vec<ComponentData>,
    /// The transient region resources, by key, restored zeroed.
    pub transient_resources: Vec<ComponentKey>,
    /// The region's asset table, if resolved when the bundle was built. A later table
    /// arrives through [`TickInput::assets`].
    pub assets: Option<PinnedTable>,
}

/// A player joining or leaving the session (architecture section 9.1): the server's input,
/// keyed by the player in [`TickInput::sessions`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionEvent {
    /// The player joins, in `region`. The sync point hands it to the [`SessionApplier`] at
    /// the first sync point at which `region` is loaded, which may be later than this
    /// frame's; meanwhile the region counts as the player's position for activation.
    Join {
        /// The region the player's entity is spawned in.
        region: RegionId,
    },
    /// The player leaves. Applied at once; a `Leave` for a `Join` not yet applied cancels
    /// it.
    Leave,
}

/// Writes a player's quantised input into the world: the typed half of input application,
/// implemented beside the input component's definition in `lactose-input` (or by `app` and
/// tests for a component of their own) and inserted by
/// [`SimulationBuilder::inputs`](crate::SimulationBuilder::inputs) as the service resource
/// `dyn InputApplier`.
///
/// The sync point calls it once per entry of [`TickInput::inputs`], in ascending `PlayerId`
/// order, after [`World::set_input_tick`] has recorded the slot. The implementation decodes
/// `sample.sample` with the component's own validation, resolves the player's entity from
/// world state (the row with [`ClaimedBy`](lactose_ecs::ClaimedBy) for the player that also
/// carries [`StreamingAnchor`](crate::StreamingAnchor), which the [`SessionApplier`]
/// spawned; never a side table, which no frame could keep current) and writes the value
/// through [`World::insert`] or [`World::get_mut`](lactose_ecs::World::get_mut). An error
/// is logged at target `curd` and the input is dropped; the tick runs. The component is
/// `derived`, or its crate adds an exclusive system of its own in a set after
/// [`sets::INPUT`](crate::sets::INPUT) that declares `writes = [Sample]`, so transient
/// readers have a writer.
///
/// The same implementation serves the server (samples from `net`), the prediction world
/// (the local player's own sample) and a replay (samples from the journal).
pub trait InputApplier: Send + Sync + 'static {
    /// Writes `sample` for `player` this tick.
    fn apply(&mut self, world: &mut World, player: PlayerId, sample: &PlayerSample) -> Result<()>;
}

/// Spawns and despawns players' entities on the server: the typed half of session
/// application, implemented by `app` or the game (the entity's components are the
/// game's) and inserted by
/// [`SimulationBuilder::sessions`](crate::SimulationBuilder::sessions) as the service
/// resource `dyn SessionApplier`.
///
/// The sync point calls it once per [`SessionEvent`] it applies, in ascending `PlayerId`
/// order. For a `Join` the implementation spawns the player's entity in the event's
/// region, with [`ClaimedBy`](lactose_ecs::ClaimedBy) for the player and
/// [`StreamingAnchor`](crate::StreamingAnchor), through [`World::spawn`] with a key from
/// [`World::allocate_net_id`](lactose_ecs::World::allocate_net_id). For a `Leave` it
/// despawns that entity, or removes the claim and the anchor, resolving it from world
/// state as the [`InputApplier`] does. An error is logged at target `curd`; the tick runs.
pub trait SessionApplier: Send + Sync + 'static {
    /// Applies `event` for `player`.
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()>;
}

/// Applies staged state to the world at the input sync point: the hook `lactose-net` fills
/// on a client (received state into `Replicated` rows, architecture section 9.2) and
/// `lactose-predict` fills in the prediction world (the kinematic proxies' per-tick history,
/// section 9.4). Inserted by
/// [`SimulationBuilder::staged_state`](crate::SimulationBuilder::staged_state) as the service
/// resource `dyn StagedState`; the server leaves the default, which does nothing.
///
/// The sync point calls it once per tick after the inputs have been applied. Everything a
/// client receives is written here and nowhere else: spawns with the server's key through
/// [`World::spawn`], binds through [`World::bind`](lactose_ecs::World::bind), whole
/// components through [`World::insert`], tracked columns through
/// [`World::columns_mut`](lactose_ecs::World::columns_mut) or
/// [`Mut::set_if_neq`](lactose_ecs::Mut::set_if_neq) so an equal value does not dirty the
/// row, despawns from tombstones. An error is logged at target `curd`; the tick runs.
pub trait StagedState: Send + Sync + 'static {
    /// Writes what was staged for `tick`.
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()>;
}
