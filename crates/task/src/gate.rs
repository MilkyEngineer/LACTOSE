//! Run permits and the `block_in_place` gate, and the wait of a scope's opener, which is
//! where the two meet.
//!
//! Modelled under loom (`models.rs`), so everything here goes through `sync`.
//!
//! A worker takes jobs only while it holds a permit. Normally permits are unlimited. While a
//! `block_in_place` runs, the gate is closed: at most its reservation may be out, and workers
//! over the limit give theirs up at the next unit boundary. Whoever gives a permit up while
//! the gate is closed tells those who could use it. There are two kinds: workers asleep in
//! the idle protocol, which the pool wakes, and openers parked on their scope latch with
//! children still on their own deque, which announce themselves here.

use crate::latch::Latch;
use crate::sleep::Sleep;
use crate::span::TARGET;
use std::time::Instant;

use crate::sync::{AtomicUsize, Mutex, Ordering};

/// No limit: the gate is open.
const OPEN: usize = usize::MAX;

/// How the gate reaches threads. The pool implements it; a loom model supplies its own.
pub(crate) trait Wake {
    /// Wakes one worker asleep in the idle protocol, if there is one.
    fn idle_one(&self);
    /// Unparks `worker`, which is parked on a scope latch.
    fn unpark(&self, worker: usize);
}

/// The permits of one pool.
pub(crate) struct Gate {
    /// Permits out. A worker that is asleep, parked on a latch or blocked holds none.
    running: AtomicUsize,
    /// The most permits that may be out, or [`OPEN`].
    allowed: AtomicUsize,
    /// The reservations of the `block_in_place` calls in progress.
    reservations: Mutex<Vec<usize>>,
    /// Openers parked on their latch without a permit while children of their scope sit on
    /// their own deque. Only they can be sure to run those children, so a freed permit must
    /// reach them. Uses the sleep protocol's announce, re-check, park discipline.
    openers: Sleep,
    /// Called by [`idle_park`] on a worker that has looked round and is about to park, with
    /// what it saw: the window between a worker's check of the gate and its sleep, where the
    /// tests of a task woken at a closed gate land a wake or the gate's opening. Each pool
    /// has its own, so a test that sets it reaches no other test's executor.
    #[cfg(all(test, not(feature = "loom")))]
    pub(crate) before_park: Hook<BeforePark>,
}

/// The hook [`Gate::before_park`] holds: the worker's index and what its look saw.
#[cfg(all(test, not(feature = "loom")))]
pub(crate) type BeforePark = std::sync::Arc<dyn Fn(usize, Look) + Send + Sync>;

/// A test hook a worker reaches on its hot path. Unset, reaching it is one `Relaxed` load
/// and no lock, so a test that sets no hook runs with no synchronisation between workers
/// that the shipped build lacks. A test sets one before the wake, queueing or timer that
/// brings the worker to it, which orders the setting before the worker's reach.
#[cfg(all(test, not(feature = "loom")))]
pub(crate) struct Hook<F> {
    set: core::sync::atomic::AtomicBool,
    hook: std::sync::Mutex<Option<F>>,
}

#[cfg(all(test, not(feature = "loom")))]
impl<F: Clone> Hook<F> {
    pub(crate) const fn new() -> Self {
        Self { set: core::sync::atomic::AtomicBool::new(false), hook: std::sync::Mutex::new(None) }
    }

    /// Sets the hook, or with `None` clears it.
    pub(crate) fn set(&self, hook: Option<F>) {
        use core::sync::atomic::Ordering::Relaxed;
        let set = hook.is_some();
        if !set {
            self.set.store(false, Relaxed);
        }
        *crate::lock(&self.hook) = hook;
        if set {
            self.set.store(true, Relaxed);
        }
    }

    /// The hook, cloned out so that it may wait while another worker reaches it too.
    pub(crate) fn get(&self) -> Option<F> {
        if !self.set.load(core::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        crate::lock(&self.hook).clone()
    }
}

impl Gate {
    pub(crate) fn new(workers: usize) -> Self {
        Self {
            running: AtomicUsize::new(0),
            allowed: AtomicUsize::new(OPEN),
            reservations: Mutex::new(Vec::new()),
            openers: Sleep::new(workers),
            #[cfg(all(test, not(feature = "loom")))]
            before_park: Hook::new(),
        }
    }

