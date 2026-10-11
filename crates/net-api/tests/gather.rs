//! The replication gather over the real ECS and sim (README, "The gather"; proof rows "The
//! gather" and "The gather's decision"): the same frames under every layout SOUR runs and
//! with one or four workers, exactly the rows the typed change query selects, an empty
//! gather when nothing changed, the full pass on a stale mark or cursor, the merge of a
//! frame not taken, the per-region slices and the structural entries.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeSet;

use common::*;
use lactose_core::{ComponentKey, StableKey, Tick};
use lactose_ecs::{LayoutConfig, LogEntry, RegionId, StructuralChange, World};
use lactose_net_api::StagingFrame;
use lactose_reflect::Reflect;
use lactose_sim::{Simulation, TickInput};

const ENTITIES: u64 = 40;

/// The scene's population: every entity has `Position` and `Health`; every third moves;
/// every fifth has the unreplicated `Secret`; every seventh is `Marked`. Alternate regions.
fn populate(world: &mut World) {
    create_regions(world, &[REGION, OTHER_REGION]);
    for n in 1..=ENTITIES {
        let region = if n % 2 == 0 { REGION } else { OTHER_REGION };
        let position = Position { x: i32::try_from(n).expect("small"), y: 0 };
        let health = Health(i32::try_from(n).expect("small") * 10);
        world.spawn(key(n), region, (position, health)).expect("a spawn");
        if n % 3 == 0 {
            world.insert(key(n), Mover(i32::try_from(n % 4).expect("small") + 1)).expect("insert");
        }
        if n % 5 == 0 {
            world.insert(key(n), Secret(0)).expect("insert");
        }
        if n % 7 == 0 {
            world.insert(key(n), Marked).expect("insert");
        }
    }
}

/// What `app` does to the server between the runs of frame `index`: direct writes,
/// spawns, despawns, moves, additions and removals.
fn script(world: &mut World, index: u32) {
    match index {
        2 => world.insert(key(5), Health(555)).expect("insert"),
        3 => {
            world
                .spawn(key(100), OTHER_REGION, (Position { x: 1, y: 1 }, Mover(2)))
                .expect("spawn");
        }
        4 => world.despawn(key(7)).expect("despawn"),
        5 => world.move_to_region(key(8), OTHER_REGION).expect("move"),
        6 => world.insert(key(9), Status(3)).expect("insert"),
        7 => {
            // The erased write of an equal value writes nothing, as the hook's upsert relies on.
            assert!(!world.insert_erased(key(9), &data(Status(3))).expect("an equal write"));
            world.insert(key(12), Health(-1)).expect("insert");
        }
        8 => {
            world.remove::<Status>(key(9)).expect("remove");
            world.remove::<Health>(key(10)).expect("remove");
        }
        9 => world.insert(key(11), Marked).expect("insert"),
        10 => {
            world.move_to_region(key(6), OTHER_REGION).expect("move");
            world.despawn(key(100)).expect("despawn");
        }
        _ => {}
    }
}

const FRAMES: u32 = 12;

/// Runs the scene: each frame scripts the world, runs one to three ticks and the passes,
/// and takes the gathered frame.
fn scene(layout: LayoutConfig, workers: u32) -> Vec<StagingFrame> {
    let executor = executor(workers);
    let mut simulation = server(layout);
    populate(simulation.world_mut());
    let mut frames = Vec::new();
    for index in 0..FRAMES {
        script(simulation.world_mut(), index);
        for _ in 0..=index % 3 {
            simulation = step(&executor, simulation, TickInput::default());
        }
        simulation = passes(&executor, simulation);
        frames.push(take(&mut simulation));
    }
    frames
}

fn keys_of(frame: &StagingFrame, component: ComponentKey) -> BTreeSet<StableKey> {
    frame.rows().filter(|row| row.component == component).map(|row| row.key).collect()
}

#[test]
fn the_frames_are_the_same_under_every_layout_and_worker_count() {
    let reference = scene(LayoutConfig::default(), 1);
    let mut runs = 0;
    for layout in layouts() {
        for workers in [1, 4] {
            let frames = scene(layout, workers);
            assert_eq!(frames.len(), reference.len());
            for (index, (frame, expected)) in frames.iter().zip(&reference).enumerate() {
                assert_eq!(
                    frame_rows(frame),
                    frame_rows(expected),
                    "frame {index} under {layout:?}"
                );
                assert_eq!(frame.structural(), expected.structural(), "frame {index}");
                assert_eq!(frame, expected, "frame {index} under {layout:?} and {workers} workers");
            }
            runs += 1;
        }
    }
    assert_eq!(runs, 26);
}

