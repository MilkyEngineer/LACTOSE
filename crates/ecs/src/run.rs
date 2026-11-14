//! What a schedule run is made of below its graph (`node.rs`): the outputs of one
//! invocation or row and the order D117 hands them on in, the environment parameters are
//! made from, the stamp slots of the run, and an exclusive system's run at its sync point.
//! Every unit and invocation sees only its parameters, its outputs are ordered by source
//! before they reach the world, and a reduction folds into per-unit partials that are
//! merged.

use core::any::{Any, TypeId};
use std::collections::BTreeMap;

use lactose_core::{Arena, ComponentKey, PlayerId, Tick};
use lactose_task::JobContext;

use crate::access::{Access, AccessKind, AccessTarget};
use crate::apply::Pending;
use crate::change::Stamp;
use crate::command::{ChannelCommand, ChannelStore, CommandBuffer};
use crate::event::{EventList, EventRecord, EventStore, PendingEmission, PresentRecord};
use crate::region::{RegionId, RegionMode};
use crate::schedule::{Plan, Source, predicts};
use crate::system::{Invocation, ReducerSlot, Slot, SystemDescriptor, mismatch};
use crate::unit::Admitted;
use crate::world::{RegionState, ResourceEntry, SLOT_WIDTH, World};

/// The outputs of one invocation or one row of a unit.
pub(crate) struct Output {
    source: Source,
    claim: Option<PlayerId>,
    /// Whether the invocation is a reduction's finalize, whose table is the finalize's.
    pub(crate) finalize: bool,
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
    pub(crate) fn of(table: &[Access]) -> Self {
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

/// A world resource lent to an invocation: written by one parameter, or read by any.
pub(crate) enum Handle<'r> {
    Mut(&'r mut (dyn Any + Send + Sync)),
    Shared(&'r (dyn Any + Send + Sync)),
    Used,
}

fn downgrade<T: ?Sized>(value: &mut T) -> &T {
    value
}

/// The world resources an invocation's parameters are lent.
pub(crate) enum Resources<'r> {
    /// Lent one by one: a resource the table writes to its one parameter, any other to
    /// every parameter that reads it.
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
pub(crate) struct Env<'r, 'j> {
    resources: Resources<'r>,
    /// The lists of the event types the table reads, by `TypeId`.
    events: &'r [(TypeId, &'r EventList)],
    channels: &'r ChannelStore,
    commands: Option<&'r mut CommandBuffer>,
    emit: SinkCursor<'r, Vec<EventRecord>>,
    present: SinkCursor<'r, Vec<PresentRecord>>,
    write_channels: SinkCursor<'r, Vec<ChannelCommand>>,
    /// The stores an exclusive system's `ChannelTake` parameters own, removed from the
    /// world for good.
    pub(crate) takes: Vec<(TypeId, Box<dyn Any + Send + Sync>)>,
    job: Option<&'r mut JobContext<'j>>,
    world: Option<&'r mut World>,
}

impl<'r, 'j> Env<'r, 'j> {
    pub(crate) fn new(
        resources: Resources<'r>,
        events: &'r [(TypeId, &'r EventList)],
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
    pub(crate) fn slot(&mut self, index: usize, access: &Access) -> Slot<'r, 'j> {
        match (access.kind(), access.target()) {
            (AccessKind::Read, AccessTarget::Resource { type_id, .. }) => {
                Slot::Resource(self.resource(type_id))
            }
            (AccessKind::Write, AccessTarget::Resource { type_id, .. }) => {
                Slot::ResourceMut(self.resource_mut(type_id))
            }
            (AccessKind::ReadEvents, AccessTarget::Events { type_id, .. }) => {
                Slot::ReadEvents(list_of(self.events, type_id))
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

/// The list of event type `type_id` among `lists`, or the empty list.
pub(crate) fn list_of<'r>(lists: &[(TypeId, &'r EventList)], type_id: TypeId) -> &'r EventList {
    lists.iter().find(|(id, _)| *id == type_id).map_or_else(|| EventList::empty(), |(_, list)| list)
}

fn mut_handles(taken: &mut [(TypeId, ResourceEntry)]) -> Resources<'_> {
    Resources::Taken(
        taken.iter_mut().map(|(id, entry)| (*id, Handle::Mut(&mut *entry.value))).collect(),
    )
}

