//! Extract (architecture section 5.3): the exclusive pass system in `Pass::Extract` that
//! copies what the render side needs out of a world between ticks. It writes the
//! instances whose placement changed to an [`InstanceSink`], which the renderer's staging
//! ring implements, and the rows presentation works on to the presentation snapshot
//! (`crate::snapshot`). [`ExtractState`] holds both outputs and the pass's marks, in a
//! service resource whose type is private to this crate; `app` reaches it through
//! [`extract_state`](crate::extract_state) to install the outputs and to publish each
//! snapshot between runs.
//!
//! The pass reads the world through `&World` and writes nothing to it: no column, no
//! resource, no event and no log entry. Every row it produces is keyed by `StableKey`, and
//! the one conversion to floats it makes is `lactose_math`'s.

use std::collections::BTreeMap;

use lactose_asset_api::AssetRef;
use lactose_core::{ComponentKey, StableKey, Tick};
use lactose_ecs::{
    ChangeMark, ChunkRef, ClaimedBy, ErasedColumn, Job, LogCursor, LogEntry, PresentEventBatch,
    ResMut, StructuralChange, World, WorldKind, system,
};
use lactose_math::render::RenderPosition;
use lactose_physics_api::{Teleported, Transform};
use lactose_reflect::{Reflect, TypeRole};

use crate::components::{MeshAsset, MeshInstance};
use crate::snapshot::SnapshotWriter;
use crate::{tick_after, tick_before};

/// A placement in render space (architecture section 12.4): the position as a grid cell
/// and an offset, the rotation as a unit quaternion of 32-bit floats. Disjoint from the
/// simulation's types as `lactose_math::render` is: [`from_sim`](Self::from_sim) is the
/// one way in, and there is no way back.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RenderTransform {
    /// Where the origin is.
    pub position: RenderPosition,
    /// How it is turned: `x`, `y`, `z`, `w`.
    pub rotation: [f32; 4],
}

impl RenderTransform {
    /// The render-space placement of a simulation transform: the position through
    /// `RenderPosition::from_sim`, each component of the rotation rounded to the nearest
    /// 32-bit float. The same transform gives the same bits on every platform.
    #[must_use]
    pub fn from_sim(transform: &Transform) -> Self {
        let [x, y, z, w] = transform.rotation.to_array();
        Self {
            position: RenderPosition::from_sim(transform.translation),
            rotation: [x.to_f32(), y.to_f32(), z.to_f32(), w.to_f32()],
        }
    }
}

/// One instance as extract hands it to the sink: the placement the row has now.
///
/// The row carries no previous placement. The sink holds, for the key, the placement it
/// held as current when the first pass of the current tick began (its base, the tick
/// before's state), and a write makes that the previous placement and the row's the
/// current one. When [`no_interpolate`](Self::no_interpolate) is set, or the sink has no
/// base for the key, the previous placement becomes the row's as well and the base is
/// kept. A pass of an earlier tick than the pass before it for its kind makes the sink
/// drop every base of that kind (`README.md`, "What the sink keeps").
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InstanceRow {
    /// The entity. The only identity a row has: a sink never learns a chunk or a row
    /// index.
    pub key: StableKey,
    /// The mesh the row's [`MeshInstance`](crate::MeshInstance) names.
    pub mesh: AssetRef<MeshAsset>,
    /// The row's [`Transform`](lactose_physics_api::Transform), in render space.
    pub transform: RenderTransform,
    /// Whether the placement is a jump and not a movement, so that nothing is drawn
    /// between the previous placement and this one (architecture section 5.3):
    /// `README.md`, "The no-interpolate flag", states when extract sets it.
    pub no_interpolate: bool,
}

/// What a pass tells the sink before anything else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExtractPass {
    /// The kind of the world the pass reads. A client's two worlds each have their own
    /// extract, and a sink given to both tells their passes apart by this, which is how
    /// the prediction world's copy of an entity shadows the replicated world's
    /// (architecture section 9.4).
    pub kind: WorldKind,
    /// The world's tick at the pass. The first pass of a tick other than the previous
    /// pass's is where the sink takes each key's base, the placement later writes of the
    /// tick are drawn from; at one earlier than the previous pass's, it drops them
    /// (`README.md`, "What the sink keeps").
    pub tick: Tick,
    /// Whether the pass is full: it writes every instance of the world, and an instance
    /// the sink holds for this kind of world that the pass does not write is gone. A pass
    /// is full when extract cannot tell what changed since its previous pass: the first
    /// pass after an output is installed or removed, the first after a restore, a
    /// whole-world load or the change-stamp rebase, and the first after the structural
    /// log trimmed a change logged since the previous pass (`README.md`, "Full and
    /// incremental passes").
    pub full: bool,
}

