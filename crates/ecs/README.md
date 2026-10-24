# CASEIN (`lactose-ecs`)

Chunked archetype storage and the scheduler: the surface every simulation crate writes
against.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | yes, see `SAFETY.md` |

**Status: naive implementation.** Every behaviour below is implemented and tested; nothing
is fast. Storage is real (archetypes, chunks, column ticks, row stamps, SOUR's layout
options), and a schedule run is one graph node on the executor that runs every sync point
and every system in execution order on one worker, without forking over chunks. Milestone 1
measures and rewrites the hot paths behind the same surface. Architecture sections 5.2, 6
and 6.1 are the design; where this file and the architecture disagree, the architecture
wins and this file is wrong. Decision D114 records the choices the sections left open and
what the design review changed.

Amending a signature is allowed in the change that needs it. Say so in the change description
and list what changed, because `sim`, `predict`, `physics-api`, `net`, `persist`,
`present-api` and the game crates are written against this surface.

## Surface

| Item | Purpose |
|---|---|
| `World`, `WorldBuilder`, `WorldKind`, `WorldSet`, `LayoutConfig`, `ChunkOrder` | The world, built for one kind with a `lactose_reflect::Registry`, a start tick and SOUR's layout options |
| `#[system]`, `#[finalize]`, `SystemDescriptor`, `SystemSet`, `Access` | A system is a function; the attribute emits a descriptor static with its access table |
| `Res`, `ResMut`, `RegionRes`, `&T`, `Mut`, `Option<&T>`, `Option<Mut<T>>`, `With`, `Without`, `Changed`, `Ctx`, `StableKeyOf`, `Job`, `&mut World` | System parameters, each a `SystemParam` with a constant `ACCESS` |
| `Commands`, `Bundle`, `CommandChannel`, `ChannelWriter`, `Channel`, `ChannelTake` | Structural commands applied at the next sync point; typed channels for other crates' commands, written through a `ChannelWriter<C>` parameter, read through `Channel<C>` and taken through `ChannelTake<C>` in an exclusive system |
| `Event`, `EventWriter`, `Events` | Cross-entity effects within a tick, indexed by target |
| `PresentEvent`, `PresentEvents`, `PresentEventBatch` | Requests to presentation, keyed by (origin, tick, kind, sequence) for rollback |
| `Schedule`, `ScheduleBuilder`, `ScheduleError` | Stages separated by sync points; a deterministic order; `run_tick` and `run_pass`; `ScheduleBuilder::order`, an edge between two sets added by the assembler |
| `Executor` | Re-exported from `lactose-task`: what `run_tick` and `run_pass` take, so a boundary crate that drives a schedule (`sim`, `predict`) names it without a `task` edge |
| `RegionId`, `RegionMode`, `RegionConfig`, `InRegion`, `OwnedBy`, `ClaimedBy`, `PredictedOrigin` | Regions and the components the world maintains |
| `World::claimed`, `Claimed` | The entities one player has claimed, from an index the world keeps beside the relationship index |
| `Rng` | The counter-based generator, from `Ctx::rng` and `World::rng` |
| `ChangeMark`, `World::changed_since`, `World::is_current`, `World::columns`, `ColumnMut`, `DirtyRows`, `ErasedColumn::changed_since`, `ChangedIndices`, `StructuralLog`, `LogCursor` | Change state read per consumer, typed or from the registry; whole-column access that records what it changed |
| `Query`, `QueryItems` | Whole-world iteration for exclusive code and tests, in key order |
| `WorldSnapshot` | The prediction world's rollback state |
| `World::region_mode` | A loaded region's mode in one lookup, for a consumer that reads every region's mode at every sync point (`World::region` counts the region's entities) |
| `World::load_region`, `World::export_region`, `RegionRows` | Bulk copy of region rows and resources to and from `lactose_bundle` documents |
| `World::get_erased`, `World::insert_erased`, `World::remove_erased`, `World::spawn_erased` | One entity's components by key from the registry, `lactose_bundle::ComponentData` in and stored bytes out: what `lactose-predict` and `net`'s generic hook are written against (D121, D122) |

The hidden `__private` module and the hidden items of `SystemDescriptor`, `Bundle`,
`QueryItem`, `QueryItems` and `Kernel` are the macro's and the implementation's, and may
change with them. Their shape today:

| Item | What it is |
|---|---|
| `Kernel { run, finalize, reducer }` | The entry points: `fn(&mut Invocation)` each, and a reduction's `ReducerOps` (how to make and merge its partials without naming the reducer) |
| `Invocation<'r, 'j>` | One value per parameter, prepared from the access table by the schedule, each taken once; and a reduction's partial or merged total |
| `Fetch<'r, 'j>`, `fetch(invocation, index)` | How each parameter type is made from its prepared value; implemented for exactly the `SystemParam` types |
| `reduce::<R>(invocation, item)`, `finish::<R>(invocation)` | A kernel's item into the partial; the merged total out for the finalize |
| `Finalize<O>` | What `#[finalize(f)]` emits as `const __finalize_f`: the finalize's access table and its entry point, built by an `unsafe` constructor |
| `writes::<T>()` | The `AccessKind::Writes` entry `writes = [T]` adds to a table |
| `BundleOut::push`, `QueryItem::fetch_item`, `QueryItems::{access, fetch_items}` | A bundle's components without their types; a query row's items |

## The `#[system]` attribute

```rust
#[system(name = "game_sim::apply_gravity", worlds = [server, prediction], in_set = sets::MOVEMENT)]
fn apply_gravity(dt: Res<TickDelta>, g: Res<Gravity>, vel: Mut<Velocity>, mass: &Mass) {
    vel.y -= g.0 * dt.0;
}
```

Arguments, in any order:

- `name = "crate::name"`, required. Its xxh64 is the `SystemKey`, which breaks ordering ties
  and orders command buffers; the schedule rejects a collision.
- `worlds = [server, client, prediction]`, any subset; omitted means `[server, prediction]`.
  A system runs in the client's replicated world only if it names `client`, since that world
  runs no simulation systems (architecture section 5.3).
- `in_set = path`: a `static SystemSet`.
- `after = f` or `after = [f, g]`: paths to system functions; each resolves to that
  function's descriptor static (`f` to `F`, `m::f` to `m::F`).
- `reduce = R`: the function returns `R::Item` for an `R: lactose_math::Reducer`, and a
  `#[finalize(f)]` function in the same module takes `R::Output` first.
- `exclusive`: the function's first parameter is `&mut World`.
- `writes = [T, ..]`, on every kind of system: components the system writes outside its
  parameter list, through `Commands::set` or, in an exclusive system, through
  `&mut World`. Each adds an `AccessKind::Writes` entry for `T` to the table, after the
  parameters' entries; nothing is fetched for it. The schedule counts it as a write of the
  column: it conflicts with `Mut`, `&T` and `Changed<T>` of `T` in the stage, but not with
  another `writes` declaration of `T`, since commands apply in execution order; and it is a
  declared writer of a transient `T`. On an exclusive system it declares the `&mut World`
  writes the system makes at its stage's sync point, which that stage's systems see; on
  any other system the writes land at the next sync point and are visible from the next
  stage. This is how a transient column written at the input sync point, or a transient
  reference column (which `Mut` cannot write), gets a declared writer. A declaration is
  trusted and never checked: an undeclared write is not seen by the schedule, and a
  declared write the system never performs is accepted.

What it emits, and what the function keeps:

- The function, unchanged except that every parameter binding is made mutable, so
  `vel.y -= ..` through `Mut` and `cmd.spawn(..)` through `Commands` compile as written.
- `pub static F: SystemDescriptor` named after the function in upper snake case, holding the
  name, the worlds, the set, the `after` edges, the kernel's access table (the `ACCESS` of
  every parameter in declaration order, then the `writes` entries), the finalize
  function's access table, the
  exclusive flag, the reducer's type name (`stringify!`, since `type_name` is not const)
  and the hidden entry points, through the `unsafe` constructor `SystemDescriptor::new`.
  The expansion is the one sound caller: the tables it writes are derived from the parameter
  types and the `writes` list, which is what makes the schedule's ordering sound
  (`SAFETY.md`). The `unsafe` block
  is spanned at the call site, never at a user token, so it lands in the invoking crate
  under the external-macro exemption and that crate may keep `#![forbid(unsafe_code)]`
  (architecture section 21, as for `#[component]`).
