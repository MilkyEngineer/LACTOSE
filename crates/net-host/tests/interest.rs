//! Per-client interest (README, "Interest and the packer"): whole sends until an
//! acknowledgement and for claimed entities always; an acknowledgement advances exactly
//! the entities a packet carried and clears exactly the acknowledged `in_flight`; a lost,
//! expired or evicted packet re-queues its entities; an acknowledged tombstone releases
//! the handle, or re-enters an entity that became relevant again; a whole resend is not
//! undone by an older packet's acknowledgement; a resync clears and refills the handles.

#![forbid(unsafe_code)]

use std::num::NonZeroU64;

use lactose_core::{NetId, PlayerId, StableKey, Tick};
use lactose_ecs::RegionId;
use lactose_net_api::{Tombstone, TombstoneCause};
use lactose_net_host::interest::{ClientInterest, Priority, SendBudget};
use lactose_net_host::packet::{
    AckField, EntityHandle, PacketRecord, PacketRing, PacketSequence, TickHeader,
};
use lactose_net_host::transport::ConnectionId;

fn key(n: u64) -> StableKey {
    StableKey::Net(
        NetId::runtime(NonZeroU64::new(n).expect("non-zero")).expect("a runtime counter"),
    )
}

fn client() -> ClientInterest {
    ClientInterest::new(PlayerId(1), ConnectionId(1), SendBudget::default())
}

/// A packet carrying the state of `keys` at `tick`.
fn carrying(client: &ClientInterest, sequence: u16, tick: u32, keys: &[u64]) -> PacketRecord {
    PacketRecord {
        sequence: PacketSequence(sequence),
        tick: Tick(tick),
        entities: keys.iter().map(|&n| (client.entities[&key(n)].handle, Tick(tick))).collect(),
        tombstones: Vec::new(),
    }
}

/// A packet carrying the tombstones `client` owes for `keys`.
fn tombstones(client: &ClientInterest, sequence: u16, tick: u32, keys: &[u64]) -> PacketRecord {
    PacketRecord {
        sequence: PacketSequence(sequence),
        tick: Tick(tick),
        entities: Vec::new(),
        tombstones: keys
            .iter()
            .map(|&n| {
                let entity = &client.entities[&key(n)];
                (entity.handle, entity.tombstone.expect("owed"))
            })
            .collect(),
    }
}

fn left(tick: u32) -> Tombstone {
    Tombstone { region: RegionId(1), tick: Tick(tick), cause: TombstoneCause::Left }
}

fn destroyed(tick: u32) -> Tombstone {
    Tombstone { region: RegionId(1), tick: Tick(tick), cause: TombstoneCause::Destroyed }
}

fn only(sequence: u16) -> AckField {
    AckField { latest: PacketSequence(sequence), bits: 0 }
}

#[test]
fn an_entity_is_sent_whole_until_one_state_is_acknowledged() {
    let mut client = client();
    let handle = client.enter(key(1)).unwrap();
    assert_eq!(client.enter(key(1)).unwrap(), handle, "entering again keeps the state");
    let entity = client.entities[&key(1)];
    assert_eq!((entity.acked, entity.in_flight, entity.tombstone), (None, None, None));
    assert!(entity.sends_whole());

    client.entities.get_mut(&key(1)).unwrap().priority.accumulate(40);
    client.sent(carrying(&client, 10, 5, &[1]));
    let entity = client.entities[&key(1)];
    assert_eq!(entity.in_flight, Some(PacketSequence(10)));
    assert_eq!(entity.priority, Priority::ZERO, "sending resets the priority");
    assert!(entity.sends_whole(), "sent is not acknowledged");

    client.acknowledge(only(10));
    let entity = client.entities[&key(1)];
    assert_eq!((entity.acked, entity.in_flight), (Some(Tick(5)), None));
    assert!(!entity.sends_whole());
    assert!(client.packets.is_empty());
}