/// Where extract writes instances: implemented by the renderer over its staging ring and
/// its slot table, and by a test that counts.
///
/// One pass calls, in this order: [`begin`](Self::begin) once; [`removed`](Self::removed)
/// at most once; [`write`](Self::write) any number of times, from several threads at
/// once; [`end`](Self::end) once. A pass at a tick other than the previous pass's always
/// calls `begin` and `end`, with nothing between them if it has nothing to write, so the
/// ticks of the sink's `begin`s are those of extract's passes; a pass at the previous
/// pass's tick that is not full and has no key to remove and no row to write calls
/// nothing. Between `begin` and `end` the world does not change.
/// In a client's replicated world a pass writes no row: remote entities' placements are
/// the interpolation buffer's, and presentation uploads them (`README.md`, "The extract
/// set").
///
/// Nothing here fails. `write` runs on the executor's workers inside extract's scope, so it
/// never blocks and takes no lock: a sink that cannot take a row (a ring whose regions are
/// all in flight) drops it and counts the drop. Extract has nothing to do with an error.
pub trait InstanceSink: Send + Sync {
    /// A pass starts.
    fn begin(&mut self, pass: ExtractPass);

    /// The keys the structural log since the previous pass records as despawned (with an
    /// unloaded region or alone), or as having lost `Transform` or `MeshInstance`, or, in
    /// a prediction world, `ClaimedBy`. In ascending key order, each once, never empty.
    /// Called before any row is written, and never on a full pass, where the rows written
    /// say what remains.
    ///
    /// A key here may never have been an instance (a despawned light): the sink ignores a
    /// key it does not hold for the pass's kind of world. A key that left and came back
    /// since the previous pass is here, and the pass then writes its row: the sink drops
    /// what it held for the key, so the row arrives as a new instance with no previous
    /// placement and no base.
    fn removed(&mut self, keys: &[StableKey]);

    /// A batch of rows, never empty. Each instance the pass writes is in exactly one
    /// batch, once. Calls come from the pass's forked jobs, several at a time, so the
    /// sink takes `&self`; how the rows are batched and in what order the batches and
    /// their rows arrive follow the world's chunk layout and the executor, and mean
    /// nothing. A sink whose result must not depend on them orders what it keeps by key,
    /// in [`end`](Self::end).
    fn write(&self, rows: &[InstanceRow]);

    /// The pass is over: every call of [`write`](Self::write) has returned.
    fn end(&mut self);
}

/// Extract's state beside the world: the two outputs and the marks of the passes so far.
/// [`setup`](crate::setup) inserts it in every kind of world inside a service resource
/// whose type is private to this crate, which only the `present::extract` system names:
/// no system of a tick can name it, and `Res<ExtractState>` or `ResMut<ExtractState>`
/// finds no resource. `app` reaches it between runs through
/// [`extract_state`](crate::extract_state) to install the outputs and to
/// [`publish`](Self::publish). A test's own pass system calls [`extract`](Self::extract)
/// on a state the test owns.
///
/// With neither output installed a pass does nothing and keeps no mark, so a dedicated
/// server, which installs none, pays for the system's dispatch and nothing else.
pub struct ExtractState {
    sink: Option<Box<dyn InstanceSink>>,
    snapshot: Option<SnapshotWriter>,
    /// The mark and cursor of the previous pass; `None` before the first, and after an
    /// output is installed.
    last: Option<(ChangeMark, LogCursor)>,
    /// The mark the previous pass that followed a tick started from: rows stamped since
    /// it are written once more by the next such pass (`README.md`, "Settling").
    settle: Option<ChangeMark>,
    /// The world's tick at the previous pass. A pass at an earlier tick flags every row
    /// (`README.md`, "The no-interpolate flag").
    tick: Option<Tick>,
    /// The tick a `Teleported` is compared with: the previous pass's tick, or, at a pass of
    /// that same tick, what the previous pass compared with (`README.md`, "The
    /// no-interpolate flag").
    covered: Option<Tick>,
    /// The keys of the presented rows in the snapshot the last pass wrote, ascending: an idle
    /// pass writes a snapshot when a row entered or left them.
    presented: Vec<StableKey>,
}

