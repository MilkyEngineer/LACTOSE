//! The world hash of architecture section 8.3 and its field-level diff, over the table of
//! monomorphised per-type hashers that the type registration fills.

use core::any::TypeId;
use std::collections::BTreeMap;
use std::collections::btree_map;

use lactose_asset_api::{AssetId, AssetVersion, VersionTable};
use lactose_core::{ComponentKey, EntityRef, Error, ErrorKind, FieldId, Result, StableKey, xxh64};
use lactose_ecs::{ClaimedBy, ErasedColumn, InRegion, Job, OwnedBy, PredictedOrigin};
use lactose_ecs::{RegionId, RegionMode, World};
use lactose_math::Scalar;
use lactose_reflect::bytemuck;
use lactose_reflect::{
    AssetReference, Field, FieldInfo, FieldKind, FieldVisitor, Persistence, Reflect, Registry,
    ScalarField, TypeRole,
};

use crate::region::{RegionPolicy, StreamingAnchor};
use crate::time::{CurrentTick, TickDelta, TimeScale};

/// Hashes one value of one registered type: the row's origin key (`None` for a region
/// resource), the value's bytes as laid out in memory, and a scratch buffer for the
/// field streams so a row allocates nothing, to the row hash of the README's encoding.
/// Monomorphised per type by [`TypeTable::register`], which is how the typed
/// `Reflect::visit` entry point survives the erased registry.
type ValueHasher = fn(Option<StableKey>, &[u8], &mut Vec<u8>) -> u64;

/// Appends one value's (field, field hash) pairs in field order, for [`RegionDigest`].
type ValueDigester = fn(&[u8], &mut Vec<u8>, &mut Vec<(FieldId, u64)>);

