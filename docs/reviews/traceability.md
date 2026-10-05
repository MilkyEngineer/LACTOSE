# Review traceability

One row per finding of `review-1-revision-1.md` (R1, 24 findings), `review-2-revision-2.md`
(R2, 30 findings) and `review-3-revision-3.1.md` (R3, 30 findings), against revision 3.2 of
`docs/architecture.md` (arch §), `docs/decisions-log.md` (log D) and `docs/verification.md`
(verif §).

Status is strict.

- **addressed**: the architecture document (or, where the finding is about them, `CLAUDE.md`,
  `crate-tiers.toml` or `docs/verification.md`) states the change, and no other statement in
  revision 3.2 contradicts it.
- **partially addressed**: some of the change is stated and some is not, or another statement in
  revision 3.2 still contradicts it. The note names what is missing.
- **rejected with reason recorded in decisions-log**: the requested change is not made, and the
  decisions log records why.
- **superseded by R*n*-*m***: a later finding explicitly reversed something this finding
  prescribed, and the architecture now states the later finding's alternative. The note says
  which parts of the earlier finding still stand and cites both entries. A later finding that
  only refines, completes or merges an earlier resolution, or that reverses a revision's own
  choice rather than the finding's prescription, does not supersede it; the note names it.

Rows not marked addressed are in bold.

## 1. Review 1