    /// The permits out and the limit, `None` while the gate is open.
    #[cfg(all(test, not(feature = "loom")))]
    pub(crate) fn permits(&self) -> (usize, Option<usize>) {
        let allowed = self.allowed.load(Ordering::SeqCst);
        (self.running.load(Ordering::SeqCst), (allowed != OPEN).then_some(allowed))
    }

    fn closed(&self) -> bool {
        self.allowed.load(Ordering::SeqCst) != OPEN
    }

    /// Whether a permit could be taken right now.
    pub(crate) fn permit_available(&self) -> bool {
        self.running.load(Ordering::SeqCst) < self.allowed.load(Ordering::SeqCst)
    }

    /// Takes a permit if the limit allows one more.
    pub(crate) fn try_acquire(&self) -> bool {
        let mut running = self.running.load(Ordering::SeqCst);
        loop {
            if running >= self.allowed.load(Ordering::SeqCst) {
                return false;
            }
            match self.running.compare_exchange_weak(
                running,
                running + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(actual) => running = actual,
            }
        }
    }

    /// Takes a permit regardless of the limit, for a worker that goes on with the job it
    /// is in the middle of.
    pub(crate) fn retake(&self) {
        self.running.fetch_add(1, Ordering::SeqCst);
    }

    /// Gives a permit up. While the gate is closed that is news: an idle worker or an
    /// opener waiting at the gate can use it.
    pub(crate) fn release(&self, wake: &impl Wake) {
        self.running.fetch_sub(1, Ordering::SeqCst);
        if self.closed() {
            self.announce_free_permit(wake);
        }
    }

    fn announce_free_permit(&self, wake: &impl Wake) {
        wake.idle_one();
        self.openers.notify_all(|worker| wake.unpark(worker));
    }

    /// Gives up the permit of a worker that has just scanned every queue, found nothing
    /// and is about to sleep through [`idle_park`]. Unlike [`Gate::release`] this wakes no
    /// idle worker, which would only take the permit, scan, find nothing and pass it on:
    /// under a closed gate with nothing to do, the pool would hand one busy core round for
    /// as long as the gate stayed closed.
    ///
    /// No wake-up is lost by that. A sleeping worker needs waking only when there is work
    /// and a permit for it. Work that was queued before this release is seen by the
    /// releasing worker itself: its next step is `idle_park`, whose re-check runs after
    /// the permit is free, finds the work and a permit, and keeps the worker awake. Work
    /// that is queued after that re-check is announced by whoever queues it, to a sleeper
    /// that now finds the permit free. Openers waiting at the gate are told, because their
    /// children are work this worker may not have been able to see as its own.
    pub(crate) fn release_idle(&self, wake: &impl Wake) {
        self.running.fetch_sub(1, Ordering::SeqCst);
        if self.closed() {
            self.openers.notify_all(|worker| wake.unpark(worker));
        }
    }

    /// Whether more permits are out than the limit allows, so that some worker has to stop
    /// taking units.
    pub(crate) fn over_limit(&self) -> bool {
        let allowed = self.allowed.load(Ordering::Relaxed);
        allowed != OPEN && self.running.load(Ordering::Relaxed) > allowed
    }

    /// Gives the calling worker's permit up if more are out than the limit allows. Returns
    /// whether it did. No permit becomes free by this, so nobody is told.
    pub(crate) fn release_if_over(&self) -> bool {
        let allowed = self.allowed.load(Ordering::SeqCst);
        if allowed == OPEN {
            return false;
        }
        let mut running = self.running.load(Ordering::SeqCst);
        while running > allowed {
            match self.running.compare_exchange_weak(
                running,
                running - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(actual) => running = actual,
            }
        }
        false
    }

    fn set_limit(&self, change: impl FnOnce(&mut Vec<usize>)) {
        let mut reservations =
            self.reservations.lock().unwrap_or_else(|poison| poison.into_inner());
        change(&mut reservations);
        let allowed = reservations.iter().copied().min().unwrap_or(OPEN);
        self.allowed.store(allowed, Ordering::SeqCst);
    }

    /// Starts a `block_in_place`: at most `reserved` workers other than the caller may hold
    /// permits. The caller blocks and gives its own up.
    pub(crate) fn close(&self, reserved: usize, wake: &impl Wake) {
        self.set_limit(|reservations| reservations.push(reserved));
        self.release(wake);
    }

