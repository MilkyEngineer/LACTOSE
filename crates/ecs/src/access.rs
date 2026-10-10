//! The access table: what a system parameter reads or writes, as the `#[system]` expansion
//! records it in a [`SystemDescriptor`](crate::SystemDescriptor) at compile time.

use core::any::TypeId;

use lactose_core::ComponentKey;

/// One entry of a system's access table, taken from one parameter.
///
/// Entries are constants: every [`SystemParam`](crate::SystemParam) has one as
/// [`SystemParam::ACCESS`](crate::SystemParam::ACCESS), and the `#[system]` expansion lists
/// them in a static. The target is evaluated at schedule build, because a resource or event
/// is keyed by its `TypeId`, which is not a constant.
#[derive(Clone, Copy, Debug)]
pub struct Access {
    kind: AccessKind,
    target: fn() -> AccessTarget,
}

impl Access {
    /// An entry of `kind` whose target `target` yields when evaluated.
    #[must_use]
    pub const fn new(kind: AccessKind, target: fn() -> AccessTarget) -> Self {
        Self { kind, target }
    }

    /// How the parameter touches its target.
    #[must_use]
    pub const fn kind(&self) -> AccessKind {
        self.kind
    }

    /// What the parameter touches.
    #[must_use]
    pub fn target(&self) -> AccessTarget {
        (self.target)()
    }
}

/// How a parameter touches its target. Two systems conflict when one writes a target the
/// other reads or writes; the schedule requires an order between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum AccessKind {
    /// Reads every row or the resource: `&T`, [`Res`](crate::Res),
    /// [`RegionRes`](crate::RegionRes), [`Channel`](crate::Channel).
    Read,
    /// Writes: [`Mut`](crate::Mut), [`ResMut`](crate::ResMut).
    Write,
    /// Reads rows that have the component and skips none: `Option<&T>`.
    OptionalRead,
    /// Writes rows that have the component and skips none: `Option<Mut<T>>`.
    OptionalWrite,
    /// Requires the component without reading it: [`With`](crate::With).
    With,
    /// Requires the component's absence: [`Without`](crate::Without).
    Without,
    /// Requires a change since the system last ran, which reads the row's change stamp:
    /// [`Changed`](crate::Changed).
    Changed,
    /// Reads the events of a type: [`Events`](crate::Events). The reader must be ordered
    /// after every emitter of the type in the same schedule.
    ReadEvents,
    /// Emits events of a type: [`EventWriter`](crate::EventWriter).
    EmitEvents,
    /// Records commands for the next sync point: [`Commands`](crate::Commands).
    Commands,
    /// Emits presentation events: [`PresentEvents`](crate::PresentEvents).
    PresentEvents,
    /// The entity's origin key: [`StableKeyOf`](crate::StableKeyOf). Only in a system with
    /// a component parameter.
    Entity,
    /// The region's context: [`Ctx`](crate::Ctx). In a system with a component parameter,
    /// or in one without, which then runs once per `Simulated` region.
    Region,
    /// The executor's job context: [`Job`](crate::Job). Only in a system that runs once per
    /// tick, an exclusive system or a finalize function.
    Job,
    /// The whole world: `&mut World` in an exclusive system.
    World,
    /// Writes a column outside the parameter list, through `Commands::set` or, in an
    /// exclusive system, through `&mut World`: what `#[system(writes = [T, ..])]` declares.
    /// No parameter carries it, so nothing is fetched for it. The schedule counts it as a
    /// write of the column, for ordering conflicts (with readers and `Mut`, not with another
    /// `Writes` of the column) and as a writer of a transient column: an exclusive system's
    /// from its own stage, any other's from the next. It is trusted, never checked.
    Writes,
}

/// What a parameter touches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum AccessTarget {
    /// A component column, by its stable key.
    Component(ComponentKey),
    /// A world resource, reflected or service, by its Rust type.
    Resource {
        /// The resource's `TypeId`; for a service resource that of the `dyn` trait type.
        type_id: TypeId,
        /// `core::any::type_name` of the resource, for diagnostics only.
        name: &'static str,
    },
    /// A region resource, by its stable key.
    RegionResource(ComponentKey),
    /// An event type.
    Events {
        /// The event's `TypeId`.
        type_id: TypeId,
        /// `core::any::type_name` of the event, for diagnostics only.
        name: &'static str,
    },
    /// A command channel.
    Channel {
        /// The channel type's `TypeId`.
        type_id: TypeId,
        /// `core::any::type_name` of the channel, for diagnostics only.
        name: &'static str,
    },
    /// The whole world, which two exclusive systems both hold: what they conflict on.
    World,
    /// No target beyond the kind itself.
    None,
}

/// The target function of a component parameter.
pub(crate) fn component_target<T: lactose_reflect::Component>() -> AccessTarget {
    AccessTarget::Component(T::KEY)
}

/// The entry `writes = [T]` adds to a system's table. Not part of the interface: the
/// `#[system]` expansion names it.
#[doc(hidden)]
#[must_use]
pub const fn writes<T: lactose_reflect::Component>() -> Access {
    Access::new(AccessKind::Writes, component_target::<T>)
}

/// The target function of a resource parameter; `T` may be a `dyn` trait type.
pub(crate) fn resource_target<T: ?Sized + 'static>() -> AccessTarget {
    AccessTarget::Resource { type_id: TypeId::of::<T>(), name: core::any::type_name::<T>() }
}

/// The target function of a region resource parameter.
pub(crate) fn region_resource_target<T: lactose_reflect::Resource>() -> AccessTarget {
    AccessTarget::RegionResource(T::KEY)
}

/// The target function of an event parameter.
pub(crate) fn events_target<E: 'static>() -> AccessTarget {
    AccessTarget::Events { type_id: TypeId::of::<E>(), name: core::any::type_name::<E>() }
}

/// The target function of a channel parameter.
pub(crate) fn channel_target<C: 'static>() -> AccessTarget {
    AccessTarget::Channel { type_id: TypeId::of::<C>(), name: core::any::type_name::<C>() }
}

/// The target function of a parameter with no target.
pub(crate) fn no_target() -> AccessTarget {
    AccessTarget::None
}
