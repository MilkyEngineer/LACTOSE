//! The ownership forest (README, Regions, "Ownership is a forest"): a self-owner and the
//! closing write of a two- and a three-entity cycle refused through every direct path with
//! nothing changed, dropped and logged at apply with the rest of the buffer applied, refused
//! whole by `load_region` within one document and across two, and refused at `bind`; a
//! chain through a dangling owner accepted and the write that closes it later refused; an
//! owned entity staying put when given to an owner elsewhere and when the owner's region
//! unloads, exported and reloaded in either order. Every case that touches storage runs
//! under every layout.

#![forbid(unsafe_code)]

mod common;

use std::sync::{Mutex, Once};

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, Overlay, Record};
use lactose_core::{EntityRef, ErrorKind, NetId, PredictionKey, Result, StableKey};
use lactose_ecs::{
    ChangeMark, Commands, LayoutConfig, OwnedBy, RegionConfig, RegionMode, Schedule, StableKeyOf,
    StructuralChange, World, WorldBuilder, WorldKind, system,
};
use lactose_reflect::Reflect;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

fn owned_by(owner: StableKey) -> OwnedBy {
    OwnedBy { owner: EntityRef::new(owner) }
}

fn owner_of(world: &World, key: StableKey) -> Option<StableKey> {
    world.get::<OwnedBy>(key).and_then(|value| value.owner.get())
}

fn data<T: Reflect>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: bytemuck::bytes_of(&value).to_vec() }
}

fn prediction() -> PredictionKey {
    PredictionKey { player: PLAYER, input_tick: t(4), ordinal: 0xABCD }
}

fn predicted() -> StableKey {
    StableKey::Predicted(prediction())
}

fn net_50() -> NetId {
    NetId::from_bits(50).unwrap()
}

/// The server's key of [`predicted`], once bound.
fn bound() -> StableKey {
    StableKey::Net(net_50())
}

fn cooked(hash: u64) -> NetId {
    NetId::cooked(hash)
}

fn key(id: NetId) -> StableKey {
    StableKey::Net(id)
}

/// The keys every test here names.
fn keys() -> Vec<StableKey> {
    let cooked_keys = [0xA1, 0xA2, 0xA6, 0xB1, 0x5].map(|hash| key(cooked(hash)));
    [net(1), net(2), net(3), net(5), predicted(), bound()].into_iter().chain(cooked_keys).collect()
}

/// What [`world_state`] observes of the keys these tests name.
fn state(world: &World, marks: &[ChangeMark]) -> WorldState {
    world_state(world, marks, &keys(), &[PLAYER])
}

fn assert_refused(result: Result<()>, context: &str) {
    let error = result.expect_err(context);
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{context}: {error}");
    assert!(error.message().contains("cycle of owners"), "{context}: {error}");
}

/// The direct paths that write `OwnedBy`.
#[derive(Clone, Copy, Debug)]
enum Path {
    Insert,
    InsertErased,
    Spawn,
    SpawnErased,
}

const PATHS: [Path; 4] = [Path::Insert, Path::InsertErased, Path::Spawn, Path::SpawnErased];

impl Path {
    fn spawns(self) -> bool {
        matches!(self, Self::Spawn | Self::SpawnErased)
    }

    /// Writes `owner` to the live `key`, or spawns `key` with it.
    fn write(self, world: &mut World, key: StableKey, owner: StableKey) -> Result<()> {
        let value = owned_by(owner);
        match self {
            Self::Insert => world.insert(key, value),
            Self::InsertErased => world.insert_erased(key, &data(value)).map(drop),
            Self::Spawn => world.spawn(key, REGION, (pos(1, 1), value)),
            Self::SpawnErased => world.spawn_erased(key, REGION, &[data(pos(1, 1)), data(value)]),
        }
    }
}

#[test]
fn a_self_owner_is_refused() {
    for layout in layouts() {
        for path in PATHS {
            let mut world = world(WorldKind::Prediction, layout);
            let start = world.mark();
            world.spawn(net(1), REGION, (pos(0, 0),)).unwrap();
            // A bind kept for an entity not yet live, so the spawn paths meet the bound key.
            world.bind(prediction(), net_50()).unwrap();
            let cases = if path.spawns() {
                vec![(net(2), net(2)), (predicted(), predicted()), (predicted(), bound())]
            } else {
                world.spawn(predicted(), REGION, (pos(4, 4),)).unwrap();
                vec![(net(1), net(1)), (predicted(), predicted()), (predicted(), bound())]
                    .into_iter()
                    .chain([(bound(), predicted()), (bound(), bound())])
                    .collect()
            };
            let marks = [start, world.mark()];
            let before = state(&world, &marks);
            for (key, owner) in cases {
                let context = format!("{layout:?} {path:?} {key:?} owned by {owner:?}");
                assert_refused(path.write(&mut world, key, owner), &context);
                assert_eq!(state(&world, &marks), before, "{context}");
            }
        }
    }
}

