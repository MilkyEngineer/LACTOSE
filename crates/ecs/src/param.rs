//! System parameters: the types a `#[system]` function takes, each declaring the access it
//! needs (architecture section 6.1).

use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use lactose_asset_api::PinnedTable;
use lactose_core::{StableKey, SystemKey, Tick};
use lactose_reflect::{Component, Resource};
use lactose_task::JobContext;

use crate::access::{
    Access, AccessKind, component_target, no_target, region_resource_target, resource_target,
};
use crate::change::Stamp;
use crate::region::RegionId;
use crate::rng::Rng;
use crate::storage::{RowView, typed, typed_mut};
use crate::system::{Fetch, Invocation, Slot, mismatch};
use crate::world::World;

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A type a `#[system]` function may take as a parameter.
///
/// The set is closed: it is exactly the parameter types this crate exports, and
/// [`ACCESS`](Self::ACCESS) is what the `#[system]` expansion records for each. The kinds:
///
/// | Parameter | Allowed in | Access |
/// |---|---|---|
/// | `&T`, [`Mut<T>`], `Option<&T>`, `Option<Mut<T>>` | per-entity | the component |
/// | [`With<T>`], [`Without<T>`], [`Changed<T>`] | per-entity | a filter on the component |
/// | [`StableKeyOf`] | per-entity | none |
/// | [`Ctx`], [`RegionRes<T>`] | per-entity, per-region | the region |
/// | [`Res<T>`] | every kind | the resource, read |
/// | [`ResMut<T>`] | once per tick, exclusive, finalize | the resource, written |
/// | [`Commands`](crate::Commands), [`EventWriter<E>`](crate::EventWriter), [`PresentEvents`](crate::PresentEvents) | every kind | output buffers |
/// | [`Events<E>`](crate::Events), [`Channel<C>`](crate::Channel) | every kind | input buffers |
/// | [`Job`] | once per tick, exclusive, finalize | the executor's job context |
/// | `&mut World` | exclusive | everything |
///
/// What a system is, from its parameters:
///
/// - **Per-entity**: it has a component parameter. It runs once per entity that has every
///   required component, lacks every excluded one, passes every `Changed` filter, resides in
///   a `Simulated` region and passes the world's own filter.
/// - **Per-region**: it has no component parameter but a [`Ctx`] or [`RegionRes`]. It runs
///   once per `Simulated` region, each run independent of the others.
/// - **Once per tick**: it has neither. It runs as one job.
/// - **Exclusive**: declared `exclusive`, it takes `&mut World` first and otherwise only
///   resource, buffer and [`Job`] parameters, which are taken out of the world for the call:
///   inside, the world does not hold them.
///
/// A system that runs more than once per tick (per-entity, per-region) may not write a world
/// resource or take [`Job`]; a system that is not per-entity may not take [`StableKeyOf`];
/// an exclusive system or a finalize function may not take [`Ctx`] or [`RegionRes`]; and
/// within one system two data parameters (`&T`, [`Mut<T>`], `Option<&T>`, `Option<Mut<T>>`)
/// may not name one component, nor may [`Without<T>`] stand beside any parameter on `T`.
/// The schedule rejects each ([`ScheduleError`](crate::ScheduleError)).
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a system parameter",
    note = "a system takes `&T`, `Mut<T>`, `Option<&T>`, `Option<Mut<T>>`, `With<T>`, `Without<T>` or `Changed<T>` of a component `T`, `Res`, `ResMut`, `RegionRes`, `Ctx`, `StableKeyOf`, `Job`, `Commands`, `EventWriter`, `Events`, `PresentEvents`, `Channel`, or `&mut World` when exclusive"
)]
pub trait SystemParam: sealed::Sealed + Sized {
    /// What this parameter reads or writes.
    const ACCESS: Access;
}

/// A world resource, read. `T` is a reflected [`Resource`] or the `dyn` trait type of a
/// service resource that `app` inserted (architecture section 4).
///
/// A service trait has `Send + Sync` supertraits and is inserted and read as the same
/// `dyn Trait`, since `dyn Trait` and `dyn Trait + Send + Sync` have different `TypeId`s.
/// A resource a system names that the world does not hold fails `run_tick` before anything
/// runs, with an error of kind `NotFound`.
pub struct Res<'w, T: ?Sized + 'static> {
    value: &'w T,
}

