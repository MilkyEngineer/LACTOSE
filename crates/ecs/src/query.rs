//! Queries over the world from outside a kernel: exclusive systems, engine passes and
//! tests (architecture section 6).

use core::marker::PhantomData;

use lactose_core::{ComponentKey, StableKey};
use lactose_reflect::Component;

use crate::access::{Access, AccessKind, AccessTarget};
use crate::change::Stamp;
use crate::param::{Changed, Mut, SystemParam, With, Without, sealed};
use crate::storage::{Archetype, Chunk, Loc, Storage};
use crate::system::{Fetch, Invocation, ReducerSlot, Slot};
use crate::world::{World, is_tracked, refuses_mut};

/// A per-entity parameter type that may appear in a query tuple: `&T`, [`Mut<T>`],
/// `Option<&T>`, `Option<Mut<T>>`, [`With<T>`], [`Without<T>`] and [`Changed<T>`]. Closed,
/// like [`SystemParam`].
pub trait QueryItem: SystemParam {
    /// The item with the query's borrow.
    type Item<'a>;

    /// The item at `index` of a query row. Not part of the interface.
    #[doc(hidden)]
    fn fetch_item<'a>(invocation: &mut Invocation<'a, '_>, index: usize) -> Self::Item<'a>;
}

macro_rules! query_item {
    ($($param:ty => $item:ty;)+) => {$(
        impl<T: Component> QueryItem for $param {
            type Item<'a> = $item;

            fn fetch_item<'a>(invocation: &mut Invocation<'a, '_>, index: usize) -> $item {
                <$item as Fetch<'a, '_>>::fetch(invocation, index)
            }
        }
    )+};
}

query_item! {
    &T => &'a T;
    Mut<'_, T> => Mut<'a, T>;
    Option<&T> => Option<&'a T>;
    Option<Mut<'_, T>> => Option<Mut<'a, T>>;
    With<T> => With<T>;
    Without<T> => Without<T>;
    Changed<T> => Changed<T>;
}

/// One [`QueryItem`] or a tuple of up to eight; a bare `&T` and `(&T,)` both work. Closed,
/// like [`SystemParam`]. The alias rule of a kernel applies: two data items on one
/// component, or `Without<T>` beside any item on `T`, is a panic at
/// [`World::query`](crate::World::query); `Changed<T>` and `With<T>` combine with one data
/// item on `T`.
pub trait QueryItems: sealed::Sealed {
    /// The items with the query's borrow.
    type Item<'a>;

    /// The access of each item, in order. Not part of the interface.
    #[doc(hidden)]
    fn access() -> Vec<Access>;

    /// The items of a query row. Not part of the interface.
    #[doc(hidden)]
    fn fetch_items<'a>(invocation: &mut Invocation<'a, '_>) -> Self::Item<'a>;
}

impl<Q: QueryItem> QueryItems for Q {
    type Item<'a> = Q::Item<'a>;

    fn access() -> Vec<Access> {
        vec![Q::ACCESS]
    }

    fn fetch_items<'a>(invocation: &mut Invocation<'a, '_>) -> Self::Item<'a> {
        Q::fetch_item(invocation, 0)
    }
}

macro_rules! tuple_query_items {
    ($(($($name:ident: $index:tt),+);)+) => {$(
        impl<$($name: QueryItem),+> sealed::Sealed for ($($name,)+) {}

        impl<$($name: QueryItem),+> QueryItems for ($($name,)+) {
            type Item<'a> = ($($name::Item<'a>,)+);

            fn access() -> Vec<Access> {
                vec![$($name::ACCESS),+]
            }

            fn fetch_items<'a>(invocation: &mut Invocation<'a, '_>) -> Self::Item<'a> {
                ($($name::fetch_item(invocation, $index),)+)
            }
        }
    )+};
}

