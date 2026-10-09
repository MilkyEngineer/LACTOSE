# LACTOSE Verification

What must be measured or re-checked before a milestone can rely on it. `docs/architecture.md`
is the binding design and its milestone table (section 22) holds the proof criteria; this file
lists the spikes, the third-party claims and the open items those milestones depend on, each
with the milestone that gates it.

## 1. Spikes

| Spike | What it records | Gates milestone |
|---|---|---|
| io_uring on CI runners | Whether io_uring is available on the CI runners | 0 |
| Clippy on primitive methods | Whether `disallowed-methods` matches inherent primitive methods on the pinned toolchain |0. **Done:** on 1.99.0 it matches both `a.min(b)` and `f32::max(a, b)` forms, and `f64::sin`, `to_bits`, `powi` and `clamp` |
| Scalar precision | Simulation scalar precision at full map extent | 1 |
| MoltenVK and KosmicKrisp | Indirect-draw behaviour, encode time for ten thousand draws, pipeline creation cost, memoryless attachments and local reads | 2 |
| Extract cost | Per-dirty-row extract cost | 2 |
| Prediction-world snapshot cost | Full-copy cost per tick, deciding whether copy-on-write is built | 1 |
| Native Metal | Argument buffers, residency, memoryless attachments, framebuffer fetch, indirect draws; `rhi-api` is frozen or amended on its findings | 2 |
| Remote transform upload | Cost per frame for 5k interpolated entities at 144 Hz | 2 |
| Physics phase stall | Share of tick and stall imposed on render recording during overlap | 1 |

## 2. Third-party claims to re-verify on pinned versions

These were reported by review against upstream sources. Each is re-checked against the exact
version in `Cargo.lock` before the milestone that depends on it.

| Claim | Gates milestone |
|---|---|
| `wide` and glam select fused or unfused multiply-add by target feature. | 0. **Done:** confirmed on wide 1.7.1 (`f32x4::mul_add`) and glam 0.34.0 (`m128_mul_add` in `sse2.rs`); the x86-64-v3 pin makes the selection constant |
| `libm`'s x86 runtime detection selects only correctly rounded paths. | 0. **Done:** on 0.2.16 the only run-time detection chooses between FMA3, FMA4 and a soft fused multiply-add, all correctly rounded; the `arch` feature otherwise selects hardware `sqrt`, `fma` and `rint` only. Pinned to `=0.2.16` (D105) |
| Whether Rapier, parry or its math crates call `wide::mul_add` on solver paths. | 1 |
| Rapier: its determinism feature combines with its parallel solver with bit-identical results for any pool size; SIMD is always on; the 64-bit build has the determinism feature; its math moved to glam-based crates. Whether a single active set makes results depend on unrelated awake bodies is tested by stepping a scene with and without a distant awake body. | 1 |
| MoltenVK multi-draw-indirect is a CPU loop. | 2 |
| Slang's Metal target supports argument buffers, function constants and framebuffer fetch. | 2 |
| Apple M1 lacks 64-bit atomics. | 3 |
| glam or its fork has a feature (such as fast-math) that changes numerics on the pinned version; if so it goes on the cargo-deny denylist. | 0. **Done:** `fast-math` changes numerics on glam 0.33.0 to 0.33.7, is a no-op from 0.33.8 and gone in 0.34; `scalar-math`, `core-simd`, `libm` and `nostd-libm` select different code. A commented ban is ready in `deny.toml`; which version Rapier selects is a milestone 1 check |
| `bytemuck_derive` on the pinned version: support for data-carrying enums under `NoUninit` and `CheckedBitPattern`, and whether a `PodInOption` derive exists for `NetId`. | 0. **Done:** 1.12.1 derives both traits for data-carrying enums with an explicit repr; it has no `PodInOption` derive, so `NetId` keeps its hand-written impls in `lactose-core`. `[T; N]` is plain data only for `T: Pod`, hence `lactose_core::PlainArray`. Its struct `CheckedBitPattern` derive needs `bytemuck` as a direct dependency and accepts `r#repr`, so `#[component]` writes and checks that impl itself (D97) |
| loom's current limitations (SeqCst modelled as AcqRel, no load buffering) and feasible preemption bound for the latch and sleep model. | 0. **Done:** confirmed on loom 0.7.2; the sleep protocol uses explicit `fence(SeqCst)` (D110). Eleven of the fourteen models are exhaustive and three are bounded (two at 5 preemptions, the idle-release model at 4); all fourteen run in about 50 s in release. Paths the models leave uncovered: the keep decision itself (`keep_woken_task`, `fire_timers`, `submit_task`; the model drives the shipped sleep and gate functions with a stand-in waker), a thief stealing a kept entry between its push and pop, shutdown with a kept entry, the real deque |
| `tracing-tracy` per-span overhead, to confirm per-system and per-range spans fit the budget. | 0. **Done:** on an M1, a `tracing` span costs about 1 ns with no subscriber and 8 ns with one; tracing-tracy 0.11.4 costs about 165 ns per span with no server connected. Per-system and per-stolen-range spans fit; per-chunk spans would not |
| Rapier's serialised snapshot is iteration-order stable enough to hash. | 1 |
| Rapier's 64-bit build uses SIMD after its glam migration, which decides the 64-bit cost argument. | 1 |

