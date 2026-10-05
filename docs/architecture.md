# LACTOSE Architecture

LACTOSE: Lock-free Asynchronous Chunked Task-Oriented Simulation Engine.
Its deterministic simulation core is CURD: Concurrent Unified Runtime, Deterministic.
LACTOSE sets the CURD.

Status: design baseline, revision 3.1, after two adversarial reviews and an editorial pass. Every section is a decision,
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
   correction-free, because the client's prediction world differs from the server's world.
   Reconciliation is defined separately (section 9.4).
3. **No threading knowledge required of gameplay programmers.** They never write a lock, spawn
   a thread, or choose execution order. They declare what data a system reads and writes.
4. **Cache-coherent data layout.** Structure-of-arrays in fixed-size chunks on every hot path.
5. **No coordinator thread.** The scheduler is work-stealing. Ticking is a timer-driven root
   task, not a thread. The OS main thread pumps events only.
6. **Multiplayer without a fixed player cap.** Server cost is bounded by active entity count
   and per-client cost by relevance, never by a constant.
7. **Modular, interface-first where the interface is knowable.** Backend-driven subsystems are
   built as a vertical slice first.

Non-goals for now: an editor, mobile, web, Intel Macs, a scripting language, multi-process
server sharding.

## 2. Decisions

| Area | Decision |
|---|---|
| Language | Rust, stable toolchain, workspace of crates. No Bevy. |
| Reference game | Open-world action-adventure in the style of Breath of the Wild, networked co-op with an arbitrary number of players. |
| Simulation tick | 60 Hz fixed. Render uncapped with interpolation. |
| Netcode | Dedicated server, authoritative. Clients send tick-stamped inputs. Local-player prediction in a separate client prediction world. Snapshot interpolation for everything else. Interest management and per-client bandwidth prioritisation are part of the first networking deliverable, never a later addition. |
| Determinism scope | Same build, identical results across threads, layouts and x86-64 versus aarch64. Reconciliation is defined in quantised space. |
| Identity | Process-local entity IDs never leave the process. Server-assigned network IDs are the stable key for every ordering and RNG rule; unconfirmed predicted spawns use their prediction key until the server binds a network ID. Persistent IDs for saves. |
| World coordinates | Simulation scalar is a `math` type alias, 64-bit by default, decided finally by a milestone 1 precision and performance test at full map extent. GPU scene stores grid cell plus 32-bit offset. |
| Physics | Rapier through `physics-api`, pinned exactly. One physics world per server process, one prediction world per client. Rapier's determinism and parallel features together. |
| Graphics | Thin RHI. Vulkan 1.3 backend first (Windows, Linux CI). Native Metal backend in milestone 3 for Apple Silicon. MoltenVK and KosmicKrisp spiked in milestone 2 as interim Apple paths. DX12 when Xbox matters. A null backend for tests and the server. |
| Shaders | Slang. The cooker emits SPIR-V for Vulkan and MSL plus metallib for Metal. Mac shaders cook on a macOS runner. Driver pipeline caches are persisted. No runtime source compilation. |
| Lighting | Deferred with a G-buffer held in tile memory on tile-based GPUs. Visibility buffer when meshlets arrive. |
| Draw submission | One indirect draw per pipeline over fixed-size clusters with programmable vertex pulling. Multi-draw-indirect only where a backend reports native support. |
| Latency | Simulation N+1 overlaps render N after an exclusive extract phase. A double-buffered presentation snapshot feeds a variable-rate presentation schedule. The full orbit camera transform is late-latched. |
| Concurrency | Systems complete by continuation counters. Fork-join only inside a system, same-scope help only. `'static` stackless futures for IO, GPU and asset work on a custom executor. No stackful fibers. No tokio. Frame-critical workers on performance cores, background class on efficiency cores. |
| Component data | `bytemuck::NoUninit + CheckedBitPattern`: padding-free, no heap ownership, validated only on load and receive. Variable-length data is owned child entities or fixed-capacity inline arrays. |
| Change tracking | Per-chunk coarse tick plus a per-row change tick on tracked columns. Each consumer keeps its own last-seen tick. |
| Error policy | Panics only for broken invariants and they abort. Everything else is `Result` up to an `app`-level policy. Tick-stamped error events inside the boundary. |
| Gameplay authoring | Rust only, split into simulation and presentation crates. Iteration via state-preserving restart with a prescribed dev profile. |
| Editor | Deferred. Reflection in scope from day one. |
| Saves | Overlay of typed records against the cooked baseline, sharing the serialisation core with netcode but never the replication stream. Server-owned. Explicit field IDs and migrations. |
| Platforms | Windows and Apple Silicon macOS clients. Linux dedicated server. CI on linux-x64, windows-x64 and macos-arm64. Consoles when Rust toolchains exist. |

## 3. Principles that bind every crate

**Declare access, never order.** A system declares the components and resources it reads and
writes. The scheduler derives concurrency from declarations and ordering from a stable
topological sort over declared system sets. Two conflicting systems with no ordering at any
level are a schedule build error.

