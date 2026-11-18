# `lactose-renderer`: the minimal renderer [CHEDDAR, LATTE]

The renderer of the milestone 2 plan's decision 5.4 and no more: the render slot table keyed
by `StableKey` (architecture section 12.4, D49), the staging ring that extract and the remote
upload write (section 5.3), the scatter that moves a frame's rows into the persistent instance
buffer in the cell-plus-offset format (D41), triangles drawn by instance from that buffer, the
orbit camera latched before the submit (D48), pipelines from cooked shaders, and the
`RenderFeature` pass-provider trait (section 12.5) with no provider. It reaches a device
through `lactose-rhi-api` and a frame through `lactose-render-graph`, and names no backend.
`lactose-presentation`, `lactose-terrain`, `lactose-debug-ui`, `lactose-ui` and a client
binary name this crate; `lactose-app` never does (decision 5.11).

| | |
|---|---|
| Tier | backend |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final for milestone 2's lanes and every body
that decides anything is `todo!()`. What runs is plain data: a slot's index, a batch's
accessors, a frame's counts. This file is the contract `renderer-impl` meets, and every
sentence under "Behaviour the signatures cannot express" is one a dependent crate may rely on.
Architecture sections 3, 5.3, 12.4 and 12.5 are the design, with the milestone 2 plan's
decisions 5.1 to 5.4, 5.11, 5.14 and 5.18; where this file and the architecture disagree, the
architecture wins and this file is wrong. Decision D161 records the choices they left open.
The surfaces below it are `crates/rhi-api/README.md` (D158), cited as **rhi-api**,
`crates/render-graph/README.md` (D160), cited as **graph**, and
`crates/present-api/README.md` (D157), cited as **present-api**. The two spikes are the
evidence for what a device does with each call, cited as **RHI**
(`docs/results/spike-rhi-apple-m1-2026-11-15.md`) and **Metal**
(`docs/results/spike-metal-apple-m1-2026-11-17.md`).

The interface is unstable (D43) until `renderer-impl`, `presentation` and `frame-impl` have
been written against it. Amending a signature is allowed in the change that needs it. Say so
in the change description and list what changed. `tests/consumers.rs` is a compile-only
sketch of what each consumer writes: a change that breaks one is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `Renderer`, `RendererDesc` | The renderer of one device: `new`, `device`, `sink`, `remote`, `add_feature`, `open`, `prepare`, `slots`, `batch`, `declare`, `latch`, `submitted`, `torn`; what it is made with |
| `Opened`, `FrameStats`, `FrameView`, `DeclaredFrame` | Whether a region is open; what a `prepare` took and did, in counts; what a `declare` is given; what it added to the declaration, by ids |
| `ExtractSink` | The renderer's `InstanceSink`: what extract writes a world's instances through. No read |
| `RemoteWriter` | What the presentation schedule writes a frame's remote instances through. No read |
| `SlotTable`, `Slot`, `SlotHolder`, `SlotIter` | The render slot table, read-only: `capacity`, `live`, `extent`, `get`, `holder`, `iter` |
| `StagingRow`, `ScatterEntry`, `InstanceRecord`, `CellTransform` | The ring's row, a batch's entry and the instance buffer's record, in the bytes the shaders read; the scatter's rule (`InstanceRecord::scatter`, `free`) and the vertex stage's (`InstanceRecord::drawn`, `CellTransform::apply`) as functions of the host |
| `ScatterBatch`, `BatchRange` | The batch of the frame last prepared: its entries and its dispatches |
| `CameraData`, `CameraLatch`, `LatchedCamera`, `Projection` | The camera buffer's contents and `CameraData::orbit`, the one statement of how they are computed; what a latch is given and what it wrote |
| `RenderFeature`, `FeatureView` | The pass-provider trait, and what a provider's passes may use of the frame |
| `Shaders`, and module `shader`: `SCATTER_GROUP`, `SCATTER_PUSH_BYTES`, `TRIANGLE_PUSH_BYTES`, `CELL_SIZE_CONSTANT`, `TRIANGLE`, `PALETTE`, `CLEAR` | The cooked shaders a renderer is made from, and the numbers the shaders and the host agree on |
| module `graph` | What a provider names from `lactose-render-graph`, re-exported and under that crate's contract: `FrameDecl`; `ImageId`, `BufferId`, `PassId`; `RenderPassDecl`, `ComputePassDecl`, `CopyPassDecl`; `ColourUse`, `DepthUse`, `BufferUse`, `BufferAccess`; `ImageImport`, `BufferImport`, `TransientImage`; `RenderRecorder`, `ComputeRecorder`, `CopyRecorder` |
| `LOG_TARGET` | `cheddar`, the target of the renderer's events |