#[test]
fn closing_a_cycle_is_refused_on_the_last_write() {
    for layout in layouts() {
        for path in PATHS {
            for len in [2, 3] {
                for through_bound in [false, true] {
                    let context =
                        format!("{layout:?} {path:?} {len} through_bound={through_bound}");
                    let mut world = world(WorldKind::Prediction, layout);
                    let start = world.mark();
                    // The cycle's entities by the key they are spawned under, and by the key
                    // the writes name them with: the predicted one by its bound `Net` key.
                    let mut members: Vec<StableKey> = (1..=len).map(net).collect();
                    let mut names = members.clone();
                    if through_bound {
                        world.bind(prediction(), net_50()).unwrap();
                        members[0] = predicted();
                        names[0] = bound();
                    }
                    let last = len as usize - 1;
                    if path.spawns() {
                        // The last member's owner is the first, not yet live; each spawn
                        // after it is owned by the one spawned before, and the first's spawn
                        // closes the cycle.
                        path.write(&mut world, members[last], names[0]).expect(&context);
                        for i in (1..last).rev() {
                            path.write(&mut world, members[i], names[i + 1]).expect(&context);
                        }
                    } else {
                        for &member in &members {
                            world.spawn(member, REGION, (pos(0, 0),)).unwrap();
                        }
                        for i in 0..last {
                            path.write(&mut world, names[i], names[i + 1]).expect(&context);
                        }
                    }
                    let marks = [start, world.mark()];
                    let before = state(&world, &marks);
                    let closing = if path.spawns() {
                        path.write(&mut world, members[0], names[1])
                    } else {
                        path.write(&mut world, names[last], names[0])
                    };
                    assert_refused(closing, &context);
                    assert_eq!(state(&world, &marks), before, "{context}");
                }
            }
        }
    }
}

// ---- Command apply ----

/// Every `casein` event whose message says something was dropped, its fields as
/// `name=value`. Installed once: Miri runs every test of the binary in one process.
static DROPPED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Drops;

struct Fields(Vec<String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push(format!("{}={value}", field.name()));
    }
}

impl Subscriber for Drops {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() == Level::DEBUG && metadata.target() == "casein"
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields(Vec::new());
        event.record(&mut fields);
        let line = fields.0.join(" ");
        if line.contains("cycle of owners") {
            DROPPED.lock().expect("the log").push(line);
        }
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

static SUBSCRIBER: Once = Once::new();

/// The first `NetId` the server allocates in [`a_command_closing_a_cycle_is_dropped`].
const FIRST_ALLOCATED: u64 = 100;

/// One buffer, from the entity with `Counter`, whose dangling owner is the key the
/// buffer's first spawn will receive.
#[system(name = "ownership::commands", worlds = [server])]
fn ownership_commands(cmd: Commands, key: StableKeyOf, _source: &Counter) {
    let source = *key;
    cmd.set(net(1), owned_by(net(2)));
    cmd.set(net(2), owned_by(net(1)));
    cmd.set(net(1), Health(1));
    // Receives the source's owner as its key, so its own owner closes the cycle.
    cmd.spawn((pos(5, 5), owned_by(source)));
    // Its own placeholder: an owner of itself.
    cmd.spawn((pos(6, 6), owned_by(StableKey::Placeholder(1))));
    let kept = cmd.spawn((pos(7, 7),));
    cmd.set(kept, owned_by(source));
    let other = cmd.spawn((pos(8, 8),));
    cmd.set(other, owned_by(other));
    cmd.set(source, Health(2));
}

#[test]
fn a_command_closing_a_cycle_is_dropped() {
    SUBSCRIBER.call_once(|| {
        tracing::subscriber::set_global_default(Drops).expect("the only subscriber");
    });
    let executor = executor();
    for layout in layouts() {
        let first = core::num::NonZeroU64::new(FIRST_ALLOCATED).unwrap();
        let mut world = WorldBuilder::new(WorldKind::Server, registry())
            .layout(layout)
            .next_net_id(first)
            .build()
            .unwrap();
        world
            .create_region(RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated })
            .unwrap();
        let source = net(9);
        world.spawn(net(1), REGION, (pos(1, 1),)).unwrap();
        world.spawn(net(2), REGION, (pos(2, 2),)).unwrap();
        world.spawn(source, REGION, (Counter(0), owned_by(net(FIRST_ALLOCATED)))).unwrap();
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("simulation", &[]).add(&OWNERSHIP_COMMANDS);
        let mut schedule = builder.build().unwrap();
        DROPPED.lock().expect("the log").clear();

        world = tick(&mut schedule, world, &executor);

        let context = format!("{layout:?}");
        assert_eq!(owner_of(&world, net(1)), Some(net(2)), "{context}: the first set applied");
        assert_eq!(owner_of(&world, net(2)), None, "{context}: the closing set dropped");
        assert_eq!(world.get::<Health>(net(1)), Some(&Health(1)), "{context}");
        assert_eq!(world.get::<Health>(source), Some(&Health(2)), "{context}");
        assert_eq!(owner_of(&world, source), Some(net(FIRST_ALLOCATED)), "{context}");
        // Each spawn allocated its key in buffer order; the two that would close a cycle
        // were dropped, the others and the sets on them applied.
        assert!(!world.contains(net(100)) && !world.contains(net(101)), "{context}");
        assert_eq!(world.get::<Position>(net(102)), Some(&pos(7, 7)), "{context}");
        assert_eq!(owner_of(&world, net(102)), Some(source), "{context}");
        assert_eq!(world.get::<Position>(net(103)), Some(&pos(8, 8)), "{context}");
        assert_eq!(owner_of(&world, net(103)), None, "{context}");
        let owned: Vec<_> = world.owned_by(source).collect();
        assert_eq!(owned, [net(102)], "{context}");

        let dropped = DROPPED.lock().expect("the log").clone();
        let sets = dropped.iter().filter(|line| line.contains("message=a set dropped")).count();
        let spawns = dropped.iter().filter(|line| line.contains("message=a spawn dropped")).count();
        assert_eq!((sets, spawns, dropped.len()), (2, 2, 4), "{context}: {dropped:?}");
    }
}

