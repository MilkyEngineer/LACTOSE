# LACTOSE Architecture

LACTOSE: Lock-free Asynchronous Chunked Task-Oriented Simulation Engine.
Its deterministic simulation core is CURD: Concurrent Unified Runtime, Deterministic.
LACTOSE sets the CURD.

Status: design baseline, revision 3.4, after three adversarial reviews and an editorial pass;
revision 3.3 settles what the whole-project review of milestone 1 found against this document
and writes in the rulings of `docs/plans/milestone-1.md`: how milestone 1's criteria are read,
what moves to later milestones, and the workflow in force; revision 3.4 follows milestone 1's
exit and states where what a world registers may be declared (D152). Every section is a decision,
not a suggestion. Change a decision by editing this file in the same change that implements it, and record why
in `docs/decisions-log.md`.
This file states what is decided. `docs/decisions-log.md` records why, which review finding
drove each decision and what it replaced. `docs/verification.md` lists the spikes, the
third-party claims re-verified against the exact versions pinned in `Cargo.lock` before the
milestone that depends on them, and the open items.

## 1. Goals

In priority order. When two conflict, the earlier one wins.

1. **Short critical path.** Minimise the longest dependency chain in a frame. Utilisation is a
   consequence, never a target.
2. **Deterministic simulation as an engineering property.** The same build, given the same
   inputs, produces the same world state regardless of thread count, scheduling order, chunk
   layout, run, or host architecture (x86-64 and aarch64). This is what makes replays, the
   determinism harness and desync debugging work. It does not by itself make client prediction
   correction-free, because the client's prediction world differs from the server's world and
   its physics solver holds state the rows do not carry (section 16).
   Reconciliation is defined separately (section 9.4).
3. **No threading knowledge required of gameplay programmers.** They never write a lock, spawn
   a thread, or choose execution order. They declare what data a system reads and writes.
4. **Cache-coherent data layout.** Structure-of-arrays in fixed-size chunks on every hot path.
5. **No coordinator thread.** The scheduler is work-stealing. Ticking is a timer-driven root
   task, not a thread. The OS main thread pumps events only.
6. **Multiplayer without a fixed player cap.** Server cost is bounded by active entity count
   and per-client cost by relevance, never by a constant.
7. **Modular, interface-first where the interface is knowable.** An interface crate a backend
   implements is frozen only once a real backend has passed its conformance suite (section 20).

Non-goals for now: an editor, mobile, web, Intel Macs, a scripting language, multi-process
server sharding.

## 2. Decisions

| Area | Decision |
|---|---|
| Language | Rust, stable toolchain, workspace of crates. No Bevy. |
| Reference game | Open-world action-adventure in the style of Breath of the Wild, networked co-op with an arbitrary number of players. |
| Simulation tick | 60 Hz fixed. Render uncapped with interpolation. |
| Netcode | Dedicated server, authoritative. Clients send tick-stamped inputs with redundant history and simulate the quantised value they sent. Local-player prediction in a separate client prediction world, reconciled at full precision within a stated tolerance on the solver-written rows of a dynamic body. Snapshot interpolation for everything else. Interest management and per-client bandwidth prioritisation are part of the first networking deliverable, never a later addition. |
| Determinism scope | Same build, identical results across threads, layouts and x86-64 versus aarch64. Reconciliation compares at full precision, with a stated tolerance on the solver-written rows of a dynamic body (section 9.4). |
| Identity | Process-local entity IDs never leave the process. Every reference, ordering rule and RNG key uses a `StableKey`: the server-assigned network ID, or the prediction key for a spawn the server has not confirmed. The server records a predicted spawn's prediction key as its permanent origin key, so binding changes nothing semantic. Network IDs double as persistent IDs. |
| World coordinates | Simulation scalar is a `math` newtype with no inherent float methods, 64-bit: milestone 1's precision test at x = 6 km decided it, with the 64-bit rollback loop measured beside it (section 16). Switching would be a Rapier crate switch, not a one-line change. GPU scene stores grid cell plus 32-bit offset. |
| Physics | Rapier through `physics-api`, pinned exactly. One physics world per server process, one prediction world per client. Rapier's determinism and parallel features together. |
| Graphics | Thin RHI. Vulkan 1.3 backend first (Windows, Linux CI). Native Metal backend in milestone 3 for Apple Silicon. MoltenVK and KosmicKrisp spiked in milestone 2 as interim Apple paths. DX12 when Xbox matters. A null backend for tests. |
| Shaders | Slang. The cooker emits SPIR-V for Vulkan and MSL plus metallib for Metal. Mac shaders cook on a macOS runner. Driver pipeline caches are persisted. No runtime source compilation. |
| Lighting | Deferred with a G-buffer held in tile memory on tile-based GPUs. Visibility buffer when meshlets arrive. |
| Draw submission | GPU culling compacts surviving clusters into one 32-bit index buffer per pipeline over a global vertex pool, drawn with one indexed indirect draw per pipeline. Vertex reuse is preserved and no backend needs multi-draw or indirect-count. |
| Latency | Simulation N+1 overlaps render N after an exclusive extract phase. A triple-buffered presentation snapshot feeds a variable-rate presentation schedule. The full orbit camera transform is late-latched. |
| Concurrency | Systems complete by continuation counters. Fork-join only inside a system, same-scope help only. `'static` stackless futures for IO, GPU and asset work on a custom executor. No stackful fibers. No tokio. Frame-critical workers on performance cores, background class on efficiency cores. |
| Component data | `bytemuck::NoUninit + CheckedBitPattern`: padding-free, no heap ownership, validated only on load and receive. Variable-length data is owned child entities or fixed-capacity inline arrays. |
| Change tracking | Per-chunk coarse tick set at dispatch, a per-row change tick on tracked columns, and a structural change log for spawns, despawns, component changes and region moves. Extract keeps a last-seen tick; replication keeps a per-client, per-entity acked tick. |
| Error policy | Panics only for broken invariants and they abort. Everything else is `Result` up to an `app`-level policy. Tick-stamped error events inside the boundary. |
| Gameplay authoring | Rust only, split into simulation and presentation crates. Iteration via state-preserving restart with a prescribed dev profile. |
| Editor | Deferred. Reflection in scope from day one. |
| Saves | Overlay of typed records against the cooked baseline, sharing the serialisation core with netcode but never the replication stream. Server-owned. Explicit field IDs and migrations. |
| Server capacity | Activation radius around every player, chunks owned per archetype so scattered players do not fragment storage, and an explicit overload ladder that never slows server time. |
| Binaries | An `app` library plus thin `app-client`, `app-server` and `app-headless` binaries. The server links no rendering, presentation, platform, audio or UI crate. |
| Platforms | Windows and Apple Silicon macOS clients. Linux dedicated server. CI on linux-x64, windows-x64 and macos-arm64. Consoles when Rust toolchains exist. |

## 3. Principles that bind every crate

**Declare access, never order.** A system declares the components and resources it reads and
writes. The scheduler derives concurrency from declarations and ordering from a stable
topological sort over declared system sets. Two conflicting systems with no ordering at any
level are a schedule build error.

**No locks on the hot path.** Per-frame work uses chunks, command buffers and atomic counters.
A `Mutex` inside a system is a review rejection. Locks are permitted in setup, teardown, IO
completion handling, and inside RHI backends on objects the graphics API requires to be
externally synchronised and on a device backend's queues and heap, each held through the driver
call it orders (D158). Two more are permitted on the per-frame path, systems' jobs included,
each named in its crate's README with every call that takes it: the snapshot channel of
`lactose-present-api`, taken at most once by extract's pass and once by each publication and
acquire, and held for a constant-time exchange of buffer handles and event batches (D157); and
the locks on the records an RHI device backend keeps to check the rules every backend checks,
the record's lock held by a call once for its checks and the updates they permit, which for a
submit is a walk over the handles it names, and for a present also over the points its submit
depends on, made again after a wait, and once more where that hold set a mark, after a submit's
driver call, or where a timeline's wait or read records the value it saw, and a lock on one
entry, a handle's or a heap slot's, held by a lookup for a compare and a count (D158). None of
them is held across a wait, a driver call or code the caller supplies.
Outside systems, the per-frame path is also permitted the two locks of a `lactose-task` lane,
which `crates/task/README.md` names with every call that takes them: a frame's call to the
present thread takes the lane's queue, held to push a call and to take what is queued, and the
call's reply, held to store or take its result or the awaiting task's waker; neither is held
while the call runs or while a waker is woken, and the frame keeps at most one such call in
flight and queues at most one a frame (D159). A call to the main thread crosses the same two and
is never per-frame work.

**Unsafe is quarantined.** `#![forbid(unsafe_code)]` outside the crates in section 21.

**One dependency rule, in one file.** `crate-tiers.toml` is the only statement of which crate
may depend on which. This document describes it; the file decides. CI enforces it.

**The determinism boundary is a crate boundary.** A crate is inside or outside. Crates that
would straddle it are split: `net` (outside) and `predict` (inside), `game-sim` and
`game-present`, `nav` and `nav-query`. Section 8.1 lists the inside crates. What a world
registers, stores and runs is held to section 8.2 wherever it is declared: a crate outside that
must declare some of it carries the boundary lint file, and a system that writes hashed state
is declared in a library, never in a binary (sections 8.1 and 8.3, D152).

**Things you run get names, things you import get descriptions.** Codenames from the glossary
(section 23) appear on binaries, tools, log targets, CI jobs, README titles and headings,
never on crate names, types, functions, modules, constants or statics. Test code is held to
the same rule: a test or bench target is a thing you run and may carry the codename of what
it runs (`tests/sour.rs`, `benches/whisk.rs`), and nothing declared inside one does.

**Everything is data.** Components are padding-free plain data registered with the reflection
registry. Entity and asset references in components are typed fields the registry knows about.

**Errors are values.** A panic means a broken invariant and aborts the process after a crash
dump. Recoverable conditions (missing asset, network drop, disk full, device lost) return
`Result` and are handled by policy in `app`. Inside the boundary, failures that affect
simulation become tick-stamped events so both server and client see the same input. Section 19
gives the details.

**Traced from day one.** The executor, every system, every render pass and every IO request
emits `tracing` spans. Tracy is the profiler target.

## 4. Workspace layout

Every engine crate is published as `lactose-<name>`; the game's crates are `game-<name>`, as the
`[game]` list of `crate-tiers.toml` names them (`game-sim`, D152). Tier is F (foundation, usable by anyone), B
(backend, usable only by assembly crates, leaf crates and the exceptions in
`crate-tiers.toml`) or A (assembly, usable only by leaf crates).
Codenames in brackets appear only where section 3 allows. Boundary marks crates inside the
determinism boundary. `crate-tiers.toml` also lists leaf crates (binaries and tools, which may
depend on anything), game crates (which may depend on foundation crates and on each other) and
test-support crates (which any crate may name as a dev-dependency); a crate missing from the
file fails the tier check.

```
crates/
  core/           F  boundary  EntityId, StableKey (NetId, PredictionKey, Placeholder), Handle<T>, FixedHashMap, arenas, tagged GlobalAlloc, Error taxonomy, thread_class helper
  math/           F  boundary  self-contained deterministic math, no glam; Scalar newtype; compare-and-select min/max/clamp; libm transcendentals; exact accumulators; typed render space
  task/           F            [WHISK] executor: continuation counters, fork-join scope, worker classes, timer root, main-thread lane
  ecs/            F  boundary  [CASEIN] entities, archetypes, chunks, per-row change ticks, structural change log, queries, scheduler, command buffers, regions and region modes as storage, world snapshot, bulk row copy, system macro
  ecs-macros/     F            proc macros: `#[system]`, `#[finalize]`
  reflect/        F            type registry, field IDs, typed references, persistence policy (a serde bridge is deferred: saves use `bundle`'s canonical encodings and `persist`'s container, D129)
  reflect-macros/ F            proc macros: `#[component]`, field IDs, reflection metadata, per-type visitors
  sim/            F  boundary  [CURD] region lifecycle (activation, streaming, unload policy), input sync point orchestration, the tick step and the frame's passes (the loop is the timer root's, in `app`), time scale, world hash (monomorphised per-type hashers)
  anim/           F  boundary  animation state machines, phase, events, root motion, sim skeleton (sockets, hands, feet, head, ragdoll bodies)
  predict/        F  boundary  prediction world, rollback ring, reconciliation
  nav-query/      F  boundary  navmesh queries
  physics-api/    F  boundary  the simulation `Transform`, `Velocity` and `Teleported` components, bodies, colliders, joints, queries, events, snapshot and restore
  asset-api/      F            AssetId, AssetRef, asset handle policy, loader and processor traits, version table types, published-assets and catalogue store traits, streaming priority and memory budget types
  bundle/         F  boundary  region bundle format shared by cook, asset and persist: baseline rows, placement GUIDs, overlay records
  present-api/    F            presentation components (mesh, light, decal, camera rig), VFX and audio request events, UI view-model, debug-draw buffer, presentation snapshot
  net-api/        F  boundary  the staged frame a client applies at the input sync point and its inbox, the replication gather and its frame, the relevance message, the remote-slot component
  net-host/       F            the host side of networking: quantised wire values, the handshake, the transport trait, packets and per-connection handles, per-client interest, the lock-free interpolation buffer with batch sampling, clock sync, control messages
  rhi-api/        F            device, queue, buffer, image, pipeline, bindless heap, encoder, resource states, memoryless attachments, local reads
  input/          F  boundary  the quantised per-tick sample (`PlayerInput`: integers only, with absolute camera yaw and pitch), its byte image on the wire and in the journal, the applier that writes it at the input sync point, the redundancy window
  input-host/     F            the host side of input: raw samples and the quantiser, the latest-sample slot for the late latch, gamepad and keyboard sources and action mapping (milestone 5)
  io/             F            async file IO: blocking pool reference backend; io_uring and IOCP optimisations
  asset/          B            [SILO] the pack format the cooker writes, loading, dependency tracking, versioned publication, hot reload, the region baseline as an asset
  physics-rapier/ B  boundary  [QUARK] Rapier backend
  net/            B            [MILKMAN] transport implementations, replication, interest management, bandwidth prioritisation, clock sync; implements `net-host`'s traits and fills `net-api`'s frames
  persist/        B            [SKIM] world overlay, save container [CARTON], migrations
  rhi-vulkan/     B            ash-based backend
  rhi-metal/      B            native Metal backend, milestone 3
  rhi-null/       B            no-op backend for tests only
  rhi-dx12/       B            later
  render-graph/   B            [GOUDA] pass declaration, compile, barriers, aliasing, parallel record
  renderer/       B            GPU scene [CHEDDAR], bindless heap [BRIE], culling [CREAM], late-latch [LATTE], skinning, GPU VFX simulation [FROTH], deferred lighting, render slot table, RenderFeature pass-provider trait
  presentation/   B            per-frame presentation schedule: camera rig, pose evaluation, interpolation sampling, remote transform upload, UI animation, audio listener, markers
  terrain/        B            clipmap rendering; collision export is a cook output consumed through asset-api
  audio/          B            mixer thread, event bridge
  ui/             B            game UI and text rendering
  nav/            B            navmesh generation
  platform/       B            [PAIL] winit window and its handles, event pump, present thread
  debug-ui/       B            egui overlay, profiler view, determinism inspector
  app/            A            library that assembles a runtime from foundation, backend and game crates
  app-client/     leaf         binary: app plus every client backend; registers RenderFeature providers
  app-server/     leaf         binary: app plus server backends only; CI denies rendering, presentation, platform, audio and UI crates
  app-headless/   leaf         binary: simulation only, for SOUR and tools
