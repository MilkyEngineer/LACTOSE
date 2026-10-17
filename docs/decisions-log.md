# LACTOSE Decisions Log

`docs/architecture.md` states what is decided. This log records why: for each decision with a
non-obvious reason, or one that reversed an earlier default, the rationale, the review finding
that drove it, and what it replaced. Where no reason is recorded in the architecture document,
its earlier revisions or the reviews, the entry says so.

Notation: R1-n is finding n of `docs/reviews/review-1-revision-1.md`; R2-n is finding n of
`docs/reviews/review-2-revision-2.md`; R3-n is finding n of `docs/reviews/review-3-revision-3.1.md`.
Revision 1 is commit `a29af04`, revision 2 `f65d2b6`, revision 3 `4c7b609`, revision 3.1 (the
editorial split) `d58492a`, revision 3.2 `0ac9fd0`. Section references (§) are to the current
`docs/architecture.md` (revision 3.2) unless a revision is named. "Recorded in revision 3.1" marks
rationale the author supplied during the editorial pass rather than one taken from a review.
`docs/reviews/traceability.md` maps every finding to where it is addressed.

## 1. Goals and scope

**D1. Determinism is an engineering property, not a prediction guarantee** (§1 goal 2, §2, §9.4)
- Why: bit-exactness pays only when a predicted entity's inputs are identical on both machines,
  and they are not: the client world holds kinematic proxies with different contents and
  insertion history, corrections arrive quantised, and the simulation reads unreplicated state
  (transient columns, contacts, warm-start, timers). What it does buy is zero corrections for
  solo traversal through static geometry when keys match and comparison happens in quantised
  space, plus SOUR, replays and desync debugging. Revision 3.1 therefore reconciled in quantised
  space, with a milestone 4 criterion of a replay hash plus a correction rate. The threshold,
  fewer than one correction per minute during solo traversal over static geometry at 100 ms
  round trip with 1% loss, is the case where bit-exact determinism should yield near-zero
  corrections, so it is the test that determinism is doing its job (recorded in revision 3.1).
  Revision 3.2 reconciles at full precision instead: lost inputs alone exceeded the threshold,
  so inputs carry redundant history and the client simulates the quantised value it sent, and
  once corrections carry every simulation-read field at full precision, comparing in quantised
  space is redundant (R3-17, D77).
- Driven by: R2-7, which disagreed with the framing R1-8 introduced; R3-17 for full-precision
  reconciliation.
- Replaced: revision 2's "cross-architecture determinism is a product requirement" and its
  milestone 4 criterion "x86 server, ARM client, hashes equal"; revision 3.1's reconciliation in
  quantised space (R3-17). Revision 1 had cross-architecture determinism as a non-goal.

**D2. No fixed player cap** (§1 goal 6, §2, §9.1, §9.7)
- Why: a user decision for revision 3 (recorded in revision 3.1). Bounding server cost by active
  entities (through `Frozen` regions) and per-client cost by relevance makes the cap a
  deployment parameter rather than an architectural one. Thirty-two clients in milestone 4 is
  enough to exercise relevance sets and priority accumulators beyond the trivial case while
  fitting a CI runner.
- Driven by: no finding; the user's decision. R3-13 later made the activation radius and the
  join cap the explicit deployment parameters (§9.7, D78).
- Replaced: four-player co-op (revisions 1 and 2); milestone 4's four clients became thirty-two.

**D3. Intel Macs are a non-goal** (§1)
- Why: their argument buffer tier cannot hold the bindless heap (revision 2 §10.1).
- Driven by: R1-12 ("Intel Mac argument buffer limits").
- Replaced: revision 1 supported macOS through MoltenVK without an architecture limit.

**D4. Tracing and reflection from day one** (§3)
- Why: Goal 1 cannot be measured without tracing (revision 2 §3). A type that cannot be
  reflected cannot be serialised, replicated, snapshotted or shown in a future editor
  (revision 2 §3); serialisation and replication need the registry (revision 1 §2).
- Driven by: no finding.

**D72. No Bevy** (§2 Language)
- Why: a user decision at the start (recorded in revision 3.1). The gap analysis found Bevy
  lacks access-derived ordering, a unified work-stealing pool, fibers or continuation
  scheduling, a frame graph with aliasing, cache-sized chunking, immutable assets, parallel
  serialisation and late-latching, and its wgpu backend blocks the render-graph and GPU-driven
  goals.
- Driven by: no finding.

## 2. Identity and ECS

**D5. Process-local `EntityId`; every stored reference is a `StableKey`** (§7, §2 Identity)
- Why: process-local IDs from a block table differ between server and client, because the
  table's state depends on the order regions requested blocks; every ID-keyed rule (RNG, command
  order, physics insertion, sorted reductions) then diverges even with bit-exact floats.
  Per-player reserved blocks forced remapping of predicted IDs already held in components,
  history and sent inputs, and could run out mid-fight. Cooked entities derive `NetId` from
  placement so clients know it without a message. Revision 3.2 makes the stable key an enum
  (`Net`, `Predicted`, `Placeholder`) used by every reference, relationship, render slot, event
  target and RNG key, because references keyed by `NetId` alone made predicted spawns
  unreferenceable and invisible for a round trip, and switching the RNG and ordering key at bind
  mispredicted every random outcome. The server records a predicted spawn's prediction key as
  its permanent origin key, the ordinal is the spawning command's position in the tick's total
  command order, a spawn the server never echoes is destroyed when the processed-input
  acknowledgement passes its input tick, and the alias table is part of the prediction snapshot
  (R3-1). `NetId` doubles as the persistent ID (D7).
- Driven by: R2-1, R3-1, R3-12.
- Replaced: revision 2's 64-bit IDs allocated in 4096-index blocks from a persistent block
  table, with per-player reserved blocks the server confirms or remaps; revision 3.1's three
  identities (`EntityId`, `NetId`, `PersistentId`) with `NetId`-only references and prediction
  keys as a temporary substitute (R3-1, R3-12).

**D6. `EntityId` carries no region bits and is `NonZeroU64`** (§7)
- Why: an 8-bit region in the ID meant an entity could not cross a region boundary without a
  new ID invalidating every reference, and capped regions at 256 (R1-5). `NonZero` makes
  `Option` free (R2-24).
- Driven by: R1-5, R2-24.
- Replaced: revision 1's 32-bit index, 24-bit generation, 8-bit region.

**D7. `NetId` is the persistent ID: cooked from (scene, authored placement GUID, child path), runtime from one server counter at command apply** (§7, §15.5)
- Why: an ID taken from the source asset's GUID collides when a prefab is placed twice or has
  children; a global counter mutated by spawning systems serialises them unless it is assigned
  at apply (R2-12). The placement GUID is authored because placement-index IDs were fragile
  (R3-4). Revision 3.2 drops the separate `PersistentId`: a persistent counter separate from the
  `NetId` counter made the post-load discrete-state test fail by construction, because `NetId`
  was not persisted while RNG streams and ordering are keyed by it, and two server counters were
  redundant (R3-12).
- Driven by: R2-12, R3-4, R3-12. R1-15 had recommended the GUID.
- Replaced: revision 2's GUID-derived cooked IDs and a server counter in global resources;
  revision 3.1's separate `PersistentId` from (scene, placement, child path) with its own server
  counter.

**D8. Typed reference fields remapped on load** (§7, §15.4)
- Why: once references stopped being serialised as `PersistentId` and `AssetId` (a revision 1
  rule that revision 2 dropped), the bulk-copy fast path copied raw `EntityId`s and handles
  into a new process.
- Driven by: R2-12.
- Replaced: revision 2's plain bulk copy.

**D9. Components are `NoUninit + CheckedBitPattern`** (§6, §2 Component data)
- Why: padding bytes made a raw-byte hash nondeterministic (R1-6). `AnyBitPattern` then banned
  `bool`, enums, `Option`, `NonZero` and niche IDs, pushing gameplay code to hand-encode `u8`
  against Goal 3; only untrusted bytes (saves and network input) need validation. `NoUninit`
  implies `Copy`, so handles cannot be refcounted and asset lifetime comes from region asset
  sets (R2-24). Because `NoUninit` forbids the padding that ordinary field orders create, the
  `#[component(key = ...)]` attribute reorders fields by alignment and inserts padding fields
  the registry zeroes and skips, and data-carrying enums are verified against the pinned
  `bytemuck` before they are permitted (R3-24).
- Driven by: R1-6, R2-24, R3-24.
- Replaced: revision 1's plain `#[repr(C)]` structs; revision 2's `NoUninit + AnyBitPattern`;
  revision 3.1's manual padding (R3-24).

**D10. Per-row change ticks, a structural change log and per-consumer read state** (§6, §2 Change tracking)
- Why: per-column, per-chunk ticks amplified sparse writes about 100 times in replication
  (R1-10). A per-row dirty bit carries no time, so extract (every tick) and clients with
  different acknowledged ticks cannot share it: whoever clears it first hides the change from
  everyone else (R2-3). A tick per row lets each consumer read rows newer than its own read
  state, at 4 bytes per row per tracked column. `set_if_neq` keeps equal rewrites out of the
  stream; slices take an explicit dirty range because whole-column slices otherwise bypass
  tracking (R2-19). Saves do not need change tracking because they diff against the cooked
  baseline (R2-3). R2-3's single last-seen tick per client, with no per-packet bookkeeping,
  cannot represent partial sends under a bandwidth cap, so replication keeps a per-client,
  per-entity acked tick (R3-3, D76). The coarse chunk tick is written once at dispatch because a
  split job writing it was a data race; despawns had no row to carry a tick, hence the
  structural change log; the tick unit is the simulation tick, clamped forward periodically
  (R3-7).
- Driven by: R1-10, R2-3, R2-19, R3-3, R3-7.
- Replaced: revision 1's per-column, per-chunk ticks; revision 2's per-row dirty bitmask read by
  replication, extract and persistence; revision 3.1's one last-seen tick per client, per-write
  coarse chunk tick and no structural log (R3-3, R3-7).

**D11. 16 KiB chunks, with SOUR varying capacity** (§6, §8.3)
- Why: the size is borrowed from Unity DOTS; a wrong size costs only performance, provided SOUR
  varies capacity.
- Driven by: R2-30, R2-15.
- Replaced: revision 2's open spike "chunk size per archetype versus fixed 16 KiB", removed in
  revision 3.

**D12. Owned entities migrate with their owner** (§6)
- Why: inventory as entities, plus region-owned entities, plus weak cross-region references,
  stranded a player's inventory in the region they left when it unloaded.
- Driven by: R2-12.

**D13. `Replicated` region mode written only at the input sync point** (§6, §9.2)
- Why: client regions that receive server state need a mode in which only replication writes
  rows (R1-14), and nothing may write simulation rows concurrently with systems or extract
  (R2-21).
- Driven by: R1-14, R2-21.
- Replaced: revision 2's `Replicated` mode in which replication wrote rows with no defined
  point.

**D73. A worked system API that the week-one facade implements** (§6.1, §20)
- Why: the week-one facade freezes the API in practice, so without a specification every agent
  would invent one. Continuation completion forces per-entity kernels, and exclusive systems,
  cross-entity writes, physics writes, event emission, command order and the key hash were
  undecided (R3-2). The `worlds` declaration selects where a system runs and the prediction
  world injects its own filter (R3-5, D32).
- Driven by: R3-2, R3-5.
- Replaced: nothing; revision 3.1 had no system API section.

## 3. Runtime model

**D14. Fork-join closures and `'static` futures as separate primitives** (§5.1)
- Why: a stackless async task cannot fork children that borrow its locals with a non-blocking
  parent, because the parent future can be dropped or forgotten while children still hold the
  borrows. The sound alternatives were unsafe at the call site, children only inside the
  parent's poll, or blocking.
- Driven by: R1-1.
- Replaced: revision 1's `Scope::spawn` of futures borrowing the parent's stack.

**D15. Systems complete by continuation; a waiting opener helps only its own scope** (§5.1)
- Why: a worker that steals unrelated work while waiting on a join nests a whole stolen system
  on its stack (worst case the physics step), and can pick up background work that nothing
  preempts. Both lengthen the critical path. Rayon has the same property. Child jobs come from a
  per-worker bump arena reset when the scope closes, because Rust has no stack allocation for an
  unbounded child count, and the opener records a deque watermark at scope open so it never runs
  older unrelated work (R3-25).
- Driven by: R2-2, R3-25.
- Replaced: revision 2's opener that "steals other work until the join counter reaches zero",
  with steal priority own frame-critical, others' frame-critical, then background; R2-2's
  stack-allocated child jobs (R3-25).

**D16. Unconditional abort and a closure-only scope** (§5.1)
- Why: Cargo ignores `panic = "abort"` for tests and benches, Miri and loom unwind, and a panic
  in a `std::thread` worker unwinds only that thread. Without a guard a panicking child leaves
  the counter above zero and the opener hangs, and an opener that unwinds frees a frame others
  still borrow. A guard-object scope is unsound under `mem::forget`. Blocking waits on a worker
  hang in SOUR's one-worker mode instead of failing. The panic hook is installed by the binaries
  only, because a global abort hook broke `#[should_panic]` and property-test shrinking; a test
  that expects an abort runs the binary in a subprocess (R3-23).
- Driven by: R2-9, R3-23.
- Replaced: revision 2's "a panic in any task aborts the process after writing a crash dump",
  which relied on the panic profile; revision 3.1's global abort hook (R3-23).

**D17. Worker classes on performance and efficiency cores; split-on-steal** (§5.1)
- Why: physical cores minus one puts frame-critical tasks on efficiency cores at two to three
  times the latency on Apple Silicon and Intel hybrid CPUs, and macOS offers only QoS, not
  pinning. A fixed 50 to 200 microsecond batch target conflicts with Goal 1: a 300 µs system
  split in two adds more to the critical path than fifteen 20 µs tasks on idle workers. The
  0.5 ms background yield keeps background work from stalling frame-critical joins (R2-2).
  Frame-critical workers equal the performance-core count on every platform because physical
  cores minus one places frame-critical work on efficiency cores on hybrid x86 as well; the
  fallback applies only where no core-type information exists. Two background threads on
  homogeneous CPUs are enough to keep asset decode and compression flowing without
  oversubscribing frame-critical workers; the OS scheduler handles the small overcommit at low
  priority (both recorded in revision 3.1).
- Driven by: R2-23, R2-2.
- Replaced: revision 2's one worker per physical core minus one, two deques per worker, and 50
  to 200 µs batches.

**D18. Timer root, present thread, futures for main-thread requests, time scale** (§5.1, §11)
- Why: with no coordinator and a main thread that only pumps events, something must wake at tick
  deadlines, cap catch-up and pace frames. A worker waiting synchronously on a main-thread job
  while the main thread blocks in present (FIFO present, or `nextDrawable` for up to a second)
  on a frame that needs that worker deadlocks. Slow motion affects every player under a global
  time scale, and pause in multiplayer needed a defined meaning. The catch-up cap stops a hitch
  spiralling into ever-longer frames, and four ticks (about 67 ms) is the largest cap that keeps
  a frame under the display interval on the target hardware (recorded in revision 3.1). Beyond
  the cap time is never slowed: a client re-syncs its clock and resends inputs, because slowing
  a client's time made it fall behind the server and cascade into corrections, and a server
  climbs the overload ladder (D78), because slowing time was its only overload response and
  slowed it for everyone. Clock sync is expressed in simulation time, and catch-up and
  re-simulation ticks share the cap (R3-20).
- Driven by: R2-22, R3-20.
- Replaced: revision 2's "fixed 60 Hz on its own cadence" and a main thread that pumps events
  and presents frames; revision 2 had also dropped revision 1's main-thread lane; revision 3.1's
  slowing of simulation time beyond the cap (R3-20).

**D19. Sync point count fixed per schedule; system sets; locks inside RHI backends** (§3, §5.2)
- Why: revision 1's "exactly four sync points", ordering without system sets, and a `Mutex`
  rule contradicted by Vulkan queue submission were inconsistencies.
- Driven by: R1-22.
- Replaced: revision 1's four fixed sync points, pairwise `before`/`after` constraints and no
  lock exception.

**D20. Stable system keys and a total command order** (§5.2, §7)
- Why: (system order, entity ID) is not a total order: one system can issue several commands for
  one entity and several sources can target one entity, so ties fell to task order. Registration
  order became semantic, so a client with extra systems or a `cfg`-gated plugin changed results;
  the same held for system IDs in RNG keys. Spawns had no ID yet. Revision 3.2 applies commands
  in the stage's execution order first, then (source stable key, per-invocation sequence),
  because applying in key order rather than execution order made a later-running system's
  removal land before an earlier system's addition (R3-2).
- Driven by: R2-14, R3-2.
- Replaced: revision 2's topological sort keyed on registration order and command order (system
  order, entity ID); revision 3.1's key-only order (R3-2).

**D21. Exclusive, internally parallel extract into a staging ring** (§5.3)
- Why: extract cannot overlap the next tick if it reads chunks the tick writes (R1-19). A
  serial 1 ms extract fails whenever everything is dirty (region load, rollback, origin rebase,
  device loss): at the review's estimate of 20 to 40 ns per dirty row, 1 ms covers about 25k to
  50k rows. Writing straight into the GPU scene races up to three frames in flight. A row that
  stopped moving still needs its previous transform set, hence rows dirty in either of the last
  two ticks (R2-18).
- Driven by: R1-19, R2-18.
- Replaced: revision 1's extract overlapping simulation; revision 2's "serial phase, budgeted
  at one millisecond", uploading directly to the GPU scene.

**D74. A vetted deque, loom on the latch and sleep protocol, spans per system and per range** (§5.1, §20)
- Why: loom cannot model a deque, so the deque is `crossbeam-deque`, loom models only the latch
  and the sleep-and-wake protocol, and aarch64 stress tests cover the weak-memory behaviour loom
  does not model. Per-job spans cost more than the jobs, so spans are emitted per system and per
  stolen range and finer levels compile out (R3-23).
- Driven by: R3-23; R2-9 for wider loom coverage.
- Replaced: revision 3.1's per-job spans.

## 4. Determinism

**D22. The boundary is a crate boundary** (§3, §4, §8.1)
- Why: the boundary was drawn per system but clippy configuration is per crate; `net` held both
  RTT measurement (needs `Instant`) and rollback (bans it); `nav`'s deterministic half was
  placed in two different crates; terrain collision came from a backend crate outside the
  boundary. The `[boundary]` list in `crate-tiers.toml` is the authoritative statement and the
  architecture references it, because the prose and the file disagreed and some crates had no
  tier; the file gained leaf and game sections, `game-content` joined the boundary, the `ecs` to
  `task` edge was reworded, and the world hash and bulk copy are generated into `sim` (R3-15).
- Driven by: R2-20, R3-15.
- Replaced: revision 2's boundary of "`net` prediction and rollback, the deterministic half of
  `nav`, and every `game` simulation system"; revision 3.1's boundary list restated in prose
  (R3-15).

**D23. Chunk, row and worker indices unreachable; reductions through combinators** (§5.2, §8.2)
- Why: results depended on chunk layout (reductions in chunk order, per-chunk command buffers,
  scratch keyed by chunk index), and a client builds regions in a different order from the
  server (R1-3). Tests cannot prove "never a semantic key", so it is enforced by construction;
  scratch "keyed by entity range" was meaningless because a chunk's rows do not form an ID range
  (R2-15). Float addition is not associative, so no order-independent float combiner exists:
  float sums use an exact accumulator, and a reduction that needs stable-key order is declared
  `ordered` and pays a parallel sort in finalize rather than a serial one. Finalize may open a
  scope, and dependents are released as range jobs (R3-11).
- Driven by: R1-3, R2-15, R3-11.
- Replaced: revision 1's reductions in chunk index order and scratch keyed by chunk index;
  revision 2's entity-ID-sorted reductions and scratch keyed by entity range; revision 3.1's
  "order-independent combiner" (R3-11).

**D24. Counter-based RNG keyed by (region seed, tick, system key, stable entity key)** (§8.2)
- Why: per-entity keys remove dependence on chunk layout (R1-3); keys must be the same on every
  machine (R2-1); a counter-based generator has no state, so a snapshotted "RNG state" was
  inconsistent (R2-14). The stable entity key is the origin key: the `NetId`, or the
  `PredictionKey` the server records permanently for a predicted spawn, because switching the
  key at bind mispredicted every random outcome (R3-1, D5).
- Driven by: R1-3, R2-1, R2-14, R3-1.
- Replaced: revision 1's region-local stream seeded from region seed and tick; revision 2's key
  of (region seed, tick, system ID, entity ID); revision 3.1's `NetId` key with the prediction
  key as a temporary substitute (R3-1).

**D25. Float rules for `min`, `max`, NaN, `powi` and the FP control register** (§8.2, §5.1)
- Why: `f32::min` and `max` may return either input for ±0, and x86 and AArch64 differ; NaN sign
  and payload are nondeterministic and leak through `total_cmp`, `to_bits`, `copysign` and
  `is_sign_negative`; `powi` is documented as precision-nondeterministic. MXCSR and FPCR are
  per-thread state that audio, codec and driver code sharing the workers may change.
- Driven by: R2-11.
- Replaced: revision 2's rules, which allowed SIMD `min` and `max` and did not cover signed zero,
  NaN sign, `powi` or the control register.

**D26. Fused multiply-add permitted; the x86-64-v3 pin enforced by `compile_error!`** (§8.2)
- Why: rustc never contracts on stable, so "FMA contraction is disabled by compiler flags" named
  flags that do not exist (R1-8). Fused multiply-add is correctly rounded and identical on both
  ISAs; the hazard is `wide` and glam choosing fused or unfused by target feature, which makes
  the pin load-bearing. The pin is silently dropped when `RUSTFLAGS` or
  `CARGO_ENCODED_RUSTFLAGS` is set or cargo runs outside the workspace, hence the
  `compile_error!` (R2-11). The pin is set through target-specific rustflags in
  `.cargo/config.toml`, never the environment variable, because `RUSTFLAGS` replaces them; build
  scripts and proc macros may not depend on boundary crates because they compile for the host
  without the pinned target features (R3-23).
- Driven by: R1-8, R2-11, R3-23.
- Replaced: revision 1's compiler-flag claim; revision 2's ban on `mul_add`.

**D27. `math` does not use glam; one `libm`** (§8.2, §4)
- Why: parry's determinism feature turns on glamx `scalar-math` and `libm`, and Cargo applies
  that to every user of the same glam version in a build, so a glam-based `math` computes
  differently in a single-crate test build than in the shipped app. glam's SIMD dot product sums
  in a different order on SSE2 and NEON. Two `libm` versions would break agreement between
  simulation and physics on transcendentals.
- Driven by: R2-11; R1-7 for sharing Rapier's `libm`.
- Replaced: revision 2's `math` with no statement about glam.