#[test]
fn a_claimed_entity_is_whole_in_every_packet() {
    let mut client = client();
    client.enter(key(1)).unwrap();
    client.sent(carrying(&client, 0, 1, &[1]));
    client.acknowledge(only(0));
    assert!(!client.entities[&key(1)].sends_whole());

    client.set_claimed(key(1), true);
    let entity = client.entities[&key(1)];
    assert!(entity.claimed && entity.sends_whole());
    assert_eq!(entity.acked, None, "a change of claim travels as a spawn");
    for sequence in 1..5 {
        client.sent(carrying(&client, sequence, u32::from(sequence) + 1, &[1]));
        client.acknowledge(only(sequence));
        assert!(client.entities[&key(1)].sends_whole(), "claimed, whatever `acked` says");
    }
    assert_eq!(client.entities[&key(1)].acked, Some(Tick(5)));
    client.set_claimed(key(1), true);
    assert_eq!(client.entities[&key(1)].acked, Some(Tick(5)), "no change, nothing voided");
    client.set_claimed(key(1), false);
    assert_eq!(client.entities[&key(1)].acked, None, "released: a spawn again");
    client.set_claimed(key(99), true);
    assert!(!client.entities.contains_key(&key(99)), "not relevant: ignored");
}

#[test]
fn an_acknowledgement_advances_exactly_what_the_packet_carried() {
    let mut client = client();
    for n in 1..=3 {
        client.enter(key(n)).unwrap();
    }
    client.sent(carrying(&client, 20, 7, &[1, 2]));
    client.sent(carrying(&client, 21, 8, &[2, 3]));
    client.acknowledge(only(20));
    let [a, b, c] = [1, 2, 3].map(|n| client.entities[&key(n)]);
    assert_eq!((a.acked, a.in_flight), (Some(Tick(7)), None));
    assert_eq!((b.acked, b.in_flight), (Some(Tick(7)), Some(PacketSequence(21))));
    assert_eq!((c.acked, c.in_flight), (None, Some(PacketSequence(21))));

    // The newer packet acknowledged later; then the older one again changes nothing.
    client.acknowledge(AckField { latest: PacketSequence(21), bits: 1 });
    let [b, c] = [2, 3].map(|n| client.entities[&key(n)]);
    assert_eq!((b.acked, b.in_flight), (Some(Tick(8)), None));
    assert_eq!((c.acked, c.in_flight), (Some(Tick(8)), None));
    assert!(client.packets.is_empty());

    // An acknowledgement of an older state never moves `acked` back.
    client.sent(carrying(&client, 22, 6, &[2]));
    client.acknowledge(only(22));
    assert_eq!(client.entities[&key(2)].acked, Some(Tick(8)));
}

#[test]
fn a_lost_packet_requeues_exactly_its_entities() {
    let mut client = client();
    for n in 1..=2 {
        client.enter(key(n)).unwrap();
    }
    client.sent(carrying(&client, 100, 1, &[1]));
    client.sent(carrying(&client, 101, 2, &[2]));
    // 101 acknowledged; 100 is still in the window, so it is not lost yet.
    client.acknowledge(only(101));
    assert_eq!(client.entities[&key(1)].in_flight, Some(PacketSequence(100)));
    // At 132 the window still reaches 100; at 133 it does not: expired, re-queued.
    client.acknowledge(only(132));
    assert_eq!(client.entities[&key(1)].in_flight, Some(PacketSequence(100)), "still covered");
    client.acknowledge(only(133));
    assert_eq!(client.entities[&key(1)].in_flight, None);
    assert_eq!(client.entities[&key(1)].acked, None);
    assert!(client.packets.is_empty());

    // Eviction from a full ring is a loss too.
    for sequence in 0..PacketRing::DEFAULT_CAPACITY as u16 {
        client.sent(carrying(&client, 200 + sequence, 3, &[2]));
    }
    client.sent(carrying(&client, 300, 3, &[1]));
    client.sent(carrying(&client, 301, 3, &[2]));
    assert_eq!(client.packets.len(), PacketRing::DEFAULT_CAPACITY);
    assert_eq!(client.entities[&key(1)].in_flight, Some(PacketSequence(300)));
    for sequence in 302..302 + PacketRing::DEFAULT_CAPACITY as u16 - 1 {
        client.sent(carrying(&client, sequence, 3, &[2]));
    }
    assert_eq!(client.entities[&key(1)].in_flight, None, "evicted, so lost");
}

