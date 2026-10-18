//! The packer's and the receiver's rules (README, "Interest and the packer" and "Handles
//! and packets") run end to end: a real server whose gathered frames feed a model of
//! `lactose-net`'s packer built on `ClientInterest`, a link that loses, delays, reorders and
//! duplicates datagrams, a reliable ordered control channel, a model receiver that stages
//! each packet whole or drops it whole, and a real client world stepped through
//! `lactose-net-api`'s fold and hook. Proved here: the mirror equals the server's
//! replicated rows and a full gather replaces it; every frame holding an exact row of an
//! entity holds every replicated component the server's entity has at the frame's tick;
//! a malformed packet changes nothing and is not acknowledged; the client converges to the
//! server under churn, loss, resyncs and claims; the cases the region rules exist for, one
//! step at a time; and the schedules review r1 found (a removal under a whole send, a
//! region flap over a cooked entity, the acknowledgement field after a lap).

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{AuthoredGuid, Baseline, BaselineRow, cooked_net_id};
use lactose_core::{ComponentKey, ErrorKind, StableKey, Tick};
use lactose_ecs::{Executor, LayoutConfig, RegionId};
use lactose_net_api::{GatherState, RegionChange, StagedFrame, Tombstone, TombstoneCause};
use lactose_net_host::packet::{EntityHandle, PacketSequence};
use lactose_reflect::Reflect;
use lactose_sim::TickInput;

const R1: RegionId = REGIONS[0];
const R2: RegionId = REGIONS[1];
const R3: RegionId = REGIONS[2];

/// The mirror as rows: (key, component) to (region, bytes).
fn mirror_rows(model: &ServerModel) -> BTreeMap<(StableKey, ComponentKey), (RegionId, Vec<u8>)> {
    let mut rows = BTreeMap::new();
    for (&key, entity) in &model.mirror {
        for (&component, (_, bytes)) in &entity.rows {
            rows.insert((key, component), (entity.region, bytes.clone()));
        }
    }
    rows
}

#[test]
fn the_mirror_equals_the_server_rows_and_a_full_gather_replaces_it() {
    let executor = executor(2);
    let mut simulation = server(LayoutConfig::default(), 40);
    let mut model = ServerModel::new(&REGIONS, Baselines::default());
    let mut rng = Rng::new(3);
    let mut next = 41;
    let (mut removals, mut fulls) = (0, 0);
    for index in 0..80 {
        churn(simulation.world_mut(), &mut rng, &mut next);
        let gone = (1..next).map(key).find(|&key| simulation.world().contains(key));
        let gone = gone.expect("a live entity");
        if index == 50 {
            // A gap: a despawn and a removal the gather never reads, the structural log
            // trimmed past them, so the next gather is a full pass.
            let world = simulation.world_mut();
            world.despawn(gone).unwrap();
            let survivor = (1..next).map(key).find(|&key| world.contains(key)).unwrap();
            world.insert(survivor, Status(1)).unwrap();
            let cursor = world.structural_log().cursor();
            world.trim_structural_log(cursor);
            assert!(mirror_rows(&model).keys().any(|&(key, _)| key == gone));
        }
        simulation = executor.block_on(simulation.step(&executor, TickInput::default())).unwrap();
        simulation = executor.block_on(simulation.run_passes(&executor)).unwrap();
        let gathered = simulation.world_mut().service_mut::<GatherState>().unwrap().take();
        fulls += usize::from(gathered.full);
        removals += gathered
            .structural()
            .iter()
            .filter(|entry| matches!(entry.change, lactose_ecs::StructuralChange::Removed { .. }))
            .count();
        model.absorb(&gathered);
        assert_eq!(mirror_rows(&model), replicated_rows(simulation.world()), "frame {index}");
        if index == 50 {
            assert!(gathered.full);
            assert!(!model.mirror.contains_key(&gone), "despawned in the gap");
        }
    }
    assert_eq!(fulls, 2, "the first gather and the one after the gap");
    assert!(removals > 10, "{removals} removals");
}

