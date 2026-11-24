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
| `Executor::run_job`, `JobRun`, `Executor::run_job_width` | One job on a frame-critical worker for a task that is in none, and the width such jobs and their children occupy |
| `Executor::sleep_until`, `Sleep`, `TickPacer`, `TickBudget` | Timers and tick pacing for the timer root |
| `lane`, `Lane`, `LanePump`, `LaneCall` | Calls onto one particular thread, returned as futures |
| `parking_lane`, `ParkingPump` | A lane whose thread parks while nothing is queued, for a thread that runs nothing but its calls |
| `ChaosConfig`, `ExecutionLog`, `Executor::execution_log`, `Executor::chaos_config` | Seeded schedule perturbation and exact replay for SOUR, and the configuration a run reports it applied |

`GraphRun`, `Task`, `JobRun` and `Sleep` resolve to a `lactose_core::Result`. The error is of kind
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
- **`JobRun`.** When it resolves with `Ok`, the closure has returned, every scope it opened
  has closed, and everything the closure and its children wrote happens before the
  resolution. It resolves to `Cancelled` only when shutdown began before the job started: the
  closure has then been dropped unrun, under the abort-on-unwind guard, on the thread that
  gave it up. A job that started finishes and hands its value back, shutdown or not, which is
  a rule a graph run does not have. Dropping a `JobRun` cancels nothing: the job runs, and
  the value is dropped by the worker that ran it or by the thread that dropped the future,
  whichever comes last. Polling it again after it resolved panics, and so does polling a
  `LaneCall` again.
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
- The opener finds its children by popping its own deque, except in a scope of a job of
  `run_job` (below), and this is the invariant that makes that complete: **while a scope is open, nothing but that scope's children is pushed
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
  inline. An arena allocates its chunk, and its list of chunks, at its first use on that
  worker, another chunk when a use outgrows the last, and one larger chunk at its first use
  after a reset that freed more than one.

**Jobs of `run_job`**

- *A job like any other.* The closure is queued when `run_job` is called, as a graph run
  starts at the call, and a frame-critical worker runs it once: never inside the call, and
  never a background worker. Where a task on a frame-critical worker calls it, the job may run
  on that worker, once the task's poll has returned, or on another that takes it sooner. A
  caller that is no worker, as the thread that drives a stepped run is, never runs it. It is
  one job as one call of `GraphNode::run` with one unit is. It has that job's `JobContext`,
  whose `slot` is the worker's; the job scratch is reset when it returns; one abort-on-unwind
  guard wraps it; it may open scopes one after another, and their children are its own. It is
  one entry and never split. A worker that takes it while the gate is closed puts it back
  unstarted, as the rest of a range is put back. It has no `finalize`. It runs untagged, as a
  graph job does, under one span of target `whisk` named `run_job`.
- *The gate and the reservation.* A worker takes it only while it holds a run permit, as it
  takes a graph's job. So while `block_in_place` holds the gate it starts on a worker the
  reservation leaves awake, or waits for the gate; a job that is running when a block begins
  finishes; and one parked on its own scope's latch holds no permit.
- *The width.* The jobs `run_job` starts, and the children their scopes fork, occupy
  `run_job_width()` frame-critical workers at most at any moment, counted over all such jobs
  together: `frame_worker_count()` less one, and one where that is one. A worker is occupied
  from the moment it takes such a job until the closure has returned, whether it is running
  the closure, running a child of one of its scopes, parked on a scope's latch or waiting for
  the gate; and for as long as it runs a child of such a job that it took from elsewhere. A
  worker with a scope of such work open runs that scope's children itself, under the place it
  holds: while one of them is still to be started it takes that one and does not park, the
  gate letting it run. One that parked and was woken looks for its scope's children again
  before it parks again, whatever woke it. So no child of its scope waits queued while its
  opener could start it, and where the width is one they all run on the job's own worker. So
  with two frame-critical workers or more, at every moment one of them is occupied by no such
  job: it takes graph jobs and tasks as before, fires the due timers where it looks for work,
  and is the keeper while it sleeps. A timed wake is then late by the keeper's wake, or by what
  is left of a job or a poll that is no such job's. The width parks nobody and spins nobody: a
  worker that may not take such work looks past it, takes whatever else there is, and sleeps
  as any idle worker does. An entry queued while fewer than the width are occupied is taken,
  and one queued while the width is full is taken once a worker leaves it, with no wake-up lost
  in either case. Under `block_in_place` the gate applies first: a worker needs a run permit to
  take anything, and a worker the width counts may hold none. It is scheduling and nothing
  else: an execution log records what ran where, and a replay follows the log. There is no
  knob.
