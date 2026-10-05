# LACTOSE Architecture

LACTOSE: Lock-free Asynchronous Chunked Task-Oriented Simulation Engine.
Its deterministic simulation core is CURD: Concurrent Unified Runtime, Deterministic.
LACTOSE sets the CURD.

Status: design baseline, pre-implementation. Every section is a decision, not a suggestion.
Change a decision by editing this file in the same change that implements it.

## 1. Goals

In priority order. When two conflict, the earlier one wins.

1. **Short critical path.** Minimise the longest dependency chain in a frame. Utilisation is a
   consequence, never a target. A 16-core machine at 60% with clean per-core working sets beats
   one at 95% bouncing cache lines.
2. **Deterministic simulation.** Same inputs produce the same world state regardless of thread
   count, scheduling order, or run. Required for networking, replays and testing.
3. **No threading knowledge required of gameplay programmers.** They never write a lock, spawn a
   thread, or choose execution order. They declare what data a system reads and writes.
4. **Cache-coherent data layout.** Structure-of-arrays in fixed-size chunks everywhere the hot
   path runs.
5. **No coordinator thread.** The scheduler is work-stealing. The OS main thread pumps events and
   presents frames and does nothing else.
6. **Modular, interface-first.** Every subsystem is an interface crate plus one or more
   implementation crates. Agents build against interfaces in parallel.

Non-goals for now: an editor, mobile, web, cross-architecture determinism, a scripting language.

## 2. Decisions

| Area | Decision |
|---|---|
| Language | Rust, stable toolchain, workspace of crates. No Bevy. |
| Reference game | Open-world action-adventure in the style of Breath of the Wild, 4-player networked co-op. |
| Simulation tick | 60 Hz fixed. Render uncapped with interpolation. |
| Netcode | Dedicated server, authoritative. Deterministic sim. Local-player prediction with rollback over a bounded region. Snapshot interpolation for everything else. |
| Determinism scope | Same binary, same architecture. Math crate designed so cross-architecture can be enabled later. |
| World simulation | Server simulates active regions around players. Clients simulate only their predicted region. |
| Physics | Rapier behind `physics-api`, replaceable. |
| Graphics | Thin render hardware interface (RHI). Vulkan 1.3 backend first (Windows, macOS via MoltenVK). Native Metal second. DX12 when Xbox matters. Bindless, indirect-count, no mesh shaders in stage one. |
| Rendering | Render graph compiled from declared pass access. GPU-driven: persistent GPU scene, compute culling, indirect draws. |
| Latency | Simulation N+1 overlaps render N. Late-latched input for camera and view. |
| Concurrency primitive | Stackless async tasks on a custom executor. No tokio. No stackful fibers. |
| Gameplay authoring | Rust only. Iteration via state-preserving restart. Dylib hot reload later if needed. |
| Editor | Deferred. Reflection and component registry in scope from day one because serialisation and replication need them. |
| Saves | Share the serialisation core and snapshot primitive with netcode, never the replication stream. World overlay against the cooked baseline, server-owned in multiplayer, schema-versioned with migrations. |
| Platforms now | Windows, macOS. Linux for CI. Consoles when Rust toolchains exist. |

## 3. Principles that bind every crate

**Declare access, never order.** A system declares the components and resources it reads and
writes. The scheduler derives concurrency from declarations and ordering from a stable
topological sort. Two systems that conflict and have no explicit ordering are a build error,
not a warning.

**No locks on the hot path.** Per-frame work uses chunks, command buffers and atomic counters.
A `Mutex` inside a system is a review rejection. Locks are permitted in setup, teardown, and
IO completion handling.

**Unsafe is quarantined.** `#![forbid(unsafe_code)]` in every crate except those listed in
section 15. Those crates carry a `SAFETY.md` explaining each invariant.

**Interface before implementation.** Each subsystem ships `<name>-api` with traits and plain
data types, and `<name>` or `<name>-<backend>` implementing them. Callers depend on `-api`
only. The exception is `ecs`, whose query API must monomorphise and therefore cannot sit
behind a trait object.

**Determinism boundary is explicit.** Section 7 lists what is inside. Code inside the boundary
follows the determinism rules. Code outside may use wall-clock time, thread-local state and
platform float libraries.

