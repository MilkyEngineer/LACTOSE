# Unsafe in lactose-task

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. The crate's tests run under Miri, with the stand-in deque described in
`src/deque.rs`; the scope latch and the sleep protocol, which decide when the borrows below
end, are modelled under loom.

Architecture section 21 lists the reasons for unsafe here: deques, scope borrows, the panic
guard, task futures, core-topology queries, the keeper's timed wait on Windows, and the
benchmark's and tests' process-CPU and task-role calls. The deque itself is
`crossbeam-deque` and needs no unsafe of ours beyond sending entries through it. The panic
guard (`guard.rs`) turned out to need none: it is a value whose destructor aborts and which
is forgotten on the normal path. What remains is below.

## Scope borrows (`job.rs`, and the `Send` impl in `pool.rs`)

A child job borrows from the opener's stack (`'scope`, `'env`) but is stored and run through
raw pointers that carry no lifetime. One invariant makes that sound:

> **A child entry exists only while its scope's latch counts its units, and
> `JobContext::scope` does not return while the latch is non-zero.**

- `Scope::submit` adds a job's units to the latch before it creates the entry. Splitting an
  entry divides its units between two entries and leaves the count alone. `run_child` counts
  down exactly the units it ran, after running them, and creates no entry afterwards.
- `JobContext::scope` waits for the latch in every path, and runs its body and the wait under
  the abort-on-unwind guard, so a panic cannot unwind past borrowed data. A worker that
  shuts down runs the children left on its deque instead of dropping them, an opener whose
  children wait on the pool's queue for the work of `Executor::run_job` runs the ones left
  there, and no other code path drops a child entry (`Entry::discard` rejects one, and the
  shutdown sweep never reads that queue).
- So for as long as any worker holds a child entry, or is inside `run_child` before its
  count-down, the `ScopeCore` in the opener's frame, the job storage in the opener's scope
  arena, and everything the closure borrowed are alive.

The wait itself rests on a second invariant, which is about liveness, not memory: while a
scope is open, the opening worker's deque receives only that scope's children
(`Shared::submit_range` sends everything else to the injector, and `Shared::keep_woken_task`,
which puts a task woken by a timer on the firing worker's own deque, refuses while the worker's
`scope_open` is set, so that task goes to the injector too), so the opener can find every
child that is still on its deque by popping. The scopes of a job of `Executor::run_job` are
the exception: their children, the halves split off them and the rest a worker puts back go
to the pool's queue for such work and never to a deque, and the opener takes them there
(`WideChildren` in `job.rs`). That queue holds the children of one scope at a time, because
such jobs run one after the other (`width.rs`) and a job opens its scopes one after another.
Breaking either would hang the opener; it could not free anything early, because the first
invariant holds regardless.

What follows from the first:

- **`unsafe impl Send for Entry`.** The raw pointers of a `Child` entry stay valid on any
  thread by the invariant. What they point to may be used from that thread because
  `Scope::fork` requires `F: Send` (the closure runs once, on one thread) and
  `Scope::fork_range` requires `F: Send + Sync` (it is called through `&F` on several).
- **Job storage.** `Scope::store` writes a `#[repr(C)]` job, header first, into memory from
  the scope arena, which hands out aligned, unaliased memory that stays put until the arena
  is reset, and the arena is reset only after the wait. Only the opener allocates from the
  arena (`Scope` is neither `Send` nor `Sync`); other workers read job bytes, never the
  arena's bookkeeping. The header's `call` pointer is instantiated for the job's concrete
  type, so casting the header pointer back to that type is sound.
- **`OnceJob`.** One unit, so it lives in exactly one entry and `call` runs at most once. It
  moves the closure out with a read and never touches the slot again; the arena drops
  nothing, so there is no double drop. Every entry is run, so there is no leak either.
- **`RangeJob`.** Shared by reference by every worker running part of the range. If the
  closure needs dropping, a `DropEntry` in the same arena records it, and the opener drops
  it in place after the wait, when no worker can still hold a reference.
- **Reading the scope from another worker.** `run_child` and `chaos::child_identity` read
  `opener`, `tag`, `position`, `wide`, `call` and `ordinal` through the raw pointers. Those fields
  are written before the first entry exists and never change.
- **The last count-down.** The decrement that brings the latch to zero lets the opener leave
  its frame at once, so it is the last access a child makes: `Latch::count_down` touches
  nothing after its `fetch_sub`, and `run_child` wakes the opener through the pool's thread
  table, which outlives the scope. `run_child` forms a reference to the latch only, which
  consists entirely of atomics, never to the whole `ScopeCore`, so no reference to plain
  fields is live across the point where the frame may end. The loom model gives the latch a
  tracked cell that stands for the frame and fails on any access that can race its end.

## Task futures (`task.rs`)

