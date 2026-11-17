//! The sketch game: one component, one system that counts ticks, and the session applier
//! that gives a joining player a counter. The component key keeps the `sketch::` prefix it
//! had in the headless binary: it enters the world hash, so renaming it moves every recorded
//! stream of the sketch.

use lactose_core::{PlayerId, Result, StableKey};
use lactose_ecs::{ClaimedBy, Mut, World, system};
use lactose_reflect::component;
use lactose_sim::{Registration, SessionApplier, SessionEvent, Stage, StreamingAnchor, TypeTable};

/// Counts ticks; replicated, so a prediction system may read it.
#[component(key = "sketch::counter", persistent, replicated)]
pub struct Counter(#[field(id = 1)] pub u32);

#[system(name = "game_sim::count", worlds = [server, prediction])]
fn count(counter: Mut<Counter>) {
    counter.0 = counter.0.wrapping_add(1);
}

/// Registers [`Counter`] with `types`: a registration's `types`. Fails as
/// [`TypeTable::register`] does.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Counter>()
}

/// Adds `game_sim::count` to [`Stage::Simulation`]: a registration's `systems`. Every tick,
/// in the server and prediction worlds, it adds one to each [`Counter`], wrapping.
pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&COUNT);
}

/// The sketch's session applier, a `Game`'s `sessions`, built once per server world. It
/// meets `crates/sim/README.md`'s contract ("What `SessionApplier::apply` must do"): a join
/// spawns the player's row in the join's region under a key from `World::allocate_net_id`,
/// a [`Counter`] at zero with `ClaimedBy` and a [`StreamingAnchor`], so the player resolves
/// as the claimed, anchored row and the input applier writes into it; a leave finds that
/// row and removes its claim and its anchor, keeping the row and its counter, so a later
/// join spawns a new row. A leave that finds no such row does nothing.
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
                let anchor = StreamingAnchor { priority: 0 };
                world.spawn(key, region, (Counter(0), ClaimedBy { player }, anchor))
            }
            SessionEvent::Leave => {
                let anchored = |key: &StableKey| world.get::<StreamingAnchor>(*key).is_some();
                let Some(key) = world.claimed(player).find(anchored) else {
                    return Ok(());
                };
                world.remove::<ClaimedBy>(key)?;
                world.remove::<StreamingAnchor>(key)?;
                Ok(())
            }
        }
    }
}
