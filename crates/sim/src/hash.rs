//! The world hash of architecture section 8.3 and its field-level diff, over the table of
//! monomorphised per-type hashers that the type registration fills.

use std::collections::BTreeMap;
use std::collections::btree_map;

use lactose_asset_api::VersionTable;
use lactose_core::{ComponentKey, FieldId, Result, StableKey};
use lactose_ecs::{Job, RegionId, RegionMode, World};
use lactose_reflect::{Reflect, Registry};

/// Hashes one value of one registered type: the row's origin key (`None` for a region
/// resource) and the value's bytes as laid out in memory, to the row hash of the README's
/// encoding. Monomorphised per type by [`TypeTable::register`], which is how the typed
/// `Reflect::visit` entry point survives the erased registry.
type ValueHasher = fn(Option<StableKey>, &[u8]) -> u64;

/// Appends one value's (field, field hash) pairs in field order, for [`RegionDigest`].
type ValueDigester = fn(&[u8], &mut Vec<(FieldId, u64)>);

/// The per-type hashers of every hashed type, keyed by component key. Built by a
/// [`TypeTable`], owned by the [`Simulation`](crate::Simulation), shared by every chunk
/// job of a hash.
#[derive(Clone, Default)]
pub struct Hashers {
    hashers: BTreeMap<ComponentKey, (ValueHasher, ValueDigester)>,
}

impl Hashers {
    /// Whether values of `component` are hashed: it was registered as persistent or
    /// transient. A derived type is registered but not covered.
    #[must_use]
    pub fn covers(&self, component: ComponentKey) -> bool {
        todo!()
    }

    /// The world hash of `region` (architecture section 8.3): every persistent and transient
    /// column and region resource, each row as (origin key, component key, field ID,
    /// canonical value), folded through an order-independent combiner, so the result is the
    /// same under every chunk layout and worker count. Derived columns and resources are
    /// excluded. The README states the encoding bit for bit.
    ///
    /// Fails with `NotFound` if the region is not loaded, and with `Unsupported` if a hashed
    /// column of the world is of a type this table does not cover, which cannot happen for
    /// a world built by the [`TypeTable`] these hashers came from.
    pub fn world_hash(&self, world: &World, region: RegionId) -> Result<u64> {
        todo!()
    }

    /// [`world_hash`](Self::world_hash) as chunk jobs: forks over
    /// [`World::columns_erased`](lactose_ecs::World::columns_erased) inside `job`'s scope,
    /// one child per chunk, and folds the children's sums. Bit-identical to the sequential
    /// form by construction. For an exclusive pass system that hashes at a sync point.
    pub fn world_hash_forked(
        &self,
        job: &mut Job<'_, '_>,
        world: &World,
        region: RegionId,
    ) -> Result<u64> {
        todo!()
    }

    /// The field-level digest of `region`: the hash of every hashed field of every row and
    /// resource, keyed by (origin key, component key, field ID), with the seed, mode and
    /// asset table beside them. [`RegionDigest::hash`] is exactly
    /// [`world_hash`](Self::world_hash) of the same state, so a digest taken on a mismatch
    /// explains the hash it stands for. Fails as `world_hash` does.
    pub fn digest(&self, world: &World, region: RegionId) -> Result<RegionDigest> {
        todo!()
    }
}

/// The registry and the hashers of every type of one runtime, filled by the registration
/// function of each crate (architecture section 18: generated functions, no
/// life-before-main) and consumed by [`SimulationBuilder::new`](crate::SimulationBuilder::new).
///
/// A game crate registers its types once, in one function, so the registry and the hash
/// table cannot disagree:
///
/// ```
/// use lactose_core::Result;
/// use lactose_reflect::component;
/// use lactose_sim::TypeTable;
///
/// #[component(key = "game::health", persistent, replicated)]
/// pub struct Health(#[field(id = 1)] pub i32);
///
/// /// Every type of this crate. `app` calls it, and the function it generates for the whole
/// /// game calls each crate's.
/// pub fn register_types(types: &mut TypeTable) -> Result<()> {
///     types.register::<Health>()?;
///     Ok(())
/// }
/// ```
pub struct TypeTable {
    registry: Registry,
    hashers: Hashers,
}

impl TypeTable {
    /// A table holding what every world needs: the components the world maintains
    /// ([`InRegion`](lactose_ecs::InRegion), [`OwnedBy`](lactose_ecs::OwnedBy),
    /// [`ClaimedBy`](lactose_ecs::ClaimedBy), [`PredictedOrigin`](lactose_ecs::PredictedOrigin))
    /// and this crate's own ([`TickDelta`](crate::TickDelta), [`CurrentTick`](crate::CurrentTick),
    /// [`TimeScale`](crate::TimeScale), [`RegionPolicy`](crate::RegionPolicy),
    /// [`StreamingAnchor`](crate::StreamingAnchor)).
    #[must_use]
    pub fn new() -> Self {
        todo!()
    }