impl<T: ?Sized + 'static> Deref for Res<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: ?Sized + 'static> sealed::Sealed for Res<'_, T> {}

impl<T: ?Sized + 'static> SystemParam for Res<'_, T> {
    const ACCESS: Access = Access::new(AccessKind::Read, resource_target::<T>);
}

/// A world resource, written. Only in a system that runs once per tick, an exclusive
/// system or a finalize function, because a kernel runs on many workers; two systems that
/// write, or read and write, the same resource must be ordered.
pub struct ResMut<'w, T: ?Sized + 'static> {
    value: &'w mut T,
}

impl<T: ?Sized + 'static> Deref for ResMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: ?Sized + 'static> DerefMut for ResMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<T: ?Sized + 'static> sealed::Sealed for ResMut<'_, T> {}

impl<T: ?Sized + 'static> SystemParam for ResMut<'_, T> {
    const ACCESS: Access = Access::new(AccessKind::Write, resource_target::<T>);
}

/// A resource of the current region, read. Region resources are written through
/// [`World::region_resource_mut`] at a sync point, never from a kernel, because the
/// kernels of one region run on many workers. An invocation in a region that lacks `T` is
/// skipped.
pub struct RegionRes<'w, T: Resource> {
    value: &'w T,
}

impl<T: Resource> Deref for RegionRes<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: Resource> sealed::Sealed for RegionRes<'_, T> {}

impl<T: Resource> SystemParam for RegionRes<'_, T> {
    const ACCESS: Access = Access::new(AccessKind::Read, region_resource_target::<T>);
}

impl<T: Component> sealed::Sealed for &T {}

impl<T: Component> SystemParam for &T {
    const ACCESS: Access = Access::new(AccessKind::Read, component_target::<T>);
}

/// A component of the entity, written.
///
/// A mutable dereference marks the row changed (architecture section 6): on a tracked
/// column it sets the row's change stamp to the current tick and this system's position,
/// and on every column it is covered by the chunk's column tick, which the dispatch of a
/// writing system sets. Reading through `Deref` marks nothing.
///
/// Not for a component the world maintains ([`InRegion`](crate::InRegion),
/// [`OwnedBy`](crate::OwnedBy), [`ClaimedBy`](crate::ClaimedBy),
/// [`PredictedOrigin`](crate::PredictedOrigin)) nor for one with an `EntityRef` field, whose
/// reverse index is kept at command apply: those are written through
/// [`Commands::set`](crate::Commands::set), and the schedule rejects `Mut` on them.
pub struct Mut<'w, T: Component> {
    value: &'w mut T,
    /// The row's stamp on a tracked column.
    stamp: Option<(&'w mut Tick, &'w mut u32)>,
    now: Stamp,
}

impl<'w, T: Component> Mut<'w, T> {
    pub(crate) fn new(row: RowView<'w>, now: Stamp) -> Self {
        Self { value: typed_mut(row.bytes), stamp: row.stamps, now }
    }

    /// Writes `value` only if it differs from the current value, so rewriting an equal
    /// value does not dirty the row; returns whether it wrote. Equality is byte equality,
    /// which is exact for a reflected component: it has no implicit padding and its
    /// explicit padding is always zero. Replicated columns are written this way.
    pub fn set_if_neq(&mut self, value: T) -> bool {
        if bytemuck::bytes_of(&*self.value) == bytemuck::bytes_of(&value) {
            return false;
        }
        **self = value;
        true
    }
}

impl<T: Component> Deref for Mut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: Component> DerefMut for Mut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        if let Some((tick, position)) = &mut self.stamp {
            **tick = self.now.tick;
            **position = self.now.position;
        }
        self.value
    }
}

impl<T: Component> sealed::Sealed for Mut<'_, T> {}

impl<T: Component> SystemParam for Mut<'_, T> {
    const ACCESS: Access = Access::new(AccessKind::Write, component_target::<T>);
}

impl<T: Component> sealed::Sealed for Option<&T> {}

impl<T: Component> SystemParam for Option<&T> {
    const ACCESS: Access = Access::new(AccessKind::OptionalRead, component_target::<T>);
}

impl<T: Component> sealed::Sealed for Option<Mut<'_, T>> {}

impl<T: Component> SystemParam for Option<Mut<'_, T>> {
    const ACCESS: Access = Access::new(AccessKind::OptionalWrite, component_target::<T>);
}

