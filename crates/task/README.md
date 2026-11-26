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
| `take_foreground_task_role`, `TaskRole`, `current_core_class`, `CoreClass` | For measurement only: the foreground task role on macOS, and the class of the core the calling thread is on ("Measurement calls" below) |

`GraphRun`, `Task`, `JobRun` and `Sleep` resolve to a `lactose_core::Result`. The error is of kind
`Cancelled` and means the executor shut down first. What each resolution guarantees:

- **`GraphRun`.** When it resolves, with `Ok` or with `Cancelled`, no unit, scope child or
  `finalize` of that run is executing or ever will, and everything they wrote happens before
  the resolution. With `Ok` every node has finalized and the graph is handed back, and the
  run holds no reference to it any more: each of its jobs dropped its own before it left the
  run, and every such drop happens before the resolution, so dropping the graph drops each
  node that nothing else holds, on the thread that drops it. After `Cancelled` the run is
  torn: some nodes finalized, some ran in part, their dependents never ran, and the graph is
  not handed back; work that shutdown left queued may hold the graph's nodes until that work
  is dropped. A job looks for shutdown before each unit and before
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
  finishes; and one parked on its own scope's latch holds no permit. A task woken while a
  block holds the gate, as the frame's render task is when its present call returns during a
  physics step, is held to the same: woken from a thread that is no worker, by a job beside the
  block or by its timer, it is polled on a worker the reservation leaves awake, or once the gate
  opens, never by a worker over the limit, and its wake is not lost (`src/executor/gate_tests.rs`).
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
  every scope wait is wrapped in an abort-on-unwind guard, and so is a worker's idle wait,
  where a tracing listener's callbacks run, and the drop of a task's future, on whichever
  thread it happens. The crate installs no panic hook; binaries do.
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
- A task is work like any other at the gate: it is polled only by a worker that holds a
  permit within the limit. A worker that took its permit before a block began and then
  takes a task, from its pool's queue or from its own deque after firing timers, while over
  the limit puts it back unpolled at the back of its pool's queue, as it puts back a job, and
  gives its permit up at its next check of the gate. It wakes nobody, since no permit comes
  free by that: every worker that holds one is awake and looks at the queue before it sleeps
  (`gate::idle_park`), an opener that holds one comes back to the queue when its scope ends,
  one that gave way to the gate is woken when a permit comes free, and the gate's opening
  wakes every worker. A worker that a task's wake finds held at the gate sleeps again beside
  the task; the gate's opening wakes it.
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
  it, or puts it back on the pool's queue if it is over the limit. A worker that finds a timer
  due just as it lies down to sleep queues the task with a wake-up like any other.
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

- Everything is under target `whisk`. The info level's spans are the node and range spans: one
  `tracing` span per node run and one per range split off it, named by `GraphNode::name`, and
  never one per unit. Each distinct node name gets its own call sites, created when a graph is
  built. A job of `run_job` runs under one span named `run_job`, and each drain of a
  `LanePump` that runs a call under one named `lane`, with the lane's name as its field `name`,
  both at info too.
- Two spans at debug level, which `lactose-app-client`'s `record-stall` reads by name, so a
  rename or a removal fails it (D165, D162):
  - `open_gate`, around `Pool::open_gate` in `JobContext::block_in_place`: from the closure's
    return to the gate's return.
  - `park`, around each of the two waits a worker makes without a permit, entered before the
    wait and exited when it returns. Its field `wait` says which: `"idle"` for `sleep.park` in
    `gate::idle_park`, whenever a worker parks in the idle protocol, whatever its last look saw,
    nothing or work with no permit to take it; `"latch"` for `opener.park(latch)` in
    `gate::wait_for_children`, a scope's opener on its latch, which `Pool::open_gate` wakes with
    every other worker. A replay's wait on a latch is not spanned: under a replay the log
    decides when each worker runs.
- Both debug spans are in every build until architecture section 5.1's compile-out of finer
  levels is built: the workspace's `tracing` sets no static maximum level. Where nothing
  listens at debug, each costs what a span nobody wants costs (see the end of this file).
