# lactose-physics-api

The physics interface: the components that describe a body, its collider and its joints and
the two the write-back writes; the command channel a kernel writes through `cmd.physics()`;
contact and trigger events; queries over the stepped state; the trait a backend implements;
and the snapshot and restore the prediction world rolls back with. `lactose-physics-rapier`
(QUARK) is the backend behind it.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; nothing here runs. This file is the contract the implementation lane and the
backend lane meet, and every sentence under "Behaviour the signatures cannot express" is one
a dependent crate may rely on. Architecture sections 4, 5.3, 6.1, 8.2, 8.3, 9.4, 9.7, 10,
15.1, 16 and 17 are the design; where this file and the architecture disagree, the
architecture wins and this file is wrong. Decision D119 records the choices those sections
left open and what the two design reviews changed. `crates/ecs/README.md` and
`crates/sim/README.md` are the contracts this crate builds on and never restates: what they
say about commands, channels, events, change marks, the structural log, snapshots, stages
and setup functions holds here unchanged.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `physics-rapier`, `predict`, `anim`,
`present-api`, `net` and the game crates are written against this surface.
`tests/consumers.rs` is a compile-only sketch of what each of them writes: a change that
breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `Transform`, `Velocity` | The placement and motion of an entity: the simulation's transform components, what the write-back writes and what any system may write |
| `Teleported` | The last tick the body was placed without a sweep: the no-interpolate flag the write-back writes |
| `RigidBody`, `BodyType`, `locked_axes` | What the solver does with a body: type, mass, damping, gravity scale, locked axes, sleep |
| `Collider`, `Shape`, `ShapeKind`, `CollisionGroups` | The one collision primitive of an entity, its material, groups, event opt-in and impact threshold; alone, a fixed collider |
| `Joint`, `JointKind` | A joint between two bodies, as a row of its own |
| `BodyHandle`, `JointHandle` | The backend's handles, `derived` and untrusted: present exactly while the row is in the solver |
| `PhysicsExt`, `PhysicsWriter`, `PhysicsCommands`, `PhysicsCommand` | `cmd.physics()`: impulses, forces and teleports, recorded into the channel and applied by the next step in command order |
| `ContactEvent`, `TriggerEvent`, `ContactPhase` | What the step emits for `Stage::Post`, indexed by the entity each is for; `Impact` is the damage phase |
| `Physics`, `PhysicsConfig` | The one resource: queries through `Res<Physics>` anywhere, the three systems through `ResMut<Physics>`; gravity and the worker reservation |
| `Ray`, `QueryFilter`, `RayHit`, `ShapeHit` | The queries' arguments and answers, keyed by `StableKey`, in `Scalar`s |
| `PhysicsBackend`, `BodyDesc`, `StepInput`, `StepOutput`, `BodyState` | The trait a backend implements and what it is given and gives back |
| `setup`, `register_types`, `register_systems`, `sets::{SYNC, STEP, WRITE_BACK}` | What `app` adds to a `SimulationBuilder`, and the three sets this crate declares |
| `snapshot`, `restore`, `corrected`, `PhysicsSnapshot` | The physics world beside a `WorldSnapshot`, and how `predict` names what it corrected |
| `conformance::{snapshot_restore, ConformanceReport, TICKS, SNAPSHOT_TICK, BODIES}` | The run a backend passes before this surface is frozen |