#[test]
fn a_malformed_packet_changes_nothing_and_is_not_acknowledged() {
    let mut receiver = ReceiverModel::new(sizes(types().registry()));
    receiver.control(Control::Full { tick: Tick(10), regions: Vec::new(), spawns: Vec::new() });
    let staged = receiver.staged.len();
    let handle = EntityHandle { index: 0, generation: 1 };
    let spawn = |claimed: bool| WireSpawn {
        handle,
        key: key(1),
        region: R1,
        claimed,
        components: vec![data(Position { x: 1, y: 2 }), data(Health(3))],
    };
    let row = |handle: EntityHandle, component: ComponentKey, bytes: Vec<u8>| WireRow {
        handle,
        component,
        tick: Tick(11),
        exact: true,
        bytes,
    };
    let health = data(Health(4)).bytes;

    let mut packet = Packet::new(PacketSequence(1), Tick(11));
    packet.spawns.push(spawn(false));
    // A row one byte short, of the entity the same packet spawns.
    packet.rows.push(row(handle, Health::KEY, health[..3].to_vec()));
    let malformed = [
        packet.clone(),
        // A row of a handle the receiver does not hold.
        {
            let mut packet = Packet::new(PacketSequence(2), Tick(12));
            packet.spawns.push(spawn(false));
            packet.rows.push(row(
                EntityHandle { index: 5, generation: 1 },
                Health::KEY,
                health.clone(),
            ));
            packet
        },
        // A spawn whose component has another length than the registry's.
        {
            let mut packet = Packet::new(PacketSequence(3), Tick(13));
            let mut whole = spawn(false);
            whole.components[1].bytes.push(0);
            packet.spawns.push(whole);
            packet
        },
    ];
    for (n, packet) in malformed.iter().enumerate() {
        receiver.packet(packet);
        assert_eq!(receiver.stats.malformed, u32::try_from(n + 1).unwrap());
        assert!(receiver.handles.is_empty(), "no handle recorded");
        assert!(receiver.slots.is_empty(), "no slot allocated");
        assert_eq!(receiver.ack(), None, "nothing acknowledged");
        assert_eq!(receiver.staged.len(), staged, "nothing staged");
    }

    // The same packet whole: staged, the handle recorded, the slot allocated, acknowledged.
    let mut packet = Packet::new(PacketSequence(4), Tick(14));
    packet.spawns.push(spawn(false));
    receiver.packet(&packet);
    assert_eq!(receiver.staged.len(), staged + 1);
    assert_eq!(receiver.handles.key_of(handle), Some(key(1)));
    assert_eq!(receiver.slots.len(), 1);
    assert_eq!(receiver.ack().map(|ack| ack.latest), Some(PacketSequence(4)));

    // Claimed: the spawn releases the slot; then exact rows of every component, whole, or
    // with one malformed row nothing of them.
    let mut packet = Packet::new(PacketSequence(5), Tick(15));
    packet.spawns.push(spawn(true));
    receiver.packet(&packet);
    assert!(receiver.slots.is_empty());
    let whole = |sequence: u16, tick: u32, health: Vec<u8>| {
        let mut packet = Packet::new(PacketSequence(sequence), Tick(tick));
        let position = data(Position { x: 9, y: 9 }).bytes;
        packet.rows.push(WireRow { tick: Tick(tick), ..row(handle, Position::KEY, position) });
        packet.rows.push(WireRow { tick: Tick(tick), ..row(handle, Health::KEY, health) });
        packet
    };
    receiver.packet(&whole(6, 16, vec![1, 2]));
    assert_eq!(receiver.stats.malformed, 4);
    assert_eq!(receiver.staged.len(), staged + 2, "the claimed entity's rows: none staged");
    assert_eq!(receiver.ack().map(|ack| ack.latest), Some(PacketSequence(5)));
    receiver.packet(&whole(7, 17, health));
    let frame: &StagedFrame = receiver.staged.last().unwrap();
    assert_eq!(frame.tick, Tick(17));
    let rows: Vec<(ComponentKey, bool)> =
        frame.rows().map(|row| (row.component, row.exact)).collect();
    let mut expected = vec![(Position::KEY, true), (Health::KEY, true)];
    expected.sort();
    assert_eq!(rows, expected);
}

