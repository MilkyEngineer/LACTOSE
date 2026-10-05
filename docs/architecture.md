# LACTOSE Architecture

LACTOSE: Lock-free Asynchronous Chunked Task-Oriented Simulation Engine.
Its deterministic simulation core is CURD: Concurrent Unified Runtime, Deterministic.
LACTOSE sets the CURD.

Status: design baseline, revision 2, after adversarial review. Every section is a decision, not
a suggestion. Change a decision by editing this file in the same change that implements it.

## 1. Goals

In priority order. When two conflict, the earlier one wins.

1. **Short critical path.** Minimise the longest dependency chain in a frame. Utilisation is a
   consequence, never a target. A 16-core machine at 60% with clean per-core working sets beats
   one at 95% bouncing cache lines.
2. **Deterministic simulation across threads and architectures.** Same inputs produce the same
   world state regardless of thread count, scheduling order, chunk layout, run, or whether the
   host is x86-64 or aarch64. The default shipping configuration is an Apple Silicon client
   predicting against a Linux x86-64 server, so cross-architecture determinism is a product
   requirement, not an option.
3. **No threading knowledge required of gameplay programmers.** They never write a lock, spawn a
   thread, or choose execution order. They declare what data a system reads and writes.
4. **Cache-coherent data layout.** Structure-of-arrays in fixed-size chunks everywhere the hot
   path runs.
5. **No coordinator thread.** The scheduler is work-stealing. The OS main thread pumps events and
   presents frames and does nothing else.
6. **Modular, interface-first where the interface is knowable.** Subsystems with known
   behaviour and multiple consumers get an interface crate before an implementation. Subsystems
   whose interface depends on a backend are built as a vertical slice first.

Non-goals for now: an editor, mobile, web, Intel Macs, a scripting language.

## 2. Decisions

| Area | Decision |
|---|---|
| Language | Rust, stable toolchain, workspace of crates. No Bevy. |
| Reference game | Open-world action-adventure in the style of Breath of the Wild, 4-player networked co-op. |
| Simulation tick | 60 Hz fixed. Render uncapped with interpolation. |
| Netcode | Dedicated server, authoritative. Deterministic sim. Local-player prediction with rollback over a bounded region. Snapshot interpolation for everything else. |
| Determinism scope | Cross-architecture: x86-64 and aarch64 must hash identically. Tested on both in CI. |
| World coordinates | 64-bit float positions in simulation. Camera-relative 32-bit positions produced at extract for rendering. Replication quantises relative to region origin. |
| World simulation | Server simulates active areas around players. Clients simulate only their predicted area. |
| Physics | Rapier, 64-bit build, behind `physics-api`. One physics world per active area. Serial per world. |
| Graphics | Thin render hardware interface (RHI). Vulkan 1.3 backend first (Windows, Apple Silicon macOS via MoltenVK). Native Metal second. DX12 when Xbox matters. |
| Shaders | Slang, compiled offline to SPIR-V by the cooker. Metal shader libraries produced at cook time from SPIR-V. No runtime shader compilation. |
| Lighting | Deferred with a G-buffer in stage one. Visibility buffer when meshlets arrive in stage three. |
| Rendering | Render graph compiled from declared pass access. GPU-driven: persistent GPU scene, compute culling, indirect draws. Zero-instance draws are the primary path, indirect-count an optimisation. |
| Latency | Simulation N+1 overlaps render N after a serial extract phase. Late-latched view rotation only. |
| Concurrency primitives | Blocking fork-join for chunk iteration inside a system. `'static` stackless futures for IO, GPU and asset work. No stackful fibers. No tokio. |
| Component data | Padding-free plain data, no heap ownership. Variable-length data is child entities or fixed-capacity inline arrays. |
| Gameplay authoring | Rust only. Iteration via state-preserving restart. Dylib hot reload later if needed. |
| Editor | Deferred. Reflection and component registry in scope from day one. |
| Saves | Share the serialisation core and snapshot primitive with netcode, never the replication stream. World overlay against the cooked baseline, server-owned in multiplayer, schema-versioned with explicit field IDs. |
| Platforms | Windows and macOS (Apple Silicon) clients. Linux dedicated server. Linux, macOS and Windows CI, including an aarch64 runner. Consoles when Rust toolchains exist. |

## 3. Principles that bind every crate

