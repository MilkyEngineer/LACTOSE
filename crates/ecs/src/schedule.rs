//! The schedule: stages separated by sync points, each stage a DAG of systems ordered by
//! sets, declared edges and access conflicts (architecture section 5.2).

use core::fmt;
use core::future::Future;
use core::ops::Range;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};

use lactose_core::{ComponentKey, Error, ErrorKind, Result, StableKey, SystemKey};
use lactose_reflect::Persistence;
use lactose_task::Executor;

use crate::access::{Access, AccessKind, AccessTarget};
use crate::change::Stamp;
use crate::region::RegionId;
use crate::run::{RunState, run};
use crate::system::{SystemDescriptor, SystemSet};
use crate::world::{World, WorldKind, WorldSet, is_tracked, refuses_mut};

/// Who recorded a command, an event or a presentation event, in the order they apply:
/// the once-per-tick invocation, per-region invocations by region, then entities by origin
/// key (architecture section 5.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Source {
    Tick,
    Region(RegionId),
    Entity(StableKey),
}

/// How a system runs, from its parameters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SystemKind {
    Exclusive,
    PerEntity,
    PerRegion,
    Once,
}

impl SystemKind {
    pub(crate) fn of(system: &SystemDescriptor) -> Self {
        let access = system.access();
        if system.is_exclusive() {
            Self::Exclusive
        } else if access.iter().any(|a| {
            // A `writes` entry declares a column written through commands; it does not make
            // the system run per entity.
            a.kind() != AccessKind::Writes && matches!(a.target(), AccessTarget::Component(_))
        }) {
            Self::PerEntity
        } else if access.iter().any(|a| {
            a.kind() == AccessKind::Region || matches!(a.target(), AccessTarget::RegionResource(_))
        }) {
            Self::PerRegion
        } else {
            Self::Once
        }
    }
}

/// Builds a [`Schedule`] for one world kind.
///
/// Stages are declared in order with [`stage`](Self::stage); each later [`add`](Self::add)
/// places a system in the most recently declared stage. A system whose
/// [`worlds`](SystemDescriptor::worlds) exclude the builder's kind is skipped, so one
/// registration function serves every world.
pub struct ScheduleBuilder {
    kind: WorldKind,
    stages: Vec<StageSpec>,
    /// The first system added before any stage.
    orphan: Option<&'static str>,
    /// Edges between sets added by [`order`](Self::order), as (before, after).
    orders: Vec<(&'static SystemSet, &'static SystemSet)>,
}

struct StageSpec {
    name: &'static str,
    sets: Vec<&'static SystemSet>,
    systems: Vec<&'static SystemDescriptor>,
}

impl ScheduleBuilder {
    /// Starts a schedule for `kind`.
    #[must_use]
    pub fn new(kind: WorldKind) -> Self {
        Self { kind, stages: Vec::new(), orphan: None, orders: Vec::new() }
    }

