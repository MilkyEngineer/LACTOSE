//! The slot table on null (`README.md`, "Slots", "Writers" and "The batch"): slots follow a
//! world through extract's sink, freed by its despawns and reconciled by the full pass after
//! a restore; a load at capacity is placed whole; the kinds apply in their order whatever
//! began first; a region's last remote batch is applied alone, stopped by a lost `Client`
//! pass and left out where a `Client` pass overlaps it; a batch names each slot once.

use lactose_core::{StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink};
use lactose_renderer::{Opened, Renderer, RendererDesc, ScatterEntry, Slot};

use crate::common::{
    Ender, RegionLog, Rig, key, next_frame, null, ok, place, row, simulation, spawn, world_pass,
};

fn renderer(instances: u32, rows: u32) -> Renderer {
    let desc = RendererDesc { instances, rows, regions: 2, shaders: None };
    ok(Renderer::new(null(), &desc))
}

fn tick(n: u32) -> Tick {
    Tick(n)
}

#[test]
fn slots_follow_a_prediction_world_through_its_sink() {
    let mut renderer = renderer(64, 256);
    let mut ender = Ender::new(&**renderer.device());
    let mut rig = Rig::new(simulation(WorldKind::Prediction), Box::new(renderer.sink()));
    for n in [3, 1, 2] {
        spawn(rig.world_mut(), n, n as f64);
    }
    rig.frame();
    let stats = ok(renderer.prepare());
    assert!(!stats.any_lost());
    assert_eq!((stats.pass_rows, stats.allocated, stats.freed, stats.live), (3, 3, 0, 3));
    let slots = renderer.slots();
    // New instances take the lowest slots in ascending key order.
    for (n, slot) in [(1, 0), (2, 1), (3, 2)] {
        assert_eq!(slots.get(WorldKind::Prediction, key(n)), Some(Slot::new(slot)));
        let holder = slots.holder(Slot::new(slot)).expect("held");
        assert_eq!((holder.kind, holder.key), (WorldKind::Prediction, key(n)));
    }
    assert_eq!(slots.get(WorldKind::Server, key(1)), None);
    let snapshot = rig.world().snapshot();
    next_frame(&mut renderer, &mut ender);

    // A despawn reaches the table through `removed`, and frees the slot.
    rig.world_mut().despawn(key(2)).expect("a despawn");
    rig.step();
    rig.frame();
    let stats = ok(renderer.prepare());
    assert_eq!((stats.freed, stats.live), (1, 2));
    assert_eq!(renderer.slots().get(WorldKind::Prediction, key(2)), None);
    assert_eq!(renderer.slots().extent(), 3);
    next_frame(&mut renderer, &mut ender);

    // A new instance takes the freed slot.
    spawn(rig.world_mut(), 4, 4.0);
    rig.step();
    rig.frame();
    let stats = ok(renderer.prepare());
    assert_eq!((stats.allocated, stats.live), (1, 3));
    assert_eq!(renderer.slots().get(WorldKind::Prediction, key(4)), Some(Slot::new(1)));
    next_frame(&mut renderer, &mut ender);

    // A rollback: the world restored to the first frame's state. The pass after a restore
    // is full, and the table holds what it wrote and nothing else.
    rig.world_mut().restore(&snapshot).expect("a restore");
    place(rig.world_mut(), 1, 9.0);
    rig.frame();
    let stats = ok(renderer.prepare());
    assert!(!stats.any_lost());
    let slots = renderer.slots();
    assert_eq!(slots.get(WorldKind::Prediction, key(4)), None, "the restore despawned it");
    for n in [1, 2, 3] {
        assert!(slots.get(WorldKind::Prediction, key(n)).is_some(), "the restore brought {n}");
    }
    assert_eq!((stats.freed, stats.allocated, stats.live), (1, 1, 3));
}

#[test]
fn a_full_pass_sweeps_what_it_did_not_write() {
    let mut renderer = renderer(16, 64);
    let mut ender = Ender::new(&**renderer.device());
    world_pass(&renderer, WorldKind::Server, tick(1), false, &[row(1, 0.0), row(2, 0.0)]);
    ok(renderer.prepare());
    next_frame(&mut renderer, &mut ender);
    world_pass(&renderer, WorldKind::Server, tick(2), true, &[row(2, 1.0)]);
    let stats = ok(renderer.prepare());
    assert_eq!((stats.freed, stats.live), (1, 1));
    assert_eq!(renderer.slots().get(WorldKind::Server, key(1)), None);
    // The sweep frees the kind's slots alone.
    next_frame(&mut renderer, &mut ender);
    world_pass(&renderer, WorldKind::Prediction, tick(2), false, &[row(1, 0.0)]);
    world_pass(&renderer, WorldKind::Server, tick(3), true, &[]);
    let stats = ok(renderer.prepare());
    assert_eq!(stats.live, 1);
    assert!(renderer.slots().get(WorldKind::Prediction, key(1)).is_some());
}