**Declare access, never order.** A system declares the components and resources it reads and
writes. The scheduler derives concurrency from declarations and ordering from a stable
topological sort. Systems belong to system sets; ordering constraints are written between sets
and only between individual systems when sets are insufficient. Two systems that conflict and
have no ordering at any level are a schedule build error, not a warning.

**No locks on the hot path.** Per-frame work uses chunks, command buffers and atomic counters.
A `Mutex` inside a system is a review rejection. Locks are permitted in setup, teardown, IO
completion handling, and inside RHI backends on objects the graphics API requires to be
externally synchronised, such as queue submission.

**Unsafe is quarantined.** `#![forbid(unsafe_code)]` in every crate except those listed in
section 16. Those crates carry a `SAFETY.md` explaining each invariant.

**Two dependency tiers.** Foundation crates may be depended on by anything. Backend crates may
be named only by `app` and by their own tests. The tier list lives in `crate-tiers.toml` and a
CI job fails any dependency edge the file does not allow.

**Things you run get names, things you import get descriptions.** Crates are named for what
they do, prefixed `lactose-`. Codenames from the glossary (section 21) appear on binaries,
tools, log targets, CI jobs, README titles and doc headings, never on crate names, types or
functions. The glossary is loaded into every agent session through `CLAUDE.md`.

**Determinism boundary is explicit.** Section 7 lists what is inside. Code inside the boundary
follows the determinism rules. Code outside may use wall-clock time, thread-local state and
platform float libraries.

**Everything is data.** Components are padding-free plain-data structs registered with the
reflection registry. If it cannot be reflected it cannot be serialised, replicated, snapshotted
or shown in a future editor, so it does not go in a component.

**Traced from day one.** The executor, every system, every render pass and every IO request
emits `tracing` spans. Goal 1 cannot be measured otherwise. Tracy is the profiler target.

## 4. Workspace layout

Every crate is published as `lactose-<name>`. Codenames in brackets are the public identity of
that subsystem and appear only where section 3 allows. Tier is F (foundation) or B (backend).

```
crates/
  core/          F  ids, handles, fixed-seed hashing, FixedHashMap, arenas, time types, errors
  math/          F  deterministic math: f32 and f64 vectors, quaternions, transforms; transcendentals via the libm crate
  task/          F  [WHISK] executor: one worker per core, work-stealing deques, fork-join scope, main-thread lane
  ecs/           F  [CASEIN] entities, archetypes, 16 KiB chunks, change tracking, queries, scheduler, command buffers
  reflect/       F  type registry, component registry, field IDs, persistence policy, serde bridge
  sim/           F  [CURD] deterministic world: regions, active areas, tick loop, snapshot ring, world hash
  anim/          F  deterministic animation logic: state machines, phase, events, root motion (inside the boundary)
  asset-api/     F  AssetId, Handle<T>, loader and processor traits, version table, streaming budget types
  physics-api/   F  bodies, colliders, joints, queries, events, snapshot and restore
  net-api/       F  transport, replication, prediction and snapshot contracts
  rhi-api/       F  device, queue, buffer, image, pipeline, bindless heap, encoder, resource states
  input/         F  action mapping, gamepad and keyboard sources, tick-stamped input samples
  asset/         B  [SILO] async loading, dependency tracking, immutable publication, hot reload
  io/            B  async file IO: blocking pool reference backend, io_uring and IOCP optimisations
  physics-rapier/B  [QUARK] Rapier backend, 64-bit
  net/           B  replication [MILKMAN], interest management, prediction area, rollback
  persist/       B  [SKIM] persistence policy, stable IDs, world overlay, save container [CARTON], migrations
  rhi-vulkan/    B  ash-based backend
  rhi-metal/     B  later
  rhi-dx12/      B  later
  render-graph/  B  [GOUDA] pass declaration, graph compile, barriers, transient aliasing, parallel record
  renderer/      B  GPU scene [CHEDDAR], bindless heap [BRIE], culling [CREAM], late-latch [LATTE],
                    pose evaluation and skinning, particles [FROTH], deferred lighting
  terrain/       B  heightfield clipmap rendering and collision export
  audio/         B  mixer thread, event bridge from simulation
  ui/            B  game UI and text rendering
  nav/           B  navmesh generation and queries (deterministic query half lives in sim)
  platform/      B  [PAIL] winit window, surface, main-thread pump
  debug-ui/      B  egui overlay, profiler view, determinism inspector
  app/           B  assembles runtime: client, server, headless; the only crate that names backends
game/
  <game crates>     gameplay systems and content; depend on foundation crates only
tools/
  cook/             [RICOTTA] offline asset cooker: meshes, textures, materials, shaders, scenes
docs/
```

