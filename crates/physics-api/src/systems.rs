//! The three systems of this crate (architecture section 16 and `lactose-sim`'s
//! `Stage::Physics` and `Stage::Post`): the sync, exclusive at the physics stage's sync
//! point, which tells the backend what rows became and takes the commands; the step, once
//! per tick beside whatever else runs in the stage, which blocks on the solver and emits
//! the events; and the write-back, exclusive at the sync point that begins `Stage::Post`,
//! which writes what moved into the rows. The module is private: [`register_systems`]
//! places all three, and a consumer orders itself against them through [`sets`] alone.

use lactose_ecs::{EventWriter, Job, Res, ResMut, World, system};
use lactose_sim::{Registration, Stage, TickDelta};

use crate::components::{BodyHandle, JointHandle, Teleported, Transform, Velocity};
use crate::event::{ContactEvent, TriggerEvent};
use crate::service::Physics;

/// The system sets this crate declares: two in `Stage::Physics`, one in `Stage::Post`.
pub mod sets {
    use lactose_ecs::SystemSet;

    /// The sync: the exclusive system at the physics sync point that creates, removes and
    /// updates the backend's bodies and joints from the rows and takes the tick's
    /// commands. An exclusive system runs at the stage's sync point before every other
    /// system of the stage, so nothing orders itself against it.
    pub static SYNC: SystemSet = SystemSet::new("physics_api::sync");

    /// The step. A `Stage::Physics` system that reads `Physics` or the contact and trigger
    /// events must be in a set declared `.after(&[&STEP])`, or the schedule fails to build
    /// on the conflict with the step's `ResMut<Physics>`: there `Res<Physics>` and the
    /// events are this tick's, but the `Transform` and `Velocity` rows are the previous
    /// tick's, since the write-back lands at the sync point that begins `Stage::Post`;
    /// nothing but the write-back writes a body's `Transform` or `Velocity` in
    /// `Stage::Physics`, since the write-back overwrites it. Readers of the stepped rows
    /// go in `Stage::Post`. A system that touches neither `Physics` nor the events runs
    /// alongside the step on the reserved workers.
    pub static STEP: SystemSet = SystemSet::new("physics_api::step").after(&[&SYNC]);

    /// The write-back: the exclusive system placed first in `Stage::Post`, at its sync
    /// point, which writes every moved body's `Transform` and `Velocity` and the tick into
    /// `Teleported` for a body placed without a sweep. Every other system of `Stage::Post`
    /// runs after it without an edge, as after any exclusive system; `app` orders any other
    /// exclusive `Stage::Post` system against it with `Registration::order`.
    pub static WRITE_BACK: SystemSet = SystemSet::new("physics_api::write_back");
}

/// Adds this crate's three systems: the sync and the step to `Stage::Physics`, the
/// write-back to `Stage::Post`. What `app` passes to `SimulationBuilder::systems`, beside
/// the [`setup`](crate::setup) closure. All three declare `worlds = [server, prediction]`,
/// so a `Client` registration adds nothing.
pub fn register_systems(registration: &mut Registration) {
    registration
        .stage(Stage::Physics)
        .sets(&[&sets::SYNC, &sets::STEP])
        .add(&SYNC_BODIES)
        .add(&STEP_WORLD);
    registration.stage(Stage::Post).sets(&[&sets::WRITE_BACK]).add(&WRITE_BACK_ROWS);
}

/// The sync: at the physics sync point, after the simulation stage's commands have
/// applied, makes the backend's bodies and joints match the rows and takes the commands.
/// The README's "The tick" states the order and the rules; in outline: region mode
/// changes; the pending log entries a restore carried over, then the structural log since
/// the cursor (or, when the cursor or mark is stale, a diff of every row against the
/// carried bookkeeping); the union of the pending keys and the rows of `RigidBody`,
/// `Collider`, `Transform`, `Velocity` and `Joint` changed since the mark, in ascending key
/// order, skipping a `Transform` or `Velocity` equal to what the last write-back wrote;
/// the waiting joints; then `World::take_channel::<PhysicsCommands>()` appended after the
/// pending commands. The handles of what was created are written into the rows, and a
/// fresh cursor and mark are taken.
#[system(
    name = "physics_api::sync",
    worlds = [server, prediction],
    exclusive,
    in_set = sets::SYNC,
    writes = [BodyHandle, JointHandle]
)]
pub(crate) fn sync_bodies(world: &mut World, physics: ResMut<Physics>) {
    todo!()
}

/// The step: hands the backend the commands the sync took and the delta inside
/// `block_in_place` with the configured reservation, keeps what the backend reported for
/// the write-back, and emits the contact events and then the trigger events, each list in
/// ascending (`target`, `other`, `phase`) order.
#[system(name = "physics_api::step", worlds = [server, prediction], in_set = sets::STEP)]
pub(crate) fn step_world(
    job: Job,
    physics: ResMut<Physics>,
    dt: Res<TickDelta>,
    contacts: EventWriter<ContactEvent>,
    triggers: EventWriter<TriggerEvent>,
) {
    todo!()
}

/// The write-back: at the sync point that begins `Stage::Post`, writes the `Transform`
/// and `Velocity` the step reported for every moved body, in ascending key order, through
/// `World::get_mut` and `Mut::set_if_neq`, so only a row whose bytes changed is stamped;
/// adds a missing `Velocity` through `World::insert`; writes the tick into `Teleported`
/// for a body the step placed without a sweep, adding the component where the row lacks
/// it; skips a reported key whose row a command despawned at this sync point. What it
/// wrote replaces the record the next sync compares row writes against.
#[system(
    name = "physics_api::write_back",
    worlds = [server, prediction],
    exclusive,
    in_set = sets::WRITE_BACK,
    writes = [Transform, Velocity, Teleported]
)]
pub(crate) fn write_back_rows(world: &mut World, physics: ResMut<Physics>) {
    todo!()
}
