# Unsafe in lactose-ecs

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. The crate's tests run under Miri.

Architecture section 21 admits `unsafe` here for chunk column storage and for the
`#[system]` expansion's call of `unsafe SystemDescriptor::new`. The crate uses both, in two
places. On the path from an access table to the code that fetches by it: three `unsafe fn`
(`SystemDescriptor::new`, `Finalize::new`, `Finalize::call`) and one `unsafe` block. In the
schedule run (`node.rs`), which lends the world's columns, resources and event lists to the
nodes of a graph through raw pointers: the `Send` and `Sync` impls of `StageView`, its ten
`unsafe` methods, the three `unsafe fn` that lend a table's resources and event lists
through it, and the blocks that call them. Storage itself is safe code. A change that adds
a place amends this file in the same commit.

## The access tables (`SystemDescriptor::new`, `Finalize::new`, `Finalize::call`)

The schedule orders systems on the strength of the descriptor's access table: two systems
that conflict on a target are ordered, and within a system each parameter names a different
component and no resource is written beside another parameter on it. A table that
understates what the kernel touches would let the parallel implementation of milestone 1
hand two kernels `&mut` to one row from safe code. So `SystemDescriptor::new` is `unsafe`,
with the obligation that the kernel's table describes exactly what both kernel entry points
fetch (`Kernel::run`, the one call of a per-region, once-per-tick or exclusive system, and
`Kernel::each`, a per-entity system's loop over the rows of one unit) and the finalize's
table what the finalize fetches. The obligation is positional: entry N of the kernel's table
describes parameter N of `Kernel::run` and of `Kernel::each`, and entry N of the finalize's
table parameter N of `Kernel::finalize`, because the run prepares and lends each parameter
by its index. The `#[system]` expansion, which derives the tables from the parameter types
in order and the `writes` list, is its one caller. A `writes` entry fetches nothing: it
declares a write the system makes through commands or `&mut World`, outside every kernel,
and only adds an ordering constraint, so it can make a table overstate what the kernel
touches but never understate it. A `ChannelWriter` fetches a sink the run made for that
parameter alone and appends to the invocation's buffer after the kernel, and a `ChannelTake`
fetches the store the run removed from the world before lending it, so neither aliases what
another parameter holds (D126).

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

Today a table that disagrees with its entry point cannot cause undefined behaviour: each
parameter is fetched from a value prepared for its index by safe code, through a checked
cast or downcast. For `Kernel::each` each parameter checks, once per unit, that the table
entry it takes is its own: a column parameter that its entry names the same component (and
the lent bytes are exactly the unit's rows of that type), a `RegionRes` that its entry is
that region resource, and every other parameter the run prepares something for (an output
sink, `Res`, `Events`, `Channel`) that its entry is its own parameter type's, kind and
target. A mismatch is a panic ("parameter N does not match the system's access table")
before the kernel reads a row; `tests/obligation.rs` holds a kernel for each of those
kinds. `Kernel::run` checks only the kind of each prepared value: a parameter of another
kind panics with the same message, but one of the right kind and another type reads the
declared value as its own wherever the cast or downcast allows it, as an empty event list
or channel, or a region resource of the same size.

The run forks per-entity units over workers on the same rule: lending is per table entry,
the bytes lent for an entry are the unit's rows of the column that entry names, and each
parameter takes them through the checked cast above. A kernel whose code disagrees with its
table can then misread only what its table declared, and soundness rests on the schedule's
conflict sets and the lending code (`node.rs`, below), never on a kernel. Nothing is cast
by parameter type from a raw pointer.

## Chunk columns (no `unsafe`)

A chunk is a slab of 8-byte words holding one contiguous column per component of its
archetype, each column starting at a multiple of eight, typed only by the registry's
`TypeInfo`. The world refuses to build with a component aligned to more than eight bytes,
so every column is aligned for its type.

- Typed access (`&T`, `Mut<T>`, `ColumnRef<T>`, `World::get`) is a checked `bytemuck` cast
  of a row's or a column's bytes: the bytes were written from a `T` or validated with `T`'s
  `CheckedBitPattern` on load, and the cast checks them again.
- Outside a schedule run, several columns of one chunk are reached at once by splitting the
  slab with `split_at_mut`, and one row of each by splitting again, so the `&mut` a `Mut`
  holds and the `&` a `&T` holds are disjoint borrows the compiler checks. Inside a run the
  nodes reach the slab through the stage view (below), which hands out the same slices.
- `Ctx` holds the region's seed, tick and asset table, not the world, so no `&World` lives
  beside the rows a kernel writes.
- A component value with a placeholder in a reference field is not a valid value of its
  type until the placeholder resolves, so command buffers keep such values typed and
  rewrite them through `visit_references` before their bytes are validated and stored.

## The run as a graph (`node.rs`)

`Schedule::run_tick` and `run_pass` move the `World` into the state a graph's nodes share
and take it back when the run resolves. The graph has a node per sync point, a node per
system that is not exclusive, and per stage a node per event type the stage both emits and
reads. Its edges: each sync point before the next; each system node of a stage after the
stage's opening sync point, before its closing one, and after its ancestors in the plan;
each event-merge node after every emitter of its type in the stage and before every
reader. The executor (`lactose-task`) runs a node's `units` before any of its units, every
unit before its `finalize` and the `finalize` before any dependent's `units`, each step
ordered after the last by its counters' acquire-release operations. That order is the
happens-before every argument below rests on.

**Phases, held by locks that are never waited on.** The world sits in an `RwLock`: a sync
point holds it alone and every other node shares it. The `StageView` sits in another,
written by sync points alone. Each system's `SystemWork` is written alone by sync points and
by its node's `units` and `finalize`, and shared by the node's unit jobs and by the merge
nodes after it; its result slots, event buffers and hand-off buffers are `Mutex`es. Every
lock is taken with `try_*`, and one that the graph's order should have left free panics ("a
node of the run outlived its phase"), so a missing edge aborts the process rather than
racing. A panic in a job aborts the process in any case (`lactose-task`), so no later node
sees a poisoned lock.

**The pointers.** With the world held alone, a stage's opening sync point takes, once per
allocation, the pointers its nodes reach the world through: each chunk's slab through
`Slab::as_mut_ptr` (`Vec::as_mut_ptr`, which creates no reference to the slab), its column
ticks and the row ticks and positions of each tracked column through `Vec::as_mut_ptr`, each
resource the stage's systems name through a `&mut` of its boxed value, and the list of each
event type the stage reads or merges through `get_mut` on the store, after inserting an
empty list for every such type, so that no insertion moves a list another pointer
addresses. The next sync point drops every pointer before it borrows the world again.
Between the two every node holds the world only shared, and nothing in a stage changes the
world's structure: commands, events and presentation events are buffered per unit and per
system and reach the world at sync points, so no allocation the view addresses is moved,
resized or freed while it lives. A node's `&World` covers the world's own fields and its
containers' headers, never the buffers the view addresses, and the code under it creates no
reference to a slab, a tick buffer, a resource's value or an event list: it reads chunk keys
and lengths, regions, channels and the alias tables, which no node writes.

**What each access may touch, by the schedule's conflict sets.** Two systems of a stage that
conflict are ordered, or the schedule refuses to build (`UnorderedConflict`): on a component
when one writes it (`Mut`, `Option<Mut>`, `writes`) and the other touches it (any data
parameter or `Changed`), on a resource when either writes it. Each `unsafe` method's
obligation is one of the following, and its caller is the node of the system whose table
grants it, not skipped:

- `bytes`: rows of a column the system reads through a data parameter, or of its region or
  claim column. No system that writes the column runs beside it, and none writes the region
  or claim column (the schedule refuses `Mut` of a world-maintained component, and
  `Lending::new` asserts it).
- `bytes_mut` and `stamps_mut`: a unit's rows of a column the system writes, and their row
  stamps. Every system that touches the column is ordered with this one; within the system
  the column is lent to one parameter (`AliasedParameter`), and the units of one chunk are
  disjoint row ranges, each run once: the executor hands each unit index to one job, and
  the job asserts the unit's result slot was empty.
- `column_tick` and `set_column_tick`: one element of a chunk's column ticks, read or
  written as a value through the pointer, never as a slice, by `units`, which runs once per
  system and run. A system writes the ticks of the columns it writes and reads those of the
  columns it filters with `Changed`, so two systems that run beside each other touch
  different elements.
- `row_stamp`: a row stamp of a column the system filters with `Changed`, read by `units`;
  every writer of the column is ordered with it.
- `resource` and `resource_mut`: a world resource the system reads, or writes from a
  once-per-tick kernel or a finalize (`ResourceWriteInKernel` refuses a resource write in a
  per-entity or per-region kernel). One invocation is lent each resource once:
  `lend_resources` makes one handle per resource, and `Env` hands a written one to one
  parameter and panics on a second.
- `list` and `list_mut`: an event type's list. The one node that writes it in a stage is the
  type's merge node, which every reader of the type in the stage follows (`check_events`
  orders every emitter before every reader, and the graph puts the merge between them). A
  type no system of the stage reads is merged at the next sync point, and two merge nodes
  write different lists.

What a unit or a finalize makes stays in its system's own slots until the system's finalize
or a later node takes it, so outputs reach the world in the order D117 fixes whatever order
the jobs ran in: units in unit order, sources by key, emitters by plan position.

`StageView` is `Send` and `Sync` because it holds only the pointers, and everything reached
through them goes through the `unsafe` methods above. If the executor shuts down mid-run,
the run resolves to `Cancelled` once no job of it is executing, and the world drops with the
shared state; the view's pointers are never used again. A future dropped mid-run leaves the
jobs to finish and drop the world; a forgotten one leaks it.
