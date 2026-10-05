//! The bundle format from the outside: golden bytes, round trips, canonical form, the
//! patched-baseline rule and decoder robustness.

#![forbid(unsafe_code)]

use core::num::NonZeroU64;

use lactose_asset_api::{AssetId, AssetSet};
use lactose_bundle::{
    Archetype, Baseline, BaselineRow, ComponentData, ComponentSchema, FieldLayout,
    MAX_COMPONENT_BYTES, MAX_DOCUMENT_BYTES, Overlay, Record,
};
use lactose_core::{ComponentKey, ErrorKind, FieldId, NetId, xxh64};

const A: ComponentKey = ComponentKey(0x0A);
const B: ComponentKey = ComponentKey(0x0B);
const C: ComponentKey = ComponentKey(0x0C);
const D: ComponentKey = ComponentKey(0x0D);
const BASELINE_HASH: u64 = 0x0123_4567_89AB_CDEF;

fn cooked(n: u64) -> NetId {
    NetId::cooked(n)
}

fn runtime(n: u64) -> NetId {
    NetId::runtime(NonZeroU64::new(n).unwrap()).unwrap()
}

fn field(id: u16, offset: u32, size: u32) -> FieldLayout {
    FieldLayout { id: FieldId(id), offset, size }
}

fn schema(
    component: ComponentKey,
    version: u32,
    size: u32,
    fields: &[FieldLayout],
) -> ComponentSchema {
    ComponentSchema::new(component, version, size, fields.iter().copied()).unwrap()
}

/// The registry of these tests. A is four bytes in two fields, B two bytes in one field,
/// C a tag, D one byte. Documents keep the schemas they use.
fn schemas() -> Vec<ComponentSchema> {
    vec![
        schema(A, 1, 4, &[field(4, 2, 2), field(1, 0, 2)]),
        schema(B, 3, 2, &[field(1, 0, 2)]),
        schema(C, 1, 0, &[]),
        schema(D, 1, 1, &[field(1, 0, 1)]),
    ]
}

fn data(component: ComponentKey, bytes: &[u8]) -> ComponentData {
    ComponentData { component, bytes: bytes.to_vec() }
}

fn a(fill: u8) -> ComponentData {
    data(A, &[fill; 4])
}

fn b(fill: u8) -> ComponentData {
    data(B, &[fill; 2])
}

fn c() -> ComponentData {
    data(C, &[])
}

fn d(fill: u8) -> ComponentData {
    data(D, &[fill])
}

fn row(id: NetId, components: &[ComponentData]) -> BaselineRow {
    BaselineRow { id, components: components.to_vec() }
}

/// A `SetField` of a field of the test registry, with bytes of the field's size.
fn set_field(id: NetId, component: ComponentKey, field: u16) -> Record {
    let size = if component == D { 1 } else { 2 };
    Record::SetField { id, component, field: FieldId(field), bytes: [0xEE, 0xFF][..size].to_vec() }
}

fn add(id: NetId, data: ComponentData) -> Record {
    Record::AddComponent { id, data }
}

fn remove(id: NetId, component: ComponentKey) -> Record {
    Record::RemoveComponent { id, component }
}

fn owner(id: NetId, owner: Option<NetId>) -> Record {
    Record::OwnershipChange { id, owner }
}

fn destroy(id: NetId) -> Record {
    Record::Destroy { id }
}

fn spawn(id: NetId, components: &[ComponentData]) -> Record {
    Record::Spawn { id, components: components.to_vec() }
}

fn from_baseline(id: NetId, components: &[ComponentData]) -> Record {
    Record::SpawnFromBaseline { id, components: components.to_vec() }
}

fn assets() -> AssetSet {
    [AssetId::from_bits(0x22), AssetId::from_bits(0x11)].into_iter().collect()
}

fn baseline_rows() -> Vec<BaselineRow> {
    vec![
        row(cooked(2), &[a(2), b(0xB2)]),
        row(cooked(1), &[a(1), b(0xB1)]),
        row(cooked(3), &[c(), a(3)]),
    ]
}

fn baseline_of(rows: &[BaselineRow]) -> Baseline {
    Baseline::from_rows(assets(), schemas(), rows.to_vec()).unwrap()
}

fn small_baseline() -> Baseline {
    baseline_of(&baseline_rows())
}

fn overlay_records() -> Vec<Record> {
    vec![
        spawn(runtime(5), &[b(9), a(5)]),
        from_baseline(cooked(7), &[a(7)]),
        destroy(cooked(1)),
        add(cooked(2), c()),
        remove(cooked(2), B),
        set_field(cooked(2), A, 4),
        owner(cooked(2), Some(runtime(5))),
        owner(runtime(5), None),
    ]
}

fn overlay_of(baseline_hash: u64, records: &[Record]) -> Overlay {
    Overlay::new(baseline_hash, schemas(), records.to_vec()).unwrap()
}

fn small_overlay() -> Overlay {
    overlay_of(BASELINE_HASH, &overlay_records())
}

/// Sixteen bytes a line, so a format change shows up as a readable diff.
fn hex(bytes: &[u8]) -> String {
    let line =
        |chunk: &[u8]| chunk.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(" ");
    bytes.chunks(16).map(line).collect::<Vec<_>>().join("\n")
}

fn unhex(text: &str) -> Vec<u8> {
    text.split_whitespace().map(|byte| u8::from_str_radix(byte, 16).unwrap()).collect()
}

