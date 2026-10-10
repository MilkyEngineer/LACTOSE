//! Compile-only consumer sketches against the facade: what `app`, the milestone 1 spike,
//! `lactose-net`, `lactose-predict` and the presentation schedule write, type-checked and
//! never run (every deciding body of the crate is `todo!()`). They pin the surface the
//! README promises: a change that breaks one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use lactose_asset_api::{PinnedTable, VersionTable};
use lactose_bundle::{Baseline, ComponentData, Overlay};
use lactose_core::{
    ComponentKey, Error, ErrorKind, NetId, PlayerId, PredictionKey, Result, StableKey, Tick,
};
use lactose_ecs::{Job, RegionId, ResMut, World, WorldKind, system};
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_api::handshake::{BoundaryBuildId, CookedDataHash, Handshake, ProtocolHash};
use lactose_net_api::interest::{ClientInterest, SendBudget};
use lactose_net_api::interpolation::{
    InterpolationBuffer, InterpolationReader, InterpolationWriter, PresentationTime, ReceivedState,
    RemoteSample, SampleStatus,
};
use lactose_net_api::packet::{
    AckField, ConnectionHandles, EntityHandle, PacketRecord, PacketSequence, TickHeader,
};
use lactose_net_api::transport::{
    ConnectionId, Datagram, DisconnectReason, LinkStats, MAX_DATAGRAM_BYTES, Transport,
    TransportEvent,
};
use lactose_net_api::{
    GatherState, InterpolationSlot, QuantisedPosition, QuantisedRotation, RegionChange,
    RegionRelevance, RemoteSlot, StagedFrame, StagedInbox, StagedSpawn, StagingFrame, Tombstone,
};
use lactose_reflect::{Reflect, component, decode};
use lactose_sim::{
    RegionExport, Registration, Simulation, SimulationBuilder, StagedState, TickInput, TypeTable,
};

// ---- 1. Two replicated components of a game crate, and `app`'s build for every kind ----

#[component(key = "sketch::position", persistent, replicated)]
pub struct Position {
    #[field(id = 1)]
    pub x: Scalar,
    #[field(id = 2)]
    pub y: Scalar,
    #[field(id = 3)]
    pub z: Scalar,
}

#[component(key = "sketch::health", persistent, replicated)]
pub struct Health(#[field(id = 1)] pub i32);

fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Position>()?;
    types.register::<Health>()?;
    lactose_net_api::register_types(types)
}

