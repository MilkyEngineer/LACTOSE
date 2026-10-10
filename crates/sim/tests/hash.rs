//! The world hash (README, "The world hash"): the encoding pinned by golden values and by
//! an independent reimplementation, what it covers and what it ignores, the digest, and
//! the registration refusals.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{
    EntityRef, ErrorKind, FieldId, NetId, PlainArray, PlayerId, PredictionKey, StableKey, Tick,
};
use lactose_ecs::{RegionConfig, RegionId, RegionMode, World, WorldKind};
use lactose_math::Scalar;
use lactose_reflect::{
    Field, FieldKind, FieldVisitor, Padding, ReferenceVisitor, ScalarField, component, resource,
    value,
};
use lactose_sim::{
    FieldDifference, RegionDigest, Simulation, SimulationBuilder, TickInput, TypeTable,
};
use xxhash_rust::xxh64::xxh64;

/// A marker for typed asset references.
struct Mesh;

/// Every leaf kind, with IDs out of declaration order.
#[component(key = "golden::probe", persistent)]
#[derive(PartialEq, Debug)]
struct Probe {
    #[field(id = 3)]
    speed: Scalar,
    #[field(id = 1)]
    target: EntityRef,
    #[field(id = 7)]
    count: u32,
    #[field(id = 2)]
    flag: bool,
    #[field(padding)]
    _pad: Padding<3>,
    #[field(id = 4)]
    mesh: AssetRef<Mesh>,
    #[field(id = 9)]
    items: PlainArray<u16, 2>,
    #[field(id = 10)]
    tail: i32,
}

/// No field at all: its presence alone is hashed.
#[component(key = "golden::tag", transient)]
struct Tag;

#[resource(key = "golden::clock", persistent)]
struct Clock(#[field(id = 5)] u64);

const REGION: RegionId = RegionId(42);
const SEED: u64 = 0xABCD_EF01;

fn net(id: u64) -> StableKey {
    StableKey::Net(NetId::from_bits(id).unwrap())
}

fn probe(n: u32) -> Probe {
    Probe {
        speed: Scalar::from_i32(n as i32) / Scalar::from_u32(3),
        target: EntityRef::new(net(u64::from(n) + 100)),
        count: n * 11,
        flag: n.is_multiple_of(2),
        _pad: Padding::default(),
        mesh: AssetRef::new(AssetId::from_bits(u64::from(n) + 77)),
        items: PlainArray([n as u16, 0xBEEF]),
        tail: -(n as i32),
    }
}

fn golden_types() -> TypeTable {
    let mut types = TypeTable::new();
    types.register::<Probe>().unwrap();
    types.register::<Tag>().unwrap();
    types.register::<Clock>().unwrap();
    types
}

/// The golden world at tick 9: two probes (one predicted, one tagged), a clock and a table.
fn golden(layout: lactose_ecs::LayoutConfig) -> Simulation {
    let mut simulation = SimulationBuilder::new(WorldKind::Server, golden_types())
        .tick(Tick(9))
        .layout(layout)
        .build()
        .unwrap();
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: REGION, seed: SEED, mode: RegionMode::Frozen }).unwrap();
    world.set_region_assets(REGION, table(&[(5, 50), (3, 30)])).unwrap();
    world.insert_region_resource(REGION, Clock(1234)).unwrap();
    world.spawn(net(1), REGION, (probe(1),)).unwrap();
    let predicted = PredictionKey { player: PlayerId(4), input_tick: Tick(8), ordinal: 0x55 };
    world.spawn(StableKey::Predicted(predicted), REGION, (probe(2), Tag)).unwrap();
    simulation
}

// ---- The reference implementation, from the README's encoding alone ----

