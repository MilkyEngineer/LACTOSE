//! Corrections: a frame's facts compared at their tick, the world restored there, the
//! facts written stamped, and every later tick re-simulated, with the result equal to a
//! run that had the server's rows from that tick on; each fact of the one entity it names,
//! and an entity it writes left with the transient components its frame states (D141).

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeSet;

use common::*;
use lactose_bundle::BaselineRow;
use lactose_core::{ComponentKey, EntityRef, NetId, StableKey, Tick};
use lactose_ecs::{LayoutConfig, OwnedBy, RegionId, StructuralChange, World};
use lactose_input::PlayerInput;
use lactose_net_api::{StagedFrame, Tombstone, TombstoneCause};
use lactose_predict::{CorrectionCause, Divergence};
use lactose_reflect::Reflect;
use lactose_sim::StagedState;
use lactose_sim::harness::{ChunkCapacity, layout_mode};

fn layouts() -> Vec<(LayoutConfig, u32)> {
    vec![
        (LayoutConfig::default(), 1),
        (layout_mode(ChunkCapacity::Rows(1), 3).expect("a layout"), 4),
        (layout_mode(ChunkCapacity::Rows(2), 7).expect("a layout"), 2),
    ]
}

#[test]
fn a_differing_row_is_corrected_at_its_tick_and_the_rest_resimulated() {
    for (layout, workers) in layouts() {
        let mut rig = Rig::joined_with(layout, workers);
        let mut reference = Rig::joined_with(layout, workers);
        rig.step_to(20, walking);
        // The player walked one a tick from tick 12: four at tick 15.
        reference.step_to(15, walking);
        assert_eq!(reference.position(me()), Some(at(4, 0)));
        reference.world_mut().insert(me(), at(100, 0)).unwrap();
        reference.step_to(21, walking);

        let mut frame = StagedFrame::new(Tick(15));
        exact(&mut frame, me(), &at(100, 0));
        rig.push(frame);
        rig.step(walking(21));
        let reconciled = rig.reconcile();

        assert_eq!(
            summaries(&reconciled),
            vec![(
                Tick(15),
                6,
                CorrectionCause::Mismatch,
                vec![me()],
                Divergence { entities: 1, fields: 1 }
            )]
        );
        assert_eq!((reconciled.confirmed, reconciled.frames), (Some(Tick(15)), 1));
        assert_eq!(reconciled.resimulated, 6);
        assert_eq!(reconciled.resync, None);
        assert_eq!(rig.tick(), Tick(21));
        assert_eq!(rig.position(me()), Some(at(106, 0)));
        assert_eq!(rig.hash(), reference.hash());
        // And they stay equal.
        rig.step_to(25, walking);
        reference.step_to(25, walking);
        assert_eq!(rig.hash(), reference.hash());

        let stats = rig.p().stats();
        assert_eq!(stats.corrections, 1);
        assert_eq!((stats.mismatched_entities, stats.mismatched_fields), (1, 1));
        assert_eq!(stats.deepest_correction, 6);
        assert_eq!(stats.ticks_resimulated, 6);
        assert_eq!(stats.ticks_predicted, 15);
        assert_eq!(stats.frames_reconciled, 2);
    }
}

#[test]
fn an_agreeing_frame_corrects_nothing() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let hash = rig.hash();
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(4, 0));
    exact(&mut frame, cart(), &at(50, 0));
    exact(&mut frame, cart(), &Health(10));
    rig.push(frame);
    rig.step(walking(21));
    let after = rig.hash();
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert_eq!(reconciled.resimulated, 0);
    assert_eq!(reconciled.confirmed, Some(Tick(15)));
    assert_ne!(hash, after);
    assert_eq!(rig.hash(), after);
    assert_eq!(rig.p().stats().ticks_resimulated, 0);
}

#[test]
fn a_fact_is_compared_at_its_own_tick_only() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    // At tick 16 the player was at five: a row saying five at tick 15 is a misprediction
    // there, whatever the other slots hold.
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(5, 0));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(reconciled.corrections[0].tick, Tick(15));
    assert_eq!(rig.position(me()), Some(at(11, 0)));
}