tuple_query_items! {
    (A: 0);
    (A: 0, B: 1);
    (A: 0, B: 1, C: 2);
    (A: 0, B: 1, C: 2, D: 3);
    (A: 0, B: 1, C: 2, D: 3, E: 4);
    (A: 0, B: 1, C: 2, D: 3, E: 4, F: 5);
    (A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6);
    (A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7);
}

/// What a query asks of an entity's components.
struct Filter {
    access: Vec<(AccessKind, ComponentKey)>,
    /// The window a `Changed` item passes: after the previous tick's point, up to now.
    changed_after: Stamp,
    /// The tick stamps are ordered from.
    now: lactose_core::Tick,
}

impl Filter {
    fn new<Q: QueryItems>(world: &World, now: Stamp) -> Self {
        let access: Vec<(AccessKind, ComponentKey)> = Q::access()
            .iter()
            .map(|access| match access.target() {
                AccessTarget::Component(key) => (access.kind(), key),
                _ => unreachable!("query items name components"),
            })
            .collect();
        for (index, &(kind, key)) in access.iter().enumerate() {
            let info = world.registry.get(key);
            let others =
                access.iter().enumerate().filter(|&(other, &(_, k))| other != index && k == key);
            let data = |kind: AccessKind| {
                matches!(
                    kind,
                    AccessKind::Read
                        | AccessKind::Write
                        | AccessKind::OptionalRead
                        | AccessKind::OptionalWrite
                )
            };
            for (_, &(other_kind, _)) in others {
                assert!(
                    !(data(kind) && data(other_kind))
                        && kind != AccessKind::Without
                        && other_kind != AccessKind::Without,
                    "a query names component {:#018x} twice in a way that aliases",
                    key.0
                );
            }
            if matches!(kind, AccessKind::Write | AccessKind::OptionalWrite) {
                assert!(
                    !info.is_some_and(refuses_mut),
                    "a query names `Mut` on a component the world maintains or one that holds \
                     a reference"
                );
            }
            if kind == AccessKind::Changed {
                assert!(info.is_some_and(is_tracked), "`Changed` needs a tracked column");
            }
        }
        // One tick back from now, across the wrap.
        let changed_after =
            Stamp::new(lactose_core::Tick(now.tick.0.wrapping_sub(1)), now.position);
        Self { access, changed_after, now: now.tick }
    }

    /// Whether every row of `archetype` has what the query requires and lacks what it
    /// excludes.
    fn admits(&self, archetype: &Archetype) -> bool {
        self.access.iter().all(|&(kind, component)| match kind {
            AccessKind::Read | AccessKind::Write | AccessKind::With | AccessKind::Changed => {
                archetype.has(component)
            }
            AccessKind::Without => !archetype.has(component),
            _ => true,
        })
    }

    /// Whether row `row` of `chunk`, in an admitted archetype, passes every `Changed` item.
    fn passes(&self, archetype: &Archetype, chunk: &Chunk, row: usize) -> bool {
        self.access.iter().all(|&(kind, component)| {
            kind != AccessKind::Changed
                || archetype.column_index(component).is_some_and(|index| {
                    chunk.stamps[index]
                        .as_ref()
                        .is_some_and(|stamps| stamps.get(row).after(self.changed_after, self.now))
                })
        })
    }

    /// Whether the entity at `loc` matches, by its archetype and its row stamps.
    fn matches(&self, storage: &Storage, loc: Loc) -> bool {
        let archetype = &storage.archetypes[loc.archetype as usize];
        self.admits(archetype)
            && self.passes(archetype, &archetype.chunks[loc.chunk as usize], loc.row as usize)
    }

