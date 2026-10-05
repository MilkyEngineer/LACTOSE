# Unsafe in lactose-task

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. The crate's tests run under Miri, with the stand-in deque described in
`src/deque.rs`; the scope latch and the sleep protocol, which decide when the borrows below
end, are modelled under loom.

Architecture section 21 lists three reasons for unsafe here: deques, scope borrows and the
panic guard. The deque itself is `crossbeam-deque` and needs no unsafe of ours beyond sending
entries through it. The panic guard (`guard.rs`) turned out to need none: it is a value whose
destructor aborts and which is forgotten on the normal path. What remains is below.

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
  shuts down runs the children left on its deque instead of dropping them, and no other code
  path drops a child entry (`Entry::discard` rejects one).
- So for as long as any worker holds a child entry, or is inside `run_child` before its
  count-down, the `ScopeCore` in the opener's frame, the job storage in the opener's scope
  arena, and everything the closure borrowed are alive.

The wait itself rests on a second invariant, which is about liveness, not memory: while a
scope is open, the opening worker's deque receives only that scope's children
(`Shared::submit_range` sends everything else to the injector), so the opener can find every
child that is still on its deque by popping. Breaking it would hang the opener; it could not
free anything early, because the first invariant holds regardless.

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
  `opener`, `tag`, `position`, `call` and `ordinal` through the raw pointers. Those fields
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
- The benchmark (`benches/whisk.rs`) calls `getrusage` on Unix with a pointer to a zeroed
  `rusage` and reads it only if the call succeeded.