**No locks on the hot path.** Per-frame work uses chunks, command buffers and atomic counters.
A `Mutex` inside a system is a review rejection. Locks are permitted in setup, teardown, IO
completion handling, and inside RHI backends on objects the graphics API requires to be
externally synchronised.

**Unsafe is quarantined.** `#![forbid(unsafe_code)]` outside the crates in section 21.

**One dependency rule, in one file.** `crate-tiers.toml` is the only statement of which crate
may depend on which. This document describes it; the file decides. CI enforces it.

**The determinism boundary is a crate boundary.** A crate is inside or outside. Crates that
would straddle it are split: `net` (outside) and `predict` (inside), `game-sim` and
`game-present`, `nav` and `nav-query`. Section 8.1 lists the inside crates.

**Things you run get names, things you import get descriptions.** Codenames from the glossary
(section 23) appear on binaries, tools, log targets, CI jobs, README titles and headings,
never on crate names, types or functions.

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

Every crate is published as `lactose-<name>`. Tier is F (foundation, usable by anyone) or B
(backend, usable only by `app`, `tools/*` and the exceptions in `crate-tiers.toml`).
Codenames in brackets appear only where section 3 allows. Boundary marks crates inside the
determinism boundary.

```
crates/
  core/           F  boundary  EntityId, NetId, PersistentId, PredictionKey, Handle<T>, FixedHashMap, arenas, Error taxonomy
  math/           F  boundary  self-contained deterministic math, no glam; Scalar alias; compare-and-select min/max/clamp; libm transcendentals; typed render space
  task/           F            [WHISK] executor: continuation counters, fork-join scope, worker classes, timer root, main-thread lane
  ecs/            F  boundary  [CASEIN] entities, archetypes, chunks, per-row change ticks, queries, scheduler, command buffers
  reflect/        F            type registry, field IDs, typed references, persistence policy, serde bridge
  sim/            F  boundary  [CURD] regions, region modes, tick loop, time scale, world hash, snapshot API
  anim/           F  boundary  animation state machines, phase, events, root motion, sim skeleton (sockets, hands, feet, head, ragdoll bodies)
  predict/        F  boundary  prediction world, rollback ring, reconciliation
  nav-query/      F  boundary  navmesh queries
  physics-api/    F  boundary  bodies, colliders, joints, queries, events, snapshot and restore
  asset-api/      F            AssetId, Handle<T> policy, loader and processor traits, version table types, streaming budget types
  present-api/    F            presentation components (mesh, light, decal, camera rig), VFX and audio request events, UI view-model, debug-draw buffer, presentation snapshot
  net-api/        F            transport, replication, staging, interest contracts
  rhi-api/        F            device, queue, buffer, image, pipeline, bindless heap, encoder, resource states, memoryless attachments, local reads
  input/          F            action mapping, gamepad and keyboard sources, tick-stamped samples with absolute camera yaw and pitch
  io/             F            async file IO: blocking pool reference backend; io_uring and IOCP optimisations
  asset/          B            [SILO] loading, dependency tracking, versioned publication, hot reload
  physics-rapier/ B  boundary  [QUARK] Rapier backend
  net/            B            [MILKMAN] transport, replication, interest management, bandwidth prioritisation, clock sync, interpolation buffer
  persist/        B            [SKIM] world overlay, save container [CARTON], migrations
  rhi-vulkan/     B            ash-based backend
  rhi-metal/      B            native Metal backend, milestone 3
  rhi-null/       B            no-op backend for tests and the server binary
  rhi-dx12/       B            later
  render-graph/   B            [GOUDA] pass declaration, compile, barriers, aliasing, parallel record
  renderer/       B            GPU scene [CHEDDAR], bindless heap [BRIE], culling [CREAM], late-latch [LATTE], skinning, VFX [FROTH], deferred lighting, render slot table
  presentation/   B            per-frame presentation schedule: camera rig, pose evaluation, interpolation sampling, UI animation, audio listener, markers
  terrain/        B            clipmap rendering; collision export is a cook output consumed through asset-api
  audio/          B            mixer thread, event bridge
  ui/             B            game UI and text rendering
  nav/            B            navmesh generation
  platform/       B            [PAIL] winit window, surface, event pump, present thread
  debug-ui/       B            egui overlay, profiler view, determinism inspector
  app/            B            assembles runtime: client, server, headless
game/
  game-sim/          boundary  gameplay simulation systems; depends on foundation boundary crates
  game-present/                gameplay presentation; depends on present-api and foundation crates
  game-ai/           boundary  perception, behaviour, decision systems; depends on nav-query and foundation boundary crates
  game-content/                data definitions shared by all three
tools/
  cook/                        [RICOTTA] asset cooker: meshes, clusters, textures, materials, shaders, scenes, collision
```

