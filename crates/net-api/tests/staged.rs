//! The staged frame, the inbox and the client hook (README, "The staged frame"; proof row
//! "The staged frame"): `push_row` keeps the newest row, the inbox accepts ascending ticks
//! only and hands the hook exactly what `due` showed `app`, and the README's hook, written
//! through the erased writes, leaves a client world as the frame says.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use lactose_bundle::BaselineRow;
use lactose_core::{EntityRef, ErrorKind, NetId, PlayerId, PredictionKey, StableKey, Tick};
use lactose_ecs::{OwnedBy, RegionConfig, RegionMode, StructuralChange};
use lactose_net_api::{
    RegionChange, RemoteSlot, StagedFrame, StagedInbox, StagedSpawn, Tombstone, TombstoneCause,
};
use lactose_reflect::Reflect;
use lactose_sim::{Simulation, TickInput, TimeScale};

// ---- The frame ----

#[test]
fn push_row_keeps_the_newest_row_and_refuses_an_older_or_equal_one() {
    let mut frame = StagedFrame::new(Tick(20));
    assert!(frame.push_row(key(1), Health::KEY, Tick(15), false, bytes(&Health(1))));
    assert!(!frame.push_row(key(1), Health::KEY, Tick(15), true, bytes(&Health(2))));
    assert!(!frame.push_row(key(1), Health::KEY, Tick(14), true, bytes(&Health(3))));
    let row = frame.row(key(1), Health::KEY).expect("staged");
    assert_eq!((row.tick, row.exact, row.bytes), (Tick(15), false, bytes(&Health(1))));
    assert!(frame.push_row(key(1), Health::KEY, Tick(17), true, bytes(&Health(4))));
    let row = frame.row(key(1), Health::KEY).expect("staged");
    assert_eq!((row.tick, row.exact, row.bytes), (Tick(17), true, bytes(&Health(4))));
    assert_eq!(frame.row_count(), 1);
    // Across the wrap, a tick after zero is newer than one before it.
    let mut wrapped = StagedFrame::new(Tick(2));
    assert!(wrapped.push_row(key(1), Health::KEY, Tick(u32::MAX), false, bytes(&Health(1))));
    assert!(wrapped.push_row(key(1), Health::KEY, Tick(1), false, bytes(&Health(2))));
    assert!(!wrapped.push_row(key(1), Health::KEY, Tick(u32::MAX - 1), false, bytes(&Health(3))));
    assert_eq!(wrapped.row(key(1), Health::KEY).map(|row| row.tick), Some(Tick(1)));
}

#[test]
fn push_row_refuses_a_row_after_the_frame_or_of_another_length_and_the_arena_never_grows() {
    let mut frame = StagedFrame::new(Tick(20));
    // A row from a tick after the frame's, held or not.
    assert!(!frame.push_row(key(1), Health::KEY, Tick(21), false, bytes(&Health(1))));
    assert_eq!(frame.row_count(), 0);
    assert!(frame.push_row(key(1), Health::KEY, Tick(20), false, bytes(&Health(1))));
    let mut frame = StagedFrame::new(Tick(1_000_000));
    // One (key, component), newer ticks each time, two alternating lengths (review r1,
    // M2): only the pushes of the held length are staged, in place. The arena is seen
    // through the derived `Debug`, which prints it.
    let small = [1u8; 1024];
    let large = [2u8; 1025];
    assert!(frame.push_row(key(1), Health::KEY, Tick(1), false, &small));
    let held = format!("{frame:?}").len();
    let mut staged = 0;
    for tick in 2..2002u32 {
        let pushed = if tick % 2 == 0 { &large[..] } else { &small[..] };
        let accepted = frame.push_row(key(1), Health::KEY, Tick(tick), tick % 3 == 0, pushed);
        assert_eq!(accepted, pushed.len() == small.len(), "tick {tick}");
        staged += usize::from(accepted);
    }
    assert_eq!(staged, 1000);
    assert_eq!(frame.row_count(), 1);
    let row = frame.row(key(1), Health::KEY).expect("held");
    assert_eq!((row.tick, row.bytes.len()), (Tick(2001), small.len()));
    // The tick and precision fields printed may differ in width, never by a kilobyte.
    assert!(format!("{frame:?}").len() < held + 32, "the arena grew");
}

