//! Queries over the world from outside a kernel: exclusive systems, engine passes and
//! tests (architecture section 6).

use core::marker::PhantomData;

use lactose_core::StableKey;
use lactose_reflect::Component;

use crate::param::{Changed, Mut, SystemParam, With, Without, sealed};

/// A per-entity parameter type that may appear in a query tuple: `&T`, [`Mut<T>`],
/// `Option<&T>`, `Option<Mut<T>>`, [`With<T>`], [`Without<T>`] and [`Changed<T>`]. Closed,
/// like [`SystemParam`].
pub trait QueryItem: SystemParam {
    /// The item with the query's borrow.
    type Item<'a>;
}

impl<T: Component> QueryItem for &T {
    type Item<'a> = &'a T;
}

impl<T: Component> QueryItem for Mut<'_, T> {
    type Item<'a> = Mut<'a, T>;
}

impl<T: Component> QueryItem for Option<&T> {
    type Item<'a> = Option<&'a T>;
}

impl<T: Component> QueryItem for Option<Mut<'_, T>> {
    type Item<'a> = Option<Mut<'a, T>>;
}

impl<T: Component> QueryItem for With<T> {
    type Item<'a> = With<T>;
}

impl<T: Component> QueryItem for Without<T> {
    type Item<'a> = Without<T>;
}

impl<T: Component> QueryItem for Changed<T> {
    type Item<'a> = Changed<T>;
}

/// One [`QueryItem`] or a tuple of up to eight; a bare `&T` and `(&T,)` both work. Closed,
/// like [`SystemParam`]. The alias rule of a kernel applies: two data items on one
/// component, or `Without<T>` beside any item on `T`, is a panic at
/// [`World::query`](crate::World::query); `Changed<T>` and `With<T>` combine with one data
/// item on `T`.
pub trait QueryItems: sealed::Sealed {
    /// The items with the query's borrow.
    type Item<'a>;
}

impl<Q: QueryItem> QueryItems for Q {
    type Item<'a> = Q::Item<'a>;
}

macro_rules! tuple_query_items {
    ($(($($name:ident),+);)+) => {$(
        impl<$($name: QueryItem),+> sealed::Sealed for ($($name,)+) {}

        impl<$($name: QueryItem),+> QueryItems for ($($name,)+) {
            type Item<'a> = ($($name::Item<'a>,)+);
        }
    )+};
}

tuple_query_items! {
    (A);
    (A, B);
    (A, B, C);
    (A, B, C, D);
    (A, B, C, D, E);
    (A, B, C, D, E, F);
    (A, B, C, D, E, F, G);
    (A, B, C, D, E, F, G, H);
}

/// A query over every entity that has the required components, in every region and every
/// mode, with no world filter: this is the whole-world view that
/// [`World::query`](crate::World::query) gives exclusive code, not the kernel's.
///
/// Iteration is in ascending origin-key order, so exclusive code that spawns, allocates or
/// emits per match does so in the same order under every layout.
pub struct Query<'w, Q: QueryItems> {
    _world: PhantomData<&'w mut ()>,
    _items: PhantomData<fn() -> Q>,
}

impl<Q: QueryItems> Query<'_, Q> {
    /// Calls `f` once per matching entity with its origin key and items, in ascending key
    /// order.
    pub fn for_each(&mut self, f: impl FnMut(StableKey, Q::Item<'_>)) {
        todo!()
    }

    /// The items of `key`, or `None` if the entity does not exist or does not match.
    #[must_use]
    pub fn get(&mut self, key: StableKey) -> Option<Q::Item<'_>> {
        todo!()
    }

    /// How many entities match.
    #[must_use]
    pub fn count(&self) -> usize {
        todo!()
    }
}