- For `#[finalize(f)]` on `fn g(out: R::Output, params..)`: the hidden entry point that `F`
  names by convention, `const __finalize_f: Finalize<R::Output>` with the function's
  visibility, so the two attributes meet without seeing each other. A missing finalize for
  a reduction is the compile error "cannot find value `__finalize_f`"; a first parameter
  that is not `R::Output` is "mismatched types" at the attribute. The `#[finalize(f)]`
  expansion also asserts, in a constant, that `F` exists and is a reduction, so a finalize
  that names a missing system or one without `reduce` is a compile error rather than a
  function nothing runs.
- `tests/compile_fail.rs` checks every error the macro reports and that a crate with
  `#![forbid(unsafe_code)]` and `#![deny(warnings)]` compiles every kind of system.

What a system is, from its parameters (`SystemParam` has the table):

- **Per-entity**: it has a component parameter. It runs once per entity that has every
  required component, lacks every `Without` component, passes every `Changed` filter,
  resides in a `Simulated` region and passes the world's own filter (`ClaimedBy` in the
  prediction world; none elsewhere). It runs as chunk jobs on the executor; nothing in its
  parameters names a row, chunk or worker.
- **Per-region**: no component parameter, but a `Ctx` or `RegionRes`. It runs once per
  `Simulated` region as independent jobs; its commands spawn into that region and its RNG is
  keyed by that region's seed.
- **Once per tick**: neither. One job, with `Job` for scratch, fork-join and
  `block_in_place`; this is the physics step. It spawns with `Commands::spawn_in`.