game/
  game-sim/          boundary  gameplay simulation systems; depends on foundation crates (boundary crates plus asset-api, present-api, bundle)
  game-present/                gameplay presentation; depends on present-api and foundation crates
  game-ai/           boundary  perception, behaviour, decision systems; depends on nav-query and foundation boundary crates
  game-content/      boundary  data definitions shared by all three; inside the boundary because game-sim reads it
tools/
  cook/              leaf      [RICOTTA] asset cooker: meshes, clusters, textures, materials, shaders, scenes, collision
  xtask/             leaf      CI driver: tier check, boundary lint and feature-tree checks, SOUR mode runner, measurement runner (`cargo xtask measure`)
```

There is no `task-api`: spawning is generic and cannot sit behind a trait object. `ecs` has
no interface split for the same reason. Backend services that `game` must reach (physics
queries, asset lookup, presentation requests) are exposed as resources whose types live in the
corresponding `-api` crate and whose instances `app` inserts at startup: a `dyn` trait, or a
concrete type the `-api` crate wraps around one where the schedule must see the step and the
queries as one target (`physics_api::Physics`).

## 5. Runtime model

### 5.1 Task system

**Systems complete by continuation.** Each system in a stage has a remaining-jobs counter.
Chunk jobs decrement it. The job that reaches zero runs the system's finalize step (where
reductions merge partials) and then decrements the counters of dependent systems, pushing any
that reach zero. No worker ever blocks waiting for a system.

**Fork-join inside a job only.** `ctx.scope(|s| s.fork(...))` on the context each job
receives, with an invariant `'scope` lifetime, closure-only, the shape of `std::thread::scope`. Children are closures that may
borrow from the scope. The opener runs only jobs belonging to its own scope while waiting,
then spins briefly, then parks on the scope's latch. It never runs unrelated system jobs and
never runs background work while waiting. A job may open scopes in sequence but never nested. Child jobs are
allocated from a per-worker bump arena reset when the scope closes. Every deque entry carries
the tag of the scope that forked it, and while a worker has a scope open, work pushed to it that
is not a child of that scope is routed to the frame injector, so the opener's deque holds only
its own children and it never runs older or unrelated work.
The physics step is the one sanctioned blocking wait on a worker (section 16):
`block_in_place` lowers the number of run permits by its reservation, a worker holds a permit
while it takes jobs, an opener parked on its latch holds none, and a permit that comes free
while the count is lowered reaches both idle workers and openers waiting beside their own
children.

**Resolution means quiescent.** `GraphRun`, `Task` and `Sleep` resolve to a `Result`, and the
error `Cancelled` means the executor shut down first. A graph run resolves, with the graph or
with `Cancelled`, only when no unit, scope child or finalize of that run is executing or can
start, and their writes happen before the resolution; this holds when the last handle is
dropped on a worker and nothing is joined. After `Cancelled` the run is torn and the graph is
not returned. A cancelled task's future has been dropped. Shutting an executor down joins its
workers unless it is called on one of them.

**Stackless `'static` tasks** for IO, GPU, asset and network work. A task is a `Future` that
owns its data and may be cancelled by drop. Waiting is an `.await`.

**Worker classes.**

- Frame-critical workers equal the performance-core count on every platform:
  `hw.perflevel0.physicalcpu` on macOS, the hybrid core-type query on Windows, CPU capacity
  from sysfs on Linux, and physical cores minus one only where no core-type information
  exists. With more than two core classes, every class above the slowest is frame-critical.
  Counts are capped by the hardware threads the process may use (affinity, container limits):
  frame-critical workers at that number minus one, background workers at what remains, each at
  least one. They run at user-interactive QoS. They run simulation,
  extract, graph compile and record.
- Background workers run on efficiency cores at utility QoS (two low-priority threads on
  homogeneous CPUs). They run asset decode, compression, and long futures. Background CPU
  work yields at 0.5 millisecond slices. An asset's version check and loader are one
  synchronous call of `asset-api`'s `ErasedLoader`, which the store yields before and cannot
  yield inside (D136; `docs/verification.md` has the row).
- IO threads (`ThreadClass::Io`, the pool in `io`) run at utility QoS, half the cores on client and
  server (section 9.7), separate from the background workers.
- The platform thread pumps OS events. A dedicated present thread calls present so a blocking
  `nextDrawable` or FIFO present never stalls the event pump or a worker.

**Scheduling mechanics.**

- Each worker owns a `crossbeam-deque`. Work stealing uses thief-side split-on-steal: a
  stolen chunk-range job is split in half and the thief pushes one half to its own deque.
  There is no fixed task-duration target.
- Idle workers spin for 50 microseconds, then `std::thread::park`. The worker keeping the
  timers steps its timed park (0.6 of the time left, repeated) and spins the last 50
  microseconds, because macOS lengthens a timed wait by a leeway proportional to the timeout
  (D113). On Windows each step is a wait on a high-resolution waitable timer and on an event
  that a waker sets, because every timed wait of the standard library that another thread can
  end is rounded up to whole milliseconds there and ends on the system's timer tick, 15.6 ms by
  default; that timer overshoots by about half a millisecond whatever is asked, so the keeper
  spins the last 500 microseconds there (D144). A worker that releases several entries keeps one
  for itself and wakes one sleeper for each of the others, and a worker that fires timers on its
  way to its next job keeps the first task they wake in the same way, but only while nothing
  waits in the frame-critical queue (D116). The latch, the permit gate, the sleep-and-wake
  protocol and run quiescence are modelled under `loom`, through the shipped functions; the
  deque is a vetted dependency. aarch64 stress tests cover weak-memory behaviour loom does not
  model.
- `tracing` spans are emitted per system and per stolen range, never per chunk job. Finer
  levels compile out.
- Packet parsing runs on a dedicated network thread at frame-critical priority, not on
  background workers, so input lateness does not grow with load.
- Main-thread requests return futures. Workers never wait synchronously on the main thread.
- The floating-point control register (MXCSR, FPCR) is set to round-to-nearest with denormals
  enabled at worker start and asserted at the start of every boundary job in debug builds.
  Threads the engine does not own (the physics solver pool, the audio mixer) get the same
  setup and their QoS class through the safe `core::thread_class` helper.

**Panics abort, unconditionally, in shipped binaries.** An abort-on-unwind guard surrounds
every job and every join wait. The binaries install a panic hook that writes a crash dump and
calls `abort`. Tests do not install the hook, so `#[should_panic]` and property-test shrinking
keep working; a test that expects an abort runs the binary in a subprocess and asserts on the
exit status. Tests run under `cargo-nextest` with one process per test and timeouts. `task`
exposes no general blocking wait; calling one on a worker is a debug assertion.

**The timer root.** A root task owns ticking: it wakes at tick deadlines, caps catch-up at four
ticks per frame and paces frames; a rollback's re-simulation ticks are bounded by the prediction
ring's depth, which the client sizes to its frame re-simulation budget (section 9.4), run beside
the capped catch-up ticks and, while that depth fits the budget, cost the client no lead (D122).
Beyond the cap a client
re-syncs its clock (section 11) and a server climbs the overload ladder (section 9.7); neither
slows time. It runs on whichever frame-critical worker
picks it up. Section 11 covers time scale and pause.

### 5.2 Scheduling

- The scheduler builds a DAG per schedule at startup from declared access and set ordering,
  cached until systems change.
- Conflicting systems are ordered by set or system constraints; a missing constraint fails the
  build. The assembly crate may add an edge between two sets that do not name each other
  (`ScheduleBuilder::order`), so the exclusive systems of two backend crates can share a stage.
  Ties are broken by system stable key, the xxh64 hash of the declared system name,
  checked for collisions, never by registration order. The resulting order is the stage's
  execution order and is identical on every build and platform.
- Each system iterates chunks as jobs. Per-entity work is independent by construction. A chunk
  job decrements its system's counter only after any scope it opened has closed, so the
  finalize step never runs mid-scope, and finalize may open a scope of its own. Dependents are
  released as one range job per system that split-on-steal divides.
- Reductions use provided combinators. Float sums use an exact accumulator (fixed-point
  `i128` or a superaccumulator) so the result is independent of order without sorting;
  integer sums and `math` min and max are order-independent by nature. A reduction that
  genuinely needs stable-key order is declared `ordered` and pays a parallel sort in finalize.
  The query API exposes no chunk, row or worker index. Scratch memory is an opaque per-job
  arena, poisoned between uses in debug.
- Structural changes go into command buffers applied at the next sync point in the stage's
  execution order, then (source stable key, per-invocation sequence) within a system. A system
  that ran earlier therefore applies earlier. Spawns receive `StableKey::Placeholder` values
  that are valid within the same command buffer and resolved at apply.
- Sync points sit at schedule stage boundaries. Their count is fixed per schedule.

### 5.3 Frame pipeline

```
tick N        tick N+1 simulation ────────────────────────────────────┐
 │ extract (exclusive, parallel)                                       ├─ overlap
 └──► presentation snapshot ──► presentation schedule (per frame)      │
                            └──► graph compile → record → latch → submit ┘
```

- Simulation runs at a fixed 60 Hz driven by the timer root. Render runs as fast as the display
  allows.
- **Extract is exclusive, not single-threaded.** Between tick N ending and tick N+1 starting,
  a fork-join over dirty chunks (rows whose change tick is newer than extract's last-seen
  tick, or newer than the tick before so previous transforms stay valid) writes into a
  staging ring. A GPU scatter pass moves staging into the persistent GPU scene, so writes never
  race frames in flight. Teleports carry a no-interpolate flag. Static cooked instances go from
  IO to the GPU scene directly and never pass through extract. Extract also produces the
  presentation snapshot. Its budget is measured in milestone 2, not assumed.
- **The presentation snapshot** holds the rows presentation computes on: a row with a
  camera rig, a light, a decal, an animation state, a remote slot or any other extracted
  component beside its mesh and transform, with those components and its transform. A row
  that is only a mesh and a transform goes to the staging ring alone. It is
  triple-buffered so a frame can interpolate between two snapshots, which are its previous
  and current transforms, while extract writes a third, and the presentation events of the
  ticks a snapshot covers are published with it. It is the only simulation-derived data the render
  side reads during the overlap. Remote entities' transforms are not in it: the presentation schedule
  samples the network interpolation buffer each frame through a lock-free `net-host` resource
  (a sequence lock per slot, the slot named by the `RemoteSlot` component a staged spawn leaves
  on the replicated row, with a batch sampling call that takes the slots and fills the caller's
  buffer) and uploads their transforms through the same staging ring as extract. On a client this remote upload, not extract, is the dominant
  path, and its cost at 144 Hz for several thousand entities is measured in milestone 2. The
  client's replicated world runs no simulation systems; only the prediction world simulates.
- **The presentation schedule** runs per render frame on the snapshot: camera rig with
  non-authoritative boom collision against the immutable static collision asset, UI animation,
  audio listener, world-space markers, full pose evaluation, and VFX requests (section 13).
  VFX simulation itself is GPU compute in the renderer.