Hidden plumbing: none. The three systems live in a private module and `sets` is the only
way to order against them; the bookkeeping of `Physics` is private and "Snapshot and
restore" says what it holds so the implementation and the reviewer agree on it. The handle
constructors and accessors are `#[doc(hidden)]`: the backend's, not game code's.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| Game crates | A body as persistent plain data keyed by `StableKey` through the ECS: shape, mass properties, material, type, groups | `RigidBody`, `Collider` (with `Shape`, `CollisionGroups`, `material`) and `Transform` on the entity; a `Joint` on a row of its own |
| | What the step writes back, as declared writes | `Transform`, `Velocity` and `Teleported` through the write-back at the sync point that begins `Stage::Post`, `writes = [Transform, Velocity, Teleported]`; every other `Stage::Post` system reads them after it with no edge |
| | Impulses, forces, teleports | `cmd.physics().apply_impulse(..)` and the rest of `PhysicsWriter`, from any stage; a command recorded after the step applies at the next tick's step |
| | Joint creation and removal | `cmd.spawn((Joint::new(..), OwnedBy { .. }))` and `cmd.despawn(joint)`: a joint is a row ("Joints" below) |
| | Moving a body from a kernel without a command | `Mut<Transform>`, `Mut<Velocity>`, `Mut<RigidBody>`, `Mut<Collider>` in `Stage::Input`, `Stage::Simulation` or `Stage::Post`: honoured at the next physics sync point ("The tick" below); never in `Stage::Physics` |
| | Contact and trigger events indexed by target, and a damage signal | `Events<ContactEvent>` and `Events<TriggerEvent>` in `Stage::Post`, `for_target(key)`; `ContactPhase::Impact` above `Collider::impact_threshold`, to a target whose own collider has `events` set |
| | Ray, shape cast and overlap from any stage, deterministic | `Res<Physics>` and its `cast_ray`, `cast_shape`, `overlap`; results in origin keys and `Scalar`s ("Queries" below) |
| | A derived handle, rebuilt on load | `BodyHandle` and `JointHandle`, written by the sync; `Option<&BodyHandle>` in a kernel says whether the entity is a live body, and nothing trusts the value |
| | A transform without physics | `Transform` and `Velocity` are this crate's for every entity; a crate that needs them and no physics depends on this crate for the types, which pull no backend |
| `physics-rapier` | The trait: bodies and joints by key, the step over a slice of commands, snapshot and restore as bytes, queries | `PhysicsBackend`; `StepInput::commands` is `&[PhysicsCommand]`, so a backend's own test steps without a world; `StepOutput` takes the moved bodies and the events |
| | The blocking wait and the solver pool | The step calls `PhysicsBackend::step` inside `Job::block_in_place` with `PhysicsConfig::reserved_workers`; the pool is the backend's own, built by `app` with frame worker count minus the reservation threads, at least one, and passed to the backend's constructor |
| | Creation and removal of bodies as rows appear and disappear | The sync's: from the structural log, keyed by origin key, with the prediction-world rule and the region modes applied ("Bodies" below); the backend is told `create_body`, `remove_body`, `set_body`, `set_frozen` and holds no world-kind rule |
| | How a backend is registered | `app` writes the `setup` closure and `register_systems` ("The registration" below); the backend crate exports a constructor and registers no system |
| | The determinism obligations, and what is not yet shown | "The backend's obligations" below, and the `PhysicsBackend` docs |
| | The conformance test | `conformance::snapshot_restore`, called from the backend's test crate with each pool size and each layout |
| `predict` | `snapshot` and `restore` beside `WorldSnapshot`, with the same identity rule, sized for the budget | `snapshot(world)` after `World::snapshot`, `restore(world, &snap)` after `World::restore`; `PhysicsSnapshot::size_in_bytes` ("Snapshot and restore" below) |
| | Re-simulation identical to the first run; corrections pushed to the solver | The snapshot carries the sync's pending work; `corrected(world, &keys)` after `restore` names the rows written without recording |
| `anim` | Ragdoll bodies and sockets: a named place and the types | Bone rows with `RigidBody::kinematic()` and a capsule `Collider` owned by the character, `Joint` rows owned by a bone, `Mut<RigidBody>` to go limp, `&Transform` in `Stage::Post` for sockets; `tests/consumers.rs`, section 2. Limited joints wait on the backend's verification item |
| `sim` and SOUR | The systems placed, readers after them, a setup function for the pool | `register_systems` places `sets::SYNC` and `sets::STEP` in `Stage::Physics` and `sets::WRITE_BACK` first in `Stage::Post`; `setup` is the setup function; `Res<TickDelta>` is the step's delta |
| | Determinism as a contract on the backend; the conformance shape; the place for snapshot hashing | `PhysicsBackend` docs and "The backend's obligations"; `conformance::snapshot_restore` with its `LayoutConfig`; `PhysicsSnapshot::hash` and `ConformanceReport::physics_hash` |
| `present-api`, `net` | The transform to extract and replicate, and the no-interpolate flag | `Transform` is `extracted` and `replicated`; `Teleported` is `persistent, extracted, replicated` and holds a tick to compare with the last one covered; `Velocity`, `RigidBody`, `Collider` and `Joint` are `replicated` |
| `persist` | What a save carries | The components above by their policies; handles are derived; untaken physics commands are not saved ("Commands" below) |
| The scalar decision | Nothing here depends on the width | "The scalar" below |