#[test]
fn a_load_that_replaces_a_kinds_keys_at_capacity_is_placed_whole() {
    let instances = 64_u32;
    let old: Vec<InstanceRow> = (1..=u64::from(instances)).map(|n| row(n, 0.0)).collect();
    let new: Vec<InstanceRow> = (1..=u64::from(instances)).map(|n| row(1000 + n, 0.0)).collect();
    let mut renderer = renderer(instances, 2 * instances + 8);
    let mut ender = Ender::new(&**renderer.device());
    world_pass(&renderer, WorldKind::Server, tick(1), false, &old);
    assert_eq!(ok(renderer.prepare()).live, instances);
    next_frame(&mut renderer, &mut ender);
    world_pass(&renderer, WorldKind::Server, tick(2), true, &new);
    let stats = ok(renderer.prepare());
    assert_eq!((stats.unplaced, stats.unplaced_in(WorldKind::Server)), (0, 0));
    assert_eq!((stats.freed, stats.allocated, stats.live), (instances, instances, instances));
    let slots = renderer.slots();
    assert!(new.iter().all(|row| slots.get(WorldKind::Server, row.key).is_some()));
    assert!(old.iter().all(|row| slots.get(WorldKind::Server, row.key).is_none()));
}

#[test]
fn a_kind_that_grows_finds_no_slot_a_later_kind_frees_and_counts_it() {
    let mut renderer = renderer(2, 64);
    let mut ender = Ender::new(&**renderer.device());
    world_pass(&renderer, WorldKind::Prediction, tick(1), false, &[row(1, 0.0), row(2, 0.0)]);
    ok(renderer.prepare());
    next_frame(&mut renderer, &mut ender);
    world_pass(&renderer, WorldKind::Server, tick(1), false, &[row(3, 0.0)]);
    world_pass(&renderer, WorldKind::Prediction, tick(2), true, &[row(1, 0.0)]);
    let stats = ok(renderer.prepare());
    assert_eq!((stats.unplaced, stats.unplaced_in(WorldKind::Server)), (1, 1));
    assert_eq!(stats.live, 1);
    // The next frame places it.
    next_frame(&mut renderer, &mut ender);
    world_pass(&renderer, WorldKind::Server, tick(2), true, &[row(3, 0.0)]);
    let stats = ok(renderer.prepare());
    assert_eq!((stats.unplaced, stats.live), (0, 2));
}

#[test]
fn the_table_does_not_depend_on_which_kinds_pass_began_first() {
    let tables: Vec<Vec<_>> = [false, true]
        .map(|prediction_first| {
            let mut renderer = renderer(16, 64);
            let (mut prediction, mut server) = (renderer.sink(), renderer.sink());
            let begin = |sink: &mut lactose_renderer::ExtractSink, kind| {
                sink.begin(ExtractPass { kind, tick: tick(1), full: false });
            };
            if prediction_first {
                begin(&mut prediction, WorldKind::Prediction);
                begin(&mut server, WorldKind::Server);
            } else {
                begin(&mut server, WorldKind::Server);
                begin(&mut prediction, WorldKind::Prediction);
            }
            prediction.write(&[row(1, 0.0)]);
            server.write(&[row(2, 0.0)]);
            prediction.end();
            server.end();
            let stats = ok(renderer.prepare());
            assert_eq!((stats.allocated, stats.unplaced), (2, 0));
            assert_eq!(renderer.slots().get(WorldKind::Server, key(2)), Some(Slot::new(0)));
            renderer.slots().iter().collect()
        })
        .into_iter()
        .collect();
    assert_eq!(tables[0], tables[1]);
}

