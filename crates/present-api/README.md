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

**Status: implemented.** Every public signature is the facade's, unchanged, and every body
runs; "Proof for milestone 2" names the tests that hold each claim. This file is the
contract, and every sentence under "Behaviour the signatures cannot express" is one a
dependent crate may rely on. Architecture sections 5.3, 6, 6.1, 8.1, 8.2, 9.4, 12.4 and 13
are the design; where this file and the architecture disagree, the architecture wins and this
file is wrong.
Decision D157 records the choices those sections left open; D152 and the milestone 2 plan's
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
| `register_types`, `setup`, `register_systems` | What `app` calls for every world kind: the four components; extract's service, which holds its state; extract in `Pass::Extract` |
| `extract_state` | How `app` reaches a world's `ExtractState`, from the `Simulation`, between runs |
| `ExtractState`, `ExtractState::{new, set_sink, set_snapshot, extract, publish}` | Extract's state beside the world: its two outputs, one pass, and the publication of what the passes wrote with the frame's events |
| `InstanceSink`, `ExtractPass`, `InstanceRow`, `RenderTransform`, `RenderTransform::from_sim` | Where extract writes instances: the protocol of one pass, the row, and the render-space placement it carries |
| `snapshot_channel`, `SnapshotWriter`, `SnapshotReader`, `SnapshotReader::acquire` | The triple buffer: the half extract writes through and the half the render side reads from |
| `PresentFrame`, `PresentFrame::{older, newer, events, transform}`, `TransformPair` | What one frame presents from: two snapshots, the events since the previous frame, and a row's two transforms |
| `PresentSnapshot`, `PresentSnapshot::{kind, tick, cut, get, rows}`, `SnapshotRows` | One world's presented rows at one pass, by `StableKey` |
| `ViewModels`, `ViewModels::{new, set, get, remove, get_erased, iter, len, is_empty}`, `ViewModel`, `ViewModelIter` | The UI view-models: at most one value of each `derived` resource type, typed or by key and field ID |
| `DebugDraw`, `DebugDraw::{new, line, shape, lines, is_empty, clear, CIRCLE_SEGMENTS}`, `DebugLine` | The debug-draw buffer: one frame's coloured segments in render space |

There is no hidden plumbing: nothing here is `#[doc(hidden)]`. The system's descriptor is
crate-private and reached through `register_systems` alone; its name, `present::extract`,
is public because `lactose_predict::EXCLUSIVE_SYSTEMS` lists it. The service resource that
holds the `ExtractState` is of a crate-private type, inserted by `setup` and reached
through `extract_state` alone ("Placement and state").

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Given by |
|---|---|---|
| `game-sim`, any crate that spawns | To say what an entity looks like, and where a camera follows it | The four components through an ordinary bundle or a cooked baseline; each is `persistent, replicated, extracted`, so a save keeps it and a client receives it |
| `game-sim`, any tick system | To ask for a sound, an effect or a debug shape without a row of the origin entity surviving | `AudioRequest::at`, `VfxRequest::at`, `DebugShape::{line, aabb, sphere}` through `PresentEvents`; each carries its place in simulation space |
| `lactose-app`, `frame-design` | One registration for every kind; the outputs of a world that is shown; the frame's order | `register_types`, `setup`, `register_systems`; `snapshot_channel`, `extract_state`, `ExtractState::{set_sink, set_snapshot}`; `Simulation::run_passes`, then `World::take_present_events`, then `ExtractState::publish` ("The registration and the frame `app` writes") |
| `lactose-renderer` | Instances by `StableKey` with no system of its own: which changed, which are gone, when everything it holds is stale | `InstanceSink` over its staging ring and slot table: `write`, `removed`, and `ExtractPass::full`; `ExtractPass::kind` tells a client's two worlds apart; `RenderTransform` is the row's placement |
| `lactose-presentation` | Two states to interpolate between, the camera's pivot, the frame's events once, the remote entities' slots and meshes | `PresentFrame`, which the frame acquires and hands over; `PresentFrame::transform`; `CameraRig::pivot_of`; `PresentFrame::events`; `PresentSnapshot::rows::<RemoteSlot>()` and `rows::<MeshInstance>()`, merged by key, on the replicated world's snapshot; `InstanceRow` and `RenderTransform::from_sim` for the remote upload |
| `lactose-physics-api` | Its `Transform` and `Teleported` extracted, the second compared with the last tick covered | "The no-interpolate flag"; `PresentFrame::transform` |
| `lactose-anim` | `AnimRig`, `AnimState` and `SimPose` beside the root transform, for the render pose | They are `extracted`, so a row with any of them is a presented row and the snapshot holds all three with its `Transform` ("The snapshot") |
| `lactose-net-api`, `lactose-net-host` | `RemoteSlot` to reach presentation; remote transforms kept out of extract | `RemoteSlot` is `extracted`, so the replicated world's snapshot holds it; that world's passes copy and write no transform ("The extract set") |
| `lactose-predict` | Extract on the prediction world every frame; a rollback that redraws what moved and cuts nothing; a resync to an earlier tick that cuts; events not delivered twice | `Predictor::passes` runs the pass; a rollback makes the next pass full and not a `cut`, and in a frame with no step both outputs draw the corrected state from the tick before ("What the sink keeps", "Events and publication"); a pass at a tick earlier than the previous one flags every row, makes the sink drop its bases and publishes a `cut` ("What the sink keeps", "The triple buffer"); the events' keys are the ECS's; `present::extract` is in `EXCLUSIVE_SYSTEMS`; `app` reaches the state with `extract_state(predictor.simulation_mut())` |
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
let extract = lactose_present_api::extract_state(&mut simulation).expect("setup");
extract.set_sink(Some(renderer_sink));
extract.set_snapshot(Some(writer));

