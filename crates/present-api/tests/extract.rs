//! Extract against the README's "Extract": which rows a pass considers in each kind of world,
//! when a pass is full, which rows an incremental pass writes and which it writes once more,
//! what `removed` lists, when a row carries `no_interpolate`, and that none of it depends on
//! the chunk layout or the worker count.

#![forbid(unsafe_code)]

use crate::common;

use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{ClaimedBy, LayoutConfig, WorldKind};
use lactose_math::Scalar;
use lactose_physics_api::{Teleported, Transform};
use lactose_present_api::{CameraRig, Light, MeshInstance, PresentSnapshot, RenderTransform};
use lactose_sim::harness::{ChunkCapacity, layout_mode};

const KINDS: [WorldKind; 3] = [WorldKind::Server, WorldKind::Client, WorldKind::Prediction];

fn rig(kind: WorldKind) -> Rig {
    Rig::new(simulation(kind, LayoutConfig::default(), Tick(100)), executor(2))
}

/// The keys of the rows of `snapshot` that have a `T`.
fn keys<T: lactose_reflect::Component>(snapshot: &PresentSnapshot) -> Vec<StableKey> {
    snapshot.rows::<T>().map(|(key, _)| key).collect()
}

fn light() -> Light {
    Light::point(lactose_math::Vec3::ZERO, Scalar::ONE, Scalar::ONE)
}

/// Key 1 is an instance alone, 2 a presented row alone (a light, with a transform), 3 both,
/// 4 neither (a transform and a walker); in a prediction world 1 to 4 are claimed, and 5 to
/// 8 are the same rows unclaimed. A `Server` world's full pass flags every row and is a cut;
/// a `Prediction` world's is neither; a `Client` world's writes no row, copies no transform,
/// and is a cut.
#[test]
fn each_kind_extracts_its_set() {
    for kind in KINDS {
        let mut rig = rig(kind);
        let world = rig.world_mut();
        let copies: &[u64] = if kind == WorldKind::Prediction { &[0, 4] } else { &[0] };
        for &offset in copies {
            let k = |n: u64| key(n + offset);
            world.spawn(k(1), REGION, (at(1.0), mesh(1))).unwrap();
            world.spawn(k(2), REGION, (at(2.0), light())).unwrap();
            world.spawn(k(3), REGION, (at(3.0), mesh(3), Shown(3), Teleported(Tick(0)))).unwrap();
            world.spawn(k(4), REGION, (at(4.0), Walker(Scalar::ZERO))).unwrap();
            if offset == 0 && kind == WorldKind::Prediction {
                for n in 1..=4 {
                    world.insert(k(n), claim()).unwrap();
                }
            }
        }

        let pass = rig.frame().expect("a full pass calls the sink");
        assert!(pass.pass.full, "{kind:?}");
        assert_eq!(pass.pass.kind, kind);
        assert_eq!(pass.pass.tick, Tick(100));
        assert_eq!(pass.removed, None);
        match kind {
            WorldKind::Client => assert!(pass.rows.is_empty(), "a client world writes no row"),
            _ => {
                assert_eq!(pass.keys(), vec![key(1), key(3)], "{kind:?}");
                let row = pass.row(key(3)).unwrap();
                assert_eq!(row.mesh, mesh(3).mesh);
                assert_eq!(row.transform, RenderTransform::from_sim(&at(3.0)));
                assert_eq!(pass.all_flagged(), kind == WorldKind::Server, "{kind:?}");
                assert_eq!(pass.flagged().is_empty(), kind == WorldKind::Prediction);
            }
        }

        let frame = rig.reader.acquire().expect("one publication");
        let snapshot = frame.newer();
        assert_eq!(snapshot.kind(), kind);
        assert_eq!(snapshot.tick(), Tick(100));
        assert_eq!(snapshot.cut(), kind != WorldKind::Prediction, "{kind:?}");
        assert_eq!(keys::<Light>(snapshot), vec![key(2)], "{kind:?}");
        assert_eq!(keys::<Shown>(snapshot), vec![key(3)]);
        assert_eq!(keys::<MeshInstance>(snapshot), vec![key(3)], "an instance alone is not in it");
        assert_eq!(snapshot.get::<Shown>(key(3)), Some(Shown(3)));
        assert_eq!(snapshot.get::<MeshInstance>(key(1)), None);
        if kind == WorldKind::Client {
            assert!(keys::<Transform>(snapshot).is_empty(), "a client world copies no transform");
            assert!(keys::<Teleported>(snapshot).is_empty());
            assert_eq!(frame.transform(key(3)), None);
        } else {
            assert_eq!(keys::<Transform>(snapshot), vec![key(2), key(3)], "{kind:?}");
            assert_eq!(keys::<Teleported>(snapshot), vec![key(3)]);
            assert_eq!(snapshot.get::<Transform>(key(2)), Some(at(2.0)));
        }
    }
}

