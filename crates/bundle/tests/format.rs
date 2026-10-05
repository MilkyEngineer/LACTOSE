//! The bundle format from the outside: golden bytes, round trips, canonical form, the
//! patched-baseline rule and decoder robustness.

use core::num::NonZeroU64;

use lactose_asset_api::{AssetId, AssetSet};
use lactose_bundle::{Baseline, BaselineRow, ComponentData, MAX_COMPONENT_BYTES, Overlay, Record};
use lactose_core::{ComponentKey, ErrorKind, FieldId, NetId, xxh64};

const A: ComponentKey = ComponentKey(0x0A);
const B: ComponentKey = ComponentKey(0x0B);
const C: ComponentKey = ComponentKey(0x0C);
const D: ComponentKey = ComponentKey(0x0D);

fn cooked(n: u64) -> NetId {
    NetId::cooked(n)
}

fn runtime(n: u64) -> NetId {
    NetId::runtime(NonZeroU64::new(n).unwrap()).unwrap()
}

fn data(component: ComponentKey, schema_version: u32, bytes: &[u8]) -> ComponentData {
    ComponentData { component, schema_version, bytes: bytes.to_vec() }
}

fn a(fill: u8) -> ComponentData {
    data(A, 1, &[fill; 4])
}

fn b(fill: u8) -> ComponentData {
    data(B, 3, &[fill; 2])
}

/// A zero-sized tag component.
fn c() -> ComponentData {
    data(C, 1, &[])
}

fn row(id: NetId, components: &[ComponentData]) -> BaselineRow {
    BaselineRow { id, components: components.to_vec() }
}

