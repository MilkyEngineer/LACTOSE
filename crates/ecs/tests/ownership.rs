//! The ownership forest (README, Regions, "Ownership is a forest"): a self-owner and the
//! closing write of a two- and a three-entity cycle refused through every direct path with
//! nothing changed, dropped and logged at apply with the rest of the buffer applied, a
//! refused predicted spawn binding nothing; a chain through a dangling owner accepted and
//! the write that closes it later refused; `load_region` refusing a cycle within one
//! document and a row whose bound id lacks its origin, and breaking a cycle through live
//! entities of other regions at the document row; `bind`, `restore` and the despawn of a
//! bound `Net` entity emptying the reference whose rewrite would close a cycle; a region's
//! own save and a client's documents loading after a cycle closed across regions; an owned
//! entity staying put when given to an owner elsewhere and when the owner's region unloads;
//! seeded random histories that always save and load; a deep chain of owners loading as
//! fast as a flat one. Every case that touches storage runs under every layout; the
//! histories are spread over them.
//!
//! The world logs through one global subscriber into one buffer for the process, and a
//! command apply logs on the executor's threads, so the log cannot be told apart by thread.
//! Every test therefore holds [`serial`] for its whole body: under plain `cargo test`, which
//! runs a binary's tests on parallel threads, they would otherwise read each other's lines.

#![forbid(unsafe_code)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, Once};

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, Column, ComponentData, Overlay, Record};
use lactose_core::{EntityRef, ErrorKind, NetId, PredictionKey, Result, StableKey};
use lactose_ecs::{
    ChangeMark, ClaimedBy, Commands, LayoutConfig, OwnedBy, RegionConfig, RegionId, RegionMode,
    RegionRows, Schedule, StableKeyOf, StructuralChange, With, World, WorldBuilder, WorldKind,
    system,
};
use lactose_reflect::Reflect;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

const THIRD_REGION: RegionId = RegionId(30);

fn owned_by(owner: StableKey) -> OwnedBy {
    OwnedBy { owner: EntityRef::new(owner) }
}

/// Whether `key` has an `OwnedBy` whose reference is empty, as a key merge leaves it.
fn emptied(world: &World, key: StableKey) -> bool {
    world.get::<OwnedBy>(key).is_some_and(|value| value.owner.get().is_none())
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

fn create(world: &mut World, region: RegionId) {
    world.create_region(RegionConfig { id: region, seed: 2, mode: RegionMode::Simulated }).unwrap();
}

/// Panics if some live entity's chain of owners returns to it.
fn assert_forest(world: &World, context: &str) {
    let live: Vec<StableKey> = [REGION, OTHER_REGION, THIRD_REGION]
        .into_iter()
        .filter(|&region| world.region(region).is_some())
        .flat_map(|region| world.entities_in(region))
        .collect();
    for &start in &live {
        let mut at = start;
        for _ in 0..=live.len() {
            let Some(owner) = owner_of(world, at).and_then(|owner| world.origin(owner)) else {
                break;
            };
            assert_ne!(owner, start, "{context}: a cycle of owners through {start:?}");
            at = owner;
        }
    }
}

// ---- What the world logs ----

/// One event at target `casein`, at `DEBUG` or `WARN`.
#[derive(Debug)]
struct Line {
    level: Level,
    fields: Vec<(&'static str, String)>,
}

impl Line {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(field, _)| *field == name).map(|(_, value)| value.as_str())
    }

    /// Whether the field `name` is `key`.
    fn names(&self, name: &str, key: StableKey) -> bool {
        self.field(name) == Some(format!("{key:?}").as_str())
    }

    fn says(&self, text: &str) -> bool {
        self.fields.iter().any(|(_, value)| value.contains(text))
    }
}

/// What the world logged at target `casein` since the last [`capture`]. One buffer for the
/// process, since a command apply logs on the executor's threads; [`serial`] keeps two tests
/// from sharing it.
static LINES: Mutex<Vec<Line>> = Mutex::new(Vec::new());

struct Capture;

struct Fields(Vec<(&'static str, String)>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        self.0.push((field.name(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push((field.name(), value.to_owned()));
    }
}

impl Subscriber for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "casein" && matches!(*metadata.level(), Level::DEBUG | Level::WARN)
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields(Vec::new());
        event.record(&mut fields);
        let line = Line { level: *event.metadata().level(), fields: fields.0 };
        LINES.lock().expect("the log").push(line);
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

static SUBSCRIBER: Once = Once::new();

/// Held for a whole test: the log buffer is one for the process, so tests that run on
/// threads of one process (plain `cargo test`) must not interleave.
static SERIAL: Mutex<()> = Mutex::new(());

/// The guard every test holds; a test that panicked while holding it poisons nothing for
/// the next.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Starts capturing `casein` events afresh.
fn capture() {
    SUBSCRIBER.call_once(|| {
        tracing::subscriber::set_global_default(Capture).expect("the only subscriber");
    });
    LINES.lock().expect("the log").clear();
}

/// What the world logged since [`capture`] or the last call.
fn take() -> Vec<Line> {
    core::mem::take(&mut *LINES.lock().expect("the log"))
}

/// Runs `f` and returns what it logged.
fn logged<R>(f: impl FnOnce() -> R) -> (R, Vec<Line>) {
    capture();
    let result = f();
    (result, take())
}

/// The lines that report a broken or emptied owner reference.
fn breaks(lines: &[Line]) -> Vec<&Line> {
    lines.iter().filter(|line| line.level == Level::WARN).collect()
}

// ---- Documents ----

/// A cooked baseline of rows, each `(hash, owner in its ecs::owned_by column)`.
fn baseline(rows: &[(u64, Option<StableKey>)]) -> Baseline {
    let registry = registry();
    let rows = rows.iter().map(|&(hash, owner)| {
        let mut components = vec![data(pos(hash as i32 & 0xFFFF, 0))];
        components.extend(owner.map(|owner| data(owned_by(owner))));
        BaselineRow { id: cooked(hash), components }
    });
    Baseline::from_rows(AssetSet::new(), schemas(&registry), rows).unwrap()
}

