//! Compile-only consumer sketches against the facade: what `app`, the milestone 1 spike,
//! `lactose-net` and `lactose-predict` write on the simulation side, type-checked and
//! never run (every deciding body of the crate is `todo!()`). They pin the surface the
//! README promises: a change that breaks one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use lactose_asset_api::{PinnedTable, VersionTable};
use lactose_bundle::{Baseline, ComponentData, Overlay};
use lactose_core::{ComponentKey, Error, ErrorKind, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{Job, RegionId, ResMut, World, WorldKind, system};
use lactose_net_api::{
    GatherState, RegionChange, RegionRelevance, RemoteSlot, StagedFrame, StagedInbox, StagedRow,
    StagedSpawn, StagingFrame, Tombstone,
};
use lactose_reflect::{Reflect, component, decode};
use lactose_sim::{
    RegionExport, Registration, Simulation, SimulationBuilder, StagedState, TickInput, TypeTable,
};

// ---- 1. Two replicated components of a game crate, and `app`'s build for every kind ----

#[component(key = "sketch::position", persistent, replicated)]
pub struct Position {
    #[field(id = 1)]
    pub x: i32,
    #[field(id = 2)]
    pub y: i32,
    #[field(id = 3)]
    pub z: i32,
}

#[component(key = "sketch::health", persistent, replicated)]
pub struct Health(#[field(id = 1)] pub i32);

fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Position>()?;
    types.register::<Health>()?;
    lactose_net_api::register_types(types)
}

/// `app`: the same three calls for the server, the client and the prediction world; the
/// client worlds start at the server's tick from the join acceptance and get their
/// `StagedState` hook, the server gets the gather.
fn app_build(
    kind: WorldKind,
    start: Tick,
    hook: Option<Box<dyn StagedState>>,
) -> Result<Simulation> {
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    let mut builder = SimulationBuilder::new(kind, types)
        .tick(start)
        .setup(lactose_net_api::setup)
        .systems(lactose_net_api::register_systems);
    if let Some(hook) = hook {
        builder = builder.staged_state(hook);
    }
    builder.build()
}

// ---- 2. The client's hook: a staged frame into the replicated world ----

/// `lactose-net`'s `StagedState` on a client, written here with typed writes for the
/// sketch's two components; milestone 4 writes every registered replicated type through
/// `World::insert_erased`, `remove_erased` and `spawn_erased`.
struct ClientHook;

fn find<T: Reflect>(components: &[ComponentData]) -> Option<T> {
    let data = components.iter().find(|data| data.component == T::KEY)?;
    decode(&data.bytes).ok()
}

/// A whole component written so that an equal value does not dirty the row.
fn write_whole<T: lactose_reflect::Component>(
    world: &mut World,
    key: StableKey,
    value: T,
) -> Result<()> {
    match world.get_mut::<T>(key) {
        Some(mut current) => {
            current.set_if_neq(value);
            Ok(())
        }
        None => world.insert(key, value),
    }
}

/// One frame entry that failed: logged and skipped, never the hook's error, since the
/// frame is already popped and the rest of it applies. `lactose-net` logs it under the
/// `milkman` target; the sketch has no logger.
fn skip(what: &str, key: StableKey, error: &Error) {
    let _logged = (what, key, error.to_string());
}

/// One spawn entry: an upsert for a live key, else a spawn under the origin key, bound if
/// predicted, with its slot.
fn apply_spawn(
    world: &mut World,
    key: StableKey,
    net: lactose_core::NetId,
    spawn: &StagedSpawn,
) -> Result<()> {
    if world.contains(key) {
        if let Some(position) = find::<Position>(&spawn.components) {
            write_whole(world, key, position)?;
        }
        if let Some(health) = find::<Health>(&spawn.components) {
            write_whole(world, key, health)?;
        }
        return match spawn.slot {
            Some(slot) => write_whole(world, key, RemoteSlot { slot }),
            None => world.remove::<RemoteSlot>(key).map(|_| ()),
        };
    }
    let position: Position = find(&spawn.components).unwrap_or(Position { x: 0, y: 0, z: 0 });
    match find::<Health>(&spawn.components) {
        Some(health) => world.spawn(key, spawn.region, (position, health))?,
        None => world.spawn(key, spawn.region, (position,))?,
    }
    if let Some(origin) = spawn.origin {
        world.bind(origin, net)?;
    }
    if let Some(slot) = spawn.slot {
        world.insert(key, RemoteSlot { slot })?;
    }
    Ok(())
}

/// One removal, typed by hand as the sketch is (`World::remove_erased` in milestone 4);
/// absent is fine.
fn apply_removed(world: &mut World, key: StableKey, component: ComponentKey) -> Result<()> {
    if component == Position::KEY {
        world.remove::<Position>(key)?;
    } else if component == Health::KEY {
        world.remove::<Health>(key)?;
    }
    Ok(())
}

/// One row, written as an upsert writes a component.
fn apply_row(world: &mut World, row: StagedRow<'_>) -> Result<()> {
    if row.component == Position::KEY {
        write_whole(world, row.key, decode::<Position>(row.bytes)?)
    } else if row.component == Health::KEY {
        write_whole(world, row.key, decode::<Health>(row.bytes)?)
    } else {
        Ok(())
    }
}

impl StagedState for ClientHook {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        // `tick` is the tick the step advanced the world to: the frames due at it.
        while let Some(frame) =
            world.service_mut::<StagedInbox>().expect("setup inserted it").pop_through(tick)
        {
            // Spawns first, ascending by NetId.
            for (&net, spawn) in &frame.spawns {
                let key = spawn.key(net);
                if let Err(error) = apply_spawn(world, key, net, spawn) {
                    skip("spawn", key, &error);
                }
            }
            // Then moves, ascending by key; an owned entity already moved with its owner
            // is already there.
            for (&key, &region) in &frame.moves {
                if world.contains(key)
                    && world.region_of(key) != Some(region)
                    && let Err(error) = world.move_to_region(key, region)
                {
                    skip("move", key, &error);
                }
            }
            // Then removals, ascending by (key, component); a row below re-adds one.
            for &(key, component) in &frame.removed {
                if world.contains(key)
                    && let Err(error) = apply_removed(world, key, component)
                {
                    skip("remove", key, &error);
                }
            }
            // Then rows, ascending by (key, component); an entity whose spawn was lost is
            // resent whole by the server, so its rows are skipped silently.
            for row in frame.rows() {
                if world.contains(row.key)
                    && let Err(error) = apply_row(world, row)
                {
                    skip("row", row.key, &error);
                }
            }
            // Tombstones last; an entity already gone with its region is not an error.
            for (&key, Tombstone { .. }) in &frame.despawns {
                if world.contains(key)
                    && let Err(error) = world.despawn(key)
                {
                    skip("tombstone", key, &error);
                }
            }
        }
        Ok(())
    }
}

