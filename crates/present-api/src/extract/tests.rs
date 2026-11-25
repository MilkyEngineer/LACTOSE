//! The snapshot's fill that keeps its order (D164) against `present-impl`'s, which sorted
//! every column and the keys at every pass that wrote: in a unit test, since what it compares
//! is private. Every pass of a test world runs both on the same world at the same point,
//! through [`Hooks::observe`], and the buffer the pass wrote must hold the rows the reference
//! writes, and the pass must write exactly when the reference would. The worlds take random
//! sequences of moves, spawns, despawns, extracted components added and removed, an
//! archetype change that keeps a row at its chunk and row, region loads and unloads, restores
//! and `set_tick`, under the reader cadences of the triple buffer's test, at the production
//! layout, at chunk capacities alone, and under every layout mode SOUR runs. The count of
//! passes that sorted is asserted to be zero in a steady scene where the layout keeps rows
//! in place, and printed under SOUR's modes, which permute rows at every sync point.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, PlayerId, StableKey, Tick};
use lactose_ecs::{
    ChangeMark, ChunkRef, ClaimedBy, ErasedColumn, Executor, LayoutConfig, LogEntry, RegionConfig,
    RegionId, RegionMode, StructuralChange, World, WorldKind, WorldSnapshot,
};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{Teleported, Transform};
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{Simulation, SimulationBuilder, TickInput, TypeTable};

use super::{ExtractState, Presented};
use crate::{CameraRig, Decal, Light, MeshInstance, PresentSnapshot, SnapshotReader};

// ---- The reference and the hooks ----

/// What a pass of a unit test compares, beside extract's own state.
#[derive(Debug)]
pub(super) struct Hooks {
    /// The keys of the presented rows the reference wrote last, ascending: `present-impl`'s
    /// kept state.
    keys: Vec<StableKey>,
    /// The passes with a writer installed, and those of them that wrote a snapshot.
    passes: u64,
    writes: u64,
    /// What differed, at most a few.
    mismatches: Vec<String>,
    mismatched: u64,
}

impl Hooks {
    pub(super) const fn new() -> Self {
        Self { keys: Vec::new(), passes: 0, writes: 0, mismatches: Vec::new(), mismatched: 0 }
    }

