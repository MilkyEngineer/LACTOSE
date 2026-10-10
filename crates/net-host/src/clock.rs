//! Client clock sync (architecture sections 9.2 and 11): the estimate the timer root reads
//! to pace the prediction world ahead of the server. The estimator itself is
//! `lactose-net`'s, milestone 4; this module names its output.
//!
//! Host-only: the timer root is `app`'s and reads a clock.

use lactose_core::Tick;

/// The largest tick-rate adjustment the timer root applies to track the estimate, in
/// thousandths: three percent either way (architecture section 9.2). Beyond what that can
/// recover within the catch-up cap, the client snaps instead.
pub const MAX_RATE_ADJUST_THOUSANDTHS: u32 = 30;

/// What the client's clock sync has concluded, as `net` publishes it to the timer root once
/// per received packet. Named only; `lactose-net` computes it from the transport's round
/// trip, the observed jitter and the server ticks stamped on packets.
///
/// The timer root paces the prediction world at `server_tick + lead` and sets the pacer's
/// scale to the replicated [`TimeScale`](lactose_sim::TimeScale) times
/// `rate_thousandths / 1000`; when `snap` is set it moves its clock to the estimate at
/// once and resends its inputs rather than slowing (architecture section 9.2).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClockEstimate {
    /// The server's tick now, as estimated.
    pub server_tick: Tick,
    /// How far ahead of `server_tick` the client should simulate: half the round trip
    /// plus the jitter margin, in ticks of simulation time under the replicated time
    /// scale.
    pub lead_ticks: f32,
    /// The rate adjustment that tracks the estimate: `1000` is the replicated scale
    /// unchanged, clamped to `1000 ± MAX_RATE_ADJUST_THOUSANDTHS`.
    pub rate_thousandths: u32,
    /// Whether the drift exceeds what the rate adjustment can recover within the
    /// catch-up cap, so the timer root snaps its clock to the estimate.
    pub snap: bool,
}