- A listener's callbacks for `open_gate` run on the opener. Those for its creation and entry
  run while the gate is still closed: every frame worker it holds, and the work queued beside
  them, waits for them to return.
- A listener's callbacks for `park` run on a worker that is going to sleep or has just been
  woken, in the idle protocol after it has announced it is asleep, so a waker may already have
  claimed it. They must return promptly, with no wait on another thread beyond a short lock
  that is rarely contended, and must never call into the executor. A callback that waits
  delays that worker's wake and loses none, whatever it waits on: a wait through the thread's
  parker (on macOS a contended `RwLock`, a contended `OnceLock` initialisation or a
  `sync_channel` hand-off) takes the token an unpark left, but the idle wait parks only while
  its sleeper bit says nobody has claimed the worker, and the latch wait checks its latch, its
  permit and shutdown again once the `park` span is entered, after the callbacks, before it
  parks. The repository's own listeners at debug, `record-stall` and this crate's span tests,
  take a `Mutex` there, held by any thread for a few pushes and reads at most: a wait that
  short is what the rule allows, and a wake `record-stall` times includes it.

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

## Measurement calls

`take_foreground_task_role` and `current_core_class` are for cost tests and the benchmark
(D164). The executor calls neither, and nothing in a frame should: the role is the whole
process's, and a core's class is a reading of where the scheduler put a thread, which it may
change at the next instruction. A test takes the role once, before it times anything, and
reads the class around the work it times, never inside it.

**The task role.** On macOS `take_foreground_task_role` asks for
`TASK_FOREGROUND_APPLICATION` with `task_policy_set` on the process's own task, the category
flavor, and returns what `task_policy_get` then reports, with what the role grants: the
scheduled priority (`thread_info`, `THREAD_EXTENDED_INFO`, `pth_priority`) of a thread it then
starts in the frame-critical class, user-interactive QoS. `TaskRole`'s `Display` is the role's
name in `<mach/task_policy.h>`, the two calls' results and that priority, the text a result
file records. The role read back is the request: macOS accepts it however the process was
started, so `is_foreground` holds under `taskpolicy -b` or `-c` and in every launchd job alike,
and only the priority shows what was granted. The record is the priority read back, whatever
it is; there is no one value that means unclamped. A process started from a terminal is in
`TASK_UNSPECIFIED`, where user-interactive QoS is clamped to priority 31. On the M1 under
macOS 27.2, before the request and then after it:

| Started by | Before | After |
|---|---|---|
| A terminal | 31 | 47 |
| launchd, `ProcessType` `Interactive` | 37 | 37 |
| launchd, no `ProcessType` or `Standard` | 20 | 20 |
| launchd, `ProcessType` `Background` or `Adaptive` | 4 | 4 |
| `taskpolicy -c utility` | 20 | 20 |
| `taskpolicy -b` | 4 | 4 |

A child process inherits the start it was given: cargo and nextest started by a launchd job read
as that job does. Every other target has no task roles: the call asks nothing and returns
`TaskRole::NotApplicable`.

The role cannot be given back. On the Apple M1 under macOS 27.2 a request for
`TASK_UNSPECIFIED` returns `KERN_INVALID_ARGUMENT` (4) and the role stays as it was;
`TASK_DEFAULT_APPLICATION` and `TASK_BACKGROUND_APPLICATION` are accepted, and a
user-interactive thread then runs at 46, which is neither role's priority. So a case that
needs the unspecified role runs in a process of its own that never takes the foreground one.

**The class of the current core.** `current_core_class` reads the CPU the calling thread is
on and looks it up in a map of the machine's CPUs, built at the first call and kept for the
process. The map follows the rule `CoreTopology` counts by, so the two agree on a machine:

