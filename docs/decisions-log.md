# LACTOSE Decisions Log

`docs/architecture.md` states what is decided. This log records why: for each decision with a
non-obvious reason, or one that reversed an earlier default, the rationale, the review finding
that drove it, and what it replaced. Where no reason is recorded in the architecture document,
its earlier revisions or the reviews, the entry says so; section 11 collects those gaps.

Notation: R1-n is finding n of `docs/reviews/review-1-revision-1.md`; R2-n is finding n of
`docs/reviews/review-2-revision-2.md`. Revision 1 is commit `a29af04`, revision 2 `f65d2b6`,
revision 3 `4c7b609`. Section references (§) are to the current `docs/architecture.md` unless a
revision is named. `docs/reviews/traceability.md` maps every finding to where it is addressed.

## 1. Goals and scope

**D1. Determinism is an engineering property, not a prediction guarantee** (§1 goal 2, §2, §9.4)
- Why: bit-exactness pays only when a predicted entity's inputs are identical on both machines,
  and they are not: the client world holds kinematic proxies with different contents and
  insertion history, corrections arrive quantised, and the simulation reads unreplicated state
  (transient columns, contacts, warm-start, timers). What it does buy is zero corrections for
  solo traversal through static geometry when keys match and comparison happens in quantised
  space, plus SOUR, replays and desync debugging. Hence reconciliation in quantised space and a
  milestone 4 criterion of a replay hash plus a correction rate.
- Driven by: R2-7, which disagreed with the framing R1-8 introduced.
- Replaced: revision 2's "cross-architecture determinism is a product requirement" and its
  milestone 4 criterion "x86 server, ARM client, hashes equal". Revision 1 had
  cross-architecture determinism as a non-goal.

**D2. No fixed player cap** (§1 goal 6, §2, §9.1)
- Why: not recorded. The bound is stated (server cost by active entities through `Frozen`
  regions, per-client cost by relevance); the reason for leaving four-player co-op is not.
- Driven by: no finding.
- Replaced: four-player co-op (revisions 1 and 2); milestone 4's four clients became
  thirty-two.

**D3. Intel Macs are a non-goal** (§1)
- Why: their argument buffer tier cannot hold the bindless heap (revision 2 §10.1).
- Driven by: R1-12 ("Intel Mac argument buffer limits").
- Replaced: revision 1 supported macOS through MoltenVK without an architecture limit.

**D4. Tracing and reflection from day one** (§3)
- Why: Goal 1 cannot be measured without tracing (revision 2 §3). A type that cannot be
  reflected cannot be serialised, replicated, snapshotted or shown in a future editor
  (revision 2 §3); serialisation and replication need the registry (revision 1 §2).
- Driven by: no finding.

## 2. Identity and ECS

**D5. Three identities plus prediction keys** (§7, §2 Identity)
- Why: process-local IDs from a block table differ between server and client, because the
  table's state depends on the order regions requested blocks; every ID-keyed rule (RNG,
  command order, physics insertion, sorted reductions) then diverges even with bit-exact
  floats. Per-player reserved blocks forced remapping of predicted IDs already held in
  components, history and sent inputs, and could run out mid-fight. Cooked entities derive
  `NetId` from placement so clients know it without a message.
- Driven by: R2-1.
- Replaced: revision 2's 64-bit IDs allocated in 4096-index blocks from a persistent block
  table, with per-player reserved blocks the server confirms or remaps.

**D6. `EntityId` carries no region bits and is `NonZeroU64`** (§7)
- Why: an 8-bit region in the ID meant an entity could not cross a region boundary without a
  new ID invalidating every reference, and capped regions at 256 (R1-5). `NonZero` makes
  `Option` free (R2-24).
- Driven by: R1-5, R2-24.
- Replaced: revision 1's 32-bit index, 24-bit generation, 8-bit region.

**D7. `PersistentId` from (scene, placement, child path); runtime IDs at command apply** (§7)
- Why: an ID taken from the source asset's GUID collides when a prefab is placed twice or has
  children; a global counter mutated by spawning systems serialises them unless it is assigned
  at apply.
- Driven by: R2-12. R1-15 had recommended the GUID.
- Replaced: revision 2's GUID-derived cooked IDs and a server counter in global resources.

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
  sets (R2-24).
- Driven by: R1-6, R2-24.
- Replaced: revision 1's plain `#[repr(C)]` structs; revision 2's `NoUninit + AnyBitPattern`.