- **Exclusive**: declared `exclusive`. It runs alone at the sync point that begins its stage,
  after the previous stage's commands have applied, as one job with `&mut World` and
  optionally `Job`. Its resource parameters are taken out of the world for the call, so
  `world.resource::<T>()` is `None` for them inside. Two exclusive systems of a stage are
  ordered by sets or edges like any conflicting pair.
- A **reduction**'s kernel returns one `R::Item` per entity; the partials are merged with
  `Reducer::merge`, which is order-independent by contract, and the finalize function runs
  once with `R::Output`, on whichever worker finishes last, with the parameter rights of a
  once-per-tick system.
- What the schedule rejects, applied to the kernel's table and the finalize's table
  separately: `ResMut` or `Job` in a per-entity or per-region system, `StableKeyOf` outside
  a per-entity system, `Ctx` or `RegionRes` in an exclusive system or a finalize, two data
  parameters (`&T`, `Mut<T>`, `Option<&T>`, `Option<Mut<T>>`) on one component or
  `Without<T>` beside any parameter on `T` (`Changed<T>` and `With<T>` combine with one data
  parameter on `T`), `Mut` on a world-maintained or reference-bearing component, `Changed`
  on an untracked column; and (`ScheduleError::InvalidParameter`) a component parameter or
  `&mut World` in a finalize, `&mut World` or `ChannelTake<C>` outside an exclusive system,
  `Commands`, `Job`, `&mut World` or `ChannelTake<C>` of one channel twice,
  `ChannelTake<C>` beside `Channel<C>` or `ChannelWriter<C>` on one channel, or `ResMut<T>`
  beside another parameter on `T`, each of which would hand one parameter what another
  holds. A per-region system whose region lacks a named `RegionRes` is skipped in that
  region.

## Behaviour the signatures cannot express

**Keys**

- Every reference is a `StableKey`; `EntityId` is storage-only and not in this API.
  `StableKeyOf` and every iterator yield the origin key: the `NetId`, or the `PredictionKey`
  of a predicted spawn, which stays the origin key after the server binds a `NetId`
  (`World::bind`) and survives a save and a region overlay through `PredictedOrigin`. Every
  method that takes a key accepts either, and storage holds origin keys: a bound `Net` key
  written into a reference field at apply, `insert` or `spawn` is rewritten to the origin
  key, and an export writes reference fields as stored, so the bytes and the hash over them
  are the same on every machine and after a load.
- `Commands::spawn` returns `StableKey::Placeholder(n)`, `n` the index among the
  invocation's spawns. It is valid in that buffer: in later commands, inside component
  values they carry (every `EntityRef` field is rewritten at apply through
  `visit_references`), in channel entries (rewritten through `CommandChannel::remap`), and
  as a key for `Ctx::rng`. It is not an event target or a presentation-event origin.
- At apply a spawn's origin key is `Predicted(player, input tick, ordinal)` when the source
  entity has `ClaimedBy`, the player has an input tick this tick (`World::set_input_tick`,
  cleared at every input sync point) and the system's worlds include both `server` and
  `prediction`; the ordinal is the low 32 bits of xxh64 over (source origin key, system key,
  spawn index within the invocation), so a spawn the other machine cannot reproduce (debris
  from a hit the client did not see) shifts nothing. A `Predicted` key that is already live
  is a collision and the spawn is not predicted. The spawned entity inherits `ClaimedBy` and
  carries `PredictedOrigin`; the server also allocates and binds a `NetId` at apply, so
  every server entity has one for the wire and the save. Otherwise, on the server, the key
  is a runtime `NetId` from the counter `WorldBuilder::next_net_id` restores and no claim
  passes on; in a client or prediction world the spawn is dropped and logged. The input
  tick is the slot the input is applied for, also when the server substitutes a previous
  input for a lost one (architecture section 9.2); re-simulation re-derives the same keys
  because the input-tick table is set every tick. Whether the source has `ClaimedBy` is
  read when its kernel runs, not at apply, so an earlier command of the same apply cannot
  change the key. The hashed bytes are the source's canonical key (one tag byte, 1 `Net`,
  2 `Predicted`, 3 `Placeholder`, then its words little-endian: the `NetId`'s 64 bits, or
  player, input tick and ordinal as 32 bits each), the system key as 64 bits and the spawn
  index as 32 bits, little-endian, seed zero; both machines must hash these exact bytes.
- `World::spawn` takes the key: replication and prediction spawn with the server's key.
  The direct methods (`spawn`, `insert` and their erased forms) rewrite a bound `Net` key in a reference field to
  the origin key but keep a reference to an entity that does not exist yet, as a load
  does, since replicated entities arrive in any order; only command apply empties it.
- `Ctx::rng(k)` is keyed by the region seed, the tick, the system, the invoking entity's
  origin key and `k`, so two entities rolling for one target draw different streams, and a
  placeholder is keyed by its spawn index.

**Commands and events**