## The registration app and a backend write

`app` builds the solver pool outside the boundary, hands it to the backend crate's
constructor, and adds the result to the `SimulationBuilder` through this crate's setup
function and registration function. The backend crate registers no system: the sync, the
step and the write-back are this crate's, and the backend is only ever told things through
the trait.

```rust
use lactose_ecs::WorldKind;
use lactose_physics_api::{PhysicsBackend, PhysicsConfig};
use lactose_sim::{SimulationBuilder, TypeTable};

// lactose-physics-rapier exports:
//   pub fn backend(pool: rayon::ThreadPool) -> Box<dyn PhysicsBackend>
// and the pool's threads enter `ThreadClass::PhysicsSolver` in its start handler. `app`
// sizes the pool as `frame_worker_count - reserved_workers`, at least one thread.

fn build(kind: WorldKind, backend: Box<dyn PhysicsBackend>) -> lactose_core::Result<lactose_sim::Simulation> {
    let config = PhysicsConfig { reserved_workers: 2, ..PhysicsConfig::default() };
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types)?;
    // ... every other crate's register_types ...
    SimulationBuilder::new(kind, types)
        .setup(move |world| lactose_physics_api::setup(world, backend, config))
        .systems(lactose_physics_api::register_systems)
        // ... every other crate's setup and register_systems ...
        .build()
}
```

What a game crate orders: a `Stage::Physics` system that reads `Physics` or the contact
and trigger events goes in a set declared `.after(&[&lactose_physics_api::sets::STEP])`,
or the schedule fails to build on the conflict with the step. A `Stage::Post` system needs
no edge: the write-back is exclusive and runs at the stage's sync point before every other
system there. `app` orders any other exclusive `Stage::Post` system against
`sets::WRITE_BACK` with `Registration::order`.

## Behaviour the signatures cannot express

**Keys**

- Every key in this crate is an origin `StableKey`: what `StableKeyOf` yields, what the
  structural log records, what a query answers and what an event targets. A command may
  name a bound predicted body by its `Net` key: the ECS resolves it at apply. A
  `QueryFilter::exclude` or `Physics::is_body` may name one by its `Net` key once the sync
  has seen the bind: the sync refreshes the aliases of its predicted bodies at every
  physics sync point, so a key bound at this tick's input sync point resolves there from
  the physics sync point on, and not before. A placeholder in a command is resolved at
  apply by the ECS; one whose spawn was dropped names nothing and the command is dropped
  and logged at target `physics`.
- Nothing here names a row, chunk, handle index or worker. `BodyHandle` and `JointHandle`
  are opaque values the backend chose; game code tests their presence and never their
  value, nothing in this crate reads the columns back (the bookkeeping `Physics` carries is
  the truth), and a forged, edited or removed handle row changes nothing but what
  `Option<&BodyHandle>` tells a kernel.

**Bodies**

- A row is a *body* in the solver while it has `Transform` and at least one of `RigidBody`
  and `Collider`; with `RigidBody` it is a body of that type, with `Collider` alone it is a
  fixed collider in every world, which is what the cooked static scenery of a region is,
  so a replicated entity that moves carries a `RigidBody`. In a `Prediction` world a row
  with `RigidBody` and no `ClaimedBy` is a *proxy*: the sync hands the backend the
  *effective* body, the row's `RigidBody` with `body_type = Kinematic`, at creation and in
  every later `set_body`, and the backend holds no world-kind rule. The sync treats
  `ClaimedBy` added to or removed from a body row as a change of its `RigidBody`:
  `set_body(key, Some(&effective))`, kinematic while unclaimed, the row's type while
  claimed; the structural log records component additions and removals, so a claim or a
  release is seen in log order like any other presence change. A proxy is moved along its
  per-tick history by `lactose-predict`'s staged-state writes to its `Transform` row,
  which the sync pushes like any row write; predicted entities collide with static
  geometry, other predicted entities and proxies, are pushed by proxies and never push
  them (architecture section 9.4). A row with `RigidBody` or `Collider` and no `Transform`
  is skipped and logged.
