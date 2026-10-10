//! Erased row access by key (`get_erased`, `insert_erased`, `remove_erased`,
//! `spawn_erased`): a typed and an erased write of the same value leave the same columns,
//! row stamps, column ticks, structural log, reverse index and claim index, under every
//! layout; equal bytes change nothing and no `Changed` reader sees them; every refusal
//! changes nothing; `get_erased` reads back what a restore puts back.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_bundle::ComponentData;
use lactose_core::{ComponentKey, EntityRef, ErrorKind, NetId, PlayerId, PredictionKey, StableKey};
use lactose_ecs::{
    ChangeMark, Changed, ClaimedBy, InRegion, LayoutConfig, LogEntry, Mut, OwnedBy,
    PredictedOrigin, Referrer, RegionId, Schedule, World, WorldKind, system,
};
use lactose_reflect::{Component, Reflect, Resource, TypeRole, component};

/// A component no registry of these tests holds.
#[component(key = "test::unregistered", persistent)]
struct Unregistered(#[field(id = 1)] u32);

/// `value` as the bytes a consumer working from the registry holds.
fn data<T: Component>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: bytemuck::bytes_of(&value).to_vec() }
}

/// A resource's bytes under its key: what an erased row write must refuse.
fn resource_data<T: Resource>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: bytemuck::bytes_of(&value).to_vec() }
}

fn bytes<T: Component>(value: T) -> Vec<u8> {
    bytemuck::bytes_of(&value).to_vec()
}

fn origin() -> PredictionKey {
    PredictionKey { player: PLAYER, input_tick: t(4), ordinal: 0xABCD }
}

fn predicted() -> StableKey {
    StableKey::Predicted(origin())
}

fn net_50() -> NetId {
    NetId::from_bits(50).unwrap()
}

/// The server's key of [`predicted`], once bound.
fn bound() -> StableKey {
    StableKey::Net(net_50())
}

fn to(key: StableKey) -> EntityRef {
    EntityRef::new(key)
}

const OTHER_PLAYER: PlayerId = PlayerId(9);

/// One chunk's column, as the world stores it.
#[derive(PartialEq, Debug)]
struct Column {
    component: ComponentKey,
    keys: Vec<StableKey>,
    regions: Vec<RegionId>,
    bytes: Vec<u8>,
    row_ticks: Option<Vec<lactose_core::Tick>>,
    column_tick: lactose_core::Tick,
}

/// Everything a direct write can change that a consumer can observe.
#[derive(PartialEq, Debug)]
struct State {
    mark: ChangeMark,
    columns: Vec<Column>,
    /// For each mark, the rows of every tracked column changed since it: the stamps'
    /// points, which column ticks alone do not show.
    changed: Vec<Option<Vec<(ComponentKey, StableKey)>>>,
    log: Vec<LogEntry>,
    referrers: Vec<Vec<Referrer>>,
    owned: Vec<Vec<StableKey>>,
    claimed: Vec<Vec<StableKey>>,
    live: Vec<bool>,
}

/// The keys every test here names.
fn keys() -> Vec<StableKey> {
    vec![net(1), net(2), net(3), net(4), predicted(), bound()]
}

fn state(world: &World, marks: &[ChangeMark]) -> State {
    let components: Vec<ComponentKey> = world
        .registry()
        .iter()
        .filter(|info| info.role() == TypeRole::Component)
        .map(|info| info.key())
        .collect();
    let mut columns = Vec::new();
    for &component in &components {
        for column in world.columns_erased(component) {
            columns.push(Column {
                component,
                keys: column.keys().to_vec(),
                regions: column.regions().to_vec(),
                bytes: column.bytes().to_vec(),
                row_ticks: column.row_ticks().map(<[_]>::to_vec),
                column_tick: column.column_tick(),
            });
        }
    }
    let changed = marks
        .iter()
        .map(|&mark| {
            let mut rows = Vec::new();
            for &component in &components {
                for column in world.columns_erased(component) {
                    if column.row_ticks().is_none() {
                        continue;
                    }
                    for row in column.changed_since(mark)? {
                        rows.push((component, column.keys()[row]));
                    }
                }
            }
            rows.sort();
            Some(rows)
        })
        .collect();
    let mut cursor = world.structural_log().cursor_at_start();
    let log =
        world.structural_log().since(&mut cursor).expect("a current cursor").copied().collect();
    State {
        mark: world.mark(),
        columns,
        changed,
        log,
        referrers: keys().into_iter().map(|key| world.referrers(key).collect()).collect(),
        owned: keys().into_iter().map(|key| world.owned_by(key).collect()).collect(),
        claimed: [PLAYER, OTHER_PLAYER].map(|player| world.claimed(player).collect()).to_vec(),
        live: keys().into_iter().map(|key| world.contains(key)).collect(),
    }
}

