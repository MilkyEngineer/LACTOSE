//! The tick schedule's stages, the pass schedules run between ticks, and how a registration
//! function adds to them without owning the stage list (architecture sections 5.2, 5.3,
//! 6.1, 9.3 and 15.3).

use lactose_ecs::{
    Schedule, ScheduleBuilder, ScheduleError, SystemDescriptor, SystemSet, WorldKind,
};

/// The four stages of the tick schedule, in execution order. Every world kind has all four;
/// a stage with no systems of the world's kind runs its sync point and nothing else, so the
/// sync-point count is the same in every world (architecture section 5.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Stage {
    /// The input sync point. Opens with `sim`'s exclusive systems in [`sets::INPUT`]
    /// ([`systems`](crate::systems)), which apply the frame; a crate that must act at the
    /// sync point after them adds its exclusive system here, ordered after them. Nothing
    /// else belongs in it.
    Input,
    /// Gameplay: the sets of `game-sim`, `game-ai` and `anim`. Physics writes recorded here
    /// through the physics command channel apply in the next stage.
    Simulation,
    /// The physics step (architecture section 16): `physics-api`'s exclusive system that
    /// takes the channel at the stage's sync point, steps, and writes bodies back, declaring
    /// what it writes. Systems that do not depend on physics may be placed here to overlap
    /// the step; everything that reads its results goes in `Post`.
    Physics,
    /// After physics: whatever reads the stepped state in the same tick, such as the sim
    /// skeleton, contact-driven damage and presentation events.
    Post,
}

impl Stage {
    /// Every stage, in execution order.
    pub const ALL: [Self; 4] = [Self::Input, Self::Simulation, Self::Physics, Self::Post];

    /// The stage's name in the schedule, for tracing.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Input => "sim::input",
            Self::Simulation => "sim::simulation",
            Self::Physics => "sim::physics",
            Self::Post => "sim::post",
        }
    }
}

/// The passes [`Simulation::step`](crate::Simulation::step) runs between ticks, after the
/// tick's closing sync point and before the next tick, in this order. Each is a
/// [`Schedule`] run with [`Schedule::run_pass`]: the tick does not advance and nothing is
/// cleared, and its systems are exclusive ones that take [`Job`](lactose_ecs::Job) and fork
/// over the world's columns. A pass with no stages is skipped.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Pass {
    /// Extract (architecture section 5.3): `presentation` copies dirty rows into the staging
    /// ring and writes the presentation snapshot. Milestone 2 fills it.
    Extract,
    /// The replication gather (architecture section 9.3): `net` copies rows whose tick
    /// advanced and the structural log into its per-tick staging buffer, keyed by its
    /// `ChangeMark` and `LogCursor`. Milestone 4 fills it.
    Gather,
}

impl Pass {
    /// Every pass, in the order they run.
    pub const ALL: [Self; 2] = [Self::Extract, Self::Gather];

    /// The pass's name, for tracing.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Extract => "sim::extract",
            Self::Gather => "sim::gather",
        }
    }
}

/// The system sets this crate declares.
pub mod sets {
    use lactose_ecs::SystemSet;

    /// The set of `sim`'s own input sync point systems, placed first in
    /// [`Stage::Input`](super::Stage::Input). A set that must run at the sync point after
    /// the frame has been applied declares `.after(&[&INPUT])`.
    pub static INPUT: SystemSet = SystemSet::new("sim::input");
}

/// What a registration function adds to: the systems and sets of each [`Stage`] of the tick
/// schedule, and the stages of each [`Pass`]. One function serves every world kind, since
/// the builder skips a system whose `worlds` exclude [`kind`](Self::kind); a function that
/// must differ by kind reads it.
///
/// ```
/// use lactose_ecs::SystemSet;
/// use lactose_sim::{Registration, Stage};
///
/// pub static MOVEMENT: SystemSet = SystemSet::new("game_sim::movement");
/// pub static COMBAT: SystemSet = SystemSet::new("game_sim::combat").after(&[&MOVEMENT]);
///
/// /// What `game-sim` adds to the tick schedule.
/// pub fn register_systems(registration: &mut Registration) {
///     registration.stage(Stage::Simulation).sets(&[&MOVEMENT, &COMBAT]);
///     // .add(&APPLY_GRAVITY).add(&SWORD_HIT) ...
/// }
/// ```
pub struct Registration {
    kind: WorldKind,
    stages: [StageBuilder; 4],
    passes: [ScheduleBuilder; 2],
}

impl Registration {
    pub(crate) fn new(kind: WorldKind) -> Self {
        Self {
            kind,
            stages: Default::default(),
            passes: [ScheduleBuilder::new(kind), ScheduleBuilder::new(kind)],
        }
    }

    /// The kind of world being registered for.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        self.kind
    }

    /// The builder of one stage of the tick schedule.
    pub fn stage(&mut self, stage: Stage) -> &mut StageBuilder {
        &mut self.stages[stage as usize]
    }

    /// The builder of one pass: an ordinary [`ScheduleBuilder`] for this kind, whose stages
    /// the caller declares. The replication gather declares its stages and systems here.
    pub fn pass(&mut self, pass: Pass) -> &mut ScheduleBuilder {
        &mut self.passes[pass as usize]
    }

    /// Builds the tick schedule, with `sim`'s own sets and systems placed first in the
    /// `Input` stage, and each pass schedule; a pass with no stages builds to `None`.
    pub(crate) fn build(
        self,
    ) -> core::result::Result<(Schedule, [Option<Schedule>; 2]), ScheduleError> {
        todo!()
    }
}

/// The sets and systems a registration function adds to one stage.
#[derive(Default)]
pub struct StageBuilder {
    sets: Vec<&'static SystemSet>,
    systems: Vec<&'static SystemDescriptor>,
}

impl StageBuilder {
    /// Declares `sets` in this stage, after any declared before. A set belongs to one stage;
    /// declaring it in two is [`ScheduleError::SetInTwoStages`] at build.
    pub fn sets(&mut self, sets: &[&'static SystemSet]) -> &mut Self {
        todo!()
    }

    /// Adds `system` to this stage, or skips it if its worlds exclude the registration's
    /// kind, as [`ScheduleBuilder::add`] does.
    pub fn add(&mut self, system: &'static SystemDescriptor) -> &mut Self {
        todo!()
    }
}
