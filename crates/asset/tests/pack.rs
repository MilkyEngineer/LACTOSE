//! The pack format (`README.md`, "The pack format"): what `PackBuilder` writes, byte for
//! byte, and what a mount refuses, each rule broken in turn.

#![forbid(unsafe_code)]

use crate::common;

use std::io::{Seek, SeekFrom, Write};

use common::capture::capture;
use common::format::{RawEntry, TRAILER_LEN, index, index_range, seal, trailer};
use common::*;
use lactose_asset::{PACK_FORMAT_VERSION, PackBuilder, Storage, Store};
use lactose_asset_api::{
    Asset, AssetCatalog, AssetFailure, AssetId, CookedAsset, CookedHeader, LoadPriority, Readiness,
};
use lactose_core::{ErrorKind, xxh64};
use lactose_io::MAX_READ_BYTES;

fn entry_of(id: AssetId, cooked: &CookedAsset, offset: u64) -> RawEntry {
    let len = cooked.bytes.len() as u64;
    RawEntry {
        id: id.to_bits(),
        version: cooked.version().to_bits(),
        kind: cooked.header.kind().0,
        format: cooked.header.format_version(),
        dependencies: cooked.header.dependencies().iter().map(|id| id.to_bits()).collect(),
        storage: 0,
        offset,
        stored: len,
        decoded: len,
    }
}

#[test]
fn an_empty_pack_is_its_count_and_its_trailer() {
    let bytes = PackBuilder::new().finish().unwrap();
    assert_eq!(bytes, seal(&[], &index(0, &[])));
    // The same 28 bytes spelled out: a zero count, then N = 4, the xxh64 of the four zero
    // bytes, the magic, version 1 and the reserved zero.
    let mut golden = vec![0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0];
    golden.extend_from_slice(&xxh64(&[0; 4], 0).to_le_bytes());
    golden.extend_from_slice(b"LPAK\x01\x00\x00\x00");
    assert_eq!(bytes, golden);
    assert_eq!(PACK_FORMAT_VERSION, 1);
}

#[test]
fn a_one_entry_pack_is_its_payload_its_entry_and_its_trailer() {
    let cooked = text(b"hello", []);
    let mut pack = PackBuilder::new();
    let version = pack.add(id("one"), &cooked, Storage::Raw).unwrap();
    assert_eq!(version, cooked.version());
    let bytes = pack.finish().unwrap();
    let entry = entry_of(id("one"), &cooked, 0);
    assert_eq!(bytes, seal(b"hello", &index(1, &[entry])));
    assert_eq!(bytes.len(), 5 + 4 + 57 + TRAILER_LEN);
    assert_eq!(&bytes[..5], b"hello");
}

#[test]
fn a_two_entry_pack_sorts_its_index_and_keeps_its_payloads_in_add_order() {
    let (low, high) = {
        let (a, b) = (id("first"), id("second"));
        if a < b { (a, b) } else { (b, a) }
    };
    let dependencies = [id("z"), id("y"), id("x")];
    let later = text(b"added first", []);
    let earlier = text(b"then this", dependencies);
    let mut pack = PackBuilder::new();
    pack.add(high, &later, Storage::Raw).unwrap();
    pack.add(low, &earlier, Storage::Raw).unwrap();
    let bytes = pack.finish().unwrap();

    let low_entry = entry_of(low, &earlier, 11);
    let high_entry = entry_of(high, &later, 0);
    let mut sorted: Vec<u64> = dependencies.iter().map(|id| id.to_bits()).collect();
    sorted.sort_unstable();
    assert_eq!(low_entry.dependencies, sorted, "dependencies are stored ascending");
    let golden = seal(b"added firstthen this", &index(2, &[low_entry, high_entry]));
    assert_eq!(bytes, golden);
    assert_eq!(index_range(&bytes).1, 4 + 2 * 57 + 3 * 8);

    // Any add order writes the same index.
    let mut other = PackBuilder::new();
    other.add(low, &earlier, Storage::Raw).unwrap();
    other.add(high, &later, Storage::Raw).unwrap();
    let other = other.finish().unwrap();
    let (start, len) = index_range(&bytes);
    let (other_start, other_len) = index_range(&other);
    assert_eq!(len, other_len);
    assert_eq!(bytes[start..start + 4], other[other_start..other_start + 4]);
    // Same entries, payload offsets swapped.
    assert_eq!(bytes[start + 4..start + 12], other[other_start + 4..other_start + 12]);
}

