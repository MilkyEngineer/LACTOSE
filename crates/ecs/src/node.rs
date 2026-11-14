//! A schedule run as a graph on the executor (D137 part B). Each sync point is a node, and
//! so is each system that is not exclusive: its units are the kernel path's (`unit.rs`), a
//! chunk or a range of one chunk's rows for a per-entity system, a region for a per-region
//! one, one unit for a once-per-tick one. An event type that a stage both emits and reads
//! gets a node of its own, which merges its emitters' events in plan order between them and
//! its readers. The graph is built once per schedule and run every tick.
//!
//! A sync point holds the world alone: it hands on what the stage before it left, applies
//! the commands, runs the stage's exclusive systems, enters every system's stamp slot in
//! plan order, and takes the pointers the stage's nodes reach the world through (the
//! [`StageView`]). A system node's `units` makes the units, its `run` runs them and keeps
//! each one's outputs in the slot of that unit, and its `finalize` merges them in unit order,
//! runs a reduction's finalize and hands everything on in the order D117 fixes. What runs
//! concurrently is what the schedule's conflict sets allow; `SAFETY.md` holds the argument.

use core::any::{Any, TypeId};
use core::ops::Range;
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError,
};

use lactose_core::{Arena, Result, Tick};
use lactose_reflect::Reflect;
use lactose_task::{Executor, Graph, GraphBuilder, GraphNode, JobContext, NodeId};

use crate::access::{Access, AccessKind, AccessTarget};
use crate::apply::Pending;
use crate::change::Stamp;
use crate::event::{EventList, EventRecord, PendingEmission};
use crate::param::Ctx;
use crate::region::{ClaimedBy, InRegion, RegionId};
use crate::rng::LayoutRng;
use crate::run::{
    Env, Handle, Output, Resources, Sinks, admitted, admitted_in, collect_in, enter_slot,
    first_pass_window, hand_on, push_events, report_skipped, resource_ids, run_exclusive_system,
};
use crate::schedule::{Plan, Source, SystemKind};
use crate::storage::{Archetype, typed_slice};
use crate::system::{Invocation, ReducerSlot, Slot, SystemDescriptor};
use crate::unit::{Lent, Unit, UnitEnv};
use crate::world::{BETWEEN, World, WorldKind};

/// What a run owns: the world and the schedule's pass state, lent to the graph for the run
/// and handed back after it.
pub(crate) struct RunState {
    world: World,
    meta: RunMeta,
}

impl RunState {
    pub(crate) fn new(world: World, tick: bool, pass_marks: Vec<Option<(u32, Stamp)>>) -> Self {
        let meta =
            RunMeta { tick, pass_marks, slot: 0, pending: Vec::new(), emissions: Vec::new() };
        Self { world, meta }
    }

    pub(crate) fn into_parts(self) -> (World, Vec<Option<(u32, Stamp)>>) {
        (self.world, self.meta.pass_marks)
    }
}

/// What the sync points of a run keep between them.
struct RunMeta {
    tick: bool,
    pass_marks: Vec<Option<(u32, Stamp)>>,
    /// The stamp slots entered so far.
    slot: u32,
    /// The command buffers waiting for the next sync point.
    pending: Vec<Pending>,
    emissions: Vec<PendingEmission>,
}

/// A schedule's graph and what its nodes share, built once and run every tick.
pub(crate) struct RunGraph {
    graph: Graph,
    shared: Arc<Shared>,
}

/// What the nodes of a run share. Each lock is taken in the phase the graph's edges give
/// it, so none is ever contended: a contended lock is a broken invariant, and panics.
struct Shared {
    plan: Arc<Plan>,
    /// Held alone by a sync point, shared by every other node.
    world: RwLock<Option<World>>,
    /// The sync points' own state.
    meta: Mutex<Option<RunMeta>>,
    /// Pointers into the world for the stage that is running.
    view: RwLock<StageView>,
    /// One per system of the plan, by position; an exclusive system's is unused.
    systems: Box<[RwLock<SystemWork>]>,
}

/// `lock`, held alone: by a sync point, or by a system node in `units` or `finalize`.
fn alone<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    match lock.try_write() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => panic!("a node of the run outlived its phase"),
    }
}

/// `lock`, shared: by the nodes of a stage.
fn beside<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    match lock.try_read() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => panic!("a node of the run ran before its phase"),
    }
}

/// `lock`, which one node at a time takes.
fn locked<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => panic!("two nodes of the run took one slot"),
    }
}

/// Every job of a run starts in the floating-point state every worker starts in
/// (architecture section 5.1), as the physics jobs assert theirs.
fn assert_fp_state() {
    debug_assert!(
        lactose_core::thread_class::fp::is_deterministic(),
        "a schedule job began without the deterministic floating-point state"
    );
}