The crate exports one trait, `RenderFeature`, which is not sealed: other crates implement it.
It has no `#[doc(hidden)]` item. Its private plumbing is `staging::Staging`, the atomics the
writers share with the renderer ("Concurrency").

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Given by |
|---|---|---|
| `renderer-impl`'s tests on null and on devices (its plan row) | Slots allocated, freed from despawns and reconciled after a restore, through extract's sink, with a prediction world's rollback; the batch held to "each slot once" on a device that runs no shader; duplicates across workgroups read back; frames held in flight; one frame's camera after the next is latched; the far-from-origin scene; a CPU reference | `Renderer::sink` in `ExtractState::set_sink`; `Renderer::slots`, `SlotTable::{get, holder, iter, live, extent}`; `Renderer::batch`, `ScatterBatch::{entries, dispatches, run}`; `FrameStats::duplicates`; `DeclaredFrame::instances` for a copy pass into a readback, and `InstanceRecord::from_bytes`; `Opened::Wait`, with a frame held behind an import's `ready` point (graph); a camera buffer for each region; `InstanceRecord::{scatter, free, drawn}`, `CellTransform::apply`, `CameraData::{orbit, clip}`, `shader::{TRIANGLE, PALETTE, CLEAR}` |
| Extract's criterion (`extract-50k`, decision 5.2) | The device's ring written by the sink with no read; a renderer made with no cooked shader; a region freed by the timeline value of an empty submit; a wait for a free region outside the measured pass | `ExtractSink`; `RendererDesc::shaders: None`; `prepare`, then `submitted` with the point the caller's submit signals; `open` returns `Opened::Wait` before the pass and waits for nothing itself |
| `lactose-presentation` (`presentation`; `remote-upload-5k`, decision 5.3) | To write each frame's remote instances through the same ring as extract, from its jobs, with no pass; each remote slot written once a frame, shown by counts; declares nothing a world registers | `Renderer::remote`, `RemoteWriter::write(&self, &[InstanceRow])`; `FrameStats::{remote_rows, duplicates, dropped}`; this crate declares no component, resource, system or applier, so a crate that names it gains none |
| The client's frame (`frame-design`, `frame-impl`): `lactose-app-client`, and `lactose-app` through a `dyn` object | The render side's steps as calls it places in the frame's order; objects that wait between steps in a world's resource or a `dyn` object's fields; a sink for each world shown; the point that frees a region, for pacing; the frame's counts; what to do after a failed submit; the pipeline cache complete at start | `open`, `prepare`, `declare`, `latch`, `submitted`, `torn`, each one call; `Renderer` is `Send + Sync + 'static` and `declare`'s borrow ends with `RenderGraph::record`; `Renderer::sink`; `Opened::Wait`; `FrameStats`; `torn`; `Renderer::new` makes every pipeline. No trait here is one `lactose-app` must name |
| `slang` (decision 5.14) | The interface of the three shaders: buffers, push blocks, constants, outputs, and the arithmetic | "The shader interface"; module `shader`; `InstanceRecord::{scatter, free, drawn}`, `CellTransform::apply`, `CameraData::clip` |
| `lactose-terrain`, `lactose-debug-ui`, later `lactose-ui` | To add passes to the frame, the first two without naming `lactose-render-graph` (`crate-tiers.toml`) | `RenderFeature`, `FeatureView`, `Renderer::add_feature`; module `graph` |
| The milestone 4 merge of a client's two worlds (verification, the row on section 9.4's shadowing) | The kind of every instance, so the prediction world's copy can shadow the replicated one | Slots are keyed by kind and key, and `SlotHolder::kind` says which |

What `frame-design` must still decide, since no signature here does:

- **Where the frame waits for a region.** `open` never waits. With `regions` regions the
  device may hold `regions - 1` frames; one more and `open` returns `Opened::Wait`. Who waits
  for that point, on which thread, and how that pacing meets the present thread's is the
  frame's. After torn frames `open` may return `Opened::Unsubmitted`, and the frame then
  owes a submit of its own ("Regions").
- **A frame with nothing to draw into.** A prepared frame is declared and submitted, or it
  is torn. A client with no target either does not prepare, so that the writers' rows wait
  in the open region and are dropped once it is full, or declares into a target of its own.
- **Whether extract of the next frame may begin before this frame is submitted.** This crate
  allows it from the moment `open` has returned `Ready` after `prepare`. `open` takes `&mut
  Renderer`, which `declare` lends to the recorders until `RenderGraph::record` returns, so
  the earliest places are between `prepare` and `declare`, and after `record`.
- **What it does with a frame's counts.** A lost kind and, after a full pass, dropped or
  unplaced rows leave instances missing until a full pass, which `ExtractState::set_sink`
  brings about ("Writers"). After `torn` it is every world shown.
- **How `lactose-app` reaches these calls.** `lactose-app` names no `[server_denylist]`
  crate, so it drives a `dyn` object of a foundation crate's trait that `lactose-app-client`
  implements over a `Renderer` and a `RenderGraph` (graph, "What each consumer needs";
  D160). `RemoteWriter` borrows the renderer, so the object lends it to the presentation
  schedule inside one of its methods.
- **The frame's values.** The target and its format, the extent, the fraction, the
  projection, the boom, and in the fixed-frame mode of decision 5.1 the sample the
  `LatestSample` holds.
- **The order of drops.** The renderer after the graph that submitted its frames, off the
  frame-critical workers ("Errors and drops").

## A frame, by step

1. **Open.** `Renderer::open` opens a free region to the writers, or returns the point that
   frees one. A new renderer has region zero open.
2. **Write.** Extract's pass for each world shown writes through its `ExtractSink`, inside
   `Simulation::run_passes`. Then the presentation schedule writes the remote instances
   through `Renderer::remote`'s writer. Nothing reaches the slot table or a submit yet.
3. **Prepare.** `Renderer::prepare` seals the region, applies what the writers left to the
   slot table, builds the frame's batch and uploads it. No region is open after it.
4. **Declare.** The caller builds a `FrameDecl`, imports its target, and calls
   `Renderer::declare`, which adds the scatter, the triangle pass and each provider's
   passes. The caller compiles and records the declaration (graph).
5. **Latch.** `Renderer::latch` reads the newest input and writes the frame's camera buffer.
6. **Submit.** The caller submits the recorded frame and tells the renderer how it ended:
   `submitted` with the frame's point, or `torn`.

Step 1 for the next frame may come any time after step 3.

## Behaviour the signatures cannot express

### Slots

- A slot is keyed by the kind of its instance's world and the instance's `StableKey`, and by
  nothing else. No `EntityId`, chunk or row index reaches this crate: `InstanceRow` carries
  none. This rules out a slot column in a world, a slot that follows an entity across a
  restore by its id, and a slot number that depends on the chunk layout, the worker count or
  the order of `write` batches.
