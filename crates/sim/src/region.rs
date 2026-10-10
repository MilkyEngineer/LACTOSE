//! Region lifecycle (architecture sections 6, 9.7 and 14): activation by anchor position
//! within a radius measured in regions, streaming ahead of it, release behind it, pinned
//! regions that are always simulated, and an unload that writes the same records as a
//! save.

use core::marker::PhantomData;
use std::collections::{BTreeMap, BTreeSet};

use lactose_asset_api::AssetFailure;
use lactose_core::{Error, ErrorKind, PlayerId, Result, Tick};
use lactose_ecs::{RegionConfig, RegionId, RegionMode, RegionRows, World, WorldKind};
use lactose_reflect::{Persistence, Registry, TypeRole, component, resource};

use crate::input::{RegionBundle, SessionApplier, SessionEvent, TickInput};

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
        let dx = self.x.abs_diff(other.x);
        let dy = self.y.abs_diff(other.y);
        if dx > dy { dx } else { dy }
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
        if self.cells.contains_key(&region) {
            return Err(Error::new(ErrorKind::InvalidData, "the region is in the atlas already"));
        }
        if self.regions.contains_key(&cell) {
            return Err(Error::new(ErrorKind::InvalidData, "another region is at that cell"));
        }
        self.cells.insert(region, cell);
        self.regions.insert(cell, region);
        Ok(())
    }

    /// The cell of `region`, if the atlas has it.
    #[must_use]
    pub fn cell(&self, region: RegionId) -> Option<RegionCell> {
        self.cells.get(&region).copied()
    }

    /// The region at `cell`, if any.
    #[must_use]
    pub fn region_at(&self, cell: RegionCell) -> Option<RegionId> {
        self.regions.get(&cell).copied()
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
    ///
    /// Costs O(m log n) for an atlas of n regions, where m is the smaller of n and the
    /// (2r + 1)² cells within radius r: those cells are looked up when there are fewer of
    /// them than regions, and the atlas is scanned otherwise. The regions are collected
    /// when this is called.
    #[must_use]
    pub fn within(&self, centre: RegionCell, radius: u32) -> Within<'_> {
        let mut regions = Vec::new();
        let side = u64::from(radius) * 2 + 1;
        if side.saturating_mul(side) < self.cells.len() as u64 {
            let r = i64::from(radius);
            for y in i64::from(centre.y) - r..=i64::from(centre.y) + r {
                for x in i64::from(centre.x) - r..=i64::from(centre.x) + r {
                    if let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(y))
                        && let Some(region) = self.region_at(RegionCell { x, y })
                    {
                        regions.push(region);
                    }
                }
            }
            regions.sort_unstable();
        } else {
            let near = self.cells.iter().filter(|(_, cell)| cell.distance(centre) <= radius);
            regions.extend(near.map(|(&region, _)| region));
        }
        Within { regions: regions.into_iter(), atlas: PhantomData }
    }
}

/// Iterates the regions within a radius of a cell, ascending by `RegionId`.
pub struct Within<'a> {
    regions: std::vec::IntoIter<RegionId>,
    atlas: PhantomData<&'a RegionAtlas>,
}

impl Iterator for Within<'_> {
    type Item = RegionId;

    fn next(&mut self) -> Option<Self::Item> {
        self.regions.next()
    }
}

/// The deployment parameters of activation, streaming and release (architecture sections
/// 9.7 and 14), a world resource the server's `app` inserts and the overload ladder
/// adjusts: the ladder's first rung shrinks `activation_radius`, and the ladder itself,
/// with its hysteresis, is `app`'s. Read by the lifecycle under
/// [`RegionControl::Anchored`] only; not saved, not hashed.
///
/// `release_radius >= streaming_radius > activation_radius` is required at build. At run
/// time the lifecycle clamps what it reads, logging the clamp: the streaming radius up to
/// one, the activation radius to `streaming_radius - 1` and the release radius up to
/// `streaming_radius`, so that an
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
/// [`SessionApplier`](crate::SessionApplier) writes on join beside
/// [`ClaimedBy`](lactose_ecs::ClaimedBy) and removes or despawns on leave, or a spectator
/// the game spawns. The region an anchor resides in is its position for the policy; a
/// claimed object the player threw is not an anchor, so a thrown bomb activates nothing.
/// Session state like `ClaimedBy`: not saved, not hashed, not replicated. A `Join` whose
/// region is not loaded yet counts as an anchor at its region until it is applied. In the
/// prediction world `lactose-predict` writes it on the local player's entity so the
/// appliers' lookup resolves; under `Directed` it has no policy effect.
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
    /// release radius, and sets every loaded region `Simulated` or `Frozen`. `pinned`
    /// regions are always wanted and never released: requested at the first sync point,
    /// `Simulated` once loaded with a table, whatever the anchors; world-global persistent
    /// state is a region resource of a pinned region. A bundle for a region that is
    /// neither requested nor within the release radius of an anchor is refused. The
    /// frame's `unloaded` and `modes` are ignored, and so is a bundle's `mode`. Only a
    /// `Server` world may use it, the atlas must not be empty, and every pinned region must
    /// be in the atlas.
    Anchored {
        /// Where every region sits.
        atlas: RegionAtlas,
        /// The regions that are always wanted.
        pinned: BTreeSet<RegionId>,
    },
    /// The frame decides: bundles load in the mode they carry, `unloaded` unloads, `modes`
    /// applies, nothing is requested. What a client or prediction world uses, since the
    /// server tells a client what is relevant, and what a test or the headless app uses to
    /// hold a region `Simulated` without anchors. The default.
    Directed,
}

