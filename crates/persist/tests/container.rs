//! The container's caps and the prefix read (README, "The container"): every refusal of
//! untrusted bytes as stated, with what a read allocates counted by the tagged global
//! allocator this test binary installs; the index read from exactly its prefix; and a
//! damaged section failing alone.
//!
//! The allocator counts allocations made through Rust's allocator on this thread; zstd's
//! own working memory is allocated by its C library and is not counted. A claimed count
//! the reader trusted would show as allocations that grow with it, or as an abort: each
//! lying count below is `u32::MAX` or close to it.

#![forbid(unsafe_code)]

mod common;

use std::num::NonZeroU64;
use std::sync::Arc;

use common::*;
use lactose_bundle::{Baseline, ComponentSchema};
use lactose_core::{AllocTag, ComponentKey, ErrorKind, NetId, TaggedAlloc, Tick, xxh64};
use lactose_ecs::{RegionConfig, RegionId, RegionMode, World, WorldBuilder, WorldKind};
use lactose_persist::{
    CONTAINER_FORMAT_VERSION, Container, ContainerHeader, Loader, MAX_CONTAINER_BYTES,
    MAX_SECTION_BYTES, Migrations, PREAMBLE_BYTES, RegionIndexEntry, RegionRecords, SchemaTable,
};
use lactose_sim::RegionExport;

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

/// What `f` returns and how many allocations it made on this thread.
fn counted<R>(f: impl FnOnce() -> R) -> (R, u64) {
    let _scope = AllocTag::Persist.scope();
    let before = ALLOCATOR.stats(AllocTag::Persist).total_allocations;
    let result = f();
    (result, ALLOCATOR.stats(AllocTag::Persist).total_allocations - before)
}

/// The block's layout, as `src/container.rs` states it.
const HEADER_BYTES: usize = 30;
const ENTRY_BYTES: usize = 65;
const COUNT_AT: usize = PREAMBLE_BYTES + 26;

/// Where entry `n`'s field at `field` lies in the file.
fn entry_at(n: usize, field: usize) -> usize {
    PREAMBLE_BYTES + HEADER_BYTES + n * ENTRY_BYTES + field
}

const REGION: usize = 0;
const SEED: usize = 8;
const PRESENT: usize = 24;
const DIGEST: usize = 25;
const OFFSET: usize = 33;
const COMPRESSED: usize = 41;
const BYTES: usize = 49;

fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

/// Seals the block again after an edit, so the edit reaches the check under test rather
/// than the block hash.
fn reseal(bytes: &mut [u8]) {
    let len = Container::index_len(bytes).unwrap();
    let hash = xxh64(&bytes[PREAMBLE_BYTES..len - 8], 0);
    put_u64(bytes, len - 8, hash);
}

/// A saved program's container, with at least three regions.
fn file() -> Vec<u8> {
    let save = saved_at(40);
    let bytes = save_bytes(&save);
    assert!(Container::index(&bytes).unwrap().regions.len() >= 3);
    bytes
}

fn refused(result: lactose_core::Result<impl core::fmt::Debug>, kind: ErrorKind, what: &str) {
    match result {
        Ok(value) => panic!("{what}: accepted as {value:?}"),
        Err(error) => assert_eq!(error.kind(), kind, "{what}: {error}"),
    }
}

#[test]
fn the_preamble_is_checked_first() {
    let bytes = file();
    let len = Container::index_len(&bytes[..PREAMBLE_BYTES]).unwrap();
    for short in [0, 1, PREAMBLE_BYTES - 1] {
        refused(Container::index_len(&bytes[..short]), ErrorKind::InvalidData, "a short preamble");
        refused(Container::index(&bytes[..short]), ErrorKind::InvalidData, "a short preamble");
    }
    refused(Container::index(&bytes[..len - 1]), ErrorKind::InvalidData, "a short prefix");

    let mut magic = bytes.clone();
    magic[0] ^= 1;
    refused(Container::index_len(&magic), ErrorKind::InvalidData, "a wrong magic");
    refused(Container::decode(&magic), ErrorKind::InvalidData, "a wrong magic");

    let mut version = bytes.clone();
    version[4..6].copy_from_slice(&(CONTAINER_FORMAT_VERSION + 1).to_le_bytes());
    refused(Container::index_len(&version), ErrorKind::VersionMismatch, "another version");
    refused(Container::index(&version), ErrorKind::VersionMismatch, "another version");
    refused(Container::decode(&version), ErrorKind::VersionMismatch, "another version");

    let mut reserved = bytes.clone();
    reserved[6] = 1;
    refused(Container::index_len(&reserved), ErrorKind::InvalidData, "a reserved word");

    let mut over = bytes[..PREAMBLE_BYTES].to_vec();
    let cap = (MAX_CONTAINER_BYTES - PREAMBLE_BYTES) as u64;
    put_u64(&mut over, 8, cap);
    assert_eq!(Container::index_len(&over).unwrap(), MAX_CONTAINER_BYTES, "the cap itself");
    put_u64(&mut over, 8, cap + 1);
    refused(Container::index_len(&over), ErrorKind::InvalidData, "a block over the cap");
    put_u64(&mut over, 8, u64::MAX);
    refused(Container::index_len(&over), ErrorKind::InvalidData, "a block of u64::MAX");
}