- Command buffers apply at the sync point ending the stage, the last stage's at the closing
  sync point of the run: in the stage's execution order, then by (source, sequence) within
  a system, sources ordered as the once-per-tick invocation, per-region invocations by
  `RegionId`, then entity sources by origin key. Events order the same way. Commands
  against a missing entity, moves to an unloaded region and region-less spawns are dropped
  and logged at target `casein`, as are a spawn that names `InRegion` or `PredictedOrigin`
  and a write that fails validation; a reference to a missing entity is written empty. A
  `set` of `OwnedBy`, or a spawn whose bundle's `OwnedBy`, would make the entity its own
  owner or close a cycle is dropped and logged the same way (Regions, "Ownership").
  `despawn` and `move_to_region` take the entities the target owns with it, and a later
  command in the same apply may move an owned entity again. `Commands::set` and
  `Commands::remove` of `InRegion` or `PredictedOrigin` panic when recorded, as the direct
  methods do.
- Relationship fields (`EntityRef` fields, `OwnedBy`) and `ClaimedBy` are written through
  `Commands::set`, `Commands::remove` or the direct methods, never through `Mut`, so the
  reverse index (`World::referrers`, `World::owned_by`) stays consistent; the schedule
  rejects `Mut` on them. At apply a reference must name a live entity; afterwards it may
  dangle across a region unload, and `World::get` returns `None`.
- Events emitted through `EventWriter<E>` are visible to systems ordered after the emitter
  in the same tick, ordered by (emitting system, source origin key, sequence), and cleared at
  the input sync point. The schedule requires every reader of `E` to be ordered after every
  emitter of `E` (`ScheduleError::EventsBeforeEmitter`), so a reader's view never depends on
  scheduling. Events are not in a snapshot: a snapshot point is between ticks.
- A channel is written through a `ChannelWriter<C>` parameter, in any kind of system: its
  entries wait in a sink of the invocation and reach the channel at the sync point that
  ends the stage, after the invocation's commands, keys resolved. Entries written in stage
  *k* are readable through `Channel<C>` in stages after *k* and taken through
  `ChannelTake<C>` in an exclusive system of a later stage, whose fetch removes the store
  before the system runs. The schedule refuses a writer of `C` in the stage of a taker of
  `C` or a later one, since its entries would land after the take, and a `Channel<C>`
  reader or a second taker placed after the taker (a later stage, or the taker's stage
  unless exclusive and ordered before it), since it would find the channel empty
  (`ScheduleError::ChannelAfterTaker`); and a writer of `C` that no later stage takes or
  reads (`ScheduleError::ChannelWithoutConsumer`). `lactose-physics-api`'s channel is taken
  at the physics sync point, so a writer of it is in `Stage::Input` or `Stage::Simulation`
  and a reader of it before the sync. The closing sync point of a run clears every channel
  after it applies the last stage's commands: a taker's channel is already empty, and a
  channel with readers alone holds what its readers saw. Channels are therefore empty
  between runs, where a snapshot point is, which the input sync point asserts, and are
  not in a snapshot (D126).
- Presentation events are keyed by (origin key, tick, kind, sequence). The world keeps the
  keys it has delivered, across `restore`, pruned below the oldest tick a restore can reach:
  a re-emission with a delivered key is not delivered again, and a delivered key whose tick
  is re-simulated without re-emitting it is reported as cancelled; one cancelled and
  emitted again before presentation takes either is neither. A `run_pass` numbers its
  emissions from 2^31 within the tick, so a re-simulated tick neither shifts nor cancels
  them. Emissions wait until `take_present_events`, so a world that never takes them, a
  dedicated server, grows the queue; the server's tick loop takes and drops them.
- Every reader of a transient column must be ordered after a declared writer of it in the
  tick (`TransientBeforeWriter`): a system with `Mut` on the column; an exclusive system
  that names it in `writes = [..]`, in the reader's stage or an earlier one; or any other
  system that names it in `writes = [..]`, in an earlier stage, since its commands apply
  at the sync point after the reader's stage started. An exclusive system that writes the
  column at the input sync point declares it; one that declares nothing is no writer of
  anything.

**Schedule**

- A stage is a sync point followed by a DAG. At the sync point the previous stage's
  commands apply, then the stage's exclusive systems run in their order, then its other
  systems run as the DAG allows; a closing sync point after the last stage applies its
  commands and clears every channel, so a run ends with every buffer and channel empty and
  a snapshot point never has a pending buffer. In a tick the first stage's sync point is
  the input sync point: transient columns are zeroed and events and input ticks cleared
  before its exclusive systems (every channel is already empty, which a debug assertion
  checks), which is where `sim`, `net` and `asset` apply inputs, staged state and version tables.
  An exclusive system runs as the only live node of its run. An exclusive system ordered
  after a non-exclusive system of its own stage cannot run there and is a
  `ScheduleError::Cycle`.
