# lactose-present-api

The presentation interface (architecture sections 5.3 and 13): what gameplay, the simulation
side and the render side agree on about what leaves a world to be shown. The mesh, light,
decal and camera rig components an entity is authored with; the audio, VFX and debug-shape
requests a tick emits; extract, the pass system that copies a world's instances to a sink
the renderer implements and its presented rows to the presentation snapshot; the snapshot
itself, triple-buffered; and the two buffers the render side fills for itself, the UI
view-models and the debug-draw segments. `game-present` and `game-sim` reach presentation
through this crate without naming a renderer, an audio device, a window or a network host.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside; `[boundary_internal] any`, so a boundary crate may depend on it; a lint-file carrier (`[lint_file_carriers]`), so held to every rule of section 8.2 |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; the registration functions, the constructors and the two conversions run, and
nothing else does. This file is the contract the implementation lane (`present-impl`) meets,
and every sentence under "Behaviour the signatures cannot express" is one a dependent crate
may rely on. Architecture sections 5.3, 6, 6.1, 8.1, 8.2, 9.4, 12.4 and 13 are the design;
where this file and the architecture disagree, the architecture wins and this file is wrong.
Decision D156 records the choices those sections left open; D152 and the milestone 2 plan's
decision 5.18 are why the crate carries the boundary's lint file. `crates/ecs/README.md`
(change marks, the structural log, presentation events, service resources),
`crates/sim/README.md` ("Runs": `run_passes` and `Pass::Extract`) and
`crates/physics-api/README.md` (`Transform`, `Teleported`) are the contracts this crate
builds on and never restates.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `lactose-presentation`, `lactose-renderer`,
`lactose-app` and the game crates are written against this surface. `tests/consumers.rs` is
a compile-only sketch of what each of them writes: a change that breaks one is a signature
change to list.

## Surface

| Item | Purpose |
|---|---|
| `MeshInstance`, `MeshAsset` | `present::mesh`: the mesh an entity is drawn with. A row with it and a `Transform` is an *instance* |
| `Light`, `LightKind` | `present::light`: a point or spot light at the entity's transform |
| `Decal`, `MaterialAsset` | `present::decal`: a material projected inside a box at the entity's transform |
| `CameraRig`, `CameraRig::pivot_of` | `present::camera_rig`: the authored half of an orbit camera, the pivot offset and the boom's rest length; and the pivot in simulation space for a transform |
| `AudioRequest`, `SoundAsset` | Presentation event `present::audio`: play a sound once at a place |
| `VfxRequest`, `EffectAsset` | Presentation event `present::vfx`: spawn an effect once at a placement |
| `DebugShape`, `DebugShapeKind` | Presentation event `present::debug`: a line, box or sphere a tick asks the overlay to draw |
| `register_types`, `setup`, `register_systems` | What `app` calls for every world kind: the four components; the `ExtractState` service; extract in `Pass::Extract` |
| `ExtractState`, `ExtractState::{new, set_sink, set_snapshot, extract, publish}` | Extract's state beside the world: its two outputs, one pass, and the publication of what the passes wrote with the frame's events |
| `InstanceSink`, `ExtractPass`, `InstanceRow`, `RenderTransform`, `RenderTransform::from_sim` | Where extract writes instances: the protocol of one pass, the row, and the render-space placement it carries |
| `snapshot_channel`, `SnapshotWriter`, `SnapshotReader`, `SnapshotReader::acquire` | The triple buffer: the half extract writes through and the half the render side reads from |
| `PresentFrame`, `PresentFrame::{older, newer, events, transform}`, `TransformPair` | What one frame presents from: two snapshots, the events since the previous frame, and a row's two transforms |
| `PresentSnapshot`, `PresentSnapshot::{kind, tick, cut, get, rows}`, `SnapshotRows` | One world's presented rows at one pass, by `StableKey` |
| `ViewModels`, `ViewModels::{new, set, get, remove, get_erased, iter, len, is_empty}`, `ViewModel`, `ViewModelIter` | The UI view-models: at most one value of each `derived` resource type, typed or by key and field ID |
| `DebugDraw`, `DebugDraw::{new, line, shape, lines, is_empty, clear, CIRCLE_SEGMENTS}`, `DebugLine` | The debug-draw buffer: one frame's coloured segments in render space |