#[test]
fn an_acknowledged_tombstone_releases_the_handle() {
    let mut client = client();
    let handle = client.enter(key(1)).unwrap();
    client.enter(key(2)).unwrap();
    client.sent(carrying(&client, 0, 1, &[1]));
    client.leave(key(1), destroyed(4));
    let entity = client.entities[&key(1)];
    assert_eq!((entity.tombstone, entity.in_flight), (Some(destroyed(4)), None));
    client.leave(key(1), destroyed(9));
    assert_eq!(client.entities[&key(1)].tombstone, Some(destroyed(4)), "the first one stays");
    client.leave(key(1), left(9));
    assert_eq!(client.entities[&key(1)].tombstone, Some(destroyed(4)), "the first one stays");
    client.leave(key(77), destroyed(4));
    assert!(!client.entities.contains_key(&key(77)), "not relevant: ignored");

    // A state of the entity acknowledged after it left does not count as the tombstone.
    client.acknowledge(only(0));
    assert!(client.entities.contains_key(&key(1)));
    client.sent(tombstones(&client, 1, 5, &[1]));
    client.acknowledge(only(1));
    assert!(!client.entities.contains_key(&key(1)));
    assert_eq!(client.handles.key_of(handle), None);
    assert_eq!(client.handles.len(), 1);
    // The slot goes to the next entity at the next generation.
    let next = client.enter(key(3)).unwrap();
    assert_eq!(next, EntityHandle { index: handle.index, generation: handle.generation + 1 });
}

#[test]
fn an_entity_relevant_again_reenters_once_its_tombstone_is_acknowledged() {
    let mut client = client();
    let old = client.enter(key(1)).unwrap();
    client.set_claimed(key(1), true);
    client.sent(carrying(&client, 0, 1, &[1]));
    client.acknowledge(only(0));
    client.leave(key(1), left(3));
    assert_eq!(client.enter(key(1)).unwrap(), old, "the old handle until the tombstone");
    assert!(client.entities[&key(1)].reentering);
    client.sent(tombstones(&client, 1, 3, &[1]));
    // Leaving again while the tombstone is owed: forgotten after all.
    client.leave(key(1), left(4));
    assert!(!client.entities[&key(1)].reentering);
    client.enter(key(1)).unwrap();
    client.acknowledge(only(1));
    let entity = client.entities[&key(1)];
    assert_ne!(entity.handle, old);
    assert_eq!(client.handles.key_of(old), None);
    assert_eq!(client.handles.key_of(entity.handle), Some(key(1)));
    assert_eq!((entity.acked, entity.tombstone, entity.reentering), (None, None, false));
    assert!(entity.claimed, "the claim is kept");
}

/// Review r1, S1: a field always acknowledges its `latest`, so a header built before
/// anything was received carries none, and the first packet stays unacknowledged.
#[test]
fn a_header_built_before_anything_was_received_acknowledges_nothing() {
    let header = TickHeader { sequence: PacketSequence(0), tick: Tick(1), ack: None };
    assert_eq!(TickHeader::default().ack, None);
    let mut client = client();
    client.enter(key(1)).unwrap();
    client.sent(carrying(&client, 0, 5, &[1]));
    if let Some(ack) = header.ack {
        client.acknowledge(ack);
    }
    let entity = client.entities[&key(1)];
    assert_eq!((entity.acked, entity.in_flight), (None, Some(PacketSequence(0))));
}