/// Restricts the kernel to entities that have `T`, without reading it.
pub struct With<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> sealed::Sealed for With<T> {}

impl<T: Component> SystemParam for With<T> {
    const ACCESS: Access = Access::new(AccessKind::With, component_target::<T>);
}

/// Restricts the kernel to entities that lack `T`.
pub struct Without<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> sealed::Sealed for Without<T> {}

impl<T: Component> SystemParam for Without<T> {
    const ACCESS: Access = Access::new(AccessKind::Without, component_target::<T>);
}

/// Restricts the kernel of system `S` to entities whose `T` changed since `S` last ran:
/// rows whose change stamp is after `S`'s position in the previous tick and before it in
/// this one, whether the write came from a system ordered before `S`, a command applied at
/// a sync point or direct access between ticks. Each change is seen exactly once by each
/// reader; `S`'s own `Mut<T>` writes are not seen by its own filter. `T` must be a tracked
/// column (replicated or extracted), since only those carry a stamp per row; the schedule
/// rejects the filter on any other. It combines with one data parameter on `T` (`&T` to
/// read the changed value, `Mut<T>` to clamp it).
///
/// Across a rollback the filter reflects the world's own change history: a restore puts
/// every stamp back as snapshotted, so a re-simulated tick sees what the first run of that
/// tick saw, and a correction `lactose-predict` writes afterwards leaves the stamps alone
/// ([`ColumnMut::write`](crate::ColumnMut::write) without recording the row). A system whose
/// effect must predict exactly does not gate on `Changed`, since the server's change
/// history is not replicated.
pub struct Changed<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> sealed::Sealed for Changed<T> {}

impl<T: Component> SystemParam for Changed<T> {
    const ACCESS: Access = Access::new(AccessKind::Changed, component_target::<T>);
}

/// The region's context: the tick, the region, its RNG streams and its assets. In a
/// per-entity system the region is the entity's; in a system without a component parameter
/// it makes the system run once per `Simulated` region.
pub struct Ctx<'w> {
    seed: u64,
    region: RegionId,
    invoker: Option<StableKey>,
    system: SystemKey,
    tick: Tick,
    assets: Option<&'w PinnedTable>,
}

impl<'w> Ctx<'w> {
    pub(crate) fn new(
        seed: u64,
        region: RegionId,
        invoker: Option<StableKey>,
        system: SystemKey,
        tick: Tick,
        assets: Option<&'w PinnedTable>,
    ) -> Self {
        Self { seed, region, invoker, system, tick, assets }
    }
}

impl Ctx<'_> {
    /// The absolute simulation tick.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// The region.
    #[must_use]
    pub fn region(&self) -> RegionId {
        self.region
    }

    /// The RNG stream for `key`: keyed by the region seed, the tick, this system, the
    /// invoking entity's origin key (none in a per-region system) and `key` (architecture
    /// section 8.2). `key` is usually the entity's own origin key or a placeholder from this
    /// invocation's spawns, which is keyed by its spawn index; any stable key is accepted.
    /// Two calls with one key give the same stream: draw from one `Rng`.
    #[must_use]
    pub fn rng(&self, key: StableKey) -> Rng {
        Rng::keyed(self.seed, self.tick, self.system, self.invoker, key)
    }

    /// The exact-version assets of the region (architecture section 14), or `None` until a
    /// table has entered at an input sync point.
    #[must_use]
    pub fn assets(&self) -> Option<&PinnedTable> {
        self.assets
    }
}

impl sealed::Sealed for Ctx<'_> {}

impl SystemParam for Ctx<'_> {
    const ACCESS: Access = Access::new(AccessKind::Region, no_target);
}

/// The entity's origin key: its `NetId`, or the `PredictionKey` of a predicted spawn,
/// which stays the origin key after the server binds a `NetId` (architecture section 7).
/// Never a placeholder. No row, chunk or worker index exists in the API.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StableKeyOf(StableKey);

impl StableKeyOf {
    /// The key.
    #[must_use]
    pub const fn key(self) -> StableKey {
        self.0
    }
}

impl Deref for StableKeyOf {
    type Target = StableKey;

    fn deref(&self) -> &StableKey {
        &self.0
    }
}

impl From<StableKeyOf> for StableKey {
    fn from(key: StableKeyOf) -> Self {
        key.0
    }
}

impl sealed::Sealed for StableKeyOf {}

