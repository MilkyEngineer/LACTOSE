//! System descriptors and sets: what the `#[system]` expansion emits and the schedule
//! consumes (architecture sections 5.2 and 6.1).

use core::any::Any;

use lactose_core::{StableKey, SystemKey};
use lactose_math::Reducer;
use lactose_task::JobContext;

use crate::access::Access;
use crate::change::Stamp;
use crate::command::{ChannelCommand, CommandBuffer};
use crate::event::{EventRecord, PresentRecord};
use crate::param::{Ctx, SystemParam};
use crate::storage::RowView;
use crate::world::{World, WorldSet};

/// A named group of systems with an order relative to other sets, declared as a static:
///
/// ```
/// use lactose_ecs::SystemSet;
///
/// pub static MOVEMENT: SystemSet = SystemSet::new("game_sim::movement");
/// pub static COMBAT: SystemSet = SystemSet::new("game_sim::combat").after(&[&MOVEMENT]);
/// ```
///
/// A system joins a set with `in_set = path::to::MOVEMENT`. Every system of a set that is
/// after another runs after every system of the other; the schedule rejects a cycle. Sets
/// are placed in a stage by [`ScheduleBuilder::stage`](crate::ScheduleBuilder::stage).
#[derive(Debug)]
pub struct SystemSet {
    name: &'static str,
    after: &'static [&'static SystemSet],
}

impl SystemSet {
    /// A set named `name`, ordered after nothing.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self { name, after: &[] }
    }

    /// This set, ordered after every set in `sets`.
    #[must_use]
    pub const fn after(self, sets: &'static [&'static SystemSet]) -> Self {
        Self { name: self.name, after: sets }
    }

    /// The name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The sets this one is ordered after.
    #[must_use]
    pub const fn ordered_after(&self) -> &'static [&'static SystemSet] {
        self.after
    }
}

/// Everything the schedule knows about a system, emitted by `#[system]` as a `pub static`
/// named after the function in upper snake case (`apply_gravity` gives `APPLY_GRAVITY`).
///
/// The function itself stays callable as written, with every parameter binding made
/// mutable. The access table is the [`SystemParam::ACCESS`](crate::SystemParam::ACCESS) of
/// each parameter in declaration order; a reduction's finalize function has a table of its
/// own, since the per-kind rules differ between the kernel and the finalize, and conflicts
/// use their union.
pub struct SystemDescriptor {
    name: &'static str,
    key: SystemKey,
    worlds: WorldSet,
    set: Option<&'static SystemSet>,
    after: &'static [&'static SystemDescriptor],
    access: &'static [Access],
    finalize_access: &'static [Access],
    exclusive: bool,
    reduce: Option<&'static str>,
    kernel: Kernel,
}

impl SystemDescriptor {
    pub(crate) fn kernel(&self) -> &Kernel {
        &self.kernel
    }

    /// Builds a descriptor. Called by the `#[system]` expansion only; the arguments are
    /// positional so a change here is a change of the macro in the same commit.
    ///
    /// # Safety
    ///
    /// `access` must describe exactly what `kernel.run` touches and `finalize_access` what
    /// `kernel.finalize` touches: every component, resource, event and channel each reads
    /// or writes, with the right kind. The schedule orders systems by these tables, and the
    /// storage hands out `&mut` rows on the strength of them (`SAFETY.md`). The `#[system]`
    /// expansion derives both from the parameter types and the `writes` list and is the
    /// one sound caller.
    #[doc(hidden)]
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "positional on purpose: see above")]
    pub const unsafe fn new(
        name: &'static str,
        worlds: WorldSet,
        set: Option<&'static SystemSet>,
        after: &'static [&'static SystemDescriptor],
        access: &'static [Access],
        finalize_access: &'static [Access],
        exclusive: bool,
        reduce: Option<&'static str>,
        kernel: Kernel,
    ) -> Self {
        Self {
            name,
            key: SystemKey::from_name(name),
            worlds,
            set,
            after,
            access,
            finalize_access,
            exclusive,
            reduce,
            kernel,
        }
    }

    /// The declared name, such as `"game_sim::apply_gravity"`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The stable key: xxh64 of the name, which breaks ordering ties and orders command
    /// buffers (architecture section 7). The schedule rejects two systems with one key.
    #[must_use]
    pub const fn key(&self) -> SystemKey {
        self.key
    }

    /// The worlds the system runs in.
    #[must_use]
    pub const fn worlds(&self) -> WorldSet {
        self.worlds
    }

    /// The set the system belongs to, if any.
    #[must_use]
    pub const fn set(&self) -> Option<&'static SystemSet> {
        self.set
    }

    /// The systems this one is ordered after.
    #[must_use]
    pub const fn ordered_after(&self) -> &'static [&'static SystemDescriptor] {
        self.after
    }

    /// The kernel's access table.
    #[must_use]
    pub const fn access(&self) -> &'static [Access] {
        self.access
    }

    /// The finalize function's access table; empty without a reduction.
    #[must_use]
    pub const fn finalize_access(&self) -> &'static [Access] {
        self.finalize_access
    }

    /// Whether the system takes the whole world and runs alone at a sync point.
    #[must_use]
    pub const fn is_exclusive(&self) -> bool {
        self.exclusive
    }

    /// The type name of the reducer, for a reduction.
    #[must_use]
    pub const fn reduce(&self) -> Option<&'static str> {
        self.reduce
    }
}