/// Two worlds, one written through the typed methods and one through the erased ones,
/// compared after every step.
struct Twin {
    typed: World,
    erased: World,
    marks: Vec<ChangeMark>,
}

impl Twin {
    fn new(layout: LayoutConfig) -> Self {
        Self {
            typed: world(WorldKind::Prediction, layout),
            erased: world(WorldKind::Prediction, layout),
            marks: Vec::new(),
        }
    }

    /// Runs one step on each world and requires the two to agree.
    fn step(&mut self, typed: impl FnOnce(&mut World), erased: impl FnOnce(&mut World)) {
        typed(&mut self.typed);
        erased(&mut self.erased);
        let mark = self.typed.mark();
        assert_eq!(mark, self.erased.mark(), "the two worlds bumped differently");
        self.marks.push(mark);
        assert_eq!(state(&self.typed, &self.marks), state(&self.erased, &self.marks));
    }

    /// The same step on both worlds.
    fn both(&mut self, f: impl Fn(&mut World)) {
        self.step(&f, &f);
    }
}

#[test]
fn typed_and_erased_writes_leave_identical_worlds_under_every_layout() {
    for layout in layouts() {
        let mut twin = Twin::new(layout);

        // Spawns, the erased entries in another order than the bundle's.
        twin.step(
            |w| w.spawn(net(1), REGION, (pos(1, 2), vel(3, 4), Health(100), Counter(0))).unwrap(),
            |w| {
                let components =
                    [data(Counter(0)), data(Health(100)), data(vel(3, 4)), data(pos(1, 2))];
                w.spawn_erased(net(1), REGION, &components).unwrap();
            },
        );
        let relations = || {
            (
                Bomb { fuse: 9 },
                ThrownBy { thrower: to(net(1)) },
                OwnedBy { owner: to(net(1)) },
                ClaimedBy { player: PLAYER },
            )
        };
        twin.step(
            |w| w.spawn(net(2), REGION, relations()).unwrap(),
            |w| {
                let (bomb, thrown, owned, claim) = relations();
                let components = [data(bomb), data(thrown), data(owned), data(claim)];
                w.spawn_erased(net(2), REGION, &components).unwrap();
            },
        );
        twin.step(
            |w| w.spawn(predicted(), REGION, (pos(5, 5), Health(7))).unwrap(),
            |w| w.spawn_erased(predicted(), REGION, &[data(pos(5, 5)), data(Health(7))]).unwrap(),
        );
        assert!(twin.erased.get::<PredictedOrigin>(predicted()).is_some());
        twin.both(|w| w.bind(origin(), net_50()).unwrap());

        // A bound `Net` key in a reference field is stored as the origin key.
        let to_bound = || (ThrownBy { thrower: to(bound()) }, OwnedBy { owner: to(bound()) });
        twin.step(
            |w| w.spawn(net(3), REGION, to_bound()).unwrap(),
            |w| {
                let (thrown, owned) = to_bound();
                w.spawn_erased(net(3), REGION, &[data(thrown), data(owned)]).unwrap();
            },
        );
        assert_eq!(
            twin.erased.get_erased(net(3), ThrownBy::KEY),
            Some(&bytes(ThrownBy { thrower: to(predicted()) })[..])
        );

        // Inserts: an added component, overwrites of an untracked and a tracked one.
        twin.step(
            |w| w.insert(net(1), Mass(lactose_math::Scalar::TWO)).unwrap(),
            |w| assert!(w.insert_erased(net(1), &data(Mass(lactose_math::Scalar::TWO))).unwrap()),
        );
        twin.step(
            |w| w.insert(net(1), vel(0, 1)).unwrap(),
            |w| assert!(w.insert_erased(net(1), &data(vel(0, 1))).unwrap()),
        );
        twin.step(
            |w| w.insert(net(1), Health(90)).unwrap(),
            |w| assert!(w.insert_erased(net(1), &data(Health(90))).unwrap()),
        );
        twin.step(
            |w| w.insert(net(1), pos(8, 8)).unwrap(),
            |w| assert!(w.insert_erased(net(1), &data(pos(8, 8))).unwrap()),
        );

        // Equal bytes: no write, no stamp, no bump of the world's point.
        let before = state(&twin.erased, &twin.marks);
        assert!(!twin.erased.insert_erased(net(1), &data(pos(8, 8))).unwrap());
        assert!(!twin.erased.insert_erased(net(1), &data(vel(0, 1))).unwrap());
        assert!(!twin.erased.insert_erased(net(2), &data(ClaimedBy { player: PLAYER })).unwrap());
        assert_eq!(state(&twin.erased, &twin.marks), before);

        // Relationships: a claim added and moved, an owner and a reference under the bound key.
        twin.step(
            |w| w.insert(net(1), ClaimedBy { player: PLAYER }).unwrap(),
            |w| assert!(w.insert_erased(net(1), &data(ClaimedBy { player: PLAYER })).unwrap()),
        );
        twin.step(
            |w| w.insert(net(2), ClaimedBy { player: OTHER_PLAYER }).unwrap(),
            |w| {
                let claim = data(ClaimedBy { player: OTHER_PLAYER });
                assert!(w.insert_erased(net(2), &claim).unwrap());
            },
        );
        twin.step(
            |w| w.insert(net(1), OwnedBy { owner: to(bound()) }).unwrap(),
            |w| assert!(w.insert_erased(net(1), &data(OwnedBy { owner: to(bound()) })).unwrap()),
        );
        twin.step(
            |w| w.insert(net(2), ThrownBy { thrower: to(bound()) }).unwrap(),
            |w| {
                let thrown = data(ThrownBy { thrower: to(bound()) });
                assert!(w.insert_erased(net(2), &thrown).unwrap());
            },
        );
        // A write through the bound key reaches the predicted entity.
        twin.step(
            |w| w.insert(bound(), Health(1)).unwrap(),
            |w| assert!(w.insert_erased(bound(), &data(Health(1))).unwrap()),
        );
        assert_eq!(twin.erased.get::<Health>(predicted()), Some(&Health(1)));
        assert_eq!(twin.erased.owned_by(predicted()).collect::<Vec<_>>(), [net(1), net(3)]);
        assert_eq!(twin.erased.claimed(OTHER_PLAYER).collect::<Vec<_>>(), [net(2)]);

        // Removals, through the origin and the bound key, and of an absent component.
        twin.step(
            |w| assert_eq!(w.remove::<Counter>(net(1)).unwrap(), Some(Counter(0))),
            |w| assert!(w.remove_erased(net(1), Counter::KEY).unwrap()),
        );
        twin.step(
            |w| assert!(w.remove::<ClaimedBy>(net(2)).unwrap().is_some()),
            |w| assert!(w.remove_erased(net(2), ClaimedBy::KEY).unwrap()),
        );
        twin.step(
            |w| assert!(w.remove::<OwnedBy>(net(3)).unwrap().is_some()),
            |w| assert!(w.remove_erased(net(3), OwnedBy::KEY).unwrap()),
        );
        twin.step(
            |w| assert!(w.remove::<Health>(bound()).unwrap().is_some()),
            |w| assert!(w.remove_erased(bound(), Health::KEY).unwrap()),
        );
        twin.step(
            |w| assert_eq!(w.remove::<Counter>(net(1)).unwrap(), None),
            |w| assert!(!w.remove_erased(net(1), Counter::KEY).unwrap()),
        );

        // The indices drive a despawn: the owner takes what it owns, transitively (net(2) is
        // owned by net(1)), and leaves what it no longer owns.
        twin.both(|w| w.despawn(predicted()).unwrap());
        assert!(!twin.erased.contains(net(1)), "the owned entity outlived its owner");
        assert!(!twin.erased.contains(net(2)), "the entity net(1) owned outlived it");
        assert!(twin.erased.contains(net(3)), "an entity whose ownership was removed went");
    }
}

