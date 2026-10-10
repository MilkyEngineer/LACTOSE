//! The input sync point (architecture sections 6, 9.2 and 14): this crate's exclusive
//! systems, placed first in `Stage::Input` under `sets::INPUT` and ordered among themselves
//! by `after` edges, so the frame is applied in one fixed order in every world:
//!
//! 1. `begin_tick`: `CurrentTick` takes the world's tick; `TimeScale` takes the frame's.
//! 2. `apply_frame_regions`: failures stamped, then under `Directed` the frame's unloads,
//!    then bundles loaded, tables applied and, under `Directed`, modes.
//! 3. `apply_sessions`: joins whose region is loaded and leaves, through the
//!    `SessionApplier`.
//! 4. `apply_inputs`: each player's input tick, then its sample through the
//!    `InputApplier`.
//! 5. `apply_staged_state`: the `StagedState` hook.
//! 6. `apply_region_policy`: under `Anchored`, requests, cancels, unloads and modes.
//!
//! The module is private: the builder places the descriptors, and a consumer orders itself
//! only through `sets::INPUT`. `save_copy` is the one system of the save pass. Every
//! system takes what it applies out of the frame slot, so a frame is applied once.

use lactose_core::{PlayerId, Result, Tick};
use lactose_ecs::{Job, RegionId, Res, ResMut, SystemDescriptor, World, WorldKind, system};

use crate::input::{InputApplier, PlayerSample, SessionApplier, SessionEvent, StagedState};
use crate::region::RegionPolicy;
use crate::simulation::{LifecycleSlot, SavePoint, SaveSlot, TickInputSlot};
use crate::time::{CurrentTick, TimeScale};

/// The six systems of `sets::INPUT`, in their order.
pub(crate) static INPUT_SYSTEMS: [&SystemDescriptor; 6] = [
    &BEGIN_TICK,
    &APPLY_FRAME_REGIONS,
    &APPLY_SESSIONS,
    &APPLY_INPUTS,
    &APPLY_STAGED_STATE,
    &APPLY_REGION_POLICY,
];

/// Writes the tick being run into [`CurrentTick`] and the frame's time scale, if any, into
/// [`TimeScale`].
#[system(name = "sim::begin_tick", worlds = [server, client, prediction], exclusive, in_set = crate::sets::INPUT)]
pub(crate) fn begin_tick(
    world: &mut World,
    now: ResMut<CurrentTick>,
    scale: ResMut<TimeScale>,
    frame: ResMut<TickInputSlot>,
) {
    *now = CurrentTick::new(world.tick());
    if let Some(time_scale) = frame.0.as_ref().and_then(|input| input.time_scale) {
        *scale = time_scale;
    }
}

/// Applies the frame's region entries; the README states the order and the rules.
#[system(
    name = "sim::apply_frame_regions",
    worlds = [server, client, prediction],
    exclusive,
    in_set = crate::sets::INPUT,
    after = begin_tick
)]
pub(crate) fn apply_frame_regions(
    world: &mut World,
    lifecycle: ResMut<LifecycleSlot>,
    frame: ResMut<TickInputSlot>,
    policy: Res<RegionPolicy>,
) {
    if let Some(input) = frame.0.as_mut() {
        lifecycle.0.apply_frame(world, &policy, input);
    }
}

/// Applies the frame's sessions and the held joins whose region is now loaded, in ascending
/// `PlayerId` order, through the [`SessionApplier`]. On a world that is not `Server` the
/// frame's sessions are dropped and logged.
#[system(
    name = "sim::apply_sessions",
    worlds = [server, client, prediction],
    exclusive,
    in_set = crate::sets::INPUT,
    after = apply_frame_regions
)]
pub(crate) fn apply_sessions(
    world: &mut World,
    applier: ResMut<dyn SessionApplier>,
    lifecycle: ResMut<LifecycleSlot>,
    frame: ResMut<TickInputSlot>,
) {
    let sessions = frame.0.as_mut().map(|input| core::mem::take(&mut input.sessions));
    let sessions = sessions.unwrap_or_default();
    if world.kind() == WorldKind::Server {
        lifecycle.0.apply_sessions(world, &mut *applier, sessions);
    } else if !sessions.is_empty() {
        tracing::debug!(
            target: "curd",
            count = sessions.len(),
            "sessions on a world that is not `Server` are dropped"
        );
    }
}

