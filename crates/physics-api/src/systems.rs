//! The two systems of the physics stage (architecture section 16 and `lactose-sim`'s
//! `Stage::Physics`): the sync, exclusive at the stage's sync point, which tells the
//! backend what rows became; and the step, once per tick beside whatever else runs in the
//! stage, which applies the tick's commands, blocks on the solver and writes back. The
//! module is private: [`register_systems`] places both, and a consumer orders itself
//! against them through [`sets`] alone.

use lactose_ecs::{Channel, Commands, EventWriter, Job, Res, ResMut, World, system};
use lactose_sim::{Registration, Stage, TickDelta};

use crate::command::PhysicsCommands;
use crate::components::{BodyHandle, JointHandle, Transform, Velocity};
use crate::event::{ContactEvent, TriggerEvent};
use crate::service::Physics;

/// The system sets this crate declares, both in `Stage::Physics`.
pub mod sets {
    use lactose_ecs::SystemSet;

    /// The sync: the exclusive system at the physics sync point that creates, removes and
    /// updates the backend's bodies and joints from the rows. Nothing can be ordered
    /// before it in the stage, as for any exclusive system.
    pub static SYNC: SystemSet = SystemSet::new("physics_api::sync");

    /// The step. A system placed in `Stage::Physics` that reads `Physics`, `Transform` or
    /// `Velocity` must be in a set declared `.after(&[&STEP])`, where it sees this tick's
    /// stepped state; one that neither reads nor writes any of them runs alongside the
    /// step on the reserved workers.
    pub static STEP: SystemSet = SystemSet::new("physics_api::step").after(&[&SYNC]);
}

/// Adds this crate's two systems to `Stage::Physics`: what `app` passes to
/// `SimulationBuilder::systems`, beside the [`setup`](crate::setup) closure. Both declare
/// `worlds = [server, prediction]`, so a `Client` registration adds nothing.
pub fn register_systems(registration: &mut Registration) {
    registration
        .stage(Stage::Physics)
        .sets(&[&sets::SYNC, &sets::STEP])
        .add(&SYNC_BODIES)
        .add(&STEP_WORLD);
}

/// The sync: at the physics sync point, after the simulation stage's commands have
/// applied, makes the backend's bodies and joints match the rows. The README's "The tick"
/// states the order and the rules; in outline: region mode changes, then the structural
/// log since the cursor (or every row, when the cursor is stale), then the rows of
/// `RigidBody`, `Collider`, `Transform` and `Velocity` changed since the mark that the
/// last write-back did not write, then the waiting joints; the handles of what was created
/// are written into the rows, and a fresh cursor and mark are taken.
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

/// The step: hands the backend the tick's commands and the delta inside `block_in_place`
/// with the configured reservation, then writes every moved body's `Transform` and
/// `Velocity` back through `Commands::set` (applied at the sync point that begins
/// `Stage::Post`), remembers what it wrote for the next sync, and emits the contact and
/// trigger events in ascending (`target`, `other`, `phase`) order.
#[system(
    name = "physics_api::step",
    worlds = [server, prediction],
    in_set = sets::STEP,
    writes = [Transform, Velocity]
)]
pub(crate) fn step_world(
    job: Job,
    physics: ResMut<Physics>,
    dt: Res<TickDelta>,
    commands: Channel<PhysicsCommands>,
    cmd: Commands,
    contacts: EventWriter<ContactEvent>,
    triggers: EventWriter<TriggerEvent>,
) {
    todo!()
}