/// Whether a registered resource type is present in a world: the probe
/// [`SimulationBuilder::build`](crate::SimulationBuilder::build) runs for every resource
/// type whose policy is not `derived`, to refuse it.
pub(crate) type ResourcePresent = fn(&World) -> bool;

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
        self.hashers.contains_key(&component)
    }

    /// The world hash of `region` (architecture section 8.3): every persistent and transient
    /// column and region resource, each row as (origin key, component key, field ID,
    /// canonical value), folded through an order-independent combiner, so the result is the
    /// same under every chunk layout and worker count; with the region's seed, mode and
    /// asset table. Derived columns and resources are excluded. The README states the
    /// encoding bit for bit.
    ///
    /// Fails with `NotFound` if the region is not loaded, and with `Unsupported` if a hashed
    /// column of the world is of a type this table does not cover, which cannot happen for
    /// a world built by the [`TypeTable`] these hashers came from.
    pub fn world_hash(&self, world: &World, region: RegionId) -> Result<u64> {
        let header = self.check(world, region)?;
        let mut scratch = Vec::new();
        let mut columns = BTreeMap::new();
        for (&component, &(hasher, _)) in &self.hashers {
            for column in world.columns_erased(component) {
                let (count, sum) = column_sum(&column, region, hasher, &mut scratch);
                fold(&mut columns, component, count, sum);
            }
        }
        let resources = self.resource_hashes(world, region, &mut scratch);
        Ok(header.hash(&columns, &resources))
    }

    /// [`world_hash`](Self::world_hash) as chunk jobs: forks over
    /// [`World::columns_erased`](lactose_ecs::World::columns_erased) inside `job`'s scope,
    /// one child per chunk, and folds the children's sums. Bit-identical to the sequential
    /// form by construction. For this crate's own pass systems.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the forked form is for a hash pass at a sync point, which no milestone 1 \
                      caller runs yet; the crate's unit tests hold it to the sequential form"
        )
    )]
    pub(crate) fn world_hash_forked(
        &self,
        job: &mut Job<'_, '_>,
        world: &World,
        region: RegionId,
    ) -> Result<u64> {
        let header = self.check(world, region)?;
        let chunks: Vec<(ComponentKey, ValueHasher, ErasedColumn<'_>)> = self
            .hashers
            .iter()
            .flat_map(|(&component, &(hasher, _))| {
                world.columns_erased(component).map(move |column| (component, hasher, column))
            })
            .collect();
        // One cell per chunk: a child writes only its own, and the sums are folded by
        // addition afterwards, so the order the children ran in decides nothing.
        let mut cells = vec![(0_u32, 0_u64); chunks.len()];
        job.context().scope(|scope| {
            for (cell, (_, hasher, column)) in cells.iter_mut().zip(&chunks) {
                scope.fork(move |_| {
                    let mut scratch = Vec::new();
                    *cell = column_sum(column, region, *hasher, &mut scratch);
                });
            }
        });
        let mut columns = BTreeMap::new();
        for ((component, _, _), (count, sum)) in chunks.iter().zip(cells) {
            fold(&mut columns, *component, count, sum);
        }
        let resources = self.resource_hashes(world, region, &mut Vec::new());
        Ok(header.hash(&columns, &resources))
    }

    /// The field-level digest of `region`: every hashed row and resource with the hash of
    /// each of its fields, keyed by (origin key, component key), with the seed, mode and
    /// asset table beside them. [`RegionDigest::hash`] is exactly
    /// [`world_hash`](Self::world_hash) of the same state, so a digest taken on a mismatch
    /// explains the hash it stands for. Fails as `world_hash` does.
    pub fn digest(&self, world: &World, region: RegionId) -> Result<RegionDigest> {
        let header = self.check(world, region)?;
        let mut scratch = Vec::new();
        let mut rows = BTreeMap::new();
        for (&component, &(_, digester)) in &self.hashers {
            for column in world.columns_erased(component) {
                let size = column.element_size();
                for (row, (&key, &at)) in column.keys().iter().zip(column.regions()).enumerate() {
                    if at == region {
                        let mut fields = Vec::new();
                        digester(&column.bytes()[row * size..][..size], &mut scratch, &mut fields);
                        rows.insert((Some(key), component), fields);
                    }
                }
            }
        }
        for (component, bytes) in world.region_resources_erased(region) {
            if let Some(&(_, digester)) = self.hashers.get(&component) {
                let mut fields = Vec::new();
                digester(bytes, &mut scratch, &mut fields);
                rows.insert((None, component), fields);
            }
        }
        Ok(RegionDigest {
            region,
            seed: header.seed,
            mode: header.mode,
            assets: header.assets,
            rows,
        })
    }

    /// The region's header, after checking that it is loaded and that every hashed type
    /// its rows and resources hold is covered.
    fn check(&self, world: &World, region: RegionId) -> Result<RegionHeader> {
        let info = world
            .region(region)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "the region is not loaded"))?;
        let uncovered = |component: ComponentKey| {
            world.registry().get(component).is_some_and(|info| {
                info.persistence() != Persistence::Derived && !self.covers(component)
            })
        };
        let unsupported = |name: &str| {
            Err(Error::new(
                ErrorKind::Unsupported,
                format!("`{name}` is hashed, but these hashers do not cover it"),
            ))
        };
        for info in world.registry().iter() {
            if info.role() == TypeRole::Component
                && uncovered(info.key())
                && world.columns_erased(info.key()).any(|column| column.regions().contains(&region))
            {
                return unsupported(info.name());
            }
        }
        for (component, _) in world.region_resources_erased(region) {
            if uncovered(component) {
                let name = world.registry().get(component).map_or("?", |info| info.name());
                return unsupported(name);
            }
        }
        Ok(RegionHeader {
            region,
            seed: info.seed,
            mode: info.mode,
            assets: world.region_assets(region).map(|table| table.table().clone()),
        })
    }

    /// The hash of every hashed resource of `region`, ascending by key.
    fn resource_hashes(
        &self,
        world: &World,
        region: RegionId,
        scratch: &mut Vec<u8>,
    ) -> Vec<(ComponentKey, u64)> {
        world
            .region_resources_erased(region)
            .filter_map(|(component, bytes)| {
                let &(hasher, _) = self.hashers.get(&component)?;
                Some((component, hasher(None, bytes, scratch)))
            })
            .collect()
    }
}

