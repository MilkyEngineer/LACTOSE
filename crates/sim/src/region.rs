//! Region lifecycle (architecture sections 6, 9.7 and 14): activation by anchor position
//! within a radius measured in regions, streaming ahead of it, release behind it, and an
//! unload that writes the same records as a save.

use std::collections::{BTreeMap, BTreeSet};

use lactose_asset_api::AssetFailure;
use lactose_core::{Error, PlayerId, Result, Tick};
use lactose_ecs::{RegionId, RegionRows};
use lactose_reflect::{component, resource};

/// A region's place on the world grid, in regions. The radii of [`RegionPolicy`] are
/// distances between cells.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RegionCell {
    /// Column.
    pub x: i32,
    /// Row.
    pub y: i32,
}

impl RegionCell {
    /// The distance to `other` in regions: the larger of the two axis distances
    /// (Chebyshev), so a radius of one covers the eight neighbours. Exact in integers.
    #[must_use]
    pub const fn distance(self, other: Self) -> u32 {
        todo!()
    }
}

/// Where every region of the world sits: the cooked region layout, handed to
/// [`SimulationBuilder::regions`](crate::SimulationBuilder::regions) inside
/// [`RegionControl::Anchored`]. Immutable for the life of a simulation. Two regions never
/// share a cell.
#[derive(Clone, Debug, Default)]
pub struct RegionAtlas {
    cells: BTreeMap<RegionId, RegionCell>,
    regions: BTreeMap<RegionCell, RegionId>,
}

impl RegionAtlas {
    /// An empty atlas.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Places `region` at `cell`. Fails with `InvalidData` if the region or the cell is
    /// already in the atlas.
    pub fn insert(&mut self, region: RegionId, cell: RegionCell) -> Result<()> {
        todo!()
    }

    /// The cell of `region`, if the atlas has it.
    #[must_use]
    pub fn cell(&self, region: RegionId) -> Option<RegionCell> {
        todo!()
    }

    /// The region at `cell`, if any.
    #[must_use]
    pub fn region_at(&self, cell: RegionCell) -> Option<RegionId> {
        todo!()
    }

    /// How many regions the atlas holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether the atlas is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The regions within `radius` of `centre` ([`RegionCell::distance`] at most `radius`),
    /// `centre`'s own region included if it exists, in ascending `RegionId` order.
    #[must_use]
    pub fn within(&self, centre: RegionCell, radius: u32) -> Within<'_> {
        todo!()
    }
}

/// Iterates the regions within a radius of a cell, ascending by `RegionId`.
pub struct Within<'a> {
    atlas: &'a RegionAtlas,
    centre: RegionCell,
    radius: u32,
    /// The region yielded last, so the walk resumes after it.
    last: Option<RegionId>,
}

impl Iterator for Within<'_> {
    type Item = RegionId;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

/// The deployment parameters of activation, streaming and release (architecture sections
/// 9.7 and 14), a world resource the server's `app` inserts and the overload ladder
/// adjusts: the ladder's first rung shrinks `activation_radius`, and the ladder itself,
/// with its hysteresis, is `app`'s. Read by the lifecycle under
/// [`RegionControl::Anchored`] only; not saved, not hashed.
///
/// `release_radius >= streaming_radius > activation_radius` is required at build. At run
/// time the lifecycle clamps what it reads, logging the clamp: the activation radius to
/// `streaming_radius - 1` and the release radius up to `streaming_radius`, so that an
/// active region is always loaded, an entity in a `Simulated` region can only ever cross
/// into a loaded one (which freezes it with the region, section 9.7), and nothing that was
/// just requested is released.
#[resource(key = "sim::region_policy", derived)]
#[derive(PartialEq, Eq, Debug)]
pub struct RegionPolicy {
    /// A region within this many regions of an anchor is `Simulated` once its bundle is
    /// loaded and its asset table has entered.
    #[field(id = 1)]
    pub activation_radius: u32,
    /// A region within this many regions of an anchor is requested and, once loaded, kept
    /// `Frozen` until it also activates. Streaming leads the anchor by
    /// `streaming_radius - activation_radius` regions.
    #[field(id = 2)]
    pub streaming_radius: u32,
    /// A loaded region stays loaded while within this many regions of an anchor; outside
    /// it, it is exported and unloaded, and a request for it is cancelled. The gap to the
    /// streaming radius is the hysteresis that keeps an anchor jittering on a border from
    /// loading and unloading the ring every tick.
    #[field(id = 3)]
    pub release_radius: u32,
}

/// Marks an entity whose position activates regions: a player's own entity, which the
/// [`SessionApplier`](crate::SessionApplier) spawns with it beside
/// [`ClaimedBy`](lactose_ecs::ClaimedBy) and despawns on leave, or a spectator the game
/// spawns. The region an anchor resides in is its position for the policy; a claimed
/// object the player threw is not an anchor, so a thrown bomb activates nothing. Session
/// state like `ClaimedBy`: not saved, not hashed, not replicated. A pending `Join` whose
/// region is not loaded yet counts as an anchor at its region until it is applied.
#[component(key = "sim::streaming_anchor", derived)]
#[derive(PartialEq, Eq, Debug)]
pub struct StreamingAnchor {
    /// Reserved for a per-anchor priority once streaming budgets exist (milestone 3);
    /// zero until then.
    #[field(id = 1)]
    pub priority: u32,
}

