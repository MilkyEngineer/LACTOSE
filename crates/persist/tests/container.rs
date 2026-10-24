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

use common::*;
use lactose_core::{AllocTag, ErrorKind, TaggedAlloc, xxh64};
use lactose_ecs::RegionId;
use lactose_persist::{
    CONTAINER_FORMAT_VERSION, Container, ContainerHeader, MAX_CONTAINER_BYTES, MAX_SECTION_BYTES,
    PREAMBLE_BYTES, RegionIndexEntry,
};

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
        content.extend_from_slice(&[0; 16]);
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
    by_claim("the layout count", &|claim| {
        let mut content = Vec::new();
        header(&mut content);
        content.extend_from_slice(&0u32.to_le_bytes());
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
    content.extend_from_slice(&[0; 12]);
    let (entry, section) = section_of(&content);
    let records = Container::decode_section(&entry, &section).unwrap();
    assert_eq!(records.region, RegionId(5));
    assert!(records.overlay.is_none() && records.resources.is_empty());

    let mut trailing = content.clone();
    trailing.push(0);
    let mut presence = content.clone();
    presence[24] = 2;
    let mut layouts = content.clone();
    layouts.truncate(content.len() - 4);
    layouts.extend_from_slice(&2u32.to_le_bytes());
    for key in [7u64, 7] {
        layouts.extend_from_slice(&key.to_le_bytes());
        layouts.extend_from_slice(&1u64.to_le_bytes());
    }
    let mut transient = content.clone();
    transient.truncate(content.len() - 8);
    transient.extend_from_slice(&2u32.to_le_bytes());
    transient.extend_from_slice(&9u64.to_le_bytes());
    transient.extend_from_slice(&8u64.to_le_bytes());
    transient.extend_from_slice(&0u32.to_le_bytes());
    for (what, bad) in [
        ("a byte after the content", trailing),
        ("a presence byte of two", presence),
        ("repeated layout digests", layouts),
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
