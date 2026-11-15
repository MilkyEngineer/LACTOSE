//! Tracing (`README.md`, "Tracing") seen by a capturing subscriber, and the cost of a large
//! mount counted in the spans of its requests.

#![forbid(unsafe_code)]

mod common;

use std::io::{Seek, SeekFrom, Write};
use std::time::Instant;

use common::capture::{Capture, Span, capture};
use common::format::{RawEntry, index, trailer};
use common::*;
use lactose_asset::Storage;
use lactose_asset_api::{Asset, AssetCatalog, AssetId, LoadPriority, PublishedAssets};
use lactose_core::xxh64;
use tracing::Level;

fn hex(bits: u64) -> String {
    format!("{bits:#018x}")
}

/// The one span named `name` from index `from` on, once closed, with its index.
fn only(capture: &Capture, from: usize, name: &str) -> (usize, Span) {
    let mut spans = capture.closed(from, name);
    assert_eq!(spans.len(), 1, "one {name}: {spans:#?}");
    spans.remove(0)
}

#[test]
fn a_mount_and_a_load_emit_the_spans_and_events_of_the_table() {
    let capture = capture();
    let env = Env::new("spans_table");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let (x, dependency) = (id("x"), id("dependency"));
    let (cooked, dependency_cooked) = (text(b"x", [dependency]), text(b"dependency", []));
    let path =
        env.pack(&[(x, cooked.clone()), (dependency, dependency_cooked.clone())], Storage::Raw);

    // The mount.
    let from = capture.span_count();
    env.mount(&store, path.clone()).unwrap();
    let (mount, span) = only(capture, from, "silo.mount");
    assert_eq!(span.target, "silo");
    assert_eq!(span.level, "INFO");
    assert_eq!(span.created.get("path"), Some(&path.display().to_string()));
    assert_eq!(span.created.get("place").map(String::as_str), Some("0"));
    assert_eq!(span.field("assets"), Some("2"));
    assert_eq!(span.field("requested"), Some("0"));
    assert_eq!(span.field("error"), None);
    let (_, open) = only(capture, from, "io.open");
    assert_eq!(open.parent, Some(mount));
    let reads = capture.closed(from, "io.read");
    assert_eq!(reads.len(), 2);
    assert!(reads.iter().all(|(_, read)| read.parent == Some(mount)));

    // A load, accepted inside a requester's span: a root all the same.
    let events = capture.event_count();
    let from = capture.span_count();
    {
        let requester = tracing::info_span!("requester");
        let _entered = requester.enter();
        store.request_newest(x, LoadPriority(3));
    }
    env.settle(&store);
    let loads = capture.closed(from, "silo.load");
    assert_eq!(loads.len(), 2, "x and its dependency");
    let field = |span: &Span, name: &str| span.field(name).map(str::to_owned);
    for (_, load) in &loads {
        assert_eq!(load.parent, None, "a root span");
        assert_eq!(load.level, "INFO");
        assert_eq!(field(load, "priority").as_deref(), Some("3"));
        assert_eq!(field(load, "kind"), Some(hex(Text::KIND.0)));
        assert_eq!(field(load, "error"), None);
        assert!(load.entered > 0, "entered for the polls of its task");
    }
    let (x_load, x_span) =
        loads.iter().find(|(_, load)| field(load, "asset") == Some(hex(x.to_bits()))).unwrap();
    assert_eq!(field(x_span, "cause").as_deref(), Some("newest"));
    assert_eq!(field(x_span, "version"), Some(hex(cooked.version().to_bits())));
    assert_eq!(field(x_span, "bytes").as_deref(), Some("1"));
    let (dependency_load, dependency_span) = loads
        .iter()
        .find(|(_, load)| field(load, "asset") == Some(hex(dependency.to_bits())))
        .unwrap();
    assert_eq!(field(dependency_span, "cause").as_deref(), Some("dependency"));
    assert_eq!(field(dependency_span, "bytes").as_deref(), Some("10"));
    for (_, read) in capture.closed(from, "io.read") {
        assert!(read.parent == Some(*x_load) || read.parent == Some(*dependency_load));
    }

    let published = capture.store_events(events, Level::INFO);
    assert_eq!(published.len(), 2, "{published:#?}");
    assert!(published.iter().all(|event| event.name == "silo.publish"));
    let x_published = published
        .iter()
        .find(|event| event.fields.get("asset") == Some(&hex(x.to_bits())))
        .unwrap();
    assert_eq!(x_published.parent, Some(*x_load));
    assert_eq!(x_published.fields.get("current").map(String::as_str), Some("true"));
    assert_eq!(x_published.fields.get("version"), Some(&hex(cooked.version().to_bits())));
    let generation = store.generation().to_string();
    assert!(published.iter().any(|event| event.fields.get("generation") == Some(&generation)));
    assert!(capture.store_events(events, Level::WARN).is_empty());

    // A request that does nothing emits nothing.
    let (spans, events) = (capture.span_count(), capture.event_count());
    store.request_newest(x, LoadPriority(1));
    store.request(dependency, dependency_cooked.version(), LoadPriority(1));
    store.request_newest(AssetId::NONE, LoadPriority(1));
    assert_eq!((capture.span_count(), capture.event_count()), (spans, events));

    // A mount that makes a published version current again: `silo.current`, and a mount's
    // own cause.
    let newer = text(b"newer x", [dependency]);
    env.provide(&store, &[(x, newer.clone())]);
    env.settle(&store);
    let events = capture.event_count();
    let from = capture.span_count();
    env.mount(&store, path).unwrap();
    let (_, remount) = only(capture, from, "silo.mount");
    assert_eq!(remount.field("requested"), Some("0"));
    let current = capture.store_events(events, Level::INFO);
    assert_eq!(current.len(), 1, "{current:#?}");
    assert_eq!(current[0].name, "silo.current");
    assert_eq!(current[0].fields.get("asset"), Some(&hex(x.to_bits())));
    assert_eq!(current[0].fields.get("version"), Some(&hex(cooked.version().to_bits())));
    assert_eq!(current[0].fields.get("generation"), Some(&store.generation().to_string()));
    env.shutdown();
}