    /// Ends a `block_in_place` started with the same `reserved`. The caller takes its
    /// permit back and then wakes everyone the gate was holding, idle workers and openers
    /// alike, through the pool.
    pub(crate) fn open(&self, reserved: usize, wake: &impl Wake) {
        self.set_limit(|reservations| {
            let at = reservations.iter().position(|&held| held == reserved);
            reservations.swap_remove(at.expect("the gate was closed with this reservation"));
        });
        self.retake();
        self.openers.notify_all(|worker| wake.unpark(worker));
    }
}

/// What a worker that is about to sleep found when it looked round one last time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Look {
    /// A reason to stay awake whatever the gate says: the executor is shutting down.
    Stop,
    /// There is work some worker with a permit could take.
    Work,
    /// Nothing.
    Nothing,
}

/// Puts an idle worker, which holds no permit, to sleep: announce, look again, park.
///
/// `look` runs after the announcement, which is what makes it a re-check: work queued
/// before it is seen, work queued after it finds the announcement. It also returns the
/// deadline to wake at, if the worker keeps a timer. Seeing work is a reason to stay awake
/// only if a permit can be had; a worker held at the gate sleeps beside work, and the gate
/// wakes it when it opens.
pub(crate) fn idle_park(
    gate: &Gate,
    sleep: &Sleep,
    worker: usize,
    look: impl FnOnce() -> (Look, Option<Instant>),
) {
    sleep.announce(worker);
    let (seen, deadline) = look();
    let stay_awake = match seen {
        Look::Stop => true,
        Look::Work => gate.permit_available(),
        Look::Nothing => false,
    };
    if stay_awake {
        sleep.cancel(worker);
    } else {
        #[cfg(all(test, not(feature = "loom")))]
        if let Some(hook) = gate.before_park.get() {
            hook(worker, seen);
        }
        // After the announcement: a listener's callbacks run on a worker that a waker may
        // already have claimed, and one that waits through the thread's parker may take the
        // waker's unpark. `Sleep::park` parks only while its bit says nobody has claimed the
        // worker, so that delays the wake and loses nothing.
        let _park = tracing::debug_span!(target: TARGET, "park", wait = "idle").entered();
        sleep.park(worker, deadline);
    }
}

/// The re-check of [`wait_for_children`] before the opener parks: `Some(false)` if the latch
/// is set, `Some(true)` holding a permit if the opener should run its children, `None` if it
/// should park.
fn recheck(latch: &Latch, gate: &Gate, children_here: bool, opener: &impl Opener) -> Option<bool> {
    if latch.is_set() {
        Some(false)
    } else if children_here && opener.shutting_down() {
        // Nobody else will run what is on this deque, and the gate no longer applies. With
        // nothing on the deque shutdown is no reason to stay up: the children are with
        // workers that run them before they exit, and the last one's count-down unparks this
        // worker.
        gate.retake();
        Some(true)
    } else if children_here && gate.try_acquire() {
        Some(true)
    } else {
        None
    }
}

/// What the opener's wait needs from the worker it runs on.
pub(crate) trait Opener {
    /// Runs this scope's children from the worker's own deque for as long as the gate lets
    /// the worker work. Returns whether it stopped because of the gate with children
    /// possibly still on the deque.
    fn run_children(&mut self) -> bool;
    /// Whether the executor is shutting down, in which case nobody else will run what is
    /// on this worker's deque and the gate no longer applies.
    fn shutting_down(&self) -> bool;
    /// Spins briefly on the latch before the first park. Returns whether it is set.
    fn spin(&mut self, latch: &Latch) -> bool;
    /// Whether, after a wake, a child of this scope may be waiting where this opener takes
    /// its children from. Asked after every wake that leaves the scope unfinished, when
    /// [`Opener::run_children`] last found none left. An opener whose children wait on its
    /// own deque answers `false`: only it pushes there, so none can have come. One whose
    /// children wait on the pool's queue for such work answers whether that queue holds
    /// any, since the half another worker splits off or puts back lands there and wakes it.
    fn look(&mut self) -> bool;
    /// Parks the opener on `latch`.
    fn park(&mut self, latch: &Latch) {
        latch.park();
    }
}

