//! Time as the simulation sees it (architecture section 11): a fixed delta, the tick being
//! simulated, and the time scale the server decides, mirrored into the world as an input.

use lactose_core::Tick;
use lactose_math::Scalar;
use lactose_reflect::resource;

/// The fixed simulation rate in ticks per second (architecture section 2). The delta is its
/// reciprocal and never changes; [`TimeScale`] changes how often ticks run, never the delta.
pub const TICK_RATE_HZ: u32 = 60;

/// The fixed tick delta in seconds, as every system that integrates over time reads it
/// (`Res<TickDelta>`, architecture section 6.1).
///
/// [`SimulationBuilder`](crate::SimulationBuilder) inserts it as `1 / TICK_RATE_HZ`,
/// computed through `lactose_math`, so every world of every kind holds the same bits. Not
/// saved and not hashed: it is a build constant, never state.
#[resource(key = "sim::tick_delta", derived)]
#[derive(PartialEq, Debug)]
pub struct TickDelta(#[field(id = 1)] pub Scalar);

impl TickDelta {
    /// The delta of a simulation running at `rate` ticks per second: `1 / rate` as a
    /// `Scalar` division, the same bits on every target.
    ///
    /// # Panics
    ///
    /// Panics if `rate` is zero.
    #[must_use]
    pub fn of_rate(rate: u32) -> Self {
        assert!(rate != 0, "a simulation runs at least one tick per second");
        Self(Scalar::ONE / Scalar::from_u32(rate))
    }
}

/// The tick being simulated, for systems that cannot take [`Ctx`](lactose_ecs::Ctx): a
/// once-per-tick system, an exclusive system or a finalize. The input sync point rewrites
/// it every tick from [`World::tick`](lactose_ecs::World::tick), which stays the source of
/// truth, and it is read through [`get`](Self::get); a write (a decoded value, or
/// `world_mut`) is a bug the type cannot prevent, bounded to one tick. Between runs it
/// equals `Simulation::tick`. Not saved and not hashed.
#[resource(key = "sim::current_tick", derived)]
#[derive(PartialEq, Eq, Debug)]
pub struct CurrentTick(#[field(id = 1)] Tick);

impl CurrentTick {
    pub(crate) const fn new(tick: Tick) -> Self {
        Self(tick)
    }

    /// The tick.
    #[must_use]
    pub const fn get(self) -> Tick {
        self.0
    }
}

/// How many ticks run per real second, in thousandths of real time (architecture section
/// 11): `1000` is real time, `0` is paused, `2000` runs sixty ticks in half a second. The
/// tick delta never changes with it.
///
/// The scale is `app`'s decision on the server, and it reaches the world as an input:
/// [`TickInput::time_scale`](crate::TickInput::time_scale), which the sync point writes
/// here, so the value is journaled and `lactose-predict`'s ring carries it. The world's
/// copy is a mirror: `app`'s timer root keeps its own value for `TickPacer::set_time_scale`
/// (on the server its decision, on a client the latest value `net` received), never reads
/// it back from the world, and no game system reads it either, since a paused world runs
/// no system that could unpause it and the scale is not simulation state. `replicated` so
/// that `net` carries it to clients, which means a pause is entered by running one tick
/// whose frame carries `Some(PAUSED)` before the server's pacer goes to zero, and left by
/// setting the pacer first and putting `Some(scale)` in the next frame. Derived; in a
/// prediction-world snapshot like every reflected resource. Presentation runs on real time
/// regardless.
#[resource(key = "sim::time_scale", derived, replicated)]
#[derive(PartialEq, Eq, Debug)]
pub struct TimeScale {
    /// Ticks per real second as thousandths of [`TICK_RATE_HZ`]; zero is paused.
    #[field(id = 1)]
    pub thousandths: u32,
}

impl TimeScale {
    /// Real time: sixty ticks per second.
    pub const REAL_TIME: Self = Self { thousandths: 1000 };

    /// Paused: no tick runs until the scale changes.
    pub const PAUSED: Self = Self { thousandths: 0 };

    /// Whether the simulation is paused.
    #[must_use]
    pub const fn is_paused(self) -> bool {
        self.thousandths == 0
    }
}
