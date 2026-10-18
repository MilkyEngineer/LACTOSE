//! Handles and packets (README, "Handles and packets"): the handle table's assign,
//! release, insert and clear rules across the generation wrap and at capacity; the
//! sequence and acknowledgement arithmetic across the sequence wrap; the packet ring; and
//! a client mirror driven by the receiver rules over a network that drops, delays,
//! reorders and duplicates, which ends equal to the server's table.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;

use lactose_core::{ErrorKind, NetId, PlayerId, StableKey, Tick};
use lactose_net_host::interest::{ClientInterest, SendBudget};
use lactose_net_host::packet::{
    AckField, ConnectionHandles, EntityHandle, PacketRecord, PacketRing, PacketSequence,
};
use lactose_net_host::transport::ConnectionId;

fn key(n: u64) -> StableKey {
    StableKey::Net(
        NetId::runtime(NonZeroU64::new(n).expect("non-zero")).expect("a runtime counter"),
    )
}

fn handle(index: u16, generation: u16) -> EntityHandle {
    EntityHandle { index, generation }
}

fn kind<T: core::fmt::Debug>(result: lactose_core::Result<T>) -> ErrorKind {
    result.expect_err("an error").kind()
}

// ---- The table ----

#[test]
fn assign_reuses_the_most_recently_released_slot_at_the_next_generation() {
    let mut handles = ConnectionHandles::new();
    let assigned: Vec<EntityHandle> = (1..=6).map(|n| handles.assign(key(n)).unwrap()).collect();
    assert_eq!(assigned, (0..6).map(|index| handle(index, 1)).collect::<Vec<_>>());
    assert_eq!(handles.assign(key(3)).unwrap(), handle(2, 1), "an assigned key keeps its handle");

    assert_eq!(handles.release(handle(2, 1)), Some(key(3)));
    assert_eq!(handles.release(handle(4, 1)), Some(key(5)));
    assert_eq!(handles.release(handle(4, 1)), None, "released twice");
    assert_eq!(handles.key_of(handle(2, 1)), None, "the released handle is stale");
    assert_eq!(handles.handle_of(key(3)), None);

    assert_eq!(handles.assign(key(7)).unwrap(), handle(4, 2), "the most recently released");
    assert_eq!(handles.assign(key(8)).unwrap(), handle(2, 2));
    assert_eq!(handles.assign(key(9)).unwrap(), handle(6, 1), "then a never-used slot");
    assert_eq!(handles.key_of(handle(4, 2)), Some(key(7)));
    assert_eq!(handles.key_of(handle(4, 1)), None);
    assert_eq!(handles.key_of(handle(40, 1)), None, "never assigned");
    assert_eq!(handles.key_of(EntityHandle::NONE), None);
    assert_eq!(handles.len(), 7);

    let order: Vec<StableKey> = handles.iter().map(|(key, _)| key).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted, "ascending key order");
}

#[test]
fn a_generation_wraps_past_the_maximum_to_one_and_the_mirror_follows() {
    let mut server = ConnectionHandles::new();
    let mut client = ConnectionHandles::new();
    let mut previous = EntityHandle::NONE;
    // 65,535 generations, then the wrap, then a few more.
    for round in 0..65_540_u32 {
        let entity = key(u64::from(round) + 1);
        let assigned = server.assign(entity).unwrap();
        assert_eq!(assigned.index, 0);
        let expected = u16::try_from(round % 65_535 + 1).expect("below 65536");
        assert_eq!(assigned.generation, expected, "round {round}");
        client.insert(assigned, entity).unwrap();
        assert_eq!(server.key_of(previous), None);
        assert_eq!(client.key_of(previous), None);
        assert_eq!(server.release(assigned), Some(entity));
        assert_eq!(client.release(assigned), Some(entity));
        previous = assigned;
    }
    // After the wrap a handle from just before it is still older: inserting it is a
    // spawn reordered after its tombstone.
    let mut client = ConnectionHandles::new();
    client.insert(handle(0, u16::MAX), key(1)).unwrap();
    assert_eq!(client.release(handle(0, u16::MAX)), Some(key(1)));
    assert_eq!(kind(client.insert(handle(0, u16::MAX), key(1))), ErrorKind::InvalidData);
    assert_eq!(kind(client.insert(handle(0, u16::MAX - 100), key(2))), ErrorKind::InvalidData);
    client.insert(handle(0, 1), key(2)).unwrap();
}