impl RunGraph {
    /// The graph of `plan`: a node per sync point, each after the one before; a node per
    /// system that is not exclusive, after its stage's sync point, before the next, and
    /// after its ancestors; and per stage a node per event type the stage both emits and
    /// reads, after its emitters and before its readers.
    pub(crate) fn new(plan: Arc<Plan>) -> Self {
        let systems =
            plan.order.iter().map(|system| RwLock::new(SystemWork::new(system))).collect();
        let shared = Arc::new(Shared {
            plan: plan.clone(),
            world: RwLock::new(None),
            meta: Mutex::new(None),
            view: RwLock::new(StageView::default()),
            systems,
        });
        let exclusive = |index: usize| plan.kinds[index] == SystemKind::Exclusive;
        let tables = |index: usize| {
            let system = plan.order[index];
            system.access().iter().chain(system.finalize_access())
        };
        let mut builder = GraphBuilder::new();
        let mut merged: Vec<Vec<TypeId>> = Vec::with_capacity(plan.stages.len());
        let mut nodes: Vec<Option<NodeId>> = vec![None; plan.order.len()];
        let mut stage_nodes: Vec<Vec<NodeId>> = Vec::with_capacity(plan.stages.len());
        for stage in &plan.stages {
            let mut ids = Vec::new();
            for index in stage.systems.clone().filter(|&index| !exclusive(index)) {
                let node = builder.add_node(Arc::new(SystemNode { shared: shared.clone(), index }));
                for &ancestor in &plan.ancestors[index] {
                    if let Some(before) = nodes[ancestor] {
                        builder.add_dependency(before, node);
                    }
                }
                nodes[index] = Some(node);
                ids.push(node);
            }
            // Every emitter of a type is ordered before every reader (`check_events`), so
            // the merge sits between them; a type no system of the stage reads is merged at
            // the next sync point.
            let mut types: BTreeMap<TypeId, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
            for index in stage.systems.clone().filter(|&index| !exclusive(index)) {
                for access in tables(index) {
                    let AccessTarget::Events { type_id, .. } = access.target() else { continue };
                    let (emitters, readers) = types.entry(type_id).or_default();
                    let list = match access.kind() {
                        AccessKind::EmitEvents => emitters,
                        AccessKind::ReadEvents => readers,
                        _ => continue,
                    };
                    if list.last() != Some(&index) {
                        list.push(index);
                    }
                }
            }
            let mut stage_merged = Vec::new();
            for (type_id, (emitters, readers)) in types {
                if emitters.is_empty() || readers.is_empty() {
                    continue;
                }
                let node = Arc::new(EventsNode { shared: shared.clone(), type_id, emitters });
                let merge = builder.add_node(node.clone());
                for &emitter in &node.emitters {
                    builder.add_dependency(nodes[emitter].expect("a node per system"), merge);
                }
                for reader in readers {
                    builder.add_dependency(merge, nodes[reader].expect("a node per system"));
                }
                stage_merged.push(type_id);
                ids.push(merge);
            }
            merged.push(stage_merged);
            stage_nodes.push(ids);
        }
        let mut syncs = Vec::with_capacity(plan.stages.len() + 1);
        let needs =
            merged.iter().enumerate().map(|(stage, merged)| StageNeeds::of(&plan, stage, merged));
        // The closing sync point opens no stage.
        for (stage, needs) in needs.chain([StageNeeds::default()]).enumerate() {
            let node = SyncNode { shared: shared.clone(), stage, needs };
            syncs.push(builder.add_node(Arc::new(node)));
        }
        for (stage, ids) in stage_nodes.iter().enumerate() {
            builder.add_dependency(syncs[stage], syncs[stage + 1]);
            for &node in ids {
                builder.add_dependency(syncs[stage], node);
                builder.add_dependency(node, syncs[stage + 1]);
            }
        }
        let graph = builder.build().expect("a plan's order has no cycle");
        Self { graph, shared }
    }

    /// Runs the graph over `state` on `executor` and hands both back, or fails with
    /// `Cancelled` if the executor shut down first; the world then drops with the graph.
    #[expect(
        clippy::disallowed_methods,
        reason = "execution only (architecture section 8.2): the one place a schedule becomes \
                  a graph run; every result is order-independent by construction"
    )]
    pub(crate) async fn run(
        self,
        executor: &Executor,
        state: RunState,
    ) -> Result<(Self, RunState)> {
        let Self { graph, shared } = self;
        *alone(&shared.world) = Some(state.world);
        *locked(&shared.meta) = Some(state.meta);
        let graph = executor.run_graph(graph).await?;
        // The run has resolved: no job of it is executing or ever will.
        let world = alone(&shared.world).take().expect("a finished run hands the world back");
        let meta = locked(&shared.meta).take().expect("a finished run hands its state back");
        Ok((Self { graph, shared }, RunState { world, meta }))
    }
}

/// What a stage's view holds: the resources and event lists its systems name, and the
/// chunks if one of them is per-entity.
#[derive(Default)]
struct StageNeeds {
    /// Ascending.
    resources: Vec<TypeId>,
    /// Ascending: every type a system of the stage reads, and every type merged in it.
    events: Vec<TypeId>,
    entities: bool,
}

impl StageNeeds {
    fn of(plan: &Plan, stage: usize, merged: &[TypeId]) -> Self {
        let mut needs = Self { events: merged.to_vec(), ..Self::default() };
        for index in plan.stages[stage].systems.clone() {
            let system = plan.order[index];
            match plan.kinds[index] {
                SystemKind::Exclusive => continue,
                SystemKind::PerEntity => needs.entities = true,
                SystemKind::PerRegion | SystemKind::Once => {}
            }
            needs.resources.extend(resource_ids(system));
            for access in system.access().iter().chain(system.finalize_access()) {
                if let (AccessKind::ReadEvents, AccessTarget::Events { type_id, .. }) =
                    (access.kind(), access.target())
                {
                    needs.events.push(type_id);
                }
            }
        }
        for ids in [&mut needs.resources, &mut needs.events] {
            ids.sort();
            ids.dedup();
        }
        needs
    }
}

/// Pointers into the world for one stage, taken by its opening sync point with the world
/// held alone and dropped by the next before it borrows the world again. Each is taken once
/// from its allocation, through `Vec::as_mut_ptr` or a `&mut` of a boxed value, and every
/// slice a node reaches through it covers exactly what the node's phase and the schedule's
/// conflict sets give it (`SAFETY.md`, "The run as a graph").
#[derive(Default)]
struct StageView {
    archetypes: Vec<ArchetypeView>,
    /// Ascending by `TypeId`.
    resources: Vec<(TypeId, *mut (dyn Any + Send + Sync))>,
    /// Ascending by `TypeId`.
    events: Vec<(TypeId, *mut EventList)>,
}

// SAFETY: the view holds pointers into the world the run's shared state owns, and reaches
// what they point at only through its `unsafe` methods, whose callers keep to the phases
// `SAFETY.md` sets out. Moving or sharing the pointers themselves touches nothing.
unsafe impl Send for StageView {}
// SAFETY: as for `Send`.
unsafe impl Sync for StageView {}

#[derive(Default)]
struct ArchetypeView {
    columns: usize,
    chunks: Vec<ChunkView>,
    /// The row ticks and positions of each tracked column, at `chunk * columns + column`.
    stamps: Vec<Option<(*mut Tick, *mut u32)>>,
}

struct ChunkView {
    slab: *mut u8,
    slab_len: usize,
    rows: usize,
    column_ticks: *mut Stamp,
}

impl StageView {
    /// Drops every pointer, before the world is borrowed again.
    fn clear(&mut self) {
        for archetype in &mut self.archetypes {
            archetype.chunks.clear();
            archetype.stamps.clear();
        }
        self.resources.clear();
        self.events.clear();
    }

