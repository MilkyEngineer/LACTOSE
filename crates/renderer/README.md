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

**Status: implemented.** Every public signature is final for milestone 2's lanes and every
body is written (D161's addendum at `renderer-impl`). The slot table, the sink, the ring, the
batch and the frame's declaration are proved on null, and the scatter, the triangle pass and
the camera on Vulkan devices, with the shaders of `shaders/renderer/` ("Proof"). This file is
the contract `renderer-impl` meets, and every sentence under "Behaviour the signatures cannot
express" is one a dependent crate may rely on.
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
| `Opened`, `FrameStats`, `FrameView`, `DeclaredFrame` | Whether a region is open; what a `prepare` took and did, in counts, with `lost`, `dropped_in` and `unplaced_in` by kind; what a `declare` is given; what it added to the declaration, by ids |
| `ExtractSink` | The renderer's `InstanceSink`: what extract writes a world's instances through. No read |
| `RemoteWriter` | What the presentation schedule writes a frame's remote instances through. No read |
| `SlotTable`, `Slot`, `SlotHolder`, `SlotIter` | The render slot table, read-only: `capacity`, `live`, `extent`, `get`, `holder`, `iter` |
| `StagingRow`, `ScatterEntry`, `InstanceRecord`, `CellTransform` | The ring's row, a batch's entry and the instance buffer's record, in the bytes the shaders read; the scatter's rule (`InstanceRecord::scatter`, `free`) and the vertex stage's (`InstanceRecord::drawn`, `CellTransform::apply`) as functions of the host |
| `ScatterBatch`, `BatchRange` | The batch of the frame last prepared: its entries and its dispatches |
| `CameraData`, `CameraLatch`, `LatchedCamera`, `Projection` | The camera buffer's contents and `CameraData::orbit`, the one statement of how they are computed; what a latch is given and what it wrote |
| `RenderFeature`, `FeatureView` | The pass-provider trait, and what a provider's passes may use of the frame, with `FeatureView::new` for a provider's own tests |
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
| The client's frame (`frame-design`, `frame-impl`): `lactose-app-client`, and `lactose-app` through a `dyn` object | The render side's steps as calls it places in the frame's order; objects that wait between steps in a world's resource or a `dyn` object's fields; a sink for each world shown; the point that frees a region, for pacing; the frame's counts, by kind; what to do after a failed submit; the pipeline cache complete at start | `open`, `prepare`, `declare`, `latch`, `submitted`, `torn`, each one call; `Renderer` is `Send + Sync + 'static` and `declare`'s borrow ends with `RenderGraph::record`; `Renderer::sink`; `Opened::Wait`; `FrameStats::{lost, dropped_in, unplaced_in}`; `torn`; `Renderer::new` makes every pipeline. No trait here is one `lactose-app` must name |
| `slang` (decision 5.14) | The interface of the three shaders: buffers, push blocks, constants, outputs, and the arithmetic | "The shader interface"; module `shader`; `InstanceRecord::{scatter, free, drawn}`, `CellTransform::apply`, `CameraData::clip` |
| `lactose-terrain`, `lactose-debug-ui`, later `lactose-ui` | To add passes to the frame, the first two without naming `lactose-render-graph` (`crate-tiers.toml`); to test their `declare` on null with no renderer and no cooked shader | `RenderFeature`, `FeatureView`, `Renderer::add_feature`; module `graph`; `FeatureView::new` over ids of a declaration of the test's own |
| The milestone 4 merge of a client's two worlds (verification, the row on section 9.4's shadowing) | The kind of every instance, so the prediction world's copy can shadow the replicated one | Slots are keyed by kind and key, and `SlotHolder::kind` says which |

What `frame-design` must still decide, since no signature here does. D162 decides each
(`crates/app/README.md`, from "The shown frame" to "Counts, torn frames and resets"):

- **Where the frame waits for a region.** `open` never waits. With `regions` regions the
  device may hold `regions - 1` frames; one more and `open` returns `Opened::Wait`. The
  wait for that point is off the frame-critical workers, and never a `block_on` inside a
  job: a worker that blocks stalls the frame (`lactose-task`). Which thread waits, and how
  that pacing meets the present thread's, is the frame's. The writers of a frame run only
  after `open` has returned `Opened::Ready`: a pass that begins with no region open is
  lost. After torn frames `open` may return `Opened::Unsubmitted`, and the frame then owes
  a submit of its own ("Regions").
- **A frame with nothing to draw into.** A prepared frame is declared and submitted, or it
  is torn. A client with no target either does not prepare, so that the writers' rows wait
  in the open region and are dropped once it is full, or declares into a target of its own.
  A region left unprepared, or left open by a refused `prepare`, holds several frames'
  passes and remote batches, and `prepare` applies the last of those batches alone
  ("Slots").
- **Whether extract of the next frame may begin before this frame is submitted.** This crate
  allows it from the moment `open` has returned `Ready` after `prepare`. `open` takes `&mut
  Renderer`, which `declare` lends to the recorders until `RenderGraph::record` returns, so
  the earliest places are between `prepare` and `declare`, and after `record`.
- **What it does with a frame's counts.** `FrameStats::lost(kind)`, `dropped_in(kind)`
  and `unplaced_in(kind)` each leave instances of that kind's world missing until a full
  pass of it, which `ExtractState::set_sink` on that world brings about ("Writers"). After
  `torn` it is every world shown. The remote batch's dropped and unplaced rows are in the
  totals alone and need nothing: its next batch writes every remote instance again.
- **How `lactose-app` reaches these calls.** `lactose-app` names no `[server_denylist]`
  crate, so it drives a `dyn` object of a trait of its own over `-api` types,
  `lactose_app::RenderSide` (D162), that `lactose-app-client`
  implements over a `Renderer` and a `RenderGraph` (graph, "What each consumer needs";
  D160). `RemoteWriter` borrows the renderer, so the object lends it to the presentation
  schedule inside one of its methods.
- **The frame's values.** The target and its format, the extent, which is one for the
  frame (`FrameView::extent`, which the latch takes too), the fraction, the projection, the
  boom, and in the fixed-frame mode of decision 5.1 the sample the `LatestSample` holds.
- **The order of drops.** The renderer after the graph that submitted its frames, off the
  frame-critical workers ("Errors and drops").

## A frame, by step

1. **Open.** `Renderer::open` opens a free region to the writers, or returns the point that
   frees one. The client's frame does not wait for it: it runs its ticks, defers its passes
   and calls `open` again at its next frame (D162); a step-driven caller may wait for the
   point on a thread of its own. A new renderer has region zero open. A frame that failed is followed by
   this step as any other is.
2. **Write.** Extract's pass for each world shown writes through its `ExtractSink`, inside
   `Simulation::run_passes`. Then, once the `Client` world's pass has ended, the
   presentation schedule writes the remote instances through `Renderer::remote`'s writer.
   Nothing reaches the slot table or a submit yet.
3. **Prepare.** `Renderer::prepare` seals the region, applies what the writers left to the
   slot table, kind by kind, builds the frame's batch and uploads it. No region is open
   after it. The frame reads its counts and gives a world whose kind they name a sink
   again.
4. **Declare.** The caller builds a `FrameDecl`, imports its target, and calls
   `Renderer::declare`, which adds the scatter, the triangle pass and each provider's
   passes. The caller compiles and records the declaration (graph).
5. **Latch.** `Renderer::latch` reads the newest input and writes the frame's camera buffer,
   at the extent the frame was declared with.
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
- **A renderer shows at most one world of each kind.** Two worlds of one kind would share
  its slots, so that each one's full pass freed the other's instances, and their passes at
  different ticks would forget each other's bases. No call refuses it; it is the frame's
  to keep.
