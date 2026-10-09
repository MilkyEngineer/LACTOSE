//! Timers and tick pacing for the timer root (architecture sections 5.1 and 11).

use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::collections::BTreeMap;
use std::sync::Weak;
use std::time::{Duration, Instant};

use lactose_core::{Error, ErrorKind, Result};

use crate::pool::Shared;
use crate::sync::{AtomicU64, Mutex, MutexGuard, Ordering};

/// The executor's pending timers.
///
/// Modelled under loom (`models.rs`, the keeper hand-off), so the state goes through `sync`.
///
/// There is no timer thread. Frame-critical workers fire due timers whenever they look for
/// work, and while all of them could be asleep one sleeper, the keeper, parks only until the
/// earliest deadline. A timer therefore fires as soon as a worker is free to notice it,
/// which is also the earliest the task it wakes could run, and that worker runs the task
/// itself (`pool::fire_timers`).
pub(crate) struct Timers {
    state: Mutex<TimerState>,
    /// The earliest deadline in nanoseconds since `epoch`, or `u64::MAX` when there is
    /// none. Lets a busy worker skip the lock.
    earliest: AtomicU64,
    epoch: Instant,
}

struct TimerState {
    pending: BTreeMap<(Instant, u64), Waker>,
    next_id: u64,
    /// The sleeping worker that watches the earliest deadline.
    keeper: Option<usize>,
}

/// Who must be woken because a new timer became the earliest.
pub(crate) enum Rearm {
    Nobody,
    Keeper(usize),
    /// No worker watches the timers yet; any sleeper will take the role.
    AnySleeper,
}

impl Timers {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(TimerState { pending: BTreeMap::new(), next_id: 0, keeper: None }),
            earliest: AtomicU64::new(u64::MAX),
            epoch: Instant::now(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TimerState> {
        self.state.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn publish_earliest(&self, state: &TimerState) {
        let earliest = state.pending.keys().next().map_or(u64::MAX, |(deadline, _)| {
            let nanos = deadline.saturating_duration_since(self.epoch).as_nanos();
            // Rounded down, so a busy worker never skips a timer that is due.
            u64::try_from(nanos).unwrap_or(u64::MAX - 1)
        });
        self.earliest.store(earliest, Ordering::Relaxed);
    }

    /// Adds or re-arms the timer `id` (allocated on first use) with `waker`.
    pub(crate) fn register(&self, deadline: Instant, id: &mut Option<u64>, waker: &Waker) -> Rearm {
        let mut state = self.lock();
        let id = *id.get_or_insert_with(|| {
            state.next_id += 1;
            state.next_id
        });
        let was_earliest = state.pending.keys().next().copied();
        state.pending.insert((deadline, id), waker.clone());
        self.publish_earliest(&state);
        if was_earliest.is_some_and(|earliest| earliest <= (deadline, id)) {
            Rearm::Nobody
        } else {
            state.keeper.map_or(Rearm::AnySleeper, Rearm::Keeper)
        }
    }

    fn remove(&self, deadline: Instant, id: u64) {
        let mut state = self.lock();
        state.pending.remove(&(deadline, id));
        self.publish_earliest(&state);
    }

    /// Wakes every timer whose deadline had passed when it was called.
    ///
    /// The timers are taken out one at a time, which allocates nothing. This runs between a
    /// tick deadline and the task that waits for it: splitting the due timers off as a map
    /// of their own allocated a node and freed another on every tick, which on an M1 cost
    /// 3 µs at the median and 21 µs at P99.
    pub(crate) fn fire_due(&self) {
        let earliest = self.earliest.load(Ordering::Relaxed);
        let now = Instant::now();
        if earliest == u64::MAX
            || now.saturating_duration_since(self.epoch).as_nanos() < u128::from(earliest)
        {
            return;
        }
        loop {
            let waker = {
                let mut state = self.lock();
                let due = state.pending.first_entry().filter(|timer| timer.key().0 <= now);
                let waker = due.map(|timer| timer.remove());
                self.publish_earliest(&state);
                waker
            };
            // Outside the lock: a waker queues a task, or does whatever a foreign waker does.
            match waker {
                Some(waker) => waker.wake(),
                None => return,
            }
        }
    }

    /// Makes `worker`, which is about to sleep, the keeper if there is none, and returns the
    /// deadline it must wake at.
    ///
    /// Registration and this run under the same lock, so either the registration sees this
    /// keeper and wakes it, or the keeper sees the registered deadline.
    pub(crate) fn keep(&self, worker: usize) -> Option<Instant> {
        let mut state = self.lock();
        if state.keeper.is_some() {
            return None;
        }
        let (deadline, _) = *state.pending.keys().next()?;
        state.keeper = Some(worker);
        Some(deadline)
    }

    /// Ends `worker`'s turn as keeper, if it was. Returns whether it was and deadlines
    /// remain, in which case another sleeper has to take over.
    pub(crate) fn release(&self, worker: usize) -> bool {
        let mut state = self.lock();
        if state.keeper != Some(worker) {
            return false;
        }
        state.keeper = None;
        !state.pending.is_empty()
    }

    /// Wakes every pending timer early. Called at shutdown, after the flag is set, so
    /// each [`Sleep`] that is polled again resolves to `Cancelled`.
    pub(crate) fn cancel_all(&self) {
        let pending = {
            let mut state = self.lock();
            let pending = core::mem::take(&mut state.pending);
            self.publish_earliest(&state);
            pending
        };
        for waker in pending.into_values() {
            waker.wake();
        }
    }
}

/// Resolves at a deadline, or to an error of kind `Cancelled` if the executor shuts down
/// first. See [`crate::Executor::sleep_until`].
#[must_use = "futures do nothing unless awaited"]
pub struct Sleep {
    shared: Weak<Shared>,
    deadline: Instant,
    /// The timer's identity once it has been registered.
    id: Option<u64>,
}

impl Sleep {
    pub(crate) fn new(shared: Weak<Shared>, deadline: Instant) -> Self {
        Self { shared, deadline, id: None }
    }
}

impl Future for Sleep {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let cancelled = || {
            Poll::Ready(Err(Error::new(
                ErrorKind::Cancelled,
                "the executor shut down before the deadline",
            )))
        };
        let Some(shared) = self.shared.upgrade() else { return cancelled() };
        let deadline = self.deadline;
        if shared.is_shut_down() {
            return cancelled();
        }
        if Instant::now() >= deadline {
            if let Some(id) = self.id.take() {
                shared.timers.remove(deadline, id);
            }
            return Poll::Ready(Ok(()));
        }
        let rearm = shared.timers.register(deadline, &mut self.id, cx.waker());
        if shared.is_shut_down() {
            // Registered first, checked second: either shutdown's sweep wakes this timer,
            // or this sees the flag.
            if let Some(id) = self.id.take() {
                shared.timers.remove(deadline, id);
            }
            return cancelled();
        }
        match rearm {
            Rearm::Nobody => {}
            Rearm::Keeper(worker) => shared.frame().wake(worker),
            Rearm::AnySleeper => shared.frame().notify_one(),
        }
        Poll::Pending
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let (Some(id), Some(shared)) = (self.id, self.shared.upgrade()) {
            shared.timers.remove(self.deadline, id);
        }
    }
}

/// What the tick loop may do this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TickBudget {
    /// New simulation ticks to run now, after the cap was applied.
    pub ticks: u32,
    /// The cap was hit with ticks still owed. A client re-syncs its clock with
    /// [`TickPacer::resync`]; a server climbs the overload ladder. Neither slows time.
    pub behind: bool,
    /// When the next tick falls due. While paused this is one tick period ahead, so the
    /// loop keeps polling at tick rate.
    pub next_deadline: Instant,
}