| Review | # | Title | Status | Where addressed | Note |
|---|---|---|---|---|---|
| R1 | 1 | Scoped async spawning is unsound | addressed | arch §5.1; log D14 | |
| R1 | 2 | Dependency rule contradicts the layout | addressed | arch §3, §4, `crate-tiers.toml`; log D62 | |
| R1 | 3 | Determinism depends on chunk layout | **superseded by R2-1** | arch §5.2, §7, §8.2, §8.3, §22; log D23, D24, D30, D5 | Indices unreachable, per-entity RNG and a layout mode gating milestone 1 stand. Its "order by entity ID" and entity-keyed RNG were reversed by R2-1 (process-local IDs differ between machines): ordering and RNG use the `StableKey` origin key (D5, D24). R2-15 also replaced ID-changing layout perturbations with identity-preserving ones (D30). |
| R1 | 4 | Snapshot ignores state outside chunks | addressed | arch §10, §16, §22 M1; log D40 | |
| R1 | 5 | Region bits in entity IDs block region crossing | **superseded by R2-8** | arch §6, §7, §16; log D6, D58 | The ID without region bits and region membership as a component stand (D6). Its "physics per active area" was reversed by R2-8: one physics world per server process (D58). |
| R1 | 6 | Padding in hashed bytes; plain-data bound unstated | **superseded by R3-19** | arch §6, §8.1, §8.3; log D9, D29 | Padding-free bounds, NaN canonicalisation and the variable-length rule stand. Its "hash via reflection" was reversed by R3-19 (reflection-dispatched hashing cost tens of milliseconds): monomorphised per-type hashers in `sim`, with `reflect` off every determinism path (D29). |
| R1 | 7 | Rapier determinism excludes parallel and SIMD | **superseded by R2-8** | arch §8.1, §8.2, §16; log D27, D58, D60; verif §2 | `physics-rapier` inside the boundary and one shared `libm` stand. Its serial-per-world premise was shown false by R2-8: Rapier 0.35 combines determinism with the parallel solver (D58). |
| R1 | 8 | FMA contraction flags do not exist | **superseded by R2-11** | arch §1 goal 2, §8.2; log D1, D26 | The false flag claim is gone. Its listing of `mul_add` as a hazard, which revision 2 turned into a ban, was reversed by R2-11: fused multiply-add is correctly rounded and the hazard is target-feature selection, so the x86-64-v3 pin is enforced instead (D26). R2-7 also disagreed with its framing of cross-architecture determinism (D1). |
| R1 | 9 | Custom lint crate conflicts with stable toolchain | addressed | arch §8.2, §8.3; log D31 | |
| R1 | 10 | Change ticks at the wrong granularity | addressed | arch §6; log D10 | Per-row tracking through `Mut<T>` and tick rebasing after restore are stated. Revision 2's per-row bit became a per-row tick under R2-3; this finding asked only for per-row tracking. |
| R1 | 11 | Late-latch contradictory | **superseded by R2-17** | arch §5.3, §12.4, §13; log D48 | One GPU buffer latched after CPU view work stands. Its "stamp inputs with the yaw displayed" was reversed by R2-17 (absolute yaw and pitch), and its GPU-culling pad by R3-29 (culling reads the latched buffer) (D48). |
| R1 | 12 | RHI overstates commonality; no shader language | **superseded by R2-5** | arch §1, §2, §12.1, §12.2, §12.4; log D3, D42, D44; verif §1 | Slang, the MoltenVK spike, reported capabilities and the Intel Mac non-goal stand. Its "zero-instance draws primary" was reversed by R2-5, whose own prescription R3-21 then reversed (D42). |
| R1 | 13 | Animation and gameplay particles outside the boundary | addressed | arch §8.1, §17; log D61 | R2-25 extended it with a sim skeleton. |
| R1 | 14 | Asset versions, hot reload and client gating | addressed | arch §6, §8.2, §9.3, §14; log D13, D39, D52 | |
| R1 | 15 | Save/load test incomplete; overlay undefined | **superseded by R2-12** | arch §7, §15.2, §15.5; log D7, D55, D57 | Simulating K ticks after load and explicit field IDs stand. Its overlay of absolute values keyed by a source-GUID `PersistentId` was reversed by R2-12 (typed records, placement-derived IDs), and R3-12 later removed `PersistentId` altogether (D7, D55). |
| R1 | 16 | kqueue is not completion IO; io_uring missing in CI | addressed | arch §4, §14; log D53; verif §1 | |
| R1 | 17 | Interface-first deadlocks on backend crates | addressed | arch §4, §20; log D65, D66 | `io` is not a slice with its own `-api`: it has none, and its backends sit behind a trait inside the crate (§20), which removes the deadlock the finding named. |
| R1 | 18 | Rollback window under-specified | **superseded by R2-6** | arch §9.4; log D32, D33 | Predicted entities colliding only with static geometry and each other stands. Its RTT-sized ring with a full-area resync was reversed by R2-6: depth derived from the re-simulation budget, and resync rebuilds the prediction world from replicated state (D33). |
| R1 | 19 | Extract cannot overlap the tick it reads | **superseded by R2-18** | arch §5.3; log D21 | Extract no longer overlaps the tick it reads. Its "serial phase" with double-buffered GPU transforms was reversed by R2-18: exclusive but internally parallel, writing a staging ring that a GPU scatter pass applies (D21). |
| R1 | 20 | Milestones prove the wrong things | addressed | arch §2, §22; log D66, D67 | |
| R1 | 21 | Decisions missing entirely | addressed | arch §3, §5.1, §9.1, §16, §17, §18, §19, §20, §22.1 | Logging (§19: `tracing` as the facade, codename log targets) and CI cost (§20: fifteen-minute pull-request budget, cost recorded per run) are now stated; deferred items carry a milestone in §22.1. |
| R1 | 22 | Small contradictions | addressed | arch §3, §5.1, §5.2; log D17, D19 | |
| R1 | 23 | Uber-shader decided without lighting | addressed | arch §12.5; log D46, D47 | |
| R1 | 24 | Render graph cache key undefined | addressed | arch §12.3 | |

## 2. Review 2