There is no hidden plumbing: nothing here is `#[doc(hidden)]`. The system's descriptor is
crate-private and reached through `register_systems` alone; its name, `present::extract`,
is public because `lactose_predict::EXCLUSIVE_SYSTEMS` lists it.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Given by |
|---|---|---|
| `game-sim`, any crate that spawns | To say what an entity looks like, and where a camera follows it | The four components through an ordinary bundle or a cooked baseline; each is `persistent, replicated, extracted`, so a save keeps it and a client receives it |
| `game-sim`, any tick system | To ask for a sound, an effect or a debug shape without a row of the origin entity surviving | `AudioRequest::at`, `VfxRequest::at`, `DebugShape::{line, aabb, sphere}` through `PresentEvents`; each carries its place in simulation space |
| `lactose-app`, `frame-design` | One registration for every kind; the outputs of a world that is shown; the frame's order | `register_types`, `setup`, `register_systems`; `snapshot_channel`, `ExtractState::{set_sink, set_snapshot}`; `Simulation::run_passes`, then `World::take_present_events`, then `ExtractState::publish` ("The registration and the frame `app` writes") |
| `present-impl` | A contract for every body, and what it may add elsewhere | "Behaviour the signatures cannot express"; "Not in this crate yet" names the one addition to `lactose-ecs` it may need |
| `lactose-renderer` | Instances by `StableKey` with no system of its own: which changed, which are gone, when everything it holds is stale | `InstanceSink` over its staging ring and slot table: `write`, `removed`, and `ExtractPass::full`; `ExtractPass::kind` tells a client's two worlds apart; `RenderTransform` is the row's placement |
| `lactose-presentation` | Two states to interpolate between, the camera's pivot, the frame's events once, the remote entities' slots and meshes | `SnapshotReader::acquire`; `PresentFrame::transform`; `CameraRig::pivot_of`; `PresentFrame::events`; `PresentSnapshot::rows::<RemoteSlot>()` and `get::<MeshInstance>` on the replicated world's snapshot; `InstanceRow` and `RenderTransform::from_sim` for the remote upload |
| `lactose-physics-api` | Its `Transform` and `Teleported` extracted, the second compared with the last tick covered | "The no-interpolate flag"; `PresentFrame::transform` |
| `lactose-anim` | `AnimRig`, `AnimState` and `SimPose` beside the root transform, for the render pose | They are `extracted`, so a row with any of them is a presented row and the snapshot holds all three with its `Transform` ("The snapshot") |
| `lactose-net-api`, `lactose-net-host` | `RemoteSlot` to reach presentation; remote transforms kept out of extract | `RemoteSlot` is `extracted`, so the replicated world's snapshot holds it; that world's passes copy and write no transform ("The extract set") |
| `lactose-predict` | Extract on the prediction world every frame; a rollback that redraws what moved and cuts nothing; events not delivered twice | `Predictor::passes` runs the pass; a rollback makes the next pass full and not a `cut`; the events' keys are the ECS's ("Events and publication"); `present::extract` is in `EXCLUSIVE_SYSTEMS` |
| `game-present` | To turn the snapshot and the events into what the UI shows and what the overlay draws | `PresentSnapshot::{get, rows}` for the game's own `extracted` components; `ViewModels::set`; `DebugDraw::line` |
| `lactose-ui` (milestone 5) | A value by type, or by key and field ID from a cooked layout | `ViewModels::{get, get_erased, iter}`, `ViewModel::{info, bytes}` with `TypeInfo::field_bytes` |
| `lactose-audio` (milestone 5) | Requests to play, cancellations to stop | The frame's `PresentEventBatch`: `emitted` with `PresentEmission::get::<AudioRequest>()`, `cancelled` by key |
| A debug overlay | The frame's segments | `DebugDraw::lines` |

## The registration and the frame `app` writes