There is no `task-api`: spawning is generic and cannot sit behind a trait object. `ecs` has
no interface split for the same reason. Backend services that `game` must reach (physics
queries, asset lookup, presentation requests) are exposed as `dyn` resources whose traits live
in the corresponding `-api` crate and whose instances `app` inserts at startup.

## 5. Runtime model

### 5.1 Task system

**Systems complete by continuation.** Each system in a stage has a remaining-jobs counter.
Chunk jobs decrement it. The job that reaches zero runs the system's finalize step (where
reductions merge partials) and then decrements the counters of dependent systems, pushing any
that reach zero. No worker ever blocks waiting for a system.

**Fork-join inside a job only.** `task::scope(|s| s.fork(...))` with an invariant `'scope`
lifetime, closure-only, the shape of `std::thread::scope`. Children are closures that may
borrow from the scope. The opener runs only jobs belonging to its own scope while waiting,
then spins briefly, then parks on the scope's latch. It never runs unrelated system jobs and
never runs background work while waiting. A job opens at most one scope and scopes do not
nest. Child jobs are allocated on the opener's stack, since the scope outlives them.

**Stackless `'static` tasks** for IO, GPU, asset and network work. A task is a `Future` that
owns its data and may be cancelled by drop. Waiting is an `.await`.

**Worker classes.**

- Frame-critical workers equal the performance-core count on every platform:
  `hw.perflevel0.physicalcpu` on macOS, the hybrid core-type query on Windows, CPU capacity
  from sysfs on Linux, and physical cores minus one only where no core-type information
  exists. They run at user-interactive QoS. They run simulation,
  extract, graph compile and record.
- Background workers run on efficiency cores at utility QoS (two low-priority threads on
  homogeneous CPUs). They run asset decode, compression, and long futures. Background CPU
  work yields at 0.5 millisecond slices.
- The platform thread pumps OS events. A dedicated present thread calls present so a blocking
  `nextDrawable` or FIFO present never stalls the event pump or a worker.

**Scheduling mechanics.**

- Each worker owns a deque. Work stealing uses split-on-steal: a stolen chunk-range job is
  split in half and the thief takes one half. There is no fixed task-duration target.
- Idle workers spin briefly, then `std::thread::park`. The sleep and wake protocol is
  modelled under `loom`.
- Main-thread requests return futures. Workers never wait synchronously on the main thread.
- The floating-point control register (MXCSR, FPCR) is set to round-to-nearest with denormals
  enabled at worker start and asserted at the start of every boundary job in debug builds.

**Panics abort, unconditionally.** An abort-on-unwind guard surrounds every job and every join
wait. A panic hook writes a crash dump and calls `abort`. This holds in tests, which run under
`cargo-nextest` with one process per test and timeouts, and under Miri and loom, where the
guard is modelled. `task` exposes no blocking wait; calling one on a worker is a debug
assertion.

**The timer root.** A root task owns ticking: it wakes at tick deadlines, caps catch-up at four
ticks per frame then slows time, and paces frames. It runs on whichever frame-critical worker
picks it up. Section 11 covers time scale and pause.

### 5.2 Scheduling

- The scheduler builds a DAG per schedule at startup from declared access and set ordering,
  cached until systems change.
- Conflicting systems are ordered by set or system constraints; a missing constraint fails the
  build. Ties are broken by system stable key, which is a declared name hash checked for
  collisions, never by registration order.
- Each system iterates chunks as jobs. Per-entity work is independent by construction.
  Reductions use provided combinators that merge in the finalize step in stable-key order or
  with an order-independent combiner. The query API exposes no chunk, row or worker index.
  Scratch memory is an opaque per-job arena, poisoned between uses in debug.
- Structural changes go into command buffers applied at the next sync point in the total order
  (system stable key, source stable key, per-invocation sequence). Spawns receive placeholder
  IDs resolved at apply.
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
- **The presentation snapshot** is double-buffered and holds transforms, camera pivot,
  presentation components and events. It is the only simulation-derived data the render side
  reads during the overlap. Remote entities are not in it: the presentation schedule samples
  the network interpolation buffer directly, each frame, through a `net-api` resource.
- **The presentation schedule** runs per render frame on the snapshot: camera rig with
  non-authoritative boom collision, UI animation, audio listener, world-space markers, full
  pose evaluation, VFX simulation (section 13).
- **Late-latch** writes exactly one GPU buffer, the full orbit camera transform (pivot from the
  snapshot interpolated to frame time, rotation from the latest input, boom length from
  presentation collision), after all CPU view-dependent work and immediately before submit.
  Light binning runs on the GPU in world space after the latch. Shadow cascades and GPU
  culling are padded by a configured maximum rotation and maximum boom-length change per
  frame, and the latched deltas are clamped to those pads. The pivot needs no pad: it comes
  from the snapshot, so its motion is already inside the swept culling bounds. Each input sample carries the absolute yaw and pitch it was produced
  with; the simulation uses those values for movement direction.

## 6. ECS data model