impl ExtractState {
    /// A state with no output and no pass behind it.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sink: None,
            snapshot: None,
            last: None,
            settle: None,
            tick: None,
            covered: None,
            presented: Vec::new(),
        }
    }

    /// Installs `sink` as where instances are written, or removes the sink with `None`,
    /// and returns the one it replaces. The next pass is full. Between runs only: a pass
    /// holds the state for its whole run.
    pub fn set_sink(
        &mut self,
        sink: Option<Box<dyn InstanceSink>>,
    ) -> Option<Box<dyn InstanceSink>> {
        self.last = None;
        core::mem::replace(&mut self.sink, sink)
    }

    /// Installs `writer` as the snapshot channel this world's passes write, or removes it
    /// with `None`, and returns the one it replaces. The next pass is full. A writer
    /// serves one world for as long as it is installed: installing in one world a writer
    /// another world has written gives the reader a pair of snapshots of two worlds.
    pub fn set_snapshot(&mut self, writer: Option<SnapshotWriter>) -> Option<SnapshotWriter> {
        self.last = None;
        core::mem::replace(&mut self.snapshot, writer)
    }

    /// One pass over `world`: what the `present::extract` system runs, and what a pass
    /// system of a test's own calls. `README.md`, "Extract", is the contract: which rows
    /// are in the world's extract set, when a pass is full, which rows an incremental pass
    /// writes and which it writes once more, and what goes into the snapshot. It forks
    /// over the chunks inside `job`'s scope, takes `world` by shared reference and
    /// writes nothing to it, and does not fail.
    ///
    /// The snapshot a pass writes is not visible to the reader until
    /// [`publish`](Self::publish).
    pub fn extract(&mut self, world: &World, job: &mut Job<'_, '_>) {
        if self.sink.is_none() && self.snapshot.is_none() {
            return;
        }
        let (kind, tick, mark) = (world.kind(), world.tick(), world.mark());
        let log = world.structural_log();
        // Full when there is no previous pass, its mark is stale, or the log no longer reaches
        // its cursor; decided before any row is read.
        let previous = self.last.filter(|&(last, _)| world.is_current(last));
        let entries: Option<Vec<LogEntry>> =
            previous.and_then(|(_, mut cursor)| Some(log.since(&mut cursor)?.copied().collect()));
        let since = previous.map(|(last, _)| last).filter(|_| entries.is_some());
        let entries = entries.unwrap_or_default();
        let full = since.is_none();
        let follows = self.tick != Some(tick);
        let idle = !full && !follows;
        let earlier = self.tick.is_some_and(|previous| tick_before(tick, previous));
        let pass = Pass {
            kind,
            flag_all: earlier || idle || (full && kind == WorldKind::Server),
            covered: match self.tick {
                Some(previous) if previous == tick => self.covered,
                previous => previous,
            },
            // Rows stamped since the settling mark after a tick, since the previous pass
            // otherwise: rules (1) and (3) of "Full and incremental passes", the settling
            // mark being no later than the previous pass's. `None` is every row.
            from: if full {
                None
            } else if follows {
                self.settle
            } else {
                since
            },
        };

        if let Some(sink) = self.sink.as_deref_mut() {
            write_instances(world, job, sink, &pass, full, follows, &entries);
        }
        if let Some(writer) = &mut self.snapshot {
            let presented = Presented::new(world);
            let chunks: Vec<ChunkRef<'_>> =
                world.chunks().filter(|chunk| presented.holds(chunk)).collect();
            let changed = since.is_some_and(|since| {
                !follows && presented.changed(&chunks, since, &entries, &self.presented)
            });
            if full || follows || changed {
                let snapshot = writer.buffer(full);
                let keys = presented.fill(snapshot, &chunks, tick, full);
                self.presented = keys;
            }
        }

        self.settle = if full {
            // A prediction world's next pass that follows a tick writes every instance once
            // more ("Settling").
            (kind != WorldKind::Prediction).then_some(mark)
        } else if follows {
            since
        } else {
            self.settle
        };
        self.last = Some((mark, log.cursor()));
        self.covered = pass.covered;
        self.tick = Some(tick);
    }

    /// Publishes what the passes since the previous publication wrote, with the
    /// presentation events of the ticks they cover: the snapshot becomes the newest the
    /// reader can acquire, and `events` joins the events the reader's next acquire
    /// returns, in one exchange, so a frame never holds a snapshot without the events of
    /// its ticks. `app` calls it once per frame, after `Simulation::run_passes` and with
    /// the batch `World::take_present_events` returned after it. A snapshot of a tick
    /// earlier than the one published last is published as a cut, and one a full pass
    /// in a prediction world wrote at the tick of the one published last takes its place
    /// (`README.md`, "The triple buffer" and "Events and publication").
    ///
    /// If no pass wrote a snapshot since the previous publication (no tick ended and
    /// nothing changed), the events alone are published. With no writer installed, or
    /// once the channel's reading half is dropped, the batch is dropped: nothing presents
    /// this world, and nothing accumulates for a reader that will never acquire.
    pub fn publish(&mut self, events: PresentEventBatch) {
        if let Some(writer) = &mut self.snapshot {
            writer.publish(events);
        }
    }
}

