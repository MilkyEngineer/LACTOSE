//! Running a schedule. A run is one graph node on the executor, which runs every sync point
//! and every system in execution order on one worker. A per-entity system runs as units,
//! each a chunk or a range of one chunk's rows (`unit.rs`), one after another; every other
//! kind runs as invocations. The model is the parallel one: each unit and invocation sees
//! only its parameters, its outputs are ordered by source before they reach the world, and
//! a reduction folds into per-unit partials that are merged.

use core::any::{Any, TypeId};
use core::ops::Range;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use lactose_core::{PlayerId, Result, Tick};
use lactose_task::{Executor, GraphBuilder, GraphNode, JobContext};

use crate::access::{Access, AccessKind, AccessTarget};
use crate::apply::Pending;
use crate::change::Stamp;
use crate::command::{ChannelCommand, ChannelStore, CommandBuffer};
use crate::event::{EventList, EventRecord, EventStore, PendingEmission, PresentRecord};
use crate::param::Ctx;
use crate::region::{ClaimedBy, InRegion, RegionId, RegionMode};
use crate::rng::LayoutRng;
use crate::schedule::{Plan, Source, SystemKind, predicts};
use crate::storage::{Chunk, ChunkViews, ColumnInfo, typed_slice};
use crate::system::{Invocation, ReducerSlot, Slot, SystemDescriptor, mismatch};
use crate::unit::{Admitted, Lent, Unit, UnitEnv};
use crate::world::{BETWEEN, RegionState, ResourceEntry, SLOT_WIDTH, World, WorldKind};

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
#[expect(
    clippy::disallowed_methods,
    reason = "execution only (architecture section 8.2): the one place a schedule becomes a \
              graph run; every result is order-independent by construction"
)]
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

/// Where a pass's window opens the first time it runs in an epoch: at the oldest point
/// the order relative to `now` has, 2^31 ticks back. Every stored stamp is younger (the
/// clamp of `World::advance_tick`), so the window holds every row.
fn first_pass_window(now: Tick) -> Stamp {
    Stamp::new(Tick(now.0.wrapping_add(1 << 31)), 0)
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
        {
            let _span =
                tracing::trace_span!(target: "casein", "sync_point", stage = stage.name).entered();
            if tick && stage_index == 0 {
                world.input_sync(stamp);
            } else {
                world.apply(core::mem::take(&mut pending), stamp);
            }
            world.permute_rows();
        }
        let _span = tracing::trace_span!(target: "casein", "stage", name = stage.name).entered();
        for index in stage.systems.clone() {
            let now = enter_slot(world, tick, &mut slot);
            // The window opens at this system's position one tick back, across the wrap; a
            // pass's, where it last ran in this epoch, or else before every stored stamp.
            let since = if tick {
                Stamp::new(Tick(now.tick.0.wrapping_sub(1)), now.position)
            } else {
                pass_marks[index]
                    .filter(|&(epoch, _)| epoch == world.epoch)
                    .map_or(first_pass_window(now.tick), |(_, stamp)| stamp)
            };
            run_system(world, plan, index, (since, now), ctx, &mut pending, &mut emissions);
            if !tick {
                pass_marks[index] = Some((world.epoch, now));
            }
        }
    }
    let stamp = enter_slot(world, tick, &mut slot);
    let _span = tracing::trace_span!(target: "casein", "sync_point", stage = "closing").entered();
    world.apply(pending, stamp);
    // No channel survives its run (D126): a taker's channel is already empty, and what a
    // channel with readers alone or a skipped taker's holds goes here, so no snapshot
    // point, save or pass between runs sees an entry.
    world.channels.clear();
    world.permute_rows();
    world.end_present(emissions, !tick);
    if tick {
        world.point = BETWEEN;
    }
    world.point_cap = u32::MAX;
    world.in_run = false;
}

/// The outputs of one invocation or one row of a unit.
pub(crate) struct Output {
    source: Source,
    claim: Option<PlayerId>,
    /// Whether the invocation is a reduction's finalize, whose table is the finalize's.
    finalize: bool,
    pub(crate) commands: CommandBuffer,
    /// One sink per parameter, or none when no parameter is an `EventWriter`; only
    /// `EventWriter` parameters fill theirs.
    pub(crate) events: Vec<Vec<EventRecord>>,
    /// One sink per parameter, or none when no parameter is `PresentEvents`; only
    /// `PresentEvents` parameters fill theirs.
    pub(crate) present: Vec<Vec<PresentRecord>>,
    /// One sink per parameter, or none when no parameter is a `ChannelWriter`; only
    /// `ChannelWriter` parameters fill theirs.
    pub(crate) channels: Vec<Vec<ChannelCommand>>,
}

