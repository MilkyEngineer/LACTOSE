//! The request spans (`README.md`, "Tracing"; "Proof for milestone 1", the tracing row): a
//! capturing subscriber sees one span per polled request with the names and fields of the
//! table, `io.open` carrying `file` from its creation and `size` on success, `error`
//! recorded on a failure and absent on success, no span for a future dropped unpolled, and
//! no event. One test, so the global subscriber sees this binary's requests only.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use lactose_core::ErrorKind;
use lactose_io::Encoding;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

use common::{LIMIT, Scratch, frame, start, text, wait};

/// What the subscriber saw of one span.
#[derive(Clone, Debug, Default)]
struct Seen {
    name: &'static str,
    target: String,
    level: String,
    /// The fields given a value at creation.
    created: BTreeMap<String, String>,
    /// The fields recorded later.
    recorded: BTreeMap<String, String>,
    entered: usize,
    /// Handles to the span still alive; zero once it closed.
    references: usize,
}

#[derive(Default)]
struct Capture {
    spans: Mutex<Vec<Seen>>,
    events: AtomicU64,
}

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

impl Capture {
    fn spans(&self) -> MutexGuard<'_, Vec<Seen>> {
        self.spans.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn index(id: &Id) -> usize {
        usize::try_from(id.into_u64() - 1).unwrap()
    }
}

impl Subscriber for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let metadata = attributes.metadata();
        let mut seen = Seen {
            name: metadata.name(),
            target: metadata.target().to_owned(),
            level: metadata.level().to_string(),
            references: 1,
            ..Seen::default()
        };
        attributes.record(&mut Fields(&mut seen.created));
        let mut spans = self.spans();
        spans.push(seen);
        Id::from_u64(spans.len() as u64)
    }

    fn record(&self, span: &Id, values: &Record<'_>) {
        values.record(&mut Fields(&mut self.spans()[Self::index(span)].recorded));
    }

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {
        self.events.fetch_add(1, Ordering::Relaxed);
    }

    fn enter(&self, span: &Id) {
        self.spans()[Self::index(span)].entered += 1;
    }

    fn exit(&self, _: &Id) {}

    fn clone_span(&self, span: &Id) -> Id {
        self.spans()[Self::index(span)].references += 1;
        span.clone()
    }

    fn try_close(&self, span: Id) -> bool {
        let mut spans = self.spans();
        let seen = &mut spans[Self::index(&span)];
        seen.references -= 1;
        seen.references == 0
    }
}

fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Arc<Capture>> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Arc::new(Capture::default());
        tracing::subscriber::set_global_default(Arc::clone(&capture)).unwrap();
        capture
    })
}

fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|&(key, value)| (key.to_owned(), value.to_owned())).collect()
}

/// The spans created since `from`, once every one of them has closed: the pool thread may
/// release a request's state just after the future that took its result.
fn closed_since(from: usize) -> Vec<Seen> {
    let deadline = Instant::now() + LIMIT;
    loop {
        let spans = capture().spans()[from..].to_vec();
        if spans.iter().all(|seen| seen.references == 0) {
            return spans;
        }
        assert!(Instant::now() < deadline, "a span never closed: {spans:#?}");
        std::thread::yield_now();
    }
}

