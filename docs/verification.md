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
| `wide` and glam select fused or unfused multiply-add by target feature. | 0 |
| `libm`'s x86 runtime detection selects only correctly rounded paths. | 0 |
| Whether Rapier, parry or its math crates call `wide::mul_add` on solver paths. | 1 |
| Rapier: its determinism feature combines with its parallel solver with bit-identical results for any pool size; SIMD is always on; the 64-bit build has the determinism feature; its math moved to glam-based crates. Whether a single active set makes results depend on unrelated awake bodies is tested by stepping a scene with and without a distant awake body. | 1 |
| MoltenVK multi-draw-indirect is a CPU loop. | 2 |
| Slang's Metal target supports argument buffers, function constants and framebuffer fetch. | 2 |
| Apple M1 lacks 64-bit atomics. | 3 |
| glam or its fork has a feature (such as fast-math) that changes numerics on the pinned version; if so it goes on the cargo-deny denylist. | 0 |
| `bytemuck_derive` on the pinned version: support for data-carrying enums under `NoUninit` and `CheckedBitPattern`, and whether a `PodInOption` derive exists for `NetId`. | 0 |
| loom's current limitations (SeqCst modelled as AcqRel, no load buffering) and feasible preemption bound for the latch and sleep model. | 0 |
| `tracing-tracy` per-span overhead, to confirm per-system and per-range spans fit the budget. | 0 |
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