- **Late-latch** writes exactly one GPU buffer, the full orbit camera transform (pivot from the
  snapshot interpolated to frame time, rotation from the latest input, boom length from
  presentation collision), after all CPU view-dependent work and immediately before submit.
  The late latch writes its buffer before the frame's submit and gates no submit on a host
  signal. On Apple a submit whose host wait is unmet for about 5 s loses the device, and a
  present behind it is held (`lactose-rhi-api`, "The swapchain").
  Light binning runs on the GPU in camera-cell-relative space after the latch, and GPU culling
  reads the latched buffer, so neither needs a pad. CPU shadow-cascade fitting is padded by a
  configured maximum rotation and maximum boom-length change per frame, and the latched deltas
  are clamped to those pads. Boom collision is rechecked at latch time with a few sphere casts
  against the static BVH. The pivot needs no pad: it comes from the snapshot, so its motion is
  already inside the swept culling bounds. Each input sample carries the absolute yaw and pitch it was produced
  with; the simulation uses those values for movement direction. The world frame is
  right-handed with `+Y` up; yaw zero faces `-Z` and a positive yaw turns left, pitch is
  positive upward. `PlayerInput::aim`, `PlayerInput::world_movement` and `Yaw::rotation` in
  `input` are the one derivation of a direction from a sample, and the camera rig uses the
  same convention. glTF assets face `+Z`; the cooker turns them half a turn about `+Y` so a
  character faces `-Z` at yaw zero.

## 6. ECS data model

- **Archetypes** group entities by exact component set. **Chunks** are 16 KiB slabs with one
  contiguous column per component. Row capacity derives from row size with a minimum of one.
  The registry warns on components over 1 KiB. The determinism harness varies chunk capacity
  in tests (section 8.3).
- **Components** are `bytemuck::NoUninit + CheckedBitPattern`: no padding, no heap ownership,
  `bool` and fieldless enums and `Option<NetId>` allowed. Byte validation runs only on save
  load and network receive. The `#[component(key = "name")]` attribute declares the stable
  component key (xxh64 of the name, collision-checked), lays the struct out as `repr(C)` in
  declaration order and rejects implicit padding at compile time with an error naming the
  field; the author closes gaps with `#[field(padding)]` fields of type `Padding<N>`, whose
  only value is zero. Enums are fieldless `#[value]` types held in a field; data-carrying
  enums are not reflected. Components cannot be generic. Inline arrays are
  `PlainArray<T, N>` plus a length with unused slots zeroed. A region-owned blob arena is permitted only if
  profiling demands it and is then part of the region snapshot.
- **Change tracking.** The tick unit is the simulation tick, held in a `u32` that is clamped
  forward every 2^30 ticks. Every column in every chunk carries a coarse change tick, written
  once at dispatch for every chunk a writing system touches, so split jobs never share the
  word. Columns marked tracked (replicated, extracted or both) also carry a change tick per
  row, set through `Mut<T>` on deref-mut; row ticks move with their rows on compaction. A
  per-world structural change log records spawns, despawns, component additions and removals
  and region moves with their tick, consumed per consumer like row ticks, so replication can
  emit tombstones and the renderer can free slots. `Changed<T>` filters mean changed since the
  reading system last ran, each change seen exactly once by each reader, which needs a
  per-row stamp and so is allowed on tracked columns only. Replicated columns use
  `set_if_neq` so rewriting an equal value does not dirty the row. Because each row's tick is its own word, chunk jobs may split at any row without a shared
  write; no atomics are involved. Whole-column slice access records the rows it changed in
  an explicit row set.
  Extract keeps a last-seen mark. Replication keeps a per-client, per-entity acked tick
  (section 9.3). After a rollback restore or a whole-world load, stored change stamps are
  rebased (a restore puts them back as snapshotted) and every consumer's state is reset,
  forcing a full pass; a region load stamps its rows at the current tick instead.
- **Regions** are the unit of streaming, relevance and persistence. Region membership is a
  component; chunks belong to archetypes, not regions, so scattered players do not fragment storage
  into mostly-empty chunks. Region operations (unload, save, relevance) are filtered scans, and the
  only world that is ever snapshotted whole is the separate prediction world. A region owns an asset
  version table and region resources. Entities move between regions by a command that changes the
  membership component; identities do not change. Ownership (`OwnedBy`) is a forest the ECS keeps
  over the live world: an entity has at most one owner, is never its own owner and no chain of
  owners returns to its start. A write that would close a cycle is refused; a region load, or a
  bind, that would close one through a saved or stale edge the world could not see leaves the
  closing reference empty and logs it, so every saved region loads and the broken edge is what the
  next save records. Entities owned by another entity (inventory items, attachments) migrate with
  their owner and are despawned with it; writing `OwnedBy` moves nothing, so an owned entity may
  reside in another region than its owner, where `OwnedBy` is a weak cross-region reference like
  any other, except in a save: an owner is written as its `NetId`, where every other reference
  is written as the origin key it holds, and is stored at a load as the origin key that `NetId`
  names in the loading world (section 7). That is the rule of the world that simulates them. A
  client's worlds apply the server's structural log entry by entry, each tombstone and each move
  to the one entity it names, because a client holds `OwnedBy` only as each region's document
  stated it (D141).
- **Region modes**: `Simulated`, `Replicated` (rows written only at the input sync point from
  staged network state), `Frozen`. The input sync point is the first sync point of a tick,
  before any stage runs; it is where the time scale, asset version tables, region bundles and
  mode changes, sessions, inputs and staged network state are applied.
- **Relationships** are components holding `StableKey` with a reverse index maintained at
  command apply and included in snapshots. Cross-region references are weak and validated at apply.
- **Resources** are singletons with declared access. A world resource holds no entity reference
  (section 8.3).
- **Queries** are generic over component tuples and monomorphise.
- **Transient columns** are zeroed by the ECS at the input sync point every tick, and the
  scheduler checks statically that every reader of a transient column is ordered after a
  declared writer. Transient safety is structural, not tested.

### 6.1 System API

This is the surface every agent writes against. Access is declared by parameter types; the
`#[system]` macro emits an explicit access table at compile time, so there is no runtime
introspection. A system body is a kernel that the scheduler runs once per entity across chunk
jobs, or once per simulated region if it has no component parameter but a region context,
or once per tick if it has neither; multi-phase logic is several systems. Exclusive systems
take the whole world.

```rust
// Per-entity kernel. `worlds` selects where it runs; the world injects its own filter, so a
// prediction-world run sees only predicted entities and game code never writes that filter.
#[system(name = "game_sim::apply_gravity", worlds = [server, prediction], in_set = MOVEMENT)]
fn apply_gravity(dt: Res<TickDelta>, g: Res<Gravity>, vel: Mut<Velocity>, mass: &Mass) {
    vel.y -= g.0 * dt.0;                  // deref-mut through Mut<T> sets the row tick
}

// Reduction. Exact accumulator by default; the finalize closure runs once when all chunk jobs
// have completed and writes the result to a resource.
#[system(name = "game_sim::total_mass", worlds = [server], reduce = ExactSum<Scalar>)]
fn total_mass(mass: &Mass) -> Scalar { mass.0 }
#[finalize(total_mass)]
fn store_total(sum: Scalar, out: ResMut<TotalMass>) { out.0 = sum; }
// `TotalMass`, like `Gravity`, is a world resource and so `derived`: a world resource is
// configuration or session state, never hashed or saved (section 8.3); state that belongs to
// no region is a resource of a pinned region.

// Spawn that references the spawned entity in the same command buffer. `HeldItem` holds a
// `#[value]` enum, since enums are fieldless types held in a field (section 6).
#[system(name = "game_sim::throw_bomb", worlds = [server, prediction])]
fn throw_bomb(ctx: Ctx, cmd: Commands, physics: ChannelWriter<PhysicsCommands>, key: StableKeyOf, held: &HeldItem, input: &PlayerInput) {
    if input.pressed.contains(Action::Throw) {
        let bomb = cmd.spawn((Transform::at(..), Bomb { fuse: 180 }));   // StableKey::Placeholder
        cmd.set(*key, HeldItem { kind: ItemKind::None });
        cmd.set(bomb, ThrownBy { thrower: EntityRef::new(*key) });
        let spread = ctx.rng(bomb).next_scalar();  // keyed by (region seed, tick, system, this entity's origin key, the placeholder's spawn index)
        physics.apply_impulse(bomb, input.aim() * spread);   // `PhysicsExt` on the writer
    }
}

// Cross-entity effect: never a direct write to another row. Emit through a typed writer, so
// the schedule orders every reader after every emitter, and a later system consumes.
#[system(name = "game_sim::sword_hit", worlds = [server, prediction])]
fn sword_hit(damage: EventWriter<Damage>, hits: &HitList, key: StableKeyOf) {
    for h in hits.iter() { damage.emit(Damage { target: h.key, source: *key, amount: h.amount }); }
}
#[system(name = "game_sim::apply_damage", worlds = [server, prediction], after = sword_hit)]
fn apply_damage(events: Events<Damage>, key: StableKeyOf, hp: Mut<Health>) {
    for d in events.for_target(*key) { hp.0 -= d.amount; }   // events are indexed by target key
}

// Presentation event. Keyed by (origin key, tick, kind, sequence) so rollback re-emission
// deduplicates and events from mispredicted ticks are cancelled. `Phase` is
// `lactose_anim`'s integer clip fraction; the set orders the reader after the advance that
// writes `AnimState`, or the schedule refuses the build.
static ANIM_READERS: SystemSet = SystemSet::new("game_sim::anim_readers").after(&[&lactose_anim::sets::ADVANCE]);
#[system(name = "game_sim::footsteps", worlds = [server, prediction], in_set = ANIM_READERS)]
fn footsteps(present: PresentEvents, key: StableKeyOf, anim: &AnimState, at: &Transform) {
    if anim.phase_crossed(Phase::HALF) { present.emit(*key, AudioRequest::at(FOOTSTEP, at.translation)); }
}

// Exclusive system: runs alone at a sync point with the whole world.
#[system(name = "sim::apply_region_load", exclusive)]
fn apply_region_load(world: &mut World, loaded: ResMut<LoadedBundles>) { /* bulk copy rows */ }
```

Physics writes go through a `ChannelWriter<PhysicsCommands>` parameter (impulses, forces, teleports,
the `PhysicsExt` methods of `lactose-physics-api` on it) or through
the body's own `Transform`, `Velocity`, `RigidBody` and `Collider` components, and both apply in
the physics stage: its sync point pushes the rows' changes into the solver and takes the commands
recorded in the input and simulation stages of the tick, then the step applies them; the step's
results reach the rows at the sync point that begins the post stage. A
`ChannelWriter<PhysicsCommands>` in the physics stage or the post stage is a schedule build error,
since the tick's closing sync point clears every channel and a command recorded after the step would
reach no step; a reaction to this tick's step that acts next tick is a persistent component holding
the decision, which a simulation-stage kernel of the next tick turns into the command, through an
event when it acts on another body (D126). Relationship
fields are written through commands so the reverse index stays consistent. `StableKeyOf` yields
the entity's origin key; no row, chunk or worker index exists in the API. The macro makes every
parameter binding mutable, `#[finalize]` finds its system by name in the same module, and
`writes = [T, ..]` declares the columns a system writes outside its parameter list (through
commands, or through the world in an exclusive system), which is how a transient column
written at the input sync point gets its declared writer. `ChannelTake<C>`, in an exclusive system
alone, takes a channel at its stage's sync point, and the schedule refuses a `ChannelWriter<C>` in
that stage or a later one and a `Channel<C>` after the take (D126). The
week-one facade exposes exactly this surface over a naive single-threaded implementation,
including regions, change ticks, the structural log, snapshots, bulk copy and command buffers,
so dependent agents build against the final API from the start; `crates/ecs/README.md` is its
contract and D114 records the choices this section left open.

## 7. Identity

| Key | Scope | Assigned | Used for |
|---|---|---|---|
| `EntityId` | one process | process-local allocator, index plus generation, `NonZeroU64` so `Option` is free | storage lookup only; never on the wire or disk |
| `StableKey` | everywhere a reference is stored | enum: `Net(NetId)`, `Predicted(PredictionKey)`, `Placeholder(u32)` within one command buffer | `EntityRef` fields, relationships, render slots, event targets, RNG keys, ordering ties |
| `NetId` | server and all clients, and saves | 64-bit with a tag bit. Cooked: hash of (authored scene GUID, authored placement GUID, child path of authored GUIDs), so clients derive it without a message and re-cooking preserves it. Every element is a GUID stored in the authored file, never a name, path or index, so renaming a scene or reordering siblings preserves the ID. Runtime: one server counter, assigned at command apply and persisted in the save; a runtime ID a world is given any other way, by a direct spawn, a load or a bind, moves the counter past it, so the counter is above every runtime ID in use | the `Net` arm of `StableKey`; the persistent ID of every entity that has one. There is no separate `PersistentId` |
| `PredictionKey` | one client plus the server | (player ID, input tick, ordinal), where the input tick is the slot the player's input is applied for this tick and the ordinal is a hash of the spawning invocation (the source entity's origin key, the system key, the spawn's index within the invocation), for a spawn commanded from an entity the player has claimed by a system that runs in both the server and the prediction world; a hash of the invocation rather than a position in the command order, so a spawn the other machine cannot reproduce shifts nothing. The server records it as the entity's permanent origin key, persisted as a component so a save or a region overlay carries it, and stored references hold origin keys so a load changes no bytes | the `Predicted` arm of `StableKey` until the server binds a `NetId`; RNG and ordering use the origin key on both machines, so binding changes nothing. A predicted spawn the server never echoes is destroyed when the processed-input acknowledgement passes its input tick |
| `SystemKey` | build | xxh64 of the declared system name, collision-checked | command order ties, RNG stream selection |
| `ComponentKey` | build, wire, disk | xxh64 of the declared component name, collision-checked | registry, protocol hash, save records |
| `PlayerId` | session | server-assigned on join | prediction keys, interest sets, input queues |