/// The row count and the wrapping sum of the row hashes of `column`'s rows in `region`.
fn column_sum(
    column: &ErasedColumn<'_>,
    region: RegionId,
    hasher: ValueHasher,
    scratch: &mut Vec<u8>,
) -> (u32, u64) {
    let size = column.element_size();
    let mut count = 0_u32;
    let mut sum = 0_u64;
    for (row, (&key, &at)) in column.keys().iter().zip(column.regions()).enumerate() {
        if at == region {
            count += 1;
            sum =
                sum.wrapping_add(hasher(Some(key), &column.bytes()[row * size..][..size], scratch));
        }
    }
    (count, sum)
}

/// Adds one chunk's count and sum into the column sums.
fn fold(
    columns: &mut BTreeMap<ComponentKey, (u32, u64)>,
    component: ComponentKey,
    count: u32,
    sum: u64,
) {
    if count > 0 {
        let entry = columns.entry(component).or_insert((0, 0));
        entry.0 += count;
        entry.1 = entry.1.wrapping_add(sum);
    }
}

/// What the region hash covers besides its columns and resources.
struct RegionHeader {
    region: RegionId,
    seed: u64,
    mode: RegionMode,
    assets: Option<VersionTable>,
}

impl RegionHeader {
    /// The region hash of the README's encoding, over `columns` (count and sum by
    /// component, only components some row has) and `resources` (hash by key, ascending).
    fn hash(
        &self,
        columns: &BTreeMap<ComponentKey, (u32, u64)>,
        resources: &[(ComponentKey, u64)],
    ) -> u64 {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.seed.to_le_bytes());
        bytes.push(mode_code(self.mode));
        encode_assets(&mut bytes, self.assets.as_ref());
        for (component, &(count, sum)) in columns {
            bytes.extend_from_slice(&component.0.to_le_bytes());
            bytes.extend_from_slice(&count.to_le_bytes());
            bytes.extend_from_slice(&sum.to_le_bytes());
        }
        for (component, hash) in resources {
            bytes.extend_from_slice(&component.0.to_le_bytes());
            bytes.extend_from_slice(&hash.to_le_bytes());
        }
        xxh64(&bytes, self.region.0)
    }
}

/// The mode as the region hash and the digest encode it.
const fn mode_code(mode: RegionMode) -> u8 {
    match mode {
        RegionMode::Simulated => 0,
        RegionMode::Replicated => 1,
        RegionMode::Frozen => 2,
    }
}

/// The asset table as the region hash and the digest encode it: a presence flag, then the
/// entry count and each entry in ascending ID order.
fn encode_assets(bytes: &mut Vec<u8>, assets: Option<&VersionTable>) {
    match assets {
        None => bytes.push(0),
        Some(table) => {
            bytes.push(1);
            let count =
                u32::try_from(table.len()).expect("an asset table holds fewer than 2^32 entries");
            bytes.extend_from_slice(&count.to_le_bytes());
            for (id, version) in table.iter() {
                bytes.extend_from_slice(&id.to_bits().to_le_bytes());
                bytes.extend_from_slice(&version.to_bits().to_le_bytes());
            }
        }
    }
}

/// The sixteen key bytes of a row: the memory bytes of its `EntityRef`, or of
/// `EntityRef::NONE` for a region resource.
fn key_bytes(key: Option<StableKey>) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes.copy_from_slice(bytemuck::bytes_of(&EntityRef::from(key)));
    bytes
}

/// The row hash from the key and the field hashes: what [`ValueHasher`] computes and what
/// [`RegionDigest::hash`] recomputes.
fn row_hash(key: Option<StableKey>, component: ComponentKey, fields: &[(FieldId, u64)]) -> u64 {
    let mut bytes = Vec::with_capacity(16 + 10 * fields.len());
    bytes.extend_from_slice(&key_bytes(key));
    for (id, hash) in fields {
        bytes.extend_from_slice(&id.0.to_le_bytes());
        bytes.extend_from_slice(&hash.to_le_bytes());
    }
    xxh64(&bytes, component.0)
}