```rust
// Once, for a world of any kind. A dedicated server stops here: with no output installed
// a pass does nothing.
let mut types = TypeTable::new();
lactose_physics_api::register_types(&mut types)?;
lactose_present_api::register_types(&mut types)?;
let mut simulation = SimulationBuilder::new(kind, types)
    .setup(lactose_present_api::setup)
    .systems(lactose_present_api::register_systems)
    .build()?;

// Once, for a world that is shown.
let (writer, reader) = lactose_present_api::snapshot_channel();
let extract = simulation.world_mut().service_mut::<ExtractState>().expect("setup");
extract.set_sink(Some(renderer_sink));
extract.set_snapshot(Some(writer));

// Every frame, after the frame's ticks (and, for a prediction world, after `reconcile`).
let mut simulation = simulation.run_passes(&executor).await?;       // extract runs here
let events = simulation.world_mut().take_present_events();
simulation.world_mut().service_mut::<ExtractState>().expect("setup").publish(events);

// Every frame, on the render side.
if let Some(frame) = reader.acquire() { /* the presentation schedule */ }
```

## Behaviour the signatures cannot express

**The boundary** (architecture sections 8.1 and 8.2; D152)

- The crate is outside the determinism boundary: it is not in `crate-tiers.toml`'s
  `[boundary]`, and the boundary's dependency allowlists do not bind it, so review holds
  what its code calls (D152). `[boundary_internal] any` names it as a crate a boundary
  crate may depend on, which is how `game-sim` reaches it. It depends on boundary and
  `[boundary_internal]` crates alone and names no RHI, platform, renderer or `net-host`
  type, which is what keeps it admissible there.
- It is a lint-file carrier: `clippy.toml` here is a byte-identical copy of
  `crates/core/clippy.toml`, and `tools/xtask/ci-policy.toml` lists the crate under
  `[lint_file_carriers]`, because it declares what a world registers: four components that
  are not `derived`, and a `#[system]`. Taken off that list, `cargo xtask boundary` reports
  those five declarations.
- So every line of the crate, not only what the lints reach, is held to section 8.2: no
  float arithmetic except through `lactose_math`, no float remainder, no clock, no hash map
  but `FixedHashMap`, no thread-local or global mutable state, no thread started, and every
  reference, ordering rule and key a `StableKey`. The one float conversion is
  `RenderTransform::from_sim`, through `lactose_math::render` and `Scalar::to_f32`. The
  snapshot channel's lock and reference counts are shared between the pass and the render
  side and are no global state: nothing a world holds, and nothing a tick's system can
  read, depends on when the render side takes the lock.

**The components** (architecture section 13)

- `MeshInstance`, `Light`, `Decal` and `CameraRig` are `persistent, replicated, extracted`:
  hashed, saved, in a region's cooked baseline, sent to a client for an entity it did not
  spawn, and tracked per row for extract. None is `derived`. They hold `lactose_math`
  quantities, asset references and integers alone, with explicit padding; their sizes are
  8, 64, 32 and 32 bytes at the 64-bit scalar (D146), asserted in `lib.rs`. Field IDs are
  explicit and stable: a field is added with a new ID and a migration, never renumbered.
- An asset reference is typed by a marker of this crate (`MeshAsset`, `MaterialAsset`,
  `SoundAsset`, `EffectAsset`), an uninhabited type that implements no asset trait: the
  runtime types and their loaders belong to the crates that own each kind and that this
  crate cannot name. A reader re-types the reference for its own type with
  `AssetRef::new(reference.id())`. The ID is the content's and is what a save, a packet
  and a baseline hold. A mesh or a material a component names must be in the region's
  asset set (architecture section 14); nothing here checks it.
- Nothing validates a component's values: a write is the game's or the cooker's, and the
  documented ranges are what the cooker emits. What a reader does with a value outside
  them is the reader's to state.
- A `MeshInstance` is for an entity. Static cooked instances are not rows: they go from IO
  to the GPU scene and never pass through a world or through extract (section 5.3).
- `Light` and `Decal` are placed by the row's `Transform`; a row with one and no transform
  shows nothing. A light points, and a decal projects, along the entity's local -Z axis.
- The camera rig a world holds is `CameraRig` alone: the authored pivot offset and rest
  length. The orbit's yaw and pitch are the input sample's (architecture section 13: the
  input carries absolute yaw and pitch into the simulation), and the boom's length this
  frame is `lactose-presentation`'s own state, evaluated per frame and never
  authoritative. Neither is a component or a resource, so presentation declares nothing a
  world registers (D152). The camera pivot the snapshot holds (section 5.3) is not a
  field: it is `CameraRig::pivot_of` the row's interpolated transform, both of which the
  snapshot holds.

