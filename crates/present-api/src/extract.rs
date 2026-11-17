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

use lactose_asset_api::AssetRef;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{
    ChangeMark, Job, LogCursor, PresentEventBatch, ResMut, World, WorldKind, system,
};
use lactose_math::render::RenderPosition;
use lactose_physics_api::Transform;

use crate::components::MeshAsset;
use crate::snapshot::SnapshotWriter;

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
/// kept (`README.md`, "What the sink keeps").
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
    /// tick are drawn from (`README.md`, "What the sink keeps").
    pub tick: Tick,
    /// Whether the pass is full: it writes every instance of the world, and an instance
    /// the sink holds for this kind of world that the pass does not write is gone. A pass
    /// is full when extract cannot tell what changed since its previous pass: the first
    /// pass after an output is installed or removed, the first after a restore, a
    /// whole-world load or the change-stamp rebase, and the first after more ticks than
    /// the structural log retains (`README.md`, "Full and incremental passes").
    pub full: bool,
}

/// Where extract writes instances: implemented by the renderer over its staging ring and
/// its slot table, and by a test that counts.
///
/// One pass calls, in this order: [`begin`](Self::begin) once; [`removed`](Self::removed)
/// at most once; [`write`](Self::write) any number of times, from several threads at
/// once; [`end`](Self::end) once. A pass that has no key to remove and no row to write,
/// and is not full, calls nothing. Between `begin` and `end` the world does not change.
/// In a client's replicated world a pass writes no row: remote entities' placements are
/// the interpolation buffer's, and presentation uploads them (`README.md`, "The extract
/// set").
///
/// Nothing here fails. A sink that cannot take a row (a ring whose regions are all in
/// flight) waits or drops inside its own contract; extract has nothing to do with an
/// error.
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
}

impl ExtractState {
    /// A state with no output and no pass behind it.
    #[must_use]
    pub const fn new() -> Self {
        Self { sink: None, snapshot: None, last: None, settle: None, tick: None, covered: None }
    }

    /// Installs `sink` as where instances are written, or removes the sink with `None`,
    /// and returns the one it replaces. The next pass is full. Between runs only: a pass
    /// holds the state for its whole run.
    pub fn set_sink(
        &mut self,
        sink: Option<Box<dyn InstanceSink>>,
    ) -> Option<Box<dyn InstanceSink>> {
        todo!()
    }

    /// Installs `writer` as the snapshot channel this world's passes write, or removes it
    /// with `None`, and returns the one it replaces. The next pass is full. A writer
    /// serves one world for as long as it is installed: installing in one world a writer
    /// another world has written gives the reader a pair of snapshots of two worlds.
    pub fn set_snapshot(&mut self, writer: Option<SnapshotWriter>) -> Option<SnapshotWriter> {
        todo!()
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
        todo!()
    }

    /// Publishes what the passes since the previous publication wrote, with the
    /// presentation events of the ticks they cover: the snapshot becomes the newest the
    /// reader can acquire, and `events` joins the events the reader's next acquire
    /// returns, in one exchange, so a frame never holds a snapshot without the events of
    /// its ticks. `app` calls it once per frame, after `Simulation::run_passes` and with
    /// the batch `World::take_present_events` returned after it. A snapshot of a tick
    /// earlier than the newest published one's is published as a cut, and one a full pass
    /// in a prediction world wrote at that tick takes its place (`README.md`, "The triple
    /// buffer" and "Events and publication").
    ///
    /// If no pass wrote a snapshot since the previous publication (no tick ended and
    /// nothing changed), the events alone are published. With no writer installed, or
    /// once the channel's reading half is dropped, the batch is dropped: nothing presents
    /// this world, and nothing accumulates for a reader that will never acquire.
    pub fn publish(&mut self, events: PresentEventBatch) {
        todo!()
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
            .finish()
    }
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