    /// Takes the pointers a stage with `needs` reaches the world through. An event type the
    /// stage reads or merges gets its list now, empty if no system has emitted it yet, so
    /// no node inserts into the store while others hold lists of it.
    fn take(&mut self, world: &mut World, needs: &StageNeeds) {
        debug_assert!(self.resources.is_empty() && self.events.is_empty());
        for &type_id in &needs.events {
            world.events.get_or_insert_with(type_id, EventList::default);
        }
        for &type_id in &needs.events {
            let list: *mut EventList = world.events.get_mut(&type_id).expect("inserted above");
            self.events.push((type_id, list));
        }
        for &type_id in &needs.resources {
            // A resource an exclusive system removed is missing; its readers are skipped.
            if let Some(entry) = world.resources.get_mut(&type_id) {
                let value: *mut (dyn Any + Send + Sync) = &mut *entry.value;
                self.resources.push((type_id, value));
            }
        }
        if !needs.entities {
            return;
        }
        let archetypes = &mut world.storage.archetypes;
        self.archetypes.resize_with(archetypes.len(), ArchetypeView::default);
        for (view, archetype) in self.archetypes.iter_mut().zip(archetypes) {
            view.columns = archetype.columns.len();
            for chunk in &mut archetype.chunks {
                let (slab, slab_len) = chunk.slab.as_mut_ptr();
                view.chunks.push(ChunkView {
                    slab,
                    slab_len,
                    rows: chunk.len(),
                    column_ticks: chunk.column_ticks.as_mut_ptr(),
                });
                view.stamps.extend(chunk.stamps.iter_mut().map(|stamps| {
                    stamps
                        .as_mut()
                        .map(|stamps| (stamps.ticks.as_mut_ptr(), stamps.positions.as_mut_ptr()))
                }));
            }
        }
    }

    fn chunk(&self, archetype: usize, chunk: usize) -> &ChunkView {
        &self.archetypes[archetype].chunks[chunk]
    }

    /// The tick of `column` in a chunk.
    ///
    /// # Safety
    ///
    /// No other node writes that tick meanwhile: the caller filters on the column or writes
    /// it, so every system that writes it is ordered with the caller.
    unsafe fn column_tick(&self, archetype: usize, chunk: usize, column: usize) -> Stamp {
        assert!(column < self.archetypes[archetype].columns, "a column of the archetype");
        // SAFETY: the pointer is to the chunk's `column_ticks`, one per column, and the
        // caller's obligation excludes a concurrent write.
        unsafe { self.chunk(archetype, chunk).column_ticks.add(column).read() }
    }

    /// Writes the tick of `column` in a chunk.
    ///
    /// # Safety
    ///
    /// No other node reads or writes that tick meanwhile: the caller writes the column, so
    /// every system that writes it or filters on it is ordered with the caller.
    unsafe fn set_column_tick(&self, archetype: usize, chunk: usize, column: usize, stamp: Stamp) {
        assert!(column < self.archetypes[archetype].columns, "a column of the archetype");
        // SAFETY: as for `column_tick`, for a write; no reference covers the element.
        unsafe { self.chunk(archetype, chunk).column_ticks.add(column).write(stamp) }
    }

    /// The row stamps of tracked column `column` in a chunk, if it is tracked.
    fn stamps(
        &self,
        archetype: usize,
        chunk: usize,
        column: usize,
    ) -> Option<(*mut Tick, *mut u32)> {
        let view = &self.archetypes[archetype];
        assert!(column < view.columns, "a column of the archetype");
        view.stamps[chunk * view.columns + column]
    }

    /// The stamp of `row` of tracked column `column` in a chunk.
    ///
    /// # Safety
    ///
    /// No other node writes that row's stamp meanwhile: the caller filters on the column,
    /// so every system that writes it is ordered with the caller.
    unsafe fn row_stamp(&self, archetype: usize, chunk: usize, column: usize, row: usize) -> Stamp {
        assert!(row < self.chunk(archetype, chunk).rows, "a live row of the chunk");
        let (ticks, positions) =
            self.stamps(archetype, chunk, column).expect("a `Changed` column is tracked");
        // SAFETY: a tracked column has a stamp per row of the chunk's capacity, and `row` is
        // live; the caller's obligation excludes a concurrent write.
        unsafe { Stamp::new(ticks.add(row).read(), positions.add(row).read()) }
    }

    /// Bytes `range` of a chunk's slab.
    ///
    /// # Safety
    ///
    /// No other node writes those bytes while the slice lives: they belong to a column the
    /// caller reads, which every system that writes it is ordered with.
    unsafe fn bytes(&self, archetype: usize, chunk: usize, range: Range<usize>) -> &[u8] {
        let view = self.chunk(archetype, chunk);
        assert!(range.start <= range.end && range.end <= view.slab_len, "bytes of the slab");
        // SAFETY: the range is inside the slab, which lives until the next sync point, and
        // the caller's obligation excludes a concurrent write.
        unsafe { core::slice::from_raw_parts(view.slab.add(range.start), range.len()) }
    }

    /// Bytes `range` of a chunk's slab, for writing.
    ///
    /// # Safety
    ///
    /// No other node reads or writes those bytes while the slice lives: they are a range of
    /// rows of a column the caller writes, which every system that touches it is ordered
    /// with, and no other unit of the caller's covers those rows.
    #[expect(clippy::mut_from_ref, reason = "the obligation makes the slice the only one")]
    unsafe fn bytes_mut(&self, archetype: usize, chunk: usize, range: Range<usize>) -> &mut [u8] {
        let view = self.chunk(archetype, chunk);
        assert!(range.start <= range.end && range.end <= view.slab_len, "bytes of the slab");
        // SAFETY: as for `bytes`, and the obligation excludes every other access.
        unsafe { core::slice::from_raw_parts_mut(view.slab.add(range.start), range.len()) }
    }

    /// The row stamps of `rows` of column `column` in a chunk, for writing, if it is tracked.
    ///
    /// # Safety
    ///
    /// As for [`bytes_mut`](Self::bytes_mut), for the stamps of those rows.
    #[expect(clippy::mut_from_ref, reason = "the obligation makes the slices the only ones")]
    unsafe fn stamps_mut(
        &self,
        archetype: usize,
        chunk: usize,
        column: usize,
        rows: Range<usize>,
    ) -> Option<(&mut [Tick], &mut [u32])> {
        assert!(rows.end <= self.chunk(archetype, chunk).rows, "live rows of the chunk");
        let (ticks, positions) = self.stamps(archetype, chunk, column)?;
        // SAFETY: a tracked column has a stamp per row of the chunk's capacity, and the
        // rows are live; the caller's obligation excludes every other access.
        unsafe {
            Some((
                core::slice::from_raw_parts_mut(ticks.add(rows.start), rows.len()),
                core::slice::from_raw_parts_mut(positions.add(rows.start), rows.len()),
            ))
        }
    }

    fn resource_ptr(&self, type_id: TypeId) -> *mut (dyn Any + Send + Sync) {
        let at = self.resources.binary_search_by_key(&type_id, |(id, _)| *id);
        self.resources[at.expect("a resource of a system that is not skipped")].1
    }