- Within a stage, two systems that touch one target with at least one write must be
  ordered by sets or `after` edges; otherwise the build fails. Non-conflicting systems run
  in parallel. The execution order is the topological order with ties by `SystemKey`.
  `ScheduleBuilder::stage` takes the stage's sets as any slice, so `&[&MOVEMENT]` works in
  a function; the sets themselves are statics, and a set belongs to one stage
  (`ScheduleError::SetInTwoStages`). Two unordered exclusive systems conflict on
  `AccessTarget::World`. `ScheduleBuilder::order(before, after)` adds an edge between two
  sets from outside them, so the assembler can order the sets of two crates that do not
  name each other; both sets must be declared in some stage
  (`ScheduleError::UnknownOrderedSet`), and an edge across stages is accepted when it
  agrees with the stage order and is a `ScheduleError::Cycle` otherwise.
- `Changed<T>` in system S passes rows whose change stamp is after S's position in the
  previous tick and before it in this one: each change is seen exactly once by each reader,
  whether it came from a system ordered before S, a command at a sync point or direct
  access between ticks; S's own writes are outside its own window. The stamp is (tick,
  position) on tracked columns only, so `Changed` on an untracked column is rejected and
  `changed_since` on one panics. A restore puts stamps back verbatim and a correction
  written through `ColumnMut::write` without recording leaves them alone, so a
  re-simulated tick sees the client's own change history, which is not the server's.
  `lactose-predict` writes its corrections stamped (D122), so a re-simulated tick's `Changed`
  readers see a correction once; the unrecorded write stays for a consumer that wants it. In a
  `run_pass`, which has no previous tick to measure from, `Changed<T>` passes what changed
  since the same schedule's previous pass of the epoch; in `World::query`, what changed
  after the current point of the previous tick (everything at tick zero).
- A run is one graph node on the executor, named `casein::tick` or `casein::pass`, which
  runs every sync point and every system in execution order on one worker; the world moves
  into the node and back out when the run resolves. If the executor has shut down first the
  run resolves to `Cancelled` before anything ran, and the world drops with the node. The
  layout options are honoured: chunk jobs are issued in `chunk_order`, rows are permuted
  within their chunks at every sync point, and a reduction's partials are split into
  batches of random size and merged.