/// A region that holds two frames' writers: frame 1 left it unprepared, or its `prepare` was
/// refused. Frame 1's batch holds `gone` and `kept`; frame 2's `Client` pass removes `gone`
/// and its batch holds `kept`. The region's last batch alone is applied.
fn last_remote_batch(refused: bool) {
    let (gone, kept) = (row(1, 0.0), row(2, 0.0));
    let mut renderer = renderer(16, 64);
    if refused {
        ok(renderer.prepare());
        assert_eq!(ok(renderer.open()), Opened::Ready);
    }
    world_pass(&renderer, WorldKind::Client, tick(1), false, &[]);
    renderer.remote().write(&[gone, kept]);
    if refused {
        assert!(renderer.prepare().is_err());
        renderer.torn();
    }
    let mut sink = renderer.sink();
    sink.begin(ExtractPass { kind: WorldKind::Client, tick: tick(2), full: false });
    sink.removed(&[gone.key]);
    sink.end();
    renderer.remote().write(&[kept]);
    let stats = ok(renderer.prepare());
    assert!(!stats.lost(WorldKind::Client));
    assert_eq!((stats.remote_rows, stats.live), (3, 1));
    assert_eq!(renderer.slots().get(WorldKind::Client, gone.key), None);
    assert!(renderer.slots().get(WorldKind::Client, kept.key).is_some());
}

#[test]
fn a_region_left_unprepared_applies_its_last_remote_batch_alone() {
    last_remote_batch(false);
}

#[test]
fn a_region_after_a_refused_prepare_applies_its_last_remote_batch_alone() {
    last_remote_batch(true);
}

/// Frame 1's `Client` pass (two notes) and its batch (two notes, two rows) fill `rows` of
/// the region but `spare`; frame 2's `Client` pass, which removes `gone`, finds no room for
/// its `begin` (`spare` 0) or for its removed key (`spare` 1), and is lost. Frame 1's batch is
/// then the region's last whole batch. Where the `begin` found no room no `Client` note lies
/// after the batch's opening, so only the loss keeps it from bringing `gone` back; where the
/// `begin` fit, its note leaves the batch out as well.
fn client_lost_after_a_batch(spare: u32) {
    let (gone, kept) = (row(1, 0.0), row(2, 0.0));
    let mut renderer = renderer(16, 6 + spare);
    world_pass(&renderer, WorldKind::Client, tick(1), false, &[]);
    renderer.remote().write(&[gone, kept]);
    let mut sink = renderer.sink();
    sink.begin(ExtractPass { kind: WorldKind::Client, tick: tick(2), full: false });
    sink.removed(&[gone.key]);
    sink.end();
    let stats = ok(renderer.prepare());
    assert!(stats.lost(WorldKind::Client));
    assert_eq!((stats.remote_rows, stats.live), (2, 0));
    assert_eq!(renderer.slots().get(WorldKind::Client, gone.key), None);
    assert_eq!(renderer.slots().get(WorldKind::Client, kept.key), None);
}

#[test]
fn a_client_pass_lost_at_its_begin_after_a_batch_stops_every_batch() {
    client_lost_after_a_batch(0);
}

#[test]
fn a_client_pass_lost_at_its_removed_keys_after_a_batch_stops_every_batch() {
    client_lost_after_a_batch(1);
}

#[test]
fn a_client_pass_lost_frees_the_kinds_held_slots() {
    let mut renderer = renderer(16, 64);
    let mut ender = Ender::new(&**renderer.device());
    renderer.remote().write(&[row(1, 0.0)]);
    world_pass(&renderer, WorldKind::Server, tick(1), false, &[row(2, 0.0)]);
    assert_eq!(ok(renderer.prepare()).live, 2);
    next_frame(&mut renderer, &mut ender);
    let mut sink = renderer.sink();
    sink.begin(ExtractPass { kind: WorldKind::Client, tick: tick(2), full: false });
    drop(sink);
    renderer.remote().write(&[row(1, 1.0)]);
    let stats = ok(renderer.prepare());
    assert!(stats.lost(WorldKind::Client) && !stats.lost(WorldKind::Server));
    assert_eq!((stats.freed, stats.live), (1, 1));
    let batch = renderer.batch();
    assert_eq!(batch.entries()[0].flags, ScatterEntry::FREE);
}

fn overlapping(batch_first: bool) {
    let (gone, kept) = (row(1, 0.0), row(2, 0.0));
    let mut renderer = renderer(16, 64);
    let mut sink = renderer.sink();
    let pass = ExtractPass { kind: WorldKind::Client, tick: tick(1), full: false };
    if batch_first {
        renderer.remote().write(&[gone, kept]);
        sink.begin(pass);
    } else {
        sink.begin(pass);
        renderer.remote().write(&[gone, kept]);
    }
    sink.removed(&[gone.key]);
    sink.end();
    let stats = ok(renderer.prepare());
    assert!(!stats.any_lost());
    assert_eq!((stats.remote_rows, stats.dropped, stats.live), (2, 0, 0));
    assert_eq!(renderer.slots().get(WorldKind::Client, gone.key), None);
    assert_eq!(renderer.slots().get(WorldKind::Client, kept.key), None);
}

