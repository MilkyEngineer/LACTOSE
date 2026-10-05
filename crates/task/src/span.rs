//! Tracing spans named at run time.
//!
//! `tracing`'s macros fix a span's name at the call site. Graph nodes are named by
//! [`crate::GraphNode::name`], so each distinct name gets its own pair of call sites, built
//! once and kept for the life of the process, the way a macro's static call site is. A
//! profiler then shows one zone name per system instead of one shared "node" zone.

use core::sync::atomic::{AtomicU8, Ordering};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tracing::callsite::{Callsite, Identifier};
use tracing::field::FieldSet;
use tracing::metadata::Kind;
use tracing::subscriber::Interest;
use tracing::{Level, Metadata, Span};

use crate::lock;

/// The log target of everything this crate emits.
pub(crate) const TARGET: &str = "whisk";

const LEVEL: Level = Level::INFO;

const NEVER: u8 = 0;
const SOMETIMES: u8 = 1;
const ALWAYS: u8 = 2;

/// A span call site whose name is chosen at run time. Does what `tracing`'s
/// `DefaultCallsite` does for a macro: caches the subscribers' interest so a span nobody
/// wants costs one atomic load.
struct NamedCallsite {
    metadata: OnceLock<Metadata<'static>>,
    interest: AtomicU8,
}

impl Callsite for NamedCallsite {
    fn set_interest(&self, interest: Interest) {
        let interest = if interest.is_never() {
            NEVER
        } else if interest.is_always() {
            ALWAYS
        } else {
            SOMETIMES
        };
        self.interest.store(interest, Ordering::Relaxed);
    }

    fn metadata(&self) -> &Metadata<'_> {
        self.metadata.get().expect("metadata is set before the call site is registered")
    }
}

impl NamedCallsite {
    fn new(name: &'static str, module_path: &'static str) -> &'static Self {
        // Leaked like a macro's static call site: the registry keeps a reference forever.
        let callsite: &'static Self = Box::leak(Box::new(Self {
            metadata: OnceLock::new(),
            // Until the registry says otherwise, ask the subscriber every time.
            interest: AtomicU8::new(SOMETIMES),
        }));
        let fields = FieldSet::new(&[], Identifier(callsite));
        let metadata =
            Metadata::new(name, TARGET, LEVEL, None, None, Some(module_path), fields, Kind::SPAN);
        callsite.metadata.set(metadata).expect("set once, here");
        tracing::callsite::register(callsite);
        callsite
    }

    fn span(&'static self) -> Span {
        if LEVEL > tracing::level_filters::STATIC_MAX_LEVEL {
            return Span::none();
        }
        let metadata = self.metadata.get().expect("set in `new`");
        let enabled = match self.interest.load(Ordering::Relaxed) {
            NEVER => false,
            ALWAYS => true,
            _ => tracing::dispatcher::get_default(|dispatch| dispatch.enabled(metadata)),
        };
        if enabled { Span::new(metadata, &metadata.fields().value_set(&[])) } else { Span::none() }
    }
}

/// The call sites of one node name.
pub(crate) struct NodeSpans {
    node: &'static NamedCallsite,
    range: &'static NamedCallsite,
}

impl NodeSpans {
    /// The span of one run of the node: the range it was released as.
    pub(crate) fn node(&self) -> Span {
        self.node.span()
    }

    /// The span of one range split off the node's units.
    pub(crate) fn range(&self) -> Span {
        self.range.span()
    }
}

/// The call sites for nodes called `name`, created on first use. Called when a graph is
/// built, never while one runs.
pub(crate) fn spans_for(name: &'static str) -> &'static NodeSpans {
    static INTERNED: Mutex<Option<HashMap<&'static str, &'static NodeSpans>>> = Mutex::new(None);
    let mut interned = lock(&INTERNED);
    interned.get_or_insert_with(HashMap::new).entry(name).or_insert_with(|| {
        Box::leak(Box::new(NodeSpans {
            node: NamedCallsite::new(name, "lactose_task::node"),
            range: NamedCallsite::new(name, "lactose_task::range"),
        }))
    })
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use core::hint::black_box;
    use std::time::Instant;

    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Subscriber};

    use super::*;

    /// A subscriber that wants every span and does nothing with it.
    struct Trivial;

    impl Subscriber for Trivial {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}
        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
        fn event(&self, _event: &Event<'_>) {}
        fn enter(&self, _span: &Id) {}
        fn exit(&self, _span: &Id) {}
    }

    fn nanos_per_call(mut call: impl FnMut()) -> f64 {
        const CALLS: u32 = 2_000_000;
        let mut best = f64::INFINITY;
        for _ in 0..5 {
            let start = Instant::now();
            for _ in 0..CALLS {
                call();
            }
            best = best.min(start.elapsed().as_nanos() as f64 / f64::from(CALLS));
        }
        best
    }

    #[test]
    fn one_name_gets_one_pair_of_call_sites() {
        let first = spans_for("span::tests::interned");
        let second = spans_for("span::tests::interned");
        assert!(core::ptr::eq(first, second));
        assert!(!core::ptr::eq(first, spans_for("span::tests::other")));
        let metadata = first.node.metadata();
        assert_eq!((metadata.name(), metadata.target()), ("span::tests::interned", TARGET));
        assert!(metadata.is_span());
        // Nobody listens in this test binary, so creating a span is a no-op.
        assert!(first.node().is_none());
    }

    /// The verification item of `docs/verification.md`: what one span costs with nobody
    /// listening and with a subscriber that does nothing. A measurement, not a check:
    ///
    /// ```text
    /// cargo test -p lactose-task --release --lib -- --ignored --nocapture span_overhead
    /// ```
    #[test]
    #[ignore = "a measurement that installs a process-wide subscriber; run it alone"]
    fn span_overhead() {
        let spans = spans_for("span::tests::overhead");
        let named = || {
            let span = spans.node();
            let _entered = span.enter();
            black_box(&span);
        };
        let stock = || {
            let span = tracing::info_span!(target: "whisk", "stock");
            let _entered = span.enter();
            black_box(&span);
        };
        let (quiet_named, quiet_stock) = (nanos_per_call(named), nanos_per_call(stock));
        println!("no subscriber:      named {quiet_named:.1} ns, macro {quiet_stock:.1} ns");
        tracing::subscriber::set_global_default(Trivial).expect("no subscriber is installed yet");
        let (heard_named, heard_stock) = (nanos_per_call(named), nanos_per_call(stock));
        println!("trivial subscriber: named {heard_named:.1} ns, macro {heard_stock:.1} ns");
    }
}