// ---- Dangling owners ----

/// A region document of cooked rows, each `(hash, owner in its ecs::owned_by column)`, and
/// the overlay records over it.
fn document(rows: &[(u64, Option<StableKey>)], records: Vec<Record>) -> (Baseline, Overlay) {
    let registry = registry();
    let rows = rows.iter().map(|&(hash, owner)| {
        let mut components = vec![data(pos(hash as i32, 0))];
        components.extend(owner.map(|owner| data(owned_by(owner))));
        BaselineRow { id: cooked(hash), components }
    });
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(&registry), rows).unwrap();
    let overlay = Overlay::new(baseline.content_hash(), schemas(&registry), records).unwrap();
    (baseline, overlay)
}

fn create(world: &mut World, region: lactose_ecs::RegionId) {
    world.create_region(RegionConfig { id: region, seed: 2, mode: RegionMode::Simulated }).unwrap();
}

#[test]
fn a_chain_through_a_dangling_owner() {
    let item = net(1);
    let dangling = net(5);
    for layout in layouts() {
        for path in PATHS {
            let context = format!("{layout:?} {path:?}");
            let mut world = world(WorldKind::Server, layout);
            world.spawn(item, REGION, (pos(1, 1),)).unwrap();
            world.insert(item, owned_by(dangling)).expect(&context);
            assert_eq!(owner_of(&world, item), Some(dangling), "{context}: the reference is kept");
            if !path.spawns() {
                world.spawn(dangling, REGION, (pos(5, 5),)).unwrap();
            }
            let marks = [world.mark()];
            let before = state(&world, &marks);
            assert_refused(path.write(&mut world, dangling, item), &context);
            assert_eq!(state(&world, &marks), before, "{context}");
        }

        // The same closing edge in a document: its row is owned by an item live in another
        // region, which the row owns. Through the baseline's column and through a record.
        let x = cooked(0x5);
        let column = document(&[(0x5, Some(item))], vec![]);
        let record = document(
            &[(0x5, None)],
            vec![Record::OwnershipChange { id: x, owner: NetId::from_bits(1) }],
        );
        for (name, (baseline, overlay)) in [("column", column), ("record", record)] {
            let context = format!("{layout:?} {name}");
            let mut world = world(WorldKind::Server, layout);
            create(&mut world, OTHER_REGION);
            world.spawn(item, OTHER_REGION, (pos(1, 1), owned_by(key(x)))).unwrap();
            let marks = [world.mark()];
            let before = state(&world, &marks);
            let error = world.load_region(REGION, &baseline, Some(&overlay)).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{context}: {error}");
            assert!(error.message().contains("cycle of owners"), "{context}: {error}");
            assert_eq!(world.entities_in(REGION).count(), 0, "{context}");
            assert_eq!(state(&world, &marks), before, "{context}");
        }

        // The control: the row without an owner loads, and the item's reference finds it.
        let (baseline, overlay) = document(&[(0x5, None)], vec![]);
        let mut world = world(WorldKind::Server, layout);
        create(&mut world, OTHER_REGION);
        world.spawn(item, OTHER_REGION, (pos(1, 1), owned_by(key(x)))).unwrap();
        world.load_region(REGION, &baseline, Some(&overlay)).unwrap();
        assert_eq!(world.owned_by(key(x)).collect::<Vec<_>>(), [item], "{layout:?}");
    }
}

