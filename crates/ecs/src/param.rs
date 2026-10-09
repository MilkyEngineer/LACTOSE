//! System parameters: the types a `#[system]` function takes, each declaring the access it
//! needs (architecture section 6.1).

use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use lactose_asset_api::PinnedTable;
use lactose_core::{StableKey, Tick};
use lactose_reflect::{Component, Resource};

use crate::access::{
    Access, AccessKind, component_target, no_target, region_resource_target, resource_target,
};
use crate::region::RegionId;
use crate::rng::Rng;
use crate::world::World;

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A type a `#[system]` function may take as a parameter.
///
/// The set is closed: it is exactly the parameter types this crate exports, and
/// [`ACCESS`](Self::ACCESS) is what the `#[system]` expansion records for each. The kinds:
///
/// | Parameter | Scope | Access |
/// |---|---|---|
/// | `&T`, [`Mut<T>`], `Option<&T>`, `Option<Mut<T>>` | per entity | the component |
/// | [`With<T>`], [`Without<T>`], [`Changed<T>`] | per entity | a filter on the component |
/// | [`Ctx`], [`StableKeyOf`] | per entity | none |
/// | [`Res<T>`], [`ResMut<T>`], [`RegionRes<T>`] | any | the resource |
/// | [`Commands`](crate::Commands), [`EventWriter<E>`](crate::EventWriter), [`PresentEvents`](crate::PresentEvents) | any | output buffers |
/// | [`Events<E>`](crate::Events), [`Channel<C>`](crate::Channel) | any | input buffers |
/// | `&mut World` | exclusive | everything |
///
/// A system with at least one component parameter is a per-entity kernel: it runs once per
/// entity that has every required component, lacks every excluded one, resides in a
/// `Simulated` region and passes the world's own filter. A system with none runs once per
/// tick and may not take a per-entity parameter. An exclusive system takes `&mut World`
/// first and otherwise only resource, command and event parameters, which are taken out of
/// the world for the call: inside, the world does not hold them.
pub trait SystemParam: sealed::Sealed + Sized {
    /// What this parameter reads or writes.
    const ACCESS: Access;
}

/// A world resource, read. `T` is a reflected [`Resource`] or the `dyn` trait type of a
/// service resource that `app` inserted (architecture section 4).
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

/// A world resource, written. Two systems that write or read and write the same resource
/// must be ordered.
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

/// A resource of the entity's region, read. Region resources are written through
/// [`World::region_resource_mut`] at a sync point, never from a kernel, because the kernels
/// of one region run on many workers.
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
/// A mutable dereference marks the row changed at the current tick (architecture section
/// 6): on a tracked column it sets the row's tick, and on every column it is covered by the
/// chunk's column tick, which the dispatch of a writing system sets. Reading through `Deref`
/// marks nothing.
pub struct Mut<'w, T: Component> {
    value: &'w mut T,
    row_tick: Option<&'w mut Tick>,
    now: Tick,
}

impl<T: Component> Mut<'_, T> {
    /// Writes `value` only if it differs from the current value, so rewriting an equal
    /// value does not dirty the row; returns whether it wrote. Equality is byte equality,
    /// which is exact for a reflected component: it has no implicit padding and its
    /// explicit padding is always zero. Replicated columns are written this way.
    pub fn set_if_neq(&mut self, value: T) -> bool {
        todo!()
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
        todo!()
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

/// Restricts the kernel to entities whose `T` changed since the previous tick: rows whose
/// change tick is the previous tick or the current one. A change earlier in the current
/// tick is seen only by a system ordered after the writer, and the schedule requires that
/// order, since the filter reads what the writer writes. On an untracked column the row
/// tick is the chunk's column tick, so the filter is exact per chunk, not per row.
pub struct Changed<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> sealed::Sealed for Changed<T> {}

impl<T: Component> SystemParam for Changed<T> {
    const ACCESS: Access = Access::new(AccessKind::Changed, component_target::<T>);
}

/// The per-entity context: the tick, the entity's region, its RNG streams and its region's
/// assets.
pub struct Ctx<'w> {
    world: &'w World,
    region: RegionId,
    key: StableKey,
    system: lactose_core::SystemKey,
    tick: Tick,
}

impl Ctx<'_> {
    /// The current tick.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// The region the entity resides in.
    #[must_use]
    pub fn region(&self) -> RegionId {
        self.region
    }

    /// The RNG stream for `key`: keyed by the region seed, the tick, this system and `key`
    /// (architecture section 8.2). `key` is usually the entity's own origin key or a
    /// placeholder from this invocation's commands; any stable key is accepted.
    #[must_use]
    pub fn rng(&self, key: StableKey) -> Rng {
        todo!()
    }

    /// The exact-version assets of the entity's region (architecture section 14), or `None`
    /// until a table has entered at an input sync point.
    #[must_use]
    pub fn assets(&self) -> Option<&PinnedTable> {
        todo!()
    }
}

impl sealed::Sealed for Ctx<'_> {}

impl SystemParam for Ctx<'_> {
    const ACCESS: Access = Access::new(AccessKind::Entity, no_target);
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

impl sealed::Sealed for &mut World {}

impl SystemParam for &mut World {
    const ACCESS: Access = Access::new(AccessKind::World, no_target);
}