- **Archetypes** group entities by exact component set. **Chunks** are 16 KiB slabs with one
  contiguous column per component. Row capacity derives from row size with a minimum of one.
  The registry warns on components over 1 KiB. The determinism harness varies chunk capacity
  in tests (section 8.3).
- **Components** are `bytemuck::NoUninit + CheckedBitPattern`: no padding, no heap ownership,
  `bool` and enums and `Option<NetId>` allowed. Byte validation runs only on save load and
  network receive. Explicit padding fields are zeroed by the API. Inline arrays are `[T; N]`
  plus a length with unused slots zeroed. A region-owned blob arena is permitted only if
  profiling demands it and is then part of the region snapshot.
- **Change tracking.** Every column in every chunk carries a coarse change tick. Columns marked
  tracked (replicated, extracted or both) also carry a `u32` change tick per row, set through
  `Mut<T>` on deref-mut. Replicated columns use `set_if_neq` so rewriting an equal value does
  not dirty the row. Because each row's tick is its own word, chunk jobs may split at any row without a shared
  write; no atomics are involved. Whole-column slice access takes an explicit dirty-range
  out-parameter.
  Each consumer (extract and each client) keeps its own last-seen tick and reads rows with a
  newer tick. After a rollback restore or a load, ticks are rebased and every consumer's
  last-seen tick reset, forcing a full pass.
- **Regions** are the unit of streaming, relevance and persistence. A region owns its chunks,
  an asset version table and region resources. Entities move between regions by a command that
  moves the row; identities do not change. Entities owned by another entity (inventory items,
  attachments) migrate with their owner.
- **Region modes**: `Simulated`, `Replicated` (rows written only at the input sync point from
  staged network state), `Frozen`. The input sync point is the first sync point of a tick,
  before any stage runs; it is where inputs, staged network state and asset version tables
  are applied.
- **Relationships** are components holding `NetId` with a reverse index maintained at command
  apply and included in snapshots. Cross-region references are weak and validated at apply.
- **Resources** are singletons with declared access.
- **Queries** are generic over component tuples and monomorphise.

## 7. Identity

| Key | Scope | Assigned | Used for |
|---|---|---|---|
| `EntityId` | one process | process-local allocator, index plus generation, `NonZeroU64` so `Option` is free | storage lookup only; never on the wire or disk |
| `NetId` | server and all clients | cooked entities derive it from their placement so clients know it without a message; runtime spawns get it from the server at command apply, so order is deterministic | every ordering rule, every RNG key, replication, render slot table, relationships |
| `PredictionKey` | one client plus server echo | (player ID, input tick, ordinal) | a client-predicted spawn before the server confirms; the server echoes the key on the entity it spawns and the client binds that `NetId` to its predicted entity instead of remapping |
| `PersistentId` | saves | cooked: hash of (scene, placement, child path); runtime: server persistent counter at command apply | overlay records and save references |
| `SystemKey` | build | declared name hash, collision-checked | command order, RNG stream selection |
| `PlayerId` | session | server-assigned on join | prediction keys, interest sets, input queues |

Typed reference fields (`EntityRef` holding `NetId`, `AssetRef` holding `AssetId`) are known to
the registry and remapped on load, so the bulk-copy load path is a copy plus a remap pass.

## 8. Determinism

### 8.1 The boundary

Inside: `core`, `math`, `ecs`, `sim`, `anim`, `predict`, `nav-query`, `physics-api`,
`physics-rapier`, `game-sim`. Gameplay particles that affect simulation (spreading fire, wind
pushing objects) live in `game-sim`.

Outside: everything else, including `net` (transport, RTT, interpolation buffer),
presentation, pose evaluation, VFX, audio, UI, asset IO timing and input sampling.

### 8.2 Rules inside the boundary

- Floating point only through `math`, which is self-contained and does not depend on glam.
  Transcendentals come from the `libm` crate. `math` owns `min`, `max` and `clamp` as explicit
  compare-and-select with defined signed-zero and NaN results, because the standard library
  versions are documented as nondeterministic on signed zero. `f32::min`, `f32::max`, `clamp`, `powi`, `total_cmp`,
  `to_bits`, `copysign` and `is_sign_negative` and their `f64` forms are banned outside
  `math`, as are trait routes to the platform library such as `num_traits::Float`; NaN sign and payload are never
  observable. NaNs are canonicalised before hashing and serialisation, and producing one in a
  persistent column is a debug assertion.
- Fused multiply-add is correctly rounded and permitted. The hazard is code that chooses fused
  or unfused by target feature, which `wide` and glam do. Therefore x86-64 targets are pinned
  to the x86-64-v3 baseline in `.cargo/config.toml`, `math` contains a `compile_error!` on
  x86-64 without the `fma` feature so a dropped pin fails the build, and `target-cpu=native`
  is denied.
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
- No thread-local state. No global mutable state.
- Hash maps are `core::FixedHashMap`. Iteration order is never observable.
- Chunk index, row index, worker index and task order are never semantic keys. The API makes
  them unavailable. RNG draws are keyed by (region seed, tick, system key, stable entity key) through a
  counter-based generator with no mutable state. The stable entity key is the `NetId`, or the
  `PredictionKey` for a predicted spawn the server has not yet confirmed.