#[test]
fn rows_ascend_by_key_then_component_whatever_the_push_order() {
    let pushes = [
        (key(3), Position::KEY, bytes(&Position { x: 3, y: 0 }).to_vec()),
        (key(1), Status::KEY, bytes(&Status(1)).to_vec()),
        (key(1), Position::KEY, bytes(&Position { x: 1, y: 0 }).to_vec()),
        (key(2), Health::KEY, bytes(&Health(2)).to_vec()),
        (key(1), Health::KEY, bytes(&Health(1)).to_vec()),
    ];
    let mut forward = StagedFrame::new(Tick(5));
    for (key, component, bytes) in &pushes {
        assert!(forward.push_row(*key, *component, Tick(4), false, bytes));
    }
    let mut backward = StagedFrame::new(Tick(5));
    for (key, component, bytes) in pushes.iter().rev() {
        assert!(backward.push_row(*key, *component, Tick(4), false, bytes));
    }
    let order: Vec<_> = forward.rows().map(|row| (row.key, row.component)).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
    assert_eq!(forward.rows().collect::<Vec<_>>(), backward.rows().collect::<Vec<_>>());
    // Equal frames compare equal however their arenas were filled.
    assert_eq!(forward, backward);
    let mut replaced = backward.clone();
    assert!(replaced.push_row(key(2), Health::KEY, Tick(5), false, bytes(&Health(9))));
    assert_ne!(replaced, forward);
}

#[test]
fn is_empty_is_exact() {
    let empty = StagedFrame::new(Tick(1));
    assert!(empty.is_empty());
    let mut each: Vec<StagedFrame> = Vec::new();
    each.push(StagedFrame::resync(Tick(1)));
    let mut frame = StagedFrame::new(Tick(1));
    frame.time_scale = Some(TimeScale::PAUSED);
    each.push(frame);
    let mut frame = StagedFrame::new(Tick(1));
    frame.acked_input = Some(Tick(0));
    each.push(frame);
    let mut frame = StagedFrame::new(Tick(1));
    frame.substituted.insert(Tick(0));
    each.push(frame);
    let mut frame = StagedFrame::new(Tick(1));
    frame.regions.insert(REGION, RegionChange::Irrelevant);
    each.push(frame);
    let mut frame = StagedFrame::new(Tick(1));
    frame.spawns.insert(net(1), spawn(REGION, Some(1), vec![]));
    each.push(frame);
    let mut frame = StagedFrame::new(Tick(1));
    frame.moves.insert(key(1), REGION);
    each.push(frame);
    let mut frame = StagedFrame::new(Tick(1));
    frame.removed.insert((key(1), Health::KEY));
    each.push(frame);
    let mut frame = StagedFrame::new(Tick(1));
    frame.despawns.insert(
        key(1),
        Tombstone { region: REGION, tick: Tick(1), cause: TombstoneCause::Destroyed },
    );
    each.push(frame);
    let mut frame = StagedFrame::new(Tick(1));
    assert!(frame.push_row(key(1), Health::KEY, Tick(1), false, bytes(&Health(1))));
    each.push(frame);
    assert_eq!(each.len(), 10);
    for frame in &each {
        assert!(!frame.is_empty(), "{frame:?}");
    }
}

// ---- The inbox ----

fn frame(tick: u32) -> Arc<StagedFrame> {
    Arc::new(StagedFrame::new(Tick(tick)))
}

fn ticks<'a>(frames: impl Iterator<Item = &'a Arc<StagedFrame>>) -> Vec<u32> {
    frames.map(|frame| frame.tick.0).collect()
}

