# Adversarial review 1, against revision 1

Ranked most severe first. Section numbers refer to revision 1 of architecture.md.

### 1. BLOCKING — §5.1 "Scoped spawning: `Scope::spawn` borrows from the parent's stack" is unsound in combination with "tasks are stackless Futures" and "tasks never block a thread"
A stackless async task cannot fork children that borrow its locals with a non-blocking parent. The parent future can be dropped or forgotten while children still hold `&'scope` references. Every sound async scoped spawn either requires unsafe at the call site, runs children only inside the parent's poll, or blocks the thread. Recommended: split into a blocking fork-join for chunk iteration (rayon model, closures not futures, cannot await) and `'static` futures for IO/GPU work.

### 2. BLOCKING — CLAUDE.md dependency rule contradicts §4 and §14 for nearly every crate
`game` depends on `ecs`, `sim`, `math`; `sim` on `ecs`; `net` on `sim`; `renderer` on `render-graph` and `ecs`. Recommended: explicit foundation and backend tiers in a machine-readable file.

### 3. BLOCKING — determinism defined "regardless of thread count" but results depend on chunk layout, and the client's predicted region cannot have the server's chunk layout
Reductions in chunk index order, per-chunk command buffers, scratch keyed by chunk index, RNG, physics insertion order. The client builds its region from a replication stream in a different order. Recommended: forbid chunk index as a semantic key; order by entity ID; RNG keyed by entity; add a SOUR mode that permutes chunk layout; gate milestone 1 on it.

### 4. BLOCKING — "snapshot(region) is a chunk copy and takes microseconds" ignores all state not in chunks
Rapier body sets, contact manifolds, warm-start impulses, island sleep state, joints; the relationship reverse index; in-flight command buffers. Recommended: define the snapshot as the union; expose physics snapshot/restore with a conformance test; measure cost in milestone 1.

### 5. BLOCKING — entity IDs encode an 8-bit region and regions never share chunks: entities cannot cross region boundaries
Crossing would require a new ID, invalidating every reference. 256-region cap too coarse. Recommended: 32-bit index plus 32-bit generation, region as a component, physics per active area rather than per region.

### 6. BLOCKING — `world_hash` over raw bytes of `repr(C)` components is nondeterministic due to padding, and the POD requirement is never stated
Padding bytes vary. Variable-length data has no home. Recommended: `bytemuck` bounds asserted by the derive; hash via reflection; decide the variable-length story; canonicalise NaNs.

### 7. SERIOUS — Rapier's determinism feature is incompatible with its parallel and SIMD features, and Rapier does not expose island solving to an external executor
Physics is a serial task per world. `physics-rapier` must be inside the boundary. `math` must use the same `libm` crate Rapier uses.

### 8. SERIOUS — "FMA contraction is disabled by compiler flags" names flags that do not exist in Rust
rustc never contracts on stable. Real hazards: `target-cpu=native`, `mul_add`, runtime feature detection, x86 vs aarch64. Cross-architecture is the default shipping configuration (Apple client, Linux server) and should be a goal.

### 9. SERIOUS — a custom lint crate conflicts with "stable toolchain"
Use clippy `disallowed-*` config and a `cargo-deny` allowlist. Provide a fixed-hash map newtype. Ban `Instant`/`SystemTime`, not `Duration`.

### 10. SERIOUS — change ticks per column per chunk are the wrong granularity
Sparse writes amplified ~100× in replication; extract is a scatter; `&mut` queries dirty every chunk visited; restoring older ticks breaks comparisons. Recommended: per-row dirty tracking through a `Mut<T>` wrapper; rebase ticks after restore.

### 11. SERIOUS — late-latch is contradictory about where it applies and what it races with
CPU culling, cascades and light binning used the un-latched view. Camera position latching conflicts with sim-side camera collision. Recommended: latch exactly one GPU buffer after CPU view work; pad GPU culling; stamp inputs with the yaw displayed.

### 12. SERIOUS — RHI "what Vulkan 1.3, DX12 and Metal 3 share" overstates commonality; no shader language chosen
Metal has no indirect-count; MoltenVK compiles SPIR-V to MSL at pipeline creation; Intel Mac argument buffer limits. Recommended: zero-instance draws primary; choose the shader toolchain before milestone 2; MoltenVK spike.

### 13. SERIOUS — animation blending and particles outside the boundary, yet both are gameplay
Climbing, hit frames, root motion, spreading fire are simulation. Add an `anim` crate; split pose evaluation from logic.

### 14. SERIOUS — asset versions and hot reload nondeterministic; client-side gating undefined
Resolve versions once per tick into a region resource; hot reload as a tick-stamped event; add a `Replicated` region mode; specify client gating.

### 15. SERIOUS — the save/load hash test does not prove completeness; overlay vs patched baseline undefined
Test by simulating K ticks after load. Overlay entries as absolute values keyed by PersistentId and stable field ID; PersistentId from source GUID; explicit field IDs.

### 16. SERIOUS — kqueue is not completion-based file IO on macOS; io_uring unavailable in Docker CI
Blocking pool as reference backend.

### 17. SERIOUS — interface-first plus one crate per task deadlocks on backend-driven crates
Vertical slices for `rhi`, `physics`, `io`, `net`; drop `task-api`; sanctioned path to amend an interface.

### 18. SERIOUS — rollback window under-specified for real round trips
12 ticks is 200 ms. Size from RTT with a cap and a full-resync path. Predicted entities collide only with static geometry and other predicted entities.

### 19. SERIOUS — extract cannot overlap the next tick if it reads chunks the tick writes
Extract is a serial phase; GPU scene double-buffers transforms.

### 20. SERIOUS — milestones prove the wrong things and omit prerequisites
Milestone 0 should prove critical-path latency; shader toolchain in milestone 2; terrain, animation, audio, input, UI missing; Linux is a server platform.

### 21. SERIOUS — decisions missing entirely
Audio, animation, input mapping, UI, localisation, navigation, terrain, coordinate precision, memory budgets, tracing, panic policy, crash reporting, logging, protocol versioning, CI cost, build times.

### 22. MINOR — small contradictions
Priority with a single deque; futex on macOS; "exactly four sync points"; `Mutex` rule vs Vulkan queue submission; O(n²) ordering without system sets; per-chunk task granularity.

### 23. MINOR — uber-shader decided without the lighting architecture
Decide deferred, forward+ or visibility buffer; name permutation axes.

### 24. MINOR — render graph cache hash input undefined
Define it: pass list, edges, resource descriptors, imported initial states, queue config.
