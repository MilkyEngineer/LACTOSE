# LACTOSE Verification

What must be measured or re-checked before a milestone can rely on it. `docs/architecture.md`
is the binding design and its milestone table (section 22) holds the proof criteria; this file
lists the spikes, the third-party claims and the open items those milestones depend on, each
with the milestone that gates it.

## 1. Spikes

| Spike | What it records | Gates milestone |
|---|---|---|
| io_uring on CI runners | Whether io_uring is available on the CI runners | 0 |
| Clippy on primitive methods | Whether `disallowed-methods` matches inherent primitive methods on the pinned toolchain | 0 |
| Scalar precision | Simulation scalar precision at full map extent | 1 |
| MoltenVK and KosmicKrisp | Indirect-draw behaviour, encode time for ten thousand draws, pipeline creation cost, memoryless attachments and local reads | 2 |
| Extract cost | Per-dirty-row extract cost | 2 |
| Chunk size | Whether a fixed 16 KiB chunk beats per-archetype sizing | 1 |
| Prediction-world snapshot cost | Full-copy cost per tick, deciding whether copy-on-write is built | 1 |

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

## 3. Open items

| Item | Gates milestone |
|---|---|
| Player portability between hosts | 4 (decided-later table, architecture section 22.1) |
| Multi-process server sharding: out of scope, kept possible by region ownership and command-based cross-region interaction (architecture section 9.6) | none |
