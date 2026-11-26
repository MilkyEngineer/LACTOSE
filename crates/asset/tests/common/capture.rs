//! A capturing `tracing` subscriber, hand-written as `lactose-io`'s and `lactose-task`'s
//! span tests write theirs: every span with its fields, parent, entries and closing, and
//! every event with its level, fields and parent. Installed as the process's global
//! subscriber, which suits nextest's one process per test.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

use super::LIMIT;

/// What the subscriber saw of one span.
#[derive(Clone, Debug, Default)]
pub struct Span {
    pub name: &'static str,
    pub target: String,
    pub level: String,
    /// The index of the parent span, `None` for a root.
    pub parent: Option<usize>,
    /// The fields given a value at creation.
    pub created: BTreeMap<String, String>,
    /// The fields recorded later.
    pub recorded: BTreeMap<String, String>,
    pub entered: usize,
    /// The names of the threads that entered it.
    pub threads: std::collections::BTreeSet<String>,
    /// Handles to the span still alive; zero once it closed.
    pub references: usize,
}

impl Span {
    /// A field's value, whether given at creation or recorded later.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.recorded.get(name).or_else(|| self.created.get(name)).map(String::as_str)
    }
}

/// What the subscriber saw of one event.
#[derive(Clone, Debug)]
pub struct Event_ {
    pub name: &'static str,
    pub target: String,
    pub level: Level,
    pub parent: Option<usize>,
    pub fields: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct Capture {
    spans: Mutex<Vec<Span>>,
    events: Mutex<Vec<Event_>>,
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

thread_local! {
    /// The spans this thread is inside, innermost last.
    static ENTERED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn index(id: &Id) -> usize {
    usize::try_from(id.into_u64() - 1).expect("a span index")
}

fn current() -> Option<usize> {
    ENTERED.with(|entered| entered.borrow().last().copied())
}

impl Capture {
    pub fn spans(&self) -> MutexGuard<'_, Vec<Span>> {
        self.spans.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn events(&self) -> MutexGuard<'_, Vec<Event_>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The spans named `name` from index `from` on, once each has closed.
    pub fn closed(&self, from: usize, name: &str) -> Vec<(usize, Span)> {
        let deadline = Instant::now() + LIMIT;
        loop {
            let spans: Vec<(usize, Span)> = self.spans()[from..]
                .iter()
                .enumerate()
                .filter(|(_, span)| span.name == name)
                .map(|(at, span)| (from + at, span.clone()))
                .collect();
            if spans.iter().all(|(_, span)| span.references == 0) {
                return spans;
            }
            assert!(Instant::now() < deadline, "a span never closed: {spans:#?}");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// The spans named `name` from index `from` on, open or not.
    pub fn named(&self, from: usize, name: &str) -> Vec<(usize, Span)> {
        let spans = self.spans();
        let found = spans[from..].iter().enumerate().filter(|(_, span)| span.name == name);
        found.map(|(at, span)| (from + at, span.clone())).collect()
    }

    pub fn span_count(&self) -> usize {
        self.spans().len()
    }

    pub fn event_count(&self) -> usize {
        self.events().len()
    }

    /// The events from index `from` on at `level` and target `silo`, once every `silo.load`
    /// span has closed. A step's events reach the subscriber after it released the store's
    /// lock, so a test that saw a load end can be ahead of them; the load's span closes
    /// only after them.
    pub fn store_events(&self, from: usize, level: Level) -> Vec<Event_> {
        self.closed(0, "silo.load");
        let events = self.events();
        let found = events[from..].iter().filter(|event| event.level == level);
        found.filter(|event| event.target == "silo").cloned().collect()
    }
}

impl Subscriber for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        // The executor's debug spans run on parking workers, where a callback must not block
        // (`lactose-task`'s README, "Tracing"); they are not the store's either.
        metadata.target() != "whisk" || *metadata.level() <= Level::INFO
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let metadata = attributes.metadata();
        let parent = if attributes.is_root() {
            None
        } else if let Some(parent) = attributes.parent() {
            Some(index(parent))
        } else {
            current()
        };
        let mut span = Span {
            name: metadata.name(),
            target: metadata.target().to_owned(),
            level: metadata.level().to_string(),
            parent,
            references: 1,
            ..Span::default()
        };
        attributes.record(&mut Fields(&mut span.created));
        let mut spans = self.spans();
        spans.push(span);
        Id::from_u64(spans.len() as u64)
    }

    fn record(&self, span: &Id, values: &Record<'_>) {
        values.record(&mut Fields(&mut self.spans()[index(span)].recorded));
    }

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        let parent = if event.is_root() {
            None
        } else if let Some(parent) = event.parent() {
            Some(index(parent))
        } else {
            current()
        };
        let mut fields = BTreeMap::new();
        event.record(&mut Fields(&mut fields));
        self.events().push(Event_ {
            name: metadata.name(),
            target: metadata.target().to_owned(),
            level: *metadata.level(),
            parent,
            fields,
        });
    }

    fn enter(&self, span: &Id) {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        let mut spans = self.spans();
        let seen = &mut spans[index(span)];
        seen.entered += 1;
        seen.threads.insert(thread);
        drop(spans);
        ENTERED.with(|entered| entered.borrow_mut().push(index(span)));
    }

    fn exit(&self, span: &Id) {
        let at = index(span);
        ENTERED.with(|entered| {
            let mut entered = entered.borrow_mut();
            if let Some(position) = entered.iter().rposition(|&open| open == at) {
                entered.remove(position);
            }
        });
    }

    fn clone_span(&self, span: &Id) -> Id {
        self.spans()[index(span)].references += 1;
        span.clone()
    }

    fn try_close(&self, span: Id) -> bool {
        let mut spans = self.spans();
        let seen = &mut spans[index(&span)];
        seen.references -= 1;
        seen.references == 0
    }
}

/// The process's capturing subscriber, installed on first use.
pub fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Arc<Capture>> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Arc::new(Capture::default());
        tracing::subscriber::set_global_default(Arc::clone(&capture))
            .expect("the first subscriber of this process");
        capture
    })
}
