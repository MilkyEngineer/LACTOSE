//! The tick schedule's stages, the pass schedules run between ticks, and how a registration
//! function adds to them without owning the stage list (architecture sections 5.2, 5.3,
//! 6.1, 9.3 and 15.3).

use lactose_ecs::{
    Schedule, ScheduleBuilder, ScheduleError, SystemDescriptor, SystemSet, WorldKind,
};

use crate::systems::{INPUT_SYSTEMS, SAVE_COPY};

/// The four stages of the tick schedule, in execution order. Every world kind has all four;
/// a stage with no systems of the world's kind runs its sync point and nothing else, so the
/// sync-point count is the same in every world (architecture section 5.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Stage {
    /// The input sync point. Opens with `sim`'s exclusive systems in [`sets::INPUT`], which
    /// apply the frame; a crate that must act at the sync point after them adds its
    /// exclusive system here in a set declared `.after(&[&sets::INPUT])`, and `app` orders
    /// two such crates' sets with [`Registration::order`]. Nothing else belongs in it.
    Input,
    /// Gameplay: the sets of `game-sim`, `game-ai` and `anim`. Physics writes recorded here
    /// through the physics command channel are what the step reads in the next stage.
    Simulation,
    /// The physics stage (architecture section 16): `physics-api`'s exclusive sync at this
    /// stage's sync point, which makes the solver's bodies follow the rows and takes the
    /// physics command channel, and its once-per-tick step, which takes
    /// [`Job`](lactose_ecs::Job), steps the solver through `block_in_place` on a pool `app`
    /// built outside the boundary and handed in through a setup function, and emits the
    /// contact events. The step is not exclusive, so systems that do not depend on physics
    /// may be placed here and run alongside it on the reserved workers; a system here that
    /// reads the physics resource or the events is ordered after `physics_api::sets::STEP`,
    /// where it sees this tick's queries and events but the previous tick's body rows;
    /// everything that reads the stepped rows goes in `Post`.
    Physics,
    /// After physics: `physics-api`'s exclusive write-back runs at this stage's sync point,
    /// before every other system here, writing every moved body's transform and velocity;
    /// then whatever reads the stepped state in the same tick, such as the sim skeleton,
    /// contact-driven damage and presentation events. `app` orders any other exclusive
    /// system placed here against `physics_api::sets::WRITE_BACK` with
    /// [`Registration::order`].
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

/// The passes [`Simulation::run_passes`](crate::Simulation::run_passes) runs between ticks,
/// in this order. Each is a [`Schedule`] of one stage, named by [`name`](Self::name), run
/// with [`Schedule::run_pass`]: the tick does not advance, nothing is zeroed and no event is
/// cleared at its first sync point, and its closing sync point clears every channel, as
/// every run's does; its systems are exclusive ones that take [`Job`](lactose_ecs::Job)
/// and fork over the world's columns. A pass to which no system of the world's kind was
/// added is skipped.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Pass {
    /// Extract (architecture section 5.3): `presentation` copies dirty rows into the staging
    /// ring and writes the presentation snapshot. Milestone 2 fills it.
    Extract,
    /// The replication gather (architecture section 9.3): `net-api`'s `net::gather` system,
    /// placed by `lactose_net_api::register_systems`, copies the rows changed since its
    /// `ChangeMark` and the structural log since its `LogCursor` into `GatherState`'s frame;
    /// one gather covers every tick since the last frame, up to the catch-up cap.
    Gather,
}

impl Pass {
    /// Every pass, in the order they run.
    pub const ALL: [Self; 2] = [Self::Extract, Self::Gather];

    /// The pass's name, which is also its one stage's name, for tracing.
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
    /// [`Stage::Input`](super::Stage::Input): the one thing a consumer names to order
    /// itself at the sync point. A set that must run after the frame has been applied, the
    /// sessions and inputs written and the region policy decided declares
    /// `.after(&[&INPUT])`; nothing can be ordered before it.
    pub static INPUT: SystemSet = SystemSet::new("sim::input");
}

/// What a registration function adds to: the systems and sets of each [`Stage`] of the tick
/// schedule, the one stage of each [`Pass`], and the edges between sets of different crates
/// that `app` adds. One function serves every world kind, since [`StageBuilder::add`] skips
/// a system whose `worlds` exclude [`kind`](Self::kind); a function that must differ by
/// kind reads it.
///
/// ```
/// use lactose_ecs::SystemSet;
/// use lactose_sim::{Pass, Registration, Stage};
///
/// pub static MOVEMENT: SystemSet = SystemSet::new("game_sim::movement");
/// pub static COMBAT: SystemSet = SystemSet::new("game_sim::combat").after(&[&MOVEMENT]);
///
/// /// What `game-sim` adds to the tick schedule.
/// pub fn register_systems(registration: &mut Registration) {
///     registration.stage(Stage::Simulation).sets(&[&MOVEMENT, &COMBAT]);
///     // .add(&APPLY_GRAVITY).add(&SWORD_HIT) ...
///     // registration.pass(Pass::Gather).add(&GATHER);   // what `net` writes
/// }
/// ```
pub struct Registration {
    kind: WorldKind,
    stages: [StageBuilder; 4],
    passes: [StageBuilder; 2],
    orders: Vec<(&'static SystemSet, &'static SystemSet)>,
    /// Each builder [`stage`](Self::stage) or [`pass`](Self::pass) handed out, with how many
    /// systems it held then. Every `add` goes through the borrow last handed out, so the
    /// systems a builder gained before its next entry here were added under this one: the
    /// entries give the order of every `add` across builders.
    handed: Vec<(Builder, usize)>,
}

