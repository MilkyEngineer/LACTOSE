//! Timers and tick pacing for the timer root (architecture sections 5.1 and 11).

use core::pin::Pin;
use core::task::{Context, Poll};
use std::time::{Duration, Instant};

/// Resolves at a deadline. See [`crate::Executor::sleep_until`].
#[must_use = "futures do nothing unless awaited"]
pub struct Sleep {
    _private: (),
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        todo!()
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
    /// When the next tick falls due.
    pub next_deadline: Instant,
}

/// Decides how many ticks are due. Pure bookkeeping over the instants it is given, so it is
/// tested without sleeping.
///
/// The tick delta is fixed; time scale changes how fast deadlines arrive, not the delta.
pub struct TickPacer {
    _private: (),
}

impl TickPacer {
    /// A pacer whose first tick falls due at `start`. `max_ticks_per_frame` is four in every
    /// shipped configuration.
    #[must_use]
    pub fn new(start: Instant, tick_period: Duration, max_ticks_per_frame: u32) -> Self {
        todo!()
    }

    /// Changes how many ticks run per real second. Zero pauses.
    pub fn set_time_scale(&mut self, scale: f64) {
        todo!()
    }

    /// The ticks due at `now`. `resimulation_ticks` are rollback re-simulation ticks already
    /// committed for this frame; they share the per-frame cap with catch-up ticks.
    pub fn poll(&mut self, now: Instant, resimulation_ticks: u32) -> TickBudget {
        todo!()
    }

    /// Forgets owed ticks and makes the next tick fall due one period after `now`.
    pub fn resync(&mut self, now: Instant) {
        todo!()
    }
}