#[test]
fn the_client_converges_under_churn_loss_reorder_resyncs_and_claims() {
    let executor = executor(2);
    for seed in [1_u64, 2, 3] {
        let mut replication = Replication::new(LayoutConfig::default(), 40, &[R1, R2], seed);
        replication.link = LinkConfig::POOR;
        replication.lag = 6;
        let mut script = Rng::new(seed * 31);
        let mut next = 41;
        for n in 0..700 {
            churn(replication.world_mut(), &mut script, &mut next);
            change_interest(&mut replication, &mut script, next);
            if n == 300 || n == 550 {
                replication.change(Change::Resync);
            }
            replication.tick(&executor);
        }
        calm(&mut replication, &executor, next);
        replication.assert_converged(&format!("seed {seed}"));
        let stats = replication.receiver.stats;
        println!("seed {seed}: {stats:?}, {} whole checks", replication.whole_checks);
        assert_eq!(stats.malformed, 0, "seed {seed}");
        assert!(stats.late > 50, "seed {seed}: {stats:?}");
        assert!(stats.unknown_tombstones > 0, "seed {seed}: {stats:?}");
        assert!(replication.whole_checks > 200, "seed {seed}: {}", replication.whole_checks);
        let fulls = replication.frames.iter().filter(|frame| frame.full).count();
        assert_eq!(fulls, 3, "the join and two resyncs");
        // Under region toggling the hook may skip a spawn or a move into a region a step
        // folded into no load (the cases below); nothing else.
        let skipped = replication.skipped.lock().unwrap().clone();
        let region_not_loaded = |&(what, _, kind): &(&str, StableKey, ErrorKind)| {
            matches!(what, "spawn" | "move") && kind == ErrorKind::NotFound
        };
        assert!(skipped.iter().all(region_not_loaded), "seed {seed}: {skipped:?}");
    }
}

/// A replication of `entities`, keyed `1..`, all in `R1`, joined and settled with
/// `relevant` relevant over a perfect link.
fn settled(entities: &[(u64, RegionId)], relevant: &[RegionId]) -> (Replication, Executor) {
    let executor = executor(1);
    let mut replication = Replication::new(LayoutConfig::default(), 0, relevant, 9);
    replication.lag = 1;
    for &(n, region) in entities {
        let at = i32::try_from(n).unwrap();
        replication
            .world_mut()
            .spawn(key(n), region, (Position { x: at, y: 0 }, Health(at)))
            .unwrap();
    }
    for _ in 0..12 {
        replication.tick(&executor);
    }
    replication.assert_converged("settled");
    (replication, executor)
}

/// The frames handed to `app` from `from` on, which one client step folds and pops.
fn frames_since(replication: &Replication, from: usize) -> &[std::sync::Arc<StagedFrame>] {
    &replication.frames[from..]
}

#[test]
fn an_entity_moved_into_a_region_relevant_and_irrelevant_again_in_one_step_leaves_with_it() {
    let (mut replication, executor) = settled(&[(1, R1), (2, R2)], &[R1]);
    replication.hold = true;
    replication.change(Change::Relevant(R2, true));
    while !replication.model.named.contains(&R2) {
        replication.tick(&executor);
    }
    replication.world_mut().move_to_region(key(1), R2).unwrap();
    replication.tick(&executor);
    replication.change(Change::Relevant(R2, false));
    for _ in 0..3 {
        replication.tick(&executor);
    }
    let from = replication.frames.len();
    let before = replication.client().tick();
    replication.hold = false;
    replication.tick(&executor);
    assert_eq!(replication.client().tick().0, before.0 + 1, "one step pops them all");

    // The step folded the region's load and unload into nothing, and the frames moved the
    // entity into it and tombstoned it there.
    let frames = frames_since(&replication, from);
    let changes: Vec<bool> = frames
        .iter()
        .filter_map(|frame| frame.regions.get(&R2))
        .map(|change| matches!(change, RegionChange::Relevant(_)))
        .collect();
    assert_eq!(changes, [true, false]);
    assert!(frames.iter().any(|frame| frame.moves.get(&key(1)) == Some(&R2)));
    // It left alive, named by the region it resides in on the server, which the client
    // never loaded: the hook does not skip it.
    let tombstone = frames.iter().find_map(|frame| frame.despawns.get(&key(1)));
    let left =
        Some(&Tombstone { region: R2, tick: tombstone.unwrap().tick, cause: TombstoneCause::Left });
    assert_eq!(tombstone, left);
    let world = replication.client().world();
    assert!(world.region(R2).is_none());
    assert!(!world.contains(key(1)), "no ghost left in the region it came from");
    assert!(!world.contains(key(2)));
    // What the hook skipped is what named the region the step never loaded: the moved
    // entity's move, and the spawns of the entity residing there.
    let skipped = replication.skipped.lock().unwrap().clone();
    assert!(skipped.contains(&("move", key(1), ErrorKind::NotFound)), "{skipped:?}");
    let named_the_region = |&(what, key, kind): &(&str, StableKey, ErrorKind)| {
        kind == ErrorKind::NotFound && (what, key) == ("spawn", common::key(2))
            || (what, key) == ("move", common::key(1))
    };
    assert!(skipped.iter().all(named_the_region), "{skipped:?}");
    for _ in 0..10 {
        replication.tick(&executor);
    }
    replication.assert_converged("after");
    assert_eq!(replication.server().world().region_of(key(1)), Some(R2));
}