There is no `task-api`. Spawning is generic over the future or closure type and cannot sit
behind a trait object, for the same reason `ecs` has no interface split.

## 5. Runtime model

### 5.1 Task system

Two primitives, with different rules.

**Fork-join scope, for chunk iteration inside a system.** `Scope::fork` takes plain closures,
not futures. Closures may borrow from the scope's stack. The worker that opens the scope
executes scope-local work and steals other work until the join counter reaches zero, then
returns. This is the rayon model and it is sound because the opening worker blocks until all
children finish. Children cannot `.await`. Nesting depth is bounded: a system opens one scope
over its chunks and chunk work opens none. The join counter is modelled under `loom`.

**Stackless `'static` tasks, for IO, GPU, asset and network work.** A task is a Rust `Future`
that owns its data. Waiting on a counter, an IO completion or a GPU fence is an `.await` that
returns the worker to the pool. These tasks never block a thread and may be cancelled by drop.

Shared properties:

- One worker per physical core minus one. The remaining core runs the platform thread.
- Each worker owns two deques, frame-critical and background. Workers pop their own
  frame-critical deque first, then steal frame-critical from others, then background. Priority
  is therefore real but coarse.
- Idle workers spin briefly, then `std::thread::park`.
- Chunk work is batched so a task body runs for roughly 50 to 200 microseconds. One hundred
  thousand entities across thirty systems must not become tens of thousands of task spawns.
- A small blocking pool exists for file IO on platforms without completion-based file APIs
  (macOS) and in CI containers where io_uring is unavailable. It is the reference IO backend.
- No thread-local state inside the determinism boundary. Scratch memory is handed in keyed by
  entity range, never by chunk index or worker index.
- A panic in any task aborts the process after writing a crash dump. There is no
  `catch_unwind` in the executor.

### 5.2 Frame pipeline

```
tick N        tick N+1 simulation ───────────────────────────┐
 │ extract (serial, ≤1 ms)                                    ├─ overlap
 └──────► frame N: graph compile → record → latch → submit ───┘
```

- Simulation runs at a fixed 60 Hz on its own cadence. Render runs as fast as the display
  allows and interpolates in the vertex shader between previous and current transforms held
  in the GPU scene.
- **Extract is a serial phase** between the end of tick N and the start of tick N+1. It reads
  dirty rows from chunks and uploads them to the GPU scene. It is on the simulation's critical
  path and budgeted at one millisecond. The render side holds no CPU mirror of simulation data.
- **Late-latch** writes exactly one GPU buffer, the view and projection matrices, after all CPU
  view-dependent work (shadow cascade fitting, light binning) has finished and immediately
  before submit. Only camera rotation is latched. Camera position is never latched because the
  simulation resolves camera collision. GPU culling pads the frustum by the maximum per-frame
  rotation so latched rotation cannot reveal culled geometry. Each input sample is stamped with
  the tick and the latched yaw actually displayed, and the simulation uses that yaw for
  movement, so the direction the player moves matches what they saw.
- Sync points sit at schedule stage boundaries. Their count is fixed per schedule and known at
  build time. Between sync points everything is a DAG of tasks.

### 5.3 Scheduling

- The scheduler builds a DAG per schedule at startup from declared access and set ordering. It
  caches the DAG and rebuilds only when systems are added or removed.
- Within a stage, independent systems run concurrently. Conflicting systems are ordered by set
  constraints or explicit system constraints. A missing constraint between two conflicting
  systems fails the schedule build.
- Order among conflicting systems is resolved by a stable topological sort keyed on
  registration order, so the result is identical across runs and thread counts.
- Each system iterates chunks in parallel through a fork-join scope. Per-entity work is
  independent by construction. Reductions are order-independent (integer accumulators) or sort
  by entity ID before combining. Chunk index is never a semantic key.
- Structural changes go into per-system command buffers and are applied at the next sync point
  in (system order, entity ID) order.

## 6. ECS data model

- **Archetypes** group entities by exact component set. **Chunks** are 16 KiB slabs holding a
  fixed number of rows of one archetype, with each component as a contiguous column. Row
  capacity is derived from the archetype's row size with a minimum of one. The registry warns
  on components over 1 KiB.