Typed reference fields (`EntityRef` holding `StableKey`, `AssetRef` holding `AssetId`) are known
to the registry, so generic code can find them: command apply resolves a placeholder and writes
a bound `Net` key as its origin key, a direct write and a bind do the same for a `Net` key, and
the world hash reads both kinds. No load rewrites the references of the rows it loads: an
`EntityRef` holds an origin key and an `AssetRef` an `AssetId`, the hash of the asset's declared
name, each the same on every machine, and each is exported and loaded as it is, so the bulk-copy
load path is a copy. The one exception is an owner, which a save writes as its `NetId` and a
load resolves through the document's own rows or the world's binds (section 6). The binds a load
makes rewrite the `Net` keys of other rows as any bind does. The prediction world keeps its
prediction-to-network alias table across a rollback restore, since a bind is a fact from the
server, and a bind for an entity the restore removed applies again when re-simulation respawns
it. A server holds the bind of an entity for as long as the entity exists, live or in the saved
state of an unloaded region, and drops it at the end of the step that despawned it, or of the
next step for a despawn between steps. A save holds each bind as the entity's persisted origin
key, and a world that starts from a save, or a replay from a journal's checkpoint, is given
every bind of the save before its first region loads, since without the owner's bind a row loads
to other bytes than its export read (D142).

## 8. Determinism

### 8.1 The boundary

Inside: the `[boundary]` list in `crate-tiers.toml`, which is authoritative. At revision 3.4
that is `core`, `math`, `ecs`, `sim`, `anim`, `predict`, `nav-query`, `physics-api`,
`physics-rapier`, `bundle`, `input`, `net-api`, `game-sim`, `game-ai` and `game-content`. Gameplay
particles that
affect simulation (spreading fire, wind pushing objects) live in `game-sim`. The world hash and
bulk serialisation are implemented in `sim` with per-type hashers and copiers generated by the
derive, so `reflect` supplies metadata only and never sits on a determinism path.

Outside: everything else, including `net` (transport, RTT, interpolation buffer),
presentation, pose evaluation, VFX, audio, UI, asset IO timing and input sampling.

What a world registers, stores and runs is held to every rule of section 8.2, those that review
holds included, whichever side its crate is on. That is every `#[system]`, a pass system
included, every `#[component]` or `#[resource]` that is not `derived`, every implementation of
`SessionApplier`, `InputApplier` or `StagedState`, every hand-written `impl Reflect`, and the
setup and migration functions of a `CrateRegistration`. Each is declared in a boundary crate,
in a crate outside it that carries the boundary lint file (section 8.3), or in test code alone.
Carrying the file brings no crate inside. The dependency allowlist does not bind a carrier, so
review holds what its declared code calls; its other code is outside, and where that code must
reach the outside world it takes a local exception with its reason. A pass system runs once a
frame with the whole world, whether or not a tick ran, so it leaves the world hash as it found
it: it writes no hashed column and nothing a tick's systems read. No tool checks that; the lane
that adds a pass system proves it with a recorded test (D152).

### 8.2 Rules inside the boundary

- Floating point only through `math`, which is self-contained and does not depend on glam.
  `Scalar` is a newtype, so no inherent float method such as `sin` or `powf` can reach the
  platform library by accident. Transcendentals come from the `libm` crate, and so does a float remainder, as `Scalar`'s `%` and `rem_euclid`: the primitive `%` and `%=` on `f32` and `f64`, and the `Rem` and `RemAssign` of glam's vectors and simba's SIMD types, call the platform `fmod` and are banned inside the boundary, by review until a lint holds them (D149). `math` owns `min`, `max` and `clamp` as explicit
  compare-and-select with defined signed-zero and NaN results, because the standard library
  versions are documented as nondeterministic on signed zero. `f32::min`, `f32::max`, `clamp`, `powi`, `total_cmp`,
  `to_bits`, `copysign`, `is_sign_negative`, `is_sign_positive`, `rem_euclid`, `div_euclid` and
  `mul_add` and their `f64` forms are banned outside
  `math`, as are trait routes to the platform library such as `num_traits::Float` and simba's
  `RealField` `min`, `max`, `copysign` and sign tests, and the math crates' own routes to it
  (num-complex's `Complex`, glamx's eigen and SVD, glam's Euclidean division and sign-bit methods);
  NaN sign and payload are never
  observable. NaNs are canonicalised before hashing and serialisation, and one in a
  persistent column is a defect: in debug builds the world hasher asserts against it when it
  hashes a persistent type, and the save's serialiser when it diffs a live world's export.
  Nothing checks at the write or at a decode, so a NaN that a document or a frame brings into
  a live world is asserted at that world's next hash or save, as the defect of the build that
  wrote it.
- Fused multiply-add is correctly rounded and permitted. The hazard is code that chooses fused
  or unfused by target feature, which `wide` and glam do. Therefore x86-64 targets are pinned
  to the x86-64-v3 baseline through target-specific rustflags and rustdocflags in
  `.cargo/config.toml` (never the environment variable, which replaces them; cargo does not
  pass rustflags to doctests). `math`'s fused multiply-add is never inlined into a caller, so
  it is always compiled under that pin, `math` contains a `compile_error!` on x86-64
  without the `fma` feature so a dropped pin fails the build, and `target-cpu=native` is
  denied.
- SIMD is permitted for IEEE basic operations. Reduction order inside SIMD code is written out
  explicitly; ISA-specific horizontal operations, approximate reciprocals and reciprocal square
  roots are banned.
- glam appears inside the boundary only through Rapier's pinned math crates in
  `physics-rapier`. `math` has no glam dependency, so nothing outside the boundary can change
  boundary numerics through Cargo feature unification. The reverse direction is accepted:
  Rapier's determinism feature may force scalar glam on the renderer, which has no hot path
  that depends on glam SIMD. `cargo-deny` denies multiple versions of `libm` and of every
  crate Rapier's math depends on, so simulation and physics share one `libm`.
- No runtime CPU feature detection that selects numerically different paths. `libm`'s own
  detection selects correctly rounded paths only and is allowlisted.
- No wall-clock time. `Instant` and `SystemTime` are banned; `Duration` as a value type is
  fine. Tick index and fixed delta only.
- No file system, process environment, subprocess or network access: what the simulation reads
  enters as a tick input. The boundary lint file bans `std::fs`, `std::env`, `std::net`,
  `std::process::Command`, standard input and what the process learns of itself (its ids, its
  terminal, its working directory, the core count, the backtrace variables); a test that needs one
  lifts the lint with `#[expect]` on its one item. Every crate of `[boundary_internal] any` carries
  these bans too, and a boundary crate's tests are held to the dependency allowlist as the crate is.
- No thread creation outside an executor. Parallel work goes through `lactose_task`'s executor, and
  the physics step through the solver pool of section 16, which `lactose-app` builds and hands to
  the backend. The boundary lint file bans `std::thread::spawn`, `Builder`'s and `Scope`'s spawns
  and `std::thread::scope`, the scope those threads start in;
  `lactose_task::Platform::spawn_thread`; rayon's and Rapier's pool builders; and the routes found
  to start rayon's global pool: rayon's free functions, iterator sources and bridges, the traits a
  parallel iterator of a crate's own is written with, Rapier's `clear_thread_pool`, and the public
  functions of the pinned Rapier and parry found to run parallel work outside
  `PhysicsPipeline::step`. The list is not provably complete (D132). `cargo xtask boundary` requires
  each listed route. A test that needs a thread lifts the lint with `#[expect]` on its one item.
  Every crate of `[boundary_internal] any` carries the standard library's entries.
- No thread-local state. No global mutable state. The one exception is the allocation tag
  in `core`, which the allocator alone reads and no simulation result can observe.
- Hash maps are `core::FixedHashMap`. Iteration order is never observable.
- Chunk index, row index, worker index and task order are never semantic keys. The API makes
  them unavailable. RNG draws are keyed by (region seed, tick, system key, stable entity key) through a
  counter-based generator with no mutable state. The stable entity key is the `NetId`, or the
  `PredictionKey` for a predicted spawn the server has not yet confirmed.
- A boundary crate may depend on a non-boundary foundation crate (`reflect`, `asset-api`,
  `present-api`) only for types and for operations that are deterministic by construction: no
  clocks, no floats computed outside `math`. `input-host` is outside and no boundary crate
  depends on it: the sample it quantises enters through `input`, a boundary crate. `net-host` is
  outside and no boundary crate depends on it: what crosses into the simulation is the staged
  frame of `net-api`, a boundary crate, which the network thread builds from what `net-host`
  decodes. `io` is outside the boundary; no boundary crate depends on it, directly or at any
  depth. `ecs` depends on `task` for
  execution only and re-exports `Executor`, which every boundary crate can then name; the
  boundary lint file bans the executor's worker counts, arbitrary work, thread creation,
  deadlines and scheduling history, so what a boundary crate can do with it is run a
  schedule, block on a future, and build or shut down an executor. Every result is
  order-independent by construction. `crate-tiers.toml`
  names the permitted edges, to workspace crates and to third-party crates at any depth,
  `cargo xtask boundary` enforces them, and the tier check forbids build scripts and proc
  macros from depending on boundary crates, because they compile for the host without the
  pinned target features.
- A third-party crate may be built from a patched copy: `[patch.crates-io]` in the root
  `Cargo.toml` names `vendor/<name>-<version>`, the published archive as `cargo vendor`
  unpacks it with the `.cargo-checksum.json` it writes, and the patch, a unified diff, sits
  beside it as `vendor/<name>-<version>.patch`. No other `[patch]`, `[source]` or `paths`
  setting exists, in a manifest or a cargo configuration file, and a crate is patched at one
  version only. A patch fixes a defect of the pinned version: inside the boundary, a route
  that breaks a rule of this section, such as a transcendental from the platform library;
  anywhere, a defect that blocks a milestone criterion; never a feature, an optimisation or
  an API change. It changes at most fifty lines, counting every line it adds or removes,
  comments included. It never touches `Cargo.toml`, `Cargo.toml.orig`, `Cargo.lock` or
  `.cargo_vcs_info.json`, so the crate's version, features, dependencies and closure stay
  upstream's; it may touch a build script, since `libm` selects its paths in `configure.rs`
  and cannot be refused, and such a patch alters what the crate compiles on every target and
  is reviewed as that, but it never creates one; it adds no `unsafe` and no dependency, and
  never touches a file that holds `unsafe`; no added line exceeds 200 characters or holds, as
  a whole identifier, `no_mangle`, `export_name`, `link_section`, `link_name`, `naked`, `asm`,
  `global_asm`, `naked_asm`, `include`, `include_str` or `include_bytes`, and no patch adds a
  `path` attribute or an out-of-line `mod name;`; every change carries a `LACTOSE patch:`
  comment, the notice Apache-2.0 asks of a modified file, and names none of those words.
  Inside the boundary a patch is reviewed as a determinism review, like a new allowlist
  name. The owner
  reports it upstream, and `vendor/README.md` records the link, the reason and what drops it.
  Over fifty lines is not a patch: the feature the route serves is refused, or the dependency
  is changed, by a decision. A bump re-applies the patch or drops it once upstream carries
  the change; a patch that no longer applies is a new patch and a new review. The copy is a
  dependency, not a member (`exclude = ["vendor"]`): no workspace lint, format or test runs
  over it, cargo caps none of its own warnings, the crate keeps its crates.io name on the
  allowlist and in `deny.toml`, and `.gitattributes` keeps `vendor/**` free of line-ending
  conversion so every host hashes the same bytes. The same mechanism and check serve a crate
  outside the boundary, since a patch applies to every user of the crate in the workspace.
- Asset availability and versions are inputs. Each region resolves its version table at the
  input sync point; hot reload of simulation assets is a server-coordinated, tick-stamped
  event.

### 8.3 Enforcement

`sim` exposes `Simulation::world_hash(region) -> Result<u64>`: each row hashes as (stable
key, component key, field ID, canonical value) through a monomorphised per-type hasher,
chunk jobs when forked, and the row hashes fold by addition, so no sort is needed and the
hash is identical under any layout. It covers persistent and transient columns and region
resources, and the region's seed, mode and asset version table; derived columns and
resources are excluded, and world resources are neither hashed nor saved: a world resource is
configuration or session state, must be `derived` and holds no entity reference, since a bind
and a restore rewrite the references of rows and region resources alone, so
`World::insert_resource` does not build for a type that holds one (D142), and world-global
persistent state is a resource of a pinned region. It runs at sync points, and a field-level
diff is produced only on mismatch. Rapier's serialised bytes are shown order-stable: SOUR records
their hash after the conformance run, equal across platforms in nightly 43703297881.
`PhysicsSnapshot::hash` is recorded beside the world hash, as a stream of its own that SOUR
compares in every mode, and is never folded into it: the bytes are the whole world's and a
function of its row history (section 16), where the world hash is a region's, a function of its
rows, and is compared between a prediction world and its server, whose solvers never agree
(D145).

The harness, named SOUR, runs every simulation test in these modes and fails on any mismatch:

| Mode | What varies | Catches |
|---|---|---|
| Workers | 1 worker versus N | scheduling-order dependence |
| Repeat | same configuration twice | uninitialised memory, time, randomness |
| Layout | test-only chunk capacities (1, 7, 64 rows, 16 KiB), random row permutation at every sync point, reversed and shuffled chunk iteration, random batch splits; identities preserved | storage-order dependence |
| Chaos | seeded steal order, injected yields, forced split-on-steal | ordering bugs invisible on 4-vCPU CI |
| Architecture | linux-x64 (including a Haswell-masked run), windows-x64, macos-arm64 | float library and SIMD divergence |

Layout and chaos modes gate milestone 1. A pull request runs workers mode and one layout
capacity; the rest of the matrix and architecture mode run nightly, and architecture mode
moves to every PR from milestone 4. The Haswell-masked run gates milestone 4 as well, wired
under an emulator that hides every CPU feature newer than the x86-64-v3 baseline. Until then
the nightly prints each runner's CPU model and feature flags beside its hashes, and a
comparison that differs between two x86 hosts wires the run at once.
Layout and chaos are runtime options on the world and
executor builders that are no-ops in production, never `cfg(test)` or Cargo features, since
neither reaches a dependency's build. Chaos runs record the job execution log so a failure
replays exactly. The one-row capacity uses a matching small slab.

Static enforcement uses stable tooling: `clippy.toml` per boundary crate with
`disallowed-methods`, `disallowed-types` and `disallowed-macros` (thread-locals are banned
through `std::thread::LocalKey`, since a local exception cannot reach an item macro; run-time
CPU feature detection through `std::arch::is_x86_feature_detected!` and
`std::arch::is_aarch64_feature_detected!`, expression macros the lint reaches, and on x86-64
through CPUID read directly, `core::arch::x86_64::__cpuid`, `__cpuid_count` and
`__get_cpuid_max`, by any path or name); the boundary dependency allowlist in
`crate-tiers.toml`, checked by `cargo xtask boundary`, directly and through
the workspace closure; `cargo-deny` with `multiple-versions = "deny"` for shared crates; `cargo
xtask vendor` for each crate built from `vendor/` (section 8.2): offline, the copy is its patch
applied to its record and the patch is within the rule; online, the record is re-derived from the
crates.io archive and the advisory check runs over the upstream versions, which `cargo-deny` skips
behind a path source. The two proc-macro crates carry the boundary `clippy.toml` as well, since the
code they emit is compiled inside boundary crates. So does each crate outside the boundary that
declares what a world registers (section 8.1): `lactose-present-api` for the presentation
components and extract's pass system; no binary is one, since the headless binary's sketch game
moved into `game-sim` at milestone 2 (D152).
`[lint_file_carriers]` in `tools/xtask/ci-policy.toml` lists the carriers, and a crate joins it in
the change that adds its first such declaration. `cargo xtask boundary` reports, in the shipped
sources of any other crate outside the boundary, a `#[system]`, a `#[component]` or `#[resource]`
that is not `derived`, an `impl` of `SessionApplier`, `InputApplier` or `StagedState`, and a
hand-written `impl Reflect`, which declares a component or a resource that no attribute shows.
Shipped sources are the crate's module tree without what only tests, benches and examples compile:
its `cfg(test)` items, `tests/`, `benches/`, `examples/`, the build script, and items behind a
feature that `[dev_only_features]` lists. Attributes and traits are matched by the last segment of
their path as written, so one brought in under another name, written by a macro the crate's source
does not spell out, or inside a `cfg_attr`, is not seen. `Component` and `Resource` implementations
are not matched, since the renderer's GPU traits share the names. A setup or a migration function
is not matched, nor is a tick-stage system told from a pass system. Review holds what the check
does not see.
`cargo xtask boundary` holds every entry of the reference `clippy.toml`, and its table, to the
lists in `tools/xtask`, and `[harness]` in
`tools/xtask/ci-policy.toml` to the packages that include the harness protocol;
`cargo xtask sour run` holds the tests that record hashes to the list beside it.
A test runs under the modes above only when it takes its configuration from the harness session and
records its hashes through it, whichever package holds it. So every system that writes a hashed
column is declared in a library, and runs on rows in at least one test that links that library and
records through the session; that test's package is listed under `[harness]`. A copy of a system in
test code is not that system, and a test can run a binary's system only as a copy. A test that
steps a world and records nothing runs once, under one configuration, and proves no determinism. No
tool reports a system that no recorded test runs: the review of the lane that adds the system does
(D152).
No custom lint crate. Miri runs on nightly as the one exception to the stable-toolchain rule.
Because Cargo resolves features per build, CI dumps `cargo tree -e features` for the boundary
dependencies of each shipped binary, the SOUR test crate among them, and requires them
identical; every other package whose tests run under SOUR must compile each boundary package
it shares with them at every version both compile and with the same features; the SOUR test
crate's dependency set mirrors the server binary; and the resolved feature set is part of the
boundary build ID.

## 9. Networking

### 9.1 Topology and scale

Dedicated server, authoritative. Players join and leave freely; there is no fixed cap. Server
cost is bounded by active entities: regions with no player within the activation radius are
`Frozen`, which is a deterministic function of inputs. Per-client cost is bounded by the
relevance set.

The handshake carries a protocol hash over the replicated types and the hashed (persistent or
transient) types only, plus the protocol constants, each type encoded by `reflect`'s canonical
schema bytes; a type registered on one side only is derived and unreplicated, or the handshake
refuses (presentation types differ between client and server by design), a boundary build ID (the
`Cargo.lock` entries, the resolved feature set, and the source hash of boundary crates and of
every crate built from `vendor/`, since the lock entry of a patched crate names neither a source
nor a checksum), and the hash of simulation-relevant cooked data. Mismatches refuse connection.
Section 9.7 gives the capacity model.

### 9.2 Data in and out of the tick

- Received packets are parsed on the dedicated network thread (section 5.1) into per-client
  staging queues.
- At the input sync point, and only there, staged inputs are applied to the server tick and staged
  state is applied to `Replicated` rows on the client. Nothing writes simulation rows concurrently
  with systems or extract. Staged state reaches each client world as one frame per server tick that
  `app` queues in the world's inbox between runs; a frame's region relevance, time scale and asset
  tables enter through the `TickInput` of the step whose sync point applies the frame, and the
  client's worlds start at the tick the join acceptance names, the prediction world that tick plus
  the lead `app` chooses until the first clock estimate arrives (D131).
- Each input packet carries the last eight ticks' samples (`lactose_input::REDUNDANT_SAMPLES`,
  amendable by `net`), so a single lost packet costs nothing. Inputs
  are quantised on the client before simulation, so the client simulates exactly the value the
  server receives. Server input policy: inputs are buffered per client with a small jitter
  window; an input still missing after redundancy is replaced by the previous input with its
  edges cleared (`PlayerInput::substitute`), the first input delivered afterwards has the edges
  the gap swallowed restored (`after_substitute`), and the client is told.
