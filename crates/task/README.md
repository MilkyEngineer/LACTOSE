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
| `ChaosConfig`, `ExecutionLog`, `Executor::execution_log`, `Executor::chaos_config` | Seeded schedule perturbation and exact replay for SOUR, and the configuration a run reports it applied |

`GraphRun`, `Task` and `Sleep` resolve to a `lactose_core::Result`. The error is of kind
`Cancelled` and means the executor shut down first. What each resolution guarantees:

- **`GraphRun`.** When it resolves, with `Ok` or with `Cancelled`, no unit, scope child or
  `finalize` of that run is executing or ever will, and everything they wrote happens before
  the resolution. With `Ok` every node has finalized and the graph is handed back. After
  `Cancelled` the run is torn: some nodes finalized, some ran in part, their dependents never
  ran, and the graph is not handed back. A job looks for shutdown before each unit and before
  `finalize`, so once shutdown has begun none of those starts, except one that was already
  past its check; jobs already running finish, including the scopes they have open, and the
  run resolves when the last of them has returned. This holds however the
  executor is shut down, including when the last handle is dropped on a worker and nothing
  is joined.
- A job of a run cannot wait for its own run, which is not quiet while that job executes.
  Jobs are synchronous and `block_on` on a worker is a debug assertion, so it cannot happen
  by accident; done deliberately in a release build, it waits for ever.
- **`Task`.** When it resolves to `Cancelled` the task's future has been dropped: its body is
  not running and will not run again. A task that is being polled when shutdown begins
  finishes that poll first.
- **`Sleep`.** Resolves to `Cancelled` as soon as shutdown begins. It guards nothing.

`lactose_task::bench_support` is hidden and exists for `benches/whisk.rs` alone.

## Behaviour the signatures cannot express

**Graphs**

- Each node has a remaining-units counter. The job that brings it to zero runs `finalize`,
  then decrements the counters of dependent nodes and pushes those that reach zero. No worker
  blocks waiting for a node.
- A released node is pushed as one range job covering all its units. A thief splits the range
  it steals in half and pushes one half to its own deque. There is no target job duration.
- A worker that finalizes a node takes one of the nodes it releases as its own next job, so
  releasing a single dependent wakes nobody.
- `units()` is called on the thread that releases the node: the worker that finalized its last
  dependency, or the caller of `run_graph` for a node without dependencies.
- A job decrements its node's counter only after every scope it opened has closed.
- `GraphRun` resolves after the last `finalize` and hands the graph back. Dropping it does
  not cancel the run. Because a run owns its graph, two runs of one graph cannot overlap. The
  run's counters live in the graph, so a run allocates nothing on the hot path.
- A stolen range's halves are deque entries held by value. They are never allocated in the
  thief's arena, whose reset is tied to an unrelated scope.
- `JobContext::slot` is dense, below `Executor::slot_count`, and held by at most one running
  job or child at a time. An opener that runs its own child inline lends the child its slot;
  the opener is suspended meanwhile. The slot is the index of the frame-critical worker.

**How a range is executed.** This is what `lactose-ecs` may rely on, and what it must not.

- `GraphNode::run` and a `fork_range` closure are called with **one unit per call**. A job is
  one such call: the `JobContext`, the job scratch arena and the abort guard are per unit, and
  nothing is amortised across the units of a range. A node that needs per-range setup does it
  in its own data, keyed by `slot`.
- The contract in the signature stays wider than that: a call may cover any sub-range, and a
  node must handle one. Replay hands a job the exact range the log names.
- A range entry is split in two places. A thief halves what it steals and pushes the upper
  half onto its own deque. The worker running an entry halves what is left of it whenever its
  own deque is empty, before the next unit, so the rest is always stealable.
- So a node of N units produces between zero and N−1 range entries beyond the one it was
  released as, each with its own tracing span. The worst case, N−1, needs every unit to be
  taken singly. A lone worker produces exactly ⌈log₂ N⌉. In between, it is one entry per steal
  or dry deque.

**Scopes**

- While waiting, the opener runs jobs of its own scope, then spins briefly, then parks on
  the scope's latch. It never runs unrelated graph jobs or background work, and it does not
  steal its own children back from the workers that took them.
- "Only its own scope" is decided by a scope tag on each deque entry, checked when popped. A
  deque length recorded at scope open is not enough, because thieves remove from the other
  end and the length drops below the mark while the opener's own children remain.