**D10. Per-row change ticks with a last-seen tick per consumer** (§6, §2 Change tracking)
- Why: per-column, per-chunk ticks amplified sparse writes about 100 times in replication
  (R1-10). A per-row dirty bit carries no time, so extract (every tick) and clients with
  different acknowledged ticks cannot share it: whoever clears it first hides the change from
  everyone else (R2-3). A tick per row lets each consumer read rows newer than its own
  last-seen tick, at 4 bytes per row per tracked column, and packet loss needs no per-packet
  bookkeeping. `set_if_neq` keeps equal rewrites out of the stream; slices take an explicit
  dirty range because whole-column slices otherwise bypass tracking (R2-19). Saves do not need
  change tracking because they diff against the cooked baseline (R2-3).
- Driven by: R1-10, R2-3, R2-19.
- Replaced: revision 1's per-column, per-chunk ticks; revision 2's per-row dirty bitmask read by
  replication, extract and persistence.

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
  preempts. Both lengthen the critical path. Rayon has the same property.
- Driven by: R2-2.
- Replaced: revision 2's opener that "steals other work until the join counter reaches zero",
  with steal priority own frame-critical, others' frame-critical, then background.

**D16. Unconditional abort and a closure-only scope** (§5.1)
- Why: Cargo ignores `panic = "abort"` for tests and benches, Miri and loom unwind, and a panic
  in a `std::thread` worker unwinds only that thread. Without a guard a panicking child leaves
  the counter above zero and the opener hangs, and an opener that unwinds frees a frame others
  still borrow. A guard-object scope is unsound under `mem::forget`. Blocking waits on a worker
  hang in SOUR's one-worker mode instead of failing.
- Driven by: R2-9.
- Replaced: revision 2's "a panic in any task aborts the process after writing a crash dump",
  which relied on the panic profile.

**D17. Worker classes on performance and efficiency cores; split-on-steal** (§5.1)
- Why: physical cores minus one puts frame-critical tasks on efficiency cores at two to three
  times the latency on Apple Silicon and Intel hybrid CPUs, and macOS offers only QoS, not
  pinning. A fixed 50 to 200 microsecond batch target conflicts with Goal 1: a 300 µs system
  split in two adds more to the critical path than fifteen 20 µs tasks on idle workers. The
  0.5 ms background yield keeps background work from stalling frame-critical joins (R2-2).
- Driven by: R2-23, R2-2.
- Replaced: revision 2's one worker per physical core minus one, two deques per worker, and
  50 to 200 µs batches.

**D18. Timer root, present thread, futures for main-thread requests, time scale** (§5.1, §11)
- Why: with no coordinator and a main thread that only pumps events, something must wake at
  tick deadlines, cap catch-up and pace frames. A worker waiting synchronously on a main-thread
  job while the main thread blocks in present (FIFO present, or `nextDrawable` for up to a
  second) on a frame that needs that worker deadlocks. Slow motion affects every player under
  a global time scale, and pause in multiplayer needed a defined meaning. The catch-up cap of
  four ticks per frame has no recorded rationale.
- Driven by: R2-22.
- Replaced: revision 2's "fixed 60 Hz on its own cadence" and a main thread that pumps events
  and presents frames; revision 2 had also dropped revision 1's main-thread lane.

**D19. Sync point count fixed per schedule; system sets; locks inside RHI backends** (§3, §5.2)
- Why: revision 1's "exactly four sync points", ordering without system sets, and a `Mutex`
  rule contradicted by Vulkan queue submission were inconsistencies.
- Driven by: R1-22.
- Replaced: revision 1's four fixed sync points, pairwise `before`/`after` constraints and no
  lock exception.

**D20. Stable system keys and a total command order** (§5.2, §7)
- Why: (system order, entity ID) is not a total order: one system can issue several commands
  for one entity and several sources can target one entity, so ties fell to task order.
  Registration order became semantic, so a client with extra systems or a `cfg`-gated plugin
  changed results; the same held for system IDs in RNG keys. Spawns had no ID yet.
- Driven by: R2-14.
- Replaced: revision 2's topological sort keyed on registration order and command order
  (system order, entity ID).

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

## 4. Determinism

**D22. The boundary is a crate boundary** (§3, §4, §8.1)
- Why: the boundary was drawn per system but clippy configuration is per crate; `net` held both
  RTT measurement (needs `Instant`) and rollback (bans it); `nav`'s deterministic half was
  placed in two different crates; terrain collision came from a backend crate outside the
  boundary.