/// A world of `kind` with walking instances 1 to 4, the first also a presented row.
fn walking(kind: WorldKind, retention: Option<u32>) -> Rig {
    let simulation = simulation_with(kind, LayoutConfig::default(), Tick(100), retention);
    let mut rig = Rig::new(simulation, executor(2));
    let world = rig.world_mut();
    for n in 1..=4 {
        world.spawn(key(n), REGION, (at(0.0), mesh(n), Walker(Scalar::ONE), claim())).unwrap();
    }
    world.insert(key(1), Shown(1)).unwrap();
    rig
}

/// What every full pass holds to: every instance, no `removed`, the flag and the cut by kind.
fn assert_full(rig: &mut Rig, pass: Option<PassLog>, cause: &str) {
    let kind = rig.world().kind();
    let pass = pass.unwrap_or_else(|| panic!("{kind:?} {cause}: a full pass calls the sink"));
    assert!(pass.pass.full, "{kind:?} {cause}");
    assert_eq!(pass.removed, None, "{kind:?} {cause}");
    assert_eq!(pass.keys(), (1..=4).map(key).collect::<Vec<_>>(), "{kind:?} {cause}");
    assert_eq!(pass.all_flagged(), kind == WorldKind::Server, "{kind:?} {cause}");
    assert_eq!(pass.flagged().is_empty(), kind == WorldKind::Prediction, "{kind:?} {cause}");
    let frame = rig.reader.acquire().expect("published");
    assert_eq!(frame.newer().cut(), kind == WorldKind::Server, "{kind:?} {cause}");
}

fn assert_incremental(rig: &mut Rig, pass: Option<PassLog>, cause: &str) {
    let pass = pass.unwrap_or_else(|| panic!("{cause}: the rows moved"));
    assert!(!pass.pass.full, "{cause}");
    assert_eq!(pass.keys(), (1..=4).map(key).collect::<Vec<_>>(), "{cause}");
    assert!(!rig.reader.acquire().expect("published").newer().cut(), "{cause}");
}

/// The first pass, a pass after `set_sink`, after `set_snapshot`, after a restore, after a
/// whole-world load (its `set_tick`) and after more ticks than the log retains with no pass
/// are full; a pass after a tick, or after a region unload, is not.
#[test]
fn a_pass_is_full_when_extract_cannot_tell_what_changed() {
    for kind in [WorldKind::Server, WorldKind::Prediction] {
        let mut rig = walking(kind, Some(3));
        let pass = rig.frame();
        assert_full(&mut rig, pass, "the first pass");
        let pass = rig.tick_frame();
        assert_incremental(&mut rig, pass, "a tick");

        let recording = Recording::default();
        drop(rig.state().set_sink(Some(recording.sink())));
        rig.recording = recording;
        let pass = rig.tick_frame();
        assert_full(&mut rig, pass, "set_sink");
        let pass = rig.tick_frame();
        assert_incremental(&mut rig, pass, "a tick after set_sink");

        let (writer, reader) = lactose_present_api::snapshot_channel();
        drop(rig.state().set_snapshot(Some(writer)));
        rig.reader = reader;
        let pass = rig.tick_frame();
        assert_full(&mut rig, pass, "set_snapshot");

        let saved = rig.world().snapshot();
        rig.step();
        rig.step();
        rig.world_mut().restore(&saved).unwrap();
        let pass = rig.frame();
        assert_full(&mut rig, pass, "a restore");
        let pass = rig.tick_frame();
        assert_incremental(&mut rig, pass, "a tick after a restore");

        // A whole-world load ends with `set_tick`, which starts an epoch.
        let tick = rig.tick();
        rig.world_mut().set_tick(Tick(tick.0 + 5)).unwrap();
        let pass = rig.tick_frame();
        assert_full(&mut rig, pass, "a load");

        // The log trims what it logged in ticks the retention no longer covers; with nothing
        // logged it still reaches the cursor, and the pass is rightly incremental.
        for n in 0..5 {
            rig.world_mut().spawn(key(100 + n), REGION, (at(0.0),)).unwrap();
            rig.step();
        }
        let pass = rig.frame();
        assert_full(&mut rig, pass, "more ticks than the log retains");

        // An unload is logged as despawns: the pass after it is incremental.
        rig.world_mut().spawn(key(9), OTHER_REGION, (at(9.0), mesh(9), claim())).unwrap();
        let pass = rig.tick_frame().expect("a spawn");
        assert!(!pass.pass.full);
        assert!(pass.keys().contains(&key(9)), "{kind:?}");
        rig.world_mut().unload_region(OTHER_REGION).unwrap();
        let pass = rig.tick_frame().expect("an unload");
        assert!(!pass.pass.full);
        assert_eq!(pass.removed, Some(vec![key(9)]));
    }
}