/// How many sinks of each kind an invocation of one table gets: one per parameter when a
/// parameter fills that kind, none otherwise. Counted once per table, so an invocation of
/// a system that writes no event, presentation event or channel allocates no sink.
#[derive(Clone, Copy)]
pub(crate) struct Sinks {
    events: usize,
    present: usize,
    channels: usize,
}

impl Sinks {
    fn of(table: &[Access]) -> Self {
        let per_parameter = |kind| {
            if table.iter().any(|access| access.kind() == kind) { table.len() } else { 0 }
        };
        Self {
            events: per_parameter(AccessKind::EmitEvents),
            present: per_parameter(AccessKind::PresentEvents),
            channels: per_parameter(AccessKind::WriteChannel),
        }
    }
}

impl Output {
    pub(crate) fn new(
        source: Source,
        claim: Option<PlayerId>,
        region: Option<RegionId>,
        sinks: Sinks,
    ) -> Self {
        Self {
            source,
            claim,
            finalize: false,
            commands: CommandBuffer::new(region),
            events: (0..sinks.events).map(|_| Vec::new()).collect(),
            present: (0..sinks.present).map(|_| Vec::new()).collect(),
            channels: (0..sinks.channels).map(|_| Vec::new()).collect(),
        }
    }

    /// Readies this output, which nothing has written to, for the row with `source`.
    pub(crate) fn open(
        &mut self,
        source: Source,
        claim: Option<PlayerId>,
        region: Option<RegionId>,
    ) {
        debug_assert!(self.is_empty() && self.commands.spawns == 0 && !self.finalize);
        self.source = source;
        self.claim = claim;
        self.commands.region = region;
    }

    /// Moves the invocation's channel entries into its command buffer, after its commands:
    /// called once the kernel or the finalize has returned, before `is_empty` decides
    /// whether the output is kept, so an invocation whose only output is a channel entry
    /// keeps it.
    pub(crate) fn close_channels(&mut self) {
        if !self.channels.is_empty() {
            self.commands.close_channels(&mut self.channels);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
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

fn downgrade<T: ?Sized>(value: &mut T) -> &T {
    value
}

/// The world resources an invocation's parameters are lent.
enum Resources<'r> {
    /// Taken out of the world for the system's run, each lent to one parameter at a time.
    Taken(Vec<(TypeId, Handle<'r>)>),
    /// Read by every invocation of a per-entity or per-region system, which writes none.
    Shared(&'r [(TypeId, &'r (dyn Any + Send + Sync))]),
}

/// The sinks of one kind, one per parameter, not yet handed out: parameters take theirs
/// in ascending order, so each splits its own off the front.
pub(crate) struct SinkCursor<'r, T> {
    rest: &'r mut [T],
    /// The parameter index of `rest[0]`.
    next: usize,
}

impl<'r, T> SinkCursor<'r, T> {
    pub(crate) fn new(sinks: &'r mut [T]) -> Self {
        Self { rest: sinks, next: 0 }
    }

    pub(crate) fn take(&mut self, index: usize) -> &'r mut T {
        let rest = core::mem::take(&mut self.rest);
        let (sink, rest) =
            rest[index - self.next..].split_first_mut().expect("one sink per parameter");
        self.rest = rest;
        self.next = index + 1;
        sink
    }
}

/// Everything an invocation's non-component parameters are made from.
struct Env<'r, 'j> {
    resources: Resources<'r>,
    events: &'r EventStore,
    channels: &'r ChannelStore,
    commands: Option<&'r mut CommandBuffer>,
    emit: SinkCursor<'r, Vec<EventRecord>>,
    present: SinkCursor<'r, Vec<PresentRecord>>,
    write_channels: SinkCursor<'r, Vec<ChannelCommand>>,
    /// The stores an exclusive system's `ChannelTake` parameters own, removed from the
    /// world for good.
    takes: Vec<(TypeId, Box<dyn Any + Send + Sync>)>,
    job: Option<&'r mut JobContext<'j>>,
    world: Option<&'r mut World>,
}