- `run_tick` takes the world by value and hands it back when the tick resolves; the future
  is `Send` and is driven by the timer root task, which owns the world and the schedule. On
  an error the world is not handed back (`Schedule::prepare` checks what can fail before
  anything runs, by reference); a future dropped mid-tick drops the world when the jobs
  finish, a forgotten one leaks it. `run_pass` runs the stages between ticks without
  advancing the tick or zeroing or clearing anything at its first sync point (its closing
  sync point clears every channel, as every run's does), its writes stamped after every
  position of the tick: extract, the replication gather and the save copy are schedules of
  exclusive systems that take `Job` and fork over `World::columns`. Children of such a fork key what
  they produce by `StableKey`; the worker index is banned by the boundary lint file.

**Regions**

- Per-entity and per-region systems run only over `Simulated` regions. `Replicated` and
  `Frozen` rows are reachable through `&mut World` at a sync point. A region's mode, seed,
  resources and exact-version asset table are the world's state; its entity set is a
  filtered scan. Commands and direct writes still apply to `Replicated` and `Frozen` rows.
- `unload_region` despawns the region's rows and logs them as despawned, but an unload is
  not a death: the aliases of its predicted entities stay, so what they own elsewhere still
  exports and a reload finds them. `allocate_net_id` passes over a runtime `NetId` that a
  live entity or a bind already holds, such as one spawned directly with its key.
- `InRegion` and `PredictedOrigin` are the world's: read-only through every path.
  `ClaimedBy` is written by `net` and `predict` on join and removed to release the claim.
  `OwnedBy` is a relationship (the two bullets below). `ClaimedBy` and `InRegion` are not
  saved; `OwnedBy` and `PredictedOrigin` are.
- Ownership is a forest. An entity has at most one owner (`OwnedBy`), is never its own
  owner, and no chain of owners returns to its start. The chain of `k` is `k`, its owner,
  its owner's owner, …, over live entities, ending at the first that has no owner or whose
  owner is not live; keys are compared as origin keys, so a bound `Net` key is its
  predicted entity. A write of `OwnedBy { owner }` to `k` is refused if `k` is on the chain
  of `owner` (`owner == k` included): `InvalidData` from `insert`, `insert_erased`, `spawn`
  and `spawn_erased`, with nothing changed, not even the point; at command apply a `set`,
  or a spawn whose bundle carries such an `OwnedBy`, is dropped and logged at target
  `casein` like a write that fails validation, and the rest of the buffer applies.
  `load_region` walks the document's rows (under the keys the load will give them) and the
  live world together before it binds or spawns anything, and refuses whole a document
  whose owners would close a cycle with each other, with the baseline's own
  `ecs::owned_by` column or with live entities of other regions, as it refuses any invalid
  row; the binds a load makes are covered by that walk. `bind` is refused if the entity at
  the end of the predicted entity's chain is owned by the `Net` key being bound, since the
  rewrite would close the cycle. The cost is one lookup per link of the chain, paid only
  by `OwnedBy` writes, the ownership edges of a load and binds. `despawn` and
  `move_to_region` visit each entity of the tree once whatever the graph. A `restore` puts
  back the snapshot's graph, a forest when taken; the one state this rule does not guard
  is a revived entity whose owner is a `Net` key bound since the snapshot to an entity
  whose chain reaches it, which needs a mispredicted `OwnedBy` on a predicted spawn
  (`OwnedBy` is not replicated and is never corrected): a debug assertion names it, and it
  is a logic error, not a hang.
- Writing `OwnedBy` moves nothing. An owned entity resides where it was spawned or last
  moved and follows its owner from the next `move_to_region` or `despawn` on (architecture
  section 6, D12), so an entity given to an owner in another region stays where it is
  until that owner moves, and an owner's region may unload while what it owns stays
  loaded. `OwnedBy` is then a weak cross-region reference like any `EntityRef` field:
  `World::get` on the owner is `None`, the export writes the owner's `NetId` and a reload
  in either region order restores it, and `owned_by` finds the entity again once the owner
  is live. The world therefore accepts no ownership state `lactose_bundle::Overlay::new`
  refuses (an entity its own owner, a cycle among one overlay's records) and refuses no
  state the format accepts (an owner the overlay destroys, an owner elsewhere or absent),
  so a region's export is always an overlay: `lactose-persist`'s diff never fails on an
  export, which is the invariant every save, unload and relevance message rests on.
- `World::claimed(player)` yields the entities with `ClaimedBy { player }` in ascending
  origin-key order, from an index the world maintains wherever `ClaimedBy` enters or
  leaves a row (a spawn, `insert`, `remove`, a despawn, a command apply, a predicted
  spawn's inherited claim, a region load or unload) and that a snapshot carries and a
  restore puts back. It allocates nothing and costs the player's claims, never a scan of
  the world: it is how an input or session applier finds the player's entity
  (`lactose-input`'s `player_entity`), since `World::query` today visits every live
  entity (`docs/verification.md`).

**Change tracking**

- The current tick is `World::tick`, absolute and never rebased; consumers keep a
  `ChangeMark` or a `LogCursor` and pass it back, or test it with `World::is_current`. A
  mark is a point in the tick's stamp order, between ticks after every position, so a
  direct write after the mark is newer than it. A restore, a whole-world load
  (`WorldBuilder::tick` plus region loads, then `set_tick`) or the 2^30 stamp rebase starts
  a new epoch, after which a stale mark or cursor yields `None` and the consumer does a
  full pass. A region load does not: its rows are stamped at the current point and logged
  as spawned.
- The tick is a `u32` that wraps. Every 2^30 ticks (tick zero included) stamps older than
  2^30 ticks are clamped forward to that age and an epoch starts, so every stored stamp is
  less than 2^31 ticks older than now, and stamps are compared relative to the current
  tick; the `Changed` windows, `changed_since` and a pass's first window in an epoch (which
  opens 2^31 ticks back) hold across the wrap.
- `set_tick` moves stamps onto the new clock by their age on the old one. Forward, a stamp
  of the old tick, possibly not yet seen by a reader, moves to the new point and anything
  older to before the new tick; back by `k` ticks, a stamp younger than `k` ticks moves to
  the new point and the rest keeps its tick. A mark taken after `set_tick` sees only later
  writes, and each `Changed` reader sees a moved stamp once more at most.
- A `Schedule` serves one world: the first `prepare` or run ties it to that world, and
  another world is `ScheduleError::WrongWorld`, since a pass keeps each system's `Changed`
  window in the schedule.
- Every column in every chunk has a coarse tick, set at dispatch for every chunk a writing
  system touches. Tracked columns (replicated or extracted) have a stamp per row, set by
  `Mut` on mutable dereference, by `Commands::set`, `World::insert`, `World::insert_erased` and `World::get_mut`,
  and by `ColumnMut::write` for the rows it records as dirty. `Mut::set_if_neq` compares
  bytes, which is exact for a reflected component, and marks nothing when equal. A move of
  a row between chunks (compaction, an archetype change, a permutation) carries its stamp
  and raises the destination chunk's column tick to the source's, so a consumer that skips
  chunks by column tick misses nothing. `ColumnRef::column_tick` and `row_ticks` are ticks,
  not points: a write made after a mark in the mark's own tick shows the mark's tick, so a
  consumer that needs the exact point uses `changed_since`: `World::changed_since` for a
  typed column, or `ErasedColumn::changed_since` on each column of `columns_erased` for a
  pass that works from the registry (the replication gather), which select the same rows
  under every layout, by stamp and never by tick.
- At the input sync point a transient column's rows are zeroed, and a tracked row whose
  bytes were not already zero is stamped there; every region's transient resources are
  zeroed too.
- `World::columns` yields one slice per chunk with the rows' keys and regions, for passes
  that fork over chunks (extract, replication gather, the world hash). Results keyed by
  anything but `StableKey` are wrong, and SOUR's layout mode catches them. `Query::for_each`
  and `referrers` are in ascending key order, so exclusive code that spawns or allocates
  per match is layout-independent.

**Snapshots and bulk copy**

- `WorldSnapshot` holds every column with its stamps, the entity table, every reflected
  resource of the world and its regions, the set of loaded regions with their modes, seeds
  and asset tables, the input-tick table and the relationship index. `restore` returns to
  the snapshot's tick, puts stamps back verbatim, clears the structural log and starts a
  new epoch; the alias table and the delivered presentation-event keys are kept, untaken
  emissions after the snapshot are dropped, and a bind for an entity the restore removed
  applies when the entity is re-spawned; one that is never re-spawned is recorded as
  despawned at the snapshot's tick, so the horizon prunes its alias. A reference the
  snapshot holds under a `Net` key bound since, in a row or a region resource, goes back to
  the origin key at the restore, as the bind left it, in storage and in the reverse index;
  unless the restore revives a `Net` entity of that key (it was live at the snapshot and
  despawned before the bind), whose references stay on it. That revived entity is stale
  server state, and `lactose-predict` despawns it again before it re-simulates; despawning a
  bound `Net` entity moves the references to it to the origin key.
  `World::set_rollback_horizon` is how the world learns what it may prune: the ring's
  oldest tick from `lactose-predict`, the current tick on the server. `World::bind` refuses
  a `NetId` that names a live entity of its own, whose references the bind would redirect.
- `load_region` takes the baseline and the overlay together into a region the caller
  created with the saved seed, skips baseline rows the overlay destroys (a cooked entity
  that moved is a live row elsewhere), restores predicted origins and aliases from
  `PredictedOrigin`, keeps reference fields as exported, and requires every component
  schema to match the registry's (`VersionMismatch` otherwise); migration is
  `lactose-persist`'s, and region resources travel beside the overlay and go in through
  `insert_region_resource_erased`. `export_region` yields persistent components in
  ascending `NetId` order, the owner beside each row, the seed and the persistent region
  resources; diffing against the baseline into an `Overlay` is `lactose-persist`'s, and the
  world hash is `lactose-sim`'s, over `columns_erased` and `region_resources_erased`.
- A transient component is not saved, but its presence is part of the entity's archetype,
  which its systems and the world hash depend on: `ExportedRow::transient` lists the keys
  of the row's transient components, `lactose-persist` encodes their presence as the
  component with zeroed bytes in the entity's records (D129), diffs that presence against
  the baseline and ignores the transient values, and `load_region` stores a transient
  component a document carries as zeroed, whatever its bytes. Transient region resources
  likewise: `RegionRows::transient_resources` lists them, and the loader puts each back with
  `insert_region_resource_zeroed`. Transient values therefore differ from a twin that never
  reloaded at the reload point, until the next input sync point zeroes both.
- Region resources are validated when inserted, typed or erased, and when updated through
  `update_region_resource`, which runs a closure on a copy and stores the result only if it
  is valid (a placeholder in a reference field is refused with `InvalidData` and the old
  value kept). A bound `Net` key in their reference fields is stored as the origin key, and
  `bind` and `restore` rewrite their references as they do rows', so the stored bytes do not
  depend on the order of an insert and a bind. References in resources are not in the
  reverse index and may dangle.

**Erased row access**

- `get_erased`, `insert_erased`, `remove_erased` and `spawn_erased` are the typed `get`,
  `insert`, `remove` and `spawn` with the component named by its `ComponentKey`, and share
  their validation and index maintenance: typed and erased writes of the same values
  leave the same columns, row stamps, column ticks, structural log, reverse index and claim
  index, and a refused write of either changes nothing, not even the point. They differ in
  two ways. `insert` stamps a write of equal bytes and `insert_erased` does not. A
  placeholder in a reference field is stored by `insert` and `spawn` as the empty
  reference, as a direct write resolves any reference outside a command buffer, and
  refused by the erased writes with `InvalidData` ("placeholder reference in `<field>`"),
  since erased bytes come from the wire or a recorded slot, where a placeholder is never
  valid. Each takes either key of a bound entity. `get_erased` returns the bytes as stored
  (reference fields hold origin keys), `None` if the entity or the component is absent.
- `ComponentData::bytes` must be the registry's element size for its component
  (`InvalidData` otherwise). The bytes are copied, padding zeroed (`TypeInfo::zero_padding`),
  validated through `TypeInfo::validate` (`InvalidData`) and a bound `Net` key in a
  reference field rewritten to the origin key, as `insert` does. An unregistered component
  is `NotFound`, a registered resource and `InRegion` or `PredictedOrigin` are
  `InvalidData`, and a missing entity is `NotFound`.
- `insert_erased` adds the component if the row lacks it (an archetype change the
  structural log records, the row stamped) and otherwise writes it only if the bytes
  differ, so an equal write stamps nothing and a `Changed` reader does not see it. It
  returns whether it wrote. `remove_erased` returns `Ok(false)` if the row lacks the
  component.
- `spawn_erased` validates every component before anything changes (two entries for one
  component are `InvalidData`) under the key rules of `spawn`: a `Net` or `Predicted` key no
  live entity has, `PredictedOrigin` added for a `Predicted` key, a placeholder refused.

**Not in this crate yet**

- The overload ladder's decimation of non-player systems in the outer activation ring
  (architecture section 9.7) and `ordered` or per-region reductions (section 5.2) are
  additive attribute flags, added when milestone 1 or 4 needs them (`docs/verification.md`).

## Proof for milestone 0

The tests in `tests/`, each a nextest test that asserts on counts and orders and never on
timing; under Miri a representative quarter of the layouts runs. Milestone 0 requires
`cargo +nightly miri test -p lactose-ecs` to pass; milestone 1 requires SOUR in workers,
repeat, layout and chaos modes.

| File | What it proves |
|---|---|
| `structure.rs` | Spawn, get, insert, remove, despawn, region moves and ownership migration, directly and through commands, each in the structural log with its tick; dropped commands; emptied references |
| `systems.rs` | The section 6.1 systems, `#[finalize]` included, for several ticks in a server and a prediction world; the prediction filter |
| `commands.rs` | Apply order (execution order, source key, sequence), the same under every layout |
| `predicted.rs` | Predicted keys equal on both machines, unshifted by a server-only spawn; collisions; binds and aliases; `PredictedOrigin` and a reference to the entity through export and load, byte for byte |
| `events.rs` | Event order and clearing, `EventsBeforeEmitter`, channels across stages; a kernel whose only output is a channel entry; two writers in parameter order; a take that yields the earlier stages' entries once; the closing sync point of every run clearing every channel, seen by a pass, the next tick and a restore; a keyed carrier set after the step reaching the next tick's physics stage in a server and a prediction world |
| `schedule_errors.rs` | Every `ScheduleError` variant, the channel placements (`ChannelAfterTaker`, `ChannelWithoutConsumer`) and the `ChannelTake` refusals among them; `run_tick` failing on `prepare` and on shutdown |
| `channel_skip.rs` | A taker skipped for a resource removed mid-run is logged at `error`, naming the channel it did not take |
| `change.rs` | `Mut` and `set_if_neq` stamps, fresh and stale marks, `Changed<T>` seen once per reader, `ColumnMut::write` with and without dirty rows, column ticks, the 2^30 rebase |
| `erased.rs` | `ErasedColumn::changed_since` selects the rows `World::changed_since` selects, under every layout, for writes inside a tick and between ticks, with a fresh and a stale mark |
| `snapshot.rs` | Ten ticks, a snapshot at three, a restore and a re-run equal in every column, row tick and `Changed` set; presentation events deduplicated and cancelled; stale cursors; binds kept; pruning |
| `bulk.rs` | A baseline and an overlay with every record kind, diffed back to the same overlay under every layout; region resources; schema mismatches |
| `layout.rs` | One program under every layout: identical exports, structural logs, reductions and presentation events |
| `pass.rs`, `query.rs`, `regions.rs` | `run_pass` with a fork over columns; whole-world queries in key order; region modes and per-region systems |
| `binds.rs`, `wrap.rs`, `reload.rs` | Binds across a restore and onto live keys; `Changed` across the `u32` wrap, the 2^30 clamp and `set_tick`; transient presence through an unload and reload |
| `regressions.rs` | The findings of the pre-review: transient references in the index, pass events across a rollback, owner keys in either load order, exports after an unload, `set_tick` stamps, transient writers, double rollbacks, failed loads, resources removed mid-run |
| `regressions_two.rs` | The findings of the second review: new chunks' column ticks above 2^31, presentation batches across the wrap, `writes` visibility by stage, a restore that revives a bound `Net` entity, `update_region_resource`, transient region resources through a reload, schedule ties, resource references through bind and restore, aliases a restore orphans |
| `wrap_program.rs` | One program with marks, cursors, `Changed` readers, a pass, column and row ticks, presentation events and a rollback, run across the 2^30 rebase, 2^31 and the `u32` wrap, equal to a run far from any boundary |
| `erased_rows.rs` | Typed and erased writes of the same values leave identical columns, row and column ticks, `changed_since` sets, structural logs, reverse and claim indices under every layout, `Net` references rewritten and `OwnedBy` and `ClaimedBy` among them; an equal `insert_erased` unseen by a `Changed` reader; every refusal with its kind and nothing changed, a refused typed `insert` included; a placeholder refused by name, and stored empty by a typed write; padding zeroed; `get_erased` through a restore and a snapshot round trip |
| `ownership.rs` | The forest rule: a self-owner and the closing write of a two- and a three-entity cycle refused through every direct path with nothing changed, dropped and logged at apply with the rest of the buffer applied, refused whole by `load_region` within one document (through the baseline's own column) and across two documents in either order, and refused at `bind`; a chain through a dangling owner accepted and the write that closes it later refused; an owned entity staying put when given to an owner elsewhere and when the owner's region unloads, exported and reloaded in either order |
| `claims.rs` | The claim index behind `World::claimed`: current after every direct method, command, predicted spawn, ownership despawn, unload and restore, in key order under every layout |
| `compile_fail.rs` | What `#[system]` and `#[finalize]` reject, and what they accept under `#![forbid(unsafe_code)]` |
