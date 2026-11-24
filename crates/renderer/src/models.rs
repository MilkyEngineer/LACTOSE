//! loom models of the staging ring's open word and the regions' cursors (`README.md`,
//! "Concurrency").
//!
//! Run with `cargo test -p lactose-renderer --features loom --release`. Every model calls
//! what the crate ships, `ExtractSink`, `RemoteWriter`, `Staging::open`, `Staging::seal` and
//! `Staging::entries`, over loom's atomics through the `sync` shim. The rows' bytes go to a
//! null device behind its own lock, which loom does not model and no property here rests
//! on: what `prepare` reads of a region is its shadow entries and counts.
//!
//! The models run at preemption bound 3 unless `LOOM_MAX_PREEMPTIONS` sets another.

use std::sync::Arc;

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_math::render::RenderPosition;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, RenderTransform};
use lactose_rhi_api::{
    BufferDesc, BufferUsage, Capabilities, Device, DeviceRequest, MemoryClass, Message,
    MessageSink, Sharing,
};
use lactose_rhi_null::NullDevice;
use loom::thread;

use crate::key::KeyBits;
use crate::scatter::StagingRow;
use crate::staging::{ExtractSink, Note, Sealed, Staging, Unsealed, Use, kind_index};

const PREEMPTION_BOUND: usize = 3;

fn model(body: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    if builder.preemption_bound.is_none() {
        builder.preemption_bound = Some(PREEMPTION_BOUND);
    }
    builder.check(body);
}

struct Silent;

