//! The schedule: stages separated by sync points, each stage a DAG of systems ordered by
//! sets, declared edges and access conflicts (architecture section 5.2).

use core::fmt;

use lactose_core::{ComponentKey, Result, SystemKey};
use lactose_task::Executor;

use crate::access::AccessTarget;
use crate::system::{SystemDescriptor, SystemSet};
use crate::world::{World, WorldKind};

/// Builds a [`Schedule`] for one world kind.
///
/// Stages are declared in order with [`stage`](Self::stage); each later [`add`](Self::add)
/// places a system in the most recently declared stage. A system whose
/// [`worlds`](SystemDescriptor::worlds) exclude the builder's kind is skipped, so one
/// registration function serves every world.
pub struct ScheduleBuilder {
    kind: WorldKind,
    stages: Vec<StageSpec>,
}

struct StageSpec {
    name: &'static str,
    sets: &'static [&'static SystemSet],
    systems: Vec<&'static SystemDescriptor>,
}

impl ScheduleBuilder {
    /// Starts a schedule for `kind`.
    #[must_use]
    pub fn new(kind: WorldKind) -> Self {
        Self { kind, stages: Vec::new() }
    }

    /// The kind this schedule is for.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        self.kind
    }

    /// Declares the next stage, named for tracing, holding `sets`. A sync point precedes
    /// every stage: the command buffers of the previous stage apply there, then the stage's
    /// exclusive systems run, then its other systems. The first stage's sync point is the
    /// input sync point of the tick, where transient columns are zeroed, events and
    /// channels are cleared, and inputs, staged network state and asset version tables are
    /// applied by the exclusive systems placed there.
    pub fn stage(&mut self, name: &'static str, sets: &'static [&'static SystemSet]) -> &mut Self {
        todo!()
    }

    /// Adds `system` to the current stage, or skips it if its worlds exclude this kind.
    /// Adding before any stage is declared is an error at [`build`](Self::build).
    pub fn add(&mut self, system: &'static SystemDescriptor) -> &mut Self {
        todo!()
    }

    /// Resolves the order and checks it. The errors, in the order they are checked:
    ///
    /// - [`ScheduleError::NoStage`], [`ScheduleError::DuplicateKey`],
    ///   [`ScheduleError::UnknownSet`]: structure.
    /// - [`ScheduleError::ExclusiveWithEntityAccess`],
    ///   [`ScheduleError::EntityParamInOnceSystem`]: a parameter list that cannot run.
    /// - [`ScheduleError::Cycle`]: among sets or declared edges within a stage, or an edge
    ///   to a later stage.
    /// - [`ScheduleError::UnorderedConflict`]: two systems of one stage touch one target
    ///   and at least one writes it, with no set or edge ordering them.
    /// - [`ScheduleError::EventsBeforeEmitter`]: a reader of an event type not ordered
    ///   after every emitter of it.
    /// - [`ScheduleError::TransientBeforeWriter`]: a reader of a transient column not
    ///   ordered after a writer of it in the same tick.
    ///
    /// Component targets are checked against the world's registry at the first
    /// [`run_tick`](Schedule::run_tick), since a schedule is built without a world.
    pub fn build(self) -> core::result::Result<Schedule, ScheduleError> {
        todo!()
    }
}

/// An ordered set of stages for one world kind, built once and run every tick.
///
/// The execution order within a stage is the topological order of its DAG with ties broken
/// by [`SystemDescriptor::key`], never by registration order or layout, so it is identical
/// on every build and platform.
pub struct Schedule {
    kind: WorldKind,
}

impl Schedule {
    /// Starts a builder for `kind`.
    #[must_use]
    pub fn builder(kind: WorldKind) -> ScheduleBuilder {
        ScheduleBuilder::new(kind)
    }

    /// The kind this schedule was built for; `world` must match at
    /// [`run_tick`](Self::run_tick).
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        self.kind
    }

    /// How many stages, and so how many sync points, the schedule has.
    #[must_use]
    pub fn stage_count(&self) -> usize {
        todo!()
    }

    /// The systems in execution order, stage by stage.
    #[must_use]
    pub fn order(&self) -> &[&'static SystemDescriptor] {
        todo!()
    }

    /// Runs one tick: advances the world's tick, then for each stage runs its sync point
    /// and its systems, with the stage's per-entity kernels as chunk jobs on `executor`
    /// and every reduction merged through its [`Reducer`](lactose_math::Reducer).
    ///
    /// Resolves to an error of kind `Cancelled` if the executor shuts down first; the world
    /// is then mid-tick and fit only for dropping. Any other error is a component in the
    /// access tables that the world's registry does not hold, reported before anything
    /// runs. The future borrows `world` for the run and is driven by the timer root; when
    /// it resolves nothing of the tick is executing.
    pub async fn run_tick(&mut self, world: &mut World, executor: &Executor) -> Result<()> {
        todo!()
    }
}

/// Why a schedule did not build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// A system was added before any stage was declared.
    NoStage {
        /// The system.
        system: &'static str,
    },
    /// Two systems have the same key.
    DuplicateKey {
        /// The first system's name.
        first: &'static str,
        /// The second system's name.
        second: &'static str,
        /// The shared key.
        key: SystemKey,
    },
    /// A system names a set that is not in its stage.
    UnknownSet {
        /// The system.
        system: &'static str,
        /// The set.
        set: &'static str,
    },
    /// An exclusive system takes a per-entity parameter.
    ExclusiveWithEntityAccess {
        /// The system.
        system: &'static str,
    },
    /// A system with no component parameter takes [`Ctx`](crate::Ctx) or
    /// [`StableKeyOf`](crate::StableKeyOf).
    EntityParamInOnceSystem {
        /// The system.
        system: &'static str,
    },
    /// Sets or edges form a cycle, or an edge points at a later stage.
    Cycle {
        /// The systems and sets on the cycle, in order.
        members: Vec<&'static str>,
    },
    /// Two systems of one stage conflict on a target and nothing orders them.
    UnorderedConflict {
        /// One system.
        first: &'static str,
        /// The other.
        second: &'static str,
        /// What they conflict on.
        target: AccessTarget,
    },
    /// A reader of an event type is not ordered after one of its emitters.
    EventsBeforeEmitter {
        /// The reader.
        reader: &'static str,
        /// The emitter.
        emitter: &'static str,
        /// The event type's name.
        event: &'static str,
    },
    /// A reader of a transient column is not ordered after a writer of it.
    TransientBeforeWriter {
        /// The reader.
        reader: &'static str,
        /// The column.
        component: ComponentKey,
    },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl std::error::Error for ScheduleError {}
