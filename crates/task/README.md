# WHISK (`lactose-task`)

The work-stealing executor.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | yes, see `SAFETY.md` |

**Status: implemented.** This file is the contract the implementation meets. Architecture
sections 5.1 and 5.2 are the design; where this file and the architecture disagree, the
architecture wins and this file is wrong. Decision D90 records why the surface has its shape.

Amending a signature is allowed in the change that needs it. Say so in the change description
and list what changed, because `lactose-ecs` is written against this surface.

## Surface

| Item | Purpose |
|---|---|
| `Executor`, `ExecutorBuilder` | Worker pools, built through a `Platform` |
| `Platform`, `StdPlatform`, `CoreTopology`, `ThreadSpec`, `ThreadHandle` | Thread creation and core counts behind a trait, so a console can replace both |
| `Graph`, `GraphBuilder`, `GraphNode`, `GraphRun` | Continuation-counted dependency graphs |
| `JobContext`, `Scope`, `ChildContext` | Fork-join inside a job, per-job scratch memory, partial-storage slots, `block_in_place` |
| `Executor::spawn`, `Task`, `yield_now` | `'static` stackless tasks |
| `Executor::sleep_until`, `Sleep`, `TickPacer`, `TickBudget` | Timers and tick pacing for the timer root |
| `lane`, `Lane`, `LanePump`, `LaneCall` | Calls onto one particular thread, returned as futures |
| `ChaosConfig`, `ExecutionLog`, `Executor::execution_log` | Seeded schedule perturbation and exact replay for SOUR |

## Behaviour the signatures cannot express

**Graphs**

- Each node has a remaining-units counter. The job that brings it to zero runs `finalize`,
  then decrements the counters of dependent nodes and pushes those that reach zero. No worker
  blocks waiting for a node.
- A released node is pushed as one range job covering all its units. A thief splits the range
  it steals in half and pushes one half to its own deque. There is no target job duration.
- The worker running a range calls `run` one unit at a time and keeps the rest stealable:
  before each unit, if its own deque is empty and more than one unit is left, it pushes the
  upper half. A range that was just stolen arrives on an empty deque, so the thief-side split
  is the same rule. Pushes therefore happen per steal, not per unit.
- `units()` is called on the thread that releases the node: the worker that finalized its last
  dependency, or the caller of `run_graph` for a node without dependencies.
- A job decrements its node's counter only after every scope it opened has closed.
- `GraphRun` resolves after the last `finalize` and hands the graph back. Dropping it does
  not cancel the run. Because a run owns its graph, two runs of one graph cannot overlap. The
  run's counters live in the graph, so a run allocates nothing.
- A stolen range's halves are deque entries held by value. They are never allocated in the
  thief's arena, whose reset is tied to an unrelated scope.
- `JobContext::slot` is dense, below `Executor::slot_count`, and held by at most one running
  job or child at a time. An opener that runs its own child inline lends the child its slot;
  the opener is suspended meanwhile. The slot is the index of the frame-critical worker.

**Scopes**

- While waiting, the opener runs jobs of its own scope, then spins briefly, then parks on
  the scope's latch. It never runs unrelated graph jobs or background work, and it does not
  steal its own children back from the workers that took them.
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
  homogeneous CPUs. A frame-critical worker never runs background work. An executor has at
  least one worker of each class.
- Each worker owns a `crossbeam-deque`. Idle workers spin briefly, then `std::thread::park`:
  about 50 µs of rescanning, or 300 µs on a frame-critical worker while a graph run is in
  flight, when the next node is usually one finalize away.
- Every worker thread is created through the `Platform`, which applies its
  `lactose_core::ThreadClass` (`StdPlatform` calls `ThreadClass::enter`) before the worker
  runs; the worker re-asserts the deterministic floating-point state itself. Every job and
  every scope wait is wrapped in an abort-on-unwind guard. The crate installs no panic hook;
  binaries do.
- A task inherits the spawner's `lactose_core::AllocTag`, and the executor re-enters it on
  whichever worker polls the task. Scope children inherit the opener's tag the same way. A
  graph job runs untagged unless its node enters a tag itself.
- `block_on` is the only general blocking wait. On a worker thread it is a debug assertion.
- `JobContext::block_in_place` is the one sanctioned blocking wait on a worker, for the
  physics step. While it runs, all but `reserved_workers` of the other frame-critical workers
  stay parked. A worker that is mid-job when the block starts finishes the unit it is on, puts
  the rest of its range back for the reserved workers, and parks.