| Review | # | Title | Status | Where addressed | Note |
|---|---|---|---|---|---|
| R2 | 1 | Process-local IDs used as cross-machine keys | addressed | arch §2 Identity, §7, §8.2, §10; log D5, D7, D24 | Every reference, ordering rule and RNG key is a `StableKey`; R3-1 completed this for unconfirmed predicted spawns. R3-12 merged the third identity, `PersistentId`, into `NetId`, which keeps a persistent ID rather than reversing the need for one. |
| R2 | 2 | Help-while-waiting lengthens the critical path | **superseded by R3-25** | arch §5.1; log D15 | Continuation completion, same-scope help and never running background work while waiting stand. Its "stack-allocated child jobs" was reversed by R3-25: a per-worker bump arena and a deque watermark (D15). |
| R2 | 3 | One dirty bit cannot serve many consumers | **superseded by R3-3** | arch §6, §9.3; log D10, D76 | Per-row ticks, extract's last-seen tick, `set_if_neq` and no change tracking for saves stand. Its single acked tick per client with "no per-packet bookkeeping" was reversed by R3-3: per-client, per-entity acked ticks with a packet-to-entity ring (D10, D76). |
| R2 | 4 | Camera-relative positions rewrite the GPU scene | addressed | arch §2 World coordinates, §12.4; log D41 | Cell plus offset, the render-space type and the conversion list (swept bounds, light grid, decals, probes, VFX emitters, cascades, clipmap, sky and atmosphere, markers, audio listener and emitters, GPU picking, debug draw, the camera) are stated. Particle state is covered only as "VFX emitters". |
| R2 | 5 | MoltenVK multi-draw-indirect is CPU-bound | **superseded by R3-21** | arch §2 Draw submission, §12.1, §12.4, §22 M3; log D42, D43; verif §1, §2 | One draw per pipeline with no indirect-count, native Metal and the extended spike stand; native Metal lands in milestone 3, not before it. Its programmable vertex pulling over fixed-vertex-count clusters was reversed by R3-21: a compacted 32-bit index buffer and one indexed indirect draw per pipeline (D42). |
| R2 | 6 | Predicted area has two writers; cost unbudgeted | **rejected with reason recorded in decisions-log** | arch §9.4, §10, §23 COW; log D32, D33, D34; verif §1 | The prediction world, derived ring depth and resync from replicated state are stated. Copy-on-write "from day one" is rejected: with a world holding only predicted entities a full copy is small, so COW is built only if the milestone 1 snapshot measurement exceeds its budget (D34). |
| R2 | 7 | Cross-architecture bit-exactness does not deliver Goal 2 | **superseded by R3-17** | arch §1 goal 2, §2, §9.4, §22 M4; log D1, D77 | Goal 2 as an engineering property and the milestone 4 replay hash plus correction rate (fewer than one per minute) stand. Its "compare in quantised space" was reversed by R3-17: full-precision reconciliation (D1, D77). Arch §2 Determinism scope still says "Reconciliation is defined in quantised space". |
| R2 | 8 | Rapier premise out of date; merge/split costs too much | addressed | arch §2 Physics, §16; log D58, D59, D82; verif §2 | R3-6 replaced revision 3.1's parking workers with a sanctioned blocking wait into a smaller rayon pool (D82), and R3-10 refined the precision test (D59). Neither reverses this finding. |
| R2 | 9 | Fork-join abort is not unconditional | **superseded by R3-23** | arch §5.1; log D16, D74 | Closure-only scope, abort guards around every job and join wait, no blocking waits on workers, nextest and loom on the sleep-wake protocol stand. Its process-wide abort hook was reversed by R3-23: the hook is installed by binaries only, so abort is unconditional in shipped binaries but not in tests (D16). |
| R2 | 10 | Milestone order inverts risk | addressed | arch §20, §22; log D43, D66 | R3-18 moved `asset-api` further forward, into milestone 0. |
| R2 | 11 | Float rules wrong in specifics | addressed | arch §5.1, §8.2, §8.3; log D25 to D28, D75 | `min`, `max`, `clamp`, `powi`, `total_cmp`, `to_bits`, `copysign`, `is_sign_negative` and trait routes such as `num_traits::Float` are banned outside `math`; the renderer losing glam SIMD is accepted explicitly (§8.2). R3-9 and R3-10 extended this. |
| R2 | 12 | Save overlay cannot represent most changes | addressed | arch §6, §7, §15.2; log D7, D8, D12, D55, D81 | Record kinds, kept runtime entities, owner migration, typed references and placement-derived IDs assigned at apply are stated. R3-4 added `SpawnFromBaseline` for moves and authored GUIDs; R3-12 made `NetId` the persistent ID. |
| R2 | 13 | Save/load test contradicts the transient policy | **superseded by R3-22** | arch §6, §15.1, §15.3, §15.5; log D54, D56, D57 | The transient definition, persisted accumulated state, in-place reload, the lighter save and the comparison against the un-reloaded original stand. Its poison test was reversed by R3-22: structural zeroing and static reader ordering (D54). R3-12 persisted `NetId` so the discrete-state comparison can pass (D57). |
| R2 | 14 | Command order is not total | **superseded by R3-2** | arch §5.2, §6.1, §7, §10; log D20, D24, D40 | Placeholders resolved at apply, declared-name system keys and the snapshot fix stand. Its total order led by the system stable key was reversed by R3-2: the stage's execution order first, then (source key, sequence) (D20). |
| R2 | 15 | `world_hash` order and layout mode | addressed | arch §5.2, §8.3; log D23, D29, D30 | Revision 3.2 uses the order-independent combiner this finding offered as its alternative; R3-19 removed revision 3.1's `NetId`-ordered variant, not this finding's prescription. |
| R2 | 16 | No route to presentation | **superseded by R3-14** | arch §4, §5.3, §13; log D50, D80 | `present-api`, the presentation snapshot and the per-frame schedule stand. Its double-buffered snapshot was reversed by R3-14: triple-buffered (D80). Arch §2 Latency still says "double-buffered". |
| R2 | 17 | Rotation-only late-latch wrong for an orbit camera | **superseded by R3-29** | arch §5.3, §12.5, §13; log D48 | The presentation camera rig, absolute yaw and pitch, the full orbit latch, GPU light binning after the latch, padded cascades and clamped deltas stand. Its culling pad was reversed by R3-29: GPU culling reads the latched buffer and needs none (D48). |
| R2 | 18 | Extract and GPU scene mechanics | addressed | arch §5.3, §12.4, §22 M2; log D21, D49 | |
| R2 | 19 | `Mut<T>` dirty bitmask races when a chunk splits | addressed | arch §6; log D10 | The dirty-range out-parameter and `set_if_neq` are stated. Per-row ticks replace the bitmask and §6 states that each row's tick is its own word, so jobs split at any row without a shared write; the 64-row split rule is unnecessary. R3-7 closed the remaining coarse-tick race. |
| R2 | 20 | Tier and policy tables contradict | addressed | arch §3, §4, §8.1, §21, `crate-tiers.toml`; log D22, D53, D62 to D64, D71 | |
| R2 | 21 | Network data path undefined; lag compensation PvP-only | addressed | arch §9.2 to §9.5; log D35 to D37 | The interpolation buffer is sampled per frame by presentation through a `net-api` resource rather than feeding the snapshot as proposed; the reason is recorded in D35. |
| R2 | 22 | Time and pacing model missing | addressed | arch §5.1, §9.2, §11; log D18, D35 | Modal event loops are not named, but neither ticking nor present depends on the event pump. |
| R2 | 23 | Executor ignores P/E cores; milestone 0 metrics | addressed | arch §5.1, §22; log D17, D67 | Frame-critical workers equal the performance-core count on every platform. |
| R2 | 24 | `AnyBitPattern` forbids ordinary data | addressed | arch §6, §7; log D6, D9 | |
| R2 | 25 | Pose evaluation outside the boundary leaves no bones | **partially addressed** | arch §4, §13, §17, §22 M1; log D61 | The sim skeleton in `anim` is stated. "Budget it in M1" became "sim-skeleton evaluation measured"; milestone 1 sets no threshold for it. |
| R2 | 26 | No error strategy; device loss unrecoverable | addressed | arch §3, §12.1, §14, §19; log D51, D70 | |
| R2 | 27 | Shader toolchain claims | addressed | arch §12.1, §12.2; log D44, D45; verif §2 | |
| R2 | 28 | Permutation axes omit pass variants; tile memory | addressed | arch §12.1, §12.5; log D46, D47; verif §1 | Pass type is an axis with a stated budget. Motion vectors, listed here as a missing pass variant, became a G-buffer target under R3-21 (D47). |
| R2 | 29 | 30-second build budget needs specific settings | addressed | arch §18, §22 M1; log D68 | |
| R2 | 30 | Numbers without derivation; protocol hash too narrow | addressed | arch §6, §8.2, §8.3, §9.1; log D11, D38, D52 | |