- A task's future lives in an `UnsafeCell` inside the task's `Arc` allocation. The task's
  phase (`IDLE`, `SCHEDULED`, `RUNNING`, `NOTIFIED`, `DONE`) admits one thread at a time to
  it: `run` moves `SCHEDULED` to `RUNNING` and holds the future until it leaves that phase,
  and `cancel` takes the future only by moving `IDLE` to `RUNNING` itself. A queue entry is
  the only thing that may call `run`, and a task has at most one entry, because only the
  `IDLE` to `SCHEDULED` and `NOTIFIED` to `SCHEDULED` transitions create one.
- `unsafe impl Sync for TaskCell<F>` rests on that exclusion, on `F: Send` (successive polls
  may be on different threads) and on `F::Output: Send` (the output crosses threads through a
  mutex).
- The future is pinned by construction: it is never moved out of the `Arc` allocation, and it
  is dropped in place, by `finish` or, for a task that lost its handle and every waker while
  suspended, by the cell's own destructor, which has exclusive access by `&mut self`.

## The keeper's timed wait (`sync.rs`, Windows)

`TimedPark` (`sync.rs`, Windows) holds two kernel handles, an auto-reset event and a
waitable timer. A handle is a process-wide value and is valid on every thread. Every call
made on them, `SetEvent`, `SetWaitableTimer` and `WaitForMultipleObjects`, is safe from any
thread, concurrently with the others. Both handles are closed once, in `Drop`.

- **`unsafe impl Send` and `unsafe impl Sync` for `TimedPark`.** A handle is a raw pointer
  in `windows-sys`, so neither is derived. Both rest on the paragraph above. The one
  condition memory safety needs is that a handle is open whenever a call is made on it, and
  that comes from ownership: `Drop` has `&mut self`, so no other call is in progress or can
  begin, and the pool owns the `TimedPark` for as long as any worker or waker can reach it.
  `Drop` may run on a worker's thread as that worker exits. That only the owning worker sets
  the timer and waits is a liveness rule, not a soundness one: a second waiter would only
  mistime a wait.
- The pool makes the handles when it is built, before any worker enters its loop, and they
  live as long as the pool, so every wake and wait that reaches one through the pool finds it
  open. Under Miri `TimedPark` is `std::thread::park_timeout` and makes no Windows call.
- **Ordering.** Not a memory-safety matter, but the kernel carries it and no model checks
  it, so it is stated here. A woken worker has to see the claim that woke it. The release is
  `SetEvent`, made after the claim's `SeqCst` read-modify-write (`Sleep::wake_worker`); the
  acquire is the wait that the signal ends, followed by the `SeqCst` load of the bit in
  `Sleep::park`. Two claims before one wait set the event twice and the wait resets it once;
  that wait comes after the second `SetEvent` in the event's order, so it sees both claims,
  and a bit load follows it. A wait the timer ends orders nothing; a claim its bit load
  misses has set the event or will, which ends the next wait. This rests on Microsoft's
  statement that the signal and wait functions are memory barriers ("Synchronization and
  Multiprocessor Issues"). loom cannot check it: loom 0.7.2's unpark joins the unparker's
  causality into the target at once, and its `SeqCst` fence joins a global one, so the
  models pass with the loom flag's swaps `Relaxed`.

## Operating-system queries (`platform.rs`)

- macOS: `sysctlbyname` is called with a NUL-terminated name, a pointer to a live `i32` and
  its size, and no new value.
- Windows: `GetLogicalProcessorInformationEx` is called once with a null buffer to learn the
  size and once with a writable buffer of at least that size, aligned for the record type.
  That is all the unsafe there is: the buffer is then copied out as plain bytes and walked by
  `detect::windows_core_ratings`, safe code that trusts only each record's eight-byte header
  and bounds every read by the buffer. No record is read through its declared type, whose
  size (that of its largest variant) is larger than a processor-core record.
- Linux reads sysfs with safe code.

## Process-CPU and task-role calls (`benches/whisk.rs`, `tests/idle_cpu.rs`)

- `getrusage` is called on Unix with a pointer to a zeroed `rusage`, which is read only if
  the call succeeded: the benchmark returns nothing otherwise, and `tests/idle_cpu.rs`
  asserts that it succeeded.
- On Windows the benchmark calls `QueryProcessCycleTime` and `QueryThreadCycleTime` with the
  pseudo handle of the current process or thread, which is always valid, and a pointer to a
  live `u64`, which it reads only if the call succeeded.
- On macOS the benchmark declares `task_policy_set` and `task_policy_get` from
  `<mach/task_policy.h>` itself and calls them on the process's own task port with the
  category flavor, whose policy is one `integer_t`: a pointer to one live `i32` with a count
  of one, and for the query live out-pointers for the count and the default flag. The calls
  write only through those pointers.