#[test]
fn a_load_whose_owners_cycle_is_refused_whole() {
    let (c1, c2) = (cooked(0xA1), cooked(0xA2));
    for layout in layouts() {
        // Within one document: a cycle the bundle cannot see, through the baseline's own
        // column, and one in the baseline alone.
        let through_column = document(
            &[(0xA1, Some(key(c2))), (0xA2, None)],
            vec![Record::OwnershipChange { id: c2, owner: Some(c1) }],
        );
        let baseline_alone = document(&[(0xA1, Some(key(c2))), (0xA2, Some(key(c1)))], vec![]);
        for (name, (baseline, overlay)) in
            [("through the column", through_column), ("the baseline alone", baseline_alone)]
        {
            let context = format!("{layout:?} {name}");
            let mut world = world(WorldKind::Server, layout);
            let marks = [world.mark()];
            let before = state(&world, &marks);
            let error = world.load_region(REGION, &baseline, Some(&overlay)).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{context}: {error}");
            assert_eq!(world.entities_in(REGION).count(), 0, "{context}");
            assert_eq!(state(&world, &marks), before, "{context}");
        }

        // The control: the record clears the column's owner, so no cycle.
        let (baseline, overlay) = document(
            &[(0xA1, Some(key(c2))), (0xA2, None)],
            vec![
                Record::OwnershipChange { id: c1, owner: None },
                Record::OwnershipChange { id: c2, owner: Some(c1) },
            ],
        );
        let mut world = world(WorldKind::Server, layout);
        world.load_region(REGION, &baseline, Some(&overlay)).unwrap();
        assert_eq!(owner_of(&world, key(c2)), Some(key(c1)), "{layout:?}");
        assert_eq!(owner_of(&world, key(c1)), None, "{layout:?}");

        // Across two documents: A's row owned by B's, B's by A's. Whichever loads second is
        // refused and the world stays as the first load left it.
        let b1 = cooked(0xB1);
        let a = (REGION, document(&[(0xA1, Some(key(b1)))], vec![]));
        let b = (OTHER_REGION, document(&[(0xB1, Some(key(c1)))], vec![]));
        for [first, second] in [[&a, &b], [&b, &a]] {
            let (first, (first_baseline, first_overlay)) = first;
            let (second, (second_baseline, second_overlay)) = second;
            let context = format!("{layout:?} {first:?} then {second:?}");
            let mut world = common::world(WorldKind::Server, layout);
            create(&mut world, OTHER_REGION);
            world.load_region(*first, first_baseline, Some(first_overlay)).expect(&context);
            assert_eq!(world.entities_in(*first).count(), 1, "{context}");
            let marks = [world.mark()];
            let before = state(&world, &marks);
            let error =
                world.load_region(*second, second_baseline, Some(second_overlay)).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{context}: {error}");
            assert_eq!(world.entities_in(*second).count(), 0, "{context}");
            assert_eq!(state(&world, &marks), before, "{context}");
        }
    }
}

#[test]
fn a_bind_that_would_close_a_cycle_is_refused() {
    for layout in layouts() {
        // The predicted entity's chain is 1, 2 or 3 long and ends at an entity owned by the
        // `NetId` the bind names.
        for len in 1..=3 {
            let context = format!("{layout:?} chain of {len}");
            let mut world = world(WorldKind::Prediction, layout);
            let chain: Vec<StableKey> =
                [predicted()].into_iter().chain((2..=len).map(net)).collect();
            for (i, &member) in chain.iter().enumerate().rev() {
                let owner = chain.get(i + 1).copied().unwrap_or(bound());
                world.spawn(member, REGION, (pos(0, 0), owned_by(owner))).unwrap();
            }
            let end = *chain.last().unwrap();
            let marks = [world.mark()];
            let before = state(&world, &marks);
            assert_refused(world.bind(prediction(), net_50()), &context);
            assert_eq!(world.alias(prediction()), None, "{context}");
            assert_eq!(owner_of(&world, end), Some(bound()), "{context}: the reference unchanged");
            assert_eq!(state(&world, &marks), before, "{context}");

            // Another `NetId` closes nothing.
            let other = NetId::from_bits(51).unwrap();
            world.bind(prediction(), other).expect(&context);
            assert_eq!(world.alias(prediction()), Some(other), "{context}");
        }
    }
}