- **Components** are `bytemuck::NoUninit + AnyBitPattern`: padding-free, no heap ownership.
  The derive macro asserts this. Variable-length data is child entities joined by a
  relationship (an inventory is entities with a `HeldBy` relationship) or a fixed-capacity
  inline array. A region-owned blob arena is permitted only if profiling demands it and is
  then part of the region snapshot.
- **Change tracking** is two-level. Each column in each chunk carries a coarse change tick. Each
  column also carries a per-row dirty bitmask, one bit per row, set through the `Mut<T>`
  wrapper on actual deref-mut, never on query borrow. Replication, extract and persistence
  read the tick as a filter and the bitmask for rows. After a rollback restore or a load, ticks
  are rebased to current-minus-one and all rows marked dirty, forcing a full extract and a
  full replication delta for the region.
- **Entity IDs** are 64-bit: 32-bit index, 32-bit generation. No region bits. Region membership
  is a component. Index space is allocated in blocks of 4096 from a block table that is itself
  region-independent, deterministic and persistent. Each region holds its current block. Each
  player has a reserved block range for predicted spawns that the server confirms or remaps.
- **Regions** are the unit of streaming, snapshotting and relevance. A region owns its chunks,
  a block cursor, an RNG stream, and an asset version table. Regions never share chunks.
  Entities move between regions by a command-buffer operation that changes the region
  component and moves the row. Their ID does not change.
- **Active areas** are connected sets of loaded regions around a group of players. An active
  area owns one physics world. Areas merge when their region sets touch and split when they
  separate, as deterministic tick-boundary operations that move bodies in entity-ID order.
- **Region modes**: `Simulated` (systems run), `Replicated` (no systems, replication writes
  rows, extract reads them), `Frozen` (loaded, nothing runs).
- **Relationships** such as parent-child are components holding entity IDs with a reverse
  index maintained by the command-buffer apply step. The reverse index is part of the region
  snapshot. Cross-region relationships are weak references validated at apply time.
- **Resources** are singletons with declared access, scheduled exactly like components.
- **Queries** are generic over component tuples and monomorphise.

## 7. Determinism

### 7.1 Inside the boundary

`core`, `math`, `ecs`, `sim`, `anim`, `physics-api`, `physics-rapier`, `net` prediction and
rollback, the deterministic half of `nav`, and every `game` simulation system. Gameplay
particles (fire that spreads, wind that pushes) are simulation and live inside. The animation
state, phase and event layer is inside.

### 7.2 Outside the boundary

Rendering, pose evaluation and skinning, VFX particles, audio, UI, asset IO timing, debug
overlay, input sampling (its output, tick-stamped samples, is an input to the boundary).

### 7.3 Rules inside the boundary

- Floating point only through `math`. Transcendentals come from the `libm` crate, which is also
  what Rapier uses under its determinism feature, so simulation and physics agree on `sin`.
  `f32::mul_add` and friends are banned. Mixing `f32` and `f64` happens only inside `math`.
- SIMD is permitted for IEEE basic operations (add, sub, mul, div, sqrt, comparisons, min, max
  with defined NaN semantics), which are bit-exact across x86-64 and aarch64. Approximate
  reciprocals and rsqrt are banned.
- Target features are pinned per target in `.cargo/config.toml` (x86-64-v3 baseline for x86,
  default for aarch64). `target-cpu=native` is denied. No runtime CPU feature detection in
  boundary crates or their dependencies.
- No wall-clock time. `Instant` and `SystemTime` are banned. `Duration` as a value type is
  fine. Tick index and fixed delta only.
- No thread-local state. No global mutable state.
- Hash maps are `core::FixedHashMap`, a newtype over a fixed-seed hasher. Iteration order over
  a map is never observable; sort before consuming when order matters.
- Chunk index, worker index and task order are never semantic keys. Reductions are
  order-independent or entity-ID-sorted. RNG draws are keyed by (region seed, tick, system ID,
  entity ID) through a counter-based generator. Command buffers apply in (system order, entity
  ID). Physics bodies are inserted in entity-ID order.
- NaNs are canonicalised before hashing and before serialisation. Producing a NaN in a
  persistent column is a debug-build assertion.
