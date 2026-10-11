//! The prediction test that records under SOUR (architecture section 8.3): a prediction
//! world with the physics test backend, four claimed moving bodies, the player's entity, a
//! cart and two proxies, predicts 150 ticks of a walk with jumps while the server's frames
//! arrive one to five ticks late with rows that differ from the prediction (the cart's
//! position and health, a body's transform), proxy rows, and substitutions of the player's
//! sample, some repaired in the same frame and some in the next. After every step and
//! every reconciliation it records, under a sequence number, a hash of the tick, the world
//! and what the reconciliation did: a correction changes the hash of a tick already
//! predicted, so a tick cannot key a record here. The world must end with the hash of the
//! straight run that had the server's rows and samples from their ticks on. It builds from
//! the runner's session through the glue `lactose-sim`'s README shows (the protocol module
//! by `#[path]`, the executor from `lactose-task`); outside the runner the session is
//! production and nothing is recorded.

#![forbid(unsafe_code)]

mod common;

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
mod protocol;

use std::num::NonZeroU32;

use common::fake::{CallLog, Fake};
use common::*;
use lactose_core::{Error, ErrorKind, Result, StableKey, Tick, xxh64};
use lactose_ecs::{Executor, LayoutConfig};
use lactose_input::PlayerInput;
use lactose_math::{Scalar, Vec3};
use lactose_net_api::StagedFrame;
use lactose_physics_api::{RigidBody, Velocity};
use lactose_predict::{CorrectionCause, Reconciled};
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_task::ChaosConfig;
use protocol::{Capacity, Layout, Recorder, RunConfig, Session};

/// What a test builds from its session, and reports back. The executor it asks for is
/// handed out beside it, since the predictor's rig owns the executor it runs on.
struct Sour {
    session: Session,
    layout: LayoutConfig,
}

impl Sour {
    /// Reads the session and builds the executor and layout it asks for.
    fn from_env() -> Result<(Self, Executor)> {
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
        Ok((Self { session, layout }, executor.build()?))
    }

    /// Opens the recorder with what the executor and the layout report they were built
    /// with.
    #[expect(
        clippy::disallowed_methods,
        reason = "SOUR's glue is test code outside the step: it reports what the executor \
                  was built with"
    )]
    fn recorder(&self, test: &str, executor: &Executor) -> Result<Recorder> {
        let config = self.session.config();
        let layout = self.layout;
        let applied = RunConfig {
            workers: config.workers.map(|_| {
                NonZeroU32::new(executor.frame_worker_count()).expect("at least one worker")
            }),
            layout: if layout == LayoutConfig::default() {
                Layout::Production
            } else {
                let capacity = match layout.chunk_rows {
                    Some(n) => Capacity::Rows(n),
                    None => Capacity::Bytes(16 * 1024),
                };
                Layout::Perturbed { capacity, seed: layout.seed }
            },
            chaos_seed: executor.chaos_config().map(|chaos| chaos.seed),
        };
        self.session.recorder(test, &applied).map_err(protocol_error)
    }

    /// Writes the execution log a chaos run records.
    #[expect(
        clippy::disallowed_methods,
        reason = "SOUR's glue is test code outside the step: it writes the chaos log"
    )]
    fn finish(&self, test: &str, executor: &Executor) -> Result<()> {
        if let Some(log) = executor.execution_log() {
            self.session.write_execution_log(test, log.as_bytes()).map_err(protocol_error)?;
        }
        Ok(())
    }
}

fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}

const TICKS: u32 = 150;
/// The claimed bodies' keys; a fixed proxy body after them.
const BODIES: core::ops::Range<u64> = 5..9;
const PROXY_BODY: u64 = 9;
/// The server's frames: the `i`th is pushed at the present `FIRST + EVERY * i`.
const FRAMES: u32 = 17;
const FIRST: u32 = 20;
const EVERY: u32 = 7;

/// The join frame of `join_frame`, with four claimed bodies moving along x at one unit a
/// second and a fixed proxy body.
fn scene_frame() -> StagedFrame {
    let mut frame = join_frame(JOIN);
    let linear = Vec3::new(Scalar::ONE, Scalar::ZERO, Scalar::ZERO);
    let moving = Velocity { linear, ..Velocity::ZERO };
    for n in BODIES {
        let x = i32::try_from(n).expect("small") * 10;
        let parts = vec![data(transform(x)), data(moving), data(floating()), data(sphere())];
        frame.spawns.insert(net(n), claimed(REGION, parts));
    }
    let parts = vec![data(transform(-20)), data(RigidBody::fixed()), data(sphere())];
    frame.spawns.insert(net(PROXY_BODY), remote(REGION, 7, parts));
    frame
}

