# WHISK (`lactose-task`)

The work-stealing executor.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | yes, see `SAFETY.md` once the implementation lands |

**Status: signatures only.** `src/` is the public surface with `todo!()` bodies. This file is
the contract the implementation must meet. Architecture sections 5.1 and 5.2 are the design;
where this file and the architecture disagree, the architecture wins and this file is wrong.
The surface was reviewed adversarially before assignment; decision D90 records why it has
this shape.

Amending a signature while implementing it is allowed. Say so in the change description and
list what changed, because `lactose-ecs` is written against this surface.

## Surface

| Item | Purpose |
|---|---|
| `Executor`, `ExecutorBuilder` | Worker pools, built through a `Platform` |
| `Platform`, `StdPlatform`, `CoreTopology`, `ThreadSpec` | Thread creation and core counts behind a trait, so a console can replace both |
| `Graph`, `GraphBuilder`, `GraphNode`, `GraphRun` | Continuation-counted dependency graphs |
| `JobContext`, `Scope`, `ChildContext` | Fork-join inside a job, per-job scratch memory, partial-storage slots, `block_in_place` |
| `Executor::spawn`, `Task`, `yield_now` | `'static` stackless tasks |
| `Executor::sleep_until`, `TickPacer` | Timers and tick pacing for the timer root |
| `lane`, `Lane`, `LanePump`, `LaneCall` | Calls onto one particular thread, returned as futures |
| `ChaosConfig`, `ExecutionLog` | Seeded schedule perturbation and exact replay for SOUR |

## Behaviour the signatures cannot express

**Graphs**

- Each node has a remaining-units counter. The job that brings it to zero runs `finalize`,
  then decrements the counters of dependent nodes and pushes those that reach zero. No worker
  blocks waiting for a node.
- A released node is pushed as one range job covering all its units. A thief splits the range
  it steals in half and pushes one half to its own deque. There is no target job duration.
- A job decrements its node's counter only after every scope it opened has closed.
- `GraphRun` resolves after the last `finalize` and hands the graph back. Dropping it does
  not cancel the run. Because a run owns its graph, two runs of one graph cannot overlap.
- A stolen range's halves are deque entries held by value. They are never allocated in the
  thief's arena, whose reset is tied to an unrelated scope.
- `JobContext::slot` is dense, below `Executor::slot_count`, and held by at most one running
  job or child at a time. An opener that runs its own child inline lends the child its slot;
  the opener is suspended meanwhile.

**Scopes**

- While waiting, the opener runs jobs of its own scope, then spins briefly, then parks on
  the scope's latch. It never runs unrelated graph jobs or background work.
- "Only its own scope" is decided by a scope tag on each deque entry, checked when popped; a
  foreign entry is pushed back. A deque length recorded at scope open is not enough, because
  thieves remove from the other end and the length drops below the mark while the opener's
  own children remain.
- Scopes never nest. `JobContext::scope` takes `&mut self`, children get a `ChildContext`
  that cannot open one, and `Scope` is neither `Send` nor `Sync`, so every fork happens on
  the opening worker.
- Each worker has three separate arenas: job scratch, child-job scratch, and scope job
  storage. Scope job storage is reset when the scope closes. Child scratch is reset after
  each child, which must not disturb the scratch of an opener that is running that child
  inline.

**Workers**

- Frame-critical workers equal the performance-core count and run at user-interactive QoS.
  Background workers run on efficiency cores at utility QoS, or as two low-priority threads on
  homogeneous CPUs. A frame-critical worker never runs background work.
- Each worker owns a `crossbeam-deque`. Idle workers spin briefly, then `std::thread::park`.
- Every worker calls `lactose_core::ThreadClass::enter` at start. Every job and every scope
  wait is wrapped in an abort-on-unwind guard. The crate installs no panic hook; binaries do.
- A task inherits the spawner's `lactose_core::AllocTag`, and the executor re-enters it on
  whichever worker polls the task. Scope children inherit the opener's tag the same way. A
  graph job runs untagged unless its node enters a tag itself.
- `block_on` is the only general blocking wait. On a worker thread it is a debug assertion.
- `JobContext::block_in_place` is the one sanctioned blocking wait on a worker, for the
  physics step. While it runs, all but `reserved_workers` of the other frame-critical workers
  stay parked.
- Dedicated threads owned by other crates (network, present, audio) are created through
  `Executor::platform`, never `std::thread` directly.

**Tracing**

- One `tracing` span per node run and one per stolen range, named by `GraphNode::name`. Never
  one per unit.

**Lanes**

- The platform thread owns the main-thread `LanePump` and drains it from its event loop. The
  present thread owns its own pump. Workers never wait synchronously on either.
- `wake` is called on the submitting thread each time a call is queued.
- Calls from one sender run in submission order. If the pump is dropped, pending and later
  calls resolve to an error of kind `Cancelled`.

**Chaos**

- With `ChaosConfig` unset, the chaos paths cost nothing measurable. It is a runtime option,
  never `cfg(test)` or a Cargo feature.
- A recorded `ExecutionLog` replays the same job-to-worker order.

## Proof required for milestone 0

Benchmarks live in `benches/` and their results are checked in under `docs/results/`, since CI
has no hybrid hardware. They run on a developer M-series Mac and an x86 desktop.

| Measure | Threshold |
|---|---|
| Makespan divided by its lower bound | at most 1.25 at P50 and 1.5 at P99 |
| Wake latency (push to an idle pool until the job starts) | under 20 µs at P99 |
| Steal (one successful steal, including the split) | under 1 µs |

The lower bound of a run is the larger of two numbers: the heaviest dependency chain, summing
the drawn job durations along it, and the total drawn work divided by the frame-critical
worker count. No scheduler can beat either, so the ratio is 1.0 for an ideal one. The
benchmark computes both from the durations it drew.

The makespan ratio is measured on three graph shapes, each with job durations drawn
log-uniformly from 10 µs to 1 ms, and each with and without two background threads running
zstd compression:

| Shape | Description |
|---|---|
| Wide-shallow | Many independent nodes, critical path of one or two nodes |
| Deep chain | One long dependency chain with little parallel slack |
| Fan-in-fan-out | Repeated spread to many nodes and join to one |

Correctness:

- The scope latch and the sleep-and-wake protocol are modelled under `loom`. The deque is a
  vetted dependency and is not modelled. loom is enabled by a `loom` Cargo feature on this
  crate that switches an internal `sync` shim, and run with
  `cargo test -p lactose-task --features loom`. Never use loom's documented
  `RUSTFLAGS="--cfg loom"`: the environment variable replaces the x86-64 target pin.
- The crate's tests pass under Miri (nightly, the one sanctioned nightly use).
- A stress test on aarch64 covers weak-memory behaviour that loom does not model.

Before relying on them, re-check on the versions pinned in `Cargo.lock` (see
`docs/verification.md`): loom's current limitations and a feasible preemption bound, and the
per-span overhead of `tracing-tracy`.