impl SystemParam for StableKeyOf {
    const ACCESS: Access = Access::new(AccessKind::Entity, no_target);
}

/// The executor's job context of the job the system runs as: fork-join over `&World`
/// through [`JobContext::scope`] (the parallel exclusive passes of architecture sections
/// 5.3, 8.3, 9.3 and 15.3, and a finalize that sorts or forks), the per-job scratch arena,
/// and [`JobContext::block_in_place`] for the physics step (section 16). Only in a system
/// that runs once per tick, an exclusive system or a finalize function.
///
/// `World` is `Sync` and every column view is `Send`, so a scope child may borrow them.
/// Children key what they produce by `StableKey` and combine it order-independently; the
/// worker index (`JobContext::slot`, `ChildContext::slot`) is never a key, and the boundary
/// lint file bans both methods outside this crate (architecture section 8.2).
pub struct Job<'w, 'j> {
    context: &'w mut JobContext<'j>,
}

impl<'j> Job<'_, 'j> {
    /// The job context.
    pub fn context(&mut self) -> &mut JobContext<'j> {
        self.context
    }
}

impl sealed::Sealed for Job<'_, '_> {}

impl SystemParam for Job<'_, '_> {
    const ACCESS: Access = Access::new(AccessKind::Job, no_target);
}

impl sealed::Sealed for &mut World {}

impl SystemParam for &mut World {
    const ACCESS: Access = Access::new(AccessKind::World, no_target);
}

// How each parameter is made from the slot the schedule prepared for it.

impl<'r, 'j, T: ?Sized + 'static> Fetch<'r, 'j> for Res<'r, T> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Resource(value) => Self { value: resource_ref(value) },
            _ => mismatch(index),
        }
    }
}

impl<'r, 'j, T: ?Sized + 'static> Fetch<'r, 'j> for ResMut<'r, T> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::ResourceMut(value) => {
                let value: &mut Box<T> = value.downcast_mut().expect("resources hold `Box<T>`");
                Self { value: &mut **value }
            }
            _ => mismatch(index),
        }
    }
}

/// The value inside the `Box<T>` a world resource is held as.
pub(crate) fn resource_ref<T: ?Sized + 'static>(value: &(dyn core::any::Any + Send + Sync)) -> &T {
    let value: &Box<T> = value.downcast_ref().expect("resources hold `Box<T>`");
    value
}

impl<'r, 'j, T: Resource> Fetch<'r, 'j> for RegionRes<'r, T> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Read(bytes) => Self { value: typed(bytes) },
            _ => mismatch(index),
        }
    }
}

impl<'r, 'j, T: Component> Fetch<'r, 'j> for &'r T {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Read(bytes) => typed(bytes),
            _ => mismatch(index),
        }
    }
}

impl<'r, 'j, T: Component> Fetch<'r, 'j> for Mut<'r, T> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Write(row, now) => Mut::new(row, now),
            _ => mismatch(index),
        }
    }
}

impl<'r, 'j, T: Component> Fetch<'r, 'j> for Option<&'r T> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Read(bytes) => Some(typed(bytes)),
            Slot::Absent => None,
            _ => mismatch(index),
        }
    }
}

impl<'r, 'j, T: Component> Fetch<'r, 'j> for Option<Mut<'r, T>> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Write(row, now) => Some(Mut::new(row, now)),
            Slot::Absent => None,
            _ => mismatch(index),
        }
    }
}

macro_rules! filter_fetch {
    ($($filter:ident),+) => {$(
        impl<'r, 'j, T: Component> Fetch<'r, 'j> for $filter<T> {
            fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
                match invocation.take(index) {
                    Slot::Unit => Self(PhantomData),
                    _ => mismatch(index),
                }
            }
        }
    )+};
}

filter_fetch!(With, Without, Changed);

impl<'r, 'j> Fetch<'r, 'j> for Ctx<'r> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Ctx(ctx) => ctx,
            _ => mismatch(index),
        }
    }
}

impl<'r, 'j> Fetch<'r, 'j> for StableKeyOf {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Key(key) => Self(key),
            _ => mismatch(index),
        }
    }
}

impl<'r, 'j> Fetch<'r, 'j> for Job<'r, 'j> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Job(context) => Self { context },
            _ => mismatch(index),
        }
    }
}

impl<'r, 'j> Fetch<'r, 'j> for &'r mut World {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::World(world) => world,
            _ => mismatch(index),
        }
    }
}