- A boundary crate may depend on a non-boundary foundation crate (`task`, `reflect`,
  `asset-api`, `input`, `present-api`) only for types and for operations that are deterministic
  by construction: no clocks, no floats computed outside `math`. The `cargo-deny` allowlist
  names the permitted edges.
- Asset availability and versions are inputs. Each region resolves its version table at the
  input sync point; hot reload of simulation assets is a server-coordinated, tick-stamped
  event.

### 8.3 Enforcement

`sim` exposes `world_hash(region) -> u64` computed through reflection, field by field, over
persistent and transient columns and region resources, iterating entities in `NetId` order.
Derived columns are excluded.

The harness, named SOUR, runs every simulation test in these modes and fails on any mismatch:

| Mode | What varies | Catches |
|---|---|---|
| Workers | 1 worker versus N | scheduling-order dependence |
| Repeat | same configuration twice | uninitialised memory, time, randomness |
| Layout | test-only chunk capacities (1, 7, 64 rows, 16 KiB), random row permutation at every sync point, reversed and shuffled chunk iteration, random batch splits; identities preserved | storage-order dependence |
| Chaos | seeded steal order, injected yields, forced split-on-steal | ordering bugs invisible on 4-vCPU CI |
| Architecture | linux-x64 (including a Haswell-masked run), windows-x64, macos-arm64 | float library and SIMD divergence |

Layout and chaos modes gate milestone 1. Architecture mode runs nightly from milestone 1 and
on every PR from milestone 4.

Static enforcement uses stable tooling: `clippy.toml` per boundary crate with
`disallowed-methods`, `disallowed-types` and `disallowed-macros`; `cargo-deny` with a
dependency allowlist for boundary crates and `multiple-versions = "deny"` for shared crates.
No custom lint crate.

## 9. Networking

### 9.1 Topology and scale

Dedicated server, authoritative. Players join and leave freely; there is no fixed cap. Server
cost is bounded by active entities: regions with no player within the activation radius are
`Frozen`, which is a deterministic function of inputs. Per-client cost is bounded by the
relevance set.

The handshake carries a protocol hash of the registry schema, a boundary build ID (the
`Cargo.lock` entries and source hash of boundary crates), and the hash of simulation-relevant
cooked data. Mismatches refuse connection.

### 9.2 Data in and out of the tick

- Received packets are parsed on background workers into per-client staging queues.
- At the input sync point, and only there, staged inputs are applied to the server tick and
  staged state is applied to `Replicated` rows on the client. Nothing writes simulation rows
  concurrently with systems or extract.
- Server input policy: inputs are buffered per client with a small jitter window; a late input
  is replaced by the previous input and the client is told.
- Client clock sync: the client runs ahead by half the round trip plus a jitter margin and
  adjusts its tick rate by up to three percent to track.

### 9.3 Replication and interest

- Per client, the server tracks the last acknowledged tick, the relevance set, and a priority
  accumulator per relevant entity. Relevance is region membership within a radius plus an
  always-relevant set. Each packet is filled by priority (distance and staleness) up to the
  client's bandwidth cap, from rows whose change tick is newer than the client's last-seen
  tick. Packet loss needs no per-packet bookkeeping.
- Positions are quantised relative to region origin. Send rate 20 to 30 Hz.
- On the client, `net` owns an interpolation buffer per remote entity holding received states.
  The presentation schedule samples it each frame, about two send intervals in the past,
  through a `net-api` resource. It never feeds simulation rows. `Replicated` rows hold the latest received state for
  simulation-side queries.
- Late join and resync use the same path: a full relevance-set snapshot from the server.

### 9.4 Prediction

- The client owns a **prediction world**, separate from its replicated world, with its own
  physics world, entity table and snapshot ring. It holds the local player, entities the player
  has claimed (thrown objects, predicted spawns), and static collision. Every other entity is a
  kinematic proxy driven by the latest received state. Predicted entities collide only with
  static geometry and other predicted entities.
- Ring depth is derived at runtime from the frame re-simulation budget divided by the measured
  per-tick cost, with a configurable cap. Corrections deeper than the ring trigger a rebuild
  from replicated state. Section 10 defines what a snapshot holds.
- **Reconciliation** compares predicted and authoritative state per field in quantised space.
  Corrections carry every simulation-read field at full precision. Mispredictions against
  non-predicted entities are expected and smoothed visually. Proxies live about half a round
  trip in the past relative to the local player; this is an accepted limitation, and predicted
  interactions with proxies are presented tentatively until confirmed.
- Later: forward nearby players' inputs so the prediction world includes them.

### 9.5 Lag compensation