#[test]
fn the_index_is_read_from_exactly_its_prefix() {
    let bytes = file();
    let len = Container::index_len(&bytes[..PREAMBLE_BYTES]).unwrap();
    let (index, allocations) = counted(|| Container::index(&bytes[..len]).unwrap());
    assert_eq!(index, Container::index(&bytes).unwrap());
    assert!(allocations <= 2, "the index is one vector: {allocations} allocations");
    let first = index.regions.first().unwrap();
    assert_eq!(first.offset as usize, len, "the first section follows the block");
}

#[test]
fn a_damaged_block_is_refused_by_its_hash() {
    let bytes = file();
    let len = Container::index_len(&bytes).unwrap();
    for at in [PREAMBLE_BYTES, COUNT_AT, entry_at(0, SEED), entry_at(1, BYTES), len - 1] {
        let mut damaged = bytes.clone();
        damaged[at] ^= 0x10;
        refused(Container::index(&damaged), ErrorKind::InvalidData, "a damaged block");
        refused(Container::decode(&damaged), ErrorKind::InvalidData, "a damaged block");
    }
}

#[test]
fn the_index_entries_are_checked() {
    let bytes = file();
    let len = Container::index_len(&bytes).unwrap() as u64;
    let edit = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut edited = bytes.clone();
        f(&mut edited);
        reseal(&mut edited);
        Container::index(&edited)
    };
    let region_1 = get_u64(&bytes, entry_at(1, REGION));
    let region_0 = get_u64(&bytes, entry_at(0, REGION));
    type Damage = Box<dyn Fn(&mut Vec<u8>)>;
    let cases: Vec<(&str, Damage)> = vec![
        ("a duplicate region", Box::new(move |b| put_u64(b, entry_at(0, REGION), region_1))),
        (
            "a descending index",
            Box::new(move |b| {
                put_u64(b, entry_at(0, REGION), region_1);
                put_u64(b, entry_at(1, REGION), region_0);
            }),
        ),
        ("a section inside the block", Box::new(move |b| put_u64(b, entry_at(0, OFFSET), len - 1))),
        ("a section at the preamble", Box::new(|b| put_u64(b, entry_at(1, OFFSET), 0))),
        (
            "a section past the cap",
            Box::new(|b| {
                let size = get_u64(b, entry_at(0, COMPRESSED));
                put_u64(b, entry_at(0, OFFSET), MAX_CONTAINER_BYTES as u64 - size + 1);
            }),
        ),
        (
            "a section whose end overflows",
            Box::new(|b| put_u64(b, entry_at(0, COMPRESSED), u64::MAX)),
        ),
        (
            "overlapping sections",
            Box::new(|b| {
                let first = get_u64(b, entry_at(0, OFFSET));
                put_u64(b, entry_at(1, OFFSET), first + 1);
            }),
        ),
        (
            "a section over the section cap",
            Box::new(|b| {
                put_u64(b, entry_at(0, BYTES), MAX_SECTION_BYTES as u64 + 1);
            }),
        ),
        (
            "an absent overlay with a digest",
            Box::new(|b| {
                b[entry_at(0, PRESENT)] = 0;
                put_u64(b, entry_at(0, DIGEST), 1);
            }),
        ),
        ("a presence byte of two", Box::new(|b| b[entry_at(0, PRESENT)] = 2)),
        ("a zero NetId counter", Box::new(|b| put_u64(b, PREAMBLE_BYTES + 18, 0))),
        (
            "an entry count too small",
            Box::new(|b| {
                let count = u32::from_le_bytes(b[COUNT_AT..COUNT_AT + 4].try_into().unwrap());
                b[COUNT_AT..COUNT_AT + 4].copy_from_slice(&(count - 1).to_le_bytes());
            }),
        ),
    ];
    for (what, case) in cases {
        refused(edit(&*case), ErrorKind::InvalidData, what);
    }
    // At the cap itself, a section is placed.
    let edited = edit(&|b: &mut Vec<u8>| {
        let count = u32::from_le_bytes(b[COUNT_AT..COUNT_AT + 4].try_into().unwrap()) as usize;
        let last = count - 1;
        let size = get_u64(b, entry_at(last, COMPRESSED));
        put_u64(b, entry_at(last, OFFSET), MAX_CONTAINER_BYTES as u64 - size);
        put_u64(b, entry_at(last, BYTES), MAX_SECTION_BYTES as u64);
    });
    edited.unwrap();
}

