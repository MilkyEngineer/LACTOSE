//! What a mount's index costs `AllocTag::Asset` (`README.md`, "The pack format"): nothing
//! reserved for a count before the count is checked against the index's length, and the
//! decoded index charged to the tag while the pack is mounted. The process's allocator
//! counts by tag, which assumes one process per test, as nextest runs them; nothing else
//! of a test allocates under `AllocTag::Asset` while its mount runs.

#![forbid(unsafe_code)]

mod common;

use common::format::{RawEntry, index, seal, trailer};
use common::*;
use lactose_asset::Storage;
use lactose_core::{AllocTag, ErrorKind, TaggedAlloc};

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

/// Mounts `bytes` on a store of its own: the mount's result and how many allocations
/// `AllocTag::Asset` counted while it ran.
fn asset_allocations_of_mount(env: &Env, bytes: Vec<u8>) -> (Result<(), ErrorKind>, u64) {
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let path = env.write(bytes);
    let before = ALLOCATOR.stats(AllocTag::Asset).total_allocations;
    let result = env.mount(&store, path).map_err(|error| error.kind());
    let after = ALLOCATOR.stats(AllocTag::Asset).total_allocations;
    (result, after - before)
}

#[test]
fn a_count_past_the_index_reserves_nothing() {
    let env = Env::new("index_count");
    // An index whose hash is wrong, refused before its count is read.
    let mut bad_hash = index(0, &[]);
    bad_hash.extend_from_slice(&trailer(4, 1, 1, 0));
    // A count of `u32::MAX` in an index of four bytes, hashed correctly.
    let huge = seal(&[], &index(u32::MAX, &[]));

    let (refused, hash_allocations) = asset_allocations_of_mount(&env, bad_hash);
    assert_eq!(refused, Err(ErrorKind::InvalidData));
    let (refused, huge_allocations) = asset_allocations_of_mount(&env, huge);
    assert_eq!(refused, Err(ErrorKind::InvalidData));
    // The count's refusal formats one message the hash's refusal does not; a reserve for
    // the count before the check would be one allocation more.
    assert!(
        huge_allocations <= hash_allocations + 1,
        "{huge_allocations} allocations for the count, {hash_allocations} for the hash"
    );
    env.shutdown();
}

#[test]
fn a_dependency_count_past_the_index_reserves_nothing() {
    let env = Env::new("index_dependency_count");
    // One entry of no dependencies whose storage code is neither raw nor zstd, refused
    // after its dependencies were read.
    let entry = RawEntry { storage: 7, ..RawEntry::raw(1, 1, 0, 0) };
    let late = seal(&[], &index(1, std::slice::from_ref(&entry)));
    // The same entry claiming `u32::MAX` dependencies, refused at its count.
    let mut claimed = index(1, &[entry]);
    claimed[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    let huge = seal(&[], &claimed);

    let (refused, late_allocations) = asset_allocations_of_mount(&env, late);
    assert_eq!(refused, Err(ErrorKind::InvalidData));
    let (refused, huge_allocations) = asset_allocations_of_mount(&env, huge);
    assert_eq!(refused, Err(ErrorKind::InvalidData));
    // Each refusal formats one message after the index's entries were reserved; an empty
    // dependency list reserves nothing, and a reserve for the count would be one more.
    assert!(
        huge_allocations <= late_allocations,
        "{huge_allocations} allocations for the count, {late_allocations} for the storage code"
    );
    env.shutdown();
}

#[test]
fn a_mounted_index_is_charged_to_the_asset_tag() {
    let env = Env::new("index_tag");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let assets: Vec<_> = (0..1000)
        .map(|at| (id(&format!("tag {at}")), text(format!("{at}").as_bytes(), [])))
        .collect();
    let path = env.pack(&assets, Storage::Raw);
    let before = ALLOCATOR.stats(AllocTag::Asset).live_bytes;
    env.mount(&store, path).unwrap();
    let after = ALLOCATOR.stats(AllocTag::Asset).live_bytes;
    // At least the 57 bytes an entry of no dependencies takes in the index.
    assert!(after.saturating_sub(before) >= 1000 * 57, "{before} bytes before, {after} after");
    drop(store);
    env.shutdown();
}