- *How the width is kept.* Such jobs, the children of their scopes, the halves split off those
  children and the rest a worker puts back wait on two queues of the frame-critical pool's
  that hold such work alone, one for jobs and one for children, and never on a worker's deque,
  so no thief takes one blind. A worker that holds no place takes from them only after raising
  a count while it is below the width, children first, and lowers it when the entry is done.
  A job raises a flag in the same word, and no worker takes a job while it is up: such jobs run
  one after the other, so every queued child is a child of the one scope that is open, and a
  worker inside one such job never runs another scope's entry or another such job, which would
  nest a scope (architecture section 5.1, "Fork-join inside a job only"). The opener takes its
  scope's children from the queue under its job's place, with no raise. A worker running such
  a child splits what is left of its range while the children's queue is empty, the rule a
  deque's range follows with the deque; a half another worker splits off, or puts back, wakes
  the opener. The pool's
  look for visible work counts these queues only while a worker without a place could take
  from them. Queueing an entry and then reading the count, and lowering the count and then
  reading the queues, is a store-buffering pair: each side has a `SeqCst` fence between its
  write and its read (`width.rs`).
- *What it is for.* The frame of `lactose-app` is its one caller (D162): a task that is in no
  job gets a `JobContext` from it, with no lock, no `Sync` bound and no world. It may not be
  used for a wait in the closure other than its own scopes, since a job that blocks keeps its
  run permit and `block_in_place` is the physics step's alone; for a world's work outside its
  schedule, nothing inside the determinism boundary being a task, which clippy refuses in a
  boundary crate (`crates/core/clippy.toml`); or for what a graph is for. Two in flight at
  once wait for each other, the second starting once the first's closure has returned.
- *What it costs.* A call allocates one cell, which holds the closure, then the value, and the
  future's waker, under `AllocTag::Task` on the calling thread; where std's mutex allocates on
  its first lock, as on Apple targets, the cell's mutex is a second allocation, made there too.
  Beyond those, a share of a block of the jobs' queue, and a child a share of a block of the
  children's queue: one block per 63 entries. It takes no lock but the completion slot's,
  and that one twice at its start, on the calling thread and on the worker that takes the
  closure; once at its end; at each poll of the future; once when the future is dropped
  unresolved; and, for a job shutdown cancels, once if a worker has taken it and twice if not.
  A fan-out cut for W − 1 workers takes W/(W − 1) as long as one cut for W; one that is not
  takes as many rounds as its pieces need on W − 1 workers (four equal children on four
  workers take twice as long).

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
  worker is inside a scope. A worker that kept its permit through the block may go on as soon
  as the gate reopens, unordered with the closure: the closure's writes reach other nodes
  through the graph, not through the gate.
- Dedicated threads owned by other crates (network, present, audio) are created through
  `Executor::platform`, never `std::thread` directly.