    /// The kind this schedule is for.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        self.kind
    }

    /// Declares the next stage, named for tracing, holding `sets`. A sync point precedes
    /// every stage: the command buffers of the previous stage apply there, then the stage's
    /// exclusive systems run in their order, then its other systems; a closing sync point
    /// after the last stage applies its buffers and runs no systems, so a run has one more
    /// sync point than stages and ends with every buffer empty. In a tick the first stage's
    /// sync point is the input sync point, where transient columns are zeroed, events,
    /// channels and input ticks are cleared, and inputs, staged network state and asset
    /// version tables are applied by the exclusive systems placed there.
    pub fn stage(&mut self, name: &'static str, sets: &[&'static SystemSet]) -> &mut Self {
        self.stages.push(StageSpec { name, sets: sets.to_vec(), systems: Vec::new() });
        self
    }

    /// Adds `system` to the current stage, or skips it if its worlds exclude this kind.
    /// Adding before any stage is declared is an error at [`build`](Self::build).
    pub fn add(&mut self, system: &'static SystemDescriptor) -> &mut Self {
        if system.worlds().contains(self.kind) {
            match self.stages.last_mut() {
                Some(stage) => stage.systems.push(system),
                None => {
                    self.orphan.get_or_insert(system.name());
                }
            }
        }
        self
    }

    /// Orders every system of `before` before every system of `after`, like
    /// [`SystemSet::after`] but added by whoever assembles the schedule rather than declared
    /// on the set, so that the sets of two crates that do not name each other can be ordered
    /// (two exclusive systems in one stage must be). Both sets must be declared in some
    /// stage of this schedule, or the build fails with
    /// [`ScheduleError::UnknownOrderedSet`]; an edge between sets of two stages is accepted
    /// when it agrees with the stage order and is a [`ScheduleError::Cycle`] otherwise.
    pub fn order(&mut self, before: &'static SystemSet, after: &'static SystemSet) -> &mut Self {
        self.orders.push((before, after));
        self
    }

    /// Resolves the order and checks it. The errors, in the order they are checked:
    ///
    /// - [`ScheduleError::NoStage`], [`ScheduleError::SetInTwoStages`],
    ///   [`ScheduleError::UnknownOrderedSet`], [`ScheduleError::DuplicateKey`],
    ///   [`ScheduleError::UnknownSet`], [`ScheduleError::UnknownSystem`]: structure.
    /// - [`ScheduleError::ExclusiveWithEntityAccess`],
    ///   [`ScheduleError::EntityParamInWorldSystem`],
    ///   [`ScheduleError::RegionParamInWorldSystem`],
    ///   [`ScheduleError::ResourceWriteInKernel`], [`ScheduleError::JobInKernel`],
    ///   [`ScheduleError::AliasedParameter`], [`ScheduleError::InvalidParameter`]: a
    ///   parameter list that cannot run. The per-kind rules apply to the kernel's table and
    ///   the finalize's table separately.
    /// - [`ScheduleError::Cycle`]: among sets or declared edges within a stage, or an edge
    ///   to a later stage.
    /// - [`ScheduleError::UnorderedConflict`]: two systems of one stage touch one target
    ///   and at least one writes it, with no set or edge ordering them. Two exclusive
    ///   systems always conflict.
    /// - [`ScheduleError::EventsBeforeEmitter`]: a reader of an event type not ordered
    ///   after every emitter of it.
    ///
    /// What needs a world is checked by [`Schedule::prepare`], since a schedule is built
    /// without one: [`ScheduleError::WrongWorld`],
    /// [`ScheduleError::UnregisteredComponent`], [`ScheduleError::ChangedOnUntracked`],
    /// [`ScheduleError::WritesMaintainedComponent`],
    /// [`ScheduleError::TransientBeforeWriter`], [`ScheduleError::MissingResource`].
    pub fn build(self) -> core::result::Result<Schedule, ScheduleError> {
        if let Some(system) = self.orphan {
            return Err(ScheduleError::NoStage { system });
        }
        let mut declared: BTreeMap<&'static str, (usize, &'static str)> = BTreeMap::new();
        for (stage, spec) in self.stages.iter().enumerate() {
            for set in &spec.sets {
                if let Some(&(first_stage, first)) = declared.get(set.name())
                    && first_stage != stage
                {
                    return Err(ScheduleError::SetInTwoStages {
                        set: set.name(),
                        first,
                        second: spec.name,
                    });
                }
                declared.insert(set.name(), (stage, spec.name));
            }
        }
        // Edges between sets: one list per stage for the edges within it; an edge across
        // stages that agrees with the stage order needs nothing, and one that disagrees
        // is a cycle, like a system edge to a later stage.
        let mut set_orders: Vec<Vec<(&'static str, &'static str)>> =
            vec![Vec::new(); self.stages.len()];
        for &(before, after) in &self.orders {
            let stage_of_set = |set: &'static SystemSet| {
                declared.get(set.name()).map(|&(stage, _)| stage).ok_or_else(|| {
                    ScheduleError::UnknownOrderedSet {
                        set: set.name(),
                        before: before.name(),
                        after: after.name(),
                    }
                })
            };
            let (first, second) = (stage_of_set(before)?, stage_of_set(after)?);
            if first > second {
                return Err(ScheduleError::Cycle { members: vec![before.name(), after.name()] });
            }
            if first == second {
                set_orders[first].push((before.name(), after.name()));
            }
        }
        let mut seen: BTreeMap<SystemKey, &'static str> = BTreeMap::new();
        let mut stage_of: BTreeMap<SystemKey, usize> = BTreeMap::new();
        for (stage, spec) in self.stages.iter().enumerate() {
            for system in &spec.systems {
                if let Some(first) = seen.insert(system.key(), system.name()) {
                    return Err(ScheduleError::DuplicateKey {
                        first,
                        second: system.name(),
                        key: system.key(),
                    });
                }
                stage_of.insert(system.key(), stage);
            }
        }
        for spec in &self.stages {
            for system in &spec.systems {
                if let Some(set) = system.set()
                    && !spec.sets.iter().any(|s| s.name() == set.name())
                {
                    return Err(ScheduleError::UnknownSet {
                        system: system.name(),
                        set: set.name(),
                    });
                }
            }
        }
        for spec in &self.stages {
            for system in &spec.systems {
                for after in system.ordered_after() {
                    if !stage_of.contains_key(&after.key()) && after.worlds().contains(self.kind) {
                        return Err(ScheduleError::UnknownSystem {
                            system: system.name(),
                            after: after.name(),
                        });
                    }
                }
            }
        }
        for spec in &self.stages {
            let mut systems = spec.systems.clone();
            systems.sort_by_key(|system| system.key());
            for system in systems {
                check_parameters(system)?;
            }
        }

        let set_stage: BTreeMap<&'static str, usize> = self
            .stages
            .iter()
            .enumerate()
            .flat_map(|(stage, spec)| spec.sets.iter().map(move |set| (set.name(), stage)))
            .collect();
        let mut order = Vec::new();
        let mut stages = Vec::new();
        let mut ancestors: Vec<BTreeSet<usize>> = Vec::new();
        for (stage, spec) in self.stages.iter().enumerate() {
            let staged = order_stage(stage, spec, &stage_of, &set_stage, &set_orders[stage])?;
            let start = order.len();
            for set in staged.ancestors {
                ancestors.push(set.into_iter().map(|local| start + local).collect());
            }
            order.extend(staged.order);
            stages.push(StagePlan { name: spec.name, systems: start..order.len() });
        }
        let kinds: Vec<SystemKind> = order.iter().map(|system| SystemKind::of(system)).collect();
        let stage_index: Vec<usize> = stages
            .iter()
            .enumerate()
            .flat_map(|(stage, plan)| plan.systems.clone().map(move |_| stage))
            .collect();
        let plan = Plan { kind: self.kind, stages, order, kinds, stage_of: stage_index, ancestors };
        plan.check_conflicts()?;
        plan.check_events()?;
        let slots = plan.order.len() + plan.stages.len() + 1;
        assert!(
            slots < (crate::world::BETWEEN / crate::world::SLOT_WIDTH) as usize - 1,
            "a schedule has fewer than 32 766 systems and stages"
        );
        let systems = plan.order.len();
        Ok(Schedule {
            kind: self.kind,
            plan: Arc::new(plan),
            pass_marks: vec![None; systems],
            world: None,
        })
    }
}