/// The hashing visitor: appends each leaf's bytes to the stream of the top-level field it
/// is reported under, and closes each top-level field with an ID, in declaration order,
/// into its field hash.
///
/// `scratch` holds the row stream (the key bytes, then each closed field's ID and hash)
/// followed by the stream of the field being visited, from `start`. `Reflect::visit`
/// reports fields in declaration order, so a field is closed when the next one starts, and
/// a field that reports no leaf hashes its empty stream.
struct FieldHasher<'a> {
    scratch: &'a mut Vec<u8>,
    start: usize,
    declared: core::slice::Iter<'static, FieldInfo>,
    current: Option<FieldId>,
    /// Where the digest collects (field, field hash) pairs, if one is being taken.
    fields: Option<&'a mut Vec<(FieldId, u64)>>,
}

impl<'a> FieldHasher<'a> {
    fn new(
        scratch: &'a mut Vec<u8>,
        key: Option<StableKey>,
        declared: &'static [FieldInfo],
        fields: Option<&'a mut Vec<(FieldId, u64)>>,
    ) -> Self {
        scratch.clear();
        scratch.extend_from_slice(&key_bytes(key));
        let start = scratch.len();
        Self { scratch, start, declared: declared.iter(), current: None, fields }
    }

    /// Makes `id` the field being visited, closing the one before and every field with an
    /// ID declared between them.
    fn enter(&mut self, id: FieldId) {
        if self.current == Some(id) {
            return;
        }
        self.close();
        while let Some(field) = self.declared.next() {
            match field.id() {
                Some(next) if next == id => {
                    self.current = Some(id);
                    return;
                }
                Some(next) => self.emit(next, xxh64(&[], u64::from(next.0))),
                None => {}
            }
        }
        panic!("`Reflect::visit` reported field {} out of declaration order", id.0);
    }

    fn close(&mut self) {
        if let Some(id) = self.current.take() {
            let hash = xxh64(&self.scratch[self.start..], u64::from(id.0));
            self.scratch.truncate(self.start);
            self.emit(id, hash);
        }
    }

    fn emit(&mut self, id: FieldId, hash: u64) {
        self.scratch.extend_from_slice(&id.0.to_le_bytes());
        self.scratch.extend_from_slice(&hash.to_le_bytes());
        self.start = self.scratch.len();
        if let Some(fields) = self.fields.as_deref_mut() {
            fields.push((id, hash));
        }
    }

    /// Closes the last field and every field with an ID after it; the row stream is then
    /// the whole of `scratch`.
    fn finish(mut self) {
        self.close();
        while let Some(field) = self.declared.next() {
            if let Some(id) = field.id() {
                self.emit(id, xxh64(&[], u64::from(id.0)));
            }
        }
    }

    fn leaf(&mut self, id: FieldId, bytes: &[u8]) {
        self.enter(id);
        self.scratch.extend_from_slice(bytes);
    }
}

impl FieldVisitor for FieldHasher<'_> {
    fn bytes(&mut self, id: FieldId, bytes: &[u8]) {
        self.leaf(id, bytes);
    }

    fn scalar<S: ScalarField>(&mut self, id: FieldId, value: &S) {
        // Registration refused any scalar tag but the engine's; this refuses any type that
        // carries the tag without being it.
        assert!(
            TypeId::of::<S>() == TypeId::of::<Scalar>(),
            "the world hash trusts `lactose_math::Scalar` alone as a scalar"
        );
        self.leaf(id, &value.canonical_bits().to_le_bytes());
    }

    fn f32(&mut self, _: FieldId, _: f32) {
        unreachable!("registration refuses a hashed type that holds a raw float");
    }

    fn f64(&mut self, _: FieldId, _: f64) {
        unreachable!("registration refuses a hashed type that holds a raw float");
    }

    fn entity_ref(&mut self, id: FieldId, value: EntityRef) {
        self.leaf(id, bytemuck::bytes_of(&value));
    }

    fn asset_ref<A: AssetReference>(&mut self, id: FieldId, value: &A) {
        self.leaf(id, &value.asset_id().to_le_bytes());
    }
}