#[test]
fn a_lying_entry_count_allocates_nothing_for_it() {
    let bytes = file();
    let len = Container::index_len(&bytes).unwrap();
    let mut counts = Vec::new();
    for claim in [1_000, u32::MAX] {
        let mut lying = bytes[..len].to_vec();
        lying[COUNT_AT..COUNT_AT + 4].copy_from_slice(&claim.to_le_bytes());
        reseal(&mut lying);
        let (result, allocations) = counted(|| Container::index(&lying));
        refused(result, ErrorKind::InvalidData, "a lying entry count");
        counts.push(allocations);
    }
    assert_eq!(counts[0], counts[1], "nothing grows with the claim");
    assert!(counts[1] <= 4, "{} allocations", counts[1]);
}

/// The first entry of `bytes` with an overlay, and its section.
fn overlay_section(bytes: &[u8]) -> (RegionIndexEntry, Vec<u8>) {
    let index = Container::index(bytes).unwrap();
    let entry = *index.regions.iter().find(|entry| entry.overlay_digest.is_some()).unwrap();
    let start = entry.offset as usize;
    (entry, bytes[start..start + entry.compressed_bytes as usize].to_vec())
}

#[test]
fn a_section_is_checked_against_its_entry() {
    let bytes = file();
    let (entry, section) = overlay_section(&bytes);
    Container::decode_section(&entry, &section).unwrap();

    let mut longer = section.clone();
    longer.push(0);
    refused(Container::decode_section(&entry, &longer), ErrorKind::InvalidData, "a longer slice");
    let shorter = &section[..section.len() - 1];
    refused(Container::decode_section(&entry, shorter), ErrorKind::InvalidData, "a shorter slice");
    let mut damaged = section.clone();
    let middle = damaged.len() / 2;
    damaged[middle] ^= 1;
    refused(Container::decode_section(&entry, &damaged), ErrorKind::InvalidData, "another hash");

    // Entries that agree with the bytes' length and hash and state something else.
    let cases: Vec<(&str, RegionIndexEntry)> = vec![
        ("another decompressed size", RegionIndexEntry { bytes: entry.bytes + 1, ..entry }),
        ("a smaller decompressed size", RegionIndexEntry { bytes: entry.bytes - 1, ..entry }),
        ("the section cap", RegionIndexEntry { bytes: MAX_SECTION_BYTES as u64, ..entry }),
        ("over the section cap", RegionIndexEntry { bytes: MAX_SECTION_BYTES as u64 + 1, ..entry }),
        ("another region", RegionIndexEntry { region: RegionId(entry.region.0 + 1), ..entry }),
        ("another seed", RegionIndexEntry { seed: entry.seed ^ 1, ..entry }),
        ("another baseline", RegionIndexEntry { baseline_hash: entry.baseline_hash ^ 1, ..entry }),
        (
            "another digest",
            RegionIndexEntry {
                overlay_digest: Some(entry.overlay_digest.map_or(1, |digest| digest ^ 1)),
                ..entry
            },
        ),
        ("no overlay", RegionIndexEntry { overlay_digest: None, ..entry }),
    ];

    for (what, lying) in cases {
        let (result, allocations) = counted(|| Container::decode_section(&lying, &section));
        refused(result, ErrorKind::InvalidData, what);
        if what.contains("cap") {
            assert!(allocations <= 2, "{what}: refused before decompressing: {allocations}");
        }
    }
}

