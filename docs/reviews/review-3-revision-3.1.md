# Adversarial review 3, against revision 3.1

Ranked most severe first. Condensed from the reviewer's report; section numbers refer to revision 3.1.

1. BLOCKING — The identity model breaks for predicted spawns. Every reference type is keyed by `NetId`, so an unconfirmed predicted spawn cannot be referenced, related or rendered; the ordinal is undefined; the ordering and RNG key switches type at bind; rejected spawns have no lifecycle. Recommended: a `StableKey` enum, origin key recorded by the server, ordinal from the total command order, destruction on input acknowledgement, alias table in the snapshot.
2. BLOCKING — The ECS system API is unspecified although every agent writes against it from week one. Continuation completion forces per-entity kernels; exclusive systems, cross-entity writes, physics writes, event emission, command order and the key hash are all undecided. Recommended: an API section with worked code.
3. BLOCKING — One last-seen tick per client cannot represent partial sends under a bandwidth cap, and the server has no defined replication read phase. Recommended: per-client per-entity acked ticks with a packet ring, a replication gather phase, short wire handles.
4. BLOCKING — Clients build regions from the unmodified cooked baseline and would instantiate destroyed cooked entities; region moves recorded as `SetField` lose the entity; placement-index IDs are fragile; `persist` cannot reach the baseline under the tier rule. Recommended: overlay sent on relevance, `Destroy` plus `SpawnFromBaseline`, authored GUIDs, a foundation bundle crate, migrated residents' assets in region sets.
5. SERIOUS — The prediction world is underspecified: system selection, coexistence with the replicated world, presentation collision queries during the overlap, event re-emission on rollback, proxy state during re-simulation, stale Rapier state after correction.
6. SERIOUS — The physics phase requires a blocking wait the executor forbids, "parking" is not a mechanism, the phase is a full barrier, rayon threads get no QoS or FP setup, and `Frozen` bodies are undefined.
7. SERIOUS — The coarse chunk tick is a shared write on split jobs; structural changes have no tick; the tick unit is undefined.
8. SERIOUS — No explicit component type IDs; a registry-wide schema hash refuses every client because client and server register different types.
9. SERIOUS — Cargo feature unification can make shipped binaries differ numerically from the SOUR build, and the lockfile-based build ID cannot see it.
10. SERIOUS — A `Scalar` type alias exposes every platform-library transcendental; the 32/64-bit switch is a crate switch; the 64-bit cost argument ignored client rollback on NEON.
11. SERIOUS — Float reductions cannot be both deterministic and cheap as specified; no float combiner is order-independent.
12. SERIOUS — `NetId` is not persisted, so the post-load discrete-state test fails by construction; two server counters are redundant.
13. SERIOUS — No server capacity model: per-region chunks fragment with scattered players, no overload policy, no server IO sizing, undefined cross-boundary movement, packet parsing at utility QoS, asset timing not journaled.
14. SERIOUS — The client's dominant render data path, remote entities, bypasses the specified pipeline; interpolation needs three snapshot buffers.
15. SERIOUS — Boundary and tier statements disagree after the split; `game-content` and leaf crates have no tier; `reflect` hashing sits outside the boundary.
16. SERIOUS — One `app` crate links every client backend into the Linux server; graph assembly and VFX ownership are ambiguous.
17. SERIOUS — The milestone 4 correction threshold is unattainable without redundant inputs and client-side input quantisation; the milestone 1 spike needs networking not listed.
18. SERIOUS — The three tracks share unlisted dependencies; most milestone 0 and 1 proofs have no pass condition; a `task` agent lacks its inputs.
19. SERIOUS — The world hash as specified costs tens of milliseconds and the SOUR matrix exceeds the PR budget.
20. SERIOUS — The timer root's hitch slowdown conflicts with clock sync on the client and is the de facto overload policy on the server.
21. SERIOUS — Non-indexed cluster draws lose post-transform vertex reuse; per-pipeline binning, per-cluster LOD and skinned bounds are unspecified; motion vectors as a pass type double geometry work. Disagrees with review 2's prescription.
22. SERIOUS — The transient poison test detects almost nothing.
23. SERIOUS — Tooling: a global abort hook breaks expected-panic tests; loom cannot model a deque; `RUSTFLAGS` drops the pin and build scripts compile for the host; Miri needs nightly; per-job spans are too expensive.
24. MINOR — Component trait bounds work but need padding markers, verified enum support and no generic components in game crates.
25. MINOR — Child jobs cannot all live on the opener's stack; the opener needs a deque watermark.
26. MINOR — Traceability, the decisions log and verification contradict revision 3.1 in places.
27. MINOR — A stale `CLAUDE.md` reached the review session.
28. MINOR — Unsafe needs in `platform`, `audio` and `physics-rapier`; the `rhi-*` glob exempts `rhi-api`; SOUR modes must be runtime options.
29. MINOR — GPU free list versus CPU slot table; light binning wording; boom collision before latch; culling pad unnecessary.
30. MINOR — Console assumptions baked into milestone 0 interfaces.
