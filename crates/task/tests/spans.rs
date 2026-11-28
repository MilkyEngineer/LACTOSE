//! Tracing, target `whisk`. At info: one span per node run and one per range split off it,
//! named by the node, and never one per unit. At debug: `open_gate` around a
//! `block_in_place`'s opening of the gate, and `park` around each of the two waits a worker
//! makes without a permit, which `lactose-app-client`'s `record-stall` reads by name and by
//! where they begin and end.
//!
//! The process has one subscriber, which forwards to the listener the running test installs
//! ([`listen`]); the tests take turns.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};
use std::thread::Thread;
use std::time::{Duration, Instant};

use common::{LIMIT, Node, executor, graph, let_workers_sleep, sized, wait_for};
use tracing::callsite::Identifier;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};

type Listener = Arc<dyn Subscriber + Send + Sync>;

/// The listener of the test that holds [`TURN`].
static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);
/// Held by each test while it runs: `cargo test` and `cargo miri test` run a binary's tests
/// side by side in one process.
static TURN: Mutex<()> = Mutex::new(());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Waits for this test's turn and makes `listener` the one every span goes to until the next
/// test's turn. Each test shuts its executors down before its turn ends, so no span of one
/// test reaches another's listener.
fn listen(listener: Listener) -> MutexGuard<'static, ()> {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| tracing::subscriber::set_global_default(Switch).unwrap());
    let turn = lock(&TURN);
    *lock(&LISTENER) = Some(listener);
    turn
}

/// The process's subscriber.
struct Switch;

impl Switch {
    fn listener() -> Option<Listener> {
        lock(&LISTENER).clone()
    }
}

impl Subscriber for Switch {
    fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
        // Asked at every span, so that each test's listener decides for itself.
        Interest::sometimes()
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        Self::listener().is_some_and(|listener| listener.enabled(metadata))
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        Self::listener().expect("a listener wanted the span").new_span(span)
    }

    fn record(&self, span: &Id, values: &Record<'_>) {
        if let Some(listener) = Self::listener() {
            listener.record(span, values);
        }
    }

    fn record_follows_from(&self, span: &Id, follows: &Id) {
        if let Some(listener) = Self::listener() {
            listener.record_follows_from(span, follows);
        }
    }

    fn event(&self, event: &Event<'_>) {
        if let Some(listener) = Self::listener() {
            listener.event(event);
        }
    }

    fn enter(&self, span: &Id) {
        if let Some(listener) = Self::listener() {
            listener.enter(span);
        }
    }

    fn exit(&self, span: &Id) {
        if let Some(listener) = Self::listener() {
            listener.exit(span);
        }
    }
}

fn thread_name() -> String {
    std::thread::current().name().unwrap_or("unnamed").to_owned()
}

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

/// Listens to the `whisk` target at info, and at debug too while `debug` is set, and notes
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

