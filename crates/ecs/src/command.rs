//! Command buffers: structural changes recorded by a kernel and applied at the next sync
//! point (architecture section 5.2), and typed channels for other crates' commands.

use core::any::{Any, TypeId};
use core::marker::PhantomData;

use lactose_core::{ComponentKey, EntityRef, FieldId, FixedHashMap, StableKey};
use lactose_reflect::{AssetReference, Component, ReferenceVisitor, Reflect, TypeInfo};

use crate::access::{Access, AccessKind, channel_target, no_target};
use crate::param::{SystemParam, sealed};
use crate::region::{InRegion, PredictedOrigin, RegionId};
use crate::system::{Fetch, Invocation, Slot, mismatch};

/// A set of components spawned together: a tuple of one to eight [`Component`]s, each of
/// a different type. One component is spelled `(Bomb { fuse: 180 },)`.
pub trait Bundle: Send + 'static {
    /// Appends the components to `out`.
    #[doc(hidden)]
    fn write_into(self, out: &mut BundleOut);
}

/// Where a bundle's components go. Not part of the interface.
#[doc(hidden)]
pub struct BundleOut {
    pub(crate) components: Vec<ErasedComponent>,
}

impl BundleOut {
    /// Appends `value`.
    pub fn push<T: Component>(&mut self, value: T) {
        self.components.push(ErasedComponent::of(value));
    }
}

macro_rules! tuple_bundles {
    ($(($($name:ident),+);)+) => {$(
        impl<$($name: Component),+> Bundle for ($($name,)+) {
            #[allow(non_snake_case, reason = "the bindings are named after the type parameters")]
            fn write_into(self, out: &mut BundleOut) {
                let ($($name,)+) = self;
                $(out.push($name);)+
            }
        }
    )+};
}

tuple_bundles! {
    (A);
    (A, B);
    (A, B, C);
    (A, B, C, D);
    (A, B, C, D, E);
    (A, B, C, D, E, F);
    (A, B, C, D, E, F, G);
    (A, B, C, D, E, F, G, H);
}

/// The components of a bundle, without their types.
pub(crate) fn erase_bundle<B: Bundle>(bundle: B) -> Vec<ErasedComponent> {
    let mut out = BundleOut { components: Vec::new() };
    bundle.write_into(&mut out);
    out.components
}

/// A component value kept typed until apply, because its references may still be
/// placeholders, which no stored value may hold: its bytes are not a valid value of the
/// type until they are resolved.
pub(crate) trait TypedComponent: Send + Sync {
    /// Calls `f` on every `EntityRef` of the value, through `visit_references`.
    fn remap(&mut self, f: &mut dyn FnMut(&mut EntityRef));

    fn bytes(&self) -> &[u8];
}

impl<T: Component> TypedComponent for T {
    fn remap(&mut self, f: &mut dyn FnMut(&mut EntityRef)) {
        struct Visitor<'a>(&'a mut dyn FnMut(&mut EntityRef));

        impl ReferenceVisitor for Visitor<'_> {
            fn entity_ref(&mut self, _: FieldId, value: &mut EntityRef) {
                (self.0)(value);
            }

            fn asset_ref<A: AssetReference>(&mut self, _: FieldId, _: &mut A) {}
        }

        self.visit_references(&mut Visitor(f));
    }

    fn bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

/// What a component value in transit holds.
pub(crate) enum Payload {
    /// A value from a bundle or a command, whose references are resolved at apply.
    Typed(Box<dyn TypedComponent>),
    /// Bytes from a document, references kept as stored.
    Bytes(Vec<u8>),
}

/// One component value in transit, its type known by its description.
pub(crate) struct ErasedComponent {
    pub(crate) info: &'static TypeInfo,
    pub(crate) payload: Payload,
}

impl ErasedComponent {
    pub(crate) fn of<T: Component>(value: T) -> Self {
        Self { info: T::SCHEMA.info(), payload: Payload::Typed(Box::new(value)) }
    }

    pub(crate) fn key(&self) -> ComponentKey {
        self.info.key()
    }

