//! The replication test that records under SOUR (architecture section 8.3): a server runs
//! a scene with spawns, despawns, moves, additions, removals and direct writes, one to
//! three ticks a frame; the gather's frame goes through a stand-in packer into a client's
//! inbox, and the client, stepped to the server's tick through the README's fold and hook,
//! must then hold exactly the server's replicated rows. After every frame it records the
//! server's tick with a hash of the server world, the gathered frame and the client world.
//! It builds from the runner's session through the glue `lactose-sim`'s README shows (the
//! protocol module by `#[path]`, the executor from `lactose-task`); outside the runner the
//! session is production and nothing is recorded.

#![forbid(unsafe_code)]

mod common;

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
mod protocol;

use std::num::NonZeroU32;
use std::sync::Arc;

use common::*;
use lactose_core::{Error, ErrorKind, Result, Tick, xxh64};
use lactose_ecs::{Executor, LayoutConfig, RegionId, World};
use lactose_net_api::{RemoteSlot, StagedInbox};
use lactose_reflect::Reflect;
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{Simulation, TickInput};
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
    #[expect(
        clippy::disallowed_methods,
        reason = "SOUR's glue is test code outside the step: it reports what the executor \
                  was built with"
    )]
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
    #[expect(
        clippy::disallowed_methods,
        reason = "SOUR's glue is test code outside the step: it writes the chaos log"
    )]
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

const FRAMES: u32 = 40;
const REGIONS: [RegionId; 2] = [REGION, OTHER_REGION];

fn populate(world: &mut World) {
    create_regions(world, &REGIONS);
    for n in 1..=60 {
        let region = REGIONS[usize::try_from(n % 2).expect("small")];
        let at = i32::try_from(n).expect("small");
        world.spawn(key(n), region, (Position { x: at, y: -at }, Health(at))).expect("spawn");
        if n % 2 == 1 {
            world.insert(key(n), Mover(at % 5 - 2)).expect("insert");
        }
        if n % 4 == 0 {
            world.insert(key(n), Secret(0)).expect("insert");
        }
        if n % 9 == 0 {
            world.insert(key(n), Marked).expect("insert");
        }
    }
}

/// What `app` does to the server between the runs of frame `index`.
fn script(world: &mut World, index: u32) -> Result<()> {
    let n = u64::from(index);
    if index % 4 == 1 {
        world.spawn(
            key(1000 + n),
            REGIONS[usize::try_from(n % 2).expect("small")],
            (Position { x: 0, y: 0 }, Mover(1)),
        )?;
    }
    if index % 6 == 2 && world.contains(key(1000 + n - 1)) {
        world.despawn(key(1000 + n - 1))?;
    }
    if index % 5 == 3 {
        let target = key(1 + n % 60);
        if world.contains(target) {
            let to = if world.region_of(target) == Some(REGION) { OTHER_REGION } else { REGION };
            world.move_to_region(target, to)?;
        }
    }
    if index % 7 == 4 {
        world.insert(key(2 + n % 50), Status(index))?;
    }
    if index % 7 == 6 {
        let _ = world.remove::<Status>(key(2 + (n - 2) % 50))?;
        let _ = world.remove::<Health>(key(3 + n % 50))?;
    }
    if index.is_multiple_of(3) {
        world.insert(key(4 + n % 40), Health(-i32::try_from(index).expect("small")))?;
    }
    if index == 25 {
        // A structural change the gather has not read, with the log trimmed past it: the
        // next gather is a full pass and the client is resynced from a full frame.
        world.despawn(key(60))?;
        let cursor = world.structural_log().cursor();
        world.trim_structural_log(cursor);
    }
    Ok(())
}

/// The scene, recording after every frame through `record` with the server's tick.
fn replicate(
    executor: &Executor,
    layout: LayoutConfig,
    record: &mut dyn FnMut(&Simulation, Tick, u64) -> Result<()>,
) -> Result<()> {
    let mut server = server(layout);
    populate(server.world_mut());
    let store = Store::with(&baseline(vec![]));
    let mut packer = Packer::new(baseline(vec![]));
    let mut client: Option<Simulation> = None;
    let mut fulls = 0;
    for index in 0..FRAMES {
        script(server.world_mut(), index)?;
        for _ in 0..=index % 3 {
            server = executor.block_on(server.step(executor, TickInput::default()))?;
        }
        server = executor.block_on(server.run_passes(executor))?;
        let gathered = take(&mut server);
        fulls += usize::from(gathered.full);
        let staged = packer.pack(&gathered, &REGIONS);
        let mut current = match client.take() {
            Some(client) => client,
            // Built at the tick of the first full frame, as `JoinAccepted` says.
            None => client_builder(staged.tick, ClientHook::default()).layout(layout).build()?,
        };
        let accepted =
            current.world_mut().service_mut::<StagedInbox>().expect("setup").push(Arc::new(staged));
        assert!(accepted, "frame {index}");
        while current.tick() < server.tick() {
            let (input, _) = between_runs(&mut current, [], &store);
            current = executor.block_on(current.step(executor, input))?;
        }
        if index > 0 {
            assert_eq!(
                replicated_rows(current.world()),
                replicated_rows(server.world()),
                "frame {index}"
            );
            for column in current.world().columns_erased(Position::KEY) {
                for &key in column.keys() {
                    assert!(current.world().get::<RemoteSlot>(key).is_some());
                }
            }
            assert!(current.lifecycle_mut().take_load_failures().is_empty());
        }
        let mut hashes = Vec::new();
        hashes.extend_from_slice(&server.hash()?.to_le_bytes());
        hashes.extend_from_slice(&frame_digest(&gathered).to_le_bytes());
        hashes.extend_from_slice(&current.hash()?.to_le_bytes());
        record(&server, server.tick(), xxh64(&hashes, 0))?;
        client = Some(current);
    }
    // The first gather and the one after the trim.
    assert_eq!(fulls, 2);
    Ok(())
}

#[test]
fn a_scene_replicated_to_a_client() {
    let test = "lactose_net_api::sour::a_scene_replicated_to_a_client";
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

/// The same scene outside the runner, recording into memory: the records are the same
/// under a perturbed layout with four workers as under production with one.
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
    assert_eq!(production.len(), usize::try_from(FRAMES).expect("small"));
    let perturbed = layout_mode(ChunkCapacity::Rows(7), 5).unwrap();
    assert_eq!(run(perturbed, 4), production);
}