#[test]
fn a_batch_written_inside_a_client_pass_is_left_out() {
    overlapping(false);
}

#[test]
fn a_batch_written_before_a_client_pass_is_left_out() {
    overlapping(true);
}

#[test]
fn a_batch_names_each_slot_once_the_later_row_kept() {
    let mut renderer = renderer(16, 64);
    let log = RegionLog::default();
    let rows = [row(1, 0.0), row(2, 0.0), row(3, 0.0), row(2, 5.0)];
    log.remote(&mut renderer, &rows);
    let stats = ok(renderer.prepare());
    let region = log.take();
    assert_eq!((stats.duplicates, stats.remote_rows, stats.live), (1, 4, 3));
    let batch = renderer.batch();
    assert_eq!(batch.entries().len(), stats.entries as usize);
    for &range in batch.dispatches() {
        let mut slots: Vec<Slot> = batch.run(range).iter().map(|entry| entry.slot).collect();
        slots.sort_unstable();
        assert!(slots.windows(2).all(|pair| pair[0] != pair[1]), "a dispatch names a slot twice");
    }
    let slot = renderer.slots().get(WorldKind::Client, key(2)).expect("written");
    let entry = batch.entries().iter().find(|entry| entry.slot == slot).expect("an entry");
    assert_eq!(region[entry.row as usize], Some(rows[3]));
    // A remote entry carries neither base flag.
    assert_eq!(entry.flags, 0);
}

#[test]
fn duplicates_of_a_world_pass_are_resolved_in_region_order_with_their_flags() {
    let mut renderer = renderer(16, 64);
    let mut ender = Ender::new(&**renderer.device());
    let log = RegionLog::default();
    let pass = |tick| ExtractPass { kind: WorldKind::Server, tick, full: false };
    log.pass(&renderer, pass(tick(1)), &[], &[row(1, 0.0), row(2, 0.0)]);
    ok(renderer.prepare());
    log.take();
    next_frame(&mut renderer, &mut ender);
    // Each instance twice in one pass, in two writes.
    let mut sink = log.sink(renderer.sink());
    sink.begin(pass(tick(2)));
    sink.write(&[row(1, 1.0), row(2, 1.0)]);
    sink.write(&[row(2, 2.0), row(1, 2.0)]);
    sink.end();
    let stats = ok(renderer.prepare());
    let region = log.take();
    assert_eq!(stats.duplicates, 2);
    let both = ScatterEntry::TAKE_BASE | ScatterEntry::HAS_BASE;
    let batch = renderer.batch();
    assert_eq!(batch.entries().len(), 2);
    for entry in batch.entries() {
        assert_eq!(entry.flags, both);
        let row = region[entry.row as usize].expect("a row");
        assert_eq!(row.transform.position.offset[0], 2.0, "the later row is kept");
    }
}

/// Two passes of one kind in a region each write a key: the passes apply in begin order, so
/// the later pass's row is the one the slot is left with.
#[test]
fn a_key_written_by_two_passes_of_a_kind_takes_the_later_pass() {
    let mut renderer = renderer(16, 64);
    let log = RegionLog::default();
    let pass = ExtractPass { kind: WorldKind::Prediction, tick: tick(1), full: false };
    log.pass(&renderer, pass, &[], &[row(1, 1.0)]);
    log.pass(&renderer, pass, &[], &[row(1, 2.0)]);
    let stats = ok(renderer.prepare());
    let region = log.take();
    assert_eq!((stats.duplicates, stats.live), (0, 1));
    let batch = renderer.batch();
    // One entry in each pass's run, the second reading the second row.
    assert_eq!(batch.dispatches().len(), 2);
    let last = batch.entries().last().expect("two entries");
    assert_eq!(region[last.row as usize].map(|row| row.transform), Some(row(1, 2.0).transform));
}

#[test]
fn rows_of_a_key_held_by_another_kind_take_a_slot_of_their_own() {
    let mut renderer = renderer(16, 64);
    world_pass(&renderer, WorldKind::Server, tick(1), false, &[row(1, 0.0)]);
    world_pass(&renderer, WorldKind::Prediction, tick(1), false, &[row(1, 0.0)]);
    let stats = ok(renderer.prepare());
    assert_eq!(stats.live, 2);
    let slots = renderer.slots();
    assert_ne!(slots.get(WorldKind::Server, key(1)), slots.get(WorldKind::Prediction, key(1)));
    let keys: Vec<StableKey> = slots.iter().map(|(_, holder)| holder.key).collect();
    assert_eq!(keys, [key(1), key(1)]);
}