- There is no timer thread. Frame-critical workers fire due timers when they look for work,
  and one sleeping worker at a time, the keeper, parks only until the earliest deadline.
  On Windows its timed wait is a high-resolution waitable timer beside an event that every
  wake of the worker sets (`Sleep::wake_worker`), and it spins the last 500 µs before a
  deadline, because `std::thread::park_timeout` there waits in whole milliseconds on the
  system's 15.6 ms timer tick and the finer timer overshoots by about half a millisecond
  (D144); a worker whose event or timer cannot be made keeps `park_timeout`. A plain unpark
  does not end that wait, so a new waker of a worker that may be asleep goes through
  `Sleep::wake_worker`; a spare wake costs one trip round the park loop.
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
- A lane has two locks, which architecture section 3 permits on the per-frame path (D159):
  its queue's, taken by `Lane::run` to queue a call, by each `LanePump::drain` to take what
  is queued and by the pump's drop to close the lane; and each call's reply slot, taken to
  store the call's result, when it returns or is dropped unrun, and by each poll of its
  `LaneCall`. Neither is held while a call runs or while a waker is woken, and neither is
  taken while the other is held.
- A `parking_lane` is drained by the thread that made it, which parks in
  `ParkingPump::wait` until a call has been queued since the last `wait` returned, and
  returns at once if one has. Its wake is a flag the queueing thread sets before it unparks
  the lane's thread, so a call that parks inside its own body, on a channel or a lock, and so
  takes the unpark of a call queued meanwhile, does not lose that call's wake; an unpark from
  elsewhere does not end the wait. The flag publishes nothing: the queue's lock orders the
  calls, and the flag's Release and Acquire are deliberate but relied on by nothing. `wait`
  runs no call, and the thread drains after it. This is the present thread's pump
  (`lactose-platform`), whose loop drains, checks its own stop and waits; the wake is
  `lactose-task`'s so that it is modelled, since no other crate names loom.

**Chaos**

- With `ChaosConfig` unset, the chaos paths cost nothing measurable. It is a runtime option,
  never `cfg(test)` or a Cargo feature.
- Recording does not serialise the workers: a job start is one increment of a shared counter
  and a push onto the worker's own list. The log grows by 28 bytes per job for the life of
  the executor.
- A recorded `ExecutionLog` replays the same job-to-worker order. The log lists every call of
  `GraphNode::run` and `finalize`, every scope child and every closure of `run_job` in the
  order they started, with the worker that ran each. Replay hands each worker exactly the job
  the log names next. It needs the frame-critical worker count the log was recorded with and a
  program that starts the same graph runs and `run_job` calls in the same order. Tasks are not
  recorded. A replay does not keep the width of `run_job`: it hands out such work by the log
  and leaves the count alone, so it runs as the recorded run did and no narrower.
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
| Timer wake (a tick deadline until the frame-critical task that slept until it resumes, at 60 Hz) | under 35 µs at P99 (D138); on macOS under 50 µs (D156); on Windows under 1 ms (D144) |
| Steal (one successful steal, including the split) | under 1 µs |

The lower bound of a run is the larger of two numbers: the heaviest dependency chain, summing
the drawn job durations along it, and the total drawn work divided by the frame-critical
worker count. No scheduler can beat either, so the ratio is 1.0 for an ideal one. The
benchmark computes both from the durations it drew.

The makespan ratio is measured on three graph shapes, each with job durations drawn
log-uniformly from 10 µs to 1 ms, and each with and without two tasks of the background class
compressing with zstd:

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
inside the executor, on the function a worker steals with. Every wake or timer sample over
1 ms, and every graph run over twice its lower bound, is listed with the UTC time it ended, to
be matched against the system's log of sleeps and wakes (`pmset -g log` on a Mac); a graph
run carries its lower bound. The JSON lists every one; the summary lists at most twenty of a
phase, the furthest over, and gives each such phase's total. Steal samples carry no time.

Correctness:

