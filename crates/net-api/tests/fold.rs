//! `app`'s fold under the real sim (README, "The staged frame", the inbox and fold
//! bullets; proof row "The fold"): the frames due at the tick the next step runs are
//! folded into its `TickInput` and popped by its hook, region changes fold per region in
//! frame order, a refused frame enters nothing, and a full frame reloads exactly the
//! regions it names. Review probes d1, d3 and s3 are here as tests of the real code.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use lactose_core::{ErrorKind, Tick};
use lactose_ecs::RegionId;
use lactose_net_api::{RegionChange, StagedFrame, StagedInbox, StagedSpawn};
use lactose_sim::{Simulation, TickInput, TimeScale};

fn spawn_in(region: RegionId, n: u32) -> StagedSpawn {
    StagedSpawn {
        origin: None,
        region,
        slot: Some(slot(n)),
        components: vec![data(Position { x: i32::try_from(n).expect("small"), y: 0 })],
    }
}

fn relevant(region: RegionId, store: &Store) -> RegionChange {
    let baseline = store.baselines.values().next().expect("one baseline");
    RegionChange::Relevant(relevance(region, baseline))
}

fn regions(simulation: &Simulation) -> BTreeSet<RegionId> {
    simulation.world().regions().collect()
}

/// What the probe of review round 2 ran: the join's full frame for 10 into a world built
/// at 10, then a delta for 12 making `OTHER_REGION` relevant and spawning into it, each
/// pushed when it arrives and the fold asked for `simulation.tick() + offset`.
fn join_then_delta(offset: u32) -> (Simulation, Skipped) {
    let executor = executor(2);
    let store = Store::with(&baseline(vec![]));
    let (mut simulation, skipped) = client(Tick(10));
    let mut full = StagedFrame::resync(Tick(10));
    full.regions.insert(REGION, relevant(REGION, &store));
    full.spawns.insert(net(1), spawn_in(REGION, 1));
    let mut pending = vec![full];
    for _ in 0..4 {
        let frames: Vec<StagedFrame> = std::mem::take(&mut pending);
        for frame in frames {
            assert!(inbox(&mut simulation).push(Arc::new(frame)));
        }
        let through = Tick(simulation.tick().0 + offset);
        let loaded: Vec<RegionId> = simulation.world().regions().collect();
        let mut input = TickInput::default();
        for frame in simulation.world().service::<StagedInbox>().expect("setup").due(through) {
            fold_frame(&mut input, frame, &loaded, &store).expect("the fold");
        }
        simulation = step(&executor, simulation, input);
        if simulation.tick() == Tick(11) {
            // The stream's frame for 12 arrives in time for the step for 12.
            let mut delta = StagedFrame::new(Tick(12));
            delta.regions.insert(OTHER_REGION, relevant(OTHER_REGION, &store));
            delta.spawns.insert(net(2), spawn_in(OTHER_REGION, 2));
            pending.push(delta);
        }
    }
    (simulation, skipped)
}

#[test]
fn a_frame_for_the_tick_the_next_step_runs_is_folded_and_popped_in_that_step() {
    let (simulation, skipped) = join_then_delta(1);
    assert_eq!(simulation.tick(), Tick(14));
    assert_eq!(regions(&simulation), BTreeSet::from([REGION, OTHER_REGION]));
    assert!(simulation.world().contains(key(1)));
    assert!(simulation.world().contains(key(2)));
    assert!(skipped.lock().expect("never poisoned").is_empty());

    // The step for 12 alone: the region is loaded and the entity live after it.
    let executor = executor(2);
    let store = Store::with(&baseline(vec![]));
    let (mut simulation, _) = client(Tick(10));
    let mut full = StagedFrame::resync(Tick(10));
    full.regions.insert(REGION, relevant(REGION, &store));
    let (input, _) = between_runs(&mut simulation, [full], &store);
    simulation = step(&executor, simulation, input);
    let mut delta = StagedFrame::new(Tick(12));
    delta.regions.insert(OTHER_REGION, relevant(OTHER_REGION, &store));
    delta.spawns.insert(net(2), spawn_in(OTHER_REGION, 2));
    let (input, accepted) = between_runs(&mut simulation, [delta], &store);
    assert_eq!(accepted, vec![true]);
    simulation = step(&executor, simulation, input);
    assert_eq!(simulation.tick(), Tick(12));
    assert!(regions(&simulation).contains(&OTHER_REGION));
    assert!(simulation.world().contains(key(2)));
    assert!(simulation.world().service::<StagedInbox>().expect("setup").is_empty());
}