/// A region document: [`baseline`] and the overlay records over it.
fn document(rows: &[(u64, Option<StableKey>)], records: Vec<Record>) -> (Baseline, Overlay) {
    let baseline = baseline(rows);
    let overlay = Overlay::new(baseline.content_hash(), schemas(&registry()), records).unwrap();
    (baseline, overlay)
}

fn cell<'a>(column: Column<'a>, index: usize) -> &'a [u8] {
    let size = column.element_size as usize;
    &column.bytes[index * size..(index + 1) * size]
}

/// The overlay `lactose-persist`'s diff writes for `rows` against `baseline`, as far as
/// these tests need it (no transient components), until persist's own tests exist: a
/// component whose bytes differ or that the baseline row lacks is `AddComponent`, one the
/// row lost `RemoveComponent`, an owner that differs from the baseline column's is
/// `OwnershipChange`, a baseline row the export lacks `Destroy`, any other row a spawn.
fn overlay_of(rows: &RegionRows, baseline: &Baseline) -> Result<Overlay> {
    let mut records = Vec::new();
    let present: BTreeSet<NetId> = rows.rows.iter().map(|row| row.id).collect();
    for archetype in baseline.archetypes() {
        for &id in archetype.ids() {
            if !present.contains(&id) {
                records.push(Record::Destroy { id });
            }
        }
    }
    for row in &rows.rows {
        assert!(row.transient.is_empty(), "these tests use no transient component");
        let Some(archetype) = baseline.archetype_of(row.id) else {
            let components = row.components.clone();
            records.push(if row.id.is_cooked() {
                Record::SpawnFromBaseline { id: row.id, components }
            } else {
                Record::Spawn { id: row.id, components }
            });
            if row.owner.is_some() {
                records.push(Record::OwnershipChange { id: row.id, owner: row.owner });
            }
            continue;
        };
        let index = archetype.ids().binary_search(&row.id).expect("a row of the archetype");
        for data in &row.components {
            let cooked = archetype.column(data.component).map(|column| cell(column, index));
            if cooked != Some(data.bytes.as_slice()) {
                records.push(Record::AddComponent { id: row.id, data: data.clone() });
            }
        }
        for column in archetype.columns() {
            let kept = row.components.iter().any(|data| data.component == column.component);
            if column.component != OwnedBy::KEY && !kept {
                records.push(Record::RemoveComponent { id: row.id, component: column.component });
            }
        }
        let cooked_owner = archetype
            .column(OwnedBy::KEY)
            .map(|column| bytemuck::checked::pod_read_unaligned::<OwnedBy>(cell(column, index)))
            .and_then(|value| value.owner.get())
            .map(|owner| match owner {
                StableKey::Net(id) => id,
                other => panic!("a cooked owner is a `Net` key, not {other:?}"),
            });
        if cooked_owner != row.owner {
            records.push(Record::OwnershipChange { id: row.id, owner: row.owner });
        }
    }
    Overlay::new(baseline.content_hash(), schemas(&registry()), records)
}

fn ownership_records(overlay: &Overlay) -> Vec<&Record> {
    overlay
        .records()
        .iter()
        .filter(|record| matches!(record, Record::OwnershipChange { .. }))
        .collect()
}

// ---- Direct writes ----

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
    let _serial = serial();
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
    let _serial = serial();
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

/// The first `NetId` the server allocates in the command tests.
const FIRST_ALLOCATED: u64 = 100;

fn server_from(first: u64, layout: LayoutConfig) -> World {
    let first = core::num::NonZeroU64::new(first).unwrap();
    let mut world = WorldBuilder::new(WorldKind::Server, registry())
        .layout(layout)
        .next_net_id(first)
        .build()
        .unwrap();
    world.create_region(RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated }).unwrap();
    world
}

/// One buffer, from the entity with `Counter`, whose dangling owner is the key the
/// buffer's third spawn will receive.
#[system(name = "ownership::commands", worlds = [server])]
fn ownership_commands(cmd: Commands, key: StableKeyOf, _source: &Counter) {
    let source = *key;
    cmd.set(net(1), owned_by(net(2)));
    cmd.set(net(2), owned_by(net(1)));
    cmd.set(net(1), Health(1));
    let kept = cmd.spawn((pos(7, 7),));
    let other = cmd.spawn((pos(8, 8),));
    // Receives the source's owner as its key, so its own owner closes the cycle.
    cmd.spawn((pos(5, 5), owned_by(source)));
    // Its own placeholder: an owner of itself.
    cmd.spawn((pos(6, 6), owned_by(StableKey::Placeholder(3))));
    cmd.set(kept, owned_by(source));
    cmd.set(other, owned_by(other));
    cmd.set(source, Health(2));
}