- The sync creates the body at the first physics sync point at which the row is one
  (`PhysicsBackend::create_body` from the rows as they are then, the region's mode as
  `frozen`), removes it at the first at which it is not (`remove_body`), and adds or
  removes the `BodyHandle` row in the same call. Between the two, a change of the row's
  `RigidBody` or `Collider` (the value, or the component's presence while the other keeps
  the row a body) reaches the backend as `set_body` or `set_collider` with the new value or
  `None`, and the handle stays. So a ragdoll goes limp by writing `body_type` through
  `Mut<RigidBody>`, and a fixed collider becomes a body by `cmd.set(key,
  RigidBody::dynamic())`. A `create_body` or `create_joint` the backend refuses
  (`InvalidData`: a NaN, a non-unit quaternion, a negative extent) is logged and the row is
  retried when it changes.
- A body in a region whose mode is not `Simulated` is a fixed collider at its last pose
  (architecture section 9.7): the sync calls `set_frozen(key, true)` for every body of a
  region when its mode leaves `Simulated` and for a body moved into such a region, and
  `set_frozen(key, false)` followed by `set_velocity` from the row when the body is
  simulated again. A region's mode is read from the world at every sync, so a change made
  at the input sync point takes effect at the physics sync point of the same tick.
- The sync reads the structural log through a cursor and the changed rows through a mark,
  both its own, and keeps the one map between rows and the solver. When either is stale
  (a world restore without `physics_api::restore`, a whole-world load, `set_tick`, the
  2^30 stamp rebase) the sync diffs every row, in ascending key order, against that map:
  a row that is a body and not in the map is created, a key in the map whose row is not a
  body is removed, and every row in both is pushed through `set_body`, `set_collider`,
  `set_transform` and `set_velocity`, which the backend compares and ignores when equal.
  It never clears the solver: sleep and warm starts survive wherever the rows agree.
- A `Velocity` is not required to be a body; a dynamic or kinematic body without one
  gains one at its first write-back, since the write-back adds the component.
- `written`, the record the sync compares `Transform` and `Velocity` row writes against,
  is replaced by every write-back with exactly what that write-back wrote; it never
  merges, so a sleeping body has no entry and a game write back to its last pose is pushed.
  Every row write that reaches the backend and every command wakes the body. A `Velocity`
  write on a kinematic body is ignored by the solver, whose kinematic velocity is the
  sweep's. The backend never mutates a solver body for an equal value.

**The tick** (architecture sections 5.2, 5.3, 6.1 and 16)

Three systems, `worlds = [server, prediction]`: two in `Stage::Physics` in `sets::SYNC` and
`sets::STEP`, one in `Stage::Post` in `sets::WRITE_BACK`. In every tick, in this order:

1. The sync point that begins `Stage::Physics` applies the simulation stage's commands,
   channel entries included.
2. `physics_api::sync` (exclusive, `&mut World`, `ResMut<Physics>`, `writes = [BodyHandle,
   JointHandle]`): region modes; then the pending log entries a restore carried and the
   structural log since the cursor, in log order (or the full diff above when the cursor
   or mark is stale); then the union of the pending keys and the rows of `RigidBody`,
   `Collider`, `Transform`, `Velocity` and `Joint` changed since the mark, in ascending key
   order (the ECS yields them in storage order and the sync sorts), skipping a `Transform`
   or `Velocity` whose value equals what the last write-back wrote for that key; then the
   waiting joints in ascending key order; then
   `World::take_channel::<PhysicsCommands>()`, appended after the pending commands a
   restore carried, as this tick's command list. The cursor and the mark are then taken.
3. `physics_api::step` (once per tick, `Job`, `ResMut<Physics>`, `Res<TickDelta>`, two
   `EventWriter`s): inside `Job::block_in_place(config.reserved_workers, ..)`, the one
   sanctioned blocking wait, `PhysicsBackend::step` applies the command list in order and
   advances the world by `TickDelta`; then the step keeps what the backend reported and
   emits the contact events sorted ascending by (`target`, `other`, `phase`) and then the
   trigger events likewise. Systems in the stage that touch neither `Physics` nor the
   events run alongside it on the reserved workers. A system there that reads `Physics` or
   the events is in a set after `sets::STEP`, where `Res<Physics>` and the events are this
   tick's but the `Transform` and `Velocity` rows are the previous tick's; nothing but the
   write-back writes a body's `Transform` or `Velocity` in `Stage::Physics`, since the
   write-back overwrites it.