/// What the player sends: a walk that turns every tick, a jump pressed every 17 ticks and
/// held for two more.
fn sample(tick: u32) -> PlayerInput {
    match tick % 17 {
        0 => jumping(2, true),
        1 | 2 => jumping(-1, false),
        _ => moving(i16::try_from(tick % 3).expect("small") - 1),
    }
}

/// One frame of the server's: when `app` pushes it and what it holds.
struct Scheduled {
    /// The present at which it is pushed; the next step applies it.
    present: u32,
    tick: u32,
    cart: Position,
    health: Health,
    /// A claimed body's transform, every third frame.
    body: Option<(StableKey, i32)>,
    /// The proxy's position, dequantised.
    proxy: Position,
    /// The tick whose sample the server substituted, in every other frame: the tick
    /// before the frame's, repaired by the frame's own, or the frame's own, repaired by the
    /// next frame.
    substituted: Option<u32>,
}

fn schedule(index: u32) -> Scheduled {
    let present = FIRST + EVERY * index;
    let tick = present - 1 - index % 5;
    let value = i32::try_from(index).expect("small");
    Scheduled {
        present,
        tick,
        cart: at(100 + value, value),
        // Agrees with the prediction in every even frame.
        health: Health(if index.is_multiple_of(2) { 10 } else { value }),
        body: index
            .is_multiple_of(3)
            .then(|| (key(BODIES.start + u64::from(index % 4)), 200 + value)),
        proxy: at(-50 - value, 0),
        substituted: match index % 4 {
            1 => Some(tick - 1),
            3 => Some(tick),
            _ => None,
        },
    }
}

fn scheduled_at(present: u32) -> Option<Scheduled> {
    (0..FRAMES).map(schedule).find(|scheduled| scheduled.present == present)
}

fn staged(scheduled: &Scheduled) -> StagedFrame {
    let mut frame = StagedFrame::new(Tick(scheduled.tick));
    exact(&mut frame, cart(), &scheduled.cart);
    exact(&mut frame, cart(), &scheduled.health);
    if let Some((body, x)) = scheduled.body {
        exact(&mut frame, body, &transform(x));
    }
    dequantised(&mut frame, proxy(), &scheduled.proxy);
    frame.substituted.extend(scheduled.substituted.map(Tick));
    frame
}

/// What the server applied: the substitute of the sample before a substituted tick, and
/// the first sample after it with its edges recomputed against the substitute. No two
/// substituted ticks are within three of each other, so neither neighbour is itself
/// substituted or repaired.
fn applied(tick: u32) -> PlayerInput {
    let substituted = |t: u32| (0..FRAMES).any(|i| schedule(i).substituted == Some(t));
    if substituted(tick) {
        sample(tick - 1).substitute()
    } else if substituted(tick - 1) {
        sample(tick).after_substitute(&sample(tick - 2).substitute())
    } else {
        sample(tick)
    }
}

fn bodies_rig(executor: Executor, layout: LayoutConfig) -> Rig {
    let backend = Box::new(Fake::new(CallLog::default()).without_contacts());
    let mut rig = Rig::with_executor(simulation(layout, Some(backend)), executor);
    rig.push(scene_frame());
    rig.step(PlayerInput::IDLE);
    assert!(rig.reconcile().corrections.is_empty());
    rig
}

/// A digest of what a reconciliation did.
fn digest(reconciled: &Reconciled) -> u64 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&reconciled.frames.to_le_bytes());
    bytes.extend_from_slice(&reconciled.resimulated.to_le_bytes());
    for correction in &reconciled.corrections {
        bytes.extend_from_slice(&correction.tick.0.to_le_bytes());
        bytes.extend_from_slice(&correction.depth.to_le_bytes());
        bytes.push(match correction.cause {
            CorrectionCause::Mismatch => 0,
            CorrectionCause::Substituted => 1,
            CorrectionCause::Repaired => 2,
        });
        bytes.extend_from_slice(&correction.divergence.entities.to_le_bytes());
        bytes.extend_from_slice(&correction.divergence.fields.to_le_bytes());
        for key in &correction.entities {
            bytes.extend_from_slice(format!("{key:?}").as_bytes());
        }
    }
    xxh64(&bytes, 0)
}