const GOLDEN_BASELINE: &str = "\
4c 42 41 53 01 00 00 00 02 00 00 00 11 00 00 00\n\
00 00 00 00 22 00 00 00 00 00 00 00 03 00 00 00\n\
0a 00 00 00 00 00 00 00 01 00 00 00 04 00 00 00\n\
02 00 00 00 01 00 00 00 00 00 02 00 00 00 04 00\n\
02 00 00 00 02 00 00 00 0b 00 00 00 00 00 00 00\n\
03 00 00 00 02 00 00 00 01 00 00 00 01 00 00 00\n\
00 00 02 00 00 00 0c 00 00 00 00 00 00 00 01 00\n\
00 00 00 00 00 00 00 00 00 00 02 00 00 00 02 00\n\
00 00 0a 00 00 00 00 00 00 00 0b 00 00 00 00 00\n\
00 00 02 00 00 00 01 00 00 00 00 00 00 80 02 00\n\
00 00 00 00 00 80 01 01 01 01 02 02 02 02 b1 b1\n\
b2 b2 02 00 00 00 0a 00 00 00 00 00 00 00 0c 00\n\
00 00 00 00 00 00 01 00 00 00 03 00 00 00 00 00\n\
00 80 03 03 03 03 a7 a8 e8 aa 13 f5 0a aa";

const GOLDEN_OVERLAY: &str = "\
4c 4f 56 4c 01 00 00 00 ef cd ab 89 67 45 23 01\n\
03 00 00 00 0a 00 00 00 00 00 00 00 01 00 00 00\n\
04 00 00 00 02 00 00 00 01 00 00 00 00 00 02 00\n\
00 00 04 00 02 00 00 00 02 00 00 00 0b 00 00 00\n\
00 00 00 00 03 00 00 00 02 00 00 00 01 00 00 00\n\
01 00 00 00 00 00 02 00 00 00 0c 00 00 00 00 00\n\
00 00 01 00 00 00 00 00 00 00 00 00 00 00 08 00\n\
00 00 01 05 00 00 00 00 00 00 00 02 00 00 00 0a\n\
00 00 00 00 00 00 00 05 05 05 05 0b 00 00 00 00\n\
00 00 00 09 09 07 05 00 00 00 00 00 00 00 00 00\n\
00 00 00 00 00 00 03 01 00 00 00 00 00 00 80 04\n\
02 00 00 00 00 00 00 80 0c 00 00 00 00 00 00 00\n\
05 02 00 00 00 00 00 00 80 0b 00 00 00 00 00 00\n\
00 06 02 00 00 00 00 00 00 80 0a 00 00 00 00 00\n\
00 00 04 00 ee ff 07 02 00 00 00 00 00 00 80 05\n\
00 00 00 00 00 00 00 02 07 00 00 00 00 00 00 80\n\
01 00 00 00 0a 00 00 00 00 00 00 00 07 07 07 07\n\
f3 b5 f0 d0 eb 85 87 b7";

#[test]
fn baseline_matches_its_golden_bytes() {
    let bytes = small_baseline().encode();
    assert_eq!(hex(&bytes), GOLDEN_BASELINE);
    assert_eq!(Baseline::decode(&unhex(GOLDEN_BASELINE)).unwrap(), small_baseline());
    assert_eq!(small_baseline().content_hash(), 0xaa0a_f513_aae8_a8a7);
}

#[test]
fn overlay_matches_its_golden_bytes() {
    let bytes = small_overlay().encode();
    assert_eq!(hex(&bytes), GOLDEN_OVERLAY);
    assert_eq!(Overlay::decode(&unhex(GOLDEN_OVERLAY)).unwrap(), small_overlay());
    assert_eq!(small_overlay().digest(), 0xb787_85eb_d0f0_b5f3);
}

#[test]
fn baseline_round_trips_and_exposes_its_rows() {
    let baseline = small_baseline();
    assert_eq!(Baseline::decode(&baseline.encode()).unwrap(), baseline);
    assert_eq!(baseline.assets(), &assets());

    let used: Vec<_> = baseline.schemas().iter().map(ComponentSchema::component).collect();
    assert_eq!(used, [A, B, C], "the registry's D is not carried");
    let a_schema = baseline.schema(A).unwrap();
    assert_eq!((a_schema.schema_version(), a_schema.element_size()), (1, 4));
    assert_eq!(a_schema.fields(), [field(1, 0, 2), field(4, 2, 2)], "fields are in ID order");
    assert_eq!(a_schema.field(FieldId(4)), Some(field(4, 2, 2)));
    assert_eq!(a_schema.field(FieldId(2)), None);
    assert!(baseline.schema(D).is_none());

    let [with_b, with_c] = baseline.archetypes() else { panic!("two archetypes") };
    let keys = |archetype: &Archetype| archetype.columns().map(|c| c.component).collect::<Vec<_>>();
    assert_eq!(with_b.ids(), [cooked(1), cooked(2)]);
    assert_eq!(keys(with_b), [A, B]);
    let column = with_b.column(B).unwrap();
    assert_eq!(column.element_size, 2);
    assert_eq!(column.bytes, [0xB1, 0xB1, 0xB2, 0xB2], "rows are in ID order");
    assert_eq!(with_b.column(A).unwrap().bytes, [1, 1, 1, 1, 2, 2, 2, 2]);
    assert!(with_b.column(C).is_none());
    assert!(with_b.has(B) && !with_b.has(C));

    assert_eq!(with_c.ids(), [cooked(3)]);
    assert_eq!(keys(with_c), [A, C]);
    assert_eq!(with_c.column(A).unwrap().bytes, [3, 3, 3, 3]);
    assert_eq!(with_c.column(C).unwrap().element_size, 0);
    assert!(with_c.column(C).unwrap().bytes.is_empty());

    assert!(baseline.contains(cooked(2)));
    assert!(!baseline.contains(cooked(4)));
    assert_eq!(baseline.archetype_of(cooked(3)), Some(with_c));
    assert_eq!(baseline.archetype_of(cooked(1)), Some(with_b));
}

#[test]
fn overlay_round_trips_and_orders_its_records() {
    let overlay = small_overlay();
    assert_eq!(Overlay::decode(&overlay.encode()).unwrap(), overlay);
    assert_eq!(overlay.baseline_hash(), BASELINE_HASH);
    let expected = [
        spawn(runtime(5), &[a(5), b(9)]),
        owner(runtime(5), None),
        destroy(cooked(1)),
        add(cooked(2), c()),
        remove(cooked(2), B),
        set_field(cooked(2), A, 4),
        owner(cooked(2), Some(runtime(5))),
        from_baseline(cooked(7), &[a(7)]),
    ];
    assert_eq!(overlay.records(), expected);
    let used: Vec<_> = overlay.schemas().iter().map(ComponentSchema::component).collect();
    assert_eq!(used, [A, B, C]);
    assert_eq!(overlay.schema(B).unwrap().schema_version(), 3);
}