- Client clock sync: the client runs ahead by half the round trip plus a jitter margin,
  measured in simulation time under the replicated time scale, and adjusts its tick rate by up
  to three percent to track. Beyond the catch-up cap it snaps its clock and resends inputs
  rather than slowing.

### 9.3 Replication and interest

- A **replication gather** phase on the server, exclusive and parallel like extract, copies
  rows whose tick advanced and the structural log into a per-tick staging buffer. Per-client
  packing then runs off the critical path against that buffer, never against live chunks. The
  gather is `net-api`'s one pass system, selecting rows by change stamp, and the buffer it fills
  is a service resource of the server world that `app` moves to the packer between runs; the
  packer keeps a mirror of every replicated row of every relevant region, since a delta buffer
  holds only changed rows.
- Per client, the server tracks the relevance set, a priority accumulator per relevant entity,
  a per-entity acked change tick, and a ring of packet-to-entity lists so an acknowledgement
  advances exactly the entities it covered. Bandwidth caps make sends partial, so a single
  last-seen tick per client cannot work. Relevance is region membership within a radius plus
  an always-relevant set; co-located clients share per-region dirty lists so packing cost is
  proportional to clients times relevant rows, not clients squared. Each packet is filled by
  priority (distance and staleness) up to the client's cap. Destroyed entities travel as
  tombstones from the structural log, one for each entity of an owned tree, and a client applies
  a tombstone or a move to the one entity it names, never to what its own `OwnedBy` edges say
  that entity owns (D141).
- Entities are addressed on the wire by short per-connection handles (a sixteen-bit index and
  generation the server assigns and the client mirrors from the spawns, which carry them), not
  by 64-bit keys. Positions are quantised relative to the origin of the region the wire names; a
  region move travels as a frame entry applied to the one entity it names through
  `World::move_alone_to_region`, and so does a component removal through `World::remove`. Send
  rate 20 to 30 Hz.
- When a region becomes relevant to a client, the server sends that region's overlay (or a
  digest the client verifies against its cached copy) with the region's seed and persistent
  region resources beside it, and the client applies it to the cooked baseline before any
  static instance is uploaded to the GPU. The overlay is a region-scoped
  document, distinct from the row replication stream.
- On the client, `net` owns an interpolation buffer per remote entity holding received states.
  The presentation schedule samples it each frame, about two send intervals in the past,
  through a `net-host` resource. It never feeds simulation rows. `Replicated` rows hold the latest received state for
  simulation-side queries.
- Late join and resync use the same path: a full relevance-set snapshot from the server.
- Asset publish timing on the server is journaled as an input so replays reproduce it.

### 9.4 Prediction

- The client owns a **prediction world**, separate from its replicated world, with its own
  physics world, entity table and snapshot ring. It holds the local player, entities the player
  has claimed (thrown objects, predicted spawns), and the immutable static collision asset.
  Until milestone 3 designs how that asset reaches the prediction world's solver outside the
  per-tick snapshot, static collision there is `Collider`-only rows, and a prediction world
  holds at most 64 fixed colliders and proxies together (section 10).
  In the prediction world a proxy with a `RigidBody` is a kinematic body moved along its
  per-tick history kept in the snapshot ring rather than teleporting, whatever its declared
  type, and becomes its declared type when the player claims it; a `Collider`-only row is a
  fixed collider in every world, so a replicated entity that moves carries a `RigidBody`.
  Predicted entities collide with static geometry, other predicted entities and proxies, are
  pushed by proxies and never push them, and a predicted interaction with a proxy is
  presented tentatively until confirmed.
- A prediction world's region is `Frozen` until its asset table is published and `Simulated`
  after, never `Replicated`: kernels run and bodies step only in a `Simulated` region. The
  client loads a region whose relevance carries no table `Frozen`, and the region change that
  brings the table makes it `Simulated`. `app` directs these modes from the frames it folds;
  the activation policy of section 9.7 runs on the server alone. A region the server holds
  `Frozen` with its table published is simulated on the client all the same, because
  `RegionChange` carries no mode, and only a frame that carries rows of an entity the client
  predicts there corrects that entity. The server sends a claimed entity whole on each update,
  changed or not (reconciliation, below), so each update corrects it while the server holds
  the region `Frozen`; a sender of changed rows alone would leave it uncorrected until the
  next full frame, since a `Frozen` region changes none. A mode variant is a `net-api` change,
  decided before `net`'s packer sends region changes.
- Systems declare the worlds they run in (section 6.1). The prediction world injects a
  predicted-only filter; game code never writes it. The local player exists in both client
  worlds, and the prediction world's copy shadows the replicated one for extract and the
  render slot table.
- Presentation events emitted during re-simulation deduplicate by (origin key, tick, kind,
  sequence), and events from ticks that were mispredicted are cancelled.
- Ring depth is derived at runtime from the frame re-simulation budget divided by the measured
  per-tick cost, with a configurable cap. Corrections deeper than the ring trigger a rebuild
  from replicated state. Section 10 defines what a snapshot holds.
- **Reconciliation** compares predicted and authoritative state per field at full precision;
  the server sends every simulation-read field of predicted entities at full precision on each
  update, since it cannot know when the client mispredicted. Beside them it states, by key and
  never by value, which of its transient components that are not replicated the entity has (a
  replicated one is among the rows): a kernel runs on a row only if the row has every component
  it names, so wherever the prediction world builds or corrects a claimed entity from a frame
  that holds such a statement, it leaves the entity with the transient components stated, zeroed
  where it adds one; a frame that holds none leaves its unreplicated ones as they are, and a
  derived component is added there by the system that owns it (D141). Every field agrees by
  canonical bytes; the `Transform` and `Velocity` the physics write-back writes on a dynamic
  body also agree when each scalar is within a stated tolerance (`lactose-predict`'s, two
  millimetres, two thousandths of a quaternion component, two centimetres per second and two
  hundredths of a radian per second by default), because the solver holds state the rows do not
  carry (section 16) and it is not corrected, so a corrected dynamic body differs from the
  server by rounding for as long as it is in contact. The tolerance decides whether a frame
  makes a correction; a correction writes every fact at its tick, the absorbed ones included,
  which is what converges. `Teleported` of a claimed entity is the client's own no-interpolate
  flag and is never compared or corrected. A replicated field a game derives from a
  solver-written value compares by canonical bytes and is corrected when it differs, so a game
  keeps derived replicated state integer. Claimed dynamic bodies in contact with each other are
  corrected every few frames until they settle, so the claim policy claims a thrown object with
  its throw and releases it at rest. Mispredictions against non-predicted entities are expected
  and smoothed visually. Proxies live about half a round trip in the past relative to the local
  player; this is an accepted limitation, and predicted interactions with proxies are presented
  tentatively until confirmed. Corrections are written stamped, so a `Changed` reader in a
  re-simulated tick sees them once (D122).