| Target | Current CPU | Its class |
|---|---|---|
| macOS | `pthread_cpu_number_np` | The logical CPUs of each performance level at boot, `hw.perflevel<n>.logicalcpu_max`: level 0 is the performance class and every other level the efficiency class |
| Windows | `GetCurrentProcessorNumberEx`, its group and number | The efficiency class of its core, from the group masks of the core records `GetLogicalProcessorInformationEx` reports; the lowest class is the efficiency class and every class above it the performance class |
| Linux | `sched_getcpu` | The `cpu_capacity` of its core in sysfs, by the same rule, or on an Intel hybrid part the CPU lists of the `cpu_core` and `cpu_atom` PMUs. A core is its package, cluster and core id (`topology/cluster_id`, 0 where the kernel has none): device-tree arm64 kernels put every CPU in package 0 and restart core ids in each cluster |

Where the system reports every core as one class (one performance level, equal efficiency
classes, equal capacities), every core is `CoreClass::Performance`. The call returns `None`
where it cannot tell: a Linux kernel that exports neither capacities nor the PMU lists, which
is every x86 kernel on a part that is not hybrid; a CPU the map does not hold; every other
target; and Miri.

What it assumes on macOS: that the kernel numbers its CPUs from the slowest performance level
up, efficiency cores first. macOS documents no map from a CPU number to its level. The device
tree of Apple silicon reports one (`ioreg -p IODeviceTree -n cpus`: each CPU's
`logical-cpu-id` and `cluster-type`, `E` or `P`), and on the M1 it names CPUs 0 to 3 `E` and
4 to 7 `P`. The test `the_apple_numbering_is_the_device_trees` holds the map against the device
tree on every Mac it runs on that reports two levels, and fails there if the tree does not
parse; the call itself reads only the sysctls. The counts are the boot's (`logicalcpu_max`),
not the current power mode's (`logicalcpu`): a CPU's number is fixed in `[0, hw.ncpu)`, so a
CPU taken offline must not shift the numbers above it.

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
  timer keeper's hand-off, the task kept by the worker that fires its timer, a task put
  back at a closed gate by a worker over the limit, the quiescence of a cancelled run
  (`src/quiesce.rs`) and its jobs' writes before its resolution, a finished run's jobs
  dropping their references to the graph before it resolves (`Quiesce::leave`), and a
  parking lane's wait under a call that takes its thread's unpark (`src/parking.rs`). The
  models call the shipped functions.
  Under loom a timed park waits on a flag that only `Sleep::wake_worker` sets and that an
  unpark alone does not satisfy, as the Windows wait is not ended by an unpark, so a waker
  that skips it fails the keeper hand-off model.
  The deque is a vetted dependency and is not modelled. loom is enabled by a `loom` Cargo
  feature on this crate that switches an internal `sync` shim, and run with
  `cargo test -p lactose-task --features loom --release`. Never use loom's documented
  `RUSTFLAGS="--cfg loom"`: the environment variable replaces the x86-64 target pin.
  Thirteen models are explored exhaustively: two with three threads, the latch's and the
  references', and eleven with two. Six more are bounded: the sleep protocol's wake for every
  push, the run's quiescence and a cancelled run's writes, each with three threads, at five
  preemptions; the idle release and the put-back beside a retaking opener, with three, at
  four; and the parking lane's, with two, at three (`LOOM_MAX_PREEMPTIONS` overrides the
  bounds). The references' model checks that every job's drop of its reference happens
  before the resolution, by a `Relaxed` store in the drop that the waiting thread reads; it
  fails beside a copy of `Quiesce::leave` that leaves before it drops, and beside a
  `Quiesce::exit` lowered to `Release` and to `Relaxed`. The cancelled run's model checks
  the same of the jobs' `Relaxed` writes when the resolver may be shutdown's
  `Quiesce::close`, and fails beside a `close` lowered to `Release` and to `Relaxed`. The
  put-back's model has two workers hold permits at a gate that allows one, the first
  putting back the task it took; the task is polled, and never taken over the limit, and it
  fails beside a put-back that skips `Gate::release_if_over` at the top of the loop and, as
  a deadlock, beside one made between the worker's look round and its park. Beside a third
  worker that retakes a permit regardless of the limit, as an opener whose latch is set
  does, a worker takes the task over the limit again and puts it back again, and the task is
  still polled within the limit.
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
  of its wake and wait. Six terms are copies in the models, since loom cannot run the types
  they live in: four of `run_job`'s, what the pool's three submissions hand
  `ShutdownFlag::queue` with the sweep they hand it, and `execute`'s put-back of a task. The
  module's doc names each and the native test that holds it.