impl<'r, 'j> Env<'r, 'j> {
    fn new(
        resources: Resources<'r>,
        events: &'r EventStore,
        channels: &'r ChannelStore,
        output: &'r mut Output,
        job: Option<&'r mut JobContext<'j>>,
        world: Option<&'r mut World>,
    ) -> Self {
        let Output { commands, events: emit, present, channels: write_channels, .. } = output;
        Self {
            resources,
            events,
            channels,
            commands: Some(commands),
            emit: SinkCursor::new(emit),
            present: SinkCursor::new(present),
            write_channels: SinkCursor::new(write_channels),
            takes: Vec::new(),
            job,
            world,
        }
    }

    fn handle(&mut self, type_id: TypeId) -> &mut Handle<'r> {
        let Resources::Taken(handles) = &mut self.resources else {
            unreachable!("a system that shares its resources writes none")
        };
        &mut handles
            .iter_mut()
            .find(|(id, _)| *id == type_id)
            .expect("a resource the schedule checked is held for the system's run")
            .1
    }

    fn resource(&mut self, type_id: TypeId) -> &'r (dyn Any + Send + Sync) {
        if let Resources::Shared(shared) = self.resources {
            return shared
                .iter()
                .find(|(id, _)| *id == type_id)
                .expect("a resource the schedule checked is held for the system's run")
                .1;
        }
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
                Slot::ReadEvents(self.events.get(&type_id).unwrap_or_else(|| EventList::empty()))
            }
            (AccessKind::EmitEvents, _) => Slot::EmitEvents(self.emit.take(index)),
            (AccessKind::Read, AccessTarget::Channel { type_id, .. }) => {
                Slot::Channel(self.channels.get(&type_id).map(|stored| &*stored.entries))
            }
            (AccessKind::WriteChannel, _) => Slot::WriteChannel(self.write_channels.take(index)),
            (AccessKind::TakeChannel, AccessTarget::Channel { type_id, .. }) => {
                // The schedule allows one take of a channel per system.
                let store = self.takes.iter().position(|(id, _)| *id == type_id);
                Slot::TakeChannel(store.map(|position| self.takes.swap_remove(position).1))
            }
            (AccessKind::Commands, _) => {
                Slot::Commands(self.commands.take().expect("one `Commands` per system"))
            }
            (AccessKind::PresentEvents, _) => Slot::Present(self.present.take(index)),
            (AccessKind::Job, _) => Slot::Job(self.job.take().expect("one `Job` per system")),
            // A `writes` declaration: no parameter fetches it.
            (AccessKind::Writes, _) => Slot::Unit,
            (AccessKind::World, _) => {
                Slot::World(self.world.take().expect("an exclusive system's one `&mut World`"))
            }
            _ => mismatch(index),
        }
    }
}