The server keeps a short history of transforms and colliders for players and for NPCs that can
be hit, capped at 250 milliseconds, and validates hits at the tick the client saw.

### 9.6 Scaling path

Multi-process sharding is out of scope. Region ownership and command-based cross-region
interaction keep it possible later. No system may hold a direct reference across regions.

## 10. Snapshots

Only the client prediction world is ever rolled back. Its `snapshot` captures mutable columns,
the entity table, region resources, the relationship index and the physics world through
`physics-api::snapshot`. Command buffers are empty at snapshot points by construction.

The server never rolls back. Save snapshots copy persistent columns only (section 15.3).

`physics-api` conformance: run ten ticks, snapshot at tick three, restore, re-run, hash equal
at tick ten. The cost is measured in milestone 1 and the prediction world is sized from it.

## 11. Time and pacing

- The timer root (section 5.1) owns tick deadlines, catch-up and frame pacing.
- Time scale is a simulation resource replicated by the server. It changes how many ticks run
  per real second; the tick delta never changes. Pause is time scale zero, server-controlled.
- Presentation runs on real time regardless of time scale.

## 12. Rendering

### 12.1 Render hardware interface

Thin, explicit, modern, with every claimed common feature verified in the milestone 2 spike.

- device, queues, swapchain
- buffers with device addresses, images, samplers
- one bindless heap; residency calls where the backend needs them
- pipelines from cooked SPIR-V or Metal libraries, with specialisation; driver pipeline caches
  persisted to disk (`VkPipelineCache`, `MTLBinaryArchive`)
- command encoders recordable on any thread
- resource states as a backend-neutral enum; backends emit barriers, and Metal backends handle
  aliased transients and argument-buffer residency explicitly rather than relying on automatic
  hazard tracking
- memoryless and transient attachments, and render-pass-local reads (dynamic rendering local
  read on Vulkan, framebuffer fetch on Metal) so the G-buffer stays in tile memory
- timeline semaphores and fences
- indirect draw and dispatch; multi-draw-indirect and indirect-count are reported capabilities
- a null backend for tests, graph unit tests and the server binary

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

- Instances, meshes, materials, previous and current transforms live in persistent GPU buffers
  with a GPU-side free-list allocator. Positions are stored as a 32-bit integer grid cell plus a
  32-bit float offset; shaders subtract the camera cell so precision is preserved anywhere on
  the map. `math` has a distinct render-space type with no implicit conversion from simulation
  scalars. Swept culling bounds, the light grid, decals, probes, VFX emitters, cascades, the clipmap
  centre, sky and atmosphere, markers, the audio listener and emitters, GPU picking, debug
  draw and the camera itself all convert through it.
- Render slots are a renderer-owned side table keyed by `NetId`, reconciled after any restore
  or load, never a column in simulation chunks.
- Draw submission is one indirect draw per pipeline over fixed-size clusters with programmable
  vertex pulling: cluster index from instance ID, fixed vertex count per cluster, degenerate
  padding. This behaves identically on every backend. Multi-draw-indirect is used only where a
  backend reports native support.
- Culling is a compute pass per view: frustum padded for late-latch, distance, LOD. Skinning
  and deformation are compute pre-passes. Transparency is sorted on the GPU per view.
- Stage two: hierarchical-Z occlusion. Stage three: meshlets, mesh shaders and a visibility
  buffer behind a feature flag.

### 12.5 Lighting and materials

Deferred with a G-buffer kept in tile memory on tile-based GPUs through memoryless attachments
and local reads. Clustered lights binned on the GPU in world space after the latch.

Uber-shader with a material ID and a bindless parameter block. No free-form artist shaders.
Permutation axes: material (alpha-tested, two-sided, skinned, vertex-deformed) by pass type
(G-buffer, depth-only, forward transparent, motion vectors). Those axes give 64 graphics pipelines; the budget is 96 to leave room for post-processing,
compute, UI and debug pipelines. All are created at load.

## 13. Presentation

`present-api` is a foundation crate so `game-present` can reach presentation without naming
a backend. It defines mesh, light, decal and camera rig components; VFX and audio request
events; UI view-model resources; a debug-draw buffer; and the presentation snapshot type.

`presentation` runs the per-frame schedule on the snapshot (section 5.3). The camera rig is
presentation, not simulation: its orbit, boom and boom collision are evaluated per frame
against static collision and are not authoritative, and the input it produces carries absolute
yaw and pitch into the simulation. Pose evaluation for rendering runs here; the simulation uses
the deterministic sim skeleton from `anim` for hit volumes, hand and foot placement, sockets
and ragdoll bodies.

## 14. Assets and streaming

- The cooker emits runtime formats only: cluster-ready mesh pools, block-compressed textures,
  resolved material blocks, compiled shaders, collision, region bundles.
- Assets are versioned and immutable after publish. Inside the boundary a region sees the
  version in its table; outside, readers see the newest.