**The events** (architecture sections 6.1 and 9.4; `crates/ecs/README.md`, "Commands and
events")

- `AudioRequest`, `VfxRequest` and `DebugShape` are `lactose_ecs::PresentEvent`s with the
  kinds `present::audio`, `present::vfx` and `present::debug`, of 32, 64 and 56 bytes. A
  system emits one through `PresentEvents::emit(origin, event)`; the ECS keys it by
  (origin key, tick, kind, sequence), delivers a re-emission of a re-simulated tick once,
  and reports as cancelled a delivered event whose tick was re-simulated without it.
- A request carries where it happens, in simulation space, at the tick it was emitted.
  Presentation needs no row of the origin entity to place it, which matters twice: the
  entity may be despawned before the event is delivered, and a row that is only an
  instance has no transform in the snapshot. A sound or an effect does not follow its
  origin afterwards.
- An empty asset reference in a request plays or spawns nothing. A cancelled request stops
  its sound, or removes its effect, if it still runs: the reader's duty, by the key.
- `DebugShape` is how a system inside a world reaches the debug-draw buffer, which no
  system can name. A shape is drawn from the frame that receives it until the first frame
  whose newer snapshot is of a later tick than the shape's, so a system that wants a shape
  to stay emits it every tick. A cancelled shape is dropped at once.
- A game crate declares its own presentation events as it declares these; nothing here
  enumerates kinds.

**Extract** (architecture sections 5.3 and 9.4; `crates/sim/README.md`, "Runs")

*Placement and state.*

- Extract is `present::extract`, an exclusive system with `worlds = [server, client,
  prediction]`, alone in `Pass::Extract`. `Simulation::run_passes` runs it once per frame
  after the frame's ticks, paused or not; `lactose-predict`'s re-simulation runs steps
  alone, so it never runs against a discarded tick.
- Its state is `ExtractState`, a service resource `setup` inserts in every kind of world.
  A service is not world state: not hashed, not saved, not in a `WorldSnapshot`, and so
  kept as it is across a restore. It has no accessor through which a system could read
  what a pass did.
- With neither output installed a pass returns at once and keeps no mark. A dedicated
  server installs none. With one output installed the pass does that output's half.
- `set_sink` and `set_snapshot` are called between runs. Either makes the next pass full
  for both outputs, so outputs are installed before a world is first shown, not while it
  is.

*Extract writes nothing to the world.*

- The pass takes the world by shared reference (`ExtractState::extract(&mut self, &World,
  ..)`): it writes no column, hashed or not, no resource, no event, no channel, no log
  entry and no stamp, and it trims nothing. What it keeps between passes is in
  `ExtractState`. So the world hash is the same before and after a pass, a tick's systems
  can read nothing a pass left, and a world's hashes do not depend on how many frames
  were drawn (D152, "A pass system leaves the world hash as it found it").
- No tool checks that of an exclusive system, so `present-impl` proves it with a recorded
  test, in the module `recorded` of this package's `integration` target (D150): a scene
  with rows that move, teleport, spawn, despawn and change region is stepped under the
  harness session in a server world and in a prediction world with a rollback, with a
  counting sink and a snapshot channel installed. After every step the test takes
  `Simulation::hash()`, runs `run_passes` twice (a pass that follows a tick, then one that
  does not) and takes the hash after each: the three are equal. The hash it records for
  the tick is the one taken after the passes, so the stream SOUR compares across its
  modes is of a world extract has run on, and it equals, tick for tick, the stream of the
  same scene stepped with no pass run. The package joins `[harness]` in that change, with
  the test's name in `tests`.

*The extract set.* Which rows of a world a pass considers:

- In a `Server` world, every row.
- In a `Prediction` world, every row that has `ClaimedBy`: the rows the prediction world
  simulates. Its other rows are proxies of entities the replicated world presents.
- In a `Client` world, every row, and no transform: the pass copies no `Transform` and no
  `Teleported` into the snapshot and writes no row to the sink, because a remote entity's
  transform is the interpolation buffer's, sampled each frame by presentation (section
  5.3). It still reports removals.