/// One zstd frame of `content`, as a section is compressed, with its entry.
fn section_of(content: &[u8]) -> (RegionIndexEntry, Vec<u8>) {
    let mut compressor = zstd::bulk::Compressor::new(3).unwrap();
    compressor.set_parameter(zstd::zstd_safe::CParameter::ContentSizeFlag(true)).unwrap();
    let section = compressor.compress(content).unwrap();
    let entry = RegionIndexEntry {
        region: RegionId(u64::from_le_bytes(content[..8].try_into().unwrap())),
        seed: 0,
        baseline_hash: 0,
        overlay_digest: None,
        offset: 0,
        compressed_bytes: section.len() as u64,
        bytes: content.len() as u64,
        hash: xxh64(&section, 0),
    };
    (entry, section)
}

#[test]
fn lying_counts_in_a_section_allocate_nothing_for_them() {
    let header = |content: &mut Vec<u8>| {
        content.extend_from_slice(&5u64.to_le_bytes());
        content.extend_from_slice(&0u64.to_le_bytes());
        content.extend_from_slice(&0u64.to_le_bytes());
        content.push(0);
    };
    let mut cases: Vec<(&str, Vec<Vec<u8>>)> = Vec::new();
    let mut by_claim = |what: &'static str, build: &dyn Fn(u32) -> Vec<u8>| {
        cases.push((what, vec![build(1_000), build(u32::MAX - 1), build(u32::MAX)]));
    };
    by_claim("the schema count", &|claim| {
        let mut content = Vec::new();
        header(&mut content);
        content.extend_from_slice(&claim.to_le_bytes());
        content
    });
    by_claim("a schema's field count", &|claim| {
        let mut content = Vec::new();
        header(&mut content);
        content.extend_from_slice(&1u32.to_le_bytes());
        // The key, version, size and layout digest.
        content.extend_from_slice(&[0; 24]);
        content.extend_from_slice(&claim.to_le_bytes());
        content
    });
    by_claim("the transient count", &|claim| {
        let mut content = Vec::new();
        header(&mut content);
        content.extend_from_slice(&0u32.to_le_bytes());
        content.extend_from_slice(&claim.to_le_bytes());
        content
    });
    by_claim("the overlay's length", &|claim| {
        let mut content = Vec::new();
        header(&mut content);
        *content.last_mut().unwrap() = 1;
        content.extend_from_slice(&u64::from(claim).to_le_bytes());
        content
    });
    for (what, contents) in cases {
        let mut counts = Vec::new();
        for content in contents {
            let (entry, section) = section_of(&content);
            let (result, allocations) = counted(|| Container::decode_section(&entry, &section));
            refused(result, ErrorKind::InvalidData, what);
            counts.push(allocations);
        }
        assert!(counts.windows(2).all(|pair| pair[0] == pair[1]), "{what}: {counts:?}");
        assert!(counts[0] <= 4, "{what}: {counts:?}");
    }
}

#[test]
fn content_that_is_not_canonical_is_refused() {
    let mut content = Vec::new();
    content.extend_from_slice(&5u64.to_le_bytes());
    content.extend_from_slice(&[0; 16]);
    content.push(0);
    content.extend_from_slice(&[0; 8]);
    let (entry, section) = section_of(&content);
    let records = Container::decode_section(&entry, &section).unwrap();
    assert_eq!(records.region, RegionId(5));
    assert!(records.overlay.is_none() && records.resources.is_empty());

    let mut trailing = content.clone();
    trailing.push(0);
    let mut presence = content.clone();
    presence[24] = 2;
    let mut schemas = content.clone();
    schemas.truncate(content.len() - 8);
    let schema = ComponentSchema::new(ComponentKey(7), 1, 0, 1, []).unwrap();
    lactose_bundle::write_table(&[schema.clone(), schema], &mut schemas);
    for key in [7u64, 7] {
        schemas.extend_from_slice(&key.to_le_bytes());
    }
    schemas.extend_from_slice(&0u32.to_le_bytes());
    let mut transient = content.clone();
    transient.truncate(content.len() - 4);
    transient.extend_from_slice(&2u32.to_le_bytes());
    transient.extend_from_slice(&9u64.to_le_bytes());
    transient.extend_from_slice(&8u64.to_le_bytes());
    for (what, bad) in [
        ("a byte after the content", trailing),
        ("a presence byte of two", presence),
        ("repeated resource schemas", schemas),
        ("descending transient keys", transient),
    ] {
        let (entry, section) = section_of(&bad);
        refused(Container::decode_section(&entry, &section), ErrorKind::InvalidData, what);
    }
    // Two frames back to back are not one section.
    let (entry, mut section) = section_of(&content);
    let twice = section.clone();
    section.extend_from_slice(&twice);
    let entry = RegionIndexEntry {
        compressed_bytes: section.len() as u64,
        hash: xxh64(&section, 0),
        ..entry
    };
    refused(Container::decode_section(&entry, &section), ErrorKind::InvalidData, "two frames");
}

