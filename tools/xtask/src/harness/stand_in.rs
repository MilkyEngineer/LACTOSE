//! Stand-ins for a simulation test, so the mode runner can be proven before `lactose-sim`
//! exists. Each uses the protocol exactly as a real test will: read the session, apply the
//! configuration, state what was applied, record a hash per tick, and write an execution log
//! under a chaos seed.
//!
//! A stand-in has no executor and no chunks. What it keeps of them is the one property the
//! harness tests: the order in which entities are visited depends on the run configuration,
//! as it does under a different worker count, layout or steal order. The sound stand-in
//! combines per-entity hashes with an order-independent sum, as `world_hash` does. Each
//! flawed one has the bug that one mode exists to catch, from [`FLAW_STARTS_AT`] on.
//!
//! `tests/harness_stand_in.rs` wraps them in tests that `cargo xtask sour self-test` runs
//! through nextest.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::protocol::{Capacity, Layout, ProtocolError, RunConfig, Session};

/// How many ticks a stand-in runs, recording one hash each.
pub const TICKS: u32 = 8;
/// The first tick at which a flawed stand-in misbehaves.
pub const FLAW_STARTS_AT: u32 = 3;

const ENTITIES: u64 = 64;

/// Which stand-in to run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flaw {
    /// Sound: order-independent, repeatable, honest about its configuration.
    None,
    /// The hash depends on visiting order, which follows the worker count.
    WorkerOrder,
    /// The hash depends on visiting order, which follows the storage layout.
    LayoutOrder,
    /// The hash depends on visiting order, which follows the steal order of a chaos seed.
    StealOrder,
    /// The hash takes in something that differs from run to run.
    Unrepeatable,
    /// The test never applies its configuration: it runs the production one and says so.
    IgnoresConfiguration,
}

/// Every stand-in: the nextest name of its test, which is also the name it records under.
pub const STAND_INS: [(&str, Flaw); 6] = [
    ("sound", Flaw::None),
    ("worker_order", Flaw::WorkerOrder),
    ("layout_order", Flaw::LayoutOrder),
    ("steal_order", Flaw::StealOrder),
    ("unrepeatable", Flaw::Unrepeatable),
    ("ignores_configuration", Flaw::IgnoresConfiguration),
];

/// The test name of the stand-in with `flaw`.
#[must_use]
pub fn test_name(flaw: Flaw) -> &'static str {
    STAND_INS.iter().find(|(_, f)| *f == flaw).map_or("", |(name, _)| name)
}

/// Runs the stand-in with `flaw` under `session`, recording as `test`.
///
/// # Errors
/// If a record or the execution log cannot be written.
pub fn run(session: &Session, test: &str, flaw: Flaw) -> Result<(), ProtocolError> {
    // The "builders": what this run is actually configured with.
    let applied = match flaw {
        Flaw::IgnoresConfiguration => RunConfig::default(),
        _ => *session.config(),
    };
    let mut recorder = session.recorder(test, &applied)?;
    if let Some(seed) = applied.chaos_seed {
        session.write_execution_log(test, &seed.to_le_bytes())?;
    }

    let order = Order::of(&applied);
    let mut state: Vec<u64> = (0..ENTITIES).map(mix).collect();
    for tick in 0..TICKS {
        for value in &mut state {
            *value = mix(*value ^ u64::from(tick));
        }
        // Every stand-in visits in the order its whole configuration gives, and a sound hash
        // does not care. A flawed fold cares about the one part its flaw is about.
        let per_entity = |first: u64| {
            (0..ENTITIES).map(move |n| (n + first) % ENTITIES).map(|e| mix(state[e as usize] ^ e))
        };
        let flawed = tick >= FLAW_STARTS_AT;
        let fold = |first: u64| per_entity(first).fold(0, |hash: u64, e| hash.rotate_left(5) ^ e);
        let sum = per_entity(order.all()).fold(0, u64::wrapping_add);
        let hash = match flaw {
            Flaw::WorkerOrder if flawed => fold(order.workers),
            Flaw::LayoutOrder if flawed => fold(order.layout),
            Flaw::StealOrder if flawed => fold(order.chaos),
            Flaw::Unrepeatable if flawed => sum ^ nonce(),
            _ => sum,
        };
        recorder.record(tick, hash)?;
    }
    Ok(())
}

/// Where each part of a configuration starts the visit: zero for the production value of
/// that part and something else for every other value the mode matrix uses.
struct Order {
    workers: u64,
    layout: u64,
    chaos: u64,
}

impl Order {
    fn of(config: &RunConfig) -> Self {
        let away_from_zero = |x: u64| mix(x) % (ENTITIES - 1) + 1;
        Self {
            workers: config.workers.map_or(0, |n| u64::from(n.get())),
            layout: match config.layout {
                Layout::Production => 0,
                Layout::Perturbed { capacity, seed } => {
                    let (Capacity::Rows(n) | Capacity::Bytes(n)) = capacity;
                    away_from_zero(u64::from(n) ^ seed)
                }
            },
            chaos: config.chaos_seed.map_or(0, away_from_zero),
        }
    }

    fn all(&self) -> u64 {
        mix(self.workers ^ self.layout.rotate_left(17) ^ self.chaos.rotate_left(31)) % ENTITIES
    }
}

/// A value that no two calls share, in one process or across processes.
fn nonce() -> u64 {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    mix(time ^ mix(u64::from(std::process::id()) ^ call.rotate_left(32))) | 1
}

/// The finaliser of SplitMix64: a fixed, well-mixed permutation of `u64`.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::super::matrix;
    use super::*;

    #[test]
    fn every_varied_configuration_visits_in_another_order_than_the_baseline() {
        let matrix = matrix();
        let baseline = Order::of(&matrix[0].variants[0].config);
        for mode in &matrix {
            for variant in &mode.variants {
                let order = Order::of(&variant.config);
                let varied = match mode.name {
                    "workers" => Some((order.workers, baseline.workers)),
                    "layout" => Some((order.layout, baseline.layout)),
                    "chaos" => Some((order.chaos, baseline.chaos)),
                    _ => None,
                };
                if let Some((here, there)) = varied.filter(|_| variant.label != "baseline") {
                    assert_ne!(here, there, "{}/{}", mode.name, variant.label);
                }
            }
        }
    }

    #[test]
    fn every_flaw_has_a_stand_in() {
        for (name, flaw) in STAND_INS {
            assert_eq!(test_name(flaw), name);
        }
    }
}
