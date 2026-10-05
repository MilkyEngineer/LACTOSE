# Review traceability

One row per finding of `review-1-revision-1.md` (R1, 24 findings) and `review-2-revision-2.md`
(R2, 30 findings), against revision 3 as split into `docs/architecture.md` (arch §),
`docs/decisions-log.md` (log D) and `docs/verification.md` (verif §).

Status is strict. **addressed** means the architecture document states the change the finding
asked for, or states a replacement whose reason the decisions log records. **partially
addressed** means some requested change is stated and some is not; the note names what is
missing. Rows not marked addressed are in bold.

## 1. Review 1

| Review | # | Title | Status | Where addressed | Note |
|---|---|---|---|---|---|
| R1 | 1 | Scoped async spawning is unsound | addressed | arch §5.1; log D14 | |
| R1 | 2 | Dependency rule contradicts the layout | addressed | arch §3, `crate-tiers.toml`; log D62 | |
| R1 | 3 | Determinism depends on chunk layout | addressed | arch §5.2, §8.2, §8.3, §22; log D23, D24, D30 | |
| R1 | 4 | Snapshot ignores state outside chunks | addressed | arch §10, §16; log D40 | |
| R1 | 5 | Region bits in entity IDs block region crossing | addressed | arch §6, §7; log D6 | Physics-per-area recommendation later superseded by R2-8 (log D58). |
| R1 | 6 | Padding in hashed bytes; plain-data bound unstated | addressed | arch §6, §8.2, §8.3; log D9, D29 | |
| R1 | 7 | Rapier determinism excludes parallel and SIMD | addressed | arch §8.1, §8.2, §16; log D58, D60, D27; verif §2 | Serial-per-world recommendation superseded by R2-8; reason in log D58. |
| R1 | 8 | FMA contraction flags do not exist | addressed | arch §8.2; log D26, D1 | Its `mul_add` ban was later reversed by R2-11 (log D26). |
| R1 | 9 | Custom lint crate conflicts with stable toolchain | addressed | arch §8.2, §8.3; log D31 | |
| R1 | 10 | Change ticks at the wrong granularity | addressed | arch §6; log D10 | |
| R1 | 11 | Late-latch contradictory | addressed | arch §5.3, §12.4; log D48 | Input stamping by displayed yaw superseded by absolute yaw and pitch (R2-17). |
| R1 | 12 | RHI overstates commonality; no shader language | addressed | arch §2, §12.1, §12.2; log D3, D42, D44; verif §1 | Zero-instance primary path superseded by R2-5 (log D42). |
| R1 | 13 | Animation and gameplay particles outside the boundary | addressed | arch §8.1, §17; log D61 | |
| R1 | 14 | Asset versions, hot reload and client gating | addressed | arch §6, §8.2, §14; log D13, D39, D52 | |
| R1 | 15 | Save/load test incomplete; overlay undefined | addressed | arch §7, §15.2, §15.5; log D7, D55, D57 | GUID-derived `PersistentId` superseded by R2-12 (log D7). |
| R1 | 16 | kqueue is not completion IO; io_uring missing in CI | addressed | arch §4, §14; log D53; verif §1 | |
| R1 | 17 | Interface-first deadlocks on backend crates | addressed | arch §4, §20; log D65, D66 | |
| R1 | 18 | Rollback window under-specified | addressed | arch §9.4; log D32, D33 | |
| R1 | 19 | Extract cannot overlap the tick it reads | addressed | arch §5.3; log D21 | |
| R1 | 20 | Milestones prove the wrong things | addressed | arch §2, §22; log D66, D67 | |
| R1 | 21 | Decisions missing entirely | **partially addressed** | arch §3, §5.1, §9.1, §17, §18, §19, §22.1 | CI cost is never decided. Logging is covered only implicitly by `tracing` spans and the `milkman` log target. |
| R1 | 22 | Small contradictions | addressed | arch §3, §5.1, §5.2; log D17, D19 | |
| R1 | 23 | Uber-shader decided without lighting | addressed | arch §12.5; log D46, D47 | |
| R1 | 24 | Render graph cache key undefined | addressed | arch §12.3 | |

## 2. Review 2