/// Applies the frame's player inputs in ascending `PlayerId` order: `set_input_tick`, then
/// the [`InputApplier`].
#[system(
    name = "sim::apply_inputs",
    worlds = [server, client, prediction],
    exclusive,
    in_set = crate::sets::INPUT,
    after = apply_sessions
)]
pub(crate) fn apply_inputs(
    world: &mut World,
    applier: ResMut<dyn InputApplier>,
    frame: ResMut<TickInputSlot>,
) {
    let inputs = frame.0.as_mut().map(|input| core::mem::take(&mut input.inputs));
    for (player, sample) in inputs.unwrap_or_default() {
        world.set_input_tick(player, sample.input_tick);
        if let Err(error) = applier.apply(world, player, &sample) {
            tracing::warn!(
                target: "curd",
                player = player.0,
                %error,
                "input applier failed: the input is dropped"
            );
        }
    }
}

/// Runs the [`StagedState`] hook.
#[system(
    name = "sim::apply_staged_state",
    worlds = [server, client, prediction],
    exclusive,
    in_set = crate::sets::INPUT,
    after = apply_inputs
)]
pub(crate) fn apply_staged_state(world: &mut World, staged: ResMut<dyn StagedState>) {
    let tick = world.tick();
    if let Err(error) = staged.apply(world, tick) {
        tracing::warn!(target: "curd", %error, "staged state failed to apply");
    }
}

/// Under `Anchored`, decides from the anchors and the policy what is requested, cancelled,
/// unloaded and in which mode each loaded region is; under `Directed`, nothing. Last of
/// this crate's systems, so it sees the sessions' spawns.
#[system(
    name = "sim::apply_region_policy",
    worlds = [server, client, prediction],
    exclusive,
    in_set = crate::sets::INPUT,
    after = apply_staged_state
)]
pub(crate) fn apply_region_policy(
    world: &mut World,
    lifecycle: ResMut<LifecycleSlot>,
    policy: Res<RegionPolicy>,
) {
    lifecycle.0.apply_policy(world, &policy);
}

/// The save copy (architecture section 15.3): forks one child per loaded region over
/// [`World::export_region`] and leaves the [`SavePoint`](crate::SavePoint) for
/// [`Simulation::save_point`](crate::Simulation::save_point).
#[system(name = "sim::save_copy", worlds = [server], exclusive)]
pub(crate) fn save_copy(world: &mut World, job: Job, out: ResMut<SaveSlot>) {
    let world: &World = world;
    let regions: Vec<RegionId> = world.regions().collect();
    // One cell per region, in ascending order: a child writes only its own.
    let mut cells: Vec<Option<Result<lactose_ecs::RegionRows>>> =
        regions.iter().map(|_| None).collect();
    job.context().scope(|scope| {
        for (cell, &region) in cells.iter_mut().zip(&regions) {
            scope.fork(move |_| *cell = Some(world.export_region(region)));
        }
    });
    let exported = regions
        .into_iter()
        .zip(cells)
        .map(|(region, rows)| {
            let rows = rows.expect("every child ran")?;
            Ok(crate::region::RegionExport { region, rows })
        })
        .collect::<Result<Vec<_>>>();
    out.0 = Some(exported.map(|regions| SavePoint {
        tick: world.tick(),
        next_net_id: world.next_net_id(),
        regions,
    }));
}

/// The hook a builder without an [`InputApplier`] inserts: every input is dropped, after
/// its input tick has been set, and logged.
pub(crate) struct NoInputs;

impl InputApplier for NoInputs {
    fn apply(&mut self, _: &mut World, player: PlayerId, _: &PlayerSample) -> Result<()> {
        tracing::debug!(
            target: "curd",
            player = player.0,
            "no input applier: the input is dropped"
        );
        Ok(())
    }
}

/// The hook a builder without a [`SessionApplier`] inserts: every session is dropped and
/// logged.
pub(crate) struct NoSessions;

impl SessionApplier for NoSessions {
    fn apply(&mut self, _: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        tracing::debug!(
            target: "curd",
            player = player.0,
            ?event,
            "no session applier: the session is dropped"
        );
        Ok(())
    }
}

/// The hook a builder without a [`StagedState`] inserts: nothing is staged.
pub(crate) struct NothingStaged;

impl StagedState for NothingStaged {
    fn apply(&mut self, _: &mut World, _: Tick) -> Result<()> {
        Ok(())
    }
}
