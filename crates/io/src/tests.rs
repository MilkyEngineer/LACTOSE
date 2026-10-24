//! The proofs that need to hold a pool thread at a chosen point of a request, through the
//! pool's test hook, to make an ordering deterministic: cancellation, the start order, the
//! replace order per path, the sweep, the pool's threads and its shutdown. Everything that
//! needs only the public surface is under `tests/`.
//!
//! Each test asserts on counts and orderings, never on timings; a bounded wait only turns a
//! hang into a failure. The counts of `AllocTag::Io` and of the process-wide temporary
//! counter assume one process per test, as `cargo nextest` runs them.

use core::num::NonZero;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use lactose_core::{AllocTag, ErrorKind, Result, TaggedAlloc, ThreadClass};
use lactose_task::{CoreTopology, Platform, StdPlatform, ThreadHandle, ThreadSpec};

use crate::backend::blocking::{COUNTER, Hook, Point, token};
use crate::request::Request;
use crate::{Io, IoConfig};

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

/// How long a test waits for something that must happen before it calls it a hang.
const LIMIT: Duration = Duration::from_secs(60);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A directory of its own under the system's temporary directory, removed when dropped.
pub(crate) struct Scratch {
    root: PathBuf,
}

impl Scratch {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "lactose-io-unit-{}-{}-{}",
            std::process::id(),
            token(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// The names in the directory, sorted.
    pub(crate) fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// The `.tmp` files in the directory.
    pub(crate) fn temporaries(&self) -> usize {
        self.names().iter().filter(|name| name.ends_with(".tmp")).count()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// `len` bytes of words from a small vocabulary, picked by a fixed generator: content zstd
/// compresses about as well as source text and decodes at about the speed of real assets.
pub(crate) fn text(len: usize, seed: u64) -> Vec<u8> {
    const WORDS: &[&[u8]] = &[
        b"curd ",
        b"whey ",
        b"rennet ",
        b"brine ",
        b"rind ",
        b"culture ",
        b"press ",
        b"mould ",
        b"aged ",
        b"salt ",
        b"vat ",
        b"wheel ",
        b"\n",
        b"0x3f, ",
        b"{ ",
        b"} ",
        b"fn ",
        b"let ",
    ];
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(len + 16);
    while out.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(WORDS[(state % WORDS.len() as u64) as usize]);
    }
    out.truncate(len);
    out
}

struct Unpark(std::thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Drives `future` to its end on this thread, failing the test if it takes [`LIMIT`].
pub(crate) fn wait<F: Future + Unpin>(future: &mut F) -> F::Output {
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let deadline = Instant::now() + LIMIT;
    loop {
        if let Poll::Ready(output) = Pin::new(&mut *future).poll(&mut cx) {
            return output;
        }
        let now = Instant::now();
        assert!(now < deadline, "a request did not resolve within {LIMIT:?}");
        std::thread::park_timeout(deadline - now);
    }
}

/// Polls `future` once, with a waker that does nothing: a first poll submits the request.
pub(crate) fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
}

/// Holds pool threads at a hook point until the test opens it.
#[derive(Default)]
pub(crate) struct Gate {
    /// How many threads have arrived, and whether it is open.
    state: Mutex<(usize, bool)>,
    changed: Condvar,
}

impl Gate {
    /// Called on a pool thread: counts the arrival and waits until the gate is open.
    pub(crate) fn hold(&self) {
        let mut state = lock(&self.state);
        state.0 += 1;
        self.changed.notify_all();
        let deadline = Instant::now() + LIMIT;
        while !state.1 {
            let now = Instant::now();
            assert!(now < deadline, "a gate was never opened");
            state = self.changed.wait_timeout(state, deadline - now).unwrap().0;
        }
    }

    pub(crate) fn open(&self) {
        lock(&self.state).1 = true;
        self.changed.notify_all();
    }

    /// Waits until `count` threads have arrived.
    pub(crate) fn arrived(&self, count: usize) {
        let mut state = lock(&self.state);
        let deadline = Instant::now() + LIMIT;
        while state.0 < count {
            let now = Instant::now();
            assert!(now < deadline, "{} of {count} threads arrived at a gate", state.0);
            state = self.changed.wait_timeout(state, deadline - now).unwrap().0;
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        lock(&self.state).1
    }
}

/// What a test's hook records of each request: its kind and path, and for a replace the
/// first byte of what it writes, which tells the test's replaces of one path apart.
fn label(request: &Request) -> String {
    match request {
        Request::Open { path, .. } => format!("open {}", name(path)),
        Request::Read { offset, len, .. } => format!("read {offset}+{len}"),
        Request::ReadEncoded { offset, len, .. } => format!("read_encoded {offset}+{len}"),
        Request::ReadFile { path, .. } => format!("read_file {}", name(path)),
        Request::Replace { path, bytes, .. } => {
            format!("replace {} {}", name(path), char::from(bytes.first().copied().unwrap_or(b'-')))
        }
        Request::ReadDir { path, .. } => format!("read_dir {}", name(path)),
        Request::CreateDirAll { path, .. } => format!("create_dir_all {}", name(path)),
    }
}

fn name(path: &Path) -> String {
    path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// A hook that records every point as a line and holds a pool thread at `Started` for a
/// request whose label is in `hold_started`, or at `Created` for a replace whose label is
/// in `hold_created`, on `gate`.
#[derive(Clone, Default)]
struct Recorder {
    seen: Arc<Mutex<Vec<String>>>,
    gate: Arc<Gate>,
    hold_started: Vec<String>,
    hold_created: Vec<String>,
}

impl Recorder {
    fn holding_started(labels: &[&str]) -> Self {
        Self { hold_started: labels.iter().map(|&l| l.to_owned()).collect(), ..Self::default() }
    }

    fn holding_created(labels: &[&str]) -> Self {
        Self { hold_created: labels.iter().map(|&l| l.to_owned()).collect(), ..Self::default() }
    }

    fn hook(&self) -> Hook {
        let recorder = self.clone();
        Arc::new(move |point| match point {
            Point::Submitted(request) => {
                lock(&recorder.seen).push(format!("submit {}", label(request)))
            }
            Point::Started(request) => {
                let label = label(request);
                lock(&recorder.seen).push(format!("start {label}"));
                if recorder.hold_started.contains(&label) {
                    recorder.gate.hold();
                }
            }
            // A created temporary is attributed to the replace that started last on this
            // thread, which is the one creating it.
            Point::Created(temporary) => {
                let file = name(temporary);
                let start = {
                    let seen = lock(&recorder.seen);
                    seen.iter().rev().find(|line| line.starts_with("start replace")).cloned()
                };
                lock(&recorder.seen).push(format!("created {file}"));
                let held = start.is_some_and(|start| {
                    recorder.hold_created.iter().any(|label| start == format!("start {label}"))
                });
                if held {
                    recorder.gate.hold();
                }
            }
        })
    }

    fn seen(&self) -> Vec<String> {
        lock(&self.seen).clone()
    }

    fn count(&self, prefix: &str) -> usize {
        lock(&self.seen).iter().filter(|line| line.starts_with(prefix)).count()
    }
}

fn threads(count: u32) -> IoConfig {
    IoConfig::new(NonZero::new(count).unwrap())
}

fn io_live_bytes() -> usize {
    ALLOCATOR.stats(AllocTag::Io).live_bytes
}

// ---- Cancellation ----

#[test]
fn a_future_dropped_unpolled_submits_nothing() {
    let scratch = Scratch::new();
    let recorder = Recorder::default();
    let io = Io::start_hooked(&StdPlatform, threads(1), recorder.hook()).unwrap();
    drop(io.replace(scratch.path("never"), b"x".to_vec()));
    drop(io.read_file(scratch.path("never")));
    drop(io.open(scratch.path("never")));
    drop(io.create_dir_all(scratch.path("never-dir")));
    io.shutdown();
    assert_eq!(recorder.seen(), Vec::<String>::new());
    assert_eq!(scratch.names(), Vec::<String>::new());
}

#[test]
fn a_future_dropped_while_queued_never_runs() {
    let scratch = Scratch::new();
    let recorder = Recorder::holding_started(&["read_dir blocker"]);
    let io = Io::start_hooked(&StdPlatform, threads(1), recorder.hook()).unwrap();
    std::fs::create_dir(scratch.path("blocker")).unwrap();
    let mut blocker = io.read_dir(scratch.path("blocker"));
    assert!(poll_once(&mut blocker).is_pending());
    recorder.gate.arrived(1);
    let mut queued = io.replace(scratch.path("slot"), b"queued".to_vec());
    let mut listing = io.read_dir(scratch.path("blocker"));
    let mut after = io.read_dir(scratch.path("blocker"));
    assert!(poll_once(&mut queued).is_pending() && poll_once(&mut listing).is_pending());
    assert!(poll_once(&mut after).is_pending());
    drop(queued);
    drop(listing);
    recorder.gate.open();
    assert_eq!(wait(&mut blocker).unwrap(), Vec::new());
    // One thread and one FIFO queue: the dropped requests were reached before this one.
    assert_eq!(wait(&mut after).unwrap(), Vec::new());
    io.shutdown();
    assert_eq!(recorder.count("submit"), 4);
    assert_eq!(recorder.count("start"), 2, "{:?}", recorder.seen());
    assert_eq!(scratch.names(), ["blocker"], "the dropped replace touched the directory");
}

#[test]
fn a_future_dropped_after_its_request_started_completes_on_the_pool() {
    let scratch = Scratch::new();
    let path = scratch.path("pack");
    std::fs::write(&path, text(1 << 20, 3)).unwrap();
    let recorder = Recorder::holding_started(&["read 0+1048576"]);
    let io = Io::start_hooked(&StdPlatform, threads(1), recorder.hook()).unwrap();
    let file = wait(&mut io.open(&path)).unwrap();
    let mut read = file.read_at(0, 1 << 20);
    assert!(poll_once(&mut read).is_pending());
    recorder.gate.arrived(1);
    let allocations = ALLOCATOR.stats(AllocTag::Io).total_allocations;
    // The drop returns while the pool thread is still inside the request.
    drop(read);
    assert!(!recorder.gate.is_open());
    drop(file);
    recorder.gate.open();
    io.shutdown();
    // The gate's lock and condition variable may have been allocated lazily on the pool
    // thread, under its tag: they go with the recorder.
    drop(recorder);
    let stats = ALLOCATOR.stats(AllocTag::Io);
    assert!(stats.total_allocations > allocations, "the read never allocated its buffer");
    assert_eq!(stats.live_bytes, 0, "{stats:?}");
}

#[test]
fn an_encoded_read_dropped_between_two_slices_frees_both_buffers() {
    let scratch = Scratch::new();
    let path = scratch.path("pack");
    let content = text(16 << 20, 5);
    let stored = zstd::bulk::compress(&content, 3).unwrap();
    std::fs::write(&path, &stored).unwrap();
    let io = Io::start(&StdPlatform, threads(1)).unwrap();
    let file = wait(&mut io.open(&path)).unwrap();
    let encoding = crate::Encoding::Zstd { decoded_len: content.len() as u64 };
    let mut read = file.read_encoded_at(0, stored.len() as u64, encoding);
    // Polled until it has woken itself: its stored bytes arrived and a slice decoded.
    struct Count(AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let count = Arc::new(Count(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&count));
    let deadline = Instant::now() + LIMIT;
    while !matches!(read.state, crate::file::ReadState::Decoding(_)) {
        assert!(Pin::new(&mut read).poll(&mut Context::from_waker(&waker)).is_pending());
        assert!(Instant::now() < deadline, "the stored bytes never arrived");
        std::thread::yield_now();
    }
    assert!(io_live_bytes() >= stored.len() + content.len(), "both buffers are the pool's");
    drop(read);
    drop(file);
    io.shutdown();
    assert_eq!(io_live_bytes(), 0);
}

// ---- Ordering ----

#[test]
fn requests_start_in_first_poll_order() {
    let scratch = Scratch::new();
    for name in ["held", "a", "b", "c"] {
        std::fs::create_dir(scratch.path(name)).unwrap();
    }
    let recorder = Recorder::holding_started(&["read_dir held"]);
    let io = Io::start_hooked(&StdPlatform, threads(1), recorder.hook()).unwrap();
    let mut held = io.read_dir(scratch.path("held"));
    assert!(poll_once(&mut held).is_pending());
    recorder.gate.arrived(1);
    // Created in one order, first polled in another.
    let mut a = io.read_dir(scratch.path("a"));
    let mut b = io.read_dir(scratch.path("b"));
    let mut c = io.read_dir(scratch.path("c"));
    for future in [&mut c, &mut a, &mut b] {
        assert!(poll_once(future).is_pending());
    }
    recorder.gate.open();
    for future in [&mut held, &mut a, &mut b, &mut c] {
        wait(future).unwrap();
    }
    io.shutdown();
    let starts: Vec<String> =
        recorder.seen().into_iter().filter(|line| line.starts_with("start")).collect();
    assert_eq!(
        starts,
        ["start read_dir held", "start read_dir c", "start read_dir a", "start read_dir b"]
    );
}

// ---- Replace order per path ----

/// The shape of the facade review's `review_replace_race.rs`: a large replace A is held
/// after its temporary exists, which a listing observes, and its future dropped; a third,
/// C, is submitted and dropped while it waits; a small B is submitted and awaited. B starts
/// only after A has finished, C never runs, and the path holds B's bytes when B resolves and
/// once the pool is idle.
#[test]
fn replaces_of_one_path_run_in_submission_order_dropped_or_not() {
    let scratch = Scratch::new();
    let slot = scratch.path("slot");
    let recorder = Recorder::holding_created(&["replace slot A"]);
    let io = Io::start_hooked(&StdPlatform, threads(2), recorder.hook()).unwrap();
    let mut a = io.replace(&slot, vec![b'A'; 8 << 20]);
    assert!(poll_once(&mut a).is_pending());
    recorder.gate.arrived(1);
    let listing = wait(&mut io.read_dir(scratch.path(""))).unwrap();
    assert!(listing.iter().any(|entry| entry.name.to_string_lossy().ends_with(".tmp")));
    drop(a);
    let mut c = io.replace(&slot, b"C".to_vec());
    assert!(poll_once(&mut c).is_pending());
    drop(c);
    let mut b = io.replace(&slot, b"B".to_vec());
    assert!(poll_once(&mut b).is_pending());
    // B waits behind A without holding the second thread, which serves a listing.
    let listing = wait(&mut io.read_dir(scratch.path(""))).unwrap();
    assert_eq!(listing.len(), 1, "{listing:?}");
    recorder.gate.open();
    wait(&mut b).unwrap();
    assert_eq!(std::fs::read(&slot).unwrap(), b"B");
    io.shutdown();
    assert_eq!(std::fs::read(&slot).unwrap(), b"B", "an older replace landed after B");
    let starts: Vec<String> =
        recorder.seen().into_iter().filter(|line| line.starts_with("start replace")).collect();
    assert_eq!(starts, ["start replace slot A", "start replace slot B"]);
    assert_eq!(scratch.temporaries(), 0);
}

#[test]
fn replaces_of_two_paths_run_at_once() {
    let scratch = Scratch::new();
    let recorder = Recorder::holding_created(&["replace one 1"]);
    let io = Io::start_hooked(&StdPlatform, threads(2), recorder.hook()).unwrap();
    let mut one = io.replace(scratch.path("one"), b"1".to_vec());
    assert!(poll_once(&mut one).is_pending());
    recorder.gate.arrived(1);
    wait(&mut io.replace(scratch.path("two"), b"2".to_vec())).unwrap();
    assert!(!recorder.gate.is_open());
    recorder.gate.open();
    wait(&mut one).unwrap();
    io.shutdown();
    assert_eq!(std::fs::read(scratch.path("one")).unwrap(), b"1");
    assert_eq!(std::fs::read(scratch.path("two")).unwrap(), b"2");
}

// ---- The atomic replace: temporaries ----

#[test]
fn the_temporary_is_created_beside_the_path() {
    let scratch = Scratch::new();
    std::fs::create_dir(scratch.path("saves")).unwrap();
    let slot = scratch.path("saves").join("slot0.carton");
    let recorder = Recorder::holding_created(&["replace slot0.carton s"]);
    let io = Io::start_hooked(&StdPlatform, threads(1), recorder.hook()).unwrap();
    let mut save = io.replace(&slot, b"save".to_vec());
    assert!(poll_once(&mut save).is_pending());
    recorder.gate.arrived(1);
    let names: Vec<String> = std::fs::read_dir(scratch.path("saves"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    let expected = format!("slot0.carton.{:08x}", token());
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].starts_with(&expected) && names[0].ends_with(".tmp"), "{names:?}");
    assert_eq!(names[0].len(), "slot0.carton.".len() + 16 + ".tmp".len());
    recorder.gate.open();
    wait(&mut save).unwrap();
    io.shutdown();
    assert_eq!(scratch.names(), ["saves"]);
    assert_eq!(std::fs::read(&slot).unwrap(), b"save");
}

#[test]
fn the_first_replace_into_a_directory_sweeps_only_foreign_temporaries() {
    let scratch = Scratch::new();
    let foreign = format!("{:08x}", token().wrapping_add(1));
    let mine = format!("{:08x}", token());
    let leftovers =
        [format!("slot.{foreign}00000007.tmp"), format!("another name.{foreign}ffffffff.tmp")];
    let kept = [
        format!("other.{mine}fffffff0.tmp"),
        "slot.bak.tmp".to_owned(),
        format!("slot.{foreign}0000007.tmp"),
    ];
    for name in leftovers.iter().chain(&kept) {
        std::fs::write(scratch.path(name), b"left").unwrap();
    }
    let io = Io::start(&StdPlatform, threads(1)).unwrap();
    wait(&mut io.replace(scratch.path("slot"), b"1".to_vec())).unwrap();
    let mut expected: Vec<String> = kept.to_vec();
    expected.push("slot".to_owned());
    expected.sort();
    assert_eq!(scratch.names(), expected);
    // The second replace into it does not list it again.
    let late = format!("late.{foreign}00000000.tmp");
    std::fs::write(scratch.path(&late), b"left").unwrap();
    wait(&mut io.replace(scratch.path("slot"), b"2".to_vec())).unwrap();
    assert!(scratch.names().contains(&late));
    io.shutdown();
    // A second pool is a fresh key, and sweeps it.
    let io = Io::start(&StdPlatform, threads(1)).unwrap();
    wait(&mut io.replace(scratch.path("slot"), b"3".to_vec())).unwrap();
    assert!(!scratch.names().contains(&late));
    io.shutdown();
}

#[test]
fn replaces_through_two_pools_or_two_spellings_both_land() {
    let scratch = Scratch::new();
    std::fs::create_dir(scratch.path("d")).unwrap();
    // Two spellings through one pool of two threads, then one spelling through two pools,
    // each pair held in flight at once after both temporaries exist. The current directory
    // is this test's own process's.
    std::env::set_current_dir(&scratch.root).unwrap();
    let path = Path::new("d/a");
    let spelled = Path::new("./d/a");
    let recorder = Recorder::holding_created(&["replace a 1", "replace a 2"]);
    let io = Io::start_hooked(&StdPlatform, threads(2), recorder.hook()).unwrap();
    let mut one = io.replace(path, b"1".to_vec());
    let mut two = io.replace(spelled, b"2".to_vec());
    assert!(poll_once(&mut one).is_pending() && poll_once(&mut two).is_pending());
    recorder.gate.arrived(2);
    assert_eq!(recorder.count("created"), 2);
    recorder.gate.open();
    wait(&mut one).unwrap();
    wait(&mut two).unwrap();
    io.shutdown();
    assert!(matches!(std::fs::read(path).unwrap().as_slice(), b"1" | b"2"));
    assert_eq!(std::fs::read_dir(scratch.path("d")).unwrap().count(), 1);

    let recorder = Recorder::holding_created(&["replace a 3", "replace a 4"]);
    let first = Io::start_hooked(&StdPlatform, threads(1), recorder.hook()).unwrap();
    let second = Io::start_hooked(&StdPlatform, threads(1), recorder.hook()).unwrap();
    let mut three = first.replace(path, b"3".to_vec());
    let mut four = second.replace(path, b"4".to_vec());
    assert!(poll_once(&mut three).is_pending() && poll_once(&mut four).is_pending());
    recorder.gate.arrived(2);
    recorder.gate.open();
    wait(&mut three).unwrap();
    wait(&mut four).unwrap();
    first.shutdown();
    second.shutdown();
    assert!(matches!(std::fs::read(path).unwrap().as_slice(), b"3" | b"4"));
    assert_eq!(std::fs::read_dir(scratch.path("d")).unwrap().count(), 1);
}

#[test]
fn a_temporary_name_already_taken_is_skipped() {
    let scratch = Scratch::new();
    let next = COUNTER.load(Ordering::Relaxed);
    let taken = |counter: u32| scratch.path(&format!("slot.{:08x}{counter:08x}.tmp", token()));
    for counter in next..next + 2 {
        std::fs::write(taken(counter), b"taken").unwrap();
    }
    let io = Io::start(&StdPlatform, threads(1)).unwrap();
    wait(&mut io.replace(scratch.path("slot"), b"new".to_vec())).unwrap();
    assert_eq!(std::fs::read(scratch.path("slot")).unwrap(), b"new");
    assert_eq!(scratch.temporaries(), 2, "a taken name was reused or removed");

    // Every one of the sixteen names it tries taken is `Io`, with the slot untouched.
    let next = COUNTER.load(Ordering::Relaxed);
    for counter in next..next + 16 {
        std::fs::write(taken(counter), b"taken").unwrap();
    }
    let bytes = b"newer".to_vec();
    let pointer = bytes.as_ptr();
    let failed = wait(&mut io.replace(scratch.path("slot"), bytes)).unwrap_err();
    assert_eq!(failed.error.kind(), ErrorKind::Io, "{}", failed.error);
    assert_eq!(failed.bytes.as_ptr(), pointer);
    assert_eq!(std::fs::read(scratch.path("slot")).unwrap(), b"new");
    io.shutdown();
}

// ---- The pool ----

/// A platform that records each thread it is asked for and each join, and refuses the
/// spawn numbered `refuse`.
#[derive(Default)]
struct Spy {
    specs: Mutex<Vec<(String, ThreadClass, u32)>>,
    joined: Arc<AtomicU32>,
    exited: Arc<AtomicU32>,
    refuse: Option<usize>,
}

impl Platform for Spy {
    fn topology(&self) -> CoreTopology {
        CoreTopology { performance: 4, efficiency: 4 }
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        let mut specs = lock(&self.specs);
        if self.refuse == Some(specs.len()) {
            return Err(lactose_core::Error::new(ErrorKind::Platform, "refused by the spy"));
        }
        specs.push((spec.name.clone(), spec.class, spec.index_in_class));
        let exited = Arc::clone(&self.exited);
        let thread = StdPlatform.spawn_thread(
            spec,
            Box::new(move || {
                body();
                exited.fetch_add(1, Ordering::SeqCst);
            }),
        )?;
        let joined = Arc::clone(&self.joined);
        Ok(ThreadHandle::new(move || {
            thread.join();
            joined.fetch_add(1, Ordering::SeqCst);
        }))
    }
}

#[test]
fn the_threads_come_from_the_platform_with_their_names_class_and_indices() {
    let spy = Spy::default();
    let io = Io::start(&spy, threads(3)).unwrap();
    assert_eq!(io.pool_threads().get(), 3);
    assert_eq!(
        *lock(&spy.specs),
        [
            ("io-0".to_owned(), ThreadClass::Io, 0),
            ("io-1".to_owned(), ThreadClass::Io, 1),
            ("io-2".to_owned(), ThreadClass::Io, 2),
        ]
    );
    io.shutdown();
    assert_eq!(spy.joined.load(Ordering::SeqCst), 3);
    assert_eq!(spy.exited.load(Ordering::SeqCst), 3);
}

#[test]
fn a_thread_the_platform_refuses_fails_the_start_after_stopping_the_others() {
    let spy = Spy { refuse: Some(2), ..Spy::default() };
    let error = Io::start(&spy, threads(4)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Platform);
    assert_eq!(lock(&spy.specs).len(), 2);
    assert_eq!(spy.joined.load(Ordering::SeqCst), 2);
    assert_eq!(spy.exited.load(Ordering::SeqCst), 2);
}

#[test]
fn pool_threads_requests_run_at_once_under_the_io_tag() {
    let scratch = Scratch::new();
    let path = scratch.path("file");
    std::fs::write(&path, b"0123").unwrap();
    // Each request waits at a barrier of two: with fewer than two threads running requests
    // at once, neither would pass.
    let barrier = Arc::new(Gate::default());
    let tags = Arc::new(Mutex::new(Vec::new()));
    let hook: Hook = {
        let (barrier, tags) = (Arc::clone(&barrier), Arc::clone(&tags));
        Arc::new(move |point| {
            if let Point::Started(_) = point {
                lock(&tags).push(AllocTag::current());
                barrier.hold();
            }
        })
    };
    let io = Io::start_hooked(&StdPlatform, threads(2), hook).unwrap();
    let mut first = io.read_file(&path);
    let mut second = io.read_file(&path);
    assert!(poll_once(&mut first).is_pending() && poll_once(&mut second).is_pending());
    barrier.arrived(2);
    barrier.open();
    let bytes = wait(&mut first).unwrap();
    wait(&mut second).unwrap();
    assert_eq!(*lock(&tags), [AllocTag::Io, AllocTag::Io]);
    // The buffer was allocated on the pool thread, under `Io`.
    let live = io_live_bytes();
    drop(bytes);
    assert_eq!(live - io_live_bytes(), 4);
    io.shutdown();
}

// ---- Shutdown ----

#[test]
fn shutdown_cancels_what_waits_finishes_what_started_and_joins() {
    let scratch = Scratch::new();
    let slot = scratch.path("slot");
    std::fs::create_dir(scratch.path("listed")).unwrap();
    let recorder = Recorder::holding_created(&["replace slot A"]);
    let spy = Spy::default();
    let io = Io::start_hooked(&spy, threads(1), recorder.hook()).unwrap();
    let mut started = io.replace(&slot, b"A".to_vec());
    assert!(poll_once(&mut started).is_pending());
    recorder.gate.arrived(1);
    let mut behind = io.replace(&slot, b"B".to_vec());
    let mut queued = io.read_dir(scratch.path("listed"));
    assert!(poll_once(&mut behind).is_pending() && poll_once(&mut queued).is_pending());
    let clone = io.clone();
    let stopping = std::thread::spawn(move || io.shutdown());
    let failed = wait(&mut behind).unwrap_err();
    assert_eq!(failed.error.kind(), ErrorKind::Cancelled);
    assert_eq!(failed.bytes, b"B");
    assert_eq!(wait(&mut queued).unwrap_err().kind(), ErrorKind::Cancelled);
    // A request from a clone left behind is cancelled at its first poll.
    let mut late = clone.read_dir(scratch.path("listed"));
    assert_eq!(wait(&mut late).unwrap_err().kind(), ErrorKind::Cancelled);
    assert_eq!(spy.joined.load(Ordering::SeqCst), 0, "joined before the started replace ended");
    recorder.gate.open();
    stopping.join().unwrap();
    assert_eq!(spy.joined.load(Ordering::SeqCst), 1);
    wait(&mut started).unwrap();
    assert_eq!(std::fs::read(&slot).unwrap(), b"A");
    assert_eq!(recorder.count("start"), 1);
}

#[test]
fn dropping_the_last_handle_stops_the_pool_without_waiting() {
    let scratch = Scratch::new();
    std::fs::create_dir(scratch.path("listed")).unwrap();
    let recorder = Recorder::holding_started(&["read_dir listed"]);
    let spy = Spy::default();
    let io = Io::start_hooked(&spy, threads(1), recorder.hook()).unwrap();
    let mut started = io.read_dir(scratch.path("listed"));
    let mut queued = io.create_dir_all(scratch.path("created"));
    assert!(poll_once(&mut started).is_pending());
    recorder.gate.arrived(1);
    assert!(poll_once(&mut queued).is_pending());
    // Returns at once, with the pool thread inside the started request.
    drop(io);
    assert_eq!(wait(&mut queued).unwrap_err().kind(), ErrorKind::Cancelled);
    recorder.gate.open();
    assert_eq!(wait(&mut started).unwrap(), Vec::new());
    let deadline = Instant::now() + LIMIT;
    while spy.exited.load(Ordering::SeqCst) < 1 {
        assert!(Instant::now() < deadline, "the pool thread never exited");
        std::thread::yield_now();
    }
    assert_eq!(spy.joined.load(Ordering::SeqCst), 0);
    assert!(!scratch.path("created").exists());
}