**Things you run get names, things you import get descriptions.** Crates are named for what
they do, prefixed `lactose-`. Codenames from the glossary (section 19) appear on binaries,
tools, log targets, CI jobs, README titles and doc headings, never on crate names, types or
functions. The glossary is loaded into every agent session through `CLAUDE.md`.

**Everything is data.** Components are plain `#[repr(C)]` structs registered with the
reflection registry. If it cannot be reflected it cannot be serialised, replicated, snapshotted
or shown in a future editor, so it does not go in a component.

## 4. Workspace layout

Every crate is published as `lactose-<name>`. Codenames in brackets are the public identity
of that subsystem and appear only where section 3 allows.

```
crates/
  core/            ids, handles, fixed-seed hashing, arenas, time types, error types
  math/            deterministic f32 math: vectors, quaternions, transforms, pure-Rust transcendentals
  task-api/        Task, Scope, Counter, spawn/await contract
  task/            [WHISK] custom executor: one worker per core, work-stealing deques, main-thread lane
  ecs/             [CASEIN] entities, archetypes, 16 KiB chunks, change ticks, queries, scheduler, command buffers
  reflect/         type registry, component registry, field metadata, serde bridge
  asset-api/       AssetId, Handle<T>, loader and processor traits, streaming budget types
  asset/           [SILO] async loading, dependency tracking, immutable publication, hot reload
  io/              platform async file IO (io_uring on Linux, IOCP on Windows, kqueue on macOS)
  rhi-api/         device, queue, buffer, image, pipeline, bindless heap, encoder, resource states
  rhi-vulkan/      ash-based backend
  rhi-metal/       later
  rhi-dx12/        later
  render-graph/    [GOUDA] pass declaration, graph compile, barriers, transient aliasing, parallel record
  renderer/        GPU scene [CHEDDAR], bindless heap [BRIE], culling [CREAM], late-latch [LATTE], particles [FROTH]
  physics-api/     colliders, bodies, queries, events
  physics-rapier/  [QUARK] Rapier backend
  net-api/         transport, replication, prediction and snapshot contracts
  net/             replication [MILKMAN], interest management, prediction region, rollback
  sim/             [CURD] deterministic world: regions, tick loop, snapshot ring [COW later], determinism hashing
  persist/         [SKIM] persistence policy, stable IDs, world overlay, save container [CARTON], migrations
  platform/        [PAIL] winit window, input, swapchain surface, main-thread pump
  debug-ui/        egui overlay, profiler view, determinism inspector
  app/             assembles runtime: client, server, headless
game/
  <game crates>    gameplay systems, content, built only on *-api crates plus ecs, sim, math
tools/
  cook/            [RICOTTA] offline asset cooker: meshes, textures, materials, scenes to runtime format
docs/
```

Dependency direction is strictly downward in this list. `game` never depends on an
implementation crate. `app` is the only crate that names backends.

## 5. Runtime model

### 5.1 Task system

- **One pool.** Worker threads equal to physical cores minus one. The remaining core runs the
  platform thread. No separate IO or async-compute pools. Priority is a field on the task, not a
  pool.
- **Work stealing.** Each worker owns a deque. Push and pop at the owner end, steal from the
  other end. Idle workers spin briefly, then park on a futex.
- **Stackless tasks.** A task is a Rust `Future`. Waiting on a counter, an IO completion or a
  GPU fence is an `.await` that returns the worker to the pool. Tasks never block a thread.
- **Scoped spawning.** `Scope::spawn` borrows from the parent's stack. A scope cannot complete
  until its children do. This gives structured concurrency and lets chunk iteration borrow
  component slices without lifetime erasure.
- **Main-thread lane.** A small queue for work that the OS requires on the main thread: window
  events, swapchain present, platform callbacks. Nothing else lands there.
- **No thread-local state inside the determinism boundary.** Task-local scratch is handed in
  as a parameter, keyed by chunk index.

### 5.2 Frame pipeline

```
tick N+1 simulation  ────────────────────────┐
                                              ├─ overlap
frame N  extract → cull → record → submit ────┘
         late-latch input just before record
```

- Simulation runs at a fixed 60 Hz on its own cadence. Render runs as fast as the display
  allows and interpolates between the last two simulation states.
- Extract is a dirty-range upload to the GPU scene, driven by change ticks, not a copy of the
  world. The render side holds no CPU mirror of simulation data beyond what late-latching needs.
- Late-latch samples input after simulation has committed and patches the camera and view
  matrices immediately before command recording. Gameplay sees the input one tick later.