/// Bytes that compress, and bytes that do not.
fn compressible(len: usize) -> Vec<u8> {
    (0..len).map(|at| b"abcabcabd"[at % 9]).collect()
}

fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

#[test]
fn a_compressed_entry_is_one_frame_with_its_content_size_and_never_longer_than_raw() {
    let shrinks = text(&compressible(4096), []);
    let grows = text(&noise(64), []);
    let empty = text(b"", []);
    let mut pack = PackBuilder::new();
    for (name, cooked) in [("shrinks", &shrinks), ("grows", &grows), ("empty", &empty)] {
        let version = pack.add(id(name), cooked, Storage::Zstd { level: 19 }).unwrap();
        assert_eq!(version, cooked.version(), "a version hashes the decoded bytes");
    }
    let bytes = pack.finish().unwrap();
    let (start, len) = index_range(&bytes);
    let mut entries = parse(&bytes[start..start + len]);
    entries.sort_by_key(|entry| entry.offset);
    assert_eq!(entries.len(), 3);

    let shrunk = entries.iter().find(|entry| entry.id == id("shrinks").to_bits()).unwrap();
    assert_eq!(shrunk.storage, 1);
    assert_eq!(shrunk.decoded, 4096);
    assert!(shrunk.stored < 4096);
    let frame = &bytes[shrunk.offset as usize..(shrunk.offset + shrunk.stored) as usize];
    assert_eq!(zstd::zstd_safe::get_frame_content_size(frame).ok(), Some(Some(4096)));
    assert_eq!(zstd::zstd_safe::find_frame_compressed_size(frame).ok(), Some(frame.len()));
    assert_eq!(zstd::decode_all(frame).unwrap(), compressible(4096));

    for name in ["grows", "empty"] {
        let raw = entries.iter().find(|entry| entry.id == id(name).to_bits()).unwrap();
        assert_eq!(raw.storage, 0, "{name} is stored raw");
        assert_eq!(raw.stored, raw.decoded);
    }

    // Payloads are back to back from offset 0 in add order.
    let mut end = 0;
    for name in ["shrinks", "grows", "empty"] {
        let entry = entries.iter().find(|entry| entry.id == id(name).to_bits()).unwrap();
        assert_eq!(entry.offset, end);
        end += entry.stored;
    }
    assert_eq!(end as usize, start);
}

/// The entries of an index this file's tests wrote, in index order.
fn parse(index: &[u8]) -> Vec<RawEntry> {
    let word = |at: usize| u64::from_le_bytes(index[at..at + 8].try_into().unwrap());
    let half = |at: usize| u32::from_le_bytes(index[at..at + 4].try_into().unwrap());
    let count = half(0);
    let mut at = 4;
    let mut entries = Vec::new();
    for _ in 0..count {
        let dependencies = half(at + 28) as usize;
        let deps = (0..dependencies).map(|n| word(at + 32 + 8 * n)).collect();
        let tail = at + 32 + 8 * dependencies;
        entries.push(RawEntry {
            id: word(at),
            version: word(at + 8),
            kind: word(at + 16),
            format: half(at + 24),
            dependencies: deps,
            storage: index[tail],
            offset: word(tail + 1),
            stored: word(tail + 9),
            decoded: word(tail + 17),
        });
        at = tail + 25;
    }
    assert_eq!(at, index.len());
    entries
}