/// The retention is counted back from this pass, not from the previous one: more ticks than
/// `log_retention` with no pass, and a spawn logged only in the last of them, which the log
/// still holds, make an incremental pass (review round 1, MINOR-3).
#[test]
fn a_change_logged_inside_the_retention_after_more_ticks_than_it_keeps() {
    let simulation =
        simulation_with(WorldKind::Server, LayoutConfig::default(), Tick(100), Some(3));
    let mut rig = Rig::new(simulation, executor(2));
    for n in 1..=4 {
        let row = (at(0.0), mesh(n), Walker(Scalar::ONE));
        rig.world_mut().spawn(key(n), REGION, row).unwrap();
    }
    assert!(rig.frame().expect("the first pass").pass.full);
    for _ in 0..5 {
        rig.step();
    }
    rig.world_mut().spawn(key(100), REGION, (at(0.0), mesh(100))).unwrap();
    rig.step();
    let pass = rig.frame().expect("the rows moved");
    assert!(!pass.pass.full, "the log still reaches the previous pass's cursor");
    assert!(pass.keys().contains(&key(100)), "the spawn is written as a forced row");
}

/// By counts, in a world of ten instances that stand still unless a test moves them: a row
/// moved at one tick is written by the pass after it and once more, with the placement it
/// stopped at, by the pass after the next tick, then not again; a row edited between ticks is
/// written by the idle pass with `no_interpolate`, and once more by the next pass after a
/// tick without it; an idle pass with nothing changed and a `set_if_neq` of equal bytes call
/// nothing, and a pass after a tick in which nothing moved since the tick before calls `begin`
/// and `end` and nothing between them, so the sink's `begin`s are at extract's pass ticks.
#[test]
fn a_row_is_written_by_the_pass_after_the_tick_that_moved_it_and_once_more() {
    for kind in [WorldKind::Server, WorldKind::Prediction] {
        let mut rig = rig(kind);
        for n in 1..=10 {
            let row = (at(0.0), mesh(n), Walker(Scalar::ZERO), claim());
            rig.world_mut().spawn(key(n), REGION, row).unwrap();
        }
        rig.world_mut().insert(key(1), Shown(1)).unwrap();
        assert!(rig.frame().expect("the first pass").pass.full);
        let bracket = |pass: Option<PassLog>| {
            pass.is_some_and(|pass| {
                !pass.pass.full
                    && pass.removed.is_none()
                    && pass.rows.is_empty()
                    && pass.batches == 0
            })
        };
        let walk = |rig: &mut Rig, keys: &[u64], speed: f64| {
            for &n in keys {
                rig.world_mut().get_mut::<Walker>(key(n)).unwrap().0 = scalar(speed);
            }
        };
        if kind == WorldKind::Prediction {
            // After a full pass a prediction world's next pass after a tick writes every row.
            assert_eq!(rig.tick_frame().expect("settling").rows.len(), 10);
        }

        walk(&mut rig, &[1, 2], 1.0);
        let pass = rig.tick_frame().expect("rows 1 and 2 moved");
        walk(&mut rig, &[1, 2], 0.0);
        assert_eq!(pass.keys(), vec![key(1), key(2)], "{kind:?}");
        assert!(pass.flagged().is_empty());
        let moved = pass.row(key(1)).unwrap().transform;

        walk(&mut rig, &[3], 1.0);
        let pass = rig.tick_frame().expect("row 3 moved, 1 and 2 settle");
        walk(&mut rig, &[3], 0.0);
        assert_eq!(pass.keys(), vec![key(1), key(2), key(3)], "{kind:?}");
        assert_eq!(pass.row(key(1)).unwrap().transform, moved, "the placement it stopped at");
        assert!(pass.flagged().is_empty());

        assert_eq!(rig.tick_frame().expect("row 3 settles").keys(), vec![key(3)]);
        assert!(bracket(rig.tick_frame()), "nothing moved at this tick or the one before");
        assert_eq!(rig.frame(), None, "an idle pass with nothing changed");

        place(rig.world_mut(), key(5), 5.0);
        let pass = rig.frame().expect("an edit between ticks");
        assert_eq!(pass.keys(), vec![key(5)]);
        assert_eq!(pass.flagged(), vec![key(5)], "an idle pass flags its rows");
        assert_eq!(rig.frame(), None);
        let pass = rig.tick_frame().expect("the edited row settles");
        assert_eq!(pass.keys(), vec![key(5)]);
        assert!(pass.flagged().is_empty());
        assert!(bracket(rig.tick_frame()));

        let same = *rig.world().get::<Transform>(key(6)).unwrap();
        assert!(!rig.world_mut().get_mut::<Transform>(key(6)).unwrap().set_if_neq(same));
        assert_eq!(rig.frame(), None, "equal bytes stamp nothing");
        assert!(bracket(rig.tick_frame()));
        // A write through `get_mut` stamps whether or not the bytes changed.
        rig.world_mut().get_mut::<Transform>(key(6)).unwrap().translation.x = Scalar::ZERO;
        assert_eq!(rig.frame().expect("a stamp").keys(), vec![key(6)]);
    }
}