// ---- 3. `lactose-predict`'s hook: the same frames into the prediction world ----

/// What `predict` keeps per proxy: the received positions by server tick, which a
/// re-simulated tick replays instead of teleporting (architecture section 9.4).
#[derive(Default)]
struct ProxyHistory {
    positions: BTreeMap<(StableKey, Tick), Position>,
}

struct PredictionHook {
    local: PlayerId,
    history: ProxyHistory,
    confirmed_input: Option<Tick>,
    /// The entities the local player has claimed, which this world predicts.
    claimed: std::collections::BTreeSet<StableKey>,
}

impl StagedState for PredictionHook {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        while let Some(frame) =
            world.service_mut::<StagedInbox>().expect("setup inserted it").pop_through(tick)
        {
            for (&net, spawn) in &frame.spawns {
                let key = spawn.key(net);
                // The server echoed a predicted spawn of ours: bind, so the alias survives
                // a restore (architecture section 7).
                if let Some(origin) = spawn.origin
                    && origin.player == self.local
                    && world.alias(origin).is_none()
                    && let Err(error) = world.bind(origin, net)
                {
                    skip("bind", key, &error);
                }
                // A spawn of a live key is a change of claim: `None` means the local
                // player claimed it (start predicting), `Some` that it was released.
                if world.contains(key) {
                    match spawn.slot {
                        None => {
                            self.claimed.insert(key);
                        }
                        Some(_) => {
                            self.claimed.remove(&key);
                        }
                    }
                }
            }
            for (&key, &region) in &frame.moves {
                if world.contains(key)
                    && world.region_of(key) != Some(region)
                    && let Err(error) = world.move_to_region(key, region)
                {
                    skip("move", key, &error);
                }
            }
            for &(key, component) in &frame.removed {
                if world.contains(key)
                    && let Err(error) = apply_removed(world, key, component)
                {
                    skip("remove", key, &error);
                }
            }
            // Full-precision rows of claimed entities are the reconciliation record;
            // dequantised rows of everything else are the proxies' history.
            for row in frame.rows() {
                if row.component != Position::KEY {
                    continue;
                }
                let value: Position = decode(row.bytes)?;
                if row.exact {
                    let _compare_per_field_at_full_precision = (row.key, value);
                } else {
                    self.history.positions.insert((row.key, row.tick), value);
                }
            }
            if let Some(acked) = frame.acked_input {
                self.confirmed_input = Some(acked);
            }
            for &substituted in &frame.substituted {
                let _resimulate_from = substituted;
            }
        }
        Ok(())
    }
}

// ---- 4. `app` on a client, between runs: every received frame into both inboxes, then
// the due frames' time scale and region changes into this step's `TickInput` ----

struct ClientStore<'a> {
    baseline_by_hash: &'a dyn Fn(u64) -> Option<Arc<Baseline>>,
    cached_overlay: &'a dyn Fn(RegionId) -> Option<Arc<Overlay>>,
    pin: &'a dyn Fn(&VersionTable) -> Option<PinnedTable>,
}