    /// The value's bytes; validate them before they are stored.
    pub(crate) fn bytes(&self) -> &[u8] {
        match &self.payload {
            Payload::Typed(value) => value.bytes(),
            Payload::Bytes(bytes) => bytes,
        }
    }
}

/// Whether `T` is one of the two components only the world writes.
pub(crate) fn is_worlds_own(key: ComponentKey) -> bool {
    key == InRegion::KEY || key == PredictedOrigin::KEY
}

/// One recorded command.
pub(crate) enum Command {
    Spawn { index: u32, region: Option<RegionId>, components: Vec<ErasedComponent> },
    Despawn(StableKey),
    Set(StableKey, ErasedComponent),
    Remove(StableKey, ComponentKey),
    Move(StableKey, RegionId),
    Channel(ChannelCommand),
}

/// A channel entry in a command buffer, with the typed operations its apply needs.
pub(crate) struct ChannelCommand {
    pub(crate) entry: Box<dyn Any + Send + Sync>,
    pub(crate) append: AppendFn,
}

/// Remaps an entry's keys and appends it to its channel's store.
pub(crate) type AppendFn = fn(
    &mut ChannelStore,
    Box<dyn Any + Send + Sync>,
    Option<StableKey>,
    &mut dyn FnMut(&mut StableKey),
);

/// Every channel's applied entries, as a `Vec<ChannelEntry<C>>` per channel type.
pub(crate) type ChannelStore = FixedHashMap<TypeId, Box<dyn Any + Send + Sync>>;

fn append_entry<C: CommandChannel>(
    store: &mut ChannelStore,
    entry: Box<dyn Any + Send + Sync>,
    source: Option<StableKey>,
    remap: &mut dyn FnMut(&mut StableKey),
) {
    let mut entry = *entry.downcast::<C::Entry>().expect("an entry of this channel");
    C::remap(&mut entry, remap);
    store
        .get_or_insert_with(TypeId::of::<C>(), || Box::new(Vec::<ChannelEntry<C>>::new()))
        .downcast_mut::<Vec<ChannelEntry<C>>>()
        .expect("the entries of this channel")
        .push(ChannelEntry { source, entry });
}

/// The commands of one invocation, in the order they were recorded.
#[derive(Default)]
pub(crate) struct CommandBuffer {
    pub(crate) commands: Vec<Command>,
    pub(crate) spawns: u32,
    /// The region a plain `spawn` goes to: the entity's or the per-region invocation's.
    pub(crate) region: Option<RegionId>,
}

impl CommandBuffer {
    pub(crate) fn new(region: Option<RegionId>) -> Self {
        Self { commands: Vec::new(), spawns: 0, region }
    }

    fn spawn(&mut self, region: Option<RegionId>, components: Vec<ErasedComponent>) -> StableKey {
        let index = self.spawns;
        self.spawns += 1;
        self.commands.push(Command::Spawn { index, region, components });
        StableKey::Placeholder(index)
    }
}

/// The command buffer of one system invocation.
///
/// Commands apply at the sync point that ends the stage, or at the closing sync point of
/// the tick for the last stage, in the stage's execution order, then by (source, sequence
/// within the invocation) inside a system, where sources order as: the once-per-tick
/// invocation, then per-region invocations by `RegionId`, then entity sources by origin
/// key. The order is therefore the same on every build, platform, worker count and layout.
///
/// A placeholder returned by [`spawn`](Self::spawn) or [`spawn_in`](Self::spawn_in) is
/// valid in this buffer only: in later commands of the same invocation, in component values
/// those commands carry and in channel entries (every `EntityRef` field and every channel
/// key is rewritten at apply), and as a key for [`Ctx::rng`](crate::Ctx::rng). It is not a
/// valid event target or presentation-event origin, since events are delivered before the
/// buffer applies; that is a debug assertion. A placeholder that reaches storage is a
/// broken invariant.
///
/// What happens to a command that cannot apply as written, in the order the buffer is
/// applied: a command against an entity that no longer exists is dropped; a `move_to_region`
/// to a region that is not loaded is dropped; a reference in a written component to an
/// entity that does not exist is written as the empty reference (architecture section 6,
/// "validated at apply"); a spawn with no region, or one that would need a `NetId` in a
/// client or prediction world, is dropped. Every drop is logged at target `casein`.
pub struct Commands<'w> {
    buffer: &'w mut CommandBuffer,
}