- The table declares no system, component, resource or applier (decision 5.18). It changes
  inside `Renderer::prepare` and `Renderer::torn` alone, from what the writers left in the
  sealed region. `Renderer::slots` returns the state after the last of those calls.
- `prepare` first frees every slot of each kind lost in the frame, and applies nothing of
  that kind's passes ("Writers"). It then applies the region's other passes and its remote
  batch one after another, in the order they began. Applying one:
  1. Each key of the pass's `removed` that holds a slot of the pass's kind frees it; a key
     that holds none is ignored (present-api, "What a pass gives the sink").
  2. Each row whose instance holds a slot keeps it. The slot's mesh becomes the row's.
  3. The rows whose instance holds none are new instances. They take slots in ascending key
     order, each the lowest free slot. When no slot is free the row is left out and counted
     (`FrameStats::unplaced`): its instance is not drawn until a later row of it finds a
     slot.
  4. At the end of a full pass, every slot of the pass's kind whose instance the pass did
     not write is freed. This is the reconciliation after a restore, a rollback or a load:
     extract's pass after one is full (present-api, "Full and incremental passes").
- So a key that a pass removes and writes is a new instance, and may take the slot it just
  freed. A freed slot's record is not drawn from the frame that frees it on ("The scatter").
- The remote batch is applied as a pass of kind `WorldKind::Client` that is not full and
  removes nothing. A `Client` world's own passes write no row (present-api): their removed
  keys and their full passes free remote instances, and the remote batch of the same frame,
  which begins after them, gives those that still exist new slots.
- `SlotTable::extent` is one more than the highest held slot. The lowest-free rule keeps it
  near `live`.
- The same sequence of applied passes, each as a set of keys, gives the same table on every
  run ("Determinism").
- A key held under two kinds holds two slots. The prediction world's copy of the local
  player shadowing the replicated one is milestone 4's (decision 5.5).
- Nothing tells the renderer that a world is no longer shown: its kind's slots stay held
  and drawn. The frame clears a kind with a full pass that writes nothing, through a sink of
  its own: `begin` with `full` set, then `end`. For `WorldKind::Client` that frees the
  remote instances too.

### Writers

- `ExtractSink` meets present-api's "What a pass gives the sink" and "What the sink keeps".
  What that contract says the sink holds, it holds on the render side: the slot table holds
  which instances exist, the instance buffer holds each one's previous placement, current
  placement and base, and the ring carries current placements (D157).
- A pass's kind is its `ExtractPass::kind`. `Renderer::sink` may be called any number of
  times; every sink writes the renderer's open region, and each world shown holds its own.
- `begin`, `removed` and `end` leave notes in the region; `write` leaves rows. A note takes
  the room of one row: one for `begin`, one for `end`, one for each removed key, and two
  for a remote batch.
- `write`, on a sink and on the remote writer:
  - reserves the room of its rows with one atomic addition, so batches written at once take
    disjoint rows;
  - writes its rows as `StagingRow`s through one `Device::write_buffer` call over the
    range it reserved, and leaves each row's key and mesh beside it in memory of the
    host's, which is where `prepare` reads them: the ring itself is never read;
  - waits for nothing, allocates nothing and takes no lock but the device backend's on its
    records (rhi-api, "Threads"), which architecture section 3 permits in a system's jobs
    (D158);
  - never fails. A row that has no room, a row written while no region is open and a row
    the device refuses are dropped and counted, in the `FrameStats::dropped` of the next
    `prepare`. Extract writes a dropped row again only as present-api says: at its
    settling write, when it changes, or at a full pass.
- A `write` or a `removed` on a sink with no pass open is a caller's breach of present-api.
  Its rows are dropped and counted, and its keys are ignored.
- **A lost pass.** A pass whose `begin`, `end` or removed key had no room, or that began
  while no region was open, is lost, and so is its kind for the frame: the next `prepare`
  applies nothing of that kind's passes, frees every slot the kind holds, forgets the
  kind's bases and the tick of its last pass, so that its next `begin` is compared with
  none, as its first was, and reports it (`FrameStats::lost`). Nothing stale is drawn. The
  kind shows nothing until a full pass, which the frame brings about with
  `ExtractState::set_sink`. A lost `Client` pass does not stop the frame's remote batch
  from being applied after it, so the remote instances are back in the same frame.
- **A remote batch with no room.** A remote batch whose notes had no room, or that was
  opened while no region was open, is left out whole and its rows are counted as dropped.
  It loses nothing: the remote instances keep the placements they had.
- **The base.** The renderer takes an instance's base lazily, at its first write in a tick,
  as present-api allows, with the count of dropped bases it requires. `prepare` decides it
  on the host for each entry and the scatter carries it out: `ScatterEntry::TAKE_BASE` on
  the first write of an instance in a tick, `ScatterEntry::HAS_BASE` when the instance has
  a base that counts at that tick. Neither is set in the tick of a pass that began at an
  earlier tick than the pass before it for its kind, by the wrapping comparison of ticks,
  nor for an instance new since the tick's first pass. An idle pass that calls nothing
  leaves nothing, and a pass at a new tick with nothing to write leaves its `begin` and
  `end`, which is how the renderer learns the tick (present-api).
- **Remote rows.** `RemoteWriter::write` takes `InstanceRow`s whose kind is
  `WorldKind::Client`. Their entries carry neither base flag, so the scatter sets both
  placements to the row's whatever `no_interpolate` says. Each remote instance is written
  once a frame; one written twice is a duplicate ("The batch").
