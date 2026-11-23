//! The present thread with no window (`README.md`, "Proof"): its spec, a call that blocks,
//! a call queued while a call parks, the stop and the drop under queuing threads, the
//! lane's close before the hand-back, an idle thread's CPU, the calls after the hand-back,
//! a second present thread on one platform thread, the drop, a platform that drops the body
//! and a call that panics. Miri runs this file, save the CPU test.

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::Wake;
use std::time::{Duration, Instant};

use lactose_core::{ErrorKind, Result, ThreadClass};
use lactose_platform::{ClientPlatform, PresentThread, present_thread};
use lactose_task::{
    CoreTopology, Executor, LaneCall, Platform, StdPlatform, ThreadHandle, ThreadSpec, WorkerClass,
};

use crate::thread_cpu::thread_cpu;

/// How long a test waits for anything: far past what any wait here takes, so that only a
/// hang reaches it. Miri's clock runs with the instructions it interprets.
const BOUND: Duration = Duration::from_secs(if cfg!(miri) { 3600 } else { 30 });

/// Polls `future` on the calling thread until it resolves, and fails past [`BOUND`].
fn wait<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    let until = Instant::now() + BOUND;
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        let now = Instant::now();
        assert!(now < until, "a wait passed its bound of {BOUND:?}");
        std::thread::park_timeout(until - now);
    }
}

