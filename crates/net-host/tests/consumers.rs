//! Consumer sketches: what `lactose-net`'s packer and receiver, the presentation
//! schedule, `app`'s join and a throwaway transport write, type-checked against the
//! surface. They pin the surface the README promises: a change that breaks one is a
//! signature change to list. The loopback transport is also run, at the end, against the
//! trait's contract; the packer and receiver run as models in `replication.rs`.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs, clippy::too_many_arguments)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use lactose_bundle::ComponentData;
use lactose_core::{
    ComponentKey, Error, ErrorKind, NetId, PlayerId, PredictionKey, Result, StableKey, Tick,
};
use lactose_ecs::{RegionId, World, WorldKind};
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_api::{
    InterpolationSlot, RemoteSlot, StagedFrame, StagedSpawn, StagingFrame, Tombstone,
};
use lactose_net_host::handshake::{BoundaryBuildId, CookedDataHash, Handshake, ProtocolHash};
use lactose_net_host::interest::{ClientInterest, SendBudget};
use lactose_net_host::interpolation::{
    InterpolationBuffer, InterpolationReader, InterpolationWriter, PresentationTime, ReceivedState,
    RemoteSample, SampleStatus,
};
use lactose_net_host::message::JoinAccepted;
use lactose_net_host::packet::{
    AckField, ConnectionHandles, EntityHandle, PacketRecord, PacketSequence, TickHeader,
};
use lactose_net_host::transport::{
    ConnectionId, Datagram, DisconnectReason, LinkStats, MAX_DATAGRAM_BYTES, Transport,
    TransportEvent,
};
use lactose_net_host::{QuantisedPosition, QuantisedRotation};
use lactose_reflect::{Reflect, component, decode};
use lactose_sim::{Simulation, SimulationBuilder, TypeTable};

/// The sketch's replicated transform: three scalars and a teleport tick, as the physics
/// crate's components are shaped.
#[component(key = "sketch::position", persistent, replicated)]
pub struct Position {
    #[field(id = 1)]
    pub x: Scalar,
    #[field(id = 2)]
    pub y: Scalar,
    #[field(id = 3)]
    pub z: Scalar,
}

