//! Running a schedule. The naive implementation: a run is one graph node on the executor,
//! which runs every sync point and every system in execution order on one worker, one
//! invocation after another. The model is still the parallel one: each invocation sees
//! only its parameters, its outputs are ordered by source before they reach the world, and
//! a reduction folds into per-batch partials that are merged.

use core::any::{Any, TypeId};
use core::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use lactose_core::{PlayerId, Result, Tick};
use lactose_task::{Executor, GraphBuilder, GraphNode, JobContext};

use crate::access::{Access, AccessKind, AccessTarget};
use crate::apply::Pending;
use crate::change::Stamp;
use crate::command::{ChannelStore, CommandBuffer};
use crate::event::{EventRecord, EventStore, PendingEmission, PresentRecord};
use crate::param::Ctx;
use crate::region::{ClaimedBy, InRegion, RegionId, RegionMode};
use crate::rng::LayoutRng;
use crate::schedule::{Plan, Source, SystemKind, predicts};
use crate::storage::typed;
use crate::system::{Invocation, ReducerSlot, Slot, SystemDescriptor, mismatch};
use crate::world::{BETWEEN, ResourceEntry, SLOT_WIDTH, World, WorldKind};

use lactose_reflect::Reflect;

/// What a run owns: the world, the plan and the schedule's pass state.
pub(crate) struct RunState {
    world: World,
    plan: Arc<Plan>,
    tick: bool,
    pass_marks: Vec<Option<(u32, Stamp)>>,
}

impl RunState {
    pub(crate) fn new(
        world: World,
        plan: Arc<Plan>,
        tick: bool,
        pass_marks: Vec<Option<(u32, Stamp)>>,
    ) -> Self {
        Self { world, plan, tick, pass_marks }
    }

    pub(crate) fn into_parts(self) -> (World, Vec<Option<(u32, Stamp)>>) {
        (self.world, self.pass_marks)
    }
}

/// The run as a graph node. The mutex hands the state to the job and back to the caller
/// once the run has resolved; it is taken twice per run and never contended.
struct RunNode {
    name: &'static str,
    state: Mutex<Option<RunState>>,
}

impl GraphNode for RunNode {
    fn name(&self) -> &'static str {
        self.name
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _: Range<u32>, ctx: &mut JobContext<'_>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(state) = state.as_mut() {
            execute(state, ctx);
        }
    }
}

/// Runs `state` as one job on `executor` and hands it back, or fails with `Cancelled` if
/// the executor shut down first; the state, and the world in it, then drops with the node.
pub(crate) async fn run(executor: &Executor, state: RunState) -> Result<RunState> {
    let name = if state.tick { "casein::tick" } else { "casein::pass" };
    let node = Arc::new(RunNode { name, state: Mutex::new(Some(state)) });
    let mut graph = GraphBuilder::new();
    graph.add_node(node.clone());
    let graph = graph.build().expect("a graph of one node has no cycle");
    executor.run_graph(graph).await?;
    let state = node.state.lock().unwrap_or_else(PoisonError::into_inner).take();
    Ok(state.expect("a finished run hands its state back"))
}

/// Enters the next slot of the stamp order and returns its stamp. In a tick each slot has
/// a fixed position, so a system's writes carry the same stamp under every layout; in a
/// pass the slots follow the current point.
fn enter_slot(world: &mut World, tick: bool, slot: &mut u32) -> Stamp {
    if tick {
        let base = (*slot + 1) * SLOT_WIDTH;
        world.point = base;
        world.point_cap = base + SLOT_WIDTH - 1;
    } else {
        world.point_cap = u32::MAX;
        world.point = world.point.saturating_add(1);
    }
    *slot += 1;
    Stamp::new(world.tick, world.point)
}