- Later: forward nearby players' inputs so the prediction world includes them.

### 9.5 Lag compensation

The server keeps a short history of transforms and colliders for players and for NPCs that can
be hit, capped at 250 milliseconds, and validates hits at the tick the client saw.

### 9.6 Scaling path

Multi-process sharding is out of scope. Region ownership and command-based cross-region
interaction keep it possible later. No system may hold a direct reference across regions.

### 9.7 Server capacity

- **Activation.** A region is `Simulated` when any player is within the activation radius,
  measured in regions. Cost per player is the regions they activate times per-region cost;
  co-located players share. Because chunks belong to archetypes, scattered players add rows,
  not mostly-empty chunks. A deployment may pin regions, which are always simulated whatever
  the players' positions; world-global persistent state lives in their resources. A loaded
  region is released only outside a release radius larger than the streaming radius, so a
  player on a border does not thrash the ring.
- **Overload ladder**, climbed when the tick exceeds budget and descended with hysteresis:
  shrink the activation radius; decimate non-player systems in the outer activation ring to
  alternate ticks, selected by tick parity so it stays deterministic; refuse new joins. Server
  time is never slowed. The radius and the join cap are the deployment parameters.
- **Cross-boundary movement.** An entity entering a `Frozen` or unloaded region is frozen at
  the boundary and becomes part of that region's state; its body is a fixed collider until the
  region activates.
- **Server IO** uses `io`'s pool sized to half the cores (`IoConfig::for_platform`, the client's
  default too until measured), separate from the background workers, so region publish latency does
  not grow with player count.
- **Packet parsing** runs on the dedicated network thread (section 5.1).

## 10. Snapshots

Only the client prediction world is ever rolled back. Its `snapshot` captures every column,
the entity table, world and region resources, region modes, seeds and asset tables, the
input-tick table, the relationship index, proxy history and the physics world through
`physics-api::snapshot`, which carries the backend's whole world as a copy the backend makes, each
collider with its shape, and the structural-log entries and changed keys the physics sync has not
yet consumed, so that the sync of a re-simulated tick makes the calls the first run's did. Two
things stay with the world across a restore because a rollback must not undo them: the
prediction-to-network alias table (section 7) and the log of delivered presentation events, which
is what lets re-emission deduplicate and a missing re-emission cancel. Command buffers and
channels are empty at snapshot points by construction: the closing sync point of a run clears
every channel after it applies the last stage's commands, so nothing but rows carries simulation
state across a tick boundary (D126).

The server never rolls back. Save snapshots copy persistent columns only (section 15.3); a
journal checkpoint carries the physics snapshot and the server's binds beside them (section
15.5).

`physics-api` conformance: run ten ticks, snapshot at tick three, restore, re-run, hash equal
at tick ten. The cost is measured in milestone 1 and the prediction world is sized from it.
A slot's cost is the backend's world, every body, collider and contact pair it holds, which
copy-on-write in `predict` cannot share between slots, so copy-on-write snapshots are not
built. The slot holds that world as a copy the backend makes of its own state, each
collider's shape shared with the live world; a backend with nothing cheaper copies its bytes.
A copied slot costs its take and the drop of the slot the ring evicts for it. The pinned
backend's bytes do not fit the budget: as bytes, the fifty claimed bodies of section 22 among
64 fixed colliders and proxies took 140.8 µs to snapshot and 193.0 µs to restore on a quiet
Apple M1, against section 22's 100 µs, and a third of the write was the claimed bodies'
contact pairs, which no bound on rows holds down (D143). The backend's bytes remain what
leaves the process or is compared: the journal checkpoint (section 15.5), the fresh solver's
bytes (section 16) and the physics hash (section 8.3). The bound on what the world holds
stands: at most 64 fixed colliders and proxies together as rows, beside the fifty claimed
bodies. The measurement of the copied slot at that count, taken into a full ring on a quiet
machine, confirms the count or changes it. It is a content rule, and nothing in the engine
enforces it. Milestone 3 designs, with streaming, how cooked static scenery reaches the
prediction world's solver outside the per-tick snapshot, and what replaces the bound.

## 11. Time and pacing

- The timer root (section 5.1) owns tick deadlines, catch-up and frame pacing.
- Time scale is a simulation resource replicated by the server. It changes how many ticks run
  per real second; the tick delta never changes. Pause is time scale zero, server-controlled.
  The scale enters the simulation as a journaled input of the tick; game systems do not
  read it.
- Presentation runs on real time regardless of time scale.
- Catch-up ticks are capped per frame; re-simulation ticks are bounded by the prediction ring's
  depth, sized to the frame's re-simulation budget, and do not count against the cap (D122). A
  client that cannot catch up re-syncs its clock; a server that cannot keep up climbs the
  overload ladder.

## 12. Rendering

### 12.1 Render hardware interface

Thin, explicit, modern, with every claimed common feature verified in the milestone 2 spike.

- device, queues, swapchain
- buffers with device addresses, images, samplers
- one bindless heap; each pass declares the heap slots and the buffers it reaches, and a
  backend that needs residency derives its calls from the declaration
- pipelines from cooked SPIR-V or Metal libraries, with specialisation; driver pipeline caches
  persisted to disk (`VkPipelineCache`, `MTLBinaryArchive`)
- command encoders recordable on any thread
- resource states as a backend-neutral enum; backends emit barriers. On Metal a resource a pass
  declares is made resident on that pass's encoder, which brings it under Metal's hazard
  tracking; aliased transients are untracked, and the backend fences every use of them
- memoryless and transient attachments, and render-pass-local reads (dynamic rendering local
  read on Vulkan, framebuffer fetch on Metal) so the G-buffer stays in tile memory
- timeline semaphores and fences
- indirect draw and dispatch; multi-draw-indirect and indirect-count are reported capabilities
- a null backend for tests and graph unit tests; the server binary links no RHI backend, the
  null one included (section 4)

Apple path: MoltenVK encodes one Metal draw per multi-draw-indirect command on the CPU, so it
cannot carry the GPU-driven path. Milestone 2 spikes MoltenVK and KosmicKrisp and records
indirect-draw behaviour and pipeline creation cost. Native Metal lands in milestone 3.

### 12.2 Shader toolchain

Slang. The cooker emits SPIR-V for Vulkan and MSL plus metallib for Metal, using Slang's Metal
target directly. Mac shaders cook on a macOS runner. Bindless access is abstracted at the Slang
level. Stage-three features that need 64-bit atomics are capability-gated because Apple M1
lacks them.

### 12.3 Render graph

- Passes declare reads and writes with usage. The graph derives order, culls unreachable
  passes, computes transient lifetimes, aliases memory, inserts transitions and assigns queues.
- Compile is cached by a hash of pass list, edges, resource descriptors, imported initial
  states, view count and queue configuration. History resources use a ping-pong primitive. The
  cache is capped.
- Each pass records on a frame-critical worker. Submission order is graph order.
- Async compute is chosen by the graph when a pass has no graphics-queue dependency inside
  its lifetime window.

### 12.4 GPU scene and submission

- Instances, meshes, materials, previous and current transforms live in persistent GPU buffers.
  Slots are allocated on the CPU by the renderer's slot table, since the CPU must know slot
  indices to write updates; the GPU buffers are the storage, not the allocator. Positions are stored as a 32-bit integer grid cell plus a
  32-bit float offset; shaders subtract the camera cell so precision is preserved anywhere on
  the map. `math` has a distinct render-space type with no implicit conversion from simulation
  scalars. Swept culling bounds, the light grid, decals, probes, VFX emitters, cascades, the clipmap
  centre, sky and atmosphere, markers, the audio listener and emitters, GPU picking, debug
  draw and the camera itself all convert through it.
- Render slots are a renderer-owned side table keyed by `StableKey`, freed from the structural
  change log, reconciled after any restore or load, never a column in simulation chunks.
- Draw submission: the cull pass bins surviving clusters per pipeline and compacts their
  triangles into one 32-bit index buffer per pipeline over a global vertex pool; each pipeline
  then issues one indexed indirect draw. Post-transform vertex reuse is preserved, no backend
  needs multi-draw or indirect-count, and it behaves identically everywhere.
- Culling is a compute pass per view reading the latched view: frustum, distance, LOD per
  instance in stage one and per cluster in stage three. Skinned instance bounds are inflated by
  the skeleton's reach. Skinning is a compute pre-pass; foliage and other vertex deformation
  run in the vertex shader under the vertex-deformed axis. Transparency is sorted on the GPU
  per view.
- Stage two: hierarchical-Z occlusion. Stage three: meshlets, mesh shaders and a visibility
  buffer behind a feature flag.

### 12.5 Lighting and materials

Deferred with a G-buffer kept in tile memory on tile-based GPUs through memoryless attachments
and local reads. Motion vectors are a G-buffer target, not a separate pass. Clustered lights
are binned on the GPU in camera-cell-relative space after the latch.

Uber-shader with a material ID and a bindless parameter block. No free-form artist shaders.
Permutation axes: material (alpha-tested, two-sided, skinned, vertex-deformed) by pass type
(G-buffer, depth-only, forward transparent). Those axes give 48 graphics pipelines; the budget
is 96 to leave room for post-processing, compute, UI and debug pipelines. All are created at
load. Passes from `ui`, `terrain` and `debug-ui` reach the graph through the renderer's
`RenderFeature` pass-provider trait, registered by the client binary.

## 13. Presentation

`present-api` is a foundation crate so `game-present` can reach presentation without naming
a backend. It defines mesh, light, decal and camera rig components; VFX and audio request
events; UI view-model resources; a debug-draw buffer; and the presentation snapshot type.

`presentation` runs the per-frame schedule on the snapshot (section 5.3). The camera rig is
presentation, not simulation: its orbit, boom and boom collision are evaluated per frame
against the immutable static collision asset (shared parry shapes also inserted into the
prediction physics world from milestone 3, section 9.4, so presentation never touches a
mutating world) and are not
authoritative, and the input it produces carries absolute
yaw and pitch into the simulation. Pose evaluation for rendering runs here; the simulation uses
the deterministic sim skeleton from `anim` for hit volumes, hand and foot placement, sockets
and ragdoll bodies.

## 14. Assets and streaming

- The cooker emits runtime formats only: cluster-ready mesh pools, block-compressed textures,
  resolved material blocks, compiled shaders, collision, region bundles. A region's cooked
  baseline is an asset of `asset`'s region-baseline kind, named by its content hash (D136).
- Assets are versioned and immutable after publish. A version is the content hash of the
  cooked header and bytes, so an asset ID and version name the same content on every machine
  and after every cook; versions are unordered and a store verifies the hash on load. Inside
  the boundary a region sees the version in its table; outside, readers see the version the
  store's index currently publishes. SILO's index is its mounted packs: a cooked pack reaches
  a running store by being mounted, the version SILO's index names for an asset is the one in
  the pack mounted latest, by the order of the mount calls, among those that hold it, and
  readers see that version once it has loaded (D136).
- A region's version table is resolved outside the boundary as the closure of its asset set
  over declared dependencies, and enters at the input sync point. Boundary worlds receive only
  exact-version access; the catalogue (newest, resolve, publication generation) is given only
  to code outside the boundary. Boundary code reads only asset kinds marked as simulation
  data, whose loaders must be pure byte decoders or live in a boundary crate.
- File IO goes through `io`, a foundation crate with a blocking-pool reference backend and
  io_uring and IOCP optimisations.
- Regions declare asset sets: the cooked baseline's assets, those named by resident entities
  the baseline does not hold (migrated in, or spawned at run time), and a spawnable set the
  baseline declares, so an owned item never loses its mesh by crossing a boundary and a spawn
  finds its assets in the region's table. A spawn that names an asset outside the table is a
  tick-stamped asset failure. Until streaming lands in milestone 3 the set is the baseline's
  alone, and nothing spawns or migrates an entity that names an asset outside it. A region
  enters `Simulated` only when its set is published.
  Streaming leads the player by a hard distance; the server holds a player at a region boundary
  until the client acknowledges readiness, an acknowledgement that enters the server tick as a
  journaled input (milestone 4). A client-only asset failure is a tick-stamped event
  that makes the client request a resync rather than diverge.

## 15. Reflection, serialisation and persistence

`reflect` holds a registry of every component and resource type with explicit
`#[field(id = N)]` IDs, typed reference fields, schema version and persistence policy. Missing
or duplicate IDs fail registration. CI rejects ID reuse.

### 15.1 Persistence policy

| Policy | Meaning | Example |
|---|---|---|
| `persistent` | Saved at full precision | transform, health, velocity, animation phase, timers, quest flags |
| `transient` | Written before read every tick, never saved; hashed. Its presence on an entity is kept: in a save's records (D129) and, for an entity a client predicts, in the frame's statement (section 9.4) | per-tick accumulators, contact lists, the player input sample |
| `derived` | Not saved, not hashed; the system that owns the value rebuilds it after a load, and on a client row the replication stream built when the type is not replicated, and there is no hook for either | physics handles, which the physics sync writes |

Transient safety is structural (section 6): the ECS zeroes transient columns every tick and
the scheduler orders every reader after a declared writer.

### 15.2 World overlay