fn le16(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

fn key16(key: Option<StableKey>) -> Vec<u8> {
    let (tag, words): (u32, [u32; 3]) = match key {
        None => (0, [0; 3]),
        Some(StableKey::Net(id)) => {
            let bits = id.to_bits();
            (1, [bits as u32, (bits >> 32) as u32, 0])
        }
        Some(StableKey::Predicted(p)) => (2, [p.player.0, p.input_tick.0, p.ordinal]),
        Some(StableKey::Placeholder(_)) => unreachable!(),
    };
    let mut bytes = tag.to_le_bytes().to_vec();
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn row(key: Option<StableKey>, component: u64, fields: &[(u16, Vec<u8>)]) -> u64 {
    let mut bytes = key16(key);
    for (id, stream) in fields {
        bytes.extend_from_slice(&le16(*id));
        bytes.extend_from_slice(&xxh64(stream, u64::from(*id)).to_le_bytes());
    }
    xxh64(&bytes, component)
}

fn probe_row(key: StableKey, value: &Probe) -> u64 {
    let scalar = |s: Scalar| ScalarField::canonical_bits(&s).to_le_bytes().to_vec();
    let mut items = value.items.0[0].to_le_bytes().to_vec();
    items.extend_from_slice(&value.items.0[1].to_le_bytes());
    let fields = [
        (3, scalar(value.speed)),
        (1, key16(value.target.get())),
        (7, value.count.to_le_bytes().to_vec()),
        (2, vec![u8::from(value.flag)]),
        (4, value.mesh.id().to_bits().to_le_bytes().to_vec()),
        (9, items),
        (10, value.tail.to_le_bytes().to_vec()),
    ];
    row(Some(key), xxh64(b"golden::probe", 0), &fields)
}

fn reference_region_hash() -> u64 {
    let probe_key = xxh64(b"golden::probe", 0);
    let tag_key = xxh64(b"golden::tag", 0);
    let clock_key = xxh64(b"golden::clock", 0);
    let predicted = StableKey::Predicted(PredictionKey {
        player: PlayerId(4),
        input_tick: Tick(8),
        ordinal: 0x55,
    });
    let probes = probe_row(net(1), &probe(1)).wrapping_add(probe_row(predicted, &probe(2)));
    let tags = row(Some(predicted), tag_key, &[]);
    // A spawn under a `Predicted` key carries its persistent `PredictedOrigin`.
    let origin_key = xxh64(b"ecs::predicted_origin", 0);
    let origin = row(
        Some(predicted),
        origin_key,
        &[
            (1, 4_u32.to_le_bytes().to_vec()),
            (2, 8_u32.to_le_bytes().to_vec()),
            (3, 0x55_u32.to_le_bytes().to_vec()),
        ],
    );
    let clock = row(None, clock_key, &[(5, 1234_u64.to_le_bytes().to_vec())]);

    let mut columns = vec![(probe_key, 2_u32, probes), (tag_key, 1, tags), (origin_key, 1, origin)];
    columns.sort();
    let mut bytes = SEED.to_le_bytes().to_vec();
    bytes.push(2); // Frozen
    bytes.push(1);
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    for (id, version) in [(3_u64, 30_u64), (5, 50)] {
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&version.to_le_bytes());
    }
    for (key, count, sum) in columns {
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&sum.to_le_bytes());
    }
    bytes.extend_from_slice(&clock_key.to_le_bytes());
    bytes.extend_from_slice(&clock.to_le_bytes());
    xxh64(&bytes, REGION.0)
}

/// Pinned: a change to the encoding changes these, and must be deliberate. They hold for
/// the 64-bit simulation scalar; the scalar decision of milestone 1 re-pins them.
const GOLDEN_REGION: u64 = 0x96bc_2e76_812e_38c3;
const GOLDEN_SIMULATION: u64 = 0xd7c1_7142_23e6_85ae;

#[test]
fn the_encoding_matches_a_reimplementation_and_the_golden_values() {
    let simulation = golden(lactose_ecs::LayoutConfig::default());
    let region = simulation.world_hash(REGION).unwrap();
    assert_eq!(region, reference_region_hash());
    let mut bytes = 9_u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&REGION.0.to_le_bytes());
    bytes.extend_from_slice(&region.to_le_bytes());
    let whole = simulation.hash().unwrap();
    assert_eq!(whole, xxh64(&bytes, 0));
    assert_eq!((region, whole), (GOLDEN_REGION, GOLDEN_SIMULATION));
}

#[test]
fn a_world_with_no_region_hashes_its_tick_alone() {
    let simulation =
        SimulationBuilder::new(WorldKind::Server, TypeTable::new()).tick(Tick(77)).build().unwrap();
    assert_eq!(simulation.hash().unwrap(), xxh64(&77_u32.to_le_bytes(), 0));
    assert_eq!(simulation.world_hash(REGION).unwrap_err().kind(), ErrorKind::NotFound);
}

