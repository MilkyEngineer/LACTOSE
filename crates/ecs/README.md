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
| `Commands`, `Bundle`, `CommandChannel`, `ChannelWriter`, `Channel` | Structural commands applied at the next sync point; typed channels for other crates' commands |
| `Event`, `EventWriter`, `Events` | Cross-entity effects within a tick, indexed by target |
| `PresentEvent`, `PresentEvents`, `PresentEventBatch` | Requests to presentation, keyed by (origin, tick, kind, sequence) for rollback |
| `Schedule`, `ScheduleBuilder`, `ScheduleError` | Stages separated by sync points; a deterministic order; `run_tick` and `run_pass` |
| `RegionId`, `RegionMode`, `RegionConfig`, `InRegion`, `OwnedBy`, `ClaimedBy`, `PredictedOrigin` | Regions and the components the world maintains |
| `Rng` | The counter-based generator, from `Ctx::rng` and `World::rng` |
| `ChangeMark`, `World::changed_since`, `World::is_current`, `World::columns`, `ColumnMut`, `DirtyRows`, `StructuralLog`, `LogCursor` | Change state read per consumer; whole-column access that records what it changed |
| `Query`, `QueryItems` | Whole-world iteration for exclusive code and tests, in key order |
| `WorldSnapshot` | The prediction world's rollback state |
| `World::load_region`, `World::export_region`, `RegionRows` | Bulk copy of region rows and resources to and from `lactose_bundle` documents |

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

What it emits, and what the function keeps:

- The function, unchanged except that every parameter binding is made mutable, so
  `vel.y -= ..` through `Mut` and `cmd.spawn(..)` through `Commands` compile as written.
- `pub static F: SystemDescriptor` named after the function in upper snake case, holding the
  name, the worlds, the set, the `after` edges, the kernel's access table (the `ACCESS` of
  every parameter in declaration order), the finalize function's access table, the
  exclusive flag, the reducer's type name (`stringify!`, since `type_name` is not const)
  and the hidden entry points, through the `unsafe` constructor `SystemDescriptor::new`.
  The expansion is the one sound caller: the tables it writes are derived from the parameter
  types, which is what makes the schedule's ordering sound (`SAFETY.md`). The `unsafe` block
  is spanned at the call site, never at a user token, so it lands in the invoking crate
  under the external-macro exemption and that crate may keep `#![forbid(unsafe_code)]`
  (architecture section 21, as for `#[component]`).
- For `#[finalize(f)]` on `fn g(out: R::Output, params..)`: the hidden entry point that `F`
  names by convention, `const __finalize_f: Finalize<R::Output>` with the function's
  visibility, so the two attributes meet without seeing each other. A missing finalize for
  a reduction is the compile error "cannot find value `__finalize_f`"; a first parameter
  that is not `R::Output` is "mismatched types" at the attribute.
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
  `&mut World` in a finalize, `&mut World` outside an exclusive system, `Commands`, `Job` or
  `&mut World` twice, or `ResMut<T>` beside another parameter on `T`, each of which would
  hand one parameter what another holds. A per-region system whose region lacks a named
  `RegionRes` is skipped in that region.

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
  The direct methods (`spawn`, `insert`) rewrite a bound `Net` key in a reference field to
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
  and a write that fails validation; a reference to a missing entity is written empty.
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
- Channel entries written in stage *k* are readable through `Channel<C>` in stages after
  *k* and through `World::take_channel` at a sync point, keys resolved; the input sync point
  clears them.
- Presentation events are keyed by (origin key, tick, kind, sequence). The world keeps the
  keys it has delivered, across `restore`, pruned below the oldest tick a restore can reach:
  a re-emission with a delivered key is not delivered again, and a delivered key whose tick
  is re-simulated without re-emitting it is reported as cancelled.

**Schedule**

- A stage is a sync point followed by a DAG. At the sync point the previous stage's
  commands apply, then the stage's exclusive systems run in their order, then its other
  systems run as the DAG allows; a closing sync point after the last stage applies its
  commands, so a run ends with every buffer empty and a snapshot point never has a pending
  buffer. In a tick the first stage's sync point is the input sync point: transient columns
  are zeroed and events, channels and input ticks cleared before its exclusive systems,
  which is where `sim`, `net` and `asset` apply inputs, staged state and version tables.
  An exclusive system runs as the only live node of its run. An exclusive system ordered
  after a non-exclusive system of its own stage cannot run there and is a
  `ScheduleError::Cycle`.