#[test]
fn the_scene_gathers_what_changed_and_nothing_else() {
    let frames = scene(LayoutConfig::default(), 4);
    let movers: BTreeSet<StableKey> = (1..=ENTITIES).filter(|n| n % 3 == 0).map(key).collect();

    // The first gather is full: every replicated row, nothing structural, no unreplicated
    // component.
    let first = &frames[0];
    assert!(first.full);
    assert!(first.structural().is_empty());
    let marked = (1..=ENTITIES).filter(|n| n % 7 == 0).count();
    assert_eq!(first.row_count(), 2 * usize::try_from(ENTITIES).expect("small") + marked);
    assert!(first.rows().all(|row| row.component != Mover::KEY && row.component != Secret::KEY));

    // Every later frame is incremental, and its `Position` rows are the movers'.
    for (index, frame) in frames.iter().enumerate().skip(1) {
        assert!(!frame.full, "frame {index}");
        assert!(frame.rows().all(|row| row.component != Secret::KEY), "frame {index}");
        let mut expected = movers.clone();
        if (3..10).contains(&index) {
            expected.insert(key(100));
        }
        if index >= 4 {
            expected.remove(&key(7));
        }
        let positions = keys_of(frame, Position::KEY);
        assert_eq!(positions, expected, "frame {index}");
    }

    // The direct writes, each in exactly one frame.
    assert_eq!(keys_of(&frames[2], Health::KEY), BTreeSet::from([key(5)]));
    assert_eq!(keys_of(&frames[3], Health::KEY), BTreeSet::new());
    assert_eq!(keys_of(&frames[6], Status::KEY), BTreeSet::from([key(9)]));
    // An equal write stamps nothing.
    assert_eq!(keys_of(&frames[7], Status::KEY), BTreeSet::new());
    assert_eq!(keys_of(&frames[7], Health::KEY), BTreeSet::from([key(12)]));
    assert_eq!(keys_of(&frames[9], Marked::KEY), BTreeSet::from([key(11)]));
    for row in frames[9].rows().filter(|row| row.component == Marked::KEY) {
        assert!(row.bytes.is_empty());
    }

    // The structural entries, in log order, in the frame after the change.
    let changes = |index: usize| -> Vec<StructuralChange> {
        frames[index].structural().iter().map(|entry| entry.change).collect()
    };
    assert!(
        changes(3).contains(&StructuralChange::Spawned { key: key(100), region: OTHER_REGION })
    );
    assert!(
        changes(4).contains(&StructuralChange::Despawned { key: key(7), region: OTHER_REGION })
    );
    assert_eq!(
        changes(5),
        vec![StructuralChange::Moved { key: key(8), from: REGION, to: OTHER_REGION }]
    );
    assert!(changes(6).contains(&StructuralChange::Added { key: key(9), component: Status::KEY }));
    assert!(
        changes(8).contains(&StructuralChange::Removed { key: key(9), component: Status::KEY })
    );
    assert!(
        changes(8).contains(&StructuralChange::Removed { key: key(10), component: Health::KEY })
    );
    assert_eq!(changes(10).len(), 2);
    for index in [0, 1, 2, 11] {
        assert_eq!(changes(index), vec![], "frame {index}");
    }

    // Rows are sorted by (region, key, component) and hold one value per (key, component).
    for frame in &frames {
        let order: Vec<_> = frame.rows().map(|row| (row.region, row.key, row.component)).collect();
        let mut sorted = order.clone();
        sorted.sort();
        sorted.dedup_by_key(|row| (row.1, row.2));
        assert_eq!(order, sorted);
    }
}

#[test]
fn a_gather_with_nothing_changed_is_empty() {
    let executor = executor(2);
    let mut simulation = server(LayoutConfig::default());
    populate(simulation.world_mut());
    simulation = step(&executor, simulation, TickInput::default());
    simulation = passes(&executor, simulation);
    assert!(take(&mut simulation).full);

    // The passes again with no tick between: nothing changed since the mark.
    simulation = passes(&executor, simulation);
    let second = take(&mut simulation);
    assert!(second.is_empty(), "{:?}", frame_rows(&second));

    // The movers' rows of the last tick are not resent (the mark is a stamp, not a tick).
    simulation = step(&executor, simulation, TickInput::default());
    simulation = passes(&executor, simulation);
    let third = take(&mut simulation);
    assert_eq!(
        keys_of(&third, Position::KEY),
        (1..=ENTITIES).filter(|n| n % 3 == 0).map(key).collect::<BTreeSet<_>>()
    );
    assert!(!keys_of(&third, Position::KEY).contains(&key(1)));
    simulation = passes(&executor, simulation);
    assert!(take(&mut simulation).is_empty());
}