/// The [`ValueHasher`] of `T`.
fn hash_value<T: Reflect>(key: Option<StableKey>, bytes: &[u8], scratch: &mut Vec<u8>) -> u64 {
    // Storage holds only values its type's validation accepted.
    let value: T = bytemuck::checked::pod_read_unaligned(bytes);
    let mut hasher = FieldHasher::new(scratch, key, T::SCHEMA.info().layout().fields(), None);
    value.visit(&mut hasher);
    hasher.finish();
    xxh64(scratch, T::KEY.0)
}

/// The [`ValueDigester`] of `T`.
fn digest_value<T: Reflect>(bytes: &[u8], scratch: &mut Vec<u8>, fields: &mut Vec<(FieldId, u64)>) {
    let value: T = bytemuck::checked::pod_read_unaligned(bytes);
    let declared = T::SCHEMA.info().layout().fields();
    let mut hasher = FieldHasher::new(scratch, None, declared, Some(fields));
    value.visit(&mut hasher);
    hasher.finish();
}

/// The [`ResourcePresent`] probe of `T`. A reflected resource and a service are held
/// alike, by type, so the service lookup finds a reflected resource of type `T` too.
fn resource_present<T: 'static>(world: &World) -> bool {
    world.service::<T>().is_some()
}

/// Refuses a hashed layout that holds a raw float, or a scalar the hasher does not trust.
fn check_hashable(name: &str, fields: &[FieldInfo]) -> Result<()> {
    for field in fields {
        check_kind(name, field.kind(), field.type_name())?;
    }
    Ok(())
}