The save is a set of typed records per region against the cooked baseline, each keyed by
`NetId`: `Spawn`, `SpawnFromBaseline`, `Destroy`, `AddComponent`, `RemoveComponent`,
`SetField`, `OwnershipChange` (a change of owning entity). A cooked entity that moves from
region A to region B becomes `Destroy` in A's set and `SpawnFromBaseline` with its current
fields in B's set, so B loads correctly without A. The baseline content hash is recorded per
region, and the region's seed and persistent region resources are saved beside its records. Region unloading writes the same records, so unloading and saving are one code path,
and the same records are what the server sends a client when the region becomes relevant
(section 9.3). The record format lives in `bundle` so `cook`, `asset`, `persist` and `net`
share it. Each region state has
one record set: a runtime-spawned or migrated entity is a `Spawn` or `SpawnFromBaseline`
carrying whole components; a changed baseline row is `AddComponent` (a whole component, also over
one the row already has), `RemoveComponent`, `SetField` and `OwnershipChange`; `persist`'s diff
writes a changed component as `AddComponent` and never as `SetField` (D129); a destroyed or departed
row is `Destroy` alone. Rebasing onto a patched baseline keeps every runtime-spawned and migrated
entity, drops every record of a cooked entity that is no longer a row, drops `RemoveComponent` and
`SetField` of a component the row no longer has, and returns the dropped records so `persist` can
re-home an entity the patch moved to another region. `persist` then diffs the rebased state against
the patched baseline, so a rebased section is the one record set of its state (D134). An
`AddComponent` the rebase keeps overwrites the patched row's component whole, so a patch to a field
of a component the save touched does not reach that entity until the diff writes field-level records
(D129).

### 15.3 Save operation

At a sync point, copy persistent columns of every loaded region on worker tasks, serialise
through `reflect`, diff into records, compress with zstd, write through `io` to a temporary
file, sync, rename over the slot. World resources and session state (claims, anchors, pending
joins) are not saved; on a load every player joins again.

### 15.4 Container and evolution

Self-describing binary container with engine version, schema hash and a per-region index.
Baseline and overlay documents carry, for each component they hold, its schema version, element
size, field layout (field ID, offset, size) and the digest of its full layout (D129), so the binary
keeps no historical layouts. The digest tells what the field layout alone cannot, a same-size
change; equal versions with different digests are refused, in a cooked baseline and in a save.
Matching schema takes the bulk-copy fast path; mismatched schema decodes by field ID and runs
migrations in version order. A migration is one registered step per type and version read, over the
value's fields by ID; the result is assembled under the current layout and validated; a transient
type is taken by presence and never migrated (D129). After load, change ticks are rebased
(section 6).

### 15.5 Enforcement

- Test: the original performs an in-place reload at tick T; a fresh process loads the save;
  both run K ticks and must hash identically. The fresh process may leave any saved region
  unloaded, as an `Anchored` resume does.
- Test: unload and reload a region; persistent columns match.
- Test: load a save from the previous schema version through its migration.
- Test: the fresh process is also compared against the un-reloaded original after K ticks,
  bit-exact for discrete state and within a stated tolerance for continuous state, so a
  reload is shown not to change gameplay outcomes. This holds because network IDs, predicted
  origins and the server counter are persisted and the loading world is given every bind of the
  save before its first region loads (section 7), so stored references, RNG streams and ordering
  survive a load.

Journal replay (checkpoint plus input journal) is a debug and replay tool, not the save
system. A checkpoint carries the physics snapshot of section 10 as bytes, since the solver
holds state the rows do not carry (section 16) and a replay from a checkpoint without it
hashes equal only until the first contact; a replay from a session's start needs none. It also
carries the server's binds at the checkpoint, since a checkpoint's regions are the loaded ones
alone (D142).

The server owns the save in multiplayer. Player portability between hosts is an open item.

## 16. Physics

Rapier, pinned to an exact version together with its math and SIMD dependencies; any bump is a
protocol and replay break. The pinned `rapier3d-f64` is built from a patched copy under `vendor/`
by the rule of section 8.2: the angular joint limit's `sin_cos` goes through `simba`'s
`ComplexField`, which `libm_force` routes to `libm`, in place of the platform library's; the patch
travels with every bump until upstream carries it.
The vendored Rapier runs the broad phase's deferred tree optimisation on the stepping thread rather
than as a detached job the stepping thread blocks on outside rayon. The step still uses every
thread of the pool (the narrow phase broadcasts to all of them above 64 updating contact pairs), so
the solver pool is the backend's alone: no job on it blocks outside rayon, and it is not built with
`use_current_thread`.
One physics world per server process and one per
client prediction world. Rapier's determinism feature is used together with its parallel solver.
The step is the one sanctioned blocking wait in `task`: a physics stage job calls into the solver's
rayon pool, which is sized to the performance-core count minus the workers reserved for render
recording and non-physics systems, and whose threads get QoS and floating-point state through the
`thread_class` helper. The pool is built by `app` outside the boundary, sized to the frame worker
count minus the reservation and at least one thread, and handed to the backend's constructor; the
backend is handed to `physics-api::setup` through a `SimulationBuilder::setup` closure, and the
step blocks on it inside `block_in_place` with that reservation. The physics stage is a barrier for
systems that depend on physics only; independent systems run alongside on the reserved workers.
Bodies in `Frozen` regions are fixed colliders. Milestone 1 measures the physics phase's share of
the tick; milestone 2 measures the stall it imposes on render recording during the overlap,
since nothing records a frame before `render-graph` exists.

The solver holds state the rows do not carry: warm-start impulses, contact manifolds and sleep
counters. Two worlds with equal rows and different solver state diverge while bodies are in
contact, by rounding for a body on static geometry and by whole outcomes for bodies in contact
with each other, and a client's solver reaches such a state after a correction, a claim of a
body in contact and a rebuild. The backend's results are also a function of the world's row
history (D119): with one awake island and world-wide constraint order, far-away awake bodies
and freed arena slots can change a body's bits in a large contact graph (a 150-body pile;
nothing in scenes of a few or fifty bodies). Bit equality therefore holds between two worlds
only when their row histories and solver states are identical, the server and its replay or a
run and its re-simulation, never the server and a client's prediction world. The backend does
not canonicalise per island: the pinned Rapier cannot, and no solve order carries the solver
state across. The difference is kept out of the comparison instead (section 9.4). An empty
solver is a fresh solver: `physics-api` keeps the bytes the backend wrote at setup; a sync
whose structural log since its last pass, the pending entries a restore carried included,
despawned every body the solver held removes them all, restores those bytes and creates from
the rows by a full diff in ascending key order; a sync whose removals leave the solver empty
restores the bytes and makes the next sync a full diff. So a world reloaded in place, in one
frame or two, has the physics state a fresh process loading the same rows has, which is what
section 15.5's first test needs.

The simulation scalar is 64-bit (D146). Milestone 1 decided it by a precision test: 32-bit
would have been adopted had positional error at the map edge stayed under one millimetre over
a ten-minute standard scene and every SOUR mode passed. The test is read at x = 6 km, since
no document states the map's extent. On upstream 32-bit Rapier the walk scene at
(6000, 0, 6000) differs from the same scene at the origin by 157.75 mm, and at
(12000, 0, 12000) by 120.84 mm; in 64 bits it differs by at most 0.032 mm at either. The
precision half fails, so the SOUR half was not run and no 32-bit branch is built. A switch
would mean `rapier3d` for `rapier3d-f64` and their parry counterparts, a crate change behind
`physics-api`. The 64-bit cost that matters is the client's rollback re-simulation on NEON,
which sets ring depth: the contact solver runs two 64-bit lanes per NEON instruction where
32 bits would run four, and a re-simulated tick of the netcode spike's fifty thrown bodies
costs 387.7 µs on a quiet Apple M1. The rule does not weigh that cost, and a shallow ring is
`predict`'s question when milestone 4 sizes it (section 9.4), not the scalar's. A 32-bit
scalar comes back only with a design that keeps simulation coordinates small, origin rebasing
per region for example, proposed and then run against this rule again; the rule is not run
again at the same coordinates.

`physics-rapier` is inside the boundary. Replacement is a backend swap behind `physics-api`
including `snapshot` and `restore`.

A backend validates every `Transform`, `Velocity`, `RigidBody`, `Shape`, `Collider`, `Joint`,
`BodyDesc` and `PhysicsCommand` it is given by the rules `physics-api` states and exports as
predicates, and adds no rule of its own on them; it also applies the same-body, step-input and
query-parameter rules `physics-api` states, so every backend and every crate that authors a body
refuses one set of values with one message (D135).

## 17. Animation

`anim` is inside the boundary: state machines, phase, events, root motion, and a deterministic
sim skeleton covering sockets, hands, feet, head and ragdoll bodies. Full pose evaluation and
skinning are presentation and renderer work. The phase is an integer fraction of the clip, the sim
pose is root-local and evaluated once per tick for an animated character, root motion is an output
the game's movement system applies, and bone rows are physics bodies the pose drives or, limp, that
drive it, in the server world alone. D128.

## 18. Gameplay authoring

- Rust only. `game-sim` depends on foundation boundary crates; `game-present` on `present-api`
  and foundation crates. `game` is split into the four crates in section 4 from the start to bound incremental build
  cost. Behaviour is data-driven where possible: state machines, ability definitions and spawn
  tables are assets.
- Dev profile: dependencies at opt-level 3, game crates at opt-level 1, line-tables-only debug
  info, split debuginfo, mold or lld on Linux, lld-link on Windows. Registration is through
  generated functions, not life-before-main. Edit-to-playing time is budgeted at thirty seconds
  per platform. Milestone 1 measures its floor: from saving a one-line change to the headless
  binary's report of one tick, on a warm dev-profile build, for an edit in the binary's own
  crate and for one in `app`, read against the slower. From milestone 2 the sketch's edit is in
  `game-sim`, which holds the sketch game (D152). Milestone 5 measures it again on the game
  crates.
- Iteration is state-preserving restart through `reflect`. Dylib reload later if needed.

## 19. Error handling

The policy is in section 3.

- `core::Error` is the taxonomy. Subsystems return `Result`; `app` holds policy. `ErrorKind`
  has stable `u8` codes, so a failure can be journaled as a tick-stamped event.
- `tracing` is also the logging facade. Log targets are the codenames from section 23; a crate
  without one logs at its role name (`io`, `physics`, `app`; D119, D130, D131).
- Tested device-lost recovery: re-upload assets, rebuild the GPU scene from region data, full
  extract.
- Network drop, reconnect, disk full and corrupt save each have a defined path by milestone 4.

## 20. Agent workflow

- Milestone 0 owns everything every track needs: `core`, `math`, `task`, `reflect`,
  `asset-api`, `bundle`, the tagged allocator and the `ecs` facade. Three tracks then run from
  milestone 1: simulation (`ecs` to `sim` to physics to the netcode spike), render (RHI spike
  to Vulkan to graph to GPU scene), assets (`io` to `asset` to `cook`). They join at extract in
  milestone 2 and streaming in milestone 3.
- The week-one `ecs` facade carries the final API of section 6.1 over a naive single-threaded
  implementation so dependent agents never block on the real `ecs`.
- A `task` agent in milestone 0 receives the scope, fork and continuation signatures that
  `ecs` consumes, the DAG shapes and pass thresholds of milestone 0, and the present-thread
  and main-thread lane contract, all written before the task is assigned.
- A crate other crates are written against gets a facade first: every public signature final
  over `todo!()` bodies, the README as its contract, and the decision text for what this
  document left open, reviewed before anything is written against it. Its implementation is
  a lane of its own, and so is each backend of an interface crate (`physics-rapier` behind
  `physics-api`, `asset` behind `asset-api`, the RHI backends behind `rhi-api`, `net` behind
  `net-host`). An interface crate a backend implements is frozen once its first backend
  passes its conformance suite, a null or test backend not counting. `rhi-api` stays unstable
  until the native Metal spike has run. `io` has no `-api`; its backends sit behind a trait
  inside the crate.
- Amending an interface in the same PR as the implementation that needs it is permitted and
  must be called out.
- One crate per lane, or one change whose brief names the crates it crosses. Build what the
  next milestone needs; the interface crate is the place for extension points, not the
  implementation.
- CI budget: a pull request run finishes in fifteen minutes and its cost is recorded per run.
  Miri, loom, the full SOUR matrix and architecture mode run nightly. Bare-metal performance
  measurements run on a developer M-series Mac and an x86 desktop and are checked in as
  results, since CI has no hybrid hardware. Milestone 1's are the Mac's; the desktop's first
  sitting is milestone 2's, and it measures milestones 0 and 1 there too (D151). A measurement
  that proves a milestone criterion is a file under `docs/results` that carries its machine,
  date, commit, detected topology, workers and load; one command, `cargo xtask measure`, writes
  those of milestone 1 and later. A number taken while other work loads the machine steers and
  proves nothing.
- Every interface crate a backend implements ships a conformance suite that each backend
  passes: `physics-api`, `asset-api`, `rhi-api`, and `net-host` for `net`'s transports, due
  with `net` at milestone 4. An interface crate no backend implements, such as `net-api`,
  ships none. RHI conformance runs under lavapipe on Linux CI and
  on a GPU runner nightly.