/// Counts, per entity, the ticks in which it saw its `Health` change.
#[system(name = "test::see_health", worlds = [server])]
fn see_health(_changed: Changed<Health>, seen: Mut<Counter>) {
    seen.0 += 1;
}

#[test]
fn an_equal_write_is_not_seen_by_a_changed_reader_and_a_different_one_is_seen_once() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn_erased(net(1), REGION, &[data(Health(100)), data(Counter(0))]).unwrap();
    world.spawn_erased(net(2), REGION, &[data(Health(50)), data(Counter(0))]).unwrap();
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("main", &[]).add(&SEE_HEALTH);
    let mut schedule = builder.build().unwrap();
    world = tick(&mut schedule, world, &executor);
    let seen = |world: &World| [net(1), net(2)].map(|key| world.get::<Counter>(key).unwrap().0);
    let first = seen(&world);

    let mark = world.mark();
    assert!(!world.insert_erased(net(1), &data(Health(100))).unwrap());
    assert_eq!(world.changed_since::<Health>(mark).unwrap().count(), 0);
    assert!(world.insert_erased(net(2), &data(Health(51))).unwrap());
    let changed: Vec<StableKey> =
        world.changed_since::<Health>(mark).unwrap().map(|(key, _)| key).collect();
    assert_eq!(changed, [net(2)]);

    world = tick(&mut schedule, world, &executor);
    assert_eq!(seen(&world), [first[0], first[1] + 1]);
    world = tick(&mut schedule, world, &executor);
    assert_eq!(seen(&world), [first[0], first[1] + 1], "a change is seen once");
}