- Within a stage, two systems that touch one target with at least one write must be
  ordered by sets or `after` edges; otherwise the build fails. Non-conflicting systems run
  in parallel. The execution order is the topological order with ties by `SystemKey`.
  `ScheduleBuilder::stage` takes the stage's sets as any slice, so `&[&MOVEMENT]` works in
  a function; the sets themselves are statics.
- `Changed<T>` in system S passes rows whose change stamp is after S's position in the
  previous tick and before it in this one: each change is seen exactly once by each reader,
  whether it came from a system ordered before S, a command at a sync point or direct
  access between ticks; S's own writes are outside its own window. The stamp is (tick,
  position) on tracked columns only, so `Changed` on an untracked column is rejected and
  `changed_since` on one panics. A restore puts stamps back verbatim and a correction
  written through `ColumnMut::write` without recording leaves them alone, so a
  re-simulated tick sees the client's own change history, which is not the server's. In a
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
  advancing the tick or clearing anything, its writes stamped after every position of the
  tick: extract, the replication gather and the save copy are schedules of exclusive
  systems that take `Job` and fork over `World::columns`. Children of such a fork key what
  they produce by `StableKey`; the worker index is banned by the boundary lint file.

**Regions**

- Per-entity and per-region systems run only over `Simulated` regions. `Replicated` and
  `Frozen` rows are reachable through `&mut World` at a sync point. A region's mode, seed,
  resources and exact-version asset table are the world's state; its entity set is a
  filtered scan.
- `InRegion` and `PredictedOrigin` are the world's: read-only through every path.
  `ClaimedBy` is written by `net` and `predict` on join and removed to release the claim.
  `OwnedBy` is a relationship. `ClaimedBy` and `InRegion` are not saved; `OwnedBy` and
  `PredictedOrigin` are.

**Change tracking**

- The current tick is `World::tick`, absolute and never rebased; consumers keep a
  `ChangeMark` or a `LogCursor` and pass it back, or test it with `World::is_current`. A
  mark is a point in the tick's stamp order, between ticks after every position, so a
  direct write after the mark is newer than it. A restore, a whole-world load
  (`WorldBuilder::tick` plus region loads, then `set_tick`) or the 2^30 stamp rebase starts
  a new epoch, after which a stale mark or cursor yields `None` and the consumer does a
  full pass. A region load does not: its rows are stamped at the current point and logged
  as spawned.
- Every column in every chunk has a coarse tick, set at dispatch for every chunk a writing
  system touches. Tracked columns (replicated or extracted) have a stamp per row, set by
  `Mut` on mutable dereference, by `Commands::set`, `World::insert` and `World::get_mut`,
  and by `ColumnMut::write` for the rows it records as dirty. `Mut::set_if_neq` compares
  bytes, which is exact for a reflected component, and marks nothing when equal. A move of
  a row between chunks (compaction, an archetype change, a permutation) carries its stamp
  and raises the destination chunk's column tick to the source's, so a consumer that skips
  chunks by column tick misses nothing. `ColumnRef::column_tick` and `row_ticks` are ticks,
  not points: a write made after a mark in the mark's own tick shows the mark's tick, so a
  consumer that needs the exact point uses `changed_since`.
- At the input sync point a transient column's rows are zeroed, and a tracked row whose
  bytes were not already zero is stamped there.
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
  applies when the entity is re-spawned. `World::set_rollback_horizon` is how the world
  learns what it may prune: the ring's oldest tick from `lactose-predict`, the current tick
  on the server.
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
| `events.rs` | Event order and clearing, `EventsBeforeEmitter`, channels across stages |
| `schedule_errors.rs` | Every `ScheduleError` variant; `run_tick` failing on `prepare` and on shutdown |
| `change.rs` | `Mut` and `set_if_neq` stamps, fresh and stale marks, `Changed<T>` seen once per reader, `ColumnMut::write` with and without dirty rows, column ticks, the 2^30 rebase |
| `snapshot.rs` | Ten ticks, a snapshot at three, a restore and a re-run equal in every column, row tick and `Changed` set; presentation events deduplicated and cancelled; stale cursors; binds kept; pruning |
| `bulk.rs` | A baseline and an overlay with every record kind, diffed back to the same overlay under every layout; region resources; schema mismatches |
| `layout.rs` | One program under every layout: identical exports, structural logs, reductions and presentation events |
| `pass.rs`, `query.rs` | `run_pass` with a fork over columns; whole-world queries in key order |
| `compile_fail.rs` | What `#[system]` and `#[finalize]` reject, and what they accept under `#![forbid(unsafe_code)]` |