- The opener finds its children by popping its own deque, and this is the invariant that
  makes that complete: **while a scope is open, nothing but that scope's children is pushed
  onto the opening worker's deque.** Anything else the worker submits meanwhile, such as a
  graph started from the scope body or from a child the opener runs itself, goes to the
  shared queue. The children are therefore the topmost entries, and an entry with another tag
  is older work from before the scope opened; it is pushed back and never run by the wait.
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
  least one worker of each class. With the default platform the counts are capped by the
  hardware threads the process may use (`std::thread::available_parallelism`, which honours
  affinity masks and container limits): workers never exceed the usable hardware threads;
  frame-critical workers stop one short and background workers take what remains, so the
  platform thread shares a hardware thread with low-priority background workers. The
  minimum of one worker of each class is the only exception. A custom `Platform`
  opts into the same cap by returning `Some` from `hardware_threads`.
- Each worker owns a `crossbeam-deque`. Idle workers spin briefly, then `std::thread::park`:
  about 50 µs of rescanning.
- Every worker thread is created through the `Platform`, which applies its
  `lactose_core::ThreadClass` (`StdPlatform` calls `ThreadClass::enter`) before the worker
  runs; the worker re-asserts the deterministic floating-point state itself. Every job and
  every scope wait is wrapped in an abort-on-unwind guard, and so is the drop of a task's
  future, on whichever thread it happens. The crate installs no panic hook; binaries do.
- A task inherits the spawner's `lactose_core::AllocTag`, and the executor re-enters it on
  whichever worker polls the task, and around the drop of its future. Scope children inherit
  the opener's tag the same way. A graph job runs untagged unless its node enters a tag
  itself.
- `block_on` is the only general blocking wait. On a worker thread it is a debug assertion.
- `JobContext::block_in_place` is the one sanctioned blocking wait on a worker, for the
  physics step. While it runs, all but `reserved_workers` of the other frame-critical workers
  stay parked. A worker that is mid-job when the block starts finishes the unit it is on, puts
  the rest of its range back for the reserved workers, and parks; an opener running its own
  children does the same. A worker parked on a scope latch holds no permit, so it never uses
  up a reservation; and an opener that gave way to the gate with children still on its own
  deque is woken when a permit comes free, so a reservation is used even when every other
  worker is inside a scope.
- Dedicated threads owned by other crates (network, present, audio) are created through
  `Executor::platform`, never `std::thread` directly.
- There is no timer thread. Frame-critical workers fire due timers when they look for work,
  and one sleeping worker at a time, the keeper, parks only until the earliest deadline.
  A worker that fires timers wakes the ones due when it looks in deadline order, ties in the
  order they were registered; workers that fire at the same time interleave theirs. That
  orders one worker's wake-ups, not the tasks: a woken task is scheduled as below, and a
  `Sleep` first polled after its deadline resolves at that poll without a timer.
  When nothing waits in the frame-critical queue, a worker that fires timers on its way to
  its next job puts the first frame-critical task they wake on its own deque and wakes nobody
  for it, so the task resumes on the worker that is already awake rather than on a sleeper
  woken for it. Every other task they wake, and every one of them while work is queued, goes
  behind the queued work and wakes a sleeper: a timer never puts a task ahead of work that
  was waiting. The kept task stays visible on the deque; under a closed `block_in_place`
  gate a firing worker without a permit parks beside it, and a worker that holds one takes
  it. A worker that finds a timer due just as it lies down to sleep queues the task with a
  wake-up like any other.
- `shutdown` lets running jobs finish, including the scope children they wait for, drops
  queued graph work and cancels every unfinished task. Each `GraphRun`, `Task` or `Sleep`
  that was cut off resolves to `Cancelled` as soon as its guarantee above allows, without
  waiting for the joins: a `Sleep` at once, a `Task` once its future has been dropped, which
  for a task being polled is when that poll returns, and a `GraphRun` once the jobs of it
  that were already running have returned.
  Dropping the last `Executor` handle does the same; a handle held by a task counts.
  Either joins every worker thread, except on one of the executor's own workers, which
  cannot wait for itself: there the workers are told to stop and exit on their own.
- A detached task that is suspended with no waker left anywhere is dropped.

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
- Recording does not serialise the workers: a job start is one increment of a shared counter
  and a push onto the worker's own list. The log grows by 28 bytes per job for the life of
  the executor.
- A recorded `ExecutionLog` replays the same job-to-worker order. The log lists every call of
  `GraphNode::run` and `finalize` and every scope child in the order they started, with the
  worker that ran each. Replay hands each worker exactly the job the log names next. It needs
  the frame-critical worker count the log was recorded with and a program that starts the
  same graph runs in the same order. Tasks are not recorded.
- A log the program cannot follow aborts the process with a diagnostic, two seconds after the
  replay has work waiting, nothing running, and the job the log names next not among it. A
  log whose next job belongs to a graph run the program has not started yet is not stuck:
  replay waits for the program, however slow.