#[test]
fn a_correction_is_stamped_so_a_derived_cache_follows_it() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let before = *rig.world().get::<Trail>(cart()).unwrap();
    assert_eq!(before.x, 50);
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, cart(), &at(77, 0));
    exact(&mut frame, cart(), &Health(10));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections[0].entities, vec![cart()]);
    let after = *rig.world().get::<Trail>(cart()).unwrap();
    assert_eq!(after.x, 77);
    assert_eq!(after.updates, before.updates + 1);
}

#[test]
fn two_frames_with_facts_correct_at_the_older_and_again_at_the_newer() {
    for (layout, workers) in layouts() {
        let mut rig = Rig::joined_with(layout, workers);
        let mut reference = Rig::joined_with(layout, workers);
        rig.step_to(20, walking);
        reference.step_to(15, walking);
        reference.world_mut().insert(me(), at(100, 0)).unwrap();
        reference.world_mut().insert(cart(), Health(3)).unwrap();
        reference.step_to(17, walking);
        reference.world_mut().insert(me(), at(200, 0)).unwrap();
        reference.world_mut().insert(cart(), at(9, 9)).unwrap();
        reference.step_to(21, walking);

        let mut first = StagedFrame::new(Tick(15));
        exact(&mut first, me(), &at(100, 0));
        // Exact rows state the whole entity: the cart's position as predicted beside it.
        exact(&mut first, cart(), &at(50, 0));
        exact(&mut first, cart(), &Health(3));
        rig.push(first);
        let mut second = StagedFrame::new(Tick(17));
        // The player's x agrees with the re-simulation from 15 (102) only by chance in
        // neither field: both differ.
        exact(&mut second, me(), &at(200, 0));
        exact(&mut second, cart(), &at(9, 9));
        exact(&mut second, cart(), &Health(3));
        rig.push(second);
        rig.step(walking(21));
        let reconciled = rig.reconcile();

        assert_eq!(
            summaries(&reconciled),
            vec![
                (
                    Tick(15),
                    6,
                    CorrectionCause::Mismatch,
                    vec![me(), cart()],
                    Divergence { entities: 2, fields: 2 }
                ),
                (
                    Tick(17),
                    4,
                    CorrectionCause::Mismatch,
                    vec![me(), cart()],
                    Divergence { entities: 2, fields: 3 }
                ),
            ]
        );
        assert_eq!(reconciled.resimulated, 6);
        assert_eq!(reconciled.frames, 2);
        assert_eq!(reconciled.confirmed, Some(Tick(17)));
        assert_eq!(rig.hash(), reference.hash());
        assert_eq!(rig.p().stats().ticks_resimulated, 6);
        assert_eq!(rig.p().stats().deepest_correction, 6);
    }
}

#[test]
fn an_entity_despawned_too_early_is_respawned_in_its_last_known_region() {
    let mut rig = Rig::joined();
    rig.step_to(13, walking);
    // The prediction world loses the cart (a prediction system's mistake, here a direct
    // write): every slot from 13 lacks it.
    rig.world_mut().despawn(cart()).unwrap();
    rig.step_to(20, walking);
    assert!(!rig.world().contains(cart()));
    let mut frame = StagedFrame::new(Tick(16));
    exact(&mut frame, cart(), &at(50, 0));
    exact(&mut frame, cart(), &Health(4));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(
        summaries(&reconciled),
        vec![(
            Tick(16),
            5,
            CorrectionCause::Mismatch,
            vec![cart()],
            Divergence { entities: 1, fields: 0 }
        )]
    );
    let world = rig.world();
    assert_eq!(world.region_of(cart()), Some(REGION));
    assert_eq!(world.get::<Health>(cart()), Some(&Health(4)));
    assert_eq!(claim(world, cart()), Some(PLAYER));
}

#[test]
fn a_fact_in_a_region_neither_its_slot_nor_the_popped_tick_holds_is_skipped() {
    // No step loads `UNLOADED`: the replicated world's hook refuses the same spawn.
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    frame.spawns.insert(net(4), claimed(UNLOADED, vec![data(at(1, 1))]));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty());
    assert!(!rig.world().contains(key(4)));
}