- File IO goes through `io`, a foundation crate with a blocking-pool reference backend and
  io_uring and IOCP optimisations.
- Regions declare asset sets. A region enters `Simulated` only when its set is published.
  Streaming leads the player by a hard distance; the server holds a player at a region boundary
  until the client acknowledges readiness. A client-only asset failure is a tick-stamped event
  that makes the client request a resync rather than diverge.

## 15. Reflection, serialisation and persistence

`reflect` holds a registry of every component and resource type with explicit
`#[field(id = N)]` IDs, typed reference fields, schema version and persistence policy. Missing
or duplicate IDs fail registration. CI rejects ID reuse.

### 15.1 Persistence policy

| Policy | Meaning | Example |
|---|---|---|
| `persistent` | Saved at full precision | transform, health, velocity, animation phase, timers, quest flags |
| `transient` | Written before read every tick, never saved; hashed | per-tick accumulators, contact lists |
| `derived` | Rebuilt by `on_load`; not saved, not hashed | physics handles |

Transient is enforced by a poison test: debug builds randomise transient columns after every
restore and load.

### 15.2 World overlay

The save is a set of typed records per region against the cooked baseline, each keyed by
`PersistentId`: `Spawn`, `Destroy`, `AddComponent`, `RemoveComponent`, `SetField`,
`OwnershipChange` (a change of owning entity; a region move is a `SetField` on the region
membership component). The baseline content hash is recorded per region. Region unloading
writes the same records, so unloading and saving are one code path. A patched baseline keeps
all runtime-spawned entities and drops only `SetField` records whose target no longer exists.

### 15.3 Save operation

At a sync point, copy persistent columns of every loaded region on worker tasks, serialise
through `reflect`, diff into records, compress with zstd, write through `io` to a temporary
file, sync, rename over the slot.

### 15.4 Container and evolution

Self-describing binary container with engine version, schema hash and a per-region index.
Matching schema takes the bulk-copy-plus-remap fast path; mismatched schema decodes by field ID
and runs migrations in version order. After load, change ticks are rebased (section 6).

### 15.5 Enforcement

- Test: the original performs an in-place reload at tick T; a fresh process loads the save;
  both run K ticks and must hash identically.
- Test: unload and reload a region; persistent columns match.
- Test: load a save from the previous schema version through its migration.
- Test: the fresh process is also compared against the un-reloaded original after K ticks,
  bit-exact for discrete state and within a stated tolerance for continuous state, so a
  reload is shown not to change gameplay outcomes.

Journal replay (checkpoint plus input journal) is a debug and replay tool, not the save
system.

The server owns the save in multiplayer. Player portability between hosts is an open item.

## 16. Physics

Rapier, pinned to an exact version together with its math and SIMD dependencies; any bump is a
protocol and replay break. One physics world per server process and one per client prediction
world. Rapier's determinism feature is used together with its parallel solver; the solver's
rayon pool is sized to the frame-critical worker count and runs during a physics phase in which
frame-critical workers park. The physics step and snapshot cost are measured in milestone 1,
and the simulation scalar is decided there by a precision test at full map extent.

`physics-rapier` is inside the boundary. Replacement is a backend swap behind `physics-api`
including `snapshot` and `restore`.

## 17. Animation

`anim` is inside the boundary: state machines, phase, events, root motion, and a deterministic
sim skeleton covering sockets, hands, feet, head and ragdoll bodies. Full pose evaluation and
skinning are presentation and renderer work.

## 18. Gameplay authoring

- Rust only. `game-sim` depends on foundation boundary crates; `game-present` on `present-api`
  and foundation crates. `game` is split into the four crates in section 4 from the start to bound incremental build
  cost. Behaviour is data-driven where possible: state machines, ability definitions and spawn
  tables are assets.
- Dev profile: dependencies at opt-level 3, game crates at opt-level 1, line-tables-only debug
  info, split debuginfo, mold or lld on Linux, lld-link on Windows. Registration is through
  generated functions, not life-before-main. Edit-to-playing time is measured per platform in
  milestone 1 and budgeted at thirty seconds.
- Iteration is state-preserving restart through `reflect`. Dylib reload later if needed.

## 19. Error handling

The policy is in section 3.

- `core::Error` is the taxonomy. Subsystems return `Result`; `app` holds policy.
- `tracing` is also the logging facade. Log targets are the codenames from section 23.
- Tested device-lost recovery: re-upload assets, rebuild the GPU scene from region data, full
  extract.
- Network drop, reconnect, disk full and corrupt save each have a defined path by milestone 4.

## 20. Agent workflow

- Three tracks from milestone 1: simulation (`ecs` to `sim` to physics to the netcode spike),
  render (RHI spike to Vulkan to graph to GPU scene), assets (`io` to `asset` to `cook`). They
  join at extract in milestone 2 and streaming in milestone 3.
- A naive single-threaded `ecs` query and system facade ships in week one so dependent agents
  can start.
