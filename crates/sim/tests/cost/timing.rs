//! Times the `casein` spans: each sync point, stage and system, summed by name until taken.
//! A module of its own so that `crates/app/tests/cost.rs` times its scenes the same way.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};

/// The time since the first call: the one place the subscriber reads the clock.
#[expect(clippy::disallowed_types, reason = "test code timing the step from outside")]
fn clock() -> Duration {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

struct Open {
    label: String,
    entered: Option<Duration>,
}

#[derive(Default)]
pub struct Spans {
    next: AtomicU64,
    open: Mutex<BTreeMap<u64, Open>>,
    totals: Mutex<BTreeMap<String, Duration>>,
}

impl Spans {
    /// The time spent in each span since the last take, by label.
    pub fn take(&self) -> BTreeMap<String, Duration> {
        core::mem::take(&mut *self.totals.lock().unwrap())
    }
}

struct Name(String);

impl Visit for Name {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "name" || field.name() == "stage" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        if field.name() == "name" || field.name() == "stage" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Subscriber for &'static Spans {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) { Interest::always() } else { Interest::never() }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_span() && metadata.target() == "casein"
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut name = Name(String::new());
        span.record(&mut name);
        let label = format!("{} {}", span.metadata().name(), name.0);
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.open.lock().unwrap().insert(id, Open { label, entered: None });
        Id::from_u64(id)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, span: &Id) {
        if let Some(open) = self.open.lock().unwrap().get_mut(&span.into_u64()) {
            open.entered = Some(clock());
        }
    }

    fn exit(&self, span: &Id) {
        let now = clock();
        let Some(open) = self.open.lock().unwrap().remove(&span.into_u64()) else { return };
        if let Some(entered) = open.entered {
            *self.totals.lock().unwrap().entry(open.label).or_default() += now - entered;
        }
    }
}