#[test]
fn a_missing_component_entity_region_or_claim_is_a_mismatch_with_no_field() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    // A component the player's row lacks, beside its position as predicted.
    exact(&mut frame, me(), &at(4, 0));
    exact(&mut frame, me(), &Health(7));
    // An entity the slot lacks, in a loaded region.
    frame.spawns.insert(net(4), claimed(OTHER_REGION, vec![data(at(1, 1))]));
    // A region.
    frame.moves.insert(cart(), OTHER_REGION);
    // A claim: the server claims the proxy for the local player.
    frame.spawns.insert(net(3), claimed(REGION, vec![data(at(-50, 0))]));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(
        summaries(&reconciled),
        vec![(
            Tick(15),
            6,
            CorrectionCause::Mismatch,
            vec![me(), cart(), proxy(), key(4)],
            Divergence { entities: 4, fields: 0 }
        )]
    );
    let world = rig.world();
    assert_eq!(world.get::<Health>(me()), Some(&Health(7)));
    assert_eq!(world.region_of(key(4)), Some(OTHER_REGION));
    assert_eq!(claim(world, key(4)), Some(PLAYER));
    assert_eq!(world.region_of(cart()), Some(OTHER_REGION));
    assert_eq!(claim(world, proxy()), Some(PLAYER));
}

#[test]
fn a_release_a_removal_and_a_tombstone_are_facts_that_correct() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    // The server releases the cart: a spawn with a slot of a live claimed entity.
    frame.spawns.insert(net(2), remote(REGION, 1, vec![data(at(50, 0)), data(Health(10))]));
    // And removes the player's position.
    frame.removed.insert((me(), Position::KEY));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(reconciled.corrections[0].entities, vec![me(), cart()]);
    assert_eq!(reconciled.corrections[0].divergence, Divergence { entities: 2, fields: 0 });
    let world = rig.world();
    assert_eq!(claim(world, cart()), None);
    assert_eq!(world.get::<Position>(me()), None);

    // A tombstone of a claimed entity: despawned at its tick and kept so.
    let mut frame = StagedFrame::new(Tick(18));
    frame.despawns.insert(
        me(),
        lactose_net_api::Tombstone {
            region: REGION,
            tick: Tick(18),
            cause: lactose_net_api::TombstoneCause::Destroyed,
        },
    );
    rig.push(frame);
    rig.step(walking(22));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(reconciled.corrections[0].tick, Tick(18));
    assert!(!rig.world().contains(me()));
}

#[test]
fn facts_are_written_in_frame_order_under_every_layout() {
    // Two frames popped in one step with facts about the same row at the same tick
    // cannot exist (a frame's tick ascends), but within a frame the hook records spawns,
    // moves, removals, rows and tombstones in that order: a removal then a row of the same
    // component leaves the row.
    let mut hashes = Vec::new();
    for (layout, workers) in layouts() {
        let mut rig = Rig::joined_with(layout, workers);
        rig.step_to(20, walking);
        let mut frame = StagedFrame::new(Tick(15));
        frame.removed.insert((cart(), Health::KEY));
        exact(&mut frame, cart(), &at(50, 0));
        exact(&mut frame, cart(), &Health(5));
        frame.moves.insert(cart(), OTHER_REGION);
        rig.push(frame);
        rig.step(walking(21));
        let reconciled = rig.reconcile();
        assert_eq!(reconciled.corrections.len(), 1);
        assert_eq!(rig.world().get::<Health>(cart()), Some(&Health(5)));
        assert_eq!(rig.world().region_of(cart()), Some(OTHER_REGION));
        hashes.push(rig.hash());
    }
    assert!(hashes.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn a_rollback_that_fails_part_way_counts_the_corrections_it_made() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(100, 0));
    rig.push(frame);
    rig.step(walking(21));
    // A hook inserted over the predictor's: the first re-simulated tick fails after the
    // correction at 15 was written.
    rig.world_mut().insert_service::<dyn StagedState>(Box::new(Inert));
    assert!(rig.try_reconcile().is_err());
    let stats = rig.p().stats();
    // The join's frame and this one.
    assert_eq!((stats.corrections, stats.frames_reconciled), (1, 2));
    assert_eq!((stats.mismatched_entities, stats.mismatched_fields), (1, 1));
    assert_eq!(stats.deepest_correction, 6);
}

/// A hook that does nothing.
struct Inert;

impl StagedState for Inert {
    fn apply(&mut self, _: &mut lactose_ecs::World, _: Tick) -> lactose_core::Result<()> {
        Ok(())
    }
}

// ---- One entity per fact, and what a row built from the stream has (D141) ----

/// A region no other test here uses: a document's.
const DOCUMENT: RegionId = RegionId(6);

fn owner() -> StableKey {
    StableKey::Net(NetId::cooked(80))
}