    /// World resource `type_id`, read.
    ///
    /// # Safety
    ///
    /// No node writes it while the reference lives: the caller reads it, so every system
    /// that writes it is ordered with the caller.
    unsafe fn resource(&self, type_id: TypeId) -> &(dyn Any + Send + Sync) {
        // SAFETY: the pointer is to the boxed value, which no sync point moves or drops
        // until the view is cleared; the caller's obligation excludes a concurrent write.
        unsafe { &*self.resource_ptr(type_id) }
    }

    /// World resource `type_id`, written.
    ///
    /// # Safety
    ///
    /// No other node reads or writes it while the reference lives: the caller writes it, so
    /// every system that touches it is ordered with the caller, and the caller holds one
    /// reference to it at a time.
    #[expect(clippy::mut_from_ref, reason = "the obligation makes the reference the only one")]
    unsafe fn resource_mut(&self, type_id: TypeId) -> &mut (dyn Any + Send + Sync) {
        // SAFETY: as for `resource`; the obligation excludes every other access.
        unsafe { &mut *self.resource_ptr(type_id) }
    }

    fn list_ptr(&self, type_id: TypeId) -> *mut EventList {
        let at = self.events.binary_search_by_key(&type_id, |(id, _)| *id);
        self.events[at.expect("a list the stage reads or merges")].1
    }

    /// The list of event type `type_id`, read.
    ///
    /// # Safety
    ///
    /// No node writes it while the reference lives: the caller reads the type, so the node
    /// that merges it this stage, if any, has finalized before the caller runs.
    unsafe fn list(&self, type_id: TypeId) -> &EventList {
        // SAFETY: the pointer is to the list's slot in the world's store, which nothing
        // inserts into or removes from until the view is cleared; the caller's obligation
        // excludes a concurrent write.
        unsafe { &*self.list_ptr(type_id) }
    }

    /// The list of event type `type_id`, written.
    ///
    /// # Safety
    ///
    /// No other node reads or writes it while the reference lives: the caller is the node
    /// that merges the type this stage, which every reader of the type in the stage follows.
    #[expect(clippy::mut_from_ref, reason = "the obligation makes the reference the only one")]
    unsafe fn list_mut(&self, type_id: TypeId) -> &mut EventList {
        // SAFETY: as for `list`; the obligation excludes every other access.
        unsafe { &mut *self.list_ptr(type_id) }
    }
}

/// The world resources `table` reads, lent shared, in `arena`.
///
/// # Safety
///
/// As for [`StageView::resource`], for each: the caller is the node of the system whose
/// table this is, not skipped, and writes none of them in this invocation.
unsafe fn read_resources<'v>(
    arena: &'v Arena,
    view: &'v StageView,
    table: &[Access],
) -> &'v [(TypeId, &'v (dyn Any + Send + Sync))] {
    let resource = |access: &Access| match access.target() {
        AccessTarget::Resource { type_id, .. } => Some(type_id),
        _ => None,
    };
    let ids = || distinct(table, resource);
    let placeholder: &(dyn Any + Send + Sync) = &();
    // SAFETY: the caller's obligation.
    let resources = ids().map(|type_id| (type_id, unsafe { view.resource(type_id) }));
    collect_in(arena, ids().count(), (TypeId::of::<()>(), placeholder), resources)
}

/// The world resources `table` names, each written by its one parameter or read by any.
///
/// # Safety
///
/// As for [`StageView::resource`] and [`StageView::resource_mut`], for each: the caller is
/// the node of the system whose table this is, not skipped, which the schedule orders with
/// every other system on a resource the table writes.
unsafe fn lend_resources<'v>(view: &'v StageView, table: &[Access]) -> Vec<(TypeId, Handle<'v>)> {
    let mut handles: Vec<(TypeId, Handle<'v>)> = Vec::new();
    for access in table {
        let AccessTarget::Resource { type_id, .. } = access.target() else { continue };
        if handles.iter().any(|(id, _)| *id == type_id) {
            // Read twice: a resource the table writes has no other entry (the schedule's
            // `InvalidParameter`).
            continue;
        }
        let handle = if access.kind() == AccessKind::Write {
            // SAFETY: the caller's obligation, and this is the one reference to it.
            Handle::Mut(unsafe { view.resource_mut(type_id) })
        } else {
            // SAFETY: the caller's obligation.
            Handle::Shared(unsafe { view.resource(type_id) })
        };
        handles.push((type_id, handle));
    }
    handles
}

/// The lists of the event types `table` reads, in `arena`.
///
/// # Safety
///
/// As for [`StageView::list`], for each: the caller is the node of the system whose table
/// this is, which follows the merge of every type it reads.
unsafe fn read_lists<'v>(
    arena: &'v Arena,
    view: &'v StageView,
    table: &[Access],
) -> &'v [(TypeId, &'v EventList)] {
    let read = |access: &Access| match (access.kind(), access.target()) {
        (AccessKind::ReadEvents, AccessTarget::Events { type_id, .. }) => Some(type_id),
        _ => None,
    };
    let ids = || distinct(table, read);
    // SAFETY: the caller's obligation.
    let lists = ids().map(|type_id| (type_id, unsafe { view.list(type_id) }));
    collect_in(arena, ids().count(), (TypeId::of::<()>(), EventList::empty()), lists)
}

/// The `TypeId`s `of` finds in `table`, each once, in table order.
fn distinct(
    table: &[Access],
    of: impl Fn(&Access) -> Option<TypeId> + Copy,
) -> impl Iterator<Item = TypeId> {
    table.iter().enumerate().filter_map(move |(at, access)| {
        let type_id = of(access)?;
        (!table[..at].iter().any(|earlier| of(earlier) == Some(type_id))).then_some(type_id)
    })
}

/// One system's state across the phases of its node: written alone by its stage's sync
/// point and its own `units` and `finalize`, shared by its `run` jobs and the merges after
/// it.
struct SystemWork {
    /// The resources the system's tables name, ascending.
    resources: Vec<TypeId>,
    /// The window of the system's `Changed` filters and the stamp its writes carry.
    window: (Stamp, Stamp),
    /// Whether an exclusive system earlier in the run removed a resource it names.
    skipped: bool,
    /// What each archetype a per-entity system is issued over lends each parameter.
    lendings: Vec<Lending>,
    /// The chunks a per-entity system runs over, in the order the layout issued them.
    chunks: Vec<ChunkPlan>,
    /// The units of a per-entity system, in chunk order and by row within a chunk.
    units: Vec<UnitRows>,
    /// The selection of each chunk by the `Changed` filters, one bit per row.
    selection: Vec<u64>,
    chunks_skipped: usize,
    /// The regions a per-region system runs, ascending.
    regions: Vec<RegionId>,
    /// One slot per unit, sized at `units`, filled by the unit's job and emptied by
    /// `finalize`.
    results: Vec<Mutex<Option<UnitResult>>>,
    /// The events the system emitted, per type it emits, ascending by `TypeId`.
    events: Vec<(TypeId, Mutex<Vec<EventRecord>>)>,
    /// The commands and presentation events the system made, for the next sync point.
    handed: Mutex<Handed>,
}