**D28. Runtime CPU detection banned only where it selects different numerics** (§8.2)
- Why: `libm` itself detects x86 features at runtime, so a blanket ban banned the crate the
  document mandates; all of its paths are correctly rounded.
- Driven by: R2-11.
- Replaced: revision 2's "no runtime CPU feature detection in boundary crates or their
  dependencies".

**D29. `world_hash` as an order-independent fold of per-entity hashes** (§8.3, §8.1)
- Why: hashing raw `repr(C)` bytes included padding (R1-6); hashing columns in storage order
  makes correct code fail layout mode (R2-15). Revision 3.1 hashed through reflection in `NetId`
  order, which cost tens of milliseconds, so revision 3.2 hashes each entity with a
  monomorphised per-type hasher generated into `sim` and folds the results through an
  order-independent combiner at sync points, which needs no sort (R3-19).
- Driven by: R1-6, R2-15, R3-19.
- Replaced: revision 1's hash over all chunk data; revision 2's field-by-field hash in storage
  order; revision 3.1's sorted reflection hash in `NetId` order on every tick (R3-19).

**D30. Identity-preserving layout perturbations, a chaos mode, a Haswell-masked run** (§8.3)
- Why: revision 2's shuffled insertion and region-load order changed the IDs entities received,
  so ID-ordered behaviour changed legitimately and produced false failures. One versus N workers
  on 4-vCPU CI runners finds far less than a seeded chaos schedule (R2-15). The Haswell-masked
  run is part of the architecture matrix R2-11 asked for. The full matrix exceeded the
  pull-request budget, so a pull request runs workers mode and one layout capacity and the rest
  runs nightly (R3-19). Layout and chaos are runtime options on the world and executor builders,
  with a recorded job log, because `cfg(test)` and Cargo features cannot reach dependencies
  (R3-28).
- Driven by: R1-3, R2-15, R2-11, R3-19, R3-28.
- Replaced: revision 2's layout mode (shuffled insertion, forced compaction, different region
  load order) and a two-runner architecture mode.

**D31. Stable clippy and `cargo-deny` instead of a custom lint crate** (§8.3)
- Why: a custom lint crate would require nightly, against the stable toolchain decision (R1-9;
  revision 2 §7.4). Clippy never lints dependencies, so `cargo-deny` allowlists them (R2-11).
  Miri runs on nightly as the one exception to the stable-toolchain rule, because Miri needs
  nightly (R3-23).
- Driven by: R1-9, R3-23.
- Replaced: revision 1's `determinism` lint crate.

**D75. Resolved feature sets compared per binary and included in the build ID** (§8.3, §9.1)
- Why: Cargo resolves features per build, so a client-only feature could change boundary
  numerics invisibly, and a lockfile-only build ID cannot see it. CI dumps
  `cargo tree -e features` for the boundary dependencies of each shipped binary and of the SOUR
  test crate and requires them identical, the SOUR test crate's dependency set mirrors the
  server binary, and the resolved feature set is part of the boundary build ID (R3-9).
- Driven by: R3-9; R2-11 for feature unification.
- Replaced: revision 3.1's lockfile-only build ID.

## 5. Networking and snapshots

**D32. A separate client prediction world** (§9.4, §2 Netcode)
- Why: region modes apply per region, so a `Simulated` client region ran every system on every
  entity while replication also wrote those rows: two writers per row. Snapshotting whole
  regions plus a Rapier clone every tick cost plausibly 5 to 20 MB per tick across a ring of up
  to 40. The server had nothing to resync a client area from. Revision 3.2 specifies how the
  prediction world runs, because its system selection, its coexistence with the replicated
  world, presentation's collision queries and re-emitted events were unspecified: systems
  declare their worlds and the prediction world injects a predicted-only filter, the prediction
  copy of the local player shadows the replicated copy, static collision is an immutable shared
  asset, presentation events deduplicate across rollback, proxies follow per-tick history kept
  in the ring, and Rapier's internal solver state is not corrected (R3-5).
- Driven by: R2-6, R3-5; R1-18 for predicted entities colliding only with static geometry and
  each other.
- Replaced: revision 2's predicted active area of client-simulated regions; revision 1's
  prediction region.

**D33. Ring depth derived from the re-simulation budget; rebuild from replicated state** (§9.4)
- Why: a 12-tick ring is 200 ms, short of real round trips (R1-18). A correction k ticks deep
  costs k times (systems, physics step, snapshot) in one frame, so the cap must come from the
  budget and the measured per-tick cost; nothing derived revision 2's 40-tick cap (R2-6).
- Driven by: R1-18, R2-6.
- Replaced: revision 1's 12-entry ring; revision 2's RTT-sized ring capped at 40 ticks with
  "requests a full area snapshot".

**D34. Copy-on-write snapshots in `lactose-predict`, built only if measured necessary** (§23 COW, §10, §22)
- Why: not built up front. With a prediction world holding only predicted entities, a full copy
  of its mutable columns is small. R2-6 asked for copy-on-write from day one; the author kept it
  measured because the smaller world removes the cost that motivated it (recorded in
  revision 3.1). It is built only if the milestone 1 measurement of prediction-world snapshot
  cost exceeds its budget.
- Driven by: R2-6, whose day-one recommendation is rejected for this reason.
- Replaced: "copy-on-write chunk snapshots, a later optimisation" in `lactose-sim` (revisions 1
  and 2).

**D35. Staged network data and a presentation-side interpolation buffer** (§9.2, §9.3)
- Why: nothing defined when received inputs and state were applied, so they could race systems
  and extract. Interpolating 20 to 30 Hz snapshots needs a per-entity buffer sampled about
  100 ms in the past; writing replicated rows directly steps visibly, and interpolating into
  rows every tick dirties every remote entity every tick (R2-21). A predicting client must run
  slightly ahead and adjust its tick rate by one to three percent as RTT changes (R2-22). The
  interpolation buffer is owned by `net` because it is network state outside the boundary and
  sampled per frame; the presentation schedule reads it through a `net-api` resource rather than
  having it copied into the presentation snapshot at extract, as R2-21 had proposed (recorded in
  revision 3.1).
- Driven by: R2-21, R2-22.
- Replaced: revision 2's "send rate 20 to 30 Hz, interpolated on the client" with no staging
  point, input policy or clock sync.

**D36. Lag compensation covers NPCs** (§9.5)
- Why: in PvE co-op the targets that need rewinding are NPCs. The 250 millisecond cap covers a
  200 ms round trip plus one send interval; beyond it the server stops rewinding and the client
  is told, so a high-latency player cannot hit targets that moved a quarter second ago (recorded
  in revision 3.1).
- Driven by: R2-21.
- Replaced: revision 2's "player transforms and colliders only".

**D37. Proxies in the past are an accepted limitation** (§9.4)
- Why: kinematic proxies sit about half a round trip plus the interpolation delay in the past
  while the local player is ahead, so co-op pushing fights; the review asked for this to be
  named with a mitigation, which is tentative presentation until confirmed.
- Driven by: R2-21.

**D38. Handshake includes a protocol hash over replicated and boundary types, a boundary build ID and a cooked-data hash** (§9.1, §6, §7)
- Why: "schema hash plus `net` version" let builds with different gameplay code, Rapier, glamx
  or `libm` versions, or different cooked simulation data, connect and silently mispredict.
  Revision 3.2 hashes only replicated and boundary component types, keyed by explicit component
  keys, because registration-order type IDs differ between client and server, whose presentation
  types differ by design, so a registry-wide schema hash would refuse every connection (R3-8).
  The build ID includes the resolved feature set (R3-9, D75).
- Driven by: R2-30, R3-8, R3-9.
- Replaced: revision 2's registry schema hash and `net` crate version; revision 3.1's
  registry-wide schema hash and lockfile-only build ID (R3-8, R3-9).

**D39. The server holds a player at a region boundary until the client is ready** (§14)
- Why: accepting a correction storm instead was considered and rejected (revision 2 §8).
- Driven by: R1-14.
- Replaced: revision 1, which did not specify client gating.

**D40. Snapshot contents; the server never rolls back** (§10)
- Why: a chunk copy misses state outside chunks: Rapier body sets, contact manifolds, warm-start
  impulses, island sleep state, joints, the relationship reverse index and in-flight command
  buffers; restoring chunks without the physics world produces different results (R1-4).
  Snapshots are per world, and only the prediction world is rolled back (R2-1). The
  counter-based RNG has no state to snapshot (R2-14). Revision 3.2 adds the
  prediction-to-network alias table (R3-1, D5), proxy history and the presentation event log
  used for deduplication (R3-5, D32) to the prediction snapshot.
- Driven by: R1-4, R2-1, R2-14, R3-1, R3-5.
- Replaced: revision 1's "a chunk copy and takes microseconds"; revision 2's `snapshot(area)`
  including the block cursor and RNG state.

**D76. A replication gather phase and per-client, per-entity acknowledgement** (§9.3)
- Why: a single last-seen tick per client cannot represent partial sends under a bandwidth cap,
  and the server had no defined read phase. A gather phase, exclusive and parallel like extract,
  copies advanced rows and the structural log into a per-tick staging buffer that per-client
  packing reads off the critical path; each client has a per-entity acked tick and a ring of
  packet-to-entity lists, so an acknowledgement advances exactly the entities it covered;
  entities travel under short per-connection handles, and destroyed entities as tombstones from
  the structural log (R3-3).
- Driven by: R3-3.
- Replaced: one last-seen tick per client and "no per-packet bookkeeping" (R2-3, D10).

**D77. Redundant inputs, quantised client inputs and full-precision reconciliation** (§2 Netcode, §9.2, §9.4)
- Why: lost inputs alone exceeded the milestone 4 correction threshold, so each input packet
  carries the last N inputs, and the client quantises inputs before simulating so it simulates
  exactly the value the server receives. Comparing in quantised space was redundant once
  corrections carry every simulation-read field at full precision, so reconciliation compares
  per field at full precision (R3-17).
- Driven by: R3-17; R2-7 earlier (D1).
- Replaced: revision 3.1's quantised-space comparison and inputs without redundancy.

**D78. A server capacity model: activation radius, archetype-owned chunks and an overload ladder** (§9.7, §6, §5.1, §9.3)
- Why: per-region chunks fragmented storage when players scattered, and the only overload
  response was slowing time for everyone. Chunks belong to archetypes and region membership is a
  component, so scattered players add rows, not mostly-empty chunks; a region is `Simulated`
  within an activation radius of any player; the overload ladder shrinks the radius, decimates
  outer-ring non-player systems by tick parity and refuses joins, and server time is never
  slowed. Server IO has its own blocking pool so region publish latency does not grow with
  player count, packets are parsed on a dedicated network thread at frame-critical priority, an
  entity entering a `Frozen` or unloaded region is frozen at the boundary, and asset publish
  timing is journaled as an input (R3-13, R3-20).
- Driven by: R3-13, R3-20.
- Replaced: revision 3.1's region-owned chunks and catch-up slowdown as the server's overload
  response.

**D79. Interest management in the first networking deliverable** (§2 Netcode, §22)
- Why: "from day one" meant designed into the first network code rather than bolted on, and
  milestone 4 is the first networking deliverable; the wording was clarified (recorded in
  revision 3.1).
- Driven by: no finding.

## 6. Rendering and presentation

**D41. Grid cell plus offset in the GPU scene; a typed render space** (§12.4)
- Why: camera-relative positions produced at extract change for every instance whenever the
  camera moves, so every tick would rewrite every instance's transforms (500k instances times
  about 96 bytes is about 48 MB per tick). The cell difference is small, so subtracting in the
  shader is precise in f32. A distinct type prevents implicit conversion from simulation
  scalars.
- Driven by: R2-4.
- Replaced: revision 2's camera-relative 32-bit positions produced at extract.

**D42. One indexed indirect draw per pipeline over a compacted index buffer** (§12.4, §2 Draw submission)
- Why: Metal has no indirect-count (R1-12). On MoltenVK, multi-draw-indirect runs a CPU loop
  encoding one Metal draw per command, and emulated indirect-count still encodes the maximum
  draw count, so zero-instance draws cost CPU encode and GPU front-end time proportional to the
  maximum every frame. R2-5 prescribed one indirect draw per pipeline with programmable vertex
  pulling over fixed-size clusters, which behaves the same on every backend and needs no
  indirect-count. Revision 3.2 reverses the vertex-pulling part of that prescription:
  non-indexed cluster draws lost post-transform vertex reuse at roughly five times the vertex
  work, so the cull pass compacts surviving clusters' triangles into one 32-bit index buffer per
  pipeline over a global vertex pool and issues one indexed indirect draw per pipeline. Skinned
  instance bounds are inflated, foliage deforms in the vertex shader, and LOD is per instance in
  stage one (R3-21).
- Driven by: R2-5, R3-21; R1-12 earlier.
- Replaced: revision 1's multi-draw indirect with count buffers; revision 2's zero-instance
  draws as the primary path; revisions 3 and 3.1's fixed-vertex-count clusters with degenerate
  padding (R3-21).

**D43. Native Metal in milestone 3; `rhi-api` unstable until the Metal spike** (§2 Graphics, §12.1, §20)
- Why: the GPU-driven path is CPU-bound on MoltenVK (R2-5). Freezing `rhi-api` after Vulkan
  alone means Metal's residency, argument buffers, heaps and indirect command buffers force
  breaking changes later (R2-10).
- Driven by: R2-5, R2-10.
- Replaced: revision 2's macOS through MoltenVK with native Metal "second", and every `-api`
  frozen once its first backend passed conformance.

**D44. Slang emits MSL and metallib directly; pipeline caches persisted** (§12.2, §2 Shaders)
- Why: SPIR-V to Metal at cook time needs Apple's `metal` compiler, which does not run on Linux.
  MoltenVK converts SPIR-V itself, so cooked metallibs serve `rhi-metal`. Drivers compile at
  pipeline creation on every backend, so "no runtime shader compilation" was false. Metal offers
  only non-returning 64-bit atomic min and max, from Apple8 (M2); M1 has none, hence the
  stage-three capability gate.
- Driven by: R2-27; R1-12 for choosing a shader toolchain at all.
- Replaced: revision 2's metallibs produced from SPIR-V at cook time and "no runtime shader
  compilation".

**D45. Metal backends handle aliasing and residency explicitly** (§12.1)
- Why: automatic hazard tracking does not cover heap-aliased transients or argument-buffer
  resources, which need residency calls.
- Driven by: R2-27.
- Replaced: revision 2's "the backend emits barriers or relies on Metal's hazard tracking".

**D46. Deferred lighting with the G-buffer in tile memory** (§12.5, §12.1)
- Why: the uber-shader needed a lighting architecture (R1-23). On tile-based GPUs an off-chip
  G-buffer costs about 150 MB per frame at 1440p, about 9 GB/s of M1's 68 GB/s, so the RHI
  gained memoryless attachments and pass-local reads, which a "what all APIs share" rule had
  excluded (R2-28).
- Driven by: R1-23, R2-28.
- Replaced: revision 2's deferred G-buffer with no tile-memory requirement.

**D47. Pass type is a permutation axis; 96 pipelines created at load** (§12.5)
- Why: fixed axes are a hard constraint of GPU-driven batching and of MoltenVK's per-pipeline
  compile cost (revision 2 §10.5). The material axes omitted pass variants; the review estimated
  about 64 to 80 graphics pipelines (R2-28). Motion vectors are a G-buffer target rather than a
  pass type, because a motion-vector pass doubled geometry work (R3-21), so the axes give 48
  graphics pipelines. The budget of 96 leaves headroom for post-processing, compute, UI and
  debug pipelines, kept small because MoltenVK compiles each at creation (recorded in
  revision 3.1).
- Driven by: R1-23, R2-28, R3-21.
- Replaced: revision 2's four material axes only; revision 3.1's motion-vector pass type, which
  gave 64 pipelines (R3-21).

**D48. Late-latch the full orbit transform; the camera rig is presentation** (§5.3, §13)
- Why: latching rotation alone turns a third-person orbit camera about the wrong point (about
  40 cm off-centre on a fast flick) and keeps a stale, collision-shortened boom. "The yaw
  actually displayed" is unknowable when input is sampled, so input carries absolute yaw and
  pitch. CPU light binning in view space and pre-latch cascades are invalid after the latch, and
  mouse rotation per frame is unbounded, hence world-space GPU binning, padding and a clamped
  delta. Revision 3.2 bins lights in camera-cell-relative space because "world space" read as
  absolute 32-bit positions, pads only CPU cascade fitting because GPU culling reads the latched
  buffer, and rechecks boom collision at latch time because it was evaluated before the latch
  (R3-29).
- Driven by: R1-11, R2-17, R3-29.
- Replaced: revision 2's rotation-only latch, camera collision in the simulation and inputs
  stamped with the displayed yaw; revision 1's camera and view matrices patched before
  recording; revision 3.1's world-space light binning and culling pad (R3-29).

**D49. Render slots in a renderer-owned side table** (§12.4)
- Why: as a derived column in simulation chunks, a rollback restored chunks but not the
  renderer, leaking or aliasing slots, and a CPU slot column contradicted the GPU-side free
  list. Revision 3.2 allocates slots on the CPU in the renderer's slot table, because the CPU
  must know slot indices to write updates and the GPU free list still contradicted a CPU slot
  table (R3-29); slots are keyed by `StableKey` (R3-1) and freed from the structural change log
  (R3-7).
- Driven by: R2-18, R3-29.
- Replaced: revision 2's render instance slots as a `derived` column; revision 3.1's GPU free
  list (R3-29).

**D50. A presentation interface crate, snapshot and schedule** (§13, §5.3)
- Why: gameplay could depend only on foundation crates and none described presentation, so it
  could not spawn VFX, play audio, drive UI or emit debug draw. During the overlap nothing was
  safe to read for camera smoothing, UI animation, the audio listener, markers or per-frame pose
  evaluation, without which skinned characters judder on 120 and 144 Hz displays. Presentation
  owns pose evaluation and the renderer owns skinning: pose evaluation is CPU work on the
  presentation snapshot per frame and skinning is a GPU compute pre-pass, so splitting by where
  the work runs keeps `presentation` free of RHI types (recorded in revision 3.1).
- Driven by: R2-16.
- Replaced: revision 2's "the render side holds no CPU mirror of simulation data".

**D51. A null RHI backend** (§12.1)
- Why: graph tests, and a server binary that must not link Vulkan, need a headless RHI.
- Driven by: R2-26.

**D80. Remote transforms upload through the staging ring; a triple-buffered presentation snapshot** (§5.3)
- Why: the client's dominant render data path, remote entities, bypassed the specified pipeline,
  and interpolation needs three snapshot buffers so a frame can interpolate between two while
  extract writes a third. The presentation schedule samples the interpolation buffer each frame
  through a lock-free `net-api` resource (a seqlock per entity with a batch sampling call) and
  uploads remote transforms through the same staging ring as extract; the client's replicated
  world runs no simulation systems (R3-14).
- Driven by: R3-14; R2-16 earlier.
- Replaced: revision 3.1's double-buffered snapshot and unspecified remote upload.

## 7. Assets and persistence

**D52. Asset versions are inputs; hot reload is server-coordinated and tick-stamped** (§8.2, §14)
- Why: an atomic pointer swap is observable mid-tick (R1-14); in multiplayer, simulation assets
  must change at the same tick on every machine (R2-30).
- Driven by: R1-14, R2-30.
- Replaced: revision 1's publication by atomic pointer swap.

**D53. `io` is a foundation crate with a blocking-pool reference backend** (§4, §14)
- Why: kqueue is not completion-based file IO on macOS and io_uring is unavailable in Docker CI
  (R1-16). As a backend crate, `io` made the edges `asset` to `io` and `persist` to `io`
  illegal (R2-20).
- Driven by: R1-16, R2-20.
- Replaced: revision 1's io_uring, IOCP and kqueue behind one completion API; revision 2's `io`
  as a backend crate.

**D54. Transient means written before read every tick; accumulated state is persistent** (§15.1)
- Why: a loaded process started with default transient columns and a fresh physics world, so the
  save/load comparison passed only for a world at rest. Animation phase drives hit frames and
  body velocity lives only inside Rapier, so neither is a cache; a falling boulder would stop
  mid-air after a load. Transient columns are zeroed by the ECS at the input sync point every
  tick and the scheduler checks statically that every reader is ordered after a declared writer,
  because R2-13's poison test detected almost nothing (R3-22).
- Driven by: R2-13, R3-22.
- Replaced: revision 2's transient as "default-initialised on load", with velocity caches and
  animation phase as examples; revision 3.1's poison test (R3-22).

**D55. Typed save records; a patched baseline keeps runtime entities** (§15.2)
- Why: (PersistentId, field ID, value) entries cannot express runtime spawns, destroyed cooked
  entities, added or removed components, or region moves, and "drops the rest" on a patched
  baseline deleted every runtime-spawned entity, players included. A cooked entity that moves
  between regions becomes `Destroy` in the source region and `SpawnFromBaseline` with its
  current fields in the destination, because a move recorded only in the source region lost the
  entity (R3-4).
- Driven by: R2-12, R3-4; R1-15 earlier.
- Replaced: revision 2's field-value overlay entries and its drop rule; revision 3.1's
  `SetField` region moves (R3-4).

**D56. A save copies persistent columns only** (§10, §15.3)
- Why: a full rollback snapshot of every area, including a physics clone a save never
  serialises, stalls all players for milliseconds.
- Driven by: R2-13.
- Replaced: revision 2's "snapshot every active area"; revision 1's claim that the snapshot "is
  a chunk copy and takes microseconds".

**D57. The save/load test reloads in place** (§15.5)
- Why: comparing hashes at the save tick cannot catch a component wrongly marked transient
  (R1-15). Comparing a loaded process with an un-reloaded original compares different states (no
  manifolds, warm-start or sleep state), so both sides reload (R2-13). The comparison of the
  fresh process against the un-reloaded original, bit-exact for discrete state, holds only
  because `NetId`s and the server counter are persisted (R3-12, D7).
- Driven by: R1-15, R2-13, R3-12.
- Replaced: revision 1's hash equality at T; revision 2's comparison at T+K against the
  original.

**D81. Region overlays reach clients on relevance; the record format lives in `bundle`** (§9.3, §15.2, §4, §14)
- Why: clients built regions from the unmodified cooked baseline and would show destroyed trees,
  so the server sends a region's overlay, or a digest the client verifies, when the region
  becomes relevant, and the client applies it before uploading any static instance. `persist`
  could not reach the baseline under the tier rule, so the record format moved into the
  foundation crate `bundle`, shared by `cook`, `asset`, `persist` and `net`. A region's asset
  set includes the assets of migrated resident entities (R3-4).
- Driven by: R3-4.
- Replaced: revision 3.1's overlay that never left the server.

## 8. Physics and animation

