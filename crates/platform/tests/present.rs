//! The present thread with no window (`README.md`, "Proof"): its spec, a call that blocks,
//! the stop under a queuing task, the calls after the hand-back, a second present thread
//! on one platform thread, the drop, a platform that drops the body and a call that
//! panics. Miri runs this file.

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::task::Wake;
use std::time::{Duration, Instant};

use lactose_core::{ErrorKind, Result, ThreadClass};
use lactose_platform::{ClientPlatform, PresentThread, present_thread};
use lactose_task::{
    CoreTopology, Executor, LaneCall, Platform, StdPlatform, ThreadHandle, ThreadSpec, WorkerClass,
    yield_now,
};

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
    let entered = Arc::new(Barrier::new(2));
    let (release, released) = mpsc::channel::<()>();
    let blocking = {
        let entered = entered.clone();
        present.run(move |order: &mut Vec<&str>| {
            order.push("blocking");
            entered.wait();
            released.recv_timeout(BOUND).expect("the test releases the call");
        })
    };
    let behind_ran = Arc::new(AtomicBool::new(false));
    let behind = {
        let behind_ran = behind_ran.clone();
        present.run(move |order: &mut Vec<&str>| {
            behind_ran.store(true, Ordering::Relaxed);
            order.push("behind");
        })
    };
    entered.wait();
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
        let entered = Arc::new(Barrier::new(2));
        let (release, released) = mpsc::channel::<()>();
        let parking = present.run({
            let entered = entered.clone();
            move |_: &mut u32| {
                entered.wait();
                released.recv_timeout(BOUND).expect("the test releases the call");
            }
        });
        entered.wait();
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

/// What the queuing task and the caller of `stop` share: the calls queued, in order, and
/// how many had been queued when `stop` was called.
struct Queued {
    calls: Vec<LaneCall<()>>,
    at_stop: Option<usize>,
}

/// What one round saw: how many calls queued after `stop` ran, and how many came back
/// `Cancelled`.
struct Round {
    ran_after: usize,
    cancelled: usize,
}

/// One round: a task queues calls in a loop, each recording its index in the state, while
/// the test calls `stop` once `before` calls are queued. Holding the lock that each queue
/// takes, the test reads which calls were queued before `stop`.
fn stop_under_a_queuing_task(executor: &Executor, before: usize, after: usize) -> Round {
    let (present, thread) = present_thread(&ClientPlatform::new(), Vec::new()).expect("a thread");
    let queued = Arc::new(Mutex::new(Queued { calls: Vec::new(), at_stop: None }));
    let queuing = {
        let queued = queued.clone();
        executor.spawn(WorkerClass::Background, async move {
            loop {
                {
                    let mut queued = queued.lock().expect("no test panics with it held");
                    let index = queued.calls.len();
                    if queued.at_stop.is_some_and(|at| index >= at + after) {
                        return;
                    }
                    let call = present.run(move |ran: &mut Vec<usize>| ran.push(index));
                    queued.calls.push(call);
                }
                yield_now().await;
            }
        })
    };
    until(|| queued.lock().expect("no test panics with it held").calls.len() >= before);
    let stopped = {
        let mut queued = queued.lock().expect("no test panics with it held");
        let stopped = thread.stop();
        queued.at_stop = Some(queued.calls.len());
        stopped
    };
    let ran = wait(stopped).expect("the hand-back gives the state in every round");
    wait(queuing).expect("the queuing task ends");
    let Queued { calls, at_stop } =
        Arc::into_inner(queued).expect("the task let go").into_inner().expect("not poisoned");
    let at_stop = at_stop.expect("stop was called");

    // The calls ran in the order they were queued, every one queued before `stop` among
    // them, and those queued after it that ran are the first of them: the last drain's.
    assert!(ran.len() >= at_stop, "a call queued before stop did not run: {ran:?}");
    assert!(ran.iter().enumerate().all(|(position, &index)| position == index), "{ran:?}");
    // A call queued after `stop` ran in the last drain or is `Cancelled`, and none ran
    // after the hand-back: the state handed back holds every call that ran.
    let mut cancelled = 0;
    for (index, call) in calls.into_iter().enumerate() {
        match wait(call) {
            Ok(()) => assert!(index < ran.len(), "call {index} ran after the hand-back"),
            Err(error) => {
                assert_eq!(error.kind(), ErrorKind::Cancelled);
                assert!(index >= ran.len(), "call {index} ran and was cancelled");
                assert!(index >= at_stop, "call {index}, queued before stop, was cancelled");
                cancelled += 1;
            }
        }
    }
    Round { ran_after: ran.len() - at_stop, cancelled }
}

#[test]
fn stop_runs_every_call_queued_before_it_and_hands_the_state_back() {
    let executor = executor();
    let (rounds, after) = if cfg!(miri) { (6, 4) } else { (2_000, 32) };
    let (mut ran_after, mut cancelled, mut played) = (0, 0, 0);
    for round in 0..rounds {
        let seen = stop_under_a_queuing_task(&executor, 1 + round % 5, after);
        ran_after += usize::from(seen.ran_after > 0);
        cancelled += usize::from(seen.cancelled > 0);
        played += 1;
        // Natively the rounds go on until both outcomes have occurred, within the cap.
        if ran_after > 0 && cancelled > 0 && !cfg!(miri) && round >= 20 {
            break;
        }
    }
    println!("{played} rounds: {ran_after} ran a later call, {cancelled} cancelled one");
    if !cfg!(miri) {
        assert!(ran_after > 0, "no round ran a call queued after stop in its last drain");
        assert!(cancelled > 0, "no round cancelled a call queued after stop");
    }
    executor.shutdown();
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