#[derive(Default)]
struct Handed {
    pending: Vec<Pending>,
    emissions: Vec<PendingEmission>,
}

/// What one unit's job made.
struct UnitResult {
    outputs: Vec<Output>,
    partial: Option<Box<dyn Any + Send>>,
    rows: usize,
    invocations: usize,
}

/// What one archetype's chunks lend each parameter of a per-entity system.
struct Lending {
    archetype: usize,
    params: Vec<Lend>,
    /// The byte offset and size of the region column, and of the claim column if any.
    region: (usize, usize),
    claims: Option<(usize, usize)>,
}

/// What a parameter is lent from a chunk: a column's offset in the slab and row size.
#[derive(Clone, Copy)]
enum Lend {
    /// Not a component parameter, or a filter.
    None,
    /// An optional component the archetype lacks.
    Absent,
    Read {
        offset: usize,
        size: usize,
    },
    Write {
        column: usize,
        offset: usize,
        size: usize,
    },
}

struct ChunkPlan {
    lending: usize,
    chunk: usize,
    /// Where the chunk's selection starts in `SystemWork::selection`; `None` without a
    /// `Changed` filter.
    selected: Option<usize>,
}

struct UnitRows {
    /// The unit's chunk, in `SystemWork::chunks`.
    chunk: usize,
    rows: Range<usize>,
}

impl SystemWork {
    fn new(system: &SystemDescriptor) -> Self {
        let mut emitted: Vec<TypeId> = system
            .access()
            .iter()
            .chain(system.finalize_access())
            .filter_map(|access| match (access.kind(), access.target()) {
                (AccessKind::EmitEvents, AccessTarget::Events { type_id, .. }) => Some(type_id),
                _ => None,
            })
            .collect();
        emitted.sort();
        emitted.dedup();
        Self {
            resources: resource_ids(system),
            window: (Stamp::default(), Stamp::default()),
            skipped: false,
            lendings: Vec::new(),
            chunks: Vec::new(),
            units: Vec::new(),
            selection: Vec::new(),
            chunks_skipped: 0,
            regions: Vec::new(),
            results: Vec::new(),
            events: emitted.into_iter().map(|id| (id, Mutex::new(Vec::new()))).collect(),
            handed: Mutex::new(Handed::default()),
        }
    }
}

impl Lending {
    fn new(archetype: usize, arch: &Archetype, table: &[Access]) -> Self {
        let column_of = |access: &Access| match access.target() {
            AccessTarget::Component(key) => arch.column_index(key),
            _ => None,
        };
        let params: Vec<Lend> = table
            .iter()
            .map(|access| {
                let kind = access.kind();
                let column = match kind {
                    AccessKind::Read
                    | AccessKind::Write
                    | AccessKind::OptionalRead
                    | AccessKind::OptionalWrite => column_of(access),
                    _ => None,
                };
                match (kind, column) {
                    (AccessKind::Write | AccessKind::OptionalWrite, Some(column)) => {
                        let ColumnSpan { offset, size } = ColumnSpan::of(arch, column);
                        Lend::Write { column, offset, size }
                    }
                    (AccessKind::Read | AccessKind::OptionalRead, Some(column)) => {
                        let ColumnSpan { offset, size } = ColumnSpan::of(arch, column);
                        Lend::Read { offset, size }
                    }
                    (AccessKind::OptionalRead | AccessKind::OptionalWrite, None) => Lend::Absent,
                    _ => Lend::None,
                }
            })
            .collect();
        // A column a kernel writes is lent to its writer alone: the schedule refuses two
        // data parameters on one component and `Mut` of the region or claim column.
        let written = |column: usize| {
            params.iter().any(|lend| matches!(*lend, Lend::Write { column: c, .. } if c == column))
        };
        let region = arch.column_index(InRegion::KEY).expect("every row has a region");
        let claims = arch.column_index(ClaimedBy::KEY);
        assert!(
            !written(region) && claims.is_none_or(|column| !written(column)),
            "a column a kernel writes is lent to its writer alone"
        );
        let span = |column| {
            let ColumnSpan { offset, size } = ColumnSpan::of(arch, column);
            (offset, size)
        };
        Self { archetype, params, region: span(region), claims: claims.map(span) }
    }
}

struct ColumnSpan {
    offset: usize,
    size: usize,
}

impl ColumnSpan {
    fn of(arch: &Archetype, column: usize) -> Self {
        let info = &arch.columns[column];
        Self { offset: info.offset, size: info.size }
    }

    /// The bytes of `rows` of the column.
    fn rows(&self, rows: &Range<usize>) -> Range<usize> {
        self.offset + self.size * rows.start..self.offset + self.size * rows.end
    }
}

/// A sync point: the run's start, the point between two stages, or its end.
struct SyncNode {
    shared: Arc<Shared>,
    /// The stage the sync point opens; the stage count for the closing one.
    stage: usize,
    needs: StageNeeds,
}