// Every frame, after the frame's ticks (and, for a prediction world, after `reconcile`).
let mut simulation = simulation.run_passes(&executor).await?;       // extract runs here
let events = simulation.world_mut().take_present_events();
lactose_present_api::extract_state(&mut simulation).expect("setup").publish(events);

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
  system can name. A shape is drawn from the frame that receives it until presentation
  acquires a snapshot of another tick than the shape's, or a cut, so a system that wants a
  shape to stay emits it every tick, and a load to an earlier tick leaves no shape behind.
  A cancelled shape is dropped at once.
- A game crate declares its own presentation events as it declares these; nothing here
  enumerates kinds.

**Extract** (architecture sections 5.3 and 9.4; `crates/sim/README.md`, "Runs")

*Placement and state.*

- Extract is `present::extract`, an exclusive system with `worlds = [server, client,
  prediction]`, alone in `Pass::Extract`. `Simulation::run_passes` runs it once per frame
  after the frame's ticks, paused or not; `lactose-predict`'s re-simulation runs steps
  alone, so it never runs against a discarded tick.
- Its state is an `ExtractState`, held in a service resource that `setup` inserts in every
  kind of world. A service is not world state: not hashed, not saved, not in a
  `WorldSnapshot`, and so kept as it is across a restore.
- No system but extract reaches the state. The service's type is private to this crate and
  only `present::extract` names it, so `Res<ExtractState>` and `ResMut<ExtractState>` find
  no resource: `SimulationBuilder::build` refuses a system that names either, in any world
  kind, with `InvalidData` carrying `ScheduleError::MissingResource`
  (`tests/registration.rs`). `app` reaches the state through `extract_state`, which takes
  the `Simulation`: an exclusive system holds the `World`, and no system holds the
  `Simulation`. So no system of a tick can read what a pass did, install or remove an
  output, run a pass or publish. An `ExtractState` a test inserts for a pass system of its
  own is that test's, and no pass of this crate reads it.
- With neither output installed a pass returns at once and keeps no mark. A dedicated
  server installs none. With one output installed the pass does that output's half.
- `set_sink` and `set_snapshot` are called between runs. Either makes the next pass full
  for both outputs, so outputs are installed before a world is first shown, not while it
  is.

*Extract writes nothing to the world.*

- The pass takes the world by shared reference (`ExtractState::extract(&mut self, &World,
  ..)`): it writes no column, hashed or not, no resource, no event, no channel, no log
  entry and no stamp, and it trims nothing. What it keeps between passes is in
  `ExtractState`, which no system of a tick can reach ("Placement and state"). So the world
  hash is the same before and after a pass, a tick's systems can read nothing a pass left,
  and a world's hashes do not depend on how many frames were drawn (D152, "A pass system
  leaves the world hash as it found it").
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
  is `None`: a structural change logged after the previous pass has been trimmed, which
  takes a change logged more than `SimulationBuilder::log_retention` steps before this
  pass). Every other pass is *incremental*.
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
  between two passes. The world does not change between `begin` and `end`. A pass at a
  tick other than the previous pass's always calls `begin` and `end`, with nothing between
  them when it has nothing to write, so the ticks of the sink's `begin`s are the ticks of
  extract's passes. A pass at the previous pass's tick that is not full and has no key to
  remove and no row to write calls nothing.