/// The snapshot half of the same: a pass after a tick writes one; an idle pass writes one only
/// when a presented row was stamped in an extracted column, entered or left the presented
/// rows, or lost a component the snapshot holds.
#[test]
fn an_idle_pass_writes_a_snapshot_only_when_a_presented_row_changed() {
    let mut rig = rig(WorldKind::Server);
    rig.world_mut().spawn(key(1), REGION, (at(1.0), Shown(1))).unwrap();
    rig.world_mut().spawn(key(2), REGION, (at(2.0),)).unwrap();
    rig.frame();
    let tick = rig.tick();
    let newer = |rig: &mut Rig| {
        let frame = rig.reader.acquire().expect("published");
        (frame.newer().tick(), frame.newer().rows::<Shown>().collect::<Vec<_>>())
    };
    let first = core::ptr::from_ref(rig.reader.acquire().unwrap().newer()) as usize;
    let unchanged =
        |rig: &mut Rig| core::ptr::from_ref(rig.reader.acquire().unwrap().newer()) as usize;

    rig.frame();
    assert_eq!(unchanged(&mut rig), first, "an idle pass with nothing changed writes nothing");
    place(rig.world_mut(), key(2), 3.0);
    rig.frame();
    assert_eq!(unchanged(&mut rig), first, "a row that is not presented");

    rig.world_mut().get_mut::<Shown>(key(1)).unwrap().0 = 7;
    rig.frame();
    assert_eq!(newer(&mut rig), (tick, vec![(key(1), Shown(7))]), "a stamped column");
    rig.world_mut().insert(key(2), Shown(2)).unwrap();
    rig.frame();
    assert_eq!(newer(&mut rig), (tick, vec![(key(1), Shown(7)), (key(2), Shown(2))]), "entered");
    rig.world_mut().despawn(key(1)).unwrap();
    rig.frame();
    assert_eq!(newer(&mut rig), (tick, vec![(key(2), Shown(2))]), "left");
    rig.world_mut().insert(key(2), light()).unwrap();
    rig.frame();
    rig.reader.acquire();
    rig.world_mut().remove::<Shown>(key(2)).unwrap();
    rig.frame();
    let frame = rig.reader.acquire().unwrap();
    assert_eq!(frame.newer().get::<Shown>(key(2)), None, "lost a component it holds");
    assert!(frame.newer().get::<Light>(key(2)).is_some());
    rig.tick_frame();
    assert_eq!(rig.reader.acquire().unwrap().newer().tick(), Tick(tick.0 + 1), "a tick");
}