- A frame has exactly four sync points: end of simulation tick, extract, graph compile, and
  present. Everything between them is a DAG of tasks.

### 5.3 Scheduling

- The scheduler builds a DAG per schedule at startup from declared access. It caches the DAG
  and rebuilds only when systems are added or removed.
- Within a sync-point window, independent systems run concurrently. Conflicting systems are
  ordered by explicit `before`/`after` constraints, and a missing constraint between two
  conflicting systems fails the build of the schedule.
- Order among conflicting systems is resolved by a stable topological sort keyed on system
  registration order, so the result is identical across runs and thread counts.
- Each system iterates chunks in parallel through `Scope::spawn`. Per-entity work is
  independent by construction. Reductions use per-chunk partials merged in chunk index order.
- Structural changes go into per-system, per-chunk command buffers applied at the next sync
  point in deterministic order.

## 6. ECS data model

- **Archetypes** group entities by exact component set. **Chunks** are 16 KiB slabs holding a
  fixed number of entities of one archetype, with each component as a contiguous column. One
  chunk fits in L1 or L2 on every target.
- **Change ticks** per column per chunk. A system writes a column's tick when it takes a
  mutable borrow of that chunk. Replication, extract and snapshotting read ticks, never data,
  to decide what to touch.
- **Entity IDs** are 64-bit: 32-bit index, 24-bit generation, 8-bit region. IDs inside the
  determinism boundary are allocated from a per-region counter that is part of the region's
  state and therefore part of its snapshot. Predicted spawns on a client use an ID derived from
  tick and player so the server can match them.
- **Regions** are the unit of simulation, streaming, snapshotting and relevance. A region is
  a set of archetypes and chunks plus region-local resources such as an RNG stream and a
  spawn counter. Regions never share chunks.
- **Relationships** such as parent-child are components holding entity IDs with a reverse
  index maintained by the command-buffer apply step, not by user code.
- **Resources** are singletons with declared access, scheduled exactly like components.
- **Queries** are generic over component tuples and monomorphise. This is why `ecs` has no
  `-api` split.

## 7. Determinism

### 7.1 Inside the boundary

Everything in `sim`, `physics-api` consumers, `net` prediction, and all `game` simulation
systems. Also `math`, `ecs`, `core`.

### 7.2 Outside the boundary

Rendering, audio, particles, animation blending, UI, asset IO timing, debug overlay.

### 7.3 Rules inside the boundary

- Floating point only through `math`. Transcendentals come from the pure-Rust `libm`
  implementation, never the platform library. No `fast-math`. FMA contraction is disabled by
  compiler flags in the workspace profile.
- No wall-clock time. Tick index and fixed delta only.
- No thread-local state. No global mutable state.
- Hash maps use a fixed-seed hasher. Iteration order over a map is never observable; sort
  before consuming when order matters.
- RNG is a region-local counter-based stream seeded from the region seed and tick.
- Reductions merge in chunk index order.
- Asset availability is an input to simulation, not an event. A region does not simulate until
  its required assets are published.

### 7.4 Enforcement

- `sim` exposes `world_hash(region) -> u64` computing a stable hash over all chunk data and
  region resources.
- The CI harness, named SOUR, runs every simulation test twice, with 1 worker and with N
  workers, and fails on hash mismatch. It also runs each test twice with the same worker count
  and fails on mismatch. A mismatch has gone sour.
- A `determinism` lint crate denies `std::time`, `std::collections::HashMap` with the default
  hasher, `thread_local!` and `f32::sin` family inside boundary crates.

## 8. Networking

- **Topology.** Dedicated server, authoritative. Clients send inputs with tick stamps. Server
  simulates and replicates state.
- **Replication.** Per client, the server tracks the last acknowledged tick. Each outgoing
  packet carries, for entities in that client's relevance set, only columns whose change tick
  is newer than the acknowledged tick. Relevance is region membership plus a distance band.
  Send rate 20 to 30 Hz, interpolated on the client.
- **Prediction region.** The client simulates ahead for the local player and anything the
  player can touch within the rollback window. This region is a `sim` region like any other.
- **Rollback.** The predicted region is snapshotted every tick by copying its chunk set, entity
  allocator and region resources into a ring buffer of 12 entries. On server correction the
  client restores the snapshot for the corrected tick, applies the correction and re-simulates
  to the present. Copy-on-write chunks are a later optimisation behind the same
  `snapshot(region)` API; not built until profiling demands it.
