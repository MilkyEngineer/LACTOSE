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

Amending a signature while implementing it is allowed. Say so in the change description and
list what changed, because `lactose-ecs` is written against this surface.

## Surface

| Item | Purpose |
|---|---|
| `Executor`, `ExecutorBuilder` | Worker pools, built through a `Platform` |
| `Platform`, `StdPlatform`, `CoreTopology`, `ThreadSpec` | Thread creation and core counts behind a trait, so a console can replace both |
| `Graph`, `GraphBuilder`, `GraphNode`, `GraphRun` | Continuation-counted dependency graphs |
| `JobContext`, `Scope`, `ChildContext` | Fork-join inside a job, and per-job scratch memory |
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
- `GraphRun` resolves after the last `finalize`. Dropping it does not cancel the run.

**Scopes**

- The opener records a deque watermark at scope open and pops only jobs above it. While
  waiting it runs jobs of its own scope, then spins briefly, then parks on the scope's latch.
  It never runs unrelated graph jobs or background work.
- Child jobs live in a per-worker `lactose_core::Arena` that is reset when the scope closes.
- Scopes never nest. `JobContext::scope` takes `&mut self`, and children get a `ChildContext`
  that cannot open one.

**Workers**

- Frame-critical workers equal the performance-core count and run at user-interactive QoS.
  Background workers run on efficiency cores at utility QoS, or as two low-priority threads on
  homogeneous CPUs. A frame-critical worker never runs background work.
- Each worker owns a `crossbeam-deque`. Idle workers spin briefly, then `std::thread::park`.
- Every worker calls `lactose_core::ThreadClass::enter` at start. Every job and every scope
  wait is wrapped in an abort-on-unwind guard. The crate installs no panic hook; binaries do.
- A task inherits the spawner's `lactose_core::AllocTag`, and the executor re-enters it on
  whichever worker polls the task.
- `block_on` is the only blocking wait. On a worker thread it is a debug assertion.

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
| Makespan divided by critical-path duration | at most 1.25 at P50 and 1.5 at P99 |
| Wake latency (push to an idle pool until the job starts) | under 20 µs at P99 |
| Steal (one successful steal, including the split) | under 1 µs |

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
  vetted dependency and is not modelled.
- The crate's tests pass under Miri (nightly, the one sanctioned nightly use).
- A stress test on aarch64 covers weak-memory behaviour that loom does not model.

Before relying on them, re-check on the versions pinned in `Cargo.lock` (see
`docs/verification.md`): loom's current limitations and a feasible preemption bound, and the
per-span overhead of `tracing-tracy`.