fn check_kind(name: &str, kind: FieldKind, tag: &str) -> Result<()> {
    let refuse = |what: &str| {
        Err(Error::new(
            ErrorKind::Unsupported,
            format!(
                "`{name}` is hashed but holds {what}; make it `derived` or use `lactose_math::Scalar`"
            ),
        ))
    };
    match kind {
        FieldKind::Float => refuse("a raw float"),
        FieldKind::Scalar { .. } if tag != <Scalar as Field>::NAME => {
            refuse(&format!("the scalar `{tag}`"))
        }
        FieldKind::Array { element, element_name, .. } => check_kind(name, *element, element_name),
        FieldKind::Struct(info) => check_hashable(name, info.fields()),
        _ => Ok(()),
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
/// use lactose_reflect::{component, resource};
/// use lactose_sim::TypeTable;
///
/// #[component(key = "game::health", persistent, replicated)]
/// pub struct Health(#[field(id = 1)] pub i32);
///
/// /// Configuration: a world resource, so `derived` (the README says why).
/// #[resource(key = "game::gravity", derived)]
/// pub struct Gravity(#[field(id = 1)] pub lactose_math::Scalar);
///
/// /// Every type of this crate. `app` calls it, and the function it generates for the whole
/// /// game calls each crate's.
/// pub fn register_types(types: &mut TypeTable) -> Result<()> {
///     types.register::<Health>()?;
///     types.register::<Gravity>()?;
///     Ok(())
/// }
/// ```
pub struct TypeTable {
    registry: Registry,
    hashers: Hashers,
    /// One probe per registered resource type whose policy is not `derived`.
    probes: Vec<(ComponentKey, ResourcePresent)>,
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
        let mut table =
            Self { registry: Registry::new(), hashers: Hashers::default(), probes: Vec::new() };
        let registered = [
            table.register::<InRegion>(),
            table.register::<OwnedBy>(),
            table.register::<ClaimedBy>(),
            table.register::<PredictedOrigin>(),
            table.register::<TickDelta>(),
            table.register::<CurrentTick>(),
            table.register::<TimeScale>(),
            table.register::<RegionPolicy>(),
            table.register::<StreamingAnchor>(),
        ];
        for result in registered {
            result.expect("the world's and this crate's own types register");
        }
        table
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
        if self.registry.contains::<T>() {
            return Ok(());
        }
        let info = T::SCHEMA.info();
        let hashed = info.persistence() != Persistence::Derived;
        if hashed {
            check_hashable(info.name(), info.layout().fields())?;
        }
        self.registry.register::<T>()?;
        if hashed {
            self.hashers.hashers.insert(T::KEY, (hash_value::<T>, digest_value::<T>));
            if info.role() == TypeRole::Resource {
                self.probes.push((T::KEY, resource_present::<T>));
            }
        }
        Ok(())
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

    pub(crate) fn into_parts(self) -> (Registry, Hashers, Vec<(ComponentKey, ResourcePresent)>) {
        (self.registry, self.hashers, self.probes)
    }
}

impl Default for TypeTable {
    fn default() -> Self {
        Self::new()
    }
}

/// What a digest keys a row by: the origin key, or `None` for a region resource, and the
/// component or resource.
type RowKey = (Option<StableKey>, ComponentKey);

/// The field-level view of one region's hashed state, from [`Hashers::digest`]: what a
/// SOUR mismatch is explained with (architecture section 8.3). Two digests of one region
/// taken on two machines are compared with [`diff`](Self::diff) after one travels as
/// [`encode`](Self::encode) bytes. Keyed by origin key, so it is the same under every
/// layout and the diff names entities, components and fields, never rows of storage.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegionDigest {
    region: RegionId,
    seed: u64,
    mode: RegionMode,
    assets: Option<VersionTable>,
    /// Every hashed row and resource, with its field hashes in field order; an empty list
    /// is a row whose type has no field with an ID, present all the same.
    rows: BTreeMap<RowKey, Vec<(FieldId, u64)>>,
}

/// The first bytes of a digest's encoding, then its version.
const DIGEST_MAGIC: [u8; 4] = *b"LSDG";
const DIGEST_VERSION: u8 = 1;

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

    /// Every hashed row and resource, ascending by (origin key, component key), each with
    /// its field hashes in field order; a `None` key is a region resource.
    #[must_use]
    pub fn rows(&self) -> DigestRows<'_> {
        DigestRows { rows: self.rows.iter() }
    }

    /// The world hash this digest stands for: equal to [`Hashers::world_hash`] of the state
    /// it was taken from.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut columns = BTreeMap::new();
        let mut resources = Vec::new();
        for (&(key, component), fields) in &self.rows {
            let hash = row_hash(key, component, fields);
            match key {
                Some(_) => fold(&mut columns, component, 1, hash),
                // `None` sorts first, so resources arrive ascending by key.
                None => resources.push((component, hash)),
            }
        }
        let header = RegionHeader {
            region: self.region,
            seed: self.seed,
            mode: self.mode,
            assets: self.assets.clone(),
        };
        header.hash(&columns, &resources)
    }

    /// Every difference between the two digests, ascending by key: a row present on one
    /// side only (`field: None`), or a field whose hash differs or exists on one side only.
    /// The seed, mode and asset table are compared by the caller through their accessors.
    #[must_use]
    pub fn diff<'a>(&'a self, other: &'a Self) -> Differences<'a> {
        Differences {
            left: self.rows.iter().peekable(),
            right: other.rows.iter().peekable(),
            pending: Vec::new().into_iter(),
        }
    }

    /// The canonical encoding, for sending a digest to the machine that holds the other
    /// side of a mismatch. Equal digests encode to equal bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&DIGEST_MAGIC);
        bytes.push(DIGEST_VERSION);
        bytes.extend_from_slice(&self.region.0.to_le_bytes());
        bytes.extend_from_slice(&self.seed.to_le_bytes());
        bytes.push(mode_code(self.mode));
        encode_assets(&mut bytes, self.assets.as_ref());
        let count = u32::try_from(self.rows.len()).expect("a region holds fewer than 2^32 rows");
        bytes.extend_from_slice(&count.to_le_bytes());
        for (&(key, component), fields) in &self.rows {
            bytes.extend_from_slice(&key_bytes(key));
            bytes.extend_from_slice(&component.0.to_le_bytes());
            let count = u16::try_from(fields.len()).expect("a type has at most 2^16 field IDs");
            bytes.extend_from_slice(&count.to_le_bytes());
            for (id, hash) in fields {
                bytes.extend_from_slice(&id.0.to_le_bytes());
                bytes.extend_from_slice(&hash.to_le_bytes());
            }
        }
        bytes
    }

    /// Decodes [`encode`](Self::encode) bytes. Fails with `InvalidData` for anything that is
    /// not the canonical encoding of a digest.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader { bytes };
        if reader.take(4)? != DIGEST_MAGIC || reader.u8()? != DIGEST_VERSION {
            return Err(not_a_digest());
        }
        let region = RegionId(reader.u64()?);
        let seed = reader.u64()?;
        let mode = match reader.u8()? {
            0 => RegionMode::Simulated,
            1 => RegionMode::Replicated,
            2 => RegionMode::Frozen,
            _ => return Err(not_a_digest()),
        };
        let assets = match reader.u8()? {
            0 => None,
            1 => {
                let mut table = VersionTable::new();
                for _ in 0..reader.u32()? {
                    let id = AssetId::from_bits(reader.u64()?);
                    let version =
                        AssetVersion::from_bits(reader.u64()?).ok_or_else(not_a_digest)?;
                    table.set(id, version);
                }
                Some(table)
            }
            _ => return Err(not_a_digest()),
        };
        let mut rows = BTreeMap::new();
        for _ in 0..reader.u32()? {
            let key: EntityRef = bytemuck::checked::try_pod_read_unaligned(reader.take(16)?)
                .map_err(|_| not_a_digest())?;
            let component = ComponentKey(reader.u64()?);
            let mut fields = Vec::new();
            for _ in 0..reader.u16()? {
                fields.push((FieldId(reader.u16()?), reader.u64()?));
            }
            rows.insert((key.get(), component), fields);
        }
        let digest = Self { region, seed, mode, assets, rows };
        // Anything the parse accepted that is not canonical (rows out of order or twice, a
        // table entry out of order, twice or empty, trailing bytes) encodes differently.
        if reader.bytes.is_empty() && digest.encode() == bytes {
            Ok(digest)
        } else {
            Err(not_a_digest())
        }
    }
}

