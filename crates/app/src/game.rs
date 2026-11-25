//! How a game reaches the assembly: one value per game crate naming its generated
//! registration functions, and one value for the game naming every crate and the ordering
//! between them (architecture section 18).

use lactose_core::{Error, Result};
use lactose_ecs::World;
use lactose_persist::Migrations;
use lactose_sim::{Registration, SessionApplier, TypeTable};

/// What one crate contributes to every world: the generated registration functions of
/// architecture section 18, as function pointers. Pointers, not closures, because they are
/// what the dev profile's restart leaves the process with (a function named by a `static`,
/// never a value built before `main`), because every world of a process is built from the
/// same set (the server's, or a client's replicated and prediction worlds), and because a
/// pointer names nothing a save could depend on. A game crate exposes the functions, and
/// the binary holds this value and lists it in a [`Game`]: a game crate may name neither
/// this crate nor `lactose-persist`, and a binary declares no setup and no migration
/// function, so both are `None` (D152). Where a game crate's registration value and its
/// migrations are declared is decided before milestone 5 (`docs/verification.md`).
#[derive(Clone, Copy, Debug)]
pub struct CrateRegistration {
    /// The crate's name, as `Cargo.toml` spells it: the log names it, and so does the
    /// build error when one of its functions fails.
    pub name: &'static str,
    /// `register_types`: every component and resource the crate defines, into the
    /// `TypeTable` of every world and every load, each of which [`Game::types`] builds.
    pub types: fn(&mut TypeTable) -> Result<()>,
    /// `setup`: the crate's configuration resources, or `None` when it inserts nothing. Runs
    /// in every world kind; what it inserts is `derived` (`crates/sim/README.md`, "Game
    /// crates").
    pub setup: Option<fn(&mut World) -> Result<()>>,
    /// `register_systems`: the crate's systems into the stages and passes. The same function
    /// serves every kind; the builder keeps what names the kind.
    pub systems: fn(&mut Registration),
    /// `register_migrations`: one step per schema version bump, or `None`
    /// (`crates/persist/README.md`, "The registration a game crate writes").
    pub migrations: Option<fn(&mut Migrations) -> Result<()>>,
}

/// The game, as the binary hands it to the assembly: every game crate in the order their
/// functions are called, and the game's own ordering between crates whose sets do not
/// name each other. The engine's crates are not listed here: the assembly registers them
/// itself, before the game's, and adds the edges between engine sets and game sets
/// (`README.md`, "Registration").
#[derive(Clone, Copy, Debug)]
pub struct Game {
    /// For the log.
    pub name: &'static str,
    /// The game crates, in call order. At milestone 5 `game-sim`, `game-ai` and
    /// `game-content`; until then `game-sim`'s sketch game, which the headless binary runs,
    /// or the sketch a test or the spike writes.
    pub crates: &'static [CrateRegistration],
    /// `Registration::order` edges between the game's own crates, or `None`. Called once per
    /// world after every crate's `systems`, before the assembly's own edges.
    pub order: Option<fn(&mut Registration)>,
    /// The game's session applier, built once per server world: what spawns or re-claims a
    /// player's entity on `Join` and resolves it on `Leave` (`crates/sim/README.md`, "What
    /// `SessionApplier::apply` must do"). The game's, since the entity is the game's rows.
    /// `None` leaves the builder's default, which drops and logs every session event: a
    /// headless run with no players.
    pub sessions: Option<fn() -> Box<dyn SessionApplier>>,
}

impl Game {
    /// The `TypeTable` a world or a load is built from, a fresh one per call and every one
    /// the same function of the same list: `TypeTable::new()`, then the engine's
    /// `register_types` in the order the README states, then every crate's `types` in list
    /// order. Every `TypeTable` of the process is built by this, so every world and every
    /// save of the process agree on the registry. The first error ends the fold, with the
    /// crate's name in the message.
    pub fn types(&self) -> Result<TypeTable> {
        /// A crate's name and its `register_types`.
        type Types = (&'static str, fn(&mut TypeTable) -> Result<()>);
        /// The engine's crates, in the README's order: before every game crate.
        const ENGINE: [Types; 6] = [
            ("lactose-input", lactose_input::register_types),
            ("lactose-net-api", lactose_net_api::register_types),
            ("lactose-physics-api", lactose_physics_api::register_types),
            ("lactose-anim", lactose_anim::register_types),
            ("lactose-present-api", lactose_present_api::register_types),
            ("lactose-predict", lactose_predict::register_types),
        ];
        let mut types = TypeTable::new();
        let game = self.crates.iter().map(|registration| (registration.name, registration.types));
        for (name, register) in ENGINE.into_iter().chain(game) {
            register(&mut types).map_err(|error| named(name, &error))?;
        }
        Ok(types)
    }

    /// The `Migrations` a loader takes, a fresh one per call: every crate's `migrations` in
    /// list order, into `Migrations::new()`. The first error ends the fold, with the
    /// crate's name in the message.
    pub fn migrations(&self) -> Result<Migrations> {
        let mut migrations = Migrations::new();
        for registration in self.crates {
            if let Some(register) = registration.migrations {
                register(&mut migrations).map_err(|error| named(registration.name, &error))?;
            }
        }
        Ok(migrations)
    }
}

/// `error` with the name of the crate whose registration function returned it.
pub(crate) fn named(name: &str, error: &Error) -> Error {
    Error::new(error.kind(), format!("`{name}`: {}", error.message()))
}
