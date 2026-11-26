//! Tracing, target `whisk`. At info: one span per node run and one per range split off it,
//! named by the node, and never one per unit. At debug: `open_gate` around a
//! `block_in_place`'s opening of the gate, and `park` around each of the two waits a worker
//! makes without a permit, which `lactose-app-client`'s `record-stall` reads by name.
//!
//! One test only: it installs a process-wide subscriber.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{LIMIT, Node, executor, graph, let_workers_sleep, sized, wait_for};
use tracing::callsite::Identifier;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};

/// A span the recorder made.
struct Made {
    name: String,
    module: String,
    level: Level,
    /// The `wait` field, which `park` carries.
    wait: Option<String>,
    callsite: Identifier,
    entered: u32,
    exited: u32,
}

#[derive(Default)]
struct Log {
    /// Indexed by span id less one.
    made: Vec<Made>,
    /// The call sites of debug spans the recorder was asked about and declined.
    declined: HashSet<Identifier>,
}

/// Listens to the `whisk` target at info, and at debug too while `debug` is set. It asks to
/// be asked at every span, so that the level can change while executors run, and it notes
/// each debug call site it declines, which shows that a site was reached at info.
#[derive(Clone, Default)]
struct Recorder {
    log: Arc<Mutex<Log>>,
    debug: Arc<AtomicBool>,
}

impl Recorder {
    fn at_debug(&self, debug: bool) {
        self.debug.store(debug, Ordering::SeqCst);
        *self.log.lock().unwrap() = Log::default();
    }

    /// How many spans named `name` with module path `module` were made.
    fn count(&self, name: &str, module: &str) -> usize {
        let log = self.log.lock().unwrap();
        log.made.iter().filter(|made| made.name == name && made.module == module).count()
    }

    /// The call site of a `park` span made with `wait`, once one has been made.
    fn park_site(&self, wait: &str) -> Option<Identifier> {
        let log = self.log.lock().unwrap();
        let made =
            log.made.iter().find(|made| made.name == "park" && made.wait.as_deref() == Some(wait));
        made.map(|made| made.callsite.clone())
    }

    fn declined(&self, site: &Identifier) -> bool {
        self.log.lock().unwrap().declined.contains(site)
    }
}

/// Reads a span's `wait` field.
struct Wait(Option<String>);

impl Visit for Wait {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "wait" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        if field.name() == "wait" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl Subscriber for Recorder {
    fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        if metadata.target() != "whisk" {
            return false;
        }
        if *metadata.level() <= Level::INFO || self.debug.load(Ordering::SeqCst) {
            return true;
        }
        self.log.lock().unwrap().declined.insert(metadata.callsite());
        false
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let metadata = span.metadata();
        let mut wait = Wait(None);
        span.record(&mut wait);
        let mut log = self.log.lock().unwrap();
        log.made.push(Made {
            name: metadata.name().to_owned(),
            module: metadata.module_path().unwrap_or_default().to_owned(),
            level: *metadata.level(),
            wait: wait.0,
            callsite: metadata.callsite(),
            entered: 0,
            exited: 0,
        });
        // Ids index the log, which `at_debug` replaces: by then every executor that made a
        // span of the log before has shut down, and its spans have closed.
        Id::from_u64(log.made.len() as u64)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, span: &Id) {
        let mut log = self.log.lock().unwrap();
        if let Some(made) = log.made.get_mut(span.into_u64() as usize - 1) {
            made.entered += 1;
        }
    }

    fn exit(&self, span: &Id) {
        let mut log = self.log.lock().unwrap();
        if let Some(made) = log.made.get_mut(span.into_u64() as usize - 1) {
            made.exited += 1;
        }
    }
}

