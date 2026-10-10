//! Lag compensation (architecture section 9.5): the named place of the history the server
//! keeps so a hit can be validated at the tick the client saw. Milestone 4 gives it its
//! shape, which depends on `physics-api`'s collider and query types.
//!
//! Boundary code may use this module: hit validation runs inside the simulation.

use lactose_sim::TICK_RATE_HZ;

/// How far back the history reaches, in ticks: 250 milliseconds at the tick rate
/// (architecture section 9.5). A hit claimed for an older tick is validated against the
/// oldest tick kept.
pub const LAG_COMPENSATION_MAX_TICKS: u32 = TICK_RATE_HZ / 4;

/// The server's per-tick history of transforms and colliders for players and for NPCs that
/// can be hit, capped at [`LAG_COMPENSATION_MAX_TICKS`]. Named only: its contents, how a
/// pass records them each tick and how a hit query rewinds against them are milestone 4's,
/// written with `physics-api`'s types.
#[derive(Debug, Default)]
pub struct LagCompensationHistory(());