#[test]
fn every_slot_assigned_is_resource_exhausted_until_one_is_released() {
    let mut handles = ConnectionHandles::new();
    for n in 0..EntityHandle::CAPACITY as u64 {
        handles.assign(key(n + 1)).unwrap();
    }
    assert_eq!(handles.len(), EntityHandle::CAPACITY);
    assert_eq!(kind(handles.assign(key(1_000_000))), ErrorKind::ResourceExhausted);
    assert_eq!(handles.assign(key(1)).unwrap(), handle(0, 1), "an assigned key still answers");
    assert_eq!(handles.release(handle(65_535, 1)), Some(key(65_536)));
    assert_eq!(handles.assign(key(1_000_000)).unwrap(), handle(65_535, 2));
}

#[test]
fn insert_refuses_what_the_receiver_rules_make_unreachable() {
    let mut mirror = ConnectionHandles::new();
    mirror.insert(handle(5, 3), key(1)).unwrap();
    mirror.insert(handle(5, 3), key(1)).unwrap();
    assert_eq!(mirror.len(), 1, "the same pair again is a no-op");
    assert_eq!(mirror.key_of(handle(5, 3)), Some(key(1)));
    assert_eq!(mirror.key_of(handle(0, 1)), None, "a slot below, never recorded");

    // Another live key in the slot; the key under another handle; the empty handle.
    assert_eq!(kind(mirror.insert(handle(5, 4), key(2))), ErrorKind::InvalidData);
    assert_eq!(kind(mirror.insert(handle(6, 1), key(1))), ErrorKind::InvalidData);
    assert_eq!(kind(mirror.insert(EntityHandle::NONE, key(3))), ErrorKind::InvalidData);

    // A slot whose generation is newer than the handle's: a spawn after its tombstone.
    assert_eq!(mirror.release(handle(5, 3)), Some(key(1)));
    assert_eq!(kind(mirror.insert(handle(5, 3), key(1))), ErrorKind::InvalidData);
    assert_eq!(kind(mirror.insert(handle(5, 2), key(1))), ErrorKind::InvalidData);
    // A newer generation is the server's next assignment, whatever it skipped.
    mirror.insert(handle(5, 9), key(1)).unwrap();
    // A slot the mirror never recorded takes any generation.
    mirror.insert(handle(9, 40_000), key(4)).unwrap();
    assert_eq!(mirror.len(), 2);
}

#[test]
fn clear_forgets_every_assignment_and_generation() {
    let mut handles = ConnectionHandles::new();
    let old: Vec<EntityHandle> = (1..=4).map(|n| handles.assign(key(n)).unwrap()).collect();
    assert_eq!(handles.release(old[1]), Some(key(2)));
    let reused = handles.assign(key(5)).unwrap();
    assert_eq!(reused, handle(1, 2));
    handles.clear();
    assert!(handles.is_empty());
    for &stale in old.iter().chain([&reused]) {
        assert_eq!(handles.key_of(stale), None);
        assert_eq!(handles.release(stale), None);
    }
    // Generations start over: the first slot is generation one again.
    assert_eq!(handles.assign(key(9)).unwrap(), handle(0, 1));
}

// ---- Sequences, acknowledgements, the ring ----

#[test]
fn sequences_and_acknowledgements_compare_across_the_wrap() {
    let near_end = PacketSequence(u16::MAX - 1);
    let wrapped = PacketSequence(3);
    assert!(wrapped.after(near_end));
    assert!(!near_end.after(wrapped));
    assert_eq!(wrapped.distance_after(near_end), 5);
    assert_eq!(near_end.distance_after(wrapped), 0);
    assert!(!wrapped.after(wrapped));
    assert!(PacketSequence(0x7FFF).after(PacketSequence(0)));
    assert!(!PacketSequence(0x8000).after(PacketSequence(0)));
    assert_eq!(PacketSequence(u16::MAX).next(), PacketSequence(0));

    // `latest` is 3; bit 0 is 2, bit 3 is 65535, bit 31 is 65507.
    let ack = AckField { latest: wrapped, bits: 1 | (1 << 3) | (1 << 31) };
    let covered: Vec<u16> = (0..=u16::MAX).filter(|&n| ack.covers(PacketSequence(n))).collect();
    assert_eq!(covered, [2, 3, 65_507, 65_535]);
}

fn record(sequence: u16) -> PacketRecord {
    PacketRecord {
        sequence: PacketSequence(sequence),
        tick: Tick(u32::from(sequence)),
        entities: vec![(handle(sequence, 1), Tick(1))],
        tombstones: Vec::new(),
    }
}

