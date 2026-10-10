//! The input sync point (architecture sections 6, 9.2 and 14): this crate's exclusive
//! systems, placed first in `Stage::Input` under `sets::INPUT` and ordered among themselves
//! by `after` edges, so the frame is applied in one fixed order in every world:
//!
//! 1. `begin_tick`: `CurrentTick` takes the world's tick; `TimeScale` takes the frame's.
//! 2. `apply_frame_regions`: failures stamped, bundles loaded, tables applied, then under
//!    `Directed` the frame's unloads and modes.
//! 3. `apply_sessions`: joins whose region is loaded and leaves, through the
//!    `SessionApplier`.
//! 4. `apply_inputs`: each player's input tick, then its sample through the
//!    `InputApplier`.
//! 5. `apply_staged_state`: the `StagedState` hook.
//! 6. `apply_region_policy`: under `Anchored`, requests, cancels, unloads and modes.
//!
//! The module is private: the builder places the descriptors, and a consumer orders itself
//! only through `sets::INPUT`. `save_copy` is the one system of the save pass.

use lactose_ecs::{Job, ResMut, World, system};

use crate::input::{InputApplier, SessionApplier, StagedState};
use crate::simulation::{LifecycleSlot, SaveSlot, TickInputSlot};
use crate::time::{CurrentTick, TimeScale};

/// Writes the tick being run into [`CurrentTick`] and the frame's time scale, if any, into
/// [`TimeScale`].
#[system(name = "sim::begin_tick", worlds = [server, client, prediction], exclusive, in_set = crate::sets::INPUT)]
pub(crate) fn begin_tick(
    world: &mut World,
    now: ResMut<CurrentTick>,
    scale: ResMut<TimeScale>,
    frame: ResMut<TickInputSlot>,
) {
    todo!()
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
) {
    todo!()
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
    todo!()
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
    todo!()
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
    todo!()
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
pub(crate) fn apply_region_policy(world: &mut World, lifecycle: ResMut<LifecycleSlot>) {
    todo!()
}

/// The save copy (architecture section 15.3): forks one child per loaded region over
/// [`World::export_region`] and leaves the [`SavePoint`](crate::SavePoint) for
/// [`Simulation::save_point`](crate::Simulation::save_point).
#[system(name = "sim::save_copy", worlds = [server], exclusive)]
pub(crate) fn save_copy(world: &mut World, job: Job, out: ResMut<SaveSlot>) {
    todo!()
}