fn set_field(id: NetId, component: ComponentKey, field: u16) -> Record {
    let schema_version = if component == B { 3 } else { 1 };
    Record::SetField {
        id,
        component,
        schema_version,
        field: FieldId(field),
        bytes: vec![0xEE, 0xFF],
    }
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

fn small_baseline() -> Baseline {
    Baseline::from_rows(assets(), baseline_rows()).unwrap()
}

fn overlay_records() -> Vec<Record> {
    vec![
        Record::Spawn { id: runtime(5), components: vec![b(9), a(5)] },
        Record::SpawnFromBaseline { id: cooked(7), components: vec![a(7)] },
        Record::Destroy { id: cooked(1) },
        Record::AddComponent { id: cooked(2), data: c() },
        Record::RemoveComponent { id: cooked(2), component: B },
        set_field(cooked(2), A, 4),
        Record::OwnershipChange { id: cooked(2), owner: Some(runtime(5)) },
        Record::OwnershipChange { id: runtime(5), owner: None },
    ]
}

fn small_overlay() -> Overlay {
    Overlay::new(0x0123_4567_89AB_CDEF, overlay_records()).unwrap()
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
0b 00 00 00 00 00 00 00 03 00 00 00 02 00 00 00\n\
0c 00 00 00 00 00 00 00 01 00 00 00 00 00 00 00\n\
02 00 00 00 02 00 00 00 0a 00 00 00 00 00 00 00\n\
0b 00 00 00 00 00 00 00 02 00 00 00 01 00 00 00\n\
00 00 00 80 02 00 00 00 00 00 00 80 01 01 01 01\n\
02 02 02 02 b1 b1 b2 b2 02 00 00 00 0a 00 00 00\n\
00 00 00 00 0c 00 00 00 00 00 00 00 01 00 00 00\n\
03 00 00 00 00 00 00 80 03 03 03 03 f9 64 21 2e\n\
9b cb ba 11";

const GOLDEN_OVERLAY: &str = "\
4c 4f 56 4c 01 00 00 00 ef cd ab 89 67 45 23 01\n\
03 00 00 00 0a 00 00 00 00 00 00 00 01 00 00 00\n\
0b 00 00 00 00 00 00 00 03 00 00 00 0c 00 00 00\n\
00 00 00 00 01 00 00 00 08 00 00 00 01 05 00 00\n\
00 00 00 00 00 02 00 00 00 0a 00 00 00 00 00 00\n\
00 04 00 00 00 05 05 05 05 0b 00 00 00 00 00 00\n\
00 02 00 00 00 09 09 07 05 00 00 00 00 00 00 00\n\
00 00 00 00 00 00 00 00 03 01 00 00 00 00 00 00\n\
80 04 02 00 00 00 00 00 00 80 0c 00 00 00 00 00\n\
00 00 00 00 00 00 05 02 00 00 00 00 00 00 80 0b\n\
00 00 00 00 00 00 00 06 02 00 00 00 00 00 00 80\n\
0a 00 00 00 00 00 00 00 04 00 02 00 00 00 ee ff\n\
07 02 00 00 00 00 00 00 80 05 00 00 00 00 00 00\n\
00 02 07 00 00 00 00 00 00 80 01 00 00 00 0a 00\n\
00 00 00 00 00 00 04 00 00 00 07 07 07 07 03 32\n\
83 40 2f e6 bc e3";

#[test]
fn baseline_matches_its_golden_bytes() {
    let bytes = small_baseline().encode();
    assert_eq!(hex(&bytes), GOLDEN_BASELINE);
    assert_eq!(Baseline::decode(&unhex(GOLDEN_BASELINE)).unwrap(), small_baseline());
    assert_eq!(small_baseline().content_hash(), 0x11ba_cb9b_2e21_64f9);
}

#[test]
fn overlay_matches_its_golden_bytes() {
    let bytes = small_overlay().encode();
    assert_eq!(hex(&bytes), GOLDEN_OVERLAY);
    assert_eq!(Overlay::decode(&unhex(GOLDEN_OVERLAY)).unwrap(), small_overlay());
    assert_eq!(small_overlay().digest(), 0xe3bc_e62f_4083_3203);
}

#[test]
fn baseline_round_trips_and_exposes_its_rows() {
    let baseline = small_baseline();
    assert_eq!(Baseline::decode(&baseline.encode()).unwrap(), baseline);
    assert_eq!(baseline.assets(), &assets());

    let [with_b, with_c] = baseline.archetypes() else { panic!("two archetypes") };
    assert_eq!(with_b.ids(), [cooked(1), cooked(2)]);
    let keys = |columns: &[lactose_bundle::Column]| {
        columns.iter().map(|c| c.component()).collect::<Vec<_>>()
    };
    assert_eq!(keys(with_b.columns()), [A, B]);
    let column = with_b.column(B).unwrap();
    assert_eq!((column.schema_version(), column.element_size()), (3, 2));
    assert_eq!(column.bytes(), [0xB1, 0xB1, 0xB2, 0xB2], "rows are in ID order");
    assert_eq!(with_b.column(A).unwrap().bytes(), [1, 1, 1, 1, 2, 2, 2, 2]);
    assert!(with_b.column(C).is_none());

    assert_eq!(with_c.ids(), [cooked(3)]);
    assert_eq!(keys(with_c.columns()), [A, C]);
    assert_eq!(with_c.column(C).unwrap().element_size(), 0);
    assert!(with_c.column(C).unwrap().bytes().is_empty());

    assert!(baseline.contains(cooked(2)));
    assert!(!baseline.contains(cooked(4)));
    assert_eq!(baseline.archetype_of(cooked(3)), Some(with_c));
}

#[test]
fn overlay_round_trips_and_orders_its_records() {
    let overlay = small_overlay();
    assert_eq!(Overlay::decode(&overlay.encode()).unwrap(), overlay);
    assert_eq!(overlay.baseline_hash(), 0x0123_4567_89AB_CDEF);
    let expected = [
        Record::Spawn { id: runtime(5), components: vec![a(5), b(9)] },
        Record::OwnershipChange { id: runtime(5), owner: None },
        Record::Destroy { id: cooked(1) },
        Record::AddComponent { id: cooked(2), data: c() },
        Record::RemoveComponent { id: cooked(2), component: B },
        set_field(cooked(2), A, 4),
        Record::OwnershipChange { id: cooked(2), owner: Some(runtime(5)) },
        Record::SpawnFromBaseline { id: cooked(7), components: vec![a(7)] },
    ];
    assert_eq!(overlay.records(), expected);
}

#[test]
fn empty_documents_round_trip() {
    let baseline = Baseline::from_rows(AssetSet::new(), []).unwrap();
    assert_eq!(baseline.encode(), sealed(b"LBAS", &[0; 12]));
    assert_eq!(Baseline::decode(&baseline.encode()).unwrap(), baseline);
    let overlay = Overlay::new(0, []).unwrap();
    assert_eq!(overlay.encode(), sealed(b"LOVL", &[0; 16]));
    assert_eq!(Overlay::decode(&overlay.encode()).unwrap(), overlay);
    // An entity with no components is still a row.
    let bare = Baseline::from_rows(AssetSet::new(), [row(cooked(1), &[])]).unwrap();
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
        let mut rows = baseline_rows();
        rng.shuffle(&mut rows);
        for row in &mut rows {
            rng.shuffle(&mut row.components);
        }
        let reordered = Baseline::from_rows(reversed_assets.clone(), rows).unwrap();
        assert_eq!(reordered, small_baseline());
        assert_eq!(reordered.encode(), baseline);

        let mut records = overlay_records();
        rng.shuffle(&mut records);
        for record in &mut records {
            if let Record::Spawn { components, .. } = record {
                rng.shuffle(components);
            }
        }
        let reordered = Overlay::new(0x0123_4567_89AB_CDEF, records).unwrap();
        assert_eq!(reordered.encode(), overlay);
    }
}

#[test]
fn hashes_follow_content() {
    let baseline = small_baseline();
    let encoded = baseline.encode();
    let (body, trailer) = encoded.split_at(encoded.len() - 8);
    assert_eq!(baseline.content_hash(), xxh64(body, 0));
    assert_eq!(baseline.content_hash().to_le_bytes(), trailer);

    let mut rows = baseline_rows();
    rows[0].components[0].bytes[0] ^= 1;
    assert_ne!(
        Baseline::from_rows(assets(), rows).unwrap().content_hash(),
        baseline.content_hash()
    );
    let fewer_assets = Baseline::from_rows(AssetSet::new(), baseline_rows()).unwrap();
    assert_ne!(fewer_assets.content_hash(), baseline.content_hash());

    let overlay = small_overlay();
    let encoded = overlay.encode();
    assert_eq!(overlay.digest(), xxh64(&encoded[..encoded.len() - 8], 0));
    assert_ne!(Overlay::new(1, overlay_records()).unwrap().digest(), overlay.digest());
    let mut records = overlay_records();
    records.pop();
    assert_ne!(Overlay::new(0x0123_4567_89AB_CDEF, records).unwrap().digest(), overlay.digest());
}

#[test]
fn baseline_construction_rejects_inconsistent_rows() {
    let rejects = |rows: &[BaselineRow]| {
        let error = Baseline::from_rows(AssetSet::new(), rows.to_vec()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    };
    rejects(&[row(runtime(1), &[a(1)])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(1), &[a(2)])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(1), &[b(2)])]);
    rejects(&[row(cooked(1), &[a(1), a(2)])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(2), &[data(A, 1, &[0; 5])])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(2), &[data(A, 2, &[0; 4])])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(2), &[data(A, 1, &[0; 5]), c()])]);
    rejects(&[row(cooked(1), &[a(1)]), row(cooked(2), &[data(A, 2, &[0; 4]), c()])]);
    let too_large = vec![0; MAX_COMPONENT_BYTES as usize + 1];
    rejects(&[row(cooked(1), &[data(A, 1, &too_large)])]);

    let largest = vec![0; MAX_COMPONENT_BYTES as usize];
    let baseline =
        Baseline::from_rows(AssetSet::new(), [row(cooked(1), &[data(A, 1, &largest)])]).unwrap();
    assert_eq!(Baseline::decode(&baseline.encode()).unwrap(), baseline);
}