impl MessageSink for Silent {
    fn message(&self, _: &Message<'_>) {}
}

/// A ring of two regions of `rows` rows on a null device, region zero open under
/// generation one.
fn ring(rows: u32) -> Arc<Staging> {
    let request = DeviceRequest {
        adapter: None,
        surface: None,
        heap_slots: 0,
        samplers: 0,
        pipeline_cache: Vec::new(),
        messages: Arc::new(Silent),
    };
    let device: Arc<dyn Device> =
        Arc::new(NullDevice::new(request, Capabilities::LEAST).expect("a null device"));
    let buffers: Vec<_> = (0..2)
        .map(|_| {
            let desc = BufferDesc {
                label: "rows",
                size: u64::from(rows) * u64::from(StagingRow::BYTES),
                usage: BufferUsage::DEVICE_ADDRESS,
                memory: MemoryClass::Upload,
                sharing: Sharing::Exclusive,
            };
            device.create_buffer(&desc).expect("a buffer")
        })
        .collect();
    Arc::new(Staging::new(device, rows, &buffers))
}

fn row(n: u64) -> InstanceRow {
    InstanceRow {
        key: StableKey::Net(NetId::from_bits(n).expect("a nonzero id")),
        mesh: AssetRef::new(AssetId::from_bits(n)),
        transform: RenderTransform {
            position: RenderPosition::default(),
            rotation: [0.0, 0.0, 0.0, 1.0],
        },
        no_interpolate: false,
    }
}

const KIND: WorldKind = WorldKind::Prediction;

const PASS: ExtractPass = ExtractPass { kind: KIND, tick: Tick(7), full: false };

const fn lost(kind: WorldKind) -> u8 {
    1 << kind_index(kind)
}

/// The notes of a sealed region, without their indices.
fn notes(staging: &Staging, sealed: &Sealed) -> Vec<(u32, Note)> {
    staging.entries(sealed).map(|entry| (entry.pass, entry.note)).collect()
}

/// What a whole pass of `PASS` writing `rows` leaves, its notes in order, the `begin` at
/// `at`. Rows written from several threads may lie in either order.
fn whole(at: u32, rows: &[u64]) -> Vec<(u32, Note)> {
    let mut notes = vec![(at, Note::Begin { kind: KIND, tick: PASS.tick, full: PASS.full })];
    for &n in rows {
        let row = row(n);
        notes.push((at, Note::Row { key: KeyBits::of(row.key), mesh: row.mesh }));
    }
    notes.push((at, Note::End));
    notes
}

/// Whether `kind`'s loss is reported by exactly one of `sealings`. A pass that finds no
/// region open marks the renderer-wide record, which the sealing before it may still take
/// if the mark lands between its compare-exchange and its swap: the loss is then reported a
/// frame early, and never twice or not at all.
fn reported_once(kind: WorldKind, sealings: &[&Sealed]) -> bool {
    let reporting = sealings.iter().filter(|sealed| sealed.counts.lost & lost(kind) != 0);
    reporting.count() == 1
}

/// Seals the open region, retrying while a pass is open.
fn seal_when_closed(staging: &Staging) -> Sealed {
    loop {
        match staging.seal() {
            Ok(sealed) => return sealed,
            Err(Unsealed::PassOpen) => thread::yield_now(),
            Err(Unsealed::NotOpen) => panic!("the model keeps a region open"),
        }
    }
}

#[test]
fn a_begin_and_a_sealing_never_both_succeed() {
    model(|| {
        let staging = ring(8);
        let shared = Arc::clone(&staging);
        let writer = thread::spawn(move || {
            let mut sink = ExtractSink::new(shared);
            sink.begin(PASS);
            sink.end();
        });
        let sealed = staging.seal();
        writer.join().expect("the writer");
        match sealed {
            // The pass had ended before the sealing, which reads it whole.
            Ok(sealed) if !notes(&staging, &sealed).is_empty() => {
                assert_eq!(notes(&staging, &sealed), whole(0, &[]));
                assert_eq!(sealed.counts.lost, 0);
            }
            // The sealing came first: the pass found no region, and is lost in the
            // renderer-wide record, not in the sealed region.
            Ok(sealed) => {
                staging.open(Use { region: 1, generation: 2 });
                let next = staging.seal().expect("no pass is open");
                assert!(notes(&staging, &next).is_empty());
                assert!(reported_once(KIND, &[&sealed, &next]));
            }
            // The begin came first and its count refused the sealing.
            Err(why) => {
                assert_eq!(why, Unsealed::PassOpen);
                let sealed = staging.seal().expect("the pass ended");
                assert_eq!(notes(&staging, &sealed), whole(0, &[]));
                assert_eq!(sealed.counts.lost, 0);
            }
        }
    });
}

#[test]
fn a_pass_that_ends_is_read_whole_by_the_sealing_after_it() {
    model(|| {
        let staging = ring(8);
        let shared = Arc::clone(&staging);
        let writer = thread::spawn(move || {
            let mut sink = ExtractSink::new(shared);
            sink.begin(PASS);
            sink.write(&[row(1)]);
            sink.removed(&[row(2).key]);
            sink.end();
        });
        let sealed = seal_when_closed(&staging);
        let read = notes(&staging, &sealed);
        writer.join().expect("the writer");
        if read.is_empty() {
            // Sealed before the begin: the pass is lost, and reported once.
            staging.open(Use { region: 1, generation: 2 });
            let next = staging.seal().expect("closed");
            assert!(reported_once(KIND, &[&sealed, &next]));
        } else {
            let row = row(1);
            let expected = vec![
                (0, Note::Begin { kind: KIND, tick: PASS.tick, full: PASS.full }),
                (0, Note::Row { key: KeyBits::of(row.key), mesh: row.mesh }),
                (0, Note::Removed(KeyBits::of(self::row(2).key))),
                (0, Note::End),
            ];
            assert_eq!(read, expected);
            assert_eq!(sealed.counts.lost, 0);
        }
    });
}

#[test]
fn rows_written_from_two_threads_are_each_read_once() {
    model(|| {
        let staging = ring(8);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(PASS);
        let sink = Arc::new(sink);
        let writers: Vec<_> = [1, 2]
            .map(|n| {
                let sink = Arc::clone(&sink);
                thread::spawn(move || sink.write(&[row(n)]))
            })
            .into_iter()
            .collect();
        for writer in writers {
            writer.join().expect("a writer");
        }
        let mut sink = Arc::into_inner(sink).expect("the writers are done");
        sink.end();
        let sealed = staging.seal().expect("closed");
        let read = notes(&staging, &sealed);
        assert!(read == whole(0, &[1, 2]) || read == whole(0, &[2, 1]), "{read:?}");
        assert_eq!(sealed.counts.dropped, 0);
    });
}

#[test]
fn no_use_of_a_region_reads_another_uses_entries() {
    // The writer's pass lands in the first use, in the second, or in no region; the
    // renderer seals, opens the same region again and seals. Whichever, the pass is read
    // whole by one sealing, or reported lost by one, and no sealing reads a part of it or
    // an entry the other use's writer left.
    model(|| {
        let staging = ring(8);
        let shared = Arc::clone(&staging);
        let writer = thread::spawn(move || {
            let mut sink = ExtractSink::new(shared);
            sink.begin(PASS);
            sink.write(&[row(1)]);
            sink.end();
        });
        let first = seal_when_closed(&staging);
        let first_read = notes(&staging, &first);
        staging.open(Use { region: 0, generation: 2 });
        let second = seal_when_closed(&staging);
        let second_read = notes(&staging, &second);
        writer.join().expect("the writer");
        staging.open(Use { region: 0, generation: 3 });
        let third = staging.seal().expect("closed");
        let reads = [&first_read, &second_read];
        let whole = whole(0, &[1]);
        let found = reads.iter().filter(|read| **read == &whole).count();
        let partial = reads.iter().filter(|read| !read.is_empty() && **read != &whole).count();
        let lost_in = [&first, &second, &third].iter().filter(|s| s.counts.lost != 0).count();
        assert_eq!(partial, 0, "{first_read:?} {second_read:?}");
        assert_eq!(found + lost_in, 1, "{first_read:?} {second_read:?} lost {lost_in}");
        assert!(notes(&staging, &third).is_empty());
    });
}

#[test]
fn a_sink_dropped_with_a_pass_open_leaves_the_region_sealable_and_the_pass_lost() {
    model(|| {
        let staging = ring(8);
        let shared = Arc::clone(&staging);
        let writer = thread::spawn(move || {
            let mut sink = ExtractSink::new(shared);
            sink.begin(PASS);
            sink.write(&[row(1)]);
            drop(sink);
        });
        let sealed = seal_when_closed(&staging);
        let read = notes(&staging, &sealed);
        writer.join().expect("the writer");
        staging.open(Use { region: 1, generation: 2 });
        let next = staging.seal().expect("closed");
        if read.is_empty() {
            // Sealed before the begin: lost in the renderer-wide record, reported once.
            assert!(reported_once(KIND, &[&sealed, &next]));
        } else {
            // The begin took region zero; the sealing after the drop reports the loss.
            assert_eq!(read[0].1, Note::Begin { kind: KIND, tick: PASS.tick, full: PASS.full });
            assert!(!read.contains(&(0, Note::End)));
            assert_eq!(sealed.counts.lost, lost(KIND));
            assert_eq!(next.counts.lost, 0);
        }
    });
}