- Each crate's `README.md` states responsibility, tier, boundary status and unsafe status.
- Agents: `.claude/agents/` defines the standard set and `docs/agents.md` the process. `lane`
  implements one crate or fix round in its own worktree against a brief; `architect` writes a
  crate's facade (final signatures, `todo!()` bodies, the README as the contract, proposed
  decision text) the same way, at the highest effort, for a crate other crates will be
  written against; `planner` writes milestone plans, proposals and the decisions that settle
  them; `reviewer` reads every lane, interface design and plan adversarially before it
  merges, `reviewer-concurrency` reads beside it the first review of code that runs on
  several threads or adds `unsafe` other than an FFI call, `reviewer-gpu` the first review of
  work that records, submits or synchronises GPU work, defines the surface it goes through or
  produces what it runs, and `reviewer-diff` reads each later fix round against the previous
  reviews; `merger` runs a merge and its gates from a brief and commits it. The coordinator
  writes the briefs, decides the findings, pushes, and keeps this document, the decisions log
  and the verification list; a long command that needs no interpretation runs in the
  background, never in a foreground agent turn.
- CI: clippy `-D warnings`, fmt, nextest, SOUR in all applicable modes, Miri nightly on
  `core`, `task` over each deque, `ecs`, `input-host` and `net-host`, `cargo-deny`, the tier,
  boundary, feature and vendor checks, and the line-ending check.
- A package with more than one integration test file builds them as one test binary,
  `integration`, whose root `tests/integration.rs` declares every other file of `tests/` as a
  module (`autotests = false`); a package of one test file keeps its own target name. A file
  whose name holds a codename is declared under a name for what it holds, with `#[path]`. The
  tests of one binary share a process under `cargo test` and `cargo miri test`, so a file that
  installs process-wide state (a `#[global_allocator]`, the tracing subscriber), measures the
  whole process, or asserts on a lock a spawned child could hold stays a binary of its own,
  listed with its reason in `[test_targets]` of `tools/xtask/ci-policy.toml`. `cargo xtask
  boundary` checks that no other package has a second test target, that every file of `tests/`
  that cargo would make a target of is compiled, and that no test module's name holds a
  codename. Each nightly Miri filter must name a module of a target its entry selects that no
  `cfg` naming `miri` compiles out, so a renamed, removed or Miri-gated module fails the check
  (D150).
- Every tracked text file outside `vendor/` has LF line endings, in the index and in every
  checkout made or refreshed under the rule: `.gitattributes` sets `* text=auto eol=lf`,
  which holds whatever `core.autocrlf` says. Code that compares, hashes or parses a tracked
  file therefore reads the same bytes on every host and owes no tolerance of CRLF from a
  checkout. Two exceptions, each a line of `.gitattributes`: `vendor/**` is `-text` and keeps
  upstream's bytes (section 8.2), and a kind of file that is not text is named `binary` in
  the change that adds its first file (D148). `tools/xtask/line-endings.sh` checks git's
  listing of the tree against the rule: `merger` runs it at every merge, and each of CI's
  check jobs, one for each package group, on the three platforms.

## 21. Unsafe policy

| Crate | Why |
|---|---|
| `task` | deques, scope borrows, panic guard, task futures, core-topology queries, the keeper's timed wait on Windows, the benchmark's and tests' process-CPU and task-role calls |
| `ecs` | chunk column storage, and the schedule run's lending of chunk columns, column ticks, row stamps, resources and event lists to concurrent system nodes through raw pointers taken at sync points (`node.rs`); the `#[system]` expansion's call of `unsafe SystemDescriptor::new`, which lands in the invoking crate like the `#[component]` impl and whose proof is in `crates/ecs/SAFETY.md` |
| `io` | platform completion APIs |
| `rhi-vulkan`, `rhi-metal`, `rhi-dx12` | FFI to graphics drivers (`rhi-api` and `rhi-null` are not exempt) |
| `platform` | present-thread QoS and window handles |
| `core` | arenas, the tagged allocator, handle tables, `unsafe impl` for generic plain-data types, the `thread_class` helper that `audio` and `physics-rapier` use instead of their own `unsafe` |
| `tools/cook` | FFI to Slang and compression libraries |

Each carries `SAFETY.md`. Every `unsafe` block has a `// SAFETY:` comment. `[unsafe_crates]` in
`tools/xtask/ci-policy.toml` equals this table, which `cargo xtask boundary` reads: each backticked
name in a row's first cell is a crate, named by its last path segment.

`reflect-macros` contains no `unsafe`, but the expansion of `#[component]`, `#[resource]` and
`#[value]` contains one `unsafe impl` (`bytemuck::CheckedBitPattern` for the struct), which
lands in the invoking crate. The `unsafe_code` lint does not apply to code generated by another
crate's macro, so those crates keep `#![forbid(unsafe_code)]`. The impl is proved by assertions
in the expansion against the final type, never by what the macro parsed;
`crates/reflect-macros/SAFETY.md` carries the argument.

## 22. Milestones

| # | Name | Delivers | Proves |
|---|---|---|---|
| 0 | Skeleton | Workspace, CI matrix, `core` with tagged allocator, `math`, `task`, `reflect`, `asset-api`, `bundle`, tracing, SOUR scaffold, `ecs` facade with the section 6.1 API | Makespan divided by its lower bound (the larger of the duration-weighted critical path and total work divided by worker count) at most 1.25 at P50 and 1.5 at P99 on three DAG shapes (wide-shallow, deep chain, fan-in-fan-out) with log-uniform job durations from 10 µs to 1 ms, with and without two background zstd tasks, on a developer M-series Mac and an x86 desktop. Tick-start latency, from a tick deadline to the timer root resuming, under 35 µs at P99 (D138), under 50 µs on macOS, where work that starts on a core that has idled costs more since macOS 27 (D156), and under 1 ms on Windows, where the finest timed wait a waker can end overshoots by about half a millisecond (D144); wake latency from an idle pool under 35 µs at P99; steal under 1 µs. Latch and sleep-wake pass loom; `task` and `ecs` pass Miri. |
| 1 | World | Sim track: `ecs`, `sim`, `physics-api` plus `physics-rapier`, `anim`, `predict`, `persist` (policy, records, container), `asset-api`, `net-api` and `input` types, headless `app`. Render track: RHI spike. Asset track: `io`, `asset`, `cook` skeleton. | 100k entities simulate a tick in at most 4 ms on 8 performance cores; SOUR passes in workers, repeat, layout and chaos modes. Physics snapshot conformance. Physics phase at most 3 ms for 2,000 awake bodies; prediction-world snapshot at most 100 µs; sim-skeleton evaluation at most 0.5 ms for 200 characters. Netcode spike over a throwaway transport: x86 server, aarch64 client, one predicted character, run as a dynamic capsule and as a kinematic body, fifty claimed dynamic bodies thrown and interacting, and a claim of a body in contact; correction rate under the section 9.4 tolerance, the largest difference the tolerance absorbed per kind, rollback cost per tick and divergence recorded as the baseline that milestone 4's threshold is judged against. Scalar decided by the section 16 rule. Edit-to-playing at most 30 s per platform. |
| 2 | Pixels | `platform`, `rhi-api`, `rhi-null`, `rhi-vulkan`, `render-graph`, minimal `renderer`, Slang toolchain, `present-api`, presentation snapshot and schedule | A triangle through the graph on Windows and macOS. MoltenVK, KosmicKrisp and native Metal spikes recorded, and `rhi-api` frozen or amended on their findings. Overlap with exclusive extract; extract at most 1 ms for 50k dirty rows; remote transform upload at most 1 ms for 5k entities at 144 Hz. lavapipe CI; macOS shader runner. |
| 3 | GPU-driven | GPU scene with cell-plus-offset, cluster submission, culling, deferred tile-local lighting, streaming regions with `persist` overlay against the cooked baseline, `terrain`, pose evaluation, `rhi-metal` | 500k instances through compacted index buffers with one indexed indirect draw per pipeline, at most 2 ms GPU on RTX 3060-class and 4 ms on M1-class at 1440p, on Vulkan and Metal. Regions persist across unload and reload. Skinned characters on terrain. |
| 4 | Network | `net`, interest management, bandwidth prioritisation, reconciliation, resync, late join, client and server `app`, SOUR architecture mode on every PR | Thirty-two simulated clients joining and leaving under injected latency and loss, with the server tick under 16 ms and each joining client converged within 2 s. A recorded server session replayed on aarch64 hashes equal. Solo traversal over static geometry at 100 ms round trip with 1% loss produces fewer than one correction per minute. |
| 5 | Game | `game-sim`, `game-present`, `audio`, `ui`, `nav`, input mapping, `debug-ui` | A playable slice: terrain, climbing, physics objects, co-op interaction, gamepad, sound. |

Milestone 1 exited on 16 November 2026, with milestone 0's carried criteria, on the Apple M1's
files under macOS 14.7.4 (`docs/plans/milestone-1.md`, section 7); the x86 half of both is read
on the desktop in milestone 2 (D151). Milestone 0's two zstd loads are tasks of the background
class, as the engine runs background CPU work (section 5.1). On the M1 they run on the
efficiency cores and barely reach the frame workers. On a hybrid part under Windows the
background class sets thread priority alone, and steering there is an open verification item.
On a part with no efficiency cores they are two low-priority threads beside the frame workers,
the harder case. The desktop's first file says which part it is (D152).

Milestone 1's criteria are read as follows (D139). Each timing criterion has its own
workload. The 100k tick runs on eight frame workers, one for each of eight performance
cores: on a hybrid part, physical cores of the fastest class, a core's hardware threads
counted once; on a part that reports no core types, eight of its physical cores. A tick inside
the budget on fewer performance cores meets the criterion, on the reading that more frame
workers do not lengthen it, and milestone 1's is the M1's on four (D151). The tick excludes the
physics phase, which is measured on its own scene, and the world hash, which SOUR pays at every
step and a shipped binary only when it asks. The composed tick, the same scene with the 2,000
bodies beside it, is read by no criterion: it takes 5,772.5 µs on the M1's four frame workers
and projects to 4,433.1 µs on eight (`docs/results/composed-tick-apple-m1-2026-11-15.md`). The
eight-core figure the desktop gives for the tick is therefore of the tick alone, and no budget
for a frame that steps physics (D152). The prediction-world snapshot is the world
snapshot and the physics snapshot of one ring slot, with the drop of the world snapshot and
the physics snapshot the ring evicts for it, on the netcode spike's thrown scene among the
fixed colliders and proxies section 10 admits; the slot's record of the claimed entities and
its restore are recorded beside it (D143). The netcode spike runs in one process, over a
seeded link that delays and drops packets, jitters them by less than a tick and reorders them
within one, with a scripted client clock that drifts, wakes late and steps. Its x86 server
and aarch64 client are SOUR's architecture mode over what both ends record, each world's hash
at every tick and a hash of every packet sent: records equal on linux-x64, windows-x64 and
macos-arm64 mean an x86 server and an aarch64 client exchange those bytes tick for tick. A
difference between platforms is a finding and calls for a run across two machines. The
scalar's rule is read at x = 6 km, D59's figure, since no document states the map's extent,
and its SOUR half is run only if its precision half passes, which it did not (section 16,
D146).

### 22.1 Decided later, by milestone

| Decision | By |
|---|---|
| Simulation scalar precision | 1 |
| Allocator tagging scheme for budgets | 0 |
| Build-time mitigation if over budget: at milestone 1 only if the floor of section 18 is over thirty seconds, and asked again on the game crates | 1 and 5 |
| Crash reporting: minidumps and symbol upload | 2 |
| Memory budgets per region, GPU, streaming, including the baseline-resident cost of overlay diffing | 3 |
| Apple interim path (MoltenVK or KosmicKrisp) until native Metal | 2 |
| Audio thread model and event bridge | 5 |
| Navigation and AI behaviour model | 5 |
| UI framework and text rendering | 5 |
| Localisation | 5 |
| Player profile portability between hosts | 4 |
| Network drop, reconnect, disk-full and corrupt-save paths | 4 |

## 23. Glossary of names

Codenames label subsystems on binaries, tools, test and bench targets, log targets, CI jobs,
README titles and doc headings. Crate names, types, functions, modules, constants and
statics stay descriptive, in test code as elsewhere (section 3).

| Name | Crate or surface | Role |
|---|---|---|
| LACTOSE | workspace | The engine. Lock-free Asynchronous Chunked Task-Oriented Simulation Engine. |
| CURD | `lactose-sim` | Deterministic simulation core. Concurrent Unified Runtime, Deterministic. |
| WHISK | `lactose-task` | Work-stealing task executor. |
| CASEIN | `lactose-ecs` | Chunked archetype storage and scheduler. |
| COW | `lactose-predict` | Copy-on-write snapshots. Not built: a slot's cost is the physics backend's world, which copy-on-write cannot share between slots (section 10). |
| GOUDA | `lactose-render-graph` | Render graph. Graph Of Unified Device Access. |
| CHEDDAR | `lactose-renderer` | Persistent GPU scene. |
| BRIE | `lactose-renderer` | Bindless resource heap. |
| CREAM | `lactose-renderer` | GPU culling. Culling Regions Encoded As Masks. |
| LATTE | `lactose-renderer` | Late-latched orbit camera. |
| FROTH | `lactose-renderer` | VFX particles, outside the determinism boundary. |
| QUARK | `lactose-physics-rapier` | Physics backend. |
| MILKMAN | `lactose-net` | Replication and interest management. Log target `milkman`. |
| SKIM | `lactose-persist` | Save system. Save Kit and Instance Manager. |
| CARTON | `lactose-persist` | Save container format. |
| SILO | `lactose-asset` | Immutable asset store and streaming. |
| RICOTTA | `tools/cook`, binary `ricotta` | Asset cooker. Ricotta means recooked. |
| PAIL | `lactose-platform` | Platform Abstraction and Input Layer. |
| SOUR | CI job `sour` | Determinism regression harness. |