#[test]
fn an_entity_moved_out_of_a_region_unloading_in_the_same_step_arrives_whole() {
    let (mut replication, executor) = settled(&[(1, R1), (2, R1), (3, R2)], &[R1, R2]);
    replication.hold = true;
    replication.world_mut().move_to_region(key(1), R2).unwrap();
    replication.tick(&executor);
    replication.change(Change::Relevant(R1, false));
    for _ in 0..3 {
        replication.tick(&executor);
    }
    let from = replication.frames.len();
    replication.hold = false;
    replication.tick(&executor);

    // The unload took the entity with the region it left, before the hook; the spawn
    // beside its move brought it back in the region it moved to.
    let frames = frames_since(&replication, from);
    assert!(frames.iter().any(|frame| frame.spawns.contains_key(&net(1))));
    assert!(frames.iter().any(|frame| frame.regions.get(&R1) == Some(&RegionChange::Irrelevant)));
    let world = replication.client().world();
    assert!(world.region(R1).is_none());
    assert_eq!(world.region_of(key(1)), Some(R2), "the moved entity is not lost");
    assert!(!world.contains(key(2)));
    assert!(world.contains(key(3)));
    for _ in 0..10 {
        replication.tick(&executor);
    }
    replication.assert_converged("after");
}

#[test]
fn two_changes_of_one_region_due_for_one_frame_are_staged_in_two() {
    let (mut replication, executor) = settled(&[(1, R1), (2, R2), (3, R3)], &[R1, R2]);
    // No datagram arrives while the region leaves and returns, so both changes are
    // pending when the next packet is staged.
    replication.link.loss = 100;
    replication.change(Change::Relevant(R2, false));
    replication.tick(&executor);
    replication.change(Change::Relevant(R2, true));
    for _ in 0..4 {
        replication.tick(&executor);
    }
    let from = replication.frames.len();
    replication.link.loss = 0;
    for _ in 0..15 {
        replication.tick(&executor);
    }
    // The region unloaded and loaded again: no load refused for a region already loaded,
    // its entity back from its re-entry spawn.
    replication.assert_converged("after");
    assert!(replication.client().world().contains(key(2)));
    assert_eq!(replication.receiver.stats.split, 1);
    let frames = frames_since(&replication, from);
    let split = &frames[0];
    assert_eq!(split.regions.get(&R2), Some(&RegionChange::Irrelevant));
    assert!(split.spawns.is_empty() && split.despawns.is_empty() && split.row_count() == 0);
    assert!(matches!(frames[1].regions.get(&R2), Some(RegionChange::Relevant(_))));
}

// ---- Review r1 ----

fn status_on_client(replication: &Replication, n: u64) -> Option<u32> {
    replication.client().world().get::<Status>(key(n)).map(|status| status.0)
}

