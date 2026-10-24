//! The container's bytes (README, "The container"): a small container written out field by
//! field from the layout `src/container.rs` states, then pinned as golden bytes, so any
//! change to the encoding fails here and moves `CONTAINER_FORMAT_VERSION`. The section's
//! schema table is `lactose-bundle`'s encoding of one, byte for byte.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, Overlay, Record};
use lactose_core::{NetId, Tick, xxh64};
use lactose_ecs::RegionId;
use lactose_persist::{
    CONTAINER_FORMAT_VERSION, Container, ContainerHeader, EngineVersion, PREAMBLE_BYTES,
    RegionRecords, SchemaHash, SchemaTable,
};
use lactose_reflect::{Reflect, Registry};

fn table() -> SchemaTable {
    let mut registry = Registry::new();
    registry.register::<Position>().unwrap();
    registry.register::<Health>().unwrap();
    registry.register::<Weather>().unwrap();
    registry.register::<Score>().unwrap();
    registry.register::<Rain>().unwrap();
    SchemaTable::from_registry(&registry).unwrap()
}

const REGION: RegionId = RegionId(5);

fn small_baseline() -> Baseline {
    let row = BaselineRow { id: NetId::cooked(77), components: vec![data(Health(1))] };
    Baseline::from_rows(AssetSet::new(), cooked_schemas(), [row]).unwrap()
}

/// Records with an overlay, two resources and a transient key.
fn records(table: &SchemaTable) -> RegionRecords {
    let baseline = small_baseline();
    let spawned = NetId::from_bits(3).unwrap();
    let records = [
        Record::Spawn { id: spawned, components: vec![data(Health(7))] },
        Record::AddComponent { id: NetId::cooked(77), data: data(Health(2)) },
    ];
    let overlay = Overlay::new(baseline.content_hash(), table.schemas().cloned(), records).unwrap();
    let mut resources = vec![data(Weather(3)), data(Score(9))];
    resources.sort_by_key(|data| data.component);
    let resource_schemas =
        resources.iter().map(|data| table.schema(data.component).unwrap().clone()).collect();
    let layouts: BTreeMap<_, _> = [Health::KEY, Weather::KEY, Score::KEY]
        .into_iter()
        .map(|key| (key, table.layout(key).unwrap()))
        .collect();
    let records = RegionRecords {
        region: REGION,
        seed: 0x1122_3344,
        baseline_hash: baseline.content_hash(),
        overlay: Some(Arc::new(overlay)),
        resources,
        resource_schemas,
        transient_resources: vec![Rain::KEY],
        layouts,
    };
    records.check().unwrap();
    records
}

fn header() -> ContainerHeader {
    ContainerHeader {
        format: CONTAINER_FORMAT_VERSION,
        engine: EngineVersion { major: 1, minor: 2, patch: 3 },
        schema_hash: SchemaHash(0x0123_4567_89ab_cdef),
        tick: Tick(77),
        next_net_id: NonZeroU64::new(9).unwrap(),
    }
}

/// The schema table as `lactose-bundle` encodes one: an overlay carrying exactly these
/// schemas, its table cut out after the shared frame and the baseline hash.
fn bundle_table(schemas: &[lactose_bundle::ComponentSchema]) -> Vec<u8> {
    let spawned = NetId::from_bits(1).unwrap();
    let components: Vec<ComponentData> = schemas
        .iter()
        .map(|schema| ComponentData {
            component: schema.component(),
            bytes: vec![0; schema.element_size() as usize],
        })
        .collect();
    let record = Record::Spawn { id: spawned, components };
    let encoded = Overlay::new(0, schemas.iter().cloned(), [record]).unwrap().encode();
    let mut len = 4;
    for schema in schemas {
        len += 8 + 4 + 4 + 4 + 10 * schema.fields().len();
    }
    encoded[16..16 + len].to_vec()
}

/// The section's content, written out from the layout the container's documentation
/// states.
fn expected_content(records: &RegionRecords) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&records.region.0.to_le_bytes());
    out.extend_from_slice(&records.seed.to_le_bytes());
    out.extend_from_slice(&records.baseline_hash.to_le_bytes());
    let overlay = records.overlay.as_ref().unwrap().encode();
    out.push(1);
    out.extend_from_slice(&(overlay.len() as u64).to_le_bytes());
    out.extend_from_slice(&overlay);
    out.extend_from_slice(&bundle_table(&records.resource_schemas));
    for data in &records.resources {
        out.extend_from_slice(&data.component.0.to_le_bytes());
        out.extend_from_slice(&data.bytes);
    }
    out.extend_from_slice(&(records.transient_resources.len() as u32).to_le_bytes());
    for key in &records.transient_resources {
        out.extend_from_slice(&key.0.to_le_bytes());
    }
    out.extend_from_slice(&(records.layouts.len() as u32).to_le_bytes());
    for (key, digest) in &records.layouts {
        out.extend_from_slice(&key.0.to_le_bytes());
        out.extend_from_slice(&digest.0.to_le_bytes());
    }
    out
}