#[test]
fn add_refuses_what_no_pack_may_hold_and_adds_nothing() {
    let cooked = text(b"content", []);
    let other = text(b"other content", []);
    let mut pack = PackBuilder::new();
    pack.add(id("kept"), &cooked, Storage::Raw).unwrap();
    let before = {
        let mut copy = PackBuilder::new();
        copy.add(id("kept"), &cooked, Storage::Raw).unwrap();
        copy.finish().unwrap()
    };

    let kind = |result: lactose_core::Result<_>| result.unwrap_err().kind();
    assert_eq!(kind(pack.add(AssetId::NONE, &cooked, Storage::Raw)), ErrorKind::InvalidData);
    assert_eq!(kind(pack.add(id("kept"), &other, Storage::Raw)), ErrorKind::InvalidData);
    for level in [0, 23] {
        let refused = pack.add(id("level"), &cooked, Storage::Zstd { level });
        assert_eq!(kind(refused), ErrorKind::InvalidData, "level {level}");
    }
    let header = CookedHeader::new(Text::KIND, Text::FORMAT_VERSION, []);
    let huge = CookedAsset { header, bytes: vec![0; MAX_READ_BYTES as usize + 1] };
    assert_eq!(kind(pack.add(id("huge"), &huge, Storage::Raw)), ErrorKind::ResourceExhausted);
    drop(huge);

    // The same version again, whatever the storage, adds nothing.
    assert_eq!(pack.add(id("kept"), &cooked, Storage::Raw).unwrap(), cooked.version());
    let again = pack.add(id("kept"), &cooked, Storage::Zstd { level: 3 }).unwrap();
    assert_eq!(again, cooked.version());
    assert_eq!(pack.finish().unwrap(), before);
}

// ---- Mounting: each rule broken in turn ----

/// A pack of two raw assets, `a` below `b`, that every mount accepts; the tests break
/// one thing in it at a time.
struct Two {
    payload: Vec<u8>,
    a: RawEntry,
    b: RawEntry,
}

fn two() -> Two {
    let (a, b) = (text(b"alpha", []), text(b"beta", []));
    let (low, high) = ordered("a", "b");
    Two { payload: b"alphabeta".to_vec(), a: entry_of(low, &a, 0), b: entry_of(high, &b, 5) }
}

fn ordered(first: &str, second: &str) -> (AssetId, AssetId) {
    let (a, b) = (id(first), id(second));
    if a < b { (a, b) } else { (b, a) }
}

impl Two {
    fn bytes(&self) -> Vec<u8> {
        seal(&self.payload, &index(2, &[self.a.clone(), self.b.clone()]))
    }
}

/// Mounts `bytes` in a fresh store and returns the mount's result, after checking that
/// nothing of a refused pack was mounted: its lower asset is `NotFound`.
fn mount(env: &Env, bytes: Vec<u8>) -> Result<(), ErrorKind> {
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let path = env.write(bytes);
    let result = env.mount(&store, path).map_err(|error| error.kind());
    if result.is_err() {
        assert_nothing_mounted(env, &store);
    }
    result
}

fn assert_nothing_mounted(env: &Env, store: &Store) {
    let (low, _) = ordered("a", "b");
    store.request_newest(low, LoadPriority::default());
    env.settle(store);
    let failure = AssetFailure { id: low, version: None, kind: ErrorKind::NotFound };
    assert_eq!(store.newest(low).map(|_| ()), Readiness::Failed(failure));
}

#[test]
fn the_unbroken_pack_and_what_the_format_allows_mount() {
    let env = Env::new("pack_allowed");
    assert_eq!(mount(&env, two().bytes()), Ok(()));
    assert_eq!(mount(&env, PackBuilder::new().finish().unwrap()), Ok(()));

    // Shared ranges and padding: a later writer may do either.
    let mut shared = two();
    shared.b.offset = 0;
    shared.b.stored = 5;
    shared.b.decoded = 5;
    shared.payload.extend_from_slice(&[0; 7]);
    assert_eq!(mount(&env, shared.bytes()), Ok(()));

    // An asset that declares itself is a failed load, not a malformed pack.
    let mut own = two();
    own.a.dependencies = vec![own.a.id];
    assert_eq!(mount(&env, own.bytes()), Ok(()));
    env.shutdown();
}