- `src/executor/gate_tests.rs` holds a task woken while `block_in_place` holds the gate, by
  order and never by a time, with two hooks only its tests compile: one a worker calls just
  before it parks, with what its look round saw, and one it calls holding a permit before it
  looks for work. Woken from a thread that is no worker, with no permit reserved, the task is
  not polled at the closed gate, the worker its wake claimed sleeping again beside it, and is
  polled once the gate opens while the blocking worker stays in its job; with one reserved it
  is polled at the closed gate under that permit. Woken by a job beside the block, the third
  worker its wake claimed sleeps again and the job's worker polls it once the job returns,
  before the gate opens; woken by a job begun before the block, or by its timer, it waits for
  the gate. A wake landed between a worker's look round and its park, and the gate's opening
  landed there, are not lost. A worker that passed its check of the gate as the gate closed
  and takes the task over the limit puts it back once and sleeps, and the task is polled once
  the gate opens, or at the closed gate by the worker the reservation leaves awake; the
  put-back wakes no worker, a sleeper held just before its park still announced once the
  worker that put the task back is about to park.
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
  crosses to another worker, which the Miri runs check too. `src/pool.rs` holds a job and a
  task taken at a closed gate, and a task its timer woke and the firing worker kept on its
  deque, each put back unstarted or unpolled, the tasks on the pool's queue; a job taken after
  shutdown began, and a task taken then over the limit, swept by its own put-back, since
  shutdown's first sweep may have passed and a shutdown that does not join sweeps only once;
  and a job, a task and a graph's range each queued as shutdown begins, the flag set between
  the push and the read by a hook that only its tests compile, and swept by the submission
  itself. `tests/job_alloc.rs` counts a call's allocations, in a binary of its own because it
  installs a global allocator.
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
  finalize, a scope child, a scope body, a task, a tracing listener's callback in a worker's
  idle wait or the drop of a cancelled task's future aborts the process, and that an
  unfollowable replay log does.
- `tests/spans.rs` checks the spans by level, and where the two debug spans stand against
  what they bracket: a frame worker held at a closed gate beside work is inside an idle
  `park`; `open_gate` is entered after the closure returns and while that worker is still
  held, and exited after the gate has let it go; an opener stays inside its latch `park` for
  as long as it waits; and a listener whose callback waits through the thread's parker after
  the last child's unpark does not cost the opener its wake-up.
- `src/measure.rs` holds the measurement calls: on macOS the foreground role taken and read
  back with the priority it grants, 47 where the process started at 31, never below the
  priority before it, and again with no change; the class map against the device tree's
  (above) and against the topology's counts; a class read on every call on four threads
  wherever the system reports the cores' types; on a Mac with two levels, as many spinning
  user-interactive threads as CPUs reading both classes between them, since macOS cannot pin
  and the performance cores alone cannot hold them all; and on Linux and Windows a thread
  pinned to each CPU the process may use in turn, where every reading is the same and is that
  CPU's. `src/platform.rs` holds the maps built from hand-made input: Apple levels, Linux
  capacities and PMU lists (and a device-tree arm64 machine where each cluster restarts its
  core ids at 0), and Windows core records with their group masks, on any host; and the Linux
  sysfs reader over a temporary tree, with and without `topology/cluster_id` and with an
  offline CPU. The tests that call the system are ignored under Miri. CI's macOS leg runs the
  role read-back and the class-map tests again with their output shown, so its log records
  the priority before and after the request and the levels the runner reports.

What the implementation relies on in its pinned dependencies:

- loom 0.7.2 treats `SeqCst` loads and stores as `AcqRel` and does not explore load
  buffering, but it does model `fence(SeqCst)`. The sleep protocol is therefore built on
  explicit fences, which loom checks as written, and not on the orderings of the accesses
  around them.
- A span costs about 1 ns with no subscriber and about 8 ns with a subscriber that does
  nothing (`cargo test -p lactose-task --release --lib -- --ignored --nocapture span_overhead`).