- Asset availability and versions are inputs. Each region resolves its asset version table once
  per tick at the sync point; the table is a region resource and part of the snapshot. Hot
  reload is a tick-stamped event, never an atomic swap observed mid-tick.

### 7.4 Enforcement

`sim` exposes `world_hash(region) -> u64` computed through reflection, field by field, over
persistent and transient columns and region resources. Derived columns are excluded.

The CI harness, named SOUR, runs every simulation test in four modes and fails on any hash
mismatch:

| Mode | What varies | Catches |
|---|---|---|
| Workers | 1 worker versus N workers | scheduling-order dependence |
| Repeat | same configuration twice | uninitialised memory, time, randomness |
| Layout | shuffled insertion order, forced chunk compaction, different region load order | chunk-index dependence, the client-versus-server layout mismatch |
| Architecture | x86-64 runner versus aarch64 runner | float library and SIMD divergence |

Layout mode gates milestone 1. Architecture mode runs nightly from milestone 1 and on every
PR from milestone 4.

Static enforcement uses stable tooling only:

- `clippy.toml` in each boundary crate with `disallowed-methods` (platform float functions,
  `mul_add`), `disallowed-types` (`Instant`, `SystemTime`, `std::collections::HashMap`),
  `disallowed-macros` (`thread_local!`, `is_x86_feature_detected!`).
- `cargo-deny` with an allowlist of dependencies for boundary crates. A new transitive
  dependency fails CI until reviewed for float and feature-detection behaviour.
- No custom lint crate. That would require nightly.

## 8. Networking

- **Topology.** Dedicated server, authoritative. Clients send tick-stamped inputs. Server
  simulates and replicates state. The handshake carries a protocol hash derived from the
  registry schema hash and the `net` crate version; mismatches refuse connection.
- **Replication.** Per client, the server tracks the last acknowledged tick. Each outgoing
  packet carries, for entities in that client's relevance set, only rows whose dirty bit is
  set since the acknowledged tick. Relevance is region membership plus a distance band.
  Positions are quantised relative to region origin. Send rate 20 to 30 Hz, interpolated on
  the client. Remote regions on the client are in `Replicated` mode.
- **Predicted area.** The client simulates ahead one active area containing the local player
  and anything the player can touch within the rollback window. Predicted entities collide
  only with static geometry and other predicted entities. Every other entity is kinematic in
  the client's physics world and driven by interpolated server state.
- **Rollback.** The predicted area is snapshotted every tick into a ring whose length is sized
  from measured round-trip time plus send interval, with a hard cap of 40 ticks. On server
  correction the client restores the snapshot for the corrected tick, applies the correction
  and re-simulates to the present. If the corrected tick is older than the ring, the client
  requests a full area snapshot and resyncs. Copy-on-write chunks are a later optimisation
  behind the same `snapshot(area)` API.
- **Client gating.** Streaming leads the player by a hard distance. The server does not move a
  player into a region the client has not acknowledged as ready; the player is held at the
  boundary. Accepting a correction storm instead was considered and rejected.
- **Lag compensation.** Server keeps a short history of player transforms and colliders only.
- **Later.** Forward other players' inputs so clients predict nearby players too.

## 9. Snapshot definition

`snapshot(area)` captures everything the next tick reads:

1. Every chunk of every region in the area, by bulk copy.
2. Each region's block cursor, RNG state, asset version table, and other region resources.
3. The relationship reverse index.
4. The physics world for the area, by Rapier's clone. This includes contact manifolds,
   warm-start impulses and sleep state. Restoring chunks without it produces different results.
5. Pending command buffers must be empty at a snapshot; snapshots are taken only at sync points.

`physics-api` exposes `snapshot` and `restore` as first-class operations with a conformance
test: run ten ticks, snapshot at tick three, restore, re-run, hash equal at tick ten.

The cost is not microseconds. Chunk copy is, but the physics clone is hundreds of microseconds
to milliseconds depending on body count. Milestone 1 measures it and the predicted area is
sized from the measurement.

## 10. Rendering

### 10.1 Render hardware interface

Thin, explicit, modern. It abstracts what Vulkan 1.3, DX12 and Metal 3 share, verified rather
than assumed:

- device, queues, swapchain
- buffers with device addresses, images, samplers
- one bindless heap for textures and buffers
- pipelines from pre-compiled SPIR-V or Metal libraries, with specialisation
- command encoders recordable on any thread
- resource states as a backend-neutral enum; the graph emits transitions, the backend emits
  barriers or relies on Metal's hazard tracking