- `prepare` first frees every slot of each kind lost in the frame, and applies nothing of
  that kind's passes ("Writers"). It then applies the other passes **kind by kind, in the
  order of `WorldKind`'s variants** (`Server`, `Client`, `Prediction`), and within a kind in
  the order they began. At most one remote batch follows the `Client` passes (below).
  Begin order across kinds is thread timing, since passes of different worlds may be open
  at once; within a kind it is the program's, since one world's sink begins and ends its
  passes in turn and the frame ends the `Client` pass before the remote batch begins.
  Applying one pass:
  1. Each key of the pass's `removed` that holds a slot of the pass's kind frees it; a key
     that holds none is ignored (present-api, "What a pass gives the sink").
  2. Each row whose instance holds a slot keeps it. The slot's mesh becomes the row's.
  3. In a full pass, every slot of the pass's kind whose instance the pass did not write is
     freed. This is the reconciliation after a restore, a rollback or a load: extract's
     pass after one is full (present-api, "Full and incremental passes"). It comes before
     step 4, so that a pass whose key set changed needs no slots beyond those of the
     instances it writes.
  4. The rows whose instance holds none are new instances. They take slots in ascending key
     order, each the lowest free slot. When no slot is free the row is left out and counted
     (`FrameStats::unplaced`, and `unplaced_in` for its kind): its instance is not drawn
     until a later row of it finds a slot.
- So a key that a pass removes and writes is a new instance, and may take the slot it just
  freed, as may a new instance of a full pass take a slot that pass's step 3 freed. A freed
  slot's record is not drawn from the frame that frees it on ("The scatter").
- **The remote batch** is applied as a pass of kind `WorldKind::Client` that is not full
  and removes nothing, after the `Client` passes. A `Client` world's own passes write no row
  (present-api): their removed keys and their full passes free remote instances, and the
  remote batch, applied after them, gives those that still exist new slots.
- **One remote batch a frame.** A region holds several frames' batches where a frame did
  not prepare, or its `prepare` was refused. `prepare` applies the last batch the region
  holds, and that one only where it is whole (its end note is in the region), no note of a
  `Client` pass (its `begin`, its `end` or a removed key) lies after the batch's opening,
  and the `Client` kind is not lost in the frame. Every other batch of the region is left
  out: the batch applied writes every remote instance again. So a batch is never applied
  over the removals of a `Client` pass that began after it, which would bring back an
  instance despawned since and count nothing, nor over those of a `Client` pass it
  overlapped, which a frame that breaks its ordering ("At once") makes: in either order
  of their beginnings that batch is left out. A lost `Client` kind applies no batch, since
  a loss has no place in the region to order it by: a pass lost for want of room may have
  left no note, and one lost with no region open is in no region. The remote instances
  are then missing until the next frame's batch, as after a batch with no room
  ("Writers"). Which batch is applied follows from the order the region holds the
  `Client` passes and the batches in, which is the program's, and from which kinds are
  lost.
- **What a batch that is not applied counts.** A remote batch that is not applied,
  whichever the reason (another batch is the region's last, a `Client` note lies after its
  opening, the kind is lost, or its end note is missing), counts its written rows in
  `FrameStats::remote_rows` alone. `FrameStats::dropped` counts only the rows a writer
  could not write: those that found no room, those written with no region open and those
  the device refused, never a written row. "Writers" and `RemoteWriter` follow this rule.
- `SlotTable::extent` is one more than the highest held slot. The lowest-free rule keeps it
  near `live`.
- The same passes of each kind, each as a set of keys, give the same table on every run,
  whichever kind's pass began first, where the same kinds are lost ("Determinism").
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
  for a remote batch, its opening and its end.
- Every note and every row a writer leaves carries the region's **open generation**, which
  `open` advances each time it opens a region and which the open word holds beside the
  region's index. The generation is 32 bits and never zero: it repeats after 2^32 - 1 opens,
  over two years at 60 frames a second, and the entries beside the rows start at zero, a
  generation no use has. A pass's notes and rows use the region and generation its `begin`
  read, and a remote batch's those its writer was made under, and neither reads the word
  again.
  `prepare` reads the sealed region's entries below `min(cursor, rows)` and skips any
  entry of another generation. Such an entry is at an index a writer of this use reserved
  and did not fill, because the device refused the write, and that writer counted the row
  as dropped: the index holds what an earlier use of the region left, which is never read
  as this use's. A writer stores a row's shadow entry only once its bytes have reached the
  device.
- `write`, on a sink and on the remote writer:
  - reserves the room of its rows with one atomic addition to the region's cursor, so
    batches written at once take disjoint rows. The cursor is 64 bits and never wraps, so
    a region left open for good, as by a client with no target that does not prepare,
    keeps refusing what does not fit;
  - writes the part of its reservation that lies below `rows` as `StagingRow`s through one
    `Device::write_buffer` call, and leaves each of those rows' key and mesh beside it in
    memory of the host's, which is where `prepare` reads them: the ring itself is never
    read. A reservation that straddles `rows` sends only its part below `rows` to the
    device, and the rest is dropped;
  - waits for nothing, allocates nothing and takes no lock but the device backend's on its
    records (rhi-api, "Threads"), which architecture section 3 permits in a system's jobs
    (D158);
  - never fails. A row that has no room, a row written while no region is open and a row
    the device refuses are dropped and counted, in the `FrameStats::dropped` of the next
    `prepare`, and a pass's also in its kind's `FrameStats::dropped_in`. Each count
    saturates. Extract writes a dropped row again only as present-api says: at its
    settling write, when it changes, or at a full pass, so the frame answers a kind's
    dropped rows with a full pass of its world.
- A `write` or a `removed` on a sink with no pass open is a caller's breach of present-api.
  Its rows are dropped and counted in the total, and its keys are ignored.
- **A lost pass.** A pass whose `begin`, `end` or removed key had no room, or that began
  while no region was open, is lost, and so is its kind for the frame: the next `prepare`
  applies nothing of that kind's passes, frees every slot the kind holds, forgets the
  kind's bases and the tick of its last pass, so that its next `begin` is compared with
  none, as its first was, and reports it (`FrameStats::lost`). Nothing stale is drawn. The
  kind shows nothing until a full pass, which the frame brings about with
  `ExtractState::set_sink`. A lost `Client` kind applies no remote batch either ("Slots"),
  so the remote instances are missing until the next frame's batch.
- **With no region open** there is no region to note a loss in. A pass that begins then,
  and a row written then, are recorded in the renderer-wide record of kinds lost and rows
  dropped: the rows of each kind's passes, and the rows with no kind on their own, whose sum
  is the total. The next `prepare` takes the record with a swap after it has sealed its
  region, and `open` never resets it, so a loss is reported no later than the kind's next
  pass is applied; reported one `prepare` early, it frees the kind until a full pass and
  shows nothing stale. A sealing may take a kind's loss and not yet the rows its pass drops
  after it, which the next takes; it never takes those rows before the loss, since a pass
  marks its loss before it drops a row and the swap takes the rows first and the lost kinds
  last, the kinds' rows added with `Release` and swapped with `Acquire`
  (`models::a_kinds_rows_are_never_taken_before_its_loss` checks the order over every
  interleaving; that the orderings carry it to weak memory, neither loom nor Miri
  checks). No frame's rows dropped by kind exceed its rows dropped: the total is summed as
  it is taken.