    /// The reference's half of a pass over the same world: whether it writes a snapshot, and
    /// if so what, against what the pass did, `written` being the buffer it wrote, if it did.
    #[expect(clippy::too_many_arguments, reason = "the snapshot half's inputs, test only")]
    pub(super) fn observe(
        &mut self,
        presented: &Presented,
        chunks: &[ChunkRef<'_>],
        since: Option<ChangeMark>,
        entries: &[LogEntry],
        full: bool,
        follows: bool,
        tick: Tick,
        written: Option<&PresentSnapshot>,
    ) {
        self.passes += 1;
        let changed = since.is_some_and(|since| {
            !follows && reference_changed(presented, chunks, since, entries, &self.keys)
        });
        let write = full || follows || changed;
        let mismatch = match (write, written) {
            (false, None) => None,
            (true, Some(written)) => {
                self.writes += 1;
                let mut reference = PresentSnapshot::empty();
                self.keys = reference_fill(presented, &mut reference, chunks, tick, full);
                (!written.same_rows(&reference))
                    .then(|| format!("wrote {written:?}, the reference {reference:?}"))
            }
            (write, written) => {
                Some(format!("wrote {}, the reference writes {write}", written.is_some()))
            }
        };
        if let Some(mismatch) = mismatch {
            self.mismatched += 1;
            if self.mismatches.len() < 3 {
                self.mismatches.push(format!("pass {}: {mismatch}", self.passes));
            }
        }
    }
}

/// `present-impl`'s `Presented::changed`: the keys of the presented rows sorted at each call.
fn reference_changed(
    presented: &Presented,
    chunks: &[ChunkRef<'_>],
    since: ChangeMark,
    entries: &[LogEntry],
    previous: &[StableKey],
) -> bool {
    let stamped = chunks.iter().any(|chunk| {
        chunk
            .components()
            .filter(|&component| presented.copies(component))
            .any(|component| super::any_changed(chunk.column(component), since))
    });
    if stamped {
        return true;
    }
    if entries.is_empty() {
        return false;
    }
    let keys = keys_of(chunks);
    keys != previous
        || entries.iter().any(|entry| match entry.change {
            StructuralChange::Removed { key, component } => {
                presented.copies(component) && keys.binary_search(&key).is_ok()
            }
            _ => false,
        })
}

/// `present-impl`'s `Presented::fill`: a list of rows for each copied column, each list sorted
/// by key, and the keys sorted once more.
fn reference_fill(
    presented: &Presented,
    snapshot: &mut PresentSnapshot,
    chunks: &[ChunkRef<'_>],
    tick: Tick,
    full: bool,
) -> Vec<StableKey> {
    let columns: Vec<ErasedColumn<'_>> = chunks
        .iter()
        .flat_map(|chunk| {
            chunk
                .components()
                .filter(|&component| presented.copies(component))
                .filter_map(|component| chunk.column(component))
        })
        .collect();
    let mut rows: BTreeMap<lactose_core::ComponentKey, Vec<(StableKey, &[u8])>> = BTreeMap::new();
    for column in &columns {
        let size = column.element_size();
        let values = rows.entry(column.component()).or_default();
        for (row, &key) in column.keys().iter().enumerate() {
            values.push((key, &column.bytes()[row * size..(row + 1) * size]));
        }
    }
    for values in rows.values_mut() {
        values.sort_unstable_by_key(|&(key, _)| key);
    }
    let components: Vec<_> = rows.keys().copied().collect();
    let cut = full && presented.kind != WorldKind::Prediction;
    snapshot.fill(presented.kind, tick, cut, &components, |component, keys, bytes| {
        for &(key, value) in &rows[&component] {
            keys.push(key);
            bytes.extend_from_slice(value);
        }
    });
    keys_of(chunks)
}

/// The keys of `chunks`' rows, ascending.
fn keys_of(chunks: &[ChunkRef<'_>]) -> Vec<StableKey> {
    let mut keys: Vec<StableKey> =
        chunks.iter().flat_map(|chunk| chunk.keys().iter().copied()).collect();
    keys.sort_unstable();
    keys
}

// ---- A world to drive ----

const REGION: RegionId = RegionId(1);
const OTHER_REGION: RegionId = RegionId(2);
const START: Tick = Tick(100);

fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::runtime(NonZeroU64::new(n).expect("non-zero")).expect("a key"))
}

fn at(x: i32) -> Transform {
    Transform::at(Vec3::new(Scalar::from_i32(x), Scalar::ZERO, Scalar::ZERO))
}

fn mesh(n: u64) -> MeshInstance {
    MeshInstance::new(AssetRef::new(AssetId::from_bits(n)))
}

fn light(n: u64) -> Light {
    Light::point(Vec3::ZERO, Scalar::from_i32(i32::try_from(n % 1000).expect("small")), Scalar::ONE)
}

fn decal(n: u64) -> Decal {
    Decal::new(AssetRef::new(AssetId::from_bits(n)), Vec3::ZERO)
}

fn claim() -> ClaimedBy {
    ClaimedBy { player: PlayerId(1) }
}

fn region(kind: WorldKind, id: RegionId) -> RegionConfig {
    let mode =
        if kind == WorldKind::Client { RegionMode::Replicated } else { RegionMode::Simulated };
    RegionConfig { id, seed: 10 + id.0, mode }
}

/// A world of `kind` with extract placed and both regions, with a writer installed and no
/// sink: the snapshot's half alone runs.
struct Scene {
    simulation: Option<Simulation>,
    executor: Executor,
    reader: SnapshotReader,
}

impl Scene {
    fn new(kind: WorldKind, layout: LayoutConfig) -> Self {
        let mut types = TypeTable::new();
        lactose_physics_api::register_types(&mut types).expect("physics registers");
        crate::register_types(&mut types).expect("present registers");
        let mut simulation = SimulationBuilder::new(kind, types)
            .layout(layout)
            .tick(START)
            .setup(crate::setup)
            .systems(crate::register_systems)
            .build()
            .expect("the world builds");
        for id in [REGION, OTHER_REGION] {
            simulation.world_mut().create_region(region(kind, id)).expect("a region");
        }
        let (writer, reader) = crate::snapshot_channel();
        drop(crate::extract_state(&mut simulation).expect("setup ran").set_snapshot(Some(writer)));
        let executor = Executor::builder()
            .frame_workers(2)
            .background_workers(1)
            .build()
            .expect("an executor");
        Self { simulation: Some(simulation), executor, reader }
    }

    fn world_mut(&mut self) -> &mut World {
        self.simulation.as_mut().expect("between runs").world_mut()
    }

    fn state(&mut self) -> &mut ExtractState {
        crate::extract_state(self.simulation.as_mut().expect("between runs")).expect("setup ran")
    }

    fn step(&mut self) {
        let simulation = self.simulation.take().expect("between runs");
        let run = simulation.step(&self.executor, TickInput::default());
        self.simulation = Some(self.executor.block_on(run).expect("the step runs"));
    }

    fn pass(&mut self) {
        let simulation = self.simulation.take().expect("between runs");
        let run = simulation.run_passes(&self.executor);
        self.simulation = Some(self.executor.block_on(run).expect("the passes run"));
    }

    fn publish(&mut self) {
        let events = self.world_mut().take_present_events();
        self.state().publish(events);
    }

    fn acquire(&mut self) {
        let _ = self.reader.acquire();
    }

    /// What the passes so far compared.
    fn hooks(&mut self) -> &Hooks {
        &self.state().test
    }

    /// Asserts that every pass so far wrote what the reference writes, and when.
    fn assert_matched(&mut self, what: &str) {
        let hooks = self.hooks();
        assert_eq!(hooks.mismatched, 0, "{what}: {:#?}", hooks.mismatches);
    }

    fn sorts(&mut self) -> u64 {
        self.state().presented.sorts
    }
}

/// A small deterministic generator: the test needs no crate for it.
struct Draw(u64);

impl Draw {
    fn next(&mut self, below: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % below
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.next(one_in) == 0
    }
}

/// The layouts the equality test runs under: the production layout, chunk capacities alone,
/// and every layout mode SOUR runs, two seeds each where the seed shuffles.
fn layouts() -> Vec<(String, LayoutConfig)> {
    let mut layouts = vec![("production".to_owned(), LayoutConfig::default())];
    for rows in [1, 7, 64] {
        let layout = LayoutConfig { chunk_rows: Some(rows), ..LayoutConfig::default() };
        layouts.push((format!("{rows} rows alone"), layout));
    }
    for capacity in [ChunkCapacity::Rows(1), ChunkCapacity::Rows(7), ChunkCapacity::Rows(64)] {
        for seed in [1, 2] {
            let layout = layout_mode(capacity, seed).expect("a SOUR layout");
            layouts.push((format!("SOUR {capacity:?} seed {seed}"), layout));
        }
    }
    let bytes = layout_mode(ChunkCapacity::Bytes(16 * 1024), 3).expect("a SOUR layout");
    layouts.push(("SOUR Bytes(16384) seed 3".to_owned(), bytes));
    layouts
}

/// One of the triple buffer test's reader cadences, or a drawn one: `W` a frame's work and its
/// pass, `P` a publication, `A` an acquire.
const CADENCES: [&str; 6] = ["WPA", "WPWPA", "WP", "WPWAP", "WAP", ""];

/// The live keys a world holds, in the order they were spawned.
struct Script {
    draw: Draw,
    next: u64,
    live: Vec<u64>,
    saved: Option<(WorldSnapshot, Vec<u64>)>,
}

impl Script {
    fn new(seed: u64) -> Self {
        Self {
            draw: Draw(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
            next: 1,
            live: Vec::new(),
            saved: None,
        }
    }

    fn pick(&mut self) -> Option<u64> {
        let len = u64::try_from(self.live.len()).expect("few rows");
        (len > 0).then(|| self.live[usize::try_from(self.draw.next(len)).expect("small")])
    }

    /// Spawns a row of a drawn shape: an instance alone, a light with or without a mesh, a
    /// decal with a mesh, a light with `Teleported`; in a prediction world, mostly claimed.
    fn spawn(&mut self, world: &mut World) {
        let n = self.next;
        self.next += 1;
        let region = if self.draw.chance(2) { REGION } else { OTHER_REGION };
        let x = i32::try_from(n).expect("few rows");
        let spawned = match self.draw.next(5) {
            0 => world.spawn(key(n), region, (at(x), mesh(n))),
            1 => world.spawn(key(n), region, (at(x), light(n))),
            2 => world.spawn(key(n), region, (at(x), mesh(n), light(n))),
            3 => world.spawn(key(n), region, (at(x), mesh(n), decal(n))),
            _ => world.spawn(
                key(n),
                region,
                (at(x), light(n), Teleported(Tick(u32::try_from(n).expect("few rows")))),
            ),
        };
        spawned.expect("a spawn");
        if world.kind() == WorldKind::Prediction && !self.draw.chance(3) {
            world.insert(key(n), claim()).expect("a live row");
        }
        self.live.push(n);
    }

    /// Adds `T` to the row, or removes it if the row has one.
    fn toggle<T: lactose_reflect::Component>(world: &mut World, n: u64, value: T) {
        if world.get::<T>(key(n)).is_some() {
            world.remove::<T>(key(n)).expect("a live row");
        } else {
            world.insert(key(n), value).expect("a live row");
        }
    }

    /// A frame's changes between ticks.
    fn edit(&mut self, world: &mut World) {
        for _ in 0..self.draw.next(5) {
            match self.draw.next(12) {
                0..=2 => self.spawn(world),
                3 => {
                    if let Some(n) = self.pick() {
                        world.despawn(key(n)).expect("a live row");
                        self.live.retain(|&live| live != n);
                    }
                }
                4 | 5 => {
                    if let Some(n) = self.pick() {
                        let step = i32::try_from(self.draw.next(3)).expect("small") - 1;
                        let mut transform = world.get_mut::<Transform>(key(n)).expect("a row");
                        transform.translation.x += Scalar::from_i32(step);
                    }
                }
                6 => {
                    if let Some(n) = self.pick()
                        && let Some(mut light) = world.get_mut::<Light>(key(n))
                    {
                        light.intensity += Scalar::ONE;
                    }
                }
                7 => {
                    if let Some(n) = self.pick() {
                        let tick = Tick(world.tick().0.wrapping_add(1));
                        Self::toggle(world, n, Teleported(tick));
                    }
                }
                8 => {
                    if let Some(n) = self.pick() {
                        Self::toggle(world, n, light(n));
                    }
                }
                9 => {
                    if let Some(n) = self.pick() {
                        Self::toggle(world, n, decal(n));
                    }
                }
                10 => {
                    if let Some(n) = self.pick() {
                        Self::toggle(world, n, mesh(n + 1000));
                    }
                }
                _ => {
                    if let Some(n) = self.pick() {
                        if world.kind() == WorldKind::Prediction && self.draw.chance(2) {
                            Self::toggle(world, n, claim());
                        } else {
                            Self::toggle(world, n, CameraRig::new(Vec3::ZERO, Scalar::ONE));
                        }
                    }
                }
            }
        }
        // Now and then what makes a pass full or rereads a region: an unload and a load, a
        // snapshot kept and restored, a tick set forward or back.
        match self.draw.next(16) {
            0 => {
                world.unload_region(OTHER_REGION).expect("loaded");
                world.create_region(region(world.kind(), OTHER_REGION)).expect("unloaded");
                self.live.retain(|&n| world.contains(key(n)));
            }
            1 => self.saved = Some((world.snapshot(), self.live.clone())),
            2 => {
                if let Some((snapshot, live)) = &self.saved {
                    world.restore(snapshot).expect("this world's snapshot");
                    self.live.clone_from(live);
                }
            }
            3 => {
                let tick = Tick(world.tick().0.wrapping_add(3));
                world.set_tick(tick).expect("between runs");
            }
            4 => {
                let tick = Tick(world.tick().0.wrapping_sub(2));
                world.set_tick(tick).expect("between runs");
            }
            _ => {}
        }
    }
}

/// Runs `frames` repetitions of `cadence` (a drawn one when empty) over a scripted world and
/// returns its counts.
fn run(kind: WorldKind, layout: LayoutConfig, cadence: &str, seed: u64, frames: u32) -> Scene {
    let mut scene = Scene::new(kind, layout);
    let mut script = Script::new(seed);
    for _ in 0..12 {
        script.spawn(scene.world_mut());
    }
    let mut ops: Vec<u8> = Vec::new();
    while ops.len() < usize::try_from(frames).expect("small") * 3 {
        if cadence.is_empty() {
            ops.push(b"WPA"[usize::try_from(script.draw.next(3)).expect("small")]);
        } else {
            ops.extend(cadence.bytes());
        }
    }
    for op in ops {
        match op {
            b'W' => {
                script.edit(scene.world_mut());
                if !script.draw.chance(3) {
                    scene.step();
                }
                scene.pass();
            }
            b'P' => scene.publish(),
            _ => scene.acquire(),
        }
    }
    scene
}

// ---- The tests ----

/// The buffer every pass writes holds the rows `present-impl`'s fill writes of the same world,
/// and a pass writes exactly when that fill would, under every layout, every reader cadence
/// and every world kind.
#[test]
fn the_fill_writes_what_present_impls_writes() {
    let mut totals = BTreeMap::new();
    for kind in [WorldKind::Server, WorldKind::Client, WorldKind::Prediction] {
        for (name, layout) in layouts() {
            for (index, cadence) in CADENCES.iter().enumerate() {
                for seed in 1..=2 {
                    let seed = seed * 31 + u64::try_from(index).expect("small");
                    let mut scene = run(kind, layout, cadence, seed, 40);
                    scene.assert_matched(&format!(
                        "{kind:?} {name} cadence {cadence:?} seed {seed}"
                    ));
                    let sorts = scene.sorts();
                    let &Hooks { passes, writes, .. } = scene.hooks();
                    assert!(writes > 0, "{kind:?} {name}: {writes} of {passes}");
                    let total = totals.entry(name.clone()).or_insert((0, 0, 0));
                    total.0 += passes;
                    total.1 += writes;
                    total.2 += sorts;
                }
            }
        }
    }
    for (name, (passes, writes, sorts)) in totals {
        println!("{name}: {passes} passes, {writes} wrote a snapshot, {sorts} sorted");
        assert!(writes < passes, "{name}: some idle pass wrote nothing");
    }
}

/// A presented row alone in its archetype that gains `Teleported` keeps its chunk and its row
/// in the new archetype: its place still holds, the pass sorts nothing, and the snapshot holds
/// its `Teleported`, read from the chunk.
#[test]
fn a_row_that_changes_archetype_in_place_is_read_from_its_chunk() {
    for layout in
        [LayoutConfig::default(), LayoutConfig { chunk_rows: Some(1), ..LayoutConfig::default() }]
    {
        let mut scene = Scene::new(WorldKind::Server, layout);
        let world = scene.world_mut();
        for n in 1..=2 {
            world.spawn(key(n), REGION, (at(0), light(n))).expect("a spawn");
        }
        world.spawn(key(3), REGION, (at(0), light(3), decal(3))).expect("a spawn");
        scene.step();
        scene.pass();
        scene.publish();
        let sorted = scene.sorts();
        scene.world_mut().insert(key(3), Teleported(Tick(START.0 + 1))).expect("a live row");
        scene.step();
        scene.pass();
        scene.publish();
        assert_eq!(scene.sorts(), sorted, "{layout:?}: the place held across the archetype");
        let frame = scene.reader.acquire().expect("published");
        assert_eq!(frame.newer().get::<Teleported>(key(3)), Some(Teleported(Tick(START.0 + 1))));
        scene.assert_matched(&format!("{layout:?}"));
    }
}

/// One place alone stops holding its key, the count unchanged: the lowest key's or the
/// highest's, a row alone in its chunk despawned and another spawned in its archetype, so that
/// one row takes the other's chunk and row. The pass sorts again and writes the new row.
#[test]
fn one_place_that_no_longer_holds_its_key_sorts_again() {
    for (alone, fresh) in [(1, 4), (3, 4)] {
        let mut scene = Scene::new(WorldKind::Server, LayoutConfig::default());
        let world = scene.world_mut();
        for n in (1..=3).filter(|&n| n != alone) {
            world.spawn(key(n), REGION, (at(0), light(n))).expect("a spawn");
        }
        world.spawn(key(alone), REGION, (at(0), light(alone), decal(alone))).expect("a spawn");
        scene.step();
        scene.pass();
        scene.publish();
        let sorted = scene.sorts();
        let world = scene.world_mut();
        world.despawn(key(alone)).expect("a live row");
        world.spawn(key(fresh), REGION, (at(0), light(fresh), decal(fresh))).expect("a spawn");
        scene.step();
        scene.pass();
        scene.publish();
        assert_eq!(scene.sorts(), sorted + 1, "key {alone}'s place alone stopped holding");
        let frame = scene.reader.acquire().expect("published");
        let keys: Vec<StableKey> = frame.newer().rows::<Light>().map(|(key, _)| key).collect();
        let expected: Vec<StableKey> = (1..=4).filter(|&n| n != alone).map(key).collect();
        assert_eq!(keys, expected);
        assert_eq!(frame.newer().get::<Decal>(key(fresh)), Some(decal(fresh)));
        scene.assert_matched(&format!("key {alone}"));
    }
}

/// A change made to a steady scene: its name, the change, and how many sorts it costs where
/// the layout keeps rows in place.
type Change = (&'static str, fn(&mut World), u64);

/// A steady scene, 200 presented rows in two archetypes and two regions, half of them moved at
/// every frame and none spawned or despawned: where the layout keeps rows in place (the
/// production layout, a chunk capacity alone) no pass after the first sorts. A pass after a
/// spawn, a despawn or an archetype change sorts once, and one after a row moved to another
/// region not at all. Under SOUR's modes, which permute the rows at every sync point, the
/// count is printed.
#[test]
fn a_steady_scene_sorts_nothing_where_the_layout_keeps_rows_in_place() {
    let mut layouts = layouts();
    // One seed of each SOUR mode is enough for a count that is printed.
    layouts.retain(|(name, _)| !name.ends_with("seed 2"));
    for (name, layout) in layouts {
        let mut scene = Scene::new(WorldKind::Server, layout);
        let world = scene.world_mut();
        for n in 1..=200 {
            let place = if n % 2 == 0 { REGION } else { OTHER_REGION };
            let x = i32::try_from(n).expect("small");
            if n % 3 == 0 {
                world.spawn(key(n), place, (at(x), mesh(n), decal(n))).expect("a spawn");
            } else {
                world.spawn(key(n), place, (at(x), light(n))).expect("a spawn");
            }
        }
        let frame = |scene: &mut Scene, moved: bool| {
            if moved {
                // Every odd row that is still there: key 7 is despawned below.
                for n in (1..=200).step_by(2) {
                    if let Some(mut transform) = scene.world_mut().get_mut::<Transform>(key(n)) {
                        transform.translation.x += Scalar::ONE;
                    }
                }
            }
            scene.step();
            scene.pass();
            scene.publish();
            scene.acquire();
        };
        frame(&mut scene, false);
        let first = scene.sorts();
        for _ in 0..30 {
            frame(&mut scene, true);
        }
        let steady = scene.sorts() - first;
        let sour = name.starts_with("SOUR");
        if sour {
            println!("{name}: sorted in {steady} of 30 steady passes");
        } else {
            assert_eq!(steady, 0, "{name}: sorted in {steady} of 30 steady passes");
        }
        // A row's region is a column of its chunk, so a row moved to another region keeps its
        // place; a row that changes archetype leaves a hole its chunk's last row fills.
        let changes: [Change; 4] = [
            (
                "a spawn",
                |world| world.spawn(key(500), REGION, (at(0), light(500))).expect("a spawn"),
                1,
            ),
            ("a despawn", |world| world.despawn(key(7)).expect("a live row"), 1),
            ("a new archetype", |world| world.insert(key(10), decal(10)).expect("a live row"), 1),
            (
                "a new region",
                |world| world.move_to_region(key(8), OTHER_REGION).expect("a live row"),
                0,
            ),
        ];
        for (change, apply, sorts) in changes {
            let before = scene.sorts();
            apply(scene.world_mut());
            frame(&mut scene, true);
            if !sour {
                assert_eq!(scene.sorts() - before, sorts, "{name}: sorts after {change}");
            }
        }
        scene.assert_matched(&name);
    }
}