#[test]
fn the_section_and_the_block_are_laid_out_as_stated() {
    let table = table();
    let records = records(&table);
    let section = records.encode_section().unwrap();
    let content = zstd::bulk::decompress(section.bytes(), 1 << 20).unwrap();
    assert_eq!(content, expected_content(&records));
    // One frame, its content size stated, no checksum.
    assert_eq!(
        zstd::zstd_safe::get_frame_content_size(section.bytes()).unwrap(),
        Some(content.len() as u64)
    );
    let descriptor = section.bytes()[4];
    assert_eq!(descriptor & 0b100, 0, "no content checksum");

    let file = Container::assemble(&header(), vec![section.clone()]).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(b"LSAV");
    expected.extend_from_slice(&CONTAINER_FORMAT_VERSION.to_le_bytes());
    expected.extend_from_slice(&0u16.to_le_bytes());
    let block_len = 30 + 65 + 8;
    expected.extend_from_slice(&(block_len as u64).to_le_bytes());
    for part in [1u16, 2, 3] {
        expected.extend_from_slice(&part.to_le_bytes());
    }
    expected.extend_from_slice(&0x0123_4567_89ab_cdef_u64.to_le_bytes());
    expected.extend_from_slice(&77u32.to_le_bytes());
    expected.extend_from_slice(&9u64.to_le_bytes());
    expected.extend_from_slice(&1u32.to_le_bytes());
    expected.extend_from_slice(&REGION.0.to_le_bytes());
    expected.extend_from_slice(&records.seed.to_le_bytes());
    expected.extend_from_slice(&records.baseline_hash.to_le_bytes());
    expected.push(1);
    expected.extend_from_slice(&records.overlay.as_ref().unwrap().digest().to_le_bytes());
    expected.extend_from_slice(&((PREAMBLE_BYTES + block_len) as u64).to_le_bytes());
    expected.extend_from_slice(&(section.bytes().len() as u64).to_le_bytes());
    expected.extend_from_slice(&(content.len() as u64).to_le_bytes());
    expected.extend_from_slice(&xxh64(section.bytes(), 0).to_le_bytes());
    let hash = xxh64(&expected[PREAMBLE_BYTES..], 0);
    expected.extend_from_slice(&hash.to_le_bytes());
    expected.extend_from_slice(section.bytes());
    assert_eq!(file, expected);
}

#[test]
fn the_schema_table_is_the_bundle_encoding() {
    let table = table();
    let records = records(&table);
    let content = zstd::bulk::decompress(records.encode_section().unwrap().bytes(), 1 << 20);
    let content = content.unwrap();
    let overlay_len = records.overlay.as_ref().unwrap().encode().len();
    let at = 8 * 3 + 1 + 8 + overlay_len;
    let expected = bundle_table(&records.resource_schemas);
    assert_eq!(&content[at..at + expected.len()], expected.as_slice());
    // And for every schema of the game's table at once, the check being blind to roles.
    let all: Vec<_> = schemas().schemas().cloned().collect();
    let every = RegionRecords {
        overlay: None,
        resources: all
            .iter()
            .map(|schema| ComponentData {
                component: schema.component(),
                bytes: vec![0; schema.element_size() as usize],
            })
            .collect(),
        resource_schemas: all.clone(),
        transient_resources: Vec::new(),
        layouts: all
            .iter()
            .map(|schema| (schema.component(), schemas().layout(schema.component()).unwrap()))
            .collect(),
        ..records
    };
    every.check().unwrap();
    let content = zstd::bulk::decompress(every.encode_section().unwrap().bytes(), 1 << 20);
    let expected = bundle_table(&all);
    assert!(all.len() > 10 && all.iter().any(|schema| schema.fields().len() > 1));
    assert_eq!(&content.unwrap()[8 * 3 + 1..8 * 3 + 1 + expected.len()], expected.as_slice());
}

/// The golden file: `encode_section` at the pinned level, assembled. zstd is pinned by
/// `Cargo.lock`; a zstd upgrade that changes these bytes changes no content, and the
/// test above still holds, so such an upgrade re-pins this one (README: a save's identity
/// is its content, never the file's bytes).
const GOLDEN: &str = concat!(
    // The preamble: magic, format 1, zero, a block of 103 bytes.
    "4c534156",
    "0100",
    "0000",
    "6700000000000000",
    // The header: engine 1.2.3, schema hash, tick 77, counter 9, one entry.
    "010002000300",
    "efcdab8967452301",
    "4d000000",
    "0900000000000000",
    "01000000",
    // The entry: region 5, seed, baseline hash, an overlay and its digest, offset 119,
    // 180 bytes compressed, 297 decompressed, the section's hash.
    "0500000000000000",
    "4433221100000000",
    "1e4ae5a43cbaada1",
    "01",
    "95aaa7275b18c190",
    "7700000000000000",
    "b400000000000000",
    "2901000000000000",
    "13c283a5d99a6df2",
    // The block's hash.
    "2c3ff36681308ef2",
    // The section: one zstd frame.
    "28b52ffd602900550500a4060500443322111e4ae5a43cbaada1016c0000004c4f564c010000002e56cab3",
    "f382b2fe040000000203074d00008095aaa7275b18c190d16d465f758a0a32afd58182e37d7a5f080809",
    "224213b3a1896277037cc96c20349a2217f8063b7ab0cc619e0f0a5ff9b74020911b002343f854843f35",
    "c730e69a52004c80af8c03c78206f10b1a5f22a20a0b27b99c21209c54247c26003b6039e61888b012e4",
    "25f03c99c734c462c900b0",
);

#[test]
fn golden_container_bytes() {
    let table = table();
    let records = records(&table);
    let mut container = Container::new(header());
    container.insert(records).unwrap();
    let file = container.encode().unwrap();
    let hex: String = file.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, GOLDEN);
    assert_eq!(Container::decode(&file).unwrap(), container);
}