- **At once.** Passes of different worlds may be open at once, on different threads, and a
  pass may be open beside the remote batch. Their rows interleave in the region and are told
  apart by what is left beside each row. Two things are the frame's to order: the `Client`
  world's pass of a frame ends before that frame's remote batch begins, and no pass is open
  during `prepare` ("Concurrency").
- Neither writer has a method that returns a row, a key or a byte, and no call of this crate
  reads the ring on the host.

### Regions

- The ring is `RendererDesc::regions` regions of `RendererDesc::rows` rows each. A region is
  one of: *free*; *open*, between `open` and `prepare`; *sealed*, from `prepare` until the
  frame is ended; *held*, from `submitted(point)` until `point` is reached, or from `torn`
  until the point of the next frame `submitted`.
- At most one region is open and at most one is sealed. `open` opens the free region of the
  lowest index. It learns that a held region's point is reached by `Device::timeline_value`
  and never waits.
- **The ring never overwrites a region a frame in flight reads.** A writer writes the open
  region alone; a region is opened only when free; and a region is free only when no submit
  that was told to the renderer can still read it. With no region free, `open` returns
  `Opened::Wait` with the point of the held region whose frame was submitted longest ago,
  and a writer that runs anyway drops its rows. So the device may hold `regions - 1` frames
  while one more is written.
- **After a torn frame** its region has no point: a submit the torn frame made may still
  read it, and the graph signals the torn frame's point with its next submit (graph,
  "Submission"). The region is held until the next `submitted(point)`, whose point is above
  it. Where every region that is not free is sealed or held that way, `open` returns
  `Opened::Unsubmitted`. With a frame prepared, the caller ends it. With none, every region
  is a torn frame's, and the caller submits a frame with nothing of the renderer's in it (a
  declaration with no pass compiles and signals its point: graph, "Declaring") and calls
  `submitted` with its point, which a `submitted` with no frame prepared gives to those
  regions. No other sequence of calls leaves the renderer without a region for good.
- The point `submitted` is given is one whose reaching means that every submit made before
  it that read a region has completed. A frame point of the graph is one. A cost test with
  no graph signals a point from a submit of its own to the queue its other submits used.
- Each region has, beside its rows, the room for its frame's batch, the frame's draw
  arguments and a camera buffer of its own. A frame's submit reads its own region's and no
  other's, so frame N's camera is what N's submit reads after frame N + 1's latch has
  written another region's.
- A region's rows are one buffer of `rows * 48` bytes, `MemoryClass::Upload` with
  `BufferUsage::DEVICE_ADDRESS`, and that buffer is what the cost tests call the ring. The
  memory type and flags a backend gives it are the backend's choice for that class and
  usage. No call of rhi-api or of this crate reports them: a results file names them from
  the backend its harness links ("How each call is honoured").
- The room for a batch is the largest a frame can need, so `prepare` never leaves an entry
  out for want of it. `instances + 2 * rows` entries bound a batch: one for each row, and
  one free for each slot held when the frame began or given in it. After a torn frame no
  slot is held, and the run that frees every slot index ever given takes the place of the
  first of those ("The batch").
- **Room.** A region holds `rows` rows and notes together, and drops nothing while every
  pass and batch written between two `prepare`s fits. One pass of each kind of world and a
  remote batch, whose rows together are at most `instances`, fit in `instances + 8` plus
  the keys the passes remove. `RendererDesc::instances` bounds what is drawn; `rows` bounds
  what one frame may change.
- `prepare` is refused (`InvalidData`) when no region is open, while a sink's pass is open,
  and while the frame prepared before it is not ended. It then seals nothing.

### The batch

- `prepare` builds one batch for the frame: for each applied pass or remote batch, in the
  order they began, a run of entries, one for each slot the pass touches. `ScatterBatch`
  shows it, and `Renderer::batch` returns it until the next `prepare`.
- Two runs come before those, each only when it has an entry. First, in the first batch
  prepared after `torn`, a `FREE` entry for every slot index the table has ever given
  ("Errors and drops"). Then a `FREE` entry for every slot of a kind lost in the frame, the
  kinds in the order `Server`, `Client`, `Prediction`.
- **A run names each slot once.** The builder resolves it on the host:
  - of two rows of one pass that name one instance, the row later in the region is kept
    and the other is left out and counted (`FrameStats::duplicates`). Extract writes each
    instance once in a pass (present-api), so a duplicate is a writer's breach, and which
    of the two is later follows the writer's batch order and nothing better;
  - a slot freed and given again within one pass has the entry of its new instance and no
    `FREE` entry;
  - every other freed slot has one `FREE` entry.
- The builder then asserts that no slot is named twice in a run, in every build. A breach is
  a panic, which aborts in a binary: two invocations of one dispatch writing one record is
  the hazard decision 5.4 names, and no device reports it.
- A run is one dispatch, or more where it has more entries than
  `Capabilities::compute_groups[0] * SCATTER_GROUP`. `ScatterBatch::dispatches` lists the
  dispatches in recording order; none holds a slot twice, and a slot named in two dispatches
  is written by them in that order (rhi-api, "States": the dispatches of a compute pass run
  in recording order, each seeing the writes of those before it).
- The order of entries within a dispatch carries no meaning. `ScatterEntry::row` counts from
  the region's first row and depends on how the writers' batches fell; `slot` and `flags` do
  not.
- No call returns a row. A test on a device that runs no shader learns which row an entry
  reads from the rows it wrote itself: the entry's slot has a holder, the holder a key, and
  the test knows the row it wrote for that key. Of rows written from one thread, a later
  `write` call's rows, and a later row of one call, have the higher `ScatterEntry::row`.
- A frame with no entry declares no scatter pass.

### The scatter

- The scatter is one compute pass, declared with the frame's region as `BufferAccess::Read`
  and the instance buffer as `BufferAccess::Write`. For each dispatch it pushes the
  addresses of the region's rows, of the dispatch's first entry and of the instance buffer,
  and the entry count ("The shader interface").