| Review | # | Title | Status | Where addressed | Note |
|---|---|---|---|---|---|
| R2 | 1 | Process-local IDs used as cross-machine keys | **partially addressed** | arch §7, §8.2, §10; log D5 | The review asked that every ordering and RNG key be `NetId` or a prediction key. Arch §2 and §8.2 name only `NetId`, and nothing says what keys an unconfirmed predicted spawn uses. CLAUDE.md lists `PredictionKey` as a stable key. |
| R2 | 2 | Help-while-waiting lengthens the critical path | **partially addressed** | arch §5.1; log D15 | Stack-allocated child jobs are not stated. |
| R2 | 3 | One dirty bit cannot serve many consumers | addressed | arch §6; log D10 | Depends on the editorial fix removing "each save" from the consumer list (see log D10). |
| R2 | 4 | Camera-relative positions rewrite the GPU scene | **partially addressed** | arch §12.4; log D41 | Cell plus offset and the render-space type are stated. The conversion list omits swept previous/current culling bounds, sky and atmosphere, audio emitters (only the listener is listed), GPU picking and the camera itself. |
| R2 | 5 | MoltenVK multi-draw-indirect is CPU-bound | addressed | arch §2, §12.1, §12.4; log D42, D43; verif §1, §2 | Native Metal lands in milestone 3, not before it. |
| R2 | 6 | Predicted area has two writers; cost unbudgeted | **partially addressed** | arch §9.4, §10; log D32, D33, D34 | Copy-on-write snapshots "from day one" are not stated; COW appears only in the glossary as "a measured optimisation". |
| R2 | 7 | Cross-architecture bit-exactness does not deliver Goal 2 | addressed | arch §1, §2, §9.4, §22; log D1 | Milestone 4's correction-rate threshold is "stated" but no number is given. |
| R2 | 8 | Rapier premise out of date; merge/split costs too much | addressed | arch §2, §16; log D58, D59; verif §2 | |
| R2 | 9 | Fork-join abort is not unconditional | addressed | arch §5.1; log D16 | |
| R2 | 10 | Milestone order inverts risk | addressed | arch §20, §22; log D43, D66 | |
| R2 | 11 | Float rules wrong in specifics | **partially addressed** | arch §5.1, §8.2, §8.3; log D25 to D28 | `min` and `max` are not in architecture.md's list of methods banned outside `math` (CLAUDE.md has them). Trait routes to platform libm (`num_traits::Float`, simba without `libm_force`) are not addressed. The renderer losing glam SIMD through feature unification is not addressed. |
| R2 | 12 | Save overlay cannot represent most changes | addressed | arch §6, §7, §15.2; log D7, D8, D12, D55 | |
| R2 | 13 | Save/load test contradicts the transient policy | **partially addressed** | arch §15.1, §15.3, §15.5; log D54, D56, D57 | The tolerance test for discrete state against the un-reloaded original is not stated. |
| R2 | 14 | Command order is not total | addressed | arch §5.2, §7, §10; log D20, D24 | |
| R2 | 15 | `world_hash` order and layout mode | addressed | arch §5.2, §8.3; log D23, D29, D30 | |
| R2 | 16 | No route to presentation | addressed | arch §4, §5.3, §13; log D50 | |
| R2 | 17 | Rotation-only late-latch wrong for an orbit camera | addressed | arch §5.3, §12.5, §13; log D48 | |
| R2 | 18 | Extract and GPU scene mechanics | addressed | arch §5.3, §12.4; log D21, D49 | |
| R2 | 19 | `Mut<T>` dirty bitmask races when a chunk splits | **partially addressed** | arch §6; log D10 | The dirty-range out-parameter and `set_if_neq` are stated. Splitting only at 64-row boundaries and register accumulation are not, and the document does not state that per-row ticks remove the race. |
| R2 | 20 | Tier and policy tables contradict | addressed | arch §3, §4, §8.1, §21, `crate-tiers.toml`; log D22, D53, D62 to D64, D71 | |
| R2 | 21 | Network data path undefined; lag compensation PvP-only | addressed | arch §9.2 to §9.5; log D35 to D37 | |
| R2 | 22 | Time and pacing model missing | addressed | arch §5.1, §9.2, §11; log D18, D35 | |
| R2 | 23 | Executor ignores P/E cores; milestone 0 metrics | addressed | arch §5.1, §22; log D17, D67 | Outside macOS the frame-critical count is "physical cores minus one", which on Intel hybrid CPUs still includes efficiency cores. |
| R2 | 24 | `AnyBitPattern` forbids ordinary data | addressed | arch §6, §7; log D6, D9 | |
| R2 | 25 | Pose evaluation outside the boundary leaves no bones | **partially addressed** | arch §4, §13, §17; log D61 | "Budget it in M1" is not stated; milestone 1's proof criteria do not mention the sim skeleton. |
| R2 | 26 | No error strategy; device loss unrecoverable | addressed | arch §3, §12.1, §14, §19; log D51, D70 | |
| R2 | 27 | Shader toolchain claims | addressed | arch §12.1, §12.2; log D44, D45; verif §2 | |
| R2 | 28 | Permutation axes omit pass variants; tile memory | addressed | arch §12.1, §12.5; log D46, D47; verif §1 | |
| R2 | 29 | 30-second build budget needs specific settings | addressed | arch §18; log D68 | |
| R2 | 30 | Numbers without derivation; protocol hash too narrow | addressed | arch §6, §8.2, §8.3, §9.1; log D11, D38, D52 | |

## 3. Summary

| Status | Review 1 | Review 2 | Total |
|---|---|---|---|
| addressed | 23 | 22 | 45 |
| partially addressed | 1 | 8 | 9 |
| not addressed | 0 | 0 | 0 |
| rejected with reason stated in doc | 0 | 0 | 0 |

Not marked addressed: R1-21, R2-1, R2-2, R2-4, R2-6, R2-11, R2-13, R2-19, R2-25.
