# Unsafe in lactose-ecs

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. The crate's tests run under Miri.

The facade contains one `unsafe fn` and no `unsafe` block. Architecture section 21 admits
`unsafe` here for one reason, chunk column storage, and the implementation that replaces the
facade's bodies is expected to need it in two places. Each is written up below as the
invariant the implementation must state and keep; a change that needs a third place amends
this file in the same commit.

## The access table (`SystemDescriptor::new`)

The schedule hands a kernel `&mut` rows and resources on the strength of the descriptor's
access table: two systems that conflict on a target are ordered, and within a system each
parameter names a different component. A table that understates what the kernel touches
would let two kernels alias one row from safe code. So `SystemDescriptor::new` is `unsafe`,
with the obligation that the table describes exactly what the kernel touches, and the
`#[system]` expansion, which derives the table from the parameter types, is its one caller.
The `unsafe` call lands in the invoking crate; the `unsafe_code` lint does not apply to code
another crate's macro generates, so that crate keeps `#![forbid(unsafe_code)]` (section 21,
as for `#[component]`).

## Chunk columns

A chunk is a 16 KiB slab holding one contiguous column per component of its archetype, typed
only by the registry's `TypeInfo`. Typed access (`&T`, `Mut<T>`, `ColumnRef<T>`) is a cast of
a column's bytes to `T`.

- A column is cast to `T` only when the column's component key is `T::KEY`, and the registry
  holds one `TypeInfo` per key, so the bytes were written as `T` or validated against `T`'s
  `CheckedBitPattern` on load or receive (architecture section 6).
- Column offsets are aligned to the component's alignment at chunk construction; an
  archetype's layout is fixed for its life.
- Row count never exceeds capacity; a row beyond the live count is never handed out.
- Two kernels never hold `&mut` to one row: within a stage the schedule orders every pair of
  systems that conflict on a column, within a system the schedule rejects two parameters on
  one component, and within a system each chunk job owns disjoint rows.
- `Ctx` holds `&World` beside live `Mut` rows of the same world, so the implementation keeps
  column storage behind `UnsafeCell` and never forms a `&` to a row a `Mut` covers.

## Sharing the world with a graph run

`Schedule::run_tick` and `run_pass` take the `World` by value and share it with chunk jobs
that the executor runs on other threads through `'static` graph nodes.

- The run owns the world for its duration (an `Arc` the nodes share, or storage moved into
  the run) and the future hands it back only after the run has resolved. A future that is
  dropped or forgotten mid-tick therefore leaks the world: no borrow outlives the jobs.
- The jobs' accesses are disjoint by the access-table argument above, and `World: Sync`
  covers the shared reads.
- When the run resolves, with `Ok` or with `Cancelled`, no unit, scope child or finalize of
  that run is executing or ever will (`crates/task/README.md`, "Resolution"); on `Cancelled`
  the world is torn and is not handed back.