#[test]
fn empty_documents_round_trip() {
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(), []).unwrap();
    assert_eq!(baseline.encode(), sealed(b"LBAS", &[0; 12]));
    assert_eq!(Baseline::decode(&baseline.encode()).unwrap(), baseline);
    let overlay = Overlay::new(0, schemas(), []).unwrap();
    assert_eq!(overlay.encode(), sealed(b"LOVL", &[0; 16]));
    assert_eq!(Overlay::decode(&overlay.encode()).unwrap(), overlay);
    // An entity with no components is still a row.
    let bare = Baseline::from_rows(AssetSet::new(), [], [row(cooked(1), &[])]).unwrap();
    assert_eq!(Baseline::decode(&bare.encode()).unwrap(), bare);
}

/// A small deterministic generator (splitmix64), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

#[test]
fn construction_order_does_not_change_the_encoding() {
    let baseline = small_baseline().encode();
    let overlay = small_overlay().encode();
    let reversed_assets: AssetSet =
        [AssetId::from_bits(0x11), AssetId::from_bits(0x22)].into_iter().collect();
    let mut rng = Rng(7);
    for _ in 0..200 {
        let mut registry = vec![
            schema(D, 1, 1, &[field(1, 0, 1)]),
            schema(C, 1, 0, &[]),
            schema(B, 3, 2, &[field(1, 0, 2)]),
            schema(A, 1, 4, &[field(1, 0, 2), field(4, 2, 2)]),
        ];
        rng.shuffle(&mut registry);

        let mut rows = baseline_rows();
        rng.shuffle(&mut rows);
        for row in &mut rows {
            rng.shuffle(&mut row.components);
        }
        let reordered = Baseline::from_rows(reversed_assets.clone(), registry.clone(), rows);
        let reordered = reordered.unwrap();
        assert_eq!(reordered, small_baseline());
        assert_eq!(reordered.encode(), baseline);

        let mut records = overlay_records();
        rng.shuffle(&mut records);
        for record in &mut records {
            if let Record::Spawn { components, .. } = record {
                rng.shuffle(components);
            }
        }
        let reordered = Overlay::new(BASELINE_HASH, registry, records).unwrap();
        assert_eq!(reordered, small_overlay());
        assert_eq!(reordered.encode(), overlay);
    }
}

#[test]
fn hashes_follow_content_and_are_kept() {
    let baseline = small_baseline();
    let encoded = baseline.encode();
    let (body, trailer) = encoded.split_at(encoded.len() - 8);
    assert_eq!(baseline.content_hash(), xxh64(body, 0));
    assert_eq!(baseline.content_hash().to_le_bytes(), trailer);
    assert_eq!(Baseline::decode(&encoded).unwrap().content_hash(), baseline.content_hash());

    let mut rows = baseline_rows();
    rows[0].components[0].bytes[0] ^= 1;
    assert_ne!(baseline_of(&rows).content_hash(), baseline.content_hash());
    let fewer_assets = Baseline::from_rows(AssetSet::new(), schemas(), baseline_rows()).unwrap();
    assert_ne!(fewer_assets.content_hash(), baseline.content_hash());
    let mut other_layout = schemas();
    other_layout[0] = schema(A, 1, 4, &[field(1, 0, 2), field(4, 2, 1)]);
    let relaid = Baseline::from_rows(assets(), other_layout, baseline_rows()).unwrap();
    assert_ne!(relaid.content_hash(), baseline.content_hash(), "the schema is content");

    let overlay = small_overlay();
    let encoded = overlay.encode();
    assert_eq!(overlay.digest(), xxh64(&encoded[..encoded.len() - 8], 0));
    assert_eq!(Overlay::decode(&encoded).unwrap().digest(), overlay.digest());
    assert_ne!(overlay_of(1, &overlay_records()).digest(), overlay.digest());
    let mut records = overlay_records();
    records.pop();
    assert_ne!(overlay_of(BASELINE_HASH, &records).digest(), overlay.digest());
}

#[test]
fn schemas_are_checked() {
    let rejects = |size: u32, fields: &[FieldLayout]| {
        let error = ComponentSchema::new(A, 1, size, fields.iter().copied()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    };
    rejects(4, &[field(1, 0, 2), field(1, 2, 2)]);
    rejects(4, &[field(1, 0, 3), field(2, 2, 2)]);
    rejects(4, &[field(1, 0, 2), field(2, 0, 2)]);
    rejects(4, &[field(1, 3, 2)]);
    rejects(4, &[field(1, 4, 1)]);
    rejects(4, &[field(1, u32::MAX, 2)]);
    rejects(4, &[field(1, 0, u32::MAX)]);
    rejects(4, &[field(1, 0, 0)]);
    rejects(0, &[field(1, 0, 1)]);
    rejects(MAX_COMPONENT_BYTES + 1, &[]);

    // Fields may leave padding between them and need not be given in order.
    let padded = schema(A, 1, 8, &[field(9, 6, 2), field(2, 0, 1), field(5, 2, 2)]);
    let ids: Vec<_> = padded.fields().iter().map(|field| field.id.0).collect();
    assert_eq!(ids, [2, 5, 9]);
    assert_eq!(schema(A, 1, MAX_COMPONENT_BYTES, &[]).element_size(), MAX_COMPONENT_BYTES);
}

#[test]
fn baseline_construction_rejects_inconsistent_rows() {
    let rejects = |rows: &[BaselineRow]| {
        let error = Baseline::from_rows(AssetSet::new(), schemas(), rows.to_vec()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    };
    rejects(&[row(runtime(1), &[a(1)])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(1), &[a(2)])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(1), &[b(2)])]);
    rejects(&[row(cooked(1), &[a(1), a(2)])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(2), &[data(A, &[0; 5])])]);
    rejects(&[row(cooked(1), &[data(A, &[0; 3])])]);
    rejects(&[row(cooked(1), &[data(C, &[0])])]);
    rejects(&[row(cooked(1), &[data(ComponentKey(0xEE), &[0])])]);

    let twice = [schema(A, 1, 4, &[]), schema(A, 2, 4, &[])];
    let error = Baseline::from_rows(AssetSet::new(), twice, [row(cooked(1), &[a(1)])]);
    assert_eq!(error.unwrap_err().kind(), ErrorKind::InvalidData);

    let largest = vec![0; MAX_COMPONENT_BYTES as usize];
    let large = [schema(A, 1, MAX_COMPONENT_BYTES, &[])];
    let baseline =
        Baseline::from_rows(AssetSet::new(), large, [row(cooked(1), &[data(A, &largest)])])
            .unwrap();
    assert_eq!(Baseline::decode(&baseline.encode()).unwrap(), baseline);
}