/// Polls `done` until it holds, and fails past [`BOUND`]: for what wakes no one.
fn until(mut done: impl FnMut() -> bool) {
    let until = Instant::now() + BOUND;
    while !done() {
        assert!(Instant::now() < until, "a condition passed its bound of {BOUND:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn executor() -> Executor {
    Executor::builder().frame_workers(2).background_workers(1).build().expect("an executor")
}

/// Records what it was asked for, and makes the thread as the client's platform does.
#[derive(Default)]
struct Spy {
    specs: Mutex<Vec<ThreadSpec>>,
}

impl Platform for Spy {
    fn topology(&self) -> CoreTopology {
        StdPlatform.topology()
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        self.specs.lock().expect("no test panics with it held").push(spec.clone());
        ClientPlatform::new().spawn_thread(spec, body)
    }
}

#[test]
fn the_present_thread_is_made_through_the_platform_with_its_spec() {
    let spy = Spy::default();
    let (present, thread) = present_thread(&spy, 0u32).expect("a present thread");
    {
        let specs = spy.specs.lock().expect("no test panics with it held");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "present");
        assert_eq!(specs[0].class, ThreadClass::Present);
        assert_eq!(specs[0].index_in_class, 0);
    }
    let name = wait(present.run(|count| {
        *count += 1;
        std::thread::current().name().map(str::to_owned)
    }));
    assert_eq!(name.expect("the call ran").as_deref(), Some("present"));
    assert_eq!(wait(thread.stop()).expect("the state comes back"), 1);
}

/// A call that blocks holds the calls queued behind it and no other thread: while it is in
/// progress, a worker's task completes and the call behind it has not run.
#[test]
fn a_call_that_blocks_holds_the_calls_behind_it_and_nothing_else() {
    let executor = executor();
    let (present, thread) = present_thread(&ClientPlatform::new(), Vec::new()).expect("a thread");
    let (entered, inside) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let blocking = present.run(move |order: &mut Vec<&str>| {
        order.push("blocking");
        entered.send(()).expect("the test waits for the call");
        released.recv_timeout(BOUND).expect("the test releases the call");
    });
    let behind_ran = Arc::new(AtomicBool::new(false));
    let behind = {
        let behind_ran = behind_ran.clone();
        present.run(move |order: &mut Vec<&str>| {
            behind_ran.store(true, Ordering::Relaxed);
            order.push("behind");
        })
    };
    inside.recv_timeout(BOUND).expect("the blocking call started");
    // The blocking call is in progress.
    let task = executor.spawn(WorkerClass::FrameCritical, async { 7 });
    assert_eq!(wait(task).expect("the task completes"), 7);
    assert!(!behind_ran.load(Ordering::Relaxed), "the call behind ran past the blocking one");
    release.send(()).expect("the call waits for the release");
    wait(blocking).expect("the blocking call ran");
    wait(behind).expect("the call behind ran");
    assert_eq!(wait(thread.stop()).expect("the state comes back"), ["blocking", "behind"]);
    executor.shutdown();
}

/// A call queued while a call that parks the present thread is in progress, as a channel's
/// receive does, runs once that call returns: its wake is not lost to the park inside the
/// call. The call is queued after the drain that runs the parking call began, so it waits
/// for the next drain, which only its wake brings about.
#[test]
fn a_call_queued_while_a_call_parks_runs_after_it() {
    let (present, thread) = present_thread(&ClientPlatform::new(), 0u32).expect("a thread");
    for round in 0..if cfg!(miri) { 3 } else { 50 } {
        let (entered, inside) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let parking = present.run(move |_: &mut u32| {
            entered.send(()).expect("the test waits for the call");
            released.recv_timeout(BOUND).expect("the test releases the call");
        });
        inside.recv_timeout(BOUND).expect("the parking call started");
        // The drain that runs `parking` has begun: this call waits for the next one.
        let behind = present.run(move |count: &mut u32| {
            *count += 1;
            *count
        });
        // Its wake has unparked the present thread inside the receive, which parks again.
        std::thread::sleep(Duration::from_millis(1));
        release.send(()).expect("the call waits for the release");
        wait(parking).expect("the parking call ran");
        assert_eq!(wait(behind).expect("the call behind ran"), round + 1);
    }
    wait(thread.stop()).expect("the state comes back");
}

/// The state of a stop round: the index of every call that ran, in order. Its drop records
/// the thread it is dropped on, for a round whose `PresentThread` is dropped.
struct Logged {
    ran: Vec<usize>,
    dropped_on: Arc<Mutex<Option<String>>>,
}

impl Drop for Logged {
    fn drop(&mut self) {
        let name = std::thread::current().name().unwrap_or("unnamed").to_owned();
        *self.dropped_on.lock().expect("not poisoned") = Some(name);
    }
}

/// What the queuing threads and the caller of `stop` share: the calls queued, in the order
/// the lock gave them, and how many had been queued when `stop` or the drop was called.
struct Queued {
    calls: Vec<LaneCall<()>>,
    at_stop: Option<usize>,
}

/// What one round saw: whether a call queued after the stop ran, and whether one came back
/// `Cancelled`.
struct Round {
    ran_after: bool,
    cancelled: bool,
}

/// One round: several threads queue calls in a loop, each under the lock that the stop is
/// made under too, and each call records its index in the state and in a log of the test's.
/// Once `before` calls are queued the test calls `stop`, or, in a round `by_drop`, drops
/// the `PresentThread`.
fn stop_under_queuing_threads(before: usize, by_drop: bool) -> Round {
    const SENDERS: usize = if cfg!(miri) { 2 } else { 3 };
    // How many more each sender queues past the stop before it ends.
    const AFTER: usize = 16;
    let dropped_on = Arc::new(Mutex::new(None));
    let state = Logged { ran: Vec::new(), dropped_on: dropped_on.clone() };
    let (present, thread) = present_thread(&ClientPlatform::new(), state).expect("a thread");
    let log = Arc::new(Mutex::new(Vec::new()));
    let queued = Arc::new(Mutex::new(Queued { calls: Vec::new(), at_stop: None }));
    let senders: Vec<_> = (0..SENDERS)
        .map(|_| {
            let (present, queued, log) = (present.clone(), queued.clone(), log.clone());
            std::thread::spawn(move || {
                loop {
                    {
                        let mut queued = queued.lock().expect("no test panics with it held");
                        let index = queued.calls.len();
                        if queued.at_stop.is_some_and(|at| index >= at + AFTER) {
                            return;
                        }
                        let log = log.clone();
                        let call = present.run(move |state: &mut Logged| {
                            state.ran.push(index);
                            log.lock().expect("not poisoned").push(index);
                        });
                        queued.calls.push(call);
                    }
                    std::thread::yield_now();
                }
            })
        })
        .collect();
    until(|| queued.lock().expect("no test panics with it held").calls.len() >= before);
    let stopped = {
        let mut queued = queued.lock().expect("no test panics with it held");
        let stopped = if by_drop {
            drop(thread);
            None
        } else {
            Some(thread.stop())
        };
        queued.at_stop = Some(queued.calls.len());
        stopped
    };
    let handed = stopped.map(|stopped| wait(stopped).expect("the hand-back gives the state"));
    for sender in senders {
        sender.join().expect("a sender ends");
    }
    let ran = if let Some(state) = &handed {
        state.ran.clone()
    } else {
        until(|| dropped_on.lock().expect("not poisoned").is_some());
        assert_eq!(dropped_on.lock().expect("not poisoned").as_deref(), Some("present"));
        log.lock().expect("not poisoned").clone()
    };
    let Queued { calls, at_stop } =
        Arc::into_inner(queued).expect("the senders let go").into_inner().expect("not poisoned");
    let at_stop = at_stop.expect("stopped");

    // The calls ran in the order they were queued, every one queued before the stop among
    // them, and those queued after it that ran are the first of them: the last drain's.
    assert!(ran.len() >= at_stop, "a call queued before the stop did not run: {ran:?}");
    assert!(ran.iter().enumerate().all(|(position, &index)| position == index), "{ran:?}");
    // A call queued after the stop ran in the last drain or is `Cancelled`.
    let mut cancelled = false;
    for (index, call) in calls.into_iter().enumerate() {
        match wait(call) {
            Ok(()) => assert!(index < ran.len(), "call {index} ran and is not in the state"),
            Err(error) => {
                assert_eq!(error.kind(), ErrorKind::Cancelled);
                assert!(index >= ran.len(), "call {index} ran and was cancelled");
                assert!(index >= at_stop, "call {index}, queued before the stop, was cancelled");
                cancelled = true;
            }
        }
    }
    // None ran after the hand-back: the test's log is the state's own.
    if let Some(state) = handed {
        assert_eq!(*log.lock().expect("not poisoned"), state.ran, "a call ran after the hand-back");
    }
    Round { ran_after: ran.len() > at_stop, cancelled }
}

/// `stop` runs every call queued before it and hands the state back, and a dropped
/// `PresentThread` runs them and drops the state on the present thread. Three threads queue
/// while the rounds alternate the two, so that a stop that could outrun its take, as one
/// queued before the take would, meets an idle present thread in many rounds.
#[test]
fn stop_runs_every_call_queued_before_it_and_hands_the_state_back() {
    let rounds = if cfg!(miri) { 4 } else { 300 };
    let (mut ran_after, mut cancelled) = (0, 0);
    for round in 0..rounds {
        let seen = stop_under_queuing_threads(1 + round % 7, round % 2 == 1);
        ran_after += usize::from(seen.ran_after);
        cancelled += usize::from(seen.cancelled);
    }
    println!("{rounds} rounds: {ran_after} ran a later call, {cancelled} cancelled one");
    if !cfg!(miri) {
        assert!(ran_after > 0, "no round ran a call queued after the stop in its last drain");
        assert!(cancelled > 0, "no round cancelled a call queued after the stop");
    }
}

/// The present lane is closed before the state is handed back: a call that the close
/// cancels has been dropped, and its drop is still running, while the stop's future is
/// pending. A call in the stop's drain queues one more, which that drain does not run; that
/// one's drop signals, then waits for the test.
#[test]
fn the_lane_is_closed_before_the_state_is_handed_back() {
    struct Signals {
        dropping: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl Drop for Signals {
        fn drop(&mut self) {
            let _ = self.dropping.send(());
            let _ = self.release.recv_timeout(BOUND);
        }
    }

    let (present, thread) = present_thread(&ClientPlatform::new(), ()).expect("a thread");
    let (entered, inside) = mpsc::channel();
    let (release_held, held) = mpsc::channel::<()>();
    let holding = present.run(move |()| {
        entered.send(()).expect("the test waits for the call");
        held.recv_timeout(BOUND).expect("the test releases the call");
    });
    inside.recv_timeout(BOUND).expect("the held call started");
    // Queued while the call is held, so the stop and the next call share the stop's drain.
    let mut stopped = pin!(thread.stop());
    let (dropping, dropped) = mpsc::channel();
    let (release, waiting) = mpsc::channel();
    let signals = Signals { dropping, release: waiting };
    let lane = present.clone();
    let queuing = present.run(move |()| drop(lane.run(move |()| drop(signals))));
    release_held.send(()).expect("the call waits for the release");
    wait(holding).expect("the held call ran");
    wait(queuing).expect("the call in the stop's drain ran");
    dropped.recv_timeout(BOUND).expect("the call the lane's close cancelled was dropped");
    let early = stopped.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    assert!(early.is_pending(), "the state was handed back before the lane was closed");
    release.send(()).expect("the drop waits for the release");
    wait(stopped).expect("the state comes back");
}

/// An idle present thread uses no CPU: after plain calls, after a call that parked with a
/// call queued behind it, and after a call that unparked its own thread. The present thread
/// reads its own CPU clock in a call before and after each idle spell, so a loaded machine
/// slows the test and cannot fail it.
#[test]
#[cfg_attr(miri, ignore = "reads the thread's CPU clock through the operating system")]
fn an_idle_present_thread_uses_no_cpu() {
    const IDLE: Duration = Duration::from_millis(500);
    const MOST: Duration = Duration::from_millis(50);
    let (present, thread) = present_thread(&ClientPlatform::new(), ()).expect("a thread");
    let idle = |after: &str| {
        let before = wait(present.run(|()| thread_cpu())).expect("the call ran");
        std::thread::sleep(IDLE);
        let spent = wait(present.run(|()| thread_cpu())).expect("the call ran") - before;
        println!("{after}: {spent:?} of the present thread's CPU over {IDLE:?}");
        assert!(spent < MOST, "{after}: the present thread used {spent:?} of CPU over {IDLE:?}");
    };
    for _ in 0..10 {
        wait(present.run(|()| ())).expect("the call ran");
    }
    idle("after plain calls");

    let (entered, inside) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let parking = present.run(move |()| {
        entered.send(()).expect("the test waits for the call");
        released.recv_timeout(BOUND).expect("the test releases the call");
    });
    inside.recv_timeout(BOUND).expect("the parking call started");
    let behind = present.run(|()| ());
    release.send(()).expect("the call waits for the release");
    wait(parking).expect("the parking call ran");
    wait(behind).expect("the call behind ran");
    idle("after a call that parked with one queued behind it");

    wait(present.run(|()| std::thread::current().unpark())).expect("the call ran");
    idle("after a call that unparked its own thread");
    wait(thread.stop()).expect("the state comes back");
}

#[test]
fn a_call_queued_after_the_hand_back_is_cancelled() {
    let (present, thread) = present_thread(&ClientPlatform::new(), 5u8).expect("a thread");
    assert_eq!(wait(thread.stop()).expect("the state comes back"), 5);
    let after = wait(present.run(|_| ())).expect_err("nothing runs after the hand-back");
    assert_eq!(after.kind(), ErrorKind::Cancelled);
}

/// What `OneThread` runs.
type Body = Box<dyn FnOnce() + Send>;

/// A platform that runs every body on its one thread, in turn.
struct OneThread {
    bodies: Mutex<Option<mpsc::Sender<Body>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl OneThread {
    fn new() -> Self {
        let (bodies, receive) = mpsc::channel::<Body>();
        let thread = std::thread::Builder::new()
            .name("the one thread".to_owned())
            .spawn(move || receive.into_iter().for_each(|body| body()))
            .expect("a thread");
        Self { bodies: Mutex::new(Some(bodies)), thread: Mutex::new(Some(thread)) }
    }

    /// Ends the thread once it has run every body given it.
    fn join(&self) {
        drop(self.bodies.lock().expect("not poisoned").take());
        let thread = self.thread.lock().expect("not poisoned").take();
        thread.expect("joined once").join().expect("no body panicked");
    }
}

impl Platform for OneThread {
    fn topology(&self) -> CoreTopology {
        StdPlatform.topology()
    }

    fn spawn_thread(
        &self,
        _: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        let bodies = self.bodies.lock().expect("not poisoned");
        bodies.as_ref().expect("not joined").send(body).expect("the thread runs");
        Ok(ThreadHandle::new(|| {}))
    }
}

/// The second body runs on the thread the first stopped on, so the stop of the first must
/// not stop the second.
#[test]
fn a_platform_with_one_thread_makes_a_second_present_thread_after_the_first() {
    let one = OneThread::new();
    let (first, thread): (_, PresentThread<u32>) =
        present_thread(&one, 0u32).expect("a first present thread");
    let first_thread = wait(first.run(|_| std::thread::current().id())).expect("it ran");
    assert_eq!(wait(thread.stop()).expect("the state comes back"), 0);
    let (second, thread) = present_thread(&one, 1u32).expect("a second present thread");
    let second_thread = wait(second.run(|count| {
        *count += 1;
        std::thread::current().id()
    }))
    .expect("the second present thread runs its calls");
    assert_eq!(first_thread, second_thread, "the platform ran both bodies on its one thread");
    assert_eq!(wait(second.run(|count| *count)).expect("it ran"), 2);
    assert_eq!(wait(thread.stop()).expect("the state comes back"), 2);
    one.join();
}

/// Records the thread its drop ran on.
struct DropsOn {
    ran: Arc<AtomicUsize>,
    dropped_on: Arc<Mutex<Option<String>>>,
}

impl Drop for DropsOn {
    fn drop(&mut self) {
        let name = std::thread::current().name().unwrap_or("unnamed").to_owned();
        *self.dropped_on.lock().expect("not poisoned") = Some(name);
    }
}

#[test]
fn a_dropped_present_thread_runs_what_was_queued_and_drops_the_state_there() {
    let ran = Arc::new(AtomicUsize::new(0));
    let dropped_on = Arc::new(Mutex::new(None));
    let state = DropsOn { ran: ran.clone(), dropped_on: dropped_on.clone() };
    let (present, thread) = present_thread(&ClientPlatform::new(), state).expect("a thread");
    let queued: Vec<_> = (0..3)
        .map(|_| {
            present.run(|state: &mut DropsOn| {
                state.ran.fetch_add(1, Ordering::Relaxed);
            })
        })
        .collect();
    drop(thread);
    until(|| dropped_on.lock().expect("not poisoned").is_some());
    assert_eq!(dropped_on.lock().expect("not poisoned").as_deref(), Some("present"));
    assert_eq!(ran.load(Ordering::Relaxed), 3, "what was queued before the drop ran");
    for call in queued {
        wait(call).expect("a call queued before the drop ran");
    }
    let after = wait(present.run(|_| ())).expect_err("the lane is closed");
    assert_eq!(after.kind(), ErrorKind::Cancelled);
}

/// Accepts the body and drops it unrun.
struct Dropping;

impl Platform for Dropping {
    fn topology(&self) -> CoreTopology {
        StdPlatform.topology()
    }

    fn spawn_thread(
        &self,
        _: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        drop(body);
        Ok(ThreadHandle::new(|| {}))
    }
}

#[test]
fn a_platform_that_drops_the_body_makes_present_thread_cancelled() {
    let refused = present_thread(&Dropping, String::new()).err().expect("no present thread");
    assert_eq!(refused.kind(), ErrorKind::Cancelled);
}

/// A call that panics ends the thread without a hand-back: the calls behind it and the
/// stop's future resolve to `Cancelled`, and nothing hangs.
#[test]
fn a_call_that_panics_ends_the_thread_and_the_stop_is_cancelled() {
    let (present, thread) = present_thread(&ClientPlatform::new(), ()).expect("a thread");
    let panicking = present.run(|()| panic!("a call that panics, on purpose"));
    let behind = present.run(|()| ());
    let panicked = wait(panicking).expect_err("the call did not return");
    assert_eq!(panicked.kind(), ErrorKind::Cancelled);
    assert_eq!(wait(behind).expect_err("dropped unrun").kind(), ErrorKind::Cancelled);
    let stopped = wait(thread.stop()).expect_err("the thread ended without handing back");
    assert_eq!(stopped.kind(), ErrorKind::Cancelled);
}