impl Default for ExtractState {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for ExtractState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtractState")
            .field("sink", &self.sink.is_some())
            .field("snapshot", &self.snapshot.is_some())
            .field("last", &self.last)
            .field("settle", &self.settle)
            .field("tick", &self.tick)
            .field("covered", &self.covered)
            .field("presented", &self.presented.len())
            .finish()
    }
}

/// What one pass decided before reading a row.
struct Pass {
    kind: WorldKind,
    /// Every row the pass writes has `no_interpolate` set.
    flag_all: bool,
    /// The tick a row's `Teleported` is compared with, `None` before the first pass.
    covered: Option<Tick>,
    /// An incremental pass writes the instances whose `Transform` or `MeshInstance` was
    /// stamped since this mark; `None` writes every instance.
    from: Option<ChangeMark>,
}

impl Pass {
    /// Whether the instance enters the extract set's rows: in a prediction world, only a
    /// claimed row.
    fn in_set(&self, chunk: &ChunkRef<'_>) -> bool {
        self.kind != WorldKind::Prediction || chunk.column(ClaimedBy::KEY).is_some()
    }

    /// Whether `component` makes a row an instance of the set: losing it takes the row out,
    /// gaining it may bring the row in.
    fn instance_component(&self, component: ComponentKey) -> bool {
        component == Transform::KEY
            || component == MeshInstance::KEY
            || (self.kind == WorldKind::Prediction && component == ClaimedBy::KEY)
    }

    fn row(
        &self,
        key: StableKey,
        mesh: &MeshInstance,
        transform: &Transform,
        teleported: Option<&Teleported>,
    ) -> InstanceRow {
        let teleported = teleported.map_or(Tick(0), |teleported| teleported.0);
        let teleport = teleported != Tick(0)
            && self.covered.is_some_and(|covered| tick_after(teleported, covered));
        InstanceRow {
            key,
            mesh: mesh.mesh,
            transform: RenderTransform::from_sim(transform),
            no_interpolate: self.flag_all || teleport,
        }
    }

    /// The rows of `chunk` the pass writes, appended to `out`, leaving out `forced`, which
    /// the pass writes in a batch of their own.
    fn chunk_rows(&self, chunk: &ChunkRef<'_>, forced: &[StableKey], out: &mut Vec<InstanceRow>) {
        let (Some(transforms), Some(meshes)) =
            (chunk.values::<Transform>(), chunk.values::<MeshInstance>())
        else {
            return;
        };
        let teleported = chunk.values::<Teleported>();
        let keys = chunk.keys();
        let rows = match self.from.and_then(|from| changed_rows(chunk, from)) {
            Some(rows) => rows,
            None => (0..keys.len()).collect(),
        };
        for row in rows {
            let key = keys[row];
            if forced.binary_search(&key).is_err() {
                let teleported = teleported.map(|values| &values[row]);
                out.push(self.row(key, &meshes[row], &transforms[row], teleported));
            }
        }
    }

    /// Whether `chunk`, an instance chunk of the set, holds a row the pass writes.
    fn writes_in(&self, chunk: &ChunkRef<'_>) -> bool {
        match self.from {
            None => !chunk.keys().is_empty(),
            Some(from) => [Transform::KEY, MeshInstance::KEY]
                .into_iter()
                .any(|component| any_changed(chunk.column(component), from)),
        }
    }
}