    /// The archetypes whose rows a query visits: only those it admits.
    fn archetypes<'s>(&self, storage: &'s Storage) -> impl Iterator<Item = (u32, &'s Archetype)> {
        storage
            .archetypes
            .iter()
            .enumerate()
            .filter(|(_, archetype)| self.admits(archetype))
            .map(|(index, archetype)| (index as u32, archetype))
    }

    /// Calls `f` with the key and place of every matching row, in storage order. Only the
    /// rows of admitted archetypes are visited.
    fn each_match(&self, storage: &Storage, mut f: impl FnMut(StableKey, Loc)) {
        for (index, archetype) in self.archetypes(storage) {
            for (chunk_index, chunk) in archetype.chunks.iter().enumerate() {
                for (row, &key) in chunk.keys.iter().enumerate() {
                    if self.passes(archetype, chunk, row) {
                        f(
                            key,
                            Loc { archetype: index, chunk: chunk_index as u32, row: row as u32 },
                        );
                    }
                }
            }
        }
    }
}

/// A query over every entity that has the required components, in every region and every
/// mode, with no world filter: this is the whole-world view that
/// [`World::query`](crate::World::query) gives exclusive code, not the kernel's.
///
/// Iteration is in ascending origin-key order, so exclusive code that spawns, allocates or
/// emits per match does so in the same order under every layout. `Changed<T>` passes rows
/// changed after the current point of the previous tick.
pub struct Query<'w, Q: QueryItems> {
    world: &'w mut World,
    filter: Filter,
    /// The stamp a `Mut` item's mutable dereference writes.
    now: Stamp,
    _items: PhantomData<fn() -> Q>,
}

impl<'w, Q: QueryItems> Query<'w, Q> {
    pub(crate) fn new(world: &'w mut World) -> Self {
        let now = world.bump();
        let filter = Filter::new::<Q>(world, now);
        Self { world, filter, now, _items: PhantomData }
    }