/// The scene, recording through `record` under a sequence number; hands the executor
/// back.
fn predict(
    executor: Executor,
    layout: LayoutConfig,
    record: &mut dyn FnMut(&Executor, u32, u64) -> Result<()>,
) -> Result<Executor> {
    let mut rig = bodies_rig(executor, layout);
    let mut sequence = 0;
    let mut record = |rig: &Rig, done: u64| -> Result<()> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&rig.tick().0.to_le_bytes());
        bytes.extend_from_slice(&rig.hash().to_le_bytes());
        bytes.extend_from_slice(&done.to_le_bytes());
        record(&rig.executor, sequence, xxh64(&bytes, 0))?;
        sequence += 1;
        Ok(())
    };
    let (mut mismatched, mut substituted, mut repaired) = (0, 0, 0);
    while rig.tick().0 < TICKS {
        let present = rig.tick().0;
        if let Some(scheduled) = scheduled_at(present) {
            rig.push(staged(&scheduled));
        }
        rig.step(sample(present + 1));
        record(&rig, 0)?;
        let reconciled = rig.reconcile();
        assert_eq!(reconciled.resync, None);
        for correction in &reconciled.corrections {
            match correction.cause {
                CorrectionCause::Mismatch => mismatched += 1,
                CorrectionCause::Substituted => substituted += 1,
                CorrectionCause::Repaired => repaired += 1,
            }
        }
        record(&rig, digest(&reconciled))?;
    }
    // Every frame's cart position differs; a substitution before its frame's tick is a
    // rollback of its own, so is a repair in the next frame.
    assert!(
        mismatched > 0 && substituted > 0 && repaired > 0,
        "{mismatched} {substituted} {repaired}"
    );
    let substitutions = (0..FRAMES).filter(|&i| schedule(i).substituted.is_some()).count();
    assert_eq!(rig.p().stats().substitutions, u64::try_from(substitutions).expect("small"));

    // The straight run that had the server's rows from their ticks on, and its samples.
    let mut straight = bodies_rig(common::executor(1), layout);
    while straight.tick().0 < TICKS {
        let present = straight.tick().0;
        if let Some(scheduled) = scheduled_at(present) {
            // The hook writes a proxy's row at the step that applies its frame.
            straight.world_mut().insert(proxy(), scheduled.proxy)?;
        }
        straight.step(applied(present + 1));
        let tick = straight.tick().0;
        for scheduled in (0..FRAMES).map(schedule).filter(|scheduled| scheduled.tick == tick) {
            let world = straight.world_mut();
            world.insert(cart(), scheduled.cart)?;
            world.insert(cart(), scheduled.health)?;
            if let Some((body, x)) = scheduled.body {
                world.insert(body, transform(x))?;
            }
        }
    }
    assert_eq!(rig.position(me()), straight.position(me()));
    assert_eq!(rig.hash(), straight.hash());
    let Rig { executor, .. } = rig;
    Ok(executor)
}

#[test]
fn a_prediction_scene_with_corrections_and_substitutions() {
    let test = "lactose_predict::sour::a_prediction_scene_with_corrections_and_substitutions";
    let (sour, executor) = Sour::from_env().unwrap();
    let mut recorder: Option<Recorder> = None;
    let mut record = |executor: &Executor, sequence: u32, hash: u64| -> Result<()> {
        if recorder.is_none() {
            recorder = Some(sour.recorder(test, executor)?);
        }
        recorder.as_mut().expect("opened").record(sequence, hash).map_err(protocol_error)
    };
    let executor = predict(executor, sour.layout, &mut record).unwrap();
    sour.finish(test, &executor).unwrap();
}

/// The same scene outside the runner, recording into memory: the records are the same
/// under a perturbed layout with four workers as under production with one.
#[test]
fn the_records_are_the_same_under_another_layout_and_worker_count() {
    let run = |layout: LayoutConfig, workers: u32| {
        let mut records = Vec::new();
        let mut record = |_: &Executor, sequence: u32, hash: u64| -> Result<()> {
            records.push((sequence, hash));
            Ok(())
        };
        predict(executor(workers), layout, &mut record).unwrap();
        records
    };
    let production = run(LayoutConfig::default(), 1);
    assert_eq!(production.len(), usize::try_from(2 * (TICKS - 11)).expect("small"));
    let perturbed = layout_mode(ChunkCapacity::Rows(3), 5).unwrap();
    assert_eq!(run(perturbed, 4), production);
}