- The scope latch and the sleep-and-wake protocol are modelled under `loom` (`src/models.rs`),
  and with them what is built on them: the opener's wait with its permit and the
  `block_in_place` gate (`src/gate.rs`), the wake that is skipped for a kept entry, the
  timer keeper's hand-off, the task kept by the worker that fires its timer, the
  quiescence of a cancelled run (`src/quiesce.rs`), and a parking lane's wait under a call
  that takes its thread's unpark (`src/parking.rs`). The models call the shipped functions.
  Under loom a timed park waits on a flag that only `Sleep::wake_worker` sets and that an
  unpark alone does not satisfy, as the Windows wait is not ended by an unpark, so a waker
  that skips it fails the keeper hand-off model.
  The deque is a vetted dependency and is not modelled. loom is enabled by a `loom` Cargo
  feature on this crate that switches an internal `sync` shim, and run with
  `cargo test -p lactose-task --features loom --release`. Never use loom's documented
  `RUSTFLAGS="--cfg loom"`: the environment variable replaces the x86-64 target pin.
  Eleven models are explored exhaustively. The three with three threads are bounded at five
  preemptions, the idle-release one at four, the parking lane's at three
  (`LOOM_MAX_PREEMPTIONS` overrides the bounds).
  Eight more hold `run_job`: the completion slot against a dropped `JobRun`, exhaustively; a
  worker taking the job against shutdown beginning, at five preemptions; the width in three
  configurations with the sleep protocol, at two: a width of one, two workers beside a main
  thread that queues two jobs; a width of two with two jobs on three workers; and a width of
  two with one job on three workers whose scope forks a range, the worker that puts a half
  back held until that half has run; a job queued at a width of two while a worker gives back
  a child's place it took for nothing, at two; a child that only the wake of a holder's
  `leave` can deliver, at one; and a job and a task queued through `ShutdownFlag::queue`
  against shutdown's flag and sweep, at one. The jobs of the width's configurations of three
  workers are queued before their workers start, which keeps those searches within minutes.
  Each model counts only beside weakened variants that fail it, which run with it
  (`weakened_*`), each at the model's bound: under loom 0.7.2 a model whose reader is the main
  thread may explore one execution. The parking lane's model fails, as a deadlock, with the
  unpark alone and with the flag alone; both run beside it as `weakened_*` models over a copy
  of its wake and wait. Five terms are copies in the models, since loom cannot run
  the types they live in: four of `run_job`'s, and what the pool's three submissions hand
  `ShutdownFlag::queue` with the sweep they hand it. The module's doc names each and the
  native test that holds it.
- `src/executor/job_tests.rs` holds the width by order and never by a time, each wait bounded
  by a thread of the test's own: with children of a `run_job` job held where they begin, a
  task's `sleep_until` resolves and a graph run completes beside them, on four
  frame-critical workers, on two, where every child runs on the job's own worker, and with
  the opener parked on its latch, its place kept, and a second job waiting behind it. There
  too: a second job waits beside a first that leaves room in the width, by the flag alone; a
  job queued while the gate holds the other worker is still queued once that worker, woken
  for it, sleeps again; a range in a job splits to a second worker; and an opener that parked
  takes the halves queued after it parked. `tests/jobs.rs` holds where a job runs, shutdown,
  the reservation, who drops the value, a job queued as the one frame-critical worker goes
  idle, 5,000 times with the gaps swept across the window before it sleeps, and a child that
  crosses to another worker, which the Miri runs check too. `src/pool.rs` holds a job taken
  at a closed gate and one taken after shutdown began, and a job, a task and a graph's range
  each queued as shutdown begins, the flag set between the push and the read by a hook that
  only its tests compile, and swept by the submission itself. `tests/job_alloc.rs` counts a call's
  allocations, in a binary of its own because it installs a global allocator.
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
  reserves workers lasts until the work it is owed has run. It also checks that opening the
  gate wakes the workers it held and that the pool regains its full width after a block. A
  slow or oversubscribed machine makes it slower, never wrong.
- A test that waits for a wake-up, a timer, a cancellation or the end of a run it is about
  bounds the wait (`tests/common`), so a regression it looks for fails it with a message
  instead of hanging it. The wake-up tests (`tests/graph.rs`, `tests/scope.rs`) release work
  while every worker sleeps and need all of them at once: a fork, and a thief's split, each
  wake a sleeper. `tests/tasks.rs` ends a keeper's wait for a timer an hour away by an
  earlier timer, by pushed work and by shutdown, which on Windows only the event ends, and on
  Windows holds the median lateness of fifty 2 ms deadlines under half a timer tick in one
  of up to three batches.
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