#[test]
fn each_rule_broken_refuses_the_whole_pack() {
    let env = Env::new("pack_rules");
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();

    cases.push(("shorter than a trailer", b"LPAK\x01\x00\x00\x00".to_vec()));
    let mut reserved = two().bytes();
    let at = reserved.len() - 2;
    reserved[at] = 1;
    cases.push(("the reserved u16 set", reserved));
    let mut hash = two().bytes();
    let at = hash.len() - 16;
    hash[at] ^= 1;
    cases.push(("an index that does not hash to the trailer", hash));
    let mut index_byte = two().bytes();
    let (start, _) = index_range(&index_byte);
    index_byte[start + 20] ^= 0x40;
    cases.push(("an index byte changed", index_byte));

    let broken = |change: &dyn Fn(&mut Two)| {
        let mut pack = two();
        change(&mut pack);
        pack.bytes()
    };
    let raw_index =
        |count: u32, entries: &[RawEntry], payload: &[u8]| seal(payload, &index(count, entries));
    let t = two();
    cases.push(("a count past the index", raw_index(3, &[t.a.clone(), t.b.clone()], &t.payload)));
    cases.push((
        "a count short of the index",
        raw_index(1, &[t.a.clone(), t.b.clone()], &t.payload),
    ));
    cases.push(("a count of u32::MAX", raw_index(u32::MAX, &[], &[])));
    cases.push(("an index of three bytes", seal(&[], &[0, 0, 0])));
    let mut left_over = index(2, &[t.a.clone(), t.b.clone()]);
    left_over.push(0);
    cases.push(("a byte left over", seal(&t.payload, &left_over)));
    let mut cut = index(2, &[t.a.clone(), t.b.clone()]);
    cut.truncate(cut.len() - 1);
    cases.push(("an entry cut short", seal(&t.payload, &cut)));
    let mut many = index(1, &[]);
    many.extend_from_slice(&t.a.id.to_le_bytes());
    many.extend_from_slice(&t.a.version.to_le_bytes());
    many.extend_from_slice(&1u64.to_le_bytes());
    many.extend_from_slice(&1u32.to_le_bytes());
    many.extend_from_slice(&u32::MAX.to_le_bytes());
    cases.push(("a dependency count of u32::MAX", seal(&[], &many)));

    cases.push(("an ID of zero", broken(&|pack| pack.a.id = 0)));
    cases.push(("IDs equal", broken(&|pack| pack.b.id = pack.a.id)));
    cases.push(("IDs descending", seal(&t.payload, &index(2, &[t.b.clone(), t.a.clone()]))));
    cases.push(("a version of zero", broken(&|pack| pack.b.version = 0)));
    cases.push(("a dependency of zero", broken(&|pack| pack.b.dependencies = vec![0, 5])));
    cases.push(("dependencies equal", broken(&|pack| pack.b.dependencies = vec![5, 5])));
    cases.push(("dependencies descending", broken(&|pack| pack.b.dependencies = vec![6, 5])));
    cases.push(("a storage code of 2", broken(&|pack| pack.b.storage = 2)));
    cases.push((
        "a range that overflows",
        broken(&|pack| {
            pack.b.offset = u64::MAX - 1;
        }),
    ));
    cases.push((
        "a range into the index",
        broken(&|pack| {
            pack.b.stored = 5;
            pack.b.decoded = 5;
        }),
    ));
    cases.push((
        "a raw length over the cap",
        broken(&|pack| {
            pack.b.offset = 0;
            pack.b.stored = MAX_READ_BYTES + 1;
            pack.b.decoded = MAX_READ_BYTES + 1;
        }),
    ));
    cases.push((
        "a decoded length over the cap",
        broken(&|pack| {
            pack.b.storage = 1;
            pack.b.decoded = MAX_READ_BYTES + 1;
        }),
    ));
    cases.push(("a raw entry whose lengths differ", broken(&|pack| pack.b.decoded = 3)));
    cases.push((
        "a zstd entry of no stored bytes",
        broken(&|pack| {
            pack.b.storage = 1;
            pack.b.stored = 0;
        }),
    ));

    for (what, bytes) in cases {
        assert_eq!(mount(&env, bytes), Err(ErrorKind::InvalidData), "{what}");
    }
    env.shutdown();
}