fn not_a_digest() -> Error {
    Error::new(ErrorKind::InvalidData, "not the canonical encoding of a region digest")
}

/// Reads a digest's encoding front to back.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < len {
            return Err(not_a_digest());
        }
        let (head, tail) = self.bytes.split_at(len);
        self.bytes = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two bytes")))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }
}

/// Iterates a digest's rows, ascending by key.
pub struct DigestRows<'a> {
    rows: btree_map::Iter<'a, RowKey, Vec<(FieldId, u64)>>,
}

impl<'a> Iterator for DigestRows<'a> {
    type Item = (Option<StableKey>, ComponentKey, &'a [(FieldId, u64)]);

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next().map(|(&(key, component), fields)| (key, component, fields.as_slice()))
    }
}

/// One difference between two digests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FieldDifference {
    /// The row's origin key, or `None` for a region resource.
    pub key: Option<StableKey>,
    /// The component or resource.
    pub component: ComponentKey,
    /// The field, or `None` when the whole row is present on one side only.
    pub field: Option<FieldId>,
    /// The hash on the left side, or `None` if the row or field is absent there. For a
    /// whole-row difference, `Some(0)` stands for presence.
    pub left: Option<u64>,
    /// The hash on the right side, or `None` if absent there.
    pub right: Option<u64>,
}

/// A cursor over one digest's rows.
type RowCursor<'a> = core::iter::Peekable<btree_map::Iter<'a, RowKey, Vec<(FieldId, u64)>>>;

/// Iterates the differences between two digests, ascending by key.
pub struct Differences<'a> {
    left: RowCursor<'a>,
    right: RowCursor<'a>,
    /// The differences of the row being compared, not yet yielded.
    pending: std::vec::IntoIter<FieldDifference>,
}

impl Iterator for Differences<'_> {
    type Item = FieldDifference;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(difference) = self.pending.next() {
                return Some(difference);
            }
            let whole = |key: &RowKey, left: bool| {
                let (key, component) = *key;
                let presence = (Some(0), None);
                let (left, right) = if left { presence } else { (presence.1, presence.0) };
                FieldDifference { key, component, field: None, left, right }
            };
            match (self.left.peek(), self.right.peek()) {
                (None, None) => return None,
                (Some(_), None) => return self.left.next().map(|(key, _)| whole(key, true)),
                (None, Some(_)) => return self.right.next().map(|(key, _)| whole(key, false)),
                (Some((left, _)), Some((right, _))) if left < right => {
                    return self.left.next().map(|(key, _)| whole(key, true));
                }
                (Some((left, _)), Some((right, _))) if left > right => {
                    return self.right.next().map(|(key, _)| whole(key, false));
                }
                _ => {
                    let (&(key, component), left) = self.left.next().expect("peeked");
                    let (_, right) = self.right.next().expect("peeked");
                    self.pending = field_differences(key, component, left, right).into_iter();
                }
            }
        }
    }
}

