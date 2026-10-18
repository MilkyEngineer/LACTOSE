# Decision: no channel survives its run, and the schedule places every channel write and take (D124 follow-up, S1)

Status: accepted by the owner as D126 (2026-10-18), who chose option (f) after design review
rounds 1 to 5 (`review-d126-r1.md` to `review-d126-r5.md`, READY at round 5) and asked that the
structural check, deferred by rounds 1 to 5 as the next decision, be part of this one: one
decision entry, one set of amendments, one lane. Revised after review round 6
(`review-d126-r6.md`, NOT READY on F1: a trusted take declaration beside a public
`World::take_channel`), which this revision closes with a take parameter, `ChannelTake<C>`,
together with its F2 to F4 and its nits; and after review round 7 (READY), whose minors
M1 to M3 and nits are taken here. Written against main 27df9b3, which carries the D124
merge (52d8dea), the predict merge (c99727b) and, since then, no change to the crates this
decision touches; the amendment anchors checked against 27df9b3: each matches exactly once
(whitespace-normalised). The amendments below are applied by the merge's doc-edit script;
until then the sentences they replace stand in the tree.

## The defect

A `PhysicsCommand` recorded in `Stage::Physics` after the step or in `Stage::Post` lands in
the channel at the sync point that ends its stage (`crates/ecs/src/run.rs:132`, the
buffers of `run.rs:471`, the tick's closing apply at `run.rs:154`), after the physics sync
has taken the channel for that tick. It waits there for the next tick's sync, because
`PhysicsCommands` declares `CLEAR_AT_INPUT = false` (`crates/physics-api/src/command.rs:23`)
and the input sync point keeps such a channel (`crates/ecs/src/apply.rs:225`). A
`ChannelEntry` carries no tick.

A save taken between those ticks copies rows (architecture section 15.3). A fresh process
loading it never has the command. `sim`'s one-frame in-place reload (every region unloaded
and loaded at the next tick's input sync point, `crates/sim/README.md`, "An unload") keeps
the world and so keeps the channel; the sync of that tick empties and rebuilds the solver
under D124, then takes the channel and hands the old command to the step
(`crates/physics-api/src/sync.rs:746-750`), which applies it to the recreated body. The two
worlds differ from the first step: probe p2 (fake backend, `Step { commands: [ApplyImpulse] }`
against `Step { commands: [] }`) and r3 (Rapier, first difference at t0+1 for t0 = 20 and
45). That contradicts `crates/physics-api/README.md`'s "a load and a replay drop them" and
the bit equality D124 and section 15.5's first test claim. The two-frame reload drops the
command only because the unload tick's step finds no body for it.

The sync cannot tell the stale entries from this tick's: at its sync point the channel holds
last tick's post-step entries followed by this tick's input- and simulation-stage entries,
in one list, unstamped. And the schedule cannot see the write: `cmd.physics()` is an
extension method on `Commands` (`PhysicsExt for Commands`,
`crates/physics-api/src/command.rs:159-172`, over `Commands::channel`,
`crates/ecs/src/command.rs:296`), whose access entry is `AccessKind::Commands` with no
target, so a kernel in any stage compiles and runs with it.

Where the tree stands. The only channel that opts out of clearing is `PhysicsCommands`; the
other `CLEAR_AT_INPUT = false` is a test channel, `Deferred` in
`crates/ecs/tests/events.rs:169`. `physics-api` is the only crate that calls
`World::take_channel` or `World::channel`; the ECS's own tests read a channel without taking
it, through the `Channel<C>` parameter (`game_sim::physics(impulses: Channel<Physics>, ..)`
in `crates/ecs/tests/common/mod.rs:320`, run by `game_schedule` from `systems.rs` and
`claims.rs`; `count_impulses` and `read_impulses_later` in `events.rs:130-157`), and two of
its schedules record into a channel nothing reads or takes (`tests/layout.rs:55-66` and
`tests/snapshot.rs:36-48`, both with `THROW_BOMB`). `World::channel`
(`crates/ecs/src/world.rs:1313`) exists for the physics consumer, which copies the untaken
entries into `PhysicsSnapshot` (`crates/physics-api/src/service.rs:367-368`) because a
restore drops every channel (`crates/ecs/src/snapshot.rs:134`). So today a rollback keeps
the untaken commands, a reload keeps them, a save and a journal checkpoint lose them, and
the world hash never sees them. D119 chose this (`docs/decisions-log.md:1579-1589`, "The
physics channel is not cleared at the input sync point", which "Rules out a game-side
carrier component for contact-driven commands"). The tree records post-step commands in more
places than the tests that name them: the harness's `act()` records a placeholder impulse
for every `Action::Spawn` (`crates/physics-api/tests/common/mod.rs:168-174`), used in
`When::Post` at `bodies.rs:279`, `snapshot.rs:137, 239, 246, 473, 483` and
`sour.rs:200, 227`; `tests/consumers.rs:80-97` (`sketch::impact_damage`) records an impulse
on the other body from an `Impact` in `Stage::Post`; the conformance scene's `react` records
one at `src/conformance.rs:418`. `lactose-predict`, merged, names no channel API.

## The options

Each fixes section 15.5's first test (reload equals fresh) unless it says otherwise. Only
the last two keep its fourth test (fresh against the un-reloaded original, "so a reload is
shown not to change gameplay outcomes") from being broken by a command untaken at the save;
under the others the un-reloaded original applies the impulse and the other two worlds never
do, a whole outcome, not D124's tolerance.

- (a) `physics-api` adds an exclusive `Stage::Input` mover that moves the channel into the
  sync's `pending_commands`, and `empty()` clears `pending_commands`. Costs, as the lane
  found them: it cannot precede `sim`'s input systems (`crates/sim/src/schedule.rs:251`,
  `Cycle`); it needs a public set every app world with input orders against
  `input::sets::APPLY` (`UnorderedConflict`, `crates/sim/tests/runs.rs:268`);
  `lactose_predict::EXCLUSIVE_SYSTEMS` grows to ten; the test support and the conformance
  scene re-order. A second source of pending commands, for one consumer. Fourth test still
  conditional.
- (b) Narrow D124, the README and the first test to "no command untaken at the save". A
  save happens when the player saves; the game cannot keep a contact reaction out of that
  tick. A conditional guarantee is none.
- (c) `sim` clears every non-clearing channel when a frame unloads every loaded region. The
  trigger is wrong: a frame that reloads the regions holding the bodies while a region
  without bodies stays loaded empties the solver (the sync's trigger is "every body it held
  despawned") and clears nothing, so the stale commands reach the rebuilt bodies, and a
  one-region reload keeps them too. The trigger that is right is the sync's own, which
  `sim` cannot see. It also puts a rule about a physics consequence in `sim`, and needs an
  ECS method for it. Fourth test still conditional.
- (d) The ECS stamps each `ChannelEntry` with the tick it was applied at, and the sync drops
  entries older than the current tick when it empties. One field, the sync's own trigger,
  no new system. It keeps the opt-out and the hole it makes: the channel's contents between
  ticks are simulation state that no save, hash or checkpoint sees, so two worlds equal by
  `world_hash` can step differently, and the fourth test stays conditional.
- (e) The save carries the untaken commands, through a `SavePoint::services` map like the
  journal's (D124). The reload path is then already right. It adds a non-row section to the
  save with plumbing per channel through `sim`, `app` and `persist`, keeps the hash hole,
  and makes reload equality depend on `sim` never clearing channels while a restore drops
  them.
- (f) Nothing but rows carries simulation state across a tick boundary: no channel
  survives the run that filled it, the opt-out goes, and a reaction to the step that acts
  next tick is a row. D119 ruled out the carrier component this needs.

## The decision

Option (f), with the write made structural. The closing sync point of a run clears every
channel after it applies the last stage's commands, and the input sync point asserts that
every channel is empty. `CommandChannel::CLEAR_AT_INPUT` and `World::channel` are removed
from `lactose-ecs`; `PhysicsCommands` is an ordinary channel; the physics sync takes the
channel alone and `PhysicsSnapshot` carries no commands. A channel is written through a
`ChannelWriter<C>` system parameter, never through `Commands`, and taken through a
`ChannelTake<C>` parameter of an exclusive system, never through a `World` method; the
schedule refuses a writer of `C` in the stage of a taker of `C` or a later one, and a reader
or a second taker after the taker, so a command recorded after the step is a build error,
not a dropped entry. A reaction to this tick's step that acts next tick is a persistent
component holding the decision, written by the reacting kernel into its own row, which a
`Stage::Simulation` kernel of the next tick turns into the command, through an event when
the command acts on another body.

Why this is the clean interface. The architecture already says it three times: everything is
data (section 3); command buffers are empty at snapshot points by construction (section 10);
a save is persistent columns (section 15.3). The non-clearing channel was the one exception,
taken by D119 for the convenience of `cmd.physics()` from a post-stage kernel, and the
exception is the whole defect. State outside rows is unsaved, so the fresh process lacks it;
unhashed, so an equal hash does not mean an equal next step; dropped by a restore but kept
by a reload, so `physics-api` had to carry it itself for one path and could not for the
other. Removing the exception needs no new system, set, edge, stamp or save section, and
deletes code in three crates: the flag, `World::channel`, `Commands::channel` and
`World::take_channel` in the ECS, `pending_commands` and the channel copy in the snapshot in
`physics-api`, the assertion in `lib.rs:102`. Making the write a parameter puts the channel
under the rule every other cross-system effect already obeys, that the schedule sees what a
system touches and orders it: an event reader is refused unless ordered after every emitter
(`crates/ecs/src/schedule.rs:661-680`, `ScheduleError::EventsBeforeEmitter`), and a channel
writer is now refused unless ordered before every taker. Section 15.5's first test then
holds with no condition, the two-frame path included, and its fourth is no longer broken by
a lost command, since nothing is in flight at any tick boundary.

Why the clear is at the closing sync point and not at the input sync point. Entries recorded
in `Stage::Physics` and `Stage::Post` are moved into the world's channel store by the tick's
closing apply (`run.rs:154`) and would sit there until the next input sync point, through
every save, snapshot and pass run between ticks (review probe q4: two entries between
ticks). They influence no step either way, but "empty between ticks" must be true, not
nearly true. Clearing after the closing apply makes it true by construction: a tick and a
pass are both `execute` (`run.rs:117-160`), the closing apply is a run's last write, and no
API touches a channel outside a run (`World` has no channel method; `World::apply` is
crate-private and called from `execute` alone). No pass consumes a channel on main, so a
pass loses nothing. A corollary of the structural check: the closing apply itself never
carries a channel entry, since a writer in a run's last stage has no later consumer and is
refused at build; what the closing clear removes is what a channel with readers alone holds,
applied at earlier sync points, or what a skipped taker left. The input sync point therefore
clears nothing: it keeps a debug assertion that every channel is empty, which documents the
construction and exposes a future write path outside a run, where a second clear would hide
it.

The structural check, exactly. The surface, so a lane implements it without an architect
pass:

- `lactose-ecs`, `AccessKind` gains `WriteChannel` and `TakeChannel`, both on
  `AccessTarget::Channel { type_id, name }`. `WriteChannel` conflicts with nothing, since its
  entries reach the store at a sync point, as `Commands` does; `TakeChannel` conflicts with
  nothing of its own, since only an exclusive system holds it, which conflicts on
  `AccessTarget::World` with every other exclusive system of its stage and runs before
  every other system of the stage.
- `impl<C: CommandChannel> SystemParam for ChannelWriter<'_, C>` with
  `ACCESS = Access::new(AccessKind::WriteChannel, channel_target::<C>)`, allowed in every
  kind of system and in a finalize, like `EventWriter`; its `Fetch` takes
  `Slot::WriteChannel(&'r mut Vec<ChannelCommand>)`. `ChannelWriter<'a, C>` keeps its one
  method, `pub fn push(&mut self, entry: C::Entry)`, and holds the sink instead of the
  buffer. `Output` (`run.rs:164-176`) gains one sink per parameter, as it has for
  `EventWriter` and `PresentEvents`; inside each invocation's block, after the kernel or the
  finalize returns and before the `Output::is_empty` check that gates a per-entity or
  per-region output (`run.rs:567`, `715`), each sink is appended to `Output::commands` as
  `Command::Channel` entries in parameter order, after every command the invocation's
  `Commands` parameter recorded, so a kernel whose only output is a channel entry keeps it
  and a placeholder a writer names resolves at apply as before (the spawn is earlier in the
  same buffer). Two `ChannelWriter<C>` parameters of one system are accepted; their entries
  append in parameter order. `Commands::channel` is removed.
- `pub struct ChannelTake<C: CommandChannel>`, the take, a parameter of an exclusive system
  alone, owning the entries: `impl<C: CommandChannel> SystemParam for ChannelTake<C>` with
  `ACCESS = Access::new(AccessKind::TakeChannel, channel_target::<C>)`, and
  `iter(&self) -> ChannelEntries<'_, C>`, `len`, `is_empty` and
  `into_entries(self) -> Vec<ChannelEntry<C>>`, the entries applied in the run's earlier
  stages, in command order with keys resolved, as `Channel<C>` yields them. Its fetch is
  the run's: `run_exclusive`'s pre-loop (`run.rs:440-456`), which moves the stores a
  `Channel<C>` parameter reads out of the world before lending it and puts them back
  afterwards, removes the store of every `TakeChannel` entry for good, by its `TypeId` as
  the `Read` arm removes a read one, and hands it to
  `Slot::TakeChannel(Option<Box<dyn Any + Send + Sync>>)`, which the `Fetch` downcasts to
  `Vec<ChannelEntry<C>>`, empty when no store exists. `World::take_channel` is removed with
  `World::channel`. A take therefore cannot be forgotten or made outside a run, and is
  skipped only with its system, when a resource it names was removed by an earlier
  exclusive system (`run_system`'s skip branch, `run.rs:346-357`, logged at `error` at
  target `casein`), whose line gains the skipped table's `TakeChannel` targets so the lost
  channel is named: the parameter exists, so the store is gone before the system's body
  runs. `check_table` refuses `ChannelTake<C>` outside an exclusive system (in a finalize
  too), twice on one channel, beside `Channel<C>` on one channel, which would read what the
  take removed (review probe r6a), and beside `ChannelWriter<C>` on one channel, whose
  entries would land after the take, each as
  `ScheduleError::InvalidParameter { system, parameter: "ChannelTake" }`, the variant that
  already names a parameter a system cannot hold as declared.
- `Plan::check_channels`, called after `check_events` at build (`schedule.rs:273`), over
  every system's tables (kernel and finalize, as `tables` chains them). For every
  `WriteChannel` entry: a `TakeChannel` entry on the same channel in a system whose stage
  is the writer's or an earlier one is
  `ScheduleError::ChannelAfterTaker { system, taker, channel }` (the two system names and
  the channel's type name), by stage index, not `Plan::before`, since an exclusive writer
  ordered before the exclusive taker in one stage is refused too: its `ChannelWriter`
  entries apply with its commands at the sync point that ends the stage, not at the
  system. For every `TakeChannel` entry: a `Read` or a `TakeChannel` on the same channel in
  a system placed after the taker, which is `Plan::before(taker, other)` (a later stage;
  the taker's stage unless exclusive and ordered before it), is `ChannelAfterTaker` too,
  naming that system, since a reader or a second taker there finds the channel empty on
  every run (review probe r6b's post-stage reader; on main a post-stage reader of the
  physics channel saw entries). The variant displays as "`{system}` uses channel
  `{channel}` after its taker `{taker}`: a writer's entries would land after the take, and
  a reader or a second taker would find it empty; place it before the taker". A writer
  with neither a `TakeChannel` nor a `Read` entry on its channel in a later stage is
  `ScheduleError::ChannelWithoutConsumer { writer, channel }`, "`{writer}` writes channel
  `{channel}`, which no system of a later stage takes or reads".
- `lactose-physics-api`: `PhysicsExt` becomes the six recording methods,
  `apply_impulse(&mut self, body: StableKey, impulse: Vec3)`, `apply_impulse_at(.., point: Vec3)`,
  `apply_torque_impulse(.., torque: Vec3)`, `apply_force(.., force: Vec3)`,
  `apply_torque(.., torque: Vec3)` and `teleport(&mut self, body: StableKey, transform: Transform)`,
  sealed and implemented for `ChannelWriter<'_, PhysicsCommands>` alone; `push` is the
  writer's own. `PhysicsWriter` and `PhysicsExt::physics` go, with no alias for the removed
  name; the crate exports `PhysicsCommand`, `PhysicsCommands` and `PhysicsExt`. A kernel
  declares `physics: ChannelWriter<PhysicsCommands>` and writes
  `physics.apply_impulse(bomb, ..)` with the trait in scope. `physics_api::sync` takes
  `commands: ChannelTake<PhysicsCommands>` beside its `&mut World` and `ResMut<Physics>`
  and hands the entries to the sync.

What the closing sync point checks: nothing. A take is a parameter the run fetches, so the
one thing the earlier rounds' guard watched for, a taker that did not take, cannot happen;
the one way a taker's channel stays untaken is the taker being skipped with its system, for
a resource an earlier exclusive system removed, which the skip logs at `error` at target
`casein`, naming the system and the channels its table takes. After the closing apply the
closing sync point clears every channel and logs nothing: a channel with `Channel<C>`
readers alone holds at the close what its readers saw, and a taker's channel is empty by
construction. The per-run set of taken channel types, the plan's taker set, the `warn`, the
debug assertion and the subprocess test of rounds 1 to 6 go. The input sync point keeps its
debug assertion that every channel is empty, the one runtime check of this decision.

What the game does instead. Three rules bound the recipe. A kernel reads its own row only:
its parameters are `&T`, `Mut`, `Option<..>`, `With`, `Without`, `Changed`, `Res`,
`RegionRes`, `Ctx`, `StableKeyOf`, `Commands`, `ChannelWriter`, `EventWriter`, `Events`,
`Channel` and `PresentEvents` (`Job` is refused in a kernel, `ScheduleError::JobInKernel`),
and another row is reachable through `&mut World` or `Query` in an exclusive system alone. A
game runs no exclusive system in the prediction world (`lactose-predict`'s build check,
D121; `crates/predict/README.md`, "The build check"). In the prediction world a per-entity
kernel runs only on claimed rows (the world's own filter, `ClaimedBy`;
`crates/ecs/README.md`, "Per-entity"), so a relay's receiving kernel must run on a claimed
row for its command to be predicted: a command recorded by an unclaimed row is the server's
alone, and the client receives its result as a correction. So a reaction that acts on
another body is a relay through an event, which is how this ECS already carries every
cross-entity effect (section 6.1's `sword_hit` and `apply_damage`), in the simulation stage
of the next tick, and the kernel that records a command on the player's body is the body's
own.

A `ContactEvent` goes to its target, so the post-stage kernel runs on the target's row and
writes a persistent component on it. The component holds the decision in integer or
discrete form, a reaction kind, a count, an `armed` flag, the other body's key, never a
solver number. A carrier of plain values is written and cleared through `Mut`: no command,
no archetype change, no allocation. A carrier that keeps a key holds it in an `EntityRef`
field, the architecture's typed reference (section 7), and the schedule refuses `Mut` on
such a component (`ScheduleError::WritesMaintainedComponent`; `refuses_mut` in
`crates/ecs/src/world.rs:431-437`): it is written and cleared through `Commands::set` on its
own row, which applies at the sync point that ends the stage, and the column is spawned with
the body so the set changes no archetype. The next tick's simulation kernel on that row
emits an event keyed by each body the reaction acts on, carrying its own row's values, and
clears the component; that body's simulation kernel, ordered after the emitter, computes
the command from its own row and the event and records it through its
`ChannelWriter<PhysicsCommands>`. Both kernels are in `Stage::Simulation`, so the commands
are in that tick's channel when the sync takes it, and land at the step a post-stage
command used to reach. The floats are never stored. The carrier is `persistent`, because a
transient column is zeroed at the input sync point before the simulation kernel runs, and
`replicated` when those kernels run in the prediction world, as D121's build check requires
of a persistent component; it is then compared by canonical bytes (D124;
`crates/predict/README.md`, "Reconciliation"), which is why it holds no solver-derived
scalar: the client's contact normal comes from a solver that reached its state by another
path, so a stored `contact.normal * 2` would differ from the server's at the end of every
reaction and be corrected, a rollback the post-stage command, never compared, did not
cause.

The delayed explosion is the model. The `Impact` arms the bomb's row in `Stage::Post`
through `Mut<Bomb>`. The next tick's simulation kernel on the bomb runs the overlap query
(`Res<Physics>` in `Stage::Simulation` answers from the previous step's state, as
`sketch::ground_check` does, and `overlap` returns keys only), emits `Blast { centre,
strength }` from its own `Transform` to each key found, and disarms the row; each body's
simulation kernel, ordered after it, reads `Events<Blast>::for_target(*key)`, takes the
direction from its own `Transform` and records `physics.apply_impulse`. Pushing the other
body of a contact, the tree's own example (`sketch::impact_damage` pushes `contact.other`,
which an `Impact` reaching only a collider with `events` set usually leaves without an
event of its own): the carrier keeps `contact.other` in an `EntityRef` field, set through
`Commands::set`; the next tick's kernel emits `Push { from, strength }` to it and clears the
carrier the same way, and the pushed body's kernel computes and records; the pushed body
carries no column. A teleport decided in `Stage::Post` is a carrier too, and the body that
is teleported records it: the character's carrier keeps the portal's key
(`PendingTeleport { portal: EntityRef }`, through `Commands::set`), and the next tick's
simulation kernel on the character reads the exit's pose from data its own kernel can
reach, a world resource a setup function fills from the level's cooked data, keyed by each
portal's cooked `NetId` (section 7), constant for the session (`Res<Portals>`, `derived`,
which the build check does not judge; a portal spawned or moved at runtime is outside the
recipe and its teleport reaches the client as a correction, since a restore returns a
reflected world resource to its snapshot's value, review probe r4c), or a fixed destination
of its own row, records `physics.teleport(*key, pose)` and clears the carrier. The exit
portal's kernel never records it: the exit is no player's claimed row, so its kernel runs
on the server alone, and every teleport of the player's character would reach the client
as a correction (review probe r3c). A `Transform` written through `Commands::set` is a
placement only for a dynamic or fixed body; for a kinematic body it is a sweep
(`crates/physics-rapier/src/backend.rs:549-560`, `set_next_kinematic_position`;
`PhysicsCommand::Teleport`'s doc: "a `Transform` row write sweeps a kinematic body
instead"), and D124's spike runs the character as a kinematic body. Review probe s2: a
kinematic cube swept 6 m through a touching crate sent the crate 18 m at 366 m/s, where
the `Teleport` command left it in place.

A `Commands::set` of a carrier on another row is not the recipe: it is last writer wins by
command order, so two reactions on one body in a tick keep one where impulses summed, and
it changes the archetype where the column is absent. The cost of the pattern is honest: the
reacting row carries a replicated column, written and cleared per reaction, two row
changes, through `Mut` for a plain-value carrier and through two `Commands::set` for a
keyed one; a body acted on needs a reader kernel and an event type, no column; the relay
adds an event type and an edge.

What it rules out. A channel that survives the run that filled it, for any crate; tick
stamps on entries; an input-stage mover and its set; a `sim` rule about channels; a channel
section in the save; a channel write through `Commands`, from any stage; a public
`World::take_channel`; a trusted declaration on either side, `channels = [C]` on the writer
or `takes = [C]` on the taker, since each guards honest authors only: an undeclared
`cmd.physics()` or `take_channel` would still compile and run (review probe r6b); a predict
tolerance for carrier columns (a carrier holds no float that needs one, and a tolerance is
a predict interface change that is not smaller); a release-only test of a command recorded
after the step, since no schedule can hold one.

What would change it. A deferred physics command that cannot be a row. None exists in the
tree, and the reference game's reactions (knockback, explosion, portal) are a carrier
holding a decision and a relay. If one appears, the amendment is an engine-provided
persistent carrier column consumed by a `physics_api` simulation-stage system, which keeps
the state in rows; never a channel that survives the sync point.

Who owns what. The ECS owns the rule and the check: a run's closing sync point clears every
channel and the input sync point asserts them empty; a channel write and a channel take are
parameters, and the schedule places them. `physics-api` owns its consequence: the sync
takes the channel through its parameter alone, carries no commands, and puts its recording
methods on the writer. The game owns deferred intent, as rows holding decisions and events
carrying them. `sim` changes two sentences of its README and one doc comment;
`ecs-macros`, `predict`, `net-api` and `input` change nothing; `EXCLUSIVE_SYSTEMS` stays
at nine.

## Lanes

The D124 lane merged first (main 52d8dea), with its gap recorded in the sentence beginning
"Commands untaken at the save" that its M1 merge text carries and in the D124 addendum's
"Open: ... (S1)". Its B1 fix is what `Predictor::resync` needs, and an ECS surface change
does not belong in a physics fix round (section 20, one crate per task).

Then one lane, S1, on main, one branch over `ecs`, `physics-api` and `physics-rapier`, with
one doc comment in `sim`, called out as an interface amendment under section 20 ("Amending
an interface in the same PR as the implementation that needs it is permitted and must be
called out"). Not an ordered pair of branches: the ECS removals (`Commands::channel`,
`CLEAR_AT_INPUT`, `World::channel`, `World::take_channel`) break `physics-api` on main, and
the closing clear breaks its post-step tests, so an ECS-only merge leaves the workspace red
unless it keeps a shim the physics branch then deletes. The branch has three commits, each
of which builds and passes: first the ECS additions (the two access kinds, the
`ChannelWriter` parameter and its sinks, `ChannelTake`, `check_channels`, the ECS tests on
the new parameters with the readers `ChannelWithoutConsumer` requires), removing nothing, so
`physics-api` still compiles against `Commands::channel` and `take_channel` and records no
`WriteChannel` entry the check could refuse (`Commands::channel` builds the same sink-backed
writer through a transitional channel tail on `CommandBuffer`, a private
`Vec<ChannelCommand>` the buffer lends as the sink and the run appends to the buffer's
commands with the parameter sinks, so the writer's one field is final from this commit);
second the `physics-api` and `physics-rapier` switch (the writer parameter and `PhysicsExt`
on it, `ChannelTake` in the sync, the harness split, the sketches, the conformance scene and
every moved post-step script), after which the check is live for the physics channel and
every post-step writer is gone; third the removals and the closing clear across `ecs` and
`physics-api` (`Commands::channel`, `CLEAR_AT_INPUT`, `StoredChannel::clear_at_input`,
`World::channel`, `World::take_channel`, the channel tail, the closing clear and the
input-sync assertion, `pending_commands`, the snapshot copy and `lib.rs:102`, the ECS tests'
takes between runs, `push_late` with the two `Deferred` tests), with the `sim` doc comment.
No facade pass: every signature, error and message is given above, and each part has a model
in the tree (`EventWriter` for the writer and its sink, `Channel<C>` and the exclusive
pre-loop for the take, `check_events` for the check). `ecs-macros` is untouched, since the
access table is generic over the parameter types (`expand.rs:200-210`). Line numbers are
main c99727b's; 024b28f and 27df9b3 changed none of these crates. Items:

1. `lactose-ecs`: the two access kinds and their docs; `ChannelWriter` as a parameter
   (`SystemParam`, `Fetch`, `Slot::WriteChannel`, the sinks in `Output`, the append inside
   each invocation's block before `Output::is_empty`, the `Env::slot` arm, `run.rs:274-304`);
   `ChannelTake<C>` (`SystemParam`, `Fetch`, `Slot::TakeChannel`, the removal in
   `run_exclusive`'s pre-loop, the `check_table` refusals); `check_channels` with
   `ChannelAfterTaker` and `ChannelWithoutConsumer`, their `Display` texts above; after the
   closing apply of every run (`run.rs:154`, ticks and passes alike) every channel cleared;
   `input_sync` (`apply.rs:222-225`) clears no channel and asserts in debug that every
   channel is empty; remove `Commands::channel`, `CommandChannel::CLEAR_AT_INPUT`,
   `StoredChannel::clear_at_input`, `World::channel` and `World::take_channel` with its
   doc (`world.rs:1299-1306`), and the channel tail of the first commit; the skip branch
   of `run_system` (`run.rs:346-357`) adds the skipped table's `TakeChannel` targets to its
   `error` line; the docs of `CommandChannel`, `ChannelWriter`, `Channel`, `input_sync`,
   `AccessKind`, the `SystemParam` table and its
   `on_unimplemented` note (`param.rs:26-65`: `ChannelWriter<C>` beside `Commands` as an
   output buffer of every kind, `ChannelTake<C>` exclusive-only, the channel taken), the
   `InvalidParameter` doc (`schedule.rs:1000-1010`, `ChannelTake` added to its list),
   `WorldSnapshot` (`snapshot.rs:17-21`, which names `World::channel` and a channel "not
   cleared at the input sync point") and the restore comment (`snapshot.rs:131-133`),
   `ScheduleBuilder::stage` (`schedule.rs:96-102`, "events, channels and input ticks are
   cleared" becomes "events and input ticks are cleared") and `run_pass`
   (`schedule.rs:881-885`: "nothing is zeroed or cleared" becomes "nothing is zeroed and no
   event is cleared at the first sync point; the closing sync point clears every channel,
   as every run's does"); the README amendments below. Tests: `schedule_errors.rs` adds
   `ChannelAfterTaker` for a writer in the taker's stage, in a later stage and an exclusive
   writer ordered before the taker in its stage, for a `Channel<C>` reader in a later stage
   and a non-exclusive one in the taker's stage, and for a second `ChannelTake<C>` in a
   later stage (r6b's writer and reader); `ChannelWithoutConsumer` for a writer with no
   taker and no reader and for one whose only reader is in an earlier stage;
   `InvalidParameter` for `ChannelTake<C>` in a per-entity, a once-per-tick and a finalize
   table, twice on one channel, beside `Channel<C>` on one channel (r6a) and beside
   `ChannelWriter<C>` on one channel; a per-entity
   kernel and a per-region kernel whose only output is a channel entry, read by a later
   stage; a `ChannelTake<C>` yielding the entries of the earlier stages with keys resolved
   and nothing in the next run; `tests/common/mod.rs`'s test `PhysicsCommands` trait
   (`mod.rs:225-233`, `cmd.apply_impulse` on `Commands`) becomes the same method on
   `ChannelWriter<'_, Physics>`, and every recording system of the tests takes the
   parameter in the first commit (`THROW_BOMB` among them) except `push_late`
   (`events.rs:180-186`), a last-stage writer `ChannelWithoutConsumer` would refuse, which
   stays on `Commands::channel` until the third commit deletes it; `section_6_1.rs` and
   `control.rs` with the amended example; `tests/events.rs`, the third commit deletes the
   two `Deferred` tests with `push_late` and writes the closing-clear test: a writer in a
   middle stage, a `Channel<C>` reader in a later stage that sees the entry, and an
   observer `Channel<C>` in the writer's stage or earlier that sees nothing on the next
   run, with the channel absent from a snapshot taken between the runs; no test takes a
   channel between runs any more (`events.rs:153` and `242` and `systems.rs:43`: each take
   goes, `systems.rs` keeping its `ImpulseCount` assertion, which already shows the
   physics stage read it); `game_schedule`
   (`tests/common/mod.rs:320-338`, used by `systems.rs` and `claims.rs`) and
   `events.rs:130-157` keep passing, since their `Channel<Physics>` readers are consumers
   in a later stage than the writers; the schedules of `tests/layout.rs:55-66` and
   `tests/snapshot.rs:36-48` add a `Channel<Physics>` reader in a stage after `simulation`
   (common's `game_sim::physics`, with its `ImpulseCount` resource inserted, or a reader of
   their own), which `ChannelWithoutConsumer` now requires, and keep `THROW_BOMB`, since
   its predicted spawn and placeholder command are what the layout program and the
   snapshot test exercise. Miri gate.
2. `lactose-physics-api`: `PhysicsExt` on the writer and the exports as above;
   `sync_bodies` (`systems.rs:70-78`) gains `commands: ChannelTake<PhysicsCommands>` and
   hands it to `Physics::sync`, whose `take_commands` (`sync.rs:746-750`) takes the entries
   from the parameter instead of the world;
   `impl CommandChannel for PhysicsCommands { type Entry; fn remap }` only; remove
   `SyncState::pending_commands`, the `World::channel` copy in `snapshot`, the `restore`
   doc's "the pending commands before what it takes" (`service.rs:381`), the `sync` doc's
   "appended after the pending commands" (`systems.rs:67`) and the assertion in
   `lib.rs:102`; the docs that name `cmd.physics()` (`lib.rs:3` and `20`, `command.rs:2`,
   `10-17`, `113` and `159`, `service.rs:130-132`, whose "grows the channel without bound"
   becomes the build refusal); the README amendments below; the `consumers.rs:80-82` doc
   ("a command recorded here, which the next tick's step applies"). The harness: the
   simulation recorder (`tests/common/mod.rs:203-204`) gains
   `physics: ChannelWriter<PhysicsCommands>`; `after_step` (`231-232`) and `post_kernel`
   (`238-243`) do not, so `act()` (`168-174`) records an `Action::Command` and an
   `Action::Spawn`'s placeholder impulse in `When::Simulation` alone, panics on an
   `Action::Command` scripted at `When::AfterStep` or `When::Post` ("a command after the
   step is a schedule build error: script it in `When::Simulation`"), and spawns there
   without the impulse, so the `When::Post` spawns at `bodies.rs:279`,
   `snapshot.rs:137, 239, 246, 473, 483` and `sour.rs:200, 227` keep their spawns.
   `sketch::impact_damage` (`consumers.rs:80-97`) is rewritten as the relay above: the
   `Impact` arms the bomb's row through `Mut<Bomb>` and sets `contact.other` into an
   `EntityRef` field of a carrier spawned with the bomb, through `Commands::set`; a
   `Stage::Simulation` kernel of the next tick emits `Push` to that key and `Blast` to each
   key an overlap query returns, from the bomb's own `Transform`, disarms through `Mut` and
   clears the carrier through `Commands::set`; a `Stage::Simulation` kernel on the receiving
   bodies, ordered after it, computes the impulse from its own `Transform` and records it
   through its writer; both the bomb and a receiver are claimed in the prediction-world run;
   `sketch::throw` (`consumers.rs:42-54`) takes the parameter. The conformance scene:
   `drive` (`conformance.rs:375-400`) takes the parameter; `react` (`418`) no longer records
   a command; a new `Stage::Simulation` kernel at `SNAPSHOT_TICK + 1` records the impulse, so
   the re-simulated tick records it again from the same script; its docs at
   `conformance.rs:84-85` ("so the snapshot point holds pending log entries, pending keys
   and a pending command") and `405-407` ("and a command, all left for the next tick's sync
   and step") say so. Tests: `tests/commands.rs`
   `the_step_applies_commands_in_command_order_across_the_tick_boundary` keeps the in-tick
   order with the torque, force and teleport of `commands.rs:37-40` scripted in
   `When::Simulation` of tick 3; a new test builds a schedule with a
   `ChannelWriter<PhysicsCommands>` kernel in `Stage::Post` and one in `Stage::Physics`
   after the step and asserts the build error carries `ChannelAfterTaker` naming the
   kernel, `physics_api::sync` and `PhysicsCommands`, which is what the earlier rounds' "a
   command recorded after the step reaches no step" and p2 inverted (q1, q1b) become, since
   no schedule can hold the scenario; `tests/reload.rs`:
   `a_command_recorded_at_the_reload_tick_before_the_physics_stage_is_applied` (an impulse
   recorded in `Stage::Simulation` of the reload tick against a reloaded body is in that
   tick's step, in both worlds; review q2 without p2's post-step impulses, which its script
   can no longer hold) and q3 adopted (a carrier written in `Stage::Post` and turned into
   the command by the next tick's simulation kernels lands at the same step on the
   straight, one-frame, fresh-process, restore and two-frame paths, with equal steps and
   hashes); `tests/snapshot.rs:140-141, 243, 474, 477` and `tests/sour.rs:193, 208, 230`:
   each post-step command moves to `When::Simulation` of the next tick, so the pending-work
   coverage keeps a command of every kind;
   `the_restore_carries_the_pending_work_of_the_snapshot_point` keeps its spawn, despawn and
   move.
3. `lactose-physics-rapier`: `tests/reload.rs`: r3 inverted is void, as p2 inverted is; s2
   adopted: a kinematic cube with a dynamic crate touching its face, a `Transform` written
   through `Commands::set` in `Stage::Post` sweeps it through the crate (`Teleported`
   absent, the crate thrown), where a `Teleport` command recorded in the next tick's
   `Stage::Simulation` by the cube's own kernel from a carrier places it (`Teleported` set,
   the crate at rest); `tests/sour.rs:195, 210, 232` as item 2, the `When::Post` spawns at
   `202, 229` kept; the harness is `physics-api`'s, included by `#[path]` (`sour.rs:12-13`),
   so item 2's `act()` reaches it.
4. `lactose-sim`: the `Pass` doc comment (`src/schedule.rs:60-62`, "the tick does not
   advance and nothing is cleared") becomes "the tick does not advance, nothing is zeroed
   and no event is cleared at its first sync point, and its closing sync point clears every
   channel, as every run's does"; the README amendments below.
5. Documents: the amendments below, through the merge's script; the merger inserts the
   D126 entry in section 9 of `docs/decisions-log.md` before D127, which has ended the
   section since 024b28f, so the log reads D125, D126, D127.

CI changes nothing. Rounds 1 to 5 added the two physics crates to the `check` job's release
step and a nightly Windows release step, because a test that recorded after the step
aborted a debug run and so could run in release alone; under the build check no schedule
holds such a test, in either build, so those tests have no scenario and the release steps
have no reason. Every test of this lane runs in debug under nextest.

The lane does not write section 15.5's fourth test as hash equality with Rapier contacts:
under D126 the fresh process and the un-reloaded original still differ at t0+1 while bodies
touch (s1), by the solver's warm state, which is D124's tolerance, not this decision's.

Gates: the merge's, with Miri for `ecs`, the ui controls of `compile_fail.rs` with the
amended example, cross clippy `-p` the three crates, nextest over `ecs`, `sim`,
`physics-api`, `physics-rapier` and `predict`, and SOUR. Review: `reviewer` first, since
the change crosses three crates and the surface every game system is written against,
then `reviewer-diff`. Cost: six to eight agent runs (the lane, two runs if the first stops
at the usage limit; `reviewer`; two or three fix rounds with their `reviewer-diff` rounds,
which is what the recent multi-crate lanes needed: five rounds for predict, three for the
Rapier solver-state lane). Folding the structural check into S1 still saves the second
edit of every recording site that a later lane would have made.

## Architecture amendments (exact text)

### §6.1 System API, replace "fn throw_bomb(ctx: Ctx, cmd: Commands, key: StableKeyOf, held: &HeldItem, input: &PlayerInput) {"

fn throw_bomb(ctx: Ctx, cmd: Commands, physics: ChannelWriter<PhysicsCommands>, key: StableKeyOf, held: &HeldItem, input: &PlayerInput) {

### §6.1 System API, replace "        cmd.physics().apply_impulse(bomb, input.aim() * spread);"

        physics.apply_impulse(bomb, input.aim() * spread);   // `PhysicsExt` on the writer

### §6.1 System API, replace "Physics writes go through the physics command buffer (impulses, forces, teleports) or through"

Physics writes go through a `ChannelWriter<PhysicsCommands>` parameter (impulses, forces,
teleports, the `PhysicsExt` methods of `lactose-physics-api` on it) or through

### §6.1 System API, replace "its sync point pushes the rows' changes into the solver and takes the commands recorded since the previous step, then the step applies them; the step's results reach the rows at the sync point that begins the post stage."

its sync point pushes the rows' changes into the solver and takes the commands recorded in
the input and simulation stages of the tick, then the step applies them; the step's results
reach the rows at the sync point that begins the post stage. A `ChannelWriter<PhysicsCommands>`
in the physics stage or the post stage is a schedule build error, since the tick's closing
sync point clears every channel and a command recorded after the step would reach no step;
a reaction to this tick's step that acts next tick is a persistent component holding the
decision, which a simulation-stage kernel of the next tick turns into the command, through
an event when it acts on another body (D126).

### §6.1 System API, replace "written at the input sync point gets its declared writer."

written at the input sync point gets its declared writer. `ChannelTake<C>`, in an exclusive
system alone, takes a channel at its stage's sync point, and the schedule refuses a
`ChannelWriter<C>` in that stage or a later one and a `Channel<C>` after the take (D126).

### §10 Snapshots, replace "which carries the backend's whole world with each collider's shape value and the physics sync's pending work, so that the sync of a re-simulated tick makes the calls the first run's did."

which carries the backend's whole world with each collider's shape value and the
structural-log entries and changed keys the physics sync has not yet consumed, so that the
sync of a re-simulated tick makes the calls the first run's did.

### §10 Snapshots, replace "Command buffers are empty at snapshot points by construction."

Command buffers and channels are empty at snapshot points by construction: the closing sync
point of a run clears every channel after it applies the last stage's commands, so nothing
but rows carries simulation state across a tick boundary (D126).

## Decisions-log amendment (exact text)

### D119, replace "`cmd.physics()` carries impulses, forces and `teleport` only."

`cmd.physics()`, since D126 a `ChannelWriter<PhysicsCommands>` parameter, carries impulses,
forces and `teleport` only.

## README amendments (exact text)

### `crates/ecs/README.md`, "Surface", replace "| `Commands`, `Bundle`, `CommandChannel`, `ChannelWriter`, `Channel` | Structural commands applied at the next sync point; typed channels for other crates' commands |"

| `Commands`, `Bundle`, `CommandChannel`, `ChannelWriter`, `Channel`, `ChannelTake` | Structural commands applied at the next sync point; typed channels for other crates' commands, written through a `ChannelWriter<C>` parameter, read through `Channel<C>` and taken through `ChannelTake<C>` in an exclusive system |

### `crates/ecs/README.md`, "The `#[system]` attribute", replace "`&mut World` outside an exclusive system, `Commands`, `Job` or `&mut World` twice, or `ResMut<T>` beside another parameter on `T`,"

`&mut World` or `ChannelTake<C>` outside an exclusive system, `Commands`, `Job`,
`&mut World` or `ChannelTake<C>` of one channel twice, `ChannelTake<C>` beside `Channel<C>`
or `ChannelWriter<C>` on one channel, or `ResMut<T>` beside another parameter on `T`,

### `crates/ecs/README.md`, "Commands and events", replace the bullet beginning "Channel entries written in stage *k*" through "snapshots what it has not taken."

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

### `crates/ecs/README.md`, "Schedule", replace "a closing sync point after the last stage applies its commands, so a run ends with every buffer empty and a snapshot point never has a pending buffer."

a closing sync point after the last stage applies its commands and clears every channel, so
a run ends with every buffer and channel empty and a snapshot point never has a pending
buffer.

### `crates/ecs/README.md`, "Schedule", replace "transient columns are zeroed and events, channels and input ticks cleared before its exclusive systems,"

transient columns are zeroed and events and input ticks cleared before its exclusive
systems (every channel is already empty, which a debug assertion checks),

### `crates/ecs/SAFETY.md`, "The access tables", replace "which derives both from the parameter types and the `writes` list, is its one caller. A `writes` entry fetches nothing: it declares a write the system makes through commands or `&mut World`, outside every kernel, and only adds an ordering constraint, so it can make a table overstate what the kernel touches but never understate it."

which derives both from the parameter types and the `writes` list, is its one caller. A
`writes` entry fetches nothing: it declares a write the system makes through commands or
`&mut World`, outside every kernel, and only adds an ordering constraint, so it can make a
table overstate what the kernel touches but never understate it. A `ChannelWriter` fetches
a sink the run made for that parameter alone and appends to the invocation's buffer after
the kernel, and a `ChannelTake` fetches the store the run removed from the world before
lending it, so neither aliases what another parameter holds (D126).

### `crates/physics-api/README.md`, replace "the command channel a kernel writes through `cmd.physics()`;"

the command channel a kernel writes through a `ChannelWriter<PhysicsCommands>` parameter;

### `crates/physics-api/README.md`, "Surface", replace "| `PhysicsExt`, `PhysicsWriter`, `PhysicsCommands`, `PhysicsCommand` | `cmd.physics()`: impulses, forces and teleports, recorded into the channel and applied by the next step in command order |"

| `PhysicsExt`, `PhysicsCommands`, `PhysicsCommand` | The `PhysicsExt` methods on a `ChannelWriter<PhysicsCommands>` parameter: impulses, forces and teleports, recorded in `Stage::Input` or `Stage::Simulation` and applied by this tick's step in command order; the schedule refuses the parameter in `Stage::Physics` or `Stage::Post` |

### `crates/physics-api/README.md`, "Surface", replace "| | Impulses, forces, teleports | `cmd.physics().apply_impulse(..)` and the rest of `PhysicsWriter`, from any stage; a command recorded after the step applies at the next tick's step |"

| | Impulses, forces, teleports | `physics.apply_impulse(..)` and the rest of `PhysicsExt` on a `physics: ChannelWriter<PhysicsCommands>` parameter, in `Stage::Input` or `Stage::Simulation`; the schedule refuses the parameter in `Stage::Physics` or `Stage::Post` ("Commands" below) |

### `crates/physics-api/README.md`, "Surface", the `persist` row, replace "untaken physics commands are not saved ("Commands" below)"

no physics command is untaken between ticks ("Commands" below)

### `crates/physics-api/README.md`, "The tick", step 2, replace "`physics_api::sync` (exclusive, `&mut World`, `ResMut<Physics>`, `writes = [BodyHandle, JointHandle]`):"

`physics_api::sync` (exclusive, `&mut World`, `ResMut<Physics>`,
`ChannelTake<PhysicsCommands>`, `writes = [BodyHandle, JointHandle]`):

### `crates/physics-api/README.md`, "The tick", step 2, replace "`World::take_channel::<PhysicsCommands>()`, appended after the pending commands a restore carried, as this tick's command list."

the entries of its `ChannelTake<PhysicsCommands>` as this tick's command list.

### `crates/physics-api/README.md`, "The tick", replace "`cmd.physics().teleport` places any body there with no sweep."

`PhysicsExt::teleport` places any body there with no sweep.

### `crates/physics-api/README.md`, "The tick", replace "a command recorded in `Stage::Post` is applied by the next tick's step."

a command cannot be recorded in `Stage::Post`: the schedule refuses the writer there
("Commands").

### `crates/physics-api/README.md`, "Commands", replace the first two bullets (from "`PhysicsCommands` declares `CLEAR_AT_INPUT = false`" through "the physics snapshot carries them ("Snapshot and restore").")

- A command is recorded through a `physics: ChannelWriter<PhysicsCommands>` parameter in
  `Stage::Input` or `Stage::Simulation`, and the step applies this tick's commands in
  command order (the ECS's: execution order of the recording systems, then source key,
  then sequence). The sync takes the channel through a `ChannelTake<PhysicsCommands>`
  parameter, so a schedule with the writer on a system in `Stage::Physics` or
  `Stage::Post`, or a `Channel<PhysicsCommands>` reader after the sync, fails to build with
  `ScheduleError::ChannelAfterTaker`, naming the system, `physics_api::sync` and the
  channel: a command recorded there would land in the channel after the sync took it and
  reach no step, since the ECS clears every channel at the closing sync point of every run
  (D126). So the channel is empty between ticks, and no command is in a `SavePoint`, a
  `Journal`, a `PhysicsSnapshot` or the world hash because none is in flight there.
- A reaction to this tick's step that acts next tick is a row holding the decision, never
  the solver's numbers, and acts on another body through an event. Three rules bound it: a
  kernel reads its own row only; a game runs no exclusive system in the prediction world
  (`lactose-predict`'s build check); and in the prediction world a kernel runs only on a
  claimed row, so the kernel that records a predicted command runs on a claimed row, and a
  command an unclaimed row records is the server's alone, reaching the client as a
  correction. The shape is section 6.1's `sword_hit` and `apply_damage`, one tick later. A
  `ContactEvent` goes to its target, so a kernel reading `Events<ContactEvent>` in
  `Stage::Post` writes what it decided into a persistent component of its own row, in
  integer or discrete form (a reaction kind, a count, an `armed` flag, the other body's
  key; a transient component is zeroed at the input sync point first): a carrier of plain
  values through `Mut`, with no command and no archetype change; a carrier that keeps a key
  holds it in an `EntityRef` field, which the schedule refuses `Mut` on, and is written and
  cleared through `Commands::set` on its own row, the column spawned with the body. A
  `Stage::Simulation` kernel of the next tick emits an event keyed by each body the
  reaction acts on, carrying its own row's values, and clears the component; that body's
  `Stage::Simulation` kernel, ordered after the emitter, computes the command from its own
  row and the event and records it through its writer, so it is in this tick's channel when
  the sync takes it and lands at the step a post-stage command used to reach. The floats
  are never stored. The component is `replicated` when those kernels run in the prediction
  world, as `lactose-predict`'s build check requires of a persistent component, and it is
  compared by canonical bytes, which is why it holds no solver-derived scalar: a stored
  contact normal differs from the server's on every reaction and is corrected (D124). The
  delayed explosion is the model: the `Impact` arms the bomb's row in `Stage::Post`; the
  next tick's simulation kernel runs the overlap query, which returns keys, emits `Blast`
  from its own `Transform` to each and disarms the row, and each body's kernel takes the
  direction from its own `Transform` and records the impulse. Pushing the other body of a
  contact, which an `Impact` reaching only a collider with `events` set usually leaves
  without an event of its own: the carrier keeps `contact.other` in an `EntityRef` field
  and the next tick's kernel emits to it; the pushed body carries no column. A teleport
  decided in `Stage::Post` is a carrier too, recorded by the body that is teleported: the
  carrier keeps the portal's key, and the next tick's kernel on that body reads the exit's
  pose from a world resource a setup function fills from the level's cooked data, keyed by
  each portal's cooked `NetId` (section 7), constant for the session, or from a fixed
  destination of its own row, and records `physics.teleport` on itself (a portal spawned or
  moved at runtime is outside the recipe and its teleport reaches the client as a
  correction); the exit portal's kernel never records it, since the exit is no claimed row
  and its command would be the server's alone. A `Transform` written through
  `Commands::set` places a dynamic or fixed body and sweeps a kinematic one through
  whatever lies between. A `Commands::set` of a carrier on another row overwrites, so two
  reactions on one body in a tick keep one, and adds the column where it is absent. The
  cost is a replicated column on the reacting row, written and cleared per reaction,
  through `Mut` or through two `Commands::set`, and an event type, a kernel and an edge for
  the relay. The intent is then saved, hashed, replicated and snapshotted with the row,
  which is what a load, a replay and a rollback need.

### `crates/physics-api/README.md`, "Commands", replace "A world whose systems record `cmd.physics()` and that holds no `Physics` resource grows the channel without bound, since nothing takes it: `app` inserts one in every world that runs such systems, and a test either inserts one or records nothing."

A world that runs a system with a `ChannelWriter<PhysicsCommands>` and not
`physics_api::sync` fails to build its schedule (`ScheduleError::ChannelWithoutConsumer`),
and one that runs the sync and holds no `Physics` resource fails `run_tick` before anything
runs (`ScheduleError::MissingResource`): `app` registers the systems and inserts the
resource in every world that runs such systems, and a test does the same or records
nothing.

### `crates/physics-api/README.md`, the D124 merge's sentence, replace "Commands untaken at the save (recorded in `Stage::Physics` or `Stage::Post` of the save tick) are still applied by a one-frame in-place reload, which a fresh process loading the save does not do; the two-frame reload drops them as a side effect. Section 15.5's first test holds for physics only when no command is untaken at the save, until S1 is decided."

No command is untaken between ticks (D126, the first bullet), so section 15.5's first test
holds for physics with no condition.

### `crates/physics-api/README.md`, "Snapshot and restore", replace "the keys changed since its mark across the five columns with the columns each changed in, and the channel's entries not yet taken (`World::channel`); or, when the cursor or mark is stale, that a full diff is due."

and the keys changed since its mark across the five columns with the columns each changed
in; or, when the cursor or mark is stale, that a full diff is due. No command is carried:
the channel is empty between ticks.

### `crates/physics-api/README.md`, "Proof for milestone 1", the `Commands` row, replace "command order (execution order, source key, sequence; the physics stage's and the post stage's commands ahead of the next tick's own) under three capacities, two seeds and one and four workers;"

command order (execution order, source key, sequence) under three capacities, two seeds
and one and four workers; a `ChannelWriter<PhysicsCommands>` in `Stage::Physics` after the
step or in `Stage::Post` fails the build with `ChannelAfterTaker` naming the system, the
sync and the channel;

### `crates/sim/README.md`, "Journal replay", replace "the backend's whole world and the sync's pending work with the untaken `PhysicsCommands`, since the solver"

the backend's whole world and the sync's pending work, since the solver

### `crates/sim/README.md`, "The step", replace "the ECS zeroes transient columns and clears events, channels and input ticks, then the exclusive systems of `Stage::Input` run"

the ECS zeroes transient columns and clears events and input ticks (every channel is
already empty, cleared at the previous run's closing sync point, D126), then the exclusive
systems of `Stage::Input` run

The code docs that name `cmd.physics()`, `Commands::channel`, `World::channel`,
`World::take_channel`, the pending commands or the clearing of channels
(`crates/physics-api/src/lib.rs`, `command.rs`, `service.rs`, `systems.rs`,
`conformance.rs` and `tests/consumers.rs`; `crates/ecs/src/command.rs`, `param.rs`,
`schedule.rs`, `apply.rs`, `world.rs` and `snapshot.rs`; `crates/sim/src/schedule.rs`) are
the lane's to rewrite (items 1, 2 and 4 above); the physics-api README's `restore` bullet
carries no such phrase.

## Decision entry (exact text; number at merge)

**D126. No channel carries entries across a tick boundary: a run's closing sync point
clears every channel, the physics channel included; a channel is written through a
`ChannelWriter<C>` parameter and taken through a `ChannelTake<C>` parameter of an
exclusive system, and the schedule refuses a writer in or after its taker's stage and a
reader after it; a reaction to the step that acts next tick is a row holding the decision,
relayed by an event when it acts on another body** (§6.1, §10, `lactose-ecs`,
`lactose-physics-api`, `lactose-physics-rapier`, `lactose-sim`)
- Why: a `PhysicsCommand` recorded after the step waited in the channel for the next tick's
  sync, since D119 made `PhysicsCommands` opt out of clearing and a `ChannelEntry` carries no
  tick. A save copies rows and so lost it; `sim`'s one-frame in-place reload kept the world
  and so kept it, and the D124 sync handed it to the rebuilt solver (p2, r3: the reloaded
  world and the fresh process differ from the first step). The two-frame reload dropped it
  only because the unload tick's step found no body. The channel's contents between ticks
  were simulation state outside the rows: unsaved, unhashed, dropped by a restore, kept by
  a reload, and carried by `physics-api` in its own snapshot for the one path it could. And
  the schedule could see neither the write, an extension method on `Commands` with no
  target, so a kernel in any stage compiled and ran with it, nor the take, a `World` method
  any exclusive system could call.
  - *Every channel is cleared at the closing sync point of a run; the input sync point
    asserts them empty.* `CommandChannel::CLEAR_AT_INPUT`, `StoredChannel::clear_at_input`
    and `World::channel` are removed. Command buffers and channels are then empty at every
    tick boundary by construction (channels are written only inside runs; a clear at the
    input sync point alone left post-stage entries in the store through the saves,
    snapshots and passes between ticks, and a second clear there would hide a future write
    path the assertion exposes), so a save, a checkpoint, a snapshot and the world hash see
    the whole state that the next step reads, and sections 3, 10 and 15.3 have no
    exception. Rules out tick stamps on entries (which would fix the first test of section
    15.5 and leave the fourth conditional), an input-stage mover in `physics-api` with its
    public set, app edge and tenth entry in `lactose_predict::EXCLUSIVE_SYSTEMS`, a `sim`
    rule that clears channels when every region unloads (the wrong trigger: the sync
    empties when every body it held is despawned, which a frame that keeps a body-less
    region loaded does without unloading every region), and a channel section in the save.
  - *A channel write and a channel take are system parameters, and the schedule places
    them.* `ChannelWriter<C>` is a `SystemParam` with `AccessKind::WriteChannel` on the
    channel, allowed in every kind of system and in a finalize; its entries wait in a sink
    of the invocation, as an `EventWriter`'s do, and are appended to the invocation's buffer
    inside the invocation's block, after its `Commands` and before the empty-output check,
    so a spawn's placeholder resolves as before and a kernel whose only output is an entry
    keeps it. `ChannelTake<C>` is a `SystemParam` with `AccessKind::TakeChannel`, held by an
    exclusive system alone, fetched by `run_exclusive`'s pre-loop, which removes the store
    from the world before lending it, and yielding the entries (`iter`, `len`, `is_empty`,
    `into_entries`); `World::take_channel` is removed, so a take cannot be forgotten or made
    outside a run, and is skipped only with its system, which the skip logs at `error`
    naming the system and the channels its table takes; `check_table` refuses the parameter
    outside an exclusive system, twice on one channel, or beside `Channel<C>` or
    `ChannelWriter<C>` on one channel (`InvalidParameter`). `check_channels`, after
    `check_events`, refuses a writer of `C` in the stage of a taker of `C` or a later one,
    by stage index, since the writer's entries apply at its stage's closing sync point
    whatever orders the two systems, and a reader or a second taker of `C` placed after the
    taker, which would find it empty (`ScheduleError::ChannelAfterTaker { system, taker,
    channel }`), and a writer that no later stage takes or reads
    (`ScheduleError::ChannelWithoutConsumer { writer, channel }`). `Commands::channel` and
    `PhysicsExt for Commands` go; `PhysicsExt` is the six recording methods on
    `ChannelWriter<'_, PhysicsCommands>`, `PhysicsWriter` goes with no alias,
    `physics_api::sync` takes `ChannelTake<PhysicsCommands>`, and section 6.1's `throw_bomb`
    takes `physics: ChannelWriter<PhysicsCommands>`. A command recorded after the step is
    then a build error in every build, as an event read before its emitter is. Rules out a
    channel write through `Commands` from any stage, a public `World::take_channel`, and a
    trusted declaration on either side (`channels = [C]` on the writer, `takes = [C]` on the
    taker), which an undeclared call would compile past.
  - *The closing sync point checks nothing.* It clears every channel after the closing
    apply: a taker's channel is empty by construction, since the take is a fetch, and a
    channel with readers alone holds what its readers saw. The per-run set of taken
    channel types, the plan's taker set, the `warn`, the debug assertion and the subprocess
    abort test of the earlier rounds go. The input sync point's debug assertion that every
    channel is empty is the one runtime check.
  - *`PhysicsCommands` is an ordinary channel.* The sync takes it alone; `SyncState` loses
    `pending_commands` and `PhysicsSnapshot` carries the structural-log entries and changed
    keys only. A world reloaded in place, in one frame or two, and a fresh process loading
    the same rows take the same commands at every tick, which with D124's fresh solver is
    what section 15.5's first test needs, and its fourth test is no longer broken by a lost
    command, since nothing is in flight at the save; its remaining difference with contacts
    is the solver's warm state, D124's tolerance, so the test is not hash equality with
    Rapier. No test records after the step, since no schedule can, so every test runs in
    debug and CI changes nothing.
  - *A reaction to the step that acts next tick is a row holding the decision, never a
    solver number, relayed by an event.* A kernel reads its own row only, a game runs no
    exclusive system in the prediction world (D121), and a prediction-world kernel runs on
    claimed rows alone, so the shape is section 6.1's `sword_hit` and `apply_damage` one
    tick later, with the command recorded by a claimed row: a post-stage kernel writes a
    persistent component of its own row in integer or discrete form (a kind, a count, an
    `armed` flag through `Mut`; the other body's key in an `EntityRef` field through
    `Commands::set`, since the schedule refuses `Mut` on a reference-bearing component); a
    simulation-stage kernel of the next tick emits an event keyed by each body it acts on,
    carrying its own row's values, and clears the component; that body's simulation kernel,
    ordered after the emitter, computes the command from its own row and records it through
    its writer, in this tick's channel, at the step a post-stage command reached. The
    carrier components are `replicated` when those kernels run in the prediction world
    (D121's check) and so compared by bytes, which a stored contact normal would fail on
    every reaction (D124). The delayed explosion is the model: armed by the `Impact`, the
    overlap query's keys relayed with the bomb's position, each body's direction from its
    own row. A teleport decided after the step is a carrier too, the portal's key, recorded
    by the teleported body's own kernel from an exit pose in a world resource a setup
    function fills from the level's cooked data, keyed by each portal's cooked `NetId` and
    constant for the session, or in a fixed destination of its own row (a portal spawned or
    moved at runtime is outside the recipe, its teleport a correction on the client), never
    by the exit's kernel, which runs on the server alone: a `Transform` write places a
    dynamic or fixed body and sweeps a kinematic one through what lies between (s2: a
    kinematic cube's `Commands::set` threw a touching crate 18 m; the `Teleport` command
    left it). A command an unclaimed row records reaches the client as a correction. The
    body acted on carries no column; a `Commands::set` of a carrier on another row
    overwrites and is not the recipe. Reverses D119's "Rules out a game-side carrier
    component for contact-driven commands", whose convenience cost the three properties
    above. Rules out a predict tolerance for carrier columns. Deferred: an engine-provided
    persistent carrier column consumed by a `physics_api` simulation-stage system, if a game
    needs one; never a channel that survives the sync point.
  - *The D124 lane merged first with the gap recorded; one lane follows, one branch over
    `ecs`, `physics-api` and `physics-rapier` with a doc comment in `sim`, called out as an
    interface amendment under section 20, in three commits that each build: the ECS
    additions, the physics switch, the removals with the closing clear; the entry goes into
    section 9 before D127.* The B1 fix is what `Predictor::resync` needs and an ECS surface
    change is not a physics fix round; an ECS-only branch would leave `physics-api` on main
    uncompilable. `ecs-macros`, `predict`, `net-api` and `input` change nothing.
- Driven by: the D124 round-1 review's S1 (probes p2 and r3), the fix lane's analysis of
  option (a), the planner's reading of the tree, the D126 design reviews' probes q1 to q4,
  s1, s2, r1, r2, r3a to r3d, r4a to r4c, r5a to r5b and r6a to r6b, and the owner's
  acceptance of option (f) with the structural check folded in
  (`docs/proposals/untaken-commands.md`, 2026-10-18).
- Replaced: D119's "The physics channel is not cleared at the input sync point" and its
  "Rules out a game-side carrier component for contact-driven commands", its "the
  channel's untaken commands" among what the physics snapshot carries, and its
  "`cmd.physics()` carries impulses, forces and `teleport` only" as a name; §6.1's
  `cmd.physics().apply_impulse(bomb, ..)` on a `Commands` parameter, its "physics command
  buffer" and its "takes the commands recorded since the previous step"; §10's "the physics
  sync's pending work" where it meant commands, and its "Command buffers are empty at
  snapshot points by construction" without channels; the D124 addendum's "Open: commands
  untaken at the save survive a one-frame reload (S1)"; `crates/ecs/README.md`'s
  non-clearing channel and `World::channel` sentences, its "every buffer empty" without
  channels, its "events, channels and input ticks cleared", its surface row without the
  writer and take parameters and its rejects list without `ChannelTake`;
  `crates/ecs/SAFETY.md`'s access-table paragraph without the channel parameters;
  `crates/physics-api/README.md`'s "Commands" bullets, its `cmd.physics()` rows, "from any
  stage", "applied by the next tick's step", "grows the channel without bound", the
  `Commands` test row and the lane's "until S1 is decided"; `crates/sim/README.md`'s "with
  the untaken `PhysicsCommands`" and "clears events, channels and input ticks".

## D124 addendum (exact text, replacing the lane's "Open: commands untaken at the save survive a one-frame reload (S1).")

The untaken commands are closed by D126: a run's closing sync point clears every channel,
so none is in flight at a save, a reload or a snapshot, and a writer of the channel in or
after the sync's stage is a schedule build error.

## Verification item (exact text)

### `docs/verification.md`, section 4, add one row

| Every channel is cleared at a run's closing sync point and asserted empty at the input sync point, a channel is written through a `ChannelWriter<C>` parameter and taken through a `ChannelTake<C>` parameter of an exclusive system, and the schedule refuses a writer in or after its taker's stage and a reader or a second taker after it (D126): `CommandChannel::CLEAR_AT_INPUT`, `World::channel`, `Commands::channel` and `World::take_channel` removed, `PhysicsCommands` an ordinary channel, `PhysicsSnapshot` without commands, `PhysicsExt` on the writer, `physics_api::sync` taking through the parameter. The tests: `ScheduleError::ChannelAfterTaker` for a writer in the taker's stage, in a later stage and an exclusive writer ordered before the taker in its stage, for a `Channel<C>` reader in a later stage and a non-exclusive one in the taker's stage, and for a second taker in a later stage (r6b); `ChannelWithoutConsumer` for a writer with no taker and no reader and for one whose only reader is earlier; `InvalidParameter` for `ChannelTake<C>` in a per-entity, a once-per-tick and a finalize table, twice on one channel, beside `Channel<C>` on one channel (r6a) and beside `ChannelWriter<C>` on one channel; a per-entity and a per-region kernel whose only output is a channel entry, read by a later stage; a take yielding the earlier stages' entries with keys resolved and nothing in the next run; `physics-api`'s build of a schedule with the writer in `Stage::Physics` after the step and in `Stage::Post` failing with `ChannelAfterTaker` naming the system, `physics_api::sync` and `PhysicsCommands`; an impulse recorded in `Stage::Simulation` of the reload tick against a reloaded body in that tick's step in both worlds (q2 without post-step impulses); a carrier written in `Stage::Post` and turned into the command by the next tick's simulation kernels landing at the same step on the straight, one-frame, fresh, restore and two-frame paths with equal steps and hashes (q3), a keyed carrier through `Commands::set` in a server and a prediction world with the rows claimed (r3b); a `Transform` written through `Commands::set` in `Stage::Post` sweeping a kinematic body through a touching crate where a `Teleport` from a carrier, recorded by the body's own kernel, places it (s2); the ECS's `game_schedule` and `events.rs` readers passing with the writer parameter, `events.rs` with no opt-out, no take between runs and the closing-clear test (a middle-stage writer, a later reader, an observer at or before the writer's stage seeing nothing on the next run, the channel absent from a snapshot between the runs), `systems.rs` without its take, `layout.rs` and `snapshot.rs` with a `Channel<Physics>` reader beside `THROW_BOMB`, the section 6.1 ui control with the amended example; the pending-work coverage of `tests/snapshot.rs`, `tests/sour.rs` (both crates) and the conformance scene with every post-step command moved to the next tick's simulation stage and the harness's placeholder impulse recorded in the simulation stage only; `sketch::impact_damage` as the simulation-stage relay. Every test runs in debug under nextest; no release step is added. Section 15.5's fourth test is not hash equality with Rapier contacts (s1: the fresh process and the un-reloaded original differ at t0+1 by solver warm state, D124's tolerance). Not run: the reconciliation of a carrier column in the prediction world against a server, which the milestone 1 spike covers | 1 |

## What the planner could not verify

Nothing of the lane was built by the planner or the reviews: not the writer parameter and
its sink, not `ChannelTake` and its fetch, not `check_channels` and its two errors or the
`check_table` refusals, not the closing clear, nor the removals (`CLEAR_AT_INPUT`,
`World::channel`, `Commands::channel`, `pending_commands`, `World::take_channel`), nor the
first commit's channel tail on `CommandBuffer`. The parts are specified against their models
in the tree (`EventWriter`'s sink and `Fetch`, `run.rs:164-200` and `event.rs:62-74`;
`Channel<C>`'s fetch and the exclusive pre-loop, `command.rs:392-432` and `run.rs:440-456`;
`check_events`, `schedule.rs:661-680`), read, not compiled. The reviews emulated the clear
from the test side by taking the channel before each tick, which drops exactly what the
clear drops, so the equivalence results (q2, q3, s2) carry over; showed the run's state at
the close (r1, r3d), that a debug assertion inside a run aborts (r2), that the schedule
refuses `Mut` on a keyed carrier (r3a), that the keyed relay through `Commands::set` records
in both world kinds with the rows claimed (r3b), that an unclaimed exit's kernel records on
the server alone (r3c), that a setup-time `derived` resource passes `sim`'s build rule and
survives a restore (r4a to r4c), that q2's script with post-step impulses leaves entries at
the close (r5a), which is now the build refusal, and on main that a taker with a
`Channel<C>` parameter takes nothing (r6a) and that an undeclared take beside a same-stage
writer and a later reader drops one entry per tick (r6b), the two cases `check_table` and
`check_channels` now refuse; neither refusal was built. Predict's comparison was read from
its README ("Reconciliation", "The build check") and D124, not from `compare.rs`, and no
client reconciled a carrier end to end. The list of post-step commands and spawns is a grep
over `When::AfterStep`, `When::Post` and `cmd.physics()` on main c99727b; one recorded
another way is now a build error of the lane's schedule, or a harness panic. Rapier with
several solver threads and x86-64 were not run. No scheduled nightly has run since 13 Oct:
the 14 to 17 Oct runs failed on account billing, on the old default branch, and the next
runs on main; nothing in this decision rests on a nightly step, since the release steps of
rounds 1 to 5 are withdrawn, and the SOUR gate is the merge's as before.