/// A world holding `net(1)` with a relationship and a claim, and `net(2)`.
fn refusal_world() -> World {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    let components = [
        data(pos(1, 1)),
        data(Health(10)),
        data(ClaimedBy { player: PLAYER }),
        data(ThrownBy { thrower: to(net(2)) }),
    ];
    world.spawn_erased(net(1), REGION, &components).unwrap();
    world.spawn_erased(net(2), REGION, &[data(Bomb { fuse: 3 })]).unwrap();
    world
}

#[test]
fn every_refusal_has_its_kind_and_changes_nothing() {
    let mut world = refusal_world();
    let marks = [world.mark()];
    let before = state(&world, &marks);
    let unregistered = ComponentKey::from_name("test::never_registered");
    let placeholder = data(ThrownBy { thrower: to(StableKey::Placeholder(0)) });
    let in_region = data(InRegion { region: REGION });
    let origin_data = data(PredictedOrigin { player: PLAYER, input_tick: t(1), ordinal: 1 });
    let mut short = data(Health(1));
    short.bytes.pop();
    let mut long = data(Health(1));
    long.bytes.push(0);
    let bad_enum = ComponentData { component: HeldItem::KEY, bytes: vec![9] };
    let mut bad_bool = data(PlayerInput {
        aim: lactose_math::Vec2::new(lactose_math::Scalar::ONE, lactose_math::Scalar::ONE),
        throw: true,
        _pad: pad(),
    });
    let throw = bad_bool.bytes.len() - 8;
    bad_bool.bytes[throw] = 2;
    let resource = resource_data(Weather(1));

    let insert_refusals: [(&ComponentData, ErrorKind); 9] = [
        (&ComponentData { component: unregistered, bytes: vec![0; 4] }, ErrorKind::NotFound),
        (&resource, ErrorKind::InvalidData),
        (&in_region, ErrorKind::InvalidData),
        (&origin_data, ErrorKind::InvalidData),
        (&short, ErrorKind::InvalidData),
        (&long, ErrorKind::InvalidData),
        (&placeholder, ErrorKind::InvalidData),
        (&bad_enum, ErrorKind::InvalidData),
        (&bad_bool, ErrorKind::InvalidData),
    ];
    for (entry, kind) in insert_refusals {
        assert_eq!(world.insert_erased(net(1), entry).unwrap_err().kind(), kind, "{entry:?}");
        assert_eq!(state(&world, &marks), before, "{entry:?}");
        // The same entry among valid ones refuses the whole spawn.
        let components = [data(pos(0, 0)), entry.clone()];
        let error = world.spawn_erased(net(3), REGION, &components).unwrap_err();
        assert_eq!(error.kind(), kind, "{entry:?}");
        assert_eq!(state(&world, &marks), before, "{entry:?}");
    }
    assert_eq!(
        world.insert_erased(net(9), &data(Health(1))).unwrap_err().kind(),
        ErrorKind::NotFound,
        "a missing entity"
    );
    assert_eq!(state(&world, &marks), before);

    let remove_refusals = [
        (net(1), unregistered, ErrorKind::NotFound),
        (net(1), Weather::KEY, ErrorKind::InvalidData),
        (net(1), InRegion::KEY, ErrorKind::InvalidData),
        (net(1), PredictedOrigin::KEY, ErrorKind::InvalidData),
        (net(9), Health::KEY, ErrorKind::NotFound),
    ];
    for (key, component, kind) in remove_refusals {
        assert_eq!(world.remove_erased(key, component).unwrap_err().kind(), kind);
        assert_eq!(state(&world, &marks), before);
    }
    assert!(!world.remove_erased(net(2), Health::KEY).unwrap(), "an absent component");
    assert_eq!(state(&world, &marks), before);

    let spawn_refusals: [(StableKey, RegionId, Vec<ComponentData>, ErrorKind); 5] = [
        (net(1), REGION, vec![data(pos(0, 0))], ErrorKind::InvalidData),
        (StableKey::Placeholder(0), REGION, vec![data(pos(0, 0))], ErrorKind::InvalidData),
        (net(3), REGION, vec![data(pos(0, 0)), data(pos(1, 1))], ErrorKind::InvalidData),
        (net(3), OTHER_REGION, vec![data(pos(0, 0))], ErrorKind::NotFound),
        (net(3), REGION, vec![in_region.clone()], ErrorKind::InvalidData),
    ];
    for (key, region, components, kind) in spawn_refusals {
        assert_eq!(world.spawn_erased(key, region, &components).unwrap_err().kind(), kind);
        assert_eq!(state(&world, &marks), before, "{key:?} {components:?}");
    }
}