#[test]
fn a_world_mut_write_between_gathers_appears_exactly_once() {
    let executor = executor(2);
    let mut simulation = server(LayoutConfig::default());
    populate(simulation.world_mut());
    simulation = step(&executor, simulation, TickInput::default());
    simulation = passes(&executor, simulation);
    let _ = take(&mut simulation);

    // Written between runs, after the gather and before any tick: stamped at the tick
    // the gather ran in, after its mark.
    simulation.world_mut().insert(key(1), Health(-5)).expect("insert");
    simulation = passes(&executor, simulation);
    let first = take(&mut simulation);
    assert_eq!(
        first.rows().map(|row| (row.key, row.component)).collect::<Vec<_>>(),
        vec![(key(1), Health::KEY)]
    );
    assert_eq!(first.rows().next().map(|row| row.bytes), Some(bytes(&Health(-5))));
    simulation = passes(&executor, simulation);
    assert!(take(&mut simulation).is_empty());
    simulation = step(&executor, simulation, TickInput::default());
    simulation = passes(&executor, simulation);
    assert!(!keys_of(&take(&mut simulation), Health::KEY).contains(&key(1)));
}

/// A server that has gathered once, fully, and a second time, incrementally.
fn gathered_twice() -> (lactose_ecs::Executor, Simulation) {
    let executor = executor(2);
    let mut simulation = server(LayoutConfig::default());
    populate(simulation.world_mut());
    for _ in 0..2 {
        simulation = step(&executor, simulation, TickInput::default());
        simulation = passes(&executor, simulation);
        let _ = take(&mut simulation);
    }
    (executor, simulation)
}

fn assert_full_with_every_row(frame: &StagingFrame, world: &World) {
    assert!(frame.full);
    assert!(frame.structural().is_empty());
    let rows: Vec<_> = frame
        .rows()
        .map(|row| ((row.key, row.component), (row.region, row.bytes.to_vec())))
        .collect();
    assert_eq!(
        rows.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
        replicated_rows(world)
    );
}

#[test]
fn a_stamp_rebase_or_a_restore_makes_the_next_frame_full() {
    let (executor, mut simulation) = gathered_twice();
    let tick = simulation.tick();
    simulation.world_mut().set_tick(Tick(tick.0 + 5)).expect("between runs");
    simulation = passes(&executor, simulation);
    let frame = take(&mut simulation);
    assert_full_with_every_row(&frame, simulation.world());
    simulation = passes(&executor, simulation);
    assert!(take(&mut simulation).is_empty());

    let snapshot = simulation.world().snapshot();
    simulation = step(&executor, simulation, TickInput::default());
    simulation.world_mut().spawn(key(500), REGION, (Position { x: 0, y: 0 },)).expect("spawn");
    simulation.world_mut().restore(&snapshot).expect("restore");
    simulation = passes(&executor, simulation);
    let frame = take(&mut simulation);
    assert_full_with_every_row(&frame, simulation.world());
    assert!(!keys_of(&frame, Position::KEY).contains(&key(500)));
}

#[test]
fn a_stale_cursor_with_a_current_mark_is_a_full_pass_that_skips_no_chunk() {
    let (executor, mut simulation) = gathered_twice();
    // A structural change the gather has not read, then the log trimmed past it: the
    // gather's cursor is stale while its mark is current.
    let world = simulation.world_mut();
    world.spawn(key(600), REGION, (Position { x: 6, y: 0 },)).expect("spawn");
    let cursor = world.structural_log().cursor();
    world.trim_structural_log(cursor);
    simulation = passes(&executor, simulation);
    let frame = take(&mut simulation);
    // Every row, including those of chunks no tick touched since the mark.
    assert_full_with_every_row(&frame, simulation.world());
    assert!(keys_of(&frame, Health::KEY).contains(&key(1)));
    simulation = passes(&executor, simulation);
    assert!(take(&mut simulation).is_empty());
}

