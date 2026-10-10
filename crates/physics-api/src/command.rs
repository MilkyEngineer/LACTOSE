//! The physics command channel (architecture section 6.1): what a kernel records through
//! `cmd.physics()` and the step applies, in command order, before it steps.

use lactose_core::StableKey;
use lactose_ecs::{ChannelWriter, CommandChannel, Commands};
use lactose_math::Vec3;

use crate::components::Transform;

/// The channel: the marker type `World::take_channel` and `World::channel` name. Entries
/// are [`PhysicsCommand`]s.
///
/// Not cleared at the input sync point (`CLEAR_AT_INPUT = false`): the sync takes every
/// entry at the physics sync point, so an entry recorded in `Stage::Input` or
/// `Stage::Simulation` reaches the step of the same tick and one recorded in
/// `Stage::Physics` or `Stage::Post` reaches the next tick's, in command order. A restore
/// drops what the channel holds; [`snapshot`](crate::snapshot) carries the untaken entries.
pub struct PhysicsCommands;

impl CommandChannel for PhysicsCommands {
    type Entry = PhysicsCommand;

    const CLEAR_AT_INPUT: bool = false;

    fn remap(entry: &mut PhysicsCommand, f: &mut dyn FnMut(&mut StableKey)) {
        f(entry.body_mut());
    }
}

/// One physics command. Each names the body it acts on; a placeholder from the recording
/// invocation's spawns is resolved at apply. A command for an entity that is not a body
/// when the step runs is dropped and logged at target `physics`.
///
/// The step applies the tick's commands in command order, after the sync has applied the
/// tick's row writes and before it steps: an impulse adds to a velocity a kernel wrote
/// through `Mut<Velocity>` in the same tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PhysicsCommand {
    /// Adds `impulse` to the body's linear momentum at its centre of mass.
    ApplyImpulse {
        /// The body.
        body: StableKey,
        /// Mass units times units per second, in world space.
        impulse: Vec3,
    },
    /// Adds `impulse` at the world-space `point`, so it also turns the body.
    ApplyImpulseAt {
        /// The body.
        body: StableKey,
        /// Mass units times units per second, in world space.
        impulse: Vec3,
        /// Where it is applied, in world space.
        point: Vec3,
    },
    /// Adds `torque` to the body's angular momentum.
    ApplyTorqueImpulse {
        /// The body.
        body: StableKey,
        /// In world space.
        torque: Vec3,
    },
    /// Applies `force` at the centre of mass for this tick's step only.
    ApplyForce {
        /// The body.
        body: StableKey,
        /// Mass units times units per second squared, in world space.
        force: Vec3,
    },
    /// Applies `torque` for this tick's step only.
    ApplyTorque {
        /// The body.
        body: StableKey,
        /// In world space.
        torque: Vec3,
    },
    /// Places the body at `transform` with no sweep, whatever its type, keeping its
    /// velocity. The one way to move a kinematic body without it pushing through what lies
    /// between; a [`Transform`] row write sweeps a kinematic body instead.
    Teleport {
        /// The body.
        body: StableKey,
        /// Where it goes.
        transform: Transform,
    },
}

impl PhysicsCommand {
    /// The body the command acts on.
    #[must_use]
    pub const fn body(&self) -> StableKey {
        match self {
            Self::ApplyImpulse { body, .. }
            | Self::ApplyImpulseAt { body, .. }
            | Self::ApplyTorqueImpulse { body, .. }
            | Self::ApplyForce { body, .. }
            | Self::ApplyTorque { body, .. }
            | Self::Teleport { body, .. } => *body,
        }
    }

    fn body_mut(&mut self) -> &mut StableKey {
        match self {
            Self::ApplyImpulse { body, .. }
            | Self::ApplyImpulseAt { body, .. }
            | Self::ApplyTorqueImpulse { body, .. }
            | Self::ApplyForce { body, .. }
            | Self::ApplyTorque { body, .. }
            | Self::Teleport { body, .. } => body,
        }
    }
}

/// Records physics commands on behalf of one invocation: what `cmd.physics()` returns.
pub struct PhysicsWriter<'a> {
    writer: ChannelWriter<'a, PhysicsCommands>,
}

impl PhysicsWriter<'_> {
    /// Records `command`.
    pub fn push(&mut self, command: PhysicsCommand) {
        self.writer.push(command);
    }

    /// [`PhysicsCommand::ApplyImpulse`].
    pub fn apply_impulse(&mut self, body: StableKey, impulse: Vec3) {
        self.push(PhysicsCommand::ApplyImpulse { body, impulse });
    }

    /// [`PhysicsCommand::ApplyImpulseAt`].
    pub fn apply_impulse_at(&mut self, body: StableKey, impulse: Vec3, point: Vec3) {
        self.push(PhysicsCommand::ApplyImpulseAt { body, impulse, point });
    }

    /// [`PhysicsCommand::ApplyTorqueImpulse`].
    pub fn apply_torque_impulse(&mut self, body: StableKey, torque: Vec3) {
        self.push(PhysicsCommand::ApplyTorqueImpulse { body, torque });
    }

    /// [`PhysicsCommand::ApplyForce`].
    pub fn apply_force(&mut self, body: StableKey, force: Vec3) {
        self.push(PhysicsCommand::ApplyForce { body, force });
    }

    /// [`PhysicsCommand::ApplyTorque`].
    pub fn apply_torque(&mut self, body: StableKey, torque: Vec3) {
        self.push(PhysicsCommand::ApplyTorque { body, torque });
    }

    /// [`PhysicsCommand::Teleport`].
    pub fn teleport(&mut self, body: StableKey, transform: Transform) {
        self.push(PhysicsCommand::Teleport { body, transform });
    }
}

mod sealed {
    pub trait Sealed {}
}

/// `cmd.physics()` on a [`Commands`] parameter (architecture section 6.1): the writer for
/// the physics channel. Implemented for `Commands` alone.
pub trait PhysicsExt: sealed::Sealed {
    /// The physics command writer of this invocation.
    fn physics(&mut self) -> PhysicsWriter<'_>;
}

impl sealed::Sealed for Commands<'_> {}

impl PhysicsExt for Commands<'_> {
    fn physics(&mut self) -> PhysicsWriter<'_> {
        PhysicsWriter { writer: self.channel::<PhysicsCommands>() }
    }
}