/// The spans of `io` reads issued since span index `from`, as (offset, len) pairs.
fn reads_since(from: usize) -> Vec<(u64, u64)> {
    let reads = capture().closed(from, "io.read");
    let field = |span: &common::capture::Span, name: &str| -> u64 {
        span.field(name).expect("a field of io.read").parse().expect("a number")
    };
    reads.iter().map(|(_, span)| (field(span, "offset"), field(span, "len"))).collect()
}

#[test]
fn the_magic_and_the_version_are_checked_before_anything_else() {
    let capture = capture();
    let env = Env::new("pack_magic");

    // Another version, with every other part of the trailer and index wrong as well.
    let mut other = vec![0xEE; 40];
    other.extend_from_slice(&trailer(u64::MAX, 7, 2, 9));
    let from = capture.span_count();
    assert_eq!(mount(&env, other), Err(ErrorKind::VersionMismatch));
    let len = 40 + TRAILER_LEN as u64;
    assert_eq!(reads_since(from), [(len - 24, 24)], "only the trailer is read");

    let mut magic = two().bytes();
    let at = magic.len() - 8;
    magic[at] = b'X';
    let len = magic.len() as u64;
    let from = capture.span_count();
    assert_eq!(mount(&env, magic), Err(ErrorKind::InvalidData));
    assert_eq!(reads_since(from), [(len - 24, 24)]);

    let from = capture.span_count();
    assert_eq!(mount(&env, vec![1, 2, 3]), Err(ErrorKind::InvalidData));
    assert_eq!(reads_since(from), [], "a file shorter than a trailer is not read");
    env.shutdown();
}

#[test]
fn an_index_length_past_the_file_is_invalid_with_no_read_of_an_index() {
    let capture = capture();
    let env = Env::new("pack_past");
    for len in [100, 1 << 31] {
        let mut bytes = vec![0; 40];
        bytes.extend_from_slice(&trailer(len, 0, 1, 0));
        let size = bytes.len() as u64;
        let from = capture.span_count();
        assert_eq!(mount(&env, bytes), Err(ErrorKind::InvalidData), "N = {len}");
        assert_eq!(reads_since(from), [(size - 24, 24)], "N = {len}");
    }
    env.shutdown();
}

#[test]
fn an_index_within_the_file_and_over_the_cap_is_resource_exhausted() {
    let capture = capture();
    let env = Env::new("pack_sparse");
    // A sparse file of a little over 1 GiB whose trailer names an index of 1 GiB and one
    // byte: written in place by std, since the bytes need not exist in memory.
    let path = env.dir.join("sparse.pack");
    let index_len = MAX_READ_BYTES + 1;
    let mut file = std::fs::File::create(&path).unwrap();
    file.set_len(index_len + 64).unwrap();
    file.seek(SeekFrom::Start(index_len + 64)).unwrap();
    file.write_all(&trailer(index_len, 0, 1, 0)).unwrap();
    drop(file);
    let size = index_len + 64 + TRAILER_LEN as u64;

    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let from = capture.span_count();
    let refused = env.mount(&store, path.clone()).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::ResourceExhausted);
    assert_eq!(reads_since(from), [(size - 24, 24)]);
    assert_nothing_mounted(&env, &store);
    std::fs::remove_file(path).unwrap();
    env.shutdown();
}

#[test]
fn a_stored_range_within_the_payload_and_over_the_cap_is_invalid() {
    let env = Env::new("pack_stored_over_the_cap");
    // A zstd entry whose stored range lies within a sparse payload area of 1 GiB and one
    // byte, so that only the cap on one read refuses it.
    let len = MAX_READ_BYTES + 1;
    let entry = RawEntry { storage: 1, stored: len, decoded: 16, ..RawEntry::raw(1, 1, 0, len) };
    let sealed = seal(&[], &index(1, &[entry]));
    let path = env.dir.join("stored-over-the-cap.pack");
    let mut file = std::fs::File::create(&path).unwrap();
    file.set_len(len).unwrap();
    file.seek(SeekFrom::Start(len)).unwrap();
    file.write_all(&sealed).unwrap();
    drop(file);

    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let refused = env.mount(&store, path.clone()).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::InvalidData);
    assert_nothing_mounted(&env, &store);
    std::fs::remove_file(path).unwrap();
    env.shutdown();
}