/// `removed` lists, once and in key order, a despawn, a light's despawn, an unload's rows, a
/// lost `MeshInstance` or `Transform`, a mesh removed and inserted again (and the pass writes
/// that row), and in a prediction world a released claim; a `Client` world lists the same and
/// writes nothing.
#[test]
fn removed_lists_what_left_the_set() {
    for kind in KINDS {
        let mut rig = rig(kind);
        let world = rig.world_mut();
        for n in 1..=6 {
            world.spawn(key(n), REGION, (at(0.0), mesh(n), claim())).unwrap();
        }
        world.spawn(key(7), OTHER_REGION, (at(0.0), mesh(7), claim())).unwrap();
        world.spawn(key(8), OTHER_REGION, (at(0.0), mesh(8), claim())).unwrap();
        world.spawn(key(9), REGION, (at(0.0), light(), claim())).unwrap();
        rig.frame();
        // A prediction world's first pass after a tick writes every instance once more.
        rig.tick_frame();

        let world = rig.world_mut();
        world.despawn(key(1)).unwrap();
        world.despawn(key(9)).unwrap();
        world.unload_region(OTHER_REGION).unwrap();
        world.remove::<MeshInstance>(key(2)).unwrap();
        world.remove::<Transform>(key(3)).unwrap();
        world.remove::<MeshInstance>(key(4)).unwrap();
        world.insert(key(4), mesh(40)).unwrap();
        world.remove::<ClaimedBy>(key(5)).unwrap();
        let pass = rig.tick_frame().expect("rows left");
        let mut expected: Vec<StableKey> = [1, 2, 3, 4, 7, 8, 9].into_iter().map(key).collect();
        if kind == WorldKind::Prediction {
            expected.push(key(5));
        }
        expected.sort();
        assert_eq!(pass.removed, Some(expected), "{kind:?}");
        if kind == WorldKind::Client {
            assert!(pass.rows.is_empty());
        } else {
            assert_eq!(pass.keys(), vec![key(4)], "{kind:?}: the row that came back");
            assert_eq!(pass.row(key(4)).unwrap().mesh, mesh(40).mesh);
        }
    }
}

/// The flag: a teleport at each tick of a three-tick frame flags its row; a `Teleported`
/// from before the previous pass (a saved one) does not, nor tick zero, nor anything at the
/// first pass of a prediction world; every row of a pass at an earlier tick than the
/// previous pass's is flagged; and all of it across the tick's wrap, where tick zero, which
/// is after the covered tick by the wrapping comparison, still flags nothing.
#[test]
fn the_flag() {
    for start in [Tick(100), Tick(u32::MAX - 3)] {
        let simulation = simulation(WorldKind::Prediction, LayoutConfig::default(), start);
        let mut rig = Rig::new(simulation, executor(2));
        for n in 1..=5 {
            let row = (at(0.0), mesh(n), Walker(Scalar::ONE), claim());
            rig.world_mut().spawn(key(n), REGION, row).unwrap();
        }
        // A `Teleported` loaded from a save, and one at the world's own tick: the first pass
        // has no covered tick and flags nothing.
        teleport(rig.world_mut(), key(4), Tick(start.0.wrapping_sub(10)));
        teleport(rig.world_mut(), key(5), start);
        let pass = rig.frame().expect("the first pass");
        assert!(pass.flagged().is_empty(), "{start:?}");
        let saved = rig.world().snapshot();

        // Three ticks in one frame, a teleport at each, of rows 1, 2 and 3.
        for n in 1..=3 {
            let tick = Tick(rig.tick().0.wrapping_add(1));
            teleport(rig.world_mut(), key(n), tick);
            rig.step();
        }
        let pass = rig.frame().expect("every row moved");
        assert_eq!(pass.rows.len(), 5);
        assert_eq!(pass.flagged(), vec![key(1), key(2), key(3)], "{start:?}");

        // Two ticks in one frame (across the wrap from `u32::MAX - 3`): a teleport at the
        // second flags its row, and one at tick zero does not.
        let covered = rig.tick();
        teleport(rig.world_mut(), key(1), Tick(covered.0.wrapping_add(2)));
        teleport(rig.world_mut(), key(4), Tick(0));
        rig.step();
        rig.step();
        let pass = rig.frame().expect("rows walk");
        assert_eq!(pass.flagged(), vec![key(1)], "{start:?}");
        let pass = rig.tick_frame().expect("rows walk");
        assert!(pass.flagged().is_empty(), "{start:?}: each teleport is flagged once");

        // A pass at an earlier tick than the previous one flags every row and is a cut.
        rig.reader.acquire();
        rig.world_mut().restore(&saved).unwrap();
        let pass = rig.frame().expect("a full pass");
        assert!(pass.pass.full);
        assert_eq!(pass.rows.len(), 5);
        assert!(pass.all_flagged(), "{start:?}");
        let frame = rig.reader.acquire().unwrap();
        assert!(frame.newer().cut(), "{start:?}: an earlier tick is a cut");
        assert_eq!(frame.newer().tick(), start);
    }
}

