//! The schedule: stages separated by sync points, each stage a DAG of systems ordered by
//! sets, declared edges and access conflicts (architecture section 5.2).

use core::fmt;
use core::future::Future;

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
    /// exclusive systems run in their order, then its other systems. In a tick the first
    /// stage's sync point is the input sync point, where transient columns are zeroed,
    /// events, channels and input ticks are cleared, and inputs, staged network state and
    /// asset version tables are applied by the exclusive systems placed there.
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
    ///   [`ScheduleError::UnknownSet`], [`ScheduleError::UnknownSystem`]: structure.
    /// - [`ScheduleError::ExclusiveWithEntityAccess`],
    ///   [`ScheduleError::EntityParamInWorldSystem`],
    ///   [`ScheduleError::RegionParamInExclusive`],
    ///   [`ScheduleError::ResourceWriteInKernel`], [`ScheduleError::JobInKernel`],
    ///   [`ScheduleError::AliasedParameter`]: a parameter list that cannot run.
    /// - [`ScheduleError::Cycle`]: among sets or declared edges within a stage, or an edge
    ///   to a later stage.
    /// - [`ScheduleError::UnorderedConflict`]: two systems of one stage touch one target
    ///   and at least one writes it, with no set or edge ordering them. Two exclusive
    ///   systems always conflict.
    /// - [`ScheduleError::EventsBeforeEmitter`]: a reader of an event type not ordered
    ///   after every emitter of it.
    ///
    /// What needs the world's registry is checked at the first [`Schedule::run_tick`] or
    /// [`Schedule::run_pass`], since a schedule is built without a world:
    /// [`ScheduleError::UnregisteredComponent`], [`ScheduleError::ChangedOnUntracked`],
    /// [`ScheduleError::WritesMaintainedComponent`],
    /// [`ScheduleError::TransientBeforeWriter`].
    pub fn build(self) -> core::result::Result<Schedule, ScheduleError> {
        todo!()
    }
}

/// An ordered set of stages for one world kind, built once and run every tick.
///
/// The execution order within a stage is the topological order of its DAG with ties broken
/// by [`SystemDescriptor::key`], never by registration order or layout, so it is identical
/// on every build and platform. A `Schedule` is `Send`, so the timer root task may own it.
pub struct Schedule {
    kind: WorldKind,
}

impl Schedule {
    /// Starts a builder for `kind`.
    #[must_use]
    pub fn builder(kind: WorldKind) -> ScheduleBuilder {
        ScheduleBuilder::new(kind)
    }

    /// The kind this schedule was built for; the world must match at
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
    /// and its systems, with the stage's per-entity kernels as chunk jobs on `executor` and
    /// every reduction merged through its [`Reducer`](lactose_math::Reducer).
    ///
    /// The world is taken by value and handed back when the tick resolves, because the run
    /// shares it with jobs on other workers: a future that is dropped or forgotten mid-tick
    /// then leaks the world rather than leaving a borrow the jobs still write through. The
    /// future is `Send`, so a `'static` task owning the world and the schedule can drive it
    /// from the timer root; when it resolves nothing of the tick is executing.
    ///
    /// Errors: kind `Cancelled` if the executor shuts down first (the world is then torn
    /// and not handed back); kind `NotFound` for a resource a system names that the world
    /// does not hold, or a schedule check that needs the registry (the message names the
    /// [`ScheduleError`]), reported before anything runs with the world handed back
    /// untouched; kind `InvalidData` if the world's kind is not the schedule's.
    #[allow(clippy::manual_async_fn, reason = "the `Send` bound is the contract")]
    pub fn run_tick<'a>(
        &'a mut self,
        world: World,
        executor: &'a Executor,
    ) -> impl Future<Output = Result<World>> + Send + 'a {
        async move { todo!() }
    }

    /// Runs the stages as a pass between ticks: like [`run_tick`](Self::run_tick) but the
    /// tick does not advance and the first sync point is an ordinary one, so nothing is
    /// zeroed or cleared. Extract (architecture section 5.3), the replication gather (9.3)
    /// and the save copy (15.3) are schedules of exclusive systems that take [`Job`](crate::Job)
    /// and fork over the world's columns, run this way.
    #[allow(clippy::manual_async_fn, reason = "the `Send` bound is the contract")]
    pub fn run_pass<'a>(
        &'a mut self,
        world: World,
        executor: &'a Executor,
    ) -> impl Future<Output = Result<World>> + Send + 'a {
        async move { todo!() }
    }
}

/// Why a schedule did not build, or cannot run against a world.
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
    /// A system's `after` edge names a system whose worlds include this kind but which was
    /// not added. An edge to a system whose worlds exclude this kind is ignored.
    UnknownSystem {
        /// The system with the edge.
        system: &'static str,
        /// The system the edge names.
        after: &'static str,
    },
    /// An exclusive system takes a component parameter or [`StableKeyOf`](crate::StableKeyOf).
    ExclusiveWithEntityAccess {
        /// The system.
        system: &'static str,
    },
    /// A system with no component parameter takes [`StableKeyOf`](crate::StableKeyOf).
    EntityParamInWorldSystem {
        /// The system.
        system: &'static str,
    },
    /// An exclusive system takes [`Ctx`](crate::Ctx) or [`RegionRes`](crate::RegionRes).
    RegionParamInExclusive {
        /// The system.
        system: &'static str,
    },
    /// A per-entity or per-region system takes [`ResMut`](crate::ResMut).
    ResourceWriteInKernel {
        /// The system.
        system: &'static str,
        /// The resource's name.
        resource: &'static str,
    },
    /// A per-entity or per-region system takes [`Job`](crate::Job).
    JobInKernel {
        /// The system.
        system: &'static str,
    },
    /// Two parameters of one system name the same component.
    AliasedParameter {
        /// The system.
        system: &'static str,
        /// The component.
        component: ComponentKey,
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
    /// A system names a component the world's registry does not hold.
    UnregisteredComponent {
        /// The system.
        system: &'static str,
        /// The component.
        component: ComponentKey,
    },
    /// [`Changed`](crate::Changed) on a column that is neither replicated nor extracted.
    ChangedOnUntracked {
        /// The system.
        system: &'static str,
        /// The component.
        component: ComponentKey,
    },
    /// [`Mut`](crate::Mut) on a component the world maintains or one with an `EntityRef`
    /// field.
    WritesMaintainedComponent {
        /// The system.
        system: &'static str,
        /// The component.
        component: ComponentKey,
    },
    /// A reader of a transient column is not ordered after a writer of it in the tick.
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