#[test]
fn a_retention_below_the_catch_up_cap_makes_every_gather_after_a_catch_up_frame_full() {
    let run = |retention: u32| -> Vec<bool> {
        let executor = executor(2);
        let mut simulation = server_builder(LayoutConfig::default())
            .log_retention(retention)
            .build()
            .expect("build");
        populate(simulation.world_mut());
        let mut fulls = Vec::new();
        for frame in 0..6u64 {
            // A catch-up frame of three ticks, a spawn in each, then the passes.
            for n in 0..3u64 {
                let spawned = key(1000 + frame * 10 + n);
                simulation
                    .world_mut()
                    .spawn(spawned, REGION, (Position { x: 0, y: 0 },))
                    .expect("spawn");
                simulation = step(&executor, simulation, TickInput::default());
            }
            simulation = passes(&executor, simulation);
            fulls.push(take(&mut simulation).full);
        }
        fulls
    };
    assert_eq!(run(1), vec![true; 6]);
    assert_eq!(run(4), vec![true, false, false, false, false, false]);
}

/// The scene's frames, gathered at every frame but taken only at the frames `takes` names,
/// as a packer that fell behind would find them.
fn scene_taken_at(takes: &[u32]) -> Vec<(u32, StagingFrame)> {
    let executor = executor(2);
    let mut simulation = server(LayoutConfig::default());
    populate(simulation.world_mut());
    let mut frames = Vec::new();
    for index in 0..FRAMES {
        script(simulation.world_mut(), index);
        for _ in 0..=index % 3 {
            simulation = step(&executor, simulation, TickInput::default());
        }
        simulation = passes(&executor, simulation);
        if takes.contains(&index) {
            frames.push((index, take(&mut simulation)));
        }
    }
    frames
}

/// The same scene with the gather run only at the frames `gathers` names: one gather over
/// every window in between.
fn scene_gathered_at(gathers: &[u32]) -> Vec<(u32, StagingFrame)> {
    let executor = executor(2);
    let mut simulation = server(LayoutConfig::default());
    populate(simulation.world_mut());
    let mut frames = Vec::new();
    for index in 0..FRAMES {
        script(simulation.world_mut(), index);
        for _ in 0..=index % 3 {
            simulation = step(&executor, simulation, TickInput::default());
        }
        if gathers.contains(&index) {
            simulation = passes(&executor, simulation);
            frames.push((index, take(&mut simulation)));
        }
    }
    frames
}

#[test]
fn a_frame_not_taken_merges_into_what_one_gather_over_both_windows_holds() {
    // Taken at 0 (full), then after windows that spawn, despawn, move, add and remove.
    let takes = [0, 3, 5, 8, 11];
    let merged = scene_taken_at(&takes);
    let single = scene_gathered_at(&takes);
    for ((index, merged), (_, single)) in merged.iter().zip(&single) {
        assert_eq!(frame_rows(merged), frame_rows(single), "frame {index}");
        assert_eq!(merged.structural(), single.structural(), "frame {index}");
        assert_eq!(merged, single, "frame {index}");
        // One value per row, the newer.
        let pairs: BTreeSet<_> = merged.rows().map(|row| (row.key, row.component)).collect();
        assert_eq!(pairs.len(), merged.row_count());
    }
    // A full frame not taken merged with the incremental ones after it is the full frame
    // one gather would take then.
    let merged = scene_taken_at(&[2]);
    let single = scene_gathered_at(&[2]);
    assert!(merged[0].1.full);
    assert_eq!(merged, single);
}

#[test]
fn region_r_is_the_rows_of_r_in_rows() {
    let frames = scene(LayoutConfig::default(), 2);
    for frame in &frames {
        let mut total = 0;
        for region in [RegionId(0), REGION, OTHER_REGION, THIRD_REGION] {
            let slice: Vec<_> = frame.region(region).collect();
            let filtered: Vec<_> = frame.rows().filter(|row| row.region == region).collect();
            assert_eq!(slice, filtered);
            assert!(slice.windows(2).all(|pair| (pair[0].key, pair[0].component) < (pair[1].key, pair[1].component)));
            total += slice.len();
        }
        assert_eq!(total, frame.row_count());
    }
    assert!(frames[0].region(REGION).next().is_some());
    assert!(frames[0].region(THIRD_REGION).next().is_none());
}