fn item() -> StableKey {
    StableKey::Net(NetId::cooked(81))
}

/// A region document placing, in [`DOCUMENT`], an owner and an item it owns, then a frame
/// claiming the owner for the local player, applied by the hook at 14.
fn claimed_owner() -> Rig {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    let owner_row =
        BaselineRow { id: NetId::cooked(80), components: vec![data(at(1, 1)), data(Health(1))] };
    let owned = OwnedBy { owner: EntityRef::new(owner()) };
    let components = vec![data(at(2, 2)), data(Health(2)), data(Diary(5)), data(owned)];
    let item_row = BaselineRow { id: NetId::cooked(81), components };
    place_document(rig.world_mut(), DOCUMENT, vec![owner_row, item_row]);
    rig.step(walking(13));
    let mut frame = StagedFrame::new(Tick(13));
    frame
        .spawns
        .insert(NetId::cooked(80), claimed(DOCUMENT, vec![data(at(1, 1)), data(Health(1))]));
    rig.push(frame);
    rig.step(walking(14));
    rig.reconcile();
    assert_eq!(claim(rig.world(), owner()), Some(PLAYER));
    rig
}

/// The item as the document gave it.
fn item_state(world: &World) -> Option<(RegionId, Position, Health, Diary)> {
    Some((
        world.region_of(item())?,
        *world.get::<Position>(item())?,
        *world.get::<Health>(item())?,
        *world.get::<Diary>(item())?,
    ))
}

/// A claimed owner with an item it owns by a document's edge, which the server released
/// long ago. A tombstone fact of the owner, and in a second run a move fact: the correction
/// despawns, or moves, the owner alone, and the item is where the document put it after
/// the re-simulation.
#[test]
fn a_correction_takes_the_one_entity_its_fact_names() {
    for tombstone in [true, false] {
        let mut rig = claimed_owner();
        rig.step_to(20, walking);
        let mut frame = StagedFrame::new(Tick(16));
        if tombstone {
            let destroyed =
                Tombstone { region: DOCUMENT, tick: Tick(16), cause: TombstoneCause::Destroyed };
            frame.despawns.insert(owner(), destroyed);
        } else {
            frame.moves.insert(owner(), OTHER_REGION);
        }
        rig.push(frame);
        rig.step(walking(21));
        let reconciled = rig.reconcile();
        assert_eq!(summaries(&reconciled).len(), 1, "{:?}", summaries(&reconciled));
        assert_eq!(reconciled.corrections[0].tick, Tick(16));
        assert_eq!(reconciled.corrections[0].entities, vec![owner()]);
        let world = rig.world();
        let owner_region = if tombstone { None } else { Some(OTHER_REGION) };
        assert_eq!(world.region_of(owner()), owner_region, "tombstone {tombstone}");
        assert_eq!(item_state(world), Some((DOCUMENT, at(2, 2), Health(2), Diary(5))));
    }
}

/// `key`'s transient components: `Aim`, with its value, and whether it has `Stance` and
/// `Crouch`.
fn transients(world: &World, key: StableKey) -> (Option<Aim>, bool, bool) {
    (
        world.get::<Aim>(key).copied(),
        world.get::<Stance>(key).is_some(),
        world.get::<Crouch>(key).is_some(),
    )
}

/// The cart, which the prediction world despawned too early, re-spawned by a frame's exact
/// rows, `Aim` among them, with the frame's statement naming `Stance`: both present,
/// zeroed, and `test::brace`, which names both, runs on it in every re-simulated tick.
#[test]
fn a_respawn_from_exact_rows_carries_its_transient_components() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    rig.world_mut().despawn_alone(cart()).unwrap();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(16));
    exact(&mut frame, cart(), &at(50, 0));
    exact(&mut frame, cart(), &Health(4));
    exact(&mut frame, cart(), &Aim(9));
    frame.transient.insert(cart(), [Stance::KEY].into());
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1, "{:?}", summaries(&reconciled));
    assert_eq!(reconciled.corrections[0].tick, Tick(16));
    let world = rig.world();
    assert_eq!(world.get::<Health>(cart()), Some(&Health(4)));
    assert_eq!(transients(world, cart()), (Some(Aim(0)), true, false));
    assert_eq!(reconciled.resimulated, 5);
    assert_eq!(braced(world, cart()), 5, "once in each re-simulated tick");
}