fn is_data(kind: AccessKind) -> bool {
    matches!(
        kind,
        AccessKind::Read | AccessKind::Write | AccessKind::OptionalRead | AccessKind::OptionalWrite
    )
}

fn is_write(kind: AccessKind) -> bool {
    matches!(kind, AccessKind::Write | AccessKind::OptionalWrite)
}

/// Whether `kind` writes a column: a `Mut` parameter or a `writes` declaration.
fn writes_column(kind: AccessKind) -> bool {
    is_write(kind) || kind == AccessKind::Writes
}

/// The per-kind rules of one table: the kernel's (`finalize` false) or the finalize's.
fn check_table(
    system: &'static SystemDescriptor,
    kind: SystemKind,
    table: &[Access],
    finalize: bool,
) -> core::result::Result<(), ScheduleError> {
    let name = system.name();
    let targets: Vec<(AccessKind, AccessTarget)> =
        table.iter().map(|access| (access.kind(), access.target())).collect();
    // A component parameter or filter; a `writes` entry is neither.
    let component = |(kind, target): &(AccessKind, AccessTarget)| {
        *kind != AccessKind::Writes && matches!(target, AccessTarget::Component(_))
    };
    let region = |(kind, target): &(AccessKind, AccessTarget)| {
        *kind == AccessKind::Region || matches!(target, AccessTarget::RegionResource(_))
    };
    let has = |wanted: AccessKind| targets.iter().any(|(kind, _)| *kind == wanted);
    let count = |wanted: AccessKind| targets.iter().filter(|(kind, _)| *kind == wanted).count();

    if finalize {
        if has(AccessKind::Entity) {
            return Err(ScheduleError::EntityParamInWorldSystem { system: name });
        }
        if targets.iter().any(region) {
            return Err(ScheduleError::RegionParamInWorldSystem { system: name });
        }
        if targets.iter().any(component) {
            return Err(ScheduleError::InvalidParameter { system: name, parameter: "component" });
        }
        if has(AccessKind::World) {
            return Err(ScheduleError::InvalidParameter { system: name, parameter: "&mut World" });
        }
    } else {
        match kind {
            SystemKind::Exclusive => {
                if targets.iter().any(component) || has(AccessKind::Entity) {
                    return Err(ScheduleError::ExclusiveWithEntityAccess { system: name });
                }
                if targets.iter().any(region) {
                    return Err(ScheduleError::RegionParamInWorldSystem { system: name });
                }
            }
            SystemKind::PerEntity | SystemKind::PerRegion | SystemKind::Once => {
                if has(AccessKind::World) {
                    return Err(ScheduleError::InvalidParameter {
                        system: name,
                        parameter: "&mut World",
                    });
                }
                if kind != SystemKind::PerEntity && has(AccessKind::Entity) {
                    return Err(ScheduleError::EntityParamInWorldSystem { system: name });
                }
                if kind != SystemKind::Once {
                    for (access, target) in &targets {
                        if let (AccessKind::Write, AccessTarget::Resource { name: resource, .. }) =
                            (access, target)
                        {
                            return Err(ScheduleError::ResourceWriteInKernel {
                                system: name,
                                resource,
                            });
                        }
                    }
                    if has(AccessKind::Job) {
                        return Err(ScheduleError::JobInKernel { system: name });
                    }
                }
            }
        }
    }

    for (index, (access, target)) in targets.iter().enumerate() {
        let AccessTarget::Component(key) = *target else { continue };
        for (other_access, other_target) in &targets[index + 1..] {
            if *other_target == AccessTarget::Component(key)
                && ((is_data(*access) && is_data(*other_access))
                    || *access == AccessKind::Without
                    || *other_access == AccessKind::Without)
            {
                return Err(ScheduleError::AliasedParameter { system: name, component: key });
            }
        }
    }
    for (wanted, parameter) in [
        (AccessKind::Commands, "Commands"),
        (AccessKind::Job, "Job"),
        (AccessKind::World, "&mut World"),
    ] {
        if count(wanted) > 1 {
            return Err(ScheduleError::InvalidParameter { system: name, parameter });
        }
    }
    for (index, (access, target)) in targets.iter().enumerate() {
        let AccessTarget::Resource { type_id, name: resource } = *target else { continue };
        let others = targets
            .iter()
            .enumerate()
            .filter(|&(other, (_, t))| {
                other != index
                    && matches!(t, AccessTarget::Resource { type_id: id, .. } if *id == type_id)
            })
            .count();
        if is_write(*access) && others > 0 {
            return Err(ScheduleError::InvalidParameter { system: name, parameter: resource });
        }
    }
    Ok(())
}