#[test]
fn overlay_construction_rejects_everything_that_is_not_canonical() {
    let rejects = |records: &[Record]| {
        let error = Overlay::new(0, schemas(), records.to_vec()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    };
    let (c1, c2, c3) = (cooked(1), cooked(2), cooked(3));
    let (r1, r2, r3) = (runtime(1), runtime(2), runtime(3));

    // The kind of ID each record takes.
    rejects(&[spawn(c1, &[a(1)])]);
    rejects(&[from_baseline(r1, &[a(1)])]);
    rejects(&[destroy(r1)]);
    // One record per (entity, kind, component, field).
    rejects(&[spawn(r1, &[a(1)]), spawn(r1, &[a(1)])]);
    rejects(&[destroy(c1), destroy(c1)]);
    rejects(&[add(c1, a(1)), add(c1, a(2))]);
    rejects(&[remove(c1, A), remove(c1, A)]);
    rejects(&[set_field(c1, A, 1), set_field(c1, A, 1)]);
    rejects(&[owner(c1, None), owner(c1, Some(c2))]);
    rejects(&[spawn(r1, &[a(1), a(2)])]);
    // A destroyed entity has nothing else.
    rejects(&[destroy(c1), from_baseline(c1, &[a(1)])]);
    rejects(&[destroy(c1), set_field(c1, A, 1)]);
    rejects(&[destroy(c1), owner(c1, None)]);
    // A spawned entity is described by its spawn alone.
    rejects(&[spawn(r1, &[a(1)]), add(r1, b(1))]);
    rejects(&[spawn(r1, &[a(1)]), add(r1, a(2))]);
    rejects(&[spawn(r1, &[a(1)]), remove(r1, B)]);
    rejects(&[spawn(r1, &[a(1)]), set_field(r1, A, 1)]);
    rejects(&[from_baseline(c1, &[a(1)]), add(c1, b(1))]);
    rejects(&[from_baseline(c1, &[a(1)]), remove(c1, A)]);
    rejects(&[from_baseline(c1, &[a(1)]), set_field(c1, A, 1)]);
    // A runtime entity exists here only through its spawn.
    rejects(&[set_field(r1, A, 1)]);
    rejects(&[add(r1, a(1))]);
    rejects(&[owner(r1, None)]);
    // A component is removed, written whole or written by field, one of the three.
    rejects(&[add(c1, a(1)), remove(c1, A)]);
    rejects(&[set_field(c1, A, 1), remove(c1, A)]);
    rejects(&[add(c1, a(1)), set_field(c1, A, 1)]);
    // Ownership.
    rejects(&[owner(c1, Some(c1))]);
    rejects(&[destroy(c1), owner(c2, Some(c1))]);
    rejects(&[owner(c1, Some(c2)), owner(c2, Some(c1))]);
    rejects(&[owner(c1, Some(c2)), owner(c2, Some(c3)), owner(c3, Some(c1))]);
    rejects(&[
        spawn(r1, &[]),
        spawn(r2, &[]),
        spawn(r3, &[]),
        owner(r1, Some(r2)),
        owner(r2, Some(r3)),
        owner(r3, Some(r2)),
    ]);
    // Sizes and fields come from the schema.
    rejects(&[add(c1, data(A, &[0; 3]))]);
    rejects(&[spawn(r1, &[data(B, &[0; 3])])]);
    rejects(&[add(c1, data(ComponentKey(0xEE), &[0]))]);
    rejects(&[set_field(c1, A, 2)]);
    rejects(&[set_field(c1, C, 1)]);
    rejects(&[Record::SetField { id: c1, component: A, field: FieldId(1), bytes: vec![0; 3] }]);
    rejects(&[Record::SetField { id: c1, component: A, field: FieldId(1), bytes: vec![] }]);
    rejects(&[Record::SetField {
        id: c1,
        component: ComponentKey(0xEE),
        field: FieldId(1),
        bytes: vec![0],
    }]);

    // The same component and field on different entities, different fields of one
    // component, and long acyclic ownership chains are all fine.
    let fine = [
        set_field(c1, A, 1),
        set_field(c1, A, 4),
        set_field(c2, A, 1),
        add(c1, b(1)),
        remove(c1, C),
        spawn(r1, &[a(1)]),
        owner(r1, Some(c1)),
        owner(c1, Some(c2)),
        owner(c2, Some(c3)),
        owner(c3, Some(cooked(99))),
        from_baseline(cooked(8), &[]),
        owner(cooked(8), Some(c3)),
    ];
    assert_eq!(Overlay::new(0, schemas(), fine).unwrap().records().len(), 12);
}

#[test]
fn rebase_keeps_what_still_has_a_target() {
    let records = vec![
        // Entity 1 leaves the baseline: everything about it is dropped.
        set_field(cooked(1), A, 1),
        set_field(cooked(1), B, 1),
        owner(cooked(1), Some(runtime(5))),
        // Entity 2 stays but loses component B.
        set_field(cooked(2), A, 1),
        set_field(cooked(2), B, 1),
        // Entity 3 stays.
        add(cooked(3), d(1)),
        remove(cooked(3), C),
        owner(cooked(3), None),
        // Entity 4 was destroyed here and has now left the baseline as well.
        destroy(cooked(4)),
        // Runtime entity 5 and migrated entity 7 are spawned by the overlay.
        spawn(runtime(5), &[a(5)]),
        owner(runtime(5), Some(cooked(3))),
        from_baseline(cooked(7), &[a(7)]),
    ];
    let original = baseline_of(&[
        row(cooked(1), &[a(1), b(1)]),
        row(cooked(2), &[a(2), b(2)]),
        row(cooked(3), &[a(3), c()]),
        row(cooked(4), &[a(4)]),
    ]);
    let overlay = overlay_of(original.content_hash(), &records);
    overlay.check_against(&original).unwrap();

    let patched = baseline_of(&[row(cooked(2), &[a(2)]), row(cooked(3), &[c(), a(3)])]);
    assert_eq!(overlay.check_against(&patched).unwrap_err().kind(), ErrorKind::VersionMismatch);
    let (rebased, dropped) = overlay.rebase(&patched).unwrap();

    assert_eq!(
        dropped,
        [
            set_field(cooked(1), A, 1),
            set_field(cooked(1), B, 1),
            owner(cooked(1), Some(runtime(5))),
            set_field(cooked(2), B, 1),
            destroy(cooked(4)),
        ]
    );
    let kept = [
        spawn(runtime(5), &[a(5)]),
        owner(runtime(5), Some(cooked(3))),
        set_field(cooked(2), A, 1),
        add(cooked(3), d(1)),
        remove(cooked(3), C),
        owner(cooked(3), None),
        from_baseline(cooked(7), &[a(7)]),
    ];
    assert_eq!(rebased.records(), kept);
    assert_eq!(rebased.baseline_hash(), patched.content_hash());
    rebased.check_against(&patched).unwrap();
    let used: Vec<_> = rebased.schemas().iter().map(ComponentSchema::component).collect();
    assert_eq!(used, [A, D], "the schema of B went with its last record");
    assert_eq!(Overlay::decode(&rebased.encode()).unwrap(), rebased, "still canonical");

    let (again, dropped_again) = rebased.rebase(&patched).unwrap();
    assert_eq!(again, rebased, "rebasing is idempotent");
    assert!(dropped_again.is_empty());
}

/// The reviewer's case: the patched row gained the component the overlay adds, never had
/// the one it removes, and the destroyed entity is gone from the baseline.
#[test]
fn rebase_onto_a_row_that_gained_the_added_component() {
    let original = baseline_of(&[row(cooked(1), &[a(1)])]);
    let patched = baseline_of(&[row(cooked(1), &[a(1), b(2)])]);
    let records = [add(cooked(1), b(7)), remove(cooked(1), C), destroy(cooked(9))];
    let overlay = overlay_of(original.content_hash(), &records);

    let (rebased, dropped) = overlay.rebase(&patched).unwrap();
    assert_eq!(rebased.records(), [add(cooked(1), b(7))], "kept: it overwrites the row's B");
    assert_eq!(dropped, [remove(cooked(1), C), destroy(cooked(9))]);
    rebased.check_against(&patched).unwrap();
    assert_eq!(rebased.rebase(&patched).unwrap(), (rebased.clone(), vec![]));
}

#[test]
fn rebase_rewrites_a_migrated_entity_that_became_a_local_row() {
    let original = baseline_of(&[row(cooked(1), &[a(1)])]);
    // The patch moves cooked entity 7 into this region, with components A, B and C.
    let patched = baseline_of(&[row(cooked(1), &[a(1)]), row(cooked(7), &[a(0), b(0), c()])]);
    let records = [from_baseline(cooked(7), &[a(7), d(7)]), owner(cooked(7), Some(cooked(1)))];
    let overlay = overlay_of(original.content_hash(), &records);
    overlay.check_against(&original).unwrap();

    let (rebased, dropped) = overlay.rebase(&patched).unwrap();
    assert!(dropped.is_empty());
    let local = [
        add(cooked(7), a(7)),
        add(cooked(7), d(7)),
        remove(cooked(7), B),
        remove(cooked(7), C),
        owner(cooked(7), Some(cooked(1))),
    ];
    assert_eq!(rebased.records(), local, "the same state, said as a local row says it");
    rebased.check_against(&patched).unwrap();
    assert_eq!(rebased.rebase(&patched).unwrap(), (rebased.clone(), vec![]));
}

#[test]
fn rebase_onto_the_same_baseline_changes_nothing() {
    let baseline = small_baseline();
    let records = [
        set_field(cooked(1), A, 1),
        set_field(cooked(1), B, 1),
        set_field(cooked(3), A, 1),
        remove(cooked(2), B),
        spawn(runtime(4), &[d(1)]),
        from_baseline(cooked(9), &[]),
    ];
    let overlay = overlay_of(baseline.content_hash(), &records);
    overlay.check_against(&baseline).unwrap();
    assert_eq!(overlay.rebase(&baseline).unwrap(), (overlay.clone(), vec![]));
}

#[test]
fn check_against_judges_what_needs_the_baseline() {
    let baseline = small_baseline();
    let fits =
        |records: &[Record]| overlay_of(baseline.content_hash(), records).check_against(&baseline);
    let rejects = |records: &[Record]| {
        assert_eq!(fits(records).unwrap_err().kind(), ErrorKind::InvalidData, "{records:?}");
    };
    // A row of this baseline is never spawned from a baseline.
    rejects(&[from_baseline(cooked(1), &[a(1)])]);
    // Records against rows that do not exist.
    rejects(&[destroy(cooked(9))]);
    rejects(&[add(cooked(9), a(1))]);
    rejects(&[remove(cooked(9), A)]);
    rejects(&[set_field(cooked(9), A, 1)]);
    rejects(&[owner(cooked(9), None)]);
    // Components the row does not have.
    rejects(&[remove(cooked(1), C)]);
    rejects(&[set_field(cooked(1), D, 1)]);

    fits(&[
        destroy(cooked(1)),
        add(cooked(2), a(9)),
        add(cooked(2), d(9)),
        remove(cooked(3), C),
        set_field(cooked(3), A, 4),
        from_baseline(cooked(9), &[a(1)]),
        owner(cooked(9), Some(cooked(2))),
        spawn(runtime(1), &[]),
    ])
    .unwrap();
}

/// Frames `body` as a document: header, body and a correct hash.
fn sealed(magic: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut bytes = magic.to_vec();
    bytes.extend_from_slice(&[1, 0, 0, 0]);
    bytes.extend_from_slice(body);
    reseal(&mut bytes);
    bytes
}

/// Appends the hash of `bytes`.
fn reseal(bytes: &mut Vec<u8>) {
    let hash = xxh64(bytes, 0);
    bytes.extend_from_slice(&hash.to_le_bytes());
}

fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn quads(quads: &[u64]) -> Vec<u8> {
    quads.iter().flat_map(|quad| quad.to_le_bytes()).collect()
}

/// One encoded schema: key, version, element size and (ID, offset, size) per field.
fn schema_bytes(key: u64, version: u32, size: u32, fields: &[(u16, u32, u32)]) -> Vec<u8> {
    let mut bytes = quads(&[key]);
    bytes.extend(words(&[version, size, fields.len() as u32]));
    for &(id, offset, size) in fields {
        bytes.extend(id.to_le_bytes());
        bytes.extend(words(&[offset, size]));
    }
    bytes
}

/// An encoded schema table.
fn table(schemas: &[Vec<u8>]) -> Vec<u8> {
    [words(&[schemas.len() as u32]), schemas.concat()].concat()
}

#[test]
fn decode_checks_the_frame() {
    let baseline = small_baseline().encode();
    let overlay = small_overlay().encode();
    assert_eq!(Overlay::decode(&baseline).unwrap_err().kind(), ErrorKind::InvalidData);
    assert_eq!(Baseline::decode(&overlay).unwrap_err().kind(), ErrorKind::InvalidData);

    let mut future = baseline[..baseline.len() - 8].to_vec();
    future[4] = 2;
    reseal(&mut future);
    assert_eq!(Baseline::decode(&future).unwrap_err().kind(), ErrorKind::VersionMismatch);

    let mut reserved = overlay[..overlay.len() - 8].to_vec();
    reserved[6] = 1;
    reseal(&mut reserved);
    assert_eq!(Overlay::decode(&reserved).unwrap_err().kind(), ErrorKind::InvalidData);

    let mut trailing = baseline[..baseline.len() - 8].to_vec();
    trailing.push(0);
    reseal(&mut trailing);
    assert_eq!(Baseline::decode(&trailing).unwrap_err().kind(), ErrorKind::InvalidData);
}

#[test]
fn decode_refuses_an_oversized_document_before_reading_it() {
    // Zeroed pages that are never touched: the length is all the decoder looks at.
    let oversized = vec![0u8; MAX_DOCUMENT_BYTES + 1];
    assert_eq!(Baseline::decode(&oversized).unwrap_err().kind(), ErrorKind::ResourceExhausted);
    assert_eq!(Overlay::decode(&oversized).unwrap_err().kind(), ErrorKind::ResourceExhausted);
}

/// Decoding allocates a bounded multiple of the input. These are the two densest cases,
/// and the README's "at most eight times" rests on these sizes on a 64-bit host.
#[test]
fn decoded_sizes_bound_the_memory_a_document_can_claim() {
    // An overlay of `Destroy` records: nine bytes each on the wire.
    assert!(size_of::<Record>() <= 8 * 9, "Record is {} bytes", size_of::<Record>());
    // A baseline of one-row archetypes without components: sixteen bytes each on the
    // wire, held as an archetype, one row ID and one index entry.
    let per_archetype = size_of::<Archetype>() + size_of::<NetId>() + size_of::<(NetId, u32)>();
    assert!(per_archetype <= 8 * 16, "an empty archetype costs {per_archetype} bytes");
}

#[test]
fn decode_rejects_well_sealed_documents_that_are_not_canonical() {
    let baseline = |body: Vec<u8>| Baseline::decode(&sealed(b"LBAS", &body));
    let overlay = |body: Vec<u8>| Overlay::decode(&sealed(b"LOVL", &body));
    let join = |parts: &[Vec<u8>]| parts.concat();
    let c1 = cooked(1).to_bits();
    let c2 = cooked(2).to_bits();
    let c3 = cooked(3).to_bits();

    // The smallest baseline with a row: no assets, component A of one byte in one field.
    let a_schema = schema_bytes(0x0A, 7, 1, &[(1, 0, 1)]);
    let a_table = table(core::slice::from_ref(&a_schema));
    let archetype = |ids: &[u64], column: &[u8]| {
        join(&[
            words(&[1]),
            quads(&[0x0A]),
            words(&[ids.len() as u32]),
            quads(ids),
            column.to_vec(),
        ])
    };
    let good = join(&[words(&[0]), a_table.clone(), words(&[1]), archetype(&[c1, c2], &[1, 2])]);
    let one_byte = [schema(A, 7, 1, &[field(1, 0, 1)])];
    let expected = Baseline::from_rows(
        AssetSet::new(),
        one_byte.clone(),
        [row(cooked(1), &[data(A, &[1])]), row(cooked(2), &[data(A, &[2])])],
    );
    assert_eq!(baseline(good).unwrap(), expected.unwrap());

    let with_archetypes = |assets: Vec<u8>, table: &[u8], archetypes: &[Vec<u8>]| {
        join(&[assets, table.to_vec(), words(&[archetypes.len() as u32]), archetypes.concat()])
    };
    let no_assets = || words(&[0]);
    let no_table = table(&[]);
    let one_row = || [archetype(&[c1], &[1])];
    let bad_baselines = [
        // Asset sets: descending, duplicated, zero.
        with_archetypes(join(&[words(&[2]), quads(&[2, 1])]), &no_table, &[]),
        with_archetypes(join(&[words(&[2]), quads(&[1, 1])]), &no_table, &[]),
        with_archetypes(join(&[words(&[1]), quads(&[0])]), &no_table, &[]),
        // A schema nothing uses, and a table that is descending or repeats a component.
        with_archetypes(no_assets(), &a_table, &[]),
        with_archetypes(
            no_assets(),
            &table(&[schema_bytes(0x0B, 1, 0, &[]), a_schema.clone()]),
            &[join(&[words(&[2]), quads(&[0x0A, 0x0B]), words(&[1]), quads(&[c1]), vec![1]])],
        ),
        with_archetypes(no_assets(), &table(&[a_schema.clone(), a_schema.clone()]), &one_row()),
        // Schemas: fields descending, repeated, overlapping, out of bounds, empty; too large.
        with_archetypes(
            no_assets(),
            &table(&[schema_bytes(0x0A, 7, 1, &[(2, 0, 1), (1, 0, 1)])]),
            &one_row(),
        ),
        with_archetypes(
            no_assets(),
            &table(&[schema_bytes(0x0A, 7, 1, &[(1, 0, 1), (1, 0, 1)])]),
            &one_row(),
        ),
        with_archetypes(
            no_assets(),
            &table(&[schema_bytes(0x0A, 7, 1, &[(1, 0, 1), (2, 0, 1)])]),
            &one_row(),
        ),
        with_archetypes(no_assets(), &table(&[schema_bytes(0x0A, 7, 1, &[(1, 1, 1)])]), &one_row()),
        with_archetypes(no_assets(), &table(&[schema_bytes(0x0A, 7, 1, &[(1, 0, 2)])]), &one_row()),
        with_archetypes(no_assets(), &table(&[schema_bytes(0x0A, 7, 1, &[(1, 0, 0)])]), &one_row()),
        with_archetypes(
            no_assets(),
            &table(&[schema_bytes(0x0A, 7, MAX_COMPONENT_BYTES + 1, &[])]),
            &[archetype(&[c1], &vec![0; MAX_COMPONENT_BYTES as usize + 1])],
        ),
        // Rows: descending, duplicated, zero, runtime, none.
        with_archetypes(no_assets(), &a_table, &[archetype(&[c2, c1], &[1, 2])]),
        with_archetypes(no_assets(), &a_table, &[archetype(&[c1, c1], &[1, 2])]),
        with_archetypes(no_assets(), &a_table, &[archetype(&[0], &[1])]),
        with_archetypes(no_assets(), &a_table, &[archetype(&[5], &[1])]),
        with_archetypes(no_assets(), &a_table, &[archetype(&[], &[])]),
        // The same archetype twice, and one entity in two archetypes.
        with_archetypes(no_assets(), &a_table, &[archetype(&[c1], &[1]), archetype(&[c2], &[2])]),
        with_archetypes(
            no_assets(),
            &a_table,
            &[join(&[words(&[0, 1]), quads(&[c1])]), archetype(&[c1], &[1])],
        ),
        // A component outside the table, and one listed twice in an archetype.
        with_archetypes(no_assets(), &no_table, &one_row()),
        with_archetypes(
            no_assets(),
            &a_table,
            &[join(&[words(&[2]), quads(&[0x0A, 0x0A]), words(&[1]), quads(&[c1]), vec![1, 1]])],
        ),
        // A column shorter than its rows.
        with_archetypes(no_assets(), &a_table, &[archetype(&[c1, c2], &[1])]),
        // Counts that the remaining bytes cannot hold.
        join(&[words(&[u32::MAX])]),
        join(&[words(&[0, u32::MAX])]),
        join(&[words(&[0, 1]), quads(&[0x0A]), words(&[7, 1, u32::MAX])]),
        join(&[words(&[0, 0, u32::MAX])]),
        with_archetypes(no_assets(), &a_table, &[join(&[words(&[u32::MAX])])]),
        with_archetypes(
            no_assets(),
            &a_table,
            &[join(&[words(&[1]), quads(&[0x0A]), words(&[u32::MAX])])],
        ),
    ];
    for (index, body) in bad_baselines.into_iter().enumerate() {
        let error = baseline(body).expect_err(&format!("baseline case {index}"));
        assert_eq!(error.kind(), ErrorKind::InvalidData, "baseline case {index}: {error}");
    }

    // Overlays: a hash, a schema table and records.
    let with_records = |table: &[u8], records: &[Vec<u8>]| {
        join(&[quads(&[0]), table.to_vec(), words(&[records.len() as u32]), records.concat()])
    };
    let record = |tag: u8, id: u64, payload: Vec<u8>| join(&[vec![tag], quads(&[id]), payload]);
    let spawn_empty = |id| record(1, id, words(&[0]));
    let destroy = |id| record(3, id, vec![]);
    let add_a = |id| record(4, id, join(&[quads(&[0x0A]), vec![9]]));
    let remove_a = |id| record(5, id, quads(&[0x0A]));
    let set_a = |id, field: u16| {
        record(6, id, join(&[quads(&[0x0A]), field.to_le_bytes().to_vec(), vec![9]]))
    };
    let owned_by = |id, owner| record(7, id, quads(&[owner]));
    let good = with_records(&a_table, &[add_a(c1), destroy(c2), set_a(c3, 1)]);
    let expected = Overlay::new(
        0,
        one_byte,
        [
            add(cooked(1), data(A, &[9])),
            Record::Destroy { id: cooked(2) },
            Record::SetField { id: cooked(3), component: A, field: FieldId(1), bytes: vec![9] },
        ],
    );
    assert_eq!(overlay(good).unwrap(), expected.unwrap());

    let bad_overlays = [
        // Record order and duplicates.
        with_records(&no_table, &[destroy(c2), destroy(c1)]),
        with_records(&no_table, &[destroy(c1), destroy(c1)]),
        // IDs: zero, and the wrong kind for the record.
        with_records(&no_table, &[destroy(0)]),
        with_records(&no_table, &[destroy(5)]),
        with_records(&no_table, &[spawn_empty(c1)]),
        with_records(&no_table, &[record(2, 5, words(&[0]))]),
        // Tags outside the set.
        with_records(&no_table, &[record(0, c1, vec![])]),
        with_records(&no_table, &[record(8, c1, vec![])]),
        // Schema table: unused entry, missing entry, descending.
        with_records(&a_table, &[destroy(c1)]),
        with_records(&no_table, &[add_a(c1)]),
        with_records(&table(&[schema_bytes(0x0B, 1, 0, &[]), a_schema.clone()]), &[add_a(c1)]),
        // A field the schema lacks.
        with_records(&a_table, &[set_a(c1, 2)]),
        // Contradictions within one entity.
        with_records(&a_table, &[add_a(c1), remove_a(c1)]),
        with_records(&a_table, &[add_a(c1), set_a(c1, 1)]),
        with_records(&a_table, &[remove_a(c1), set_a(c1, 1)]),
        with_records(&no_table, &[destroy(c1), remove_a(c1)]),
        with_records(&a_table, &[spawn_empty(5), add_a(5)]),
        with_records(&no_table, &[spawn_empty(5), remove_a(5)]),
        with_records(&a_table, &[record(2, c1, words(&[0])), set_a(c1, 1)]),
        with_records(&a_table, &[add_a(5)]),
        with_records(&no_table, &[owned_by(5, 0)]),
        // Ownership: self, a destroyed owner, a cycle.
        with_records(&no_table, &[owned_by(c1, c1)]),
        with_records(&no_table, &[destroy(c1), owned_by(c2, c1)]),
        with_records(&no_table, &[owned_by(c1, c2), owned_by(c2, c1)]),
        // Data cut short by the end of the document.
        with_records(&a_table, &[record(4, c1, quads(&[0x0A]))]),
        // Counts that the remaining bytes cannot hold.
        join(&[quads(&[0]), words(&[u32::MAX])]),
        join(&[quads(&[0]), words(&[0, u32::MAX])]),
        with_records(&no_table, &[record(1, 5, words(&[u32::MAX]))]),
        // A record cut short.
        with_records(&no_table, &[vec![3], vec![0; 4]]),
    ];
    for (index, body) in bad_overlays.into_iter().enumerate() {
        let error = overlay(body).expect_err(&format!("overlay case {index}"));
        assert_eq!(error.kind(), ErrorKind::InvalidData, "overlay case {index}: {error}");
    }
}

/// One of the two decoders, as a function that reports the re-encoding of what it accepted.
type Decoder = fn(&[u8]) -> lactose_core::Result<Vec<u8>>;

fn decoders() -> [(Decoder, Vec<u8>); 2] {
    [
        (
            |bytes| Baseline::decode(bytes).map(|document| document.encode()),
            small_baseline().encode(),
        ),
        (
            |bytes| Overlay::decode(bytes).map(|document| document.encode()),
            small_overlay().encode(),
        ),
    ]
}

#[test]
fn decode_rejects_every_truncation_and_every_bit_flip() {
    for (decode, good) in decoders() {
        assert_eq!(decode(&good).unwrap(), good);
        for len in 0..good.len() {
            assert!(decode(&good[..len]).is_err(), "truncated to {len} bytes");
        }
        for bit in 0..good.len() * 8 {
            let mut flipped = good.clone();
            flipped[bit / 8] ^= 1 << (bit % 8);
            assert!(decode(&flipped).is_err(), "bit {bit} flipped");
        }
    }
}

/// Corrupts `bytes` in one of the ways a damaged or hostile document differs from a good
/// one. Counts, offsets and sizes are `u32`s, so overwriting four bytes is what hits them.
fn corrupt(rng: &mut Rng, bytes: &mut Vec<u8>) {
    let at = rng.below(bytes.len());
    match rng.below(6) {
        0 => bytes[at] = rng.next() as u8,
        1 | 2 => {
            let interesting = [0, 1, 2, 0x7FFF_FFFF, 0x8000_0000, u32::MAX, bytes.len() as u32];
            let value = match rng.below(3) {
                0 => rng.next() as u32,
                1 => rng.below(64) as u32,
                _ => interesting[rng.below(interesting.len())],
            };
            let end = (at + 4).min(bytes.len());
            bytes[at..end].copy_from_slice(&value.to_le_bytes()[..end - at]);
        }
        3 => bytes.truncate(at),
        4 => bytes.insert(at, rng.next() as u8),
        _ => {
            bytes.remove(at);
        }
    }
}

#[test]
fn decode_survives_seeded_corruption() {
    for (seed, (decode, good)) in decoders().into_iter().enumerate() {
        let mut rng = Rng(0x5EED + seed as u64);
        let mut accepted = 0;
        for case in 0..40_000 {
            let mut bytes = good[..good.len() - 8].to_vec();
            for _ in 0..=rng.below(3) {
                if !bytes.is_empty() {
                    corrupt(&mut rng, &mut bytes);
                }
            }
            // Half the cases carry the right hash for their corrupted contents, so the
            // decoder's own checks are what stands between them and a document.
            let resealed = rng.below(2) == 0;
            if resealed {
                reseal(&mut bytes);
            } else {
                bytes.extend_from_slice(&good[good.len() - 8..]);
            }
            match decode(&bytes) {
                // Whatever decodes is the canonical encoding of what it decoded to.
                Ok(encoded) => {
                    assert_eq!(encoded, bytes, "case {case} is accepted but not canonical");
                    assert!(resealed || bytes == good, "case {case} passed with a stale hash");
                    accepted += 1;
                }
                Err(error) => {
                    let kind = error.kind();
                    assert!(
                        matches!(kind, ErrorKind::InvalidData | ErrorKind::VersionMismatch),
                        "case {case}: {error}"
                    );
                }
            }
        }
        // Corrupting opaque component bytes yields another valid document, so some cases
        // must get through; if none do, the generator is not reaching past the frame.
        assert!(accepted > 100, "only {accepted} corrupted documents were well-formed");
    }
}