**D58. One physics world per server process, determinism and parallel together, pinned exactly** (§16, §2 Physics)
- Why: Rapier since v0.35 combines its determinism feature with its parallel solver (re-checked
  per `docs/verification.md`). Per-area worlds with merge and split lose contact manifolds,
  warm-start, sleep state and joints on every merge, raise spurious contact events, couple
  unrelated players and thrash without hysteresis. Rapier shipped five breaking releases in
  2026, including snapshot format breaks, so any version bump is a protocol and replay break.
- Driven by: R2-8, superseding R1-7's serial-per-world recommendation.
- Replaced: revision 2's one serial physics world per active area; revision 1's engine-driven
  island solving.

**D59. Simulation scalar is a `math` newtype, 64-bit by default, decided in milestone 1** (§2 World coordinates, §8.2, §16)
- Why: R2-8 asked for the precision to be decided by a test at maximum extent, arguing that f32
  gives about 0.5 mm at ±6 km while f64 costs about twice the solver arithmetic and memory on
  NEON. 64-bit stays the default because half a millimetre at the map edge is within Rapier's
  tolerance but marginal, so the safer default is kept until the test decides (recorded in
  revision 3.1). A type alias exposed every platform-library transcendental, so `Scalar` is a
  newtype; switching precision is a Rapier crate switch, not a one-line change; and the 64-bit
  cost that matters is the client's rollback re-simulation on NEON, which sets ring depth, so
  the milestone 1 test measures that loop (R3-10).
- Driven by: R2-8, R3-10.
- Replaced: revision 2's fixed 64-bit positions and 64-bit Rapier build; revision 1's f32 math;
  revision 3.1's type alias (R3-10).

**D60. `physics-rapier` is inside the boundary** (§8.1, §16)
- Why: R1-7 required it together with sharing Rapier's `libm`; no further reason is recorded.
- Driven by: R1-7.
- Replaced: revision 1, whose physics section put Rapier inside the boundary but whose
  boundary list named only "`physics-api` consumers".

**D61. Animation logic and a sim skeleton inside the boundary** (§17)
- Why: climbing, hit frames, root motion and spreading fire are simulation (R1-13). Melee hit
  volumes, hand and foot placement, IK grabs, weapon sockets and ragdolls need bone transforms
  inside the boundary; otherwise combat feels detached from the animation, or someone reads
  render-side poses and breaks determinism (R2-25).
- Driven by: R1-13, R2-25.
- Replaced: revision 1's animation blending and particles outside the boundary; revision 2's
  animation logic inside without bones.

**D82. The physics step is the one sanctioned blocking wait** (§16, §5.1, §9.7)
- Why: "frame-critical workers park" was not a mechanism and contradicted the rule that workers
  never block. A physics stage job calls into the solver's rayon pool, sized to the
  performance-core count minus the workers reserved for render recording and non-physics
  systems; its threads get QoS and floating-point state through the `thread_class` helper; the
  stage is a barrier only for systems that depend on physics; bodies in `Frozen` regions are
  fixed colliders; milestone 1 measures the stall the step imposes on render recording (R3-6).
- Driven by: R3-6; R2-8 earlier (D58).
- Replaced: revision 3.1's parking of frame-critical workers during the physics phase.

## 9. Workflow, tiers and milestones

**D62. One dependency rule, in `crate-tiers.toml`** (§3)
- Why: CLAUDE.md, the principles and the layout stated three different rules and forbade edges
  the design needs (`asset` to `io`, `persist` to `io`, `ui` to `renderer` and `render-graph`,
  `debug-ui` to `platform`).
- Driven by: R1-2, R2-20.
- Replaced: revision 1's "dependency direction is strictly downward"; revision 2's rule stated
  differently in three places.

**D63. ID and handle types live in `core`, which may `unsafe impl` generic plain-data types** (§4, §21)
- Why: `PersistentId` sat in backend `persist` although `sim` needs it; a generic `Handle<T>`
  needs a manual `unsafe impl` because bytemuck derives reject generic structs, and only `core`
  may contain unsafe code. Since revision 3.2 there is no separate `PersistentId`: `core` holds
  `StableKey` with `NetId` and `PredictionKey` (D5, D7), and the `thread_class` helper (D84).
- Driven by: R2-20.
- Replaced: revision 2's `PersistentId` in `persist` and `Handle<T>` in `asset-api`.

**D64. `dyn` resources wired by `app`** (§4)
- Why: the tier rule forces physics, asset and network access through `dyn`, contradicting the
  claim that the RHI is the only place for boxed trait objects.
- Driven by: R2-20.
- Replaced: revision 2's "the RHI is the one place where a boxed trait object per subsystem is
  the right design".

**D65. No `task-api`, no `ecs` interface split** (§4)
- Why: spawning is generic and queries monomorphise, so neither can sit behind a trait object.
- Driven by: R1-17.
- Replaced: revision 1's `task-api` crate.

**D66. Vertical slices, three tracks, a week-one facade and a milestone 1 netcode spike** (§20, §22)
- Why: interface-first with one crate per task deadlocks on backend-driven crates (R1-17).
  Netcode was validated last although it shapes every milestone 1 structure; `persist` and `sim`
  depended on crates scheduled later; independent render work was serialised; `ecs` blocked the
  other agents (R2-10). Revision 3.2 moves `asset-api`, `bundle`, `reflect`, the tagged
  allocator and the facade into milestone 0 because the three tracks shared dependencies no
  milestone listed, adds pass conditions to the milestone proofs because most had none,
  schedules the native Metal spike and fixes the `task` agent's inputs before assignment
  (R3-18). The netcode spike runs over a throwaway transport with fifty claimed dynamic bodies
  (R3-17).
- Driven by: R1-17, R2-10, R3-17, R3-18.
- Replaced: revision 1's interface-first rule for every subsystem with one crate per task;
  revision 2's single-track milestones with `persist` in milestone 1, `asset-api` in 3, `input`
  in 5 and netcode only in 4; revision 3.1's measurements without thresholds (R3-18).

**D67. Milestone 0 proves makespan over critical-path length** (§22)
- Why: milestone 0 should prove critical-path latency (R1-20), which Goal 1 sets as the target.
  "Within 10% of its theoretical bound" named no bound, DAG or machine, and CI noise alone
  exceeds 10%; spawn time is off the critical path while wake and steal latency are on it
  (R2-23).
- Driven by: R1-20, R2-23.
- Replaced: revision 1's "task system saturates cores"; revision 2's "within 10%" and "task
  spawn under 100 ns".

**D68. A prescribed dev profile, split game crates, a measured 30-second loop** (§18)
- Why: the 30-second budget is credible only with opt-level 1 game crates, line-tables-only
  debug info and a fast linker per platform; Windows exceeds it without `lld-link`. Registration
  machinery is a known compile-time sink if it grows deep; the review recommended generated
  functions over life-before-main without giving a separate reason for that choice. `game-ai` is
  the fourth game crate: R2-29 asked for at least four, and AI behaviour is the largest
  independently compilable slice of gameplay and is inside the boundary (recorded in
  revision 3.1).
- Driven by: R2-29.
- Replaced: revision 2's "build time is measured in milestone 1 and mitigated before it exceeds
  thirty seconds".

**D69. State-preserving restart rather than dylib reload** (§18)
- Why: it reuses machinery the engine needs anyway and avoids the ABI hazards of dynamic
  library reloading in Rust (revision 1 §14).
- Driven by: no finding.

**D70. An error policy and tested device-lost recovery** (§3, §19)
- Why: "panic aborts" with no policy meant agent code would `unwrap` IO, asset, network and GPU
  errors; a client-only asset failure is a determinism input; with no CPU mirror, device loss
  had no recovery path.
- Driven by: R2-26; R1-21 for a panic policy.
- Replaced: revision 2, which had no error policy.

**D71. `tools/cook` may contain unsafe code** (§21)
- Why: FFI to Slang and compression libraries.
- Driven by: R2-20.

**D83. An `app` library, three thin binaries and a server denylist** (§2 Binaries, §4, §12.5, `crate-tiers.toml`)
- Why: one `app` crate linked every client backend into the Linux server, and VFX ownership was
  ambiguous. `app` is a library and `app-client`, `app-server` and `app-headless` are thin
  binaries; CI denies the server binary rendering, presentation, platform, audio and UI crates;
  passes from `ui`, `terrain` and `debug-ui` reach the graph through the renderer's
  `RenderFeature` pass-provider trait, registered by the client binary; GPU VFX simulation lives
  in the renderer (R3-16).
- Driven by: R3-16.
- Replaced: revision 3.1's single `app` crate.

**D84. The unsafe list names crates, not globs; a `thread_class` helper in `core`** (§21, §5.1)
- Why: the `rhi-*` glob exempted the interface crate `rhi-api`, and thread setup needs FFI in
  crates not on the list. `rhi-api` and `rhi-null` are not exempt, `platform` is added for
  present-thread QoS and window handles, and `audio` and `physics-rapier` use the safe
  `core::thread_class` helper instead of their own `unsafe` (R3-28).
- Driven by: R3-28.
- Replaced: revision 3.1's `rhi-*` glob.

**D85. A revision marker in `CLAUDE.md`, checked against HEAD** (`CLAUDE.md`)
- Why: a stale copy of `CLAUDE.md` reached a review session through the harness (R3-27).
- Driven by: R3-27.

**D86. Console assumptions kept out of milestone 0 interfaces; a tagged allocator from milestone 0** (§4, §22; verification §3)
- Why: interfaces written in milestone 0 would otherwise hard-code desktop assumptions (R3-30).
- Driven by: R3-30.

**D87. Proc-macro crates and an `xtask` leaf crate in the tier file** (§4, `crate-tiers.toml`)
- Why: the tier check fails any crate missing from the file, and the `#[component]` and
  `#[system]` macros need crates of their own. `lactose-reflect-macros` and `lactose-ecs-macros`
  are foundation crates outside the boundary, since they compile for the host. `lactose-xtask`
  is the leaf crate that carries the tier check, the boundary lint and feature-tree checks and
  the SOUR mode runner, so CI logic is Rust that runs the same on every platform.
- Driven by: milestone 0 foundation.

**D88. Allocator tagging: a fixed tag per subsystem, one trailing tag byte, a thread-local current tag** (§22.1, `lactose-core::alloc`)
- Why: budgets need live bytes per subsystem, and a free can happen on any thread under any
  scope. Each allocation stores its tag in one byte after the block, found again from the
  layout the caller passes back, so frees are charged back correctly without a side table
  and page-aligned buffers waste nothing (a header before the block would cost a full
  alignment; foundation review, finding 10). The
  current tag is a const-initialised thread-local set by `TagScope` and carried across threads
  by the executor. It is the one thread-local inside the boundary; no simulation result can
  observe it, and §8.2 names it as the one exception. Counters belong to the allocator instance, so tests need no global state.
- Driven by: the "allocator tagging scheme" item of §22.1, due in milestone 0.

**D89. `EntityRef` is the plain-data form of an optional `StableKey`; `EntityId` has no ordering** (§7, `lactose-core::id`)
- Why: components must be padding-free plain data, and a Rust enum with payloads of different
  sizes is not. `StableKey` stays the ergonomic enum; `EntityRef` encodes `Option<StableKey>`
  in four `u32` words with a canonical-encoding check on load and receive, and all-zero as the
  empty reference. `EntityId` implements neither `Ord` nor `PartialOrd`, so it cannot become an
  ordering rule by accident. This removes the dependence on `bytemuck_derive` support for
  data-carrying enums for references. The check rejects placeholders, which mean something
  only inside the command buffer that issued them.
- Driven by: milestone 0 foundation; verification §2 (`bytemuck_derive` claim).

**D90. The `task` surface: `'static` graph nodes, an explicit job context, typed thread lanes** (§5.1, `crates/task/README.md`)
- Why: three choices keep the executor's public API free of `unsafe` obligations for callers.
  Graph nodes are `Arc<dyn GraphNode>` and own or share their data, so dropping a `GraphRun`
  future can never leave jobs borrowing freed memory; `ecs` carries the declared-access
  argument that makes its nodes sound. Fork-join is reached through a `JobContext` passed to
  each job, not a free function over thread-local state, and `scope` takes `&mut self`, so
  "inside a job only" and "never nested" are compile errors rather than debug assertions. A
  lane is typed over state that never leaves its thread, so the main thread's window and the
  present thread's surface need not be `Send`. The foundation review tightened four points:
  `Scope` is neither `Send` nor `Sync`, so a child cannot fork into its parent's scope from
  another worker; a run takes its `Graph` by value and hands it back, so two runs can never
  share counters; `JobContext::slot` gives reductions lock-free per-slot partial storage and is
  documented as never a semantic key; and `block_in_place` with a reserved-worker count is the
  §16 physics wait, with dedicated threads created through `Executor::platform`.
- Driven by: milestone 0 foundation; §20 requires these signatures before the task is assigned.

**D91. Thread-locals are banned through `std::thread::LocalKey`** (§8.3, boundary `clippy.toml`)
- Why: on the pinned toolchain a local `#[expect(clippy::disallowed_macros)]` does not reach
  `thread_local!`, so the one sanctioned exception (D88) could only have been made crate-wide.
  Banning the type the macro declares gives the same coverage with an exception scoped to one
  function.
- Driven by: milestone 0 foundation.
- Replaced: the `disallowed-macros` entry of revision 3.2.

**D92. The milestone 0 makespan threshold is measured against the scheduling lower bound** (§22, `crates/task/README.md`)
- Why: revision 3.2 divided makespan by critical-path length. On the wide-shallow shape that
  ratio is large for any scheduler, an ideal one included: a thousand independent jobs on
  eight workers take at least total work divided by eight, many times the longest single
  job. The denominator is now the larger of the duration-weighted critical path and total
  work divided by worker count, which no scheduler can beat, so 1.0 is ideal on every shape
  and the 1.25 and 1.5 thresholds mean the same thing on all three.
- Driven by: foundation review, finding 6.
- Replaced: revision 3.2's critical-path-only denominator.

**D93. An assembly tier, game-to-game edges and test-support crates** (§3, §4, `crate-tiers.toml`)
- Why: `lactose-app` assembles a runtime from backend crates, which the backend rule forbade a
  backend crate to do; it is now the one `[assembly]` crate, may depend on foundation, backend
  and game crates, and may be named only by leaf crates. Game crates may depend on each other
  (`game-sim` reads `game-content`), and backend crates may not depend on game crates.
  `[test_support]` (today `lactose-rhi-null`) lists crates any crate may dev-depend on; all
  other dev-dependencies obey the normal rules. `[exceptions]` lifts backend-to-backend edges
  only. A path dependency must be a workspace member.
- Driven by: xtask lane review, findings 8 and 11.
- Replaced: `lactose-app` in `[backend]`; "game crates may depend on foundation crates only".

**D94. Proc-macro crates carry the boundary lint file** (§8.3, `tools/xtask/ci-policy.toml`)
- Why: `lactose-reflect-macros` and `lactose-ecs-macros` are outside the boundary because they
  compile for the host, but the code they write is compiled inside boundary crates. They carry
  a byte-identical copy of `crates/core/clippy.toml`, and `cargo xtask boundary` checks the
  copies and that the reference file still bans a required minimum.
- Driven by: xtask lane review, findings 13 and 19.

**D95. The boundary dependency allowlist lives in `crate-tiers.toml` and covers the whole closure** (§3, §8.2)
- Why: §3 makes `crate-tiers.toml` the only statement of which crate may depend on which, and
  cargo-deny cannot express an allowlist per depending crate. Every third-party crate a
  boundary crate compiles through normal and build edges, at any depth and through any
  workspace crate, must be allowlisted there; `cargo xtask boundary` enforces it, and
  `deny.toml` keeps the workspace-wide rules (sources, licences, advisories, duplicate versions).
- Driven by: xtask lane review, findings 14 and 15.
- Replaced: revision 3.2's cargo-deny allowlist.

**D96. The SOUR run configuration is a versioned environment schema, and each run records the configuration it applied** (§8.3, `tools/xtask`)
- Why: layout and chaos are runtime options, so the runner passes one configuration per test
  process in `LACTOSE_SOUR_*` variables. A malformed environment is an error, never a fallback
  to production, and each test records the configuration its world and executor builders
  report, which the runner checks against what it launched, so a mode cannot pass without
  running. Chaos runs always write their execution log; the runner keeps the logs of runs
  that disagreed.
- Driven by: milestone 0 xtask lane and its review, findings 5 to 7.

**D97. Components are laid out by the author and checked by the compiler; arrays are `PlainArray`; the attribute writes `CheckedBitPattern` itself** (§6, §21, `lactose-reflect`)
- Why: a proc macro cannot evaluate alignment and hidden padding fields break struct literals,
  so `#[component]` emits per-field `offset_of!` assertions and `Padding<N>` has zero as its
  only value. bytemuck 1.25.2 treats `[T; N]` as plain data only for `T: Pod`, so
  `lactose-core::PlainArray` carries the `unsafe impl`s. `bytemuck_derive` 1.12.1 does not
  forward its crate path to the nested `AnyBitPattern` derive, so its struct derive compiles
  only where `bytemuck` is a direct dependency; the attribute writes the impl instead. Because
  an attribute below ours, or a raw-identifier spelling such as `r#repr`, can change the type
  after the macro read it, the expansion asserts size, alignment, every field offset, the
  exact field list and the exact field types against the final type, and enums get the
  equivalent checks. `Reflect` stays a safe trait: `Schema<T>` takes size, alignment and
  validity from `T`, so `validate` is a proof for `T::SCHEMA` and registry lookups and nothing
  else is. Field types carry a stable type tag and structs a retired-ID list, so a schema dump
  diff can reject field-ID reuse. Data-carrying enums are supported by the pinned bytemuck
  but not reflected: nothing in milestones 0 or 1 needs them.
- Driven by: milestone 0 reflect lane and its two adversarial reviews; verification §2.
- Replaced: revision 3.2's field reordering and inserted padding; bare `[T; N]` fields.

**D98. Asset versions are content hashes; dependencies are by ID** (§14, `lactose-asset-api`)
- Why: a cooker-assigned counter needs a ledger and can publish different bytes under one
  version after a clean cook or on a second machine, which a server and client would not
  detect. A hash of the cooked header and bytes cannot, and needs no order because "newest" is
  a fact about the store's index. Dependencies name an `AssetId`, never a version, so a
  published version means the same thing whichever dependency version is loaded.
- Driven by: asset-api lane review, finding 3.

**D99. The asset store is two traits, and boundary code reads simulation kinds through a pinned table** (§4, §8.2, §14)
- Why: "newest" and "resolve" depend on load timing, so they live on a catalogue trait that
  boundary worlds never receive; exact-version access is the only store a boundary world
  holds, which makes the rule structural instead of a lint. `resolve` returns the dependency
  closure so a pinned table holds everything a region can reach, and a publication generation
  counter avoids re-resolving every region every tick. Loaded assets are `Arc`-backed handles,
  because a shared `dyn` store cannot lend borrows from a table it publishes into; boundary
  reads borrow from a region's `PinnedTable` and touch no atomic. Loaders run outside the
  boundary, so kinds the simulation reads are marked and their loaders constrained.
- Driven by: milestone 0 asset-api lane and its review, findings 5, 6 and 11.

**D100. `typed_plain_data!` in `lactose-core`** (§21)
- Why: bytemuck cannot derive `Pod` for a struct with a non-`Pod` marker parameter, and
  `asset-api` forbids `unsafe`. The macro writes the struct and the impls together and accepts
  no attribute, so no caller can change the representation and the unsafe text stays in core.
  A core type alias was rejected: the orphan rule would stop `asset-api` implementing the
  `reflect` field traits for its own reference type.
- Driven by: milestone 0 asset-api lane.

**D101. Bundle documents are canonical in bytes and in meaning, self-describing, and end in their own xxh64** (§9.3, §15.2, §15.4, `lactose-bundle`)
- Why: one trailing hash is integrity check, baseline content hash and overlay digest.
  Allowing several record sets for one region state would make the digest depend on the code
  path that produced it, so each state has exactly one. Carrying field layouts lets an old
  save be decoded by field ID without the binary keeping every historical layout. Rebase
  drops every record that is no longer valid against the patched baseline and returns them,
  since the literal "drops only `SetField`" left records naming entities that no longer exist.
- Driven by: asset-api lane review, findings 1, 2 and 4.
- Replaced: revision 3.2's "drops only `SetField` records whose target no longer exists".

**D102. Cooked identity hashes authored GUIDs only** (§7, `lactose-bundle`)
- Why: hashing a name-derived scene key would orphan every save when a scene file is renamed.
- Driven by: asset-api lane review, finding 9.

**D103. Stable `u8` codes for `ErrorKind`** (§19, `lactose-core`)
- Why: an asset failure that affects simulation is a journaled, tick-stamped event, which
  needs an encoding that does not change when the enum gains a kind.
- Driven by: asset-api lane review, finding 8.

**D104. `math` min, max and clamp: numeric order refined by -0 < +0, NaN absorbing** (§8.2, `lactose-math`)
- Why: §5.2 uses `math` min and max as order-independent reductions, which needs them
  commutative and associative on every input. A bare compare-and-select returns a different
  operand depending on which side the NaN is on. The rule is IEEE 754-2019 minimum and
  maximum; `clamp(x, lo, hi)` is `x.max(lo).min(hi)` and never panics.
- Driven by: milestone 0 math lane.

**D105. Canonical bits collapse every NaN and keep negative zero; `sqrt` and fused multiply-add use the hardware instruction; `libm` is pinned exactly** (§8.2, `lactose-math`)
- Why: negative zero is observable through arithmetic, so collapsing it would let two different
  states hash equal. `sqrt` and `mul_add` are correctly rounded, so only the route matters:
  under the x86-64-v3 pin they compile to one instruction, whereas `libm::fma` on x86-64
  selects through CPU detection at run time. `Scalar::mul_add` is `inline(never)`, because an
  inlined copy is compiled with the caller's target features and a caller built without the
  pin (a doctest, for one) would call the platform `fma`; the float `mul_add` methods are on
  the boundary ban list for the same reason. `libm` is pinned to one exact version with only
  its `arch` feature: its releases change transcendental algorithms, so a bump is a
  determinism change that regenerates the golden table.
- Driven by: milestone 0 math lane and its review, findings 1 and 7; verification §2.

**D106. Float reductions use an exact superaccumulator, and reducers are sealed** (§5.2, `lactose-math::ExactSum`)
- Why: a fixed-point `i128` must quantise inputs and saturate, and saturation is not
  associative. A 2240-bit integer in units of the smallest subnormal holds every finite sum
  exactly and rounds once, so the result is the correctly rounded true sum for any order and
  grouping, at 288 bytes per partial per scalar lane. `Reducer` is sealed and `Min`/`Max`
  accept only `Scalar` and primitive integers, because a reducer carries a determinism
  contract (a tie-breaking `Ord` would make the result depend on order) that a game crate
  must not be able to implement wrongly.