#[test]
fn a_damaged_section_fails_alone() {
    let bytes = file();
    let index = Container::index(&bytes).unwrap();
    let whole = Container::decode(&bytes).unwrap();
    for damaged_entry in &index.regions {
        let mut damaged = bytes.clone();
        let at = damaged_entry.offset as usize + damaged_entry.compressed_bytes as usize / 2;
        damaged[at] ^= 0x40;
        assert_eq!(Container::index(&damaged).unwrap(), index, "the block is untouched");
        refused(Container::decode(&damaged), ErrorKind::InvalidData, "the whole file");
        for entry in &index.regions {
            let start = entry.offset as usize;
            let section = &damaged[start..start + entry.compressed_bytes as usize];
            let result = Container::decode_section(entry, section);
            if entry.region == damaged_entry.region {
                refused(result, ErrorKind::InvalidData, "the damaged section");
                let region = Container::decode_region(&damaged, entry.region);
                refused(region, ErrorKind::InvalidData, "the damaged region");
            } else {
                assert_eq!(Some(&result.unwrap()), whole.region(entry.region));
                let region = Container::decode_region(&damaged, entry.region).unwrap();
                assert_eq!(Some(&region), whole.region(entry.region));
            }
        }
    }
    let missing = RegionId(1);
    refused(Container::decode_region(&bytes, missing), ErrorKind::NotFound, "a missing region");
    // A file cut short inside its last section.
    let cut = &bytes[..bytes.len() - 1];
    Container::index(cut).unwrap();
    refused(Container::decode(cut), ErrorKind::InvalidData, "a file cut short");
}

#[test]
fn assemble_refuses_what_no_reader_would_accept() {
    let save = saved_at(40);
    let header = ContainerHeader::of(&save, schemas());
    let sections: Vec<_> =
        save.regions.iter().map(|export| records_of(export).encode_section().unwrap()).collect();
    let mut twice = sections.clone();
    twice.push(sections[0].clone());
    refused(
        Container::assemble(&header, twice),
        ErrorKind::InvalidData,
        "two sections of one region",
    );
    let other = ContainerHeader { format: CONTAINER_FORMAT_VERSION + 1, ..header };
    refused(
        Container::assemble(&other, sections.clone()),
        ErrorKind::VersionMismatch,
        "another format",
    );
    // The header states the save point.
    let index = Container::index(&Container::assemble(&header, sections).unwrap()).unwrap();
    assert_eq!(index.header, header);
    assert_eq!(index.header.tick, save.tick);
    assert_eq!(index.header.next_net_id, save.next_net_id);
    assert_eq!(index.header.schema_hash, schemas().hash());
}

// ---- The sections against each other and against the header (README, "The container") ----

/// A world of [`registry`]'s types with each region loaded from its baseline.
fn loaded_world(regions: &[(RegionId, &Baseline)]) -> World {
    let mut world = bare_world();
    for &(region, baseline) in regions {
        let config =
            RegionConfig { id: region, seed: seed_of(region), mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
        world.load_region(region, baseline, None).unwrap();
    }
    world
}

/// `region`'s records in `world`, against `baseline`, under the table of [`registry`].
fn records_in_world(world: &World, region: RegionId, baseline: &Baseline) -> RegionRecords {
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let export = RegionExport { region, rows: world.export_region(region).unwrap() };
    RegionRecords::from_export(&export, baseline, &table).unwrap()
}

/// A world that played `region` alone: its first runtime entity spawned there.
fn played(region: RegionId, baseline: &Baseline) -> (World, NetId) {
    let mut world = loaded_world(&[(region, baseline)]);
    let id = world.allocate_net_id().unwrap();
    world.spawn(net(id), region, (pos(9, 9),)).unwrap();
    (world, id)
}

/// The bytes of a container of `sections` under a header whose counter is `counter`,
/// assembled as a server writes one: `assemble` never reads the records, so this is also
/// what splicing sections from two saves gives.
fn spliced(counter: u64, sections: &[&RegionRecords]) -> Vec<u8> {
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let header = ContainerHeader::new(Tick(3), NonZeroU64::new(counter).unwrap(), table.hash());
    let encoded = sections.iter().map(|records| records.encode_section().unwrap()).collect();
    Container::assemble(&header, encoded).unwrap()
}

/// `result` is refused with `InvalidData` and a message holding every one of `named`.
fn refused_naming(result: lactose_core::Result<impl core::fmt::Debug>, named: &[&str]) {
    match result {
        Ok(value) => panic!("accepted as {value:?}"),
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
            for name in named {
                assert!(error.message().contains(name), "`{error}` names `{name}`");
            }
        }
    }
}