#[test]
fn a_mount_accepts_its_loads_with_its_own_cause() {
    let capture = capture();
    let env = Env::new("spans_mount_cause");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, text(b"one", []))]);
    store.request_newest(x, LoadPriority(0));
    env.settle(&store);
    let from = capture.span_count();
    env.provide(&store, &[(x, text(b"two", []))]);
    env.settle(&store);
    let (mount, span) = only(capture, from, "silo.mount");
    assert_eq!(span.field("requested"), Some("1"));
    let (_, load) = only(capture, from, "silo.load");
    assert_eq!(load.parent, None, "a root, also when accepted inside the mount's span");
    assert_ne!(load.parent, Some(mount));
    assert_eq!(load.field("cause"), Some("mount"));
    assert_eq!(load.field("priority"), Some("0"));
    env.shutdown();
}

#[test]
fn every_recorded_failure_is_one_warn_inside_its_load() {
    let capture = capture();
    let env = Env::new("spans_failures");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let (refused, dependant, missing) = (id("refused"), id("dependant"), id("missing"));
    let refused_cooked = gated(b"refuse", []);
    env.provide(
        &store,
        &[(refused, refused_cooked.clone()), (dependant, text(b"dependant", [refused]))],
    );

    // Ended inside the call: no entry, so no version and no kind.
    let events = capture.event_count();
    let from = capture.span_count();
    store.request_newest(missing, LoadPriority(0));
    let (load, span) = only(capture, from, "silo.load");
    assert_eq!(span.field("asset"), Some(hex(missing.to_bits()).as_str()));
    assert_eq!(span.field("version"), None);
    assert_eq!(span.field("kind"), None);
    assert_eq!(span.field("error"), Some("NotFound"));
    let warned = capture.store_events(events, Level::WARN);
    assert_eq!(warned.len(), 1);
    assert_eq!(warned[0].parent, Some(load));
    assert_eq!(warned[0].fields.get("kind").map(String::as_str), Some("NotFound"));
    assert!(!warned[0].fields.contains_key("version"));

    // A loader's refusal, and a dependant failed for it.
    let events = capture.event_count();
    let from = capture.span_count();
    store.request_newest(dependant, LoadPriority(0));
    env.settle(&store);
    let loads = capture.closed(from, "silo.load");
    assert_eq!(loads.len(), 2);
    let warned = capture.store_events(events, Level::WARN);
    assert_eq!(warned.len(), 2, "{warned:#?}");
    for event in &warned {
        let (_, span) = loads.iter().find(|(at, _)| event.parent == Some(*at)).unwrap();
        assert_eq!(span.field("asset"), event.fields.get("asset").map(String::as_str));
        assert_eq!(span.field("error"), Some("InvalidData"));
        assert_eq!(event.fields.get("kind").map(String::as_str), Some("InvalidData"));
        assert!(event.fields.contains_key("message"), "{event:#?}");
    }
    let of_dependant = warned
        .iter()
        .find(|event| event.fields.get("asset") == Some(&hex(dependant.to_bits())))
        .unwrap();
    assert_eq!(of_dependant.fields.get("dependency"), Some(&hex(refused.to_bits())));
    let of_refused = warned
        .iter()
        .find(|event| event.fields.get("asset") == Some(&hex(refused.to_bits())))
        .unwrap();
    assert_eq!(of_refused.fields.get("version"), Some(&hex(refused_cooked.version().to_bits())));
    assert!(!of_refused.fields.contains_key("dependency"));

    // Nothing else at `INFO` or above.
    let others: Vec<_> = capture
        .events()
        .iter()
        .filter(|event| event.target == "silo" && event.level <= Level::INFO)
        .filter(|event| event.level != Level::WARN)
        .filter(|event| event.name != "silo.publish" && event.name != "silo.current")
        .cloned()
        .collect();
    assert!(others.is_empty(), "{others:#?}");
    env.shutdown();
}