/// Review r1, B2: an entity that left the client's set alive and is destroyed before
/// the Left tombstone is acknowledged owes a Destroyed one instead, since a client whose
/// region was loaded again keeps an entity for a Left tombstone. The acknowledgement of
/// the packet that carried the Left one no longer releases the handle.
#[test]
fn a_destruction_replaces_an_owed_left_tombstone() {
    let mut client = client();
    let handle = client.enter(key(1)).unwrap();
    client.sent(carrying(&client, 0, 1, &[1]));
    client.acknowledge(only(0));
    client.leave(key(1), left(2));
    client.sent(tombstones(&client, 1, 2, &[1]));
    assert!(client.entities[&key(1)].in_flight.is_some());
    client.leave(key(1), destroyed(3));
    let entity = client.entities[&key(1)];
    assert_eq!((entity.tombstone, entity.in_flight), (Some(destroyed(3)), None));
    client.acknowledge(only(1));
    assert!(client.entities.contains_key(&key(1)), "the Left tombstone is no longer owed");
    client.sent(tombstones(&client, 2, 3, &[1]));
    client.acknowledge(only(2));
    assert!(!client.entities.contains_key(&key(1)));
    assert_eq!(client.handles.key_of(handle), None);
}

#[test]
fn a_whole_resend_is_not_undone_by_an_older_acknowledgement() {
    let mut client = client();
    client.enter(key(1)).unwrap();
    client.enter(key(2)).unwrap();
    client.sent(carrying(&client, 0, 1, &[1, 2]));
    client.acknowledge(only(0));
    client.sent(carrying(&client, 1, 2, &[1, 2]));
    // The entity moves between two regions the client has: owed whole again.
    client.resend_whole(key(1));
    assert_eq!(client.entities[&key(1)].acked, None);
    client.acknowledge(only(1));
    assert_eq!(client.entities[&key(1)].acked, None, "packet 1 was sent before the resend");
    assert_eq!(client.entities[&key(2)].acked, Some(Tick(2)), "the other entity advances");
    client.sent(carrying(&client, 2, 3, &[1]));
    client.acknowledge(only(2));
    assert_eq!(client.entities[&key(1)].acked, Some(Tick(3)));
}

#[test]
fn a_resync_clears_and_refills_the_handles() {
    let mut client = client();
    for n in 1..=5 {
        client.enter(key(n)).unwrap();
    }
    client.set_claimed(key(2), true);
    client.sent(carrying(&client, 0, 1, &[1, 2, 3, 4, 5]));
    client.acknowledge(only(0));
    client.leave(key(3), destroyed(2));
    client.leave(key(4), left(2));
    client.enter(key(4)).unwrap();
    client.handles.release(client.entities[&key(5)].handle);
    client.handles.assign(key(5)).unwrap();
    client.sent(carrying(&client, 1, 3, &[1]));

    client.resync(Tick(10));
    assert!(client.packets.is_empty());
    let kept: Vec<StableKey> = client.entities.keys().copied().collect();
    assert_eq!(kept, [key(1), key(2), key(4), key(5)], "the tombstoned entity is gone");
    let handles: Vec<(StableKey, EntityHandle)> = client.handles.iter().collect();
    let fresh: Vec<(StableKey, EntityHandle)> = kept
        .iter()
        .zip(0..)
        .map(|(&key, index)| (key, EntityHandle { index, generation: 1 }))
        .collect();
    assert_eq!(handles, fresh, "fresh handles in ascending key order");
    for (key, entity) in &client.entities {
        assert_eq!(entity.handle, client.handles.handle_of(*key).unwrap());
        assert_eq!(
            (entity.acked, entity.in_flight, entity.tombstone),
            (Some(Tick(10)), None, None)
        );
        assert!(!entity.reentering);
    }
    assert!(client.entities[&key(2)].claimed && client.entities[&key(2)].sends_whole());
    // An acknowledgement of a packet from before the resync finds nothing.
    client.acknowledge(only(1));
    assert_eq!(client.entities[&key(1)].acked, Some(Tick(10)));
}