#[test]
fn one_span_per_polled_request_with_the_fields_of_the_table() {
    let capture = capture();
    let scratch = Scratch::new();
    let content = text(10_000, 1);
    let stored = frame(&content);
    std::fs::write(scratch.path("pack"), &stored).unwrap();
    let pack = scratch.path("pack").display().to_string();
    let io = start(1);

    // Unpolled: nothing.
    drop(io.open(scratch.path("pack")));
    drop(io.read_file(scratch.path("pack")));
    drop(io.replace(scratch.path("slot"), b"x".to_vec()));
    drop(io.read_dir(&scratch.root));
    drop(io.create_dir_all(scratch.path("dir")));
    assert_eq!(capture.spans().len(), 0);

    // `io.open`: `file` from creation, `size` on success. The open dropped above took id 0.
    let file = wait(io.open(scratch.path("pack"))).unwrap();
    assert_eq!(file.id().to_string(), "1");
    let spans = closed_since(0);
    assert_eq!(spans.len(), 1);
    let open = &spans[0];
    assert_eq!((open.name, open.target.as_str(), open.level.as_str()), ("io.open", "io", "INFO"));
    assert_eq!(open.created, fields(&[("path", &pack), ("file", "1")]));
    assert_eq!(open.recorded, fields(&[("size", &stored.len().to_string())]));
    assert!(open.entered >= 1);

    // `io.read`, raw and encoded; the encoded one is entered by its decode slices too.
    let len = stored.len().to_string();
    let decoded = content.len().to_string();
    wait(file.read_at(0, 16)).unwrap();
    let encoded = Encoding::Zstd { decoded_len: content.len() as u64 };
    assert_eq!(wait(file.read_encoded_at(0, stored.len() as u64, encoded)).unwrap(), content);
    let spans = closed_since(1);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].name, "io.read");
    let raw = [("file", "1"), ("offset", "0"), ("len", "16"), ("encoding", "raw")];
    let mut raw = fields(&raw);
    raw.insert("decoded_len".to_owned(), "16".to_owned());
    assert_eq!(spans[0].created, raw);
    assert_eq!(spans[0].recorded, BTreeMap::new());
    assert_eq!(spans[1].name, "io.read");
    let zstd = [("file", "1"), ("offset", "0"), ("len", &len), ("encoding", "zstd")];
    let mut zstd = fields(&zstd);
    zstd.insert("decoded_len".to_owned(), decoded.clone());
    assert_eq!(spans[1].created, zstd);
    assert_eq!(spans[1].recorded, BTreeMap::new());
    assert!(spans[1].entered >= 2, "entered {} times", spans[1].entered);

    // `io.read_file`, `io.replace`, `io.read_dir`, `io.create_dir_all` on success.
    wait(io.read_file(scratch.path("pack"))).unwrap();
    wait(io.replace(scratch.path("slot"), b"four".to_vec())).unwrap();
    wait(io.read_dir(&scratch.root)).unwrap();
    wait(io.create_dir_all(scratch.path("dir"))).unwrap();
    let spans = closed_since(3);
    let names: Vec<&str> = spans.iter().map(|seen| seen.name).collect();
    assert_eq!(names, ["io.read_file", "io.replace", "io.read_dir", "io.create_dir_all"]);
    assert_eq!(spans[0].created, fields(&[("path", &pack)]));
    assert_eq!(spans[0].recorded, fields(&[("len", &len)]));
    let slot = scratch.path("slot").display().to_string();
    assert_eq!(spans[1].created, fields(&[("path", &slot), ("len", "4")]));
    assert_eq!(spans[1].recorded, BTreeMap::new());
    let root = scratch.root.display().to_string();
    assert_eq!(spans[2].created, fields(&[("path", &root)]));
    assert_eq!(spans[2].recorded, fields(&[("entries", "2")]));
    let dir = scratch.path("dir").display().to_string();
    assert_eq!(spans[3].created, fields(&[("path", &dir)]));
    assert_eq!(spans[3].recorded, BTreeMap::new());
    assert!(spans.iter().all(|seen| seen.entered >= 1));

    // `error` on a failure, with the kind, and nothing else recorded.
    let missing = scratch.path("missing");
    assert_eq!(wait(io.open(&missing)).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(wait(file.read_at(1 << 20, 1)).unwrap_err().kind(), ErrorKind::InvalidData);
    let failed = wait(io.replace(missing.join("slot"), b"x".to_vec())).unwrap_err();
    assert_eq!(failed.error.kind(), ErrorKind::NotFound);
    let spans = closed_since(7);
    let names: Vec<&str> = spans.iter().map(|seen| seen.name).collect();
    assert_eq!(names, ["io.open", "io.read", "io.replace"]);
    assert_eq!(spans[0].recorded, fields(&[("error", "NotFound")]));
    assert_eq!(spans[1].recorded, fields(&[("error", "InvalidData")]));
    assert_eq!(spans[2].recorded, fields(&[("error", "NotFound")]));

    // A request refused before submission still has its span, at its first poll.
    let refused = wait(file.read_at(u64::MAX, 2)).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::InvalidData);
    let spans = closed_since(10);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].recorded, fields(&[("error", "InvalidData")]));

    drop(file);
    io.shutdown();
    assert_eq!(capture.events.load(Ordering::Relaxed), 0, "an event was logged");
}