impl Commands<'_> {
    /// Spawns an entity in the current region with `bundle`, and returns the placeholder
    /// that names it until apply. From a once-per-tick or exclusive system there is no
    /// current region: use [`spawn_in`](Self::spawn_in).
    ///
    /// At apply the entity receives its origin key. It is `Predicted` when the source entity
    /// has [`ClaimedBy`](crate::ClaimedBy), the player has an input tick this tick and the
    /// system's worlds include both `server` and `prediction`: the player, that input tick
    /// and an ordinal that is the low 32 bits of xxh64 over (source origin key, system key,
    /// spawn index within the invocation), so no spawn the other machine cannot reproduce
    /// shifts it; the entity inherits the claim and carries
    /// [`PredictedOrigin`](crate::PredictedOrigin), and on the server a runtime `NetId` is
    /// allocated and bound to it as well (architecture section 7). A `Predicted` key that
    /// is already live is a collision: the spawn is then not predicted. Otherwise, on the
    /// server, the key is a runtime `NetId` and no claim passes on; in a client or
    /// prediction world the spawn is dropped.
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> StableKey {
        let region = self.buffer.region;
        self.buffer.spawn(region, erase_bundle(bundle))
    }

    /// Spawns an entity in `region`, which must be loaded at apply, with the key rule of
    /// [`spawn`](Self::spawn).
    pub fn spawn_in<B: Bundle>(&mut self, region: RegionId, bundle: B) -> StableKey {
        self.buffer.spawn(Some(region), erase_bundle(bundle))
    }

    /// Despawns `key` at apply, and with it every entity it owns.
    pub fn despawn(&mut self, key: StableKey) {
        self.buffer.commands.push(Command::Despawn(key));
    }

    /// Writes the whole component `value` to `key` at apply, adding the component if the
    /// entity lacks it. The row is marked changed. This is how a relationship field or
    /// [`ClaimedBy`](crate::ClaimedBy) is written, never through [`Mut`](crate::Mut), so the
    /// indexes stay consistent. [`InRegion`](crate::InRegion) and
    /// [`PredictedOrigin`](crate::PredictedOrigin) are the world's and cannot be set: that
    /// is a panic here.
    pub fn set<T: Component>(&mut self, key: StableKey, value: T) {
        assert!(!is_worlds_own(T::KEY), "`{}` is written by the world only", T::SCHEMA.name());
        self.buffer.commands.push(Command::Set(key, ErasedComponent::of(value)));
    }

    /// Removes component `T` from `key` at apply, if present. Removing
    /// [`ClaimedBy`](crate::ClaimedBy) releases the claim. [`InRegion`](crate::InRegion)
    /// and [`PredictedOrigin`](crate::PredictedOrigin) cannot be removed: that is a panic
    /// here.
    pub fn remove<T: Component>(&mut self, key: StableKey) {
        assert!(!is_worlds_own(T::KEY), "`{}` is written by the world only", T::SCHEMA.name());
        self.buffer.commands.push(Command::Remove(key, T::KEY));
    }

    /// Moves `key` and the entities it owns to `region` at apply. Identities do not change.
    /// A later command in the same apply may move an owned entity elsewhere again. A move
    /// into a `Frozen` region freezes the entity with it (architecture section 9.7).
    pub fn move_to_region(&mut self, key: StableKey, region: RegionId) {
        self.buffer.commands.push(Command::Move(key, region));
    }

    /// The writer for channel `C`, for another crate's commands. `lactose-physics-api`
    /// wraps this in an extension trait so a kernel writes `cmd.physics().apply_impulse(..)`.
    pub fn channel<C: CommandChannel>(&mut self) -> ChannelWriter<'_, C> {
        ChannelWriter { buffer: self.buffer, _channel: PhantomData }
    }
}