- The sink is infallible. `write` runs on the executor's workers inside extract's scope,
  several at once, so it never waits for a ring region, a fence or another job (the region
  a pass writes is free before the pass begins), and takes no lock that architecture
  section 3 does not permit inside a system. A sink that cannot take a row (the pass
  writes more rows than its ring region holds) drops it and counts the drop. Extract
  writes that row again only as it would have without the drop: by the settling write
  if it is still to come, once it changes again, or at a full pass; and a pass after
  `ExtractState::set_sink` is full.

*What the sink keeps.* A row carries the placement the instance has now and no previous
one: the world holds one `Transform` per row, and extract keeps no table of its own. What
follows is the contract the renderer's sink implements, and a test's sink checks.

- The sink holds, per key and world kind, a previous placement, a current one and a
  *base*: the placement it held as current when the first pass of the current tick began,
  that is, at its `begin` for that kind whose `ExtractPass::tick` differs from the tick of
  the `begin` before it for that kind. The base is the tick before's state as the render
  side placed it, or none for a key the sink did not hold then.
- At a `begin` whose tick is earlier than the tick of the `begin` before it for that kind,
  by the wrapping comparison, the sink drops every base of that kind and takes none until
  the next tick: what it holds is of a later tick than the world, and leads into nothing.
- A `write` sets the previous placement to the base and the current one to the row's.
  When `no_interpolate` is set, or the key has no base, it sets both to the row's and
  leaves the base as it was.
- The base costs one more placement per key and kind. A sink may take it lazily, at a
  key's first write in a tick, from the current placement it holds then, which no write
  has changed since the tick's first `begin`; it then also keeps the tick it took it at
  and how many `begin`s for that kind had dropped the bases then. A base counts only at
  that tick and that count, and the sink takes none in the tick of the last `begin` that
  dropped them.
- So the first write of a tick draws the instance from where the tick before left it, and
  a later write in the same tick draws it from that same place: not from where the frame
  before showed it, and not from a placement a flagged write of the tick put there. The
  one pass that writes rows at the tick of the previous pass without setting
  `no_interpolate` is a full pass in a prediction world, which is in practice a rollback in
  a frame with no step. `lactose-predict` reconciles every frame and ticks are fewer than
  frames, so a misprediction usually lands in such a frame. An idle pass and a full pass
  in a `Server` world flag every row, and a `Client` world's passes write none. After a
  resync to an earlier tick, whose pass flags every row, such a rollback finds no base and
  shows the corrected state at once, as the frame does: its snapshot takes the place of a
  cut, and is one.