fn check_parameters(system: &'static SystemDescriptor) -> core::result::Result<(), ScheduleError> {
    let kind = SystemKind::of(system);
    check_table(system, kind, system.access(), false)?;
    if system.reduce().is_some() || !system.finalize_access().is_empty() {
        check_table(system, kind, system.finalize_access(), true)?;
    }
    Ok(())
}

/// One stage in execution order, with each system's ancestors in the stage's DAG.
struct StagedSystems {
    order: Vec<&'static SystemDescriptor>,
    /// Indices into `order`.
    ancestors: Vec<BTreeSet<usize>>,
}

fn order_stage(
    stage: usize,
    spec: &StageSpec,
    stage_of: &BTreeMap<SystemKey, usize>,
    set_stage: &BTreeMap<&'static str, usize>,
    set_orders: &[(&'static str, &'static str)],
) -> core::result::Result<StagedSystems, ScheduleError> {
    let systems = &spec.systems;
    let index_of: BTreeMap<SystemKey, usize> =
        systems.iter().enumerate().map(|(index, system)| (system.key(), index)).collect();
    let mut preds: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); systems.len()];

    // Declared edges.
    for (index, system) in systems.iter().enumerate() {
        for after in system.ordered_after() {
            match stage_of.get(&after.key()) {
                Some(&other) if other == stage => {
                    preds[index].insert(index_of[&after.key()]);
                }
                Some(&other) if other > stage => {
                    return Err(ScheduleError::Cycle {
                        members: vec![system.name(), after.name()],
                    });
                }
                _ => {}
            }
        }
    }

    // Sets: their order within the stage, transitively.
    let sets: Vec<&'static SystemSet> = spec.sets.clone();
    let set_index: BTreeMap<&'static str, usize> =
        sets.iter().enumerate().map(|(index, set)| (set.name(), index)).collect();
    let mut set_preds: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); sets.len()];
    for (index, set) in sets.iter().enumerate() {
        for after in set.ordered_after() {
            match set_stage.get(after.name()) {
                Some(&other) if other == stage => {
                    set_preds[index].insert(set_index[after.name()]);
                }
                Some(&other) if other > stage => {
                    return Err(ScheduleError::Cycle { members: vec![set.name(), after.name()] });
                }
                _ => {}
            }
        }
    }
    // Edges the assembler added between sets of this stage: both sets are declared here,
    // checked by `build`.
    for &(before, after) in set_orders {
        set_preds[set_index[after]].insert(set_index[before]);
    }
    let set_order = topological(&set_preds, |index| sets[index].name()).map_err(|cycle| {
        ScheduleError::Cycle {
            members: cycle.into_iter().map(|index| sets[index].name()).collect(),
        }
    })?;
    let mut set_ancestors: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); sets.len()];
    for &set in &set_order {
        let mut all = BTreeSet::new();
        for &pred in &set_preds[set] {
            all.insert(pred);
            all.extend(set_ancestors[pred].iter().copied());
        }
        set_ancestors[set] = all;
    }
    let set_of: Vec<Option<usize>> = systems
        .iter()
        .map(|system| system.set().and_then(|set| set_index.get(set.name()).copied()))
        .collect();
    for (index, set) in set_of.iter().enumerate() {
        let Some(set) = set else { continue };
        for (other, other_set) in set_of.iter().enumerate() {
            if let Some(other_set) = other_set
                && set_ancestors[*set].contains(other_set)
            {
                preds[index].insert(other);
            }
        }
    }

    // Exclusive systems run first at the stage's sync point, so none can follow another
    // system of the stage.
    for (index, system) in systems.iter().enumerate() {
        if system.is_exclusive()
            && let Some(&pred) = preds[index].iter().find(|&&pred| !systems[pred].is_exclusive())
        {
            return Err(ScheduleError::Cycle {
                members: vec![system.name(), systems[pred].name()],
            });
        }
    }

    let order = topological(&preds, |index| {
        // Exclusive systems first, then by key: the stage's execution order.
        (!systems[index].is_exclusive(), systems[index].key())
    })
    .map_err(|cycle| ScheduleError::Cycle {
        members: cycle.into_iter().map(|index| systems[index].name()).collect(),
    })?;
    let position: BTreeMap<usize, usize> =
        order.iter().enumerate().map(|(position, &index)| (index, position)).collect();
    let mut ancestors: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); systems.len()];
    for &index in &order {
        let mut all = BTreeSet::new();
        for &pred in &preds[index] {
            all.insert(pred);
            all.extend(ancestors[pred].iter().copied());
        }
        ancestors[index] = all;
    }
    Ok(StagedSystems {
        order: order.iter().map(|&index| systems[index]).collect(),
        ancestors: order
            .iter()
            .map(|&index| ancestors[index].iter().map(|pred| position[pred]).collect())
            .collect(),
    })
}