#[test]
fn a_move_is_in_structural_and_moves_the_row_to_the_other_dirty_list() {
    let executor = executor(2);
    let mut simulation = server(LayoutConfig::default());
    let world = simulation.world_mut();
    create_regions(world, &[REGION, OTHER_REGION]);
    world.spawn(key(77), REGION, (Position { x: 1, y: 0 }, Mover(1))).expect("spawn");
    simulation = step(&executor, simulation, TickInput::default());
    simulation = passes(&executor, simulation);
    let _ = take(&mut simulation);
    simulation.world_mut().move_to_region(key(77), OTHER_REGION).expect("move");
    simulation = step(&executor, simulation, TickInput::default());
    simulation = passes(&executor, simulation);
    let frame = take(&mut simulation);
    let entries: Vec<LogEntry> = frame.structural().to_vec();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].change,
        StructuralChange::Moved { key: key(77), from: REGION, to: OTHER_REGION }
    );
    assert_eq!(frame.region(REGION).count(), 0);
    assert_eq!(frame.region(OTHER_REGION).map(|row| row.key).collect::<Vec<_>>(), vec![key(77)]);
}

/// A deterministic generator for the randomised scene.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

#[test]
fn the_gather_selects_exactly_what_the_typed_change_query_selects() {
    let mut checks = 0;
    let mut rebases = 0;
    for (index, layout) in layouts().into_iter().enumerate().step_by(3) {
        for start in [Tick(5), Tick((1 << 30) - 3), Tick(u32::MAX - 3)] {
            let executor = executor(4);
            let mut simulation = server_builder(layout).tick(start).build().expect("build");
            populate(simulation.world_mut());
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (index as u64) << 8 ^ u64::from(start.0));
            let mut next = 200;
            simulation = passes(&executor, simulation);
            let _ = take(&mut simulation);
            let mut mark = simulation.world().mark();
            for _ in 0..25 {
                let world = simulation.world_mut();
                let live: Vec<StableKey> = world
                    .columns_erased(Position::KEY)
                    .flat_map(|column| column.keys().to_vec())
                    .collect();
                for _ in 0..4 {
                    let target =
                        live[usize::try_from(rng.below(live.len() as u64)).expect("small")];
                    if !world.contains(target) {
                        continue;
                    }
                    match rng.below(6) {
                        0 => world.insert(target, Position { x: 3, y: 3 }).expect("insert"),
                        1 => world.despawn(target).expect("despawn"),
                        2 => {
                            world
                                .spawn(key(next), REGION, (Position { x: 0, y: 0 }, Mover(1)))
                                .expect("spawn");
                            next += 1;
                        }
                        3 => {
                            let to = if world.region_of(target) == Some(REGION) {
                                OTHER_REGION
                            } else {
                                REGION
                            };
                            world.move_to_region(target, to).expect("move");
                        }
                        4 => world.insert(target, Status(1)).expect("insert"),
                        _ => {}
                    }
                }
                for _ in 0..rng.below(3) {
                    simulation = step(&executor, simulation, TickInput::default());
                }
                simulation = passes(&executor, simulation);
                let frame = take(&mut simulation);
                match simulation.world().changed_since::<Position>(mark) {
                    Some(rows) => {
                        let typed: BTreeSet<StableKey> = rows.map(|(key, _)| key).collect();
                        assert!(!frame.full);
                        assert_eq!(keys_of(&frame, Position::KEY), typed);
                        checks += 1;
                    }
                    None => {
                        // The 2^30 stamp rebase started a new epoch inside the frame.
                        assert_full_with_every_row(&frame, simulation.world());
                        rebases += 1;
                    }
                }
                mark = simulation.world().mark();
            }
        }
    }
    // At least one rebase per layout, from the start just below 2^30.
    assert!(rebases >= 5, "{rebases}");
    assert_eq!(checks + rebases, 5 * 3 * 25);
}