/// Decides how many ticks are due. Pure bookkeeping over the instants it is given, so it is
/// tested without sleeping.
///
/// The tick delta is fixed; time scale changes how fast deadlines arrive, not the delta.
pub struct TickPacer {
    period: Duration,
    max_ticks_per_frame: u32,
    scale: f64,
    /// The instant up to which elapsed time has been turned into owed simulation time.
    accounted: Instant,
    /// Simulation time owed and not yet run. A tick is due for every full period.
    owed: Duration,
}

/// Below this the pacer treats the time scale as paused, so a deadline never lands
/// centuries away.
const MIN_TIME_SCALE: f64 = 1e-6;

impl TickPacer {
    /// A pacer whose first tick falls due at `start`. `max_ticks_per_frame` is four in every
    /// shipped configuration.
    ///
    /// # Panics
    ///
    /// Panics if `tick_period` is zero.
    #[must_use]
    pub fn new(start: Instant, tick_period: Duration, max_ticks_per_frame: u32) -> Self {
        assert!(!tick_period.is_zero(), "the tick period must be positive");
        Self {
            period: tick_period,
            max_ticks_per_frame,
            scale: 1.0,
            accounted: start,
            owed: tick_period,
        }
    }

    /// Changes how many ticks run per real second. Zero pauses. Takes effect from the next
    /// [`TickPacer::poll`]; time since the previous poll is counted at the new scale.
    ///
    /// # Panics
    ///
    /// Panics if `scale` is negative or not finite.
    pub fn set_time_scale(&mut self, scale: f64) {
        assert!(scale.is_finite() && scale >= 0.0, "time scale must be finite and non-negative");
        self.scale = if scale < MIN_TIME_SCALE { 0.0 } else { scale };
    }

