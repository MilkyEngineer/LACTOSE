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