    /// Registers `T` with the registry ([`Registry::register`]) and, if it is persistent or
    /// transient, builds its hasher: the monomorphised `Reflect::visit` of `T` under the
    /// hashing visitor. Registering a type again is a no-op.
    ///
    /// Fails as `Registry::register` does, and with `Unsupported` if a hashed `T` holds a
    /// raw `f32` or `f64` anywhere in its layout, or a scalar field whose type tag is not
    /// `lactose_math::Scalar`'s: the hasher trusts that one type and refuses the rest
    /// (`lactose_reflect::ScalarField`). A derived type may hold floats; it is not hashed.
    pub fn register<T: Reflect>(&mut self) -> Result<()> {
        todo!()
    }

    /// The registry so far.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The hashers so far.
    #[must_use]
    pub fn hashers(&self) -> &Hashers {
        &self.hashers
    }

    pub(crate) fn into_parts(self) -> (Registry, Hashers) {
        (self.registry, self.hashers)
    }
}

impl Default for TypeTable {
    fn default() -> Self {
        Self::new()
    }
}

/// The field-level view of one region's hashed state, from [`Hashers::digest`]: what a
/// SOUR mismatch is explained with (architecture section 8.3). Two digests of one region
/// taken on two machines are compared with [`diff`](Self::diff) after one travels as
/// [`encode`](Self::encode) bytes. Keyed by origin key, so it is the same under every
/// layout and the diff names entities, components and fields, never rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegionDigest {
    region: RegionId,
    seed: u64,
    mode: RegionMode,
    assets: Option<VersionTable>,
    /// `None` keys are region resources.
    fields: BTreeMap<FieldKey, u64>,
}

impl RegionDigest {
    /// The region.
    #[must_use]
    pub fn region(&self) -> RegionId {
        self.region
    }

    /// The region seed at the digest.
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The region mode at the digest.
    #[must_use]
    pub fn mode(&self) -> RegionMode {
        self.mode
    }

    /// The asset table at the digest, if one had entered.
    #[must_use]
    pub fn assets(&self) -> Option<&VersionTable> {
        self.assets.as_ref()
    }

    /// The hash of every hashed field, by (origin key, component key, field ID); a `None`
    /// key is a region resource. Ascending.
    #[must_use]
    pub fn fields(&self) -> DigestFields<'_> {
        DigestFields { fields: self.fields.iter() }
    }

    /// The world hash this digest stands for: equal to [`Hashers::world_hash`] of the state
    /// it was taken from.
    #[must_use]
    pub fn hash(&self) -> u64 {
        todo!()
    }

    /// Every field whose hash differs between the two digests, or exists on one side only,
    /// ascending by key. The seed, mode and asset table are compared by the caller through
    /// their accessors.
    #[must_use]
    pub fn diff<'a>(&'a self, other: &'a Self) -> Differences<'a> {
        todo!()
    }

    /// The canonical encoding, for sending a digest to the machine that holds the other
    /// side of a mismatch. Equal digests encode to equal bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        todo!()
    }

    /// Decodes [`encode`](Self::encode) bytes. Fails with `InvalidData` for anything that is
    /// not the canonical encoding of a digest.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        todo!()
    }
}

/// One field that differs between two digests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FieldDifference {
    /// The row's origin key, or `None` for a region resource.
    pub key: Option<StableKey>,
    /// The component or resource.
    pub component: ComponentKey,
    /// The field.
    pub field: FieldId,
    /// The hash on the left side, or `None` if the row or field is absent there.
    pub left: Option<u64>,
    /// The hash on the right side, or `None` if absent there.
    pub right: Option<u64>,
}

/// What a digest keys a field hash by.
type FieldKey = (Option<StableKey>, ComponentKey, FieldId);

/// Iterates a digest's field hashes, ascending by key.
pub struct DigestFields<'a> {
    fields: btree_map::Iter<'a, FieldKey, u64>,
}

impl Iterator for DigestFields<'_> {
    type Item = (Option<StableKey>, ComponentKey, FieldId, u64);

    fn next(&mut self) -> Option<Self::Item> {
        self.fields.next().map(|(&(key, component, field), &hash)| (key, component, field, hash))
    }
}

/// A cursor over one digest's fields.
type FieldCursor<'a> = core::iter::Peekable<btree_map::Iter<'a, FieldKey, u64>>;

/// Iterates the differences between two digests, ascending by key.
pub struct Differences<'a> {
    left: FieldCursor<'a>,
    right: FieldCursor<'a>,
}

impl Iterator for Differences<'_> {
    type Item = FieldDifference;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}
