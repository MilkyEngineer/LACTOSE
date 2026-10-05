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
| loom's current limitations (SeqCst modelled as AcqRel, no load buffering) and feasible preemption bound for the latch and sleep model. | 0. **Done:** confirmed on loom 0.7.2; the sleep protocol uses explicit `fence(SeqCst)` (D110). Eight of the ten models are exhaustive and the two three-thread models are bounded at 5 preemptions; all ten run in about 45 s in release |
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
| `lactose-reflect` field impls for the `lactose-math` types (`Scalar` as `ScalarField`, vectors as structs of scalars) are added with the `ecs` facade | 0 |
| Wake latency at tick start against the 20 µs P99 criterion: the executor measures P50 11.8 µs and P99 31.3 µs on an M1 (provisional, loaded machine), and bare `std::thread::park`/`unpark` on the same machine has a P99 of 42 to 115 µs, so the executor is at the operating-system floor. A spike looks for a way under 20 µs without a permanently spinning core; if none exists the criterion is revisited with x86 desktop numbers | 0 |
| The Linux and Windows core-topology parsers of `lactose-task` are unit-tested and cross-linted only; they first run for real on CI and the x86 desktop | 0 |
| A CI grep stops any crate but `lactose-task` naming loom; fold it into `cargo xtask features` together with the `conformance` rule above | 1 |
| Whether Apple cores implement FEAT_AFP. `fp::is_deterministic` checks the alternate-mode bits either way | none |