impl sealed::Sealed for Commands<'_> {}

impl SystemParam for Commands<'_> {
    const ACCESS: Access = Access::new(AccessKind::Commands, no_target);
}

impl<'r, 'j> Fetch<'r, 'j> for Commands<'r> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Commands(buffer) => Self { buffer },
            _ => mismatch(index),
        }
    }
}

/// A typed command channel another crate declares: a marker type naming its entry type and
/// how to rewrite the keys an entry holds.
///
/// Entries written through [`Commands::channel`] in one stage are readable through
/// [`Channel`] in every later stage of the same tick, and through
/// [`World::take_channel`](crate::World::take_channel) at a sync point, in command order,
/// with every placeholder resolved. The input sync point clears every channel.
pub trait CommandChannel: 'static {
    /// One command.
    type Entry: Send + Sync + 'static;

    /// Calls `f` on every entity key the entry holds, so the world can resolve placeholders
    /// at apply. An entry that holds a key and does not report it keeps a placeholder, which
    /// names nothing.
    fn remap(entry: &mut Self::Entry, f: &mut dyn FnMut(&mut StableKey));
}

/// Appends entries to a channel on behalf of one invocation.
pub struct ChannelWriter<'a, C: CommandChannel> {
    buffer: &'a mut CommandBuffer,
    _channel: PhantomData<fn() -> C>,
}

impl<C: CommandChannel> ChannelWriter<'_, C> {
    /// Records `entry` with this invocation's source and the next sequence number.
    pub fn push(&mut self, entry: C::Entry) {
        self.buffer.commands.push(Command::Channel(ChannelCommand {
            entry: Box::new(entry),
            append: append_entry::<C>,
        }));
    }
}

/// One entry of a channel, in command order, keys resolved.
pub struct ChannelEntry<C: CommandChannel> {
    /// The origin key of the entity whose kernel invocation wrote the entry, or `None` from
    /// a per-region, once-per-tick or exclusive system.
    pub source: Option<StableKey>,
    /// The command.
    pub entry: C::Entry,
}

impl<C: CommandChannel> Clone for ChannelEntry<C>
where
    C::Entry: Clone,
{
    fn clone(&self) -> Self {
        Self { source: self.source, entry: self.entry.clone() }
    }
}

impl<C: CommandChannel> core::fmt::Debug for ChannelEntry<C>
where
    C::Entry: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelEntry")
            .field("source", &self.source)
            .field("entry", &self.entry)
            .finish()
    }
}

/// The entries of channel `C` written in earlier stages of this tick, read by a system.
pub struct Channel<'w, C: CommandChannel> {
    entries: &'w [ChannelEntry<C>],
}

impl<C: CommandChannel> Channel<'_, C> {
    /// The entries in command order.
    #[must_use]
    pub fn iter(&self) -> ChannelEntries<'_, C> {
        ChannelEntries { entries: self.entries.iter() }
    }

    /// How many entries there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<C: CommandChannel> sealed::Sealed for Channel<'_, C> {}

impl<C: CommandChannel> SystemParam for Channel<'_, C> {
    const ACCESS: Access = Access::new(AccessKind::Read, channel_target::<C>);
}

impl<'r, 'j, C: CommandChannel> Fetch<'r, 'j> for Channel<'r, C> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Channel(store) => Self {
                entries: store.map_or(&[], |store| {
                    store
                        .downcast_ref::<Vec<ChannelEntry<C>>>()
                        .expect("the entries of this channel")
                        .as_slice()
                }),
            },
            _ => mismatch(index),
        }
    }
}

/// Iterates a channel's entries in command order.
pub struct ChannelEntries<'a, C: CommandChannel> {
    entries: core::slice::Iter<'a, ChannelEntry<C>>,
}

impl<'a, C: CommandChannel> Iterator for ChannelEntries<'a, C> {
    type Item = &'a ChannelEntry<C>;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next()
    }
}