/// Where the typed and erased writes part (review round 1, MINOR-3): a placeholder in a
/// reference field, which a typed write stores empty and an erased one refuses by name; and
/// a refused typed insert, which now leaves the point as a refused erased one does.
#[test]
fn a_placeholder_is_refused_by_name_and_a_refused_typed_insert_changes_nothing() {
    let mut world = refusal_world();
    let marks = [world.mark()];
    let before = state(&world, &marks);
    let placeholder = ThrownBy { thrower: to(StableKey::Placeholder(7)) };
    let expected = "placeholder reference in `thrower` of `";

    let error = world.insert_erased(net(1), &data(placeholder)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains(expected), "{error}");
    let components = [data(pos(0, 0)), data(placeholder)];
    let error = world.spawn_erased(net(3), REGION, &components).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains(expected), "{error}");
    assert_eq!(state(&world, &marks), before);

    // A typed insert of an unregistered component, or of anything to a missing entity, is
    // refused before the point moves.
    let error = world.insert(net(1), Unregistered(1)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert_eq!(world.mark(), marks[0]);
    assert_eq!(state(&world, &marks), before);
    assert_eq!(world.insert(net(9), Health(1)).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(state(&world, &marks), before);

    // A typed write resolves a placeholder outside its buffer to the empty reference.
    world.insert(net(1), placeholder).unwrap();
    assert_eq!(world.get::<ThrownBy>(net(1)), Some(&ThrownBy { thrower: EntityRef::NONE }));
}

#[test]
fn padding_is_zeroed_before_validation_and_matches_the_typed_value() {
    let input = PlayerInput {
        aim: lactose_math::Vec2::new(lactose_math::Scalar::ONE, lactose_math::Scalar::TWO),
        throw: true,
        _pad: pad(),
    };
    let mut padded = data(input);
    let last = padded.bytes.len() - 1;
    padded.bytes[last] = 0xFF;
    let mut erased = world(WorldKind::Server, LayoutConfig::default());
    let mut typed = world(WorldKind::Server, LayoutConfig::default());
    erased.spawn_erased(net(1), REGION, &[data(pos(0, 0))]).unwrap();
    typed.spawn(net(1), REGION, (pos(0, 0),)).unwrap();
    assert!(erased.insert_erased(net(1), &padded).unwrap());
    typed.insert(net(1), input).unwrap();
    assert_eq!(erased.get_erased(net(1), PlayerInput::KEY), Some(&bytes(input)[..]));
    assert_eq!(state(&erased, &[]), state(&typed, &[]));
    // The same value with other padding is equal bytes once zeroed.
    assert!(!erased.insert_erased(net(1), &padded).unwrap());
}

#[test]
fn get_erased_reads_the_stored_bytes_through_a_restore_and_a_snapshot_round_trip() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    world.spawn_erased(predicted(), REGION, &[data(pos(1, 1))]).unwrap();
    world.bind(origin(), net_50()).unwrap();
    let thrown = ThrownBy { thrower: to(bound()) };
    world.spawn_erased(net(1), REGION, &[data(pos(2, 2)), data(thrown)]).unwrap();
    let stored_thrown = bytes(ThrownBy { thrower: to(predicted()) });

    assert_eq!(world.get_erased(net(1), Position::KEY), Some(&bytes(pos(2, 2))[..]));
    assert_eq!(world.get_erased(net(1), ThrownBy::KEY), Some(&stored_thrown[..]));
    assert_eq!(
        world.get_erased(bound(), Position::KEY),
        world.get_erased(predicted(), Position::KEY)
    );
    assert_eq!(
        world.get_erased(net(1), InRegion::KEY),
        Some(&bytes(InRegion { region: REGION })[..])
    );
    assert_eq!(world.get_erased(net(1), Health::KEY), None, "an absent component");
    assert_eq!(world.get_erased(net(9), Position::KEY), None, "a missing entity");
    assert_eq!(world.get_erased(net(1), ComponentKey::from_name("test::never_registered")), None);

    let snapshot = world.snapshot();
    assert!(world.insert_erased(net(1), &data(pos(3, 3))).unwrap());
    assert!(world.remove_erased(net(1), ThrownBy::KEY).unwrap());
    world.spawn_erased(net(2), REGION, &[data(Health(4))]).unwrap();
    assert_eq!(world.get_erased(net(1), Position::KEY), Some(&bytes(pos(3, 3))[..]));

    world.restore(&snapshot).unwrap();
    assert_eq!(world.get_erased(net(1), Position::KEY), Some(&bytes(pos(2, 2))[..]));
    assert_eq!(world.get_erased(net(1), ThrownBy::KEY), Some(&stored_thrown[..]));
    assert_eq!(world.get_erased(net(2), Health::KEY), None);
    assert_eq!(world.referrers(predicted()).map(|r| r.source).collect::<Vec<_>>(), [net(1)]);

    // A snapshot of the restored world restores to the same bytes again.
    let again = world.snapshot();
    assert!(world.insert_erased(bound(), &data(pos(7, 7))).unwrap());
    world.restore(&again).unwrap();
    assert_eq!(world.get_erased(predicted(), Position::KEY), Some(&bytes(pos(1, 1))[..]));
    assert_eq!(world.get_erased(net(1), Position::KEY), Some(&bytes(pos(2, 2))[..]));
    assert_eq!(world.get_erased(net(1), ThrownBy::KEY), Some(&stored_thrown[..]));
}