- Each invocation applies `InstanceRecord::free` or `InstanceRecord::scatter` to the record
  of its entry's slot, with the entry's flags and the row the entry names. That function is
  the rule; the shader is a transcription of it.
- The scatter is declared only in a frame that also declares the triangle pass, which reads
  the instance buffer on the graphics queue. So the graph never moves it to the compute
  queue (graph, "Queues"), and the instance buffer never changes queue.
- The instance buffer's records are written by the scatter alone. The host never writes or
  reads the buffer; a test reads it through a copy pass of its own.

### Formats

All little-endian, with no padding but the reserved bytes shown, which are written as zero.
A placement is 40 bytes: the cell as three `i32`, the offset from the cell's centre as three
`f32` in metres, the rotation as four `f32`, `x y z w` (`RenderTransform`, D41). A position
is `cell * 64 + offset` on each axis (`RenderPosition`).

| Record | Bytes | Layout |
|---|---|---|
| `StagingRow`, a row of the ring | 48 | placement (0); flags (40): bit 0 `NO_INTERPOLATE`; reserved (44) |
| `ScatterEntry`, an entry of a batch | 12 | slot (0); row (4); flags (8): bit 0 `FREE`, bit 1 `TAKE_BASE`, bit 2 `HAS_BASE` |
| `InstanceRecord`, a record of the instance buffer | 128 | previous placement (0); current placement (40); base (80); flags (120): bit 0 `LIVE`; reserved (124) |
| `CameraData`, the camera buffer | 80 | `clip_from_cell`, sixteen `f32`, column-major (0); cell, three `i32` (64); reserved (76) |
| A vertex of the built-in triangle | 12 | position, three `f32` |
| The draw's arguments | 20 | `IndexedIndirectArgs` (rhi-api) |

`StagingRow` is the ring's final format (decision 5.4): the placement extract hands the sink
and the one flag the scatter reads from a row. A later milestone's instance data (mesh,
material, bounds) is keyed by slot and reaches the GPU scene beside the ring, from the slot
table, which already holds each instance's mesh; it adds no field to the row.

### The triangle pass

- One render pass: `view.target` as colour attachment zero with `Load::Clear(shader::CLEAR)`,
  and a transient `Depth32Float` image of `view.extent` cleared to one, compared with
  `Compare::Less` and written. It declares the instance buffer and the camera buffer as
  `BufferAccess::Read`, the triangle's vertices as `Read`, its indices as `Index` and the
  frame's arguments as `Indirect`.
- It records one `draw_indexed_indirect` of one record: three indices, `SlotTable::extent`
  instances, first instance zero. So `SV_VulkanInstanceID` is the slot. No capability beyond
  rhi-api's required set is used.
- For each instance the vertex stage reads the slot's record. Without `LIVE` it writes a
  position no fragment comes from, `(0, 0, 0, 0)`. Otherwise the vertex `v` of
  `shader::TRIANGLE` is drawn at `camera.clip(record.drawn(fraction, camera.cell).apply(v))`,
  in the words of `InstanceRecord::drawn`, `CellTransform::apply` and `CameraData::clip`.
  The cells are subtracted as integers before anything becomes a float, so a scene and its
  camera moved together by whole cells render the same pixels.
- The fragment stage writes `shader::PALETTE[slot % 7]`, each channel over 255.
- The pass draws every held slot. Culling, meshes and materials are milestone 3's.
- The renderer keeps nothing sized by the target. A resize is a `FrameView` with another
  extent and a target of the caller's: the depth image is a transient of the frame's plan,
  and a new extent compiles a new plan (graph, "Transients").

### The camera and the latch

- `CameraData::orbit` is the one statement of the camera. `latch` reads
  `LatestSample::read` once, inside the call, takes the yaw and the pitch of what it read,
  and writes `CameraData::orbit(pivot, yaw, pitch, boom, projection, extent)` to the frame's
  camera buffer, then flushes it. It returns what it wrote.
- The latch writes that one buffer (architecture section 5.3). It waits for no device and
  no signal and gates no submit; its one loop is `LatestSample::read`'s, which retries only
  while a publish is in flight (`lactose-input-host`). The caller makes it between
  `RenderGraph::record` and `RenderGraph::submit` (graph, "A frame, by step"). No recorder
  reads the camera's bytes; shaders reach it through its address.
- The boom is the caller's value. The recheck of the boom at the latch is not in milestone 2
  (decision 5.4).
- A declared frame is latched once before it is submitted. A frame submitted with no latch
  is drawn with whatever its region's camera buffer held.

### Pipelines and shaders

- `Renderer::new` makes every pipeline the renderer uses: the scatter's compute pipeline and
  the triangle pass's graphics pipeline, with `Shaders::target` as its colour format,
  `Depth32Float`, `Compare::Less`, depth write on and `Cull::None`. No frame makes one.
- It holds each `CookedShader`'s reflection to the interface before it reaches the device,
  and refuses (`InvalidData`, naming the shader and the field) a wrong stage, a push block of
  another size, a heap, a constant the interface does not name or a missing one, an input
  attachment, or outputs other than attachment zero.
- It sets the vertex stage's constant `CELL_SIZE_CONSTANT` to `RenderPosition::CELL_SIZE`.
- The renderer takes decoded shaders and opens no file. The pipeline cache is the device's:
  the client binary hands last run's bytes to the backend's constructor and writes
  `Device::pipeline_cache` to its file after `Renderer::new` has returned, when the cache
  holds every pipeline above (rhi-api, "The pipeline cache").
- `RendererDesc::shaders: None` makes no pipeline. `declare` is then refused, and
  everything else stands: the ring, the writers, `prepare`, the slot table and the batch.