/// One of a registration's builders.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Builder {
    Stage(Stage),
    Pass(Pass),
}

impl Registration {
    pub(crate) fn new(kind: WorldKind) -> Self {
        Self {
            kind,
            stages: [const { StageBuilder::new() }; 4],
            passes: [const { StageBuilder::new() }; 2],
            orders: Vec::new(),
            handed: Vec::new(),
        }
    }

    /// The kind of world being registered for.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        self.kind
    }

    /// The builder of one stage of the tick schedule.
    pub fn stage(&mut self, stage: Stage) -> &mut StageBuilder {
        self.handed.push((Builder::Stage(stage), self.stages[stage as usize].systems.len()));
        &mut self.stages[stage as usize]
    }

    /// The builder of one pass's single stage. Every crate adds to the same stage, so what
    /// one crate registers never lands in another's stage and registration order decides
    /// nothing: the stage's execution order is the topological order of its sets and edges
    /// with ties by system key, as in every stage.
    pub fn pass(&mut self, pass: Pass) -> &mut StageBuilder {
        self.handed.push((Builder::Pass(pass), self.passes[pass as usize].systems.len()));
        &mut self.passes[pass as usize]
    }

    /// Orders every system of `before` before every system of `after`, through
    /// [`ScheduleBuilder::order`](lactose_ecs::ScheduleBuilder::order) of the schedule that
    /// declares `before`: the tick schedule, whichever stages hold the two sets, or the
    /// pass that holds both. How `app` orders the sets of two crates that do not name each
    /// other and both add an exclusive system to one pass, or to `Stage::Input` after
    /// [`sets::INPUT`]. A set no schedule declares, or two sets in different schedules, is
    /// [`ScheduleError::UnknownOrderedSet`] at build; an edge against the stage order, or
    /// one that puts anything before [`sets::INPUT`], is a [`ScheduleError::Cycle`].
    pub fn order(&mut self, before: &'static SystemSet, after: &'static SystemSet) -> &mut Self {
        self.orders.push((before, after));
        self
    }

    /// Every system the schedules of this kind will hold, in registration order: `sim`'s
    /// input sync point systems, which open the tick schedule, then each system in the order
    /// it was added through [`StageBuilder::add`], whatever its stage or pass, then the save
    /// pass's; a system whose `worlds` exclude the kind is left out. What
    /// [`SimulationBuilder::check`](crate::SimulationBuilder::check) is handed.
    pub(crate) fn added(&self) -> Vec<&'static SystemDescriptor> {
        let builder = |which: Builder| match which {
            Builder::Stage(stage) => &self.stages[stage as usize],
            Builder::Pass(pass) => &self.passes[pass as usize],
        };
        let mut added: Vec<&'static SystemDescriptor> = INPUT_SYSTEMS.to_vec();
        for (index, &(which, start)) in self.handed.iter().enumerate() {
            let systems = &builder(which).systems;
            let end = self.handed[index + 1..]
                .iter()
                .find(|&&(later, _)| later == which)
                .map_or(systems.len(), |&(_, next)| next);
            added.extend_from_slice(&systems[start..end]);
        }
        added.push(&SAVE_COPY);
        added.retain(|system| system.worlds().contains(self.kind));
        added
    }

    /// Builds the tick schedule, with `sim`'s own set and systems placed first in the
    /// `Input` stage, and each pass schedule, each with the orders whose sets it declares;
    /// a pass with no system builds to `None`.
    pub(crate) fn build(
        self,
    ) -> core::result::Result<(Schedule, [Option<Schedule>; 2]), ScheduleError> {
        let kind = self.kind;
        let mut tick = Schedule::builder(kind);
        for stage in Stage::ALL {
            let spec = &self.stages[stage as usize];
            if stage == Stage::Input {
                let mut sets = vec![&sets::INPUT];
                sets.extend(&spec.sets);
                tick.stage(stage.name(), &sets);
                for system in INPUT_SYSTEMS {
                    tick.add(system);
                }
            } else {
                tick.stage(stage.name(), &spec.sets);
            }
            for system in &spec.systems {
                tick.add(system);
            }
        }
        // A pass none of whose systems runs in this kind of world does not exist in it.
        let mut passes: [Option<ScheduleBuilder>; 2] = [None, None];
        for pass in Pass::ALL {
            let spec = &self.passes[pass as usize];
            if spec.systems.iter().any(|system| system.worlds().contains(kind)) {
                let mut builder = Schedule::builder(kind);
                builder.stage(pass.name(), &spec.sets);
                for system in &spec.systems {
                    builder.add(system);
                }
                passes[pass as usize] = Some(builder);
            }
        }

        for &(before, after) in &self.orders {
            // `sim`'s systems open the input sync point: nothing is ordered before them.
            if after.name() == sets::INPUT.name() {
                return Err(ScheduleError::Cycle { members: vec![before.name(), after.name()] });
            }
            let unknown = |set: &'static SystemSet| ScheduleError::UnknownOrderedSet {
                set: set.name(),
                before: before.name(),
                after: after.name(),
            };
            let target = match (self.declaring(before), self.declaring(after)) {
                (Some(first), Some(second)) if first != second => return Err(unknown(after)),
                (Some(target), _) | (None, Some(target)) => target,
                // The tick schedule reports both sets as unknown.
                (None, None) => Declared::Tick,
            };
            match target {
                Declared::Tick => {
                    tick.order(before, after);
                }
                Declared::Pass(pass) => match &mut passes[pass as usize] {
                    Some(builder) => {
                        builder.order(before, after);
                    }
                    // Both sets are this absent pass's, so the edge orders nothing here.
                    None if self.declaring(before) == self.declaring(after) => {}
                    None => {
                        let missing = if self.declaring(before).is_none() { before } else { after };
                        return Err(unknown(missing));
                    }
                },
            }
        }

        let tick = tick.build()?;
        let [extract, gather] = passes;
        Ok((
            tick,
            [
                extract.map(ScheduleBuilder::build).transpose()?,
                gather.map(ScheduleBuilder::build).transpose()?,
            ],
        ))
    }

    /// The schedule that declares `set`, if any: the tick schedule (whichever stage), or
    /// one pass.
    fn declaring(&self, set: &'static SystemSet) -> Option<Declared> {
        let declares = |spec: &StageBuilder| spec.sets.iter().any(|s| s.name() == set.name());
        if set.name() == sets::INPUT.name() || self.stages.iter().any(declares) {
            return Some(Declared::Tick);
        }
        Pass::ALL
            .into_iter()
            .find(|&pass| declares(&self.passes[pass as usize]))
            .map(Declared::Pass)
    }
}