#[test]
fn a_command_closing_a_cycle_is_dropped() {
    let _serial = serial();
    let executor = executor();
    for layout in layouts() {
        let mut world = server_from(FIRST_ALLOCATED, layout);
        let source = net(9);
        world.spawn(net(1), REGION, (pos(1, 1),)).unwrap();
        world.spawn(net(2), REGION, (pos(2, 2),)).unwrap();
        world.spawn(source, REGION, (Counter(0), owned_by(net(FIRST_ALLOCATED + 2)))).unwrap();
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("simulation", &[]).add(&OWNERSHIP_COMMANDS);
        let mut schedule = builder.build().unwrap();

        let (mut world, lines) = logged(|| tick(&mut schedule, world, &executor));

        let context = format!("{layout:?}");
        assert_eq!(owner_of(&world, net(1)), Some(net(2)), "{context}: the first set applied");
        assert_eq!(owner_of(&world, net(2)), None, "{context}: the closing set dropped");
        assert_eq!(world.get::<Health>(net(1)), Some(&Health(1)), "{context}");
        assert_eq!(world.get::<Health>(source), Some(&Health(2)), "{context}");
        assert_eq!(owner_of(&world, source), Some(net(FIRST_ALLOCATED + 2)), "{context}");
        // The two spawns that would close a cycle were dropped and used no `NetId`; the
        // others and the sets on them applied.
        assert_eq!(world.get::<Position>(net(100)), Some(&pos(7, 7)), "{context}");
        assert_eq!(owner_of(&world, net(100)), Some(source), "{context}");
        assert_eq!(world.get::<Position>(net(101)), Some(&pos(8, 8)), "{context}");
        assert_eq!(owner_of(&world, net(101)), None, "{context}");
        assert!(!world.contains(net(102)) && !world.contains(net(103)), "{context}");
        assert_eq!(world.allocate_net_id().unwrap().to_bits(), 102, "{context}");
        let owned: Vec<_> = world.owned_by(source).collect();
        assert_eq!(owned, [net(100)], "{context}");

        let dropped: Vec<&Line> = lines
            .iter()
            .filter(|line| line.level == Level::DEBUG && line.says("cycle of owners"))
            .collect();
        let sets = dropped.iter().filter(|line| line.says("a set dropped")).count();
        let spawns = dropped.iter().filter(|line| line.says("a spawn dropped")).count();
        assert_eq!((sets, spawns, dropped.len()), (2, 2, 4), "{context}: {dropped:?}");
        assert!(breaks(&lines).is_empty(), "{context}: {lines:?}");
    }
}

/// One spawn from the claimed entity with `Counter`, predicted: with `Counter(1)` its
/// bundle names the spawn as its own owner, so the apply refuses it.
#[system(name = "ownership::predicted_spawn", worlds = [server, prediction])]
fn predicted_spawn(cmd: Commands, _claimed: With<ClaimedBy>, counter: &Counter) {
    if counter.0 == 0 {
        cmd.spawn((pos(3, 3),));
    } else {
        cmd.spawn((pos(3, 3), owned_by(StableKey::Placeholder(0))));
    }
}

#[test]
fn a_refused_predicted_spawn_binds_nothing() {
    let _serial = serial();
    let executor = executor();
    let schedule = || {
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("input", &[]).add(&READ_INPUT);
        builder.stage("spawn", &[]).add(&PREDICTED_SPAWN);
        builder.build().unwrap()
    };
    for layout in layouts() {
        // Twins whose one spawn derives the same key; the second's is refused.
        let twins = [0, 1].map(|counter| {
            let mut world = server_from(FIRST_ALLOCATED, layout);
            world.spawn(net(1), REGION, (pos(1, 1), Counter(counter))).unwrap();
            world.insert(net(1), ClaimedBy { player: PLAYER }).unwrap();
            logged(|| tick(&mut schedule(), world, &executor))
        });
        let [(accepted, _), (mut refused, lines)] = twins;
        let context = format!("{layout:?}");
        let spawned = accepted.entities_in(REGION).find(|&key| key != net(1)).expect(&context);
        let StableKey::Predicted(prediction) = spawned else {
            panic!("{context}: {spawned:?} is not predicted");
        };
        assert_eq!(accepted.alias(prediction).map(NetId::to_bits), Some(FIRST_ALLOCATED));

        assert_eq!(refused.alias(prediction), None, "{context}: no bind for a refused spawn");
        assert!(!refused.contains(spawned), "{context}");
        assert_eq!(refused.entities_in(REGION).count(), 1, "{context}");
        assert_eq!(
            refused.allocate_net_id().unwrap().to_bits(),
            FIRST_ALLOCATED,
            "{context}: the refused spawn used no NetId"
        );
        let dropped = lines.iter().filter(|line| line.says("a spawn dropped")).count();
        assert_eq!(dropped, 1, "{context}: {lines:?}");
    }
}

// ---- Dangling owners ----

#[test]
fn a_chain_through_a_dangling_owner() {
    let _serial = serial();
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

        // The same closing edge in a document: its row X is owned by an item live in
        // another region, which X owns. Through the baseline's column and through a record,
        // the load breaks it at X.
        let x = cooked(0x5);
        let column = document(&[(0x5, Some(item))], vec![]);
        let record = document(
            &[(0x5, None)],
            vec![Record::OwnershipChange { id: x, owner: NetId::from_bits(1) }],
        );
        let setup = || {
            let mut world = world(WorldKind::Server, layout);
            create(&mut world, OTHER_REGION);
            world.spawn(item, OTHER_REGION, (pos(1, 1), owned_by(key(x)))).unwrap();
            world
        };
        for (name, (baseline, overlay)) in [("column", column), ("record", record)] {
            let context = format!("{layout:?} {name}");
            let mut world = setup();
            let (loaded, lines) = logged(|| world.load_region(REGION, &baseline, Some(&overlay)));
            loaded.expect(&context);
            assert!(world.get::<OwnedBy>(key(x)).is_none(), "{context}: X loads without one");
            assert_eq!(owner_of(&world, item), Some(key(x)), "{context}: the item is untouched");
            assert_eq!(world.owned_by(key(x)).collect::<Vec<_>>(), [item], "{context}");
            let broken = breaks(&lines);
            assert_eq!(broken.len(), 1, "{context}: {lines:?}");
            assert!(broken[0].names("row", key(x)) && broken[0].names("owner", item), "{context}");
            assert_forest(&world, &context);

            // The export carries the break: a cleared column owner, or nothing.
            let export = overlay_of(&world.export_region(REGION).unwrap(), &baseline).unwrap();
            let expected: Vec<Record> = if name == "column" {
                vec![Record::OwnershipChange { id: x, owner: None }]
            } else {
                vec![]
            };
            assert_eq!(
                ownership_records(&export),
                expected.iter().collect::<Vec<_>>(),
                "{context}"
            );

            // Reloaded into a world set up the same way, it breaks nothing and gives the same
            // state, so the break changed nothing but the loaded row.
            let mut again = setup();
            let (loaded, lines) = logged(|| again.load_region(REGION, &baseline, Some(&export)));
            loaded.expect(&context);
            assert!(breaks(&lines).is_empty(), "{context}: {lines:?}");
            assert_eq!(state(&again, &[]), state(&world, &[]), "{context}");
        }

        // The control: the row without an owner loads, and the item's reference finds it.
        let (baseline, overlay) = document(&[(0x5, None)], vec![]);
        let mut world = setup();
        let (loaded, lines) = logged(|| world.load_region(REGION, &baseline, Some(&overlay)));
        loaded.unwrap();
        assert!(breaks(&lines).is_empty(), "{layout:?}: {lines:?}");
        assert_eq!(world.owned_by(key(x)).collect::<Vec<_>>(), [item], "{layout:?}");
    }
}