/// The world resources `system`'s tables name, ascending and without repeats.
pub(crate) fn resource_ids(system: &SystemDescriptor) -> Vec<TypeId> {
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

/// Reports a system skipped because an exclusive system earlier in the run removed a
/// resource it names; the channels it would have taken are named, since nothing else
/// takes them.
pub(crate) fn report_skipped(system: &SystemDescriptor) {
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
}

/// Hands on the outputs of system `index`, which ran over `world`, in the order D117
/// fixes whatever order they were made in: by source, a reduction's finalize after its
/// kernel, then by parameter and emission. Events go to `events` with their targets
/// resolved, commands to `pending` and presentation events to `emissions`, both keyed by
/// the system's position in the plan.
pub(crate) fn hand_on(
    world: &World,
    system: &SystemDescriptor,
    index: usize,
    mut outputs: Vec<Output>,
    events: &mut Vec<(TypeId, EventRecord)>,
    pending: &mut Vec<Pending>,
    emissions: &mut Vec<PendingEmission>,
) {
    outputs.sort_by_key(|output| (output.source, output.finalize));
    let predicts = predicts(system.worlds());
    for output in outputs {
        let table = if output.finalize { system.finalize_access() } else { system.access() };
        for (param, records) in output.events.into_iter().enumerate() {
            let AccessTarget::Events { type_id, .. } = table[param].target() else { continue };
            for mut record in records {
                (record.target, record.alias) = world.event_target(record.target);
                events.push((type_id, record));
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

/// Appends `events` to the world's lists, in their order, and indexes each list touched.
pub(crate) fn push_events(
    world: &mut World,
    events: impl IntoIterator<Item = (TypeId, EventRecord)>,
) {
    let mut touched: Vec<TypeId> = Vec::new();
    for (type_id, record) in events {
        if !touched.contains(&type_id) {
            touched.push(type_id);
        }
        world.events.get_or_insert_with(type_id, EventList::default).push(record);
    }
    for type_id in touched {
        world.events.get_mut(&type_id).expect("a list just appended to").index();
    }
}

/// Runs exclusive system `index` of the plan at its sync point, with the world held alone,
/// and hands its outputs on: events to the world, after those of every system before it,
/// commands and presentation events to the run.
pub(crate) fn run_exclusive_system(
    world: &mut World,
    plan: &Plan,
    index: usize,
    ctx: &mut JobContext<'_>,
    pending: &mut Vec<Pending>,
    emissions: &mut Vec<PendingEmission>,
) {
    let system = plan.order[index];
    let _span = tracing::trace_span!(target: "casein", "system", name = system.name()).entered();
    // The system's resources leave the world for its run, so the world can be lent whole
    // beside them; nothing else runs while it does.
    let wanted = resource_ids(system);
    let mut taken: Vec<(TypeId, ResourceEntry)> =
        wanted.iter().filter_map(|&id| Some((id, world.resources.remove(&id)?))).collect();
    if taken.len() != wanted.len() {
        // `prepare` found every resource; an exclusive system earlier in the run removed
        // one. The system cannot run, and the rest of the run goes on.
        report_skipped(system);
        for (id, entry) in taken {
            world.resources.insert(id, entry);
        }
        return;
    }
    let mut outputs = Vec::new();
    run_exclusive(world, system, &mut taken, ctx, &mut outputs);
    for (id, entry) in taken {
        // An exclusive system that inserted one of its own resources anew keeps that value.
        world.resources.get_or_insert_with(id, || entry);
    }
    let mut events = Vec::new();
    hand_on(world, system, index, outputs, &mut events, pending, emissions);
    push_events(world, events);
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
        let lists: Vec<(TypeId, &EventList)> =
            events.sorted().into_iter().map(|(&id, list)| (id, list)).collect();
        let handles = mut_handles(taken);
        let mut env =
            Env::new(handles, &lists, &channels, &mut output, Some(ctx), Some(&mut *world));
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

/// Enters the next slot of the stamp order and returns its stamp. In a tick each slot has
/// a fixed position, so a system's writes carry the same stamp under every layout; in a
/// pass the slots follow the current point.
pub(crate) fn enter_slot(world: &mut World, tick: bool, slot: &mut u32) -> Stamp {
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
pub(crate) fn first_pass_window(now: Tick) -> Stamp {
    Stamp::new(Tick(now.0.wrapping_add(1 << 31)), 0)
}

/// The regions whose rows a per-entity system with `table` runs: simulated, and holding
/// every region resource the system reads. Ascending by ID.
pub(crate) fn admitted<'w>(
    regions: &'w BTreeMap<RegionId, RegionState>,
    table: &'w [Access],
) -> impl Iterator<Item = (RegionId, &'w RegionState)> {
    regions.iter().map(|(&id, state)| (id, state)).filter(|(_, state)| {
        state.mode == RegionMode::Simulated
            && region_resources(table).all(|key| state.resources.contains_key(&key))
    })
}

/// The admitted regions of `table`, made in `arena`.
pub(crate) fn admitted_in<'w>(
    arena: &'w Arena,
    regions: &'w BTreeMap<RegionId, RegionState>,
    table: &'w [Access],
) -> &'w [Admitted<'w>] {
    let empty = Admitted { id: RegionId(0), seed: 0, assets: None, resources: &[] };
    let count = admitted(regions, table).count();
    collect_in(
        arena,
        count,
        empty,
        admitted(regions, table).map(|(id, state)| {
            let count = region_resources(table).count();
            let bytes = region_resources(table).map(|key| state.resources[&key].bytes.as_bytes());
            Admitted {
                id,
                seed: state.seed,
                assets: state.assets.as_ref(),
                resources: collect_in(arena, count, &[][..], bytes),
            }
        }),
    )
}

fn region_resources(table: &[Access]) -> impl Iterator<Item = ComponentKey> + '_ {
    table.iter().filter_map(|access| match access.target() {
        AccessTarget::RegionResource(key) => Some(key),
        _ => None,
    })
}

/// The `len` items of `items`, collected into `arena`; `fill` is a placeholder each item
/// overwrites.
pub(crate) fn collect_in<T: Copy>(
    arena: &Arena,
    len: usize,
    fill: T,
    items: impl IntoIterator<Item = T>,
) -> &[T] {
    let slice = arena.alloc_slice_fill(len, fill);
    let mut filled = 0;
    for (place, item) in slice.iter_mut().zip(items) {
        *place = item;
        filled += 1;
    }
    assert_eq!(filled, len, "as many items as the slice holds");
    slice
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