- The two outputs agree on a row that is both an instance and a presented row, when every
  pass is published before the next and the reader acquires every publication, as `app`
  runs them: the sink's previous and current placements are the frame's
  `PresentFrame::transform` from the older and the newer snapshot, and the sink sets both
  to the row's exactly when the frame's `no_interpolate` is set. Two exceptions. A key new
  to the sink whose row the older snapshot holds (a row that gains a `MeshInstance`, a
  sink installed while a world is shown) is shown by the sink at its placement until the
  next tick, while the frame interpolates the row. After an edit made between ticks, until
  the next tick, the frame shows every presented row at the newer state ("A frame's
  transforms"); the sink does not.
- The sink compares the tick of each `begin` with the tick of the `begin` before it, which
  is the tick of extract's previous pass because every pass after a tick calls `begin`
  ("What a pass gives the sink"). So the sink drops its bases at exactly the passes whose
  tick is earlier than the previous pass's, which are the passes that flag every row. Were
  a pass after a tick in which nothing moved to call nothing, the sink's last `begin`
  would be older than extract's previous pass, and a resync to a tick at or after it would
  drop no base: the rollback after it would be drawn from a base of before the resync.
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
- on every row of a pass whose tick is earlier than the previous pass's, by the wrapping
  comparison: a prediction world resynchronised to an earlier tick (`Predictor::resync`),
  where nothing the sink holds leads into the state, and the sink drops its bases ("What
  the sink keeps"). The snapshot of such a pass is a cut ("The triple buffer"). The drop
  and the flag each show that pass's rows at once in a sink that keeps the bases; the
  flag alone does so for a sink that keeps none, and is what a consumer of the row reads.
  The drop alone keeps the rollback after the resync from being drawn from a base of
  before it;
- otherwise, when the row has a `Teleported` whose tick is not zero and is later than the
  pass's *covered tick*, by the wrapping comparison of ticks (the difference is in
  `1..2^31`). The covered tick is the world's tick at the previous pass or, at a pass of
  that same tick, the previous pass's covered tick: the tick of the state the sink keeps as
  its base ("What the sink keeps"), and, for a reader that keeps up, of the older
  snapshot of the pair. A frame that ran several ticks loses no teleport
  (`crates/physics-api/README.md`); a `Teleported` loaded from a save is in the past and
  flags nothing. With no previous pass this rule flags nothing. A correction's placement
  sets `Teleported` at the tick that re-simulates it (`crates/physics-api/README.md`,
  "Snapshot and restore") and is judged by the same rule, so it is flagged only when that
  tick is after the covered tick.
- A full pass in a prediction world, which a rollback causes at every misprediction,
  follows the last two rules alone. In a frame with no step its tick is the previous
  pass's: the sink draws each row it writes from its base, and the snapshot takes the place
  of the one published last ("Events and publication"), so both draw the local player
  from the state at the tick before to where the corrected run puts it, over what is left
  of the tick, also when a teleport earlier in the tick was flagged and shown at once. A
  correction that re-simulates that tick alone sets `Teleported` at it, after the covered
  tick, and both show it at once. A rollback before the world's first tick is shown finds
  no base and a pair of one tick, and both show it at once. The visual blend of a
  correction is presentation's, from `lactose-predict`'s `Correction`.

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
- What the snapshot's half costs (D164). A pass that writes a snapshot copies every
  presented row, so at every pass that follows a tick the half is a copy of the presented
  rows, whatever the dirty count, on the pass's own thread after the sink's half. The copy
  walks the presented rows once for each copied component, skipping the rows whose chunk
  lacks it, so it grows with the presented rows times the components the snapshot copies:
  in the review's probe (release, one worker, loads 9.5 to 14.5, steering) twelve more
  `extracted` components, each on one row, took the half at 5,000 presented rows from 107 to
  139 µs. The fill sorts the presented rows' places (each key with its chunk and its row) by
  key once and keeps them in `ExtractState` for the next pass. Each pass checks every place
  against its chunk's key before it copies, and sorts again only when a place no longer
  holds its key or the count of presented rows changed: after a presented row is spawned or
  despawned or changes archetype and so leaves its place, and under SOUR's layout modes,
  which permute rows at every sync point, at a pass after a permutation moved a presented
  row (1,409 to 1,551 sorts over 1,682 passes, 1,553 of which wrote, in the unit tests). An archetype change
  that keeps a row at its chunk and its row, and a move to another region, sort nothing: the
  place still holds its key, and a region is a column of the chunk. A place is a hint
  and never a key (architecture section 8.2): no byte of a snapshot depends on one, and the
  columns a row is copied from are read from its chunk at every pass, since a row can keep
  its chunk and its row while its archetype changes.

*The triple buffer.*

- A channel owns exactly three snapshot buffers. The reader holds the two of its last
  `acquire`; the writer writes the third. `acquire` exchanges handles under the channel's
  lock and copies nothing. Each side waits for the other at most for one exchange: the
  lock is taken once by a pass that writes a snapshot, once by each `publish` and once by
  each `acquire`, and is held for a constant-time exchange of buffer handles and event
  batches alone, never while a snapshot is written or read, an event batch folded or
  dropped, or a sink called. It is the one lock architecture section 3 permits inside a
  system for this channel (D157). The code is safe Rust.
- `acquire` returns `None` before the first publication, and called again with nothing
  published since, the same pair. Otherwise its newer snapshot is the newest published,
  and its older the snapshot that one follows, if the channel still has it (published, or
  held by the reader), else the reader's newer one. A snapshot follows the newest the
  channel had when it was published: the newest published, else the reader's newer. One
  that takes another's place ("Events and publication") follows what that one followed,
  and once acquired a snapshot follows the older of its pair. So a reader that acquires
  every publication gets the two newest published, and the first acquire returns the one
  snapshot published twice. An acquire stops publishing every snapshot published before
  its newer one: no later pair holds them.
- When the writer needs a buffer and none is free, the newest published snapshot is one
  the reader does not hold: the writer takes it back and writes it. A reader slower than
  the passes therefore misses snapshots and never blocks a pass, and its pair then spans
  more than one pass. Nothing the reader holds is ever written.
- An acquire made while nothing published after the reader's newer snapshot is still
  published (a pass took back the latest publication and is writing it) returns the
  reader's newer snapshot twice, and lets the older one go. With three buffers, the
  writer's and the reader's two, nothing else is left: a reader that held its pair there
  instead, acquiring at that point every time, would never see a later snapshot. Letting
  go, it leaves the next pass a free buffer, so what that pass publishes stays published,
  and the acquire after pairs the snapshot with it.
- A reader at the pass rate whose acquire falls inside a pass's span, between the pass's
  taking a buffer and its publication, gets such a pair every second acquire and sees every
  second snapshot. A pass after an acquire that returned two different snapshots finds no
  buffer free and takes back, unseen, what the pass before it published; the acquire inside
  it returns a window pair, which frees a buffer for the next pass, and the acquire inside
  that one pairs with what the window's pass published. A slower reader whose acquire falls
  there gets one every second acquire as well. Motion stays forward, and every transform of
  a window pair has `no_interpolate` set, its two snapshots being of one tick: a stutter,
  never a wrong draw. A frame that acquires after `publish` gets no window pair. The
  client's frame acquires once in every frame that published, after its `publish`, and in
  no other (D162), so it gets none: no fourth buffer is asked for, and architecture section
  5.3's triple buffer stands.
- `PresentSnapshot::cut` is set on a snapshot nothing before it leads into:
  - the snapshot of a full pass in a `Server` or `Client` world (a load, a restore, a
    world shown for the first time);
  - in every kind, a snapshot whose tick is earlier than the tick of the snapshot
    published last, whether or not that one is still published, by the wrapping
    comparison, which `publish` makes: in a prediction world, a resync to an earlier
    tick (`Predictor::resync` to an earlier clock target);
  - a snapshot that takes the place of a cut ("Events and publication").
- The channel keeps a *mark*: the last snapshot published that is a cut by one of the
  causes above, including one a second pass kept before `publish` ("Events and
  publication"). Until an acquire returns a newer snapshot published at or after the mark,
  every snapshot published is marked a cut as well, and a snapshot marked so does not move
  the mark. So the first acquire whose newer snapshot was published since a cut has a cut
  as its newer one, whatever was taken back, replaced or published after it, and a cut
  taken back before any acquire saw it still reaches the reader. That snapshot may be of
  a later pass than the one that cut: what a reader is guaranteed is that it blends
  nothing across a cut, not that it sees the cut's own state. An acquire made while a
  pass writes a snapshot it took back may return a snapshot from before the cut twice,
  which blends nothing either.
- The mark is passed at the latest by the third acquire after it that follows a
  publication: one may return a pair from before it, the next then finds nothing published
  after its newer snapshot and returns that one twice, and the third pairs it with what
  the pass after published. A reader that acquires every publication passes it at the
  first. Marking every snapshot until an acquire returned the latest one published would
  keep marking them for good under a reader that acquires once per two publications,
  while the second pass writes the first's taken-back buffer.
- Any other full pass in a prediction world is not a cut: it is a rollback, or a resync to
  the tick shown or a later one (`Predictor::resync` sets the tick and changes no row).
  `lactose-predict` reports a resync, and presentation blends a correction.
- One channel serves one world. A client's two worlds each have a channel and an
  `ExtractState`, and presentation reads both readers; `PresentSnapshot::kind` says which
  is which.

*Events and publication.*

- A snapshot a pass wrote is invisible to the reader until `ExtractState::publish`. A pass
  that runs again before `publish` updates the same unpublished snapshot, which stays a cut
  if a pass that wrote it made it one, and the snapshot of a full pass if one was full.
- `publish(events)` makes the unpublished snapshot, if there is one, the newest published,
  and appends `events` to the events the reader's next `acquire` returns, in one exchange:
  a frame never holds a snapshot without the events of the ticks it covers. With no
  snapshot to publish the events are appended alone, and an `acquire` before the first
  publication, which returns `None`, leaves them for the first frame. With no writer
  installed the batch is dropped, and so is every batch once the channel's reading half
  has been dropped: nothing accumulates for a reader that will never acquire.
- The channel keeps each published batch as it is, and the `acquire` that takes them folds
  them after the lock is released, in time linear in the events; the batches a dropped
  reader never took are dropped after it too. A reader that stops acquiring while its
  world keeps publishing (a hidden window, a hitch, a debugger) costs memory, one batch per
  publication with an event, until it acquires again; nothing done under the lock grows
  with that backlog: `publish` links a batch whose list node it allocated before taking
  the lock. The first acquire after the pause returns every event of it, folded. The client's
  frame never makes that backlog (D162): it acquires in every frame that published, a
  hidden window's among them, and a frame that publishes nothing leaves its world's events
  in the world's queue, for `log_retention` ticks at most. No event is dropped or capped.
- A snapshot written by a full pass in a prediction world, of the tick of the snapshot
  published last, takes that snapshot's place instead of following it: it follows what
  that one followed ("The triple buffer"), and is published in its place if that one
  still is, so a rollback in a frame with no step gives the pair (S_{t-1}, S_t') and not
  (S_t, S_t'). The corrected state is interpolated to from the tick before, as the sink's
  placements are ("What the sink keeps"). A snapshot that takes the place of a cut is a
  cut. The events published with the replaced snapshot are not affected.
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
  (`crates/sim/README.md`, "Runs", step 4), unless it was built with
  `SimulationBuilder::keep_present_events`, which `lactose-app` sets for a world it shows
  (D162): then the frame takes them after its passes, as above.

*A frame's transforms.*

- `PresentFrame::transform(key)` is the row's `Transform` in the newer snapshot as
  `current`, and in the older as `previous`, or `current` where the older holds none.
- Its `no_interpolate` is set when the newer snapshot is a `cut`; when the older holds no
  transform for the key; when the two snapshots are of one tick; or when the row's
  `Teleported` in the newer snapshot is not zero and is later than the older snapshot's
  tick, by the wrapping comparison.
- A pair is of one tick, or a pass writes a snapshot of the tick already published, in
  these cases and no other:
  - The first snapshot a channel publishes is published alone, so the pair is of its
    tick until a snapshot of another tick is published.
  - An acquire made while a pass writes a snapshot it took back returns the reader's newer
    snapshot twice ("The triple buffer"): shown at its state, with nothing drawn in
    between, until the next acquire pairs it with what that pass published. A full pass in
    a prediction world of that snapshot's tick then pairs from it, not from the tick
    before, which the reader let go.
  - An idle pass after an edit made between ticks to a presented row publishes a snapshot
    of the same tick beside the one before it, so until the next publication of a
    snapshot of another tick every presented row is shown at the newer state with
    nothing drawn in between. `app` edits no row between ticks in a running game
    (`crates/sim/README.md`).
  - A full pass in a `Server` or `Client` world of the tick already published (after
    `set_sink` or `set_snapshot`, a restore or a whole-world load to the same tick, the
    2^30 stamp rebase) publishes a cut, shown at once.
  - A full pass in a prediction world of the tick already published takes the place of
    that snapshot, and the pair goes on from the tick before. A rollback in a frame with
    no step replaces it with the corrected state. After `set_sink`, `set_snapshot` on a
    channel that has published, the stamp rebase, or `Predictor::resync` to the tick
    shown, the replacement holds the same state: nothing shown changes.
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
- A `cut` for a prediction world's resync to the tick shown or a later one, which this
  crate treats as a rollback, and the visual blend of a correction: presentation's, from
  `lactose-predict`'s surface, at milestone 4.
- Per-instance scale, a material override, the sun, shadows and light profiles: no reader
  before the milestone 3 renderer. `Light` and `Decal` reach the GPU scene through the
  snapshot at milestone 3; nothing draws them at milestone 2.
- Erased access to a snapshot (by `ComponentKey`): no reader names a component it cannot
  type yet.
- The engine's own view-model types, UI animation, the audio listener and markers:
  milestone 5's crates.

## Proof for milestone 2

The integration tests are one binary (D150), `tests/integration.rs`, with a module per file.
Each row below names the tests that hold it. The rollback, triple buffer and events rows
also name changes to the library that fail a test there: the two of the rollback row that
are tests, and the rest shown by hand, each change undone after.

| What | How | Held by |
|---|---|---|
| Registration | A simulation of every kind builds with the crate registered, and a prediction world under `lactose_predict::check_prediction_reads`, which admits extract by its key alone; a system naming `Res<ExtractState>` or `ResMut<ExtractState>` is refused at build in every kind; `lib.rs` asserts the bounds, the policies and the sizes at build | `registration.rs` |
| Extract writes nothing | The recorded test under "Extract writes nothing to the world", in every SOUR mode: after every step the world hash and each region's are equal before the passes, after a pass that follows the tick and after one that does not; the hash after the passes is recorded; outside the runner each stream equals, tick for tick, the stream of the same scene with no pass, and the passes wrote, flagged and removed rows and ran full passes | `sour.rs` (the module `recorded`): `a_server_scene_with_extract`, `a_prediction_scene_with_extract_and_a_rollback`, `the_streams_equal_those_with_no_pass` |
| The extract set | A world of each kind with instances, presented rows, rows that are both and rows that are neither: a counting sink and the snapshot hold exactly the stated rows; a prediction world's unclaimed rows are in neither; a `Client` world's sink gets no row and its snapshot no `Transform` | `extract::each_kind_extracts_its_set` |
| Full passes | The first pass, a pass after `set_sink` or `set_snapshot`, after a restore, after a whole-world load (its `set_tick`), after the structural log was trimmed past the previous pass's cursor: `full`, every instance written, no `removed`; `no_interpolate` and `cut` by kind; a pass after a tick or a region unload is incremental, and so is a pass after more ticks than the log's retention when what it logged since the previous pass is still held | `extract::a_pass_is_full_when_extract_cannot_tell_what_changed`, `extract::a_change_logged_inside_the_retention_after_more_ticks_than_it_keeps` |
| Incremental passes | By counts: a row moved at one tick is written by the pass after it and by the pass after the next tick with the placement it stopped at, then not again; a row edited between ticks is written by an idle pass with `no_interpolate`; an idle pass with nothing changed calls nothing, and a pass after a tick in which nothing moved calls `begin` and `end` alone; a `set_if_neq` of equal bytes writes nothing; an idle pass writes a snapshot only when a presented row changed | `extract::a_row_is_written_by_the_pass_after_the_tick_that_moved_it_and_once_more`, `extract::an_idle_pass_writes_a_snapshot_only_when_a_presented_row_changed` |
| A rollback in a frame with no step | One claimed row of a prediction world, four frames a tick, through every sequence of D157's row (S2, S2B, T, T2, R2, R2b, CU, CUT, F, RS, RSR, LZ, RS2) and three in which the row stands still for two or three ticks before a resync to the tick before the newest (RSS, RSS3) and a rollback after it (RSSR): a sink that takes bases at a tick's first `begin`, one that takes them lazily at a key's first write, and `PresentFrame::transform` give the same placement pair at every frame, and no frame draws from a placement of a later tick than the one it draws to. The two mutation tests assert their lists exactly: without the base drop RSR and RSSR fail; without the lazy sink's count of drops RS, RSR, LZ and RS2 fail. By hand: without the earlier-tick cut in `publish` RS fails; with a pass after a tick that calls nothing when it has nothing to write, RSSR fails; without extract's earlier-tick flag the agreement holds through the base drop, `extract::the_flag` fails, and the base-drop list grows to RS, RSR, RS2, RSS, RSS3 and RSSR | `rollback::the_sink_and_the_frame_agree_in_every_sequence`, `rollback::without_the_base_drop_a_sequence_fails`, `rollback::without_the_lazy_sinks_count_of_drops_a_sequence_fails` |
| Removals | A despawn, an unload, a removed `MeshInstance` or `Transform`, a released claim: listed once, in key order; a key that left and came back is listed and written | `extract::removed_lists_what_left_the_set` |
| The flag | A teleport at each tick of a three-tick frame flags the row; a saved `Teleported` does not; tick zero does not; every row of a pass at a tick earlier than the previous pass's is flagged; the comparisons hold across the tick's wrap | `extract::the_flag` |
| Layout and workers | The sink's rows sorted by key, and the snapshot's bytes, are equal under every layout capacity and chunk order of SOUR's modes and one, two and four workers | `extract::layout_and_workers_change_nothing` |
| The fill | The buffer every pass writes holds what `present-impl`'s fill (a list per column, each sorted, and the keys sorted again) writes of the same world at the same point, and a pass writes exactly when that fill would, and a pass that writes nothing leaves standing a snapshot that still holds what that fill writes of the world, in a `Server`, a `Client` and a `Prediction` world, over random sequences of moves, spawns, despawns, extracted components added and removed, region unloads and loads, restores and `set_tick` forward and back, under the five reader cadences of the triple buffer's row and a drawn one, at the production layout, at 1, 7 and 64 rows a chunk alone and under every layout mode of SOUR's matrix; a row that gains `Teleported` at its chunk and row is copied with it, its place holding; one place alone that stops holding its key, the lowest or the highest, sorts again; in a steady scene no pass sorts at the production layout or at a chunk capacity alone, a spawn, a despawn and an archetype change each sort once and a region change not at all, and under each of SOUR's modes the count is printed (30 sorts in 30 steady passes). Unit tests, since the place count is private. A fill reads the places only through the order `settle` returns for the pass's chunks, so a fill from places not settled on them does not compile, and it pushes each row's key with its bytes. A fill that reads a chunk's columns as they were at the pass that last sorted, a place check that skips the first row or the last, one that does not sort again when the count changes, and an idle-pass rule without its check of the log's entries in the fill and in the reference alike each fail a test here (shown by hand, each change undone after) | `extract::tests` (the library's unit tests) |
| The triple buffer | Every write is a real pass whose presented row holds the write's number, checked against a model of this file's rules after every acquire, under the five cadences of D157's row and random sequences: a held snapshot is never written (`Arc::get_mut` asserts the buffer a pass writes is unshared) and no pass waits for the reader but for one exchange; the pair is the model's; a reader that acquires every publication gets the two newest published; after the first publication no pair holds one snapshot twice except a window pair, never two in a row, every second acquire in the inside-the-span cadence; no pair backward in time but at a cut; the first newer snapshot published since a cut is one, also when the cut was taken back or replaced; the mark is passed by the third acquire after it, after one stale pair at most in the slow cadence; a prediction world's replacement keeps a cut and pairs from what it followed; a second pass before `publish` keeps a cut and a full mark; the render side on its own thread; across threads, at drawn cadences, each emission reaches one frame and a frame whose newer snapshot is of a tick has every emission published with or before that tick's first publication; a window pair driven across threads, with the taken-back snapshot's events in its frame. The exchange is safe Rust under one lock; no loom model or Miri run covers it. A mark cleared only by an acquire of a snapshot published after it, not at it, fails seven of these tests; publishing the snapshot and its events, or taking the pair and the events, in two exchanges fails `exchange::events_reach_one_frame_with_their_snapshot_across_threads`: with a yield between the two exchanges, in every run; with none, in most runs (of 20 runs each, taking them apart failed 18 and 20, publishing them apart 17 and 12, in fix round 2 and in review round 2) | `exchange.rs` |
| Events | Each emission is in exactly one frame's batch under four cadences with rollbacks that re-emit and cancel; an emission and its cancellation between two acquires are in neither list; a rollback's re-emission is not delivered again; with the reader dropped `publish` keeps no event; the events published before the first snapshot reach the first frame. Without the fold, or with `publish` replacing the batches it holds, a test fails | `frame::each_emission_reaches_one_frame`, `frame::a_cancellation_folds_into_its_emission`, `frame::with_the_reader_gone_publish_keeps_no_event`, `frame::events_published_before_the_first_snapshot_reach_the_first_frame` |
| The frame | `transform` for a moved, a teleported, a new and a `Client`-world row; one-tick pairs and cuts do not interpolate | `frame::a_frames_transforms` |
| View-models and debug draw | `set`, `get`, `remove`, erased reads by field ID, key order; a key collision panics; a non-`derived` view-model and a snapshot read of a non-`extracted` component check and do not build, and the build names each failing call (a scratch package of `tests/ui`); the segments of each shape | `view.rs`, `compile_fail.rs` |
| Cost | Extract's pass at 50,000 rows moved among 100,000 in four archetypes and sixteen regions, 5,000 of them presented rows the snapshot holds, half of them moved, their keys interleaved across the chunks of both presented archetypes so that no chunk order is key order (D164), with 10,000 printed beside: on a null sink that counts, under one worker and four, with no snapshot installed, with one, and with one whose order breaks at every tick (a presented row despawned and another spawned), the snapshot's part being the median with it less the median without. The counts asserted, and the layout at a small scene and at the timed one (every presented chunk of two rows or more split by a key of the other presented archetype; within an archetype the chunks run in key order), the times measured by an ignored test run by hand in release and reported, never asserted. The device's ring is the renderer's and is measured there | `cost.rs` |