#[test]
fn a_frame_is_built_by_hand_and_finished() {
    let mut frame = StagingFrame::new(Tick(9));
    assert!(frame.is_empty());
    frame.push_row(OTHER_REGION, key(2), Health::KEY, Tick(4), bytes(&Health(1)));
    frame.push_row(REGION, key(3), Health::KEY, Tick(4), bytes(&Health(2)));
    frame.push_row(REGION, key(1), Health::KEY, Tick(6), bytes(&Health(3)));
    // A newer value of a row already pushed, then an older one, then an equal tick.
    frame.push_row(OTHER_REGION, key(2), Health::KEY, Tick(5), bytes(&Health(4)));
    frame.push_row(OTHER_REGION, key(2), Health::KEY, Tick(3), bytes(&Health(5)));
    frame.push_row(REGION, key(3), Health::KEY, Tick(4), bytes(&Health(6)));
    frame.finish();
    let rows: Vec<_> =
        frame.rows().map(|row| (row.region, row.key, row.tick, row.bytes.to_vec())).collect();
    assert_eq!(
        rows,
        vec![
            (REGION, key(1), Tick(6), bytes(&Health(3)).to_vec()),
            (REGION, key(3), Tick(4), bytes(&Health(6)).to_vec()),
            (OTHER_REGION, key(2), Tick(5), bytes(&Health(4)).to_vec()),
        ]
    );
    let finished = frame.clone();
    frame.finish();
    assert_eq!(frame, finished);
    assert!(!frame.is_empty());
    // Across the wrap, a tick just after zero is newer than one just before it.
    let mut wrapped = StagingFrame::new(Tick(1));
    wrapped.push_row(REGION, key(1), Health::KEY, Tick(0), bytes(&Health(1)));
    wrapped.push_row(REGION, key(1), Health::KEY, Tick(u32::MAX), bytes(&Health(2)));
    wrapped.finish();
    assert_eq!(wrapped.rows().map(|row| row.tick).collect::<Vec<_>>(), vec![Tick(0)]);
    // Equal content pushed in another order compares equal once finished.
    let mut other = StagingFrame::new(Tick(9));
    other.push_row(OTHER_REGION, key(2), Health::KEY, Tick(5), bytes(&Health(4)));
    other.push_row(REGION, key(3), Health::KEY, Tick(4), bytes(&Health(6)));
    other.push_row(REGION, key(1), Health::KEY, Tick(6), bytes(&Health(3)));
    other.finish();
    assert_eq!(other, finished);
}

#[test]
fn merge_applies_the_newer_frames_structure_to_the_older_rows() {
    let row = |frame: &mut StagingFrame, region, n, component, tick, value: i32| {
        frame.push_row(region, key(n), component, Tick(tick), bytes(&Health(value)));
    };
    let mut older = StagingFrame::new(Tick(10));
    row(&mut older, REGION, 1, Health::KEY, 9, 1);
    row(&mut older, REGION, 2, Health::KEY, 9, 2);
    row(&mut older, REGION, 3, Health::KEY, 9, 3);
    row(&mut older, REGION, 3, Status::KEY, 9, 3);
    row(&mut older, REGION, 4, Health::KEY, 9, 4);
    older.push_structural(LogEntry {
        tick: Tick(9),
        change: StructuralChange::Spawned { key: key(4), region: REGION },
    });
    older.finish();
    let mut newer = StagingFrame::new(Tick(12));
    row(&mut newer, REGION, 1, Health::KEY, 11, 10);
    newer.push_structural(LogEntry {
        tick: Tick(11),
        change: StructuralChange::Despawned { key: key(2), region: REGION },
    });
    newer.push_structural(LogEntry {
        tick: Tick(11),
        change: StructuralChange::Removed { key: key(3), component: Status::KEY },
    });
    newer.push_structural(LogEntry {
        tick: Tick(12),
        change: StructuralChange::Moved { key: key(4), from: REGION, to: OTHER_REGION },
    });
    newer.finish();
    let mut merged = older.clone();
    merged.merge(newer.clone());
    assert_eq!(merged.tick, Tick(12));
    assert!(!merged.full);
    let rows: Vec<_> =
        merged.rows().map(|row| (row.region, row.key, row.component, row.tick)).collect();
    assert_eq!(
        rows,
        vec![
            (REGION, key(1), Health::KEY, Tick(11)),
            (REGION, key(3), Health::KEY, Tick(9)),
            (OTHER_REGION, key(4), Health::KEY, Tick(9)),
        ]
    );
    let mut structural = older.structural().to_vec();
    structural.extend_from_slice(newer.structural());
    assert_eq!(merged.structural(), structural.as_slice());

    // A full newer frame supersedes; a full older frame stays full with no structure.
    let mut full = StagingFrame::new(Tick(13));
    full.full = true;
    row(&mut full, REGION, 9, Health::KEY, 13, 9);
    full.finish();
    let mut superseded = older.clone();
    superseded.merge(full.clone());
    assert_eq!(superseded, full);
    let mut kept = full.clone();
    kept.merge(newer);
    assert!(kept.full);
    assert!(kept.structural().is_empty());
    assert_eq!(kept.tick, Tick(12));
    assert_eq!(kept.row_count(), 2);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a frame is read after `finish`")]
fn reading_a_frame_not_finished_is_a_debug_assertion() {
    let mut frame = StagingFrame::new(Tick(1));
    frame.push_row(REGION, key(1), Health::KEY, Tick(1), bytes(&Health(1)));
    let _ = frame.region(REGION).count();
}