- Driven by: milestone 0 math lane and its review, finding 4.

**D107. Executor futures resolve to `Result`, and resolution means quiescent** (§5.1, `lactose-task`)
- Why: a future that never resolves after shutdown hangs its waiter, so `GraphRun`, `Task` and
  `Sleep` resolve to `Cancelled`. `lactose-ecs` frees or reuses world state when a run
  resolves, so a run that resolved while one of its units was still executing would race it:
  a run counts its executing jobs, a job registers and reads the closing flag in one atomic
  step, and the run resolves only at zero. A unit already past its shutdown check may still
  start; it finishes before the resolution. A job cannot wait for its own run.
- Driven by: milestone 0 task lane, first review (hang after shutdown) and second review,
  finding 2.

**D108. Scope-open routing replaces the deque watermark** (§5.1, `lactose-task`)
- Why: a watermark cannot stop work pushed to an opener's deque after the scope opened (a
  released dependent, a woken task) from sitting above it, where the opener would run it or
  strand it while parked. Entries carry their scope's tag and anything that is not a child of
  the open scope goes to the frame injector. A parked opener gives up its run permit, so it
  does not consume a `block_in_place` reservation; an opener with children on its deque
  announces itself before parking and a freed permit unparks it.
- Driven by: milestone 0 task lane, first review (scope-wait deadlock, reservation consumed by
  a parked opener) and second review, finding 1.
- Addendum (lane task-flake2, 2026-10-11): `a_parked_opener_does_not_use_up_the_reservation`
  failed on Windows CI (run 38126882535) with a correct pool. It asserted that a child finished
  inside a 100 ms wall-clock window during `block_in_place(1)`, which a descheduled physics
  thread (the thieves finish every child first) or a descheduled permit holder misses; the old
  test with the physics thread held 600 ms fails with the CI message. The rewritten
  `block_in_place` tests synchronise on events: a unit or child begun before the block holds
  its worker until the block is in place; in the reservation and all-openers tests the block
  begins only once every other worker is held mid-range or inside one of its children; and a
  block that reserves workers lasts until the work it is owed has run, bounded at 60 s. The
  parked-opener test runs both orders: the opener parks before the block, or parks inside it
  after the thieves gave way, where the permit it gives up under the closed gate must reach an
  idle worker. The all-openers test begins the block with both openers held inside one of their
  own children and checks that at most one child admitted under the closed gate runs at once.
  The reservation test skips each worker's first unit after the block began, the one it may
  have been admitted to before, so `peak <= reserved` and nothing running with nothing reserved
  hold on any schedule; it relies on roots released in node order into the FIFO injector. Each
  of the three rewritten tests fails under a mutation of the code it covers: a parked opener
  keeping its permit; a permit freed under the closed gate waking no idle worker, or the opener
  releasing without telling anyone; a graph worker or an opener never giving way mid-range; a
  gate that ignores the reservation. Not covered yet (review round 2): the recovery test is
  caught by no listed mutation and hangs rather than fails when the gate keeps its reservation,
  and no test checks that opening the gate wakes the workers it held.

**D109. Wake policy: park after 50 µs, keep one, wake one per extra entry** (§5.1, `lactose-task`)
- Why: measured on an M1. A longer spin or an extended rescan before parking bought no wake
  latency and cost idle CPU; a releasing worker that keeps one entry and wakes nobody for it
  saves a wake-up per fan-out. A 3 ms lingering spinner reached a P99 of 14.7 µs but burned
  99% of a core and was not adopted. Wake latency at tick start is bounded below by the
  operating system's park and unpark (verification, section 4).
- Driven by: milestone 0 task lane benchmarks.

**D110. The sleep protocol uses `fence(SeqCst)`** (§5.1, `lactose-task::sleep`)
- Why: the protocol is a store-then-load on both sides (announce then re-check; push then read
  sleepers), which needs a total order. loom 0.7.2 models `SeqCst` loads and stores as
  `AcqRel`, so a version written with `SeqCst` accesses could not be checked; with explicit
  fences loom explores the orderings that matter and the models fail when a fence is removed.
- Driven by: milestone 0 task lane; verification section 2, loom limits.

**D111. Worker counts: classes above the slowest are frame-critical, capped by usable hardware threads** (§5.1, `lactose-task::Platform`)
- Why: a three-class CPU has no single "performance" class, and a process confined by affinity
  or a container must not start a worker per physical core. `Platform::hardware_threads`
  carries the limit (default `None`: take the topology as given), so the builder never
  overrides a console platform's fixed topology. One thread is left for the platform thread.
- Driven by: milestone 0 task lane, first review (Windows topology) and second review,
  finding 6.

**D112. Shutdown joins unless called on one of the executor's own workers; the execution log is read, not taken** (§5.1, `lactose-task::Executor`)
- Why: a worker cannot join itself, so there shutdown does everything else and the workers
  exit on their own; D107 makes resolution, not the join, the point of quiescence. A worker of
  a different executor joins normally. `execution_log` replaced `take_execution_log` because
  SOUR writes the log of every chaos run (D96) and a failing test must still be able to read
  it afterwards.
- Driven by: milestone 0 task lane, second review, finding 7.

**D113. Two wake-latency criteria: tick start under 20 µs, push from an idle pool under 35 µs** (§22, `lactose-task`)
- Why: the wake-latency spike (`docs/results/spike-wake-apple-m1-2026-10-06.md`) showed that
  on an idle M1 every wake primitive, QoS class and real-time policy lands at P99 24 to 29 µs,
  because the cost is the cores leaving deep idle, not the scheduler; only a thread waking
  every 150 µs or less removes it, at 1.3 to 1.9% of a core and an unmeasured power cost. A
  known deadline is different: the timer keeper can step its timed park and spin the last
  50 µs, so the latency that decides tick timing can meet 20 µs. The push criterion is set
  at what the operating system gives, and no warm sleeper or heartbeat is adopted; a
  pre-warm in the lead-up to a tick deadline may return as a builder option once power can
  be measured. macOS lengthens every timed wait by a leeway proportional to the timeout, so a
  single `park_timeout` to a deadline is never acceptable in the keeper.
- Driven by: milestone 0 wake-latency spike; owner decision 2026-10-10.

**D114. The ECS facade: what section 6.1 left open** (§6, §6.1, `lactose-ecs`)
- Why: the facade is written before its implementation so every dependent crate builds
  against the final surface; each choice below is one the section's prose or example did not
  settle, and each is in `crates/ecs/README.md` as a contract.
  - *Events are emitted through a typed `EventWriter<E>` parameter, not `cmd.event(..)`.*
    The schedule must know every emitter of `E` to order every `Events<E>` reader after them,
    or a reader's view would depend on scheduling; a method call on `Commands` is invisible
    to the `#[system]` macro, and an attribute listing the emitted types would be a second
    declaration that can disagree with the body. The 6.1 example changes accordingly.
  - *Predicted keys come from `ClaimedBy` and `World::set_input_tick`.* A spawn commanded
    from an entity the player has claimed gets `Predicted(player, input tick, ordinal)` with
    the ordinal counted among that player's spawns in the tick's command order, and the
    spawned entity inherits the claim, so a bomb's shrapnel is predicted as the bomb was. The
    same rule on the server and the client gives the same key without a message; the input
    tick is the one fact the netcode supplies, at the input sync point. `ClaimedBy` doubles as
    the prediction world's filter, since a claimed entity is exactly a predicted one.
  - *A placeholder is an RNG key.* `ctx.rng(bomb)` in the example keys a draw by an entity
    whose origin key is assigned only at apply; the draw is keyed by the invoking entity's
    origin key and the placeholder's spawn index instead, which both machines agree on.
  - *Stages are declared on the builder; sets are statics with `after` lists; exclusive
    systems run at the sync point that begins their stage; a system without a component
    parameter runs once per tick* (refined by the review: once per region if it has a
    `Ctx`). The sync-point count is then fixed per schedule as section 5.2 requires, and a
    shared registration function serves every world kind because `add` skips a system whose
    worlds exclude the builder's kind.
  - *`World::spawn` takes the key; only `Commands::spawn` assigns one.* Replication and
    prediction insert entities under the server's key; a hidden policy in the direct method
    would be a second key source.
  - *Regions, the snapshot and bulk row copy are the ECS's; the world hash and the overlay
    diff are `sim`'s and `persist`'s.* Kernels must skip rows in `Replicated` and `Frozen`
    regions and the prediction filter is a storage property, so region mode lives with
    storage; what needs the registry and the baseline (hashing with per-type hashers, diffing
    into records) stays where section 8.1 put it, over `World::columns_erased` and
    `World::export_region`.
  - *Service resources are `Res<dyn Trait>` through `World::insert_service`,* unreflected
    and outside hashing, saving and snapshots, so an `-api` crate's trait object needs no
    registry entry.
  - *Change consumers hold a `ChangeMark` or `LogCursor` carrying the world's epoch; a stale
    one yields `None`.* A restore or load must force a full pass (§6), and a value the
    consumer cannot mistake for progress is the way to force it.
  - *`Schedule::run_tick` is an `async fn` taking `&mut World` and `&Executor`.* The timer
    root is a task on a worker, where `block_on` is a debug assertion; the implementation
    lends the world to the run's graph nodes on the strength of the `GraphRun` resolution
    guarantee (`crates/ecs/SAFETY.md`). Superseded by the review: the world is taken by
    value (below).
  - *Bundles are tuples only.* A blanket `impl Bundle for T: Component` is rejected by
    coherence beside tuple impls, because `Component` is a foreign trait.
  - *`StableKeyOf` dereferences to `StableKey`;* `*key` where a plain key is needed. The
    macro makes every parameter binding mutable so the example's `vel.y -= ..` and
    `cmd.spawn(..)` compile as written.
  - *`RegionRes<T>` is read-only in kernels.* The kernels of one region run on many workers;
    region resources are written through `World::update_region_resource` at a sync point or by
    a reduction's finalize through a world resource.
- What the design review changed (9 blocking, 16 serious, 9 minor findings; `aa3f05d` was
  the facade before it): `run_tick` takes the world by value and hands it back, because a
  dropped or forgotten future would otherwise leave jobs writing through a dead borrow, and
  `run_pass` runs a schedule between ticks for extract and the gather; a `Job` parameter
  gives once-per-tick, exclusive and finalize code the executor's job context, so parallel
  exclusive passes and the physics step's `block_in_place` can be written without `unsafe`
  outside `ecs`; a system with no component parameter but a `Ctx` runs once per `Simulated`
  region, which gives it a region, a seed and a spawn target; `ResMut` and `Job` are
  rejected in anything that runs more than once per tick, two parameters on one component
  are rejected, and so is `Mut` on a world-maintained or reference-bearing component; the
  tick is settable and never rebased, stamps are; a predicted origin key is persisted as
  `PredictedOrigin` so it survives a save and a region overlay; the predicted-key rule gained
  the input-tick-slot rule, the clearing of input ticks at the sync point and the
  `prediction`-worlds condition, and `World::spawn` gives a `Predicted` key its origin
  component; channels resolve placeholders through `CommandChannel::remap`; `Changed<T>` is
  exact through a (tick, position) stamp and allowed on tracked columns only, since a
  chunk-tick fallback made it layout-dependent; the alias table and the delivered
  presentation-event keys survive `restore`, the present-event key gained a sequence, and
  the snapshot covers every column and resource; `Query::for_each` and `referrers` are in
  key order so exclusive code that spawns per match is layout-independent; marks and cursors
  are ordered and `is_current` tests a mark; a region load no longer starts an epoch;
  `load_region` takes baseline and overlay together so a reloaded region does not collide
  with a cooked entity that moved out of it; exports carry the seed and region resources;
  `ColumnMut::write` passes the keys and records a row set; `SystemDescriptor::new` is
  `unsafe`, since the schedule's soundness rests on the table it takes. Deferred as additive
  attribute flags: the overload ladder's decimation (§9.7) and `ordered` or per-region
  reductions (§5.2).
- What the second review round changed (0 blocking, 7 serious, 10 minor; `a0b2ac4` was the
  facade between the rounds): the descriptor carries the finalize function's access table
  separately, since a flat table could not tell a finalize's `ResMut` from the kernel's; the
  predicted ordinal is a hash of the spawning invocation (source origin key, system key,
  spawn index) rather than a position in the command order, because one spawn the client
  cannot reproduce shifted every later key of the player, and only systems that run in both
  the server and the prediction world produce predicted keys; the boundary lint file bans
  `JobContext::slot` and `ChildContext::slot`, since `Job` would otherwise hand game code
  the worker index; the alias rule is two data parameters on one component or `Without`
  beside any, so `Changed<T>` beside `&T` is allowed; a restore puts change stamps back
  verbatim and a correction written without recording leaves them alone, so a re-simulated
  tick's `Changed` filters see what the first run saw; stored references hold origin keys
  and exports write them as stored, so a reference to a predicted entity round-trips and
  the hash is unchanged by a load; a region's seed and persistent resources travel beside
  the overlay, since `Overlay` cannot carry them, and `load_region` loads into a region the
  caller created; `Schedule::prepare` runs the world-dependent checks by reference, as
  `run_tick` cannot hand the world back on an error; a closing sync point applies the last
  stage's buffers so a snapshot point never has a pending buffer; a `ChangeMark` is a point
  in the stamp order so a direct write after it is newer than it; `World::query` and
  `columns_mut` refuse the components `get_mut` refuses; `set_rollback_horizon` tells the
  world what it may prune; events order as commands do; `worlds` omitted means
  `[server, prediction]`; `QueryItems` is sealed; the `unsafe` block of the expansion is
  spanned at the call site and the finalize's hidden fetches are `unsafe fn`; section 21
  names the expansion.
- Driven by: milestone 0, the ECS facade and its two design-review rounds (2026-10-10).

**D115. The boundary check resolves features as a workspace build does** (§8.3, `tools/xtask`)
- Why: `cargo xtask boundary` computed each boundary crate's third-party closure from a
  `cargo metadata --all-features` graph. With `lactose-ecs` depending on `lactose-task` that
  resolution selected `lactose-task/loom`, a feature only the loom CI job enables, and
  reported 28 crates (`loom`, `tracing-subscriber`, `cc`, ...) that no boundary build
  compiles; `valuable` had already been allowlisted as the same artefact. The metadata is now
  resolved without `--all-features`: the union of what every member asks of its dependencies,
  which is what the shipped binaries compile once they exist and what `cargo xtask features`
  compares. The declared-edge checks (tiers, direct allowlists) read manifests and are
  unaffected. `crossbeam-deque`, `crossbeam-epoch` and `crossbeam-utils` are the real
  additions to the closure and are allowlisted with their review.
- Driven by: milestone 0, the ECS facade.

**D116. Keep the first timer-woken task only when the injector is empty** (§5.1, `lactose-task`)
- Why: measured on the M1 at 60 Hz, the keeper that fires a tick deadline already resumed
  the timer root in 4,947 of 5,000 deadlines; the cost lay on its own path. It unparked a
  sleeper that found the task gone (P50 3.6 µs, P99 12.9 µs), and it split the due timers
  into a new map, allocating on every tick (P50 2.9 µs, P99 21 µs). A worker that fires
  timers on its way to its next job now keeps the first frame-critical task they wake on its
  own deque and wakes nobody for it, as D109 keeps a released node, and due timers are taken
  out one at a time. It keeps only while the frame-critical injector is empty: the review of
  the first version showed that otherwise a worker woken for a queued task spends that
  task's wake-up on the timer's task (P99 2 ms for the queued task), and a task whose timer
  is due whenever its poll returns is kept on every pass and starves the queue, which on a
  one-worker topology (D111) is the whole frame pool. A worker already announced as about
  to sleep keeps nothing, because nothing orders its push before its permit check; a loom
  variant that does keep deadlocks. In interleaved A/B runs tick start went from P50 about
  12 µs and P99 41 to 44 µs to P50 3 to 5 µs and P99 23 to 30 µs, and process CPU at 60 Hz
  from 0.0147 to 0.0103 s/s. The 20 µs criterion is not yet shown on this machine, which
  alternates between a state where the fixed keeper measures P99 under 6 µs and one where
  it measures 22 to 25 µs (`docs/verification.md`).
- Addendum (lane task-small, 2026-10-11): the wake order is stated and the order of the tasks is
  not. A worker that fires timers wakes the ones due when it looks in deadline order, ties in
  registration order; workers that fire at the same time interleave theirs (the review measured
  56 to 150 global inversions per 4,000 wake-ups on four firing workers, none within one). A woken
  task is scheduled by the rule above, and a `Sleep` first polled after its deadline resolves at
  that poll. `sleep_until_resolves_at_its_deadline` asserted that three tasks on two workers
  finished in deadline order and failed under load on macOS CI (run 38081901338): a task polled
  late resolves in poll order, and a late worker that finds two timers due keeps one task and
  queues the other for a sleeper that can run it first. The test keeps its other assertions;
  `due_timers_wake_in_deadline_order` (one firing worker held past every deadline) and
  `a_later_timer_does_not_hold_an_earlier_one_back` (the keeper re-arms for a new earliest
  deadline) prove the contract without depending on scheduling, and each fails under the
  mutation of the code it covers.
- Driven by: the D113 tick-start criterion; the keeper fix round and its review.

**D117. The naive ECS implementation: what the facade left to it** (§6, `lactose-ecs`)
- Why: the implementation lane met the facade's contract and had to settle what it did not
  say; three review rounds (1 blocking, 3 serious, 8 minor; then 2 blocking, 5 serious, 5
  minor, two of them regressions of the first fix round; then a verification round) shaped
  the answers. Each is a contract a consumer may rely on.
  - *Run model.* A run is one graph node; the world moves in and out through an uncontended
    mutex. `Job` needs a `JobContext`, which only a graph job provides.
  - *Storage is safe code.* `Ctx` holds no `&World`; values that may hold placeholders stay
    typed until apply. The crate's `unsafe` is `SystemDescriptor::new` and the hidden
    finalize plumbing.
  - *Claim timing.* Whether a spawn's source has `ClaimedBy` is read when its kernel runs.
  - *Ordinal bytes.* The predicted ordinal hashes the source key (tag byte, then words), the
    system key (u64) and the spawn index (u32), little-endian, xxh64 seed 0.
  - *References and binds.* Direct `spawn` and `insert` keep references to entities not yet
    live; only command apply empties them. `bind` rewrites stored references from the Net
    key to the origin key, in rows and in region resources, and refuses a Net key that names
    a live entity. A restore does the same rewrite for binds made after its snapshot, except
    for a Net entity the restore revives, which keeps its references and which
    `lactose-predict` despawns again. A predicted entity a restore removes is recorded as
    despawned at the snapshot tick, so the rollback horizon prunes its alias.
  - *NetIds and unloads.* `allocate_net_id` skips ids already in use; `unload_region` keeps
    aliases.
  - *Transient writers: `writes = [T, ..]`.* A transient column's declared writers are
    `Mut` on it or a `writes` declaration, which also declares writes through `Commands` or
    `&mut World` for conflicts (against readers and `Mut`, not against another declaration).
    On an exclusive system the writes land at its sync point and satisfy readers ordered
    after it in that stage; on any other system they land at the next sync point and satisfy
    readers from the next stage on. A declaration is trusted, never checked; an exclusive
    system declares nothing implicitly. The first fix round had counted every exclusive
    system as a writer of every column, which let any input system defeat the check.
  - *Transient presence.* It travels with exports: `ExportedRow::transient` for components,
    `RegionRows::transient_resources` for region resources; `persist` encodes presence and
    ignores values; `load_region` stores a transient component zeroed and the loader
    re-inserts transient resources with `insert_region_resource_zeroed`; the input sync
    point zeroes transient columns and transient region resources.
  - *Region resource writes.* Only through `update_region_resource`, which runs a closure on
    a copy, validates and canonicalises the result and keeps the old value on `InvalidData`;
    no `&mut` to stored bytes is handed out, so stored bytes are always a valid value.
    `region_resource_mut` of the facade is removed.
  - *Stamps across the wrap.* Stamps are compared relative to the current tick, valid
    because every 2^30 ticks (tick 0 included) older stamps are clamped to 2^30 ticks old,
    so every stored stamp is less than 2^31 ticks old; a pass's first window in an epoch
    opens 2^31 ticks back; `set_tick` moves stamps onto the new clock by their age; a new
    chunk's column ticks are its first row's, and no zero sentinel is stored (the sentinel
    the first fix round kept won the relative comparison above tick 2^31).
  - *Presentation events.* A batch orders ticks by their age at the take; a pass numbers its
    emissions from 2^31; a cancelled key re-emitted before it is taken is neither cancelled
    nor delivered again.
  - *Schedules.* A `Schedule` serves one world, tied by its first successful `prepare`; a
    set belongs to one stage; a resource removed mid-run skips the systems that name it,
    logged; `Changed` in `World::query` means after this point one tick back; a finalize
    asserts at compile time that its system exists and is a reduction.
- Driven by: milestone 0, the ECS implementation lane and its reviews (2026-10-10).