fn mut_handles(taken: &mut [(TypeId, ResourceEntry)]) -> Resources<'_> {
    Resources::Taken(
        taken.iter_mut().map(|(id, entry)| (*id, Handle::Mut(&mut *entry.value))).collect(),
    )
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
    let wanted = resource_ids(system);
    let mut taken: Vec<(TypeId, ResourceEntry)> =
        wanted.iter().filter_map(|&id| Some((id, world.resources.remove(&id)?))).collect();
    if taken.len() != wanted.len() {
        // `prepare` found every resource; an exclusive system earlier in the run removed
        // one. The system cannot run, and the rest of the run goes on; the channels it
        // would have taken are named, since nothing else takes them.
        let untaken: Vec<&str> = system
            .access()
            .iter()
            .filter(|access| access.kind() == AccessKind::TakeChannel)
            .filter_map(|access| match access.target() {
                AccessTarget::Channel { name, .. } => Some(name),
                _ => None,
            })
            .collect();
        tracing::error!(
            target: "casein",
            system = system.name(),
            untaken_channels = ?untaken,
            "a resource the system names was removed during the run; the system is skipped"
        );
        for (id, entry) in taken {
            world.resources.insert(id, entry);
        }
        return;
    }
    let mut outputs = Vec::new();
    let mut partials: Vec<Box<dyn Any + Send>> = Vec::new();
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
        let mut output = Output::new(Source::Tick, None, None, Sinks::of(table));
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
        output.close_channels();
        outputs.push(output);
    }
    for (id, entry) in taken {
        // An exclusive system that inserted one of its own resources anew keeps that value.
        world.resources.get_or_insert_with(id, || entry);
    }

    outputs.sort_by_key(|output| (output.source, output.finalize));
    let predicts = predicts(system.worlds());
    let mut emitted: Vec<TypeId> = Vec::new();
    for output in outputs {
        let table = if output.finalize { system.finalize_access() } else { system.access() };
        for (param, records) in output.events.into_iter().enumerate() {
            let AccessTarget::Events { type_id, .. } = table[param].target() else { continue };
            if !records.is_empty() && !emitted.contains(&type_id) {
                emitted.push(type_id);
            }
            for mut record in records {
                (record.target, record.alias) = world.event_target(record.target);
                world.events.get_or_insert_with(type_id, EventList::default).push(record);
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
    for type_id in emitted {
        world.events.get_mut(&type_id).expect("a list the system appended to").index();
    }
}

fn run_exclusive(
    world: &mut World,
    system: &SystemDescriptor,
    taken: &mut [(TypeId, ResourceEntry)],
    ctx: &mut JobContext<'_>,
    outputs: &mut Vec<Output>,
) {
    // What the parameters read leaves the world too, so the world can be lent whole; what
    // they take leaves it for good.
    let mut events = EventStore::new();
    let mut channels = ChannelStore::new();
    let mut takes = Vec::new();
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
            (AccessKind::TakeChannel, AccessTarget::Channel { type_id, .. }) => {
                if let Some(stored) = world.channels.remove(&type_id) {
                    takes.push((type_id, stored.entries));
                }
            }
            _ => {}
        }
    }
    let table = system.access();
    let mut output = Output::new(Source::Tick, None, None, Sinks::of(table));
    {
        let handles = mut_handles(taken);
        let mut env =
            Env::new(handles, &events, &channels, &mut output, Some(ctx), Some(&mut *world));
        env.takes = takes;
        let slots = table.iter().enumerate().map(|(p, access)| env.slot(p, access)).collect();
        let mut invocation = Invocation::new(slots, ReducerSlot::None);
        (system.kernel().run)(&mut invocation);
    }
    output.close_channels();
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
    partials: &mut Vec<Box<dyn Any + Send>>,
) {
    let table = system.access();
    let mut output = Output::new(Source::Tick, None, None, Sinks::of(table));
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
    output.close_channels();
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
    partials: &mut Vec<Box<dyn Any + Send>>,
) {
    let table = system.access();
    let shared = shared_handles(taken);
    let sinks = Sinks::of(table);
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
        let mut output = Output::new(Source::Region(region), None, Some(region), sinks);
        let mut partial = system.kernel().reducer.map(|reducer| (reducer.identity)());
        {
            let handles = Resources::Shared(&shared);
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
        output.close_channels();
        partials.extend(partial);
        if !output.is_empty() {
            outputs.push(output);
        }
    }
}

/// One chunk a per-entity system is issued over.
struct Issued<'w> {
    archetype: usize,
    chunk: usize,
    columns: &'w [ColumnInfo],
    data: &'w mut Chunk,
}

/// The units of an issued chunk that has rows to visit.
struct ChunkUnits {
    /// Where the chunk's selection by the `Changed` filters starts in the run's selection
    /// words; `None` without a filter.
    selected: Option<usize>,
    /// The chunk's row ranges, one per unit, ascending and covering every row.
    ranges: Range<usize>,
}

/// A chunk's column, split among its units: a column a parameter writes is lent to that
/// parameter alone, a range per unit; any other is shared by every parameter and the unit.
enum Part<'w> {
    Shared(&'w [u8]),
    Rest(&'w mut [u8], Option<(&'w mut [Tick], &'w mut [u32])>),
}

impl<'w> Part<'w> {
    fn shared(&self, rows: &Range<usize>, size: usize) -> &'w [u8] {
        match self {
            Self::Shared(bytes) => {
                let bytes: &'w [u8] = bytes;
                &bytes[rows.start * size..rows.end * size]
            }
            Self::Rest(..) => unreachable!("a column a kernel writes is lent to its writer alone"),
        }
    }

    /// The next `rows` rows of a written column, split off its front.
    fn split_off(&mut self, rows: usize, size: usize) -> Lent<'w> {
        let Self::Rest(bytes, stamps) = self else {
            unreachable!("a written column is split among the units")
        };
        let (front, back) = core::mem::take(bytes).split_at_mut(rows * size);
        *bytes = back;
        let front_stamps = match stamps.take() {
            Some((ticks, positions)) => {
                let (tick_front, ticks) = ticks.split_at_mut(rows);
                let (position_front, positions) = positions.split_at_mut(rows);
                *stamps = Some((ticks, positions));
                Some((tick_front, position_front))
            }
            None => None,
        };
        Lent::Write(front, front_stamps)
    }
}

