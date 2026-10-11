//! Writing a frame's samples into the world (architecture sections 6, 6.1 and 9.2): the
//! `InputApplier` the input sync point calls, the writer declaration that gives the
//! transient column a declared writer, and the registration pair `app` calls.
//!
//! Deterministic by construction: the applier decodes integers, resolves the player's
//! entity through the world's claim index in key order and writes through the world; no
//! float, no clock, no allocation beyond the ECS's own.

use lactose_core::{PlayerId, Result, StableKey};
use lactose_ecs::{World, system};
use lactose_sim::{InputApplier, PlayerSample, Registration, Stage, StreamingAnchor, TypeTable};

use crate::sample::PlayerInput;

/// The system sets this crate declares.
pub mod sets {
    use lactose_ecs::SystemSet;

    /// The set of this crate's writer declaration, in
    /// [`Stage::Input`](lactose_sim::Stage::Input) after
    /// [`lactose_sim::sets::INPUT`], where the samples have been written. `app` orders
    /// another crate's sync-point set against it with `Registration::order`.
    pub static APPLY: SystemSet =
        SystemSet::new("input::apply").after(&[&lactose_sim::sets::INPUT]);
}

/// The player's own entity: the first key of [`World::claimed`]`(player)`, which yields
/// the rows with [`ClaimedBy`](lactose_ecs::ClaimedBy) for `player` in ascending origin-key
/// order from the world's claim index, whose row also carries
/// [`StreamingAnchor`](lactose_sim::StreamingAnchor) by [`World::get`]. The session
/// applier writes the anchor on the server and `lactose-predict` writes it on the local
/// player in the prediction world; never a side table (`crates/sim/README.md`, "The input
/// sync point"), which a rollback would leave stale. With two such rows (a game's bug) the
/// lesser key is the answer on every machine. Costs the player's claims, allocates
/// nothing and takes `&World`. `None` when the player has no such row. The one definition
/// of "the player's entity", for every applier and for `net` and `predict`.
#[must_use]
pub fn player_entity(world: &World, player: PlayerId) -> Option<StableKey> {
    world.claimed(player).find(|&key| world.get::<StreamingAnchor>(key).is_some())
}

/// The [`InputApplier`] for [`PlayerInput`]: what `app` hands to
/// [`SimulationBuilder::inputs`](lactose_sim::SimulationBuilder::inputs) for the server,
/// the prediction world and a replay alike.
///
/// For each entry of the frame, after `sim` has set the input tick: decodes
/// `sample.sample` with [`PlayerInput::from_sample_bytes`] (`InvalidData` drops the
/// entry); resolves the entity with [`player_entity`], and returns `Ok(())` without
/// writing when there is none, since a player whose `Join` is held while its region
/// streams in has samples before it has a row and that is not an error worth a log line
/// per tick; then writes the value with [`World::insert`], which adds the component to a
/// row that lacks it (an archetype change the structural log records, once per player)
/// and overwrites it otherwise. Because the column is transient, a row keeps the
/// component across ticks and reads as [`PlayerInput::IDLE`] on a tick without a sample.
/// `sample.substituted` and `sample.input_tick` are not read: the first is the journal's
/// and the second was applied by `sim`. An error is logged by `sim` and that entry is
/// dropped; the tick runs.
#[derive(Debug, Default)]
pub struct PlayerInputApplier {
    _private: (),
}

impl PlayerInputApplier {
    /// The applier. It holds no state: the world is the only input.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl InputApplier for PlayerInputApplier {
    fn apply(&mut self, world: &mut World, player: PlayerId, sample: &PlayerSample) -> Result<()> {
        let input = PlayerInput::from_sample_bytes(&sample.sample)?;
        match player_entity(world, player) {
            Some(key) => world.insert(key, input),
            None => Ok(()),
        }
    }
}

/// The declared writer of the transient [`PlayerInput`] column (`crates/ecs/README.md`,
/// "Commands and events"): an exclusive system at the input sync point, after `sim`'s
/// systems, that declares `writes = [PlayerInput]` and does nothing, because the write
/// happened inside `sim::apply_inputs` through the applier. The declaration is what lets a
/// kernel in a later stage take `&PlayerInput`. The body is final: there is nothing to
/// decide.
#[system(
    name = "input::player_input_writer",
    worlds = [server, prediction],
    exclusive,
    in_set = sets::APPLY,
    writes = [PlayerInput]
)]
fn player_input_writer(world: &mut World) {
    let _ = world;
}

/// Registers this crate's one component, [`PlayerInput`]. `app` calls it on the
/// `TypeTable` every world is built from.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<PlayerInput>()
}

/// Adds [`sets::APPLY`] and the writer declaration to [`Stage::Input`]. `app` passes it to
/// [`SimulationBuilder::systems`](lactose_sim::SimulationBuilder::systems) for every world
/// kind; the declaration is skipped in a `Client` world, which runs no kernel. This crate
/// has no `setup` function: it inserts no resource, and the applier goes in through
/// `SimulationBuilder::inputs`.
pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Input).sets(&[&sets::APPLY]).add(&PLAYER_INPUT_WRITER);
}