impl GraphNode for SyncNode {
    fn name(&self) -> &'static str {
        "casein::sync"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _: Range<u32>, ctx: &mut JobContext<'_>) {
        assert_fp_state();
        let shared = &*self.shared;
        let plan = &*shared.plan;
        let mut world = alone(&shared.world);
        let world = world.as_mut().expect("a run holds the world");
        let mut meta = locked(&shared.meta);
        let meta = meta.as_mut().expect("a run holds its state");
        let mut view = alone(&shared.view);
        // No pointer of the stage before outlives it: the world is borrowed below.
        view.clear();
        let (stage, tick) = (self.stage, meta.tick);
        let closing = stage == plan.stages.len();
        if stage == 0 {
            world.in_run = true;
            if tick {
                world.advance_tick();
            }
            meta.slot = 0;
        } else {
            take_handed(shared, stage - 1, world, meta);
        }
        let stamp = enter_slot(world, tick, &mut meta.slot);
        {
            let name = if closing { "closing" } else { plan.stages[stage].name };
            let _span =
                tracing::trace_span!(target: "casein", "sync_point", stage = name).entered();
            if tick && stage == 0 {
                world.input_sync(stamp);
            } else {
                world.apply(core::mem::take(&mut meta.pending), stamp);
            }
            if closing {
                // No channel survives its run (D126): a taker's channel is already empty,
                // and what a channel with readers alone or a skipped taker's holds goes
                // here, so no snapshot point, save or pass between runs sees an entry.
                world.channels.clear();
            }
            world.permute_rows();
            if closing {
                world.end_present(core::mem::take(&mut meta.emissions), !tick);
            }
        }
        if closing {
            if tick {
                world.point = BETWEEN;
            }
            world.point_cap = u32::MAX;
            world.in_run = false;
            return;
        }
        // Every system's slot is entered here, in plan order, so each carries the stamp
        // its position gives it whatever order the stage's nodes then run in.
        for index in plan.stages[stage].systems.clone() {
            let now = enter_slot(world, tick, &mut meta.slot);
            // The window opens at this system's position one tick back, across the wrap;
            // a pass's, where it last ran in this epoch, or else before every stored stamp.
            let since = if tick {
                Stamp::new(Tick(now.tick.0.wrapping_sub(1)), now.position)
            } else {
                meta.pass_marks[index]
                    .filter(|&(epoch, _)| epoch == world.epoch)
                    .map_or(first_pass_window(now.tick), |(_, stamp)| stamp)
            };
            if plan.kinds[index] == SystemKind::Exclusive {
                run_exclusive_system(
                    world,
                    plan,
                    index,
                    ctx,
                    &mut meta.pending,
                    &mut meta.emissions,
                );
            } else {
                let mut work = alone(&shared.systems[index]);
                work.window = (since, now);
                // `prepare` found every resource; an exclusive system earlier in the run
                // removed one. The system cannot run, and the rest of the run goes on.
                work.skipped = !work.resources.iter().all(|id| world.resources.contains_key(id));
                if work.skipped {
                    report_skipped(plan.order[index]);
                }
            }
            if !tick {
                meta.pass_marks[index] = Some((world.epoch, now));
            }
        }
        view.take(world, &self.needs);
    }
}

/// Hands on what the systems of `stage` left: the events of every type no system of the
/// stage read, in plan order, to the world; commands and presentation events to the run.
fn take_handed(shared: &Shared, stage: usize, world: &mut World, meta: &mut RunMeta) {
    let plan = &*shared.plan;
    let mut events = Vec::new();
    for index in plan.stages[stage].systems.clone() {
        if plan.kinds[index] == SystemKind::Exclusive {
            continue;
        }
        let mut work = alone(&shared.systems[index]);
        let work = &mut *work;
        for (type_id, records) in &mut work.events {
            let records = records.get_mut().unwrap_or_else(PoisonError::into_inner);
            events.extend(records.drain(..).map(|record| (*type_id, record)));
        }
        let handed = work.handed.get_mut().unwrap_or_else(PoisonError::into_inner);
        meta.pending.append(&mut handed.pending);
        meta.emissions.append(&mut handed.emissions);
    }
    push_events(world, events);
}

/// A system that is not exclusive.
struct SystemNode {
    shared: Arc<Shared>,
    /// The system's position in the plan.
    index: usize,
}

impl SystemNode {
    fn system(&self) -> &'static SystemDescriptor {
        self.shared.plan.order[self.index]
    }

    fn span(&self) -> tracing::span::EnteredSpan {
        tracing::trace_span!(target: "casein", "system", name = self.system().name()).entered()
    }
}

impl GraphNode for SystemNode {
    fn name(&self) -> &'static str {
        self.system().name()
    }

    fn units(&self) -> u32 {
        assert_fp_state();
        let _span = self.span();
        let shared = &*self.shared;
        let world = beside(&shared.world);
        let world = world.as_ref().expect("a run holds the world");
        let view = beside(&shared.view);
        let mut work = alone(&shared.systems[self.index]);
        let work = &mut *work;
        let system = self.system();
        let units = if work.skipped {
            0
        } else {
            match shared.plan.kinds[self.index] {
                SystemKind::PerEntity => {
                    plan_entities(world, &view, system, work);
                    work.units.len()
                }
                SystemKind::PerRegion => {
                    work.regions.clear();
                    work.regions
                        .extend(admitted(&world.regions, system.access()).map(|(id, _)| id));
                    work.regions.len()
                }
                SystemKind::Once => 1,
                SystemKind::Exclusive => unreachable!("an exclusive system runs at a sync point"),
            }
        };
        work.results.clear();
        work.results.resize_with(units, || Mutex::new(None));
        u32::try_from(units).expect("fewer than 2^32 units")
    }

    fn run(&self, units: Range<u32>, ctx: &mut JobContext<'_>) {
        assert_fp_state();
        // No span here: the executor calls `run` once per unit, and opens its own span per
        // stolen range, named after the system (architecture 5.1: never per chunk job).
        let shared = &*self.shared;
        let world = beside(&shared.world);
        let world = world.as_ref().expect("a run holds the world");
        let view = beside(&shared.view);
        let work = beside(&shared.systems[self.index]);
        let system = self.system();
        let units = units.start as usize..units.end as usize;
        match shared.plan.kinds[self.index] {
            SystemKind::PerEntity => {
                run_entities(ctx.scratch(), world, &view, &work, system, units)
            }
            SystemKind::PerRegion => run_regions(ctx.scratch(), world, &view, &work, system, units),
            SystemKind::Once => run_once(world, &view, &work, system, ctx),
            SystemKind::Exclusive => unreachable!("an exclusive system runs at a sync point"),
        }
    }

    fn finalize(&self, ctx: &mut JobContext<'_>) {
        assert_fp_state();
        let _span = self.span();
        let shared = &*self.shared;
        let world = beside(&shared.world);
        let world = world.as_ref().expect("a run holds the world");
        let view = beside(&shared.view);
        let mut work = alone(&shared.systems[self.index]);
        let work = &mut *work;
        if work.skipped {
            return;
        }
        let system = self.system();
        let (count, mut rows, mut invocations) = (work.results.len(), 0, 0);
        let mut outputs = Vec::new();
        let mut partials: Vec<Box<dyn Any + Send>> = Vec::new();
        // In unit order, which the layout fixes, never the order the units finished in.
        for slot in work.results.drain(..) {
            let result = slot.into_inner().unwrap_or_else(PoisonError::into_inner);
            let result = result.expect("every unit runs before the finalize");
            rows += result.rows;
            invocations += result.invocations;
            outputs.extend(result.outputs);
            partials.extend(result.partial);
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
                // SAFETY: this node runs system `index`, which is not skipped, in its
                // finalize phase; the schedule orders it with every system that touches a
                // resource its finalize writes and with the merge of every type it reads.
                let (handles, lists) = unsafe {
                    (lend_resources(&view, table), read_lists(ctx.scratch(), &view, table))
                };
                let mut env = Env::new(
                    Resources::Taken(handles),
                    lists,
                    &world.channels,
                    &mut output,
                    Some(&mut *ctx),
                    None,
                );
                let slots =
                    table.iter().enumerate().map(|(p, access)| env.slot(p, access)).collect();
                let mut invocation = Invocation::new(slots, ReducerSlot::Total(Some(total)));
                (system.kernel().finalize)(&mut invocation);
            }
            output.close_channels();
            outputs.push(output);
        }
        if shared.plan.kinds[self.index] == SystemKind::PerEntity {
            tracing::trace!(
                target: "casein",
                system = system.name(),
                units = count,
                chunks_skipped = work.chunks_skipped,
                rows,
                invocations,
                "per_entity"
            );
        }
        let mut events = Vec::new();
        let handed = work.handed.get_mut().unwrap_or_else(PoisonError::into_inner);
        hand_on(
            world,
            system,
            self.index,
            outputs,
            &mut events,
            &mut handed.pending,
            &mut handed.emissions,
        );
        for (type_id, record) in events {
            let at = work.events.binary_search_by_key(&type_id, |(id, _)| *id);
            let buffer = &mut work.events[at.expect("a type the system emits")].1;
            buffer.get_mut().unwrap_or_else(PoisonError::into_inner).push(record);
        }
    }
}