#[component(key = "sketch::teleported", persistent, replicated)]
pub struct Teleported(#[field(id = 1)] pub Tick);

// ---- 1. The packer: a mirror built from the gather's frames, packed per client ----

/// Every replicated row of every relevant region, as the server last gathered it: rows
/// replace, `Despawned` removes the entity's rows, `Removed` drops that row, `Moved`
/// re-homes; a full frame replaces the mirror outright.
#[derive(Default)]
struct Mirror {
    rows: BTreeMap<(StableKey, ComponentKey), (RegionId, Tick, Vec<u8>)>,
}

impl Mirror {
    fn absorb(&mut self, frame: &StagingFrame) {
        if frame.full {
            self.rows.clear();
        }
        for row in frame.rows() {
            self.rows.insert((row.key, row.component), (row.region, row.tick, row.bytes.to_vec()));
        }
        for entry in frame.structural() {
            match entry.change {
                lactose_ecs::StructuralChange::Despawned { key, .. } => {
                    self.rows.retain(|(k, _), _| *k != key);
                }
                lactose_ecs::StructuralChange::Removed { key, component } => {
                    self.rows.remove(&(key, component));
                }
                lactose_ecs::StructuralChange::Moved { key, to, .. } => {
                    for ((k, _), (region, _, _)) in self.rows.iter_mut() {
                        if *k == key {
                            *region = to;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

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
    mirror: &Mirror,
    client: &mut ClientInterest,
    region_origin: &dyn Fn(RegionId) -> Vec3,
    sequence: PacketSequence,
) -> Result<PacketRecord> {
    let mut record =
        PacketRecord { sequence, tick: frame.tick, entities: Vec::new(), tombstones: Vec::new() };
    let header = TickHeader { sequence, tick: frame.tick, ack: AckField::default() };
    let regions: Vec<RegionId> = client.regions.iter().copied().collect();
    for region in regions {
        let origin = region_origin(region);
        for row in frame.region(region) {
            let Some(interest) = client.entities.get_mut(&row.key) else { continue };
            if interest.tombstone.is_some() || interest.sends_whole() {
                continue;
            }
            if interest.acked.is_some_and(|acked| acked == row.tick) {
                continue;
            }
            if row.component == Position::KEY {
                let value: Position = decode(row.bytes)?;
                let relative = Vec3::new(value.x, value.y, value.z) - origin;
                // The wire names the handle and the region the position is relative to.
                let _on_the_wire = (interest.handle, region, QuantisedPosition::quantise(relative));
            }
            record.entities.push((interest.handle, row.tick));
        }
    }
    // An entity entering, a lost row or a claimed entity: whole from the mirror, not from
    // the frame.
    for (key, interest) in &client.entities {
        if interest.tombstone.is_none() && interest.sends_whole() {
            for ((k, component), (region, tick, bytes)) in &mirror.rows {
                if k == key {
                    let _whole = (interest.handle, *region, *component, *tick, bytes.as_slice());
                }
            }
            record.entities.push((interest.handle, frame.tick));
        }
    }
    for entry in frame.structural() {
        match entry.change {
            lactose_ecs::StructuralChange::Despawned { key, .. } => client.leave(key, entry.tick),
            lactose_ecs::StructuralChange::Removed { key, component } => {
                // On the wire as a removal, into `StagedFrame::removed` at the receiver.
                let _removal_on_the_wire = client.entities.get(&key).map(|e| (e.handle, component));
            }
            _ => {}
        }
    }
    for interest in client.entities.values() {
        if let Some(tick) = interest.tombstone
            && interest.in_flight.is_none()
        {
            record.tombstones.push((interest.handle, tick));
        }
    }
    client.sent(record.clone());
    let _ = header;
    Ok(record)
}

fn on_ack(client: &mut ClientInterest, header: &TickHeader) {
    client.acknowledge(header.ack);
}

fn client_joins(player: PlayerId, connection: ConnectionId) -> ClientInterest {
    ClientInterest::new(player, connection, SendBudget::default())
}

/// The server on a `ResyncRequest`: the client's handles cleared and every relevant
/// entity re-entered, so the full frame's spawns carry fresh handles.
fn on_resync_request(client: &mut ClientInterest, mirror: &Mirror, tick: Tick) {
    client.resync(tick);
    let _full_frame_from = (&mirror.rows, client.entities.keys());
}

// ---- 2. The client's network thread: wire values into a staged frame and the
// interpolation buffer, with the server's handles mirrored ----

struct ClientReceiver {
    handles: ConnectionHandles,
    slots: BTreeMap<StableKey, InterpolationSlot>,
    newest: BTreeMap<(StableKey, ComponentKey), Tick>,
    /// The tick of the newest frame staged: a packet is staged only if its tick is after.
    staged: Option<Tick>,
    /// Set from a `ResyncRequest` or `JoinAccepted` until the full frame for that tick is
    /// staged: nothing else is staged meanwhile.
    awaiting_full: Option<Tick>,
    writer: InterpolationWriter,
    to_app: Sender<Arc<StagedFrame>>,
}

/// What the receiver does with one server packet, by the rules of the README.
enum Admission {
    /// Stage it: its tick is after the newest frame staged.
    Stage,
    /// Drop it whole and do not acknowledge it, so the server resends what it carried.
    Drop,
}

impl ClientReceiver {
    fn new(to_app: Sender<Arc<StagedFrame>>) -> (Self, InterpolationReader) {
        let (writer, reader) = InterpolationBuffer::split(4096);
        let receiver = Self {
            handles: ConnectionHandles::new(),
            slots: BTreeMap::new(),
            newest: BTreeMap::new(),
            staged: None,
            awaiting_full: None,
            writer,
            to_app,
        };
        (receiver, reader)
    }

    /// A `JoinAccepted` or a `ResyncRequest` sent: from now until the full frame for
    /// `tick` nothing is staged, and every packet not after `tick` is dropped.
    fn expect_full(&mut self, tick: Tick) {
        self.awaiting_full = Some(tick);
    }

    /// The gate every server packet passes first: a delta packet is staged only if its
    /// header tick is after the newest frame staged and no full frame is awaited; a full
    /// frame only if it is the awaited one.
    fn admit(&self, header: &TickHeader, full: bool) -> Admission {
        let after_newest = self.staged.is_none_or(|newest| header.tick > newest);
        match (self.awaiting_full, full) {
            (Some(awaited), true) if header.tick == awaited => Admission::Stage,
            (Some(_), _) => Admission::Drop,
            (None, true) => Admission::Drop,
            (None, false) if after_newest => Admission::Stage,
            (None, false) => Admission::Drop,
        }
    }

    /// A delta packet admitted: the frame it is staged into.
    fn begin_delta(&mut self, header: &TickHeader) -> StagedFrame {
        self.staged = Some(header.tick);
        StagedFrame::new(header.tick)
    }

    /// The awaited full frame, admitted: the mirror and the slots start over now, not
    /// when the request was sent, since packets from before it were still being staged.
    fn begin_full(&mut self, tick: Tick) -> Result<StagedFrame> {
        debug_assert_eq!(self.awaiting_full, Some(tick));
        self.awaiting_full = None;
        self.staged = Some(tick);
        self.handles.clear();
        for (_, slot) in std::mem::take(&mut self.slots) {
            self.writer.release(slot)?;
        }
        self.newest.clear();
        Ok(StagedFrame::resync(tick))
    }

    /// A component the server removed from a live entity.
    fn removed(
        &mut self,
        frame: &mut StagedFrame,
        handle: EntityHandle,
        component: ComponentKey,
    ) -> Result<()> {
        let key = self
            .handles
            .key_of(handle)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "a stale handle"))?;
        self.newest.remove(&(key, component));
        frame.removed.insert((key, component));
        Ok(())
    }

    fn spawn(
        &mut self,
        frame: &mut StagedFrame,
        net: NetId,
        origin: Option<PredictionKey>,
        region: RegionId,
        handle: EntityHandle,
        claimed: bool,
        components: Vec<ComponentData>,
    ) -> Result<()> {
        let key = origin.map_or(StableKey::Net(net), StableKey::Predicted);
        self.handles.insert(handle, key)?;
        let slot = if claimed {
            if let Some(slot) = self.slots.remove(&key) {
                self.writer.release(slot)?;
            }
            None
        } else {
            Some(match self.slots.get(&key) {
                Some(&slot) => slot,
                None => {
                    let slot = self.writer.allocate()?;
                    self.slots.insert(key, slot);
                    slot
                }
            })
        };
        frame.spawns.insert(net, StagedSpawn { origin, region, slot, components });
        Ok(())
    }

    fn row(
        &mut self,
        frame: &mut StagedFrame,
        handle: EntityHandle,
        tick: Tick,
        region: RegionId,
        origin_of: &dyn Fn(RegionId) -> Vec3,
        position: QuantisedPosition,
        rotation: QuantisedRotation,
        teleported: Tick,
    ) -> Result<()> {
        let key = self
            .handles
            .key_of(handle)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "a stale handle"))?;
        // Never a row older than one already staged for (key, component).
        if self.newest.get(&(key, Position::KEY)).is_some_and(|&newest| tick <= newest) {
            return Ok(());
        }
        self.newest.insert((key, Position::KEY), tick);
        let position = position.dequantise() + origin_of(region);
        let image = Position { x: position.x, y: position.y, z: position.z };
        let _ = frame.push_row(
            key,
            Position::KEY,
            tick,
            false,
            lactose_reflect::bytemuck::bytes_of(&image),
        );
        if let Some(&slot) = self.slots.get(&key) {
            let state =
                ReceivedState { tick, teleported, position, rotation: rotation.dequantise() };
            self.writer.push(slot, state)?;
        }
        Ok(())
    }

    fn moved(&mut self, frame: &mut StagedFrame, handle: EntityHandle, to: RegionId) -> Result<()> {
        let key = self
            .handles
            .key_of(handle)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "a stale handle"))?;
        frame.moves.insert(key, to);
        Ok(())
    }

    fn tombstone(
        &mut self,
        frame: &mut StagedFrame,
        handle: EntityHandle,
        region: RegionId,
        tick: Tick,
    ) -> Result<()> {
        // A tombstone for a handle the mirror does not hold: the entity left before its
        // spawn was ever staged.
        let Some(key) = self.handles.release(handle) else { return Ok(()) };
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

// ---- 3. The presentation schedule: slots from the rows, one batch sample, render space ----

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

// ---- 4. The handshake and the join ----

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

/// `app` on a client: the worlds start at the server's tick, so the full frame for that
/// tick is due at their first sync point.
fn client_worlds(accepted: JoinAccepted) -> Result<(Simulation, Simulation)> {
    let build = |kind| {
        let mut types = TypeTable::new();
        lactose_net_api::register_types(&mut types)?;
        SimulationBuilder::new(kind, types)
            .tick(accepted.tick)
            .setup(lactose_net_api::setup)
            .systems(lactose_net_api::register_systems)
            .build()
    };
    let _local = accepted.player;
    Ok((build(WorldKind::Client)?, build(WorldKind::Prediction)?))
}

// ---- 5. A loopback transport: the whole implementation, one page ----

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

// ---- 6. Values the spike pins ----

fn constants_pinned() {
    let _: usize = Handshake::ENCODED_LEN;
    let _ = EntityHandle::CAPACITY;
    let _ = lactose_net_host::quant::POSITION_UNITS_PER_METRE;
    let _ = lactose_net_host::interpolation::STATES_PER_SLOT;
    let _ = lactose_net_host::clock::MAX_RATE_ADJUST_THOUSANDTHS;
    let _ = lactose_net_host::handshake::PROTOCOL_VERSION;
}

// ---- 7. The loopback, run against the trait's contract ----

#[test]
fn the_loopback_keeps_the_transport_contract() {
    let (mut a, mut b) = Loopback::pair();
    let mut events = Vec::new();
    a.poll(&mut events);
    assert!(matches!(events.as_slice(), [TransportEvent::Connected(ConnectionId(1))]));
    assert_eq!(a.stats(ConnectionId(1)).and_then(|stats| stats.rtt), Some(Duration::ZERO));
    assert!(a.stats(ConnectionId(7)).is_none());

    // The size limit: a payload over it fails with `InvalidData` and nothing is sent.
    let too_big = vec![0; MAX_DATAGRAM_BYTES + 1];
    let error = a.send(ConnectionId(1), &too_big).expect_err("over the limit");
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    a.send(ConnectionId(1), &[1, 2, 3]).expect("an established connection");
    a.send(ConnectionId(1), &[0; MAX_DATAGRAM_BYTES]).expect("at the limit");
    let error = a.send(ConnectionId(9), &[1]).expect_err("an unknown connection");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    // Per connection: `Connected` first, then what was received, in order.
    events.clear();
    b.poll(&mut events);
    let received: Vec<(ConnectionId, usize)> = events
        .iter()
        .filter_map(|event| match event {
            TransportEvent::Received(datagram) => {
                Some((datagram.connection, datagram.payload().len()))
            }
            _ => None,
        })
        .collect();
    assert!(matches!(events.first(), Some(TransportEvent::Connected(ConnectionId(2)))));
    assert_eq!(received, [(ConnectionId(2), 3), (ConnectionId(2), MAX_DATAGRAM_BYTES)]);
    assert_eq!(a.stats(ConnectionId(1)).map(|stats| stats.datagrams_sent), Some(2));

    // `disconnect` makes a later poll yield `Disconnected` with `Local`, last; a send after
    // it fails with `NotFound`; on an unknown connection it does nothing.
    a.disconnect(ConnectionId(5));
    a.disconnect(ConnectionId(1));
    let error = a.send(ConnectionId(1), &[1]).expect_err("disconnected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    events.clear();
    a.poll(&mut events);
    assert!(matches!(
        events.last(),
        Some(TransportEvent::Disconnected { connection: ConnectionId(1), reason: DisconnectReason::Local })
    ));
    assert!(a.stats(ConnectionId(1)).is_none());
    events.clear();
    a.poll(&mut events);
    assert!(events.is_empty(), "nothing after `Disconnected`");
}