impl core::fmt::Debug for SystemDescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SystemDescriptor")
            .field("name", &self.name)
            .field("key", &self.key)
            .field("worlds", &self.worlds)
            .field("exclusive", &self.exclusive)
            .field("reduce", &self.reduce)
            .finish_non_exhaustive()
    }
}

/// The entry points of a system. Not part of the interface: the implementation shapes
/// these together with the `#[system]` expansion.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct Kernel {
    /// Runs the system over one invocation: one entity, one region, or the once-per-tick
    /// call.
    pub run: fn(&mut Invocation<'_, '_>),
    /// Runs the finalize function of a reduction, or does nothing.
    pub finalize: fn(&mut Invocation<'_, '_>),
    /// How the reducer's partials are made and merged, for a reduction.
    pub reducer: Option<ReducerOps>,
}

impl Kernel {
    /// The finalize entry point of a system that is not a reduction.
    pub fn no_finalize(_: &mut Invocation<'_, '_>) {}
}

/// How the schedule makes and merges a reduction's partials without naming the reducer.
/// Not part of the interface.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct ReducerOps {
    pub(crate) identity: fn() -> Box<dyn Any>,
    pub(crate) merge: fn(&mut dyn Any, &dyn Any),
}

impl ReducerOps {
    /// The operations of `R`.
    #[must_use]
    pub const fn of<R: Reducer + 'static>() -> Self {
        Self { identity: identity::<R>, merge: merge::<R> }
    }
}

fn identity<R: Reducer + 'static>() -> Box<dyn Any> {
    Box::new(R::identity())
}

fn merge<R: Reducer + 'static>(into: &mut dyn Any, other: &dyn Any) {
    let into = into.downcast_mut::<R>().expect("partials of one reducer");
    into.merge(other.downcast_ref::<R>().expect("partials of one reducer"));
}

/// What a kernel entry point receives: one value per parameter, prepared from the access
/// table and each taken exactly once. Not part of the interface.
#[doc(hidden)]
pub struct Invocation<'r, 'j> {
    slots: Vec<Slot<'r, 'j>>,
    reducer: ReducerSlot<'r>,
}

impl<'r, 'j> Invocation<'r, 'j> {
    pub(crate) fn new(slots: Vec<Slot<'r, 'j>>, reducer: ReducerSlot<'r>) -> Self {
        Self { slots, reducer }
    }

    /// The slot of parameter `index`, which a fetch takes.
    pub(crate) fn take(&mut self, index: usize) -> Slot<'r, 'j> {
        core::mem::replace(&mut self.slots[index], Slot::Taken)
    }
}