4. `physics_api::write_back` (exclusive, first in `Stage::Post`, `writes = [Transform,
   Velocity, Teleported]`): at the sync point that begins `Stage::Post`, after the physics
   stage's commands have applied, for every `BodyState` the step reported, in ascending key
   order, writes `Transform` and `Velocity` through `World::get_mut` and `Mut::set_if_neq`,
   so only a row whose bytes changed is stamped, adds a missing `Velocity` through
   `World::insert`, and for a body the step placed without a sweep writes the tick into
   `Teleported` (through `set_if_neq`, adding the component where the row lacks it); it
   skips a reported key whose row a command despawned at this sync point. What it wrote
   replaces `written`. Readers there, extract and replication see this tick's state;
   `Changed<Transform>` passes the moved bodies and every other `Transform` write since the
   reader last ran. Per key rather than per column: the cost is in the moved bodies, not in
   every row that has a transform (measured on the naive ECS at 100k entities and 2,000
   moved bodies: 0.43 ms per tick against 3.2 ms for a `columns_mut` walk).

So a kernel in `Stage::Simulation` may write `Velocity` through `Mut` *and* record an
impulse: the sync sets the velocity, then the step adds the impulse, then steps. A
`Transform` write places a dynamic or fixed body there with no sweep (the write-back then
writes `Teleported`) and sets where a kinematic body sweeps to during the step;
`cmd.physics().teleport` places any body there with no sweep. A write made in `Stage::Post`
after the write-back is seen by the next tick's sync and overrides what the step wrote; a
command recorded in `Stage::Post` is applied by the next tick's step. `Teleported` holds a
tick rather than a per-tick flag so that a frame which runs several ticks before its passes
(catch-up, re-simulation, a gather over several ticks) loses no teleport: extract and the
gather compare it with the last tick they covered, and zero means never.

**Commands**

- `PhysicsCommands` declares `CLEAR_AT_INPUT = false`, so its entries stay until the sync
  takes them: a command recorded in `Stage::Input` or `Stage::Simulation` reaches this
  tick's step, one recorded in `Stage::Physics` or `Stage::Post` the next tick's, and the
  step applies every command recorded since the previous step, in command order (the
  ECS's: execution order of the recording systems, then source key, then sequence, ticks
  in order). A kernel reading `Events<ContactEvent>` in `Stage::Post` may therefore record
  an explosion impulse or a portal teleport, and it lands one tick later.
- Untaken commands (those recorded after the step of the last tick) are not in a
  `SavePoint` or a `Journal`: a load and a replay drop them, as they drop the solver's
  state. A rollback keeps them: the physics snapshot carries them ("Snapshot and
  restore").
- A world whose systems record `cmd.physics()` and that holds no `Physics` resource grows
  the channel without bound, since nothing takes it: `app` inserts one in every world that
  runs such systems, and a test either inserts one or records nothing.
- A command for a key that is not a body when the step runs is dropped and logged at
  target `physics`. An impulse or force on a kinematic, fixed or frozen body is dropped
  likewise. A `Teleport` works on every body.
- `ApplyForce` and `ApplyTorque` act for one step alone; a sustained force is recorded
  every tick.

**Events**

- `ContactEvent`s are emitted for pairs of solid colliders: `Started` and `Stopped`, when
  the pair begins and ends touching during the step, go to both sides of a pair if either
  of its colliders has `Collider::events` set; `Impact`, in every step whose normal
  impulse on the pair exceeds the target's `Collider::impact_threshold` (Rapier's
  `ContactForceEvent`, whose force threshold the backend converts), goes to a target only
  if the target's own collider has `events` set, whether or not the pair started or
  stopped in that step. `Started` may come from a speculative contact a little before the
  bodies meet, with an impulse near zero: a damage rule reads `Impact`. Each event is for
  one entity as `target`, at most one per (`target`, `other`, `phase`) per tick.
  `TriggerEvent`s for pairs with at least one sensor, likewise, `Started` and `Stopped`
  only, to both sides; a sensor has `events` set by `Collider::sensor`. A sensor pushes
  nothing and is pushed by nothing. Pairs that go on touching below the threshold emit
  nothing: a resting contact is found with a query.