- **Lag compensation.** Server keeps a short history of player transforms and colliders only,
  used to validate hits at the tick the client saw.
- **Later.** Forward other players' inputs so clients predict nearby players too. Widens the
  predicted region, changes nothing else.

## 9. Rendering

### 9.1 Render hardware interface

Thin, explicit, modern. It abstracts what Vulkan 1.3, DX12 and Metal 3 share:

- device, queues, swapchain
- buffers with device addresses, images, samplers
- one bindless heap for textures and buffers
- pipelines from pre-compiled SPIR-V or equivalent, with specialisation
- command encoders recordable on any thread
- resource states as a backend-neutral enum; the graph emits transitions, the backend emits
  barriers
- timeline semaphores and fences
- indirect draw and dispatch with count buffers; backends without count support report it and
  the renderer falls back to zero-instance draws

Nothing above the RHI knows which backend is active. The RHI is the one place where a boxed
trait object per subsystem is the right design.

### 9.2 Render graph

- Passes declare reads and writes of named resources with usage. The graph derives order,
  culls unreachable passes, computes transient resource lifetimes, aliases memory, inserts
  transitions and assigns queues.
- Compile is cached by graph topology hash. A steady-state frame skips compile.
- Each pass records on a worker through the task system. Submission order is the graph order.
- Async compute is chosen by the graph when a pass has no graphics-queue dependency inside its
  lifetime window.

### 9.3 GPU-driven pipeline

Stage one, in scope now:

- **GPU scene.** Persistent buffers for instances, meshes, materials, transforms. Allocated
  through a GPU-side free-list allocator. Uploaded by dirty range from ECS change ticks.
- **Culling.** Compute pass per view: frustum, distance, LOD selection. Writes compacted
  instance lists and indirect draw commands with a count.
- **Draw.** Multi-draw indirect per pipeline. Bindless material and texture access by index.
- **Skinning and deformation** as compute pre-passes writing into the same vertex pools.
- **Transparency** sorted on the GPU per view.

Stage two: hierarchical-Z two-pass occlusion culling. Stage three: meshlets and mesh shaders
behind a feature flag, only when content density demands it.

### 9.4 Material model

Uber-shader with a material ID and a bindless parameter block. No free-form artist shaders.
Shader permutations are pre-compiled by the cooker. This is a hard constraint of GPU-driven
batching and is decided now so the asset format can assume it.

## 10. Assets and streaming

- **Cook step.** `tools/cook` converts source assets into the runtime format: mesh pools ready
  for the GPU scene, block-compressed textures, resolved material parameter blocks, scenes as
  region bundles. The runtime never parses a source format.
- **Immutable after publish.** An asset is built on worker tasks and published by an atomic
  pointer swap. Readers hold a `Handle<T>` and dereference without locks. Hot reload publishes
  a new version; old versions are freed when their last handle drops.
- **Async IO.** `io` wraps io_uring, IOCP and kqueue behind one completion-based API. A read
  is a task `.await`. Decompression and parsing are ordinary tasks.
- **Streaming.** Regions declare their asset sets. A streaming budget task loads regions by
  predicted player movement. A region enters simulation only when its set is published, which
  keeps asset timing outside the determinism boundary.

## 11. Reflection and serialisation

- `reflect` holds a registry of every component and resource type: name, stable ID, size,
  alignment, fields, serde hooks, schema version and persistence policy. Registration is by
  derive macro. A type without an explicit persistence policy fails registration.
- Scenes serialise as region bundles: for each archetype, columns written in bulk. Loading is
  a bulk copy into chunks. Text form exists for authoring and diffing; binary for cooked
  output.
- Replication, snapshots and the future editor all read the same registry. There is one source
  of truth for what a component is.

## 12. Save games and persistence

Saves share the serialisation core and the snapshot primitive with netcode. They do not share
the replication stream, which is a bandwidth policy with the wrong baseline, the wrong
precision and the wrong coverage for persistence.

### 12.1 Persistence policy

Every component and resource declares one of:

| Policy | Meaning | Example |
|---|---|---|
| `persistent` | Written to saves at full precision | transform, health, inventory, quest flags |
| `transient` | Dropped on save, default-initialised on load | velocity caches, animation phase |
| `derived` | Dropped on save, rebuilt by an `on_load` system | physics handles, render instance slots |