- Driven by: R2-20.
- Replaced: revision 2's boundary of "`net` prediction and rollback, the deterministic half of
  `nav`, and every `game` simulation system".

**D23. Chunk, row and worker indices unreachable; reductions through combinators** (§5.2, §8.2)
- Why: results depended on chunk layout (reductions in chunk order, per-chunk command buffers,
  scratch keyed by chunk index), and a client builds regions in a different order from the
  server (R1-3). Tests cannot prove "never a semantic key", so it is enforced by construction;
  scratch "keyed by entity range" was meaningless because a chunk's rows do not form an ID
  range (R2-15).
- Driven by: R1-3, R2-15.
- Replaced: revision 1's reductions in chunk index order and scratch keyed by chunk index;
  revision 2's entity-ID-sorted reductions and scratch keyed by entity range.

**D24. Counter-based RNG keyed by (region seed, tick, system key, `NetId`)** (§8.2)
- Why: per-entity keys remove dependence on chunk layout (R1-3); keys must be the same on
  every machine (R2-1); a counter-based generator has no state, so a snapshotted "RNG state"
  was inconsistent (R2-14).
- Driven by: R1-3, R2-1, R2-14.
- Replaced: revision 1's region-local stream seeded from region seed and tick; revision 2's key
  of (region seed, tick, system ID, entity ID).

**D25. Float rules for `min`, `max`, NaN, `powi` and the FP control register** (§8.2, §5.1)
- Why: `f32::min` and `max` may return either input for ±0, and x86 and AArch64 differ; NaN sign
  and payload are nondeterministic and leak through `total_cmp`, `to_bits`, `copysign` and
  `is_sign_negative`; `powi` is documented as precision-nondeterministic. MXCSR and FPCR are
  per-thread state that audio, codec and driver code sharing the workers may change.
- Driven by: R2-11.
- Replaced: revision 2's rules, which allowed SIMD `min` and `max` and did not cover signed zero,
  NaN sign, `powi` or the control register.

**D26. Fused multiply-add permitted; the x86-64-v3 pin enforced by `compile_error!`** (§8.2)
- Why: rustc never contracts on stable, so "FMA contraction is disabled by compiler flags"
  named flags that do not exist (R1-8). Fused multiply-add is correctly rounded and identical on
  both ISAs; the hazard is `wide` and glam choosing fused or unfused by target feature, which
  makes the pin load-bearing. The pin is silently dropped when `RUSTFLAGS` or
  `CARGO_ENCODED_RUSTFLAGS` is set or cargo runs outside the workspace, hence the
  `compile_error!` (R2-11).
- Driven by: R1-8, R2-11.
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

**D29. `world_hash` in `NetId` order through reflection** (§8.3)
- Why: hashing raw `repr(C)` bytes included padding (R1-6); hashing columns in storage order
  makes correct code fail layout mode (R2-15).
- Driven by: R1-6, R2-15.
- Replaced: revision 1's hash over all chunk data; revision 2's field-by-field hash in storage
  order.

**D30. Identity-preserving layout perturbations, a chaos mode, a Haswell-masked run** (§8.3)
- Why: revision 2's shuffled insertion and region-load order changed the IDs entities received,
  so ID-ordered behaviour changed legitimately and produced false failures. One versus N
  workers on 4-vCPU CI runners finds far less than a seeded chaos schedule (R2-15). The
  Haswell-masked run is part of the architecture matrix R2-11 asked for.
- Driven by: R1-3, R2-15, R2-11.
- Replaced: revision 2's layout mode (shuffled insertion, forced compaction, different region
  load order) and a two-runner architecture mode.

**D31. Stable clippy and `cargo-deny` instead of a custom lint crate** (§8.3)
- Why: a custom lint crate would require nightly, against the stable toolchain decision
  (R1-9; revision 2 §7.4). Clippy never lints dependencies, so `cargo-deny` allowlists them
  (R2-11).
- Driven by: R1-9.
- Replaced: revision 1's `determinism` lint crate.

## 5. Networking and snapshots

**D32. A separate client prediction world** (§9.4, §2 Netcode)
- Why: region modes apply per region, so a `Simulated` client region ran every system on every
  entity while replication also wrote those rows: two writers per row. Snapshotting whole
  regions plus a Rapier clone every tick cost plausibly 5 to 20 MB per tick across a ring of up
  to 40. The server had nothing to resync a client area from.