- An *instance* is a row of the set that has both `Transform` and `MeshInstance`. A
  *presented row* is a row of the set that has at least one component the world's
  registry declares `extracted` other than `Transform`, `Teleported` and `MeshInstance`:
  a camera rig, a light, a decal, an animated character, a remote slot, a game component
  the UI shows. A row may be both, either or neither.
- Every key extract hands out is the row's origin key, which a bind does not change
  (`crates/ecs/README.md`).

*Full and incremental passes.*

- A pass is *full* when extract cannot tell what changed since its previous pass: there
  is none since an output was last installed or removed; the previous pass's mark is not
  current (`World::is_current`: a restore, a whole-world load, the 2^30 stamp rebase); or
  the structural log no longer reaches the previous pass's cursor (`StructuralLog::since`
  is `None`: more ticks than `SimulationBuilder::log_retention` ran with no pass). Every
  other pass is *incremental*.
- A region load or unload does not make a pass full: loaded rows are logged as spawned and
  stamped, unloaded rows are logged as despawned.
- An incremental pass *follows a tick* when the world's tick is not its tick at the
  previous pass, and is *idle* otherwise. More than one tick may have run (catch-up, a
  re-simulation): the pass covers them all.
- An idle pass that finds nothing changed calls no sink method and writes no snapshot. Its
  cost is the chunks a writing system was issued over since the previous pass
  (`crates/ecs/README.md`, "Change tracking"), which is none.
- "Stamped" below is the ECS's row stamp on a tracked column: a write through `Mut`, `set`,
  `insert` or `get_mut` stamps whether or not the bytes changed, `set_if_neq` only if they
  did. Rows are selected by stamp against a mark (`changed_since`), never by tick, so the
  selection is the same under every chunk layout and worker count.

*What a pass gives the sink.* Only in a `Server` or a `Prediction` world are rows written;
`begin`, `removed` and `end` are called in every kind.

- A full pass calls `begin` with `full: true`, writes every instance of the world, and
  calls `end`; it never calls `removed`. An instance the sink holds for the pass's kind
  that the pass did not write is gone. In a `Client` world that is every instance of the
  kind: the remote upload of the same frame, which follows the passes, writes again those
  that exist.
- An incremental pass with something to say calls `begin` with `full: false`, then
  `removed` if any key qualifies, then `write` for its rows, then `end`.
- `removed` lists every key the structural log since the previous pass records as
  despawned (an unload included), or as having lost `Transform` or `MeshInstance`, or, in
  a prediction world, `ClaimedBy`: in ascending key order, each once. The key may never
  have been an instance (a despawned light): a sink ignores a key it does not hold. A key
  listed and an instance again at this pass (a region unloaded and loaded again, a mesh
  removed and inserted) is written by the same pass, and arrives as a new instance.
- An incremental pass writes an instance when any of these holds: (1) its `Transform` or
  its `MeshInstance` was stamped since the previous pass; (2) the structural log since the
  previous pass records it as spawned, or as having gained `Transform`, `MeshInstance` or,
  in a prediction world, `ClaimedBy`; (3) the pass follows a tick, and its `Transform` or
  `MeshInstance` was stamped since the *settling mark*.
- *Settling.* The settling mark is the mark the previous pass that followed a tick started
  from, which is the mark of the pass before it. So a row a tick moved is written by the
  pass after that tick and once more by the pass after the next tick, moved again or not,
  and then no more until it is stamped again. The second write carries the same placement,
  which makes the sink's previous placement equal to its current one: this is section
  5.3's "or newer than the tick before so previous transforms stay valid". A row an idle
  pass wrote is likewise written once more by the next pass that follows a tick. After a
  full pass the settling mark is that pass's own mark, except in a prediction world, where
  it is none and the next pass that follows a tick writes every instance once more.
- Each instance a pass writes is in exactly one `write` batch, once. Batches are never
  empty; how rows are batched, and the order of batches and of rows within one, follow the
  chunk layout and the executor and carry no meaning. `write` is called from the pass's
  forked jobs, several at once. A sink whose result must not depend on that order sorts
  what it kept by key in `end`.
- `begin` and `end` bracket every pass that calls anything, and extract calls nothing
  between two passes. The world does not change between `begin` and `end`.
- The sink is infallible. One that cannot take a row waits or drops inside its own
  contract.