/// The rows of `chunk` whose `Transform` or `MeshInstance` was stamped since `from`,
/// ascending, or `None` if the mark is stale, which writes every row.
fn changed_rows(chunk: &ChunkRef<'_>, from: ChangeMark) -> Option<Vec<usize>> {
    let mut rows = Vec::new();
    for component in [Transform::KEY, MeshInstance::KEY] {
        let column = chunk.column(component)?;
        rows.extend(column.changed_since(from)?);
    }
    rows.sort_unstable();
    rows.dedup();
    Some(rows)
}

/// Whether a row of `column` was stamped since `from`. Its chunk is skipped by its column
/// stamp without reading a row when none was; a stale mark counts as a change.
fn any_changed(column: Option<ErasedColumn<'_>>, from: ChangeMark) -> bool {
    column.is_none_or(|column| {
        column.changed_since(from).is_none_or(|mut rows| rows.next().is_some())
    })
}

/// The sink's half of a pass: `begin`, `removed`, the writes from the pass's forked jobs,
/// `end`; nothing when an incremental pass at the previous pass's tick has nothing to say.
/// A pass that `follows` a tick brackets `begin` and `end` even with nothing between them:
/// the sink takes and drops its bases at the ticks of its `begin`s (`README.md`, "What a
/// pass gives the sink").
fn write_instances(
    world: &World,
    job: &mut Job<'_, '_>,
    sink: &mut dyn InstanceSink,
    pass: &Pass,
    full: bool,
    follows: bool,
    entries: &[LogEntry],
) {
    let mut removed: Vec<StableKey> = entries
        .iter()
        .filter_map(|entry| match entry.change {
            StructuralChange::Despawned { key, .. } => Some(key),
            StructuralChange::Removed { key, component } if pass.instance_component(component) => {
                Some(key)
            }
            _ => None,
        })
        .collect();
    removed.sort_unstable();
    removed.dedup();

    let writes = pass.kind != WorldKind::Client;
    // Rule (2): a row the log records as spawned or as having gained what makes it an
    // instance of the set, whatever its stamps say. Read by key and written in one batch;
    // the chunks' batches leave these keys out, so each row is written once.
    let mut forced_keys: Vec<StableKey> = entries
        .iter()
        .filter_map(|entry| match entry.change {
            StructuralChange::Spawned { key, .. } => Some(key),
            StructuralChange::Added { key, component } if pass.instance_component(component) => {
                Some(key)
            }
            _ => None,
        })
        .filter(|_| writes && !full)
        .collect();
    forced_keys.sort_unstable();
    forced_keys.dedup();
    let forced: Vec<InstanceRow> = forced_keys
        .iter()
        .filter_map(|&key| {
            let transform = world.get::<Transform>(key)?;
            let mesh = world.get::<MeshInstance>(key)?;
            if pass.kind == WorldKind::Prediction && world.get::<ClaimedBy>(key).is_none() {
                return None;
            }
            Some(pass.row(key, mesh, transform, world.get::<Teleported>(key)))
        })
        .collect();
    let forced_keys: Vec<StableKey> = forced.iter().map(|row| row.key).collect();

    let chunks: Vec<ChunkRef<'_>> = world
        .chunks()
        .filter(|chunk| {
            writes
                && pass.in_set(chunk)
                && chunk.column(Transform::KEY).is_some()
                && chunk.column(MeshInstance::KEY).is_some()
                && pass.writes_in(chunk)
        })
        .collect();
    if !full && !follows && removed.is_empty() && forced.is_empty() && chunks.is_empty() {
        return;
    }

    sink.begin(ExtractPass { kind: pass.kind, tick: world.tick(), full });
    if !removed.is_empty() {
        sink.removed(&removed);
    }
    let shared: &dyn InstanceSink = sink;
    let units = u32::try_from(chunks.len()).expect("fewer than 2^32 chunks");
    let (chunks, forced_keys) = (&chunks, &forced_keys);
    job.context().scope(|scope| {
        if !forced.is_empty() {
            scope.fork(move |_| shared.write(&forced));
        }
        if units > 0 {
            scope.fork_range(0..units, move |range, _| {
                let mut rows = Vec::new();
                for unit in range {
                    pass.chunk_rows(&chunks[unit as usize], forced_keys, &mut rows);
                }
                if !rows.is_empty() {
                    shared.write(&rows);
                }
            });
        }
    });
    sink.end();
}