#[test]
fn the_inbox_accepts_only_a_frame_after_the_newest_it_ever_accepted() {
    let mut inbox = StagedInbox::new();
    assert_eq!(inbox.accepted_tick(), None);
    assert!(inbox.push(frame(10)));
    assert!(!inbox.push(frame(10)));
    assert!(!inbox.push(frame(9)));
    assert!(inbox.push(frame(12)));
    assert!(!inbox.push(frame(11)));
    assert_eq!(inbox.accepted_tick(), Some(Tick(12)));
    assert_eq!(inbox.len(), 2);
    // Also after a pop: a frame older than one already applied is refused (review probe s4).
    assert_eq!(inbox.pop_through(Tick(12)).map(|frame| frame.tick), Some(Tick(10)));
    assert_eq!(inbox.pop_through(Tick(12)).map(|frame| frame.tick), Some(Tick(12)));
    assert!(inbox.is_empty());
    assert!(!inbox.push(frame(9)));
    assert!(!inbox.push(frame(12)));
    assert_eq!(inbox.pop_through(Tick(13)), None);
    assert_eq!(inbox.accepted_tick(), Some(Tick(12)));
    // A full frame older than accepted deltas is refused (review probe d2), as any frame.
    let mut inbox = StagedInbox::new();
    assert!(inbox.push(frame(51)));
    assert!(inbox.push(frame(52)));
    assert!(!inbox.push(Arc::new(StagedFrame::resync(Tick(50)))));
    assert!(inbox.push(Arc::new(StagedFrame::resync(Tick(53)))));
    // Across the wrap.
    let mut inbox = StagedInbox::new();
    assert!(inbox.push(frame(u32::MAX)));
    assert!(inbox.push(frame(0)));
    assert!(!inbox.push(frame(u32::MAX - 1)));
    assert_eq!(ticks(inbox.due(Tick(u32::MAX))), vec![u32::MAX]);
    assert_eq!(ticks(inbox.due(Tick(1))), vec![u32::MAX, 0]);
    assert_eq!(inbox.pop_through(Tick(0)).map(|frame| frame.tick), Some(Tick(u32::MAX)));
    assert_eq!(inbox.pop_through(Tick(0)).map(|frame| frame.tick), Some(Tick(0)));
}

#[test]
fn due_and_pop_through_agree_and_hand_frames_up_to_a_tick_oldest_first() {
    let mut inbox = StagedInbox::new();
    for tick in [3, 4, 7, 8, 12] {
        assert!(inbox.push(frame(tick)));
    }
    assert_eq!(ticks(inbox.due(Tick(2))), Vec::<u32>::new());
    assert_eq!(ticks(inbox.due(Tick(3))), vec![3]);
    assert_eq!(ticks(inbox.due(Tick(7))), vec![3, 4, 7]);
    assert_eq!(ticks(inbox.due(Tick(11))), vec![3, 4, 7, 8]);
    // `due` takes nothing.
    assert_eq!(inbox.len(), 5);
    for through in [2, 7, 7, 11, 20] {
        let shown = ticks(inbox.due(Tick(through)));
        let mut popped = Vec::new();
        while let Some(frame) = inbox.pop_through(Tick(through)) {
            popped.push(frame.tick.0);
        }
        assert_eq!(popped, shown, "through {through}");
    }
    assert!(inbox.is_empty());
    // A frame due later stops the iteration even if the caller keeps asking.
    let mut inbox = StagedInbox::new();
    assert!(inbox.push(frame(5)));
    assert!(inbox.push(frame(9)));
    let mut due = inbox.due(Tick(6));
    assert_eq!(due.next().map(|frame| frame.tick), Some(Tick(5)));
    assert!(due.next().is_none());
    assert!(due.next().is_none());
}