- Driven by: R2-6; R1-18 for predicted entities colliding only with static geometry and each
  other.
- Replaced: revision 2's predicted active area of client-simulated regions; revision 1's
  prediction region.

**D33. Ring depth derived from the re-simulation budget; rebuild from replicated state** (§9.4)
- Why: a 12-tick ring is 200 ms, short of real round trips (R1-18). A correction k ticks deep
  costs k times (systems, physics step, snapshot) in one frame, so the cap must come from the
  budget and the measured per-tick cost; nothing derived revision 2's 40-tick cap (R2-6).
- Driven by: R1-18, R2-6.
- Replaced: revision 1's 12-entry ring; revision 2's RTT-sized ring capped at 40 ticks with
  "requests a full area snapshot".

**D34. Copy-on-write snapshots in `lactose-predict`** (§23, COW)
- Why: not recorded. R2-6 recommended copy-on-write from day one for the prediction world's
  mutable columns; the glossary calls COW "a measured optimisation" and the body does not say
  when it is built.
- Driven by: R2-6.
- Replaced: "copy-on-write chunk snapshots, a later optimisation" in `lactose-sim`
  (revisions 1 and 2).

**D35. Staged network data and a presentation-side interpolation buffer** (§9.2, §9.3)
- Why: nothing defined when received inputs and state were applied, so they could race systems
  and extract. Interpolating 20 to 30 Hz snapshots needs a per-entity buffer sampled about
  100 ms in the past; writing replicated rows directly steps visibly, and interpolating into
  rows every tick dirties every remote entity every tick (R2-21). A predicting client must run
  slightly ahead and adjust its tick rate by one to three percent as RTT changes (R2-22).
- Driven by: R2-21, R2-22.
- Replaced: revision 2's "send rate 20 to 30 Hz, interpolated on the client" with no staging
  point, input policy or clock sync.

**D36. Lag compensation covers NPCs** (§9.5)
- Why: in PvE co-op the targets that need rewinding are NPCs. The 250 millisecond cap has no
  recorded rationale.
- Driven by: R2-21.
- Replaced: revision 2's "player transforms and colliders only".

**D37. Proxies in the past are an accepted limitation** (§9.4)
- Why: kinematic proxies sit about half a round trip plus the interpolation delay in the past
  while the local player is ahead, so co-op pushing fights; the review asked for this to be
  named with a mitigation, which is tentative presentation until confirmed.
- Driven by: R2-21.

**D38. Handshake includes a boundary build ID and a cooked-data hash** (§9.1)
- Why: "schema hash plus `net` version" let builds with different gameplay code, Rapier, glamx
  or `libm` versions, or different cooked simulation data, connect and silently mispredict.
- Driven by: R2-30.
- Replaced: revision 2's registry schema hash and `net` crate version.

**D39. The server holds a player at a region boundary until the client is ready** (§14)
- Why: accepting a correction storm instead was considered and rejected (revision 2 §8).
- Driven by: R1-14.
- Replaced: revision 1, which did not specify client gating.

**D40. Snapshot contents; the server never rolls back** (§10)
- Why: a chunk copy misses state outside chunks: Rapier body sets, contact manifolds,
  warm-start impulses, island sleep state, joints, the relationship reverse index and
  in-flight command buffers; restoring chunks without the physics world produces different
  results (R1-4). Snapshots are per world, and only the prediction world is rolled back
  (R2-1). The counter-based RNG has no state to snapshot (R2-14).
- Driven by: R1-4, R2-1, R2-14.
- Replaced: revision 1's "a chunk copy and takes microseconds"; revision 2's `snapshot(area)`
  including the block cursor and RNG state.

## 6. Rendering and presentation

**D41. Grid cell plus offset in the GPU scene; a typed render space** (§12.4)
- Why: camera-relative positions produced at extract change for every instance whenever the
  camera moves, so every tick would rewrite every instance's transforms (500k instances times
  about 96 bytes is about 48 MB per tick). The cell difference is small, so subtracting in the
  shader is precise in f32. A distinct type prevents implicit conversion from simulation
  scalars.
- Driven by: R2-4.
- Replaced: revision 2's camera-relative 32-bit positions produced at extract.