    /// The ticks due at `now`. `resimulation_ticks` are rollback re-simulation ticks already
    /// committed for this frame; they share the per-frame cap with catch-up ticks.
    pub fn poll(&mut self, now: Instant, resimulation_ticks: u32) -> TickBudget {
        if now < self.accounted {
            // Before the first tick.
            return TickBudget { ticks: 0, behind: false, next_deadline: self.accounted };
        }
        let paused = self.scale == 0.0;
        if !paused {
            self.owed += (now - self.accounted).mul_f64(self.scale);
        }
        self.accounted = now;

        let due = u32::try_from(self.owed.as_nanos() / self.period.as_nanos()).unwrap_or(u32::MAX);
        let ticks = due.min(self.max_ticks_per_frame.saturating_sub(resimulation_ticks));
        self.owed -= self.period * ticks;
        let behind = ticks < due;
        let next_deadline = if behind {
            now
        } else if paused {
            now + self.period
        } else {
            now + (self.period - self.owed).div_f64(self.scale)
        };
        TickBudget { ticks, behind, next_deadline }
    }

    /// Forgets owed ticks and makes the next tick fall due one period after `now`.
    pub fn resync(&mut self, now: Instant) {
        self.accounted = now;
        self.owed = Duration::ZERO;
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    const PERIOD: Duration = Duration::from_millis(10);

    fn pacer(start: Instant) -> TickPacer {
        TickPacer::new(start, PERIOD, 4)
    }

    #[test]
    fn first_tick_is_due_at_start_and_then_every_period() {
        let start = Instant::now() + Duration::from_secs(1);
        let mut pacer = pacer(start);
        let early = pacer.poll(start - PERIOD, 0);
        assert_eq!(early, TickBudget { ticks: 0, behind: false, next_deadline: start });
        let first = pacer.poll(start, 0);
        assert_eq!(first, TickBudget { ticks: 1, behind: false, next_deadline: start + PERIOD });
        let between = pacer.poll(start + PERIOD / 2, 0);
        assert_eq!(between, TickBudget { ticks: 0, behind: false, next_deadline: start + PERIOD });
        let third = pacer.poll(start + PERIOD * 2, 0);
        assert_eq!(third.ticks, 2);
        assert_eq!(third.next_deadline, start + PERIOD * 3);
    }

    #[test]
    fn catch_up_is_capped_and_shares_the_cap_with_resimulation() {
        let start = Instant::now();
        let mut pacer = pacer(start);
        // Eleven ticks owed: the first plus ten periods.
        let now = start + PERIOD * 10;
        let capped = pacer.poll(now, 0);
        assert_eq!(capped, TickBudget { ticks: 4, behind: true, next_deadline: now });
        let shared = pacer.poll(now, 3);
        assert_eq!((shared.ticks, shared.behind), (1, true));
        let none = pacer.poll(now, 9);
        assert_eq!((none.ticks, none.behind), (0, true));
        // Time is never slowed: the debt stays until it is run or forgotten.
        assert_eq!(pacer.poll(now, 0).ticks, 4);
        let last = pacer.poll(now, 0);
        assert_eq!(last, TickBudget { ticks: 2, behind: false, next_deadline: now + PERIOD });
    }

    #[test]
    fn resync_forgets_the_debt() {
        let start = Instant::now();
        let mut pacer = pacer(start);
        let now = start + PERIOD * 100;
        assert!(pacer.poll(now, 0).behind);
        pacer.resync(now);
        let after = pacer.poll(now, 0);
        assert_eq!(after, TickBudget { ticks: 0, behind: false, next_deadline: now + PERIOD });
        assert_eq!(pacer.poll(now + PERIOD, 0).ticks, 1);
    }

    #[test]
    fn time_scale_changes_the_rate_and_zero_pauses() {
        let start = Instant::now();
        let mut pacer = pacer(start);
        assert_eq!(pacer.poll(start, 0).ticks, 1);
        pacer.set_time_scale(2.0);
        let fast = pacer.poll(start + PERIOD, 0);
        assert_eq!(fast.ticks, 2);
        assert_eq!(fast.next_deadline, start + PERIOD + PERIOD / 2);

        pacer.set_time_scale(0.0);
        let paused_at = start + PERIOD * 50;
        let paused = pacer.poll(paused_at, 0);
        assert_eq!(
            paused,
            TickBudget { ticks: 0, behind: false, next_deadline: paused_at + PERIOD }
        );

        pacer.set_time_scale(0.5);
        let slow = pacer.poll(paused_at + PERIOD, 0);
        assert_eq!(slow.ticks, 0);
        assert_eq!(slow.next_deadline, paused_at + PERIOD * 2);
        assert_eq!(pacer.poll(paused_at + PERIOD * 2, 0).ticks, 1);
    }
}