- Readers are in `Stage::Post`, or in `Stage::Physics` after `sets::STEP`; the ECS rejects
  a reader ordered before the emitter. Events are cleared at the input sync point and are
  not in a snapshot.
- The emission order is a pure function of the step's result, not of the backend's
  iteration order, so two readers on two machines see one order.

**Queries**

- `Res<Physics>` answers from the state the last step left: in `Stage::Input` and
  `Stage::Simulation` the previous tick's step, in `Stage::Post` this tick's, and in
  `Stage::Physics` after `sets::STEP` this tick's. A row write, a spawn or a correction
  made this tick is not in the query's state until the next step; the row is.
- `cast_ray` and `cast_shape` answer the nearest hit among the colliders the filter admits,
  the lowest origin key among hits at exactly equal distance; `overlap` appends every
  admitted collider the shape overlaps in ascending origin-key order, each key once. A
  sensor is admitted only when `QueryFilter::sensors` is set. `QueryFilter::groups` is
  tested against each collider's `CollisionGroups` as two colliders are tested against
  each other: the query's membership must meet the collider's filter and the collider's
  membership the query's filter.
- The answers depend on the backend's state and the query alone: equal on every machine,
  under every chunk layout and worker count, and from every worker that asks at once.
  `Physics` is `Sync` and the queries take `&self`.

**Joints**

- A `Joint` is a row. It is created in the solver at the first physics sync point at which
  the row exists and both `body_a` and `body_b` name rows that are bodies; until then it
  waits, and the sync retries the waiting joints at every sync point in ascending key
  order. A joint whose reference is empty (a body despawned, so the `EntityRef` was emptied
  at apply) is removed from the solver and not retried until its `Joint` row changes. Its
  `JointHandle` row is written when it is created and removed when it leaves the solver:
  when its row is despawned or loses `Joint`, when either body stops being a body (the
  backend removes the joints of a removed body; the joint then waits again), or when the
  `Joint` value changes, which removes and recreates it.
- A joint between a frozen body and a simulated one is a joint to a fixed body. A joint
  whose bodies are in two regions is allowed within one process (architecture section 9.6
  keeps cross-region references weak, and this one is validated at every sync).
- Spawn a joint owned by one of its bodies (`OwnedBy`) so it migrates and despawns with
  it. Joint rows are `persistent, replicated`: a ragdoll's joints survive a save and are
  predicted with the bones.

**Snapshot and restore** (architecture section 10)

- `snapshot(world)` is taken between ticks at the point `World::snapshot` is, and
  `restore(world, &snap)` after `World::restore` of the matching world snapshot. The
  snapshot has two parts. The bytes are the backend's whole world, with each collider's
  `Shape` value (an asset reference once meshes exist) in place of the backend's shape
  serialisation; `restore` rebuilds shapes from those values while deserialising, with no
  change flag, wake or mass update; there is no `set_collider` replay and no
  fixed-collider reconciliation. This crate's part is its bookkeeping and the sync's
  pending work: which rows are bodies and joints with their handles, the waiting joints,
  the region modes, the aliases, `written`, the structural log entries after the sync's
  cursor in order, the keys changed since its mark across the five columns, and the
  channel's entries not yet taken (`World::channel`); or, when the cursor or mark is
  stale, that a full diff is due. `snapshot` records this crate's bookkeeping and what the
  next sync would see; `restore` puts them back, and the next sync processes the pending
  log entries before the log since the restore, and the union of pending and newly changed
  keys in key order, so a re-simulated tick's sync makes exactly the backend calls the
  first run's did.
- `restore`, in order: the backend's restore of the bytes; then the carried bookkeeping
  replaces this crate's and a fresh cursor and mark are taken. The snapshot carries the
  resource's identity and the world's tick, and `restore` refuses, with `InvalidData`, a
  snapshot from another resource or a world whose tick is not the snapshot's: restore the
  world first. On any failure nothing changes and the next sync diffs every row
  ("Bodies").
- `corrected(world, &keys)`: `lactose-predict` writes a correction through
  `ColumnMut::write` without recording, so that `Changed<T>` does not see it, and the sync
  cannot either; it names the corrected body rows after `restore` and before the next step,
  and the next sync reads their five columns as changed and pushes what differs from the
  solver. A correction written to a non-body is ignored. Rapier's internal solver state is
  not corrected (architecture section 9.4), so a corrected dynamic body converges over a
  few ticks.