- timeline semaphores and fences
- indirect draw and dispatch. Indirect-count is a reported capability. The primary culling
  output is a fixed maximum draw count with zero-instance entries for culled draws; count
  buffers are used when available.

Constraints the macOS path imposes: MoltenVK compiles SPIR-V to MSL at pipeline creation and
stalls for hundreds of milliseconds per pipeline, so the renderer keeps pipeline count small
and creates all pipelines at load. Intel Macs are unsupported because their argument buffer
tier cannot hold the bindless heap. MoltenVK's Vulkan 1.3 core status, buffer device address
and descriptor indexing limits are verified in a one-day spike before `rhi-api` is written.

Nothing above the RHI knows which backend is active. The RHI is the one place where a boxed
trait object per subsystem is the right design.

### 10.2 Shader toolchain

Slang is the shader language. The cooker compiles Slang to SPIR-V for Vulkan and produces
Metal shader libraries from that SPIR-V at cook time. Bindless access is abstracted at the
Slang level so one source serves Vulkan descriptor indexing, DX12 resource heaps and Metal
argument buffers. There is no runtime shader compilation.

### 10.3 Render graph

- Passes declare reads and writes of named resources with usage. The graph derives order,
  culls unreachable passes, computes transient resource lifetimes, aliases memory, inserts
  transitions and assigns queues.
- Compile is cached by a hash of: the pass list, edges, resource descriptors including sizes
  and formats, imported resource initial states, view count, and queue configuration.
  Ping-pong history resources use a dedicated primitive so alternation does not thrash the
  cache. The cache is capped.
- Each pass records on a worker through the task system. Submission order is the graph order.
- Async compute is chosen by the graph when a pass has no graphics-queue dependency inside its
  lifetime window.

### 10.4 GPU-driven pipeline

Stage one:

- **GPU scene.** Persistent buffers for instances, meshes, materials, previous and current
  transforms. Allocated through a GPU-side free-list allocator. Uploaded by dirty row from the
  ECS bitmask during extract.
- **Culling.** Compute pass per view: frustum (padded for late-latch), distance, LOD selection.
  Writes compacted instance lists and indirect draw commands.
- **Draw.** Multi-draw indirect per pipeline. Bindless material and texture access by index.
- **Lighting.** Deferred G-buffer with clustered lights.
- **Skinning and deformation** as compute pre-passes writing into the same vertex pools, fed by
  pose evaluation that runs outside the determinism boundary from the deterministic animation
  state.
- **Transparency** sorted on the GPU per view.

Stage two: hierarchical-Z two-pass occlusion culling. Stage three: meshlets, mesh shaders and
a visibility buffer behind a feature flag, only when content density demands it.

### 10.5 Material model

Uber-shader with a material ID and a bindless parameter block. No free-form artist shaders.
Permutation axes are fixed: alpha-tested, two-sided, skinned, vertex-deformed. The cooker
emits every permutation. This is a hard constraint of GPU-driven batching and of MoltenVK's
pipeline compile cost.

## 11. Assets and streaming

- **Cook step.** `tools/cook` converts source assets into the runtime format: mesh pools ready
  for the GPU scene, block-compressed textures, resolved material parameter blocks, compiled
  shaders, scenes as region bundles. The runtime never parses a source format.
- **Immutable after publish.** An asset is built on worker tasks and published as a new
  version. Readers hold a `Handle<T>`. Inside the boundary, the version a region sees is the one
  in its per-tick version table. Outside, readers see the newest.
- **Async IO.** `io` has a blocking-pool backend as reference and universal fallback, with
  io_uring and IOCP as optimisations. A read is a task `.await`.
- **Streaming.** Regions declare their asset sets. A streaming budget task loads regions by
  predicted player movement. A region enters `Simulated` mode only when its set is published.

## 12. Reflection and serialisation

- `reflect` holds a registry of every component and resource type: name, stable type ID,
  size, alignment, fields with explicit `#[field(id = N)]` IDs, serde hooks, schema version
  and persistence policy. Registration is by derive macro. A type without a persistence policy
  or with a missing or duplicate field ID fails registration. CI checks that field IDs are
  never reused after removal.
- Scenes serialise as region bundles: for each archetype, columns written in bulk. Loading is
  a bulk copy into chunks. Text form exists for authoring and diffing; binary for cooked
  output.
