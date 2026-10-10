# Unsafe in lactose-ecs

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. The crate's tests run under Miri.

Architecture section 21 admits `unsafe` here for chunk column storage and for the
`#[system]` expansion's call of `unsafe SystemDescriptor::new`. The naive implementation
needs only the second: storage is safe code, and the crate holds three `unsafe fn`
(`SystemDescriptor::new`, `Finalize::new`, `Finalize::call`) and one `unsafe` block, all on the path from an access table to the code that fetches by it. A
change that adds a place amends this file in the same commit.

## The access tables (`SystemDescriptor::new`, `Finalize::new`, `Finalize::call`)

The schedule orders systems on the strength of the descriptor's access table: two systems
that conflict on a target are ordered, and within a system each parameter names a different
component and no resource is written beside another parameter on it. A table that
understates what the kernel touches would let the parallel implementation of milestone 1
hand two kernels `&mut` to one row from safe code. So `SystemDescriptor::new` is `unsafe`,
with the obligation that the kernel's table and the finalize's table describe exactly what
each entry point fetches, and the `#[system]` expansion, which derives both from the
parameter types, is its one caller.

The `#[finalize]` pairing is by name: `#[finalize(f)]` emits `__finalize_f`, a
`Finalize<O>` holding the finalize function's table and its entry point, and the
`#[system]` expansion passes `__finalize_f.access()` to `SystemDescriptor::new` and runs
the entry point through `Finalize::call`. `Finalize::new` is `unsafe` with the same
obligation as `SystemDescriptor::new`, so safe code cannot supply a `__finalize_f` whose
entry point fetches what its table does not declare; `Finalize::call` is `unsafe` with the
obligation that the invocation was prepared from that table, which the schedule does for
the finalize of the descriptor built with it. The one `unsafe` block in the crate is the
call inside `Finalize::call`, which relies on the two obligations together.

The expansions' `unsafe` blocks are spanned at the call site, never at a user token: the
`unsafe_code` lint exempts code another crate's macro generates only under a macro span, so
the invoking crate keeps `#![forbid(unsafe_code)]` (section 21, as for `#[component]`).
`tests/compile_fail.rs` compiles every kind of system in such a crate.

In the naive implementation a table that disagrees with its entry point cannot cause
undefined behaviour: each parameter is fetched from a value prepared for its index by safe
code, and a mismatch is a panic ("parameter N does not match the system's access table").
The obligation exists for the parallel implementation, which will hand out rows to
concurrent jobs by the table.

## Chunk columns (no `unsafe`)

A chunk is a slab of 8-byte words holding one contiguous column per component of its
archetype, each column starting at a multiple of eight, typed only by the registry's
`TypeInfo`. The world refuses to build with a component aligned to more than eight bytes,
so every column is aligned for its type.

- Typed access (`&T`, `Mut<T>`, `ColumnRef<T>`, `World::get`) is a checked `bytemuck` cast
  of a row's or a column's bytes: the bytes were written from a `T` or validated with `T`'s
  `CheckedBitPattern` on load, and the cast checks them again.
- Several columns of one chunk are reached at once by splitting the slab with
  `split_at_mut`, and one row of each by splitting again, so the `&mut` a `Mut` holds and
  the `&` a `&T` holds are disjoint borrows the compiler checks.
- `Ctx` holds the region's seed, tick and asset table, not the world, so no `&World` lives
  beside the rows a kernel writes.
- A component value with a placeholder in a reference field is not a valid value of its
  type until the placeholder resolves, so command buffers keep such values typed and
  rewrite them through `visit_references` before their bytes are validated and stored.

## Sharing the world with a graph run (no `unsafe`)

`Schedule::run_tick` and `run_pass` take the `World` by value and move it, with the plan,
into a mutex inside the one graph node of the run. The node's job locks it and runs every
sync point and system in order; after the run resolves the caller takes the world back.
The mutex is taken twice per run and never contended. If the executor has shut down first,
the run resolves to `Cancelled`, the node never ran, and the world drops with it. A future
dropped mid-run leaves the node to finish and drop the world; a forgotten one leaks it. No
borrow outlives the run.