*What the sink keeps.* A row carries the placement the instance has now and no previous
one: the world holds one `Transform` per row, and extract keeps no table of its own.

- The sink holds, per key and world kind, the last placement it was given. A `write` makes
  the held placement the previous one and the row's the current one; when
  `no_interpolate` is set, or the sink does not hold the key, the previous placement
  becomes the row's as well.
- That is the whole of what "previous and current transforms" (section 5.3) means for an
  instance: both live on the render side, and the staging ring carries current placements.
- `InstanceRow::mesh` is the row's `MeshInstance::mesh` at the pass, on every write, so a
  sink needs no other channel to learn a new instance's mesh or a change of it.
- `RenderTransform` is `RenderPosition::from_sim` of the translation, which rounds to
  nearest and is the same on every platform, and the rotation's four components rounded
  to 32 bits. Render space has no way back to simulation space.

*The no-interpolate flag.* `InstanceRow::no_interpolate` is set:

- on every row of a full pass in a `Server` world, where what the sink held is not what
  led to the world's state;
- on every row of an idle pass: a write between ticks is a placement, not a movement over
  a tick;
- otherwise, when the row has a `Teleported` whose tick is not zero and is later than the
  world's tick at the previous pass, by the wrapping comparison of ticks (the difference
  is in `1..2^31`). A frame that ran several ticks loses no teleport
  (`crates/physics-api/README.md`); a `Teleported` loaded from a save is in the past and
  flags nothing. With no previous pass this rule flags nothing. A correction's placement
  sets `Teleported` at the tick that re-simulates it (`crates/physics-api/README.md`,
  "Snapshot and restore") and is judged by the same rule, so it is flagged only when that
  tick is after the previous pass's.
- A full pass in a prediction world, which a rollback causes at every misprediction,
  follows the last rule alone: the local player is drawn from where it was shown to where
  the corrected run puts it. The visual blend of a correction is presentation's, from
  `lactose-predict`'s `Correction`.

**The snapshot** (architecture section 5.3)

*What it holds.*

- A `PresentSnapshot` is one world's presented rows at one pass: for each, every
  `extracted` component the row has, by value, by the row's key. That includes the row's
  `MeshInstance` and, outside a `Client` world, its `Transform` and `Teleported`. It
  holds the world's kind and tick, and nothing else: no resource, no event, no row that
  is only an instance.
- So the bulk of a scene, rows with a mesh and a transform and nothing else, is not in
  the snapshot: extract writes those to the sink alone, and their interpolation is the
  render side's between the sink's two placements. The snapshot is what presentation
  computes on with the CPU: cameras, lights, decals, poses, remote slots, the UI's data.
- "Remote entities are not in it" (section 5.3) is true of their transforms. Their rows
  are in the replicated world's snapshot with `RemoteSlot`, `MeshInstance` and their other
  extracted components, which is how presentation learns which slot to sample for which
  key and what to draw there.
- `get` and `rows` return values, and `rows` is in ascending key order. Both refuse at
  build a type that is not declared `extracted`.
- A pass writes a snapshot when a writer is installed and: it is full; or it follows a
  tick; or it is idle and a presented row was stamped in an extracted column, or entered
  or left the presented rows, since the previous pass. A snapshot is always the whole
  state, never a difference, however the implementation fills it.

*The triple buffer.*

- A channel owns exactly three snapshot buffers. The reader holds the two of its last
  `acquire`; the writer writes the third. `acquire` exchanges handles under the channel's
  lock, copies nothing and never waits for a pass; a pass never waits for the reader. The
  lock is held for the exchange alone, never while a snapshot is written or read. The
  code is safe Rust.
- `acquire` returns the two newest published snapshots, older first, or `None` before the
  first publication. With one published, both are that one. Called again before another
  publication it returns the same pair.
- When the writer needs a buffer and the reader holds two, the third is the newest
  published snapshot, which the reader has not acquired: the writer takes it back and
  writes it. A reader slower than the passes therefore misses snapshots, never blocks a
  pass, and its pair then spans more than one pass. Nothing the reader holds is ever
  written.
