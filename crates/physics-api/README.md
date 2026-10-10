# lactose-physics-api

The physics interface: the components that describe a body, its collider and its joints and
the two the step writes back; the command channel a kernel writes through `cmd.physics()`;
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
a dependent crate may rely on. Architecture sections 4, 6.1, 8.2, 8.3, 9.4, 9.7, 10, 15.1,
16 and 17 are the design; where this file and the architecture disagree, the architecture
wins and this file is wrong. Decision D119 records the choices those sections left open.
`crates/ecs/README.md` and `crates/sim/README.md` are the contracts this crate builds on
and never restates: what they say about commands, channels, events, change marks, the
structural log, snapshots, stages and setup functions holds here unchanged.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `physics-rapier`, `predict`, `anim`,
`present-api`, `net` and the game crates are written against this surface.
`tests/consumers.rs` is a compile-only sketch of what each of them writes: a change that
breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `Transform`, `Velocity` | The placement and motion of an entity: what the step writes back and what any system may write |
| `RigidBody`, `BodyType`, `locked_axes` | What the solver does with a body: type, mass, damping, gravity scale, locked axes, sleep |
| `Collider`, `Shape`, `ShapeKind`, `CollisionGroups` | The one collision primitive of an entity, its material and its groups; alone, a fixed collider |
| `Joint`, `JointKind` | A joint between two bodies, as a row of its own |
| `BodyHandle`, `JointHandle` | The backend's handles, `derived`: present exactly while the row is in the solver |
| `PhysicsExt`, `PhysicsWriter`, `PhysicsCommands`, `PhysicsCommand` | `cmd.physics()`: impulses, forces and teleports, recorded into the channel and applied by the step in command order |
| `ContactEvent`, `TriggerEvent`, `ContactPhase` | What the step emits for `Stage::Post`, indexed by the entity each is for |
| `Physics`, `PhysicsConfig` | The one resource: queries through `Res<Physics>` anywhere, the step and the sync through `ResMut<Physics>`; gravity and the worker reservation |
| `Ray`, `QueryFilter`, `RayHit`, `ShapeHit` | The queries' arguments and answers, keyed by `StableKey`, in `Scalar`s |
| `PhysicsBackend`, `BodyDesc`, `StepInput`, `StepOutput`, `BodyState` | The trait a backend implements and what it is given and gives back |
| `setup`, `register_types`, `register_systems`, `sets::{SYNC, STEP}` | What `app` adds to a `SimulationBuilder`, and the two sets a consumer orders itself against |
| `snapshot`, `restore`, `PhysicsSnapshot` | The physics world beside a `WorldSnapshot` |
| `conformance::{snapshot_restore, ConformanceReport, TICKS, SNAPSHOT_TICK, BODIES}` | The run a backend passes before this surface is frozen |