- **A sink dropped with a pass open** leaves that pass lost and, where its `begin` took a
  count, gives it back, so the region can still be sealed and the `prepare` that seals it
  sees the loss (`ExtractSink`'s `Drop`; "Concurrency"). Only a pass whose `begin` took a
  count gives one back, by its `end` or by the drop: a pass lost at its `begin` took none.
  A sink passed to `mem::forget` with a pass open never gives its count back, so every
  later `prepare` of the renderer refuses, with `InvalidData`, as it refuses one made while
  a pass is open.
- **A sink outlives its renderer.** A sink holds the ring's shared state and the device.
  What it writes after the renderer's drop is lost: a pass finds no region open, or the
  one the renderer left open, whose buffers the renderer destroyed and the device refuses
  writes to, and no `prepare` reads the counts. The device lives until the last sink is
  dropped (`writers::a_sink_outlives_its_renderer`). `lactose-app` drops the render side,
  the graph before the renderer, before `Server::shutdown` drops the world and its sink,
  and the device's last holder after both (`crates/app/README.md`, "Shutdown").
- **The remote batch takes no count** of the open word: the writer borrows the renderer
  mutably, which keeps `prepare` out already. So a `RemoteWriter` passed to `mem::forget`
  cannot keep the renderer from preparing: its batch has no end note, and a remote batch
  with no end note is left out whole, counted as "Slots" says.
- **A remote batch with no room.** A remote batch whose notes had no room, or that was
  opened while no region was open, is left out whole, counted as "Slots" says: its written
  rows in `remote_rows`, and those that found no room or no region in `dropped`.
  It frees nothing, and the remote instances that hold slots keep the placements they had.
  After a full or a lost `Client` pass in the same frame, which freed them, the remote
  instances are missing until the next frame's batch.
- **The base.** The renderer takes an instance's base lazily, at its first write in a tick,
  as present-api allows, with the count of dropped bases it requires. `prepare` decides it
  on the host for each entry and the scatter carries it out: `ScatterEntry::TAKE_BASE` on
  the first write of an instance in a tick, `ScatterEntry::HAS_BASE` when the instance has
  a base that counts at that tick. Neither is set in the tick of a pass that began at an
  earlier tick than the pass before it for its kind, by the wrapping comparison of ticks,
  nor for an instance new since the tick's first pass. An idle pass that calls nothing
  leaves nothing, and a pass at a new tick with nothing to write leaves its `begin` and
  `end`, which is how the renderer learns the tick (present-api). Lazy and eager agree
  because only a kind's own passes write its instances with base flags: a `Client`
  world's passes write no row, and remote entries take no base (`tests/model.rs`, whose
  model is eager, writes no row in a `Client` pass).
- **Remote rows.** `RemoteWriter::write` takes `InstanceRow`s whose kind is
  `WorldKind::Client`. Their entries carry neither base flag, so the scatter sets both
  placements to the row's whatever `no_interpolate` says. Each remote instance is written
  once a frame; one written twice is a duplicate ("The batch").
- **At once.** Passes of different worlds may be open at once, on different threads, and a
  pass may be open beside the remote batch. Their rows interleave in the region and are told
  apart by what is left beside each row, and the order in which they began changes nothing
  of the table ("Slots"). Two things are the frame's to order: the `Client` world's pass of
  a frame ends before that frame's remote batch begins, and no pass is open during
  `prepare` ("Concurrency"). A frame that overlaps the two has that batch left out
  ("Slots"), and its remote instances wait for the next frame's batch.
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
  first of those ("The batch"). A `BatchRange` names an entry by a `u32`, so
  `Renderer::new` refuses, with `InvalidData`, a description whose bound is past
  `u32::MAX`, before it makes a buffer; the batch's own check that it is below 2^32
  entries is then an assertion that cannot fail.
- **Room.** A region holds `rows` rows and notes together, and drops nothing while every
  pass and batch written between two `prepare`s fits. One pass of each kind of world and a
  remote batch, whose rows together are at most `instances`, fit in `instances + 8` plus
  the keys the passes remove. `RendererDesc::instances` bounds what is drawn, a load that
  replaces a kind's whole key set included, since a full pass frees what it did not write
  before it places what is new ("Slots"). That holds within one kind: a kind applied
  earlier that grows in a frame cannot take the slots a later kind's full pass frees in the
  same frame. Its rows that find none are unplaced and counted (`unplaced_in`), the frame
  answers with a full pass of that kind's world, and the next frame places them. `rows`
  bounds what one frame may change.
- `prepare` is refused (`InvalidData`) when no region is open, while a sink's pass is open,
  and while the frame prepared before it is not ended. It then seals nothing.

### The batch

- `prepare` builds one batch for the frame: for each applied pass or remote batch, in the
  order they are applied ("Slots"), a run of entries, one for each slot the pass touches.
  `ScatterBatch` shows it, and `Renderer::batch` returns it until the next `prepare`.
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
  `min(compute_groups[0], u32::MAX / SCATTER_GROUP) * SCATTER_GROUP` of
  `Capabilities`, computed in `u64`: the product alone overflows `u32` under MoltenVK on the
  M1, which reports 2^30 groups, and at a desktop driver's 2^31 - 1.
  `ScatterBatch::dispatches` lists the dispatches in recording order; none holds a slot
  twice, and a slot named in two dispatches is written by them in that order (rhi-api,
  "States": the dispatches of a compute pass run in recording order, each seeing the writes
  of those before it).
- The order of entries within a dispatch carries no meaning. `ScatterEntry::row` counts from
  the region's first row and depends on how the writers' batches fell; `slot` and `flags` do
  not.
- No call returns a row. A test on a device that runs no shader learns which row an entry
  reads from the rows it wrote itself: the entry's slot has a holder, the holder a key, and
  the test knows the row it wrote for that key. Of rows written from one thread, a later
  `write` call's rows, and a later row of one call, have the higher `ScatterEntry::row`.
- A frame with no entry declares no scatter pass.

### The scatter

- The scatter is one compute pass. A region is several buffers, and the pass declares them
  by buffer: the region's entries buffer `BufferAccess::Read`, the instance buffer
  `BufferAccess::Write`, and the region's rows buffer `BufferAccess::Read` only where the
  batch has an entry that is not `FREE`. For each dispatch it pushes the addresses of the
  region's rows, of the dispatch's first entry and of the instance buffer, and the entry
  count ("The shader interface"); in a frame whose batch is `FREE` entries alone it pushes
  zero for the rows' address, which no invocation reads. So no frame declares a rows buffer
  no writer has filled, which null refuses (`undefined-read`): a full pass that writes
  nothing, a lost kind and the run after `torn` each make such a batch.
- Each invocation applies `InstanceRecord::free` or `InstanceRecord::scatter` to the record
  of its entry's slot, with the entry's flags and the row the entry names. That function is
  the rule; the shader is a transcription of it.
- The scatter is declared only in a frame whose triangle pass also declares the instance
  buffer, which it does in every frame with a scatter, a frame whose batch leaves
  `SlotTable::extent` at zero included ("The triangle pass"). So the scatter always has a
  graphics-queue dependency, the graph never moves it to the compute queue (graph,
  "Queues"), and the instance buffer never changes queue.
- The instance buffer's records are written by the scatter alone. The host never writes or
  reads the buffer; a test reads it through a copy pass of its own.