**D42. One indirect draw per pipeline over fixed-size clusters** (§12.4, §2 Draw submission)
- Why: Metal has no indirect-count (R1-12). On MoltenVK, multi-draw-indirect runs a CPU loop
  encoding one Metal draw per command, and emulated indirect-count still encodes the maximum
  draw count, so zero-instance draws cost CPU encode and GPU front-end time proportional to the
  maximum every frame. Programmable vertex pulling behaves the same on every backend and needs
  no indirect-count (R2-5).
- Driven by: R2-5; R1-12 earlier.
- Replaced: revision 1's multi-draw indirect with count buffers; revision 2's zero-instance
  draws as the primary path.

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
  compile cost (revision 2 §10.5). The material axes omitted pass variants; the real count is
  about 64 to 80 graphics pipelines (R2-28). The specific budget of 96 has no recorded
  rationale.
- Driven by: R1-23, R2-28.
- Replaced: revision 2's four material axes only.

**D48. Late-latch the full orbit transform; the camera rig is presentation** (§5.3, §13)
- Why: latching rotation alone turns a third-person orbit camera about the wrong point (about
  40 cm off-centre on a fast flick) and keeps a stale, collision-shortened boom. "The yaw
  actually displayed" is unknowable when input is sampled, so input carries absolute yaw and
  pitch. CPU light binning in view space and pre-latch cascades are invalid after the latch,
  and mouse rotation per frame is unbounded, hence world-space GPU binning, padding and a
  clamped delta.
- Driven by: R1-11, R2-17.
- Replaced: revision 2's rotation-only latch, camera collision in the simulation and inputs
  stamped with the displayed yaw; revision 1's camera and view matrices patched before
  recording.

**D49. Render slots in a renderer-owned side table** (§12.4)
- Why: as a derived column in simulation chunks, a rollback restored chunks but not the
  renderer, leaking or aliasing slots, and a CPU slot column contradicted the GPU-side free
  list.
- Driven by: R2-18.
- Replaced: revision 2's render instance slots as a `derived` column.

**D50. A presentation interface crate, snapshot and schedule** (§13, §5.3)
- Why: gameplay could depend only on foundation crates and none described presentation, so it
  could not spawn VFX, play audio, drive UI or emit debug draw. During the overlap nothing was
  safe to read for camera smoothing, UI animation, the audio listener, markers or per-frame pose
  evaluation, without which skinned characters judder on 120 and 144 Hz displays.
- Driven by: R2-16.
- Replaced: revision 2's "the render side holds no CPU mirror of simulation data".

**D51. A null RHI backend** (§12.1)
- Why: graph tests, and a server binary that must not link Vulkan, need a headless RHI.
- Driven by: R2-26.

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
- Why: a loaded process started with default transient columns and a fresh physics world, so
  the save/load comparison passed only for a world at rest. Animation phase drives hit frames
  and body velocity lives only inside Rapier, so neither is a cache; a falling boulder would
  stop mid-air after a load.
- Driven by: R2-13.
- Replaced: revision 2's transient as "default-initialised on load", with velocity caches and
  animation phase as examples.

**D55. Typed save records; a patched baseline keeps runtime entities** (§15.2)
- Why: (PersistentId, field ID, value) entries cannot express runtime spawns, destroyed cooked
  entities, added or removed components, or region moves, and "drops the rest" on a patched
  baseline deleted every runtime-spawned entity, players included.
- Driven by: R2-12; R1-15 earlier.
- Replaced: revision 2's field-value overlay entries and its drop rule.

**D56. A save copies persistent columns only** (§10, §15.3)
- Why: a full rollback snapshot of every area, including a physics clone a save never
  serialises, stalls all players for milliseconds.
- Driven by: R2-13.
- Replaced: revision 2's "snapshot every active area"; revision 1's claim that the snapshot "is
  a chunk copy and takes microseconds".

**D57. The save/load test reloads in place** (§15.5)
- Why: comparing hashes at the save tick cannot catch a component wrongly marked transient
  (R1-15). Comparing a loaded process with an un-reloaded original compares different states
  (no manifolds, warm-start or sleep state), so both sides reload (R2-13).
- Driven by: R1-15, R2-13.
- Replaced: revision 1's hash equality at T; revision 2's comparison at T+K against the
  original.

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

**D59. Simulation scalar as a type alias, 64-bit by default, decided in milestone 1** (§2 World coordinates, §16)
- Why: R2-8 asked for the precision to be decided by a test at maximum extent, arguing that f32
  gives about 0.5 mm at ±6 km while f64 costs about twice the solver arithmetic and memory on
  NEON. Why 64-bit remains the default is not recorded.
