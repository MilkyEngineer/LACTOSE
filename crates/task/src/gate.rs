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
    pub(crate) before_park: std::sync::Mutex<Option<BeforePark>>,
}

/// The hook [`Gate::before_park`] holds: the worker's index and what its look saw.
#[cfg(all(test, not(feature = "loom")))]
pub(crate) type BeforePark = std::sync::Arc<dyn Fn(usize, Look) + Send + Sync>;

impl Gate {
    pub(crate) fn new(workers: usize) -> Self {
        Self {
            running: AtomicUsize::new(0),
            allowed: AtomicUsize::new(OPEN),
            reservations: Mutex::new(Vec::new()),
            openers: Sleep::new(workers),
            #[cfg(all(test, not(feature = "loom")))]
            before_park: std::sync::Mutex::new(None),
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
        {
            // Cloned out first: the hook may wait, and another worker may be looking too.
            let hook = gate.before_park.lock().unwrap_or_else(|poison| poison.into_inner()).clone();
            if let Some(hook) = hook {
                hook(worker, seen);
            }
        }
        sleep.park(worker, deadline);
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
            let proceed = if latch.is_set() {
                Some(false)
            } else if children_here && opener.shutting_down() {
                // Nobody else will run what is on this deque, and the gate no longer
                // applies. With nothing on the deque shutdown is no reason to stay up:
                // the children are with workers that run them before they exit, and the
                // last one's count-down unparks this worker.
                gate.retake();
                Some(true)
            } else if children_here && gate.try_acquire() {
                Some(true)
            } else {
                opener.park(latch);
                None
            };
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