/// Which schedule declares a set.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Declared {
    Tick,
    Pass(Pass),
}

/// The sets and systems a registration function adds to one stage.
///
/// Only a [`Registration`] makes one, so a registration function adds through the builders
/// it is handed and cannot replace or move one, which the order of the systems a
/// [`SimulationBuilder::check`](crate::SimulationBuilder::check) is handed relies on:
///
/// ```compile_fail
/// fn register(registration: &mut lactose_sim::Registration) {
///     *registration.stage(lactose_sim::Stage::Simulation) = lactose_sim::StageBuilder::default();
/// }
/// ```
///
/// ```compile_fail
/// fn register(registration: &mut lactose_sim::Registration) {
///     let moved = std::mem::take(registration.stage(lactose_sim::Stage::Simulation));
///     *registration.pass(lactose_sim::Pass::Extract) = moved;
/// }
/// ```
///
/// ```compile_fail
/// fn register(registration: &mut lactose_sim::Registration) {
///     let mut moved = lactose_sim::StageBuilder::new();
///     std::mem::swap(registration.stage(lactose_sim::Stage::Simulation), &mut moved);
///     std::mem::swap(registration.pass(lactose_sim::Pass::Extract), &mut moved);
/// }
/// ```
///
/// ```compile_fail
/// fn register(registration: &mut lactose_sim::Registration) {
///     std::mem::swap(
///         registration.stage(lactose_sim::Stage::Simulation),
///         registration.pass(lactose_sim::Pass::Extract),
///     );
/// }
/// ```
pub struct StageBuilder {
    sets: Vec<&'static SystemSet>,
    systems: Vec<&'static SystemDescriptor>,
}

impl StageBuilder {
    const fn new() -> Self {
        Self { sets: Vec::new(), systems: Vec::new() }
    }

    /// Declares `sets` in this stage, after any declared before. A set belongs to one stage;
    /// declaring it in two is [`ScheduleError::SetInTwoStages`] at build.
    pub fn sets(&mut self, sets: &[&'static SystemSet]) -> &mut Self {
        self.sets.extend_from_slice(sets);
        self
    }

    /// Adds `system` to this stage, or skips it if its worlds exclude the registration's
    /// kind, as [`ScheduleBuilder::add`](lactose_ecs::ScheduleBuilder::add) does.
    pub fn add(&mut self, system: &'static SystemDescriptor) -> &mut Self {
        // The schedule builder skips it if its worlds exclude the kind.
        self.systems.push(system);
        self
    }
}
