# LACTOSE architecture rev 2: adversarial review

I read `docs/architecture.md`, `CLAUDE.md`, `crate-tiers.toml`, `README.md` and the rev 1 → rev 2 diff, and checked the external claims against current sources. Three verified facts change the analysis:
- **Rapier:** since v0.35 (Aug 2026) its determinism feature works with its parallel solver, and SIMD is always on. That breaks the premise of §14.
- **MoltenVK:** multi-draw-indirect is a CPU loop that encodes one Metal draw per command.
- **Rust floats:** `min`/`max` on ±0 and the sign of NaN results are documented as non-deterministic.

Findings tagged **[rev2]** were introduced by the first review's fixes.

## BLOCKING

### 1. [BLOCKING][rev2] Entity IDs are local to each process but are used as an ordering key across machines (§6 Entity IDs, §7.3, §8, §9)
- **Claim:** 64-bit IDs come from a block table that is "region-independent, deterministic and persistent". Each region holds a current block, and each player has reserved blocks that the server "confirms or remaps". Entity ID orders command application, physics insertion and reductions, and keys every RNG draw.
- **Why it fails:**
  - **IDs differ between server and client.** The block table's state depends on the order regions requested blocks. On the server that order comes from all four players' streaming. On a client it comes from that client's own streaming, and the client never simulates most regions. So the same cooked boulder gets different IDs on server and client. Every ID-keyed rule (RNG draws, command order, physics insertion order, sorted reductions) then diverges even with bit-exact floats. The replication ID mapping is also unspecified.
  - **Table ownership is undefined.** Entities keep their ID when they migrate, so one area's lookup and generation entries end up scattered across blocks owned by other regions, some in other areas or unloaded. A per-area snapshot can't bulk-copy them. Restoring area X must not touch area Y's entries. When B frees an index from A's block, nothing says whose free list gets it. §9's snapshot list doesn't include the entity table at all, only "block cursor".
  - **Predicted spawns force remapping everywhere.** A mispredicted spawn (a throw the server rejects) consumes reserved indices, so client and server cursors drift and later IDs need remapping. References to a predicted ID already exist in components, in the rollback ring, in VFX/audio/UI state, and in inputs already sent ("detonate bomb P"). Remapping means rewriting all of those through reflection, including history.
  - **Exhaustion.**
    - If blocks never reuse indices, 2^32 at 5k spawns/s of arrows, debris and fire lasts about 10 days of play. The table persists across a co-op campaign.
    - A player's 4096-index reserved block runs out mid-fight, and prediction stalls a full round trip waiting for the next block.
    - 32-bit generation wrap is not a practical issue.
- **Change:**
  - **Three identities:**
    - `EntityId`: process-local, never on the wire or on disk.
    - `NetId`: server-assigned. Cooked entities derive it from their cooked placement ID, so clients know it without a message; runtime spawns get it from a server counter and it is replicated.
    - `PersistentId`: for saves.
  - **Stable keys only:** every ordering and RNG key inside the boundary uses the stable key (NetId, or a prediction key), never `EntityId`.
  - **Prediction keys instead of reserved blocks.** A predicted spawn is keyed by (player, input tick, ordinal). The server echoes the key on the entity it spawns. The client merges its predicted entity into the authoritative one instead of remapping. Delete reserved blocks.
  - **Snapshots are per world.** Only the client's prediction world (finding 6) is ever rolled back, and it has its own table.

### 2. [BLOCKING][rev2] A worker waiting on a fork-join picks up unrelated work, which lengthens the critical path (§5.1, §5.3, Goal 1, M0)
- **Claim:** the worker that opens a scope "executes scope-local work and steals other work until the join counter reaches zero". Steal priority is own frame-critical, then others' frame-critical, then background.
- **Why it fails:**
  - **Stolen system tasks nest on the stack.** Once the opener's children are stolen, it takes whatever is ready. If that is a whole system task, the stolen system's scope opens on top of the opener's stack. The original system can't return until the stolen one finishes, even if its own children finished long ago. Worst case is the physics step: one serial task lasting milliseconds.
  - **Background work stalls frame-critical joins.** With no frame-critical work left, the waiting worker takes background work: a texture transcode, the zstd save compression (§13.4 runs it "on worker tasks"), or a long future poll. Nothing preempts it.
  - **M0's metric fails under mixed load.** This hits Goal 1 directly. Rayon has the same property, which is why frame-oriented engines use fibers or continuation-based job graphs instead.
- **Change:**
  - **Systems complete by continuation.** The scheduler counts down each system's chunk jobs and releases dependents at zero. No worker ever blocks on a system.
  - **Blocking joins only inside a system.** Keep them for cases that need the result before continuing (a reduction). A waiting worker may run only jobs of the same scope; when none remain it spins, then parks on the scope's latch.
  - **Never background while waiting.** Background CPU work yields at ≤0.5 ms slices or runs on a separate thread class (finding 22).
  - **Stack-allocated child jobs.** The scope blocks, so they need no allocation.

### 3. [BLOCKING] One dirty bit can't serve per-client replication and extract at the same time (§6 Change tracking, §8 Replication)
- **Claim:** each packet carries "rows whose dirty bit is set since the acknowledged tick". Extract and persistence read the same bitmask.
- **Why it fails:** a bit carries no time. Extract consumes it every tick. Replication sends at 20–30 Hz to four clients with four different acknowledged ticks, and a lost packet means resending everything since an older ack. Whoever clears the bit first hides the change from everyone else; never clearing it leaves every row dirty forever. As written, this can't be implemented.
- **Change:**
  - **Per-row tick.** Give each replicated or extracted column a per-row change tick (u32), next to the existing per-chunk tick, which still filters whole chunks.
  - **Per-consumer "last seen" tick:** extract keeps its own, and each client has its acked tick. Each reads rows with tick > seen. Packet loss needs no per-packet bookkeeping.
  - **Cost:** 4 bytes per row per tracked column.
  - **Alternative:** keep the bit as "written this tick" and have each consumer OR it into its own accumulated mask.
  - **Change on value, not on write.** Add `set_if_neq`, or the stream carries rows rewritten with the same value.
  - **Saves don't need change tracking;** they diff against the cooked baseline.