fn execute(state: &mut RunState, ctx: &mut JobContext<'_>) {
    let RunState { world, plan, tick, pass_marks } = state;
    let tick = *tick;
    world.in_run = true;
    if tick {
        world.advance_tick();
    }
    let mut pending = Vec::new();
    let mut emissions = Vec::new();
    let mut slot = 0;
    for (stage_index, stage) in plan.stages.iter().enumerate() {
        let stamp = enter_slot(world, tick, &mut slot);
        if tick && stage_index == 0 {
            world.input_sync(stamp);
        } else {
            world.apply(core::mem::take(&mut pending), stamp);
        }
        world.permute_rows();
        let _span = tracing::trace_span!(target: "casein", "stage", name = stage.name).entered();
        for index in stage.systems.clone() {
            let now = enter_slot(world, tick, &mut slot);
            let since = if tick {
                Stamp::new(Tick(now.tick.0.saturating_sub(1)), now.position)
            } else {
                pass_marks[index]
                    .filter(|&(epoch, _)| epoch == world.epoch)
                    .map_or(Stamp::ZERO, |(_, stamp)| stamp)
            };
            run_system(world, plan, index, (since, now), ctx, &mut pending, &mut emissions);
            if !tick {
                pass_marks[index] = Some((world.epoch, now));
            }
        }
    }
    let stamp = enter_slot(world, tick, &mut slot);
    world.apply(pending, stamp);
    world.permute_rows();
    world.end_present(emissions);
    if tick {
        world.point = BETWEEN;
    }
    world.point_cap = u32::MAX;
    world.in_run = false;
}

/// The outputs of one invocation.
struct Output {
    source: Source,
    claim: Option<PlayerId>,
    /// Whether the invocation is a reduction's finalize, whose table is the finalize's.
    finalize: bool,
    commands: CommandBuffer,
    /// One sink per parameter; only `EventWriter` parameters fill theirs.
    events: Vec<Vec<EventRecord>>,
    /// One sink per parameter; only `PresentEvents` parameters fill theirs.
    present: Vec<Vec<PresentRecord>>,
}

impl Output {
    fn new(
        source: Source,
        claim: Option<PlayerId>,
        region: Option<RegionId>,
        params: usize,
    ) -> Self {
        Self {
            source,
            claim,
            finalize: false,
            commands: CommandBuffer::new(region),
            events: (0..params).map(|_| Vec::new()).collect(),
            present: (0..params).map(|_| Vec::new()).collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.commands.commands.is_empty()
            && self.events.iter().all(Vec::is_empty)
            && self.present.iter().all(Vec::is_empty)
    }
}

/// A taken-out world resource, lent to one parameter at a time.
enum Handle<'r> {
    Mut(&'r mut (dyn Any + Send + Sync)),
    Shared(&'r (dyn Any + Send + Sync)),
    Used,
}

fn downgrade<'r, T: ?Sized>(value: &'r mut T) -> &'r T {
    value
}

/// Everything an invocation's non-component parameters are made from.
struct Env<'r, 'j> {
    resources: Vec<(TypeId, Handle<'r>)>,
    events: &'r EventStore,
    channels: &'r ChannelStore,
    commands: Option<&'r mut CommandBuffer>,
    emit: Vec<Option<&'r mut Vec<EventRecord>>>,
    present: Vec<Option<&'r mut Vec<PresentRecord>>>,
    job: Option<&'r mut JobContext<'j>>,
    world: Option<&'r mut World>,
}

impl<'r, 'j> Env<'r, 'j> {
    fn new(
        resources: Vec<(TypeId, Handle<'r>)>,
        events: &'r EventStore,
        channels: &'r ChannelStore,
        output: &'r mut Output,
        job: Option<&'r mut JobContext<'j>>,
        world: Option<&'r mut World>,
    ) -> Self {
        let Output { commands, events: emit, present, .. } = output;
        Self {
            resources,
            events,
            channels,
            commands: Some(commands),
            emit: emit.iter_mut().map(Some).collect(),
            present: present.iter_mut().map(Some).collect(),
            job,
            world,
        }
    }

    fn handle(&mut self, type_id: TypeId) -> &mut Handle<'r> {
        &mut self
            .resources
            .iter_mut()
            .find(|(id, _)| *id == type_id)
            .expect("a resource the schedule checked is held for the system's run")
            .1
    }

    fn resource(&mut self, type_id: TypeId) -> &'r (dyn Any + Send + Sync) {
        let handle = self.handle(type_id);
        let shared = match core::mem::replace(handle, Handle::Used) {
            Handle::Mut(value) => downgrade(value),
            Handle::Shared(value) => value,
            Handle::Used => unreachable!("a resource is written by one parameter alone"),
        };
        *handle = Handle::Shared(shared);
        shared
    }