/// Pass 1 of a per-entity system: the chunks it is issued over, in the layout's order; the
/// chunks its `Changed` filters skip; the column ticks of the columns it writes, marked on
/// every issued chunk before any unit runs; each chunk's selection by its filters; and the
/// units, a chunk each or under `split_batches` a random range of one chunk's rows, drawn
/// from (seed, tick, system, archetype, chunk) so no cut depends on another chunk or on the
/// order units run in. The units' jobs only read what this leaves.
fn plan_entities(
    world: &World,
    view: &StageView,
    system: &SystemDescriptor,
    work: &mut SystemWork,
) {
    let table = system.access();
    let (since, now) = work.window;
    let (tick, layout) = (world.tick, world.layout);
    let component = |access: &Access| match access.target() {
        AccessTarget::Component(key) => Some(key),
        _ => None,
    };
    let keys_of = |wanted: &[AccessKind]| -> Vec<lactose_core::ComponentKey> {
        table.iter().filter(|a| wanted.contains(&a.kind())).filter_map(component).collect()
    };
    let mut required =
        keys_of(&[AccessKind::Read, AccessKind::Write, AccessKind::With, AccessKind::Changed]);
    if world.kind == WorldKind::Prediction {
        // The prediction world's own filter: claimed entities only (architecture 9.4).
        required.push(ClaimedBy::KEY);
    }
    let excluded = keys_of(&[AccessKind::Without]);
    let changed = keys_of(&[AccessKind::Changed]);
    let written = keys_of(&[AccessKind::Write, AccessKind::OptionalWrite]);

    let archetypes = &world.storage.archetypes;
    let mut issued = Vec::new();
    for (archetype, arch) in archetypes.iter().enumerate() {
        if required.iter().all(|&key| arch.has(key)) && !excluded.iter().any(|&key| arch.has(key)) {
            issued.extend((0..arch.chunks.len()).map(|chunk| (archetype, chunk)));
        }
    }
    let issued = layout.order(issued, (tick.0.into(), system.key().0));

    work.lendings.clear();
    work.chunks.clear();
    work.units.clear();
    work.selection.clear();
    work.chunks_skipped = 0;
    let system_draw = LayoutRng::new(layout.seed, tick.0.into(), system.key().0).next();
    let mut lending_of: Vec<Option<usize>> = vec![None; archetypes.len()];
    let mut changed_columns: Vec<usize> = Vec::with_capacity(changed.len());
    for (archetype, chunk) in issued {
        let arch = &archetypes[archetype];
        changed_columns.clear();
        changed_columns.extend(changed.iter().filter_map(|&key| arch.column_index(key)));
        // A column tick is at or after every row stamp of its chunk, so a chunk whose tick
        // on a filtered column is not after the window holds no row the filter passes.
        let unchanged = changed_columns.iter().any(|&column| {
            // SAFETY: the system filters on the column.
            let column_tick = unsafe { view.column_tick(archetype, chunk, column) };
            !column_tick.after(since, now.tick)
        });
        // The dispatch of a writing system marks every chunk it is issued over, skipped or
        // not, before any unit runs.
        for column in written.iter().filter_map(|&key| arch.column_index(key)) {
            // SAFETY: the system writes the column.
            unsafe { view.set_column_tick(archetype, chunk, column, now) };
        }
        let len = arch.chunks[chunk].len();
        if unchanged {
            work.chunks_skipped += 1;
        }
        if unchanged || len == 0 {
            continue;
        }
        let selected = (!changed_columns.is_empty()).then(|| {
            let start = work.selection.len();
            work.selection.resize(start + len.div_ceil(64), 0);
            for row in 0..len {
                let in_window = changed_columns.iter().all(|&column| {
                    // SAFETY: the system filters on the column.
                    let stamp = unsafe { view.row_stamp(archetype, chunk, column, row) };
                    stamp.after(since, now.tick) && now.after(stamp, now.tick)
                });
                if in_window {
                    work.selection[start + row / 64] |= 1 << (row % 64);
                }
            }
            start
        });
        let lending = match lending_of[archetype] {
            Some(lending) => lending,
            None => {
                work.lendings.push(Lending::new(archetype, arch, table));
                lending_of[archetype] = Some(work.lendings.len() - 1);
                work.lendings.len() - 1
            }
        };
        let plan = work.chunks.len();
        work.chunks.push(ChunkPlan { lending, chunk, selected });
        if layout.split_batches {
            let mut batches = LayoutRng::new(system_draw, archetype as u64, chunk as u64);
            let mut start = 0;
            for row in 1..len {
                if batches.below(3) == 0 {
                    work.units.push(UnitRows { chunk: plan, rows: start..row });
                    start = row;
                }
            }
            work.units.push(UnitRows { chunk: plan, rows: start..len });
        } else {
            work.units.push(UnitRows { chunk: plan, rows: 0..len });
        }
    }
}

