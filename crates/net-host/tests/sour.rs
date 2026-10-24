//! The test that records under SOUR (architecture section 8.3): the replication of
//! `replication.rs` (a server, the packer and receiver models over a link that loses and
//! reorders, a client world stepped through the fold and the hook, a resync), built from
//! the runner's session, records after every server tick a hash of the server world, the
//! client world, and the wire values of every server entity: a position and a rotation
//! derived from its `Position` through `QuantisedPosition` and `QuantisedRotation`, the
//! quantised integers and the dequantised bits. The architecture mode compares those bits
//! between x86-64 and aarch64, which is the golden table of `dequantise` on every target the
//! engine ships. It builds from the runner's session through the glue `lactose-sim`'s
//! README shows (the protocol module by `#[path]`, the executor from `lactose-task`);
//! outside the runner the session is production and nothing is recorded.

#![forbid(unsafe_code)]

mod common;

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "the boundary lints these expectations name are not configured outside the boundary"
)]
mod protocol;

use std::num::NonZeroU32;

use common::*;
use lactose_core::{Error, ErrorKind, Result, Tick, xxh64};
use lactose_ecs::{Executor, LayoutConfig, World};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_host::{QuantisedPosition, QuantisedRotation};
use lactose_reflect::Reflect;
use lactose_sim::Simulation;
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_task::ChaosConfig;
use protocol::{Capacity, Layout, Recorder, RunConfig, Session};

/// What a test builds from its session, and reports back.
struct Sour {
    session: Session,
    executor: Executor,
    layout: LayoutConfig,
}

impl Sour {
    /// Reads the session and builds the executor and layout it asks for.
    fn from_env() -> Result<Self> {
        let session = Session::from_env().map_err(protocol_error)?;
        let config = *session.config();
        let layout = match config.layout {
            Layout::Production => layout_mode(ChunkCapacity::Production, 0)?,
            Layout::Perturbed { capacity: Capacity::Rows(n), seed } => {
                layout_mode(ChunkCapacity::Rows(n), seed)?
            }
            Layout::Perturbed { capacity: Capacity::Bytes(n), seed } => {
                layout_mode(ChunkCapacity::Bytes(n), seed)?
            }
        };
        let mut executor = Executor::builder();
        if let Some(workers) = config.workers {
            executor = executor.frame_workers(workers.get());
        }
        if let Some(seed) = config.chaos_seed {
            executor = executor.chaos(ChaosConfig {
                seed,
                shuffle_steals: true,
                inject_yields: true,
                force_split: true,
                record: true,
            });
        }
        Ok(Self { session, executor: executor.build()?, layout })
    }

    /// Opens the recorder with what the executor and the world report they were built
    /// with.
    fn recorder(&self, test: &str, simulation: &Simulation) -> Result<Recorder> {
        let config = self.session.config();
        let layout = simulation.world().layout();
        let applied = RunConfig {
            workers: config.workers.map(|_| {
                NonZeroU32::new(self.executor.frame_worker_count()).expect("at least one worker")
            }),
            layout: if *layout == LayoutConfig::default() {
                Layout::Production
            } else {
                let capacity = match layout.chunk_rows {
                    Some(n) => Capacity::Rows(n),
                    None => Capacity::Bytes(16 * 1024),
                };
                Layout::Perturbed { capacity, seed: layout.seed }
            },
            chaos_seed: self.executor.chaos_config().map(|chaos| chaos.seed),
        };
        self.session.recorder(test, &applied).map_err(protocol_error)
    }

    /// Writes the execution log a chaos run records.
    fn finish(&self, test: &str) -> Result<()> {
        if let Some(log) = self.executor.execution_log() {
            self.session.write_execution_log(test, log.as_bytes()).map_err(protocol_error)?;
        }
        Ok(())
    }
}

fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}

const TICKS: u32 = 240;
const RESYNC_AT: u32 = 120;