/// The fields of one row that differ between two digests, ascending by field ID.
fn field_differences(
    key: Option<StableKey>,
    component: ComponentKey,
    left: &[(FieldId, u64)],
    right: &[(FieldId, u64)],
) -> Vec<FieldDifference> {
    let mut fields: BTreeMap<FieldId, (Option<u64>, Option<u64>)> = BTreeMap::new();
    for &(id, hash) in left {
        fields.entry(id).or_default().0 = Some(hash);
    }
    for &(id, hash) in right {
        fields.entry(id).or_default().1 = Some(hash);
    }
    fields
        .into_iter()
        .filter(|(_, (left, right))| left != right)
        .map(|(id, (left, right))| FieldDifference { key, component, field: Some(id), left, right })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lactose_ecs::{RegionConfig, Schedule, WorldBuilder, WorldKind, system};
    use lactose_reflect::component;
    use std::sync::Arc;

    #[component(key = "sim_test::value", persistent)]
    struct Value(#[field(id = 1)] u64);

    #[component(key = "sim_test::marker", transient)]
    struct Marker;

    /// Where the hash pass leaves its result.
    struct Forked(Option<Result<u64>>, Hashers);

    #[system(name = "sim_test::hash_pass", worlds = [server], exclusive)]
    fn hash_pass(world: &mut World, job: Job, out: lactose_ecs::ResMut<Forked>) {
        let world: &World = world;
        let hashed = out.1.world_hash_forked(&mut job, world, RegionId(1));
        out.0 = Some(hashed);
    }

    #[test]
    fn the_forked_hash_equals_the_sequential_hash_under_every_layout() {
        let executor = lactose_ecs::Executor::builder().frame_workers(2).build().unwrap();
        let mut expected = None;
        for chunk_rows in [Some(1), Some(7), None] {
            let mut types = TypeTable::new();
            types.register::<Value>().unwrap();
            types.register::<Marker>().unwrap();
            let (registry, hashers, _) = types.into_parts();
            let layout = lactose_ecs::LayoutConfig {
                chunk_rows,
                seed: 3,
                permute_rows: true,
                chunk_order: lactose_ecs::ChunkOrder::Shuffled,
                split_batches: true,
            };
            let mut world = WorldBuilder::new(WorldKind::Server, Arc::new(registry))
                .layout(layout)
                .build()
                .unwrap();
            for region in [1, 2] {
                world
                    .create_region(RegionConfig {
                        id: RegionId(region),
                        seed: region,
                        mode: RegionMode::Simulated,
                    })
                    .unwrap();
            }
            for id in 1..40_u64 {
                let key = StableKey::Net(lactose_core::NetId::from_bits(id).unwrap());
                let region = RegionId(1 + id % 2);
                if id % 3 == 0 {
                    world.spawn(key, region, (Value(id * 7), Marker)).unwrap();
                } else {
                    world.spawn(key, region, (Value(id * 7),)).unwrap();
                }
            }
            let sequential = hashers.world_hash(&world, RegionId(1)).unwrap();
            world.insert_service(Box::new(Forked(None, hashers)));
            let mut builder = Schedule::builder(WorldKind::Server);
            builder.stage("hash", &[]).add(&HASH_PASS);
            let mut pass = builder.build().unwrap();
            let mut world = executor.block_on(pass.run_pass(world, &executor)).unwrap();
            let forked = world.service_mut::<Forked>().unwrap().0.take().unwrap().unwrap();
            assert_eq!(forked, sequential);
            assert_eq!(*expected.get_or_insert(sequential), sequential);
        }
    }
}