Hidden plumbing: none. The two systems live in a private module and `sets` is the only way
to order against them; the bookkeeping of `Physics` is private and the README says what it
holds so the implementation and the reviewer agree on it.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| Game crates | A body as persistent plain data keyed by `StableKey` through the ECS: shape, mass properties, material, type, groups | `RigidBody`, `Collider` (with `Shape`, `CollisionGroups`) and `Transform` on the entity; a `Joint` on a row of its own |
| | What the step writes back, as declared writes | `Transform` and `Velocity` through `Commands::set` from the step, `writes = [Transform, Velocity]`, visible from `Stage::Post` |
| | Impulses, forces, teleports | `cmd.physics().apply_impulse(..)` and the rest of `PhysicsWriter`, recorded in `Stage::Input` or `Stage::Simulation` |
| | Joint creation and removal | `cmd.spawn((Joint::new(..), OwnedBy { .. }))` and `cmd.despawn(joint)`: a joint is a row ("Joints" below) |
| | Moving a body from a kernel without a command | `Mut<Transform>`, `Mut<Velocity>`, `Mut<RigidBody>`, `Mut<Collider>`: honoured at the next physics sync point ("The tick" below) |
| | Contact and trigger events indexed by target | `Events<ContactEvent>` and `Events<TriggerEvent>` in `Stage::Post`, `for_target(key)` |
| | Ray, shape cast and overlap from any stage, deterministic | `Res<Physics>` and its `cast_ray`, `cast_shape`, `overlap`; results in origin keys and `Scalar`s ("Queries" below) |
| | A derived handle, rebuilt on load | `BodyHandle` and `JointHandle`, written by the sync; `Option<&BodyHandle>` in a kernel says whether the entity is a live body |
| `physics-rapier` | The trait: bodies and joints by key, the step over the tick's commands, write-back, snapshot and restore as bytes, queries | `PhysicsBackend`; `StepInput` carries the channel, `StepOutput` takes the moved bodies and the events |
| | The blocking wait and the solver pool | The step calls `PhysicsBackend::step` inside `Job::block_in_place` with `PhysicsConfig::reserved_workers`; the pool is the backend's own, built by `app` and passed to the backend's constructor |
| | Creation and removal of bodies as rows appear and disappear | The sync's: from the structural log, keyed by origin key, with the prediction-world filter and the region modes applied ("Bodies" below); the backend is told `create_body`, `remove_body`, `set_frozen` |
| | How a backend is registered | `app` writes the `setup` closure and `register_systems` ("The registration" below); the backend crate exports a constructor and registers no system |
| | The determinism obligations | "The backend's obligations" below, and the `PhysicsBackend` docs |
| | The conformance test | `conformance::snapshot_restore`, called from the backend's test crate with each pool size |
| `predict` | `snapshot` and `restore` beside `WorldSnapshot`, with the same identity rule, sized for the budget | `snapshot(world)` after `World::snapshot`, `restore(world, &snap)` after `World::restore`; `PhysicsSnapshot::size_in_bytes` ("Snapshot and restore" below) |
| | The physics world and the rows consistent after a restore and through re-simulation | Handles are columns, so they come back with the world; the sync's cursor and mark are refreshed by `restore`; a world restored without a physics restore is rebuilt from rows |
| `anim` | Ragdoll bodies and sockets: a named place and the types | Bone rows with `RigidBody::kinematic()` and a capsule `Collider` owned by the character, `Joint` rows owned by a bone, `Mut<RigidBody>` to go limp, `&Transform` in `Stage::Post` for sockets; nothing else is needed here and nothing speculative is added (`tests/consumers.rs`, section 2) |
| `sim` and SOUR | The step placed in `Stage::Physics`, readers in `Stage::Post`, a setup function for the pool | `register_systems` places `sets::SYNC` and `sets::STEP`; `setup` is the setup function; `Res<TickDelta>` is the step's delta |
| | Determinism as a contract on the backend; the conformance shape; the place for snapshot hashing | `PhysicsBackend` docs and "The backend's obligations"; `conformance::snapshot_restore`; `PhysicsSnapshot::hash` |
| `present-api`, `net` | The transform to extract and replicate | `Transform` is `extracted` and `replicated`; `Velocity`, `RigidBody`, `Collider` and `Joint` are `replicated` |
| The scalar decision | Nothing here depends on the width | "The scalar" below |

## The registration app and a backend write

`app` builds the solver pool outside the boundary, hands it to the backend crate's
constructor, and adds the result to the `SimulationBuilder` through this crate's setup
function and registration function. The backend crate registers no system: the sync and
the step are this crate's, and the backend is only ever told things through the trait.