/// `settled` with entity 1 holding `Status` on both ends.
fn with_status() -> (Replication, Executor) {
    let (mut replication, executor) = settled(&[(1, R1)], &[R1, R2]);
    replication.world_mut().insert(key(1), Status(7)).unwrap();
    for _ in 0..10 {
        replication.tick(&executor);
    }
    assert_eq!(status_on_client(&replication, 1), Some(7));
    (replication, executor)
}

/// Review r1, B1: a component removed in the tick the entity moves between two named
/// regions. The move makes the entity owed whole, and a whole send without its removals
/// leaves the client's component in place under the spawn's upsert for good.
#[test]
fn a_removal_beside_a_move_between_named_regions_reaches_the_client() {
    let (mut replication, executor) = with_status();
    let world = replication.world_mut();
    let _ = world.remove::<Status>(key(1)).unwrap();
    world.move_to_region(key(1), R2).unwrap();
    for _ in 0..30 {
        replication.tick(&executor);
    }
    assert_eq!(status_on_client(&replication, 1), None);
    replication.assert_converged("removed and moved");
}

/// Review r1, B1: the same with a claim, which also makes the entity owed whole.
#[test]
fn a_removal_beside_a_claim_reaches_the_client() {
    let (mut replication, executor) = with_status();
    let _ = replication.world_mut().remove::<Status>(key(1)).unwrap();
    replication.change(Change::Claimed(key(1), true));
    for _ in 0..30 {
        replication.tick(&executor);
    }
    assert_eq!(status_on_client(&replication, 1), None);
    replication.assert_converged("removed and claimed");
}

/// Review r1, B1: not a same-tick artefact. The removal's packets are lost, the entity
/// moves two ticks later, and the link recovers.
#[test]
fn a_removal_lost_before_a_move_reaches_the_client() {
    let (mut replication, executor) = with_status();
    let _ = replication.world_mut().remove::<Status>(key(1)).unwrap();
    replication.link.loss = 100;
    replication.tick(&executor);
    replication.tick(&executor);
    replication.world_mut().move_to_region(key(1), R2).unwrap();
    replication.tick(&executor);
    replication.link.loss = 0;
    for _ in 0..30 {
        replication.tick(&executor);
    }
    assert_eq!(status_on_client(&replication, 1), None);
    replication.assert_converged("removed, lost, moved");
}

fn cooked_key() -> StableKey {
    StableKey::Net(cooked_net_id(AuthoredGuid([7; 16]), AuthoredGuid([9; 16]), &[]))
}

/// A replication whose region `R2` is cooked with one entity carrying the unreplicated
/// `Mover` beside its replicated components, which the server holds alike, joined and
/// settled with `R1` and `R2` relevant. The client has `Mover` from the bundle alone.
fn cooked() -> (Replication, Executor) {
    let StableKey::Net(net) = cooked_key() else { unreachable!() };
    let components = vec![data(Position { x: 5, y: 5 }), data(Health(5)), data(Mover(0))];
    let rows = vec![BaselineRow { id: net, components }];
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(types().registry()), rows);
    let executor = executor(1);
    let mut replication = Replication::new(LayoutConfig::default(), 0, &[R1, R2], 9);
    replication.cook(R2, baseline.expect("a cooked baseline"));
    replication.lag = 1;
    let world = replication.world_mut();
    world.spawn(cooked_key(), R2, (Position { x: 5, y: 5 }, Health(5), Mover(0))).unwrap();
    world.spawn(key(1), R1, (Position { x: 1, y: 0 }, Health(1))).unwrap();
    for _ in 0..12 {
        replication.tick(&executor);
    }
    replication.assert_converged("settled");
    assert!(cooked_mover(&replication), "loaded from the bundle");
    (replication, executor)
}

fn cooked_mover(replication: &Replication) -> bool {
    replication.client().world().get::<Mover>(cooked_key()).is_some()
}