/// The value prepared for one parameter.
pub(crate) enum Slot<'r, 'j> {
    /// Already fetched.
    Taken,
    /// A filter: `With`, `Without`, `Changed`.
    Unit,
    /// An optional component the entity lacks.
    Absent,
    /// A component row or a region resource, read.
    Read(&'r [u8]),
    /// A component row, written, and the stamp a mutable dereference writes.
    Write(RowView<'r>, Stamp),
    /// A world resource, read: the `Box<T>` the world holds.
    Resource(&'r (dyn Any + Send + Sync)),
    /// A world resource, written.
    ResourceMut(&'r mut (dyn Any + Send + Sync)),
    Key(StableKey),
    Ctx(Ctx<'r>),
    Commands(&'r mut CommandBuffer),
    EmitEvents(&'r mut Vec<EventRecord>),
    ReadEvents(&'r [EventRecord]),
    Present(&'r mut Vec<PresentRecord>),
    /// The `Vec<ChannelEntry<C>>` of a channel, if it has entries.
    Channel(Option<&'r (dyn Any + Send + Sync)>),
    /// The sink of one `ChannelWriter` parameter.
    WriteChannel(&'r mut Vec<ChannelCommand>),
    /// The `Vec<ChannelEntry<C>>` of a channel, removed from the world, if it had entries.
    TakeChannel(Option<Box<dyn Any + Send + Sync>>),
    Job(&'r mut JobContext<'j>),
    World(&'r mut World),
}

/// A reduction's partial for a kernel, or its merged total for the finalize.
pub(crate) enum ReducerSlot<'r> {
    None,
    Partial(&'r mut dyn Any),
    Total(Option<Box<dyn Any>>),
}

/// How a parameter is made from its slot. Implemented for exactly the [`SystemParam`]
/// types. Not part of the interface.
#[doc(hidden)]
pub trait Fetch<'r, 'j>: Sized {
    /// The parameter at `index`.
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self;
}

/// The parameter at `index`, for the expansions.
#[doc(hidden)]
pub fn fetch<'r, 'j, P: SystemParam + Fetch<'r, 'j>>(
    invocation: &mut Invocation<'r, 'j>,
    index: usize,
) -> P {
    P::fetch(invocation, index)
}

/// A slot that does not hold what the access table said it would: the table and the
/// entry point disagree, which `SystemDescriptor::new`'s contract rules out.
#[cold]
pub(crate) fn mismatch(index: usize) -> ! {
    panic!("parameter {index} does not match the system's access table")
}

/// Folds a kernel's result into the invocation's partial, for the `#[system]` expansion.
#[doc(hidden)]
pub fn reduce<R: Reducer + 'static>(invocation: &mut Invocation<'_, '_>, item: R::Item) {
    match &mut invocation.reducer {
        ReducerSlot::Partial(partial) => {
            partial.downcast_mut::<R>().expect("the partial of this reducer").add(item);
        }
        _ => panic!("a reduction's kernel runs with a partial"),
    }
}

/// The merged result of a reduction, for the finalize entry point of the `#[system]`
/// expansion.
#[doc(hidden)]
pub fn finish<R: Reducer + 'static>(invocation: &mut Invocation<'_, '_>) -> R::Output {
    match &mut invocation.reducer {
        ReducerSlot::Total(total) => {
            let total = total.take().expect("a reduction finishes once");
            total.downcast::<R>().expect("the total of this reducer").finish()
        }
        _ => panic!("a reduction's finalize runs with its total"),
    }
}

/// What `#[finalize(f)]` emits as `__finalize_f`: the finalize function's access table and
/// its entry point, which takes the reducer's output first. Not part of the interface.
#[doc(hidden)]
pub struct Finalize<O: 'static> {
    access: &'static [Access],
    run: unsafe fn(&mut Invocation<'_, '_>, O),
}

impl<O: 'static> Finalize<O> {
    /// Pairs an access table with the entry point that fetches by it.
    ///
    /// # Safety
    ///
    /// `access` must describe exactly what `run` fetches, as for
    /// [`SystemDescriptor::new`]. The `#[finalize]` expansion derives both from the
    /// function's parameter types and is the one sound caller.
    #[must_use]
    pub const unsafe fn new(
        access: &'static [Access],
        run: unsafe fn(&mut Invocation<'_, '_>, O),
    ) -> Self {
        Self { access, run }
    }

    /// The finalize function's access table.
    #[must_use]
    pub const fn access(&self) -> &'static [Access] {
        self.access
    }

    /// Runs the finalize function with `output`.
    ///
    /// # Safety
    ///
    /// `invocation` must have been prepared from [`access`](Self::access), which holds when
    /// the descriptor that runs it was built with that table as its finalize table.
    pub unsafe fn call(&self, invocation: &mut Invocation<'_, '_>, output: O) {
        // SAFETY: the caller prepared `invocation` from `self.access`, and the constructor's
        // caller promised that `run` fetches exactly what that table describes.
        unsafe { (self.run)(invocation, output) }
    }
}
