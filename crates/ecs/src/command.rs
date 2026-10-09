//! Command buffers: structural changes recorded by a kernel and applied at the next sync
//! point (architecture section 5.2), and typed channels for other crates' commands.

use core::marker::PhantomData;

use lactose_core::StableKey;
use lactose_reflect::Component;

use crate::access::{Access, AccessKind, channel_target, no_target};
use crate::param::{SystemParam, sealed};
use crate::region::RegionId;

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
    _private: (),
}

macro_rules! tuple_bundles {
    ($(($($name:ident),+);)+) => {$(
        impl<$($name: Component),+> Bundle for ($($name,)+) {
            fn write_into(self, out: &mut BundleOut) {
                todo!()
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

/// The command buffer of one system invocation.
///
/// Commands apply at the sync point that ends the stage, in the stage's execution order,
/// then by (source origin key, sequence within the invocation) inside a system; a
/// per-region invocation sorts by its region before the entity sources, and a once-per-tick
/// invocation before those. The order is therefore the same on every build, platform,
/// worker count and layout.
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
    _buffer: PhantomData<&'w mut ()>,
}

impl Commands<'_> {
    /// Spawns an entity in the current region with `bundle`, and returns the placeholder
    /// that names it until apply. From a once-per-tick or exclusive system there is no
    /// current region: use [`spawn_in`](Self::spawn_in).
    ///
    /// At apply the entity receives its origin key. It is `Predicted` when the source entity
    /// has [`ClaimedBy`](crate::ClaimedBy), the player has an input tick this tick and the
    /// system's worlds include `prediction`: the player, that input tick and the spawn's
    /// ordinal among the player's predicted spawns this tick, in command order; the entity
    /// inherits the claim and carries [`PredictedOrigin`](crate::PredictedOrigin), and on
    /// the server a runtime `NetId` is allocated and bound to it as well (architecture
    /// section 7). Otherwise, on the server, the key is a runtime `NetId`; in a client or
    /// prediction world the spawn is dropped.
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> StableKey {
        todo!()
    }

    /// Spawns an entity in `region`, which must be loaded at apply, with the key rule of
    /// [`spawn`](Self::spawn).
    pub fn spawn_in<B: Bundle>(&mut self, region: RegionId, bundle: B) -> StableKey {
        todo!()
    }

    /// Despawns `key` at apply, and with it every entity it owns.
    pub fn despawn(&mut self, key: StableKey) {
        todo!()
    }

    /// Writes the whole component `value` to `key` at apply, adding the component if the
    /// entity lacks it. The row is marked changed. This is how a relationship field or
    /// [`ClaimedBy`](crate::ClaimedBy) is written, never through [`Mut`](crate::Mut), so the
    /// indexes stay consistent. [`InRegion`](crate::InRegion) and
    /// [`PredictedOrigin`](crate::PredictedOrigin) are the world's and cannot be set.
    pub fn set<T: Component>(&mut self, key: StableKey, value: T) {
        todo!()
    }

    /// Removes component `T` from `key` at apply, if present. Removing
    /// [`ClaimedBy`](crate::ClaimedBy) releases the claim.
    pub fn remove<T: Component>(&mut self, key: StableKey) {
        todo!()
    }

    /// Moves `key` and the entities it owns to `region` at apply. Identities do not change.
    /// A later command in the same apply may move an owned entity elsewhere again. A move
    /// into a `Frozen` region freezes the entity with it (architecture section 9.7).
    pub fn move_to_region(&mut self, key: StableKey, region: RegionId) {
        todo!()
    }

    /// The writer for channel `C`, for another crate's commands. `lactose-physics-api`
    /// wraps this in an extension trait so a kernel writes `cmd.physics().apply_impulse(..)`.
    pub fn channel<C: CommandChannel>(&mut self) -> ChannelWriter<'_, C> {
        todo!()
    }
}

impl sealed::Sealed for Commands<'_> {}

impl SystemParam for Commands<'_> {
    const ACCESS: Access = Access::new(AccessKind::Commands, no_target);
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
    _buffer: PhantomData<&'a mut ()>,
    _channel: PhantomData<fn() -> C>,
}

impl<C: CommandChannel> ChannelWriter<'_, C> {
    /// Records `entry` with this invocation's source and the next sequence number.
    pub fn push(&mut self, entry: C::Entry) {
        todo!()
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
    _buffer: PhantomData<&'w ()>,
    _channel: PhantomData<fn() -> C>,
}

impl<C: CommandChannel> Channel<'_, C> {
    /// The entries in command order.
    #[must_use]
    pub fn iter(&self) -> ChannelEntries<'_, C> {
        todo!()
    }

    /// How many entries there are.
    #[must_use]
    pub fn len(&self) -> usize {
        todo!()
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

/// Iterates a channel's entries in command order.
pub struct ChannelEntries<'a, C: CommandChannel> {
    _buffer: PhantomData<&'a ()>,
    _channel: PhantomData<fn() -> C>,
}

impl<'a, C: CommandChannel> Iterator for ChannelEntries<'a, C> {
    type Item = &'a ChannelEntry<C>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}