/// What the lifecycle asks of the side outside the boundary, taken with
/// [`RegionLifecycle::take_requests`]. `Load` and `Cancel` under
/// [`RegionControl::Anchored`] only; `Release` after every unload of a `Server` world,
/// under either control.
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
    /// Not issued for a region the same frame unloads and loads again, which is replaced
    /// rather than released.
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

/// A session event the sync point refused: a `Join` for a region the atlas lacks
/// (`NotFound`), which is neither held nor applied.
#[derive(Debug)]
pub struct SessionFailure {
    /// The tick whose sync point refused the event.
    pub tick: Tick,
    /// The player.
    pub player: PlayerId,
    /// Why.
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
/// restore keeps is unchanged. A journal records it with its checkpoint
/// ([`journal::Journal`](crate::journal::Journal)), since a save does not. A prediction
/// world's lifecycle holds failures alone.
#[derive(Debug)]
pub struct RegionLifecycle {
    control: RegionControl,
    requested: BTreeSet<RegionId>,
    /// Joins whose region is not loaded yet, by player.
    held_joins: BTreeMap<PlayerId, RegionId>,
    requests: Vec<RegionRequest>,
    exports: Vec<RegionExport>,
    load_failures: Vec<LoadFailure>,
    session_failures: Vec<SessionFailure>,
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
            session_failures: Vec::new(),
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
        self.requested.contains(&region)
    }

    /// The region a `Join` for `player` waits for, if one is held.
    #[must_use]
    pub fn held_join(&self, player: PlayerId) -> Option<RegionId> {
        self.held_joins.get(&player).copied()
    }

    /// Replaces the held joins and the requested regions with those a journal recorded at
    /// its checkpoint ([`Journal::held_joins`](crate::journal::Journal::held_joins) and
    /// [`Journal::requested`](crate::journal::Journal::requested)): what a replay driver
    /// calls through [`Simulation::lifecycle_mut`](crate::Simulation::lifecycle_mut) before
    /// its first step, since a save carries no session state. Nothing is issued for them:
    /// the requests were issued in the recorded run. Like a write through `world_mut`, it
    /// is not journaled.
    pub fn restore_session_state(
        &mut self,
        held_joins: BTreeMap<PlayerId, RegionId>,
        requested: BTreeSet<RegionId>,
    ) {
        self.held_joins = held_joins;
        self.requested = requested;
    }

    /// The requests issued since the last take, in the order they were issued.
    pub fn take_requests(&mut self) -> Vec<RegionRequest> {
        core::mem::take(&mut self.requests)
    }

    /// The regions exported and unloaded since the last take, in the order they were
    /// unloaded: what `lactose-persist` writes as the regions' records. Always empty on a
    /// world that is not `Server`, whose unloads do not export.
    pub fn take_exports(&mut self) -> Vec<RegionExport> {
        core::mem::take(&mut self.exports)
    }

    /// The bundles refused since the last take.
    pub fn take_load_failures(&mut self) -> Vec<LoadFailure> {
        core::mem::take(&mut self.load_failures)
    }

    /// The session events refused since the last take.
    pub fn take_session_failures(&mut self) -> Vec<SessionFailure> {
        core::mem::take(&mut self.session_failures)
    }

    /// The asset failures stamped since the last take, in tick order and, within a tick,
    /// in the order the frame carried them.
    pub fn take_asset_failures(&mut self) -> Vec<AssetFailureEvent> {
        core::mem::take(&mut self.asset_failures)
    }