- Driven by: R2-8.
- Replaced: revision 2's fixed 64-bit positions and 64-bit Rapier build; revision 1's f32 math.

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
  may contain unsafe code.
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
  depended on crates scheduled later; independent render work was serialised; `ecs` blocked
  the other agents (R2-10).
- Driven by: R1-17, R2-10.
- Replaced: revision 1's interface-first rule for every subsystem with one crate per task;
  revision 2's single-track milestones with `persist` in milestone 1, `asset-api` in 3, `input`
  in 5 and netcode only in 4.

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
  debug info and a fast linker per platform; Windows exceeds it without `lld-link`.
  Registration machinery is a known compile-time sink if it grows deep; the review recommended
  generated functions over life-before-main without giving a separate reason for that choice.
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
- **Editorial split of revision 3**: the architecture document was split into
  `docs/architecture.md`, this log and `docs/verification.md`, with no decision changed.
  `docs/reviews/traceability.md` maps every finding of both reviews to where it is addressed.

## 11. Rationale recorded in revision 3.1

These decisions had no recorded reason until the editorial pass asked for one.

| Decision | Rationale |
|---|---|
| No fixed player cap | User decision for revision 3. Bounding server cost by active entities and per-client cost by relevance makes the cap a deployment parameter rather than an architectural one. Thirty-two clients in milestone 4 is enough to exercise relevance sets and priority accumulators beyond the trivial case while fitting a CI runner. |
| No Bevy | User decision at the start. The gap analysis found Bevy lacks access-derived ordering, a unified work-stealing pool, fibers or continuation scheduling, a frame graph with aliasing, cache-sized chunking, immutable assets, parallel serialisation and late-latching, and its wgpu backend blocks the render-graph and GPU-driven goals. |
| 64-bit simulation scalar default | Review 2 showed 32-bit precision at the map edge is about half a millimetre, within Rapier's tolerance but marginal, and that 64-bit costs about double on NEON. The cost lands on the x86 server, where AVX gives 64-bit SIMD the same lane count, and on the small client prediction world. The safer default is kept and the type alias makes the milestone 1 test a one-line switch. |
| Pipeline budget of 96 | The four material axes by four pass types give 64 graphics pipelines. The remainder is headroom for post-processing, compute, UI and debug pipelines, kept small because MoltenVK compiles each at creation. |
| Catch-up cap of four ticks per frame | A hitch longer than four ticks (about 67 ms) slows simulation time rather than spiralling into ever-longer frames. Four is the largest cap that keeps a frame under the display interval on the target hardware. |
| Lag compensation cap of 250 ms | Covers a 200 ms round trip plus one send interval. Beyond that the server stops rewinding and the client is told, so a high-latency player cannot hit targets that moved a quarter second ago. |
| Copy-on-write snapshots | Not built up front. With a prediction world holding only predicted entities, a full copy of its mutable columns is small. Review 2 asked for copy-on-write from day one; the author kept it measured because the smaller world removes the cost that motivated it. |
| Two background threads on homogeneous CPUs | Enough to keep asset decode and compression flowing without oversubscribing frame-critical workers; the OS scheduler handles the small overcommit at low priority. |
| Interest management in the first networking deliverable | "From day one" meant designed into the first network code rather than bolted on; milestone 4 is the first networking deliverable. The wording was clarified. |
| `game-ai` as the fourth game crate | Review 2 asked for at least four game crates to bound incremental build cost. AI behaviour is the largest independently compilable slice of gameplay and is inside the boundary. |
| Milestone 4 correction threshold | Fewer than one correction per minute during solo traversal over static geometry at 100 ms round trip with 1% loss. This is the case where bit-exact determinism should yield near-zero corrections, so it is the test that determinism is doing its job. |
| Presentation owns pose evaluation; renderer owns skinning | Pose evaluation is CPU work on the presentation snapshot per frame; skinning is a GPU compute pre-pass. Splitting by where the work runs keeps `presentation` free of RHI types. |
| Interpolation buffer owned by `net` | It is network state outside the boundary and sampled per frame, so it belongs to `net` and is read through a `net-api` resource rather than copied into the presentation snapshot at extract. |
| Frame-critical workers equal performance cores everywhere | Review 2 showed "physical cores minus one" places frame-critical work on efficiency cores on hybrid x86. The fallback applies only where no core-type information exists. |