    /// The items of the matching row at `loc`.
    fn row(&mut self, loc: Loc) -> Q::Item<'_> {
        let archetype = &mut self.world.storage.archetypes[loc.archetype as usize];
        let chunk = &mut archetype.chunks[loc.chunk as usize];
        for &(kind, component) in &self.filter.access {
            if matches!(kind, AccessKind::Write | AccessKind::OptionalWrite)
                && let Some(index) =
                    archetype.columns.binary_search_by_key(&component, |c| c.key).ok()
            {
                let tick = &mut chunk.column_ticks[index];
                *tick = self.now;
            }
        }
        let mut views: Vec<Option<_>> =
            chunk.row_views(&archetype.columns, loc.row as usize).into_iter().map(Some).collect();
        let slots = self
            .filter
            .access
            .iter()
            .map(|&(kind, component)| {
                // Filters on the component of a data item take no view.
                let data = matches!(
                    kind,
                    AccessKind::Read
                        | AccessKind::Write
                        | AccessKind::OptionalRead
                        | AccessKind::OptionalWrite
                );
                let view = archetype
                    .columns
                    .binary_search_by_key(&component, |c| c.key)
                    .ok()
                    .filter(|_| data)
                    .and_then(|index| views[index].take());
                match (kind, view) {
                    (AccessKind::Read | AccessKind::OptionalRead, Some(view)) => {
                        Slot::Read(view.bytes)
                    }
                    (AccessKind::Write | AccessKind::OptionalWrite, Some(view)) => {
                        Slot::Write(view, self.now)
                    }
                    (AccessKind::OptionalRead | AccessKind::OptionalWrite, None) => Slot::Absent,
                    _ => Slot::Unit,
                }
            })
            .collect();
        let mut invocation = Invocation::new(slots, ReducerSlot::None);
        Q::fetch_items(&mut invocation)
    }

    /// Calls `f` once per matching entity with its origin key and items, in ascending key
    /// order.
    pub fn for_each(&mut self, mut f: impl FnMut(StableKey, Q::Item<'_>)) {
        let mut rows = Vec::new();
        self.filter.each_match(&self.world.storage, |key, loc| rows.push((key, loc)));
        // Keys are unique, so this order is total and independent of the layout.
        rows.sort_unstable_by_key(|&(key, _)| key);
        for (key, loc) in rows {
            f(key, self.row(loc));
        }
    }

    /// The items of `key`, or `None` if the entity does not exist or does not match.
    #[must_use]
    pub fn get(&mut self, key: StableKey) -> Option<Q::Item<'_>> {
        let key = self.world.origin(key)?;
        let loc = self.world.storage.loc(key)?;
        if !self.filter.matches(&self.world.storage, loc) {
            return None;
        }
        Some(self.row(loc))
    }

    /// How many entities match.
    #[must_use]
    pub fn count(&self) -> usize {
        let mut count = 0;
        self.filter.each_match(&self.world.storage, |_, _| count += 1);
        count
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lactose_core::NetId;
    use lactose_reflect::{Registry, component};

    use super::*;
    use crate::{ClaimedBy, InRegion, OwnedBy, PredictedOrigin, RegionConfig, RegionId};
    use crate::{RegionMode, WorldBuilder, WorldKind};

    #[component(key = "ecs_test::common", persistent)]
    struct Common(#[field(id = 1)] u64);

    #[component(key = "ecs_test::rare", persistent, replicated)]
    struct Rare(#[field(id = 1)] u64);

    /// Rows in the world: enough that visiting every row would show at full size, few
    /// enough under Miri that the crate's run stays inside the nightly job.
    const ROWS: u64 = if cfg!(miri) { 105 } else { 10_005 };
    /// Rows that also have `Rare`.
    const RARE: u64 = 5;
    /// Every `STRIDE`-th id has `Rare`, from id 0, which places exactly `RARE` of them.
    const STRIDE: u64 = (ROWS - RARE) / RARE + 1;

    /// Every row has `Common`; `RARE` of them, one every `STRIDE` ids, have `Rare` too.
    fn world() -> World {
        let mut registry = Registry::new();
        registry.register::<InRegion>().unwrap();
        registry.register::<OwnedBy>().unwrap();
        registry.register::<ClaimedBy>().unwrap();
        registry.register::<PredictedOrigin>().unwrap();
        registry.register::<Common>().unwrap();
        registry.register::<Rare>().unwrap();
        let mut world = WorldBuilder::new(WorldKind::Server, Arc::new(registry)).build().unwrap();
        let region = RegionId(1);
        world
            .create_region(RegionConfig { id: region, seed: 1, mode: RegionMode::Simulated })
            .unwrap();
        for id in 0..ROWS {
            let key = StableKey::Net(NetId::cooked(id));
            if id % STRIDE == 0 {
                world.spawn(key, region, (Common(id), Rare(id))).unwrap();
            } else {
                world.spawn(key, region, (Common(id),)).unwrap();
            }
        }
        world
    }

    /// The rows a query over `Q` visits: those of the archetypes it admits.
    fn visited<Q: QueryItems>(world: &mut World) -> u64 {
        let query = world.query::<Q>();
        query
            .filter
            .archetypes(&query.world.storage)
            .flat_map(|(_, archetype)| &archetype.chunks)
            .map(|chunk| u64::try_from(chunk.len()).unwrap())
            .sum()
    }

    #[test]
    fn a_query_visits_only_the_rows_of_archetypes_it_admits() {
        let mut world = world();
        assert_eq!(visited::<&Rare>(&mut world), RARE);
        assert_eq!(visited::<(&Common, With<Rare>)>(&mut world), RARE);
        assert_eq!(visited::<(&Common, Without<Rare>)>(&mut world), ROWS - RARE);
        assert_eq!(visited::<Changed<Rare>>(&mut world), RARE);
        assert_eq!(visited::<Option<&Rare>>(&mut world), ROWS);

        let mut seen = Vec::new();
        world.query::<&Rare>().for_each(|key, rare| seen.push((key, rare.0)));
        let expected: Vec<(StableKey, u64)> = (0..RARE)
            .map(|n| n * STRIDE)
            .map(|id| (StableKey::Net(NetId::cooked(id)), id))
            .collect();
        assert_eq!(seen, expected);
        let count = world.query::<(&Common, Without<Rare>)>().count();
        assert_eq!(u64::try_from(count).unwrap(), ROWS - RARE);
    }
}