#[test]
fn the_hash_is_the_same_under_every_layout_and_registration_order() {
    let reference = golden(lactose_ecs::LayoutConfig::default()).world_hash(REGION).unwrap();
    for capacity in [
        lactose_sim::harness::ChunkCapacity::Rows(1),
        lactose_sim::harness::ChunkCapacity::Rows(7),
        lactose_sim::harness::ChunkCapacity::Rows(64),
        lactose_sim::harness::ChunkCapacity::Bytes(16384),
    ] {
        for seed in [1, 2, 0xDEAD] {
            let layout = lactose_sim::harness::layout_mode(capacity, seed).unwrap();
            assert_eq!(golden(layout).world_hash(REGION).unwrap(), reference);
        }
    }
    // Another registration order, and unrelated types registered too.
    let mut types = TypeTable::new();
    types.register::<Clock>().unwrap();
    register_types(&mut types).unwrap();
    types.register::<Tag>().unwrap();
    types.register::<Probe>().unwrap();
    let mut simulation =
        SimulationBuilder::new(WorldKind::Server, types).tick(Tick(9)).build().unwrap();
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: REGION, seed: SEED, mode: RegionMode::Frozen }).unwrap();
    world.set_region_assets(REGION, table(&[(3, 30), (5, 50)])).unwrap();
    world.insert_region_resource(REGION, Clock(1234)).unwrap();
    let predicted = PredictionKey { player: PlayerId(4), input_tick: Tick(8), ordinal: 0x55 };
    world.spawn(StableKey::Predicted(predicted), REGION, (Tag, probe(2))).unwrap();
    world.spawn(net(1), REGION, (probe(1),)).unwrap();
    assert_eq!(simulation.world_hash(REGION).unwrap(), reference);
}

/// The game's world with one region holding a few rows of every kind.
fn base() -> Simulation {
    let mut simulation = game(WorldKind::Server).build().unwrap();
    let region = region_at(1, 1);
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: region, seed: 5, mode: RegionMode::Simulated }).unwrap();
    world.insert_region_resource(region, Weather(1)).unwrap();
    world.insert_region_resource_zeroed(region, <Rain as lactose_reflect::Reflect>::KEY).unwrap();
    world.set_region_assets(region, table(&[(1, 1)])).unwrap();
    world.spawn(net(1), region, (Health(5), Contacts(0))).unwrap();
    world.spawn(net(2), region, (Health(6), Age(1))).unwrap();
    simulation
}

/// A named edit of the base world.
type Change = (&'static str, fn(&mut World));

#[test]
fn derived_state_changes_nothing_and_every_hashed_thing_changes_the_hash() {
    let region = region_at(1, 1);
    let hash = |simulation: &Simulation| simulation.world_hash(region).unwrap();
    let reference = hash(&base());
    let same: [Change; 3] = [
        ("a derived column", |world| world.insert(net(1), Cache(9)).unwrap()),
        ("a world resource", |world| {
            world.insert_resource(Gravity(Scalar::TWO));
        }),
        ("another region", |world| {
            world
                .create_region(RegionConfig { id: RegionId(7), seed: 1, mode: RegionMode::Frozen })
                .unwrap();
            world.spawn(net(9), RegionId(7), (Health(1),)).unwrap();
        }),
    ];
    for (what, change) in same {
        let mut simulation = base();
        change(simulation.world_mut());
        assert_eq!(hash(&simulation), reference, "{what} changed the hash");
    }
    let differ: [Change; 8] = [
        ("a transient value", |world| world.insert(net(1), Contacts(3)).unwrap()),
        ("a persistent value", |world| world.insert(net(2), Health(7)).unwrap()),
        ("a region resource", |world| {
            world.insert_region_resource(region_at(1, 1), Weather(2)).unwrap();
        }),
        ("the transient region resource's value", |world| {
            world.update_region_resource::<Rain>(region_at(1, 1), |rain| rain.0 = 1).unwrap();
        }),
        ("the mode", |world| world.set_region_mode(region_at(1, 1), RegionMode::Frozen).unwrap()),
        ("the table", |world| {
            world.set_region_assets(region_at(1, 1), table(&[(1, 2)])).unwrap();
        }),
        ("a marker's presence", |world| world.insert(net(2), Marker).unwrap()),
        ("a row's key", |world| {
            world.despawn(net(2)).unwrap();
            world.spawn(net(3), region_at(1, 1), (Health(6), Age(1))).unwrap();
        }),
    ];
    for (what, change) in differ {
        let mut simulation = base();
        change(simulation.world_mut());
        assert_ne!(hash(&simulation), reference, "{what} did not change the hash");
    }
    // The seed: the same rows in a region created with another seed.
    let mut simulation = game(WorldKind::Server).build().unwrap();
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: region, seed: 6, mode: RegionMode::Simulated }).unwrap();
    world.insert_region_resource(region, Weather(1)).unwrap();
    world.insert_region_resource_zeroed(region, <Rain as lactose_reflect::Reflect>::KEY).unwrap();
    world.set_region_assets(region, table(&[(1, 1)])).unwrap();
    world.spawn(net(1), region, (Health(5), Contacts(0))).unwrap();
    world.spawn(net(2), region, (Health(6), Age(1))).unwrap();
    assert_ne!(hash(&simulation), reference, "the seed did not change the hash");
}