- **The instance buffer's import.** `declare` imports the instance buffer in every frame,
  whether a pass of the frame uses it or not, with `last` `BufferState::ShaderRead` on the
  graphics queue. So every frame leaves it there: by its last use, or, where no pass used
  it, by the one transition the graph makes from `initial` (graph, "Imports").
  `renderer-impl` keeps the import unconditional, since the rule that follows rests on it.
  `declare` imports the buffer from `ShaderRead`, an `Exclusive` import in the state and
  on the queue it is in, which needs no `ready` (graph, "Imports"). The renderer's first
  frame, and the first frame declared after `torn`, import it from `Undefined` instead,
  which keeps its bytes on the one queue it is used on (graph, "Submission": a torn
  frame's imports are in no state the graph vouches for). D160 allows it: `initial` is the
  caller's word.
- **What a frame's first pass waits for.** The import from `ShaderRead` narrows nothing as
  the frame is declared. The scatter also imports the region's entries, and its rows
  wherever an entry reads one, from `Undefined`, which rhi-vulkan carries out with a source
  of every command and every memory write, recorded just before the scatter, and a
  barrier's execution scope is not its buffer's. So the scatter waits for every earlier
  command on the graphics queue, the previous frame's fragment shading, attachment writes
  and copies included; a frame with no scatter waits the same way at the triangle pass,
  through the camera's, the vertices', the indices' and the arguments' imports from
  `Undefined`. The cost is that a frame's first pass never overlaps the work before it on
  the queue: one drain of the queue a frame, which no figure here measures. The lever, for
  when the drain is measured to cost, is to narrow those imports: the vertex and index
  buffers from the states their first frame leaves them in, and each region's buffers from
  their `last` state after the region's first use, each with a bit for its first use as the
  instance buffer has `fresh`, so that null's `state` rule holds. `renderer-impl` keeps
  them as they are. The held-frame tests rest on these masks (`tests/device.rs`, `Hold`):
  the `Lag` form on the `Undefined` imports' every-command source, which alone orders the
  scatter and the triangle pass behind the lag's dispatch, an independent pass at the
  graph's head; the `Gate` form on rhi-vulkan's wait on the target's `ready` point at every
  command (`submit.rs`, the `point` closure), which alone keeps the scatter and the camera's
  vertex read behind the release. Adopting the lever, or narrowing that wait, must re-run
  the device round's mutants `open-ignores-point` and `one-camera-buffer` against each form
  alone (`for hold in [Hold::Lag]`, then `Gate`): on a GPU that overlaps independent
  dispatches, the narrowed form would pass both.
- **Plans.** The graph keys a plan on the declaration's shape (graph, "Compile and its
  cache"), and a frame's shape changes with whether it scatters, whether its batch reads
  rows, whether it draws and whether it imports the instance buffer from `Undefined`. In
  steady state the declaration alternates three plans: a frame with no tick, a tick with
  rows, and a tick that only despawns. The renderer's first frame, a frame that frees every
  slot, a frame with the table empty and the first frame after `torn` take one each:
  `graph::the_steady_state_alternates_three_plans_and_each_edge_takes_one` sees seven
  plans in sixteen frames with `GraphDesc::plans` at 16. Below three the cache thrashes:
  at two, every frame of the alternation compiles. Each plan owns its transients (graph,
  "Transients"), here the depth image at the target's extent, `Depth32Float`: about
  14.7 MB at 2560x1440 and about 33 MB at 3840x2160 wherever it is not memoryless, as on
  a desktop GPU, and each miss costs a compile and an image's creation. So the caller's
  `GraphDesc::plans` holds the steady state's three beside the plans of its own passes.

### Formats

All little-endian, with no padding but the reserved bytes shown, which are written as zero.
A placement is 40 bytes: the cell as three `i32`, the offset from the cell's centre as three
`f32` in metres, the rotation as four `f32`, `x y z w` (`RenderTransform`, D41). A position
is `cell * 64 + offset` on each axis (`RenderPosition`). How a shader declares each record
so that its offsets are these is "The shader interface", "Records".

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
  `Compare::Less` and written. The viewport and the scissor are the whole of `view.extent`,
  which `declare` refuses (`InvalidData`) where a side is zero or above the device's
  `Capabilities::viewport_extent`: a submit would refuse such a viewport (`bounds`), and
  so tear every frame at that extent.
- Where `SlotTable::extent` is above zero the pass declares, by buffer, the instance buffer,
  the region's camera buffer and the triangle's vertex buffer `BufferAccess::Read`, the
  triangle's index buffer, a buffer of its own, `BufferAccess::Index`, and the region's
  arguments buffer `BufferAccess::Indirect`, and records one `draw_indexed_indirect` of one
  record: three indices, `SlotTable::extent` instances, first instance zero. So
  `SV_VulkanInstanceID` is the slot. No capability beyond rhi-api's required set is used.
- Where `SlotTable::extent` is zero it records no draw, and the pass clears the target and
  the depth alone. It then declares none of those buffers, but for the instance buffer
  `BufferAccess::Read` in a frame that has a scatter, which keeps the scatter on the
  graphics queue ("The scatter") and reads what that scatter wrote. So no frame before the
  first scatter declares an instance buffer nothing has written, which null refuses
  (`undefined-read`). `extent > 0` means a scatter wrote the held slots' records in an
  earlier frame that was submitted, or writes them in this one before the draw; after
  `torn` the run that frees every slot index is that scatter.
- For each instance the vertex stage reads the slot's record. Without `LIVE` it writes
  `(2, 2, 2, 1)` to all three vertices, a position outside the clip volume's `x <= w`
  plane, so clipping alone discards the triangle, with depth clamp or without. Otherwise
  the vertex `v` of `shader::TRIANGLE` is drawn at
  `camera.clip(record.drawn(fraction, camera.cell).apply(v))`, in the words of
  `InstanceRecord::drawn`, `CellTransform::apply` and `CameraData::clip`. The cells are
  subtracted before anything becomes a float, as `uint` with the result reinterpreted as
  `int`, `int(uint(a) - uint(b))`: that wraps on every target, where a signed `int`
  subtraction that overflows is undefined in MSL, and the wrap is reachable at the world's
  edge, where `RenderPosition` saturates its cell. So a scene and its camera moved
  together by whole cells render the same pixels.
- The fragment stage writes `shader::PALETTE[slot % 7]`, each channel over 255.
- The pass draws every held slot. Culling, meshes and materials are milestone 3's.
- The renderer keeps nothing sized by the target. A resize is a `FrameView` with another
  extent and a target of the caller's: the depth image is a transient of the frame's plan,
  and a new extent compiles a new plan (graph, "Transients").

### The camera and the latch

- `CameraData::orbit` is the one statement of the camera. `latch` reads
  `LatestSample::read` once, inside the call, takes the yaw and the pitch of what it read,
  and writes `CameraData::orbit(pivot, yaw, pitch, boom, projection, extent)` to the frame's
  camera buffer, with `extent` the one the frame was declared with, then flushes it. It
  returns what it wrote.
- The latch writes that one buffer (architecture section 5.3). It waits for no device and
  no signal and gates no submit; its one loop is `LatestSample::read`'s, which retries only
  while a publish is in flight (`lactose-input-host`). The caller makes it between
  `RenderGraph::record` and `RenderGraph::submit` (graph, "A frame, by step"). No recorder
  reads the camera's bytes; shaders reach it through its address.
- The boom is the caller's value. The recheck of the boom at the latch is not in milestone 2
  (decision 5.4).
- A declared frame is latched once before it is submitted, at the frame's one extent, the
  `FrameView::extent` `declare` stored, so that the aspect ratio is the viewport's.
  `CameraLatch` carries no extent. `latch` refuses (`InvalidData`) a prepared frame that was
  not declared, which has no extent.
- A frame submitted with no latch is the caller's breach. Where its region's camera buffer
  was never written, null refuses the submit (`undefined-read`); otherwise a device draws
  it with the camera an earlier frame latched into that region.

### Pipelines and shaders

- `Renderer::new` makes every pipeline the renderer uses: the scatter's compute pipeline and
  the triangle pass's graphics pipeline, with `Shaders::target` as its colour format,
  `Depth32Float`, `Compare::Less`, depth write on and `Cull::None`. No frame makes one.
- It holds each `CookedShader`'s reflection to the interface before it reaches the device,
  and refuses (`InvalidData`, naming the shader and the field) a wrong stage, a push block of
  another size than "The shader interface" gives (32 bytes for the scatter and the vertex
  stage; 0 or 32 for the fragment stage, which reads nothing of it), a heap, a constant the
  interface does not name or a missing one, a `CELL_SIZE_CONSTANT` whose `ConstantSlot::ty`
  is not `ConstantType::Float`, an input attachment, or outputs other than
  attachment zero.
- **The scatter's workgroup size is checked.** `CookedShader` carries it as `workgroup` since
  the format's version 2 (rhi-api ask (c), D161; D158's addendum of lane `shader-format`),
  and `renderer-impl`'s `new` refuses (`InvalidData`, naming the shader and the field) a
  scatter whose size is not `(SCATTER_GROUP, 1, 1)`. A scatter cooked at a smaller size
  leaves entries unwritten, and one at a larger size runs invocations past a dispatch's
  groups that the count check stops.