```rust
use lactose_ecs::WorldKind;
use lactose_physics_api::{PhysicsBackend, PhysicsConfig};
use lactose_sim::{SimulationBuilder, TypeTable};

// lactose-physics-rapier exports:
//   pub fn backend(pool: rayon::ThreadPool) -> Box<dyn PhysicsBackend>
// and the pool's threads enter `ThreadClass::PhysicsSolver` in its start handler.

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

A game crate orders nothing against physics unless it places a system in `Stage::Physics`
that reads `Physics`, `Transform` or `Velocity`, which goes in a set declared
`.after(&[&lactose_physics_api::sets::STEP])`.

## Behaviour the signatures cannot express

**Keys**

- Every key in this crate is an origin `StableKey`: what `StableKeyOf` yields, what the
  structural log records, what a query answers and what an event targets. A command, a
  `QueryFilter::exclude` or `Physics::is_body` may name a bound predicted body by its
  `Net` key as well: the sync refreshes the aliases of its predicted bodies at every
  physics sync point, so a key the server bound at this tick's input sync point resolves.
  A placeholder in a command is resolved at apply by the ECS; one whose spawn was dropped
  names nothing and the command is dropped and logged at target `physics`.
- Nothing here names a row, chunk, handle index or worker. `BodyHandle` and `JointHandle`
  are opaque values the backend chose; game code tests their presence and never their
  value, and the backend keeps its own key-to-handle map.

**Bodies**

- A row is a *body* in the solver while it has `Transform` and at least one of `RigidBody`
  and `Collider`; with `RigidBody` it is a body of that type, with `Collider` alone it is a
  fixed collider, which is what the cooked static scenery of a region is. In a
  `Prediction` world a row with `RigidBody` is a body only while it also has `ClaimedBy`
  (architecture section 9.4: predicted entities collide with static geometry and other
  predicted entities; a proxy is driven by `lactose-predict`'s staged state and is not in
  the physics world), and a row with `Collider` alone is a fixed collider in every kind.
  A row with `RigidBody` or `Collider` and no `Transform` is skipped and logged.
- The sync creates the body at the first physics sync point at which the row is one
  (`PhysicsBackend::create_body` from the rows as they are then, the region's mode as
  `frozen`), removes it at the first at which it is not (`remove_body`), and writes or
  removes the `BodyHandle` row in the same call. Between the two, a change of the row's
  `RigidBody` or `Collider` (the value, or the component's presence while the other
  keeps the row a body) reaches the backend as `set_body` or `set_collider` with the new
  value or `None`, and the handle stays. So a ragdoll goes limp by writing
  `body_type` through `Mut<RigidBody>`, and a fixed collider becomes a body by
  `cmd.set(key, RigidBody::dynamic())`.
- A body in a region whose mode is not `Simulated` is a fixed collider at its last pose
  (architecture section 9.7): the sync calls `set_frozen(key, true)` for every body of a
  region when its mode leaves `Simulated` and for a body moved into such a region, and
  `set_frozen(key, false)` followed by `set_velocity` from the row when the body is
  simulated again. A region's mode is read from the world at every sync, so a change made
  at the input sync point takes effect at the physics sync point of the same tick.
- The sync reads the structural log through a cursor and the changed rows through a mark,
  both the sync's own. When either is stale (a restore without `restore`, a whole-world
  load, `set_tick`, the stamp rebase) the sync calls `PhysicsBackend::clear`, walks every
  row in ascending key order (`World::query`) and recreates what is a body and what is a
  joint, rewriting every handle: the physics world is then consistent with the rows and
  the solver's state is fresh. `physics_api::restore` takes a fresh cursor and mark after
  the backend restore, so a rollback is incremental, not a rebuild.
- A `Velocity` is not required to be a body; a dynamic or kinematic body without one
  gains one at its first write-back, since `Commands::set` adds the component.

**The tick** (architecture sections 5.2, 6.1 and 16)

Both systems are in `Stage::Physics`, in `sets::SYNC` and `sets::STEP`, with `worlds =
[server, prediction]`. In every tick, in this order:

1. The sync point that begins `Stage::Physics` applies the simulation stage's commands.
2. `physics_api::sync` (exclusive, `&mut World`, `ResMut<Physics>`, `writes = [BodyHandle,
   JointHandle]`): region modes, then the structural log in log order, then the rows of
   `RigidBody`, `Collider`, `Transform` and `Velocity` changed since the mark, in
   ascending key order, then the waiting joints in ascending key order. A changed
   `Transform` or `Velocity` whose value equals what the last step wrote back for that key
   is the step's own write-back and is skipped; any other changed value reaches the
   backend as `set_transform` or `set_velocity`. A changed `RigidBody` or `Collider` always
   reaches it. The cursor and the mark are then taken.
3. `physics_api::step` (once per tick, `Job`, `ResMut<Physics>`, `Res<TickDelta>`,
   `Channel<PhysicsCommands>`, `Commands`, two `EventWriter`s, `writes = [Transform,
   Velocity]`): inside `Job::block_in_place(config.reserved_workers, ..)`, the one
   sanctioned blocking wait, `PhysicsBackend::step` applies the tick's commands in command
   order and advances the world by `TickDelta`; then the step records `Commands::set` of
   `Transform` and `Velocity` for every `BodyState` the backend reported, remembers those
   values for the next sync, and emits the contact events sorted ascending by (`target`,
   `other`, `phase`) and then the trigger events likewise. Systems in the stage that touch
   none of `Physics`, `Transform` and `Velocity` run alongside it on the reserved workers;
   a system that touches one must be in a set ordered after `sets::STEP`, or the schedule
   fails to build on the conflict. One that reads `BodyHandle` or `JointHandle` in the
   stage conflicts with the sync's declared writes and is ordered after `sets::SYNC`,
   which a set after `sets::STEP` already is.
4. The sync point that begins `Stage::Post` applies the write-backs: readers there, extract
   and replication see them, and `Changed<Transform>` passes exactly the moved bodies.

So a kernel in `Stage::Simulation` may write `Velocity` through `Mut` *and* record an
impulse: the sync sets the velocity, then the step adds the impulse, then steps. A
`Transform` write places a dynamic or fixed body there with no sweep and sets where a
kinematic body sweeps to during the step; `cmd.physics().teleport` places any body there
with no sweep. A write made in `Stage::Post` or later is seen by the next tick's sync and
overrides what the step wrote.

**Commands**

- `PhysicsCommand`s recorded in `Stage::Input` or `Stage::Simulation` reach this tick's
  step, in command order (the ECS's: execution order of the recording systems, then source
  key, then sequence). Entries recorded in `Stage::Physics` or `Stage::Post` never reach a
  step: the input sync point clears every channel.
- A command for a key that is not a body when the step runs is dropped and logged at
  target `physics`. An impulse or force on a kinematic or fixed body is dropped likewise. A
  `Teleport` works on every body.
- `ApplyForce` and `ApplyTorque` act for this tick's step alone; a sustained force is
  recorded every tick.

**Events**

- `ContactEvent`s are emitted for pairs of solid colliders that started or stopped
  touching during the step, twice per pair with each entity as `target`; `TriggerEvent`s
  for pairs with at least one sensor, likewise. Pairs that go on touching emit nothing: a
  resting contact is found with a query. A sensor pushes nothing and is pushed by nothing.
- Readers are in `Stage::Post`, or in `Stage::Physics` after `sets::STEP`; the ECS rejects
  a reader ordered before the emitter. Events are cleared at the input sync point and are
  not in a snapshot.
- The emission order is a pure function of the step's result, not of the backend's
  iteration order, so two readers on two machines see one order.

**Queries**

- `Res<Physics>` answers from the state the last step left: in `Stage::Input` and
  `Stage::Simulation` the previous tick's step, in `Stage::Post` this tick's, and in
  `Stage::Physics` whichever the set order says. A `Transform` written by a kernel this
  tick is not in the query's state until the sync; the row is.
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
  order. Its `JointHandle` row is written when it is created and removed when it leaves
  the solver: when its row is despawned or loses `Joint`, when either body stops being a
  body (the backend removes the joints of a removed body; the joint then waits again), or
  when the `Joint` value changes, which removes and recreates it. An `EntityRef` emptied at
  apply (a body despawned) makes the joint wait.
- A joint between a frozen body and a simulated one is a joint to a fixed body. A joint
  whose bodies are in two regions is allowed within one process (architecture section 9.6
  keeps cross-region references weak, and this one is validated at every sync).
- Spawn a joint owned by one of its bodies (`OwnedBy`) so it migrates and despawns with
  it. Joint rows are `persistent, replicated`: a ragdoll's joints survive a save and are
  predicted with the bones.

**Snapshot and restore** (architecture section 10)

- `snapshot(world)` is taken between ticks at the point `World::snapshot` is, and
  `restore(world, &snap)` after `World::restore` of the matching world snapshot. The bytes
  are the backend's whole world: bodies, colliders, joints, solver state and its
  key-to-handle map. What is rebuilt from rows at `restore`: which rows are bodies and
  joints and their handles (from the restored `BodyHandle` and `JointHandle` columns, which
  the world snapshot carries like every column), the waiting joints (the `Joint` rows
  without a handle), the region modes, and the aliases; then a fresh cursor and mark are
  taken, so the next sync is incremental. The snapshot carries the resource's identity and
  the world's tick, and `restore` refuses, with `InvalidData`, a snapshot from another
  resource or a world whose tick is not the snapshot's: restore the world first.
- Re-simulation after a restore replays the ring's frames through `Simulation::step`; each
  tick's sync sees the same rows and the same log it saw the first time, in the same
  order, and the backend starts from the same bytes, so the physics world and the rows
  stay consistent and the re-simulated ticks hash equal (`conformance::snapshot_restore`
  proves it). A correction `lactose-predict` writes into a row after the restore is a row
  write like any other: the sync pushes it to the backend at the next physics sync point,
  and Rapier's internal solver state is not corrected (architecture section 9.4), so a
  corrected dynamic body converges over a few ticks.
- `PhysicsSnapshot::size_in_bytes` is what milestone 1 measures against the 100 µs
  prediction-world budget, and `hash` is the named place where section 8.3 adds the bytes
  to the world hash once the backend's serialisation is shown to be order-stable
  (`docs/verification.md`); until then SOUR records nothing of it.

**The backend's obligations** (architecture section 8.2; the `PhysicsBackend` docs restate
them per method)

- Inside the boundary: `#![forbid(unsafe_code)]` except what section 21 grants it (nothing;
  its pool's threads enter `ThreadClass::PhysicsSolver` through `lactose_core`), the
  boundary `clippy.toml`, every float through its pinned math crates, no clock, no thread
  or worker identity, no thread-local or global state, no iteration over an unordered map
  that reaches a result.