    // ---- The input sync point (README, "Regions") ----

    /// `sim::apply_frame_regions`: the frame's failures, unloads, bundles, tables and
    /// modes, in that order, taken out of `frame`.
    pub(crate) fn apply_frame(
        &mut self,
        world: &mut World,
        policy: &RegionPolicy,
        frame: &mut TickInput,
    ) {
        let tick = world.tick();
        for failure in core::mem::take(&mut frame.failures) {
            self.asset_failures.push(AssetFailureEvent { tick, failure });
        }
        let directed = matches!(self.control, RegionControl::Directed);

        let unloaded = core::mem::take(&mut frame.unloaded);
        // A region this frame unloads and loads again is replaced, not released: the
        // `Release` its unload issued is withdrawn once the bundle loads.
        let first_request = self.requests.len();
        let mut replaced = BTreeSet::new();
        if directed {
            for region in unloaded {
                if is_loaded(world, region) {
                    self.unload(world, region);
                    replaced.insert(region);
                } else {
                    tracing::debug!(
                        target: "curd",
                        region = region.0,
                        "unload of a region that is not loaded: ignored"
                    );
                }
            }
        } else if !unloaded.is_empty() {
            tracing::debug!(
                target: "curd",
                count = unloaded.len(),
                "the frame's unloads are ignored under `Anchored`"
            );
        }

        // This frame's joins count as anchors in the bundle check, so a join and its
        // region's bundle in one frame load and apply together.
        let joins: Vec<RegionId> = frame
            .sessions
            .values()
            .filter_map(|event| match *event {
                SessionEvent::Join { region } => Some(region),
                SessionEvent::Leave => None,
            })
            .collect();
        let anchors = match &self.control {
            RegionControl::Anchored { atlas, .. } => self.anchor_cells(world, atlas, &joins),
            RegionControl::Directed => BTreeSet::new(),
        };
        let release = Radii::clamped(policy).release;
        for (region, bundle) in core::mem::take(&mut frame.loaded) {
            if let Err(error) = self.load(world, &anchors, release, region, &bundle) {
                tracing::warn!(target: "curd", region = region.0, %error, "region bundle refused");
                self.load_failures.push(LoadFailure { tick, region, error });
            }
        }
        replaced.retain(|&region| is_loaded(world, region));
        if !replaced.is_empty() {
            let mut index = 0;
            self.requests.retain(|request| {
                let release = match *request {
                    RegionRequest::Release { region } => replaced.contains(&region),
                    RegionRequest::Load { .. } | RegionRequest::Cancel { .. } => false,
                };
                let withdrawn = index >= first_request && release;
                index += 1;
                !withdrawn
            });
        }

        for (region, table) in core::mem::take(&mut frame.assets) {
            if world.set_region_assets(region, table).is_err() {
                tracing::debug!(
                    target: "curd",
                    region = region.0,
                    "asset table for a region that is not loaded: dropped"
                );
            }
        }

        let modes = core::mem::take(&mut frame.modes);
        if directed {
            for (region, mode) in modes {
                if world.set_region_mode(region, mode).is_err() {
                    tracing::debug!(
                        target: "curd",
                        region = region.0,
                        "mode for a region that is not loaded: ignored"
                    );
                }
            }
        } else if !modes.is_empty() {
            tracing::debug!(
                target: "curd",
                count = modes.len(),
                "the frame's modes are ignored under `Anchored`"
            );
        }
    }