**D118. The `sim` facade: what sections 4, 6, 8.2, 8.3, 9.1, 9.7, 11, 14 and 15 left open** (§4,
§5.2, §6, §6.1, §8.2, §8.3, §9.1, §9.7, §10, §11, §14, §15.3, §16, `lactose-sim`, `lactose-ecs`)
- Why: the facade is written before its implementation so `app`, `predict`, `net`, `persist`,
  `physics-api`, `anim`, `input` and the game crates build against the final surface; each
  choice below is one the sections did not settle, and each is a contract in
  `crates/sim/README.md`.
  - *The timer root lives in `app`; `sim` provides clockless runs.* Section 11 gives the root
    tick deadlines, catch-up and pacing, and `task` gives it `TickPacer`; a boundary crate may
    read no clock. `Simulation::step` runs one tick from one frame, trims the structural log to
    a retention (a ring of one log cursor per step, trimmed to the oldest; a consumer further
    behind does a full pass) and, on the server, drains the presentation events;
    `Simulation::run_passes` runs a frame's passes (extract, then the gather) once per frame,
    paused or not, on the server and on both client worlds, so edits between ticks are still
    extracted and replicated, the prediction world's local player is extracted every frame, and
    a gather covers every tick since the last frame; re-simulation calls `step` alone, so
    extract never runs against a discarded tick. Every run takes the simulation by value and
    hands it back, as `Schedule::run_tick` does with the world; a run that fails before anything
    ran (the kind check, `prepare`, which `run_passes` performs for both passes first) hands it
    back inside `RunError`, never the frame. Rules out a `sim`-owned loop with injected clocks,
    and passes inside the step.
  - *One `TickInput` per tick is the unit of everything that enters the boundary*: the time
    scale, asset failures, directed unloads, region bundles (each with its first asset table),
    asset tables, directed mode changes, sessions and players' inputs, applied in that order.
    Every keyed field is a map, so a frame cannot hold two entries for one key and assembly
    order decides nothing. The journal is the sequence of frames and `predict`'s ring stores one
    per tick. Rules out any between-tick call that changes simulation state (`world_mut` is
    unjournaled: tests only, and `app`'s `RegionPolicy`).
  - *Sessions are server inputs.* `TickInput::sessions` carries `Join { region }` and `Leave`
    per player; a `SessionApplier` the game or `app` implements spawns the player's entity, or
    re-claims the one the game keeps for the player across saves (found by a persistent
    component of the game's), adding `ClaimedBy` and `StreamingAnchor`, or despawns it,
    resolving it from world state and never from a side table. A `Join` whose region is not
    loaded is held as an anchor position until the region loads, counting from its own frame's
    bundle check, so the join itself brings its region in; a `Join` for a region the atlas lacks
    is refused and reported; a `Leave` cancels a held join. A save carries no session state, so
    on a load every player joins again; a journal records the joined players, held joins and
    requested regions at its checkpoint. Clients learn of players through staged state. Rules
    out a player table in `sim` and unjournaled `ClaimedBy`/anchor writes.
  - *Players' inputs are bytes plus a typed hook.* `PlayerSample::sample` is the input
    component's plain-data bytes, journaled verbatim; `InputApplier` (beside the component in
    `lactose-input`) decodes and writes them after `sim` has called `World::set_input_tick` with
    the slot being filled. Chosen because the ECS has no erased row write and `sim` must not
    define the input component.
  - *Staged network state and the proxy update are one hook*, `StagedState`, filled by `net`
    (client) or `predict` (prediction world); the server's default does nothing. In the
    prediction world `predict` also writes `StreamingAnchor` on the local player's entity so the
    appliers' lookup resolves; under `Directed` it has no policy effect.
  - *The three hooks are `dyn` service resources taken as `ResMut<dyn Trait>` by exclusive
    systems*, because input ticks and transient columns are cleared inside `run_tick` at the
    sync point, so nothing applied between ticks survives. The sync point's systems are private
    and `sets::INPUT` is the only thing a consumer orders itself after; the lifecycle is held
    under a type no system can name.
  - *Setup functions insert what systems name.* `SimulationBuilder::setup` runs after `sim`'s
    own inserts and before any registration function and the prepares, one per crate, so `build`
    can prepare every schedule and a missing resource is a build error. World resources are
    configuration or session state and must be `derived`: `build` refuses any other policy among
    the resource types the `TypeTable` knows, since world resources are neither hashed nor
    saved; simulation state that is hashed or saved is a component or a region resource, and
    world-global persistent state is a resource of a pinned region. The 6.1 example's `Gravity`
    and `TotalMass` are therefore `derived`. The physics backend's solver pool, which section 16
    sizes by the worker count, is built by `app` outside the boundary and handed to the backend
    through a setup function.
  - *Time scale enters through the frame.* `TickInput::time_scale` is `app`'s decision on the
    server, mirrored into the `derived, replicated` `TimeScale` resource by the sync point so it
    is journaled and replicated; the pacer's scale is `app`'s own copy (on a client, `net`'s
    latest value) and no game system reads the resource, so a paused world can be unpaused. A
    pause is entered by one tick whose frame carries `PAUSED` and then the pacer, and left by
    the pacer first and the next frame. Rules out `Res<TimeScale>` in game code and an
    unjournaled `world_mut` write.
  - *Four fixed stages*: `Input` (the sync point: `sim`'s six exclusive systems first,
    consumers' exclusive systems after `sets::INPUT`), `Simulation`, `Physics` (the
    once-per-tick step with `Job`, writing back through `Commands`), `Post`; the same count in
    every world kind. Sets are declared per stage through `Registration::stage(..).sets(..)`, so
    `game-sim` adds sets without owning the stage list. Two crates that both add exclusive
    systems to one pass, or after `sets::INPUT`, do not name each other: `app` orders their sets
    with `Registration::order`, through the new `ScheduleBuilder::order` of `lactose-ecs`, an
    edge between two sets added by the assembler, which must agree with the stage order.
  - *Two passes, one fixed stage each*: `Extract` then `Gather`, filled through
    `Registration::pass(..)`, skipped when no system of the kind was added; the save copy is a
    third, internal pass run on demand by `save_point`, one child per region over
    `World::export_region`, server only (single-player runs a `Server` world in-process).
  - *Region control is `Anchored` on the server and `Directed` elsewhere.* Anchored: the regions
    of rows with `StreamingAnchor` and of held joins are the positions; `RegionAtlas` gives
    cells; Chebyshev distance; three radii in `RegionPolicy`, `release >= streaming >
    activation` enforced at build and clamped at run time: within activation → `Simulated` once
    loaded with a table, within streaming → requested and kept `Frozen`, outside release →
    unloaded and a pending request cancelled (`RegionRequest::Cancel`); the gap between
    streaming and release is the hysteresis. Pinned regions are always wanted and never
    released, so world-global persistent state has a home that is simulated whenever the server
    runs. A bundle for a region neither pinned, nor requested, nor within the release radius is
    refused. The policy runs last at the sync point so it sees the sessions' spawns. Directed:
    the frame says what unloads, loads and which mode; the client's, the prediction world's and
    the test's mode. No table waits for a region (a bundle brings its first), so nothing about
    regions lives outside the world and the frames; the lifecycle's remaining state is
    server-only session state that the server never rolls back, section 10's list is unchanged,
    and the journal records it at its checkpoint. Rules out a player-positions input and an
    unjournaled region load.
  - *Unload and save are one path*: both produce `RegionExport { region, rows: RegionRows }` on
    the server (every row is bound at apply, so the export cannot fail); off the server an
    unload does not export. `persist` diffs the export into the overlay, which is also what the
    server sends on relevance. A refused bundle changes nothing: resources, transient keys and
    the overlay are validated before `create_region`.
  - *The hash is two-level and additive*: field hash (xxh64 seeded by field ID over the field's
    stream, possibly empty, for every top-level ID'd field), row hash (seeded by component key
    over the key bytes and the field hashes; a type with no ID'd field hashes its key alone, so
    presence counts), column sum by wrapping addition (no sort), region hash over seed, mode,
    asset table, columns ascending by key and resources ascending by key; `Simulation::hash`
    over the tick and the regions. The digest is keyed by row with its field hashes and
    reproduces the region hash exactly, so a mismatch is explained field by field, a marker
    component included.
  - *The typed entry point survives the erased registry through `TypeTable`*: one
    `register::<T>()` per type does `Registry::register` and, for persistent and transient
    types, monomorphises `T::visit` under the hashing visitor; raw floats are refused at
    registration by layout kind and the scalar is trusted by its `Field::NAME` tag at
    registration and by `TypeId` in the visitor. A game crate writes three functions:
    `register_types`, `setup`, `register_systems`.
  - *Time*: `TickDelta` is `1 / 60` through `lactose_math`, derived, never changed;
    `CurrentTick` is a derived resource mirroring `World::tick`, rewritten every tick and read
    through `get`; a write is a bug the type cannot prevent, bounded to one tick.
  - *SOUR*: `harness::layout_mode(ChunkCapacity, seed)` is the layout-mode policy (permute and
    split from the seed; the chunk order by capacity, reversed for seven rows and the 16 KiB
    slab and shuffled otherwise, so the matrix covers both orders under the runner's fixed
    seeds; `Bytes` other than 16 KiB is `Unsupported` rather than silently the slab); chaos mode
    is every `ChaosConfig` flag plus `record`, built by the test's glue; `run_recorded` records
    `(tick, hash)` after every step; test crates include the protocol module by `#[path]` and
    dev-depend on `lactose-task` until the protocol module moves, after which one test-support
    crate outside the boundary, `lactose-sour`, holds the glue; `[harness]` lists `lactose-sim`
    until `lactose-app-headless` exists.
  - *`lactose-ecs` re-exports `Executor`*, which reaches every boundary crate; the boundary lint
    file bans what a boundary crate must not do with it (`frame_worker_count`, `slot_count`,
    `spawn`, `run_graph`, `platform`, `sleep_until`, `execution_log`), so what a boundary crate
    can do with it is run a schedule, block on a future, and build or shut down an executor;
    `ecs` opts out at its one `run_graph` call.
- What the two design reviews changed (round 1: 2 blocking, 11 serious, 13 minor; round 2: 0
  blocking, 3 serious, 8 minor; `edc7eec` was the facade before them and `6845b2f` between
  them): setup functions; sessions as inputs with a held join as an anchor position and the
  policy last; maps for every keyed frame field; a bundle's first table and no pending tables;
  no export off the server; the release radius and `Cancel`; the row-keyed digest; world
  resources must be `derived`; time scale through the frame; `run_passes` apart from `step`;
  `RunError` with the simulation; log retention; a journal that can rebuild a region and, with
  its checkpoint, the session state a save does not carry; the executor bans; private systems
  and lifecycle; one stage per pass, with `ScheduleBuilder::order` and `Registration::order` so
  the assembler orders two crates' sets; pinned regions as the home of world-global persistent
  state; a frame's joins counted in its bundle check and a join outside the atlas reported;
  unloads before loads; the chunk order by capacity; the server's presentation drain in `step`.
- Implementation (the lane and its fix rounds; one review round found 2 serious and 5 minor, the
  next found two minor items, a test gap and a precondition, applied before the merge):
  - A `Leave` cancels a held `Join` without reaching the applier, and a newer `Join` replaces a
    held one.
  - An edge between two sets of a pass absent from the kind is ignored; an edge before
    `sets::INPUT` is a `Cycle`.
  - An unload that cannot export keeps the region.
  - `Release` follows every server unload under either control, except a region the same frame
    loads again.
  - The world hash checks coverage through the world's registry.
  - `Executor::chaos_config` reports the chaos seed and is banned in the boundary lint file.
  - The policy's cost follows the rings: `RegionAtlas::within` walks the cells within the radius
    when that is fewer than the atlas's regions, and the loaded set is read once per sync point
    (the first implementation was O(anchors × atlas), caught by the review).
  - The input component is `transient` with an exclusive writer, never `derived`, since a stale
    sample would drive the simulation from unsaved, unhashed state (the facade's README had
    allowed `derived`; the review's mid-run journal replay diverged on it).
  - `run_passes` prepares `Gather` again after `Extract` and hands the simulation back on
    failure; an export failure in `save_point` also hands it back.
  - `RegionLifecycle::restore_session_state` is how a replay driver restores the held joins and
    requested regions a journal records.
  - The streaming radius is clamped to at least one.
  - A prediction world's asset failures are `predict`'s to take, and `predict` strips them from
    re-simulated frames.
- Driven by: milestone 1, the `sim` facade brief, its two design reviews and the
  implementation's reviews (2026-10-10).

**D119. The `physics-api` facade: what sections 4, 6.1, 9.4, 9.7, 10, 15.1 and 16 left open**
(§4, §5.3, §6.1, §8.2, §8.3, §9.4, §9.7, §10, §15.1, §16, §17, `lactose-physics-api`,
`lactose-ecs`)
- Why: the facade is written before the backend so `physics-rapier`, `predict`, `anim`,
  `present-api`, `net` and the game crates build against the final surface; each choice below is
  one the sections did not settle, and each is a contract in `crates/physics-api/README.md`.
  - *A body is a row, and the sync makes the solver follow rows.* A row is a body while it has
    `Transform` and a `RigidBody` or a `Collider`; a `Collider` alone is a fixed collider in
    every world (cooked static scenery), so a replicated entity that moves carries a
    `RigidBody`. An exclusive system at the physics sync point replays the structural log
    through a cursor and the changed rows of `RigidBody`, `Collider`, `Transform`, `Velocity`
    and `Joint` through a change mark, and tells the backend `create_body`, `remove_body`,
    `set_body`, `set_collider`, `set_transform`, `set_velocity`, `set_frozen`; it keeps the one
    map between rows and the solver, which the derived `BodyHandle` and `JointHandle` columns
    mirror and nothing trusts. A stale cursor or mark means a full diff of every row against
    that map, never a cleared solver, so sleep and warm starts survive the 2^30 stamp rebase and
    a world restore without a physics restore wherever the rows agree. Rules out the backend
    reading the world, bodies created by channel commands, a body table outside rows, and
    `clear`.
  - *Row writes are physics input; the channel carries what a row cannot hold.* A
    `Mut<Velocity>` or `Mut<Transform>` write in a kernel, or a `Commands::set` of them from
    `Stage::Input`, `Stage::Simulation` or `Stage::Post`, is honoured at the next physics sync
    point (a dynamic or fixed body is placed with no sweep, a kinematic body sweeps during the
    step); the write-back's own writes are recognised by value against the record of the last
    write-back, which every write-back replaces, and skipped. `cmd.physics()` carries impulses,
    forces and `teleport` only. The section 6.1 `apply_gravity` kernel therefore works on a
    body. In `Stage::Physics` nothing but the write-back writes a body's `Transform` or
    `Velocity`, and a reader there after the step sees this tick's queries and events but the
    previous tick's rows; readers of stepped rows go in `Stage::Post`. Rules out `set_velocity`,
    `move_kinematic` and `set_body_type` commands, and leaves one stated hazard instead of a
    silent one.
  - *Three systems, not one.* The exclusive sync at the physics sync point; the once-per-tick
    step beside whatever else runs in `Stage::Physics`, which calls the backend inside
    `Job::block_in_place` with the reservation `app` chose; and an exclusive write-back first in
    `Stage::Post`, writing every moved body's `Transform` and `Velocity` per key through
    `get_mut` and `set_if_neq`, so only rows whose bytes changed are stamped and the cost is in
    the moved bodies (0.43 ms per tick for 2,000 moved bodies among 100k rows on the naive ECS,
    against 3.2 ms for a `columns_mut` walk and 1.6 ms for `Commands::set`). Every other
    `Stage::Post` system runs after it with no edge, as after any exclusive system; `app` orders
    other exclusive `Stage::Post` systems against it. Rules out an exclusive step (nothing could
    run alongside), a step that reads columns (a once-per-tick system cannot), and a write-back
    through `Commands::set`.
  - *The physics channel is not cleared at the input sync point.*
    `CommandChannel::CLEAR_AT_INPUT` (default true) is false for `PhysicsCommands`; the sync
    takes the channel at every physics sync point, so a command recorded in `Stage::Input` or
    `Stage::Simulation` applies at this tick's step and one recorded in `Stage::Physics` or
    `Stage::Post` (contact-driven impulses, portal teleports) at the next, in command order.
    `World::channel` reads a channel between ticks without taking it, so the consumer of such a
    channel can snapshot what it has not taken; a restore drops every channel. Untaken commands
    are not in a save or a journal, dropped on load and replay as solver state is; a world that
    records them without a `Physics` resource grows the channel, so `app` inserts one wherever
    such systems run. Rules out a game-side carrier component for contact-driven commands.
  - *Joints are rows.* A `Joint` component with `EntityRef` bodies, spawned owned by one body,
    saved, replicated and snapshotted with the row, created in the solver when both bodies are
    bodies and waiting otherwise; a joint with an emptied reference is removed and not retried
    until its row changes. Rules out joint identities minted by the channel, which no save or
    snapshot would carry.
  - *One `Physics` resource rather than `dyn PhysicsQuery` beside `dyn PhysicsBackend`.* The
    concrete `Physics` wraps the `Box<dyn PhysicsBackend>`; queries take `&self` through
    `Res<Physics>`, the three systems take `ResMut<Physics>`, so the schedule sees every
    conflict (a `Stage::Physics` reader of `Physics` or the events is in a set after
    `sets::STEP`, or the build fails). Rules out two resources aliasing one solver and a lock on
    the hot path.
  - *The blocking wait is this crate's; the pool is the backend's.* The backend never touches
    the executor; `app` builds the pool outside the boundary with frame worker count minus the
    reservation threads, at least one, hands it to the backend crate's constructor, and the
    backend to `physics_api::setup` from a `SimulationBuilder::setup` closure.
    `StepInput::commands` is a plain slice, so a backend's own test steps without a world. Rules
    out a `Job` or an ECS parameter type in the trait and any `lactose-task` edge from the
    backend.
  - *`Transform` and `Velocity` are defined here*, translation plus rotation and no scale, as
    the simulation's transform components for every entity; a crate that needs a transform and
    no physics depends on this crate for the types, which pull no backend. `Teleported` holds
    the last tick a body was placed without a sweep, section 5.3's no-interpolate flag as a tick
    rather than a per-tick mark, so a frame that runs several ticks before its passes loses no
    teleport; extract and the gather compare it with the last tick they covered.
  - *Prediction world: a `RigidBody` row without `ClaimedBy` is a proxy, handed to the backend
    as the effective body with `body_type = Kinematic`; `ClaimedBy` added to or removed from a
    body row is a change of its `RigidBody`, kinematic while unclaimed, the row's type while
    claimed; the backend holds no world-kind rule.* A proxy is moved along its per-tick history
    by `lactose-predict`'s writes to its `Transform` row; predicted entities collide with static
    geometry, other predicted entities and proxies, are pushed by proxies and never push them.
    Rules out proxies absent from the physics world and a proxy status fixed at creation.
  - *Frozen is per body, derived from region modes by the sync*: a body in a region whose mode
    is not `Simulated` is a fixed collider at its last pose; back in `Simulated`,
    `set_frozen(false)` then the row's velocity.
  - *A physics snapshot is the backend's whole world plus the sync's pending work.* The bytes
    are the backend's whole world, with each collider's `Shape` value (an asset reference once
    meshes exist) in place of the backend's shape serialisation; `restore` rebuilds shapes from
    those values while deserialising, with no change flag, wake or mass update; there is no
    `set_collider` replay and no fixed-collider reconciliation. The crate's part carries the
    bookkeeping and what the next sync would see: the log since the cursor, the keys changed
    since the mark, the channel's untaken commands, or that a full diff is due; `restore` puts
    it back after `World::restore`, so a re-simulated tick's sync makes exactly the backend
    calls the first run's did. `corrected(world, keys)` names the rows `predict` wrote without
    recording. The "50 claimed bodies plus N statics" measurement decides how many statics a
    prediction world may hold. Rules out rebuilding from the handle columns, a fresh cursor and
    mark at restore, bodies-only bytes spliced into a solver whose arena, BVH and contact graph
    hold the statics too, and shapes replayed through setters that disturb the solver.
  - *Events are sorted by this crate; `Impact` is the damage phase.* `Started` and `Stopped` go
    to both sides of a pair if either collider has `events`; `Impact`, in every step whose
    normal impulse exceeds the target's `impact_threshold`, goes to a target only if the
    target's own collider has `events`; a tick's events are emitted in ascending (target, other,
    phase) order whatever the backend's order; `Started` may be speculative, so damage reads
    `Impact`. Queries are pure functions of the backend state with ties to the lowest key and
    `overlap` in key order.
  - *The backend's determinism obligation.* Equal outputs, answers and bytes under any pool size
    and whatever ran before, with every `set_*` a no-op on an equal value and every value
    validated. The obligation is section 2's, identical on x86-64 and aarch64; the verification
    items name the two known gaps of the pinned Rapier (platform-libm `sin_cos` on limited
    joints, the sign of zero in SIMD min/max), which the backend lane closes before SOUR's
    architecture mode gates; until then the property is unproven, not excused.
  - *Every component is its scalars then an eight-byte-multiple tail*, sizes asserted for both
    scalar widths; a field added later takes the next free ID and never renumbers another;
    switching the scalar changes `lactose-math`, the `Scalar64` tag and the backend's crate
    names only.
  - *The conformance run is a function of this crate* (`conformance::snapshot_restore`, with a
    `LayoutConfig`) that a backend's test calls once per pool size and layout and compares; its
    scene sets an input tick every tick, spawns from a claimed kernel, carries pending work of
    every kind across the snapshot, repeats every post-restore event in the first run at the
    same ticks, and reports the physics bytes' hash until the order-stability item decides.
  - Named and deferred: cooked collision meshes (`ShapeKind::Mesh` with an
    `AssetRef<CollisionMesh>`, a schema bump; the snapshot then carries the reference), compound
    colliders, CCD, lag-compensation history, hashing the snapshot bytes, per-region body counts
    and contacts of a body.
- What the first design review changed (1 blocking, 10 serious, 8 minor; `50ee157` was the
  facade before it): the snapshot carries the bookkeeping and the pending work instead of a
  fresh mark and cursor; `corrected`; proxies as kinematic bodies; the exclusive write-back and
  the step without `Commands`; `StepInput::commands` as a slice; the layout parameter, the
  richer scene and the physics hash in the conformance run; `Impact` and the impact threshold;
  `CLEAR_AT_INPUT` and `World::channel` in the ECS; the five-column scan; the `written`
  semantics; hidden handle accessors; the full diff instead of `clear`; the sketch's set order;
  `Teleported`; the validation contract; `Collider::events` and `material`; the solver-thread
  invariant.
- What the second design review changed (0 blocking, 3 serious, 8 minor; `a263cd7` was the
  facade between the rounds): the effective body by value in place of a proxy flag, with a claim
  or release as a `RigidBody` change; whole-world bytes with `Shape` values in place of
  bodies-only bytes with a `set_collider` replay and fixed-collider reconciliation; `Teleported`
  as a persistent tick in place of a transient flag; `Impact` only to targets with `events`; no
  edge for `Stage::Post` readers; the input tick and the claimed spawn in the conformance scene;
  the §9.4 text; section 2's cross-architecture obligation stated as unproven rather than
  scoped; the save, journal and no-`Physics` sentences for the channel; `Collider` IDs kept; the
  despawned-key skip in the write-back.
  - Backend dependencies, proposed for the backend lane (the closure resolved for rapier3d
    0.36.0 with `enhanced-determinism`, `parallel` and `serde-serialize`): direct
    `rapier3d-f64`, `parry3d-f64`, `nalgebra`, `glamx`, `glam`, `simba`, `wide`, `num-traits`,
    `rayon`, `serde` and a byte format (`postcard` or `bincode`, shown to be a fixed function of
    the value); transitive `approx`, `arrayvec`, `bitflags`, `byteorder`, `downcast-rs`,
    `either`, `ena`, `equivalent`, `foldhash`, `hash32`, `hashbrown`, `heapless`, `indexmap`,
    `log`, `matrixmultiply`, `nalgebra-macros`, `num-complex`, `num-derive`, `num-integer`,
    `num-rational`, `ordered-float`, `profiling`, `profiling-procmacros`, `rawpointer`,
    `rayon-core`, `rstar`, `serde_arrays`, `serde_core`, `serde_derive`, `slab`, `smallvec`,
    `stable_deref_trait`, `static_assertions`, `thiserror`, `thiserror-impl`, `typenum` and the
    host-only `autocfg`; `matrixmultiply` is the explicit second exception beside `libm`
    (kernels selected by CPU feature, unreachable from the step, with
    `Multibody::inverse_kinematics` banned in the boundary lint file); `deny.toml` denies
    multiple versions of the math crates and glam's `fast-math` and `core-simd` features, since
    `enhanced-determinism` itself enables `scalar-math`, `libm` and `nostd-libm`.
- Implementation (the lane and its fix rounds; the first review found 1 serious and 5 minor, the
  second 2 minor, the third nothing):
  - The sync reads the structural log in order against the rows as they are, then the keys
    changed in the five columns in key order, carrying for each pending key the columns it
    changed in, so a re-simulated tick's sync repeats the first run's calls.
  - It keeps the frozen bodies and each joint's value and bodies, reads region modes through the
    new `World::region_mode` (a lookup, not an entity count), and drops commands naming no body
    before the step.
  - Keys named through `corrected` get `set_body` and `set_collider` with the rows' values, and
    `set_transform` or `set_velocity` only where the row differs from what the last write-back
    wrote; the backend's own equality check filters the rest.
  - A joint whose body was despawned keeps its reference, because the ECS empties none on a
    despawn, and waits for that key.
  - `lactose-sim` now seeds its log-cursor ring with the log's end at build; it used to trim the
    first step's entries at the end of that step, which made tick 2's physics sync a full diff.
  - The feature-tree check compares a harness package that is no shipped binary on the packages
    it shares with each root before it, by name, requiring the same version and features, and
    keeps the full comparison for shipped binaries, since two harness packages always differ by
    themselves and their dev-dependencies (section 8.3 amended); `lactose-physics-api` is listed
    under `[harness]`.
  - Predicted-origin bodies with no alias yet are kept in a set carried with the sync state and
    only those are looked up at a sync point; a bind is permanent, so a bound body's alias stays
    until the body is removed and the cost no longer grows with the bound bodies (50,000 resting
    bodies: 1.34 ms with bound predicted keys against 1.28 ms with `Net` keys, from 7.69 ms when
    the first implementation rebuilt the alias map from every predicted body each tick, caught
    by the review); a body or event the backend reports twice is ordered by value after its key,
    so the copy kept does not depend on the backend's order.
  - A `Stage::Physics` write to a body the step reports is overwritten by the write-back, and
    one to a body it does not report is pushed at the next sync, so game code writes a body's
    rows there never.
  - After a rollback, a bind made after the restore point resolves from the first re-simulated
    physics sync point on, since the world keeps binds across a restore (D114).
  - SOUR's prediction scene carries the conformance scene's pending work at the rollback tick
    and records the physics bookkeeping as well as the rows: a claimed test row holds
    `Physics::body_count` and whether the body despawned on the rollback tick is still a body,
    rewritten every tick in `Stage::Post`. A restore that drops any part of the carried work, a
    lost structural-log removal included, therefore changes a recorded hash even where the test
    backend moves no row; all six restore mutations the reviews wrote fail `cargo xtask sour run
    --pr` on its own.
  - `lactose-physics-api` may depend on `tracing`.
  - Measured at 100k entities with 2,000 moved bodies (release build, load average about 3.5):
    sync 0.96 ms, write-back 0.42 ms; the two column scans (`changed_since` over `Transform` and
    `Collider`, about 0.75 ms together) wait on the chunk-skipping item in the verification
    list.
- Backend (`lactose-physics-rapier`, the lane and its three review rounds, the third clean):
  - Mass: the backend recomputes a body's mass properties when it inserts it and when an update
    wakes it, so a command in the same tick sees the new mass (Rapier otherwise does it after
    the tick's commands).
  - Logs: what the `PhysicsBackend` contract says is logged goes to target `physics`; what only
    this backend logs (a joint whose bodies are not in the solver, a refused solver thread
    class) goes to `quark`.
  - Joints: the axis is checked only for revolute and prismatic joints; limits out of order are
    refused for every kind; physics-api's validation doc and test backend state and apply the
    same rules.
  - History: results do not depend on the order of calls between two steps, but they are a
    function of the row history (every create, remove, write and command, in order), not of
    the rows alone. A body's results depend on every body the world holds or held: far-away
    awake bodies that touch only each other can change them (a pile beside a distant 40-ball
    pile first differs at tick 238; beside far balls teleported every 25 ticks, at tick 274,
    keyed above or below, on 1 and 4 threads; bouncing or sleeping far balls changed nothing
    in 400 ticks), and so do bodies stepped once and removed, whose freed arena slots later
    bodies take. Bit equality holds only between worlds with identical row histories. Reading
    the patched Rapier 0.36.0 points to the cause: all awake bodies are solved as one island,
    and constraint order comes from world-wide structures (BVH traversal order, the contact
    graph's single edge vector with swap_remove, solver-body slots in the awake island, the
    colour buckets); not confirmed by experiment. Rebuilding Rapier's sets in key order on
    every membership change moved the history dependence rather than removing it (a rebuild
    discards contacts, warm-starting and islands); rebuilding every step removed it in every
    case tried, at about 1.3 ms per step plus a cold-contact step (about 3.7 ms against
    1.2 ms) at 2,000 bodies, but would not remove the dependence on far bodies that are
    present, which are part of the current rows. Canonical results per island would need
    Rapier to solve islands separately in a canonical order.
  - Dependency policy: every boundary `clippy.toml` also bans `MultibodyJointSet::insert` and
    `insert_kinematic` (a multibody's mass assembly above five degrees of freedom reaches
    `matrixmultiply`). `cargo xtask features` fails if any boundary package's Rust sources
    (outside the package's own top-level `target` directory and `.git`) test
    `feature = "default"`, `CARGO_FEATURE_DEFAULT` or `CARGO_CFG_FEATURE`, the condition under
    which folding a redundant `default` is sound; the pinned Cargo (1.99.0) gives build scripts
    `CARGO_CFG_FEATURE` with `default` in the list when it is on. A text heuristic: a name
    built at run time or a walk of `env::vars()` is not caught.
  - `rapier3d-f64` 0.36.0 is built from `vendor/` with one six-line patch, the first case of
    D123.
- Driven by: milestone 1, the `physics-api` facade brief, its two design reviews and the
  implementation's reviews
  (2026-10-10).

**D120. The `input` facade: what sections 4, 5.3, 6.1, 8.2 and 9.2 left open** (§3, §4, §5.3,
§6.1, §8.1, §8.2, §9.1, §9.2, §15.1, `lactose-input`, `lactose-input-host`, `lactose-ecs`)
- Why: the facade is written before its implementation so `app`, `predict`, `net`, presentation
  and the game crates build against the final surface; each choice below is one the sections did
  not settle, and each is a contract in `crates/input/README.md` or
  `crates/input-host/README.md`.
  - *Input is two crates, split at the boundary.* `lactose-input` (the sample, its byte image,
    the applier, the redundancy window) is inside the determinism boundary: the floats a kernel
    reads from a sample (`aim`, `world_movement`, `radians`, `to_vec2`) are computed there,
    under the boundary lint file and the boundary build ID, with no dependency a boundary crate
    does not already have, and `PlayerInput` is therefore a boundary component type the section
    9.1 protocol hash covers. `lactose-input-host` (raw samples, the quantiser, the
    latest-sample slot, sources, mapping) is outside, and no boundary crate may depend on it;
    the one thing that crosses is the integer sample its quantiser produces. Rules out a crate
    that straddles the boundary (§3) and a path lint to police it.
  - *The sample is twenty bytes of integers and the sample bytes are the component bytes.*
    `PlayerInput` (`input::player_input`, `transient`) holds a movement vector in sixteen bits
    per axis, absolute yaw (`u16`, wrapping) and pitch (`i16`, a quarter turn each way) in
    1/65536 of a turn, and three 32-bit action sets (`held`, `pressed`, `released`); its
    `repr(C)` image is `PlayerSample::sample`, the journal's bytes and the wire's, with no
    version byte, since the component's schema version is covered by the protocol hash. A
    journal replays on the build that wrote it; the checkpoint records no schema. Rules out a
    separate wire struct, floats in the sample, and a derived value in the image.
  - *`validate` enforces what kernels rely on*: the pitch limit, no `i16::MIN` axis, the unit
    disc plus the quantiser's rounding (`x² + y² <= AXIS_SCALE² + 2·AXIS_SCALE`), `pressed ⊆
    held`, `released ∩ held = ∅`. `net` must validate every received image and treat a failure
    as lost, so a forged image never reaches a frame. Rules out diagonal speed and an edge every
    tick from a modified client.
  - *`aim` is computed, never stored.* Section 6.1's `input.aim` becomes `input.aim()`: a `Vec3`
    from the two angles through `lactose_math` (`sin_cos`, products in a stated order) at read
    time, so a client and the server derive identical bits from identical bytes and no float the
    client computed enters the journal. `world_movement` and `Yaw::rotation` likewise. Rules out
    a `Vec3` field the server would have to trust or re-verify.
  - *The world frame is right-handed, `+Y` up, yaw zero facing `-Z`, positive yaw turning left
    (`Quat::from_rotation_y`), positive pitch up.* Every direction a kernel reads from a sample
    comes from `aim`, `world_movement` and `Yaw::rotation`, so the convention lives in one
    place; the camera rig uses the same one; glTF assets face `+Z`, so the cooker turns them
    half a turn about `+Y` so a character faces `-Z` at yaw zero (RICOTTA's rule, milestone 1
    asset track). Rules out each game crate re-deriving directions.
  - *Edges are the client's.* `pressed = held.without(previous_held)` and `released =
    previous_held.without(held)` are computed once by the `Quantiser`, which holds the previous
    `held` and runs once per tick (so catch-up ticks from one frame carry no repeated edge), and
    travel in the sample; the server and a replay keep no button history and a sample is a pure
    value. `throw` in 6.1 becomes `input.pressed.contains(Action::Throw)`. Rules out server-side
    edge detection and per-player state in the applier.
  - *Named bits and generic bits share one 32-bit mask.* Bits `0..16` (`0..Actions::GAME_FIRST`)
    are the engine's: `0..5` are `Jump`, `Throw`, `Interact`, `Sprint`, `Crouch`, and `5..16` are
    reserved for actions the engine names later (`Action` is non-exhaustive). Bits `16..32` are
    the game's, assigned by the milestone 5 action map: bit `k` is `GAME_FIRST + k`. Naming more
    adds variants and changes no encoding (corrected by the implementation, below).
  - *The applier writes with `World::insert`* and finds the player's entity through the ECS
    claim index: `World::claimed(player)` yields the rows with `ClaimedBy { player }` in
    ascending origin-key order, from an index the world maintains wherever `ClaimedBy` enters or
    leaves a row and carries in a snapshot, and `player_entity` is the first of them that
    carries `StreamingAnchor`, so a lookup costs the player's claims and never a scan
    (`World::query` today visits every live entity: 5.4 ms per lookup at 100k rows). Never
    cached: a side table is what `sim` forbids and a rollback would leave it stale. A player
    with no entity yet (a held join) is `Ok(())`, not an error. A row keeps the component across
    ticks and reads `IDLE` on a tick without a sample.
  - *The transient column's declared writer is an empty exclusive system*
    `input::player_input_writer` in `sets::APPLY`, `Stage::Input`, after `sim::sets::INPUT`,
    declaring `writes = [PlayerInput]`: the write happened inside `sim::apply_inputs`, and the
    declaration is what lets a kernel in `Stage::Simulation` take `&PlayerInput` (confirmed on
    the real ECS by the review: without it, `TransientBeforeWriter`).
  - *The redundancy window is a tick-indexed window of eight samples,* `SampleHistory`, the same
    type on both ends: the client inserts each tick (`insert`) and packs `iter()`; the server
    inserts every packet entry through `insert_bounded(consumed, ..)`, which stores only ticks
    in `(consumed, consumed + CAPACITY]`, so an entry off the wire can neither move the window
    away from the ticks still to come nor resurrect an applied tick; the first delivery of a
    tick wins; ticks compare by wrapping difference. `REDUNDANT_SAMPLES = 8` is not part of any
    schema; `net` may amend it and may keep a deeper buffer of its own.
  - *Substitution and its repair are pure functions.* `PlayerInput::substitute` refines §9.2's
    "replaced by the previous input": the previous applied sample with `pressed` and `released`
    cleared, `IDLE` for a player with none. `after_substitute(applied)` recomputes the edges of
    the first delivered sample after a substitute against `applied`, the last substitute
    applied, replacing the ones the client computed against its own lost sample (`pressed =
    held.without(applied.held)`, `released = applied.held.without(held)`), so the swallowed
    press fires and no release or re-press of an action no applied sample held can; what it
    costs is a tap, or a release and re-press, wholly inside the gap. `net` applies it once and
    the journal records the applied bytes. When a tick counts as lost, the jitter window and
    telling the client are `net`'s; `net` sets `PlayerSample::substituted`.
  - uantisation is one host-side step,* `Quantiser::next`: floats to integers with stated
    rounding, the movement clamped to the unit disc, the pitch to its limit, the yaw wrapped
    into a turn; a `RawSample::latched` set, which `hold` adds to and only `Quantiser::next`
    clears, makes a tap pressed and released between two polls one tick of held, so a sub-tick
    tap reaches a sample; the client simulates the quantised value it sends (§9.2). The raw
    sample combines sources instead of overwriting them: `begin_frame` empties `held` and the
    movement each frame, sources `hold` (union), `offer_movement` (the larger magnitude wins)
    and `turn` (the yaw is wrapped into `[-PI, PI)` after every delta, so an accumulated `f32`
    yaw never grows coarser than a sample unit).
  - *`LatestSample` is a sequence-locked slot holding the raw sample,* published by the input
    task and read by the late latch without locking. Publish: compare-exchange (`Acquire`) the
    sequence from even to odd, then `fence(Release)`, then the words `Relaxed`, then store the
    next even value (`Release`), so a publisher may wait for a publisher, never for a reader,
    two publishers cannot interleave, and the fence orders the word writes after the sequence
    was taken (without it a single publisher tears under the memory model). Read: load the
    sequence `Acquire`, the words `Relaxed`, `fence(Acquire)`, reload the sequence, retry on
    mismatch or odd. It holds the raw floats because the latch wants the freshest rotation and
    the quantisation step is far below a frame's resolution.
  - *Milestone 5 gets named places only:* `sources::Source::poll(&mut self, raw: &mut
    RawSample)` and an empty `mapping` module in `lactose-input-host`; nothing touches a device
    or a clock.
- What the design reviews changed (round 1: 1 blocking, 2 serious, 8 minor; round 2: 3 minor, the seqlock's release fence, replaced rather than unioned edges after a substitute, and the latched tap; `36a7083` was the facade
  before it): the crate split at the boundary and `lactose-input` inside it; the ECS claim index
  and `player_entity(&World, ..)` over it; `validate`'s four rules and `net`'s duty to validate;
  the CAS seqlock contract; `insert_bounded`; `after_substitute`; the `Quantiser`; the glTF
  half-turn and `Yaw::rotation`; the raw sample's frame start and combining rules and the
  wrapped yaw; the `released` field; `Ok(())` for a player without an entity.
- Implementation (milestone 1; the first review found 3 minor, the second none): every
  `todo!()` of `lactose-input` is replaced with no signature change; `lactose-input-host` is still
  the facade. `SampleHistory` is a ring indexed relative to its newest slot, so it is correct
  across the `u32` wrap for any `CAPACITY`. The README's "the lookup costs the player's claims,
  not the world" is proved by the answer among 100,000 rows and zero allocations under the tagged
  global allocator in the test binary; the time is measured by an ignored test and reported: 46 ns
  per lookup at 1,000 other rows and 51 ns at 100,000 (release, Apple M1, 64 players, 2,000
  rounds; load averages 2.80 2.31 2.44 before, 3.87 2.56 2.53 after). `World::insert` allocates
  twice per call even when overwriting (the erased copy), so the applier costs two allocations
  per sample; the applier itself adds none. `Actions::GAME_FIRST` (16) is a fixed constant: bits
  below it are the engine's, named by `Action` or reserved, and the game's bit `k` is
  `GAME_FIRST + k`, so naming more engine actions never renumbers a game's bits; the facade's
  "bits `5..32` are the game's" contradicted "naming more adds variants and changes no encoding"
  and is corrected above. The game has 16 bits; a bit past 31 is ignored by `with_bit` and false
  from `contains_bit`. `lactose-input` joins `[harness]` in `tools/xtask/ci-policy.toml`, and
  CI's release step runs its golden bits.
- Driven by: milestone 1, the `input` facade brief and its first design review (2026-10-10).

**D121. The `net-api` facade and the `net-host` split: what sections 4, 5.3, 8.1, 8.2, 9.1, 9.2,
9.3, 9.4 and 14 left open** (§4, §5.3, §6, §8.1, §8.2, §9.1, §9.2, §9.3, §9.4, §9.5, §10, §11,
§14, §15.2, `lactose-net-api`, `lactose-net-host`, `lactose-ecs`, `lactose-reflect`,
`lactose-sim`)
- Why: the facade is written before `lactose-net` so that the milestone 1 netcode spike, `app`,
  `predict`, presentation and `persist` build against the final surface; each choice below is
  one the sections did not settle, and each is a contract in `crates/net-api/README.md` or
  `crates/net-host/README.md`.
  - *Two crates at the boundary, as `input`.* `lactose-net-api`, inside: the staged frame and
    its inbox, the gather and its frame, the relevance message, the `RemoteSlot` component and
    lag compensation's place, under the boundary lints and the build ID. `lactose-net-host`,
    outside and absent from `[boundary_internal]`: quantisation, the handshake, the transport,
    packets and handles, per-client interest, the interpolation buffer, clock sync and the
    control messages. What crosses into the simulation is the staged frame, which the network
    thread builds from what the host crate decodes. Rules out a module-split lint (a
    `disallowed-types` entry fires only in type position; a struct literal or an inferred result
    slips) and a host crate a boundary crate could name.
  - *One staged frame per server tick, shared by `Arc`, through an inbox in the world; a frame's
    control fields enter the `TickInput` of the step whose hook pops it.* The network thread
    builds a `StagedFrame` per server tick; `app` pushes it into each client world's
    `StagedInbox` between runs (the call is `#[must_use]`; the inbox refuses a frame not after
    the newest tick it ever accepted, so no row goes back in time across frames, and under the
    receiver rules below it never has to) and folds `due(t)` for `t` the tick the next step will
    run (`Simulation::tick` + 1, the tick the hook receives, since the step advances the world
    before the sync point) into that step's `TickInput`: the time scale (the last `Some`), and
    region changes per region in frame order with the later winning, except that `Irrelevant`
    then `Relevant` is an unload plus a load; a full frame first discards the step's earlier
    region changes, then unloads every loaded region, so the step loads exactly what the full
    frame names. The client's worlds are built at, or `set_tick` to, the tick `JoinAccepted`
    names. Every keyed field is a map; rows live in one arena and reading allocates nothing.
    Rules out a channel inside the world (an mpsc `Receiver` is `!Sync`; a `Mutex<Receiver>`
    takes no lock through `get_mut`, but it still puts a channel in a world and lets the hook
    see a frame before `app` has folded its region changes), folding every pending frame into
    one step (which lost unloads), and folding `due(Simulation::tick())` (which lost every frame
    that arrived in time).
  - *A staged spawn is an upsert when its key is live; the hook despawns nothing; a change of
    claim is a spawn.* A region becoming relevant loads its rows by bundle and the stream spawns
    them; a full frame spawns entities the client holds; a claim change must reach the
    interpolation slot. So a spawn of a live key writes each component whole (so an equal value
    does not dirty the row) and sets or removes `RemoteSlot`; a full frame reloads every region
    through the `TickInput`; the network thread clears its handle mirror and slots before a full
    frame. Rules out a spawn that refuses a live key and a hook that despawns what a frame omits
    (it would remove cooked rows with no replicated component).
  - *Moves and component removals are frame fields.* `StagedFrame::moves` from the gather's
    `Moved` entries, applied after the spawns and before the rows through
    `World::move_to_region`; `StagedFrame::removed` from the `Removed` entries of replicated
    components, applied after the moves and before the rows (a row of the same frame re-adds the
    component), through `World::remove` per registered type until the ECS's `remove_erased`; the
    wire names the region each quantised position is relative to; an entity moving out of the
    client's set leaves with a tombstone. Rules out learning a move from `InRegion` (derived and
    world-owned) or from a row (a move changes none), and a client that keeps a removed
    component's last value forever.
  - *Rows are whole images keyed by origin key; precision is a flag, not a second path.* The
    network thread dequantises before staging; `exact` marks the full-precision rows of claimed
    entities (the reconciliation record) against the dequantised rows of proxies; the resync and
    late-join message is a `StagedFrame` with `full`. Rules out per-field deltas and separate
    reconciliation and snapshot types.
  - *The gather decides full or incremental first, then selects by stamp through the ECS's
    erased `changed_since`.* `net::gather` is an exclusive `Pass::Gather` system taking `Job`
    and `ResMut<GatherState>`; it first decides from `World::is_current(mark)` and
    `StructuralLog::since` on a copy of the cursor; only an incremental gather skips chunks by
    column tick before any fork, forks over the rest with `fork_range`, selects rows with
    `ErasedColumn::changed_since(mark)` exactly as a typed consumer would, appends the
    structural log since its cursor, sorts by (region, key, component), and merges into the
    frame `app` has not taken; a full pass takes every row of every replicated column with no
    chunk skipped, replaces the packer's mirror, voids every client's acked ticks and is
    followed by a resync, never a resend from the frame alone; `app` takes the frame between
    runs. `SimulationBuilder::log_retention` must be at least the catch-up cap, or every gather
    after a catch-up frame is full; `build` cannot check it. Rules out a tick comparison (which
    re-sent every row of the last tick), a strict-after one (which lost direct writes), and
    detecting staleness inside the scan (which set `full` on a frame that had skipped chunks).
  - *The packer keeps a mirror; a claimed entity is whole in every packet that carries it.* A
    delta frame holds only changed rows, and whole sends, resends and section 9.4's every-field
    updates need the rest, so the packer keeps every replicated row of every relevant region,
    built from the first full frame and every later one: rows replace, `Despawned` removes the
    entity, `Removed` drops the row, `Moved` re-homes, and a full frame replaces the mirror (it
    carries no structural entries, so an entity despawned in the gap would otherwise be resent
    as a spawn). Rules out amending 9.4.
  - *Handles are assigned on the server and mirrored on the client under the receiver's rules.*
    A sixteen-bit index and generation (the generation comparison wrapping); every spawn carries
    its handle, the client records it with `insert`, releases on the tombstone, and clears when
    it stages a full frame; a ring of 64 packets and an acknowledgement window of 32. The
    receiver stages a server packet only if its tick is after the newest frame it staged and
    drops any other whole, unacknowledged, so the server resends it; from a resync request or
    the join until the full frame it stages nothing, drops every packet not after the full
    frame's tick, and clears its mirror and slots only then; the server clears the client's
    handles on a resync and re-enters every relevant entity. With these, the inbox's
    newest-ever-accepted rule holds by construction. Rules out a client that assigns, which
    drifts under release timing and reordering, and a full frame on the reliable channel
    overtaken by later deltas and refused.
  - *Interpolation slots travel through the staged spawn and `RemoteSlot`; the sampler takes
    slots; the sequence lock is atomics only with stated orderings; the teleport is a tick.* The
    writer stores the odd sequence, a release fence, the words, the even sequence with release;
    a reader loads the sequence with acquire, the words, an acquire fence, the sequence again (a
    relaxed first load tore on aarch64, acquire did not); `ReceivedState::teleported` is the
    `Teleported` value so a teleport is told under any arrival order, and the sampler holds on
    either side of it. Rules out a key-to-slot directory and a boolean teleport computed at
    arrival.
  - *The protocol hash covers replicated and hashed types plus the protocol constants; its
    per-type encoding is `lactose-reflect`'s `TypeInfo::write_schema`; a type registered on one
    side only is derived and unreplicated, or the handshake refuses.* Section 9.1's "boundary
    component types" is not knowable from the registry; what must agree is what crosses the wire
    and what a desync digest compares. Rules out hashing the dump string and a covered set that
    a presentation type on one side would break.
  - *The transport sees connections as events; quantisation goes through `lactose_math` with its
    limits stated; control messages travel on a reliable ordered channel `net` builds over the
    datagrams.* `JoinAccepted` carries the player and the start tick; `OverlayRequest` follows a
    digest mismatch; the readiness hold moves simulation state and enters the server tick as a
    journaled input at milestone 4 (`SessionEvent::Ready { region }` or a `TickInput` field),
    `ClientInterest::ready` being the network's copy only.
  - *`app` moves `net`'s data in and out of the world between runs.* `StagedInbox` and
    `GatherState` are service resources and not world state.
  - Named and deferred: an erased row write and removal in the ECS (`World::insert_erased`,
    `World::remove_erased`, `World::spawn_erased`), which the upsert, the removal and milestone
    4's generic hook are written against; the packer and every byte encoding; the clock-sync
    estimator; the lag history's shape; the region-origin table; a `lactose-predict` build check
    that prediction-world systems read only replicated or derived components.
- What the first design review changed (2 blocking, 7 serious, 9 minor; `2b54eed` was the facade
  before it): the split into `net-api` and `net-host`; the upsert and the full-frame reload
  instead of a despawn step; `StagedFrame::moves`; `ErasedColumn::changed_since` in the ECS and
  the gather by stamp; `ConnectionHandles::{insert, clear}`; `StagedInbox::due`, the fold rule,
  the start tick and `#[must_use]`; the newest-ever-accepted rule; the packer's mirror and whole
  claimed entities; the exact sequence-lock orderings; the skip-then-`fork_range` strategy with
  its numbers; the retention rule; the quantisation caveats; `teleported: Tick`; the readiness
  hold as a journaled input; `JoinAccepted`, `OverlayRequest` and the reliable channel; the
  one-sided-registration rule and `TypeInfo::write_schema`; the `Mutex::get_mut` and resync
  corrections.
- What the second design review changed (1 blocking, 2 serious, 3 minor, three of them
  introduced by the first fix; `f398b98` was the facade before it): the fold tick, which had
  dropped every frame that arrived in time; the receiver rules and the resync sequencing;
  `StagedFrame::removed`; the full frame's fold; the gather's decision first and the mirror
  replaced; claims as spawns of a live key; failed entries logged and skipped; the wrapping
  generation.
- Implementation of `lactose-net-api` (milestone 1; the first review found 2 serious and 4 minor,
  the second 3 minor in documentation, applied at the merge). `StagingFrame::merge` applies the
  newer frame's structural entries to the older pending rows: a row of an entity it despawned, or
  of a component it removed, is dropped, and a row of an entity it moved is re-homed to the
  entity's new region. Of two rows for one (key, component), the row with the later tick wins,
  and on equal ticks the newer frame's. A full older frame stays full and keeps `structural`
  empty. So a frame not taken, merged with the next, equals the frame one gather over both
  windows produces. `finish` puts a frame in canonical order (region, key, component), with the
  newest tick winning, and rebuilds the arena, so equality of finished frames is equality of
  content; `StagedFrame` compares by content, not by arena layout. The gather forks over the
  kept chunks with one `OnceLock` cell per chunk, joined in chunk order, so the frame is
  independent of layout and worker count; it measured 0.46 ms median on 4 workers and 0.61 ms
  on one (release, 100k entities by 20 components, 4,000 changed rows), and a full gather of
  2M rows 190 to 215 ms. Tick order in `StagedInbox` and `push_row` wraps, matching the world's
  tick. `StagedFrame::push_row` (`#[must_use]`) refuses a row older than or equal to the one
  held, a row from a tick after the frame's, and a row of another length than the one held; the
  receiver checks each row's length against the registry before it pushes it, so a held row is
  overwritten in place and the arena never grows with the packets.
  `StagedInbox::oldest_after(tick)` is the tick of the oldest queued frame after `tick`, the
  first one `due(tick)` leaves out; `lactose-predict`'s `Behind` reads it. `StagedInbox::reset`
  empties the inbox and forgets its accepted tick: a re-join, whose ticks may lie before the old
  session's, resets the inbox of every client world it moves with `set_tick` ("built at, or
  `set_tick` to" above), the receiver restarts its newest staged tick, and frames of the old
  session not yet pushed are dropped, never pushed after the reset. A `Behind` or `Divergence`
  resync does not reset. `RegionRelevance::to_bundle` checks the overlay against the baseline
  whether it was carried (`Full`) or cached (`Digest`). The decision "current cursor, stale
  mark" is defensive: every epoch change resets the structural log too, so the public API never
  reaches it, and it is held by a unit test. `lactose-net-api` is listed under `[harness]` in
  `tools/xtask/ci-policy.toml`; its SOUR test replicates a scene with cooked baseline rows, a
  bound predicted spawn, untaken frames merged, a region unloaded and reloaded, and a claim and
  its release, asserts that every hook path ran, and records server hash, frame digest and
  client hash per frame.
- Driven by: milestone 1, the `net-api` facade brief and its two design reviews (2026-10-10).

**D122. The `predict` facade: what sections 5.1, 7, 9.2, 9.4, 10, 11 and 22 left open** (§5.1, §5.3,
§6, §7, §9.2, §9.4, §10, §11, §16, §22, `lactose-predict`, `lactose-ecs`, `lactose-sim`,
`lactose-task`, `lactose-net-api`, `lactose-net-host`, `lactose-physics-api`)
- Why: the facade is written before its implementation so the client's `app`, `lactose-net`,
  presentation and the milestone 1 netcode spike build against the final surface; each choice below
  is one the sections did not settle, and each is a contract in `crates/predict/README.md`.
  - *`predict` owns the prediction `Simulation` and runs by value.* `Predictor::{step, reconcile,
    passes}` take the predictor and hand it back, as the simulation's runs do, because a run shares
    the world with jobs on other workers; a failure whose world survived (before anything ran, a
    step whose hook did not run, a re-simulation that stopped) hands it back inside `PredictError`
    with the ring trimmed to the world's tick, `None` after `Cancelled`. `new` refuses a world that
    is not `Prediction` or lacks a `StagedInbox`, and fixes whether the world has `Physics`; `step`
    refuses when that changed or when the hook did not run, which catches a `dyn StagedState`
    inserted over it; `resync` reinstalls the hook, the repair. Rules out a `&mut self` API over an
    `Option<Simulation>`, which could express a torn world only by poisoning, and a hook silently
    replaced or irreparably replaced.
  - *Frames are applied by the hook inside the tick; reconciliation runs between runs after the
    frame's steps.* `PredictionHook`, installed by `Predictor::new`, pops the frames due at its tick
    (section 9.2: staged state is applied at the input sync point and only there), applies every
    structural field as the replicated world's hook does without `RemoteSlot` so the prediction
    world holds every relevant entity as a proxy, writes each proxy's `Transform`, `Velocity` and
    `Teleported` as that tick's position, binds every predicted origin, keeps `ClaimedBy` and
    `StreamingAnchor` on the player's own entity (`LocalPlayer::entity`, held once in the
    predictor's record and replaced between runs by `set_local`; the hook itself holds nothing)
    before and after the frames, despawns a predicted spawn of the local player whose input tick the
    acknowledgement has passed, and records the claimed entities' facts; `reconcile` reads the
    record, never the inbox. Rules out a driver that writes rows between runs and a hook that cannot
    be replayed.
  - *A ring slot is the world snapshot after the tick, the physics snapshot beside it (or none,
    counted, if it failed), the `TickInput` that produced it, the frames popped at it (the proxies'
    per-tick history of section 10) and a record of every replicated component of every claimed
    entity as bytes, read through `World::get_erased`.* A frame's facts are compared at the slot of
    the frame's tick and nowhere else; a fact whose region that slot lacks is skipped, the region
    being the fact's or the one the predictor last knew the entity in. The predictor remembers the
    region of every entity the local player has claimed from its first fact or record until the
    server's tombstone or release fact for it, never dropping one for the ring's age and keeping
    them across a resync, so a claimed entity the client despawned too early is re-spawned by its
    next fact however long the server left it unchanged; the map is bounded by the server's live
    claims. A rollback restores the slot, writes the facts there, tells the physics what was
    corrected, re-takes the slot (keeping no physics snapshot when the slot had none, since the
    physics was not restored and a snapshot then would hold a later tick's solver state) and
    re-simulates to the present from the ring's inputs with the frames replayed, comparing again at
    every later tick a frame has facts for, in frame order. A call first finds the newest full frame
    it popped; every frame popped before it is from before the restart whatever its tick, and
    contributes no correction point, substitution or `TooOld`. The ring is a window of `depth`
    slots, the oldest the horizon; a frame older than it is a resync (section 9.4) unless it is from
    before the ring's restart. Rules out comparing against the live world, comparing at a later slot
    (one correction per frame for a round trip), a ring that confirmation must drain, and a map of
    known entities that the ring's age prunes.
  - *Corrections are written stamped.* A correction is a change of state, the server's, and a
    `Changed<T>` reader in the re-simulated ticks sees it once, so a derived component a prediction
    system maintains from a replicated one follows the correction; written without recording it
    never would (a cache from `Changed<Position>` stayed at the mispredicted value through thirty
    re-simulated ticks in the review's probe), and the build check lets prediction systems read
    derived components. This reverses, for this crate, D114's "a correction written without
    recording leaves them alone" and supersedes, for this crate, D119's "`lactose-predict` corrected
    the rows without recording the writes": `crates/physics-api/README.md` and the `corrected` doc
    say so, and D119 stands as history. `ColumnMut::write` without recording stays in the ECS for
    any other consumer. `physics_api::corrected` is still called with the written keys, the contract
    the backend lane was given and the one the sync's pending set serves whatever the writer's
    stamping; with stamped writes the sync's fresh mark after `restore` would see them regardless.
  - *Re-simulation does not share the catch-up cap.* A rollback's re-simulation completes to the
    present in the `reconcile` that detects it, bounded by the ring depth; it costs the client no
    ticks of lead while the depth times the per-tick cost `app` measured fits the frame
    re-simulation budget, which is what `set_depth` is for: `app` sizes the depth from that budget
    divided by the measured cost, under its own cap. `DEFAULT_DEPTH` is 60, a placeholder and not a
    sized value; with a depth `app` has not sized, a rollback to the oldest slot can overrun the
    frame, and the ticks the pacer then finds due are catch-up under the cap. Sharing the cap would
    cost lead on every correction: a correction's depth is about a round trip plus margin, more than
    four ticks at 100 ms, the convergence storm after a dynamic-body correction corrects on
    consecutive frames, and clock sync's three percent could not restore the lost lead (the review's
    probe: one tick lost per frame at 60 fps for any correction of four or more ticks). Sections 5.1
    and 11 are amended; `app` passes zero to `TickPacer::poll` until `lactose-task` drops the
    parameter; `TickBudget::behind` keeps its meaning, catch-up alone beyond the cap, a clock resync
    and never a world resync.
  - *Full precision means canonical bytes per field, no tolerance.* A fact is a claimed entity's
    presence, region, claim, a component's presence, or its bytes; fields compare over every ID'd
    field of the schema, scalar leaves by canonical bits. The same build on the same bytes gives the
    same bits, so a difference is a different input. Rapier's uncorrected solver state makes a
    corrected dynamic body differ again for a few ticks; those are counted as corrections, and a
    tolerance is a milestone 4 decision against the spike's baseline. `Divergence` is counts of
    entities and fields; a magnitude across mixed units means nothing and this crate computes no
    float.
  - *A substituted tick replaces the ring's sample and re-simulates from the tick before; every
    unsubstituted tick after a substituted one is repaired through `after_substitute` once a frame
    at or after it is processed, found by input tick since a frame spans several ticks;
    substitutions and repairs apply in ascending tick, are skipped at or before the ring's restart
    or when their frame was popped before the call's newest full frame, a repair whose preceding
    substitution was skipped is skipped too, and they are `TooOld` when the tick before them
    precedes the horizon.*
  - *A full frame is a rebuild and `resync` is a restart.* Both set `Ring::rebuilt_at`; while the
    ring holds that slot, a frame for an earlier tick is from before the restart: its facts and
    substitutions are skipped and its tick confirmed, checked before the horizon, so a resync is one
    resync and a join is one rebuild. Of several full frames in one call only the newest restarts
    the ring. The rebuild tick's own sample is lost and the local inputs between the full frame's
    tick and the restart are not in the rebuilt state, so the first frame at or after the restart
    corrects once. `resync` also reinstalls the hook and keeps the known regions. `app` builds the
    prediction world at the join tick plus its lead.
  - *The local player's entity is an explicit `LocalPlayer::entity`, replaced by `set_local`.*
    `StreamingAnchor` is derived and unreplicated, and the lowest claimed key is not reliably the
    player's (cooked ids sort above runtime ids). `JoinAccepted` should carry it, and a change at a
    re-join is a `net` control message (below).
  - *The build check of D121 is a pure function in `predict`, run at the prediction world's build
    through a check function `app` hands the builder.* It refuses any component a prediction-world
    system names, in its kernel or finalize table, that is persistent and not replicated (`OwnedBy`
    and `PredictedOrigin` among them), and any exclusive prediction-world system whose
    `SystemDescriptor::key` is not in `EXCLUSIVE_SYSTEMS`, the engine's exclusive systems that run
    there by system key (`sim::begin_tick`, `sim::apply_frame_regions`, `sim::apply_sessions`,
    `sim::apply_inputs`, `sim::apply_staged_state`, `sim::apply_region_policy`,
    `input::player_input_writer`, `physics_api::sync`, `physics_api::write_back`), since `&mut
    World` shows no table and a set name can be joined by any system; an engine crate that adds or
    renames one changes the list in the same commit; test-support scenes (physics-api's conformance
    `input_tick`) are not shipped and not listed. Resources are not judged: world resources are
    derived by `sim`'s rule; region resources are corrected only by relevance, a `net` item.
  - *Every cost the spike records is a value:* `Reconciled`, `Correction`, `Divergence` and
    `PredictionStats`; `app` measures time around `reconcile` and divides.
  - *The implementation lane starts only when what it is written against runs:* the four erased row
    methods of `lactose-ecs` below, `lactose-net-api`'s `StagedInbox`, and `lactose-input`'s
    `substitute`, `after_substitute`, `player_entity` and `PlayerInputApplier`. No typed fallback
    exists.
  - Named and deferred: `SimulationBuilder::check`, `JoinAccepted::entity`, `TickPacer::poll`
    without its parameter, forwarded inputs, a dynamic-body tolerance, per-tick region resources and
    a replicated `OwnedBy`, the visual blend, copy-on-write snapshots or a cap on the statics a
    prediction world holds as rows (the review measured a full-copy `World::snapshot` at 134 to 149
    µs and 2.43 MB with 10,000 statics as rows, over the 100 µs budget before physics).
- What the first design review changed (1 blocking, 6 serious, 9 minor; `a89817a` was the facade
  before it): the erased ECS precondition in place of a typed fallback; stamped corrections;
  re-simulation outside the cap; comparison at the frame's slot only; the repair rule by input tick;
  the restart rule before the horizon and for substitutions; `resync` as a restart; the anchor in a
  rebuild and after the frames; `set_local` and the lead at join; the inbox precondition and the
  hook-ran and physics-presence checks; the exclusive allowlist; the sketch's setups and hand-backs;
  the slot without physics; the dropped scalar error. What the second review changed (2 serious, 7
  minor; `cebd949`): the allowlist by system key in place of set names; the known regions bounded by
  the server's facts in place of the ring's age; the newest-full-frame-first rule; the skipped
  repair after a skipped substitution; the no-lead-cost condition and `DEFAULT_DEPTH` as a
  placeholder; physics-api's contract text; `resync` reinstalling the hook; one copy of `local`; the
  re-taken slot without physics.
- Implementation of the ECS and simulation preconditions (lane ecs-erased; the first review found
  3 minor, the second none): `World::{get_erased, insert_erased, remove_erased, spawn_erased}` in
  `lactose-ecs` and `SimulationBuilder::check` in `lactose-sim`. The erased methods share the typed
  paths' validation and index maintenance; a registered resource key is `InvalidData`;
  `insert_erased` writes only when the bytes differ, so an equal write stamps nothing; a
  placeholder in a reference field is refused by name ("placeholder reference in `<field>`"), where
  a typed write stores the empty reference, because erased bytes come from the wire or a recorded
  slot; a refused write, typed or erased, leaves the point unchanged (typed `insert` now checks the
  registry before the point moves). `check` may be called more than once; checks run in the order
  added at `build`, after the setup and registration functions and before any schedule is built or
  prepared, each given the world's registry and every system of the builder's kind in add-call
  order (sim's six input systems first, `sim::save_copy` last on a server); the first error fails
  the build as returned. `StageBuilder` has no public constructor, so a registration function adds
  only through the builders it is handed, which the order `check` sees relies on.
  `lactose_predict::check_prediction_reads` is still `todo!()`, so the tests use local check
  functions. Generic code (the erased writes, the reference walkers) trusts `FieldKind::EntityRef`:
  a type that declares that kind without being an `EntityRef` is read and rewritten as one.
- `TickPacer::poll` without its parameter (lane task-small): `poll(now)` counts catch-up alone
  under the per-frame cap, the test `catch_up_is_capped` replaces
  `catch_up_is_capped_and_shares_the_cap_with_resimulation`, and the timer-root sketches of
  `lactose-sim` and `lactose-predict` call it without the zero; the deferred item is closed.
- Driven by: milestone 1, the `predict` facade brief and its two design reviews (2026-10-10 and
  2026-10-11).

**D123. A third-party crate is built from a patched copy only under `vendor/`, by one rule
for every crate, checked by `cargo xtask vendor`** (§8.2, §8.3, §9.1, §16, §20,
`crate-tiers.toml`, `deny.toml`, `.gitattributes`, `docs/agents.md`, `tools/xtask`)
- Why: the Rapier backend needed one route moved from the platform library to `libm` (the
  angular joint limit's `sin_cos` in `joint_constraint_helper.rs`, six changed lines) and no
  document said whether a boundary crate may be patched, so the lane vendored the crate and
  proposed a rule. Patching is admitted because the alternative, refusing `Joint::limited`,
  drops a feature `anim`'s ragdoll needs for a six-line fix, and a git fork would move source
  out of the repository and past `deny.toml`'s `unknown-git = "deny"`.
  - *One mechanism, every crate.* `[patch.crates-io]` applies to every user of the crate in
    the workspace, inside the boundary or not, so the mechanism and its check are
    workspace-wide; admission differs: inside the boundary a patch fixes a section 8.2 route
    and is reviewed as a determinism review, like a new allowlist name; elsewhere it fixes a
    defect that blocks a milestone criterion; never a feature. A crate is patched at one
    version only, and no `[patch]`, `[source]` or `paths` setting lives anywhere but the root
    manifest, since cargo honours one in `.cargo/config.toml` silently.
  - *Fifty changed lines, comments included; never the manifest records; a build script if
    it must; no `unsafe`; a notice on every change.* The cap the lane brief used; it
    separates a fix from a fork. The sign of zero in SIMD `min` and `max` (about 26 sites
    over rapier, parry, simba and wide) is what it excludes, and that stays a verification
    item for SOUR's architecture mode, not a patch. `Cargo.toml`, `Cargo.toml.orig`,
    `Cargo.lock` and `.cargo_vcs_info.json` stay upstream's so the version, features,
    dependencies and closure the allowlist review saw do not move; a build script may be
    patched because `libm` selects its paths in `configure.rs` and cannot be refused, and
    such a patch is reviewed as a change to what every target compiles. The `LACTOSE patch:`
    comment is the notice Apache-2.0 section 4(b) asks of a modified file, and counting it
    keeps the cap the diff's line count.
  - *The record is what `cargo vendor` writes plus a patch file, and the check re-derives
    it.* `.cargo-checksum.json` carries every upstream file's sha256 and the archive's, in
    the format cargo itself reads; the patch is a unified diff `git apply` consumes. Offline
    the check reverses the patch on a copy, hashes and compares; that proves the copy against
    its record only, since a rewritten hash in the record passes it, so online the check
    re-derives the record from the crates.io archive with `cargo vendor` from a shadow
    manifest and requires equality. Rules out a diff quoted in prose (the lane's README), a
    checksum in prose, and a tool that applies patches at build time (cargo has no hook; the
    tree must exist before `cargo metadata`).
  - *`cargo-deny` sees the name but not the advisories.* Probed on cargo-deny 0.20.2: `bans`,
    `licenses` and `sources` match a patched crate by name and version; `advisories` skips it
    as a path source (`smallvec` 1.6.0, RUSTSEC-2021-0003: reported from the registry, silent
    when vendored and patched). The check runs `cargo deny check advisories` over the same
    shadow manifest, pinned as the root `[workspace.dependencies]` entry pins it and resolved
    against a copy of the workspace lock, and its exit status decides.
  - *The build ID hashes the vendored files.* A patched crate's `Cargo.lock` entry has
    neither `source` nor `checksum`, so section 9.1 hashes the vendored directory as it
    hashes the boundary crates' sources: that names what the binary compiled whether or not
    the check ran, where an archive checksum plus a patch would stay equal for a tampered
    record until the online step caught it. `.gitattributes` marks `vendor/**` `-text` so a
    Windows checkout hashes the same bytes.
  - *A dependency, not a member.* Cargo does not make a patch path a member, but
    `exclude = ["vendor"]` states it, so no workspace lint, format or test runs over the copy
    and `[profile.dev.package."*"]` applies as to the registry build. Cargo caps no lints on a
    path source, so the copy's own warnings show in a clippy run; `-D warnings` reaches the
    selected packages only, so they fail nothing.
  - *Reported upstream by the owner, dropped at the first bump that carries it.* A patch
    nobody reports is a fork by instalments. Rapier's master still called the inherent
    `sin_cos` on 2026-10-11.
  - Rules out: a git fork; a patch over the cap; a patch to a manifest record; a vendored
    copy with no patch; prose as the record; a per-boundary mechanism; a patch declared
    anywhere but the root manifest.
- What the proposal's review changed (0 blocking, 3 serious, 5 minor, 2 process): the shadow
  manifest's `[workspace]` table, pinning and command, and cargo-deny's exit status as the
  verdict; the online re-derivation of the record; cargo configuration files and non-member
  path packages outside `vendor/` as findings; the one-version rule and `package` keys; the
  determinism review in the binding text; the file list in place of "under `src/`"; the
  notice; the row 83 replacement; the check as its own lane before Rapier; the owner files
  the upstream report.
- Driven by: the `physics-rapier` lane (2026-10-10), its report's proposed rule and the review
  brief's vendored-patch item; the planner's and the reviewer's probes (2026-10-11).
- Addendum (lane physics-rapier S5, 2026-10-18): the rapier3d-f64 patch carries a second
  change, admitted as a defect blocking the milestone 1 physics criteria on pools above one
  thread: the broad phase's deferred optimisation runs on the stepping thread (two files,
  sixteen changed lines in all; D124's S5 addendum).
- Replaced: nothing stated; the lane brief's "prefer the patch if it is under fifty lines,
  else refuse".
- What the check's two reviews changed (round 1: 1 blocking, 4 serious, 4 minor; round 2:
  2 serious, 4 minor). Cargo reads nothing the check does not: an `include` key in a cargo
  configuration file, a cargo configuration file or `.cargo` directory committed as a
  symbolic link, one git tracks under a directory the file walk skips, a config alias that
  passes `--config` or `-Z`, any `--config` in a workflow or script, and a line that sets
  `CARGO_HOME` are findings (`cargo xtask boundary` refuses the `include` key and the link
  too). git is the authority on what a patch touches (`git apply --numstat`); a hunk git
  places at an offset or with fuzz is refused in both directions, and the record with the
  patch applied forward must equal the copy byte for byte. The graph is read with every
  feature on. Nothing lives directly under `vendor/` but the copies, their patches and
  `README.md`. An added line holds at most 200 characters and none of `unsafe`,
  `no_mangle`, `export_name`, `link_section`, `link_name`, `naked`, `asm`, `global_asm`,
  `naked_asm`, `include`, `include_str` or `include_bytes` as a whole identifier, with or
  without `!`; a hunk's added lines hold no `path` attribute and no out-of-line `mod name;`.
  A patch never touches a file that holds `unsafe`, before or after it, so no added line
  sits inside an upstream `unsafe` block. A patch may change a build script but never
  create one (`build.rs`, or what the manifest's `package.build` names). A token scan cannot
  see unsafe code reached through macros the crate already has, or through a build
  script's output; review is what covers those.

**D124. The physics solver holds state the rows do not carry: reconciliation compares a
dynamic body's solver-written rows within a tolerance that gates the rollback, `Teleported`
of a claimed entity is never compared, an empty solver is a fresh solver, and a journal
checkpoint carries the physics snapshot** (§1, §2, §9.4, §10, §15.5, §16, §22,
`lactose-predict`, `lactose-physics-api`, `lactose-physics-rapier`, `lactose-sim`)
- Why: the client's prediction world is not bit-equal to the server for a dynamic body
  however good the inputs, and D122's "no tolerance" would correct what is not a
  misprediction and then correct its own corrections. The source is not, in the scenes
  prediction meets, the far-body and arena history of D119 (a capsule on static geometry,
  one pushing a crate, one bumping a stack, one pressing crates against a wall and fifty
  thrown interacting bodies were bit-equal with far bodies awake throughout; the 150-body
  pile and a solver that has freed its slots were not); it is the solver state the rows do
  not carry, warm starts, manifolds and sleep, which the client's solver reaches by another
  path after a correction, a claim in contact and a rebuild. The planner's and the
  reviewers' probes (`docs/verification.md`) bound it: after a correction a body on static
  geometry or pushing one crate differs by up to 7.5e-4 m for as long as it is in contact,
  and bodies in contact with each other by whole outcomes; a fresh solver against the
  continued one by 1.5 mm to 1.6 m; at 20 Hz a bit-exact rule costs 8 to 200 rollbacks per
  misprediction and the tolerance one, or 7 to 16 after a misprediction while a stack of
  claimed crates moves and 22 to 35 after a claim mid-bump.
  - *The backend keeps its history dependence.* Canonical results per island need Rapier
    to solve islands separately in a canonical order, which the pinned version cannot and
    no fifty-line patch adds (D123), and no solve order carries solver state across a
    correction, a claim or a rebuild; a rebuild every step discards exactly that state at
    1.3 ms a step plus a cold-contact step (D119). Rules out canonicalisation in
    `physics-rapier`, a second engine and a physics world per island.
  - *A tolerance on the solver-written rows of a dynamic body gates the rollback; the
    rollback is D122's.* `Transform` and `Velocity` of a claimed entity whose `RigidBody`
    is `Dynamic` at the compared slot or the one before (the compared slot alone when the
    record has no earlier one) agree by canonical bytes or, failing that, when each scalar
    leaf is within `PredictionTolerance` (2e-3 m, 2e-3 per quaternion component, 2e-2 m/s
    and 2e-2 rad/s by default; `app` sets them through `Predictor::set_tolerance`). The
    measured margins over the largest absorbed residual are 80 times for a lone body, 2.7
    for a body pushing one crate, 1.7 and 1.8 for a rolled ball, 1.05 for a crate claimed
    mid-push and none for a moving stack, so the defaults bound what is corrected rather
    than guarantee an absorbed residual; a residual at the margin costs one correction, not
    a storm, while the next frames fall below it, which every single-contact case did; the
    scalar decision re-measures the defaults and the spike doubles one whose single-contact
    absorbed maximum exceeds half of it, a condition the probes already meet for translation
    and rotation, so those two are expected to go to 4e-3 (a pixel at five metres on a 1440p
    display at 60 degrees of view, and 4 mm at the rim of a body of 0.5 m radius); the owner
    kept 2e-3 so that the spike measures the shipped path first. Every other field,
    kinematic and fixed bodies' rows included, agrees by canonical bytes only, and a NaN
    leaf agrees with an equal NaN and nothing else. A frame makes a correction point when
    any fact does not agree, and the rollback then writes every fact at *k*, the absorbed
    ones included, as D122 says: writing them is what converges (a moving stack 7 to 16
    corrections against 18 to 34 when only mismatching components were written, a derived
    integer 1 against 72 to 83). Rules out a tolerance on every field, a tolerance on
    discrete fields, a rollback that leaves absorbed facts as predicted, an authority-only
    rule that applies physics facts without comparing them, and a quantised comparison grid.
  - *`Teleported` of a claimed entity is never recorded, compared or corrected.* It is the
    client's own no-interpolate flag: `physics-rapier` reports a body whose transform was
    written as `teleported` and the write-back sets the tick, so a correction's placement
    set it on the client alone and every corrected `Transform` was a second correction one
    frame later; no boundary system reads it. A defect of D122 found by reading. A proxy's
    `Teleported` is still written by the hook as its position for the tick.
  - *A replicated field derived from a solver-written value compares by bytes and is
    corrected; a game keeps such state integer.* The correction converges because the
    rollback writes every fact at *k*; a derived replicated scalar would mismatch on every
    frame while a contact lasts. The build check cannot see it, so section 9.4 states it.
  - *The spike records what the tolerance absorbed and claims a body in contact.*
    `PredictionStats::absorbed` holds the largest absorbed difference per kind as canonical
    bits and the count of absorbed fields, so milestone 4 re-sets the defaults from
    measurement; the spike runs the character as a dynamic capsule and as a kinematic body
    (a kinematic body and its queries were bit-exact under every history), its fifty
    claimed bodies thrown and interacting, and a claim of a crate being pushed and of a
    stack being bumped, the case that storms.
  - *Claimed dynamic bodies in contact with each other are corrected every few frames
    until they settle, under any rule.* A three-crate stack was corrected 7 to 16 times in
    200 frames after a misprediction while it moved, 22 to 35 times in 167 frames when
    claimed mid-bump, and once when corrected or claimed at rest; the divergence is
    physical, so no tolerance holds it and no canonical solve would either. The lever is
    the claim: `net`'s policy at milestone 4 claims a thrown object with its throw, releases
    it once it rests or touches an unclaimed body, and never claims a body in contact with
    another dynamic one.
  - *An empty solver is a fresh solver.* `sim`'s in-place reload and a client's full-frame
    rebuild unload and load every region in one frame, which the sync sees as `Despawned`
    then `Spawned` per key and leaves untouched, so the solver kept its whole state, the
    reloaded world equalled the untouched twin and not a fresh process (a stack mid-bump by
    3.9e-3 to 1.1e-2 m, a 75-body pile with joints across two regions by 1.24 to 1.40 m),
    and section 15.5's first test did not hold with physics (D57 intended both sides equal);
    the handle rows were lost besides. `physics-api` keeps the bytes the backend wrote at
    `setup`; a sync whose structural log since its last pass, the pending entries a restore
    carried included, despawned every body the solver held removes them all, restores those
    bytes and creates from the rows by a full diff in ascending key order; a sync whose
    removals leave the solver empty restores the bytes and makes the next sync a full diff;
    a `Spawned` entry for a key the bookkeeping holds re-inserts its handle row. Measured
    through the sync bit-equal to the fresh process on the one-frame and the two-frame path,
    the stack and the pile with its joints. D119's "never clears the solver" narrows to a
    solver whose log kept a body. Rules out amending section 15.5's first test to a
    tolerance, and a trigger on the solver being empty after a sync alone, which the
    one-frame path never reaches.
  - *A journal checkpoint carries the physics snapshot.* A checkpoint that drops solver
    state replays equal only until the first contact, since the replay's solver is fresh
    (1.5 mm to 1.6 m against the continued one); the milestone 4 replay criterion is met
    from a checkpoint that restores the snapshot after the regions and before the first
    step, or from a session's start. `PhysicsSnapshot` carries an `Arc<()>` identity
    compared by pointer, for the ring, and `sim` cannot name `physics-api`, so
    `journal::Journal` gains `services`, keyed by the xxh64 of a service's declared name
    (never a `TypeId`) to opaque bytes `app` fills and hands back, and `physics-api` gains
    the byte form, carrying a format version, the backend's identity (name, Rapier version,
    the D123 patch set's hash), the `PhysicsConfig`, the tick, the backend's bytes and the
    bookkeeping, and its restore into a fresh `Physics` at the checkpoint's tick, which
    refuses a mismatch with `InvalidData` and rebuilds the handle rows: the journal tooling
    lane's, sized at the milestone 1 scale first. `sim` never reads the bytes and the
    journal is not hashed state.
  - *SOUR needs nothing.* Its modes vary threads, layout and schedule under one row
    history, which is where bit equality holds and what `tests/determinism.rs` shows; the
    sign-of-zero item and the snapshot-hash item stand as they are.
  - *Risk, not this decision's:* on a four-thread pool a small scene stepped in 39 ms at worst,
    most 1,800-tick runs took 20 to 100 ms and 41 of 300 took 0.4 to 4.4 s, against 19 to 30 ms
    on one thread (whose own worst run was 319 ms and worst step 14.5 ms), and it once hung in
    Rapier's deferred broad-phase optimisation; the D123 patch (second hunk) runs that pass on
    the stepping thread, and `crates/physics-rapier/tests/pool.rs` steps a small scene, under
    the narrow phase's 64-pair broadcast threshold, with every other pool thread held.
  - Named and deferred: the defaults re-set at milestone 1 (scalar) and 4 (spike); `net`'s
    claim policy; the snapshot's byte form, `services` and their size; the full diff's cost
    on a server-sized reload.
- Driven by: the D119 History addendum, the planner's probes and the proposal's three design
  reviews, whose probes supplied the claim-start, in-contact, derived-field, fresh-load,
  far-awake and through-the-sync reload measurements and the candidate trigger; accepted by
  the owner with the defaults kept at 2e-3/2e-3/2e-2/2e-2 and the spike deciding any doubling
  (`docs/proposals/physics-history-reconciliation.md`, 2026-10-11).
- Addendum (lane physics-rapier S5, 2026-10-18): S5 closed. The four-thread hang was Rapier's
  deferred broad-phase optimisation. The stepping thread, a pool worker, queued it with
  `rayon::spawn` and joined it later in the step with a blocking `mpsc::recv`, a wait outside
  rayon. rayon tries to wake a thread for a job a pool thread pushes to its own deque but does
  not guarantee it; it relies on the pushing thread to pop the job itself, which a thread
  blocked in `recv` never does. The step hung with every other thread asleep and the job
  stealable: once in the D124 review; in the S5 review's five-scene probe, in 4 of 9 unpatched
  runs and in 4 of 12 unpatched processes of its walk scene on six two-thread pools in a row
  (load 1.4-16; at load 33-41 the same probe did not hang in 9); and every time with every
  other pool thread held (`crates/physics-rapier/tests/pool.rs`, which fails in 20 s
  unpatched). A second D123 hunk runs the pass on the stepping thread. It is in
  `BroadPhaseBvh::take_deferred_optimize`, because `solve.rs` holds `unsafe`. The patched build
  hung in none of ten runs of the five-scene probe. Reports and snapshot bytes for thirteen
  scenes on one, two and four threads equal the unpatched one-thread build's. The step never
  waits on work that needs a pool thread to be woken outside rayon's guarantee. Its waits are
  rayon's own, which run other work meanwhile and wake every thread they wait on, or the staged
  solver's spin on a batch another thread is already running. It still uses every thread of the
  pool, because the narrow phase broadcasts above 64 updating contact pairs. So the solver pool
  must be the backend's alone: no job on it may block outside rayon, and it must not be built
  with `use_current_thread`. On that condition the prediction world's pool need not stay at one
  thread, and the spike's step watchdog is not needed for this hang. The D124 review's
  four-thread tails (41 of 300 runs at 0.4-4.4 s) did not recur as a four-thread effect:
  patched, at load 4-10, runs of 400 ms or more were 2 of 600 on four threads and 6 of 600 on
  one, and the D124 review's own binary gave 0 of 600 runs from 100 ms at load 5. They are not
  attributed.
- Addendum (lane physics-api D124, 2026-10-18): implemented in physics-api (b5634dd, fix round
  b07339b). `setup` keeps the backend's snapshot of its empty world and fails as that snapshot
  fails. A sync whose log since its last pass despawns every body the solver held removes them
  all in ascending key order, restores those bytes and runs the full diff; that log includes
  the pending entries a restore carried, and on a stale cursor it is the log retained in the
  epoch since the pass. A sync whose removals leave the solver empty restores the bytes and
  sets `diff_all`. A `Spawned` entry, and the full diff, write back the handle row of a key the
  bookkeeping holds. When the sync's cursor is stale (`set_tick`, a world restore without a
  physics restore, a whole-world load, the 2^30 rebase), its reload test reads the log the
  world retains in the epoch that began after its last pass, and a physics snapshot taken then
  carries that log. So a client's re-join (`set_tick`, then the full frame) and a rollback to a
  slot taken after `set_tick` get a fresh solver, and the full diff writes back the
  `BodyHandle` and `JointHandle` of every body and joint it keeps. An unload made before
  `set_tick` lies in the earlier epoch and is not seen: the supported order is `set_tick`, then
  the full frame. A log trimmed past the cursor is read as far as it is retained. A world
  restore or a whole-world load logs no despawns, so neither triggers the reload by itself; the
  whole-world load must empty the solver itself when that API lands. Waiting joints are kept
  and resolved by the full diff. `conformance::empty_restore` checks the backend obligation
  that the empty world's bytes restore to a world that steps as a new one does. Open: commands
  untaken at the save survive a one-frame reload (S1).
- Replaced: section 2's and 9.4's "at full precision" without qualification; D122's "Full
  precision means canonical bytes per field, no tolerance" for the solver-written rows of a
  dynamic body, its "a tolerance for dynamic bodies is decided at milestone 4" and its
  "this crate computes no float"; D119's "never a cleared solver" for a solver whose log
  despawned every body it held or whose removals emptied it (the two clauses of this
  decision's reload rule); `crates/sim/README.md`'s "dropped at a checkpoint, as solver
  state is".

**D125. Linkers in CI and dev builds, and the Windows jobs: lld-link on Windows, clippy and the
release golden vectors beside the Windows test build** (§18, §20, `.cargo/config.toml`,
`.github/workflows/ci.yml`)
- Why: check (windows-latest) took 889 s of its 900 on main f70c25f (run 38128490654) and grew
  508, 466, 552, 606, 680, 754 s, a timeout, then 889 s across successive merges, about a
  minute per crate merged; section 20's budget is fifteen minutes per run.
- *Windows links with lld-link.* `.cargo/config.toml` sets `linker = "rust-lld"` for
  x86_64-pc-windows-msvc. rust-lld ships in the sysroot of the pinned x86_64-pc-windows-msvc
  1.99.0 toolchain (`lib/rustlib/x86_64-pc-windows-msvc/bin`); rustc puts that directory on the
  linker's PATH and drives it as lld-link (`-flavor link`) with the MSVC library paths, so the
  runners need no install step. Only the linker is set; the x86-64-v3 pin stays the only rust
  flags, and RUSTFLAGS is never set.
- *Linux and macOS.* x86_64-unknown-linux-gnu needs no entry: rustc links it with its bundled
  lld by default since 1.90. mold was not taken: lld already covers what section 18 asks, and
  mold would need an install on every runner and developer machine. macOS keeps Apple's linker.
- *Scope.* Section 18 asks for a fast linker in dev builds, but cargo has no per-profile linker
  key: the setting covers every profile, build scripts, proc-macros and doctests on the target.
  Accepted: lld-link is a production linker, and the release golden vectors link as the debug
  tests do.
- *Why it is not the whole fix.* The Windows test build runs 1.4 to 2.3 times Linux's, about as
  much in the early phase, which links almost nothing (190 s of the 9m56s build in run
  38128490654), as in the late phase, which links the test binaries (407 s); the linker's share
  is estimated small (linking was 4.3% of the test build's core-seconds on an M1 with
  ld-prime). The rest is rustc on the Windows runner.
- *Clippy and the release golden vectors move beside the Windows test build.* They run in
  windows-lint-release, "clippy and release tests (windows-latest)", and check (windows-latest)
  skips both. With both moved the check job is estimated at about 12.6 minutes, a margin of
  about 2.4 warm. The cost is a second Windows runner per run: its setup plus a cache restore
  and save, about 1 to 1.5 Windows minutes on top of the 2.3 the moved steps took. rust-cache
  keys caches by job ID, so the two Windows jobs never restore or overwrite each other's cache,
  and no job may set `shared-key`.
- *Cold runs and the cache.* The linker is part of every unit's fingerprint, so the first run
  after the change rebuilds every dependency, an estimated 2 to 3 minutes more, which may hit
  the timeout; both Windows jobs set `cache-on-failure: true`, so a run that times out late in
  its build saves what it built. rust-cache saves only when its key missed, so a run on a new
  key that fails or is cancelled before its build finishes leaves a partial entry that later
  runs restore and never replace, until Cargo.lock, a manifest, `.cargo/config.toml` or the
  toolchain changes; recovery is `gh cache list --key v0-rust-check-Windows` then `gh cache
  delete <key>`. A push that changes the cache key is pushed alone, and nothing follows it
  until its Windows check run completes.
- *Next lever, due now.* The margin covers one or two more crate merges. One integration-test
  binary per crate is the next CI lane: lactose-ecs's 22 test binaries built as one took 5.2 s
  instead of 14 (M1, -j4, CARGO_INCREMENTAL=0).
- Driven by: the Windows CI timeout of run 38126882535 and the CI link lane with two review
  rounds (2026-10-11).

## 10. Review history

- **Revision 1** (`a29af04`): the design baseline.
- **Review 1** (`docs/reviews/review-1-revision-1.md`): 24 findings, 6 blocking, 15 serious,
  3 minor.
- **Revision 2** (`f65d2b6`) addressed the first review: unsound scoped async spawning, a
  contradictory dependency rule, chunk-layout-dependent determinism, snapshots missing physics
  state, region bits in entity IDs, padding in hashed bytes.
- **Review 2** (`docs/reviews/review-2-revision-2.md`): 30 findings, 6 blocking, 22 serious,
  2 minor, after checking Rapier, MoltenVK and Rust float claims against upstream sources.
- **Revision 3** (`4c7b609`) addressed the second review: process-local IDs used as
  cross-machine keys, help-while-waiting lengthening the critical path, a single dirty bit
  shared by many consumers, camera-relative positions rewriting the whole GPU scene, MoltenVK's
  CPU-side multi-draw loop, and a predicted area with two writers per row; plus the float rules,
  the save overlay, the transient policy, presentation access, the orbit camera, extract
  mechanics, tier contradictions, network staging, time and pacing, worker classes, component
  trait bounds, the sim skeleton, error handling, the shader toolchain, tile memory, build
  settings and the protocol hash.
- **Editorial split of revision 3** (revision 3.1, `d58492a`): the architecture document was
  split into `docs/architecture.md`, this log and `docs/verification.md`, with no decision changed.
  `docs/reviews/traceability.md` maps every finding of both reviews to where it is addressed.
- **Review 3** (`docs/reviews/review-3-revision-3.1.md`): 30 findings against revision 3.1, 4
  blocking, 19 serious, 7 minor.
- **Revision 3.2** (`0ac9fd0`) addressed the third review. The six most significant changes: a
  `StableKey` enum with server-recorded origin keys, so predicted spawns can be referenced and
  rendered (R3-1, D5); a worked system API that the week-one facade implements (R3-2, D73);
  per-client, per-entity acknowledged ticks behind a replication gather phase (R3-3, D76); region
  overlays sent to clients on relevance, cooked moves as `Destroy` plus `SpawnFromBaseline`, and
  `NetId` doubling as the persistent ID (R3-4, R3-12, D7, D81); a server capacity model with
  archetype-owned chunks and an overload ladder that never slows time (R3-13, R3-20, D78); and a
  compacted index buffer with one indexed indirect draw per pipeline, reversing R2-5's vertex
  pulling (R3-21, D42). The rationale first recorded in revision 3.1 and the revision 3.2 decision
  table were later folded into the entries above (D72 to D86 added), and
  `docs/reviews/traceability.md` was regenerated over all three reviews.