/// Every addition and removal of `Stance`, `Crouch` and `Aim` on `key` the log of `world`
/// holds, in its order.
fn presence_log(world: &World, key: StableKey) -> Vec<(ComponentKey, bool)> {
    let ruled = [Stance::KEY, Crouch::KEY, Aim::KEY];
    let log = world.structural_log();
    let entries = log.since(&mut log.cursor_at_start()).expect("a cursor at the start");
    entries
        .filter_map(|entry| match entry.change {
            StructuralChange::Added { key: of, component } if of == key => Some((component, true)),
            StructuralChange::Removed { key: of, component } if of == key => {
                Some((component, false))
            }
            _ => None,
        })
        .filter(|(component, _)| ruled.contains(component))
        .collect()
}

/// The bytes of the local player's `PlayerInput`.
fn input_bytes(world: &World) -> Vec<u8> {
    world.get_erased(me(), PlayerInput::KEY).expect("the applier writes it").to_vec()
}

/// The local player's entity given `Crouch` and `Aim` at 15 (a prediction system's
/// mistake, written directly), then a frame at 18 whose exact rows differ from what was
/// predicted, popped at 21, and stating `statement` beside them. Returns the rig after the
/// reconcile, the presence changes the correction made and the bytes of `PlayerInput`.
fn stated_at_18(
    layout: LayoutConfig,
    workers: u32,
    statement: Option<BTreeSet<ComponentKey>>,
) -> (Rig, Vec<(ComponentKey, bool)>, Vec<u8>) {
    let mut rig = Rig::joined_with(layout, workers);
    rig.step_to(15, walking);
    rig.world_mut().insert(me(), Crouch(3)).unwrap();
    rig.world_mut().insert(me(), Aim(4)).unwrap();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(18));
    exact(&mut frame, me(), &at(100, 0));
    if let Some(statement) = statement {
        frame.transient.insert(me(), statement);
    }
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1, "{:?}", summaries(&reconciled));
    assert_eq!(reconciled.corrections[0].tick, Tick(18));
    let log = presence_log(rig.world(), me());
    let bytes = input_bytes(rig.world());
    (rig, log, bytes)
}

/// A statement names presence, never a value, and is exact: the local player's
/// `PlayerInput`, written by the applier before the hook, has the same bytes after a full
/// frame's upsert and after a correction whose statement names it as in a run whose frames
/// state nothing; an unreplicated transient component the statement does not name is
/// removed and one named and absent inserted zeroed, in ascending key order, under every
/// layout; with no statement none is removed; a replicated one the exact rows do not name
/// is removed either way; a key that is not a transient unreplicated component is skipped;
/// and a release leaves every transient component as it is.
#[test]
fn a_statement_is_exact_and_writes_no_value() {
    let named = || -> BTreeSet<ComponentKey> {
        // `Health` is persistent and `Aim` replicated: neither a statement's to name.
        [Stance::KEY, PlayerInput::KEY, Health::KEY, Aim::KEY].into()
    };
    let mut expected = vec![(Crouch::KEY, false), (Stance::KEY, true), (Aim::KEY, false)];
    expected.sort_by_key(|&(component, _)| component);
    let mut hashes = Vec::new();
    for (layout, workers) in layouts() {
        let (rig, log, bytes) = stated_at_18(layout, workers, Some(named()));
        let (_, _, unstated) = stated_at_18(layout, workers, None);
        assert_eq!(log, expected, "the corrected slot's changes, then none re-simulated");
        assert_eq!(bytes, unstated, "the statement wrote no value of `PlayerInput`");
        assert_ne!(bytes, vec![0; bytes.len()], "a sample of a walk, not zeroed");
        let world = rig.world();
        assert_eq!(transients(world, me()), (None, true, false));
        assert_eq!(world.get::<Health>(me()), None, "a persistent key is skipped");
        hashes.push(rig.hash());
    }
    assert!(hashes.windows(2).all(|pair| pair[0] == pair[1]));

    // No statement: the unreplicated one stays, the replicated one the rows do not name
    // goes.
    let (rig, log, _) = stated_at_18(LayoutConfig::default(), 1, None);
    assert_eq!(log, vec![(Aim::KEY, false)]);
    assert_eq!(transients(rig.world(), me()), (None, false, true));

    // A full frame's upsert of the local player with a statement naming `PlayerInput`.
    let full = |stated: bool| {
        let mut rig = Rig::joined();
        rig.step_to(20, walking);
        let mut frame = join_frame(Tick(20));
        if stated {
            frame.transient.insert(me(), [PlayerInput::KEY].into());
        }
        rig.push(frame);
        rig.step(walking(21));
        input_bytes(rig.world())
    };
    assert_eq!(full(true), full(false));

    // A release leaves the cart's transient components.
    let mut rig = Rig::joined();
    rig.step_to(15, walking);
    rig.world_mut().insert(cart(), Stance(1)).unwrap();
    rig.world_mut().insert(cart(), Crouch(1)).unwrap();
    rig.world_mut().insert(cart(), Aim(1)).unwrap();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(17));
    frame.spawns.insert(net(2), remote(REGION, 1, vec![data(at(50, 0)), data(Health(10))]));
    frame.transient.insert(cart(), BTreeSet::new());
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections.len(), 1);
    assert_eq!(claim(rig.world(), cart()), None);
    assert_eq!(transients(rig.world(), cart()), (Some(Aim(0)), true, true));
}