## 3. Console assumptions to keep out of milestone 0 interfaces

| Assumption | Why it will hurt | Mitigation now |
|---|---|---|
| Software zstd and LZ4 in `io` and the pack format | PlayStation and Xbox have hardware decompression | `io` exposes a read-with-decompress request, not a read followed by a decompress call |
| std threads, `park`, QoS and sysctl core queries | Consoles need fixed affinity | Worker creation goes through a platform trait in `task` |
| Writable disk for pipeline caches and crash dumps | Not guaranteed | Both paths are optional and report failure as a value |
| Slang has no public PSSL target | PlayStation shaders | The cooker's backend list is a table, not code |
| No allocator decision | Milestone 3 budgets need tagging from the start | Tagged `GlobalAlloc` in `core` from milestone 0 |

## 4. Open items

| Item | Gates milestone |
|---|---|
| Player portability between hosts | 4 (decided-later table, architecture section 22.1) |
| Multi-process server sharding: out of scope, kept possible by region ownership and command-based cross-region interaction (architecture section 9.6) | none |
| Local x86-64 runs on Apple Silicon: Rosetta on macOS 14 raises an illegal-instruction fault on the x86-64-v3 baseline, so x86-64 tests and SOUR architecture mode run on CI runners and the x86 desktop, never on the Mac. Re-check after a macOS upgrade | none |
| Windows efficiency-core steering: `ThreadClass::Background` sets thread priority only. Steering to efficiency cores on hybrid CPUs needs the power-throttling (EcoQoS) thread setting; add and measure it with the `task` benchmarks on the x86 desktop | 0 |
| `lld-link` on Windows and mold or lld on Linux (architecture section 18) are not configured yet; they go in with the edit-to-playing measurement, once CI runners and developer machines are known to have them | 1 |
| Assets of runtime-spawned entities are in no region's asset set (architecture section 14). Decide with `sim`: the set is the baseline's set plus the `AssetRef`s of resident entities plus a declared spawnable set, and a spawn may name only assets already in the table | 1 |
| Enforcing that the loader of a simulation-read asset kind is a pure byte decoder or lives in a boundary crate (D99); today it is a documented obligation on `SimulationAsset` | 1 |
| The store traits have no residency, unload or budget methods; milestone 3 streaming amends them | 3 |
| `cargo xtask features` should fail if any normal or build edge enables `lactose-asset-api/conformance` (the reference store is for dev-dependencies only) | 1 |
| Cross-architecture agreement of `lactose-math`: the golden table was generated on aarch64; the proof is the first green CI run of `golden_vectors_match` on x86-64 Linux and Windows | 0 |
| `cargo xtask boundary` should check that `rustdocflags` in `.cargo/config.toml` equals the rustflags pin | 1 |
| `lactose-reflect` field impls for the `lactose-math` types (`Scalar` as `ScalarField`, vectors as structs of scalars) are added by the `ecs` implementation lane; the facade does not need them | 0 |
| The ECS facade (`crates/ecs`, D114) has had its design review; the implementation lane replaces every `todo!()` and writes the `#[system]` expansion. To re-check against the milestone 1 netcode spike: the predicted-key rule (`ClaimedBy`, the input-tick slot set every tick, ordinal among the player's predicted spawns) must give the same key on server and client under lost and substituted inputs and after a disconnect, and the placeholder RNG keying must match on both | 1 |
| A region's seed and persistent region resources travel beside its overlay in a save and in the relevance message (architecture sections 9.3 and 15.2, amended with the ECS facade's second review); `lactose_bundle::Overlay` has no field for them, so `persist` and `net` carry them beside the document until a bundle amendment adds them to the container | 1 |
| Two section 5.2 and 9.7 features are not in the ECS facade and are added as attribute flags when first needed: `ordered` and per-region reductions (a sort by origin key in finalize, needing `Job`), and the overload ladder's decimation of non-player systems by tick parity per region (`#[system(.., decimate)]` plus a per-region parity on the world) | 1 and 4 |
| Wake latency against the 20 µs P99 criterion. **Spike done** (`docs/results/spike-wake-apple-m1-2026-10-06.md`): at a known tick deadline a stepped timed park reaches P99 6 to 10 µs for 0.6% of a core, and the shipped keeper is being fixed, since macOS timer coalescing made it 5.5 ms late at 60 Hz. From an unpredictable push, no primitive, QoS class or real-time policy gets under 20 µs on the M1 (best P99 24 to 29 µs): the floor is the cores leaving deep idle, removable only by a thread waking every 150 µs or less (P99 6 to 8.5 µs at 1.3 to 1.9% of a core, power cost unmeasured). Decided (D113): tick start under 20 µs, push from an idle pool under 35 µs, no warm sleeper; x86 desktop numbers still to come | 0 |
| The executor benchmark runs under the `TASK_UNSPECIFIED` task role from a terminal, where user-interactive QoS is clamped to priority 31. **Done:** it sets `TASK_FOREGROUND_APPLICATION` on macOS and prints the role it ran in | 0 |
| The benchmark Mac enters idle or maintenance sleep for 15 to 17 minutes at a time when unattended, with dark wakes between (`pmset -g log`, 6 and 10 October); runs and agent sessions freeze meanwhile, and `caffeinate -i` alone does not stop it. Every long run holds `caffeinate -i -s`; the executor README and `benches/whisk.rs` say so. This explains the wake-latency spike's 16 to 18 minute phase stalls. The one 176 ms outlier in the 2026-10-05 benchmark is possibly a sleep transition (`Instant` on macOS stops during sleep, so only the entry and exit can add time), but no timestamp exists for that sample: the benchmark should log the wall-clock time of every over-1 ms sample so it can be matched against `pmset -g log` | 0 |
| The Linux and Windows core-topology parsers of `lactose-task` are unit-tested and cross-linted only; they first run for real on CI and the x86 desktop | 0 |
| Tick-start latency against the 20 µs P99 criterion. After D116 (the keeper runs the task it fires, injector guard) interleaved A/B runs at 60 Hz give P50 3 to 5 µs and P99 23 to 30 µs against P99 41 to 44 µs before, with 0 or 1 of 3,000 deadlines over 1 ms; the reviewer measured the same build at P99 5.7 µs in one state of this machine and 22 to 24 µs in another at similar load, cause unknown. Not demonstrated: needs a run on a quiet machine, and the x86 desktop numbers. The full benchmark on the merged tree (`docs/results/whisk-apple-m1-2026-10-09.md`, UTC date, commit `00f1f3d`) measures P50 3.7 µs, P99 24.5 µs, max 46.3 µs, none of 5,000 over 1 ms, at load 3.7 falling to 1.7. Known remaining costs: when other timers are pending the keeper still unparks a sleeper to take over the watch before it runs the kept task, and each re-registration wakes a sleeper to become the keeper | 0 |
| The executor's idle-CPU regression test (`tests/idle_cpu.rs`) uses `getrusage` and is Unix-only; Windows relies on the loom models for the same logic | none |
| A CI grep stops any crate but `lactose-task` naming loom; fold it into `cargo xtask features` together with the `conformance` rule above | 1 |
| Whether Apple cores implement FEAT_AFP. `fp::is_deterministic` checks the alternate-mode bits either way | none |
