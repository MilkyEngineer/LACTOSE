//! How a game reaches the assembly: one value per game crate naming its generated
//! registration functions, and one value for the game naming every crate and the ordering
//! between them (architecture section 18).

use lactose_core::Result;
use lactose_ecs::World;
use lactose_persist::Migrations;
use lactose_sim::{Registration, SessionApplier, TypeTable};

/// What one crate contributes to every world: the generated registration functions of
/// architecture section 18, as function pointers. Pointers, not closures, because they are
/// what the dev profile's restart leaves the process with (a function named by a `static`,
/// never a value built before `main`), because every world of a process is built from the
/// same set (the server's, or a client's replicated and prediction worlds), and because a
/// pointer names nothing a save could depend on. A game crate exposes one of these as a
/// `pub static`; the binary lists them in a [`Game`].
#[derive(Clone, Copy, Debug)]
pub struct CrateRegistration {
    /// The crate's name, as `Cargo.toml` spells it: the log names it, and so does the
    /// build error when one of its functions fails.
    pub name: &'static str,
    /// `register_types`: every component and resource the crate defines, into the one
    /// `TypeTable` every world of the process is built from.
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
    /// `game-content`; at milestone 1 the sketch a test or the spike writes.
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
    /// The one `TypeTable` every world of the process is built from: `TypeTable::new()`,
    /// then the engine's `register_types` in the order the README states, then every
    /// crate's `types` in list order. The first error ends the fold, with the crate's
    /// name in the message.
    pub fn types(&self) -> Result<TypeTable> {
        todo!("fold the engine's and every crate's register_types into one table")
    }

    /// The one `Migrations` a loader takes: every crate's `migrations` in list order, into
    /// `Migrations::new()`. The first error ends the fold, with the crate's name in the
    /// message.
    pub fn migrations(&self) -> Result<Migrations> {
        todo!("fold every crate's register_migrations into one table")
    }
}