/// The wire values of every server entity, appended to `bytes` in key order: a position
/// of about a tenth of a metre to a few kilometres and a rotation from its `Position`,
/// each quantised, then dequantised as canonical bits.
fn wire_values(world: &World, bytes: &mut Vec<u8>) {
    for ((_, component), (_, row)) in replicated_rows(world) {
        if component != Position::KEY {
            continue;
        }
        let word = |at: usize| i32::from_ne_bytes(row[at..at + 4].try_into().expect("an i32"));
        let (x, y) = (word(0), word(4));
        let s = Scalar::from_i32;
        let position = Vec3::new(s(x) / s(7), s(y) / s(3), s(x.wrapping_mul(y)) / s(1000));
        let quantised = QuantisedPosition::quantise(position);
        for value in [quantised.x, quantised.y, quantised.z] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let back = quantised.dequantise();
        for value in [back.x, back.y, back.z] {
            bytes.extend_from_slice(&value.to_canonical_bits().to_le_bytes());
        }
        let rotation = Quat::from_rotation_z(s(x) / s(50))
            * Quat::from_rotation_x(s(y) / s(30))
            * Quat::new(s(x), s(y), s(x - y), s(7)).normalize();
        let quantised = QuantisedRotation::quantise(rotation);
        for value in [quantised.x, quantised.y, quantised.z, quantised.w] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in quantised.dequantise().to_array() {
            bytes.extend_from_slice(&value.to_canonical_bits().to_le_bytes());
        }
    }
}

/// The replication, recording after every server tick through `record`.
fn replicate(
    executor: &Executor,
    layout: LayoutConfig,
    record: &mut dyn FnMut(&Simulation, Tick, u64) -> Result<()>,
) -> Result<()> {
    let mut replication = Replication::new(layout, 40, &[REGIONS[0], REGIONS[1]], 5);
    replication.link = LinkConfig::POOR;
    replication.lag = 6;
    let mut script = Rng::new(17);
    let mut next = 41;
    for n in 0..TICKS {
        churn(replication.world_mut(), &mut script, &mut next);
        change_interest(&mut replication, &mut script, next, &[]);
        if n == RESYNC_AT {
            replication.change(Change::Resync);
        }
        replication.tick(executor);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&replication.server().hash()?.to_le_bytes());
        let client = replication.client.as_ref().map(Simulation::hash).transpose()?;
        bytes.extend_from_slice(&client.unwrap_or(0).to_le_bytes());
        wire_values(replication.server().world(), &mut bytes);
        record(replication.server(), replication.server().tick(), xxh64(&bytes, 0))?;
    }
    calm(&mut replication, executor, next);
    replication.assert_converged("the recorded replication");
    Ok(())
}

#[test]
fn a_replication_and_its_wire_values() {
    let test = "lactose_net_host::sour::a_replication_and_its_wire_values";
    let sour = Sour::from_env().unwrap();
    let mut recorder: Option<Recorder> = None;
    let mut record = |server: &Simulation, tick: Tick, hash: u64| -> Result<()> {
        if recorder.is_none() {
            recorder = Some(sour.recorder(test, server)?);
        }
        recorder.as_mut().expect("opened").record(tick.0, hash).map_err(protocol_error)
    };
    replicate(&sour.executor, sour.layout, &mut record).unwrap();
    sour.finish(test).unwrap();
}

/// The same replication outside the runner, recording into memory: the records are the
/// same under a perturbed layout with four workers as under production with one.
#[test]
fn the_records_are_the_same_under_another_layout_and_worker_count() {
    let run = |layout: LayoutConfig, workers: u32| {
        let mut records = Vec::new();
        let mut record = |_: &Simulation, tick: Tick, hash: u64| -> Result<()> {
            records.push((tick, hash));
            Ok(())
        };
        replicate(&executor(workers), layout, &mut record).unwrap();
        records
    };
    let production = run(LayoutConfig::default(), 1);
    assert_eq!(production.len(), usize::try_from(TICKS).expect("small"));
    let perturbed = layout_mode(ChunkCapacity::Rows(7), 5).unwrap();
    assert_eq!(run(perturbed, 4), production);
}