- `PresentSnapshot::cut` is set on the snapshot of a full pass in a `Server` or `Client`
  world: nothing before it leads into it (a load, a restore, a world shown for the first
  time). It is kept when a cut snapshot the reader never acquired is taken back and
  written again, so a reader cannot miss a cut. It is never set in a prediction world,
  where a full pass is a rollback; a prediction world's resync is reported by
  `lactose-predict`, not here.
- One channel serves one world. A client's two worlds each have a channel and an
  `ExtractState`, and presentation reads both readers; `PresentSnapshot::kind` says which
  is which.

*Events and publication.*

- A snapshot a pass wrote is invisible to the reader until `ExtractState::publish`. A pass
  that runs again before `publish` updates the same unpublished snapshot.
- `publish(events)` makes the unpublished snapshot, if there is one, the newest published,
  and appends `events` to the events the reader's next `acquire` returns, in one exchange:
  a frame never holds a snapshot without the events of the ticks it covers. With no
  snapshot to publish the events are appended alone. With no writer installed the batch
  is dropped.
- Events travel beside the snapshots, not in them: each emission is in exactly one
  frame's batch, whether or not the snapshot it was published with was ever acquired.
  Batches published between two acquires are folded in publication order by the ECS's own
  rule: an emission followed by its cancellation is in neither list, and a cancellation
  followed by an emission of the same key is in neither. An `acquire` with nothing
  published since the previous one returns an empty batch.
- `app` publishes once per frame, after `run_passes`, with the batch
  `World::take_present_events` returns after it, for each world it shows. For a prediction
  world that is after `reconcile` (`crates/predict/README.md`, "Presentation").
- A `Server` world's step takes and drops its presentation events
  (`crates/sim/README.md`, "Runs", step 4), so a shown world of that kind delivers none
  until `lactose-sim` lets a shown server keep them: "Not in this crate yet".

*A frame's transforms.*

- `PresentFrame::transform(key)` is the row's `Transform` in the newer snapshot as
  `current`, and in the older as `previous`, or `current` where the older holds none.
- Its `no_interpolate` is set when the newer snapshot is a `cut`; when the older holds no
  transform for the key; when the two snapshots are of one tick (the first snapshot, or
  an edit between ticks); or when the row's `Teleported` in the newer snapshot is not
  zero and is later than the older snapshot's tick, by the wrapping comparison.
- An edit made between ticks to a presented row publishes a snapshot of the same tick, so
  for that frame every presented row is shown at the newer state with nothing drawn in
  between. `app` edits no row between ticks in a running game (`crates/sim/README.md`).
- How far a frame is between the pair's two ticks is presentation's to compute; nothing
  here holds a time.

**View-models** (architecture section 13)

- A view-model is a reflected resource declared `#[resource(key = "...", derived)]`: never
  hashed, saved or replicated, free to hold a raw `f32`, and declarable in a crate that
  carries no lint file (section 8.1). `ViewModels` holds at most one value of each type,
  by the type's key; `set`, `get` and `remove` refuse at build a type that is not
  `derived`.
- The table is the render side's. The presentation schedule owns one; `game-present` and
  `lactose-presentation` write it each frame from the snapshot and the frame's events, and
  the UI reads it. No world holds a view-model and no system writes one: what the
  simulation tells the UI reaches it as `extracted` components, which the snapshot holds,
  and as presentation events.
- `get_erased` and `iter` give a value as its `TypeInfo` and bytes, for a reader that binds
  by key and field ID. `iter` is in ascending key order. The bytes are a valid value of
  the type, at no particular alignment.
- Two types whose declared names hash to one key cannot both be stored: `set` panics on
  the second. No registry was asked, because no world registers a view-model.

**Debug draw** (architecture section 13)

- `DebugDraw` is one frame's segments in render space, in the order added, with colours as
  `0xRRGGBBAA` (sRGB, straight alpha). It is a plain value its owner passes by `&mut`:
  the presentation schedule fills it and a debug pass reads it when the frame is recorded.
- `DebugDraw::shape` expands a `DebugShape` into segments through
  `RenderPosition::from_sim`: one for a line, twelve for a box, and for a sphere three
  circles of `CIRCLE_SEGMENTS` segments each, in an order that depends on the shape alone.
- Nothing in the buffer reaches a world.

## Not in this crate yet