- Backend-driven crates (`rhi`, `physics`, `net`) are vertical slices: one task owns the
  `-api` and the first backend, and the `-api` is frozen once that backend passes its
  conformance suite. `rhi-api` stays unstable until the native Metal spike has run. `io` has
  no `-api`; its backends sit behind a trait inside the crate.
- Interface-first only where behaviour is known and there are two or more consumers:
  `asset-api`, `input`, `present-api`.
- Amending an interface in the same PR as the implementation that needs it is permitted and
  must be called out.
- One crate per task otherwise. Build what the next milestone needs; the interface crate is
  the place for extension points, not the implementation.
- CI budget: a pull request run finishes in fifteen minutes. Miri, loom and the architecture
  mode run nightly.
- Every `-api` ships a conformance suite. RHI conformance runs under lavapipe on Linux CI and
  on a GPU runner nightly.
- Each crate's `README.md` states responsibility, tier, boundary status and unsafe status.
- CI: clippy `-D warnings`, fmt, nextest, SOUR in all applicable modes, Miri nightly on `task`
  and `ecs`, `cargo-deny`, the tier check.

## 21. Unsafe policy

| Crate | Why |
|---|---|
| `task` | deques, scope borrows, panic guard |
| `ecs` | chunk column storage |
| `io` | platform completion APIs |
| `rhi-*` | FFI to graphics drivers |
| `core` | arenas, handle tables, `unsafe impl` for generic plain-data types |
| `tools/cook` | FFI to Slang and compression libraries |

Each carries `SAFETY.md`. Every `unsafe` block has a `// SAFETY:` comment.

## 22. Milestones

| # | Name | Delivers | Proves |
|---|---|---|---|
| 0 | Skeleton | Workspace, CI matrix, `core`, `math`, `task`, `reflect`, tracing, SOUR scaffold, naive `ecs` facade | Makespan divided by critical-path length at P50 and P99 on representative DAGs, on bare-metal performance-plus-efficiency and x86 machines, with and without background load. Wake and steal latency microbenchmarks. Scope and sleep-wake pass loom and Miri. |
| 1 | World | Sim track: `ecs`, `sim`, `physics-api` plus `physics-rapier`, `anim`, `predict`, `persist` (policy, records, container), `asset-api`, `net-api` and `input` types, headless `app`. Render track: RHI spike, `rhi-null`. Asset track: `io`, `asset`, `cook` skeleton. | 100k entities, SOUR in workers, repeat, layout and chaos modes. Physics snapshot conformance. Netcode spike: x86 server, aarch64 client, one predicted character, fifty bodies; correction rate, rollback cost and divergence measured. Scalar precision decided. Physics, snapshot, sim-skeleton evaluation and build times measured. |
| 2 | Pixels | `platform`, `rhi-api`, `rhi-vulkan`, `render-graph`, minimal `renderer`, Slang toolchain, `present-api`, presentation snapshot and schedule | A triangle through the graph on Windows and macOS. MoltenVK and KosmicKrisp spike recorded. Overlap with exclusive extract; extract cost per dirty row measured. lavapipe CI; macOS shader runner. |
| 3 | GPU-driven | GPU scene with cell-plus-offset, cluster submission, culling, deferred tile-local lighting, streaming regions with `persist` overlay against the cooked baseline, `terrain`, pose evaluation, `rhi-metal` | 500k instances with one indirect draw per pipeline on Vulkan and Metal. Regions persist across unload and reload. Skinned characters on terrain. |
| 4 | Network | `net`, interest management, bandwidth prioritisation, reconciliation, resync, late join, client and server `app`, SOUR architecture mode on every PR | Thirty-two simulated clients joining and leaving under injected latency and loss. A recorded server session replayed on aarch64 hashes equal. Solo traversal over static geometry at 100 ms round trip with 1% loss produces fewer than one correction per minute. |
| 5 | Game | `game-sim`, `game-present`, `audio`, `ui`, `nav`, input mapping, `debug-ui` | A playable slice: terrain, climbing, physics objects, co-op interaction, gamepad, sound. |

### 22.1 Decided later, by milestone

| Decision | By |
|---|---|
| Simulation scalar precision | 1 |
| Build-time mitigation if over budget | 1 |
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

Codenames label subsystems on binaries, tools, log targets, CI jobs, README titles and doc
headings. Crate names, types and functions stay descriptive.

| Name | Crate or surface | Role |
|---|---|---|
| LACTOSE | workspace | The engine. Lock-free Asynchronous Chunked Task-Oriented Simulation Engine. |
| CURD | `lactose-sim` | Deterministic simulation core. Concurrent Unified Runtime, Deterministic. |
| WHISK | `lactose-task` | Work-stealing task executor. |
| CASEIN | `lactose-ecs` | Chunked archetype storage and scheduler. |
| COW | `lactose-predict` | Copy-on-write snapshots. Not built unless the milestone 1 measurement of prediction-world snapshot cost exceeds budget. |
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