#[test]
fn a_digest_explains_its_hash() {
    let left = golden(lactose_ecs::LayoutConfig::default());
    let digest = left.digest(REGION).unwrap();
    assert_eq!(digest.hash(), left.world_hash(REGION).unwrap());
    assert_eq!((digest.region(), digest.seed(), digest.mode()), (REGION, SEED, RegionMode::Frozen));
    assert_eq!(digest.assets().unwrap().len(), 2);
    let rows: Vec<_> =
        digest.rows().map(|(key, component, fields)| (key, component, fields.len())).collect();
    let probe_key = <Probe as lactose_reflect::Reflect>::KEY;
    let tag_key = <Tag as lactose_reflect::Reflect>::KEY;
    let clock_key = <Clock as lactose_reflect::Reflect>::KEY;
    let predicted = StableKey::Predicted(PredictionKey {
        player: PlayerId(4),
        input_tick: Tick(8),
        ordinal: 0x55,
    });
    let mut expected = vec![
        (None, clock_key, 1),
        (Some(net(1)), probe_key, 7),
        (Some(predicted), probe_key, 7),
        (Some(predicted), tag_key, 0),
        (Some(predicted), <lactose_ecs::PredictedOrigin as lactose_reflect::Reflect>::KEY, 3),
    ];
    expected.sort();
    assert_eq!(rows, expected);
    let ids: Vec<u16> = digest.rows().nth(1).unwrap().2.iter().map(|(id, _)| id.0).collect();
    assert_eq!(ids, [3, 1, 7, 2, 4, 9, 10], "fields in declaration order");

    // One field changed gives one difference.
    let mut right = golden(lactose_ecs::LayoutConfig::default());
    let mut changed = probe(1);
    changed.count += 1;
    right.world_mut().insert(net(1), changed).unwrap();
    let other = right.digest(REGION).unwrap();
    assert_eq!(other.hash(), right.world_hash(REGION).unwrap());
    let differences: Vec<FieldDifference> = digest.diff(&other).collect();
    assert_eq!(differences.len(), 1);
    let difference = differences[0];
    assert_eq!(
        (difference.key, difference.component, difference.field),
        (Some(net(1)), probe_key, Some(FieldId(7)))
    );
    assert!(
        difference.left.is_some()
            && difference.right.is_some()
            && difference.left != difference.right
    );

    // A marker on one side is a whole-row difference.
    let mut marked = golden(lactose_ecs::LayoutConfig::default());
    marked.world_mut().insert(net(1), Tag).unwrap();
    let marked = marked.digest(REGION).unwrap();
    let differences: Vec<FieldDifference> = digest.diff(&marked).collect();
    assert_eq!(
        differences,
        vec![FieldDifference {
            key: Some(net(1)),
            component: tag_key,
            field: None,
            left: None,
            right: Some(0)
        }]
    );
    let back: Vec<FieldDifference> = marked.diff(&digest).collect();
    assert_eq!(back[0].left, Some(0));
    assert_eq!(digest.diff(&digest).count(), 0);
}

#[test]
fn a_digest_travels_as_its_canonical_encoding() {
    let digest = golden(lactose_ecs::LayoutConfig::default()).digest(REGION).unwrap();
    let bytes = digest.encode();
    let decoded = RegionDigest::decode(&bytes).unwrap();
    assert_eq!(decoded, digest);
    assert_eq!(decoded.encode(), bytes);
    assert_eq!(decoded.hash(), digest.hash());
    let bad = |bytes: &[u8]| RegionDigest::decode(bytes).unwrap_err().kind();
    assert_eq!(bad(&bytes[..bytes.len() - 1]), ErrorKind::InvalidData);
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(bad(&longer), ErrorKind::InvalidData);
    let mut wrong_mode = bytes.clone();
    wrong_mode[4 + 1 + 16] = 9;
    assert_eq!(bad(&wrong_mode), ErrorKind::InvalidData);
    assert_eq!(bad(b"nope"), ErrorKind::InvalidData);
    // Every single-byte corruption is refused or decodes to a digest that encodes to it.
    for at in 0..bytes.len() {
        let mut corrupt = bytes.clone();
        corrupt[at] ^= 0x41;
        if let Ok(decoded) = RegionDigest::decode(&corrupt) {
            assert_eq!(decoded.encode(), corrupt);
        }
    }
}

