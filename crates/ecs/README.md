# CASEIN (`lactose-ecs`)

Chunked archetype storage and the scheduler: the surface every simulation crate writes
against.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | yes, see `SAFETY.md` |

**Status: facade.** Every public signature is final and every body is `todo!()`. The naive
single-threaded implementation replaces the bodies, the `#[system]` and `#[finalize]`
expansions and the hidden `__private` items, without changing a public signature; it removes
the crate-level `allow(dead_code, unused_variables)` with the last `todo!()`. Architecture
sections 5.2, 6 and 6.1 are the design; where this file and the architecture disagree, the
architecture wins and this file is wrong. Decision D114 records the choices the sections left
open.

Amending a signature is allowed in the change that needs it. Say so in the change description
and list what changed, because `sim`, `predict`, `physics-api`, `net`, `persist`,
`present-api` and the game crates are written against this surface.

## Surface

| Item | Purpose |
|---|---|
| `World`, `WorldBuilder`, `WorldKind`, `WorldSet`, `LayoutConfig`, `ChunkOrder` | The world, built for one kind with a `lactose_reflect::Registry`; SOUR's layout options |
| `#[system]`, `#[finalize]`, `SystemDescriptor`, `SystemSet`, `Access` | A system is a function; the attribute emits a descriptor static with its access table |
| `Res`, `ResMut`, `RegionRes`, `&T`, `Mut`, `Option<&T>`, `Option<Mut<T>>`, `With`, `Without`, `Changed`, `Ctx`, `StableKeyOf`, `&mut World` | System parameters, each a `SystemParam` with a constant `ACCESS` |
| `Commands`, `Bundle`, `CommandChannel`, `ChannelWriter`, `Channel` | Structural commands applied at the next sync point; typed channels for other crates' commands |
| `Event`, `EventWriter`, `Events` | Cross-entity effects within a tick, indexed by target |
| `PresentEvent`, `PresentEvents`, `PresentEventBatch` | Requests to presentation, keyed by (origin, tick, kind) for rollback |
| `Schedule`, `ScheduleBuilder`, `ScheduleError` | Stages separated by sync points; a deterministic order; `run_tick` |
| `RegionId`, `RegionMode`, `RegionConfig`, `InRegion`, `OwnedBy`, `ClaimedBy` | Regions and the components the world maintains on every entity |
| `Rng` | The counter-based generator, from `Ctx::rng` |
| `ChangeMark`, `World::changed_since`, `World::columns`, `ColumnMut`, `DirtyRange`, `StructuralLog`, `LogCursor` | Change state read per consumer; whole-column access with an explicit dirty range |
| `Query`, `QueryItems` | Whole-world iteration for exclusive code and tests |
| `WorldSnapshot` | The prediction world's rollback state |
| `World::import_baseline`, `World::apply_overlay`, `World::export_region`, `RegionRows` | Bulk copy of region rows to and from `lactose_bundle` documents |

The hidden `__private` module and the hidden items of `SystemDescriptor`, `Bundle` and
`Kernel` are the macro's and may change with it.

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
- `worlds = [server, client, prediction]`, any subset; omitted means every world.
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
  name, the worlds, the set, the `after` edges, the access table (the `ACCESS` of every
  parameter in declaration order, then of the finalize function's parameters), the exclusive
  flag, the reducer's type name and the hidden entry points.
- For `#[finalize(f)]` on `fn g(out: R::Output, params..)`: the hidden entry point that `F`
  names by convention, `__finalize_f`, so the two attributes meet without seeing each other.
  A missing finalize for a reduction is the compile error "cannot find `__finalize_f`".

What a kernel is:

- A system with at least one component parameter runs once per entity that has every
  required component, lacks every `Without` component, passes every `Changed` filter,
  resides in a `Simulated` region and passes the world's own filter (`ClaimedBy` in the
  prediction world; none elsewhere). It runs as chunk jobs on the executor; nothing in its
  parameters names a row, chunk or worker.
- A system with no component parameter runs once per tick as one job. It may not take
  `Ctx` or `StableKeyOf`.
- An exclusive system runs alone at the sync point that begins its stage, after the previous
  stage's commands have applied. Its resource parameters are taken out of the world for the
  call, so `world.resource::<T>()` is `None` for them inside.
- A reduction's kernel returns one `R::Item` per entity; the partials are merged with
  `Reducer::merge`, which is order-independent by contract, and the finalize function runs
  once with `R::Output`, on whichever worker finishes last.

## Behaviour the signatures cannot express

**Keys**

- Every reference is a `StableKey`; `EntityId` is storage-only and not in this API.
  `StableKeyOf` and every iterator yield the origin key: the `NetId`, or the `PredictionKey`
  of a predicted spawn, which stays the origin key after the server binds a `NetId`
  (`World::bind`). Every method that takes a key accepts either.
- `Commands::spawn` returns `StableKey::Placeholder(n)`, `n` the sequence number within the
  invocation. It is valid in that buffer: in later commands, inside component values they
  carry (every `EntityRef` field is rewritten at apply through `visit_references`), and as a
  key for `Ctx::rng`, where it is keyed through the invoking entity's origin key and `n`.
