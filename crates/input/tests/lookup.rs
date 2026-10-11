//! What the lookup costs (README, "The applier": `player_entity` "costs the player's
//! claims, allocates nothing, takes a shared world"). Asserted on what does not depend on
//! the machine: the answer among 100,000 other rows, and the allocations the lookup and the
//! applier make, counted by the tagged global allocator this test binary installs. The time
//! is measured by an ignored test, run by hand, and reported, never asserted.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{AllocTag, PlayerId, StableKey, TaggedAlloc, Tick};
use lactose_ecs::{ClaimedBy, LayoutConfig};
use lactose_input::{PlayerInput, PlayerInputApplier, player_entity};
use lactose_sim::{InputApplier, Simulation, StreamingAnchor};

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

const PLAYERS: u32 = 64;

/// A server world with `rows` unclaimed rows of two archetypes, `PLAYERS` players with one
/// anchored row each, and `TWICE`'s three rows.
fn populated(rows: u64) -> Simulation {
    let mut simulation = build_server(LayoutConfig::default());
    let world = simulation.world_mut();
    for n in 0..rows {
        let key = key(1_000 + n);
        if n % 2 == 0 {
            world.spawn(key, REGION, (Seen::default(),)).unwrap();
        } else {
            world.spawn(key, REGION, (Seen::default(), PlayerInput::IDLE)).unwrap();
        }
    }
    for player in 0..PLAYERS {
        let claim = ClaimedBy { player: PlayerId(100 + player) };
        let row = (PlayerInput::IDLE, Seen::default(), claim, StreamingAnchor { priority: 0 });
        world.spawn(key(500 + u64::from(player)), REGION, row).unwrap();
    }
    simulation
}

fn allocations_during(f: impl FnOnce()) -> u64 {
    let _scope = AllocTag::Tools.scope();
    let before = ALLOCATOR.stats(AllocTag::Tools).total_allocations;
    f();
    ALLOCATOR.stats(AllocTag::Tools).total_allocations - before
}

#[test]
fn the_lookup_answers_among_many_rows_and_allocates_nothing() {
    for rows in [0, 1_000, 100_000] {
        let simulation = populated(rows);
        let world = simulation.world();
        let mut found: Vec<Option<StableKey>> = Vec::with_capacity(PLAYERS as usize + 2);
        let allocations = allocations_during(|| {
            for player in 0..PLAYERS {
                found.push(player_entity(world, PlayerId(100 + player)));
            }
            found.push(player_entity(world, TWICE));
            found.push(player_entity(world, STRANGER));
        });
        assert_eq!(allocations, 0, "{rows} rows");
        let expected: Vec<Option<StableKey>> = (0..PLAYERS)
            .map(|player| Some(key(500 + u64::from(player))))
            .chain([Some(key(LESSER)), None])
            .collect();
        assert_eq!(found, expected, "{rows} rows");
    }
}

/// Applying a sample to a row that holds the component is a decode, the lookup and the
/// ECS's `World::insert`: the applier allocates exactly what the same inserts made
/// directly allocate (the ECS's own erased copy), whatever the world's size.
#[test]
fn the_applier_allocates_only_what_the_insert_does() {
    let mut simulation = populated(100_000);
    let world = simulation.world_mut();
    let mut applier = PlayerInputApplier::new();
    let samples: Vec<PlayerInput> =
        (0..PLAYERS).map(|player| client_sample(PlayerId(100 + player), 9)).collect();
    let entries: Vec<_> = samples.iter().map(|sample| sample.to_player_sample(Tick(9))).collect();
    let applied = allocations_during(|| {
        for (player, entry) in (0..PLAYERS).zip(&entries) {
            applier.apply(world, PlayerId(100 + player), entry).unwrap();
        }
    });
    for (player, sample) in (0..PLAYERS).zip(&samples) {
        assert_eq!(world.get::<PlayerInput>(key(500 + u64::from(player))), Some(sample));
    }
    let inserted = allocations_during(|| {
        for (player, sample) in (0..PLAYERS).zip(&samples) {
            world.insert(key(500 + u64::from(player)), *sample).unwrap();
        }
    });
    assert_eq!(applied, inserted);
}

/// The lookup's time for 64 players among 1,000 and 100,000 other rows, printed for the
/// report. Run with `cargo nextest run -p lactose-input --run-ignored only --cargo-profile
/// release --no-capture`.
#[test]
#[ignore = "a measurement, run by hand: timings are reported, not asserted"]
#[expect(clippy::disallowed_types, reason = "test code measuring the lookup, outside the step")]
fn the_lookup_time_against_the_world_size() {
    use std::time::Instant;
    const ROUNDS: u32 = 2_000;
    for rows in [1_000, 100_000] {
        let simulation = populated(rows);
        let world = simulation.world();
        let start = Instant::now();
        let mut hits = 0u32;
        for _ in 0..ROUNDS {
            for player in 0..PLAYERS {
                hits += u32::from(player_entity(world, PlayerId(100 + player)).is_some());
            }
        }
        let elapsed = start.elapsed();
        assert_eq!(hits, ROUNDS * PLAYERS);
        let per_lookup = elapsed.as_nanos() / u128::from(ROUNDS * PLAYERS);
        println!("{rows:>7} other rows: {per_lookup} ns per lookup ({ROUNDS} rounds of {PLAYERS})");
    }
}