- Replication, snapshots, saves and the future editor all read the same registry.

## 13. Save games and persistence

Saves share the serialisation core and the snapshot primitive with netcode. They do not share
the replication stream.

### 13.1 Persistence policy

| Policy | Meaning | Example |
|---|---|---|
| `persistent` | Written to saves at full precision | transform, health, inventory entities, quest flags |
| `transient` | Not saved, default-initialised on load, but deterministic and hashed | velocity caches, animation phase |
| `derived` | Not saved, not hashed, rebuilt by an `on_load` system | physics handles, render instance slots |

### 13.2 Stable identity

Persistent entities carry a `PersistentId`. For cooked entities it derives from the source
asset's GUID, so re-cooking never orphans a save. For runtime spawns it comes from a
server-only persistent counter held in global persistent resources.

### 13.3 World overlay

The save is a diff against the cooked world, kept per region. Overlay entries are
(`PersistentId`, field ID, absolute value), with the baseline content hash recorded per region.
Applying an overlay to a patched baseline keeps entries whose IDs still exist and drops the rest
with a log line. Region unloading and saving are the same code path.

### 13.4 Save operation

1. At a sync point, snapshot every active area (section 9).
2. On worker tasks, serialise through `reflect`, diff against the cooked baseline, compress
   with zstd.
3. Write through `io` to a temporary file, sync, rename over the slot.

### 13.5 Container and schema evolution

- Self-describing binary container with engine version, registry schema hash, and a per-region
  index.
- Matching schema hash takes the fast path: bulk copy into chunks. Mismatched schema decodes by
  field ID, applies registered per-type migrations in version order, then bulk-copies.
- After load, change ticks are rebased (section 6).

### 13.6 Multiplayer

The server owns the world save. Player entities are part of it. Portability of a player between
hosts is an open item.

### 13.7 Enforcement

- Test: save at tick T, load into a fresh process, simulate K ticks in both, compare persistent
  columns at T+K. Comparing at T alone cannot catch a component wrongly marked transient.
- Test: unload a region, reload it, persistent columns match.
- Test: load a save written by the previous schema version through its migration.

### 13.8 Journal replay

Checkpoint plus input journal is a debug and replay tool, not the save system.

## 14. Physics

Rapier, 64-bit build, through `physics-api`. Rapier's determinism feature is incompatible with
its parallel and SIMD features, so each physics world steps as one serial task. Parallelism is
across active areas only. The physics step is budgeted per area in milestone 1 and the
predicted area is sized against it. `physics-rapier` is inside the determinism boundary.
Replacement with an in-house solver is a backend swap behind the same interface, including
`snapshot` and `restore`.

## 15. Gameplay authoring

- Rust only. Gameplay systems live in `game` crates and depend on foundation crates only.
- Iteration loop: edit, incremental build, state-preserving restart through `reflect`. This
  costs a link plus a reflect round-trip and drops transient state by design. Build time is
  measured in milestone 1 and mitigated before it exceeds thirty seconds for a `game` change.
- Animation logic (states, phases, events, root motion) is simulation code in `anim`. Pose
  evaluation is renderer code.
- Behaviour is data-driven where possible: state machines, ability definitions and spawn
  tables are assets.

## 16. Unsafe policy

| Crate | Why |
|---|---|
| `task` | work-stealing deques, fork-join scope borrows |
| `ecs` | chunk column storage, typed views over raw memory |
| `io` | platform completion APIs |
| `rhi-*` | FFI to graphics drivers |
| `core` | arenas and handle tables |

Each carries `SAFETY.md`. Every `unsafe` block has a `// SAFETY:` comment. `task` and `ecs`
unit tests run under Miri in a nightly CI job. The fork-join join counter is modelled under
`loom`.

## 17. Agent workflow

- **Vertical slice for backend-driven crates.** `rhi`, `physics`, `io`, `net`: one task owns
  the `-api` crate and the first backend together. The `-api` is marked unstable until the first
  backend passes its own conformance tests, then frozen.
- **Interface-first only where behaviour is known** and there are at least two consumers:
  `asset-api`, `input`.
- **Amending an interface** is permitted in the same PR as the implementation that needs it,
  with the change called out in the PR description. Silent drift is not.
- **Tests are the contract.** Every `-api` ships a conformance suite. Determinism tests are
  mandatory inside the boundary. RHI conformance runs on Linux CI under lavapipe and on a
  GPU runner nightly.
