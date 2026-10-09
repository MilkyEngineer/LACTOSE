# Unsafe in lactose-ecs

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. The crate's tests run under Miri.

The facade contains no `unsafe`. Architecture section 21 admits it here for one reason, chunk
column storage, and the implementation that replaces the facade's bodies is expected to need
it in two places. Each is written up below as the invariant the implementation must state and
keep; a change that needs a third place amends this file in the same commit.

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
  systems that conflict on a column, and within a system each chunk job owns disjoint rows.

## Lending the world to a graph run

`Schedule::run_tick` lends `&World` or `&mut World` to chunk jobs that the executor runs on
other threads through `'static` graph nodes.

- The borrow is sound because of the `GraphRun` contract in `crates/task/README.md`: when a
  run resolves, with `Ok` or with `Cancelled`, no unit, scope child or finalize of that run is
  executing or ever will. `run_tick` does not return while a run of this tick is unresolved.
- The nodes hold a raw pointer that is dereferenced only inside `run` and `finalize` of that
  run, never stored elsewhere and never after the run resolves.
