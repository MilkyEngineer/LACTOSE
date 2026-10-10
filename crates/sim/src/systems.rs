//! The input sync point (architecture sections 6, 9.2 and 14): this crate's exclusive
//! systems, placed first in [`Stage::Input`](crate::Stage::Input) under
//! [`sets::INPUT`](crate::sets::INPUT) and ordered among themselves by `after` edges, so
//! the frame is applied in one fixed order in every world:
//!
//! 1. [`begin_tick`]: the [`CurrentTick`] resource takes the world's tick.
//! 2. [`apply_region_lifecycle`]: failures stamped, bundles loaded, tables applied, then the
//!    policy or the frame decides unloads and modes.
//! 3. [`apply_inputs`]: each player's input tick, then its sample through the
//!    [`InputApplier`].
//! 4. [`apply_staged_state`]: the [`StagedState`] hook.
//!
//! A crate that must act at the sync point after the frame orders its own exclusive system
//! after the last of these: `after = lactose_sim::systems::apply_staged_state`, which names
//! the descriptor static `APPLY_STAGED_STATE`. The functions themselves are not callable
//! from outside; the descriptors are the interface.
//!
//! [`save_copy`] is the one system of the save pass, not of the tick.

use lactose_ecs::{Job, ResMut, World, system};

use crate::input::{InputApplier, StagedState};
use crate::region::RegionLifecycle;
use crate::simulation::{SaveSlot, TickInputSlot};
use crate::time::CurrentTick;

/// Writes the tick being run into [`CurrentTick`].
#[system(name = "sim::begin_tick", worlds = [server, client, prediction], exclusive, in_set = crate::sets::INPUT)]
pub(crate) fn begin_tick(world: &mut World, now: ResMut<CurrentTick>) {
    todo!()
}

/// Applies the frame's region entries and runs the lifecycle policy; the README states the
/// order and the rules.
#[system(
    name = "sim::apply_region_lifecycle",
    worlds = [server, client, prediction],
    exclusive,
    in_set = crate::sets::INPUT,
    after = begin_tick
)]
pub(crate) fn apply_region_lifecycle(
    world: &mut World,
    lifecycle: ResMut<RegionLifecycle>,
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
    after = apply_region_lifecycle
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

/// The save copy (architecture section 15.3): forks one child per loaded region over
/// [`World::export_region`] and leaves the [`SavePoint`](crate::SavePoint) for
/// [`Simulation::save_point`](crate::Simulation::save_point).
#[system(name = "sim::save_copy", worlds = [server], exclusive)]
pub(crate) fn save_copy(world: &mut World, job: Job, out: ResMut<SaveSlot>) {
    todo!()
}