### 12.2 Stable identity

Persistent entities carry a `PersistentId`, a 64-bit value assigned at cook time for placed
entities and at spawn time for runtime ones, independent of the runtime entity ID. References
between persistent entities serialise as `PersistentId`. Asset references serialise as
`AssetId`.

### 12.3 World overlay

The save is a diff against the cooked world, kept per region.

- When a region unloads, `persist` serialises its persistent columns, diffs them against the
  cooked baseline for that region, and writes the diff into the overlay.
- When a region loads, the cooked baseline is loaded and the overlay diff is applied before
  the region enters simulation.
- A save file is: the overlay for every region that has ever changed, active regions in full,
  global persistent resources, and the player entities.

Region unloading and saving are therefore the same code path, and saves stay small.

### 12.4 Save operation

1. At a tick boundary, snapshot every active region through the same `snapshot(region)` call
   that rollback uses. This is a chunk copy and takes microseconds.
2. On worker tasks, serialise the snapshot through `reflect`, diff against the cooked
   baseline, compress with zstd.
3. Write through `io` to a temporary file, sync, rename over the slot.

The frame never stalls. The save is consistent at exactly one tick. A crash during a write
leaves the previous slot intact.

### 12.5 Container and schema evolution

- Self-describing binary container with a header holding the engine version, the registry
  schema hash, and a per-region index.
- Matching schema hash takes the fast path: bulk copy into chunks.
- Mismatched schema decodes field by field by name and stable field ID, applies registered
  per-type migrations in version order, then bulk-copies.
- Removing a persistent component or field requires a migration. CI checks that every
  registered type's schema version increments when its fields change.

### 12.6 Multiplayer

The server owns the world save. Player entities are part of it. Clients hold no authoritative
persistent state. Whether a player's character can be carried between hosts is a product
decision tracked as an open item.

### 12.7 Enforcement

- Test: save at tick T, load into a fresh process, `world_hash` equals the pre-save hash for
  every active region. Any component missing a correct policy shows up as a mismatch.
- Test: unload a region, reload it, `world_hash` matches.
- Test: load a save written by the previous schema version through its migration.

### 12.8 Journal replay

A deterministic sim can restore state from a checkpoint plus the input journal. This is kept as
a debug and replay tool. It is not the save system, because load time grows with play time and
any binary change invalidates the journal.

## 13. Physics

Rapier through `physics-api`. The API exposes bodies, colliders, joints, scene queries and
contact events as plain data. Rapier runs inside the determinism boundary with its
deterministic feature enabled. Its internal threading is disabled; the engine's task system
drives island solving through the API where Rapier allows it. Replacement with an in-house
solver is a backend swap.

## 14. Gameplay authoring

- Rust only. Gameplay systems live in `game` crates, depend only on `-api` crates, `ecs`,
  `sim` and `math`.
- Iteration loop: edit, incremental build, state-preserving restart. The running app serialises
  its world through `reflect`, the new binary deserialises it. This reuses machinery the engine
  needs anyway and avoids the ABI hazards of dynamic library reloading in Rust. Dylib hot
  reload may be added later behind a `#[repr(C)]` system-registration boundary.
- Behaviour is data-driven where possible: state machines, ability definitions and spawn
  tables are assets, not code.

## 15. Unsafe policy

Crates permitted to contain `unsafe`:

| Crate | Why |
|---|---|
| `task` | work-stealing deques, scoped borrows across spawned tasks |
| `ecs` | chunk column storage, typed views over raw memory |
| `io` | platform completion APIs |
| `rhi-*` | FFI to graphics drivers |
| `core` | arenas and handle tables |

Each carries `SAFETY.md`. Every `unsafe` block has a `// SAFETY:` comment naming the invariant.
`task` and `ecs` run under Miri in CI for their unit tests. `task` additionally runs its
scheduler model under `loom`.

## 16. Agent workflow

This engine is built by AI agent teams working in parallel. The structure exists to make that
safe.

- **One crate per task.** An agent is assigned a crate and its `-api`. It does not edit other
  crates except to add a dependency on a published `-api`.
- **Interface first.** The `-api` crate is written, reviewed and merged before any
  implementation starts. Other agents build against it immediately with a stub backend.