/// Polls `done` until it holds or [`LIMIT`] has passed; returns whether it held.
fn eventually(done: impl Fn() -> bool) -> bool {
    let begun = Instant::now();
    while !done() {
        if begun.elapsed() >= LIMIT {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    true
}

type Until = Arc<dyn Fn() -> bool + Send + Sync>;

/// Runs one `block_in_place` and then a scope whose opener parks on its latch: its first
/// child, which a thief takes, holds on until `latch_parked` holds, or for [`LIMIT`], and the
/// second keeps the opener busy until the first is with the thief. Then waits until
/// `idle_parked` holds, which workers parking in the idle protocol after the run make
/// true, or for [`LIMIT`].
fn open_and_park(latch_parked: Until, idle_parked: Until) {
    let executor = executor(2, 1);
    let_workers_sleep();
    let gate = Node::new("spans::gate").run(|_, ctx| ctx.block_in_place(0, || ())).build();
    let stolen = Arc::new(AtomicBool::new(false));
    let latch = {
        let stolen = stolen.clone();
        Node::new("spans::latch")
            .run(move |_, ctx| {
                let (stolen, latch_parked) = (&stolen, &latch_parked);
                ctx.scope(|scope| {
                    scope.fork(move |_| {
                        stolen.store(true, Ordering::Release);
                        eventually(|| latch_parked());
                    });
                    scope.fork(move |_| wait_for(stolen));
                });
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![gate, latch], &[(0, 1)]))).unwrap();
    eventually(|| idle_parked());
    executor.shutdown();
}

#[test]
fn whisk_spans_by_level() {
    let recorder = Recorder::default();
    tracing::subscriber::set_global_default(recorder.clone()).unwrap();

    // At debug: the opening, and the park at each of the two waits, every park entered
    // once and exited once, all of them closed by the time the executor has shut down.
    recorder.at_debug(true);
    let latch_parked: Until = {
        let recorder = recorder.clone();
        Arc::new(move || recorder.park_site("latch").is_some())
    };
    let idle_parked: Until = {
        let recorder = recorder.clone();
        Arc::new(move || recorder.park_site("idle").is_some())
    };
    open_and_park(latch_parked, idle_parked);
    let openings = recorder.count("open_gate", "lactose_task::job");
    assert_eq!(openings, 1, "one `open_gate` span for the one `block_in_place`");
    let sites = {
        let log = recorder.log.lock().unwrap();
        let debug: Vec<&Made> =
            log.made.iter().filter(|made| !made.name.starts_with("spans::")).collect();
        for made in &debug {
            assert_eq!(made.level, Level::DEBUG, "{} is a debug span", made.name);
            assert_eq!(
                (made.entered, made.exited),
                (1, 1),
                "{} entered {} and exited {}",
                made.name,
                made.entered,
                made.exited
            );
            match made.name.as_str() {
                "open_gate" => assert_eq!(made.wait, None),
                "park" => {
                    assert_eq!(made.module, "lactose_task::gate");
                    let wait = made.wait.as_deref();
                    assert!(matches!(wait, Some("idle" | "latch")), "a park's wait {wait:?}");
                }
                other => panic!("a debug span named {other}"),
            }
        }
        let site = |name: &str, wait: Option<&str>| {
            debug
                .iter()
                .find(|made| made.name == name && made.wait.as_deref() == wait)
                .map(|made| made.callsite.clone())
        };
        [site("open_gate", None), site("park", Some("idle")), site("park", Some("latch"))]
    };
    let [Some(opening), Some(idle), Some(latch)] = sites.clone() else {
        panic!("at debug: `open_gate`, `park` idle and `park` latch: {sites:?}");
    };
    assert!(idle != latch, "the two waits are two call sites");

    // At info: none of them, though each site is reached and asks.
    recorder.at_debug(false);
    let units = sized(4096, 64);
    let runs = 3;

    // One worker makes the count exact. The worker halves what it holds each time its
    // deque runs dry: 4096 units leave the deque as ranges of 2048, 1024, ... 1, so a run
    // is one node span and log2(units) range spans, however many units there are.
    let single = executor(1, 1);
    let node = Node::new("spans::single").units(units).build();
    let mut graph_single = graph(vec![node], &[]);
    for _ in 0..runs {
        graph_single = single.block_on(single.run_graph(graph_single)).unwrap();
    }
    single.shutdown();
    assert_eq!(recorder.count("spans::single", "lactose_task::node"), runs);
    assert_eq!(
        recorder.count("spans::single", "lactose_task::range"),
        runs * units.ilog2() as usize
    );

    // With thieves the count depends on how often they come: one range span per steal or
    // dry deque, and never more than one per unit beyond the first, which is the bound
    // when every unit is stolen singly.
    let workers = sized(4, 2);
    let several = executor(workers, 1);
    let node = |name| Node::new(name).units(units).run(|_, _| std::thread::yield_now()).build();
    let mut graph_several = graph(vec![node("spans::wide"), node("spans::other")], &[(0, 1)]);
    for _ in 0..runs {
        graph_several = several.block_on(several.run_graph(graph_several)).unwrap();
    }
    several.shutdown();
    for name in ["spans::wide", "spans::other"] {
        assert_eq!(recorder.count(name, "lactose_task::node"), runs);
        let ranges = recorder.count(name, "lactose_task::range");
        assert!(ranges >= runs, "{name}: a range was never split across {workers} workers");
        assert!(ranges <= runs * (units as usize - 1), "{name}: {ranges} range spans");
    }

    let latch_asked: Until = {
        let (recorder, latch) = (recorder.clone(), latch.clone());
        Arc::new(move || recorder.declined(&latch))
    };
    let idle_asked: Until = {
        let (recorder, idle) = (recorder.clone(), idle.clone());
        Arc::new(move || recorder.declined(&idle))
    };
    open_and_park(latch_asked, idle_asked);
    for (site, what) in [(&opening, "open_gate"), (&idle, "park idle"), (&latch, "park latch")] {
        assert!(recorder.declined(site), "at info the {what} site was never reached");
    }
    let log = recorder.log.lock().unwrap();
    assert!(
        log.made.iter().all(|made| made.name.starts_with("spans::")),
        "at info: {:?}",
        log.made.iter().map(|made| &made.name).collect::<Vec<_>>()
    );
}