#[test]
fn folding_due_of_the_current_tick_instead_loses_the_region_loads() {
    // What the README warns against: the hook pops the frame for 12 in the step for 12,
    // but `due(11)` before that step does not show it, so its region never loads and the
    // spawn into it fails.
    let (simulation, skipped) = join_then_delta(0);
    assert!(!regions(&simulation).contains(&OTHER_REGION));
    assert!(!simulation.world().contains(key(2)));
    let skipped = skipped.lock().expect("never poisoned").clone();
    assert!(skipped.contains(&("spawn", key(2), ErrorKind::NotFound)), "{skipped:?}");
}

/// A client at 12 that has loaded `REGION` from the join's frame, with `key(1)` in it, and
/// stepped on without frames, so that the next frames all fall due in one step.
fn joined() -> (Simulation, Store) {
    let executor = executor(2);
    let store = Store::with(&baseline(vec![]));
    let (mut simulation, _) = client(Tick(10));
    let mut full = StagedFrame::resync(Tick(10));
    full.regions.insert(REGION, relevant(REGION, &store));
    full.spawns.insert(net(1), spawn_in(REGION, 1));
    let (input, _) = between_runs(&mut simulation, [full], &store);
    simulation = step(&executor, simulation, input);
    simulation = step(&executor, simulation, TickInput::default());
    assert_eq!(simulation.tick(), Tick(12));
    (simulation, store)
}

fn frame_with(tick: u32, changes: Vec<(RegionId, RegionChange)>) -> StagedFrame {
    let mut frame = StagedFrame::new(Tick(tick));
    frame.regions.extend(changes);
    frame
}

#[test]
fn relevant_then_irrelevant_in_one_step_leaves_the_region_unloaded() {
    // Review probe s3, through the fold rule.
    let (mut simulation, store) = joined();
    // Both due at the step for 14: the client fell a tick behind.
    let executor = executor(2);
    simulation = step(&executor, simulation, TickInput::default());
    let first = frame_with(13, vec![(OTHER_REGION, relevant(OTHER_REGION, &store))]);
    let second = frame_with(14, vec![(OTHER_REGION, RegionChange::Irrelevant)]);
    let (input, accepted) = between_runs(&mut simulation, [first, second], &store);
    assert_eq!(accepted, vec![true, true]);
    assert!(input.loaded.is_empty());
    assert_eq!(input.unloaded, BTreeSet::from([OTHER_REGION]));
    simulation = step(&executor, simulation, input);
    assert_eq!(regions(&simulation), BTreeSet::from([REGION]));
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
}

#[test]
fn irrelevant_then_relevant_in_one_step_unloads_and_reloads_the_region() {
    let (mut simulation, store) = joined();
    let executor = executor(2);
    simulation = step(&executor, simulation, TickInput::default());
    let first = frame_with(13, vec![(REGION, RegionChange::Irrelevant)]);
    let second = frame_with(14, vec![(REGION, relevant(REGION, &store))]);
    let (input, accepted) = between_runs(&mut simulation, [first, second], &store);
    assert_eq!(accepted, vec![true, true]);
    assert_eq!(input.unloaded, BTreeSet::from([REGION]));
    assert_eq!(input.loaded.keys().copied().collect::<Vec<_>>(), vec![REGION]);
    simulation = step(&executor, simulation, input);
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
    // Reloaded from its message: the entity the stream spawned went with the unload.
    assert_eq!(regions(&simulation), BTreeSet::from([REGION]));
    assert!(!simulation.world().contains(key(1)));
}