/// Kahn's algorithm, taking the ready node with the least `priority` first. On a cycle,
/// returns the nodes of one cycle in order.
fn topological<P: Ord>(
    preds: &[BTreeSet<usize>],
    priority: impl Fn(usize) -> P,
) -> core::result::Result<Vec<usize>, Vec<usize>> {
    let mut remaining: Vec<usize> = preds.iter().map(BTreeSet::len).collect();
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); preds.len()];
    for (node, node_preds) in preds.iter().enumerate() {
        for &pred in node_preds {
            succs[pred].push(node);
        }
    }
    let mut ready: BTreeSet<(P, usize)> = (0..preds.len())
        .filter(|&node| remaining[node] == 0)
        .map(|node| (priority(node), node))
        .collect();
    let mut order = Vec::with_capacity(preds.len());
    while let Some((_, node)) = ready.pop_first() {
        order.push(node);
        for &succ in &succs[node] {
            remaining[succ] -= 1;
            if remaining[succ] == 0 {
                ready.insert((priority(succ), succ));
            }
        }
    }
    if order.len() == preds.len() {
        return Ok(order);
    }
    // Walk back through unfinished predecessors until a node repeats.
    let mut node = (0..preds.len()).find(|&node| remaining[node] > 0).expect("a node is left");
    let mut path = Vec::new();
    let mut seen = BTreeMap::new();
    while !seen.contains_key(&node) {
        seen.insert(node, path.len());
        path.push(node);
        node = *preds[node].iter().find(|&&pred| remaining[pred] > 0).expect("a cycle");
    }
    let mut cycle = path.split_off(seen[&node]);
    cycle.reverse();
    Err(cycle)
}

/// One stage of a built schedule.
pub(crate) struct StagePlan {
    pub(crate) name: &'static str,
    /// Indices into the execution order: exclusive systems first.
    pub(crate) systems: Range<usize>,
}

/// What a built schedule runs.
pub(crate) struct Plan {
    pub(crate) kind: WorldKind,
    pub(crate) stages: Vec<StagePlan>,
    pub(crate) order: Vec<&'static SystemDescriptor>,
    pub(crate) kinds: Vec<SystemKind>,
    pub(crate) stage_of: Vec<usize>,
    /// Each system's ancestors in its stage's DAG, as indices into `order`.
    pub(crate) ancestors: Vec<BTreeSet<usize>>,
}

impl Plan {
    /// Whether system `before` is ordered before system `after`.
    fn before(&self, before: usize, after: usize) -> bool {
        let (stage, other) = (self.stage_of[before], self.stage_of[after]);
        stage < other
            || (stage == other
                && (self.ancestors[after].contains(&before)
                    || (self.kinds[before] == SystemKind::Exclusive
                        && self.kinds[after] != SystemKind::Exclusive)))
    }

