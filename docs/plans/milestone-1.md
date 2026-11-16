# Milestone 1 plan: World

Status: approved by the owner on 14 November 2026 (D139). Written at main `3264406`, architecture revision 3.2.
This plan maps each milestone 1 criterion of architecture section 22, and every
`docs/verification.md` row that gates milestone 1, to a lane, the coordinator or the owner. It
orders the lanes that remain and takes the decisions that order needs. It proposes two
`docs/decisions-log.md` entries and the verification row text that goes with them (section 6). It
amends no architecture sentence: revision 3.3 is a separate pass, and section 8 lists what it must
settle. The findings of its first review are applied, and the owner's answers of 14 November 2026
are written in (section 5).

Conventions:

- A verification row is cited by its line number in `docs/verification.md` at `3264406` and by its
  opening words, since line numbers move.
- Finding ids such as S1 or X1 are those of the whole-project review of 14 November 2026 at
  `6ed8461`. Each is glossed where it is first used. Its D-series is always written "finding D1",
  since the decisions log has a D1 of its own.
- This plan re-checked in the tree the findings it builds a decision on: S1, X1, C1, C2, rows 68,
  70 and 82, the unwired Haswell run and the lock's glam versions. The review's other findings are
  cited as its verifier confirmed them, and the lanes that fix them re-prove them.
- A name in code font such as `ecs-fork` is this plan's handle for a lane (section 3). It is not a
  crate name.
- Cost is counted in agent runs. An implementation lane is seven: the lane, its review, about two
  fix rounds with a diff review each, and the merge. A second reviewer beside the first review
  adds one, and a third fix round this plan expects adds two. A small lane with one fix round is
  five. A facade is six: the architect, the review, a fix, a diff review, the last fix and the
  merge. A text-only lane, or a probe that delivers a result file, is four. Lanes so far took two
  to five review rounds, so the totals are a floor.

## 1. The criteria

Section 22 gives milestone 1 nine criteria. Status is what the tree and CI show at `3264406`.