/// The snapshot's half of a pass: which rows are presented rows, which of their components
/// it copies, and whether an idle pass has anything to write.
struct Presented {
    kind: WorldKind,
    /// The components the world's registry declares `extracted`, ascending.
    extracted: Vec<ComponentKey>,
}

impl Presented {
    fn new(world: &World) -> Self {
        let mut extracted: Vec<ComponentKey> = world
            .registry()
            .iter()
            .filter(|info| info.role() == TypeRole::Component && info.tracking().extracted)
            .map(|info| info.key())
            .collect();
        extracted.sort_unstable();
        Self { kind: world.kind(), extracted }
    }

    /// Whether the snapshot copies `component` of a presented row: every extracted one, but
    /// a `Client` world's transforms, which are the interpolation buffer's.
    fn copies(&self, component: ComponentKey) -> bool {
        self.extracted.binary_search(&component).is_ok()
            && !(self.kind == WorldKind::Client
                && (component == Transform::KEY || component == Teleported::KEY))
    }

    /// Whether `chunk`'s rows are presented rows of the extract set.
    fn holds(&self, chunk: &ChunkRef<'_>) -> bool {
        (self.kind != WorldKind::Prediction || chunk.column(ClaimedBy::KEY).is_some())
            && chunk.components().any(|component| {
                component != Transform::KEY
                    && component != Teleported::KEY
                    && component != MeshInstance::KEY
                    && self.extracted.binary_search(&component).is_ok()
            })
    }

    /// Whether an idle pass writes a snapshot: a presented row was stamped in a column the
    /// snapshot copies since `since`, or a row entered or left the presented rows (their
    /// keys are not `previous`, those of the snapshot last written), or a presented row lost
    /// a component the snapshot copies.
    fn changed(
        &self,
        chunks: &[ChunkRef<'_>],
        since: ChangeMark,
        entries: &[LogEntry],
        previous: &[StableKey],
    ) -> bool {
        let stamped = chunks.iter().any(|chunk| {
            chunk
                .components()
                .filter(|&component| self.copies(component))
                .any(|component| any_changed(chunk.column(component), since))
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
                    self.copies(component) && keys.binary_search(&key).is_ok()
                }
                _ => false,
            })
    }

    /// Writes the presented rows of `chunks` into `snapshot`, and returns their keys,
    /// ascending. A full pass in a world that is not a prediction world makes it a cut.
    fn fill(
        &self,
        snapshot: &mut crate::PresentSnapshot,
        chunks: &[ChunkRef<'_>],
        tick: Tick,
        full: bool,
    ) -> Vec<StableKey> {
        let columns: Vec<ErasedColumn<'_>> = chunks
            .iter()
            .flat_map(|chunk| {
                chunk
                    .components()
                    .filter(|&component| self.copies(component))
                    .filter_map(|component| chunk.column(component))
            })
            .collect();
        let mut rows: BTreeMap<ComponentKey, Vec<(StableKey, &[u8])>> = BTreeMap::new();
        for column in &columns {
            let size = column.element_size();
            let values = rows.entry(column.component()).or_default();
            for (row, &key) in column.keys().iter().enumerate() {
                values.push((key, &column.bytes()[row * size..(row + 1) * size]));
            }
        }
        let cut = full && self.kind != WorldKind::Prediction;
        snapshot.fill(self.kind, tick, cut, rows);
        keys_of(chunks)
    }
}

/// The keys of `chunks`' rows, ascending.
fn keys_of(chunks: &[ChunkRef<'_>]) -> Vec<StableKey> {
    let mut keys: Vec<StableKey> =
        chunks.iter().flat_map(|chunk| chunk.keys().iter().copied()).collect();
    keys.sort_unstable();
    keys
}

/// The service resource [`setup`](crate::setup) inserts: the world's [`ExtractState`] under
/// a type no other crate can name, so that only `present::extract` takes it as a parameter
/// and only [`extract_state`](crate::extract_state) reaches it, from outside any run.
pub(crate) struct ExtractService(pub(crate) ExtractState);

/// Extract, placed in `Pass::Extract` in every kind of world by
/// [`register_systems`](crate::register_systems): [`ExtractState::extract`] over the
/// world. It names `&mut World` because an exclusive system does, and reads the world
/// through a shared reference.
#[system(name = "present::extract", worlds = [server, client, prediction], exclusive)]
pub(crate) fn extract(world: &mut World, job: Job, state: ResMut<ExtractService>) {
    let world: &World = world;
    state.0.extract(world, &mut job);
}