### Features

- `Renderer::add_feature` keeps a provider for the renderer's life. `declare` calls each
  provider's `RenderFeature::declare` after the triangle pass, in registration order, with a
  `FeatureView` of the frame's ids. `RenderFeature::label` names the provider in the
  renderer's events and is not required to be unique.
- A provider's passes are the graph's as any pass is: kept, ordered and recorded by graph's
  rules. This crate adds two: a provider draws over the target with `Load::Keep`, and it
  does not write the camera buffer or the instance buffer.
- A provider's compute pass that reads the camera buffer shares it with the triangle pass,
  so the graph keeps it on the graphics queue (graph, "Queues"): no buffer of the
  renderer's reaches a second queue through a provider.
- `FeatureView` is not exhaustive. Milestone 3 adds what its passes introduce.
- A provider that may name `lactose-renderer` and not `lactose-render-graph` writes against
  module `graph`.

### Concurrency

| Call | Thread | Borrow |
|---|---|---|
| `ExtractSink::{begin, removed, end}` | The worker that runs extract's system | `&mut` sink |
| `ExtractSink::write` | Extract's forked jobs, several at once | `&` sink |
| `RemoteWriter::write` | The presentation schedule's jobs, several at once | `&` writer, which borrows the renderer mutably |
| `open`, `prepare`, `latch`, `submitted`, `torn`, `remote`, `add_feature` | The render side's job, one at a time | `&mut Renderer` |
| `declare`, and the recorders it adds | The declaring job; the graph's recording jobs, several at once | `&'f Renderer` |

- **What is shared across threads** is `staging::Staging`, between every sink and the
  renderer: the word that says which region is open and how many passes are open on it, each
  region's cursor and counts, and the key, mesh and pass left beside each row. Every such
  field is an atomic: a `StableKey` and an `AssetRef` go into words and back through
  `lactose-core`'s and `lactose-asset-api`'s public conversions, so the crate needs no cell
  and no `unsafe`. It has no `Mutex`, `RwLock`, condition variable or channel, and adds
  none: architecture section 3 permits none here.
- **The only locks a call of this crate takes** are the device backend's, inside
  `Device::write_buffer`, `flush_buffer` and `timeline_value`: the lock on its records and
  the lock on a handle's entry, which rhi-api names ("Threads") and section 3 permits on
  the per-frame path, systems' jobs included (D158). Neither is held while the bytes of a
  write are copied: section 3 holds them to no code the caller supplies.
- **The ownership rule.** A region's rows and notes are written while it is open and read,
  by `prepare`, after it is sealed. A pass's `begin` and `prepare`'s sealing change one
  atomic word, so they cannot both succeed: a `prepare` made while a pass is open is
  refused, and a pass that begins after the sealing finds no region open and is lost. The
  remote batch cannot be open during `prepare`: its writer borrows the renderer.
- A `write` that is still running when its pass's `end` returns is not possible: extract
  joins its jobs before it calls `end` (present-api).
- A recorder captures handles and counts by value and reads the renderer through `&`. It
  blocks on nothing (graph, "Recording").
- Nothing here waits. `Device::wait_timeline` is called by the frame, on `Opened::Wait`,
  and by the renderer's drop.

### Determinism

- The crate is outside the determinism boundary and writes nothing a world hashes: with the
  render side attached or not, a world's hashes are the same (criterion 3c).
- The slot table and each entry's `slot` and `flags` are a function of the passes applied,
  each as its kind, its tick, its fullness, its set of removed keys and its set of rows.
  They do not depend on the chunk layout, the worker count, the order or the split of
  `write` batches, or a hash map's iteration order. New instances take slots in ascending
  key order, by `StableKey`'s ordering.
- Two things do depend on how batches fell, and both are counted: which rows are dropped
  when a region has no room, and which of two duplicate rows is kept.

### Errors and drops

- A refused call (`InvalidData`) changes nothing. A device's error from `prepare` or `latch`
  leaves the frame prepared and not submittable: the caller ends it with `torn`. So does an
  error from the caller's compile, record or submit of a frame the renderer declared into.
- **`torn`.** Graph's rule after a failed submit is to rely on no buffer's bytes. Of the
  renderer's buffers a submit writes the instance buffer alone; every other is written by
  the host and read by a submit, so a torn frame cannot have changed it. `torn` therefore
  gives up the instance buffer and nothing else: it frees every slot, forgets every base
  and the tick of every kind's last pass, and empties the batch. The renderer keeps no
  placement on the host, so the instances return only as extract and the remote upload
  write them again, each as a new instance.
- **No record a torn frame left is drawn.** A record is drawn when its slot index is below
  `SlotTable::extent` and it has `LIVE`. The first batch prepared after `torn` begins with a
  run that frees every slot index the table has ever given, and its later runs write the
  slots given since. A record at an index no slot was ever given is below no extent: slots
  are given lowest first. If that frame is torn as well, the next one's batch begins the
  same way.
- A frame prepared and then ended by `submitted` without a `declare`, on a renderer made
  with shaders, never ran its batch, and is ended as `torn` ends it, but that its region is
  free at the point given.
- `Renderer`'s drop waits for the point of the last frame it was told was submitted, then
  destroys its buffers and pipelines. It is dropped after the `RenderGraph` whose frames
  used its buffers, off the frame-critical workers; the graph's drop has by then waited for
  every submit it made, a torn frame's among them (graph, "Errors and drops"). A destroy
  the device refuses because a submit is still pending (rhi-api, "Handles and lifetimes"),
  which is one of a frame the renderer was told no point for, is logged under `cheddar` and
  the object is leaked. While the thread unwinds a panic the drop waits for nothing and
  destroys what the device lets it.
- A sink dropped with a pass open leaves that pass lost.

## The shader interface

Three stages, cooked by RICOTTA for each target and handed over as `Shaders`. Every buffer
is reached through a device address in the push block; no stage uses the heap. A shader
reads `SV_VulkanVertexID` and `SV_VulkanInstanceID` (rhi-api, "Pipelines and the cooked
shader format"). Records are laid out as "Formats" gives them.

**Scatter, compute.** Workgroup `(64, 1, 1)` (`SCATTER_GROUP`). Push block, 32 bytes
(`SCATTER_PUSH_BYTES`):

| Offset | Field |
|---|---|
| 0 | address of the region's first `StagingRow`, read |
| 8 | address of the dispatch's first `ScatterEntry`, read |
| 16 | address of the instance buffer's first `InstanceRecord`, read and written |
| 24 | `count`, `u32`: the dispatch's entries |
| 28 | zero |

Invocation `i` returns when `i >= count`. Otherwise, with `e` the entry `i` and `r` the
record `e.slot`: if `e.flags` has `FREE`, `r.flags = 0`; else `r` becomes what
`InstanceRecord::scatter(e.flags, rows[e.row])` leaves. No specialisation constant.

**Triangle, vertex.** Push block, 32 bytes (`TRIANGLE_PUSH_BYTES`), shared with the fragment
stage:

| Offset | Field |
|---|---|
| 0 | address of the instance buffer's first `InstanceRecord`, read |
| 8 | address of the frame's `CameraData`, read |
| 16 | address of the triangle's first vertex, read |
| 24 | `fraction`, `f32` |
| 28 | zero |

One specialisation constant, id 0 (`CELL_SIZE_CONSTANT`): the cell size, `f32`. Outputs: the
clip-space position, and the slot as a `u32` that is not interpolated. The arithmetic is "The
triangle pass".

**Triangle, fragment.** Input: the slot. Output: colour attachment zero, one location, the
palette colour. No input attachment, no constant.

`slang`'s engine shader of decision 5.14 proves an interface specialised statically and a
link-time constant. Where it is one of these three, the workgroup size is the natural
link-time constant and the cell size stays a specialisation constant, which the host sets.

## How each call is honoured

Every call reaches a device through rhi-api and graph alone, so it is honoured on null and on
native Metal as the calls it makes are. The backend columns are those of rhi-api's own table.

| Call | What it calls | Null | Native Metal | Vulkan (MoltenVK on the M1) | Evidence |
|---|---|---|---|---|---|
| `Renderer::new`: buffers | `create_buffer`, every buffer `Sharing::Exclusive`: each region and its camera and arguments as `MemoryClass::Upload`, the instance buffer as `Device` with `DEVICE_ADDRESS` and `COPY_SOURCE`, the triangle's vertices and indices as `Upload`, written once | Bytes and a record for each | Shared storage, which is coherent; `gpuAddress` | Host-visible memory with a device address; type 1 on the M1, write-combined on a discrete GPU | RHI item 2, point 3; Metal item 2, point 3 |
| `Renderer::new`: pipelines | `create_compute_pipeline`, `create_graphics_pipeline` with one `Constant` | The reflection, held to the pass at a draw | A library function specialised by a function constant; the archive | A module and a specialisation; the driver's cache | RHI items 4 and 5; Metal items 6 and 7 |
| A writer's `write` | `write_buffer` on the open region, disjoint ranges at once | Fills the range; refuses an overlap (`host-overlap`) | A write into the buffer's contents | A write into the mapping | rhi-api, "Buffers and host access" |
| `open` | `timeline_value` | The counter | `MTLSharedEvent`'s value | The semaphore's counter | RHI item 8; Metal item 9 |
| `prepare` | `write_buffer` for the entries and the arguments; `flush_buffer` over what the frame reads | Marks the ranges flushed; a submit that names a buffer with an unflushed range is refused on a set that is not coherent (`unflushed`) | Nothing: shared memory is coherent | `vkFlushMappedMemoryRanges` where the type is not coherent | RHI item 2; Metal point 3 |
| `declare`: imports | `FrameDecl::import_buffer`, each `Sharing::Exclusive`, from `Undefined` on the graphics queue, which keeps a buffer's bytes; `transient_image` for the depth | States tracked from the declaration | No layouts; `useResource` from the pass's reach | Barriers the graph derives | graph, "Imports" |
| `declare`: the scatter | `compute_pass`; `ComputeRecorder::{bind_pipeline, push_address, push_bytes, dispatch}` | Typed pushes against the declaration; runs nothing | `dispatchThreadgroups` on a serial encoder, the buffers declared with their usage | `vkCmdDispatch`, a shader-write barrier between dispatches | RHI item 2; Metal item 2 |
| `declare`: the triangles | `render_pass`; `RenderRecorder::{bind_pipeline, bind_index_buffer, push_address, push_bytes, draw_indexed_indirect}` | Checks states and declarations; draws nothing | `drawIndexedPrimitives` with an indirect buffer, one record | `vkCmdDrawIndexedIndirect`, count one | RHI items 9 and 10; Metal items 5 and 12 |
| `latch` | `write_buffer`, `flush_buffer` on the frame's camera buffer | Judged at the submit, after the write | A write into shared memory | A write into the mapping | graph, "A frame, by step"; rhi-api, "Buffers and host access" |
| The renderer's drop | `wait_timeline`, the destroys | Refuses a destroy while pending | Objects released | Objects destroyed | rhi-api, "Handles and lifetimes" |

Null runs no shader. On null `renderer-impl` proves the batch from `Renderer::batch`, with
`InstanceRecord::scatter` standing for the shader, and the region rule from null's own:
`host-overlap`, `unflushed`, and a host write under a pending writer (`pending`). A host
write under a pending *reader* null cannot see (rhi-api), so the frames-in-flight proof is a
device test with the GPU made to lag.

**What neither spike shows.** The instance buffer is `MemoryClass::Device`: the host never
writes or reads it, the scatter writes it through its address and a test reads it through a
copy. RHI lists a buffer in device-only memory under "Not shown": every buffer of both
spikes sat in host-visible memory, which on the M1 is the only kind. rhi-api's sentence
"`MemoryClass::Device` is never touched by the host and is filled by a copy" is read here as
a rule for the host: a pass that declares the buffer `BufferAccess::Write` fills it as a
copy does. `renderer-impl`'s device test that reads the instance buffer back is the first
evidence of a shader's write into such a buffer, and on a discrete GPU the first of
device-local memory at all. No signature of this crate names the instance buffer's memory
class, so a finding there changes `Renderer::new`'s body and rhi-api's sentence, and nothing
a consumer wrote.

Three rules of the layers below that this crate leans on:

- **A buffer imported from `Undefined` keeps its bytes** (graph, "Imports"; D160). The
  renderer imports every buffer of its own that way, each frame, on the graphics queue.
  D160's item 7 gives null and rhi-api one exception, once a release toward another queue
  has run. No buffer of the renderer's is ever released: none is used on a second queue
  ("The scatter"). The facade stands with item 7 applied and without it.
- **One buffer, one flush state on null.** Null's `unflushed` is judged by buffer, not by
  range. Each region is therefore buffers of its own, so that a submit that reads one
  region is not refused for bytes a writer has put in the open one.
- **A host write does not land in a range a pending submit reads** (rhi-api). Null does not
  check it; "Regions" is how this crate keeps it.

## Not in this crate yet

Each is absent because no lane of milestone 2 needs it (decision 5.4). Adding one is an
amendment of the kind section 20 allows.

- **Meshes and materials in the GPU scene, cluster submission, culling, deferred lighting,
  the heap's use by materials, skinning, GPU VFX, light binning.** Milestone 3 and later.
  `SlotHolder::mesh` is kept and not drawn.
- **The cascade pads and the boom's recheck at the latch.** Decision 5.4.
- **Device-lost recovery.** Tested with the GPU scene it rebuilds, at milestone 3. `torn`
  is what milestone 2 has: every instance is written again by its world.
- **The shadowing of the replicated copy by the prediction world's.** Milestone 4
  (decision 5.5).
- **A forked `prepare`.** `prepare` takes no `JobContext` and resolves a frame's rows on one
  worker. `renderer-impl` reports its cost at 50,000 rows as steering; a forked form is an
  added call.
- **Views.** One camera, one target. The caller's `FrameDecl` has one view.
- **Static cooked instances from IO** (section 5.3), which reach the GPU scene without
  extract, at milestone 3.
- **A provider's own heap slots, a G-buffer in `FeatureView`, a fallible
  `RenderFeature::declare`.** With the first provider.

## Proof

There is nothing to run against a facade but its plain data. `tests/consumers.rs` compiles
what each consumer writes: `renderer-impl`'s proofs (slots through a world's sink, the batch
on null, the instance buffer read back, frames held in flight, the CPU reference), the cost
tests of the ring with no shader, the presentation schedule's remote upload from forked jobs,
the client's frame through a `dyn` object, and a provider written against module `graph`
alone. `tests/values.rs` runs the bodies that are real and holds this file's surface table to
the crate's exports. The static assertions in `src/lib.rs` hold `Renderer`, `ExtractSink` and
`SlotTable` to `Send + Sync + 'static`, `ExtractSink` to `InstanceSink`, `RenderFeature` to a
`Send + Sync` trait object, `RemoteWriter` to `Send + Sync` whatever it borrows, every
description and report type to plain data, and each format to its size.