#[test]
fn the_ring_evicts_the_oldest_and_takes_exactly_what_is_asked() {
    let mut ring = PacketRing::new(4);
    let start = u16::MAX - 2;
    let sequences: Vec<u16> = (0..6).map(|n| start.wrapping_add(n)).collect();
    let evicted: Vec<Option<PacketSequence>> =
        sequences.iter().map(|&s| ring.record(record(s)).map(|r| r.sequence)).collect();
    assert_eq!(
        evicted,
        [None, None, None, None, Some(PacketSequence(start)), Some(PacketSequence(start + 1))]
    );
    assert_eq!(ring.len(), 4);
    // Held: 65535, 0, 1, 2.
    assert_eq!(ring.acknowledge(PacketSequence(0)), Some(record(0)));
    assert_eq!(ring.acknowledge(PacketSequence(0)), None, "taken once");
    assert_eq!(ring.acknowledge(PacketSequence(start)), None, "evicted");
    assert_eq!(ring.len(), 3);
    let expired: Vec<PacketSequence> =
        ring.expire(PacketSequence(2)).into_iter().map(|r| r.sequence).collect();
    assert_eq!(expired, [PacketSequence(u16::MAX), PacketSequence(1)]);
    assert_eq!(ring.len(), 1);
    assert!(ring.expire(PacketSequence(2)).is_empty());
    assert_eq!(ring.acknowledge(PacketSequence(2)), Some(record(2)));
    assert!(ring.is_empty());

    let mut none = PacketRing::new(0);
    assert_eq!(none.record(record(7)), Some(record(7)), "a ring of nothing evicts at once");
    assert_eq!(PacketRing::new(PacketRing::DEFAULT_CAPACITY).capacity(), 64);
}

// ---- The mirror under the receiver rules ----

/// A deterministic generator for the scripts and the network's choices.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// What one server packet carries, as far as handles go.
#[derive(Clone)]
struct Packet {
    sequence: PacketSequence,
    tick: Tick,
    spawns: Vec<(EntityHandle, StableKey)>,
    tombstones: Vec<EntityHandle>,
}

/// The receiver's handle side: the mirror, the newest staged tick, the packets it staged
/// (which alone it acknowledges), and the full frame it waits for after a resync.
#[derive(Default)]
struct Receiver {
    mirror: ConnectionHandles,
    staged: Option<Tick>,
    received: BTreeSet<u16>,
    latest: Option<PacketSequence>,
    awaiting_full: bool,
    dropped: u32,
    unknown_tombstones: u32,
}

impl Receiver {
    fn packet(&mut self, packet: &Packet) {
        let after_newest = self.staged.is_none_or(|newest| packet.tick > newest);
        if self.awaiting_full || !after_newest {
            self.dropped += 1;
            return;
        }
        for &(handle, key) in &packet.spawns {
            self.mirror.insert(handle, key).expect("the receiver rules keep insert valid");
        }
        for &handle in &packet.tombstones {
            if self.mirror.release(handle).is_none() {
                self.unknown_tombstones += 1;
            }
        }
        self.staged = Some(packet.tick);
        self.received.insert(packet.sequence.0);
        if self.latest.is_none_or(|latest| packet.sequence.after(latest)) {
            self.latest = Some(packet.sequence);
        }
    }

    fn full(&mut self, tick: Tick, spawns: &[(EntityHandle, StableKey)]) {
        assert!(self.awaiting_full);
        self.awaiting_full = false;
        self.mirror.clear();
        for &(handle, key) in spawns {
            self.mirror.insert(handle, key).expect("fresh handles");
        }
        self.staged = Some(tick);
    }

    fn ack(&self) -> Option<AckField> {
        let latest = self.latest?;
        let mut bits = 0;
        for n in 0..u32::from(AckField::WINDOW) {
            let sequence = latest.0.wrapping_sub(u16::try_from(n + 1).expect("small"));
            if self.received.contains(&sequence) {
                bits |= 1 << n;
            }
        }
        Some(AckField { latest, bits })
    }
}