// ---- Loads ----

#[test]
fn a_load_whose_owners_cycle_is_refused_whole() {
    let _serial = serial();
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
            assert!(error.message().contains("cycle of owners"), "{context}: {error}");
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
    }
}

#[test]
fn a_row_whose_bound_id_lacks_its_origin_is_refused() {
    let _serial = serial();
    let row = vec![Record::Spawn { id: net_50(), components: vec![data(pos(6, 0))] }];
    for layout in layouts() {
        for live in [false, true] {
            let context = format!("{layout:?} predicted entity live={live}");
            let mut world = world(WorldKind::Server, layout);
            create(&mut world, OTHER_REGION);
            if live {
                world.spawn(predicted(), REGION, (pos(4, 4),)).unwrap();
            }
            world.bind(prediction(), net_50()).unwrap();
            let (baseline, overlay) = document(&[], row.clone());
            let marks = [world.mark()];
            let before = state(&world, &marks);
            let error = world.load_region(OTHER_REGION, &baseline, Some(&overlay)).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{context}: {error}");
            assert!(error.message().contains("PredictedOrigin"), "{context}: {error}");
            assert_eq!(world.entities_in(OTHER_REGION).count(), 0, "{context}");
            assert_eq!(state(&world, &marks), before, "{context}");
        }

        // The control: without the bind the row is a `Net` entity like any other.
        let (baseline, overlay) = document(&[], row.clone());
        let mut world = world(WorldKind::Server, layout);
        world.load_region(REGION, &baseline, Some(&overlay)).unwrap();
        assert_eq!(world.get::<Position>(bound()), Some(&pos(6, 0)), "{layout:?}");
    }
}

/// Loads `documents` into `world` in order, and returns what the loads logged.
fn load_all(world: &mut World, documents: &[(RegionId, &Baseline, &Overlay)]) -> Vec<Line> {
    capture();
    for &(region, baseline, overlay) in documents {
        world.load_region(region, baseline, Some(overlay)).unwrap();
    }
    take()
}

#[test]
fn a_cycle_across_two_documents_is_broken_at_the_document_row() {
    let _serial = serial();
    let (a1, b1) = (cooked(0xA1), cooked(0xB1));
    let (a, b) = (REGION, OTHER_REGION);
    // A's row owned by B's, B's by A's.
    let (baseline_a, overlay_a) = document(&[(0xA1, Some(key(b1)))], vec![]);
    let (baseline_b, overlay_b) = document(&[(0xB1, Some(key(a1)))], vec![]);
    let doc = |region| {
        if region == a {
            (region, &baseline_a, &overlay_a)
        } else {
            (region, &baseline_b, &overlay_b)
        }
    };
    // Whichever loads second has the row on the cycle whose owner is live outside it.
    for (order, broken, kept) in [([a, b], b1, a1), ([b, a], a1, b1)] {
        let mut exports: Option<(RegionRows, RegionRows)> = None;
        for layout in layouts() {
            let context = format!("{layout:?} {order:?}");
            let mut world = world(WorldKind::Server, layout);
            create(&mut world, b);
            let lines = load_all(&mut world, &order.map(doc));
            let broke = breaks(&lines);
            assert_eq!(broke.len(), 1, "{context}: {lines:?}");
            assert!(broke[0].names("row", key(broken)) && broke[0].names("owner", key(kept)));
            assert!(world.get::<OwnedBy>(key(broken)).is_none(), "{context}");
            assert_eq!(owner_of(&world, key(kept)), Some(key(broken)), "{context}");
            assert_forest(&world, &context);

            let export = (world.export_region(a).unwrap(), world.export_region(b).unwrap());
            match &exports {
                None => exports = Some(export.clone()),
                Some(first) => assert_eq!(&export, first, "{context}: the layout shows"),
            }

            // The broken region's re-export carries the cleared owner; the re-exports load
            // into a fresh world in the same order without a break and export equal.
            let again_a = overlay_of(&export.0, &baseline_a).unwrap();
            let again_b = overlay_of(&export.1, &baseline_b).unwrap();
            let again_broken = if broken == b1 { &again_b } else { &again_a };
            assert_eq!(
                ownership_records(again_broken),
                [&Record::OwnershipChange { id: broken, owner: None }],
                "{context}"
            );
            let mut fresh = common::world(WorldKind::Server, layout);
            create(&mut fresh, b);
            let docs = order.map(|region| {
                if region == a {
                    (region, &baseline_a, &again_a)
                } else {
                    (region, &baseline_b, &again_b)
                }
            });
            let lines = load_all(&mut fresh, &docs);
            assert!(breaks(&lines).is_empty(), "{context}: {lines:?}");
            assert_eq!((fresh.export_region(a).unwrap(), fresh.export_region(b).unwrap()), export);
        }
    }

    // Three regions: the cycle c1 → a1 → c2 → b1 → c1 has two rows of C's document whose
    // owners are live outside it. The smaller key loses its owner, the larger keeps it.
    let (c1, c2) = (cooked(0xC1), cooked(0xC2));
    let (small, large) = if key(c1) < key(c2) { (c1, c2) } else { (c2, c1) };
    let (baseline_a, overlay_a) = document(&[(0xA1, Some(key(c2)))], vec![]);
    let (baseline_b, overlay_b) = document(&[(0xB1, Some(key(c1)))], vec![]);
    let (baseline_c, overlay_c) = document(&[(0xC1, Some(key(a1))), (0xC2, Some(key(b1)))], vec![]);
    let outside = |row: NetId| if row == c1 { a1 } else { b1 };
    for layout in layouts() {
        let context = format!("{layout:?} three regions");
        let mut world = world(WorldKind::Server, layout);
        create(&mut world, b);
        create(&mut world, THIRD_REGION);
        let lines = load_all(
            &mut world,
            &[
                (a, &baseline_a, &overlay_a),
                (b, &baseline_b, &overlay_b),
                (THIRD_REGION, &baseline_c, &overlay_c),
            ],
        );
        let broke = breaks(&lines);
        assert_eq!(broke.len(), 1, "{context}: {lines:?}");
        assert!(broke[0].names("row", key(small)), "{context}: {broke:?}");
        assert!(world.get::<OwnedBy>(key(small)).is_none(), "{context}");
        assert_eq!(owner_of(&world, key(large)), Some(key(outside(large))), "{context}");
        assert_forest(&world, &context);
    }
}