| # | Criterion | What proves it | Status at `3264406` | Closed by |
|---|---|---|---|---|
| 1 | 100k entities simulate a tick in at most 4 ms on 8 performance cores | `cost_of_a_tick_at_100k_rows` in `crates/sim/tests/cost.rs`: 100,064 rows, four archetypes, sixteen regions, eight kernels and two exclusive systems, the median of 60 ticks, the world hash excluded. Result file `tick-100k` | Not met, and the test is not on main: it lands with ECS hot paths part A (proposed D137, in review). That lane measured 41.3 ms on four frame workers and 40.8 ms on one (Apple M1, load about 7), from 94.1 ms. Every system runs as one job, so the worker count changes nothing | `ecs-a`, `ecs-kernel`, `ecs-fork`; proven by session B's run on eight performance cores (section 4) |
| 2 | SOUR passes in workers, repeat, layout and chaos modes | `cargo xtask sour run --nightly` on linux-x64, windows-x64 and macos-arm64, the nightly run's id recorded in row 122 | Met in nightly 43703297881 at `c50b688`, the first that holds `sour-evidence` and `ecs-fork`: every mode passed on linux-x64, windows-x64 and macos-arm64 (on linux-x64 five modes, ten runs of 1,206 tests), and its architecture job found the three platforms' 10,334 hash lines equal. `sour-evidence` made the assembly's test and `lactose-predict`'s report the world's layout and the executor's chaos seed as applied (X1); `ecs-fork` forks each system's units over workers (row 74); `lactose-ecs` is a `[harness]` package. CI run 43556574172 passed the pull-request modes at `3264406` | `sour-evidence`, `ecs-fork`, then the first nightly that holds both |
| 3 | Physics snapshot conformance | `crates/physics-rapier/tests/conformance.rs` over `physics_api::conformance::snapshot_restore`: ten ticks, a snapshot at three, a restore, equal hashes at ten, on solver pools of one, two and four threads and two layouts | Met inside one process on every CI platform (run 43556574172), and across them: nightly 43703297881 at `c50b688` found the conformance run's world hashes and its physics bytes' hash equal on linux-x64, windows-x64 and macos-arm64 (M3) | `sour-evidence` records them; the first nightly that compares them closes it |
| 4 | Physics phase at most 3 ms for 2,000 awake bodies | A new cost test: the physics stage of `Simulation::step` (`physics-api`'s sync, the Rapier step, the write-back) at 100k rows with 2,000 awake bodies. Result file `physics-phase` | Not measured as a phase. Its parts, on the M1 at load 3.5 to 5 (row 87): the step 0.97 to 0.99 ms on four solver threads and 1.24 to 1.27 ms on one, the sync 0.96 ms, the write-back 0.42 ms. The sum is 2.35 to 2.65 ms | `measure`; `ecs-kernel` takes the column scans (0.75 ms of the sync) down |
| 5 | Prediction-world snapshot at most 100 µs | A new cost test: the take of one ring slot (`World::snapshot` plus `physics_api::snapshot`) through `lactose-predict` with Rapier, on the spike's scene. Result file `prediction-slot` | Not measured as a slot. Its parts: the world snapshot 5.4 µs with no statics (row 17); Rapier's write 32 µs beside no fixed collider and 92 µs beside 100 (row 87). The quiet window measured the slot at 140.8 µs: not met (D143, `docs/plans/criterion-5-proposal.md`); the second quiet window, at `c50b688`, measured the copied slot at 90.0 µs: met | `measure`; `slot-copy`, then the second quiet window (section 3); decision 5.5 as D143 amends it |
| 6 | Sim-skeleton evaluation at most 0.5 ms for 200 characters | `the_sim_skeleton_budget` in `crates/anim/tests/cost.rs`, its figure (b). Result file `sim-skeleton` | Met on the M1: 196.9 µs on one worker and 189.3 µs on four, at load 3 to 4 (row 115). No result file, and no x86 figure | The quiet window and session B (section 4) |
| 7 | Netcode spike over a throwaway transport: x86 server, aarch64 client, four scenarios, the correction rate, the largest absorbed difference per kind, rollback cost per tick and divergence recorded | `crates/app/tests/spike.rs` made to run (decision 5.3): its scenes recorded under SOUR, its figures in result file `spike-net`, which `cargo xtask measure spike-net` writes from the eight ten-minute replays | The spike runs and records under SOUR; `cargo xtask measure spike-net` writes result file `spike-net` (lane `spike-net-file`), checked in from the second quiet window at `c50b688`. Nightly 43703297881 at `c50b688` found the four recorded scenes' 7,200 hash lines equal on linux-x64, windows-x64 and macos-arm64, which stands for the x86 server and the aarch64 client (decision 5.3, as answered): met | `client-fix`, `sour-evidence`, `spike-net` |
| 8 | Scalar decided by the section 16 rule | A precision probe's result file `scalar-precision` and a decision entry | Met: 64-bit stays (D146). On upstream 32-bit Rapier the walk scene translated to x = 6 km differs from the origin run by 157.75 mm, and by 120.84 mm at 12 km, against one millimetre; the 64-bit control stays at or under 0.032 mm (`docs/results/scalar-precision-apple-m1-2026-11-14.md`). The rule is read at x = 6 km, D59's figure, since no document states the map's extent. The SOUR half is moot and the 32-bit branch is not built | `scalar-probe`, then the planner's decision (decision 5.4, `docs/plans/scalar-decision.md`) |
| 9 | Edit-to-playing at most 30 s per platform | The measurement section 4 defines. Result file `edit-to-playing` | Not measured and not defined. The dev profile and the linkers are configured (D125, row 63). No game crate exists | `measure`, the quiet window, session B |

Of what section 22 says milestone 1 delivers, the simulation track's crates are merged, with
`lactose-ecs` still the naive single-job runner. `lactose-io` is merged. `lactose-asset` is a
merged facade whose implementation is in flight. `tools/cook` cooks a sim skeleton from glTF into
one pack (`ricotta`, D147). The RHI spike ran under MoltenVK on the M1
(`docs/results/spike-rhi-apple-m1-2026-11-15.md`); `lactose-rhi-null` is not started.

## 2. The verification rows

Every row whose gate names milestone 1 has an owner below, or is re-gated with its reason. "Pass"
is the coordinator's verification pass: one edit of `docs/verification.md` after the `ecs-a` merge
that marks closed rows Done with the run or file that closed them and cites rows by their opening
words. Section 6 gives the exact text of every row this plan changes.

| Row | Opening words | Owner | What closes it |
|---|---|---|---|
| 14 | Scalar precision | `scalar-probe`, then the planner | Closed: the probe's result file and the scalar decision entry, D146. 64-bit stays |
| 17 | Prediction-world snapshot cost | `measure`, `spike-net` | The slot, proxies included, measured through `lactose-predict` with Rapier; decision 5.5 settles copy-on-write |
| 20 | Physics phase stall | `measure` for the share; re-gated for the stall | Split in two. The phase's share of the tick is measured at milestone 1 on the composed tick. The stall on render recording moves to milestone 2: nothing records a frame before `render-graph` exists |
| 31 | Whether Rapier, parry or its math crates call `wide::mul_add` | `sour-evidence` | A reading of the pinned sources. This plan's search found no call site in `rapier3d-f64` 0.36.0, `parry3d-f64` 0.31.1, `nalgebra` 0.35.0 or `glamx` 0.3.1, and twenty mentions in `simba` 0.10.2 that the lane classifies |
| 40 | Rapier's serialised snapshot is iteration-order stable | `sour-evidence` | The physics bytes' hash recorded under SOUR and equal on three platforms in a nightly |
| 41 | Rapier's 64-bit build uses SIMD | `scalar-probe` | Closed: a reading of the vendored source and one disassembly. The contact solver runs two 64-bit lanes per NEON instruction against 32 bits' four, and four per AVX instruction on x86-64-v3. It is the cost half of the scalar decision, which D146 records beside the measured 64-bit rollback loop; no 32-bit loop is owed |
| 42 | `std::fs::File::sync_all` issues `F_FULLFSYNC` | Pass, then `data-path` | The pass records the Windows CI run of `crates/io`'s tests. `data-path` reads the pinned toolchain's std for the two Windows calls. Durability after a power loss is not measurable here and is stated as the platform's contract (row text in section 6) |
| 43 | Directory fsync after a rename | Re-gated to 4 | macOS is measured. The file systems still open are deployment targets, and section 22.1 gives the disk-full and corrupt-save paths to milestone 4 |
| 63 | `lld-link` on Windows | `measure`, the quiet window, session B | The edit-to-playing result files |
| 64 | The first CI runs after the lld-link change | Pass | The trend of the Windows jobs over the runs since |
| 65 | Assets of runtime-spawned entities | Re-gated to 3 | Decision 5.8 |
| 66 | Enforcing that the loader of a simulation-read asset kind | `xtask-policy` | `cargo xtask boundary` requires every `impl SimulationAsset` outside tests to sit in a boundary crate, as `lactose-anim`'s three do |
| 68 | `cargo xtask features` should fail if any normal or build edge | Pass | Closed by code: `[dev_only_features]` in `crate-tiers.toml`, enforced in `tools/xtask/src/tiers.rs` |
| 70 | `cargo xtask boundary` should check that `rustdocflags` | `xtask-policy` | A check with a failing fixture. `boundary.rs` reads `rustflags` only today |
| 72 | The ECS facade | `spike-net` | The predicted-key rule under lost and substituted inputs and after a disconnect, on both ends |
| 73 | A region's seed and persistent region resources | Pass | Closed by D129: `RegionRecords` carries them beside the overlay, and so does the relevance message |
| 74 | ECS implementation gaps | `ecs-fork`, `silo`, `binds-design`, `binds-impl` | `ecs-fork` makes workers and chaos modes mean something. `silo` tests `Ctx::assets` through a pinned table. D122 settled the correction path (corrections are written stamped, section 9.4). `binds-design` and `binds-impl` own the world resources on bind or restore; `ecs-fork` owns the finalize's error and the 2^31 clamp end to end, or re-gates them with a reason |
| 75 | Two section 5.2 and 9.7 features | Re-gated to 4 and 5 | Decimation is milestone 4's. `ordered` and per-region reductions have no consumer before game systems. `ecs-fork` reports that its merge of partials leaves `ordered` additive |
| 82 | A CI grep stops any crate but `lactose-task` naming loom | `xtask-policy` | The rule inside `cargo xtask features`; the workflow step goes |
| 83 | `lactose-sim` (D118) is implemented | `ecs-a`, `binds-impl`; the rest re-gated to 3 | The `ecs-a` merge records the hash's cost. `binds-impl` adds `Debug`. The single-pass hash, the frame-level region request and the late-insert check are milestone 3's |
| 86 | `lactose-physics-api` (D119) is implemented | `sour-evidence`, `spike-net` | The conformance scene, then the spike's contact scenes, equal on three platforms in the nightlies |
| 87 | Physics costs on the milestone 1 ECS | `measure`, `ecs-kernel` | The whole phase measured; `changed_since` skips chunks whose column tick predates the mark |
| 88 | The physics snapshot contract | `sour-evidence`, D145 | The recorded physics hash. The reading of Rapier's deserialisation is `crates/physics-rapier/README.md`, "The snapshot bytes", re-read at `b2e2a41` with the tests that hold it (`docs/plans/physics-evidence-proposal.md`, section 2) |
| new | The further gaps of the `lactose-physics-api` (D119) row | D145 | The trace of the platform library's callers over the nine recorded Rapier scenes: `log2` alone, accepted by arithmetic; the other sites unreachable by what the backend builds, the eigen decomposition re-gated to milestone 3 (`docs/plans/physics-evidence-proposal.md`, section 1) |
| new | The physics hash beside the world hash (D145) | `physics-records` | Its streams equal in every SOUR mode on the M1, then on three platforms in the first nightly that holds it |
| 89 | `World::query` visits every live entity | `ecs-a` | Its merge |
| 90 | `lactose-input` is implemented | Pass | The x86-64 confirmations are CI's. The world-frame check against the camera rig stays at milestone 2 |
| 93 | `lactose-net-api` is implemented | `sour-evidence`, `spike-net`, the scalar decision | The `ProtocolHash` pinned over `Game::types()` (C3). The spike measures the gather path. The quantiser bounds follow the scalar, which stays 64-bit (D146): a position round-trips exactly for every quantised value, a 32-bit scalar's bound of 2^24 units does not apply, and the rotation quantiser's figures stand as measured |
| 94 | `lactose-predict` is implemented | `spike-net`, `measure` | The slot with Rapier and the absorbed figures. Milestone 1 records no derived ring depth: `app` builds at `DEFAULT_DEPTH` (D146), and the depth moves to the verification row "The frame re-simulation budget has no figure", gated at milestone 4. How a re-spawned body presents is milestone 2's (row text in section 6) |
| 99 | `World::insert` of a component the row already holds | `ecs-a` | Its merge |
| 101 | `cargo-deny` 0.20.2 skips a path source | Workflow pass | The workflow pins `cargo-deny`. The row becomes a standing check at each change of the pin, gate none |
| 102 | The `sin_cos` patch is not upstream | Already reported upstream; the coordinator links the reports | The links in `vendor/README.md` |
| 105 | The reconciliation tolerance (D124) | `spike-net`, the scalar decision | The spike's figures under the defaults; the defaults are re-set only if the scalar changes, and it does not (D146). Milestone 1's part is closed. The re-set from the spike's figures is milestone 4's, after the sleep-state rule for claimed bodies |
| 106 | An empty solver is a fresh solver (D124) | Pass, `measure` | x86-64 is CI's nextest of `crates/physics-rapier/tests/reload.rs`. `measure` records the full diff's cost on a 100k-entity reload |
| 108 | Rapier step never blocks a pool thread | Pass | Closed by D123 |
| 109 | Every channel is cleared at a run's closing sync point | `spike-net` | A carrier column reconciled against a server |
| 110 | The save's cost at the milestone 1 scale | Pass | Closed as measured. No criterion names it and no x86 run is owed |
| 112 | `SchemaTable`'s schema derivation | `schema-digest` | `lactose-ecs` exports `document_schema`; `lactose-bundle` exports `write_table` and `read_table` |
| 113 | A layout change without a version bump | `schema-digest`, `schema-guard` | The digest in `ComponentSchema`; the committed version-and-digest record |
| 114 | A persistent region resource over 16 KiB | `schema-digest` | Registration refuses it |
| 115 | Sim-skeleton evaluation | The quiet window, session B | Result files |
| 116 | Bodies' cost per tick | `measure` | A Rapier run, recorded outside the criterion |
| 117 | A dynamic character's yaw | `spike-net` | Measured on the spike's capsule |
| 122 | The `app` implementation (D131) | Pass | The first nightly at or after the `sour-evidence` merge |
| 125 | The headless binary's 120-tick run | The quiet window, session B | The hash on x86-64 equal to the one the quiet window printed on the M1 at the same commit |
| 132 | Mount cost (D136) | `silo` | Recorded with the implementation |
| 133 | The cooked pack set of a fresh process | `ricotta`; milestone 2's watch | Milestone 1 half closed by D147 (`crates/asset/tests/pack_set.rs`); the watch's half when milestone 2 lands |
| new | A milestone 1 tick at 100k entities (added by the `ecs-a` merge) | `measure`, the quiet window, session B | Its last sentence becomes "Measured on eight performance cores in session B; the M1's projection of decision 5.1 steers" |

Rows 32, 84, 85 and 123 are closed. Row 36's remaining check, which glam Rapier selects, is closed:
`Cargo.lock` gives `glamx` 0.3.1 its `glam` 0.33.12, and `deny.toml` bans `fast-math` and
`core-simd`. Row 29 names glam 0.34.0, which the lock does not hold; the pass corrects it.

Milestone 0 left seven rows without a Done mark. The pass closes row 12 (io_uring on CI runners)
from the nightly probe and row 69 (cross-architecture agreement of `lactose-math`) from CI's x86-64
runs. Rows 76, 78, 79 and 80 are carried into milestone 1 by the first entry of section 6. Row 62
(Windows efficiency-core steering) is re-gated to milestone 2.

## 3. The lanes

Three lanes are in flight at `3264406`. Their costs are what remains.

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `ecs-a` | ECS hot paths part A (proposed D137): queries by archetype, an allocation-free overwrite, the event index, no allocation per row | In review | `crates/sim/tests/cost.rs` on main; rows 89 and 99 closed by counts, not timing | 2 |
| `client-fix` | The client fix round: a prediction world's regions are `Simulated` (S1); the resync target after a stall (S2); the lifecycle's outputs taken (S5); the solver pool's start handler (C1, the `app` half) | In flight | In `crates/app/tests/proof/client.rs`: a kernel advanced a claimed row after a folded full frame; one resync after several queued frames; a refused bundle reported once | 6 |
| `silo` | `lactose-asset` implemented against its facade (D136) | In flight; the concurrency reviewer reads beside its first review | The README's proof table; `asset-api`'s conformance suite over raw and zstd packs; row 132; `Ctx::assets`; the README sentence of decision 5.8 | 7 |

The lanes that follow, in the order their dependencies allow:

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `sour-evidence` | `applied` built from the world's layout and the executor's chaos configuration in the assembly's and `lactose-predict`'s tests (X1). A recording test for the conformance scene: the world hash each tick and the physics bytes' hash at the end (M3). `ProtocolHash` pinned over `Game::types()` of an empty game (C3). The reading for row 31 | `client-fix` merged | A world that ignores its layout fails layout mode. Rows 40, 86, 88 and 122 rewritten from the first nightly after the merge | 7 |
| `ecs-kernel` | ECS part B, first half: the per-chunk kernel path of the `#[system]` expansion, and `changed_since` skipping chunks by column tick | `ecs-a` merged; the concurrency reviewer reads beside its first review if it adds `unsafe` or changes `SystemDescriptor::new`'s obligation (one more run) | No public signature changes. Counts hold it: allocations, rows visited, chunks skipped. Miri. SOUR in the pull-request, layout and chaos modes. The cost test before and after, with load. Steering figure: the one-core tick at 100k rows at most 21 ms | 7, or 8 |
| `ecs-fork` | ECS part B, second half: each system a graph node whose units are chunks (decision 5.2). The debug assertion of the floating-point state at every job (C1, the ECS half). `lactose-ecs` joins `[harness]`. The cost scene, scaled down, recorded under SOUR | `ecs-kernel` merged; the concurrency reviewer reads beside its first review | Every SOUR mode. A merge of outputs in completion order fails chaos mode. Miri. The cost test on one worker and on every performance core. A finalize's `InvalidParameter` names its type and the 2^31 clamp is tested end to end, or each is re-gated with a reason (row 74) | 10 |
| `stream-design` | Architect: what a client world gets from the stream. The `OwnedBy` cascade on a client (S3) and transient presence on the wire (S4). Corrects D133 | `client-fix` merged | A facade and decision text; row 96 widened | 6 |
| `spike-net` | The netcode spike (decision 5.3) | `client-fix` and `sour-evidence` merged | Criterion 7; rows 72, 93, 94, 105, 109 and 117 | 9 |
| `stream-impl` | Implements `stream-design` in `lactose-net-api`, `lactose-predict`, `lactose-ecs` and `lactose-anim` | `stream-design`, `spike-net` and `ecs-fork` merged | The review's two probes as tests: a released item survives its former owner's tombstone on the client; a claimed spawn carries its transient components | 7 |
| `xtask-policy` | The backend's allowlist cut to its manifest, and trait routes to the platform library banned (X2). The whole lint file pinned (X3). `[harness]` and the recording tests tied to the tree (X4). The unsafe list compared with section 21 (X5). Rows 66, 70 and 82 | None | Each rule has a fixture that fails: a deleted ban, a dropped harness package, a swapped unsafe crate, a deleted `rustdocflags` | 7 |
| `measure` | `cargo xtask measure` and the result-file convention (decision 5.11). The missing cost tests: the physics phase through `Simulation::step` with Rapier, a prediction slot (the character, fifty claimed bodies, fixed colliders and proxies) and a seven-tick rollback through `lactose-predict` with Rapier, the composed tick, the full diff of a 100k reload, bone bodies on Rapier. The edit-to-playing measurement, both edits (decision 5.9). The 100k test run on one frame worker and on N, the machine's performance cores up to eight, in one sitting, printing T₁, T_N and the projection of decision 5.1 | `ecs-a` and `xtask-policy` merged | One command writes every result file on the M1, each with the header section 4 names, and a short run of `cargo xtask measure --all` on the linux-x64 and windows-x64 runners in CI, its numbers discarded but for the edit-to-playing figure decision 5.9 keeps, so that session B's command is known to finish. Times are printed, never asserted | 7 |
| `schema-digest` | The D129 additions: `document_schema` exported, `write_table` and `read_table` exported, the layout digest in `ComponentSchema` (a bundle format bump) and compared by `World::load_region`, a non-derived resource over 16 KiB refused at registration | `silo` and `ecs-fork` merged | Rows 112, 113 and 114. A same-size layout change without a version bump is refused at a region load | 7 |
| `ricotta` | The RICOTTA skeleton in `tools/cook`: inputs, processors behind `asset-api`'s traits, one pack out, the cooked pack set's naming rule (D147). One kind cooked end to end, chosen in its brief: `lactose-anim`'s kinds have cooked formats today, and no authored scene format exists for a baseline | `schema-digest` merged | A pack `ricotta` wrote mounts in SILO, and its assets load and pin. Cooking the same inputs twice writes the same bytes. Row 133's milestone 1 half | 7 |
| `schema-guard` | A committed record of each saved type's version and layout digest, and the test that fails on a digest changed without a version bump and on a field ID reused | `schema-digest` merged | Section 15's "CI rejects ID reuse" becomes true: both mutations fail | 5 |
| `scalar-probe` | The precision probe of decision 5.4, and the readings for row 41 | None | Result file `scalar-precision` | 4 |
| `spike-rhi` | The RHI spike (decision 5.10), under MoltenVK on the M1 | The GPU reviewer defined (`.claude/agents/reviewer-gpu.md`); the Vulkan SDK on the development Mac (1.4.363.1) | Result file `spike-rhi`, each "no" marked as MoltenVK's or as a design question | 4 |
| `rhi-api` | Architect: the facade of `lactose-rhi-api`, unstable until the native Metal spike (D43), with its conformance suite's shape | `spike-rhi` merged; the owner's ruling on the move to milestone 2 (decision 5.10) | A facade; the reviewer and the GPU reviewer both read it | 7 |
| `rhi-null` | `lactose-rhi-null` against the facade | `rhi-api` merged; revision 3.3's ruling on whether the server links it (C2) | The conformance suite passes on the null backend; it stays test support (`crate-tiers.toml`) | 7 |
| `data-path` | A container's sections checked against each other and its header (finding D3). A zero-size field refused at registration (finding D4). The migrate, rebase and bundle sequence tested for a type the baseline carries (finding D5). The reading for row 42 | None | The review's three probes as tests | 7 |
| `binds-design` | Architect: binds restored or carried across a save, the journal checkpoint included (finding D1), and what a bind or a restore does to reflected world resources (row 74). Corrects D133 | `data-path` merged | A facade and decision text | 6 |
| `binds-impl` | Implements `binds-design` in `lactose-persist`, `lactose-sim`, `lactose-ecs` and `lactose-app` | `binds-design` and `ecs-fork` merged | A fresh process that leaves a predicted owner's region unloaded hashes equal to the original, beside section 15.5's tests; the `NetId` counter above every runtime ID a world is given (the row "A `Server` world's direct spawn under a runtime `NetId`"); a world resource that holds an entity reference refused where it is inserted (the row "ECS implementation gaps after its reviews (D117)"); a server holding no bind of an entity despawned in a step between steps | 7 |
| `keeper` | One bounded attempt at the tick-start criterion (section 6, first entry), and the wall-clock log of samples over 1 ms (row 78) | Session A's numbers; `timer-windows` merged, since both edit `crates/task/src/sleep.rs`. The Windows timed wait is not this lane's (D144) | Result file `whisk` from a quiet M1, or the cause stated | 4 to 8, counted as 6 |
| `readme-sweep` | Text only: README sentences that are stale, misattribute the `net-api` and `net-host` split, wait for crates that have merged, or name a crate by a codename (the review's C4, C6, C8, C9 a and c, and S6) | `stream-design` decided, for the one sentence that cites it | Every corrected sentence names what it now agrees with | 4 |
| `slot-copy` | The prediction slot holds the backend's world as a copy (D143). `lactose-physics-api`: the trait `BackendCopy` with its one method `encode`; `PhysicsBackend::copy` and `PhysicsBackend::restore_copy`, with default bodies through `snapshot` and `restore`; `PhysicsSnapshot` holds the copy, and `PhysicsSnapshot::size_in_bytes` and `PhysicsSnapshot::hash` encode it once on their first call and keep the length and the hash; the doc comments of `PhysicsBackend::snapshot`, of the free `snapshot` and `restore`, and of `ConformanceReport::snapshot_bytes`. No bytes accessor. `lactose-physics-rapier`: the copy is a clone of every set and of the bookkeeping, built by destructuring `PhysicsWorld` with no `..`, shapes shared, and refused for a shape the backend did not build; a restore from it builds a fresh pipeline. `lactose-predict`: `SlotBytes`, `Ring::slot_bytes` and `Ring::bytes` report the bytes the backend would write, not what a slot holds, and their doc comments say so and state their cost. `crates/app/tests/spike.rs`: `cost_of_a_prediction_slot` takes into a full ring. The interface amendment is called out in its merge (section 20). The three READMEs' slot sentences | `stream-impl` merged, which rewrites `crates/predict/README.md` | `physics-api`'s conformance run through the copy on every pool size and layout, with the bytes of a copy equal to the backend's bytes after each of its ten ticks. The first review's backend probe adopted as a test under a name that says what it proves: joints of every kind made and removed, sleeping bodies woken, a shape replaced while older copies share the old one, on pools of one, two and four threads; from every restore point twelve ticks agree in bytes and reports between the copy, the bytes and the run itself; it fails when the restore keeps the live pipeline. The refusal at a take, as two tests, since no public call can put a foreign shape in a Rapier world: a unit test of `lactose-physics-rapier` that puts one into the crate-private collider set and sees the take refused, and `lactose-predict`'s take through its `Flaky` test backend, the slot taken without its physics. The spike's four recording tests record the hashes they recorded before the change, in the pull-request, layout and chaos modes. `cargo xtask measure prediction-slot rollback-seven-ticks` before and after, with load, as steering. The ring's memory per slot, counted in a test under `lactose-core`'s tagged allocator, since no accessor reports it | 7 |
| `timer-windows` | The keeper's timed wait on Windows (D144), in `lactose-task` alone. The `sync` shim gains `TimedPark`, one worker's timed wait. On Windows, outside loom and Miri, it holds an auto-reset event and a high-resolution waitable timer that is not manual-reset; `wait` sets the timer before every wait and waits on both; `wake` sets the event. Elsewhere `wait` is `std::thread::park_timeout` and `wake` does nothing. Under loom it is an `AtomicBool` that `wake` sets with `swap(true, SeqCst)` and `wait` takes with `swap(false, SeqCst)`, parking while it was false; a plain store and a flag behind a loom mutex do not serve. The frame-critical pool's `Sleep` holds one for each worker: the pool makes them when it is built, before a worker enters its loop, and closes them in its `Drop`, after the last handle to the executor and the last worker. `Sleep::wake_claimed` wakes a claimed worker's `TimedPark` and unparks it; it takes the unpark as a closure, or is generic over the thread handle, so the pool's std threads and the models' loom threads share it. It is called only after a successful claim, by `Pool::announce_entry`, by `Pool::wake` when `Sleep::notify` returned true and by `Pool::wake_all`; `Pool::wake`'s unpark of a worker it did not claim stays an unpark. The models' waker sides call the same function. A worker whose event or timer cannot be made keeps `std::thread::park_timeout`, logged once; a failed `SetWaitableTimer` or a `WAIT_FAILED` is logged once and that wait is a `park_timeout`. `unsafe impl Send` and `Sync` for `TimedPark`, with its `SAFETY.md` section. The spin threshold is 500 µs on Windows and 50 µs elsewhere, set in the lane's last commit, with the commit before it holding 50 µs on Windows; the step stays 0.6. `crates/task/Cargo.toml` adds `Win32_System_Threading` and `Win32_Security` to its `windows-sys` features, and `Win32_System_WindowsProgramming` as a dev-dependency feature for the benchmark. The README's sentences; the benchmark's timer threshold on Windows, 1 ms, and its process CPU column on Windows, from `QueryProcessCycleTime` | None | In `check (windows-latest)`, a debug nextest run: the median lateness of fifty 2 ms deadlines through `sleep_until` on an otherwise idle pool is under 7.8 ms, half a default tick, where today's code gives about 14 ms and the fix under 1 ms, with that reasoning in the test; on a one-worker frame pool a keeper waiting on a deadline an hour away is ended by an earlier timer, by pushed work and by shutdown, each inside `LIMIT`, and with the event unset the keeper sleeps 36 minutes and the test times out. Under loom the fourteen models pass, and the keeper hand-off model fails when a waker of it unparks without `Sleep::wake_claimed` and when `wake_claimed` does not set the flag. Miri passes unchanged. The Windows cross-target clippy and `cargo xtask features` pass. The merge is pushed alone (D125). After the merge is pushed, the laptop runs the benchmark at the merge's first parent, at the lane's 50 µs commit and at the merge, as labelled data: the timer rows with their process CPU, and the makespan and wake rows beside them. If the merge's process CPU at 60 Hz is more than 0.015 s a second above the 50 µs commit's there, or its P99 is not under 1 ms, a follow-up commit on main sets the Windows threshold back to 50 µs and the benchmark's to 2 ms, recorded as D144's addendum | 8: seven, and the concurrency reviewer beside its first review |
| `physics-records` | The physics hash recorded beside the world hash (D145), in tests alone. Each recording test that steps Rapier opens a second recorder, named as the test with `::physics` appended, and records after every tick `lactose_physics_api::snapshot(world)?.hash()` of each world it steps: `lactose-physics-rapier`'s `a_server_scene`, `a_prediction_scene_with_a_rollback`, its re-simulated ticks again, and `a_limited_hinge_driven_against_its_limits`; `lactose-app`'s four spike scenes, the server's world and the client's prediction world as one value per frame, and `a_recorded_headless_run`. `the_conformance_run` keeps its one value. The ten-minute replays compute no physics hash. Each `::physics` name opens its own recorder and, under a chaos seed, writes the test's execution log under its own name. `tools/xtask/ci-policy.toml` lists the eight names under `[harness]`. One test in `crates/physics-rapier/tests/backend.rs`: a body with a `RigidBody` and no `Collider` that gains an offset, tilted collider reports, from that tick on, the states of a body created with it. The body is pushed (an impulse and a torque impulse) in the tick it gains the collider and lands on a floor, so that a dropped closing rebuild fails it as a kept sum does. The sentences of `crates/physics-api/README.md`, `crates/sim/README.md` and the doc comment of `PhysicsSnapshot::hash` that call it the place where the bytes join the world hash | No lane in flight edits `crates/app/tests/spike.rs` or the `[harness]` list | `cargo xtask sour run --nightly` on the M1: every new stream equal in every mode, and the prediction scene's re-simulated physics hashes equal to its first run's. Every line the existing names record is byte-identical to main's at the lane's base, the spike's digest over its four world streams, b6a4efa4db9780a4, among them. One mutation of solver state that moves no row at the tick it is made fails a physics stream at that tick; the lane picks it and says why the world stream cannot see it then. `cargo xtask sour run --pr` fails when a listed physics stream records nothing. `cargo xtask measure spike-net --short` before and after, as steering. The test time the new streams add to one SOUR launch, about one second by the first review's measure. Then the first nightly that holds the lane: equal on three platforms, or a finding for the planner | 5 |

Document work beside the tracks: this plan's review and answer (2 runs); revision 3.3 (3); the
scalar decision (3); the definition of the GPU reviewer and its review (2); the exit of section 7
(8). The coordinator's verification pass and its workflow pass are its own work.

The whole is about 165 runs at two fix rounds a lane, and about 200 if a third of the lanes take
four or five: 15 to finish the lanes in flight, 131 for the lanes above and 18 for the document
work. Document rounds have run long too, and this plan's answer and revision 3.3's eighteen items
are counted low. If the owner moves `rhi-api` and `rhi-null` to milestone 2 (decision 5.10),
fourteen of these runs leave milestone 1.

D143 adds `slot-copy`, seven runs, and five for its proposal's two reviews, two revisions
and its application.

D144 adds `timer-windows`, eight runs, and six for its proposal: its writing, two reviews, two
revisions and its application.

D145 adds `physics-records`, five runs, and four for its proposal: its writing, a review, a revision
and its application.

Three lanes need a facade first: `stream-impl`, `binds-impl` and `rhi-null`. No other lane does.
`lactose-ecs`'s surface is final but for `schema-digest`'s export of `document_schema`,
`tools/cook` is a leaf that writes SILO's format, and the spike lives in `lactose-app`'s tests.

**Order.** Three tracks run at once, with the document work beside them. Each default queue below
is in dependency order, and the coordinator refills a free slot from any queue whose head is
unblocked.

| Queue | Lanes, in order | Runs |
|---|---|---|
| 1, the ECS and then render | `scalar-probe`, `ecs-a`, `ecs-kernel`, `ecs-fork`, `spike-rhi`, `rhi-api`, `rhi-null`, `timer-windows`, `keeper`, `readme-sweep` | 59 |
| 2, the client and then the save path | `client-fix`, `sour-evidence`, `spike-net`, `stream-impl`, `slot-copy`, `data-path`, `binds-design`, `binds-impl`, `physics-records` | 61 |
| 3, assets and tools | `silo`, `stream-design`, `xtask-policy`, `measure`, `schema-digest`, `ricotta`, `schema-guard` | 46 |

Seven rules bind whatever the coordinator reorders:

- One lane at a time edits `lactose-ecs`. `schema-digest`, `stream-impl` and `binds-impl` all edit
  it, and each starts after `ecs-fork` has merged. An architect round that adds a signature to it
  merges between two ECS lanes, never during one.
- One lane at a time edits `tools/xtask/src`: `xtask-policy`, then `measure`. Edits to
  `ci-policy.toml` are ordered by merge.
- `scalar-probe` starts in the first slot that frees. It needs nothing and edits nothing in the
  workspace, and its result comes before any figure it could void (decision 5.4).
- The measurement commit is the first commit of main that holds `ecs-fork`, `measure`, `spike-net`
  and `stream-impl`. The quiet window and session B run there (section 4). The render lanes and
  the save-path lanes need not have merged by then. If `stream-impl` is the only one missing, the
  commit does not wait for it: the spike's result file names the gap, and the spike is measured
  again in a second quiet window after it merges.
- A lane that merges after the measurement commit and edits a measured crate (`binds-impl`,
  `schema-digest`, `stream-impl` if it is late) runs `cargo xtask measure` for the criteria it
  touches and reports the figures as steering. The exit's reviewer compares them with the measured
  files.
- Criterion 5 is proven at the first commit of main that holds `slot-copy`, not at the measurement
  commit. The coordinator runs
  `cargo xtask measure prediction-slot rollback-seven-ticks spike-net` there in a second quiet
  window; the first two files replace the measurement commit's for criteria 5 and 7, and the third
  is criterion 7's first `spike-net` file. The cost test takes into a full ring by then, so the
  snapshot's figure includes the drop of the world snapshot and the physics snapshot it evicts.
  Every other file of the measurement commit stands, and session B is unchanged: its slot figure
  is recorded, of the byte form (D143).
- On Windows, milestone 0's `whisk` file is written at the first commit of main that holds
  `timer-windows` and, if one is made, the follow-up that sets its Windows threshold back, by
  `cargo xtask measure whisk`, not at the measurement commit. Before the lane its timer rows fail
  on any machine and its makespan rows are taken after gaps of a tick (D144). Every other file of
  session B stands at the measurement commit.

**Where this reorders the coordinator's queue, and why.**

- ECS part B is two lanes, and the kernel path comes first (decision 5.2).
- `measure` moves ahead of the spike's figures and of the last ECS merge. Every number after it is
  a file with its machine, commit and load, which is what the review's M5 found missing.
- `schema-digest` moves ahead of `ricotta`. It follows `silo` and `ecs-fork`, since it edits
  `lactose-ecs`. A bundle format bump moves every test that pins document bytes, and is cheapest
  before the cooker adds its own (decision 5.6).
- `scalar-probe` moves to the front. It costs four runs and conflicts with nothing, and a pass
  would change the scalar under every figure taken after it.
- `spike-net` starts as soon as `client-fix` and `sour-evidence` have merged. It does not wait for
  the two architect rounds. `stream-design` runs beside it and `stream-impl` follows it, so the
  figures checked in at the measurement commit come from the hooks that ship.
- `xtask-policy` moves ahead of the second architect round and of the data-path lane, because
  `measure` waits on it.
- The render lanes start when queue 1 is free of the ECS, not after everything else. They are a
  chain of eighteen runs with three outside prerequisites, and nothing else depends on them. The
  spike is four of the eighteen; the owner is asked whether the other fourteen move to milestone 2
  (decision 5.10).

**The coordinator's passes.**

- The verification pass, after the `ecs-a` merge: section 2's Done marks, the row text of section
  6, rows cited by opening words, machine and load on every milestone 1 number that has them.
- The workflow pass, pushed alone: `cargo-deny` and `cargo-nextest` pinned; the stale comments in
  `ci.yml` rewritten; a step in the nightly SOUR jobs that prints the runner's CPU model and
  feature flags (decision 5.7); the nightly Miri job split per crate. That job took 38 to 57
  minutes of its 60 in the eight green nightlies of 5 to 13 November, and `ecs-kernel` and
  `ecs-fork` add tests to it. One manually dispatched job follows its lane: `measure`'s short run
  on the linux-x64 and windows-x64 runners.
- For the owner: the commands of session A, now. Row 102 needs no report: both defects were
  already reported upstream, and `vendor/README.md` links them.

## 4. Machines, workloads and result files

The only machine the agents can measure on is the developer's Apple M1: four performance and four
efficiency cores, under agent load. The x86 desktop is the owner's. The owner has said it has
eight or more performance cores; its CPU model, its operating systems and its GPU come back with
session A's files.

**Three sittings produce every number a criterion rests on.**

- *Session A*, the owner's, now, before this plan is approved; the coordinator gives the owner the
  commands. On the x86 desktop at main's head: `cargo bench -p lactose-task --bench whisk` and
  `cargo nextest run -p lactose-task`. The owner sends back the two files the benchmark writes, the
  CPU model, the operating systems and the GPU. It closes, or fails, the x86 half of milestone 0's
  criteria, and the topology it prints confirms the eight performance cores criterion 1 is run on.
  It needs no new code. No result file exists from Windows or Linux, where CI lints the benchmark
  and does not run it; if it fails there, that failure is the session's result and goes to `keeper`,
  except the timer rows on Windows, whose cause is known and which `timer-windows` fixes (D144).
- *The quiet window*, the coordinator's, at the measurement commit. Every track stops, the machine
  holds `caffeinate -i -s`, and `cargo xtask measure --all` runs once. A number measured under
  agent load stays in a lane's report: it steers, and it proves nothing.
- *Session B*, the owner's, at the same commit. On the x86 desktop, on each operating system it
  boots: `cargo xtask measure --all`, and the spike's recording tests, whose hashes the result
  file compares with the nightly's. The owner commits the files it writes, or sends them back. Its
  run of the 100k tick on eight frame workers is criterion 1's proof.

**What each timing criterion runs, and against what.**

| Criterion | Workload | On the M1 | On the x86 desktop |
|---|---|---|---|
| 1, the 100k tick | `cost_of_a_tick_at_100k_rows`: T₁ and T_N, the median step on one frame worker and on N, in one sitting; N is the performance cores up to eight, and S is derived from the two (decision 5.1) | Steers, and proves nothing: 7·T₄ − T₁ at most 24 ms | The proof: T₈ at most 4 ms on eight frame workers, one for each of eight performance cores |
| 4, the physics phase | The new phase test: 100k rows, 2,000 awake bodies on a floor, the solver pool at `lactose-app`'s default (frame workers minus one) | At most 3 ms | At most 3 ms |
| 5, the snapshot | The new slot test: the spike's scene, one character, fifty claimed bodies, its fixed colliders and its proxies | The take with its eviction at most 100 µs; the restore recorded | Recorded; the client of criterion 7 is the aarch64 machine |
| 6, the sim skeleton | `the_sim_skeleton_budget`, figure (b) | At most 0.5 ms | At most 0.5 ms |
| 7, rollback cost per tick | The spike's measurement test | Recorded; it has no budget | Not run |
| 9, edit-to-playing | From saving a one-line change to the report line of `cargo run -p lactose-app-headless -- 1`; dev profile, warm build, the median of five. Two edits, each in a function body: the sketch game's system in `crates/app-headless/src/main.rs`, and a function of `lactose-app`. The criterion is read against the slower (decision 5.9) | At most 30 s | At most 30 s on each operating system it boots; a runner's figure, marked as such, for one it does not |
| Milestone 0, WHISK | `cargo bench -p lactose-task --bench whisk` | Its own thresholds (`crates/task/README.md`) | The same; on Windows the 60 Hz timer row against 1 ms at P99, in a file written at the first commit that holds `timer-windows` and, if one is made, the follow-up that sets its Windows threshold back (D144) |

The composed tick, the cost scene with the 2,000 awake bodies in it, is recorded beside criteria 1
and 4. It is the share of the tick section 16 asks milestone 1 to measure, and it is not a
criterion.

The headless binary's 120-tick run (row 125) is part of `measure`: the hash it prints on x86-64
must equal the one the quiet window's run printed on the M1 at the same commit. Row 125's own hash
is from an older commit and is not the reference.

**How a result is checked in.** `docs/results/<measurement>-<machine>-<date>.md`, with a `.json`
beside it where the measurement writes one. The date is UTC. Each file opens with the CPU and
operating system, the topology the executor detected, the workers used, the commit (with `-dirty`
if the tree had changes), the build profile, the command, and the load average before and after
(on Windows, the processor's idle share over the run). `cargo xtask measure` writes that header;
`docs/results/README.md` lists each measurement with its command and where its threshold is
stated. A verification row cites the file and no longer carries the number's history. The file
for criterion 1 also records the scene's parameters (rows, archetypes, regions, systems, ticks)
and the world hash after the last tick, so a later edit to the test cannot change the criterion's
workload unseen.

## 5. Decisions this plan takes

Each is a recommendation the owner approves with the plan. Each names what would change it.

**5.1 Criterion 1 is the tick without physics, and the desktop's eight performance cores prove
it.** Each criterion of section 22 keeps its own workload: the 100k tick is the scene of
`crates/sim/tests/cost.rs`, which holds no body, and the physics phase is measured on its own
scene. The world hash is outside the tick: SOUR pays it at every step, and a shipped binary pays it
only when it asks. The owner's desktop has eight or more performance cores, so the criterion is
proven as written, by a direct run in session B: T₈, the median of 60 ticks on eight frame workers,
is at most 4 ms. Eight frame workers means one worker for each of eight performance cores as the
executor counts them (architecture section 5.1). On a hybrid part those are physical cores of the
fastest class, a core's two hardware threads counted once. On a part that reports no core types
they are eight of its physical cores. The run uses eight even where the desktop has more. The M1
has four performance cores and cannot prove the criterion, so there a projection steers. The cost
test runs twice in one sitting, on one frame worker (T₁) and on N (T_N), each the median of 60
ticks. S = (N·T_N − T₁)/(N − 1) is the time that did not shrink with workers, whatever its cause
(sync points, exclusive systems, finalize steps, dispatch, join tails, wakes), and the projected
eight-core tick is S + (T₁ − S)/8, to be at most 4 ms. On the M1 that is 7·T₄ − T₁ at most 24 ms.
It assumes the speed-up measured from one worker to four continues on the same curve to eight, so
it is a projection and never the proof. If session A shows fewer than eight performance cores, the
projection comes back to the owner as a question. What would change this: the owner reading the
4 ms as the whole tick with the physics phase inside it, which leaves the ECS about 1 ms and makes
the kernel path's target four times harder.

**5.2 ECS part B is two lanes, and both conform to sections 5.2 and 6.** Part A's lane measured
that about 40 of the 41 ms are per-row invocation overhead, and estimated the fork alone at 11 to
12 ms on four cores. The fork cannot reach the criterion without the kernel path, and the kernel
path rewrites the code the fork would otherwise parallelise first and lose. So `ecs-kernel` lands
the per-chunk kernel path sequentially, and `ecs-fork` then forks the shape that stays. With S
near 0.5 ms and part A's 3.5 times on four workers, 7·T₄ − T₁ at most 24 ms needs the one-core
tick to fall from 41 ms to about 21.5 ms (T₄ about 6.5 ms). The review of part A left six design
questions about the fork, and the architecture already answers them:

- A system is a graph node with a remaining-units counter. Its units are chunks, released as one
  range that splits on steal (sections 5.1 and 5.2). There is no scope per system.
- A column's coarse tick is written once at dispatch, by the parent, before any unit runs
  (section 6).
- Partials merge through order-independent combinators. A reduction that needs key order is
  `ordered` and sorts in finalize (section 5.2). Nothing folds in chunk order.
- Nothing is pooled by worker slot, which the boundary lint file bans, so `lactose-task`'s
  README's per-range setup "keyed by slot" is not used. A unit's outputs live in storage the node
  owns, indexed by chunk and sized when `units()` releases the node; the job arena holds only
  scratch.
- The layout mode's random splits are drawn before the fork, or keyed by tick, system, archetype
  and chunk.
- Outputs, events and commands merge in the order D117 fixes, whatever order the units finished.

What would change this: `ecs-kernel` missing its steering figure by a wide margin, in which case
the coordinator asks the planner for a ruling before `ecs-fork` is briefed.

**5.3 The netcode spike runs in one process over a seeded link, and SOUR's architecture mode is its
x86 server.** The spike is `crates/app/tests/spike.rs` made to run: a server and a client built by
`lactose-app`, a throwaway transport that delays and drops packets from a seed, a scripted clock,
scripted input samples, and the four scenarios of section 22. Its scenes are recording tests. Each
records both worlds' hashes every tick and a hash of every packet either end sends, and the nightly
architecture mode compares them across linux-x64, windows-x64 and macos-arm64. Each end is a
function of the bytes it receives, the scripted samples, the clock script and the seed. So if every
platform records the same bytes, an x86 server paired with an aarch64 client exchanges those bytes
too, tick by tick, and the aarch64 client's run on the M1 is a run against the x86 server. No
second binary, socket or two-machine session is built. A live link adds more than jitter: two
machines' clocks drift apart by tens of parts per million, real pacing wakes late, and a socket
coalesces and reorders packets within a tick. So the seeded link also delays each packet by a
seeded amount below a tick and reorders within a tick. The client's clock is scripted with a rate
offset, seeded late wakes and one step, and is read through the same `ClockEstimate` path a live
client uses. The correction rate is recorded with the clock script and without it. The figures the
criterion names come from an ignored release test, run on the M1 and checked in as result file
`spike-net`, that replays a recorded scene and asserts that its stream hash equals the recording
test's. The bytes are the spike's own packer's; `net`'s packer is proven at milestone 4. Each
recorded scene is 1,800 ticks, thirty seconds of play, and shorter if the four together would add
more than ten seconds to one SOUR run on linux-x64, where the whole nightly matrix takes five to
six minutes today. The ignored test runs each scene on to 36,000 ticks, the ten minutes a rate per
minute needs, and asserts the hash over the recorded ticks. The spike does not wait for S3 and S4,
since no scenario owns an entity through another or carries an animated character; its checked-in
figures are taken after `stream-impl`. What would change this: the recorded bytes differing between
platforms. That difference is then the finding (row 86), and a live run across two machines is
needed to measure the correction rate it causes.

**5.4 The 32-bit branch is built only if a probe passes the precision half.** Section 16's rule
adopts 32 bits only if positional error at the map edge stays under one millimetre over ten minutes
and every SOUR mode passes. The precision half needs no engine change. `scalar-probe` runs the walk
scene of `crates/physics-rapier/tests/support` for 36,000 ticks on upstream `rapier3d` 0.36.0 in 32
bits, in a throwaway crate outside the workspace, at the origin and translated by the extent, and
reports the largest difference between the two runs' positions relative to their scene origins. It
runs the same pair in 64 bits as the control. Differences are taken in f64. A 64-bit control that
differs by a millimetre or more means the scene amplifies differences through its contacts, and the
probe is redesigned, not read. The result file carries the probe's source. No document states the
map extent, so the probe runs at 6 km (D59's figure) and at 8 km. The probe is the first lane to
start: a pass would change the scalar under the spike's baseline, the quantiser bounds, the
tolerances and every figure of the measurement commit, so its result comes before any of them. If
the error is at or over one millimetre, 64 bits stay, the SOUR half is moot, and the planner writes
the decision with the 64-bit rollback loop's cost on the M1 from `measure`. The expectation, by
arithmetic and not by measurement: a 32-bit position between 4.1 and 8.2 km is spaced 0.49 mm, a
walking step rounds by up to half of that each tick, and 36,000 ticks accumulate about 27 mm as a
random walk. What would change this: the probe passing. The owner is then asked whether to
commission the branch (the scalar in `lactose-math` with new golden vectors, a second vendored
Rapier, the quantiser bounds, every SOUR mode), which is worth its cost only if the 64-bit rollback
loop leaves the ring shallower than the deepest correction the spike records. The probe did not
pass: on upstream 32-bit Rapier the walk scene at x = 6 km differs from the origin run by
157.75 mm (`docs/results/scalar-precision-apple-m1-2026-11-14.md`). Its second run took 12 km in
place of 8 km, which lies in 6 km's binade, and it found a speed bias where this paragraph
expected a random walk. 64 bits stay and the branch is not built. The cost condition above does
not arise, since it was written for a probe that passed. A shallow ring is `lactose-predict`'s
question at its milestone 4 review (D146, `docs/plans/scalar-decision.md`).

**5.5 Copy-on-write snapshots are not built, and a prediction world holds at most 64 fixed
colliders and proxies as rows.** D34 builds copy-on-write only if the milestone 1 measurement
exceeds the budget. The measurements in rows 17 and 87 already decide it. The part copy-on-write
would share, the world snapshot, is 5.4 µs with no statics and 28 µs with a thousand. The cost is
Rapier's serialisation of each fixed collider: 0.6 µs to write and 1.0 to 1.2 µs to restore, so the
take reaches 100 µs near 100 fixed colliders and the restore near 75, and copy-on-write in
`lactose-predict` cannot share Rapier's bytes. At 64 the take is about 76 µs and the restore about
92 µs. A proxy with a `RigidBody` is a kinematic body in the prediction world's solver (section
9.4) and is serialised too, at about the same 0.6 µs, going by row 87's 32 µs for fifty bodies and
a floor. So the bound counts both: at most 64 fixed colliders and proxies together, beside the
fifty claimed bodies, until `measure`'s slot test, which holds proxies, gives the mix a figure of
its own. The spike's scenes need a floor and a handful of blocks. The bound is a content rule at
milestone 1, stated in the two READMEs and recorded by the spike's result file (each replay's
`prediction_fixed_and_proxies`, which `cargo xtask measure spike-net` checks against 64).
Milestone 3 must design what replaces it. Section 9.4 gives the prediction world "the immutable
static collision asset", but today `physics-api`'s snapshot carries the backend's whole world
(section 10, D119), so holding cooked static scenery outside the per-tick snapshot changes that
contract; it is listed for revision 3.3. What would change this: the slot measured through
`lactose-predict` at the measurement commit exceeding 100 µs on the spike's scene. It did: the quiet
window measured 140.8 µs. D143 replaces this decision's cost sentences, from "The cost is Rapier's
serialisation of each fixed collider" to "At 64 the take is about 76 µs and the restore about
92 µs", which are of bodies that touch nothing. The slot holds a copy, and the bound of 64 stands
until the copied slot is measured (`docs/plans/criterion-5-proposal.md`).

**5.6 The bundle's layout digest lands in milestone 1, before the cooker.** D129 left
`ComponentSchema` without the digest, so a same-size layout change without a version bump is
refused in a save and not seen in a baseline or an overlay. Adding it is a bundle format bump that
changes every baseline's and overlay's bytes and every baseline's content hash. Each test that pins
document bytes moves with it, and those only multiply: SILO's baseline kind is being implemented
and the cooker comes next. So `schema-digest` precedes `ricotta`, behind `silo` and `ecs-fork`
since it edits `lactose-ecs`, and row 113 is not re-gated. What would change this: the lane finding
that the digest needs a change to `lactose-bundle`'s surface beyond the one D129 names. It then
stops, and the row moves to milestone 3 with that reason.

**5.7 The Haswell-masked run stays in section 8.3 and gates milestone 4.** It is the one dynamic
check that no dependency selects a different numeric path on a CPU newer than the x86-64-v3
baseline. That matters when clients on arbitrary x86 CPUs predict against a server, which is
milestone 4, where architecture mode also moves to every pull request. It gets a verification row
now. Until then the nightly prints each runner's CPU model and feature flags beside its hashes,
so the history shows which feature sets have agreed. What would change this: any nightly
comparison, the spike's stream above all, differing between two x86 hosts. The run is then wired
at once.

**5.8 A region's asset set at milestone 1 is `Baseline::assets`, and row 65 moves to milestone
3.** Section 14, `crates/asset/README.md` and row 65 give the set three ways. Nothing at
milestone 1 spawns or migrates an entity that references an asset outside its region's baseline,
and `lactose-persist` already defers a section's resident assets to milestone 3. The asset README
is corrected with `silo` (section 6). The rule milestone 3 needs is row 65's: the baseline's set,
the `AssetRef`s of resident entities the baseline does not hold, and a spawnable set the baseline
declares, with a spawn that names an asset outside the table a tick-stamped asset failure. It
widens section 14 and is listed for revision 3.3. What would change this: a milestone 1 lane
needing a runtime-spawned entity to read an asset through its region's table.

**5.9 Edit-to-playing at milestone 1 is measured on the sketch game, and no build-time mitigation
is taken unless it is over.** No game crate exists before milestone 5, so milestone 1 measures the
floor, with two edits. The first is a one-line change in the body of the sketch game's system, in
the headless binary's own crate: one crate recompiled, the link and the start. The second is a
one-line change in the body of a function of `lactose-app`, a library the binary depends on, which
is where a game crate will sit: the library recompiled, then the binary, the link and the start.
Both are recorded, and the criterion is read against the slower. For an operating system the
desktop does not boot, the figure of `measure`'s run on that system's CI runner stands, marked as a
runner's. Under thirty seconds it shows the platform; over it, the platform is recorded as not
measured, with the reason. Section 22.1's row "Build-time mitigation if over budget" closes at
milestone 1 with the measurement if it is under thirty seconds on every platform, and the row is
asked again at milestone 5 against the real game crates. If the floor is over, the planner writes
the mitigation as a proposal before milestone 1 exits. Section 18's own fallback is dylib reload.

**5.10 The RHI spike is milestone 1's; whether `rhi-api`'s facade and `rhi-null` follow it there is
the owner's question.** The spike's question: what must `rhi-api` expose for section 12.1's list to
run on Vulkan 1.3 with no call a null backend cannot honour? Its evidence is a throwaway `ash`
program, headless, on the M1 under MoltenVK. It is never checked in, inside the workspace or
outside it: `unsafe` stays in the crates of section 21. It walks the list: a device and queues, a
buffer with a device address, one bindless heap, a pipeline from precompiled SPIR-V with a
specialisation constant, a pipeline cache saved and reloaded, encoders recorded on two threads, a
pass with a transient attachment and a local read, a timeline semaphore, an indexed indirect draw,
and a readback of the triangle. Its result file records, for each item, the feature or extension
used, whether MoltenVK reports it, the state the caller carried between calls (which is what a null
backend must keep), and what needed external synchronisation. MoltenVK translates Vulkan to Metal
and has a row of its own (row 15), so a "no" from it is not yet a "no" of the design. The result
file marks each "no" as MoltenVK's, judged against MoltenVK's published limits and the
feature's Vulkan requirement, or as a design question. The conformant reference run, the same list
under lavapipe, moves to milestone 2 with the Vulkan backend's first Linux job, and settles the
design questions. The desktop's GPU is `rhi-vulkan`'s at milestone 2 too.

Section 22 delivers `lactose-rhi-null` in milestone 1 and `lactose-rhi-api` in milestone 2, and the
first implements the second. As written, then, the facade is a milestone 1 delivery, unstable until
the native Metal spike (D43), and `rhi-null` follows it: fourteen runs with no consumer in
milestone 1. The alternative moves both to milestone 2, behind its MoltenVK, KosmicKrisp and native
Metal spikes, so the facade is written once with every spike's result and the null backend once
against it. This plan recommends the move. Section 20 freezes an `-api` on its first real backend,
and a null backend honours any surface, so the pair proves nothing in milestone 1. The move takes
no run out of the project. Fourteen leave milestone 1, about 165 becoming about 150, and arrive in
milestone 2, less what a facade written before the Metal spike risks: an amendment round of about
four runs and a rework of the null backend of about three. It takes revision 3.3's ruling on C2 off
milestone 1's path. It needs revision 3.3, since section 22 names `rhi-null` in milestone 1. What
would change the recommendation: the owner wanting milestone 2's Vulkan backend to start against a
facade the day milestone 1 exits. Until the owner rules, the two lanes stay at the end of queue 1's
render work and nothing waits on them. If they stay, revision 3.3 first settles whether the server
links `rhi-null` (C2): `crate-tiers.toml` and D83 say it does not, and this plan assumes that.

**5.11 A measurement that proves a criterion is a file, written by one command.** Section 20
already requires results to be checked in. `cargo xtask measure` makes that one command on any
machine, with the header of section 4, so an owner session is a clone and a command.

**5.12 Milestone 0's unshown criteria are exit conditions of milestone 1.** The first entry of
section 6 is the decision.

**For the owner.** Answered on 14 November 2026, and written into this plan:

- The desktop has eight or more performance cores. Its CPU model, operating systems and GPU come
  back with session A's files.
- Session A runs now, before this plan is approved, and the coordinator gives the owner the
  commands. Session B runs at the measurement commit.
- The owner installs the Vulkan SDK on the development Mac before `spike-rhi` is briefed.
- Row 102: both defects of the vendored patch were already reported upstream (dimforge/rapier
  issue #1054 with pull request #1074, and issue #1008 with pull request #1018), so nothing is
  filed and `vendor/README.md` links them.

Answered the same day, each as this plan recommended:

1. Whether criterion 1's 4 ms is the tick without the physics phase (decision 5.1). Recommended:
   yes.
2. Whether architecture mode over the spike's recorded bytes may stand for "x86 server, aarch64
   client", with drift, late pacing, and sub-tick jitter and reordering modelled in the one
   process (decision 5.3). Recommended: yes.
3. Whether `rhi-api`'s facade and `rhi-null` move to milestone 2, behind its three spikes
   (decision 5.10). Recommended: yes. Fourteen runs leave milestone 1 and none leaves the project.
4. If `keeper` does not reach 20 µs: whether the tick-start criterion is restated at 35 µs in
   revision 3.3. Recommended: yes, with the cause or the failed attempt recorded.
No longer open (D146):

5. The reference map's extent, only if the probe's 6 km and 8 km runs fall on different sides of
   one millimetre. Moot: the runs read, at 6 km and 12 km, both exceed one millimetre, and no
   extent is set.

Withdrawn: whether the projection may stand as proof of criterion 1. It no longer does. It comes
back as a question only if session A shows fewer than eight performance cores.

## 6. Text for the coordinator (exact)

The numbers were allocated at the commit: D138 and D139.

### Decision entry: milestone 0's carried criteria

```
**D138. Milestone 0's unshown criteria are exit conditions of milestone 1** (§5.1, §20, §22, `lactose-task`, `docs/results`)
- Why: milestone 1's lanes began with part of section 22's milestone 0 proof not shown, and no
  decision recorded it. On the developer's Apple M1 the two checked-in runs of the benchmark
  (`docs/results/whisk-apple-m1-2026-10-09.md` at `00f1f3d`,
  `docs/results/whisk-apple-m1-2026-10-10.md` at `8cf544c`) pass the makespan ratio (at most 1.139
  at P50 and 1.206 at P99, against 1.25 and 1.5), the wake from an idle pool (P99 32.8 and 30.4 µs
  against 35) and the steal (P99 125 and 167 ns against 1 µs). They fail tick start: P99 24.5 and
  29.9 µs at 60 Hz against 20, with P50 3.7 and 2.8 µs and none of 5,000 deadlines over 1 ms. The
  quiet machine is the slower of the two states the keeper shows, and the cause is not known
  (D116). At `3264406` no x86 desktop had run the benchmark, so the x86 half of every criterion
  was unshown.
  Five verification rows gated at milestone 0 are open: the wake and tick-start rows, the
  wall-clock log of samples over 1 ms, the first run of the Linux and Windows topology parsers on
  real hardware, and Windows efficiency-core steering. Loom and Miri pass nightly.
  - *Milestone 1 was not held for them.* The criteria prove the executor. Nothing a milestone 1
    lane builds depends on an x86 figure or on the 10 µs between the measured tick start and its
    criterion: each consumer measures its own cost. Rules out reading milestone 0 as closed.
  - *Each is an exit condition of milestone 1.* It closes by a result file under `docs/results`,
    or by an owner decision that restates it, recorded as an addendum here.
  - *The x86 half closes in one session on the owner's desktop* at main's head, which the owner
    runs before the plan is approved: the benchmark and `lactose-task`'s tests. The files it
    writes pass or fail makespan, wake, tick start and steal on x86, and the topology it prints
    closes the parsers' row.
  - *Tick start gets one bounded attempt.* One lane on `lactose-task`, after the x86 numbers,
    finds what separates the two machine states or shows a keeper that meets 20 µs at P99 on a
    quiet M1 for at most 1.5% of a core. This reopens D113's rejection of a warm sleeper for the
    keeper alone; its power cost is recorded with the attempt. One hypothesis it tests, not
    confirmed: the keeper spins the last 50 µs, and a final timed park that ends late by the
    deep-idle exit the wake spike measured (P99 24 to 29 µs) passes the deadline. The same lane
    logs the wall-clock time of every sample over 1 ms. If the attempt fails, the criterion is
    restated at the 35 µs of the wake criterion in the next architecture revision, with the
    attempt recorded, as the owner decided on 14 November 2026. Rules out a second attempt,
    and a criterion that stays failed and unrecorded.
  - *Windows efficiency-core steering is re-gated to milestone 2*, where the first Windows client
    runs asset work beside frame work. It needs hybrid x86 hardware to measure.
- Driven by: the whole-project review at `6ed8461` (no milestone 1 plan; milestone 0's failed
  criterion carried without a record); `docs/plans/milestone-1.md`.
- Replaced: nothing. It records what was done without a record.
```

### Decision entry: the plan's rulings

```
**D139. The milestone 1 plan: how its criteria are judged, what is re-gated, and the decisions that came due** (§5.2, §8.3, §9.4, §10, §12.1, §14, §15.4, §16, §18, §20, §22, §22.1, §23, `docs/plans/milestone-1.md`)
- Why: no document mapped section 22's milestone 1 criteria, or the verification rows that gate
  milestone 1, to a lane or an owner. Four criteria had nothing proving them, criterion 1 had
  been measured only on four performance cores, and several rows waited on decisions nobody had
  been asked to take. `docs/plans/milestone-1.md` is the map and gives each decision's reasons;
  these are the decisions.
  - *Each criterion keeps its own workload.* The 100k tick is the scene of
    `crates/sim/tests/cost.rs`, without physics and without the world hash; the physics phase is
    measured on its own scene; the composed tick is recorded and is not a criterion. Rules out
    reading the 4 ms as a tick with the 3 ms physics phase inside it.
  - *Criterion 1 is proven by a direct run on eight performance cores*, the owner's desktop: the
    median tick on eight frame workers at most 4 ms, one worker for each of eight performance
    cores as the executor counts them. On the M1 a projection steers and proves nothing:
    S + (T₁ − S)/8 at most 4 ms, T₁ and T_N the median tick on one and on N frame workers in one
    sitting, and S = (N·T_N − T₁)/(N − 1). On the M1 that is 7·T₄ − T₁ at most 24 ms. Rules out a
    projection standing as proof while a machine with eight performance cores is in reach.
  - *The parallel runner is two lanes, the per-chunk kernel path first, then chunks as the units
    of a graph node, released as one range that splits on steal.* Both conform to sections 5.2
    and 6 as written, and a unit's outputs live in storage the node owns, indexed by chunk. Rules
    out a scope per system, partials folded in chunk order and buffers pooled by worker slot.
  - *The netcode spike runs in one process over a seeded link and a scripted clock, and SOUR's
    architecture mode over the bytes both ends record is its x86 server.* The link delays below a
    tick and reorders within one, and the client's clock drifts, wakes late and steps by script,
    since a live link adds those and not only jitter. Rules out a second binary and a two-machine
    session, unless the bytes differ between platforms.
  - *The 32-bit scalar branch is built only if a probe on upstream 32-bit Rapier passes the
    precision half of section 16's rule.* Rules out a second vendored Rapier and a scalar switch
    in `lactose-math` built to test a rule the arithmetic says it fails.
  - *Copy-on-write snapshots are not built* (D34's condition): the cost of a slot is Rapier's
    serialisation of colliders and bodies, about 0.6 µs each to write and about 1.1 µs for a
    fixed collider to restore, which copy-on-write in `lactose-predict` cannot share. A prediction
    world holds at most 64 fixed colliders and proxies together as rows. Milestone 3 must design
    how cooked static scenery reaches the solver outside the per-tick snapshot, since
    `physics-api`'s snapshot carries the backend's whole world today (section 10, D119).
  - *The bundle's layout digest lands in milestone 1*, before the cooker: it changes the bytes
    of every baseline and overlay, and every test that pins them moves with it.
  - *The Haswell-masked run stays in section 8.3 and gates milestone 4*, with architecture mode's
    move to every pull request. Until then the nightly prints each runner's CPU model and flags.
  - *A region's asset set at milestone 1 is `Baseline::assets`.* The assets of residents the
    baseline does not hold, and a declared spawnable set, are milestone 3's, with
    `lactose-persist`'s deferral.
  - *Edit-to-playing at milestone 1 is measured on the headless binary's sketch game*, by an edit
    in the binary's crate and an edit in `lactose-app`, read against the slower. Section 22.1's
    build-time mitigation is decided only if that floor is over thirty seconds.
  - *The RHI spike is milestone 1's: a throwaway program, never checked in, that walks section
    12.1's list under MoltenVK on the M1.* Its result file marks each "no" as MoltenVK's or as a
    design question, and the conformant reference run under lavapipe is milestone 2's, with the
    Vulkan backend's first Linux job. `lactose-rhi-api`'s facade and `lactose-rhi-null` move to
    milestone 2, behind its MoltenVK, KosmicKrisp and native Metal spikes, so the facade is
    written once with every spike's result; the next revision moves `rhi-null` in section 22.
    Rules out a facade and a null backend with no consumer, written before the spikes that amend
    them, and FFI code outside section 21's crates.
  - *A measurement that proves a criterion is a file under `docs/results`* with its machine,
    date, commit, topology and load, written by `cargo xtask measure`.
  - *Re-gated*: the physics phase's stall on render recording to milestone 2; how a re-spawned
    body presents to milestone 2; the directory sync on other file systems to milestone 4;
    `ordered` and per-region reductions to milestone 5. Durability after a power loss is stated
    as each platform's contract and is not measured.
- Driven by: the whole-project review at `6ed8461` (its findings on the missing plan, the
  unproven criteria, the unrecorded measurements, the bundle digest, the asset set and the
  Haswell run); the owner's decision of 14 November 2026 to plan before revision 3.3, and the
  owner's answers of the same day (the desktop has eight or more performance cores; criterion 1
  is the tick without the physics phase; the netcode spike runs in one process; `rhi-api` and
  `rhi-null` move to milestone 2; tick start is restated at 35 µs if the keeper's attempt
  fails).
- Replaced: nothing in `docs/architecture.md` until the next revision, which states what
  criterion 1's tick excludes, the spike's sentence and the sentences
  `docs/plans/milestone-1.md` section 8 lists.
```

The entry's RHI bullet is written for the recommended answer to the owner's question 3. If the
owner keeps the facade and the null backend in milestone 1, the bullet is this instead:

```
  - *The RHI spike is milestone 1's: a throwaway program, never checked in, that walks section
    12.1's list under MoltenVK on the M1.* Its result file marks each "no" as MoltenVK's or as a
    design question, and the conformant reference run under lavapipe is milestone 2's, with the
    Vulkan backend's first Linux job. `lactose-rhi-api`'s facade is written in milestone 1,
    unstable (D43), since `lactose-rhi-null` implements it. Rules out FFI code outside section
    21's crates.
```

### Verification rows

Each change is anchored on the row's opening words. Apply them after the `ecs-a` merge, which
rewrites rows 74, 83, 89 and 99.

```
--- section 1, the row "Physics phase stall": replace with two rows ---
| Physics phase share | The physics phase's share of the tick, on the composed tick of `cargo xtask measure` (the 100k cost scene with 2,000 awake bodies) | 1 |
| Physics phase stall | The stall the physics phase imposes on render recording during the overlap; nothing records a frame before `render-graph` exists | 2 |

--- section 1, the row "Scalar precision": replace with ---
| Scalar precision | Simulation scalar precision at full map extent: the walk scene for 36,000 ticks on upstream 32-bit Rapier, at the origin and translated by 6 km and by 8 km, against the same pair in 64 bits as the control, differences taken in f64; the result file carries the probe. The 32-bit branch is built only if the error stays under one millimetre (`docs/plans/milestone-1.md`, decision 5.4) | 1 |

--- section 2, the row "`std::fs::File::sync_all` issues `F_FULLFSYNC`": append to its text; its gate stays `| 1 |` ---
 The replaced file's durability after a power loss is not measurable here: it is stated as each platform's contract and is not verified (D139)

--- section 2, the row "Directory fsync after a rename (D130)": append to its text, and replace its gate `| 1 |` with `| 4 |` ---
 Re-gated to milestone 4 (D139): the file systems still unmeasured are deployment targets, and architecture section 22.1 gives the disk-full and corrupt-save paths to milestone 4

--- section 4, the row "Windows efficiency-core steering": append to its text, and replace its gate `| 0 |` with `| 2 |` ---
 Re-gated to milestone 2 (D138): the first Windows client is where background work runs beside frame work, and the measurement needs hybrid x86 hardware

--- section 4, the row "Assets of runtime-spawned entities": replace with ---
| A region's asset set beyond its baseline's (architecture section 14). At milestone 1 the set is `Baseline::assets` and nothing spawns or migrates an entity that names an asset outside it. Decide with streaming: the set is the baseline's set, the `AssetRef`s of resident entities the baseline does not hold (read from the region's section by `persist`), and a spawnable set the baseline declares; a spawn that names an asset outside the table is a tick-stamped asset failure. It widens section 14 | 3 |

--- section 4, the row "Two section 5.2 and 9.7 features": replace its gate `| 1 and 4 |` with ---
| 4 (decimation) and 5 (`ordered` and per-region reductions, first needed by game systems) |

--- section 4, the rows "Wake latency against the 20 µs P99 criterion", "The benchmark Mac enters idle or maintenance sleep", "The Linux and Windows core-topology parsers" and "Tick-start latency against the 20 µs P99 criterion": replace each gate `| 0 |` with ---
| 1 (carried, D138) |

--- section 4, the row "`lactose-predict` is implemented (D122 addendum)": append to its text, and replace its gate `| 1 |` with `| 1 and 2 |` ---
 (the presentation check is milestone 2's, with the presentation schedule; D139)

--- section 4, the row "The headless binary's 120-tick run": append to its text ---
; session B compares the hash on x86-64 with the one the quiet window prints on the M1 at the measurement commit, not with this one (`docs/plans/milestone-1.md`, section 4)

--- section 4, the row "A milestone 1 tick at 100k entities against the 4 ms target (D137)", which the ECS part A merge adds: replace its last sentence "Re-measure on 8 performance cores" with ---
Measured on eight performance cores in session B; the M1's projection of decision 5.1 steers (`docs/plans/milestone-1.md`)

--- section 4, the row "A layout change without a version bump": replace with ---
| A layout change without a version bump is refused by the digest in a save and not seen by `World::load_region` in a baseline or overlay until `lactose-bundle`'s `ComponentSchema` carries the digest (D129). The per-type version-and-digest record that would stop one being written, and the check on a reused field ID that section 15 names, do not exist yet. Both land in milestone 1, the digest before the cooker (D139) | 1 |

--- section 4, the row "`lactose-sim` (D118) is implemented": once `binds-impl` has given `Simulation` its `Debug`, strike that clause and replace its gate `| 1 and 3 |` with ---
| 3 |

--- section 4, the row "`cargo-deny` 0.20.2 skips a path source": replace its gate `| 1 |` with `| none |` once the workflow pins `cargo-deny`, and append ---
 The workflow pins `cargo-deny`, so an upgrade is a change of that pin and the re-check goes with it

--- section 4, add rows ---
| The Haswell-masked linux-x64 run of architecture mode (architecture section 8.3, D30) is not wired: the nightly's SOUR jobs run on whatever CPU the runner has. Wire it under an emulator that hides everything newer than the x86-64-v3 baseline (Intel SDE or `qemu-x86_64 -cpu Haswell`), over the baseline run of the recording tests only, and measure what it costs the nightly, whose whole SOUR matrix takes five to six minutes on linux-x64 today. Until then the nightly prints each runner's CPU model and feature flags beside its hashes, so the history shows which feature sets have agreed; a comparison that differs between two x86 hosts wires it at once (D139) | 4 |
| A prediction world holds at most 64 fixed colliders and proxies together as rows (D139): with Rapier a slot's take reaches the 100 µs budget near 100 fixed colliders beside fifty bodies and its restore near 75, and a kinematic proxy is serialised as a fixed collider is. `physics-api`'s snapshot carries the backend's whole world (architecture section 10, D119); design with streaming how cooked static scenery reaches the prediction world's solver outside the per-tick snapshot, and what enforces the bound until then | 3 |
| The netcode spike's recorded streams (D139): `crates/app/tests/spike.rs` records both worlds' hashes each tick and a hash of every packet either end sends under SOUR, over a seeded link that delays below a tick and reorders within one, and a scripted client clock that drifts, wakes late and steps. The nightly architecture mode compares them across linux-x64, windows-x64 and macos-arm64. Equal bytes on every platform are what stands for "x86 server, aarch64 client" in section 22; a difference is a finding and calls for a run across two machines | 1 |
| Section 22's milestone 1 timing criteria are proven by result files under `docs/results`, written by `cargo xtask measure` on a quiet Apple M1 and on the owner's x86 desktop at one commit (`docs/plans/milestone-1.md`, section 4). Criterion 1 is proven by the desktop's run on eight performance cores; on the M1 a projection from a one-worker and a four-worker run steers (D139). Edit-to-playing is measured on the headless binary's sketch game, by an edit in the binary's crate and one in `lactose-app`; section 22.1's build-time mitigation is decided only if it is over thirty seconds, and asked again at milestone 5 | 1 |
| The RHI spike ran under MoltenVK alone (D139): its result file marks each "no" as MoltenVK's, against MoltenVK's published limits and the feature's Vulkan requirement, or as a design question. Run the same list of architecture section 12.1 under lavapipe with `rhi-vulkan`'s first Linux job, as the conformant reference, and settle each design question from it | 2 |
| The nightly Miri job took 38 to 57 minutes of its 60-minute limit in the eight green nightlies of 5 to 13 November 2026, before the ECS hot-path work added tests, and a timeout also drops the two weak-memory steps after `lactose-ecs`. Split the job per crate before the parallel ECS runner merges | 1 |
```

### One README sentence, for the `silo` lane

In `crates/asset/README.md` the sentence wraps over two lines of a bullet, each indented by two
spaces. Replace these two lines:

```
  of an unload needs the baseline alone. The region's asset set is `Baseline::assets`,
  which its driver requests by name.
```

with these four:

```
  of an unload needs the baseline alone. At milestone 1 the region's asset set is
  `Baseline::assets`, which its driver requests by name; the assets of residents the
  baseline does not hold join it at milestone 3 (architecture section 14,
  `docs/verification.md`).
```

## 7. The exit

Milestone 1 exits when these have happened, in this order.

1. The measurement commit exists. The quiet window and session B have run on it, and their result
   files are on main. Session B's run on eight frame workers is criterion 1's proof. The second
   quiet window has run at the first commit that holds `slot-copy`, and its three files are
   criterion 5's proof, criterion 7's rollback figure and criterion 7's first `spike-net` file
   (D143). If the desktop boots Windows, its `whisk` file there has been written at the first commit
   that holds `timer-windows` and, if one is made, the follow-up that sets its Windows threshold
   back (D144).
2. The planner has written the scalar decision from the probe and the measured rollback loop, and
   the owner has approved it: `docs/plans/scalar-decision.md`, D146.
3. Every queue of section 3 has drained, so section 22's delivers are on main: the parallel ECS
   runner, SILO, the RICOTTA skeleton, the RHI spike's result, and `lactose-rhi-null` unless the
   owner has moved it to milestone 2.
4. The coordinator's pass has left every row of section 2 Done or re-gated.
5. A `reviewer` reads each of the nine criteria, and each carried criterion of milestone 0,
   against its test and its result file. It reports one verdict per criterion: met; met as
   restated, naming the decision; or not met. It re-runs what it can on the M1 and states what it
   could not. It compares the steering figures of every lane merged after the measurement commit
   with the measured files.
6. The owner rules on every "not met": another lane, or a restatement recorded in the decisions
   log. Milestone 1 does not exit on an unrecorded miss.
7. The follow-up project review reads the determinism and readiness seams at that commit. Its
   blocking and serious findings are fixed or routed before step 8.
8. The planner writes the milestone 2 plan.

## 8. For revision 3.3

Revision 3.3 is the planner's next pass. It must settle these. The first seven are the review's.

| Item | What 3.3 decides |
|---|---|
| C2: the null RHI backend is given to the server by sections 2 and 12.1 and denied to it by section 4 and `crate-tiers.toml` | Recommended: strike the server from both sentences; D83 replaced D51's clause. It gates `rhi-null` |
| C10: section 20's interface rules are not the process in force | State the rule in force: a crate other crates are written against gets a facade first, and backends follow as their own lanes. Narrow "every `-api` ships a conformance suite" to interfaces a backend implements |
| Finding D7: section 15.1 says derived types are "Rebuilt by `on_load`", and no such hook exists | Say who rebuilds a derived value: the system that owns it |
| S1's sentence: a prediction world's region modes | State that a prediction world's regions are `Simulated`, and what a server-`Frozen` region is on a client, from the `client-fix` report |
| C1's option: section 8.2's debug assertion on a NaN in a persistent column has no check | Add the check, or strike the sentence and open a row. Recommended: keep the sentence and assert where NaNs are already canonicalised, in the hasher and the serialiser, in debug builds; the check then goes with `binds-impl`, which already edits `lactose-sim` |
| C9(b): codenames on test glue (`struct Sour`, `sour_*`, `silo_*` test functions) | Rename the glue, or give section 3 a test-glue exception. Recommended: rename |
| X6's option: strike the Haswell-masked run from section 8.3 | Recommended: keep it, gated at milestone 4 (decision 5.7) |
| Section 22, milestone 1, criterion 1 | The machine stands as written. State that the tick excludes the physics phase and the world hash, and what eight performance cores means for frame workers on a hybrid part (decision 5.1) |
| Section 22, milestone 1, the netcode spike | State what "x86 server, aarch64 client" is proven by (decision 5.3) |
| Section 22, rows 1 and 2 | If the owner moves them: `rhi-null` leaves row 1 for row 2, and C2 is settled with it there. If not: `rhi-api`'s facade is a milestone 1 delivery, unstable until milestone 2's spikes (decision 5.10) |
| Section 22, milestone 0, tick start | Only if `keeper` fails and the owner restates it: 35 µs, and D113's premise corrected. If `keeper` succeeds by a longer spin, section 5.1's 50 µs and D113's rejection of a warm sleeper are restated for the keeper |
| Section 16, "Milestone 1 measures the physics phase's share of the tick and the stall it imposes on render recording during the overlap" | The stall moves to milestone 2 |
| Section 22.1, "Build-time mitigation if over budget" | Closes at milestone 1 on the floor measurement and is asked again at milestone 5 |
| Section 18, "Edit-to-playing time is measured per platform in milestone 1" | Say what milestone 1 measures before a game crate exists (decision 5.9) |
| Section 14, a region's asset set | Row 65's rule: residents the baseline does not hold, and a declared spawnable set (decision 5.8) |
| Sections 9.4, 10 and 23, copy-on-write and static scenery | Copy-on-write is not built. Section 10's snapshot carries the backend's whole world: say that milestone 3 designs how static scenery stays outside the per-tick snapshot, and state the bound on fixed colliders and proxies until then (decision 5.5) |
| Section 20, results | The result-file convention and `cargo xtask measure`; section 4's `xtask` line gains it |
| Section 8.3, "Hashing `physics-api::snapshot` bytes is added once Rapier's serialisation is shown to be order-stable" | Amended by the `sour-evidence` merge if the nightly shows it, not by 3.3; listed so 3.3 does not restate the old sentence |