    /// Loads one bundle, or refuses it having changed nothing.
    fn load(
        &mut self,
        world: &mut World,
        anchors: &BTreeSet<RegionCell>,
        release: u32,
        region: RegionId,
        bundle: &RegionBundle,
    ) -> Result<()> {
        if is_loaded(world, region) {
            return Err(Error::new(ErrorKind::InvalidData, "the region is loaded already"));
        }
        let mode = match &self.control {
            RegionControl::Directed => bundle.mode,
            RegionControl::Anchored { atlas, pinned } => {
                let cell = atlas.cell(region).ok_or_else(|| {
                    Error::new(ErrorKind::NotFound, "the region is not in the atlas")
                })?;
                let wanted = pinned.contains(&region)
                    || self.requested.contains(&region)
                    || anchors.iter().any(|anchor| anchor.distance(cell) <= release);
                if !wanted {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "not wanted: neither pinned, nor requested, nor within the release radius \
                         of an anchor",
                    ));
                }
                if bundle.mode != RegionMode::Frozen {
                    tracing::debug!(
                        target: "curd",
                        region = region.0,
                        "a bundle's mode is ignored under `Anchored`"
                    );
                }
                RegionMode::Frozen
            }
        };
        if let Some(overlay) = &bundle.overlay {
            overlay.check_against(&bundle.baseline)?;
        }
        check_resources(world.registry(), bundle)?;

        world.create_region(RegionConfig { id: region, seed: bundle.seed, mode })?;
        let filled =
            world.load_region(region, &bundle.baseline, bundle.overlay.as_deref()).and_then(|()| {
                // Validated above, so none of these fails; if one did, the region goes.
                for resource in &bundle.resources {
                    world.insert_region_resource_erased(region, resource.clone())?;
                }
                for &resource in &bundle.transient_resources {
                    world.insert_region_resource_zeroed(region, resource)?;
                }
                if let Some(table) = &bundle.assets {
                    world.set_region_assets(region, table.clone())?;
                }
                Ok(())
            });
        if let Err(error) = filled {
            // `load_region` validates before it mutates, so the region is still empty and
            // removing it leaves no row and no log entry.
            let _ = world.unload_region(region);
            return Err(error);
        }
        self.requested.remove(&region);
        Ok(())
    }

    /// `sim::apply_sessions` on a `Server` world: each held join whose region is now
    /// loaded and each of the frame's sessions, in ascending `PlayerId` order.
    pub(crate) fn apply_sessions(
        &mut self,
        world: &mut World,
        applier: &mut dyn SessionApplier,
        sessions: BTreeMap<PlayerId, SessionEvent>,
    ) {
        let tick = world.tick();
        // Sessions load nothing, so the loaded set holds until the appliers run.
        let loaded: BTreeSet<RegionId> = world.regions().collect();
        let mut due: BTreeMap<PlayerId, SessionEvent> = self
            .held_joins
            .iter()
            .filter(|&(_, region)| loaded.contains(region))
            .map(|(&player, &region)| (player, SessionEvent::Join { region }))
            .collect();
        for (player, event) in sessions {
            match event {
                SessionEvent::Join { region } => {
                    if let RegionControl::Anchored { atlas, .. } = &self.control
                        && atlas.cell(region).is_none()
                    {
                        tracing::warn!(
                            target: "curd",
                            player = player.0,
                            region = region.0,
                            "join for a region the atlas lacks: refused"
                        );
                        let error =
                            Error::new(ErrorKind::NotFound, "the region is not in the atlas");
                        self.session_failures.push(SessionFailure { tick, player, error });
                        continue;
                    }
                    // A newer join replaces a held one.
                    if loaded.contains(&region) {
                        self.held_joins.remove(&player);
                        due.insert(player, event);
                    } else {
                        self.held_joins.insert(player, region);
                        due.remove(&player);
                    }
                }
                SessionEvent::Leave => {
                    if self.held_joins.remove(&player).is_some() {
                        // The join never reached the world: cancelling it is the whole leave.
                        due.remove(&player);
                    } else {
                        due.insert(player, event);
                    }
                }
            }
        }
        for (player, event) in due {
            if matches!(event, SessionEvent::Join { .. }) {
                self.held_joins.remove(&player);
            }
            if let Err(error) = applier.apply(world, player, event) {
                tracing::warn!(
                    target: "curd",
                    player = player.0,
                    ?event,
                    %error,
                    "session applier failed: the event is skipped"
                );
            }
        }
    }

    /// `sim::apply_region_policy`: under `Anchored`, unloads, cancels, requests and modes;
    /// under `Directed`, nothing.
    pub(crate) fn apply_policy(&mut self, world: &mut World, policy: &RegionPolicy) {
        let RegionControl::Anchored { atlas, pinned } = &self.control else {
            return;
        };
        let radii = Radii::clamped(policy);
        let anchors = self.anchor_cells(world, atlas, &[]);
        let within = |radius: u32| {
            let mut set = pinned.clone();
            for &cell in &anchors {
                set.extend(atlas.within(cell, radius));
            }
            set
        };
        let (active, wanted, kept) =
            (within(radii.activation), within(radii.streaming), within(radii.release));

        let loaded: Vec<RegionId> = world.regions().collect();
        for &region in loaded.iter().filter(|region| !kept.contains(region)) {
            self.unload(world, region);
        }
        let loaded: BTreeSet<RegionId> = world.regions().collect();
        let cancelled: Vec<RegionId> =
            self.requested.iter().copied().filter(|region| !kept.contains(region)).collect();
        for region in cancelled {
            self.requested.remove(&region);
            self.requests.push(RegionRequest::Cancel { region });
        }
        for region in wanted {
            if !loaded.contains(&region) && self.requested.insert(region) {
                self.requests.push(RegionRequest::Load { region });
            }
        }
        for region in loaded {
            let simulated = active.contains(&region) && world.region_assets(region).is_some();
            let mode = if simulated { RegionMode::Simulated } else { RegionMode::Frozen };
            world.set_region_mode(region, mode).expect("the region is loaded");
        }
    }

    /// The cells of every anchor: the regions of the rows with [`StreamingAnchor`], of the
    /// held joins and of `joins`, whatever their mode.
    fn anchor_cells(
        &self,
        world: &World,
        atlas: &RegionAtlas,
        joins: &[RegionId],
    ) -> BTreeSet<RegionCell> {
        let mut regions: BTreeSet<RegionId> = BTreeSet::new();
        for column in world.columns::<StreamingAnchor>() {
            regions.extend(column.regions());
        }
        regions.extend(self.held_joins.values());
        regions.extend(joins);
        regions.into_iter().filter_map(|region| atlas.cell(region)).collect()
    }

    /// Unloads `region`: on a `Server` world it is exported first and released after.
    fn unload(&mut self, world: &mut World, region: RegionId) {
        let server = world.kind() == WorldKind::Server;
        if server {
            match world.export_region(region) {
                Ok(rows) => self.exports.push(RegionExport { region, rows }),
                Err(error) => {
                    // Every server row is bound at apply, so this is a broken invariant;
                    // the region stays loaded rather than losing its state.
                    tracing::error!(
                        target: "curd",
                        region = region.0,
                        %error,
                        "region export failed: the region stays loaded"
                    );
                    return;
                }
            }
        }
        world.unload_region(region).expect("the region is loaded");
        if server {
            self.requests.push(RegionRequest::Release { region });
        }
    }
}