    fn resource_mut(&mut self, type_id: TypeId) -> &'r mut (dyn Any + Send + Sync) {
        match core::mem::replace(self.handle(type_id), Handle::Used) {
            Handle::Mut(value) => value,
            _ => unreachable!("a resource is written by one parameter alone"),
        }
    }

    /// The slot of parameter `index`, for every kind but components, keys and regions.
    fn slot(&mut self, index: usize, access: &Access) -> Slot<'r, 'j> {
        match (access.kind(), access.target()) {
            (AccessKind::Read, AccessTarget::Resource { type_id, .. }) => {
                Slot::Resource(self.resource(type_id))
            }
            (AccessKind::Write, AccessTarget::Resource { type_id, .. }) => {
                Slot::ResourceMut(self.resource_mut(type_id))
            }
            (AccessKind::ReadEvents, AccessTarget::Events { type_id, .. }) => {
                Slot::ReadEvents(self.events.get(&type_id).map_or(&[], Vec::as_slice))
            }
            (AccessKind::EmitEvents, _) => {
                Slot::EmitEvents(self.emit[index].take().expect("one sink per parameter"))
            }
            (AccessKind::Read, AccessTarget::Channel { type_id, .. }) => {
                Slot::Channel(self.channels.get(&type_id).map(|entries| &**entries))
            }
            (AccessKind::Commands, _) => {
                Slot::Commands(self.commands.take().expect("one `Commands` per system"))
            }
            (AccessKind::PresentEvents, _) => {
                Slot::Present(self.present[index].take().expect("one sink per parameter"))
            }
            (AccessKind::Job, _) => Slot::Job(self.job.take().expect("one `Job` per system")),
            (AccessKind::World, _) => {
                Slot::World(self.world.take().expect("an exclusive system's one `&mut World`"))
            }
            _ => mismatch(index),
        }
    }
}

fn mut_handles(taken: &mut [(TypeId, ResourceEntry)]) -> Vec<(TypeId, Handle<'_>)> {
    taken.iter_mut().map(|(id, entry)| (*id, Handle::Mut(&mut *entry.value))).collect()
}