/// The fold rule: per region in frame order, the later change wins, except that an
/// `Irrelevant` followed by a `Relevant` becomes an unload plus a load; a full frame
/// first discards what an earlier due frame of the step folded, then unloads every loaded
/// region, so the step loads exactly the regions the full frame names.
fn fold_frame(
    input: &mut TickInput,
    frame: &StagedFrame,
    loaded_regions: &[RegionId],
    store: &ClientStore<'_>,
) -> Result<()> {
    if let Some(scale) = frame.time_scale {
        input.time_scale = Some(scale);
    }
    if frame.full {
        input.loaded.clear();
        input.assets.clear();
        input.unloaded.extend(loaded_regions.iter().copied());
    }
    for (&region, change) in &frame.regions {
        match change {
            RegionChange::Relevant(relevance) => {
                let baseline =
                    (store.baseline_by_hash)(relevance.baseline_hash).ok_or_else(|| {
                        Error::new(ErrorKind::NotFound, "the baseline is not in the store")
                    })?;
                let cached = (store.cached_overlay)(region);
                let assets = relevance.assets.as_ref().and_then(store.pin);
                input
                    .loaded
                    .insert(region, relevance.to_bundle(baseline, cached.as_ref(), assets)?);
            }
            RegionChange::Irrelevant => {
                input.loaded.remove(&region);
                input.assets.remove(&region);
                input.unloaded.insert(region);
            }
            RegionChange::Assets(table) => {
                if !input.unloaded.contains(&region)
                    && let Some(pinned) = (store.pin)(table)
                {
                    input.assets.insert(region, pinned);
                }
            }
        }
    }
    Ok(())
}

fn client_between_runs(
    replicated: &mut Simulation,
    prediction: &mut Simulation,
    from_net: &Receiver<Arc<StagedFrame>>,
    store: &ClientStore<'_>,
) -> Result<TickInput> {
    // Push first: a refused frame enters nothing.
    while let Ok(frame) = from_net.try_recv() {
        let accepted =
            replicated.world_mut().service_mut::<StagedInbox>().expect("setup").push(frame.clone());
        if accepted {
            let _ = prediction.world_mut().service_mut::<StagedInbox>().expect("setup").push(frame);
        }
    }
    // Then fold exactly the frames this step's hook will pop: the step advances the world
    // to `tick() + 1` and the hook receives that, so `due` is asked for it, not for
    // `tick()`, which would leave out every frame that arrived in time.
    let mut input = TickInput::default();
    let loaded: Vec<RegionId> = replicated.world().regions().collect();
    let next = Tick(replicated.tick().0 + 1);
    let inbox = replicated.world().service::<StagedInbox>().expect("setup");
    for frame in inbox.due(next) {
        fold_frame(&mut input, frame, &loaded, store)?;
    }
    Ok(input)
}

// ---- 5. The server: the gather in `Pass::Gather`, the frame out to the packer, the
// relevance message from an export ----

/// The crate's own `net::gather` is what `register_systems` places; a spike that wants
/// its own pass system calls the same operation.
#[system(name = "sketch::gather", worlds = [server], exclusive)]
fn spike_gather(world: &mut World, job: Job, state: ResMut<GatherState>) {
    let world: &World = world;
    state.gather(world, &mut job).expect("the gather reads the world");
}

fn register_spike_systems(registration: &mut Registration) {
    registration.pass(lactose_sim::Pass::Gather).add(&SPIKE_GATHER);
}

fn server_between_runs(simulation: &mut Simulation, to_packer: &Sender<StagingFrame>) {
    let frame = simulation.world_mut().service_mut::<GatherState>().expect("setup").take();
    if !frame.is_empty() {
        let _ = to_packer.send(frame);
    }
}

/// The server's `app` on a region becoming relevant: the export through `persist`'s diff
/// (sketched as a closure) into the message, digested for a client known to cache it.
fn relevance_message(
    export: &RegionExport,
    baseline: &Baseline,
    diff: &dyn Fn(&RegionExport, &Baseline) -> Option<Arc<Overlay>>,
    table: Option<VersionTable>,
    client_has_cached: bool,
) -> RegionRelevance {
    let overlay = diff(export, baseline);
    let message = RegionRelevance::from_export(export, baseline.content_hash(), overlay, table);
    if client_has_cached { message.with_digest() } else { message }
}

/// A frame built by hand, as the spike's receiver or a test does.
fn hand_built_frame(tick: Tick) -> StagedFrame {
    let mut frame = StagedFrame::resync(tick);
    let _ = frame.push_row(
        StableKey::Net(lactose_core::NetId::from_bits(7).expect("non-zero")),
        Position::KEY,
        tick,
        false,
        lactose_reflect::bytemuck::bytes_of(&Position { x: 1, y: 2, z: 3 }),
    );
    frame
}
