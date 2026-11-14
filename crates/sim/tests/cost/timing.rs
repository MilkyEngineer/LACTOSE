//! Times a step from its spans, summed by label until taken: each `casein` sync point and
//! each system. A system's time is its graph node's: the executor's span of every range it
//! ran (target `whisk`, named after the node), summed over the ranges, so on several workers
//! it is the system's work, not its wall time. A node's last range holds its `finalize` and
//! the `units` of the nodes it releases, so per-system figures overlap by them. An exclusive
//! system runs inside a sync point's node, so its time is its `casein` "system" span. A span
//! entered inside one of the same label, as a system's `casein` span is inside its node's
//! range, is not counted again.
//! A module of its own so that `crates/app/tests/cost.rs` times its scenes the same way.
//!
//! Cheap per span: no allocation once a label has been seen, one lock where a span is
//! created and one where it closes; the label's text is built only by `take`.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};
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

/// The modules of the executor's node and range spans; its other spans are not a node's.
const NODE_SPANS: [&str; 2] = ["lactose_task::node", "lactose_task::range"];

/// What a span's time is summed under, printed as `<kind> <name>`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Label {
    kind: &'static str,
    name: &'static str,
}

#[derive(Default)]
struct State {
    /// Every label seen, by index; a span's id is its label's index plus one.
    labels: Vec<Label>,
    index: BTreeMap<Label, usize>,
    /// Field values interned once, so a label never owns a string.
    names: BTreeSet<&'static str>,
    /// The time of each label since the last take, by index.
    totals: Vec<Option<Duration>>,
}

impl State {
    fn intern(&mut self, name: &str) -> &'static str {
        if let Some(&interned) = self.names.get(name) {
            return interned;
        }
        let interned: &'static str = Box::leak(name.to_owned().into_boxed_str());
        self.names.insert(interned);
        interned
    }

    fn label(&mut self, label: Label) -> usize {
        if let Some(&index) = self.index.get(&label) {
            return index;
        }
        self.labels.push(label);
        self.totals.push(None);
        self.index.insert(label, self.labels.len() - 1);
        self.labels.len() - 1
    }
}

#[derive(Default)]
pub struct Spans {
    state: Mutex<State>,
}

impl Spans {
    /// The time spent under each label since the last take.
    pub fn take(&self) -> BTreeMap<String, Duration> {
        let mut state = self.state.lock().unwrap();
        let State { labels, totals, .. } = &mut *state;
        labels
            .iter()
            .zip(totals.iter_mut())
            .filter_map(|(label, total)| {
                Some((format!("{} {}", label.kind, label.name), total.take()?))
            })
            .collect()
    }
}

thread_local! {
    /// The spans this thread is inside, innermost last (a span is entered and exited on one
    /// thread): each one's label, and when it was entered, or `None` when it is inside a span
    /// of its own label and is not counted.
    #[expect(clippy::disallowed_types, reason = "test code timing the step from outside")]
    static ENTERED: RefCell<Vec<(usize, Option<Duration>)>> = const { RefCell::new(Vec::new()) };
}

/// This thread's entered spans.
fn entered<R>(f: impl FnOnce(&mut Vec<(usize, Option<Duration>)>) -> R) -> R {
    ENTERED.with_borrow_mut(f)
}

/// The `casein` span's `name` or `stage` field, interned.
struct Name<'a> {
    state: &'a mut State,
    name: &'static str,
}

impl Visit for Name<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "name" || field.name() == "stage" {
            self.name = self.state.intern(value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        if field.name() == "name" || field.name() == "stage" {
            self.name = self.state.intern(&format!("{value:?}"));
        }
    }
}

impl Subscriber for &'static Spans {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) { Interest::always() } else { Interest::never() }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_span()
            && match metadata.target() {
                "casein" => true,
                "whisk" => metadata.module_path().is_some_and(|path| NODE_SPANS.contains(&path)),
                _ => false,
            }
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let metadata = span.metadata();
        let mut state = self.state.lock().unwrap();
        let label = if metadata.target() == "whisk" {
            // The sync points' and the events' nodes are `casein::*`; every other node of a
            // step is a system's.
            let name = metadata.name();
            let kind = if name.starts_with("casein::") { "node" } else { "system" };
            Label { kind, name }
        } else {
            let mut name = Name { state: &mut state, name: "" };
            span.record(&mut name);
            let name = name.name;
            Label { kind: metadata.name(), name }
        };
        let index = state.label(label);
        Id::from_u64(index as u64 + 1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, span: &Id) {
        let index = (span.into_u64() - 1) as usize;
        entered(|entered| {
            let nested = entered.iter().any(|&(outer, _)| outer == index);
            entered.push((index, (!nested).then(clock)));
        });
    }

    fn exit(&self, span: &Id) {
        let now = clock();
        let index = (span.into_u64() - 1) as usize;
        let Some(entered) = entered(|entered| {
            let at = entered.iter().rposition(|&(open, _)| open == index)?;
            entered.remove(at).1
        }) else {
            return;
        };
        let mut state = self.state.lock().unwrap();
        *state.totals[index].get_or_insert_default() += now - entered;
    }
}