/// A frame whose statement differs from the cart only in presence makes no correction and
/// re-simulates nothing, nor does any later delta frame that differs only so: a
/// `[server]`-only presence change is not followed until a full frame, or a correction
/// something recorded causes. The next frame with a component that differs makes one,
/// after which the cart has the stated component; and in a second run a full frame alone,
/// with nothing recorded differing, leaves the cart with it.
#[test]
fn a_difference_in_presence_alone_is_no_correction_point() {
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    for (tick, health) in [(16, 10), (17, 10), (18, 3)] {
        let mut frame = StagedFrame::new(Tick(tick));
        exact(&mut frame, cart(), &at(50, 0));
        exact(&mut frame, cart(), &Health(health));
        frame.transient.insert(cart(), [Stance::KEY].into());
        rig.push(frame);
        rig.step(walking(tick + 5));
        let reconciled = rig.reconcile();
        if health == 10 {
            assert!(reconciled.corrections.is_empty(), "{:?}", summaries(&reconciled));
            assert_eq!(reconciled.resimulated, 0);
            assert_eq!(transients(rig.world(), cart()), (None, false, false));
        } else {
            assert_eq!(summaries(&reconciled).len(), 1, "{:?}", summaries(&reconciled));
            assert_eq!(reconciled.corrections[0].tick, Tick(18));
            assert_eq!(transients(rig.world(), cart()), (None, true, false));
        }
    }
    assert_eq!(rig.p().stats().corrections, 1);

    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = join_frame(Tick(20));
    frame.transient.insert(cart(), [Stance::KEY].into());
    rig.push(frame);
    rig.step(walking(21));
    assert_eq!(transients(rig.world(), cart()), (None, true, false));
}

/// The test game adds `Trail` with its `Without<Trail>` kernel, from `Position`: the cart a
/// correction re-spawns at the present has no `Trail` in the corrected slot and has it,
/// equal to its corrected position, after the next tick; the `Changed<Position>` cache
/// follows the server's later moves (`tests/check.rs` holds that the check passes it).
#[test]
fn a_derived_component_is_added_by_its_system() {
    let mut rig = Rig::joined();
    rig.step_to(12, walking);
    assert_eq!(rig.world().get::<Trail>(cart()).map(|trail| trail.x), Some(50));
    rig.world_mut().despawn_alone(cart()).unwrap();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(21));
    exact(&mut frame, cart(), &at(60, 0));
    exact(&mut frame, cart(), &Health(4));
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1, "{:?}", summaries(&reconciled));
    assert_eq!((reconciled.corrections[0].tick, reconciled.resimulated), (Tick(21), 0));
    assert_eq!(rig.position(cart()), Some(at(60, 0)));
    assert_eq!(rig.world().get::<Trail>(cart()), None, "never sent, not yet added");
    rig.step(walking(22));
    assert_eq!(rig.world().get::<Trail>(cart()).map(|trail| trail.x), Some(60));

    let mut frame = StagedFrame::new(Tick(22));
    exact(&mut frame, cart(), &at(70, 0));
    exact(&mut frame, cart(), &Health(4));
    rig.push(frame);
    rig.step(walking(23));
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections[0].tick, Tick(22));
    assert_eq!(rig.world().get::<Trail>(cart()).map(|trail| trail.x), Some(70));
}
