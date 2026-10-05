//! A stand-in for a simulation test, so the mode runner can be proven before `lactose-sim`
//! exists. It uses the protocol exactly as a real test will: read the session, apply the
//! configuration, record a hash per tick.
//!
//! The stand-in has no executor and no chunks. What it keeps of them is the one property the
//! harness tests: the order in which entities are visited depends on the run configuration,
//! as it does under a different worker count, layout or steal order. The sound stand-in
//! combines per-entity hashes with an order-independent sum, as `world_hash` does. The flawed
//! one switches to an order-dependent fold at [`FLAW_STARTS_AT`], which is the bug each
//! mode exists to catch.
//!
//! `tests/sour_stand_in.rs` wraps the two in tests that `cargo xtask sour self-test` runs
//! through nextest.

use super::protocol::{Capacity, Layout, ProtocolError, RunConfig, Session};

/// The nextest name of the sound stand-in test, and the name it records under.
pub const SOUND_TEST: &str = "sound";
/// The nextest name of the flawed stand-in test, and the name it records under.
pub const FLAWED_TEST: &str = "order_dependent";

/// How many ticks a stand-in runs, recording one hash each.
pub const TICKS: u32 = 8;
/// The first tick at which the flawed stand-in's hash depends on visiting order.
pub const FLAW_STARTS_AT: u32 = 3;

const ENTITIES: u64 = 64;

/// Which stand-in to run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flaw {
    /// Order-independent throughout.
    None,
    /// Order-dependent from [`FLAW_STARTS_AT`].
    OrderDependent,
}

/// Runs the stand-in under `session`, recording as `test`.
///
/// # Errors
/// If the recorder cannot be opened or written.
pub fn run(session: &Session, test: &str, flaw: Flaw) -> Result<(), ProtocolError> {
    let mut recorder = session.recorder(test)?;
    let first = visiting_offset(session.config());
    let mut state: Vec<u64> = (0..ENTITIES).map(mix).collect();
    for tick in 0..TICKS {
        for value in &mut state {
            *value = mix(*value ^ u64::from(tick));
        }
        let visited = (0..ENTITIES).map(|n| (n + first) % ENTITIES);
        let per_entity = visited.map(|entity| mix(state[entity as usize] ^ entity));
        let hash = if flaw == Flaw::OrderDependent && tick >= FLAW_STARTS_AT {
            per_entity.fold(0, |hash: u64, entity| hash.rotate_left(5) ^ entity)
        } else {
            per_entity.fold(0, u64::wrapping_add)
        };
        recorder.record(tick, hash)?;
    }
    Ok(())
}

/// The entity the stand-in visits first, standing for the visiting order a real run would
/// get. Every varied configuration of the mode matrix starts somewhere other than the
/// baseline does, which the test below pins.
fn visiting_offset(config: &RunConfig) -> u64 {
    let workers = config.workers.map_or(0, |n| u64::from(n.get()));
    let layout = match config.layout {
        Layout::Production => 0,
        Layout::Perturbed { capacity, seed } => {
            let amount = match capacity {
                Capacity::Rows(n) | Capacity::Bytes(n) => u64::from(n),
            };
            mix(amount ^ seed) | 1
        }
    };
    let chaos = config.chaos_seed.map_or(0, |seed| mix(seed) | 1);
    mix(workers ^ layout.rotate_left(17) ^ chaos.rotate_left(31)) % ENTITIES
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
        let baseline = matrix[0].variants[0].config;
        for mode in &matrix {
            for variant in mode.variants.iter().filter(|v| v.config != baseline) {
                assert_ne!(
                    visiting_offset(&variant.config),
                    visiting_offset(&baseline),
                    "{}/{}",
                    mode.name,
                    variant.label
                );
            }
        }
    }
}