## 3. Review 3

| Review | # | Title | Status | Where addressed | Note |
|---|---|---|---|---|---|
| R3 | 1 | Identity model breaks for predicted spawns | addressed | arch §2 Identity, §6.1, §7, §8.2, §9.4, §10, §12.4; log D5, D24, D40 | |
| R3 | 2 | ECS system API unspecified | addressed | arch §5.2, §6.1, §20; log D20, D73 | |
| R3 | 3 | One last-seen tick per client; no replication read phase | addressed | arch §6, §9.3; log D10, D76 | |
| R3 | 4 | Clients build regions from the unmodified baseline | addressed | arch §4, §7, §9.3, §14, §15.2, `crate-tiers.toml`; log D7, D55, D81 | |
| R3 | 5 | Prediction world underspecified | addressed | arch §6.1, §9.4, §10, §13; log D32, D40 | |
| R3 | 6 | Physics phase needs a forbidden blocking wait | addressed | arch §5.1, §9.7, §16, §22 M1; log D82 | |
| R3 | 7 | Coarse chunk tick races; structural changes untracked | addressed | arch §6; log D10 | |
| R3 | 8 | No explicit component keys; schema hash refuses clients | addressed | arch §6, §7, §9.1; log D38 | |
| R3 | 9 | Feature unification can change shipped numerics | addressed | arch §8.3, §9.1; log D38, D75 | |
| R3 | 10 | `Scalar` alias exposes platform transcendentals | addressed | arch §2 World coordinates, §4, §8.2, §16; log D59 | |
| R3 | 11 | Float reductions cannot be deterministic and cheap | addressed | arch §5.2, §6.1; log D23 | |
| R3 | 12 | `NetId` not persisted; two server counters | addressed | arch §2 Identity, §7, §15.5; log D7, D57 | |
| R3 | 13 | No server capacity model | **partially addressed** | arch §5.1, §6, §9.3, §9.7; log D78 | Activation, archetype-owned chunks, the overload ladder, server IO, cross-boundary movement and journaled asset timing are stated, and §5.1 and §9.7 move packet parsing to a dedicated network thread. §9.2 still says received packets "are parsed on background workers". |
| R3 | 14 | Remote entities bypass the render pipeline | **partially addressed** | arch §5.3, §22 M2; log D80 | The upload path, the lock-free buffer with batch sampling and the triple-buffered snapshot are stated in §5.3. §2 Latency still says "A double-buffered presentation snapshot". |
| R3 | 15 | Boundary and tier statements disagree | addressed | arch §4, §8.1, §8.2, `crate-tiers.toml`; log D22 | |
| R3 | 16 | One `app` crate links client backends into the server | addressed | arch §2 Binaries, §4, §5.3, §12.5, `crate-tiers.toml`; log D83 | |
| R3 | 17 | Correction threshold unattainable without redundant inputs | **partially addressed** | arch §2 Netcode, §9.2, §9.4, §22 M1, §22 M4; log D1, D66, D77 | Redundant inputs, quantised client inputs, full-precision reconciliation (§2 Netcode, §9.4), the throwaway spike transport and fifty claimed bodies are stated. §2 Determinism scope still says "Reconciliation is defined in quantised space". |
| R3 | 18 | Tracks share unlisted dependencies; proofs lack thresholds | **partially addressed** | arch §20, §22; log D66, D67 | Milestone 0 ownership, the `task` agent's inputs and the native Metal spike are stated, and most proofs gained thresholds. Milestone 1's "sim-skeleton evaluation measured" and the netcode spike's recorded correction rate, rollback cost and divergence still have no pass condition. |
| R3 | 19 | World hash too expensive; SOUR matrix over budget | addressed | arch §8.1, §8.3, §20; log D29, D30 | |
| R3 | 20 | Hitch slowdown conflicts with clock sync and overload | addressed | arch §5.1, §9.2, §9.7, §11; log D18, D78 | |
| R3 | 21 | Non-indexed cluster draws lose vertex reuse | addressed | arch §2 Draw submission, §12.4, §12.5, §22 M3; log D42, D47 | Reverses R2-5's prescription. |
| R3 | 22 | Transient poison test detects almost nothing | addressed | arch §6, §15.1; log D54 | |
| R3 | 23 | Tooling: abort hook, loom, `RUSTFLAGS`, Miri, spans | addressed | arch §5.1, §8.2, §8.3, §20, `crate-tiers.toml`, `CLAUDE.md`; log D16, D26, D31, D74 | |
| R3 | 24 | Component bounds need padding markers and enum checks | addressed | arch §6; log D9 | |
| R3 | 25 | Child jobs cannot all live on the opener's stack | addressed | arch §5.1; log D15 | |
| R3 | 26 | Traceability, log and verification contradict revision 3.1 | **partially addressed** | this file; `docs/verification.md`; `CLAUDE.md`; log §10 and the entries | Revision 3.2 recorded no decision for it. A later editorial pass regenerated this file and folded the log's rationale tables into the entries; `docs/verification.md` and `CLAUDE.md` agree with revision 3.2. Three contradictions inside architecture.md remain (§2 Determinism scope against §9.4, §2 Latency against §5.3, §9.2 against §5.1 and §9.7), so the log cannot agree with every line of it. |
| R3 | 27 | A stale `CLAUDE.md` reached the review session | addressed | `CLAUDE.md` (revision marker and HEAD check); log D85 | The harness again supplied a pre-3.1 `CLAUDE.md` to the editorial pass that produced this file; the marker check exposed it and the file on disk was used. |
| R3 | 28 | Unsafe needs; `rhi-*` glob; SOUR runtime options | addressed | arch §5.1, §8.3, §21, `CLAUDE.md`; log D30, D84 | |
| R3 | 29 | Slot table, light binning, boom collision, culling pad | addressed | arch §5.3, §12.4, §12.5; log D48, D49 | |
| R3 | 30 | Console assumptions in milestone 0 interfaces | addressed | arch §4, §22; verif §3; log D86 | |

## 4. Summary

| Status | Review 1 | Review 2 | Review 3 | Total |
|---|---|---|---|---|
| addressed | 14 | 19 | 25 | 58 |
| partially addressed | 0 | 1 | 5 | 6 |
| rejected with reason recorded in decisions-log | 0 | 1 | 0 | 1 |
| superseded by a later finding | 10 | 9 | 0 | 19 |
| **Total** | **24** | **30** | **30** | **84** |

Not marked addressed (26):

- Superseded (19): R1-3 by R2-1; R1-5 by R2-8; R1-6 by R3-19; R1-7 by R2-8; R1-8 by R2-11;
  R1-11 by R2-17; R1-12 by R2-5; R1-15 by R2-12; R1-18 by R2-6; R1-19 by R2-18; R2-2 by R3-25;
  R2-3 by R3-3; R2-5 by R3-21; R2-7 by R3-17; R2-9 by R3-23; R2-13 by R3-22; R2-14 by R3-2;
  R2-16 by R3-14; R2-17 by R3-29.
- Rejected with reason recorded (1): R2-6 (copy-on-write from day one; log D34).
- Partially addressed (6): R2-25, R3-13, R3-14, R3-17, R3-18, R3-26.