- For one sequence of calls with equal arguments it gives equal `StepOutput`s as sets,
  equal query answers and equal `snapshot` bytes on x86-64 and aarch64, under any thread
  count of its pool and whatever ran on those threads before. The thread count may change
  how long a step takes and nothing else. Rapier's determinism feature with its parallel
  solver is how `physics-rapier` meets this, and `docs/verification.md` re-checks the
  claim on the pinned version.
- It converts what reaches a row, an event or a query answer to `Scalar` exactly, and a
  NaN it produces is canonicalised by the row write; it never produces one in a persistent
  column deliberately.
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
  and `Collider`. Until then static scenery is primitives.
- Several colliders on one body: child rows carrying `Collider` and a reference to the
  body, when a shape needs it. One collider per entity today.
- Continuous collision detection for fast projectiles: a `RigidBody` field with a schema
  bump, when the netcode spike's thrown objects show tunnelling.
- Proxies as kinematic bodies in the prediction world, so a predicted body can stand on a
  replicated platform: architecture section 9.4 says predicted entities collide with
  static geometry and other predicted entities only; if that changes, the sync's filter
  ("Bodies") is the one place that changes.
- Lag compensation's history of transforms and colliders (architecture section 9.5):
  `net`'s, over the `Transform` and `Collider` rows, milestone 4.