- `PhysicsSnapshot::size_in_bytes` is the backend's part, what milestone 1 measures
  against the 100 µs prediction-world budget for 50 claimed bodies plus N statics; that
  measurement decides how many statics the prediction world may hold. `hash` is the named
  place where section 8.3 adds the bytes to the world hash once the backend's
  serialisation is shown to be order-stable (`docs/verification.md`); until then
  `ConformanceReport::physics_hash` reports it and SOUR records nothing of it.

**The backend's obligations** (architecture sections 1, 2 and 8.2; the `PhysicsBackend`
docs restate them per method)

- Inside the boundary: `#![forbid(unsafe_code)]` except what section 21 grants it (nothing;
  its pool's threads enter `ThreadClass::PhysicsSolver` through `lactose_core`), the
  boundary `clippy.toml`, every float through its pinned math crates, no clock, no thread
  or worker identity, no thread-local or global state. Every map its math crates iterate
  on a result path is an insertion-ordered `IndexMap` with an unseeded `FxHasher32`, which
  Rapier's `enhanced-determinism` feature selects; `hashbrown` is that map's index table
  and `foldhash` is compiled but not on the path.
- For one sequence of calls with equal arguments it gives equal `StepOutput`s as sets,
  equal query answers and equal `snapshot` bytes under any thread count of its pool and
  whatever ran on those threads before. The thread count may change how long a step takes
  and nothing else. Rapier's determinism feature with its parallel solver is how
  `physics-rapier` meets this (every parallel site is ordered or sorted and the solver
  layout is calibrated to a constant worker count); Rapier's own tests check a tolerance,
  so bit identity across pool sizes is a design claim that SOUR's workers mode proves.
- The obligation is section 2's, identical on x86-64 and aarch64; the verification items
  name the two known gaps of the pinned Rapier (platform-libm `sin_cos` on limited joints,
  the sign of zero in SIMD min/max), which the backend lane closes before SOUR's
  architecture mode gates; until then the property is unproven, not excused. The `sin_cos`
  route is patched or vendored, or `Joint::limited` refused until it is; the sign-of-zero
  difference is measured through the conformance scene. `matrixmultiply` selects kernels
  by CPU feature and is allowlisted as an explicit second exception beside `libm` because
  it is unreachable from the step: only multibody inverse kinematics reaches it, and the
  boundary lint file bans that entry point.
- It validates every value it is given and never aborts on a row's bytes: an invalid value
  is logged and the call does nothing, `create_body` and `create_joint` return
  `InvalidData`. It compares before it mutates: an equal value changes nothing and wakes
  nothing. It converts what reaches a row, an event or a query answer to `Scalar` exactly,
  and never produces a NaN in a persistent column deliberately.
- The order of the lists in `StepOutput` is free; this crate sorts what is observable.
- A call for a key it does not hold does nothing and logs at target `physics`; it never
  panics on a key. `create_body` is never called for a key it holds.

**The scalar** (architecture section 16)

- Every quantity here is a `Scalar`, `Vec3` or `Quat` of `lactose_math`; no `f32` or `f64`
  appears in the surface. Every component is its scalars first, then a tail whose size is
  a multiple of eight bytes, and `lib.rs` asserts each size as `n * Scalar::BYTES + tail`,
  so no padding changes if milestone 1 picks a four-byte scalar.
- What changes then: `lactose-math`'s `Scalar` and the `Scalar::NAME` tag (`Scalar64` to
  `Scalar32`, a schema change every save and cook sees), and in `physics-rapier` the
  crates `rapier3d-f64` and `parry3d-f64` become `rapier3d` and `parry3d`. Nothing in this
  crate changes, and a backend converts between `Scalar` and its own float at the
  boundary of each call, exactly in either direction when the widths agree.

## Not in this crate yet

- Cooked collision meshes for terrain and static scenery (architecture sections 14 and
  22, milestone 3): a `ShapeKind::Mesh` with an `AssetRef<CollisionMesh>` field on `Shape`
  and the `CollisionMesh` simulation asset kind, whose loader is a pure byte decoder here
  and whose bytes the cooker emits. Adding the field is a schema version bump of `Shape`
  and `Collider`, and a physics snapshot then carries the reference, not the mesh. Until
  then static scenery is primitives.