/// Who decides which regions are loaded and in which mode.
#[derive(Clone, Debug)]
pub enum RegionControl {
    /// The server's policy (architecture section 9.7): at every input sync point, last
    /// among `sim`'s systems, the lifecycle reads the anchors and [`RegionPolicy`],
    /// requests loads within the streaming radius, cancels requests and unloads outside the
    /// release radius, and sets every loaded region `Simulated` or `Frozen`. A bundle for a
    /// region that is neither requested nor within the release radius of an anchor is
    /// refused. The frame's `unloaded` and `modes` are ignored, and so is a bundle's
    /// `mode`. Only a `Server` world may use it, and the atlas must not be empty.
    Anchored(RegionAtlas),
    /// The frame decides: bundles load in the mode they carry, `unloaded` unloads, `modes`
    /// applies, nothing is requested. What a client or prediction world uses, since the
    /// server tells a client what is relevant, and what a test or the headless app uses to
    /// hold a region `Simulated` without anchors. The default.
    Directed,
}

/// What the lifecycle asks of the side outside the boundary, taken with
/// [`RegionLifecycle::take_requests`]. Under [`RegionControl::Anchored`] only.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum RegionRequest {
    /// Load this region: fetch its baseline and overlay, resolve its asset set, and answer
    /// through [`TickInput::loaded`](crate::TickInput::loaded). Issued once; a region stays
    /// requested until its bundle loads or the request is cancelled.
    Load {
        /// The region.
        region: RegionId,
    },
    /// The region left the release radius before its bundle arrived: the request is
    /// withdrawn, and a bundle that still arrives is refused.
    Cancel {
        /// The region.
        region: RegionId,
    },
    /// This region was exported and unloaded; its bundle and pinned table may be released.
    Release {
        /// The region.
        region: RegionId,
    },
}

/// A region's state as an unload or a save copied it out: the rows and resources of
/// [`World::export_region`](lactose_ecs::World::export_region). `lactose-persist` diffs the
/// rows against the baseline into the overlay of architecture section 15.2, and that
/// overlay is what the server sends a client on relevance: the server's `app` runs
/// `persist`'s diff on the export and `net` sends the result. An unload and a save produce
/// the export through the one path, so the records are the same.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegionExport {
    /// The region.
    pub region: RegionId,
    /// Its rows, seed and resources.
    pub rows: RegionRows,
}

/// A bundle the sync point refused, with the error, so `lactose-persist` can migrate or
/// `app` can retry. Nothing was loaded; under `Anchored` a requested region stays
/// requested.
#[derive(Debug)]
pub struct LoadFailure {
    /// The tick whose sync point refused the bundle.
    pub tick: Tick,
    /// The region.
    pub region: RegionId,
    /// Why: `VersionMismatch` for an overlay against another baseline or a component or
    /// resource schema the registry disagrees with; `InvalidData` for a bundle that does
    /// not fit, a region that is loaded already, or, under `Anchored`, a region that is
    /// neither requested nor within the release radius ("not wanted"); `NotFound` for a
    /// region the atlas lacks under `Anchored`.
    pub error: Error,
}

/// An asset failure stamped with the tick it entered: the tick-stamped event of
/// architecture section 14, which a client turns into a resync request.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AssetFailureEvent {
    /// The tick whose frame carried the failure.
    pub tick: Tick,
    /// The failure.
    pub failure: AssetFailure,
}

/// The region lifecycle's state beside the world: the control mode, what has been
/// requested, joins waiting for their region, and the outputs of the last sync points.
/// Reached through [`Simulation::lifecycle`](crate::Simulation::lifecycle); no system can
/// name it.
///
/// Everything here but the failures is server-only session state, which the server never
/// rolls back: it is not in a snapshot, and architecture section 10's list of what a
/// restore keeps is unchanged. A prediction world's lifecycle holds failures alone.
#[derive(Debug)]
pub struct RegionLifecycle {
    control: RegionControl,
    requested: BTreeSet<RegionId>,
    /// Joins whose region is not loaded yet, by player.
    held_joins: BTreeMap<PlayerId, RegionId>,
    requests: Vec<RegionRequest>,
    exports: Vec<RegionExport>,
    load_failures: Vec<LoadFailure>,
    asset_failures: Vec<AssetFailureEvent>,
}

impl RegionLifecycle {
    pub(crate) fn new(control: RegionControl) -> Self {
        Self {
            control,
            requested: BTreeSet::new(),
            held_joins: BTreeMap::new(),
            requests: Vec::new(),
            exports: Vec::new(),
            load_failures: Vec::new(),
            asset_failures: Vec::new(),
        }
    }

    /// Who decides loads and modes.
    #[must_use]
    pub fn control(&self) -> &RegionControl {
        &self.control
    }

    /// Whether a load of `region` has been requested and neither answered nor cancelled.
    #[must_use]
    pub fn is_requested(&self, region: RegionId) -> bool {
        todo!()
    }

    /// The region a `Join` for `player` waits for, if one is held.
    #[must_use]
    pub fn held_join(&self, player: PlayerId) -> Option<RegionId> {
        todo!()
    }

    /// The requests issued since the last take, in the order they were issued.
    pub fn take_requests(&mut self) -> Vec<RegionRequest> {
        todo!()
    }

    /// The regions exported and unloaded since the last take, in the order they were
    /// unloaded: what `lactose-persist` writes as the regions' records. Always empty on a
    /// world that is not `Server`, whose unloads do not export.
    pub fn take_exports(&mut self) -> Vec<RegionExport> {
        todo!()
    }

    /// The bundles refused since the last take.
    pub fn take_load_failures(&mut self) -> Vec<LoadFailure> {
        todo!()
    }

    /// The asset failures stamped since the last take, in tick order and, within a tick,
    /// in the order the frame carried them.
    pub fn take_asset_failures(&mut self) -> Vec<AssetFailureEvent> {
        todo!()
    }
}