// ---- Key merges ----

#[test]
fn a_bind_that_would_close_a_cycle_empties_the_reference() {
    let _serial = serial();
    for layout in layouts() {
        // The predicted entity's chain is 1, 2 or 3 long and ends at an entity owned by the
        // `NetId` the bind names.
        for len in 1..=3 {
            let context = format!("{layout:?} chain of {len}");
            let chain: Vec<StableKey> =
                [predicted()].into_iter().chain((2..=len).map(net)).collect();
            let end = *chain.last().unwrap();
            let setup = || {
                let mut world = world(WorldKind::Prediction, layout);
                for (i, &member) in chain.iter().enumerate().rev() {
                    let owner = chain.get(i + 1).copied().unwrap_or(bound());
                    world.spawn(member, REGION, (pos(0, 0), owned_by(owner))).unwrap();
                }
                // Another reference to the bound key, whose rewrite closes nothing.
                world.spawn(net(9), REGION, (pos(9, 9), owned_by(bound()))).unwrap();
                world
            };

            let mut world = setup();
            let (bind, lines) = logged(|| world.bind(prediction(), net_50()));
            bind.expect(&context);
            assert_eq!(world.alias(prediction()), Some(net_50()), "{context}");
            assert!(emptied(&world, end), "{context}: emptied in place");
            assert!(!world.owned_by(predicted()).any(|key| key == end), "{context}");
            assert!(!world.referrers(predicted()).any(|r| r.source == end), "{context}");
            assert_eq!(owner_of(&world, net(9)), Some(predicted()), "{context}: rewritten");
            for pair in chain.windows(2) {
                assert_eq!(owner_of(&world, pair[0]), Some(pair[1]), "{context}: kept");
            }
            let broke = breaks(&lines);
            assert_eq!(broke.len(), 1, "{context}: {lines:?}");
            assert!(broke[0].names("entity", end) && broke[0].names("owner", bound()));
            assert_forest(&world, &context);

            // Another `NetId` closes nothing: the end keeps its owner.
            let other = NetId::from_bits(51).unwrap();
            let mut world = setup();
            let (bind, lines) = logged(|| world.bind(prediction(), other));
            bind.expect(&context);
            assert_eq!(world.alias(prediction()), Some(other), "{context}");
            assert_eq!(owner_of(&world, end), Some(bound()), "{context}");
            assert!(breaks(&lines).is_empty(), "{context}: {lines:?}");
        }
    }
}

#[test]
fn a_rekey_that_would_revive_a_cycle_empties_the_reference() {
    let _serial = serial();
    for layout in layouts() {
        // A restore that revives an entity owned by a `Net` key bound since the snapshot to
        // an entity whose chain reaches it.
        let context = format!("{layout:?} restore");
        let mut world = world(WorldKind::Prediction, layout);
        let owner = net(10);
        world.spawn(owner, REGION, (pos(0, 0), owned_by(bound()))).unwrap();
        world.spawn(predicted(), REGION, (pos(1, 1), owned_by(owner))).unwrap();
        let snapshot = world.snapshot();
        world.despawn(owner).unwrap();
        assert!(!world.contains(predicted()), "{context}: despawned with its owner");
        world.bind(prediction(), net_50()).unwrap();
        let (restored, lines) = logged(|| world.restore(&snapshot));
        restored.expect(&context);
        assert!(emptied(&world, owner), "{context}");
        assert_eq!(owner_of(&world, predicted()), Some(owner), "{context}");
        let broke = breaks(&lines);
        assert_eq!(broke.len(), 1, "{context}: {lines:?}");
        assert!(broke[0].names("entity", owner) && broke[0].names("owner", bound()));
        assert_forest(&world, &context);

        // The despawn of a bound `Net` entity a restore revived beside the predicted entity:
        // the references to it go to the origin key, which would close E → P → E.
        let context = format!("{layout:?} unload");
        let (a, b) = (REGION, OTHER_REGION);
        let e = net(7);
        let mut world = common::world(WorldKind::Prediction, layout);
        create(&mut world, b);
        world.spawn(bound(), b, (pos(0, 0),)).unwrap();
        world.spawn(e, a, (pos(1, 0), owned_by(bound()))).unwrap();
        world.spawn(predicted(), a, (pos(2, 0), owned_by(e))).unwrap();
        let snapshot = world.snapshot();
        world.despawn(bound()).unwrap();
        capture();
        world.bind(prediction(), net_50()).unwrap();
        world.restore(&snapshot).unwrap();
        world.unload_region(b).unwrap();
        let lines = take();
        assert!(emptied(&world, e), "{context}");
        assert_eq!(owner_of(&world, predicted()), Some(e), "{context}");
        let broke = breaks(&lines);
        assert_eq!(broke.len(), 1, "{context}: {lines:?}");
        assert!(broke[0].names("entity", e) && broke[0].names("owner", bound()), "{context}");
        assert_forest(&world, &context);
        let export = world.export_region(a).unwrap();
        overlay_of(&export, &baseline(&[])).expect("A's export is an overlay");
    }
}

// ---- Histories the world's own saves must survive ----

