//! System descriptors and sets: what the `#[system]` expansion emits and the schedule
//! consumes (architecture sections 5.2 and 6.1).

use lactose_core::SystemKey;

use crate::access::Access;
use crate::world::WorldSet;

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
/// each parameter in declaration order, followed by those of the finalize function's
/// parameters for a reduction.
pub struct SystemDescriptor {
    name: &'static str,
    key: SystemKey,
    worlds: WorldSet,
    set: Option<&'static SystemSet>,
    after: &'static [&'static SystemDescriptor],
    access: &'static [Access],
    exclusive: bool,
    reduce: Option<&'static str>,
    kernel: Kernel,
}

impl SystemDescriptor {
    /// Builds a descriptor. Called by the `#[system]` expansion only; the arguments are
    /// positional so a change here is a change of the macro in the same commit.
    ///
    /// # Safety
    ///
    /// `access` must describe exactly what `kernel` touches: every component, resource,
    /// event and channel it reads or writes, with the right kind. The schedule orders
    /// systems by this table, and the storage hands out `&mut` rows on the strength of it
    /// (`SAFETY.md`). The `#[system]` expansion derives the table from the parameter types
    /// and is the one sound caller.
    #[doc(hidden)]
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "positional on purpose: see above")]
    pub const unsafe fn new(
        name: &'static str,
        worlds: WorldSet,
        set: Option<&'static SystemSet>,
        after: &'static [&'static SystemDescriptor],
        access: &'static [Access],
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

    /// The access table.
    #[must_use]
    pub const fn access(&self) -> &'static [Access] {
        self.access
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
    pub run: fn(&mut Invocation<'_>),
    /// Runs the finalize function of a reduction, or does nothing.
    pub finalize: fn(&mut Invocation<'_>),
}

/// What a kernel entry point receives. Not part of the interface.
#[doc(hidden)]
pub struct Invocation<'w> {
    _private: core::marker::PhantomData<&'w mut ()>,
}