/// The wait of a scope's opener: run the children that wait for this worker, then park on
/// `latch` until the ones other workers took are done. A scope's children wait on its
/// opener's deque, or for a scope of a job of `Executor::run_job` on the pool's queue for
/// such work, where one can be queued after the opener parked; such an opener looks there
/// again after every wake before it parks again ([`Opener::look`]).
///
/// The opener parks without its permit, so a worker that runs nothing never uses up a
/// `block_in_place` reservation. If the gate stopped it with children still on its deque, it
/// announces itself before it parks; a permit that is given up afterwards unparks it, and
/// one given up before is found by the re-check. Without that, two openers that both gave
/// way to the gate would park beside their own children with the reserved permit unused.
///
/// The caller holds a permit on entry and holds one again on return.
pub(crate) fn wait_for_children(
    latch: &Latch,
    gate: &Gate,
    worker: usize,
    wake: &impl Wake,
    opener: &mut impl Opener,
) {
    let mut spun = false;
    loop {
        let mut children_here = opener.run_children();
        if latch.is_set() {
            return;
        }
        if !core::mem::replace(&mut spun, true) && opener.spin(latch) {
            return;
        }

        gate.release(wake);
        loop {
            if children_here {
                gate.openers.announce(worker);
            }
            // The re-check. A latch that is set, a permit, or shutdown each end the park
            // before it starts; whichever of them happens later comes with an unpark.
            let mut proceed = recheck(latch, gate, children_here, opener);
            if proceed.is_none() {
                // A listener's callbacks run between the re-check and the park, and one that
                // waits through this thread's parker takes the token an unpark sent meanwhile
                // left: the set latch, freed permit or shutdown it came with would be missed,
                // and the opener would sleep beside it for good. So the re-check runs again
                // once they have.
                let _park = tracing::debug_span!(target: TARGET, "park", wait = "latch").entered();
                proceed = recheck(latch, gate, children_here, opener);
                if proceed.is_none() {
                    opener.park(latch);
                }
            }
            if children_here {
                gate.openers.cancel(worker);
            }
            match proceed {
                Some(false) => {
                    gate.retake();
                    return;
                }
                Some(true) => break,
                None => {}
            }
            if latch.is_set() {
                gate.retake();
                return;
            }
            // Woken with the scope unfinished: by a gate opening, a freed permit, shutdown,
            // a child queued where this opener takes them from, or a stale token. Only
            // children waiting for this opener are a reason to go round again; without
            // any, park until the last child's count-down or the next wake.
            if !children_here {
                children_here = opener.look();
                if !children_here {
                    continue;
                }
            }
            if opener.shutting_down() {
                gate.retake();
                break;
            }
            if gate.try_acquire() {
                break;
            }
        }
    }
}