#[test]
fn a_refused_frame_enters_nothing() {
    let (mut simulation, store) = joined();
    let accepted_first = frame_with(13, vec![]);
    let mut refused = frame_with(13, vec![(THIRD_REGION, relevant(THIRD_REGION, &store))]);
    refused.time_scale = Some(TimeScale::PAUSED);
    refused.spawns.insert(net(3), spawn_in(THIRD_REGION, 3));
    let (input, accepted) = between_runs(&mut simulation, [accepted_first, refused], &store);
    assert_eq!(accepted, vec![true, false]);
    assert!(input.loaded.is_empty());
    assert_eq!(input.time_scale, None);
    let executor = executor(2);
    simulation = step(&executor, simulation, input);
    assert!(!regions(&simulation).contains(&THIRD_REGION));
    assert!(!simulation.world().contains(key(3)));
}

#[test]
fn a_full_frame_unloads_every_loaded_region_and_reloads_the_relevant_ones() {
    let (mut simulation, store) = joined();
    let executor = executor(2);
    let mut delta = frame_with(13, vec![(OTHER_REGION, relevant(OTHER_REGION, &store))]);
    delta.spawns.insert(net(2), spawn_in(OTHER_REGION, 2));
    let (input, _) = between_runs(&mut simulation, [delta], &store);
    simulation = step(&executor, simulation, input);
    assert_eq!(regions(&simulation), BTreeSet::from([REGION, OTHER_REGION]));

    let mut full = StagedFrame::resync(Tick(14));
    full.regions.insert(OTHER_REGION, relevant(OTHER_REGION, &store));
    full.regions.insert(THIRD_REGION, relevant(THIRD_REGION, &store));
    full.spawns.insert(net(2), spawn_in(OTHER_REGION, 5));
    full.spawns.insert(net(3), spawn_in(THIRD_REGION, 6));
    full.time_scale = Some(TimeScale::REAL_TIME);
    let (input, _) = between_runs(&mut simulation, [full], &store);
    assert_eq!(input.unloaded, BTreeSet::from([REGION, OTHER_REGION]));
    assert_eq!(input.time_scale, Some(TimeScale::REAL_TIME));
    simulation = step(&executor, simulation, input);
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
    assert_eq!(regions(&simulation), BTreeSet::from([OTHER_REGION, THIRD_REGION]));
    assert!(!simulation.world().contains(key(1)));
    assert!(simulation.world().contains(key(2)));
    assert!(simulation.world().contains(key(3)));
}

#[test]
fn a_delta_then_a_full_frame_due_in_one_step_load_exactly_the_full_frames_regions() {
    // Review probe d3: two frames arrive late; a delta making `THIRD_REGION` relevant,
    // then a full frame in which only `REGION` is.
    let (mut simulation, store) = joined();
    let executor = executor(2);
    simulation = step(&executor, simulation, TickInput::default());
    let delta = frame_with(13, vec![(THIRD_REGION, relevant(THIRD_REGION, &store))]);
    let mut full = StagedFrame::resync(Tick(14));
    full.regions.insert(REGION, relevant(REGION, &store));
    full.spawns.insert(net(1), spawn_in(REGION, 1));
    let (input, accepted) = between_runs(&mut simulation, [delta, full], &store);
    assert_eq!(accepted, vec![true, true]);
    assert_eq!(input.loaded.keys().copied().collect::<Vec<_>>(), vec![REGION]);
    simulation = step(&executor, simulation, input);
    assert_eq!(regions(&simulation), BTreeSet::from([REGION]));
    assert!(simulation.world().contains(key(1)));
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
}

#[test]
fn the_time_scale_is_the_last_one_among_the_due_frames() {
    let (mut simulation, store) = joined();
    let executor = executor(2);
    simulation = step(&executor, simulation, TickInput::default());
    simulation = step(&executor, simulation, TickInput::default());
    let mut a = StagedFrame::new(Tick(13));
    a.time_scale = Some(TimeScale::PAUSED);
    let mut b = StagedFrame::new(Tick(14));
    b.time_scale = Some(TimeScale { thousandths: 500 });
    let c = StagedFrame::new(Tick(15));
    let (input, _) = between_runs(&mut simulation, [a, b, c], &store);
    assert_eq!(input.time_scale, Some(TimeScale { thousandths: 500 }));
}