// ---- Registration ----

lactose_core::typed_plain_data! {
    /// A scalar that is not the engine's.
    pub struct Fake<T>(u64);
}

/// Claims a tag that is not `lactose_math::Scalar`'s.
impl Field for Fake<u8> {
    const KIND: FieldKind = FieldKind::scalar::<Self>();
    const NAME: &'static str = "FakeScalar";

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.scalar(id, self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

impl ScalarField for Fake<u8> {
    fn canonical_bits(&self) -> u64 {
        0
    }
}

/// Claims `lactose_math::Scalar`'s tag without being it.
impl Field for Fake<u16> {
    const KIND: FieldKind = FieldKind::scalar::<Self>();
    const NAME: &'static str = <Scalar as Field>::NAME;

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.scalar(id, self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

impl ScalarField for Fake<u16> {
    fn canonical_bits(&self) -> u64 {
        0
    }
}

#[component(key = "refuse::float", persistent)]
struct RawFloat(#[field(id = 1)] f32);

#[component(key = "refuse::derived_float", derived)]
struct DerivedFloat(#[field(id = 1)] f64);

#[value]
struct Nested {
    #[field(id = 1)]
    inner: PlainArray<f64, 2>,
}

#[resource(key = "refuse::nested", transient)]
struct NestedFloat(#[field(id = 1)] Nested);

#[component(key = "refuse::fake", persistent)]
struct WrongTag(#[field(id = 1)] Fake<u8>);

#[component(key = "refuse::impostor", persistent)]
struct Impostor(#[field(id = 1)] Fake<u16>);

#[test]
fn registration_refuses_what_the_hasher_cannot_hash() {
    let mut types = TypeTable::new();
    assert_eq!(types.register::<RawFloat>().unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(types.register::<NestedFloat>().unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(types.register::<WrongTag>().unwrap_err().kind(), ErrorKind::Unsupported);
    assert!(!types.registry().contains::<RawFloat>(), "a refused type is not registered");
    types.register::<DerivedFloat>().unwrap();
    types.register::<DerivedFloat>().unwrap();
    let hashers = types.hashers();
    assert!(!hashers.covers(<DerivedFloat as lactose_reflect::Reflect>::KEY));
    types.register::<Probe>().unwrap();
    assert!(types.hashers().covers(<Probe as lactose_reflect::Reflect>::KEY));
}

#[test]
#[should_panic(expected = "trusts `lactose_math::Scalar` alone")]
fn a_scalar_that_carries_the_tag_without_being_the_type_is_refused_by_the_visitor() {
    let mut types = TypeTable::new();
    types.register::<Impostor>().unwrap();
    let mut simulation = SimulationBuilder::new(WorldKind::Server, types).build().unwrap();
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: REGION, seed: 1, mode: RegionMode::Frozen }).unwrap();
    world.spawn(net(1), REGION, (Impostor(lactose_reflect::bytemuck::cast(3_u64)),)).unwrap();
    let _ = simulation.world_hash(REGION);
}

#[test]
fn a_column_the_hashers_do_not_cover_is_unsupported() {
    // A world built from one table, hashed with another's hashers.
    let simulation = golden(lactose_ecs::LayoutConfig::default());
    let hashers = TypeTable::new().hashers().clone();
    let error = hashers.world_hash(simulation.world(), REGION).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(
        hashers.digest(simulation.world(), REGION).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn the_tick_is_hashed_and_a_step_changes_nothing_else_in_an_empty_frame() {
    let executor = executor();
    let simulation = golden(lactose_ecs::LayoutConfig::default());
    let region = simulation.world_hash(REGION).unwrap();
    let simulation = step(&executor, simulation, TickInput::default());
    // The region is frozen and nothing of the golden table runs, but the tag is transient
    // and was zeroed, which it already was.
    assert_eq!(simulation.world_hash(REGION).unwrap(), region);
    let mut bytes = 10_u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&REGION.0.to_le_bytes());
    bytes.extend_from_slice(&region.to_le_bytes());
    assert_eq!(simulation.hash().unwrap(), xxh64(&bytes, 0));
}