#[test]
fn oldest_after_names_the_oldest_frame_after_the_present_with_one_still_due() {
    // `lactose-predict` after a frame in which no step ran: the frame for the present is
    // still queued, and the one after it is what the server is ahead by (review r1, S1).
    let mut inbox = StagedInbox::new();
    for tick in [11, 14, 15] {
        assert!(inbox.push(frame(tick)));
    }
    let present = Tick(11);
    assert_eq!(inbox.oldest_after(present), Some(Tick(14)));
    assert_eq!(inbox.oldest_after(Tick(10)), Some(Tick(11)));
    assert_eq!(inbox.oldest_after(Tick(14)), Some(Tick(15)));
    assert_eq!(inbox.oldest_after(Tick(15)), None);
    // It reads and takes nothing, and agrees with `due`: the first frame `due` leaves out.
    assert_eq!(inbox.len(), 3);
    for present in 9..17 {
        let due = inbox.due(Tick(present)).count();
        assert_eq!(
            inbox.oldest_after(Tick(present)),
            ticks(inbox.due(Tick(u32::MAX >> 1))).get(due).map(|&tick| Tick(tick)),
            "present {present}"
        );
    }
    // Across the wrap.
    let mut inbox = StagedInbox::new();
    for tick in [u32::MAX - 1, u32::MAX, 0, 3] {
        assert!(inbox.push(frame(tick)));
    }
    assert_eq!(inbox.oldest_after(Tick(u32::MAX)), Some(Tick(0)));
    assert_eq!(inbox.oldest_after(Tick(0)), Some(Tick(3)));
    assert_eq!(inbox.oldest_after(Tick(3)), None);
    assert_eq!(StagedInbox::new().oldest_after(Tick(0)), None);
}

#[test]
fn reset_lets_a_rejoin_through_set_tick_accept_the_new_sessions_frames() {
    // A client that accepted frames up to 100 re-joins a server whose tick is 40 (review
    // r1, S2): without `reset` the inbox refuses the resync and everything after it.
    for with_reset in [false, true] {
        let (mut client, _) = client(Tick(95));
        for tick in 96..=100 {
            assert!(inbox(&mut client).push(frame(tick)));
        }
        assert_eq!(inbox(&mut client).accepted_tick(), Some(Tick(100)));
        client.world_mut().set_tick(Tick(40)).expect("set_tick");
        if with_reset {
            inbox(&mut client).reset();
            assert!(inbox(&mut client).is_empty());
            assert_eq!(inbox(&mut client).accepted_tick(), None);
        }
        let full = inbox(&mut client).push(Arc::new(StagedFrame::resync(Tick(40))));
        let next = inbox(&mut client).push(frame(41));
        assert_eq!((full, next), (with_reset, with_reset), "with reset: {with_reset}");
        let queued = ticks(inbox(&mut client).due(Tick(1 << 30)));
        let expected = if with_reset { vec![40, 41] } else { vec![96, 97, 98, 99, 100] };
        assert_eq!(queued, expected, "with reset: {with_reset}");
    }
}

// ---- The hook ----

fn spawn(
    region: lactose_ecs::RegionId,
    slot_index: Option<u32>,
    components: Vec<lactose_bundle::ComponentData>,
) -> StagedSpawn {
    StagedSpawn { origin: None, region, slot: slot_index.map(slot), components }
}

/// A client at tick 10 with `REGION` and `OTHER_REGION` loaded by the world directly.
fn client_with_regions() -> (Simulation, Skipped) {
    let (mut simulation, skipped) = client(Tick(10));
    for region in [REGION, OTHER_REGION] {
        let config =
            RegionConfig { id: region, seed: seed_of(region), mode: RegionMode::Replicated };
        simulation.world_mut().create_region(config).expect("a region");
    }
    (simulation, skipped)
}

/// One step with `frame` due in it.
fn apply(simulation: Simulation, frame: StagedFrame) -> Simulation {
    let mut simulation = simulation;
    assert_eq!(frame.tick.0, simulation.tick().0 + 1, "the frame is for the next step");
    assert!(inbox(&mut simulation).push(Arc::new(frame)));
    step(&executor(2), simulation, TickInput::default())
}

fn skipped_of(skipped: &Skipped) -> Vec<(&'static str, StableKey, ErrorKind)> {
    skipped.lock().expect("never poisoned").clone()
}