### 4. [BLOCKING] Camera-relative f32 positions at extract contradict a persistent, dirty-row GPU scene (§2 World coordinates, §5.2, §10.4)
- **Why it fails:** when the camera moves, every instance's camera-relative position changes, dirty or not. Producing them at extract means rewriting every instance's previous and current transform each tick: 500k instances (the M3 target) × ~96 B ≈ 48 MB per tick, against a 1 ms budget. Previous and current must also share a reference frame while the camera itself interpolates per render frame.
- **Change:**
  - **Coarse render origin in the GPU scene.** Store positions relative to a grid cell (int32 cell + f32 offset, or a hi/lo float pair as in UE5's tile + offset), uploaded only when the row changes.
  - **Subtract in the shader:** `(cell − cam_cell)·size + (offset − cam_offset)`. That is precise in f32 because the cell difference is small.
  - **If a single moving origin is used instead,** a rebase is a deliberate full-scene event, amortised over frames, rebasing prev, current and every history buffer (TAA, motion vectors, HZB) together.
  - **Type it.** Make "render space" a type in `math` with no implicit f64→f32 conversion.
  - **Also needs the conversion:**
    - **GPU culling:** instance bounds, using swept prev/current bounds.
    - **Lighting:** the clustered light grid, decals and probes.
    - **Particles:** FROTH emitters and particle state.
    - **Shadows:** cascade fitting and shadow-caster culling.
    - **World geometry:** the terrain clipmap centre and sky/atmosphere.
    - **UI:** world-space markers.
    - **Audio:** the listener and emitters (audio engines are f32).
    - **Debug:** Rapier debug-render (it emits f64 lines) and GPU picking.
    - **The camera itself.**

### 5. [BLOCKING] On macOS via MoltenVK the GPU-driven path is CPU-bound (§2 Rendering, §10.1, §10.4, M3)
- **Verified** in MoltenVK `MVKCmdDraw.mm` (main):
  - `vkCmdDraw(Indexed)Indirect` with `drawCount > 1` runs a CPU `for (drawIdx < _drawCount)` loop that encodes one Metal `drawIndexedPrimitives:indirectBuffer:` per draw, with per-draw offset rebinding and a CPU-written draw-ID buffer.
  - Indirect-count is emulated by a compute pass that zeroes the commands past the count. The CPU still encodes `maxDrawCount` draws.
- **Why it fails:** "fixed maximum draw count with zero-instance entries" costs CPU encode time and GPU front-end time proportional to the maximum draw count, every frame, on the platform §1 calls the default shipping configuration. M3's "handful of CPU submissions" is nominal only.
- **Change:**
  - **Primary path: one indirect draw per pipeline** over instances or clusters, with programmable vertex pulling: mesh/cluster index from the instance ID, a fixed vertex count per cluster, degenerate triangles as padding. This behaves the same on every backend and needs no indirect-count.
  - **MDI as an optimisation** only where a backend reports native support.
  - **Extend the §19 spike:**
    - CPU encode time for a 10k-draw MDI.
    - KosmicKrisp (LunarG's Mesa Vulkan-on-Metal driver, now Vulkan 1.3/1.4 conformant): whether it lowers MDI to Metal indirect command buffers. Unverified; check it.
  - **Native Metal before M3** if Apple Silicon remains the primary client.

### 6. [BLOCKING] The predicted area is defined by regions, but prediction is per entity, and its cost isn't budgeted (§6 Region modes, §8, §9)
- **Why it fails:**
  - **Two writers per row.** Region modes apply per region, so a `Simulated` region on the client runs every system on every entity in it: NPC AI, other players, props. Replication also writes those rows. Nothing arbitrates.
  - **Snapshot cost.**
    - Snapshotting "every chunk of every region in the area" copies static props, NPCs and so on, plausibly 5–20 MB per tick.
    - The ring holds up to 40 of those.
    - On top of that is a Rapier world clone every tick. It allocates, and §9 puts it at "hundreds of µs to ms".
  - **Re-simulation cost multiplies.** A correction k ticks deep costs k × (systems + physics step + snapshot) in one frame. To absorb a 40-tick rollback inside 16.6 ms, the predicted tick must cost under ~0.3 ms. At 150 ms RTT, every correction is 10–12 ticks deep. Nothing derives the 40-tick cap.
  - **There is nothing to resync from.** "Requests a full area snapshot" assumes something the server doesn't have. Its physics world has different contents and handles from the client's predicted area.
- **Change:**
  - **Prediction world.** A client-only `Predicted` marker plus a separate client prediction world holding only predicted entities and static collision.
  - **Copy-on-write snapshots now.** Snapshot only that world's mutable columns, using copy-on-write from day one.
  - **Derived ring depth:** frame resim budget / measured per-tick cost. Smear deeper corrections over several frames, or resync.
  - **Resync** = rebuild the prediction world from replicated state.

## SERIOUS

### 7. [SERIOUS] Cross-architecture bit-exactness doesn't deliver what Goal 2 promises (§1 Goal 2, §2, §8, M4)
This disagrees with the first review's framing.
- **Claim:** cross-arch determinism is a "product requirement" because the Apple client predicts against the x86 server.
- **Why it fails:** bit-exactness only pays when a predicted entity's inputs are identical on both machines. The design breaks that four ways:
  - **Keys:** IDs and ordering keys differ (finding 1).
  - **World contents:** the client world has kinematic proxies, different handles and insertion history, and probably a different constraint ordering. Rapier ≥0.35 solves "awake bodies as a single active set". Verify by stepping the same scene with and without an unrelated distant awake body and comparing hashes.
  - **Quantised corrections:** corrections arrive quantised relative to region origin, so after any misprediction the client restores a state the server never had.
  - **Unreplicated state:** the sim reads transient columns, Rapier contacts and warm-start, and timers, none of which are replicated.

  M4's "x86 server, ARM client, hashes equal" names no hash the two sides could share.
- **What it does buy:** zero corrections for solo traversal through static geometry, provided keys match and comparison happens in quantised space. Plus SOUR, replays and desync debugging.
- **Change:**
  - **Restate Goal 2** as an engineering property: same build gives the same result across threads, layouts and architectures.
  - **Define reconciliation:**
    - compare predicted state with the server's in quantised space per field;
    - corrections carry every sim-read field at full precision, or the server quantises its authoritative state inside the sim;
    - interactions with non-predicted entities are expected to mispredict, and are smoothed.
  - **Replace M4's criterion** with "a recorded server session replayed on aarch64 hashes equal, and the solo-traversal correction rate stays under N/min".

### 8. [SERIOUS] The Rapier premise in §14 is out of date, and per-area worlds with merge/split cost more than they buy (§2, §6 Active areas, §14, §19)
This disagrees with two rev 2 defaults.
- **Verified** in the Rapier CHANGELOG:
  - v0.35.0 (Aug 2026): "`enhanced-determinism` can now be combined with `parallel`, with results bitwise identical for any thread-pool size... incompatible with `simd8`".
  - SIMD (`wide`, 4 lanes) is always on; `simd-stable` was removed.
  - v0.32 (Jan 2026) moved Rapier's math from nalgebra to glam/glamx.
  - `rapier3d-f64` has `enhanced-determinism`.

  So §14's "serial per world, parallel across areas only" rests on a false constraint, and the §19 spike's question is already answered.
- **Why merge/split is a bad deal:**
  - **It loses contact state.** Rapier can't move a body together with its contacts between worlds. Re-inserting means:
    - new handles;
    - lost contact manifolds and warm-start, so stacks jitter or wake on the merge tick;
    - lost island sleep state;
    - every joint re-created;
    - spurious "contact started" events that pressure plates and impact damage will act on.
  - **It couples unrelated players.** A boulder's result depends on whether a player 2 km away caused a merge.
  - **It thrashes** without hysteresis.
  - **It adds a second thread pool.** `parallel` uses rayon's own pool, which must be reconciled with WHISK.
  - **Rapier churns.** It shipped five breaking releases between Jan and Sep 2026, including solver, sleep and CCD defaults, and "snapshots serialized with previous versions can't be loaded anymore".
- **f64 costs more than it needs to:**
  - `simd8` is f32-only.
  - On NEON (128-bit vectors) a 4-lane f64 vector takes two operations, so the Apple client pays about 2× solver arithmetic plus 2× memory in every snapshot and transform.
  - At a BotW-sized ±6 km, f32 precision is ~0.5 mm, an order of magnitude below Rapier's default allowed error.
- **Change:**
  - **One physics world** per server process (plus one prediction world per client), using `enhanced-determinism` + `parallel`, with rayon's pool lent workers during a WHISK physics phase.
  - **Drop merge/split** unless profiling demands it.
  - **Decide f64** by an M1 precision test at the world's maximum extent.
  - **Pin versions exactly** for Rapier, parry, glamx, wide and libm, and treat any bump as a protocol and replay break.

### 9. [SERIOUS][rev2] The fork-join's safety relies on an abort that isn't unconditional, and the scope API's shape isn't fixed (§5.1, §16)
- **Why it fails:**
  - **The abort doesn't always happen.**
    - Cargo ignores `panic = "abort"` for tests and benches, and Miri and loom unwind as well.
    - A panic in a `std::thread` worker unwinds only that thread; it doesn't abort the process.
    - Without a guard, a child that panics on another worker leaves the counter above zero, and the opener hangs.
    - If the opener panics while helping, unwinding leaves the scope frame while other workers still borrow from it: use-after-free.
  - **A guard-object scope is unsound.** One joined in `Drop` breaks under `mem::forget`, the original `thread::scoped` bug. The doc never says how a `Scope` is obtained.
  - **"Children cannot `.await`" only covers await.** The type system forbids awaiting, but not blocking: `block_on`, channel `recv`, spinning on a sibling's flag. Under SOUR's 1-worker mode those hang instead of failing.
  - **"Chunk work opens none" needs a runtime check.**
  - **A background `'static` future can open a scope too.** It then blocks inside `poll`, which is acceptable only with finding 2's same-scope rule.
- **Change:**
  - **Closure-only API:** `task::scope(|s| s.fork(..))` with an invariant `'scope` lifetime, as rayon and `std::thread::scope` do.
  - **Unconditional abort:**
    - an abort-on-unwind guard held across the join wait and around every job (abort if `thread::panicking()`);
    - plus a panic hook that writes the dump and then calls `abort()`.
  - **No blocking waits from workers:** `task` exposes none (debug-assert if one is called on a worker).
  - **cargo-nextest:** one process per test, with timeouts.
  - **Wider loom coverage:** model the sleep/wake protocol under loom, not just the join counter.

### 10. [SERIOUS] Milestone order inverts the project's risk and its critical path (§17, §18)
- **Why it fails:**
  - **The netcode is validated last.** Every M1 structure (IDs, regions, areas, snapshot API, change tracking, determinism scope) is shaped by netcode that isn't exercised until M4. Findings 1, 3, 6 and 7 would surface after `ecs`, `sim` and `persist` are frozen.
  - **Dependencies arrive after the milestone that needs them.**
    - `persist` (M1) diffs against a cooked baseline that only exists in M3, and writes through `io` (M3).
    - `sim` in M1 needs `asset-api` types (version tables are snapshot state) and `input` (M5).
  - **Independent tracks are serialised.** The render track (rhi → graph → renderer) touches `ecs` only at extract.
  - **`rhi-api` freezes too early.** It freezes after the Vulkan backend alone; Metal's residency, argument buffers, heaps and indirect command buffers will then force breaking changes.
  - **`ecs` blocks the other agents.** It has no interface split, so `sim`, `anim`, `persist` and `game` agents all wait on it. Realistic M1 parallelism is 2–3 agents.
- **Change:**
  - **Three tracks from M1:**
    - sim: ecs → sim → physics → net spike;
    - render: rhi spike → vulkan → graph → GPU scene;
    - assets: io → asset → cook.

    They join at extract (M2) and streaming (M3).
  - **An M1 netcode spike:** x86 server, aarch64 client, one predicted character, 50 bodies. Measure correction rate, rollback cost and divergence.
  - **Move `asset-api` and `input` types into M1.**
  - **An `ecs` system/query facade in week 1** with a naive single-threaded implementation, so dependent agents can start.
  - **Keep `rhi-api` unstable** until a native Metal spike has run.

### 11. [SERIOUS] The float rules are wrong in specifics that will break cross-architecture hashes (§7.3, §7.4)
- **min/max.** These aren't bit-exact. Rust's docs for `f32::min`: "If the inputs compare equal (such as for the case of +0.0 and -0.0), either input may be returned non-deterministically." x86 `minss` returns the second operand for ±0; AArch64 `fminnm` orders −0 < +0. SIMD `minps` and NEON `fmin`/`fminnm` also differ on NaN. `f32::minimum` is still unstable.
- **NaN sign.** Sign and payload of NaN results are non-deterministic. Canonicalising only at hash and serialise time isn't enough: `total_cmp` sorts negative NaN first and positive NaN last, and `to_bits`, `copysign` and `is_sign_negative` leak the sign into results.
- **`mul_add` and target features.** The ban is aimed at the wrong thing: fused multiply-add is correctly rounded and identical on both ISAs. The real hazard is choosing fused or unfused by target feature:
  - **Verified in `wide`** (a Rapier dependency): `mul_add` is fused on aarch64 always, but on x86 only under `cfg(target_feature="fma")`, otherwise `(a*b)+c`. glam's SSE2 `Vec4::mul_add` follows the same pattern.
  - **So the x86-64-v3 pin is load-bearing for correctness.**
  - **The pin is silently dropped** whenever `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` is set (they replace config `rustflags`), and when cargo runs from outside the workspace directory.
- **glam's SIMD dot product differs by ISA.** SSE2 `dot4` sums `(x+z)+(y+w)`; NEON `vaddvq_f32` sums `(x+y)+(z+w)`.
- **Feature unification.** parry's `enhanced-determinism` turns on `glamx/scalar-math` + `glamx/libm`. Cargo applies that to every user of glam 0.33 in the same build. Consequences:
  - `math` built on glam computes differently in a `-p lactose-math` test build than in the shipped app;
  - the renderer silently loses glam SIMD.

  Rapier's own workspace manifest warns that glam feature unification is "a cross-platform determinism hazard".
- **Runtime detection.** `libm` itself does runtime `cpuid` detection on x86 (`arch/x86/detect.rs`), so the blanket rule bans the crate the doc mandates. The detection is harmless (both paths correctly rounded); the rule should target numerically different dispatch.
- **Gaps in the ban list.**
  - `powi` is documented "precision non-deterministic" and isn't banned.
  - Trait routes to platform libm (`num_traits::Float`, simba without `libm_force`) aren't banned either.
  - Clippy never lints dependencies.
- **FP control registers.** MXCSR and FPCR (flush-to-zero, denormals) are per-thread state, and workers are shared with audio, codec and driver code that may change them.
- **Two `libm` versions in the tree** break "simulation and physics agree on `sin`".
- **Change:**
  - **Explicit compare-and-select.** `math` owns `min`/`max`/`clamp` as compare-and-select with defined ±0 and NaN results, and clippy disallows `min/max/powi/total_cmp/to_bits` outside `math`.
  - **No glam in boundary math.** `math` doesn't use glam, or enables `scalar-math` itself unconditionally.
  - **Enforce the pin:** `#[cfg(all(target_arch="x86_64", not(target_feature="fma")))] compile_error!`.
  - **FP state:** set and assert the FP control register at worker start and per task batch.
  - **One libm:** cargo-deny `multiple-versions = "deny"` for libm and glam.
  - **Architecture matrix:** linux-x64 (AVX-512 host plus a Haswell-masked run under SDE or `qemu -cpu Haswell`), windows-x64, macos-arm64.

### 12. [SERIOUS] The save overlay can't represent most world changes, and its drop rule deletes runtime entities (§13.2, §13.3, §13.5, §6)
- **Why it fails:**
  - **Field edits only.** Entries are (PersistentId, field ID, value). That can't express:
    - runtime spawns (there is no baseline);
    - destroyed cooked entities;
    - components added or removed (a tree gains `Burnt`);
    - region moves. A boulder pushed from A into B reappears in A when A loads without B, and B has nothing to load.
  - **The drop rule deletes runtime entities.** "Drops the rest" on a baseline patch deletes every runtime-spawned entity, including players (§13.6).
  - **IDs collide for reused prefabs.** A PersistentId taken "from the source asset's GUID" collides when a prefab is placed twice or has children.
  - **The fast path copies stale references.** Rev 1's rule ("references serialise as PersistentId, assets as AssetId") was dropped in rev 2, so the "bulk copy into chunks" fast path copies raw `EntityId`s and `Handle`s into a new process.
  - **Inventories strand [rev2].** Inventory-as-entities plus region-owned entities plus weak cross-region references means a player walks out of A, A unloads, and the inventory goes with A.
  - **The ID counter serialises spawning.** A global PersistentId counter mutated by spawning systems in every area serialises them unless it is assigned at command apply.
- **Change:**
  - **Record kinds:** Spawn, Destroy, AddComponent, RemoveComponent, SetField and ownership change.
  - **Keep runtime entities** on baseline patches.
  - **Owned entities migrate with their owner.**
  - **Typed references.** `EntityRef` and `AssetRef` fields are typed in the registry and remapped on load, so the fast path becomes bulk copy plus a remap pass.
  - **IDs from placement:** PersistentId from (scene, placement, child path), assigned at apply on the server.

### 13. [SERIOUS][rev2] The save/load test contradicts the transient/derived policy (§13.1, §13.4, §13.7, §9)
- **Why it fails:**
  - **The two processes start from different states.** The loaded process has default transient columns and a rebuilt physics world: no manifolds, no warm-start, fresh sleep state. The original has all of them. §9 itself says restoring without physics "produces different results", so the K-tick comparison passes only for a world at rest.
  - **The §13.1 examples are wrong.** Animation phase drives hit frames, and body velocity lives only inside Rapier, so a falling boulder stops mid-air after a load. Neither is a cache.
  - **The save snapshot is too heavy.** §13.4 takes a full §9 snapshot, including a physics clone a save never serialises, across every area at once: a multi-ms stall for all players.
- **Change:**
  - **Define transient as "written before read every tick"** and enforce it with a poison test: randomise transient columns after every restore or load in debug.
  - **Persist accumulated state:** velocities mirrored from physics, animation phase, timers.
  - **The test compares like with like:**
    - the original does an in-place reload at T;
    - a fresh process loads the save;
    - both run K ticks and must match bit-exactly.

    Plus a tolerance test against the un-reloaded original for discrete state.
  - **Lighter save snapshot:** copy persistent columns only.

### 14. [SERIOUS] "(system order, entity ID)" isn't a total order, and system order comes from registration (§5.3, §7.3)
- **Why it fails:**
  - **Ties.** One system can issue several commands for the same entity, and several sources can target one entity (two arrows despawn one enemy), so ties fall to task order.
  - **Spawns have no ID yet.** If IDs are reserved during parallel chunk work, allocation order follows task timing.
  - **Registration order becomes semantic.** Non-conflicting systems whose commands interact are ordered by registration, which a client with extra systems or a `cfg`-gated plugin changes. The same applies to "system ID" in RNG keys.
  - **The RNG has no state.** It is counter-based, so §6 and §9's "RNG state" in the snapshot is inconsistent.
- **Change:**
  - **Total order:** (system stable key, source stable key, per-invocation sequence).
  - **Spawns get placeholders** resolved at apply.
  - **Explicit system names:** stable keys are declared name hashes, checked for collisions.
  - **Fix the snapshot list** (drop the RNG state entry).

### 15. [SERIOUS][rev2] `world_hash` order and SOUR layout mode either always fail or test nothing (§7.4, §5.1, §5.3)
- **Why it fails:**
  - **Storage-order hashing.** A field-by-field hash over columns in storage order depends on layout, so correct code fails layout mode.
  - **ID-changing perturbations.** "Shuffled insertion order" and "different region load order" change the IDs entities receive. ID-ordered semantics then change legitimately: false failures.
  - **Tests can't prove "never a semantic key".**
- **Change:**
  - **Hash in stable-key order,** or with an order-independent combiner.
  - **Perturbations that keep identity:**
    - a test-only chunk capacity (1, 7 or 64 rows, plus 16 KiB);
    - random row permutation at every sync point;
    - reversed and shuffled chunk iteration;
    - random batch splits.
  - **A chaos-schedule mode:** seeded steal order, injected yields. That finds far more than 1 vs N workers on 4-vCPU CI runners.
  - **Enforce by construction:**
    - the query API exposes no chunk, row or worker index;
    - reductions only through provided combinators;
    - scratch is an opaque per-task arena, poisoned between uses.

    That replaces §5.1's "scratch keyed by entity range", which is meaningless anyway because a chunk's rows don't form an ID range.

### 16. [SERIOUS] Game code has no route to presentation, and per-frame presentation has nothing to read (§4 tiers, §5.2 "no CPU mirror", §15)
- **Why it fails:**
  - **No route to presentation.** `game` may depend only on foundation crates, and none describes presentation. Gameplay can't spawn VFX (FROTH lives in `renderer`), play audio, drive UI, place decals, set camera parameters or emit debug draw. `rhi-api` is the wrong layer, and it is frozen late.
  - **Nothing safe to read during the overlap.** While tick N+1 mutates chunks and the renderer holds "no CPU mirror", these have no safe data source:
    - camera smoothing;
    - UI animation;
    - the audio listener;
    - world-space markers;
    - per-frame pose evaluation (without it, skinned characters judder on 120/144 Hz displays).
- **Change:**
  - **A foundation presentation crate,** interface-first, holding:
    - components for mesh, light, decal and camera rig;
    - VFX and audio request events;
    - UI view-model resources;
    - a debug-draw buffer.
  - **A double-buffered presentation snapshot** produced at extract.
  - **A variable-rate presentation schedule** that runs on that snapshot.
  - **Amend "no CPU mirror"** accordingly.

### 17. [SERIOUS] Late-latching rotation only is wrong for a third-person game, and the input stamp is ill-defined (§5.2)
- **Timeline:** input I(N+1) must exist when tick N ends (t0). Frame N latches at t_L > t0, during tick N+1, and is shown one or more vsyncs later. With uncapped rendering, several frames per tick each latch a different yaw, and which one is shown is decided after t0. "The latched yaw actually displayed" is therefore unknowable when the input is sampled. The definable value is "the yaw latched into the last frame submitted before sampling", and the promise "moves where they saw" is not kept.
- **Bigger problem, the orbit camera:** for a third-person orbit camera, position = pivot − R·boom. Latching R while keeping the sim's position rotates the view about the wrong point:
  - the character slides off-centre by boom × Δθ, about 40 cm on a fast flick;
  - the collision-shortened boom is stale, so the camera can clip into walls.
- **Other breakage:**
  - **CPU light binning in view space** before the latch is invalid after the latch.
  - **Shadow cascades** fitted to the pre-latch frustum leave the newly revealed edge without shadow coverage.
  - **Mouse rotation** per frame is unbounded, so the frustum pad has no bound.
- **Change:**
  - **The camera rig leaves the boundary.** It becomes presentation, per frame, with non-authoritative collision.
  - **The input carries absolute yaw and pitch.**
  - **Latch the full orbit transform,** not rotation alone.
  - **Bin lights on the GPU after the latch,** or in world space.
  - **Pad cascades and culling,** and clamp the latched delta to the pad.

### 18. [SERIOUS] Extract and GPU scene mechanics (§5.2, §6, §10.4, §13.1)
- **Why it fails:**
  - **The budget fails when everything is dirty:**
    - region load;
    - every client rollback;
    - origin rebase;
    - device-lost rebuild.

    My estimate is ~20–40 ns per dirty row single-threaded, so 1 ms covers about 25–50k rows; one region load can reach that. Measure it in M2.
  - **CPU/GPU race.** Writing straight into the persistent GPU scene while up to three frames are in flight races the GPU. Writes need a staging ring plus a GPU scatter pass.
  - **Interpolation bookkeeping.** A row that moved at N and stopped at N+1 must still have its previous transform set to its current one. Extract has to cover rows dirty in either of the last two ticks, and teleports need a no-interpolate flag.
  - **Render slots corrupt on rollback.** "Render instance slots" are a `derived` column inside sim chunks:
    - rollback restores chunks but not the renderer, so slots leak or alias two entities;
    - §10.4's "GPU-side free-list allocator" contradicts a slot column on the CPU.
- **Change:**
  - **"Serial" means exclusive.** Extract is internally parallel over dirty chunks.
  - **Static cooked instances** go IO → GPU and never pass through extract.
  - **Render slots** move to a renderer-owned side table keyed by stable key, reconciled after every restore.

### 19. [SERIOUS] The `Mut<T>` dirty bitmask is race-free only if a chunk is never split (§6, §5.1)
- **Why it fails:**
  - **Splitting chunks creates a data race.** The 50–200 µs target forces splitting chunks with expensive per-entity work (128 rows × 10 µs is 1.3 ms). Two workers doing read-modify-write on one u64 is a data race unless it is atomic: about 20 cycles for `lock or`, plus cache-line ping-pong.
  - **Raw-pointer bit-setting blocks vectorisation.** LLVM must assume the store may alias the component column.
  - **Whole-column slices bypass tracking.** `&mut [T]` slices (SIMD, transform propagation) either mark everything dirty or lose changes.
  - **No-op writes mark rows dirty.**
- **Change:**
  - **Split only at 64-row word boundaries,** ideally 512-row cache-line boundaries. No atomics are needed, and at 512 there is no false sharing either.
  - **Accumulate bits in a register** per 64 rows and store once, through `&mut` rather than a raw pointer.
  - **Slices take an explicit dirty-mask out-parameter.**
  - **`set_if_neq`** for replicated columns.

### 20. [SERIOUS][rev2] The tier and policy tables contradict each other and forbid edges the design needs (§3, §4, §7.1, §10.1, §16, crate-tiers.toml, CLAUDE.md)
- **Why it fails:**
  - **Three different rules:**
    - §3: "only `app` and their own tests";
    - §4: "only `app`";
    - the toml and CLAUDE.md: `app`, `tools/*` and three exceptions.
  - **Forbidden edges the design needs** (the first agent to build SILO fails CI):
    - `asset→io`;
    - `persist→io`;
    - `ui→renderer`/`render-graph`;
    - `debug-ui→platform`.
  - **Types in the wrong crate.**
    - `PersistentId` sits in backend `persist` but sim needs it.
    - Generic `Handle<T>` in forbid-unsafe `asset-api` needs a manual `unsafe impl` (bytemuck derives reject generic structs), which only `core` may contain.
  - **Boundary is per system, enforcement is per crate.**
    - The boundary is drawn per system, but clippy config is per crate.
    - `net` holds both RTT measurement (needs `Instant`) and rollback (bans it).
    - `nav`'s deterministic half is placed in `sim` by §4 and in `nav` by §7.1.
    - Terrain collision comes from a backend crate outside the boundary.
  - **Smaller inconsistencies.**
    - `tools/cook` needs FFI but isn't in the unsafe table.
    - §4 says "one worker per core"; §5.1 says per physical core minus one.
    - §10.1 says boxed trait objects belong only in the RHI, yet the tier rule forces physics, asset and net access through `dyn` too.
- **Change:**
  - **One rule,** stated only in the toml.
  - **`io` becomes foundation,** or the missing edges become exceptions with reasons.
  - **ID and handle types move to `core`.**
  - **Split crates on the boundary line:** `net-predict`/`net`, `game-sim`/`game-present`, `nav-query`.
  - **Cook collision; add `tools/*` to the unsafe table.**
  - **Document `dyn` resources wired by `app`.**

### 21. [SERIOUS] How network data enters and leaves the tick is undefined, and lag compensation uses the PvP model (§8, §6)
- **Why it fails:**
  - **No staging point.** Nothing says received inputs and state are staged and applied only at a defined sync point, never concurrently with extract or systems.
  - **No interpolation buffer.**
    - Interpolating 20–30 Hz snapshots needs a per-entity received-state buffer sampled at render time about 100 ms in the past.
    - Writing replicated rows directly gives visible 20–30 Hz steps.
    - Interpolating into rows every tick makes every remote entity dirty every tick.
  - **No input policy.** Server input buffering, late-input policy and client clock sync are unspecified.
  - **Proxies live in the past.** Kinematic proxies sit ~RTT/2 + interpolation delay in the past while the local player is ahead, so co-op pushing fights.
  - **The wrong rewind target.** Lag compensation rewinds only players; in PvE co-op the targets that need rewinding are NPCs.
- **Change:**
  - **Specify staging queues and sync points.**
  - **The interpolation buffer** feeds the presentation snapshot (finding 16), not sim rows.
  - **Hit history:** NPC hit history, or client-claimed hits with server plausibility checks.
  - **Name the time-frame mismatch** as an accepted limitation, with a mitigation.

### 22. [SERIOUS] The time and pacing model is missing (Goal 5, §2, §5.1, §5.2)
- **Gaps:**
  - **Who ticks.** "Fixed 60 Hz on its own cadence" plus "no coordinator" plus a main thread that does nothing else: something must wake at tick deadlines, cap catch-up after a hitch, and pace frames.
  - **Client clock sync.** A predicting client must run slightly ahead, adjusting its tick rate by 1–3% as RTT changes.
  - **Slow-motion and pause.** Bullet time and flurry rush affect all four players under a global time scale, and pause semantics in multiplayer are undecided.
  - **The main-thread lane.** It was specified in rev 1 and is gone from §5.1 in rev 2.
    - **Deadlock:** a worker synchronously waiting on a main-thread job while the main thread blocks in present (FIFO `vkQueuePresentKHR`, or `nextDrawable`, which can block up to 1 s) on a frame that needs that worker.
    - **Modal loops:** macOS live resize and Windows move/size loops stop the normal event pump.
- **Change:**
  - **A timer-driven DAG root** owns ticking and catch-up.
  - **Specify:** client clock sync, time scale as a sim resource with a fixed physics dt, and pause.
  - **Workers never wait synchronously on the main thread;** main-thread requests return futures.
  - **Present off the event thread.**

### 23. [SERIOUS] The executor ignores performance/efficiency cores, and M0's metrics don't measure what matters (§5.1, §18 M0)
- **Why it fails:**
  - **E-cores on the critical path.** Apple Silicon (M1: 4 performance + 4 efficiency) and Intel hybrid CPUs. "Physical cores − 1" puts frame-critical tasks on efficiency cores at 2–3× latency, and macOS offers only QoS, not pinning.
  - **"Within 10% of its theoretical bound" is undefined:**
    - which bound;
    - which DAG;
    - which machine.

    On shared CI VMs, noise alone exceeds 10%.
  - **"Spawn under 100 ns" is trivial and off the critical path.** A local deque push is ~10–30 ns. Wake latency (5–50 µs) and steal latency are what lengthen the critical path.
  - **Fixed 50–200 µs batches conflict with Goal 1.** A 300 µs system split into two 150 µs tasks adds more to the critical path than fifteen 20 µs tasks on idle workers.
- **Change:**
  - **Frame-critical workers = performance-core count** (`hw.perflevel0.physicalcpu`), at user-interactive QoS.
  - **A background thread class on efficiency cores** (utility QoS / EcoQoS).
  - **Split-on-steal** instead of a fixed time target.
  - **Define the M0 metric:** makespan / critical-path length at P50 and P99 on representative DAGs, on bare-metal performance+efficiency and x86 machines, with and without background load. Plus wake and steal microbenchmarks.

### 24. [SERIOUS][rev2] `AnyBitPattern` forbids ordinary game data; `NoUninit` is all that's needed (§6)
- **Why it fails:**
  - **Ordinary data is banned.** `AnyBitPattern` excludes:
    - `bool`, `char` and all enums;
    - `Option`, `NonZero` and niche IDs;
    - `ArrayVec`.

    Gameplay code ends up hand-encoding `u8`, against Goal 3.
  - **Only untrusted bytes need validation.**
    - Hashing and serialising need only `NoUninit`.
    - Snapshot restore copies bytes that were valid values in the same process, so it needs only `Copy`.
    - Only save files and network input need validation.
  - **Handles can't be refcounted.** `NoUninit` implies `Copy`, so asset lifetime must come from region asset sets, and a migrating entity must carry its asset references.
- **Change:**
  - **`NoUninit + CheckedBitPattern`,** validated on load and receive only.
  - **Pad fields zeroed** by the API.
  - **`EntityId`:** a sentinel, or `NonZero` with `PodInOption`, in `core`.
  - **Inline arrays:** `[T; N]` + len, unused slots zeroed.

### 25. [SERIOUS] Pose evaluation outside the boundary leaves gameplay without bones (§7.2, §10.4, §15)
- **Why it fails:** these all need bone transforms inside the boundary:
  - melee hit volumes;
  - climbing hand and foot placement;
  - IK grabs;
  - weapon sockets;
  - ragdolls in Rapier.

  With poses only on the render side, the sim has root motion and authored curves. Combat feels detached from the visible animation, or someone reads render-side poses and breaks determinism.
- **Change:**
  - **A deterministic "sim skeleton"** (sockets, hands, feet, head, ragdoll bodies) evaluated in `anim`.
  - **The full pose stays outside** for rendering.
  - **Budget it in M1.**

### 26. [SERIOUS] There is no error-handling strategy, and device loss is unrecoverable by design (missing; §5.1, §5.2)
- **Why it fails:**
  - **Every error becomes a crash.** "Panic aborts" plus no policy means agent code will `unwrap` IO, asset, network and GPU errors.
  - **Unhandled cases:**
    - missing asset: the region never simulates, and the player is held at the boundary forever;
    - asset failure on the client only, which is a determinism input;
    - reconnect;
    - disk full or corrupt save;
    - migration failure;
    - `DEVICE_LOST`: with no CPU mirror, recovery needs a full asset re-upload plus a full re-extract.
  - **No headless RHI** for graph tests or for a server binary that must not link Vulkan.
- **Change:**
  - **An error policy in §3:**
    - panics only for broken invariants;
    - a `core::Error` taxonomy;
    - `Result` up to an `app`-level policy;
    - tick-stamped error events inside the boundary.
  - **A tested device-lost path.**
  - **A `rhi-null` backend.**

### 27. [SERIOUS] Shader toolchain claims (§10.1, §10.2, §10.4 stage 3)
- **Why it fails:**
  - **Host constraint.** SPIR-V→Metal at cook time means SPIRV-Cross plus Apple's `metal` compiler, which runs on macOS or Windows but not Linux, so Mac shaders can't be cooked on the Linux CI.
  - **MoltenVK won't use the output.** MoltenVK converts SPIR-V itself at pipeline creation. Cooked metallibs help it only via its MSL magic-number path, with bindings matching MoltenVK's own conventions. Their real consumer is `rhi-metal`.
  - **Slang can target Metal directly:**
    - `ParameterBlock` → argument buffers;
    - specialisation constants → function constants;
    - `SubpassInput` → framebuffer fetch;
    - mesh shaders;
    - ray-tracing acceleration structures are unsupported.
  - **Drivers still compile at runtime.** "No runtime shader compilation" is false on every backend: drivers compile to GPU code at pipeline creation. Persist `VkPipelineCache` and `MTLBinaryArchive`.
  - **64-bit atomics.** Metal has only non-returning `atomic_ulong` min/max, on Apple8 (M2) and later; M1 has none. That limits a stage-3 software rasteriser.
  - **Hazard tracking doesn't apply.** "Relies on Metal's hazard tracking" fails for heap-aliased transients and for argument-buffer resources, which need residency calls.
- **Change:**
  - **Two outputs from Slang:** SPIR-V, and MSL/metallib for `rhi-metal`.
  - **Cook Mac shaders on a macOS runner.**
  - **Persist pipeline caches.**
  - **Capability-gate the stage-3 raster.**
  - **Fix the hazard sentence.**

### 28. [SERIOUS] The fixed permutation axes omit pass variants, and deferred on TBDR wastes tile memory (§10.4, §10.5, §10.1)
- **Why it fails:**
  - **Pass variants are missing.**
    - The axes declared as a "hard constraint" omit forward-lit transparency, depth-only shadow and prepass, and motion vectors.
    - The real count is about 16 × 4–5 ≈ 64–80 graphics pipelines, which matters given MoltenVK's per-pipeline cost.
  - **Deferred on TBDR is fine in principle, but the G-buffer must stay on-chip.** Deferred with an uber-shader and material IDs is consistent: it isn't a visibility buffer, because materials are evaluated in the G-buffer pass. On Apple's tile-based GPUs, though, the G-buffer should stay in tile memory. Otherwise it costs ~150 MB/frame at 1440p, about 9 GB/s of M1's 68 GB/s shared bandwidth. The RHI's "what all APIs share" rule excludes subpasses.
- **Change:**
  - **Pass type becomes an axis,** with a stated pipeline budget.
  - **RHI and graph gain:**
    - memoryless/transient attachments;
    - render-pass-local reads (`VK_KHR_dynamic_rendering_local_read` on Vulkan, framebuffer fetch on Metal).
  - **Verify on MoltenVK and KosmicKrisp.**

## MINOR

### 29. [MINOR] The 30 s build budget is credible only with specific settings; measure edit-to-playing (§15, M1)
- **Estimate for a `game` change:**
  - **Compile:** incremental front-end 3–8 s plus codegen 2–10 s, at opt-level 1. Opt-level 0 is too slow for a 60 Hz sim with 100k entities.
  - **Link:**
    - 1–3 s with mold or lld;
    - 2–4 s with ld-prime;
    - 5–20 s with MSVC `link.exe` and full debug info.

  That gives about 8–25 s on Linux and macOS. Windows exceeds 30 s without `lld-link` and line-tables-only debug info.
- **Compile-time risks:**
  - **Registration machinery.** Monomorphised queries are fine if it stays shallow; Bevy-style parameter introspection is a known compile-time sink.
  - **Restart cost on macOS.** "State-preserving restart" adds MoltenVK pipeline creation: hundreds of ms × 64–80 pipelines on a cold cache, which can exceed the build itself.
  - **Slang.** Use prebuilt `slangc` from the CLI (building Slang from source takes tens of minutes), and keep `tools/` out of `default-members`.
- **Change:**
  - **Prescribe the dev profile:**
    - dependencies at opt-level 3, game at 1;
    - `debug="line-tables-only"`;
    - split debuginfo;
    - a fast linker per platform.
  - **Split `game` into ≥4 crates.**
  - **Register through generated functions,** not life-before-main.
  - **Measure edit → playing again** per platform.

### 30. [MINOR] Numbers without a derivation, and the protocol hash (§5.1, §6, §8)
- **16 KiB chunks:** borrowed from Unity DOTS. With f64 transforms, rows of 300–1000 B give 16–50 rows per chunk. A wrong size costs only performance, provided SOUR varies capacity (finding 15).
- **4096-index blocks:** not sized against anything. Moot under finding 1.
- **Other numbers:**
  - the 40-tick cap: finding 6;
  - the 1 ms extract: finding 18;
  - 100 ns and 50–200 µs: finding 23;
  - 30 s: finding 29.
- **The protocol hash is too narrow.** "Schema hash + `net` version" lets builds with different gameplay code, Rapier, glamx or libm versions, or different cooked sim data, connect and silently mispredict.
- **Change:**
  - **Add a boundary-build ID:** Cargo.lock entries plus the source tree of boundary crates.
  - **Add a hash of sim-relevant cooked data.**
  - **Server-coordinated hot reload** of sim assets in multiplayer.

## Still to verify
- Whether Rapier ≥0.35's single active set makes results depend on unrelated awake bodies. Test: the same scene with and without a distant awake body, compare hashes.
- How KosmicKrisp lowers multi-draw-indirect.
- The per-dirty-row extract cost (M2 measurement).
- Whether Rapier, parry or glamx call `wide::mul_add` on solver paths.

## Sources
- [Rapier CHANGELOG](https://github.com/dimforge/rapier/blob/master/CHANGELOG.md)
- [rapier3d-f64 Cargo.toml](https://github.com/dimforge/rapier/blob/master/crates/rapier3d-f64/Cargo.toml)
- [Rapier workspace Cargo.toml (glam feature-unification note)](https://github.com/dimforge/rapier/blob/master/Cargo.toml)
- [parry3d Cargo.toml (enhanced-determinism → glamx/scalar-math, libm)](https://github.com/dimforge/parry/blob/master/crates/parry3d/Cargo.toml)
- [MoltenVK MVKCmdDraw.mm](https://github.com/KhronosGroup/MoltenVK/blob/main/MoltenVK/MoltenVK/Commands/MVKCmdDraw.mm)
- [Rust core f32 (min docs)](https://github.com/rust-lang/rust/blob/master/library/core/src/num/f32.rs)
- [Rust std f32 (powi docs)](https://github.com/rust-lang/rust/blob/master/library/std/src/num/f32.rs)
- [NaN bit patterns](https://github.com/rust-lang/rust/blob/master/library/core/src/primitive_docs.rs)
- [libm arch/mod.rs](https://github.com/rust-lang/compiler-builtins/blob/master/libm/src/math/arch/mod.rs)
- [libm x86 runtime detection](https://github.com/rust-lang/compiler-builtins/blob/master/libm/src/math/arch/x86/detect.rs)
- [wide f32x4 mul_add](https://github.com/Lokathor/wide/blob/main/src/f32x4_.rs)
- [glam sse2.rs](https://github.com/bitshifter/glam-rs/blob/main/src/sse2.rs)
- [glam neon.rs](https://github.com/bitshifter/glam-rs/blob/main/src/neon.rs)
- [Slang Metal target docs](https://github.com/shader-slang/slang/blob/master/docs/user-guide/a2-02-metal-target-specific.md)
- [MoltenVK issue 1567 (64-bit atomics)](https://github.com/KhronosGroup/MoltenVK/issues/1567)
- [Apple Metal feature sets](https://developer.apple.com/metal/feature-sets)
- [Dawn 64-bit atomics on Metal](https://dawn.googlesource.com/dawn/+/b620b2fe514a5ee2e6a2d924fd998c514019da87)
- [LunarG KosmicKrisp conformance](https://www.lunarg.com/lunarg-achieves-vulkan-1-3-conformance-with-kosmickrisp-on-apple-silicon/)
- [Phoronix on KosmicKrisp](https://www.phoronix.com/news/KosmicKrisp-Vulkan-Metal)