- It sets the vertex stage's constant `CELL_SIZE_CONSTANT` to `RenderPosition::CELL_SIZE`,
  as a `ConstantValue::Float`.
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
| `declare`, and the recorders it adds | The declaring job; the graph's recording jobs, several at once | `&'f Renderer`; `declare` stores the declared word |

- **What is shared across threads** is `staging::Staging`, between every sink and the
  renderer: the word that says which region is open, under which open generation, and how
  many passes are open on it; each region's 64-bit cursor and counts; the key, mesh, pass
  and generation left beside each row; and the renderer-wide record of kinds lost and rows
  dropped while no region is open. Every such field is an atomic: a `StableKey` and an
  `AssetRef` go into words and back through `lactose-core`'s and `lactose-asset-api`'s
  public conversions, so the crate needs no cell and no `unsafe`. It has no `Mutex`,
  `RwLock`, condition variable or channel, and adds none: architecture section 3 permits
  none here.
- **The open word** is one `AtomicU64`: the open generation in the high 32 bits, the open
  region's index in the next 16, all ones while no region is open, so `Renderer::new`
  refuses more than 65,535 regions, and the count of open passes in the low 16. A `begin`
  that would take the count past 65,535 is lost and takes no count, so no give-back would
  carry a lost mark of the region's to the sealing: its lost mark goes in the
  renderer-wide record, as a pass's that began with no region open does, which the sealing
  cannot miss and `open` cannot reset. The generation is never zero, and the
  entries beside the rows start at zero, so no entry a use did not write matches it
  ("Writers"). Only a pass whose `begin` took a count gives one back, by its `end` or by
  its sink's drop, so a give-back never borrows from the index or the generation.
- **The declared word.** Outside `Staging` the renderer holds one more `AtomicU64`: the
  prepared frame's declared extent, `width << 32 | height`, zero while the frame is not
  declared, which `declare`'s refusal of a zero side leaves free to mean that. So the word
  is both the extent `latch` reads and the mark that the frame is declared. `declare`
  checks its arguments, then stores the extent with one compare-exchange from zero, and
  only then adds its passes: of two `declare` calls on one `&Renderer` exactly one
  succeeds, and one refused leaves the word as it was. `Relaxed` is enough: `latch`,
  `prepare`, `submitted` and `torn` reach the word through `&mut self`, which orders them
  after every `declare`, and `prepare` clears it.
- **The orderings.** `open`'s store of the open word is `Release`, so a `begin` that reads
  the new region also reads its cursor and counts reset, and every read the previous
  `prepare` made of that region, of its notes and its rows' shadow entries, comes before
  the new use's writes to it. A `begin`'s compare-exchange of the word, which adds its
  pass to the count, is `Acquire` on success and `Relaxed` on failure. A pass's `end`
  gives back its count with a `Release` subtraction, after its notes and its rows' shadow
  entries are stored, and a sink dropped with a pass open gives back its count with the
  same `Release` subtraction, after the pass's lost mark. Every change of the word between
  `open`'s store and the sealing is a read-modify-write, a `begin`'s compare-exchange or a
  give-back's subtraction, and never a plain store: the sealing reads only the word's last
  value, and it reads every give-back's release through that chain. The sealing's
  compare-exchange, which succeeds only at a count of zero, is `Acquire`, so `prepare` then
  reads every note, shadow entry and lost mark of every pass that ended or was dropped. A
  reservation is a `Relaxed` addition to the region's cursor: what makes a pass's rows
  visible to `prepare` is its `end`'s release, not the reservation. A row or a pass
  recorded while no region is open goes to the renderer-wide record, which `prepare` takes
  with swaps after the sealing's acquire, the lost kinds last. A pass's lost mark is a
  `Relaxed` bit set made before its rows' additions, a kind's `Release` and the rows with
  no kind `Relaxed`; the swap of a kind's rows is `Acquire`, so the swap of the lost kinds
  after it sees every loss marked before the rows it took ("Writers").
  The remote batch's writes are ordered by the mutable borrow its writer holds, which ends
  before `prepare` is called.
- **The only locks a call of this crate takes** are the device backend's, inside
  `Device::write_buffer`, `flush_buffer` and `timeline_value`: the lock on its records and
  the lock on a handle's entry, which rhi-api names ("Threads") and section 3 permits on
  the per-frame path, systems' jobs included (D158). Neither is held while the bytes of a
  write are copied: section 3 holds them to no code the caller supplies.
- **The ownership rule.** A region's rows and notes are written while it is open and read,
  by `prepare`, after it is sealed. A pass's `begin` and `prepare`'s sealing change one
  atomic word, so they cannot both succeed: a `prepare` made while a pass is open is
  refused, and a pass that begins after the sealing finds no region open and is lost. The
  remote batch cannot be open during `prepare`: its writer borrows the renderer. The
  generation in the word also means that a `begin` whose load read one use of a region
  cannot succeed on a later use of it, sealed and opened again between its load and its
  compare-exchange.
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
- That holds across kinds whichever pass began first: `prepare` applies the passes kind by
  kind ("Slots"), and a renderer shows at most one world of each kind.
- Three things do depend on how batches fell, and each is counted: which rows are dropped
  when a region has no room, which kind is lost when a region has no room (the pass whose
  note did not fit, `FrameStats::lost`), and which of two duplicate rows is kept.

### Errors and drops

- A refused call (`InvalidData`) changes nothing. A device's error from `prepare` or `latch`
  leaves the frame prepared and not submittable: the caller ends it with `torn`. So does an
  error from the caller's compile, record or submit of a frame the renderer declared into.
- **`torn`.** Graph's rule after a failed submit is to rely on no buffer's bytes. Of the
  renderer's buffers a submit writes the instance buffer alone; every other is written by
  the host and read by a submit, so a torn frame cannot have changed it. `torn` therefore
  gives up the instance buffer and nothing else: it frees every slot, forgets every base
  and the tick of every kind's last pass, and empties the batch, and the next frame declared
  imports the instance buffer from `Undefined` ("The scatter"). The renderer keeps no
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
- Two lifetime paths are reached by no test, and neither matters at milestone 2. A torn
  frame's region is held until the next `submitted`'s point; were it freed at once, no
  submit would read it, since a renderer's frame is one submit and on one queue a torn
  frame made none: it matters only with a provider's pass on another queue, which
  milestone 2 has none of. A drop without the wait for the last point would destroy under
  a pending submit, which the device refuses: the drop logs a leak, never a use after free.
- A sink dropped with a pass open leaves that pass lost and gives back its count
  ("Writers").

## The shader interface

Three stages, cooked by RICOTTA for each target and handed over as `Shaders`: the entries
`cs_main` of `shaders/renderer/scatter.slang` and `vs_main` and `fs_main` of
`shaders/renderer/triangle.slang`, which import the records from
`shaders/modules/lactose/renderer.slang`, cooked as `renderer/scatter.cs_main` and so on.
Each is a transcription of the host's statement of its rule: `InstanceRecord::scatter` and
`free`, and `drawn`, `CellTransform::apply` and `CameraData::clip`. Every buffer
is reached through a device address in the push block; no stage uses the heap. A shader
reads `SV_VulkanVertexID` and `SV_VulkanInstanceID` (rhi-api, "Pipelines and the cooked
shader format").

**Records.** Each record is laid out as "Formats" gives it, and declared so that the layout
is valid under Vulkan's standard rules for a physical storage buffer, with no
`scalarBlockLayout`, which rhi-api's required set does not have. So a placement is written
as arrays of 4-byte scalars and never as vectors, and no record holds a vector or matrix
member at an offset its standard alignment does not allow:

| Record | As a shader declares it | Stride |
|---|---|---|
| A placement | `int cell[3]` (0), `float offset[3]` (12), `float rotation[4]` (24) | 40 |
| `StagingRow` | a placement (0), `uint flags` (40), `uint reserved` (44) | 48 |
| `ScatterEntry` | `uint slot` (0), `uint row` (4), `uint flags` (8) | 12 |
| `InstanceRecord` | placements `previous` (0), `current` (40) and `base` (80), `uint flags` (120), `uint reserved` (124) | 128 |
| `CameraData` | `float4x4 clip_from_cell` (0), column-major, `int3 cell` (64), `uint reserved` (76) | 80 |
| A vertex of `TRIANGLE` | `float position[3]` (0) | 12 |

`reviewer-gpu`'s round-2 probe cooked this table as written: valid under `spirv-val` with
the standard rules and no `scalarBlockLayout`, the camera at offsets 0, 64 and 76 with a
stride of 80, MSL's `packed_int3` at 64, and the images of a camera declared as scalar
arrays, bit for bit, under MoltenVK.

The natural `int3 cell; float3 offset; float4 rotation;` puts a `float3` at 12 and a
`float4` at 24, which Slang emits and which the standard rules call an improper straddle:
the module is invalid on a device without `scalarBlockLayout`, and the cook does not catch
it.

**Metal's buffer index.** Each stage's push block is an entry-point `uniform` parameter of
the stage, never a global shader parameter of its module. Slang then gives it Metal buffer
index 0, `[[buffer(0)]]`, in the scatter and in the vertex stage, whatever else the module
declares; the fragment stage declares none. A global push block takes its place among the
module's globals, so a module that declares the scatter's block and the triangle's gives
one of them `[[buffer(1)]]`, which the cook (`slang`, at 6b6ab77, not yet merged) refuses:
it refuses a push block at any index but 0. `reviewer-gpu`'s round-2 probe cooked both
forms; the entry-point form passed `spirv-val` and `metal -Werror` and rendered the global
form's images under MoltenVK. `rhi-metal` sets the push bytes at the index the cooked
shader declares, and binds no vertex buffer, so the vertex stage's index 0 is the block's.

**Scatter, compute.** Workgroup `(64, 1, 1)` (`SCATTER_GROUP`), which `Renderer::new` holds
the cooked `workgroup` to ("Pipelines and shaders"; the format's version 2). Push block, 32
bytes (`SCATTER_PUSH_BYTES`):

| Offset | Field |
|---|---|
| 0 | address of the region's first `StagingRow`, read; zero where the batch has only `FREE` entries |
| 8 | address of the dispatch's first `ScatterEntry`, read |
| 16 | address of the instance buffer's first `InstanceRecord`, read and written |
| 24 | `count`, `u32`: the dispatch's entries |
| 28 | zero |

Invocation `i` returns when `i >= count`. Otherwise, with `e` the entry `i` and `r` the
record `e.slot`: if `e.flags` has `FREE`, `r.flags = 0`; else `r` becomes what
`InstanceRecord::scatter(e.flags, rows[e.row])` leaves. No specialisation constant.

**Triangle, vertex.** Push block, 32 bytes (`TRIANGLE_PUSH_BYTES`), the pipeline's one
block; the fragment stage reads nothing of it, and its cooked block is 0 or 32 bytes:

| Offset | Field |
|---|---|
| 0 | address of the instance buffer's first `InstanceRecord`, read |
| 8 | address of the frame's `CameraData`, read |
| 16 | address of the triangle's first vertex, read |
| 24 | `fraction`, `f32` |
| 28 | zero |

One specialisation constant, id 0 (`CELL_SIZE_CONSTANT`): the cell size, `f32`. Outputs: the
clip-space position, `(2, 2, 2, 1)` for a record without `LIVE`, and the slot as a `u32`
that is not interpolated. The cells are subtracted as `uint` and reinterpreted as `int`.
The arithmetic is "The triangle pass".

**Triangle, fragment.** Input: the slot. Output: colour attachment zero, one location, the
palette colour. No push block read, no input attachment, no constant.

`slang`'s engine shader of decision 5.14 proves an interface specialised statically and a
link-time constant. Where it is one of these three, the workgroup size is the natural
link-time constant and the cell size stays a specialisation constant, which the host sets.

## How each call is honoured

Every call reaches a device through rhi-api and graph alone, so it is honoured on null and on
native Metal as the calls it makes are. The backend columns are those of rhi-api's own table.

| Call | What it calls | Null | Native Metal | Vulkan (MoltenVK on the M1) | Evidence |
|---|---|---|---|---|---|
| `Renderer::new`: the `Upload` buffers | `create_buffer`, `Sharing::Exclusive`, `MemoryClass::Upload`: each region's rows, entries, arguments and camera, and the triangle's vertices and indices, two buffers written once | Bytes and a record for each | Shared storage, which is coherent; `gpuAddress` | Host-visible memory with a device address; type 1 on the M1, write-combined on a discrete GPU | RHI item 2, point 3; Metal item 2, point 3 |
| `Renderer::new`: the instance buffer | `create_buffer`, `Sharing::Exclusive`, `MemoryClass::Device` with `DEVICE_ADDRESS` and `COPY_SOURCE` | Bytes and a record | Private storage; `gpuAddress`; `useResource` on both encoders | Device-local, not host-visible; type 0 on the M1 | `reviewer-gpu`'s round-1 probe under MoltenVK, not checked in; native Metal and a discrete GPU owed ("What neither spike shows") |
| `Renderer::new`: pipelines | `create_compute_pipeline`, `create_graphics_pipeline` with one `Constant` | The reflection, held to the pass at a draw | A library function specialised by a function constant; the archive | A module and a specialisation; the driver's cache | RHI items 4 and 5; Metal items 6 and 7 |
| A writer's `write` | `write_buffer` on the open region, disjoint ranges at once | Fills the range; refuses an overlap (`host-overlap`) | A write into the buffer's contents | A write into the mapping | rhi-api, "Buffers and host access" |
| `open` | `timeline_value` | The counter | `MTLSharedEvent`'s value | The semaphore's counter | RHI item 8; Metal item 9 |
| `prepare` | `write_buffer` for the entries and the arguments; `flush_buffer` over what the frame reads | Marks the ranges flushed; a submit that names a buffer with an unflushed range is refused on a set that is not coherent (`unflushed`) | Nothing: shared memory is coherent | `vkFlushMappedMemoryRanges` where the type is not coherent | RHI item 2; Metal point 3 |
| `declare`: imports | `FrameDecl::import_buffer`, each `Sharing::Exclusive` on the graphics queue: the region's buffers from `Undefined`, which keeps a buffer's bytes; the instance buffer from `ShaderRead`, and from `Undefined` in the renderer's first frame and the first declared after `torn` ("The scatter"); `transient_image` for the depth | States tracked from the declaration | No layouts; `useResource` from the pass's reach | Barriers the graph derives | graph, "Imports" |
| `declare`: the scatter | `compute_pass`; `ComputeRecorder::{bind_pipeline, push_address, push_bytes, dispatch}` | Typed pushes against the declaration; runs nothing | `dispatchThreadgroups` on a serial encoder, the buffers declared with their usage | `vkCmdDispatch`, a shader-write barrier between dispatches | RHI item 2; Metal item 2 |
| `declare`: the triangles | `render_pass`; `RenderRecorder::{bind_pipeline, bind_index_buffer, push_address, push_bytes, draw_indexed_indirect}`, the draw only where `SlotTable::extent` is above zero | Checks states and declarations; draws nothing | `drawIndexedPrimitives` with an indirect buffer, one record | `vkCmdDrawIndexedIndirect`, count one | RHI items 9 and 10; Metal items 5 and 12 |
| `latch` | `write_buffer`, `flush_buffer` on the frame's camera buffer | Judged at the submit, after the write | A write into shared memory | A write into the mapping | graph, "A frame, by step"; rhi-api, "Buffers and host access" |
| The renderer's drop | `wait_timeline`, the destroys | Refuses a destroy while pending | Objects released | Objects destroyed | rhi-api, "Handles and lifetimes" |

Null runs no shader. On null `renderer-impl` proves the batch from `Renderer::batch`, with
`InstanceRecord::scatter` standing for the shader; the writers' ranges from null's
`host-overlap`; the flushes from `unflushed`, on a null whose host memory is not coherent
(`graph::frames_on_a_null_whose_host_memory_is_not_coherent_flush_their_writes`); and the
region rule from `open`'s answers alone
(`frames::frames_held_in_flight_leave_no_region_to_open_until_the_oldest_point`). Null's
`pending` sees a host write under a pending writer, and no submit writes a region, so it
shows nothing of the rule; a host write under a pending *reader* null cannot see
(rhi-api), so the frames-in-flight proof is a device test with the GPU made to lag.

**What neither spike shows.** The instance buffer is `MemoryClass::Device`: the host never
writes or reads it, the scatter writes it through its address and a test reads it through a
copy. RHI lists a buffer in device-only memory under "Not shown": every buffer of both
spikes sat in host-visible memory. rhi-api's "Buffers and host access" says that a pass
that declares the buffer `ShaderWrite`, as a graph pass that declares it
`BufferAccess::Write` does, fills it as a copy does (ask (b) of D161, met at `rhi-null`'s
merge; `device_buffer_written_by_a_pass_is_read_after_it` holds that null accepts it). The
path is shown under MoltenVK on the M1 by
`reviewer-gpu`'s round-1 probe, which is not checked in: a `DEVICE_ADDRESS` buffer in
memory type 0, device-local and not host-visible, written by the scatter through its
address, read by the vertex stage through its address in the same command buffer and
copied out exact, over two frames, silent under the validation layer and Metal's API and
shader validation. `renderer-impl`'s device test that reads the instance buffer back is the
first checked-in evidence, and the first on native Metal (a private buffer with
`gpuAddress` and `useResource`) and on a discrete GPU. No signature of this crate names
the instance buffer's memory class, so a finding there changes `Renderer::new`'s body and
rhi-api's sentence, and nothing a consumer wrote.

Three rules of the layers below that this crate leans on:

- **A buffer imported from `Undefined` keeps its bytes** (graph, "Imports"; D160). The
  renderer imports every buffer of its own that way, each frame, on the graphics queue, but
  the instance buffer, which it imports from `ShaderRead` but in its first frame and the
  first after `torn` ("The scatter").
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
  worker. `renderer-impl` reports its cost as steering ("Proof"), a load that replaces the
  key set included; a forked form is an added call.
- **Views.** One camera, one target. The caller's `FrameDecl` has one view.
- **Static cooked instances from IO** (section 5.3), which reach the GPU scene without
  extract, at milestone 3.
- **A provider's own heap slots, a G-buffer in `FeatureView`, a fallible
  `RenderFeature::declare`.** With the first provider.

## Proof

The null proofs are modules of the `integration` binary: `tests/slots.rs`, the slot table
through a world's sink and through passes by hand; `tests/frames.rs`, frames through a
render graph on null; `tests/graph.rs`, the frame's plans, queues, flushes and shader
checks through a graph on null; `tests/sequences.rs`, present-api's sequences through
`ExtractSink` with the batch scattered on the host by `InstanceRecord::scatter`;
`tests/model.rs`, random sequences of passes, batches, losses, refusals and torn frames
against a model of this file's rules; `tests/writers.rs`, writes from several jobs, run
again until two overlapped; `tests/camera.rs`, the orbit against `lactose-input`; and
`tests/cost.rs`, the steering figures, ignored. `src/models.rs`'s loom models hold the
ring's orderings: they search the shipped functions' interleavings over loom's atomics, at
a preemption bound where the search is exhaustive, with each shadow entry checked as one
location, so that a read of it by one use's `prepare` unordered with the next use's write
fails. `src/staging.rs`'s unit tests include three writers' passes racing a sealer on the
standard library's atomics, which fails a reservation or a `begin` that is not one
read-modify-write on real threads; the nightly runs them under Miri with four seeds, whose
weak-memory emulation checks the orderings on the executions those seeds reach and
searches no others. `tests/device.rs` holds the rows that
need a shader to run, as ignored tests that `cargo xtask gpu` runs under its drivers and
validation, with `shaders/renderer/` cooked: the instance buffer read back through a copy
into a `Readback` buffer, frames held behind an import's `ready` point that the host signals
once a later frame's writes are made and behind a dispatch of about a second at the frame's
head, and each image against a reference computed on the CPU, with its three counts
printed. `tests/consumers.rs` compiles
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
| (the same) A load that replaces a kind's whole key set at `instances` is placed whole | A full pass of `instances` new keys over `instances` held: `unplaced` zero | One that places before it sweeps |
| (the same) The table does not depend on which kind's pass began first | Two frames, one with the `Prediction` pass begun before the `Server` pass: equal tables | One that applies in begin order across kinds |
| (the same) A region that holds two frames applies its last remote batch alone | A region left by a frame that did not prepare, and one left by a refused `prepare`: frame 1's batch holds `gone` and `kept`, frame 2's `Client` pass removes `gone` and its batch holds `kept`. `gone` holds no slot | One that applies every batch after every `Client` pass: `gone` comes back, and no count shows it |
| (the same) A `Client` pass lost after a batch stops it | The same region, with frame 2's `Client` pass lost for want of room: `lost(Client)`, and no slot of the kind held | One that applies the region's batches whatever kind is lost: frame 1's batch brings `gone` back |
| (the same) A batch that overlaps a `Client` pass is left out | One frame that breaks its ordering, from an empty table: a `Client` pass begins, the batch writes `gone` and `kept`, and the pass removes `gone` and ends; and the batch first, then the pass. In both orders neither holds a slot, `remote_rows` is 2 and nothing is lost | One that compares the batch's opening with the `Client` begins alone: the pass-first order applies the batch over the removal, and `gone` is drawn with no count |
| A batch names each slot once, resolved in batch order and asserted | On null: duplicate rows through `RemoteWriter::write`, then `ScatterBatch` and `FrameStats::duplicates`. Null's assertion is what proves the resolution | One without the resolution: the assertion fires |
| Many duplicates in different workgroups of a large batch | On a device: slots made `LIVE` at a known placement `c0` by an earlier frame, then duplicates from a world pass at a new tick, entries `TAKE_BASE \| HAS_BASE`, each instance's two rows at least 2^15 rows apart in the region (row `i` and row `n + i`); the instance buffer copied out, each record's `base` and `previous` equal to `c0` and `current` to the kept row. It shows that a resolved batch scatters right; an entry with neither flag reads nothing of its record, so remote rows on fresh slots cannot show it | Without the resolution and with the assertion off, the second entry of a slot lacks `TAKE_BASE`, and run in order its records match: the host's check of the flags fails that build. The records fail one that marks both entries `TAKE_BASE \| HAS_BASE`, at that spacing. Null's assertion is the proof of the resolution |
| The scatter's dispatches each apply their own entries, all of them and no more | On a device: a batch of two runs, of 37 and 5 entries, each `TAKE_BASE \| HAS_BASE`, in a region whose earlier use had a longer batch; every record read back equal to `InstanceRecord::scatter`'s on the host | One that reads each dispatch's entries from the batch's first; one that pushes one entry too many; one that rounds the groups down; one that pushes no rows address |
| With the GPU made to lag, the ring never overwrites a region a frame in flight reads | Frames held behind an import's `ready` point, and behind a dispatch of about a second at the frame's head; `Opened::Wait`; the readback of a held frame after later writes | One whose `open` ignores the point |
| Frame N's readback shows N's camera after N + 1's has been written | The same, with two latches | One camera buffer for all regions |
| An instance in the camera's cell and one in a neighbouring cell along an axis the scene is moved 2^25 + 1 cells on, its triangle covering pixels at the origin, render the pixels they render at the origin | Two scenes 2^25 + 1 cells apart on that axis, with instances in the camera's cell and the next along it: equal images | A shader that subtracts after converting to float: at 2^25 + 1 cells the neighbour's difference converts to zero cells. In one cell alone, below 2^24 cells, or with the neighbour along an axis the scene was not moved on, where every cell converts exactly, it renders the same pixels and fails nothing |
| A record without `LIVE` produces no fragment | A slot freed in frame 2, below the highest slot held: its triangle's pixels are `CLEAR`, and the highest slot's are drawn | One that ignores `LIVE`, which draws the freed record at the placement it last held; one that draws `SlotTable::live` instances, not `extent`. The record is placed at `(2, 2, 2, 1)` and not `(0, 0, 0, 0)`, whose result no specification defines, and which the M1 draws nothing for |
| A world stepped, extracted, rendered and compared with the CPU reference, at a fraction strictly between 0 and 1 | `InstanceRecord::drawn`, `CellTransform::apply`, `CameraData::clip`, `shader::{TRIANGLE, PALETTE, CLEAR}`. Exact at each pixel whose centre is at least a quarter of a pixel inside or outside every edge the reference computes in `f64`, decision 5.1's margin, justified from four bits of sub-pixel snapping and 32-bit error at the scene's extent; the others accepted either way, and the three counts printed: surely drawn, surely clear, in the margin, with zero wrong. Of each image's 16,384 pixels, under MoltenVK and KosmicKrisp alike (`--nocapture` runs of renderer-impl fix round 2): the camera's frame 1, Gate and Lag, 404, 15,932 and 48, and its frame 2 0, 16,384 and 0; the held frame whose instance the next frame moves, frames 1 and 2, 392, 15,934 and 58; the far scene at the origin, 798, 15,481 and 105; the freed slot's frames 1 and 2, 784, 15,484 and 116, then 392, 15,934 and 58; the world, 2,428, 13,681 and 275 | One that draws nothing; one that draws every instance in the clear colour; one that pushes a fraction of 1 |
| The first frames on null | A renderer's first frame with nothing extracted, and a frame that frees every slot in a region no writer has used: each submitted, none refused (`undefined-read`) | One that declares the instance buffer or the rows in every frame |
| The sink and the frame agree (present-api's proof row "A rollback in a frame with no step") | present-api's sequences run through `ExtractSink`, the records read back on a device or computed with `InstanceRecord::scatter` on null | One that sets `HAS_BASE` in a dropped tick; one that takes the base at every write |

**Where the device tests run.** In CI's `gpu (lavapipe)` job, where they cannot skip, and on
the M1. A run with no device says that it skipped. Every dependency between the scatter and
the draw passes through device addresses, which synchronisation validation does not track,
and lavapipe runs each dispatch to completion before the next command, so neither shows a
dropped barrier. The barrier between two dispatches of the scatter is the one that matters:
nothing else orders them. Dropped, with the scatter's `FREE` path made to lag, it fails the
test of `65,536` slots freed by one run and given by the next under KosmicKrisp, and the
lag with the barrier kept passes it; without the lag it passes, and under MoltenVK, whose
serial compute encoder orders the dispatches anyway, it passes with the lag. No shipped
shader lags, so `cargo xtask gpu` alone cannot fail the mutation: the mutation and its lag
are a script's, on the M1 and on the desktop. The other two, the
scatter-to-vertex barrier and the barrier across frames, cannot fail as a frame is
declared: dropping the instance buffer's transition leaves other imports' transitions from
`Undefined` before the same pass (the entries' before the scatter; the camera's, the
vertices', the indices' and the arguments' before the triangle pass), each a full execution
dependency: a mutation fails as the frame is declared only if it drops every transition the
graph records before the pass. Across frames, a write after a read, that is all the hazard
needs, so the mutation is correct by the specification. Before the triangle pass the
mutation drops the memory dependency on the instance buffer alone, which desktop drivers
carry out as a global cache operation, so a desktop run in which it survives does not show
the barrier unneeded.

It also owes:

- a loom model of `Staging`'s open word and the regions' cursors, over the shipped functions
  through a `sync` shim behind the crate's own `loom` feature, with `lactose-renderer` added
  to xtask's `LOOM_MODELS`: that a `begin` and a sealing cannot both succeed, that a pass
  that ends is read whole by the `prepare` that seals after it, that a pass of an earlier use
  of a region is never read as a later use's, that no `prepare` reads an entry of a later
  use, with each shadow entry an access loom checks, that a loss with no region open is
  reported, that no sealing's rows dropped by kind exceed its rows dropped, that a sink
  dropped with a pass open does not keep the region from being sealed, that the `prepare`
  that seals after it reports that pass lost, and that a remote batch written beside a pass
  is read whole, at a preemption bound where the search is exhaustive; and Miri, with four
  seeds, over `Staging`'s tests, one of which races three writers' passes against a sealer
  on threads;
- a test that `ExtractSink::write` from several jobs leaves every row once, run again until
  two writes overlapped;
- as steering, the ring's write cost, and `prepare`'s cost, median and maximum, at 50,000
  incremental rows among 100,000 held, at a full pass of 100,000 held keys, and at a load
  that replaces 100,000 held keys with 100,000 new ones, which `reviewer`'s round-1 model
  put at 18 to 20 ms on one worker.