#[test]
fn the_hook_spawns_binds_and_sets_the_slot() {
    let (simulation, skipped) = client_with_regions();
    let origin = PredictionKey { player: PlayerId(3), input_tick: Tick(8), ordinal: 1 };
    let mut frame = StagedFrame::new(Tick(11));
    frame.spawns.insert(
        net(1),
        spawn(REGION, Some(1), vec![data(Position { x: 1, y: 2 }), data(Health(5))]),
    );
    frame.spawns.insert(
        net(2),
        StagedSpawn {
            origin: Some(origin),
            region: OTHER_REGION,
            slot: None,
            components: vec![data(Position { x: 7, y: 7 }), data(Marked)],
        },
    );
    let simulation = apply(simulation, frame);
    let world = simulation.world();
    assert_eq!(world.get::<Position>(key(1)), Some(&Position { x: 1, y: 2 }));
    assert_eq!(world.get::<Health>(key(1)), Some(&Health(5)));
    assert_eq!(world.get::<RemoteSlot>(key(1)), Some(&RemoteSlot { slot: slot(1) }));
    assert_eq!(world.region_of(key(1)), Some(REGION));
    // The predicted spawn is spawned under its origin key and bound to the server's key.
    let predicted = StableKey::Predicted(origin);
    assert_eq!(world.get::<Position>(predicted), Some(&Position { x: 7, y: 7 }));
    assert!(world.get::<Marked>(predicted).is_some());
    assert_eq!(world.alias(origin), Some(net(2)));
    assert_eq!(world.get::<Position>(key(2)), Some(&Position { x: 7, y: 7 }));
    assert_eq!(world.get::<RemoteSlot>(predicted), None);
    assert_eq!(world.region_of(predicted), Some(OTHER_REGION));
    assert!(skipped_of(&skipped).is_empty());
}

#[test]
fn a_spawn_of_a_live_key_is_an_upsert_that_sets_or_removes_the_slot() {
    let (simulation, skipped) = client_with_regions();
    let mut first = StagedFrame::new(Tick(11));
    first.spawns.insert(net(1), spawn(REGION, Some(1), vec![data(Position { x: 1, y: 1 })]));
    first.spawns.insert(net(2), spawn(REGION, None, vec![data(Position { x: 2, y: 2 })]));
    let simulation = apply(simulation, first);
    assert_eq!(simulation.world().get::<RemoteSlot>(key(2)), None);
    // The player claims 1 (slot `None`) and releases 2 (slot set); 1 gains `Health`.
    let mut second = StagedFrame::new(Tick(12));
    second
        .spawns
        .insert(net(1), spawn(REGION, None, vec![data(Position { x: 5, y: 5 }), data(Health(3))]));
    second.spawns.insert(net(2), spawn(REGION, Some(4), vec![data(Position { x: 2, y: 2 })]));
    let simulation = apply(simulation, second);
    let world = simulation.world();
    assert_eq!(world.get::<RemoteSlot>(key(1)), None);
    assert_eq!(world.get::<Position>(key(1)), Some(&Position { x: 5, y: 5 }));
    assert_eq!(world.get::<Health>(key(1)), Some(&Health(3)));
    assert_eq!(world.get::<RemoteSlot>(key(2)), Some(&RemoteSlot { slot: slot(4) }));
    assert!(skipped_of(&skipped).is_empty());
}