/// Review r1, B2: the region flaps while no datagram arrives, so its two changes are
/// staged in two frames and the Left tombstone comes after the reload. A despawn there
/// followed by the re-entry spawn would lose the cooked `Mover`.
#[test]
fn a_region_flap_staged_in_two_frames_keeps_the_cooked_rows() {
    let (mut replication, executor) = cooked();
    replication.link.loss = 100;
    replication.change(Change::Relevant(R2, false));
    replication.tick(&executor);
    replication.change(Change::Relevant(R2, true));
    for _ in 0..4 {
        replication.tick(&executor);
    }
    replication.link.loss = 0;
    for _ in 0..30 {
        replication.tick(&executor);
    }
    assert_eq!(replication.receiver.stats.split, 1);
    replication.assert_converged("after the flap");
    assert!(cooked_mover(&replication));
}

/// Review r1, B2: no loss; the unload and the reload reach `app` in frames one client
/// step pops, as with a held prediction world or a slow frame.
#[test]
fn a_region_flap_popped_in_one_step_keeps_the_cooked_rows() {
    let (mut replication, executor) = cooked();
    replication.hold = true;
    replication.change(Change::Relevant(R2, false));
    replication.tick(&executor);
    replication.tick(&executor);
    replication.change(Change::Relevant(R2, true));
    for _ in 0..3 {
        replication.tick(&executor);
    }
    replication.hold = false;
    for _ in 0..30 {
        replication.tick(&executor);
    }
    replication.assert_converged("after the flap");
    assert!(cooked_mover(&replication));
}

/// Review r1, B2, the control: the same flap over separate steps.
#[test]
fn a_region_flap_over_separate_steps_keeps_the_cooked_rows() {
    let (mut replication, executor) = cooked();
    replication.change(Change::Relevant(R2, false));
    for _ in 0..6 {
        replication.tick(&executor);
    }
    assert!(!replication.client().world().contains(cooked_key()));
    replication.change(Change::Relevant(R2, true));
    for _ in 0..30 {
        replication.tick(&executor);
    }
    replication.assert_converged("after the flap");
    assert!(cooked_mover(&replication));
}

/// The flap of the first case with the entity destroyed while its Left tombstone is still
/// owed: the Destroyed tombstone replaces it, and the client, which loaded the region
/// again with the entity in it, despawns the entity instead of skipping the tombstone.
#[test]
fn an_entity_destroyed_while_its_left_tombstone_is_owed_leaves_no_ghost() {
    let (mut replication, executor) = cooked();
    replication.link.loss = 100;
    replication.change(Change::Relevant(R2, false));
    replication.tick(&executor);
    replication.change(Change::Relevant(R2, true));
    while !replication.model.named.contains(&R2) {
        replication.tick(&executor);
    }
    let owed = replication.model.interest.entities[&cooked_key()];
    assert_eq!(owed.tombstone.map(|tombstone| tombstone.cause), Some(TombstoneCause::Left));
    replication.world_mut().despawn(cooked_key()).unwrap();
    replication.tick(&executor);
    let owed = replication.model.interest.entities[&cooked_key()];
    assert_eq!(owed.tombstone.map(|tombstone| tombstone.cause), Some(TombstoneCause::Destroyed));
    replication.link.loss = 0;
    for _ in 0..30 {
        replication.tick(&executor);
    }
    replication.assert_converged("after the flap and the destruction");
    assert!(!replication.client().world().contains(cooked_key()));
}

/// Review r1, M8: a receiver that remembers every sequence it staged acknowledges, after a
/// lap, a packet it never received.
#[test]
fn the_receiver_acknowledges_no_lost_packet_after_the_sequence_wraps() {
    let mut receiver = ReceiverModel::new(sizes(types().registry()));
    receiver.control(Control::Full { tick: Tick(1), regions: Vec::new(), spawns: Vec::new() });
    let lost = PacketSequence(100);
    let mut tick = 2_u32;
    for lap in 0..2 {
        for sequence in (0..=u16::MAX).map(PacketSequence) {
            tick += 1;
            if lap == 1 && sequence == lost {
                continue;
            }
            receiver.packet(&Packet::new(sequence, Tick(tick)));
            receiver.staged.clear();
            if lap == 1 && sequence == lost.next() {
                let ack = receiver.ack().unwrap();
                assert_eq!(ack.latest, sequence);
                assert!(!ack.covers(lost), "{ack:?}");
                return;
            }
        }
    }
    unreachable!("the second lap reaches the lost packet");
}