fn resource_ids(system: &SystemDescriptor) -> Vec<TypeId> {
    let mut ids: Vec<TypeId> = system
        .access()
        .iter()
        .chain(system.finalize_access())
        .filter_map(|access| match access.target() {
            AccessTarget::Resource { type_id, .. } => Some(type_id),
            _ => None,
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Runs system `index` of the plan over the world, then hands its outputs on: events to
/// the world in source order, commands and presentation events to the run.
fn run_system(
    world: &mut World,
    plan: &Plan,
    index: usize,
    window: (Stamp, Stamp),
    ctx: &mut JobContext<'_>,
    pending: &mut Vec<Pending>,
    emissions: &mut Vec<PendingEmission>,
) {
    let system = plan.order[index];
    let _span = tracing::trace_span!(target: "casein", "system", name = system.name()).entered();
    // The system's resources leave the world for its run, so an exclusive system's world
    // does not hold them and one parameter can write a resource while others read others.
    let mut taken: Vec<(TypeId, ResourceEntry)> = resource_ids(system)
        .into_iter()
        .filter_map(|id| Some((id, world.resources.remove(&id)?)))
        .collect();
    let mut outputs = Vec::new();
    let mut partials: Vec<Box<dyn Any>> = Vec::new();
    match plan.kinds[index] {
        SystemKind::Exclusive => run_exclusive(world, system, &mut taken, ctx, &mut outputs),
        SystemKind::PerEntity => {
            run_per_entity(world, system, window, &taken, &mut outputs, &mut partials);
        }
        SystemKind::PerRegion => run_per_region(world, system, &taken, &mut outputs, &mut partials),
        SystemKind::Once => run_once(world, system, &mut taken, ctx, &mut outputs, &mut partials),
    }
    if let Some(reducer) = system.kernel().reducer {
        let mut total = (reducer.identity)();
        for partial in &partials {
            (reducer.merge)(&mut *total, &**partial);
        }
        let table = system.finalize_access();
        let mut output = Output::new(Source::Tick, None, None, table.len());
        output.finalize = true;
        {
            let handles = mut_handles(&mut taken);
            let mut env = Env::new(
                handles,
                &world.events,
                &world.channels,
                &mut output,
                Some(&mut *ctx),
                None,
            );
            let slots = table.iter().enumerate().map(|(p, access)| env.slot(p, access)).collect();
            let mut invocation = Invocation::new(slots, ReducerSlot::Total(Some(total)));
            (system.kernel().finalize)(&mut invocation);
        }
        outputs.push(output);
    }
    for (id, entry) in taken {
        world.resources.insert(id, entry);
    }

    outputs.sort_by_key(|output| (output.source, output.finalize));
    let predicts = predicts(system.worlds());
    for output in outputs {
        let table = if output.finalize { system.finalize_access() } else { system.access() };
        for (param, records) in output.events.into_iter().enumerate() {
            let AccessTarget::Events { type_id, .. } = table[param].target() else { continue };
            for mut record in records {
                (record.target, record.alias) = world.event_target(record.target);
                world.events.get_or_insert_with(type_id, Vec::new).push(record);
            }
        }
        for (param, records) in output.present.into_iter().enumerate() {
            let param = (u32::from(output.finalize) << 16) | param as u32;
            for (sequence, record) in records.into_iter().enumerate() {
                emissions.push(PendingEmission {
                    order: (index as u32, output.source, param, sequence as u32),
                    record,
                });
            }
        }
        if !output.commands.commands.is_empty() {
            pending.push(Pending {
                order: index as u32,
                source: output.source,
                claim: output.claim,
                system: system.key(),
                predicts,
                buffer: output.commands,
            });
        }
    }
}

fn run_exclusive(
    world: &mut World,
    system: &SystemDescriptor,
    taken: &mut [(TypeId, ResourceEntry)],
    ctx: &mut JobContext<'_>,
    outputs: &mut Vec<Output>,
) {
    // What the parameters read leaves the world too, so the world can be lent whole.
    let mut events = EventStore::new();
    let mut channels = ChannelStore::new();
    for access in system.access() {
        match (access.kind(), access.target()) {
            (AccessKind::ReadEvents, AccessTarget::Events { type_id, .. }) => {
                if let Some(records) = world.events.remove(&type_id) {
                    events.insert(type_id, records);
                }
            }
            (AccessKind::Read, AccessTarget::Channel { type_id, .. }) => {
                if let Some(entries) = world.channels.remove(&type_id) {
                    channels.insert(type_id, entries);
                }
            }
            _ => {}
        }
    }
    let table = system.access();
    let mut output = Output::new(Source::Tick, None, None, table.len());
    {
        let handles = mut_handles(taken);
        let mut env =
            Env::new(handles, &events, &channels, &mut output, Some(ctx), Some(&mut *world));
        let slots = table.iter().enumerate().map(|(p, access)| env.slot(p, access)).collect();
        let mut invocation = Invocation::new(slots, ReducerSlot::None);
        (system.kernel().run)(&mut invocation);
    }
    for (type_id, records) in events.sorted_take() {
        world.events.insert(type_id, records);
    }
    for (type_id, entries) in channels.sorted_take() {
        world.channels.insert(type_id, entries);
    }
    outputs.push(output);
}

/// Drains a store keyed by `TypeId`; the order is irrelevant, since each entry goes back
/// under its own key.
trait SortedTake<V> {
    fn sorted_take(self) -> Vec<(TypeId, V)>;
}

impl<V> SortedTake<V> for lactose_core::FixedHashMap<TypeId, V> {
    fn sorted_take(mut self) -> Vec<(TypeId, V)> {
        let keys: Vec<TypeId> = self.sorted().into_iter().map(|(&key, _)| key).collect();
        keys.into_iter().filter_map(|key| Some((key, self.remove(&key)?))).collect()
    }
}

fn run_once(
    world: &mut World,
    system: &SystemDescriptor,
    taken: &mut [(TypeId, ResourceEntry)],
    ctx: &mut JobContext<'_>,
    outputs: &mut Vec<Output>,
    partials: &mut Vec<Box<dyn Any>>,
) {
    let table = system.access();
    let mut output = Output::new(Source::Tick, None, None, table.len());
    let mut partial = system.kernel().reducer.map(|reducer| (reducer.identity)());
    {
        let handles = mut_handles(taken);
        let mut env =
            Env::new(handles, &world.events, &world.channels, &mut output, Some(ctx), None);
        let slots = table.iter().enumerate().map(|(p, access)| env.slot(p, access)).collect();
        let reducer =
            partial.as_mut().map_or(ReducerSlot::None, |p| ReducerSlot::Partial(&mut **p));
        let mut invocation = Invocation::new(slots, reducer);
        (system.kernel().run)(&mut invocation);
    }
    partials.extend(partial);
    outputs.push(output);
}

fn shared_handles(taken: &[(TypeId, ResourceEntry)]) -> Vec<(TypeId, &(dyn Any + Send + Sync))> {
    taken.iter().map(|(id, entry)| (*id, &*entry.value)).collect()
}

fn run_per_region(
    world: &mut World,
    system: &SystemDescriptor,
    taken: &[(TypeId, ResourceEntry)],
    outputs: &mut Vec<Output>,
    partials: &mut Vec<Box<dyn Any>>,
) {
    let table = system.access();
    let shared = shared_handles(taken);
    let world = &*world;
    for (&region, state) in &world.regions {
        if state.mode != RegionMode::Simulated {
            continue;
        }
        let lacks_resource = table.iter().any(|access| match access.target() {
            AccessTarget::RegionResource(key) => !state.resources.contains_key(&key),
            _ => false,
        });
        if lacks_resource {
            continue;
        }
        let mut output = Output::new(Source::Region(region), None, Some(region), table.len());
        let mut partial = system.kernel().reducer.map(|reducer| (reducer.identity)());
        {
            let handles = shared.iter().map(|&(id, value)| (id, Handle::Shared(value))).collect();
            let mut env =
                Env::new(handles, &world.events, &world.channels, &mut output, None, None);
            let slots = table
                .iter()
                .enumerate()
                .map(|(p, access)| match (access.kind(), access.target()) {
                    (AccessKind::Region, _) => Slot::Ctx(Ctx::new(
                        state.seed,
                        region,
                        None,
                        system.key(),
                        world.tick,
                        state.assets.as_ref(),
                    )),
                    (_, AccessTarget::RegionResource(key)) => {
                        Slot::Read(state.resources[&key].bytes.as_bytes())
                    }
                    _ => env.slot(p, access),
                })
                .collect();
            let reducer =
                partial.as_mut().map_or(ReducerSlot::None, |p| ReducerSlot::Partial(&mut **p));
            let mut invocation = Invocation::new(slots, reducer);
            (system.kernel().run)(&mut invocation);
        }
        partials.extend(partial);
        if !output.is_empty() {
            outputs.push(output);
        }
    }
}

fn run_per_entity(
    world: &mut World,
    system: &SystemDescriptor,
    (since, now): (Stamp, Stamp),
    taken: &[(TypeId, ResourceEntry)],
    outputs: &mut Vec<Output>,
    partials: &mut Vec<Box<dyn Any>>,
) {
    let table = system.access();
    let shared = shared_handles(taken);
    let World { storage, regions, events, channels, kind, layout, tick, .. } = world;
    let (tick, layout) = (*tick, *layout);
    let component = |access: &Access| match access.target() {
        AccessTarget::Component(key) => Some(key),
        _ => None,
    };
    let keys_of = |wanted: &[AccessKind]| -> Vec<lactose_core::ComponentKey> {
        table.iter().filter(|a| wanted.contains(&a.kind())).filter_map(component).collect()
    };
    let mut required =
        keys_of(&[AccessKind::Read, AccessKind::Write, AccessKind::With, AccessKind::Changed]);
    if *kind == WorldKind::Prediction {
        // The prediction world's own filter: claimed entities only (architecture 9.4).
        required.push(ClaimedBy::KEY);
    }
    let excluded = keys_of(&[AccessKind::Without]);
    let changed = keys_of(&[AccessKind::Changed]);
    let written = keys_of(&[AccessKind::Write, AccessKind::OptionalWrite]);

    let mut jobs = Vec::new();
    for (archetype, arch) in storage.archetypes.iter().enumerate() {
        if required.iter().all(|&key| arch.has(key)) && !excluded.iter().any(|&key| arch.has(key)) {
            jobs.extend((0..arch.chunks.len()).map(|chunk| (archetype, chunk)));
        }
    }
    let jobs = layout.order(jobs, (tick.0.into(), system.key().0));
    let mut batches = LayoutRng::new(layout.seed, tick.0.into(), system.key().0);
    let reducer = system.kernel().reducer;

    for (archetype, chunk) in jobs {
        let arch = &mut storage.archetypes[archetype];
        let columns = &arch.columns;
        let column_of = |key| columns.binary_search_by_key(&key, |c| c.key).ok();
        let params: Vec<Option<usize>> =
            table.iter().map(|access| component(access).and_then(column_of)).collect();
        let region_column = column_of(InRegion::KEY).expect("every row has a region");
        let claim_column = column_of(ClaimedBy::KEY);
        let changed_columns: Vec<usize> =
            changed.iter().filter_map(|&key| column_of(key)).collect();
        let chunk = &mut arch.chunks[chunk];
        // The dispatch of a writing system marks every chunk it is issued over.
        for column in written.iter().filter_map(|&key| column_of(key)) {
            let column_tick = &mut chunk.column_ticks[column];
            *column_tick = (*column_tick).max(now);
        }
        let len = chunk.len();
        let mut views = chunk.split(columns);
        let mut partial = reducer.map(|reducer| (reducer.identity)());
        for row in 0..len {
            let key = views.keys[row];
            let mut pieces: Vec<Option<_>> =
                views.columns.iter_mut().map(|view| Some(view.next_row())).collect();
            let piece = |column: usize| pieces[column].as_ref().expect("an unused piece");
            let region = typed::<InRegion>(piece(region_column).bytes).region;
            let Some(state) = regions.get(&region).filter(|s| s.mode == RegionMode::Simulated)
            else {
                continue;
            };
            let in_window = changed_columns.iter().all(|&column| {
                piece(column).stamp().is_some_and(|stamp| since < stamp && stamp < now)
            });
            let lacks_resource = table.iter().any(|access| match access.target() {
                AccessTarget::RegionResource(key) => !state.resources.contains_key(&key),
                _ => false,
            });
            if !in_window || lacks_resource {
                continue;
            }
            let claim = claim_column.map(|column| typed::<ClaimedBy>(piece(column).bytes).player);
            if layout.split_batches
                && row > 0
                && let Some(reducer) = reducer
                && batches.below(3) == 0
            {
                partials.extend(partial.replace((reducer.identity)()));
            }

            let mut output = Output::new(Source::Entity(key), claim, Some(region), table.len());
            {
                let handles =
                    shared.iter().map(|&(id, value)| (id, Handle::Shared(value))).collect();
                let mut env = Env::new(handles, events, channels, &mut output, None, None);
                let slots = table
                    .iter()
                    .enumerate()
                    .map(|(p, access)| {
                        let view = params[p].and_then(|column| pieces[column].take());
                        match (access.kind(), access.target(), view) {
                            (AccessKind::Read | AccessKind::OptionalRead, _, Some(view)) => {
                                Slot::Read(view.bytes)
                            }
                            (AccessKind::Write | AccessKind::OptionalWrite, _, Some(view)) => {
                                Slot::Write(view, now)
                            }
                            (AccessKind::OptionalRead | AccessKind::OptionalWrite, _, None) => {
                                Slot::Absent
                            }
                            (AccessKind::With | AccessKind::Without | AccessKind::Changed, ..) => {
                                Slot::Unit
                            }
                            (AccessKind::Entity, ..) => Slot::Key(key),
                            (AccessKind::Region, ..) => Slot::Ctx(Ctx::new(
                                state.seed,
                                region,
                                Some(key),
                                system.key(),
                                tick,
                                state.assets.as_ref(),
                            )),
                            (_, AccessTarget::RegionResource(resource), _) => {
                                Slot::Read(state.resources[&resource].bytes.as_bytes())
                            }
                            _ => env.slot(p, access),
                        }
                    })
                    .collect();
                let reducer =
                    partial.as_mut().map_or(ReducerSlot::None, |p| ReducerSlot::Partial(&mut **p));
                let mut invocation = Invocation::new(slots, reducer);
                (system.kernel().run)(&mut invocation);
            }
            if !output.is_empty() {
                outputs.push(output);
            }
        }
        partials.extend(partial);
    }
}
