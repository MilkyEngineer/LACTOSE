//! The sketch game: one component, one system that counts ticks, and the session applier
//! that gives a joining player a counter. Its component key and its system's name keep the
//! `sketch::` prefix they had in the headless binary: the component key enters the world
//! hash, so renaming it moves every recorded stream of the sketch.

use lactose_core::{PlayerId, Result, StableKey};
use lactose_ecs::{ClaimedBy, Mut, World, system};
use lactose_reflect::component;
use lactose_sim::{Registration, SessionApplier, SessionEvent, Stage, TypeTable};

/// Counts ticks; replicated, so a prediction system may read it.
#[component(key = "sketch::counter", persistent, replicated)]
pub struct Counter(#[field(id = 1)] pub u32);

#[system(name = "sketch::count", worlds = [server, prediction])]
fn count(counter: Mut<Counter>) {
    counter.0 = counter.0.wrapping_add(1);
}

/// Registers [`Counter`] with `types`: a registration's `types`. Fails as
/// [`TypeTable::register`] does.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Counter>()
}

/// Adds `sketch::count` to [`Stage::Simulation`]: a registration's `systems`. Every tick, in
/// the server and prediction worlds, it adds one to each [`Counter`], wrapping.
pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&COUNT);
}

/// The sketch's session applier, a `Game`'s `sessions`, built once per server world: a join
/// spawns the player's [`Counter`] at zero, claimed, under a fresh network ID, in the join's
/// region; a leave keeps the row, claim included, so the counter outlives its player.
#[must_use]
pub fn sessions() -> Box<dyn SessionApplier> {
    Box::new(Sessions)
}

struct Sessions;

impl SessionApplier for Sessions {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Join { region } => {
                let key = StableKey::Net(world.allocate_net_id()?);
                world.spawn(key, region, (Counter(0), ClaimedBy { player }))
            }
            SessionEvent::Leave => Ok(()),
        }
    }
}