/// Gives the entity to the target of its `ThrownBy`, as a game's kernel would, and drops
/// the request.
#[system(name = "ownership::give", worlds = [server])]
fn give(cmd: Commands, key: StableKeyOf, want: &ThrownBy) {
    cmd.set(*key, OwnedBy { owner: want.thrower });
    cmd.remove::<ThrownBy>(*key);
}

/// A server with regions A and B loaded from empty documents.
fn two_regions(kind: WorldKind, layout: LayoutConfig, mode: RegionMode) -> World {
    let mut world = WorldBuilder::new(kind, registry()).layout(layout).build().unwrap();
    for region in [REGION, OTHER_REGION] {
        world.create_region(RegionConfig { id: region, seed: 7, mode }).unwrap();
    }
    world
}

#[test]
fn a_saved_region_loads_after_a_cycle_closed_through_an_unloaded_entity() {
    let _serial = serial();
    let executor = executor();
    let empty = baseline(&[]);
    let (a, b) = (REGION, OTHER_REGION);
    let (e, p, x) = (net(1), net(2), net(3));
    let give_to = |world: World, schedule: &mut Schedule, what: StableKey, to: StableKey| {
        let mut world = world;
        world.insert(what, ThrownBy { thrower: EntityRef::new(to) }).unwrap();
        tick(schedule, world, &executor)
    };
    for layout in layouts() {
        let context = format!("{layout:?}");
        let mut world = two_regions(WorldKind::Server, layout, RegionMode::Simulated);
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("simulation", &[]).add(&GIVE);
        let mut schedule = builder.build().unwrap();
        world.spawn(e, a, (pos(1, 0),)).unwrap();
        world.spawn(p, a, (pos(2, 0),)).unwrap();
        world.spawn(x, b, (pos(3, 0),)).unwrap();
        // Three legal commands and an unload: P is given to X, X to E, B unloads, E is given
        // to P, whose chain ends at X, not live.
        world = give_to(world, &mut schedule, p, x);
        world = give_to(world, &mut schedule, x, e);
        let doc_b = overlay_of(&world.export_region(b).unwrap(), &empty).unwrap();
        world.unload_region(b).unwrap();
        world = give_to(world, &mut schedule, e, p);
        assert_eq!(owner_of(&world, e), Some(p), "{context}: the command applied");
        let doc_a = overlay_of(&world.export_region(a).unwrap(), &empty).unwrap();

        // The server reloads its own save of B: X loads without its owner.
        create(&mut world, b);
        let lines = load_all(&mut world, &[(b, &empty, &doc_b)]);
        let broke = breaks(&lines);
        assert_eq!(broke.len(), 1, "{context}: {lines:?}");
        assert!(broke[0].names("row", x) && broke[0].names("owner", e), "{context}");
        assert!(world.get::<OwnedBy>(x).is_none(), "{context}");
        assert_eq!((owner_of(&world, e), owner_of(&world, p)), (Some(p), Some(x)), "{context}");
        assert_forest(&world, &context);

        // The export after the reload carries the break, and reloads without another.
        let doc_b_again = overlay_of(&world.export_region(b).unwrap(), &empty).unwrap();
        assert!(ownership_records(&doc_b_again).is_empty(), "{context}");
        world.unload_region(b).unwrap();
        create(&mut world, b);
        let lines = load_all(&mut world, &[(b, &empty, &doc_b_again)]);
        assert!(breaks(&lines).is_empty(), "{context}: {lines:?}");
        assert!(world.get::<OwnedBy>(x).is_none(), "{context}");

        // A restart from the two saves, in either order: [A, B] empties X's edge, [B, A]
        // P's, each the document row whose owner is live outside it.
        for (order, broken, owner) in [([a, b], x, e), ([b, a], p, x)] {
            let context = format!("{layout:?} restart {order:?}");
            let mut fresh = two_regions(WorldKind::Server, layout, RegionMode::Simulated);
            let docs =
                order.map(|region| (region, &empty, if region == a { &doc_a } else { &doc_b }));
            let lines = load_all(&mut fresh, &docs);
            let broke = breaks(&lines);
            assert_eq!(broke.len(), 1, "{context}: {lines:?}");
            assert!(broke[0].names("row", broken) && broke[0].names("owner", owner));
            assert!(fresh.get::<OwnedBy>(broken).is_none(), "{context}");
            assert_forest(&fresh, &context);
        }
    }
}