/// Runs `units` of a per-entity system, each from the plan `units()` left: a column a
/// parameter writes lent as that unit's rows alone, any other column shared.
fn run_entities(
    arena: &Arena,
    world: &World,
    view: &StageView,
    work: &SystemWork,
    system: &SystemDescriptor,
    units: Range<usize>,
) {
    let table = system.access();
    // SAFETY: this node runs the system, not skipped, in its run phase; a per-entity kernel
    // writes no resource, and the system follows the merge of every type it reads.
    let (resources, events) =
        unsafe { (read_resources(arena, view, table), read_lists(arena, view, table)) };
    let env = UnitEnv {
        table,
        system: system.key(),
        tick: world.tick,
        now: work.window.1,
        resources,
        events,
        channels: &world.channels,
        regions: admitted_in(arena, &world.regions, table),
        sinks: Sinks::of(table),
    };
    for unit in units {
        let UnitRows { chunk: plan, rows } = &work.units[unit];
        let plan = &work.chunks[*plan];
        let lending = &work.lendings[plan.lending];
        let (archetype, chunk) = (lending.archetype, plan.chunk);
        let keys = &world.storage.archetypes[archetype].chunks[chunk].keys[rows.clone()];
        let span = |offset, size| ColumnSpan { offset, size }.rows(rows);
        // SAFETY (each block below): the bytes are `rows` of a column this system's table
        // reads, or writes through the one parameter lent it; the schedule orders every
        // system that writes a column this one reads, or touches a column this one writes,
        // with it, and no other unit of this system covers `rows` of the chunk.
        let columns = lending
            .params
            .iter()
            .map(|lend| match *lend {
                Lend::None => Lent::None,
                Lend::Absent => Lent::Absent,
                Lend::Read { offset, size } => {
                    // SAFETY: see above.
                    Lent::Read(unsafe { view.bytes(archetype, chunk, span(offset, size)) })
                }
                Lend::Write { column, offset, size } => {
                    // SAFETY: see above.
                    let bytes = unsafe { view.bytes_mut(archetype, chunk, span(offset, size)) };
                    // SAFETY: see above, for the column's row stamps.
                    let stamps = unsafe { view.stamps_mut(archetype, chunk, column, rows.clone()) };
                    Lent::Write(bytes, stamps)
                }
            })
            .collect();
        let (offset, size) = lending.region;
        // SAFETY: see above; no kernel writes the region column.
        let regions = unsafe { view.bytes(archetype, chunk, span(offset, size)) };
        let claims = lending.claims.map(|(offset, size)| {
            // SAFETY: see above; no kernel writes the claim column.
            let bytes = unsafe { view.bytes(archetype, chunk, span(offset, size)) };
            typed_slice::<ClaimedBy>(bytes, rows.len())
        });
        let mut rows_of = Unit::new(
            &env,
            keys,
            typed_slice::<InRegion>(regions, rows.len()),
            claims,
            plan.selected.map(|start| (&work.selection[start..], rows.start)),
            columns,
        );
        (system.kernel().each)(&mut rows_of);
        let result = UnitResult {
            rows: rows_of.len(),
            invocations: rows_of.invocations,
            outputs: rows_of.outputs,
            partial: rows_of.partial,
        };
        let previous = locked(&work.results[unit]).replace(result);
        assert!(previous.is_none(), "a unit runs once in a run");
    }
}

/// Runs `units` of a per-region system, one region each.
fn run_regions(
    arena: &Arena,
    world: &World,
    view: &StageView,
    work: &SystemWork,
    system: &SystemDescriptor,
    units: Range<usize>,
) {
    let table = system.access();
    // SAFETY: this node runs the system, not skipped, in its run phase; a per-region kernel
    // writes no resource, and the system follows the merge of every type it reads.
    let (resources, lists) =
        unsafe { (read_resources(arena, view, table), read_lists(arena, view, table)) };
    let sinks = Sinks::of(table);
    for unit in units {
        let region = work.regions[unit];
        let state = &world.regions[&region];
        let mut output = Output::new(Source::Region(region), None, Some(region), sinks);
        let mut partial = system.kernel().reducer.map(|reducer| (reducer.identity)());
        {
            let mut env = Env::new(
                Resources::Shared(resources),
                lists,
                &world.channels,
                &mut output,
                None,
                None,
            );
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
        let outputs = if output.is_empty() { Vec::new() } else { vec![output] };
        let result = UnitResult { outputs, partial, rows: 0, invocations: 1 };
        let previous = locked(&work.results[unit]).replace(result);
        assert!(previous.is_none(), "a unit runs once in a run");
    }
}

/// Runs a once-per-tick system, its one unit.
fn run_once(
    world: &World,
    view: &StageView,
    work: &SystemWork,
    system: &SystemDescriptor,
    ctx: &mut JobContext<'_>,
) {
    let table = system.access();
    let mut output = Output::new(Source::Tick, None, None, Sinks::of(table));
    let mut partial = system.kernel().reducer.map(|reducer| (reducer.identity)());
    {
        // SAFETY: this node runs the system, not skipped, in its run phase; the schedule
        // orders it with every system that touches a resource it writes and with the merge
        // of every type it reads.
        let (handles, lists) =
            unsafe { (lend_resources(view, table), read_lists(ctx.scratch(), view, table)) };
        let mut env = Env::new(
            Resources::Taken(handles),
            lists,
            &world.channels,
            &mut output,
            Some(ctx),
            None,
        );
        let slots = table.iter().enumerate().map(|(p, access)| env.slot(p, access)).collect();
        let reducer =
            partial.as_mut().map_or(ReducerSlot::None, |p| ReducerSlot::Partial(&mut **p));
        let mut invocation = Invocation::new(slots, reducer);
        (system.kernel().run)(&mut invocation);
    }
    output.close_channels();
    let result = UnitResult { outputs: vec![output], partial, rows: 0, invocations: 1 };
    let previous = locked(&work.results[0]).replace(result);
    assert!(previous.is_none(), "a unit runs once in a run");
}

/// Merges one event type's events from the stage's emitters, in plan order, into the
/// world's list, before any reader of the type in the stage runs.
struct EventsNode {
    shared: Arc<Shared>,
    type_id: TypeId,
    /// The emitters in the stage, ascending by plan position.
    emitters: Vec<usize>,
}

impl GraphNode for EventsNode {
    fn name(&self) -> &'static str {
        "casein::events"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _: Range<u32>, _: &mut JobContext<'_>) {
        assert_fp_state();
        let shared = &*self.shared;
        let view = beside(&shared.view);
        // SAFETY: this node merges the type in this stage: every reader of the type in the
        // stage follows it, and no other node writes the list.
        let list = unsafe { view.list_mut(self.type_id) };
        for &emitter in &self.emitters {
            let work = beside(&shared.systems[emitter]);
            let at = work.events.binary_search_by_key(&self.type_id, |(id, _)| *id);
            let mut records = locked(&work.events[at.expect("a type the emitter emits")].1);
            for record in records.drain(..) {
                list.push(record);
            }
        }
        list.index();
    }
}