- Dedicated threads owned by other crates (network, present, audio) are created through
  `Executor::platform`, never `std::thread` directly.
- There is no timer thread. Frame-critical workers fire due timers when they look for work,
  and one sleeping worker at a time parks only until the earliest deadline.
- `shutdown` lets running jobs finish, including the scope children they wait for, drops
  queued graph work and cancels every unfinished task. A `GraphRun`, `Task` or `Sleep` that
  was cut off never resolves.

**Tracing**

- One `tracing` span per node run and one per range split off it, named by `GraphNode::name`,
  target `whisk`. Never one per unit. Each distinct node name gets its own call sites, created
  when a graph is built.

**Lanes**

- The platform thread owns the main-thread `LanePump` and drains it from its event loop. The
  present thread owns its own pump. Workers never wait synchronously on either.
- `wake` is called on the submitting thread each time a call is queued.
- Calls from one sender run in submission order. If the pump is dropped, pending and later
  calls resolve to an error of kind `Cancelled`.

**Chaos**

- With `ChaosConfig` unset, the chaos paths cost nothing measurable. It is a runtime option,
  never `cfg(test)` or a Cargo feature.
- A recorded `ExecutionLog` replays the same job-to-worker order. The log lists every call of
  `GraphNode::run` and `finalize` and every scope child in the order they started, with the
  worker that ran each. Replay hands each worker exactly the job the log names next. It needs
  the frame-critical worker count the log was recorded with and a program that starts the
  same graph runs in the same order. Tasks are not recorded.

## Proof for milestone 0

Benchmarks live in `benches/` and their results are checked in under `docs/results/`, since CI
has no hybrid hardware. They run on a developer M-series Mac and an x86 desktop, on a quiet
machine, with one command:

```text
cargo bench -p lactose-task --bench whisk
```

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
| Wide-shallow | 192 independent nodes, 64 of them with one dependent: a critical path of one or two nodes |
| Deep chain | One chain of 128 nodes; every fourth link forks a side node that rejoins two links on |
| Fan-in-fan-out | 16 rounds of one node spreading to 8 and joining to one |

The lower bound is not always reachable. The harness also simulates, on the same draws, a
scheduler with no overhead that never idles a worker while a node is ready, and reports its
ratio as `ideal` beside the measured one. Fan-in-fan-out spreads to eight because a wider
spread makes the bound itself unreachable: with a spread of sixteen on four workers the ideal
scheduler already scores 1.32 at P50.

Correctness:

- The scope latch and the sleep-and-wake protocol are modelled under `loom` (`src/models.rs`).
  The deque is a vetted dependency and is not modelled. loom is enabled by a `loom` Cargo
  feature on this crate that switches an internal `sync` shim, and run with
  `cargo test -p lactose-task --features loom --release`. Never use loom's documented
  `RUSTFLAGS="--cfg loom"`: the environment variable replaces the x86-64 target pin. With
  the feature on, only the models run; the executor itself must not be used.
  Four models are explored exhaustively. The fifth, two sleepers and a pusher, is bounded at
  five preemptions (`LOOM_MAX_PREEMPTIONS` overrides the bound).
- The crate's tests pass under Miri (nightly, the one sanctioned nightly use):
  `cargo +nightly miri test -p lactose-task`. Under Miri a mutex-backed stand-in replaces
  `crossbeam-deque` (`src/deque.rs`), because Miri's default aliasing model rejects
  `crossbeam-epoch`; the check then covers everything this crate does around the deque.
- `tests/stress.rs` hammers steal, split, scope and wake paths on the host, which on aarch64
  covers weak-memory behaviour that loom does not model. It runs for two seconds;
  `WHISK_STRESS_SECONDS` sets a longer soak.
- `tests/abort.rs` re-runs itself as a subprocess and checks that a panic in a job, a
  finalize, a scope child, a scope body or a task aborts the process.

Third-party claims re-checked on the pinned versions (`docs/verification.md`):

- loom 0.7.2 still treats `SeqCst` loads and stores as `AcqRel` and does not explore load
  buffering, but it does model `fence(SeqCst)`. The sleep protocol is therefore built on
  explicit fences, which loom checks as written, and not on the orderings of the accesses
  around them.
- A span costs about 1 ns with no subscriber and about 8 ns with a subscriber that does
  nothing (`cargo test -p lactose-task --release --lib -- --ignored --nocapture span_overhead`).
  Through `tracing-tracy` 0.11.4 on a `tracing-subscriber` registry, with no Tracy server
  attached, it costs about 165 ns, of which the registry is about 65 ns.