/// The server's packet for this tick: a spawn for every entity sent whole and not owed a
/// tombstone, a tombstone for every one owed, recorded through `sent`.
fn pack(client: &mut ClientInterest, sequence: PacketSequence, tick: Tick) -> Packet {
    let mut packet = Packet { sequence, tick, spawns: Vec::new(), tombstones: Vec::new() };
    let mut record = PacketRecord { sequence, tick, ..PacketRecord::default() };
    for (&key, entity) in &client.entities {
        match entity.tombstone {
            Some(at) => {
                packet.tombstones.push(entity.handle);
                record.tombstones.push((entity.handle, at));
            }
            None if entity.sends_whole() => {
                packet.spawns.push((entity.handle, key));
                record.entities.push((entity.handle, tick));
            }
            None => {}
        }
    }
    client.sent(record);
    packet
}

/// A network leg: each message arrives after one to eight ticks, or never, or twice.
struct Leg<T> {
    in_flight: BTreeMap<(u32, u64), T>,
    loss: u64,
}

impl<T: Clone> Leg<T> {
    fn new(loss: u64) -> Self {
        Self { in_flight: BTreeMap::new(), loss }
    }

    fn send(&mut self, now: u32, message: T, rng: &mut Rng) {
        if rng.chance(self.loss) {
            return;
        }
        let copies = if rng.chance(5) { 2 } else { 1 };
        for _ in 0..copies {
            let at = now + 1 + u32::try_from(rng.below(8)).expect("small");
            self.in_flight.insert((at, rng.next()), message.clone());
        }
    }

    fn due(&mut self, now: u32) -> Vec<T> {
        let later = self.in_flight.split_off(&(now + 1, 0));
        let due = std::mem::replace(&mut self.in_flight, later);
        due.into_values().collect()
    }
}

/// A full frame on the reliable channel: when it arrives, its tick and its spawns.
type FullFrame = (u32, Tick, Vec<(EntityHandle, StableKey)>);

#[test]
fn the_mirror_ends_equal_to_the_server_table_under_loss_and_reorder() {
    for seed in [1_u64, 7, 99, 12_345] {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed);
        let mut server = ClientInterest::new(PlayerId(1), ConnectionId(1), SendBudget::default());
        let mut receiver = Receiver::default();
        let mut down: Leg<Packet> = Leg::new(20);
        let mut up: Leg<AckField> = Leg::new(20);
        let mut full_frame: Option<FullFrame> = None;
        let mut sequence = PacketSequence(u16::MAX - 300);
        let mut stats = (0_u32, 0_u32);
        for now in 1..=3_000_u32 {
            let tick = Tick(now);
            let calm = now > 2_500;
            if !calm {
                for _ in 0..3 {
                    let entity = key(1 + rng.below(40));
                    let relevant = server.entities.get(&entity).is_some_and(|interest| {
                        interest.tombstone.is_none() || interest.reentering
                    });
                    if relevant {
                        server.leave(entity, tick);
                        stats.0 += 1;
                    } else {
                        server.enter(entity).unwrap();
                        stats.1 += 1;
                    }
                }
                if now == 1_200 {
                    // A resync: the client asks and waits; the server starts over and
                    // sends the full frame on the reliable channel.
                    receiver.awaiting_full = true;
                    server.resync(tick);
                    let spawns =
                        server.entities.iter().map(|(&key, entity)| (entity.handle, key)).collect();
                    full_frame = Some((now + 6, tick, spawns));
                }
            }
            if let Some((at, full_tick, spawns)) = &full_frame
                && *at == now
            {
                receiver.full(*full_tick, spawns);
                full_frame = None;
            }
            for packet in down.due(now) {
                receiver.packet(&packet);
            }
            if let Some(ack) = receiver.ack() {
                up.send(now, ack, &mut rng);
            }
            for ack in up.due(now) {
                server.acknowledge(ack);
            }
            let packet = pack(&mut server, sequence, tick);
            sequence = sequence.next();
            if calm {
                down.loss = 0;
                up.loss = 0;
            }
            down.send(now, packet, &mut rng);
        }
        assert!(server.entities.values().all(|e| e.tombstone.is_none() && e.acked.is_some()));
        let server_table: Vec<_> = server.handles.iter().collect();
        let mirror: Vec<_> = receiver.mirror.iter().collect();
        assert_eq!(mirror, server_table, "seed {seed}");
        assert_eq!(server.handles.len(), server.entities.len());
        // The cases the rules exist for happened.
        assert!(receiver.dropped > 100, "seed {seed}: {} dropped", receiver.dropped);
        assert!(receiver.unknown_tombstones > 0, "seed {seed}");
        assert!(stats.0 > 1_000 && stats.1 > 1_000, "seed {seed}: {stats:?}");
    }
}