#[test]
fn two_sections_that_spawn_one_runtime_id_are_refused_at_decode() {
    // Two worlds, each spawning its first runtime entity: the same `NetId` in A and in B.
    let (a, b) = (baseline_a(), baseline_b());
    let (one, id_a) = played(A, &a);
    let (two, id_b) = played(B, &b);
    assert_eq!(id_a, id_b);
    let (in_a, in_b) = (records_in_world(&one, A, &a), records_in_world(&two, B, &b));
    let id = id_a.to_bits();
    let (named_a, named_b) = (format!("regions {} and {}", A.0, B.0), format!("{id:#x}"));
    let named = [named_a.as_str(), named_b.as_str(), "both spawn NetId"];

    // The counter is above the id, so the duplicate alone is refused.
    let bytes = spliced(id + 1, &[&in_a, &in_b]);
    refused_naming(Container::decode(&bytes), &named);
    // Each section is canonical on its own and decodes alone: only the whole shows it.
    let index = Container::index(&bytes).unwrap();
    for entry in &index.regions {
        let at = entry.offset as usize..(entry.offset + entry.compressed_bytes) as usize;
        Container::decode_section(entry, &bytes[at]).unwrap();
    }
    // The in-memory writer refuses what the reader refuses.
    let mut container = Container::new(index.header);
    container.insert(in_a.clone()).unwrap();
    container.insert(in_b.clone()).unwrap();
    refused_naming(container.encode(), &named);
    // Either section alone, under that counter, is a save.
    for records in [&in_a, &in_b] {
        let alone = Container::decode(&spliced(id + 1, &[records])).unwrap();
        assert_eq!(alone.region(records.region), Some(records));
    }

    // What the refusal prevents: both sections bundle, and the second region's load in
    // one world finds the entity live.
    let migrations = Migrations::new();
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let loader = Loader::new(&table, &migrations);
    let mut world = bare_world();
    for (records, baseline) in [(&in_a, &a), (&in_b, &b)] {
        let bundle =
            loader.bundle(records, Arc::new(baseline.clone()), RegionMode::Frozen, None).unwrap();
        let config =
            RegionConfig { id: records.region, seed: bundle.seed, mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
        let loaded = world.load_region(records.region, &bundle.baseline, bundle.overlay.as_deref());
        if records.region == B {
            assert!(loaded.unwrap_err().message().contains("already live"));
        } else {
            loaded.unwrap();
        }
    }
}

#[test]
fn a_cooked_entity_spawned_from_the_baseline_by_two_sections_is_refused_at_decode() {
    // A's sixth cooked entity moved into B in one play and into D in another: two
    // sections spawn it from the baseline.
    let d = RegionId(77);
    let (a, b) = (baseline_a(), baseline_b());
    let moved = |to: RegionId| {
        let mut world = loaded_world(&[(A, &a), (to, &b)]);
        world.move_to_region(net(c(A, 6)), to).unwrap();
        records_in_world(&world, to, &b)
    };
    let (in_b, in_d) = (moved(B), moved(d));
    let named = format!("regions {} and {}", d.0, B.0);
    refused_naming(
        Container::decode(&spliced(1, &[&in_b, &in_d])),
        &[named.as_str(), "both spawn NetId"],
    );
    // Either alone is a save: a cooked ID is not the counter's.
    Container::decode(&spliced(1, &[&in_b])).unwrap();
    Container::decode(&spliced(1, &[&in_d])).unwrap();
}

#[test]
fn a_counter_not_above_a_spawned_runtime_id_is_refused_at_decode() {
    let a = baseline_a();
    let (world, id) = played(A, &a);
    let records = records_in_world(&world, A, &a);
    let id = id.to_bits();
    assert_eq!(world.next_net_id().get(), id + 1);
    let region = format!("region {}", A.0);
    for counter in [1, id] {
        let named =
            [region.as_str(), &format!("runtime NetId {id}"), &format!("NetId counter {counter}")];
        refused_naming(Container::decode(&spliced(counter, &[&records])), &named);
        let header = Container::index(&spliced(counter, &[&records])).unwrap().header;
        let mut container = Container::new(header);
        container.insert(records.clone()).unwrap();
        refused_naming(container.encode(), &named);
    }
    // The world's own counter, one above the id, is a save; so is any higher one.
    for counter in [id + 1, id + 100] {
        let decoded = Container::decode(&spliced(counter, &[&records])).unwrap();
        assert_eq!(decoded.header().next_net_id.get(), counter);
    }

    // What the refusal prevents: a server started at the low counter hands the id out
    // again, and the kept region that holds it is refused when it comes back.
    let mut server = WorldBuilder::new(WorldKind::Server, Arc::new(registry()))
        .next_net_id(NonZeroU64::MIN)
        .build()
        .unwrap();
    let again = server.allocate_net_id().unwrap();
    assert_eq!(again.to_bits(), id);
    let b = baseline_b();
    let config = RegionConfig { id: B, seed: seed_of(B), mode: RegionMode::Simulated };
    server.create_region(config).unwrap();
    server.load_region(B, &b, None).unwrap();
    server.spawn(net(again), B, (pos(1, 1),)).unwrap();
    let config = RegionConfig { id: A, seed: seed_of(A), mode: RegionMode::Simulated };
    server.create_region(config).unwrap();
    let error = server.load_region(A, &a, records.overlay.as_deref()).unwrap_err();
    assert!(error.message().contains("already live"), "{error}");
}

/// A cooked entity moved out of its home region in one play (the home's section destroys
/// it, the other section spawns it from the baseline), and the home section of another
/// play, which changes the row or not: the sections one splice takes. Returns the other
/// play's home, then the moving play's home and destination.
fn cooked_moved_and_home(
    to: RegionId,
    home_changes: bool,
) -> (RegionRecords, RegionRecords, RegionRecords) {
    let (a, b) = (baseline_a(), baseline_b());
    let mut stay = loaded_world(&[(A, &a), (to, &b)]);
    if home_changes {
        stay.insert(net(c(A, 6)), pos(60, 60)).unwrap();
    }
    let home = records_in_world(&stay, A, &a);
    let mut moved = loaded_world(&[(A, &a), (to, &b)]);
    moved.move_to_region(net(c(A, 6)), to).unwrap();
    (home, records_in_world(&moved, A, &a), records_in_world(&moved, to, &b))
}

#[test]
fn a_cooked_entity_spawned_elsewhere_while_its_home_section_changes_it_is_refused_at_decode() {
    // B above A and D below it, so the section that spawns is given second and first.
    let d = RegionId(77);
    let id = format!("NetId {:#x}", c(A, 6).to_bits());
    for to in [B, d] {
        let (home, destroyed, spawned) = cooked_moved_and_home(to, true);
        let spawns = format!("the section of region {} spawns {id}", to.0);
        let holds = format!("which the section of region {} holds as a row of its baseline", A.0);
        let named = [spawns.as_str(), holds.as_str()];
        let mut sections = [&home, &spawned];
        sections.sort_by_key(|records| records.region);
        refused_naming(Container::decode(&spliced(1, &sections)), &named);
        let header = Container::index(&spliced(1, &[&home])).unwrap().header;
        refused_naming(Container::check_sections(&header, sections), &named);
        // The play the move happened in destroys it at home: a save.
        let mut sections = [&destroyed, &spawned];
        sections.sort_by_key(|records| records.region);
        Container::decode(&spliced(1, &sections)).unwrap();
    }

    // A home section that leaves the row unchanged has no record of it: the sections
    // alone cannot show it, and the second region's load finds the entity live, which
    // `lactose_app::load` refuses from the baselines.
    let (a, b) = (baseline_a(), baseline_b());
    let (home, _, spawned) = cooked_moved_and_home(B, false);
    assert!(home.overlay.is_none());
    let decoded = Container::decode(&spliced(1, &[&home, &spawned])).unwrap();
    let migrations = Migrations::new();
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let loader = Loader::new(&table, &migrations);
    let mut world = bare_world();
    for (region, baseline) in [(A, &a), (B, &b)] {
        let records = decoded.region(region).unwrap();
        let bundle =
            loader.bundle(records, Arc::new(baseline.clone()), RegionMode::Frozen, None).unwrap();
        let config = RegionConfig { id: region, seed: bundle.seed, mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
        let loaded = world.load_region(region, &bundle.baseline, bundle.overlay.as_deref());
        if region == B {
            assert!(loaded.unwrap_err().message().contains("already live"));
        } else {
            loaded.unwrap();
        }
    }
}

#[test]
fn two_sections_that_change_one_cooked_row_are_refused_at_decode() {
    // Two regions cooked over one baseline, each changing the same row in its own play.
    let (a, d) = (baseline_a(), RegionId(77));
    let changed = |region: RegionId| {
        let mut world = loaded_world(&[(region, &a)]);
        world.insert(net(c(A, 6)), pos(60, 60)).unwrap();
        records_in_world(&world, region, &a)
    };
    let (in_d, in_a) = (changed(d), changed(A));
    let named = format!(
        "the sections of regions {} and {} both hold NetId {:#x} as a row of their baselines",
        d.0,
        A.0,
        c(A, 6).to_bits()
    );
    refused_naming(Container::decode(&spliced(1, &[&in_d, &in_a])), &[named.as_str()]);
}

/// D142: a server world's direct spawn under a runtime `NetId` at or above its counter
/// moves the counter past it, so its own export passes the check under the world's counter.
/// `assemble` reads no record and writes a stale counter as given, which the check and the
/// decode refuse: a writer runs `check_sections` first (`app`'s `Server::encode_save` does).
#[test]
fn a_direct_runtime_spawn_above_the_counter_moves_it_and_its_export_passes_check_sections() {
    let a = baseline_a();
    let mut world = loaded_world(&[(A, &a)]);
    let direct = NetId::runtime(NonZeroU64::new(1000).unwrap()).unwrap();
    world.spawn(net(direct), A, (pos(1, 1),)).unwrap();
    assert_eq!(world.next_net_id().get(), 1001);
    let records = records_in_world(&world, A, &a);
    let bytes = spliced(world.next_net_id().get(), &[&records]);
    let header = Container::index(&bytes).unwrap().header;
    Container::check_sections(&header, [&records]).unwrap();
    assert_eq!(Container::decode(&bytes).unwrap().header(), &header);

    let stale = spliced(1000, &[&records]);
    let header = Container::index(&stale).unwrap().header;
    let region = format!("region {}", A.0);
    let named = [region.as_str(), "runtime NetId 1000", "NetId counter 1000"];
    refused_naming(Container::check_sections(&header, [&records]), &named);
    refused_naming(Container::decode(&stale), &named);
}

/// The two orders the check rests on and does not enforce itself: an overlay's records
/// ascend by `NetId` (`lactose-bundle`'s canonical form), and a runtime `NetId` orders
/// below every cooked one (`lactose-core`'s cooked bit). A change to either fails here.
#[test]
fn the_check_across_sections_rests_on_the_overlay_order_with_runtime_ids_first() {
    let largest_runtime = NetId::runtime(NonZeroU64::new((1 << 63) - 1).unwrap()).unwrap();
    assert!(largest_runtime < NetId::cooked(0));
    assert!(NetId::runtime(NonZeroU64::MIN).unwrap() < largest_runtime);

    // A section with every kind: runtime spawns, a cooked row changed, one destroyed by a
    // move out, one spawned from the baseline by a move in.
    let (a, b) = (baseline_a(), baseline_b());
    let mut world = loaded_world(&[(A, &a), (B, &b)]);
    let mut runtime = Vec::new();
    for n in 0..3 {
        let id = world.allocate_net_id().unwrap();
        world.spawn(net(id), A, (pos(n, n),)).unwrap();
        runtime.push(id);
    }
    world.insert(net(c(A, 6)), pos(60, 60)).unwrap();
    world.move_to_region(net(c(A, 7)), B).unwrap();
    world.move_to_region(net(c(B, 1)), A).unwrap();
    let records = records_in_world(&world, A, &a);
    let list = records.overlay.as_ref().unwrap().records();
    assert!(list.windows(2).all(|pair| pair[0].id() <= pair[1].id()));
    let first_cooked = list.iter().position(|record| record.id().is_cooked()).unwrap();
    assert!(list[first_cooked..].iter().all(|record| record.id().is_cooked()));
    assert_eq!(list[first_cooked - 1].id(), runtime[2]);

    // The highest runtime spawn sits just below the cooked records: a counter equal to it
    // is refused, one above it is a save.
    let top = runtime[2].to_bits();
    let named = [format!("runtime NetId {top}"), format!("NetId counter {top}")];
    refused_naming(Container::decode(&spliced(top, &[&records])), &[&named[0], &named[1]]);
    Container::decode(&spliced(top + 1, &[&records])).unwrap();
}