#[test]
fn a_client_loads_documents_sent_at_different_times() {
    let _serial = serial();
    let empty = baseline(&[]);
    let (a, b) = (REGION, OTHER_REGION);
    let (e, p, x) = (net(1), net(2), net(3));
    for layout in layouts() {
        let context = format!("{layout:?}");
        let mut server = two_regions(WorldKind::Server, layout, RegionMode::Simulated);
        server.spawn(e, a, (pos(1, 0),)).unwrap();
        server.spawn(p, a, (pos(2, 0),)).unwrap();
        server.spawn(x, b, (pos(3, 0),)).unwrap();
        // t1: X is owned by E, and B becomes relevant to the client.
        server.insert(x, owned_by(e)).unwrap();
        let b_t1 = overlay_of(&server.export_region(b).unwrap(), &empty).unwrap();
        // t2: X is released, P given to X and E to P, a forest at every step; A becomes
        // relevant.
        server.remove::<OwnedBy>(x).unwrap();
        server.insert(p, owned_by(x)).unwrap();
        server.insert(e, owned_by(p)).unwrap();
        let a_t2 = overlay_of(&server.export_region(a).unwrap(), &empty).unwrap();

        let mut client = two_regions(WorldKind::Client, layout, RegionMode::Replicated);
        let lines = load_all(&mut client, &[(b, &empty, &b_t1), (a, &empty, &a_t2)]);
        let broke = breaks(&lines);
        assert_eq!(broke.len(), 1, "{context}: {lines:?}");
        assert!(broke[0].names("row", p) && broke[0].names("owner", x), "{context}");
        assert!(client.get::<OwnedBy>(p).is_none(), "{context}");
        assert_eq!((owner_of(&client, e), owner_of(&client, x)), (Some(p), Some(e)), "{context}");
        assert_forest(&client, &context);

        // B's next document, the server's state now, loads over a recreated B.
        let b_t3 = overlay_of(&server.export_region(b).unwrap(), &empty).unwrap();
        client.unload_region(b).unwrap();
        client
            .create_region(RegionConfig { id: b, seed: 7, mode: RegionMode::Replicated })
            .unwrap();
        let lines = load_all(&mut client, &[(b, &empty, &b_t3)]);
        assert!(breaks(&lines).is_empty(), "{context}: {lines:?}");
        assert!(client.get::<OwnedBy>(x).is_none(), "{context}");
        assert_forest(&client, &context);
    }
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// One seeded history of direct writes, unloads and reloads over regions A and B, checked
/// after every step and by a restart at the end; returns the number of breaks it logged.
/// `live_only` draws owners among live entities, what a command can name; otherwise among
/// every key the history has used, what a direct write can name.
fn history(seed: u64, live_only: bool, layout: LayoutConfig) -> usize {
    let (a, b) = (REGION, OTHER_REGION);
    let mut rng = Rng(seed);
    let baselines: BTreeMap<RegionId, Baseline> = [
        (
            a,
            baseline(&[
                (0xA1, None),
                (0xA2, Some(key(cooked(0xA1)))),
                (0xA3, Some(key(cooked(0xA2)))),
                (0xA4, Some(key(cooked(0xB1)))),
            ]),
        ),
        (b, baseline(&[(0xB1, None), (0xB2, Some(key(cooked(0xB1)))), (0xB3, None)])),
    ]
    .into_iter()
    .collect();
    let server = || two_regions(WorldKind::Server, layout, RegionMode::Simulated);
    let loaded = |world: &World| -> Vec<RegionId> {
        [a, b].into_iter().filter(|&region| world.region(region).is_some()).collect()
    };
    let live = |world: &World| -> Vec<StableKey> {
        loaded(world).into_iter().flat_map(|region| world.entities_in(region)).collect()
    };

    capture();
    let mut world = server();
    for (&region, baseline) in &baselines {
        let empty = Overlay::new(baseline.content_hash(), schemas(&registry()), vec![]).unwrap();
        world.load_region(region, baseline, Some(&empty)).unwrap();
    }
    let mut pool: Vec<StableKey> = [0xA1, 0xA2, 0xA3, 0xA4, 0xB1, 0xB2, 0xB3]
        .map(|hash| key(cooked(hash)))
        .into_iter()
        .chain([net(900_001), net(900_002)])
        .collect();
    let mut saved: BTreeMap<RegionId, Overlay> = BTreeMap::new();
    let mut log: Vec<String> = Vec::new();
    let mut ordinal = 0u32;
    let pick_owner = |rng: &mut Rng, world: &World, pool: &[StableKey]| -> StableKey {
        let live = live(world);
        if !live_only {
            pool[rng.below(pool.len())]
        } else if live.is_empty() {
            pool[0]
        } else {
            live[rng.below(live.len())]
        }
    };
    let ops = 20 + rng.below(60);
    for _ in 0..ops {
        let context = |log: &[String]| {
            let tail = log.len().saturating_sub(30);
            format!("seed {seed:#x} live_only={live_only} {layout:?}:\n{}", log[tail..].join("\n"))
        };
        let live_keys = live(&world);
        let regions = loaded(&world);
        match rng.below(100) {
            0..=14 if !regions.is_empty() => {
                let region = regions[rng.below(regions.len())];
                let id = world.allocate_net_id().unwrap();
                pool.push(key(id));
                let row = (pos(id.to_bits() as i32, 1),);
                if rng.chance(50) {
                    let owner = pick_owner(&mut rng, &world, &pool);
                    log.push(format!("spawn {id:?} in {region:?} owned by {owner:?}"));
                    let _ = world.spawn(key(id), region, (row.0, owned_by(owner)));
                } else {
                    log.push(format!("spawn {id:?} in {region:?}"));
                    world.spawn(key(id), region, row).unwrap();
                }
            }
            15..=19 if !regions.is_empty() => {
                // A predicted spawn as the server applies it: the spawn, then the bind.
                let region = regions[rng.below(regions.len())];
                ordinal += 1;
                let prediction = PredictionKey { player: PLAYER, input_tick: t(1), ordinal };
                let id = world.allocate_net_id().unwrap();
                pool.push(StableKey::Predicted(prediction));
                pool.push(key(id));
                let owner = pick_owner(&mut rng, &world, &pool);
                log.push(format!(
                    "predicted spawn {prediction:?}={id:?} in {region:?} owned by {owner:?}"
                ));
                let row = (pos(9, 9), owned_by(owner));
                if world.spawn(StableKey::Predicted(prediction), region, row).is_ok() {
                    world
                        .bind(prediction, id)
                        .unwrap_or_else(|error| panic!("{error}: {}", context(&log)));
                } else {
                    // Never bound, so a reference to it could not be exported.
                    pool.retain(|&key| key != StableKey::Predicted(prediction));
                }
            }
            20..=54 if !live_keys.is_empty() => {
                let k = live_keys[rng.below(live_keys.len())];
                let owner = pick_owner(&mut rng, &world, &pool);
                log.push(format!("set {k:?} owned by {owner:?}"));
                let _ = if rng.chance(50) {
                    world.insert(k, owned_by(owner))
                } else {
                    world.insert_erased(k, &data(owned_by(owner))).map(drop)
                };
            }
            55..=59 if !live_keys.is_empty() => {
                let k = live_keys[rng.below(live_keys.len())];
                log.push(format!("clear the owner of {k:?}"));
                world.remove::<OwnedBy>(k).unwrap();
            }
            60..=69 if !live_keys.is_empty() && !regions.is_empty() => {
                let k = live_keys[rng.below(live_keys.len())];
                let region = regions[rng.below(regions.len())];
                log.push(format!("move {k:?} to {region:?}"));
                world.move_to_region(k, region).unwrap();
            }
            70..=77 if !live_keys.is_empty() => {
                let k = live_keys[rng.below(live_keys.len())];
                log.push(format!("despawn {k:?}"));
                world.despawn(k).unwrap();
            }
            78..=88 if !regions.is_empty() => {
                let region = regions[rng.below(regions.len())];
                log.push(format!("unload {region:?}"));
                let rows = world
                    .export_region(region)
                    .unwrap_or_else(|error| panic!("{error}: {}", context(&log)));
                let overlay = overlay_of(&rows, &baselines[&region])
                    .unwrap_or_else(|error| panic!("{error}: {}", context(&log)));
                saved.insert(region, overlay);
                world.unload_region(region).unwrap();
            }
            89..=99 if !saved.is_empty() => {
                let unloaded: Vec<RegionId> = saved.keys().copied().collect();
                let region = unloaded[rng.below(unloaded.len())];
                log.push(format!("reload {region:?}"));
                let overlay = saved.remove(&region).unwrap();
                create(&mut world, region);
                world
                    .load_region(region, &baselines[&region], Some(&overlay))
                    .unwrap_or_else(|error| panic!("{error}: {}", context(&log)));
            }
            _ => continue,
        }
        let context = context(&log);
        assert_forest(&world, &context);
        for region in loaded(&world) {
            let rows = world.export_region(region).expect(&context);
            if let Err(error) = overlay_of(&rows, &baselines[&region]) {
                panic!("{error}: {context}\n{rows:#?}");
            }
        }
    }

    // A restart from the saves: every region's document, loaded in either order into a
    // fresh world, then a second round trip of that world's own exports, which is exact.
    let context = format!("seed {seed:#x} live_only={live_only} {layout:?}: {log:#?}");
    for region in loaded(&world) {
        let rows = world.export_region(region).unwrap();
        saved.insert(region, overlay_of(&rows, &baselines[&region]).unwrap());
    }
    for order in [[a, b], [b, a]] {
        let context = format!("{context} restart {order:?}");
        let mut fresh = server();
        for region in order {
            fresh.load_region(region, &baselines[&region], Some(&saved[&region])).expect(&context);
        }
        assert_forest(&fresh, &context);
        let exports = order.map(|region| fresh.export_region(region).unwrap());
        let mut again = server();
        let first = take();
        for (region, rows) in order.iter().zip(&exports) {
            let overlay = overlay_of(rows, &baselines[region]).expect(&context);
            again.load_region(*region, &baselines[region], Some(&overlay)).expect(&context);
        }
        let second = take();
        assert!(breaks(&second).is_empty(), "{context}: the round trip broke {second:?}");
        LINES.lock().expect("the log").extend(first);
        assert_eq!(order.map(|region| again.export_region(region).unwrap()), exports, "{context}");
    }
    breaks(&take()).len()
}

/// Histories per owner mode: 300, a few under Miri, or `LACTOSE_OWNERSHIP_HISTORIES` for a
/// longer run.
#[expect(clippy::disallowed_methods, reason = "test code outside the step: the run's length")]
fn histories() -> u64 {
    std::env::var("LACTOSE_OWNERSHIP_HISTORIES")
        .ok()
        .map(|count| count.parse().expect("LACTOSE_OWNERSHIP_HISTORIES is a count"))
        .unwrap_or(if cfg!(miri) { 2 } else { 300 })
}

#[test]
fn random_histories_always_save_and_load() {
    let _serial = serial();
    let per_mode = histories();
    let layouts = layouts();
    for live_only in [true, false] {
        let mut broken = 0;
        for i in 0..per_mode {
            let seed = i.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ u64::from(live_only);
            broken += history(seed, live_only, layouts[i as usize % layouts.len()]);
        }
        println!("live_only={live_only}: {per_mode} histories, {broken} breaks logged");
    }
}

// ---- Regions ----

#[test]
fn an_owned_entity_stays_where_it_is() {
    let _serial = serial();
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

// ---- Cost ----

/// A document of `n` runtime rows: with `deep`, row k is owned by row k - 1, a chain of `n`;
/// otherwise every row by the first.
fn chain_document(n: u64, deep: bool) -> (Baseline, Overlay) {
    let id = |bits: u64| NetId::from_bits(bits).unwrap();
    let mut records = Vec::new();
    for k in 1..=n {
        records.push(Record::Spawn { id: id(k), components: vec![data(pos(k as i32, 0))] });
        if k > 1 {
            let owner = if deep { k - 1 } else { 1 };
            records.push(Record::OwnershipChange { id: id(k), owner: Some(id(owner)) });
        }
    }
    document(&[], records)
}

/// The best of three loads of `document` into a fresh world.
#[expect(clippy::disallowed_types, reason = "test code outside the step: a cost ratio")]
fn best_load(document: &(Baseline, Overlay)) -> std::time::Duration {
    (0..3)
        .map(|_| {
            let mut world = WorldBuilder::new(WorldKind::Server, registry()).build().unwrap();
            create(&mut world, REGION);
            let start = std::time::Instant::now();
            world.load_region(REGION, &document.0, Some(&document.1)).unwrap();
            start.elapsed()
        })
        .min()
        .expect("three loads")
}

/// The load walks owners once, before any spawn, and the spawns walk nothing: a chain of
/// 8000 owners loads in about the time of 8000 rows with one owner, where a walk per row
/// costs about 260 times as much. The bound of 5 holds on a loaded machine.
#[test]
#[cfg_attr(miri, ignore = "a timing ratio; Miri's timings mean nothing")]
fn a_deep_chain_loads_as_fast_as_a_flat_one() {
    let _serial = serial();
    let flat = best_load(&chain_document(8000, false));
    let deep = best_load(&chain_document(8000, true));
    assert!(
        deep < flat * 5,
        "a chain of 8000 owners took {deep:?} to load against {flat:?} flat: the load walks \
         each row's chain again (a per-row walk in the spawns)"
    );
}
