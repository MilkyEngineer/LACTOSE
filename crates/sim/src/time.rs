//! Time as the simulation sees it (architecture section 11): a fixed delta, the tick being
//! simulated, and a time scale that changes how many ticks run per real second.

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
        todo!()
    }
}

/// The tick being simulated, for systems that cannot take [`Ctx`](lactose_ecs::Ctx): a
/// once-per-tick system, an exclusive system or a finalize. Written by
/// [`systems::begin_tick`](crate::systems) at the input sync point from
/// [`World::tick`](lactose_ecs::World::tick), which stays the source of truth; between
/// ticks the two agree. Not saved and not hashed.
#[resource(key = "sim::current_tick", derived)]
#[derive(PartialEq, Eq, Debug)]
pub struct CurrentTick(#[field(id = 1)] pub Tick);

/// How many ticks run per real second, in thousandths of real time (architecture section
/// 11): `1000` is real time, `0` is paused, `2000` runs sixty ticks in half a second. The
/// tick delta never changes with it.
///
/// A simulation resource the server controls and replicates: game code on the server
/// writes it (`ResMut<TimeScale>` in a once-per-tick or exclusive system, or
/// [`World::resource_mut`](lactose_ecs::World::resource_mut) between ticks), `net` carries
/// it to clients, and the timer root in `app` reads it between ticks and hands
/// `thousandths / 1000` to `TickPacer::set_time_scale`. Presentation runs on real time
/// regardless. An integer, so the wire, the hash and the pacer agree exactly; the
/// conversion to the pacer's float happens outside the boundary. Not saved; part of a
/// prediction-world snapshot like every reflected resource.
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