impl Wait {
    fn of(span: &Attributes<'_>) -> Option<String> {
        let mut wait = Self(None);
        span.record(&mut wait);
        wait.0
    }
}

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
        let wait = Wait::of(span);
        let mut log = self.log.lock().unwrap();
        log.made.push(Made {
            name: metadata.name().to_owned(),
            module: metadata.module_path().unwrap_or_default().to_owned(),
            level: *metadata.level(),
            wait,
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
    let _turn = listen(Arc::new(recorder.clone()));

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

const OPEN_GATE: u64 = 1;
const PARK_IDLE: u64 = 2;
const PARK_LATCH: u64 = 3;

/// How long `open_gate`'s entry holds the opener before it looks at the held worker.
const PAUSE: Duration = Duration::from_millis(20);
/// How long a park must have lasted to count as held, not passing through.
const HELD: Duration = Duration::from_millis(5);
/// How long `open_gate`'s exit waits for the held worker to leave its park.
const LEAVE: Duration = Duration::from_secs(10);

/// Each thread's parks of one kind, by thread name: when each began and ended.
type Parks = HashMap<String, Vec<(Instant, Option<Instant>)>>;

#[derive(Default)]
struct Places {
    idle: Parks,
    latch: Parks,
    /// The idle park, by thread and start, that the gate's closure found a frame worker held
    /// in beside work.
    held: Option<(String, Instant)>,
    /// At each entry of `open_gate`: whether the gate's closure had returned, and whether
    /// `held` was still in progress after the [`PAUSE`].
    entries: Vec<(bool, bool)>,
    /// At each exit of `open_gate`: whether `held` ended within [`LEAVE`].
    exits: Vec<bool>,
}

impl Places {
    fn in_progress(parks: &Parks, (name, began): &(String, Instant)) -> bool {
        parks.get(name).is_some_and(|parks| parks.contains(&(*began, None)))
    }
}

/// Notes when each `park` begins and ends, and checks at `open_gate`'s entry and exit
/// where the gate's opening stands.
#[derive(Default)]
struct Placement {
    places: Mutex<Places>,
    /// Set by the gate's closure as its last act.
    returned: AtomicBool,
}

impl Placement {
    /// A frame worker other than `me` whose last idle park began after `since` and has
    /// lasted [`HELD`], still in progress.
    fn held_beside_work(&self, me: &str, since: Instant) -> Option<(String, Instant)> {
        let places = lock(&self.places);
        places.idle.iter().find_map(|(name, parks)| {
            let &(began, ended) = parks.last()?;
            let held = name != me
                && name.starts_with("whisk-frame-")
                && began > since
                && ended.is_none()
                && began.elapsed() >= HELD;
            held.then(|| (name.clone(), began))
        })
    }

    /// Whether a thread other than `me` has been inside a latch park for [`HELD`].
    fn latch_held(&self, me: &str) -> bool {
        let places = lock(&self.places);
        places.latch.iter().any(|(name, parks)| {
            name != me
                && parks
                    .last()
                    .is_some_and(|&(began, ended)| ended.is_none() && began.elapsed() >= HELD)
        })
    }
}

impl Subscriber for Placement {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "whisk"
            && *metadata.level() == Level::DEBUG
            && matches!(metadata.name(), "open_gate" | "park")
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        if span.metadata().name() == "open_gate" {
            return Id::from_u64(OPEN_GATE);
        }
        match Wait::of(span).as_deref() {
            Some("idle") => Id::from_u64(PARK_IDLE),
            Some("latch") => Id::from_u64(PARK_LATCH),
            other => panic!("a `park` span with wait {other:?}"),
        }
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, span: &Id) {
        let now = Instant::now();
        match span.into_u64() {
            OPEN_GATE => {
                let returned = self.returned.load(Ordering::SeqCst);
                std::thread::sleep(PAUSE);
                let mut places = lock(&self.places);
                let held = places
                    .held
                    .as_ref()
                    .is_some_and(|held| Places::in_progress(&places.idle, held));
                places.entries.push((returned, held));
            }
            PARK_IDLE => {
                lock(&self.places).idle.entry(thread_name()).or_default().push((now, None))
            }
            PARK_LATCH => {
                lock(&self.places).latch.entry(thread_name()).or_default().push((now, None));
            }
            _ => unreachable!(),
        }
    }

    fn exit(&self, span: &Id) {
        let now = Instant::now();
        if span.into_u64() == OPEN_GATE {
            let begun = Instant::now();
            let left = loop {
                let places = lock(&self.places);
                let Some(held) = &places.held else { break false };
                if !Places::in_progress(&places.idle, held) {
                    break true;
                }
                drop(places);
                if begun.elapsed() >= LEAVE {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(1));
            };
            lock(&self.places).exits.push(left);
            return;
        }
        let mut places = lock(&self.places);
        let parks = if span.into_u64() == PARK_IDLE { &mut places.idle } else { &mut places.latch };
        if let Some(park) = parks.get_mut(&thread_name()).and_then(|parks| parks.last_mut()) {
            park.1 = Some(now);
        }
    }
}

/// Where the spans begin and end against what they bracket. A frame worker held at a closed
/// gate beside work parks inside an idle `park` span; `open_gate` is entered after the
/// gate's closure has returned and while that worker is still held, and exited once the gate
/// has let it go.
#[test]
#[cfg_attr(miri, ignore = "holds a worker parked for real milliseconds")]
fn open_gate_brackets_the_opening_and_a_held_worker_parks_in_a_span() {
    let placement = Arc::new(Placement::default());
    let _turn = listen(placement.clone());
    let executor = executor(2, 1);
    let_workers_sleep();
    let node = {
        let (executor, placement) = (executor.clone(), placement.clone());
        Node::new("spans::held")
            .run(move |_, ctx| {
                let me = thread_name();
                ctx.block_in_place(0, || {
                    // Queued at a closed gate with no worker reserved: the other frame worker
                    // sees it, can take no permit, and parks beside it.
                    let job = executor.run_job(|_| ());
                    let queued = Instant::now();
                    let begun = Instant::now();
                    let mut held = None;
                    while held.is_none() && begun.elapsed() < LIMIT {
                        held = placement.held_beside_work(&me, queued);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    lock(&placement.places).held = held;
                    drop(job);
                    placement.returned.store(true, Ordering::SeqCst);
                });
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    executor.shutdown();

    let places = lock(&placement.places);
    let held = places.held.as_ref().unwrap_or_else(|| {
        panic!(
            "no frame worker was inside an idle `park` span for {HELD:?} beside the job queued \
             at the closed gate: a park whose look saw work has no span, or the span does not \
             last as long as the park"
        )
    });
    let [(returned, still_held)] = places.entries[..] else {
        panic!("one entry of `open_gate` for one `block_in_place`: {:?}", places.entries);
    };
    assert!(returned, "`open_gate` was entered before the gate's closure returned");
    assert!(
        still_held,
        "{held:?} had left its park {PAUSE:?} into `open_gate`'s entry: the gate opened before \
         the span was entered"
    );
    assert_eq!(
        places.exits,
        [true],
        "{held:?} was still parked {LEAVE:?} into `open_gate`'s exit: the span ended before \
         the gate opened"
    );
}

/// A scope's opener parked on its latch is inside a latch `park` span for as long as it
/// waits, not only at some moment before.
#[test]
#[cfg_attr(miri, ignore = "holds an opener parked for real milliseconds")]
fn an_opener_waits_on_its_latch_inside_a_park_span() {
    let placement = Arc::new(Placement::default());
    let _turn = listen(placement.clone());
    let executor = executor(2, 1);
    let_workers_sleep();
    let seen = Arc::new(AtomicBool::new(false));
    let node = {
        let (placement, seen) = (placement.clone(), seen.clone());
        Node::new("spans::latch")
            .run(move |_, ctx| {
                let stolen = AtomicBool::new(false);
                let (stolen, seen, placement) = (&stolen, &seen, &placement);
                ctx.scope(|scope| {
                    // Taken by the thief and held until the opener has been inside its
                    // latch's span for a while.
                    scope.fork(move |_| {
                        stolen.store(true, Ordering::Release);
                        let me = thread_name();
                        eventually(|| placement.latch_held(&me));
                        seen.store(placement.latch_held(&me), Ordering::SeqCst);
                    });
                    scope.fork(move |_| wait_for(stolen));
                });
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    executor.shutdown();
    assert!(
        seen.load(Ordering::SeqCst),
        "the opener was never inside a latch `park` span for {HELD:?} while its child held the \
         scope open"
    );
}

/// How long the opener's scope may take before the test calls its wake-up lost.
const LOST: Duration = Duration::from_secs(10);
const STAGED: u64 = 4;

/// A listener whose `enter` of the opener's latch `park` waits through the thread's parker,
/// after the scope's last child has counted down and unparked the opener. Such a wait takes
/// the token the unpark left.
struct TokenTaker {
    /// Set by the job just before it opens its scope, so only that scope's park is staged.
    armed: AtomicBool,
    /// The opener's thread, once it has begun its latch park.
    opener: Mutex<Option<Thread>>,
    /// The thread of the stolen child, once that child is about to return.
    thief: Mutex<Option<String>>,
    /// The thief has begun an idle park since, so the child's count-down and the unpark
    /// that comes with it are done.
    unparked: AtomicBool,
    /// One end of a channel with no room, whose other end a helper takes from late: a send
    /// waits for it through the thread's parker, on every platform.
    writer: SyncSender<()>,
    /// The send has begun.
    sending: AtomicBool,
}

impl Subscriber for TokenTaker {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "whisk" && metadata.name() == "park"
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let wait = Wait::of(span);
        if wait.as_deref() == Some("idle")
            && lock(&self.thief).as_deref() == Some(thread_name().as_str())
        {
            self.unparked.store(true, Ordering::SeqCst);
        }
        if wait.as_deref() == Some("latch") && self.armed.swap(false, Ordering::SeqCst) {
            *lock(&self.opener) = Some(std::thread::current());
            eventually(|| self.unparked.load(Ordering::SeqCst));
            return Id::from_u64(STAGED);
        }
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, span: &Id) {
        if span.into_u64() == STAGED {
            self.sending.store(true, Ordering::SeqCst);
            let _ = self.writer.send(());
        }
    }

    fn exit(&self, _span: &Id) {}
}

/// A listener's callbacks that take the opener's park token, after the last child has
/// counted down, do not cost it its wake-up: the opener reads its latch again once the
/// `park` span is entered.
#[test]
#[cfg_attr(miri, ignore = "stages a listener's wait against a helper's real sleep")]
fn an_opener_whose_listener_takes_its_park_token_still_finds_its_latch_set() {
    let (writer, records) = mpsc::sync_channel(0);
    let taker = Arc::new(TokenTaker {
        armed: AtomicBool::new(false),
        opener: Mutex::new(None),
        thief: Mutex::new(None),
        unparked: AtomicBool::new(false),
        writer,
        sending: AtomicBool::new(false),
    });
    let _turn = listen(taker.clone());
    let helper = {
        let taker = taker.clone();
        std::thread::spawn(move || {
            // Late enough that the send has to wait.
            if eventually(|| taker.sending.load(Ordering::SeqCst)) {
                std::thread::sleep(Duration::from_millis(50));
                let _ = records.recv_timeout(LIMIT);
            }
        })
    };
    let executor = executor(2, 1);
    let_workers_sleep();
    let node = {
        let taker = taker.clone();
        Node::new("spans::token")
            .run(move |_, ctx| {
                let stolen = AtomicBool::new(false);
                let (stolen, taker) = (&stolen, &taker);
                taker.armed.store(true, Ordering::SeqCst);
                ctx.scope(|scope| {
                    // Taken by the thief and held until the opener has begun its latch park.
                    scope.fork(move |_| {
                        stolen.store(true, Ordering::Release);
                        eventually(|| lock(&taker.opener).is_some());
                        *lock(&taker.thief) = Some(thread_name());
                    });
                    scope.fork(move |_| wait_for(stolen));
                });
            })
            .build()
    };
    let (done, finished) = mpsc::channel();
    let runner = {
        let executor = executor.clone();
        std::thread::spawn(move || {
            executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
            let _ = done.send(());
        })
    };
    let outcome = finished.recv_timeout(LOST);
    let rescued = outcome.is_ok() || {
        // Only the wake-up is missing: one unpark of the opener's thread ends the run.
        if let Some(opener) = lock(&taker.opener).as_ref() {
            opener.unpark();
        }
        finished.recv_timeout(LOST).is_ok()
    };
    if rescued {
        runner.join().unwrap();
        helper.join().unwrap();
        executor.shutdown();
    }
    assert!(
        taker.unparked.load(Ordering::SeqCst) && taker.sending.load(Ordering::SeqCst),
        "the staging did not hold: the listener's wait did not follow the last child's unpark"
    );
    assert_eq!(
        outcome,
        Ok(()),
        "the opener stayed parked {LOST:?} with its last child counted down: the listener's \
         wait took its wake-up"
    );
}