/// The policy's radii as the lifecycle uses them: clamped so that `release >= streaming >
/// activation` holds whatever the overload ladder wrote.
struct Radii {
    activation: u32,
    streaming: u32,
    release: u32,
}

impl Radii {
    fn clamped(policy: &RegionPolicy) -> Self {
        let mut streaming = policy.streaming_radius;
        if streaming == 0 {
            tracing::debug!(target: "curd", "streaming radius clamped up to one");
            streaming = 1;
        }
        let mut activation = policy.activation_radius;
        let mut release = policy.release_radius;
        let ceiling = streaming.saturating_sub(1);
        if activation > ceiling {
            tracing::debug!(
                target: "curd",
                activation,
                streaming,
                "activation radius clamped below the streaming radius"
            );
            activation = ceiling;
        }
        if release < streaming {
            tracing::debug!(
                target: "curd",
                release,
                streaming,
                "release radius clamped up to the streaming radius"
            );
            release = streaming;
        }
        Self { activation, streaming, release }
    }
}

/// Whether `region` is loaded.
fn is_loaded(world: &World, region: RegionId) -> bool {
    world.regions().any(|loaded| loaded == region)
}

/// Checks a bundle's region resources against the registry before anything is created.
fn check_resources(registry: &Registry, bundle: &RegionBundle) -> Result<()> {
    let invalid = |message: String| Err(Error::new(ErrorKind::InvalidData, message));
    let mut seen = BTreeSet::new();
    for data in &bundle.resources {
        let key = data.component;
        let Some(info) = registry.get(key).filter(|info| {
            info.role() == TypeRole::Resource && info.persistence() == Persistence::Persistent
        }) else {
            return invalid(format!(
                "resource {:#018x} is not a registered persistent resource",
                key.0
            ));
        };
        if data.bytes.len() != info.layout().size() {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!("the bundle's `{}` is not the registry's size", info.name()),
            ));
        }
        info.validate(&data.bytes)?;
        if !seen.insert(key) {
            return invalid(format!("the bundle carries `{}` twice", info.name()));
        }
    }
    for &key in &bundle.transient_resources {
        let transient = registry.get(key).is_some_and(|info| {
            info.role() == TypeRole::Resource && info.persistence() == Persistence::Transient
        });
        if !transient {
            return invalid(format!(
                "resource {:#018x} is not a registered transient resource",
                key.0
            ));
        }
        if !seen.insert(key) {
            return invalid(format!("the bundle carries resource {:#018x} twice", key.0));
        }
    }
    Ok(())
}