- At apply a spawn's origin key is `Predicted(player, input tick, ordinal)` if the source
  entity has `ClaimedBy`: the player from the component, the input tick from
  `World::set_input_tick`, the ordinal the spawn's position among that player's spawns in
  this tick's command order. The spawned entity inherits `ClaimedBy`. Otherwise, on the
  server, it is a runtime `NetId` from the counter `WorldBuilder::next_net_id` restores.
  The server also allocates and binds a `NetId` for a predicted spawn at apply, so every
  server entity has one for the wire and the save.
- `World::spawn` takes the key: replication and prediction spawn with the server's key.

**Commands and events**

- Command buffers apply at the sync point ending the stage: in the stage's execution order,
  then by (source origin key, sequence) within a system; commands from a system without
  entity parameters sort first. A command against an entity that no longer exists is
  dropped. `despawn` and `move_to_region` take the entities the target owns with it.
- Relationship fields (`EntityRef`, `OwnedBy`) are written through `Commands::set` or the
  direct methods, never through `Mut`, so the reverse index (`World::referrers`,
  `World::owned_by`) stays consistent. At apply a reference must name a live entity;
  afterwards it may dangle across a region unload, and `World::get` returns `None`.
- Events emitted through `EventWriter<E>` are visible to systems ordered after the emitter
  in the same tick, ordered by (emitting system, source origin key, sequence), and cleared at
  the input sync point. The schedule requires every reader of `E` to be ordered after every
  emitter of `E` (`ScheduleError::EventsBeforeEmitter`), so a reader's view never depends on
  scheduling. Events are not in a snapshot: a snapshot point is between ticks.
- Channel entries written in stage *k* are readable through `Channel<C>` in stages after
  *k* and through `World::take_channel` at a sync point; the input sync point clears them.
- Presentation events are keyed by (origin key, tick, kind). The world keeps the keys it has
  delivered: a re-emission with a delivered key is not delivered again, and a delivered key
  whose tick is re-simulated without re-emitting it is reported as cancelled. The log is
  part of a snapshot, so a rollback does not forget what presentation has seen.

**Schedule**

- A stage is a sync point followed by a DAG. At the sync point the previous stage's
  commands apply, then the stage's exclusive systems run in order, then its other systems
  run as the DAG allows. The first stage's sync point is the input sync point: transient
  columns are zeroed and events and channels cleared before its exclusive systems, which is
  where `sim`, `net` and `asset` apply inputs, staged state and version tables.
- Within a stage, two systems that touch one target with at least one write must be
  ordered by sets or `after` edges; otherwise the build fails. Non-conflicting systems run
  in parallel. The execution order is the topological order with ties by `SystemKey`.
- `Changed<T>` means a row tick of the previous tick or the current one. A write earlier
  in the current tick is seen only by a system ordered after the writer, which the conflict
  rule already requires. On an untracked column the filter is per chunk.
- `run_tick` advances the world's tick first. It is an `async fn` driven by the timer root
  and resolves when nothing of the tick is executing, with `Cancelled` if the executor shut
  down; the world is then mid-tick.

**Regions**

- Kernels run only over rows in `Simulated` regions. `Replicated` and `Frozen` rows are
  reachable through `&mut World` at a sync point. A region's mode, seed, resources and
  exact-version asset table are the world's state; its entity set is a filtered scan.
- `InRegion` is maintained by the world; a system may read it and never writes it.
  `ClaimedBy` is written by `net` and `predict` on join. Neither is saved; `OwnedBy` is.

**Change tracking**

- The current tick is `World::tick`; consumers keep a `ChangeMark` or a `LogCursor` and
  pass it back. A restore, an `apply_overlay` or a rebase starts a new epoch, after which a
  stale mark or cursor yields `None` and the consumer does a full pass.
- Every column in every chunk has a coarse tick, set at dispatch for every chunk a writing
  system touches. Tracked columns (replicated or extracted) have a tick per row, set by
  `Mut` on mutable dereference, by `Commands::set`, `World::insert` and `World::get_mut`,
  and by `ColumnMut::write` for the rows in the dirty range. `Mut::set_if_neq` compares
  bytes, which is exact for a reflected component, and marks nothing when equal.
- `World::columns` yields one slice per chunk with the rows' keys and regions, for passes
  that fork over chunks (extract, replication gather, the world hash). Results keyed by
  anything but `StableKey` are wrong, and SOUR's layout mode catches them.

**Snapshots and bulk copy**

- `WorldSnapshot` holds mutable columns, the entity table, the alias table, region
  resources, the relationship index, the presentation event log and the regions' modes and
  seeds. `restore` rebases ticks, clears the structural log and starts a new epoch.
- `import_baseline` and `apply_overlay` require every component schema to match the
  registry's (`VersionMismatch` otherwise); migration is `lactose-persist`'s. `export_region`
  yields persistent components in ascending `NetId` order with references as `Net` keys;
  diffing against the baseline into an `Overlay` is `lactose-persist`'s, and the world hash
  is `lactose-sim`'s, over `columns_erased` and the region's resources.

## Proof for milestone 0

The facade proves nothing. The implementation lane adds the tests this crate's README then
lists; milestone 0 requires it to pass Miri, and milestone 1 requires SOUR in workers,
repeat, layout and chaos modes.