- A per-chunk view of several columns at once, in `lactose-ecs`: extract reads
  `Transform`, `MeshInstance` and `Teleported` of the same rows, and `World::columns`
  yields one column. `present-impl` may add the join there, in the change that needs it,
  or pair the columns by key; no signature here changes either way.
- A shown `Server` world's presentation events: `Simulation::step` drops them. A listen
  server or a single-player world needs a `SimulationBuilder` option in `lactose-sim` that
  leaves them for `app`; `frame-design` proposes it if milestone 2's local world is of
  that kind.
- Extracted *resources* in the snapshot. No type is declared so today and the ECS has no
  erased iteration of world resources; the snapshot holds components alone until a reader
  needs one.
- The merge of a client's two worlds: the prediction world's copy of the local player
  shadowing the replicated one, by the alias the ECS keeps between a `Predicted` origin key
  and its `NetId`. It is the reader's, on both outputs, at milestone 4 (the milestone 2
  plan, decision 5.5); `ExtractPass::kind` and `PresentSnapshot::kind` are what it needs
  from here.
- The remote upload's writer. Presentation writes remote instances as `InstanceRow`s, with
  `no_interpolate`, outside any pass, through a handle the renderer's facade defines, not
  through `InstanceSink`.
- A `cut` for a prediction world's resync, and the visual blend of a correction:
  presentation's, from `lactose-predict`'s surface, at milestone 4.
- Per-instance scale, a material override, the sun, shadows and light profiles: no reader
  before the milestone 3 renderer. `Light` and `Decal` reach the GPU scene through the
  snapshot at milestone 3; nothing draws them at milestone 2.
- Erased access to a snapshot (by `ComponentKey`): no reader names a component it cannot
  type yet.
- The engine's own view-model types, UI animation, the audio listener and markers:
  milestone 5's crates.

## Proof for milestone 2

Nothing but the registration, the constructors and the two conversions runs yet:
`tests/registration.rs` builds a simulation of every kind with the crate registered, and a
prediction world under `lactose_predict::check_prediction_reads`, which admits extract by
its key alone; `lib.rs` asserts the bounds, the policies and the sizes at build.
`present-impl` replaces every `todo!()` and proves:

| What | How |
|---|---|
| Extract writes nothing | The recorded test under "Extract writes nothing to the world", in every SOUR mode |
| The extract set | A world of each kind with instances, presented rows, rows that are both and rows that are neither: a counting sink and the snapshot hold exactly the stated rows; a prediction world's unclaimed rows are in neither; a `Client` world's sink gets no row and its snapshot no `Transform` |
| Full passes | The first pass, a pass after `set_sink` or `set_snapshot`, after a restore, after a whole-world load, after more than the retention's ticks with no pass: `full`, every instance written, no `removed`; `no_interpolate` and `cut` by kind |
| Incremental passes | A row moved at one tick is written by the pass after it and by the pass after the next tick, then not again; a row edited between ticks is written by an idle pass with `no_interpolate`; an idle pass with nothing changed calls nothing; a `set_if_neq` of equal bytes writes nothing |
| Removals | A despawn, an unload, a removed `MeshInstance`, a released claim: listed once, in key order; a key that left and came back is listed and written |
| The flag | A teleport at each tick of a three-tick frame flags the row; a saved `Teleported` does not; tick zero does not; the comparison holds across the tick's wrap |
| Layout and workers | The sink's rows sorted by key, and the snapshot's bytes, are equal under every layout capacity, chunk order and worker count of SOUR's modes |
| The triple buffer | A reader that acquires every publication, every second and never: the pair is always the two newest published, a held snapshot is never written, `cut` survives a snapshot taken back, and no pass waits; a model of the exchange under loom, and the tests under Miri |
| Events | Each emission is in exactly one frame's batch across skipped and taken-back snapshots; an emission and its cancellation between two acquires are in neither list; a rollback's re-emission is not delivered again |
| The frame | `transform` for a moved, a teleported, a new and a `Client`-world row; one-tick pairs and cuts do not interpolate |
| View-models and debug draw | `set`, `get`, `remove`, erased reads by field ID, key order; a key collision panics; a non-`derived` type does not build (a compile-fail case); the segments of each shape |
| Cost | Extract's pass at 50,000 rows moved among 100,000 on the null sink and on the device's ring (the milestone 2 plan, decision 5.2), with the file saying what the snapshot held |