- **Tests are the contract.** Every `-api` ships a conformance test suite that any
  implementation must pass. Determinism tests are mandatory for anything inside the boundary.
- **No speculative generality.** Build what the next milestone needs. The interface crate is
  the place for extension points, not the implementation.
- **Docs live with code.** Each crate has a `README.md` stating its responsibility, its
  dependencies, what is inside or outside the determinism boundary, and its unsafe status.
- **CI gates.** `cargo clippy -D warnings`, `cargo fmt --check`, `cargo test`, determinism
  double-run, Miri on unsafe crates, and a dependency-direction check that fails if a crate
  depends on an implementation crate it should not.

## 17. Milestones

Each milestone ends with a runnable binary and a demo.

| # | Name | Delivers | Proves |
|---|---|---|---|
| 0 | Skeleton | Workspace, CI, `core`, `math`, `task-api`, `task`, `reflect` | Task system saturates cores on a synthetic DAG with no coordinator. Determinism lint passes. |
| 1 | World | `ecs`, `sim`, `persist`, `physics-api`, `physics-rapier`, headless `app` | 100k entities, parallel systems, `world_hash` identical across 1 and N workers. Snapshot and restore of a region. Save, load into a fresh process, hash equal. |
| 2 | Pixels | `platform`, `rhi-api`, `rhi-vulkan`, `render-graph`, minimal `renderer` | A triangle through the graph on Windows and macOS. Pipelined render overlapping simulation. |
| 3 | GPU-driven | GPU scene, culling, indirect draws, `asset-api`, `asset`, `io`, `cook` | 500k instances culled and drawn with a handful of CPU submissions. Streaming regions with world overlay persisting across unload and reload. |
| 4 | Network | `net-api`, `net`, client and server `app` | Four clients, prediction and rollback over a region, delta replication. Chaos test with injected latency and loss. |
| 5 | Game | `game` crates, `debug-ui` | A playable slice: terrain, climbing, physics objects, co-op interaction. |

Milestone 0 starts with `task` because every later crate depends on it and because its
behaviour under contention is the hardest thing to retrofit.

## 18. Open items

Tracked here until resolved in a milestone.

- MoltenVK support for draw-indirect-count and descriptor indexing limits: verify in
  milestone 2 before designing the culling output format.
- Rapier's determinism under the engine's task system rather than its own threading: verify in
  milestone 1.
- Whether chunk size should vary per archetype or stay fixed at 16 KiB: measure in milestone 1.
- Console toolchains: no action until Rust is supported on the target platform.
- Player profile portability between hosts: product decision, affects whether player entities
  get a separate save container.

## 19. Glossary of names

Codenames label subsystems on binaries, tools, log targets, CI jobs, README titles and doc
headings. Crate names, types and functions stay descriptive.

| Name | Crate or surface | Role |
|---|---|---|
| LACTOSE | workspace | The engine. Lock-free Asynchronous Chunked Task-Oriented Simulation Engine. |
| CURD | `lactose-sim` | Deterministic simulation core. Concurrent Unified Runtime, Deterministic. |
| WHISK | `lactose-task` | Work-stealing task executor. |
| CASEIN | `lactose-ecs` | Chunked archetype storage and scheduler. |
| COW | `lactose-sim` | Copy-on-write chunk snapshots, a later optimisation. |
| GOUDA | `lactose-render-graph` | Render graph. Graph Of Unified Device Access. |
| CHEDDAR | `lactose-renderer` | Persistent GPU scene. |
| BRIE | `lactose-renderer` | Bindless resource heap. |
| CREAM | `lactose-renderer` | GPU culling. Culling Regions Encoded As Masks. |
| LATTE | `lactose-renderer` | Late-latched transform and eye. |
| FROTH | `lactose-renderer` | Particles and VFX, outside the determinism boundary. |
| QUARK | `lactose-physics-rapier` | Physics backend. |
| MILKMAN | `lactose-net` | Delta replication. Log target `milkman`. |
| SKIM | `lactose-persist` | Save system. Save Kit and Instance Manager. |
| CARTON | `lactose-persist` | Save container format. |
| SILO | `lactose-asset` | Immutable asset store and streaming. |
| RICOTTA | `tools/cook`, binary `ricotta` | Asset cooker. Ricotta means recooked. |
| PAIL | `lactose-platform` | Platform Abstraction and Input Layer. |
| SOUR | CI job `sour` | Determinism regression harness. |