- Several colliders on one body: child rows carrying `Collider` and a reference to the
  body, when a shape needs it. One collider per entity today.
- Continuous collision detection for fast projectiles: a `RigidBody` field with a schema
  bump, when the netcode spike's thrown objects show tunnelling. With it, a pair may start
  and stop in one step; the events' order key already tells the two apart.
- Lag compensation's history of transforms and colliders (architecture section 9.5):
  `net`'s, over the `Transform` and `Collider` rows, milestone 4.
- Hashing the snapshot bytes into the world hash: `PhysicsSnapshot::hash` is the place;
  the verification item decides when.
- A per-region body count and the contacts of one body between events: queries a game
  system has not asked for; `Physics::body_count` is the whole world's.

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves, under SOUR
in workers, repeat, layout and chaos modes, against a backend (the backend lane's Rapier,
or a test backend that integrates velocities and reports every body). The scenes named
"probe" are the design reviews' probes in their rewritten (round 2) form, which ran against
the real ECS with stand-ins for the three systems; the lane turns each into a test of the
real systems.

| What | How |
|---|---|
| Bodies follow rows | Spawn, `cmd.set` of `RigidBody` and `Collider`, their removal, despawn, region move, region load and unload: `create_body`, `set_body`, `set_collider`, `remove_body` and `set_frozen` in the stated order, handles present exactly meanwhile; the same under every layout |
| The prediction rule | In a `Prediction` world a `RigidBody` row without `ClaimedBy` is created with an effective kinematic body and a claimed one with the row's type; `ClaimedBy` added or removed on a body row is `set_body` with the effective body; a `Collider` row is a fixed collider in every kind |
| Row writes | A `Mut<Velocity>` write and an impulse in one tick: the backend sees `set_velocity` then the command; a `Transform` write to a kinematic body is `set_transform`; the step's own write-back is never pushed back (probe F: the sync saw 2,000 changed rows and pushed none); a write in `Stage::Post` after the write-back is pushed next tick |
| Commands | Order equals command order under every layout, across the tick boundary for a command recorded in `Stage::Post` or in `Stage::Physics` after the step (probe B); a command for a non-body is dropped and logged |
| Events | Emitted sorted whatever order the backend reports; `Impact` above the threshold to a target with `events`, never to one without; `Started` and `Stopped` to both sides when either has `events`; a reader in `Stage::Post` sees each once; readers before the step fail to build |
| Queries | `cast_ray` ties go to the lowest key; `overlap` ascending; a `Stage::Physics` reader of `Physics` without an edge after `sets::STEP` fails to build; an unordered `Stage::Post` reader of `Transform` builds and reads this tick's rows (probe S1); another crate's exclusive `Stage::Post` system fails to build until `app` orders it (probe S2) |
| Joints | A joint spawned before its bodies waits and is created when they exist; a body's removal removes it and it waits again; an emptied reference parks it until the row changes; a `Joint` value change recreates it |
| Snapshot and restore | `conformance::snapshot_restore` with the test backend under the production layout and SOUR's; probe B's scene (a `Stage::Post` move, spawn, despawn and impulse and a `Stage::Physics` command at the snapshot tick, with an input tick set every tick): the re-simulated syncs make the same backend calls as the first run and the rows are equal; probe B without an input tick: a predicted spawn is dropped, so the scene sets one; a correction named through `corrected` is pushed (probe C); a restore of the world alone diffs without clearing and keeps running; the 2^30 rebase diffs without clearing (probe D); a snapshot from another resource is refused |
| The write-back | Only rows whose bytes changed are stamped; `Teleported` holds the teleport tick after two further ticks in one frame (probe T); a reported key despawned at the Post sync point is skipped; the per-key cost for 2,000 moved bodies among 100k rows (probe F: 0.43 ms, against 3.2 ms for a `columns_mut` walk and 1.6 ms for `Commands::set`) re-measured on the milestone 1 ECS against the 3 ms physics phase |
| The scalar | The size assertions in `lib.rs`, and the schema dump of every component contains no `f32` or `f64` |
| The snapshot size | `PhysicsSnapshot::size_in_bytes` and `snapshot` time for 50 claimed bodies plus N statics, against the 100 µs budget; the result decides how many statics a prediction world may hold |