- **One crate per task** otherwise. No speculative generality.
- **Docs live with code.** Each crate's `README.md` states responsibility, tier, boundary
  status and unsafe status.
- **CI gates.** clippy with `-D warnings`, fmt, tests, SOUR in all applicable modes, Miri
  nightly, `cargo-deny`, and the tier check against `crate-tiers.toml`.

## 18. Milestones

| # | Name | Delivers | Proves |
|---|---|---|---|
| 0 | Skeleton | Workspace, CI, `core`, `math`, `task`, `reflect`, tracing | Critical-path latency on a synthetic DAG matches its theoretical bound within 10%. Fork-join scope passes Miri and loom. Task spawn under 100 ns. SOUR scaffold runs. |
| 1 | World | `ecs`, `sim`, `persist`, `physics-api`, `physics-rapier`, `anim` logic, headless `app` | 100k entities, parallel systems, SOUR passes in workers, repeat and layout modes. Physics snapshot and restore conformance. Save, load, simulate, compare. Physics step and snapshot cost measured. Build time measured. |
| 2 | Pixels | `platform`, `rhi-api`, `rhi-vulkan`, `render-graph`, minimal `renderer`, Slang toolchain in `cook` | A triangle through the graph on Windows and macOS. MoltenVK spike results recorded. Pipelined render overlapping simulation with serial extract. lavapipe CI. |
| 3 | GPU-driven | GPU scene, culling, indirect draws, deferred lighting, `asset-api`, `asset`, `io`, `cook`, `terrain`, pose evaluation | 500k instances culled and drawn with a handful of CPU submissions. Streaming regions with overlay persistence. Skinned characters on terrain. |
| 4 | Network | `net-api`, `net`, client and server `app`, SOUR architecture mode on every PR | Four clients, prediction and rollback over an area, delta replication, resync path. Chaos test with injected latency and loss. x86 server, ARM client, hashes equal. |
| 5 | Game | `game` crates, `input`, `audio`, `ui`, `nav`, `debug-ui` | A playable slice: terrain, climbing, physics objects, co-op interaction, gamepad, sound. |

### 18.1 Decided later, by milestone

| Decision | By |
|---|---|
| Memory budgets per region, GPU, streaming; baseline-resident cost of overlay diffing | 3 |
| Crash reporting: minidumps and symbol upload | 2 |
| Build-time mitigation strategy if over budget | 1 |
| Audio thread model and event bridge | 5 |
| Navigation representation and AI behaviour model | 5 |
| UI framework and text rendering | 5 |
| Localisation | 5 |
| Protocol versioning beyond the handshake hash | 4 |
| Player profile portability between hosts | 4 |

## 19. Open items needing a spike

- MoltenVK: Vulkan 1.3 core conformance, buffer device address, descriptor indexing limits,
  draw-indirect-count. Before `rhi-api`.
- Rapier: confirm on the pinned version that the determinism feature excludes parallel and
  SIMD, and measure serial step cost. Milestone 1.
- io_uring availability on the CI runners. Milestone 0.
- Clippy `disallowed-methods` matching inherent primitive methods on the pinned toolchain.
  Milestone 0.
- Chunk size per archetype versus fixed 16 KiB. Milestone 1.

## 20. Review history

Revision 2 incorporates an adversarial review that found six blocking issues: unsound scoped
async spawning, a self-contradicting dependency rule, chunk-layout-dependent determinism,
snapshots that omitted physics state, region bits in entity IDs, and padding in hashed bytes.
All are addressed above.

## 21. Glossary of names

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
| LATTE | `lactose-renderer` | Late-latched view rotation. |
| FROTH | `lactose-renderer` | VFX particles, outside the determinism boundary. |
| QUARK | `lactose-physics-rapier` | Physics backend. |
| MILKMAN | `lactose-net` | Delta replication. Log target `milkman`. |
| SKIM | `lactose-persist` | Save system. Save Kit and Instance Manager. |
| CARTON | `lactose-persist` | Save container format. |
| SILO | `lactose-asset` | Immutable asset store and streaming. |
| RICOTTA | `tools/cook`, binary `ricotta` | Asset cooker. Ricotta means recooked. |
| PAIL | `lactose-platform` | Platform Abstraction and Input Layer. |
| SOUR | CI job `sour` | Determinism regression harness. |