/// The regions whose rows a per-entity system with `table` runs: simulated, and holding
/// every region resource the system reads. Ascending by ID.
fn admitted<'w>(
    regions: &'w BTreeMap<RegionId, RegionState>,
    table: &[Access],
) -> Vec<Admitted<'w>> {
    let region_resources = || {
        table.iter().filter_map(|access| match access.target() {
            AccessTarget::RegionResource(key) => Some(key),
            _ => None,
        })
    };
    regions
        .iter()
        .filter(|(_, state)| {
            state.mode == RegionMode::Simulated
                && region_resources().all(|key| state.resources.contains_key(&key))
        })
        .map(|(&id, state)| Admitted {
            id,
            seed: state.seed,
            assets: state.assets.as_ref(),
            resources: region_resources()
                .map(|key| state.resources[&key].bytes.as_bytes())
                .collect(),
        })
        .collect()
}

/// Runs a per-entity system as units: one per chunk it is issued over, or under the layout
/// mode's `split_batches` a random range of one chunk's rows each. Everything the units
/// share is decided before the first runs: the chunks the `Changed` filters skip, the
/// column ticks of the columns the system writes, each chunk's selection by its filters and
/// the columns lent to each unit. Each unit keeps its own outputs and partial, merged here
/// after the last has run, by key and by the reducer, never in the order they ran.
fn run_per_entity(
    world: &mut World,
    system: &SystemDescriptor,
    (since, now): (Stamp, Stamp),
    taken: &[(TypeId, ResourceEntry)],
    outputs: &mut Vec<Output>,
    partials: &mut Vec<Box<dyn Any + Send>>,
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
    let env = UnitEnv {
        table,
        system: system.key(),
        tick,
        now,
        resources: &shared,
        events,
        channels,
        regions: admitted(regions, table),
        sinks: Sinks::of(table),
    };

    let mut issued = Vec::new();
    for (archetype, arch) in storage.archetypes.iter_mut().enumerate() {
        if required.iter().all(|&key| arch.has(key)) && !excluded.iter().any(|&key| arch.has(key)) {
            let columns = &arch.columns;
            issued.extend(arch.chunks.iter_mut().enumerate().map(|(chunk, data)| Issued {
                archetype,
                chunk,
                columns,
                data,
            }));
        }
    }
    let mut issued = layout.order(issued, (tick.0.into(), system.key().0));

    let mut chunks_skipped = 0_usize;
    let mut selection: Vec<u64> = Vec::new();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut plans: Vec<Option<ChunkUnits>> = Vec::with_capacity(issued.len());
    let mut changed_columns: Vec<usize> = Vec::with_capacity(changed.len());
    for Issued { archetype, chunk, columns, data } in &mut *issued {
        let column_of = |key| columns.binary_search_by_key(&key, |c| c.key).ok();
        changed_columns.clear();
        changed_columns.extend(changed.iter().filter_map(|&key| column_of(key)));
        // A column tick is at or after every row stamp of its chunk, so a chunk whose tick
        // on a filtered column is not after the window holds no row the filter passes.
        let unchanged =
            changed_columns.iter().any(|&column| !data.column_ticks[column].after(since, now.tick));
        // The dispatch of a writing system marks every chunk it is issued over, skipped or
        // not, before any unit runs.
        for column in written.iter().filter_map(|&key| column_of(key)) {
            data.column_ticks[column] = now;
        }
        let len = data.len();
        if unchanged {
            chunks_skipped += 1;
        }
        if unchanged || len == 0 {
            plans.push(None);
            continue;
        }
        let selected = (!changed_columns.is_empty()).then(|| {
            let start = selection.len();
            selection.resize(start + len.div_ceil(64), 0);
            for row in 0..len {
                let in_window = changed_columns.iter().all(|&column| {
                    let stamp = data.stamps[column].as_ref().expect("a tracked column").get(row);
                    stamp.after(since, now.tick) && now.after(stamp, now.tick)
                });
                if in_window {
                    selection[start + row / 64] |= 1 << (row % 64);
                }
            }
            start
        });
        let first = ranges.len();
        if layout.split_batches {
            // Batches of random size, drawn from (seed, tick, system, archetype, chunk), so no
            // split depends on another chunk or on the order the units run in.
            let system_draw = LayoutRng::new(layout.seed, tick.0.into(), system.key().0).next();
            let mut batches = LayoutRng::new(system_draw, *archetype as u64, *chunk as u64);
            let mut start = 0;
            for row in 1..len {
                if batches.below(3) == 0 {
                    ranges.push(start..row);
                    start = row;
                }
            }
            ranges.push(start..len);
        } else {
            ranges.push(0..len);
        }
        plans.push(Some(ChunkUnits { selected, ranges: first..ranges.len() }));
    }

    let selection = &selection[..];
    let mut units = Vec::with_capacity(ranges.len());
    let mut params: Vec<Option<usize>> = Vec::with_capacity(table.len());
    for (Issued { columns, data, .. }, plan) in issued.into_iter().zip(plans) {
        let Some(plan) = plan else { continue };
        let column_of = |key| columns.binary_search_by_key(&key, |c| c.key).ok();
        // The column each data parameter is lent; filters on the same component take none.
        params.clear();
        params.extend(table.iter().map(|access| match access.kind() {
            AccessKind::Read
            | AccessKind::Write
            | AccessKind::OptionalRead
            | AccessKind::OptionalWrite => component(access).and_then(column_of),
            _ => None,
        }));
        let writes = |column: usize| {
            table.iter().zip(&params).any(|(access, &param)| {
                matches!(access.kind(), AccessKind::Write | AccessKind::OptionalWrite)
                    && param == Some(column)
            })
        };
        let region_column = column_of(InRegion::KEY).expect("every row has a region");
        let claim_column = column_of(ClaimedBy::KEY);
        let ChunkViews { keys, columns: views } = data.split(columns);
        let mut parts: Vec<Part<'_>> = views
            .into_iter()
            .enumerate()
            .map(|(column, view)| {
                if writes(column) {
                    Part::Rest(view.bytes, view.stamps)
                } else {
                    Part::Shared(view.bytes)
                }
            })
            .collect();
        for rows in &ranges[plan.ranges] {
            let size = |column: usize| columns[column].size;
            let lent = table
                .iter()
                .zip(&params)
                .map(|(access, &param)| match (access.kind(), param) {
                    (AccessKind::Write | AccessKind::OptionalWrite, Some(column)) => {
                        parts[column].split_off(rows.len(), size(column))
                    }
                    (AccessKind::Read | AccessKind::OptionalRead, Some(column)) => {
                        Lent::Read(parts[column].shared(rows, size(column)))
                    }
                    (AccessKind::OptionalRead | AccessKind::OptionalWrite, None) => Lent::Absent,
                    _ => Lent::None,
                })
                .collect();
            let regions = parts[region_column].shared(rows, size(region_column));
            let claims = claim_column.map(|column| {
                typed_slice::<ClaimedBy>(parts[column].shared(rows, size(column)), rows.len())
            });
            units.push(Unit::new(
                &env,
                &keys[rows.clone()],
                typed_slice::<InRegion>(regions, rows.len()),
                claims,
                plan.selected.map(|start| (&selection[start..], rows.start)),
                lent,
            ));
        }
    }

    for unit in &mut units {
        (system.kernel().each)(unit);
    }
    let (count, mut rows, mut invocations) = (units.len(), 0, 0);
    for unit in units {
        rows += unit.len();
        invocations += unit.invocations;
        outputs.extend(unit.outputs);
        partials.extend(unit.partial);
    }
    tracing::trace!(
        target: "casein",
        system = system.name(),
        units = count,
        chunks_skipped,
        rows,
        invocations,
        "per_entity"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pass's first window holds a stamp of any age the clamp allows: up to 2^30 at a
    /// rebase point, and up to 2^31 - 1 just before the next one, across the wrap.
    #[test]
    fn a_first_pass_window_holds_every_stamp_the_clamp_allows() {
        for now in [1000, (1 << 31) + 3, u32::MAX, 5] {
            let now = Tick(now);
            for age in [0, 1, 1 << 30, (1 << 30) + 2, (1 << 31) - 1] {
                let stamp = Stamp::new(Tick(now.0.wrapping_sub(age)), 0);
                assert!(
                    stamp.after(first_pass_window(now), now),
                    "a stamp {age} ticks old at tick {} is outside the window",
                    now.0
                );
            }
        }
    }
}