## Proof for milestone 0

Benchmarks live in `benches/` and their results are checked in under `docs/results/`, since CI
has no hybrid hardware. They run on a developer M-series Mac and an x86 desktop, on a quiet
machine, with one command:

```text
cargo bench -p lactose-task --bench whisk
```

On a Mac, keep the machine awake for the whole run, on AC power with
`caffeinate -i -s cargo bench -p lactose-task --bench whisk`. A Mac left alone sleeps and
wakes in the dark about every quarter of an hour, and that freezes the run. `Instant` does not
count the time asleep, so neither the phase timings nor the latencies show the stall for what
it is.

| Measure | Threshold |
|---|---|
| Makespan divided by its lower bound | at most 1.25 at P50 and 1.5 at P99 |
| Wake latency (push to an idle pool until the job starts) | under 35 µs at P99: the floor on macOS is the cores leaving deep idle, 24 to 29 µs at P99 with every wait primitive |
| Timer wake (a tick deadline until the frame-critical task that slept until it resumes, at 60 Hz) | under 20 µs at P99 |
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

"Many" in the last shape is twice the frame-critical worker count, so the spread is wider
than the pool on every machine. The harness runs 2,000 graph runs per case, 5,000 wake-ups
and 400,000 steals, and reports the counts. Beside each measured ratio it reports the ratio of
two simulated schedulers with no overhead (first-ready and longest-first), which show how far
the bound is from reachable for the shape; the process's CPU time per run next to the work
drawn; and the CPU the process uses while it does nothing but wake up. Steal time is measured
inside the executor, on the function a worker steals with.

Correctness:

- The scope latch and the sleep-and-wake protocol are modelled under `loom` (`src/models.rs`),
  and with them what is built on them: the opener's wait with its permit and the
  `block_in_place` gate (`src/gate.rs`), the wake that is skipped for a kept entry, the
  timer keeper's hand-off, the task kept by the worker that fires its timer, and the
  quiescence of a cancelled run (`src/quiesce.rs`). The models call the shipped functions.
  The deque is a vetted dependency and is not modelled. loom is enabled by a `loom` Cargo
  feature on this crate that switches an internal `sync` shim, and run with
  `cargo test -p lactose-task --features loom --release`. Never use loom's documented
  `RUSTFLAGS="--cfg loom"`: the environment variable replaces the x86-64 target pin.
  Eleven models are explored exhaustively. The three with three threads are bounded at five
  preemptions, the idle-release one at four (`LOOM_MAX_PREEMPTIONS` overrides the bounds).
- **The `loom` feature is for that one command.** No other crate may enable it and nothing
  may be built with `--all-features`: features unify across a build, so every executor in it
  would run on loom primitives, which panic outside a loom model.
- The crate's tests pass under Miri (nightly, the one sanctioned nightly use) in two
  configurations. `cargo +nightly miri test -p lactose-task` uses Miri's default aliasing
  model with a mutex-backed stand-in for `crossbeam-deque` (`src/deque.rs`), because that
  model rejects `crossbeam-epoch`; it checks everything this crate does around the deque.
  `LACTOSE_MIRI_REAL_DEQUE=1 MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri test -p lactose-task`
  runs the same tests over the real deque, with the settings it needs. The variable is read
  by `build.rs` and does nothing outside Miri.
- `tests/stress.rs` hammers steal, split, scope and wake paths on the host, which on aarch64
  covers weak-memory behaviour that loom does not model. It runs for two seconds;
  `WHISK_STRESS_SECONDS` sets a longer soak.
- `tests/block_in_place.rs` proves the reservation by counting, not timing: a unit or child
  that begins before a block holds its worker until the block is in place, and a block that
  reserves workers lasts until the work it is owed has run. A slow or oversubscribed machine
  makes it slower, never wrong.
- `tests/abort.rs` re-runs itself as a subprocess and checks that a panic in a job, a
  finalize, a scope child, a scope body, a task or the drop of a cancelled task's future
  aborts the process, and that an unfollowable replay log does.

What the implementation relies on in its pinned dependencies:

- loom 0.7.2 treats `SeqCst` loads and stores as `AcqRel` and does not explore load
  buffering, but it does model `fence(SeqCst)`. The sleep protocol is therefore built on
  explicit fences, which loom checks as written, and not on the orderings of the accesses
  around them.
- A span costs about 1 ns with no subscriber and about 8 ns with a subscriber that does
  nothing (`cargo test -p lactose-task --release --lib -- --ignored --nocapture span_overhead`).