#[test]
fn overlay_construction_rejects_contradictions() {
    let rejects = |records: &[Record]| {
        let error = Overlay::new(0, records.to_vec()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    };
    let spawn = |id| Record::Spawn { id, components: vec![a(1)] };
    let from_baseline = |id| Record::SpawnFromBaseline { id, components: vec![a(1)] };
    let add = |id, data| Record::AddComponent { id, data };
    let remove = |id, component| Record::RemoveComponent { id, component };
    let owner = |id, owner| Record::OwnershipChange { id, owner };

    rejects(&[spawn(cooked(1))]);
    rejects(&[from_baseline(runtime(1))]);
    rejects(&[Record::Destroy { id: runtime(1) }]);
    rejects(&[spawn(runtime(1)), spawn(runtime(1))]);
    rejects(&[Record::Destroy { id: cooked(1) }, Record::Destroy { id: cooked(1) }]);
    rejects(&[Record::Destroy { id: cooked(1) }, from_baseline(cooked(1))]);
    rejects(&[Record::Destroy { id: cooked(1) }, set_field(cooked(1), A, 1)]);
    rejects(&[Record::Destroy { id: cooked(1) }, owner(cooked(1), None)]);
    rejects(&[add(cooked(1), a(1)), add(cooked(1), a(2))]);
    rejects(&[add(cooked(1), a(1)), remove(cooked(1), A)]);
    rejects(&[set_field(cooked(1), A, 1), remove(cooked(1), A)]);
    rejects(&[remove(cooked(1), A), remove(cooked(1), A)]);
    rejects(&[set_field(cooked(1), A, 1), set_field(cooked(1), A, 1)]);
    rejects(&[owner(cooked(1), None), owner(cooked(1), Some(cooked(2)))]);
    rejects(&[owner(cooked(1), Some(cooked(1)))]);
    rejects(&[Record::Spawn { id: runtime(1), components: vec![a(1), a(2)] }]);
    rejects(&[add(cooked(1), a(1)), add(cooked(2), data(A, 2, &[0; 4]))]);
    rejects(&[add(cooked(1), data(A, 2, &[0; 4])), set_field(cooked(2), A, 1)]);
    let too_large = vec![0; MAX_COMPONENT_BYTES as usize + 1];
    rejects(&[add(cooked(1), data(A, 1, &too_large))]);

    // The same component and field on different entities, and different fields of one
    // component, are distinct records.
    let fine = [
        set_field(cooked(1), A, 1),
        set_field(cooked(1), A, 2),
        set_field(cooked(2), A, 1),
        add(cooked(1), b(1)),
        remove(cooked(1), C),
        spawn(runtime(1)),
        set_field(runtime(1), A, 1),
    ];
    assert_eq!(Overlay::new(0, fine).unwrap().records().len(), 7);
}

#[test]
fn rebase_drops_only_set_fields_whose_target_is_gone() {
    let add_d = Record::AddComponent { id: cooked(3), data: data(D, 1, &[1]) };
    let records = vec![
        // Entity 1 leaves the baseline.
        set_field(cooked(1), A, 1),
        set_field(cooked(1), B, 2),
        Record::OwnershipChange { id: cooked(1), owner: Some(runtime(5)) },
        // Entity 2 stays but loses component B.
        set_field(cooked(2), A, 1),
        set_field(cooked(2), B, 2),
        // Entity 3 stays; the overlay itself adds D.
        add_d.clone(),
        set_field(cooked(3), D, 1),
        // Entity 4 was never in either baseline.
        Record::Destroy { id: cooked(4) },
        // Runtime entity 5 and migrated entity 7 are spawned by the overlay.
        Record::Spawn { id: runtime(5), components: vec![a(5)] },
        set_field(runtime(5), B, 1),
        Record::SpawnFromBaseline { id: cooked(7), components: vec![a(7)] },
        set_field(cooked(7), A, 1),
        // Runtime entity 6 has no spawn record.
        set_field(runtime(6), A, 1),
    ];
    let original = small_baseline();
    let overlay = Overlay::new(original.content_hash(), records).unwrap();

    let patched =
        Baseline::from_rows(assets(), [row(cooked(2), &[a(2)]), row(cooked(3), &[c(), a(3)])])
            .unwrap();
    let (rebased, dropped) = overlay.rebase(&patched);

    assert_eq!(
        dropped,
        [
            set_field(runtime(6), A, 1),
            set_field(cooked(1), A, 1),
            set_field(cooked(1), B, 2),
            set_field(cooked(2), B, 2),
        ]
    );
    let kept = [
        Record::Spawn { id: runtime(5), components: vec![a(5)] },
        set_field(runtime(5), B, 1),
        Record::OwnershipChange { id: cooked(1), owner: Some(runtime(5)) },
        set_field(cooked(2), A, 1),
        add_d,
        set_field(cooked(3), D, 1),
        Record::Destroy { id: cooked(4) },
        Record::SpawnFromBaseline { id: cooked(7), components: vec![a(7)] },
        set_field(cooked(7), A, 1),
    ];
    assert_eq!(rebased.records(), kept);
    assert_eq!(rebased.baseline_hash(), patched.content_hash());
    assert_eq!(Overlay::decode(&rebased.encode()).unwrap(), rebased, "still canonical");

    let (again, dropped_again) = rebased.rebase(&patched);
    assert_eq!(again, rebased, "rebasing is idempotent");
    assert!(dropped_again.is_empty());
}

#[test]
fn rebase_onto_the_same_baseline_keeps_valid_records() {
    let baseline = small_baseline();
    let records = [
        set_field(cooked(1), A, 1),
        set_field(cooked(1), B, 1),
        set_field(cooked(3), A, 1),
        Record::RemoveComponent { id: cooked(2), component: B },
        Record::Destroy { id: cooked(9) },
    ];
    let overlay = Overlay::new(baseline.content_hash(), records).unwrap();
    assert_eq!(overlay.rebase(&baseline), (overlay.clone(), vec![]));
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
fn decode_rejects_well_sealed_documents_that_are_not_canonical() {
    let baseline = |body: Vec<u8>| Baseline::decode(&sealed(b"LBAS", &body));
    let overlay = |body: Vec<u8>| Overlay::decode(&sealed(b"LOVL", &body));
    let join = |parts: &[Vec<u8>]| parts.concat();
    let c1 = cooked(1).to_bits();
    let c2 = cooked(2).to_bits();

    // The smallest baseline with a row: no assets, component A of size 1, one archetype.
    let table = join(&[words(&[1]), quads(&[0x0A]), words(&[7, 1])]);
    let archetype = |ids: &[u64], column: &[u8]| {
        join(&[
            words(&[1]),
            quads(&[0x0A]),
            words(&[ids.len() as u32]),
            quads(ids),
            column.to_vec(),
        ])
    };
    let good = join(&[words(&[0]), table.clone(), words(&[1]), archetype(&[c1, c2], &[1, 2])]);
    let expected = Baseline::from_rows(
        AssetSet::new(),
        [row(cooked(1), &[data(A, 7, &[1])]), row(cooked(2), &[data(A, 7, &[2])])],
    );
    assert_eq!(baseline(good).unwrap(), expected.unwrap());

    let with_archetypes = |assets: Vec<u8>, table: &[u8], archetypes: &[Vec<u8>]| {
        join(&[assets, table.to_vec(), words(&[archetypes.len() as u32]), archetypes.concat()])
    };
    let no_assets = || words(&[0]);
    let bad_baselines = [
        // Asset sets: descending, duplicated, zero.
        with_archetypes(join(&[words(&[2]), quads(&[2, 1])]), &words(&[0]), &[]),
        with_archetypes(join(&[words(&[2]), quads(&[1, 1])]), &words(&[0]), &[]),
        with_archetypes(join(&[words(&[1]), quads(&[0])]), &words(&[0]), &[]),
        // A component table entry nothing uses.
        with_archetypes(no_assets(), &table, &[]),
        // Rows: descending, duplicated, zero, runtime, none.
        with_archetypes(no_assets(), &table, &[archetype(&[c2, c1], &[1, 2])]),
        with_archetypes(no_assets(), &table, &[archetype(&[c1, c1], &[1, 2])]),
        with_archetypes(no_assets(), &table, &[archetype(&[0], &[1])]),
        with_archetypes(no_assets(), &table, &[archetype(&[5], &[1])]),
        with_archetypes(no_assets(), &table, &[archetype(&[], &[])]),
        // The same archetype twice, and one entity in two archetypes.
        with_archetypes(no_assets(), &table, &[archetype(&[c1], &[1]), archetype(&[c2], &[2])]),
        with_archetypes(
            no_assets(),
            &table,
            &[join(&[words(&[0, 1]), quads(&[c1])]), archetype(&[c1], &[1])],
        ),
        // A component outside the table, and one listed twice in an archetype.
        with_archetypes(no_assets(), &words(&[0]), &[archetype(&[c1], &[1])]),
        with_archetypes(
            no_assets(),
            &table,
            &[join(&[words(&[2]), quads(&[0x0A, 0x0A]), words(&[1]), quads(&[c1]), vec![1, 1]])],
        ),
        // A component larger than the limit.
        with_archetypes(
            no_assets(),
            &join(&[words(&[1]), quads(&[0x0A]), words(&[7, MAX_COMPONENT_BYTES + 1])]),
            &[archetype(&[c1], &vec![0; MAX_COMPONENT_BYTES as usize + 1])],
        ),
        // Counts that the remaining bytes cannot hold.
        join(&[words(&[u32::MAX])]),
        join(&[words(&[0, u32::MAX])]),
        join(&[words(&[0, 0, u32::MAX])]),
        with_archetypes(no_assets(), &table, &[join(&[words(&[u32::MAX])])]),
        with_archetypes(
            no_assets(),
            &table,
            &[join(&[words(&[1]), quads(&[0x0A]), words(&[u32::MAX])])],
        ),
    ];
    for (index, body) in bad_baselines.into_iter().enumerate() {
        let error = baseline(body).expect_err(&format!("baseline case {index}"));
        assert_eq!(error.kind(), ErrorKind::InvalidData, "baseline case {index}: {error}");
    }

    // Overlays: a hash, a schema table and records.
    let schemas = join(&[words(&[1]), quads(&[0x0A]), words(&[1])]);
    let no_schemas = words(&[0]);
    let with_records = |schemas: &[u8], records: &[Vec<u8>]| {
        join(&[quads(&[0]), schemas.to_vec(), words(&[records.len() as u32]), records.concat()])
    };
    let record = |tag: u8, id: u64, payload: Vec<u8>| join(&[vec![tag], quads(&[id]), payload]);
    let destroy = |id| record(3, id, vec![]);
    let add_a = |id| record(4, id, join(&[quads(&[0x0A]), words(&[1]), vec![9]]));
    let remove_a = |id| record(5, id, quads(&[0x0A]));
    let good = with_records(&schemas, &[add_a(c1), destroy(c2)]);
    let expected = Overlay::new(
        0,
        [
            Record::AddComponent { id: cooked(1), data: data(A, 1, &[9]) },
            Record::Destroy { id: cooked(2) },
        ],
    );
    assert_eq!(overlay(good).unwrap(), expected.unwrap());

    let bad_overlays = [
        // Record order and duplicates.
        with_records(&no_schemas, &[destroy(c2), destroy(c1)]),
        with_records(&no_schemas, &[destroy(c1), destroy(c1)]),
        // IDs: zero, and the wrong kind for the record.
        with_records(&no_schemas, &[destroy(0)]),
        with_records(&no_schemas, &[destroy(5)]),
        with_records(&no_schemas, &[record(1, c1, words(&[0]))]),
        with_records(&no_schemas, &[record(2, 5, words(&[0]))]),
        // Tags outside the set.
        with_records(&no_schemas, &[record(0, c1, vec![])]),
        with_records(&no_schemas, &[record(8, c1, vec![])]),
        // Schema table: unused entry, missing entry, descending.
        with_records(&schemas, &[destroy(c1)]),
        with_records(&no_schemas, &[add_a(c1)]),
        with_records(
            &join(&[words(&[2]), quads(&[0x0B]), words(&[1]), quads(&[0x0A]), words(&[1])]),
            &[add_a(c1)],
        ),
        // Contradictions.
        with_records(&schemas, &[add_a(c1), remove_a(c1)]),
        with_records(&schemas, &[destroy(c1), remove_a(c1)]),
        with_records(&no_schemas, &[record(7, c1, quads(&[c1]))]),
        // Data longer than the limit or than the document.
        with_records(
            &schemas,
            &[record(4, c1, join(&[quads(&[0x0A]), words(&[MAX_COMPONENT_BYTES + 1])]))],
        ),
        with_records(&schemas, &[record(4, c1, join(&[quads(&[0x0A]), words(&[2]), vec![9]]))]),
        // Counts that the remaining bytes cannot hold.
        join(&[quads(&[0]), words(&[u32::MAX])]),
        join(&[quads(&[0]), words(&[0, u32::MAX])]),
        with_records(&no_schemas, &[record(1, 5, words(&[u32::MAX]))]),
        // A record cut short.
        with_records(&no_schemas, &[vec![3], vec![0; 4]]),
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
/// one. Lengths and counts are `u32`s, so overwriting four bytes is what hits them.
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