/// What the sink got, sorted by key, and what the snapshots held, for one scripted scene.
type Outcome =
    (Vec<(Tick, bool, Option<Vec<StableKey>>, Vec<lactose_present_api::InstanceRow>)>, Vec<String>);

fn scripted(kind: WorldKind, layout: LayoutConfig, workers: u32) -> Outcome {
    let mut rig = Rig::new(simulation(kind, layout, Tick(100)), executor(workers));
    let mut passes = Vec::new();
    let mut snapshots = Vec::new();
    for frame in 0..24 {
        scene(&mut rig, frame);
        if frame % 3 != 2 {
            rig.step();
        }
        if let Some(pass) = rig.frame() {
            passes.push((pass.pass.tick, pass.pass.full, pass.removed, pass.rows));
        }
        if frame.is_multiple_of(2) {
            let frame = rig.reader.acquire().expect("published");
            for snapshot in [frame.older(), frame.newer()] {
                snapshots.push(format!(
                    "{:?} {:?} {:?} {:?} {:?} {:?}",
                    snapshot.tick(),
                    snapshot.cut(),
                    snapshot.rows::<Transform>().collect::<Vec<_>>(),
                    snapshot.rows::<Shown>().collect::<Vec<_>>(),
                    snapshot.rows::<MeshInstance>().collect::<Vec<_>>(),
                    snapshot.rows::<CameraRig>().collect::<Vec<_>>(),
                ));
            }
        }
    }
    (passes, snapshots)
}

/// Every layout capacity and chunk order of SOUR's modes, and one to four workers, give the
/// same sink rows sorted by key and the same snapshots: what extract selects is by stamp and
/// by key, never by chunk or row.
#[test]
fn layout_and_workers_change_nothing() {
    let mut layouts = vec![LayoutConfig::default()];
    for capacity in [ChunkCapacity::Rows(1), ChunkCapacity::Rows(7), ChunkCapacity::Rows(64)] {
        for seed in [1, 2] {
            layouts.push(layout_mode(capacity, seed).unwrap());
        }
    }
    layouts.push(layout_mode(ChunkCapacity::Bytes(16 * 1024), 3).unwrap());
    for kind in [WorldKind::Server, WorldKind::Prediction] {
        let reference = scripted(kind, LayoutConfig::default(), 1);
        assert!(reference.0.len() > 10 && reference.0.iter().any(|pass| !pass.3.is_empty()));
        for layout in &layouts {
            for workers in [1, 2, 4] {
                assert_eq!(scripted(kind, *layout, workers), reference, "{kind:?} {layout:?}");
            }
        }
    }
}

/// A frame of the scripted scene: rows spawn, walk, teleport, change region, lose and gain a
/// mesh, despawn, and presented rows gain a camera rig between ticks.
pub fn scene(rig: &mut Rig, frame: u32) {
    let world = rig.world_mut();
    let n = u64::from(frame);
    let live = |world: &lactose_ecs::World, k: u64| k > 0 && world.contains(key(k));
    if frame < 16 {
        let region = if frame.is_multiple_of(2) { REGION } else { OTHER_REGION };
        let row =
            (at(f64::from(frame)), mesh(n + 1), Walker(scalar(f64::from(frame % 3))), claim());
        world.spawn(key(n + 1), region, row).unwrap();
        if frame.is_multiple_of(4) {
            world.insert(key(n + 1), Shown(frame)).unwrap();
        }
    }
    if frame % 5 == 1 && live(world, n) {
        let tick = Tick(world.tick().0.wrapping_add(1));
        teleport(world, key(n), tick);
    }
    if frame % 6 == 3 && live(world, n - 2) {
        world.move_to_region(key(n - 2), REGION).unwrap();
    }
    if frame % 7 == 4 && live(world, n - 3) {
        world.remove::<MeshInstance>(key(n - 3)).unwrap();
    }
    if frame % 7 == 6 && live(world, n - 5) {
        world.insert(key(n - 5), mesh(77)).unwrap();
    }
    if frame % 8 == 5 && live(world, n - 4) {
        world.despawn(key(n - 4)).unwrap();
    }
    if frame % 4 == 2 && live(world, n - 1) {
        world.insert(key(n - 1), CameraRig::new(lactose_math::Vec3::ZERO, Scalar::ONE)).unwrap();
    }
}