`renderer-impl` owes its plan row, each sentence against this surface:

| The row's sentence | Shown through | The build that fails it |
|---|---|---|
| Slots are allocated on the CPU, freed from despawns and reconciled after a restore, handed over by extract's sink, with a prediction world's rollback | A world with the sink installed; `SlotTable` after each `prepare` | One that ignores `removed`; one that skips the sweep at a full pass's end |
| A batch names each slot once, resolved in batch order and asserted | On null: duplicate rows through `RemoteWriter::write`, then `ScatterBatch` and `FrameStats::duplicates` | One without the resolution: the assertion fires |
| Many duplicates in different workgroups of a large batch | On a device: the instance buffer copied out, each record equal to `InstanceRecord::scatter` of the kept row | The same build, with the assertion off |
| With the GPU made to lag, the ring never overwrites a region a frame in flight reads | Frames held behind an import's `ready` point; `Opened::Wait`; the readback of a held frame after later writes | One whose `open` ignores the point |
| Frame N's readback shows N's camera after N + 1's has been written | The same, with two latches | One camera buffer for all regions |
| Far from the origin in one cell, the same pixels | Two scenes a whole number of cells apart | A shader that subtracts after converting to float |
| A world stepped, extracted, rendered and compared with the CPU reference | `InstanceRecord::drawn`, `CellTransform::apply`, `CameraData::clip`, `shader::{TRIANGLE, PALETTE, CLEAR}` | |
| The sink and the frame agree (present-api's proof row "A rollback in a frame with no step") | present-api's sequences run through `ExtractSink`, the records read back on a device or computed with `InstanceRecord::scatter` on null | One that sets `HAS_BASE` in a dropped tick; one that takes the base at every write |

It also owes: the loom model or the argument for `Staging`'s open word, that a `begin` and a
sealing cannot both succeed; a test that `ExtractSink::write` from several jobs leaves every
row once; and, as steering, the ring's write cost and `prepare`'s cost for 50,000 rows.