#[test]
fn an_owned_entity_stays_where_it_is() {
    let (a2, a6, b1) = (cooked(0xA2), cooked(0xA6), cooked(0xB1));
    let (a, b) = (REGION, OTHER_REGION);
    for layout in layouts() {
        let context = format!("{layout:?}");
        let (baseline_a, empty_a) = document(&[(0xA2, None), (0xA6, None)], vec![]);
        let (baseline_b, empty_b) = document(&[(0xB1, None)], vec![]);
        let mut world = world(WorldKind::Server, layout);
        create(&mut world, b);
        world.load_region(a, &baseline_a, Some(&empty_a)).unwrap();
        world.load_region(b, &baseline_b, Some(&empty_b)).unwrap();
        world.move_to_region(key(a2), b).unwrap();

        let mut cursor = world.structural_log().cursor();
        world.insert(key(a6), owned_by(key(a2))).expect(&context);
        assert_eq!(world.region_of(key(a6)), Some(a), "{context}: the owned entity stayed");
        let logged: Vec<_> =
            world.structural_log().since(&mut cursor).unwrap().map(|entry| entry.change).collect();
        assert!(
            !logged.iter().any(|change| matches!(change, StructuralChange::Moved { .. })),
            "{context}: {logged:?}"
        );

        let export_a = world.export_region(a).unwrap();
        let export_b = world.export_region(b).unwrap();
        let ids: Vec<NetId> = export_a.rows.iter().map(|row| row.id).collect();
        assert_eq!(ids, [a6], "{context}: A's export lacks the departed owner");
        assert_eq!(export_a.rows[0].owner, Some(a2), "{context}");
        let ids: Vec<NetId> = export_b.rows.iter().map(|row| row.id).collect();
        assert_eq!(ids, [a2, b1], "{context}: the owner resides in B");

        // The owner's region unloads; what it owns stays, and so does the reference.
        world.unload_region(b).unwrap();
        assert_eq!(world.export_region(a).unwrap(), export_a, "{context}");
        assert_eq!(owner_of(&world, key(a6)), Some(key(a2)), "{context}");
        assert!(world.get::<Position>(key(a2)).is_none(), "{context}");

        // The documents a save writes, loaded back in either region order.
        let overlay_a = Overlay::new(
            baseline_a.content_hash(),
            schemas(&registry()),
            vec![Record::Destroy { id: a2 }, Record::OwnershipChange { id: a6, owner: Some(a2) }],
        )
        .unwrap();
        let overlay_b = Overlay::new(
            baseline_b.content_hash(),
            schemas(&registry()),
            vec![Record::SpawnFromBaseline { id: a2, components: vec![data(pos(0xA2, 0))] }],
        )
        .unwrap();
        for order in [[a, b], [b, a]] {
            let context = format!("{layout:?} {order:?}");
            let mut world = common::world(WorldKind::Server, layout);
            create(&mut world, b);
            for region in order {
                let (baseline, overlay) =
                    if region == a { (&baseline_a, &overlay_a) } else { (&baseline_b, &overlay_b) };
                world.load_region(region, baseline, Some(overlay)).expect(&context);
            }
            assert_eq!(world.export_region(a).unwrap(), export_a, "{context}");
            assert_eq!(world.export_region(b).unwrap(), export_b, "{context}");
            assert_eq!(world.owned_by(key(a2)).collect::<Vec<_>>(), [key(a6)], "{context}");
            assert_eq!(world.region_of(key(a2)), Some(b), "{context}");
            assert_eq!(world.region_of(key(a6)), Some(a), "{context}");
        }
    }
}

/// The one state the rule does not guard (README, "Ownership is a forest"): a revived
/// entity whose owner is a `Net` key bound since the snapshot to an entity whose chain
/// reaches it. The debug assertion names it.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "the restore closed a cycle of owners")]
fn a_restore_that_revives_a_cycle_is_named() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    let owner = net(10);
    world.spawn(owner, REGION, (pos(0, 0), owned_by(bound()))).unwrap();
    world.spawn(predicted(), REGION, (pos(1, 1), owned_by(owner))).unwrap();
    let snapshot = world.snapshot();
    world.despawn(owner).unwrap();
    assert!(!world.contains(predicted()));
    world.bind(prediction(), net_50()).unwrap();
    let _ = world.restore(&snapshot);
}