    fn tables(&self, index: usize) -> impl Iterator<Item = Access> + '_ {
        let system = self.order[index];
        system.access().iter().chain(system.finalize_access()).copied()
    }

    fn check_conflicts(&self) -> core::result::Result<(), ScheduleError> {
        for stage in &self.stages {
            for first in stage.systems.clone() {
                for second in first + 1..stage.systems.end {
                    if self.before(first, second) || self.before(second, first) {
                        continue;
                    }
                    let exclusive = SystemKind::Exclusive;
                    if self.kinds[first] == exclusive && self.kinds[second] == exclusive {
                        return Err(self.conflict(first, second, AccessTarget::World));
                    }
                    for a in self.tables(first) {
                        for b in self.tables(second) {
                            if let Some(target) = conflict(a, b) {
                                return Err(self.conflict(first, second, target));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn conflict(&self, first: usize, second: usize, target: AccessTarget) -> ScheduleError {
        ScheduleError::UnorderedConflict {
            first: self.order[first].name(),
            second: self.order[second].name(),
            target,
        }
    }

    fn check_events(&self) -> core::result::Result<(), ScheduleError> {
        for reader in 0..self.order.len() {
            for read in self.tables(reader) {
                let AccessTarget::Events { type_id, name } = read.target() else { continue };
                if read.kind() != AccessKind::ReadEvents {
                    continue;
                }
                for emitter in 0..self.order.len() {
                    let emits = self.tables(emitter).any(|access| {
                        access.kind() == AccessKind::EmitEvents
                            && matches!(access.target(), AccessTarget::Events { type_id: id, .. } if id == type_id)
                    });
                    if emits && !self.before(emitter, reader) {
                        return Err(ScheduleError::EventsBeforeEmitter {
                            reader: self.order[reader].name(),
                            emitter: self.order[emitter].name(),
                            event: name,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

/// The target two accesses of different systems conflict on, if any.
fn conflict(a: Access, b: Access) -> Option<AccessTarget> {
    let target = a.target();
    if target != b.target() {
        return None;
    }
    let touches = |kind: AccessKind| is_data(kind) || kind == AccessKind::Changed;
    match target {
        // A declared write through commands counts like a `Mut`; two declarations of one
        // column do not conflict, since commands apply in execution order.
        AccessTarget::Component(_) => ((writes_column(a.kind()) && touches(b.kind()))
            || (writes_column(b.kind()) && touches(a.kind())))
        .then_some(target),
        AccessTarget::Resource { .. } => {
            (is_write(a.kind()) || is_write(b.kind())).then_some(target)
        }
        _ => None,
    }
}

/// An ordered set of stages for one world kind, built once and run every tick.
///
/// The execution order within a stage is the topological order of its DAG with ties broken
/// by [`SystemDescriptor::key`], never by registration order or layout, so it is identical
/// on every build and platform. A `Schedule` is `Send`, so the timer root task may own it.
///
/// A schedule serves one world: the first [`prepare`](Self::prepare), or the first run,
/// ties it to that world, and another world is refused with
/// [`ScheduleError::WrongWorld`], since a pass's `Changed` windows are kept per schedule.
pub struct Schedule {
    kind: WorldKind,
    plan: Arc<Plan>,
    /// Where each system's `Changed` window starts in a pass: the epoch and point it last
    /// ran at in one.
    pass_marks: Vec<Option<(u32, Stamp)>>,
    /// The world the schedule serves, once it has met one; compared by address only.
    world: Option<Weak<()>>,
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

    /// How many stages the schedule has; a run has one more sync point than that.
    #[must_use]
    pub fn stage_count(&self) -> usize {
        self.plan.stages.len()
    }

    /// Checks the schedule against `world`: its kind, its registry (every component the
    /// tables name is registered, `Changed` columns are tracked, `Mut` never names a
    /// maintained component, every transient reader follows a writer) and its resources
    /// (every resource a system names is held). [`run_tick`](Self::run_tick) and
    /// [`run_pass`](Self::run_pass) run it first and fail the same way, but they cannot hand
    /// the world back on failure, so a caller checks here before it starts the loop.
    pub fn prepare(&mut self, world: &World) -> core::result::Result<(), ScheduleError> {
        let plan = self.plan.clone();
        let plan = &*plan;
        if world.kind() != plan.kind {
            return Err(ScheduleError::WrongWorld { schedule: plan.kind, world: world.kind() });
        }
        let identity = Arc::downgrade(&world.identity);
        if self.world.as_ref().is_some_and(|served| !Weak::ptr_eq(served, &identity)) {
            return Err(ScheduleError::WrongWorld { schedule: plan.kind, world: world.kind() });
        }
        let registry = world.registry();
        let components = |index: usize| {
            plan.tables(index).filter_map(|access| match access.target() {
                AccessTarget::Component(key) => Some((access.kind(), key)),
                _ => None,
            })
        };
        for (index, system) in plan.order.iter().enumerate() {
            for (_, component) in components(index) {
                if registry.get(component).is_none() {
                    return Err(ScheduleError::UnregisteredComponent {
                        system: system.name(),
                        component,
                    });
                }
            }
        }
        for (index, system) in plan.order.iter().enumerate() {
            for (kind, component) in components(index) {
                let info = registry.get(component).expect("checked above");
                if kind == AccessKind::Changed && !is_tracked(info) {
                    return Err(ScheduleError::ChangedOnUntracked {
                        system: system.name(),
                        component,
                    });
                }
            }
        }
        for (index, system) in plan.order.iter().enumerate() {
            for (kind, component) in components(index) {
                let info = registry.get(component).expect("checked above");
                if is_write(kind) && refuses_mut(info) {
                    return Err(ScheduleError::WritesMaintainedComponent {
                        system: system.name(),
                        component,
                    });
                }
            }
        }
        for (reader, system) in plan.order.iter().enumerate() {
            for (kind, component) in components(reader) {
                let info = registry.get(component).expect("checked above");
                if !matches!(
                    kind,
                    AccessKind::Read | AccessKind::OptionalRead | AccessKind::Changed
                ) || info.persistence() != Persistence::Transient
                {
                    continue;
                }
                // A declared writer has `Mut` on the column, or declares it in `writes`: an
                // exclusive system writes it at its own sync point, before the stage's other
                // systems; any other system's commands apply at the next sync point, so they
                // are visible from the next stage only.
                let written_before = (0..plan.order.len()).any(|writer| {
                    plan.before(writer, reader)
                        && components(writer).any(|(kind, key)| {
                            key == component
                                && (is_write(kind)
                                    || (kind == AccessKind::Writes
                                        && (plan.stage_of[writer] < plan.stage_of[reader]
                                            || plan.kinds[writer] == SystemKind::Exclusive)))
                        })
                });
                if !written_before {
                    return Err(ScheduleError::TransientBeforeWriter {
                        reader: system.name(),
                        component,
                    });
                }
            }
        }
        for (index, system) in plan.order.iter().enumerate() {
            for access in plan.tables(index) {
                if let AccessTarget::Resource { type_id, name } = access.target()
                    && !world.resources.contains_key(&type_id)
                {
                    return Err(ScheduleError::MissingResource {
                        system: system.name(),
                        resource: name,
                    });
                }
            }
        }
        // Tied only once every check has passed, so a failed prepare ties nothing.
        self.world.get_or_insert(identity);
        Ok(())
    }

    /// The systems in execution order, stage by stage.
    #[must_use]
    pub fn order(&self) -> &[&'static SystemDescriptor] {
        &self.plan.order
    }

    /// Runs one tick: advances the world's tick, then for each stage runs its sync point
    /// and its systems, with the stage's per-entity kernels as chunk jobs on `executor` and
    /// every reduction merged through its [`Reducer`](lactose_math::Reducer).
    ///
    /// The world is taken by value and handed back when the tick resolves, because the run
    /// shares it with jobs on other workers: a future dropped mid-tick drops the world when
    /// the run's jobs have finished, and a forgotten one leaks it, but neither leaves a
    /// borrow the jobs still write through. The future is `Send`, so a `'static` task
    /// owning the world and the schedule can drive it from the timer root; when it resolves
    /// nothing of the tick is executing.
    ///
    /// Errors, on none of which the world is handed back: kind `Cancelled` if the executor
    /// shuts down first (the world is torn); kind `InvalidData` carrying a
    /// [`ScheduleError`] message if [`prepare`](Self::prepare) fails, reported before
    /// anything runs. A caller that wants to keep the world on a `prepare` failure calls
    /// `prepare` first.
    #[allow(clippy::manual_async_fn, reason = "the `Send` bound is the contract")]
    pub fn run_tick<'a>(
        &'a mut self,
        world: World,
        executor: &'a Executor,
    ) -> impl Future<Output = Result<World>> + Send + 'a {
        self.run(world, executor, true)
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
        self.run(world, executor, false)
    }

    async fn run(&mut self, world: World, executor: &Executor, tick: bool) -> Result<World> {
        self.prepare(&world).map_err(|error| {
            Error::new(ErrorKind::InvalidData, error.to_string()).with_source(error)
        })?;
        let marks = core::mem::replace(&mut self.pass_marks, vec![None; self.plan.order.len()]);
        let state = RunState::new(world, self.plan.clone(), tick, marks);
        let state = run(executor, state).await?;
        let (world, marks) = state.into_parts();
        self.pass_marks = marks;
        Ok(world)
    }
}

/// Why a schedule did not build, or cannot run against a world.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
    /// An edge added with [`ScheduleBuilder::order`] names a set that no stage declares.
    UnknownOrderedSet {
        /// The set no stage declares.
        set: &'static str,
        /// The edge's first set.
        before: &'static str,
        /// The edge's second set.
        after: &'static str,
    },
    /// One set is declared in two stages; a set belongs to one.
    SetInTwoStages {
        /// The set.
        set: &'static str,
        /// The first stage that declares it.
        first: &'static str,
        /// The second.
        second: &'static str,
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
    /// An exclusive system or a finalize function takes [`Ctx`](crate::Ctx) or
    /// [`RegionRes`](crate::RegionRes).
    RegionParamInWorldSystem {
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
    /// Two data parameters (`&T`, `Mut<T>`, `Option<&T>`, `Option<Mut<T>>`) of one system
    /// name the same component, or `Without<T>` stands beside any parameter on `T`.
    /// `Changed<T>` and `With<T>` beside one data parameter on `T` are fine.
    AliasedParameter {
        /// The system.
        system: &'static str,
        /// The component.
        component: ComponentKey,
    },
    /// A parameter the system cannot hold as declared, which no variant above names:
    /// `&mut World` in a system that is not exclusive, a component parameter or
    /// `&mut World` in a finalize function, or a second parameter that would alias the
    /// first (`Commands`, `Job` or `&mut World` twice, or a resource written beside another
    /// parameter on the same resource).
    InvalidParameter {
        /// The system.
        system: &'static str,
        /// The parameter's type: `Commands`, `Job`, `&mut World`, the resource's type name,
        /// or `component` for a component parameter of a finalize, whose type the table
        /// does not record.
        parameter: &'static str,
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
    /// The world is not of the kind the schedule was built for, or is not the world the
    /// schedule first prepared or ran with, which it serves alone (the two kinds are then
    /// equal).
    WrongWorld {
        /// The schedule's kind.
        schedule: WorldKind,
        /// The world's kind.
        world: WorldKind,
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
    /// A reader of a transient column is not ordered after a declared writer of it in the
    /// tick: a system with `Mut` on the column; an exclusive system that names it in
    /// `writes = [..]`, in the reader's stage or an earlier one; or any other system that
    /// names it in `writes = [..]`, in an earlier stage, since its commands apply at the
    /// sync point after the reader ran.
    TransientBeforeWriter {
        /// The reader.
        reader: &'static str,
        /// The column.
        component: ComponentKey,
    },
    /// A system names a resource the world does not hold.
    MissingResource {
        /// The system.
        system: &'static str,
        /// The resource's name.
        resource: &'static str,
    },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStage { system } => {
                write!(f, "`{system}` was added before any stage was declared")
            }
            Self::DuplicateKey { first, second, key } => write!(
                f,
                "`{first}` and `{second}` share the system key {:#018x}; rename one",
                key.0
            ),
            Self::UnknownSet { system, set } => {
                write!(f, "`{system}` is in set `{set}`, which is not in its stage")
            }
            Self::UnknownOrderedSet { set, before, after } => write!(
                f,
                "`{before}` is ordered before `{after}`, but `{set}` is declared in no stage"
            ),
            Self::SetInTwoStages { set, first, second } => {
                write!(
                    f,
                    "set `{set}` is declared in stages `{first}` and `{second}`; a set belongs to one"
                )
            }
            Self::UnknownSystem { system, after } => {
                write!(f, "`{system}` is ordered after `{after}`, which was not added")
            }
            Self::ExclusiveWithEntityAccess { system } => write!(
                f,
                "the exclusive system `{system}` takes a component parameter or `StableKeyOf`"
            ),
            Self::EntityParamInWorldSystem { system } => {
                write!(f, "`{system}` takes `StableKeyOf` but runs once per region or tick")
            }
            Self::RegionParamInWorldSystem { system } => write!(
                f,
                "`{system}` takes `Ctx` or `RegionRes` in an exclusive system or a finalize"
            ),
            Self::ResourceWriteInKernel { system, resource } => write!(
                f,
                "`{system}` writes `{resource}` but runs once per entity or region; write it \
                 from a once-per-tick system, an exclusive system or a finalize"
            ),
            Self::JobInKernel { system } => {
                write!(f, "`{system}` takes `Job` but runs once per entity or region")
            }
            Self::AliasedParameter { system, component } => write!(
                f,
                "`{system}` names component {:#018x} in two parameters that alias",
                component.0
            ),
            Self::InvalidParameter { system, parameter } => {
                write!(f, "`{system}` cannot take `{parameter}` as declared")
            }
            Self::Cycle { members } => {
                write!(f, "the order has a cycle or an edge to a later stage: ")?;
                for (index, member) in members.iter().enumerate() {
                    let separator = if index == 0 { "" } else { " -> " };
                    write!(f, "{separator}`{member}`")?;
                }
                Ok(())
            }
            Self::UnorderedConflict { first, second, target } => {
                let target = match target {
                    AccessTarget::World => "the world".to_owned(),
                    AccessTarget::Component(key) => format!("component {:#018x}", key.0),
                    AccessTarget::Resource { name, .. } => format!("resource `{name}`"),
                    other => format!("{other:?}"),
                };
                write!(
                    f,
                    "`{first}` and `{second}` conflict on {target} and nothing orders them; \
                     add an `after` edge or a set order"
                )
            }
            Self::EventsBeforeEmitter { reader, emitter, event } => write!(
                f,
                "`{reader}` reads `{event}` but is not ordered after its emitter `{emitter}`"
            ),
            Self::WrongWorld { schedule, world } if schedule == world => write!(
                f,
                "the schedule serves another world of the same kind; a schedule serves one world"
            ),
            Self::WrongWorld { schedule, world } => {
                write!(f, "the schedule is for a {schedule:?} world, not a {world:?} world")
            }
            Self::UnregisteredComponent { system, component } => write!(
                f,
                "`{system}` names component {:#018x}, which the registry does not hold",
                component.0
            ),
            Self::ChangedOnUntracked { system, component } => write!(
                f,
                "`{system}` filters on `Changed` of component {:#018x}, which is not tracked",
                component.0
            ),
            Self::WritesMaintainedComponent { system, component } => write!(
                f,
                "`{system}` takes `Mut` of component {:#018x}, which the world maintains or \
                 which holds a reference; write it through `Commands::set`",
                component.0
            ),
            Self::TransientBeforeWriter { reader, component } => write!(
                f,
                "`{reader}` reads transient component {:#018x} without being ordered after a \
                 writer of it",
                component.0
            ),
            Self::MissingResource { system, resource } => {
                write!(f, "`{system}` names resource `{resource}`, which the world does not hold")
            }
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Whether a system's spawns can be predicted: it runs in the server and the prediction
/// world alike (architecture section 7).
pub(crate) fn predicts(worlds: WorldSet) -> bool {
    worlds.contains(WorldKind::Server) && worlds.contains(WorldKind::Prediction)
}