/// A pack of `ENTRIES` raw entries over a sparse payload area of `ENTRIES * STRIDE`
/// bytes, written in place by std: the payloads need not exist for a mount.
const ENTRIES: u64 = 100_000;
const STRIDE: u64 = 3_000;

#[test]
fn a_mount_of_100000_entries_is_one_open_and_two_reads() {
    let capture = capture();
    let env = Env::new("spans_mount_cost");
    let entries: Vec<RawEntry> =
        (1..=ENTRIES).map(|n| RawEntry::raw(n, n, (n - 1) * STRIDE, STRIDE)).collect();
    let index = index(ENTRIES as u32, &entries);
    let payload_len = ENTRIES * STRIDE;
    let path = env.dir.join("large.pack");
    let mut file = std::fs::File::create(&path).unwrap();
    file.set_len(payload_len).unwrap();
    file.seek(SeekFrom::Start(payload_len)).unwrap();
    file.write_all(&index).unwrap();
    file.write_all(&trailer(index.len() as u64, xxh64(&index, 0), 1, 0)).unwrap();
    drop(file);
    let size = payload_len + index.len() as u64 + 24;

    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let from = capture.span_count();
    let started = Instant::now();
    env.mount(&store, path.clone()).unwrap();
    let elapsed = started.elapsed();

    let (mount, span) = only(capture, from, "silo.mount");
    assert_eq!(span.field("assets"), Some(ENTRIES.to_string().as_str()));
    let (_, open) = only(capture, from, "io.open");
    assert_eq!(open.parent, Some(mount));
    let reads: Vec<(u64, u64)> = capture
        .closed(from, "io.read")
        .iter()
        .map(|(_, read)| {
            assert_eq!(read.parent, Some(mount));
            let number = |name| read.field(name).unwrap().parse::<u64>().unwrap();
            (number("offset"), number("len"))
        })
        .collect();
    let index_len = index.len() as u64;
    assert_eq!(reads, [(size - 24, 24), (size - 24 - index_len, index_len)]);
    let names: Vec<&str> = capture.spans()[from..].iter().map(|span| span.name).collect();
    assert_eq!(names.len(), 4, "the mount, its open and its two reads: {names:?}");

    // The last entry is mounted: its exact version is accepted from this pack.
    let last = lactose_asset_api::AssetVersion::from_bits(ENTRIES).unwrap();
    store.request(AssetId::from_bits(ENTRIES), last, LoadPriority(0));
    let load = capture.named(from, "silo.load");
    assert_eq!(load.len(), 1);

    let uptime = std::process::Command::new("uptime").output().map(|out| out.stdout);
    let uptime = uptime.map(|out| String::from_utf8_lossy(&out).trim().to_owned());
    eprintln!(
        "mount cost: {ENTRIES} entries, index {index_len} bytes, file {size} bytes: {elapsed:?} \
         (uptime: {})",
        uptime.unwrap_or_default()
    );
    drop(store);
    std::fs::remove_file(path).unwrap();
    env.shutdown();
}