/// `app`: the same three calls for the server, the client and the prediction world; the
/// client worlds get their `StagedState` hook, the server gets the gather.
fn app_build(kind: WorldKind, hook: Option<Box<dyn StagedState>>) -> Result<Simulation> {
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    let mut builder = SimulationBuilder::new(kind, types)
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
/// the erased write the README proposes.
struct ClientHook;

fn find<T: Reflect>(components: &[ComponentData]) -> Option<T> {
    let data = components.iter().find(|data| data.component == T::KEY)?;
    decode(&data.bytes).ok()
}

impl StagedState for ClientHook {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        while let Some(frame) =
            world.service_mut::<StagedInbox>().expect("setup inserted it").pop_through(tick)
        {
            // Spawns first, ascending by NetId, under the origin key, bound if predicted.
            for (&net, spawn) in &frame.spawns {
                let key = spawn.key(net);
                let position: Position = find(&spawn.components).unwrap_or(Position {
                    x: Scalar::ZERO,
                    y: Scalar::ZERO,
                    z: Scalar::ZERO,
                });
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
            }
            // Then rows, ascending by (key, component): equal values do not dirty the row.
            for row in frame.rows() {
                if row.component == Position::KEY {
                    let value: Position = decode(row.bytes)?;
                    match world.get_mut::<Position>(row.key) {
                        Some(mut current) => {
                            current.set_if_neq(value);
                        }
                        None => world.insert(row.key, value)?,
                    }
                } else if row.component == Health::KEY {
                    let value: Health = decode(row.bytes)?;
                    match world.get_mut::<Health>(row.key) {
                        Some(mut current) => {
                            current.set_if_neq(value);
                        }
                        None => world.insert(row.key, value)?,
                    }
                }
            }
            // Tombstones last; an entity already gone with its region is not an error.
            for (&key, Tombstone { .. }) in &frame.despawns {
                if world.contains(key) {
                    world.despawn(key)?;
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
}

impl StagedState for PredictionHook {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        while let Some(frame) =
            world.service_mut::<StagedInbox>().expect("setup inserted it").pop_through(tick)
        {
            // The server echoed a predicted spawn of ours: bind, so the alias survives a
            // restore (architecture section 7).
            for (&net, spawn) in &frame.spawns {
                if let Some(origin) = spawn.origin
                    && origin.player == self.local
                {
                    world.bind(origin, net)?;
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

// ---- 4. `app` on a client, between runs: frames from the network thread into the inputs
// and the inboxes of both worlds ----

fn client_between_runs(
    replicated: &mut Simulation,
    prediction: &mut Simulation,
    from_net: &Receiver<Arc<StagedFrame>>,
    baseline_by_hash: &dyn Fn(u64) -> Option<Arc<Baseline>>,
    cached_overlay: &dyn Fn(RegionId) -> Option<Arc<Overlay>>,
    pin: &dyn Fn(&VersionTable) -> Option<PinnedTable>,
) -> Result<TickInput> {
    let mut input = TickInput::default();
    while let Ok(frame) = from_net.try_recv() {
        if let Some(scale) = frame.time_scale {
            input.time_scale = Some(scale);
        }
        for (&region, change) in &frame.regions {
            match change {
                RegionChange::Relevant(relevance) => {
                    let baseline = baseline_by_hash(relevance.baseline_hash).ok_or_else(|| {
                        Error::new(ErrorKind::NotFound, "the baseline is not in the store")
                    })?;
                    let cached = cached_overlay(region);
                    let assets = relevance.assets.as_ref().and_then(pin);
                    input
                        .loaded
                        .insert(region, relevance.to_bundle(baseline, cached.as_ref(), assets)?);
                }
                RegionChange::Irrelevant => {
                    input.unloaded.insert(region);
                }
                RegionChange::Assets(table) => {
                    if let Some(pinned) = pin(table) {
                        input.assets.insert(region, pinned);
                    }
                }
            }
        }
        replicated.world_mut().service_mut::<StagedInbox>().expect("setup").push(frame.clone());
        prediction.world_mut().service_mut::<StagedInbox>().expect("setup").push(frame);
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

// ---- 6. The packer: a frame's per-region dirty lists against one client's interest ----

/// A packer that fell behind merges the frames it finds, newest last.
fn packer_frame(from_gather: &Receiver<StagingFrame>) -> Option<StagingFrame> {
    let mut merged: Option<StagingFrame> = None;
    while let Ok(frame) = from_gather.try_recv() {
        match &mut merged {
            Some(older) => older.merge(frame),
            None => merged = Some(frame),
        }
    }
    merged
}

fn pack_one(
    frame: &StagingFrame,
    client: &mut ClientInterest,
    region_origin: &dyn Fn(RegionId) -> Vec3,
    sequence: PacketSequence,
) -> Result<PacketRecord> {
    let mut record = PacketRecord { sequence, tick: frame.tick, entities: Vec::new() };
    let header = TickHeader { sequence, tick: frame.tick, ack: AckField::default() };
    let regions: Vec<RegionId> = client.regions.iter().copied().collect();
    for region in regions {
        let origin = region_origin(region);
        for row in frame.region(region) {
            let Some(interest) = client.entities.get_mut(&row.key) else { continue };
            if interest.acked.is_some_and(|acked| acked == row.tick) {
                continue;
            }
            if row.component == Position::KEY {
                let value: Position = decode(row.bytes)?;
                let relative = Vec3::new(value.x, value.y, value.z) - origin;
                let _on_the_wire = (interest.handle, QuantisedPosition::quantise(relative));
            }
            interest.priority.reset();
            interest.in_flight = Some(sequence);
            record.entities.push((interest.handle, row.tick));
        }
    }
    for entry in frame.structural() {
        if let lactose_ecs::StructuralChange::Despawned { key, .. } = entry.change {
            client.leave(key, entry.tick);
        }
    }
    let _evicted = client.packets.record(record.clone());
    let _ = header;
    Ok(record)
}

fn on_ack(client: &mut ClientInterest, header: &TickHeader) {
    client.acknowledge(header.ack);
}

fn client_joins(player: PlayerId, connection: ConnectionId) -> ClientInterest {
    ClientInterest::new(player, connection, SendBudget::default())
}

// ---- 7. The client's network thread: wire values into a staged frame and the
// interpolation buffer ----

struct ClientReceiver {
    handles: ConnectionHandles,
    slots: BTreeMap<StableKey, InterpolationSlot>,
    writer: InterpolationWriter,
    to_app: Sender<Arc<StagedFrame>>,
}

impl ClientReceiver {
    fn new(to_app: Sender<Arc<StagedFrame>>) -> (Self, InterpolationReader) {
        let (writer, reader) = InterpolationBuffer::split(4096);
        (Self { handles: ConnectionHandles::new(), slots: BTreeMap::new(), writer, to_app }, reader)
    }

    fn spawn(
        &mut self,
        frame: &mut StagedFrame,
        net: NetId,
        origin: Option<PredictionKey>,
        region: RegionId,
        handle: EntityHandle,
        components: Vec<ComponentData>,
    ) -> Result<()> {
        let key = origin.map_or(StableKey::Net(net), StableKey::Predicted);
        let assigned = self.handles.assign(key)?;
        debug_assert_eq!(assigned, handle, "the client mirrors the server's handle table");
        let slot = self.writer.allocate()?;
        self.slots.insert(key, slot);
        frame.spawns.insert(net, StagedSpawn { origin, region, slot: Some(slot), components });
        Ok(())
    }

    fn row(
        &mut self,
        frame: &mut StagedFrame,
        handle: EntityHandle,
        tick: Tick,
        origin: Vec3,
        position: QuantisedPosition,
        rotation: QuantisedRotation,
    ) -> Result<()> {
        let key = self
            .handles
            .key_of(handle)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "a stale handle"))?;
        let position = position.dequantise() + origin;
        let image = Position { x: position.x, y: position.y, z: position.z };
        frame.push_row(
            key,
            Position::KEY,
            tick,
            false,
            lactose_reflect::bytemuck::bytes_of(&image),
        );
        if let Some(&slot) = self.slots.get(&key) {
            let state =
                ReceivedState { tick, teleport: false, position, rotation: rotation.dequantise() };
            self.writer.push(slot, state)?;
        }
        Ok(())
    }

    fn tombstone(
        &mut self,
        frame: &mut StagedFrame,
        handle: EntityHandle,
        region: RegionId,
        tick: Tick,
    ) -> Result<()> {
        let key = self.handles.release(handle).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, "a tombstone for a handle never assigned")
        })?;
        if let Some(slot) = self.slots.remove(&key) {
            self.writer.release(slot)?;
        }
        frame.despawns.insert(key, Tombstone { region, tick });
        Ok(())
    }

    fn finish(&mut self, frame: StagedFrame) {
        let _ = self.to_app.send(Arc::new(frame));
    }
}

// ---- 8. The presentation schedule: slots from the rows, one batch sample, render space ----

fn present_remote(
    world: &World,
    reader: &InterpolationReader,
    at: PresentationTime,
    samples: &mut Vec<RemoteSample>,
    uploads: &mut Vec<(StableKey, RenderPosition, Quat)>,
) {
    let mut keys = Vec::new();
    let mut slots = Vec::new();
    for column in world.columns::<RemoteSlot>() {
        for (key, value) in column.keys().iter().zip(column.values()) {
            keys.push(*key);
            slots.push(value.slot);
        }
    }
    samples.clear();
    samples.resize(slots.len(), RemoteSample::MISSING);
    reader.sample_batch(&slots, at, samples);
    for (key, sample) in keys.iter().zip(samples.iter()) {
        if sample.status != SampleStatus::Missing {
            uploads.push((*key, RenderPosition::from_sim(sample.position), sample.rotation));
        }
    }
}

// ---- 9. The handshake, from the registry the world was built with ----

fn our_handshake(types: &TypeTable, build: BoundaryBuildId, cooked: CookedDataHash) -> Handshake {
    Handshake { protocol: ProtocolHash::of(types.registry()), build, cooked }
}

fn accept(ours: Handshake, first_datagram: &[u8]) -> Result<()> {
    let bytes: &[u8; Handshake::ENCODED_LEN] = first_datagram
        .try_into()
        .map_err(|_| Error::new(ErrorKind::InvalidData, "a handshake is 24 bytes"))?;
    ours.check(Handshake::from_bytes(bytes))?;
    Ok(())
}

// ---- 10. A loopback transport: the whole implementation, one page ----

/// Two ends joined by channels; each names the other by one connection ID.
struct Loopback {
    peer: ConnectionId,
    me_at_peer: ConnectionId,
    tx: Sender<Datagram>,
    rx: Receiver<Datagram>,
    announced: bool,
    open: bool,
    closing: bool,
    stats: LinkStats,
}

impl Loopback {
    fn pair() -> (Self, Self) {
        let (a_tx, a_rx) = channel();
        let (b_tx, b_rx) = channel();
        let end = |peer, me_at_peer, tx, rx| Self {
            peer,
            me_at_peer,
            tx,
            rx,
            announced: false,
            open: true,
            closing: false,
            stats: LinkStats { rtt: Some(Duration::ZERO), ..LinkStats::default() },
        };
        (
            end(ConnectionId(1), ConnectionId(2), a_tx, b_rx),
            end(ConnectionId(2), ConnectionId(1), b_tx, a_rx),
        )
    }
}

impl Transport for Loopback {
    fn send(&mut self, to: ConnectionId, payload: &[u8]) -> Result<()> {
        if to != self.peer || !self.open {
            return Err(Error::new(ErrorKind::NotFound, "no such connection"));
        }
        let datagram = Datagram::new(self.me_at_peer, payload.to_vec())?;
        self.tx.send(datagram).map_err(|_| Error::new(ErrorKind::Network, "the peer is gone"))?;
        self.stats.datagrams_sent += 1;
        self.stats.bytes_sent += payload.len() as u64;
        Ok(())
    }

    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        if !self.announced {
            self.announced = true;
            events.push(TransportEvent::Connected(self.peer));
        }
        while let Ok(datagram) = self.rx.try_recv() {
            if !self.open {
                break;
            }
            self.stats.datagrams_received += 1;
            self.stats.bytes_received += datagram.payload().len() as u64;
            events.push(TransportEvent::Received(datagram));
        }
        if self.closing {
            self.closing = false;
            events.push(TransportEvent::Disconnected {
                connection: self.peer,
                reason: DisconnectReason::Local,
            });
        }
    }

    fn stats(&self, connection: ConnectionId) -> Option<LinkStats> {
        (connection == self.peer && self.open).then_some(self.stats)
    }

    fn disconnect(&mut self, connection: ConnectionId) {
        if connection == self.peer && self.open {
            self.open = false;
            self.closing = true;
        }
    }
}

/// The network thread's loop shape: a transport behind the trait, events in, datagrams
/// out, bounded by the datagram size.
fn network_thread_step(transport: &mut dyn Transport, events: &mut Vec<TransportEvent>) {
    events.clear();
    transport.poll(events);
    for event in events.drain(..) {
        match event {
            TransportEvent::Connected(connection) => {
                let _ = transport.send(connection, &[0; MAX_DATAGRAM_BYTES]);
            }
            TransportEvent::Received(datagram) => {
                let _ = (datagram.connection, datagram.payload());
            }
            TransportEvent::Disconnected { connection, reason } => {
                let _ = (connection, reason);
            }
        }
    }
}

// ---- 11. Values the spike pins ----

fn constants_pinned() {
    let _ = ComponentKey::from_name("net::remote_slot");
    let _: usize = Handshake::ENCODED_LEN;
    let _ = EntityHandle::CAPACITY;
    let _ = lactose_net_api::quant::POSITION_UNITS_PER_METRE;
    let _ = lactose_net_api::interpolation::STATES_PER_SLOT;
    let _ = lactose_net_api::clock::MAX_RATE_ADJUST_THOUSANDTHS;
    let _ = lactose_net_api::lag::LAG_COMPENSATION_MAX_TICKS;
    let _ = lactose_net_api::handshake::PROTOCOL_VERSION;
}