#[test]
fn moves_apply_once_and_an_owned_entity_follows_its_owner() {
    let (mut simulation, skipped) = client_with_regions();
    let world = simulation.world_mut();
    world.spawn(key(1), REGION, (Position { x: 0, y: 0 },)).expect("spawn");
    world
        .spawn(key(2), REGION, (Position { x: 0, y: 0 }, OwnedBy { owner: EntityRef::new(key(1)) }))
        .expect("spawn");
    world.spawn(key(3), REGION, (Position { x: 0, y: 0 },)).expect("spawn");
    let cursor = world.structural_log().cursor();
    let mut frame = StagedFrame::new(Tick(11));
    frame.moves.insert(key(1), OTHER_REGION);
    frame.moves.insert(key(2), OTHER_REGION);
    frame.moves.insert(key(3), REGION);
    frame.moves.insert(key(9), OTHER_REGION);
    let simulation = apply(simulation, frame);
    let world = simulation.world();
    assert_eq!(world.region_of(key(1)), Some(OTHER_REGION));
    assert_eq!(world.region_of(key(2)), Some(OTHER_REGION));
    assert_eq!(world.region_of(key(3)), Some(REGION));
    let mut cursor = cursor;
    let moved: Vec<StableKey> = world
        .structural_log()
        .since(&mut cursor)
        .expect("current")
        .filter_map(|entry| match entry.change {
            StructuralChange::Moved { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(moved.iter().filter(|&&moved| moved == key(2)).count(), 1);
    assert_eq!(moved.iter().filter(|&&moved| moved == key(3)).count(), 0);
    assert!(skipped_of(&skipped).is_empty());
}

#[test]
fn removals_go_and_a_row_of_the_same_frame_re_adds_the_component() {
    let (mut simulation, skipped) = client_with_regions();
    let world = simulation.world_mut();
    world.spawn(key(1), REGION, (Position { x: 0, y: 0 }, Health(1), Status(1))).expect("spawn");
    let mut frame = StagedFrame::new(Tick(11));
    frame.removed.insert((key(1), Health::KEY));
    frame.removed.insert((key(1), Status::KEY));
    // Absent component and absent entity: nothing.
    frame.removed.insert((key(1), Marked::KEY));
    frame.removed.insert((key(9), Health::KEY));
    assert!(frame.push_row(key(1), Status::KEY, Tick(11), false, bytes(&Status(7))));
    let simulation = apply(simulation, frame);
    let world = simulation.world();
    assert_eq!(world.get::<Health>(key(1)), None);
    assert_eq!(world.get::<Status>(key(1)), Some(&Status(7)));
    assert!(skipped_of(&skipped).is_empty());
}

#[test]
fn rows_are_written_tombstones_despawn_and_an_equal_row_keeps_its_stamp() {
    let (mut simulation, skipped) = client_with_regions();
    let world = simulation.world_mut();
    world.spawn(key(1), REGION, (Position { x: 0, y: 0 }, Health(1))).expect("spawn");
    world.spawn(key(2), REGION, (Position { x: 0, y: 0 }, Health(2))).expect("spawn");
    world.spawn(key(3), OTHER_REGION, (Position { x: 0, y: 0 },)).expect("spawn");
    let executor = executor(2);
    simulation = step(&executor, simulation, TickInput::default());
    let mark = simulation.world().mark();
    let mut frame = StagedFrame::new(Tick(12));
    assert!(frame.push_row(
        key(1),
        Position::KEY,
        Tick(12),
        false,
        bytes(&Position { x: 4, y: 4 })
    ));
    // Equal to what the row holds.
    assert!(frame.push_row(key(2), Health::KEY, Tick(12), false, bytes(&Health(2))));
    // A row for an entity that does not exist (its spawn was lost): nothing.
    assert!(frame.push_row(key(8), Health::KEY, Tick(12), false, bytes(&Health(8))));
    frame.despawns.insert(
        key(3),
        Tombstone { region: OTHER_REGION, tick: Tick(12), cause: TombstoneCause::Destroyed },
    );
    frame.despawns.insert(
        key(9),
        Tombstone { region: OTHER_REGION, tick: Tick(12), cause: TombstoneCause::Destroyed },
    );
    let simulation = apply(simulation, frame);
    let world = simulation.world();
    assert_eq!(world.get::<Position>(key(1)), Some(&Position { x: 4, y: 4 }));
    assert!(!world.contains(key(3)));
    assert!(!world.contains(key(8)));
    let changed: BTreeSet<StableKey> =
        world.changed_since::<Position>(mark).expect("current").map(|(key, _)| key).collect();
    assert_eq!(changed, BTreeSet::from([key(1)]));
    let changed: Vec<StableKey> =
        world.changed_since::<Health>(mark).expect("current").map(|(key, _)| key).collect();
    assert_eq!(changed, vec![]);
    assert!(skipped_of(&skipped).is_empty());
}

/// A `Left` tombstone is skipped only for an entity residing in the region it names while
/// that region is loaded (the region's own unload or reload settles it); one of an entity
/// residing elsewhere, and every `Destroyed` one, despawns.
#[test]
fn a_left_tombstone_is_skipped_only_for_an_entity_its_loaded_region_holds() {
    let (mut simulation, skipped) = client_with_regions();
    let world = simulation.world_mut();
    for (n, region) in [(1, REGION), (2, REGION), (3, OTHER_REGION), (4, REGION), (5, REGION)] {
        world.spawn(key(n), region, (Position { x: 0, y: 0 },)).expect("spawn");
    }
    let tombstone = |region, cause| Tombstone { region, tick: Tick(11), cause };
    let mut frame = StagedFrame::new(Tick(11));
    // Loaded where the server has it: skipped.
    frame.despawns.insert(key(1), tombstone(REGION, TombstoneCause::Left));
    // Moved on the server into a region the client does not hold.
    frame.despawns.insert(key(2), tombstone(THIRD_REGION, TombstoneCause::Left));
    frame.despawns.insert(key(3), tombstone(OTHER_REGION, TombstoneCause::Destroyed));
    // In another region than the one the tombstone names (a move the client never got).
    frame.despawns.insert(key(4), tombstone(OTHER_REGION, TombstoneCause::Left));
    frame.despawns.insert(key(5), tombstone(REGION, TombstoneCause::Destroyed));
    let simulation = apply(simulation, frame);
    let world = simulation.world();
    let live: Vec<bool> = (1..=5).map(|n| world.contains(key(n))).collect();
    assert_eq!(live, [true, false, false, false, false]);
    assert!(skipped_of(&skipped).is_empty());
}

#[test]
fn a_failing_entry_is_skipped_and_the_rest_of_the_frame_applies() {
    let (mut simulation, skipped) = client_with_regions();
    simulation.world_mut().spawn(key(1), REGION, (Position { x: 0, y: 0 },)).expect("spawn");
    simulation.world_mut().spawn(key(2), REGION, (Position { x: 0, y: 0 },)).expect("spawn");
    let mut frame = StagedFrame::new(Tick(11));
    // A spawn into a region the world has not loaded.
    frame.spawns.insert(net(5), spawn(THIRD_REGION, Some(1), vec![data(Position { x: 5, y: 5 })]));
    frame.spawns.insert(net(6), spawn(REGION, Some(2), vec![data(Position { x: 6, y: 6 })]));
    // A move into a region the world has not loaded.
    frame.moves.insert(key(1), THIRD_REGION);
    // A row whose bytes the world refuses.
    assert!(frame.push_row(key(1), Health::KEY, Tick(11), false, &[1, 2, 3]));
    assert!(frame.push_row(
        key(2),
        Position::KEY,
        Tick(11),
        false,
        bytes(&Position { x: 2, y: 2 })
    ));
    frame.despawns.insert(
        key(1),
        Tombstone { region: REGION, tick: Tick(11), cause: TombstoneCause::Destroyed },
    );
    let simulation = apply(simulation, frame);
    let world = simulation.world();
    assert!(!world.contains(key(5)));
    assert_eq!(world.get::<RemoteSlot>(key(6)), Some(&RemoteSlot { slot: slot(2) }));
    assert_eq!(world.get::<Position>(key(2)), Some(&Position { x: 2, y: 2 }));
    assert!(!world.contains(key(1)));
    let kinds: Vec<(&str, StableKey)> =
        skipped_of(&skipped).into_iter().map(|(what, key, _)| (what, key)).collect();
    assert_eq!(kinds, vec![("spawn", key(5)), ("move", key(1)), ("row", key(1))]);
    let errors: Vec<ErrorKind> =
        skipped_of(&skipped).into_iter().map(|(_, _, kind)| kind).collect();
    assert_eq!(errors, vec![ErrorKind::NotFound, ErrorKind::NotFound, ErrorKind::InvalidData]);
}

#[test]
fn a_region_loaded_by_its_bundle_and_then_spawned_by_the_stream_ends_with_its_slot() {
    // Review probe s1: the region's overlay carries a cooked entity, and the stream sends
    // the same entity whole as a spawn in the same step.
    let (mut simulation, skipped) = client(Tick(10));
    let cooked = NetId::cooked(REGION.0 * 1000 + 1);
    let baseline = baseline(vec![BaselineRow {
        id: cooked,
        components: vec![data(Position { x: 5, y: 5 }), data(Health(5))],
    }]);
    let store = Store::with(&baseline);
    let mut frame = StagedFrame::new(Tick(11));
    frame.regions.insert(REGION, RegionChange::Relevant(relevance(REGION, &baseline)));
    frame.spawns.insert(
        cooked,
        spawn(REGION, Some(3), vec![data(Position { x: 6, y: 5 }), data(Health(5))]),
    );
    let (input, accepted) = between_runs(&mut simulation, [frame], &store);
    assert_eq!(accepted, vec![true]);
    assert!(input.loaded.contains_key(&REGION));
    let executor = executor(2);
    let simulation = step(&executor, simulation, input);
    let world = simulation.world();
    let entity = StableKey::Net(cooked);
    assert_eq!(world.region_of(entity), Some(REGION));
    assert_eq!(world.get::<Position>(entity), Some(&Position { x: 6, y: 5 }));
    assert_eq!(world.get::<RemoteSlot>(entity), Some(&RemoteSlot { slot: slot(3) }));
    assert!(skipped_of(&skipped).is_empty());
}

#[test]
fn a_full_frame_over_a_client_with_stale_rows_ends_with_the_frames_values_and_slots() {
    // Review probe s2: a client holds entities from the stream; a resync's full frame
    // names the same region and entities with other values and fresh slots, and one
    // entity the client holds is gone on the server.
    let executor = executor(2);
    let (mut simulation, skipped) = client(Tick(10));
    let baseline = baseline(vec![]);
    let store = Store::with(&baseline);
    let mut first = StagedFrame::resync(Tick(10));
    first.regions.insert(REGION, RegionChange::Relevant(relevance(REGION, &baseline)));
    first.regions.insert(OTHER_REGION, RegionChange::Relevant(relevance(OTHER_REGION, &baseline)));
    first.spawns.insert(net(1), spawn(REGION, Some(1), vec![data(Position { x: 1, y: 1 })]));
    first.spawns.insert(net(2), spawn(REGION, Some(2), vec![data(Position { x: 2, y: 2 })]));
    first.spawns.insert(net(3), spawn(OTHER_REGION, Some(3), vec![data(Position { x: 3, y: 3 })]));
    let (input, _) = between_runs(&mut simulation, [first], &store);
    simulation = step(&executor, simulation, input);
    assert!(simulation.world().contains(key(3)));

    let mut full = StagedFrame::resync(Tick(12));
    full.regions.insert(REGION, RegionChange::Relevant(relevance(REGION, &baseline)));
    full.spawns.insert(net(1), spawn(REGION, Some(7), vec![data(Position { x: 10, y: 1 })]));
    full.spawns.insert(net(2), spawn(REGION, None, vec![data(Position { x: 20, y: 2 })]));
    let (input, accepted) = between_runs(&mut simulation, [full], &store);
    assert_eq!(accepted, vec![true]);
    // Every loaded region is unloaded, and the relevant one reloads from its message.
    assert_eq!(input.unloaded, BTreeSet::from([REGION, OTHER_REGION]));
    assert_eq!(input.loaded.keys().copied().collect::<Vec<_>>(), vec![REGION]);
    simulation = step(&executor, simulation, input);
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
    let world = simulation.world();
    assert_eq!(world.regions().collect::<Vec<_>>(), vec![REGION]);
    assert_eq!(world.get::<Position>(key(1)), Some(&Position { x: 10, y: 1 }));
    assert_eq!(world.get::<RemoteSlot>(key(1)), Some(&RemoteSlot { slot: slot(7) }));
    assert_eq!(world.get::<Position>(key(2)), Some(&Position { x: 20, y: 2 }));
    assert_eq!(world.get::<RemoteSlot>(key(2)), None);
    assert!(!world.contains(key(3)));
    assert!(skipped_of(&skipped).is_empty());
}
