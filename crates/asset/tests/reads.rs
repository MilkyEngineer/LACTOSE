//! A pack read through `io`, raw and compressed, and damage found as a value: a version
//! its content does not hash to, a payload byte flipped.

#![forbid(unsafe_code)]

mod common;

use common::capture::capture;
use common::format::{index_range, reseal};
use common::*;
use lactose_asset::Storage;
use lactose_asset_api::{
    AssetCatalog, AssetFailure, AssetId, AssetRef, AssetVersion, LoadPriority, PublishedAssets,
    Readiness,
};
use lactose_core::ErrorKind;
use tracing::Level;

fn compressible(len: usize) -> Vec<u8> {
    (0..len).map(|at| b"the quick brown fox "[at % 20] ^ (at / 4096) as u8).collect()
}

fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

/// Assets of several sizes, the empty one and several MiB among them.
fn contents() -> Vec<(AssetId, Vec<u8>)> {
    vec![
        (id("empty"), Vec::new()),
        (id("one byte"), vec![7]),
        (id("a page"), compressible(4096)),
        (id("noise"), noise(70_000)),
        (id("several MiB"), compressible(3 * 1024 * 1024 + 7)),
    ]
}

#[test]
fn assets_read_back_equal_raw_and_compressed() {
    let capture = capture();
    for (name, storage) in [("raw", Storage::Raw), ("zstd", Storage::Zstd { level: 3 })] {
        let env = Env::new(&format!("reads_{name}"));
        let (loaders, _) = test_loaders();
        let store = env.store(loaders);
        let assets: Vec<_> =
            contents().into_iter().map(|(id, bytes)| (id, text(&bytes, []))).collect();
        let path = env.pack(&assets, storage);
        env.mount(&store, path).unwrap();

        let from = capture.span_count();
        for (id, _) in &assets {
            store.request_newest(*id, LoadPriority::default());
        }
        env.settle(&store);
        let catalog: &dyn AssetCatalog = &store;
        for (id, cooked) in &assets {
            let handle = catalog.newest_typed(AssetRef::<Text>::new(*id)).ready().unwrap();
            assert_eq!(handle.version(), cooked.version());
            assert_eq!(handle.0, cooked.bytes, "{name}: {id:?}");
            let exact = catalog.get_typed(AssetRef::<Text>::new(*id), cooked.version());
            assert_eq!(exact.ready().unwrap().0, cooked.bytes);
        }

        let reads = capture.closed(from, "io.read");
        assert_eq!(reads.len(), assets.len(), "{name}: one read per asset");
        let compressed: Vec<_> =
            reads.iter().filter(|(_, span)| span.field("encoding") == Some("zstd")).collect();
        if storage == Storage::Raw {
            assert!(compressed.is_empty());
        } else {
            // The page and the several MiB compress; the empty asset and the noise do not.
            assert_eq!(compressed.len(), 2, "{name}");
            for (_, span) in compressed {
                let decoded: u64 = span.field("decoded_len").unwrap().parse().unwrap();
                assert!(decoded == 4096 || decoded == 3 * 1024 * 1024 + 7);
                let on_worker =
                    span.threads.iter().any(|thread| thread.starts_with("whisk-background"));
                assert!(on_worker, "the decode ran on a background worker: {:?}", span.threads);
            }
        }
        env.shutdown();
    }
}

/// Rewrites the version the index of `pack` claims for its entry `at` (in ID order).
fn claim(pack: &mut [u8], at: usize, version: u64) {
    let (start, _) = index_range(pack);
    let entry = start + 4 + 57 * at;
    pack[entry + 8..entry + 16].copy_from_slice(&version.to_le_bytes());
    reseal(pack);
}

/// The offset of the middle of entry `at`'s stored bytes.
fn middle(pack: &[u8], at: usize) -> usize {
    let (start, _) = index_range(pack);
    let entry = start + 4 + 57 * at + 33;
    let word = |at: usize| u64::from_le_bytes(pack[at..at + 8].try_into().unwrap()) as usize;
    word(entry) + word(entry + 8) / 2
}

#[test]
fn damage_is_a_failure_with_the_claimed_version_and_the_rest_of_the_pack_loads() {
    let capture = capture();
    let env = Env::new("reads_damage");
    let (low, high) = {
        let (a, b) = (id("damaged"), id("intact"));
        if a < b { (a, b) } else { (b, a) }
    };
    let damaged_at = 0;
    let cases: [(&str, Storage, bool); 4] = [
        ("claim raw", Storage::Raw, true),
        ("claim zstd", Storage::Zstd { level: 3 }, true),
        ("flip raw", Storage::Raw, false),
        ("flip zstd", Storage::Zstd { level: 3 }, false),
    ];
    for (what, storage, claims) in cases {
        let (loaders, _) = test_loaders();
        let store = env.store(loaders);
        let damaged = text(&compressible(10_000), []);
        let intact = text(&compressible(9_000), []);
        let mut bytes = pack(&[(low, damaged.clone()), (high, intact.clone())], storage);
        let claimed = if claims {
            let claimed = AssetVersion::from_bits(0x1234_5678_9ABC_DEF0).unwrap();
            claim(&mut bytes, damaged_at, claimed.to_bits());
            claimed
        } else {
            let at = middle(&bytes, damaged_at);
            bytes[at] ^= 0x01;
            damaged.version()
        };
        let path = env.write(bytes);
        env.mount(&store, path).unwrap();

        let warns = capture.event_count();
        store.request_newest(low, LoadPriority::default());
        store.request_newest(high, LoadPriority::default());
        env.settle(&store);
        let failure =
            AssetFailure { id: low, version: Some(claimed), kind: ErrorKind::InvalidData };
        assert_eq!(store.newest(low).map(|_| ()), Readiness::Failed(failure), "{what}");
        assert_eq!(store.get(low, claimed).map(|_| ()), Readiness::Failed(failure), "{what}");
        let handle = store.newest(high).ready().unwrap();
        assert_eq!(handle.version(), intact.version(), "{what}");

        let warned = capture.silo_events(warns, Level::WARN);
        let asset = format!("{:#018x}", low.to_bits());
        assert_eq!(warned.len(), 1, "{what}: {warned:#?}");
        assert_eq!(warned[0].fields.get("asset"), Some(&asset));
        assert_eq!(warned[0].fields.get("kind").map(String::as_str), Some("InvalidData"));
    }
    env.shutdown();
}