- Hashing the snapshot bytes into the world hash: `PhysicsSnapshot::hash` is the place;
  the verification item decides when.
- A fallback for the write-back if `Commands::set` per moved body exceeds the physics
  budget: an exclusive system at the `Stage::Post` sync point writing through
  `World::columns_mut`, with the same surface. Measured in milestone 1.

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves, under SOUR
in workers, repeat, layout and chaos modes, against a backend (the backend lane's Rapier,
or a test backend that integrates velocities and reports every body):

| What | How |
|---|---|
| Bodies follow rows | Spawn, `cmd.set` of `RigidBody` and `Collider`, their removal, despawn, region move, region load and unload: `create_body`, `set_body`, `set_collider`, `remove_body` and `set_frozen` in the stated order, handles present exactly meanwhile; the same under every layout |
| The prediction filter | In a `Prediction` world a `RigidBody` row is a body only with `ClaimedBy`; a `Collider` row always |
| Row writes | A `Mut<Velocity>` write and an impulse in one tick: the backend sees `set_velocity` then the command; a `Transform` write to a kinematic body is `set_transform`; the step's own write-back is never pushed back |
| Commands | Order equals command order under every layout; a command in `Stage::Post` never reaches a step; a command for a non-body is dropped and logged |
| Events | Emitted sorted whatever order the backend reports; a reader in `Stage::Post` sees each once; readers before the step fail to build |
| Queries | `cast_ray` ties go to the lowest key; `overlap` ascending; a `Stage::Physics` reader without an edge after `sets::STEP` fails to build |
| Joints | A joint spawned before its bodies waits and is created when they exist; a body's removal removes it and it waits again; a `Joint` value change recreates it |
| Snapshot and restore | `conformance::snapshot_restore` with the test backend; a restore of the world alone rebuilds from rows and keeps running; a snapshot from another resource is refused |
| The scalar | The size assertions in `lib.rs`, and the schema dump of every component contains no `f32` or `f64` |
| The write-back cost | `Commands::set` for 2,000 moved bodies per tick measured within the 3 ms physics phase, or the `columns_mut` fallback adopted |