/// The opener's wait against a tracing listener that takes the thread's park token: each of
/// the three reasons that end the wait with an unpark, landing between the re-check and the
/// park, where the latch `park` span's callbacks run.
#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::thread::Thread;
    use std::time::Duration;
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    /// How long the opener's park waits for a wake-up before it looks again on its own: long
    /// enough to tell one from none on a loaded machine.
    const BOUND: Duration = Duration::from_secs(2);

    /// The pool as the gate sees it: the opener, worker 0, on the test's thread.
    struct Opened(Thread);

    impl Wake for Opened {
        fn idle_one(&self) {}

        fn unpark(&self, worker: usize) {
            assert_eq!(worker, 0, "the opener is the only worker");
            self.0.unpark();
        }
    }

    /// An opener with one child: on its own deque, where the gate stops it and which it
    /// runs once it may go on, or with another worker, which counts it down.
    struct OneChild<'a> {
        latch: &'a Latch,
        shutdown: &'a AtomicBool,
        on_deque: bool,
        stopped: bool,
        /// How long each park lasted.
        parks: Vec<Duration>,
    }

    impl Opener for OneChild<'_> {
        fn run_children(&mut self) -> bool {
            if !self.on_deque {
                return false;
            }
            if !core::mem::replace(&mut self.stopped, true) {
                return true;
            }
            self.on_deque = false;
            assert!(self.latch.count_down(1), "the scope's one child");
            false
        }

        fn shutting_down(&self) -> bool {
            self.shutdown.load(Ordering::SeqCst)
        }

        fn spin(&mut self, _latch: &Latch) -> bool {
            false
        }

        fn look(&mut self) -> bool {
            false
        }

        fn park(&mut self, _latch: &Latch) {
            let begun = Instant::now();
            std::thread::park_timeout(BOUND);
            self.parks.push(begun.elapsed());
        }
    }

    /// Listens to the latch `park`. Its `new_span` brings about `event`, which ends the wait
    /// and unparks the opener, as another worker would; its `enter` then waits through the
    /// thread's parker, which takes that unpark's token.
    struct TakesTheToken<F>(std::sync::Mutex<Option<F>>);

    impl<F: FnOnce() + Send + 'static> Subscriber for TakesTheToken<F> {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.name() == "park"
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            if let Some(event) = self.0.lock().unwrap().take() {
                event();
            }
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}
        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
        fn event(&self, _event: &Event<'_>) {}

        fn enter(&self, _span: &Id) {
            std::thread::park_timeout(Duration::from_millis(1));
        }

        fn exit(&self, _span: &Id) {}
    }

    /// Runs the opener's wait for a scope of one child under [`TakesTheToken`] with `event`,
    /// and returns how long each of its parks lasted.
    fn wait_through(
        gate: &Gate,
        latch: &Latch,
        shutdown: &AtomicBool,
        on_deque: bool,
        event: impl FnOnce() + Send + 'static,
    ) -> Vec<Duration> {
        let wake = Opened(std::thread::current());
        let mut opener = OneChild { latch, shutdown, on_deque, stopped: false, parks: Vec::new() };
        let listener = TakesTheToken(std::sync::Mutex::new(Some(event)));
        tracing::subscriber::with_default(listener, || {
            wait_for_children(latch, gate, 0, &wake, &mut opener);
        });
        assert!(latch.is_set(), "the wait ended with the scope's child done");
        opener.parks
    }

    #[test]
    fn a_listener_that_takes_the_last_childs_unpark_costs_no_wake_up() {
        let (gate, latch) = (Gate::new(1), Arc::new(Latch::new()));
        latch.add(1);
        assert!(gate.try_acquire(), "the opener's permit");
        let event = {
            let (latch, opener) = (latch.clone(), Opened(std::thread::current()));
            move || {
                assert!(latch.count_down(1), "the child with another worker is the last");
                opener.unpark(0);
            }
        };
        let parks = wait_through(&gate, &latch, &AtomicBool::new(false), false, event);
        assert!(parks.is_empty(), "the opener parked on a set latch for {parks:?}");
        assert_eq!(gate.permits(), (1, None), "the opener holds its permit again");
    }

    #[test]
    fn a_listener_that_takes_a_freed_permits_unpark_costs_no_wake_up() {
        let (gate, latch) = (Arc::new(Gate::new(1)), Latch::new());
        latch.add(1);
        // The opener's, another worker's and that of a `block_in_place` reserving one, which
        // gives its own up as it closes the gate.
        for _ in 0..3 {
            assert!(gate.try_acquire());
        }
        gate.close(1, &Opened(std::thread::current()));
        let event = {
            let (gate, opener) = (gate.clone(), Opened(std::thread::current()));
            move || gate.release(&opener)
        };
        let parks = wait_through(&gate, &latch, &AtomicBool::new(false), true, event);
        assert!(
            parks.is_empty(),
            "the opener parked beside its child and a free permit for {parks:?}"
        );
        assert_eq!(gate.permits(), (1, Some(1)), "the opener holds the reserved permit");
    }

    #[test]
    fn a_listener_that_takes_shutdowns_unpark_costs_no_wake_up() {
        let (gate, latch, shutdown) =
            (Gate::new(1), Latch::new(), Arc::new(AtomicBool::new(false)));
        latch.add(1);
        // The opener's and that of a `block_in_place` reserving none.
        for _ in 0..2 {
            assert!(gate.try_acquire());
        }
        gate.close(0, &Opened(std::thread::current()));
        let event = {
            let (shutdown, opener) = (shutdown.clone(), Opened(std::thread::current()));
            move || {
                shutdown.store(true, Ordering::SeqCst);
                opener.unpark(0);
            }
        };
        let parks = wait_through(&gate, &latch, &shutdown, true, event);
        assert!(
            parks.is_empty(),
            "the opener parked beside its child after shutdown for {parks:?}"
        );
        assert_eq!(gate.permits(), (1, Some(0)), "the opener took a permit back past the limit");
    }
}
