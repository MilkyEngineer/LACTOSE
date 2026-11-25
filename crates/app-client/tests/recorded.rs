//! Criterion 3(c) (`crates/app/README.md`, "Determinism of a shown world"): the wiring that
//! ships, with `game-sim`'s mover and the binary's scene, steps a world with `Shown::step`
//! and its render side on `lactose-rhi-null`, and the same world with `Server::step`, under
//! the runner's session. The shown world's hash after every tick is recorded, with its
//! `PhysicsSnapshot::hash` as a stream of its own (D145), and the unshown world's hash is
//! held equal to it at every tick; SOUR finds the recorded streams equal in every mode.

use std::num::NonZeroU32;
use std::sync::Arc;

use lactose_app::{Cooked, Start, Workers, WorldConfig, build_server};
use lactose_app_client::{Frames, GAME, InMemory};
use lactose_core::{Error, ErrorKind, Result};
use lactose_physics_api::snapshot;
use lactose_rhi_api::{Capabilities, Extent, QueueLayout};
use lactose_sim::harness::{ChunkCapacity, layout_mode, layout_mode_of};
use lactose_task::ChaosConfig;

use crate::null::{Run, name};
use crate::protocol::{self, Capacity, Layout, RunConfig, Session};
use crate::support::bounded;

const E1: Extent = Extent { width: 64, height: 48 };
const E2: Extent = Extent { width: 80, height: 40 };

/// The steps' extents, in turn: drawn at E1, hidden, drawn at E2, back to E1.
const EXTENTS: [Option<Extent>; 8] =
    [Some(E1), Some(E1), Some(E1), None, Some(E2), Some(E2), Some(E1), Some(E1)];

/// The steps whose presents null stalls, so that frames are held on the one queue.
const STALLED: std::ops::Range<u32> = 20..24;

const TICKS: u32 = 48;

fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}

/// The layout a world reports, as the runner spells it.
fn applied_layout(layout: &lactose_ecs::LayoutConfig) -> Result<Layout> {
    Ok(match layout_mode_of(layout)? {
        (ChunkCapacity::Production, _) => Layout::Production,
        (ChunkCapacity::Rows(n), seed) => Layout::Perturbed { capacity: Capacity::Rows(n), seed },
        (ChunkCapacity::Bytes(n), seed) => Layout::Perturbed { capacity: Capacity::Bytes(n), seed },
    })
}

fn chaos(seed: u64) -> ChaosConfig {
    ChaosConfig { seed, shuffle_steals: true, inject_yields: true, force_split: true, record: true }
}

/// [`TICKS`] steps of the scene on one queue: drawn, hidden and held frames, the extent
/// changing, and the hashes of the shown world and the unshown one equal at every tick.
#[test]
fn a_shown_world_on_null_hashes_as_the_world_unshown() -> Result<()> {
    let test = "lactose_app_client::recorded::a_shown_world_on_null_hashes_as_the_world_unshown";
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
    let workers = Workers {
        frame_workers: config.workers.map(NonZeroU32::get),
        chaos: config.chaos_seed.map(chaos),
        ..Workers::default()
    };
    let world = WorldConfig { layout, ..WorldConfig::shown() };
    let capabilities = Capabilities { queues: QueueLayout::ONE, ..Capabilities::LEAST };
    let mut run = Run::new(capabilities, workers, world.clone());
    let cooked = Arc::new(InMemory::new()?);
    let mut frames = Frames::new(&cooked)?;
    let cooked = cooked as Arc<dyn Cooked>;
    let mut plain = Some(build_server(&run.executor, &GAME, world, Start::Fresh, cooked)?);
    let built = run.shown().server().simulation().world();
    let applied = RunConfig {
        workers: config.workers.map(|_| {
            NonZeroU32::new(run.executor.frame_worker_count()).expect("at least one worker")
        }),
        layout: applied_layout(built.layout())?,
        chaos_seed: run.executor.chaos_config().map(|chaos| chaos.seed),
    };
    let physics_test = format!("{test}::physics");
    let mut recorder = session.recorder(test, &applied).map_err(protocol_error)?;
    let mut physics = session.recorder(&physics_test, &applied).map_err(protocol_error)?;
    let mut seen = Vec::new();
    for step in 0..TICKS {
        if step == STALLED.start {
            assert!(run.control.stall(true), "the device is gone");
        }
        if step == STALLED.end {
            assert!(run.control.stall(false), "the device is gone");
        }
        seen.push(name(&run.step(EXTENTS[step as usize % EXTENTS.len()])));
        let server = plain.take().expect("the unshown world");
        let input = frames.next(server.tick());
        let stepped = bounded(&run.executor, server.step(&run.executor, input), "the unshown step");
        let (server, output) = stepped?;
        if !output.failures.is_empty() {
            return Err(Error::new(ErrorKind::InvalidData, format!("{step}: {output:?}")));
        }
        let shown = run.shown().server();
        assert_eq!(server.tick(), shown.tick(), "step {step}");
        let hash = shown.hash()?;
        assert_eq!(server.hash()?, hash, "step {step}");
        recorder.record(shown.tick().0, hash).map_err(protocol_error)?;
        let snapshot = snapshot(shown.simulation().world())?;
        physics.record(shown.tick().0, snapshot.hash()).map_err(protocol_error)?;
        plain = Some(server);
    }
    // Every frame drew, but for the hidden ones; the held ones are the stall's.
    assert!(seen.iter().all(|name| ["drawn", "held", "hidden"].contains(name)), "{seen:?}");
    assert!(seen.contains(&"hidden"), "{seen:?}");
    assert!(seen.contains(&"held"), "{seen:?}");
    plain.take().expect("the unshown world").shutdown();
    if let Some(log) = run.stop() {
        for name in [test, &physics_test] {
            session.write_execution_log(name, log.as_bytes()).map_err(protocol_error)?;
        }
    }
    Ok(())
}
