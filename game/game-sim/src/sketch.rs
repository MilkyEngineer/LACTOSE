//! The sketch game: a component and a system that counts ticks, a component and a system
//! that move a row round a circle, and the session applier that gives a joining player a
//! counter. The component keys keep the `sketch::` prefix the first had in the headless
//! binary: a key enters the world hash, so renaming one moves every recorded stream of the
//! sketch.

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{PlayerId, Result, StableKey};
use lactose_ecs::{ClaimedBy, Ctx, Mut, World, system};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_present_api::MeshInstance;
use lactose_reflect::component;
use lactose_sim::{Registration, SessionApplier, SessionEvent, Stage, StreamingAnchor, TypeTable};

/// Counts ticks; replicated, so a prediction system may read it.
#[component(key = "sketch::counter", persistent, replicated)]
pub struct Counter(#[field(id = 1)] pub u32);

#[system(name = "game_sim::count", worlds = [server, prediction])]
fn count(counter: Mut<Counter>) {
    counter.0 = counter.0.wrapping_add(1);
}

/// Moves its row round a horizontal circle, a function of the tick alone: the mover of the
/// client's local world (`crates/app/README.md`, "The mover of the local world"). A row
/// with it has no body.
#[component(key = "sketch::orbit", persistent, replicated)]
pub struct Orbit {
    /// The circle's centre.
    #[field(id = 1)]
    pub centre: Vec3,
    /// The circle's radius.
    #[field(id = 2)]
    pub radius: Scalar,
    /// Ticks to go round once. Zero leaves the row where it is.
    #[field(id = 3)]
    pub period: u32,
    /// Ticks added to the world's tick: where on the circle the row is.
    #[field(id = 4)]
    pub phase: u32,
}

impl Orbit {
    /// Where the row is at `tick`, or `None` for a period of zero. No state accumulates, so
    /// a rollback, a load and a full pass place the row where an unbroken run does. At the
    /// tick's wrap the row jumps unless `period` divides 2^32.
    fn at(&self, tick: u32) -> Option<Transform> {
        if self.period == 0 {
            return None;
        }
        let step = tick.wrapping_add(self.phase) % self.period;
        let angle = Scalar::TAU * Scalar::from_u32(step) / Scalar::from_u32(self.period);
        let (sin, cos) = angle.sin_cos();
        let offset = Vec3::new(cos, Scalar::ZERO, sin);
        Some(Transform {
            translation: self.centre + self.radius * offset,
            rotation: Quat::from_rotation_y(angle),
        })
    }
}

/// Writes the row's `Transform` from its [`Orbit`] at the tick being run, every tick, and
/// never `Teleported`, so the row is interpolated.
#[system(name = "game_sim::orbit", worlds = [server, prediction])]
fn orbit(ctx: Ctx, orbit: &Orbit, transform: Mut<Transform>) {
    if let Some(placed) = orbit.at(ctx.tick().0) {
        *transform = placed;
    }
}

/// Registers [`Counter`] and [`Orbit`] with `types`: a registration's `types`. Fails as
/// [`TypeTable::register`] does.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Counter>()?;
    types.register::<Orbit>()
}

/// Adds `game_sim::count` and `game_sim::orbit` to [`Stage::Simulation`]: a registration's
/// `systems`. Every tick, in the server and prediction worlds, the first adds one to each
/// [`Counter`], wrapping, and the second places each row with an [`Orbit`] of a period
/// other than zero: with `t` the tick being run, `step = (t + phase) mod period`, the
/// addition wrapping, `angle = TAU * step / period`, the translation
/// `centre + radius * (cos angle, 0, sin angle)` and the rotation `angle` about Y.
pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&COUNT).add(&ORBIT);
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

/// The rows of the client's fixed-frame scene (decision 5.1): seven, each its own colour.
pub const SCENE_ROWS: u32 = 7;

/// The scene's period in ticks: seven seconds at sixty ticks a second, so that each row's
/// phase is a whole number of ticks.
pub const SCENE_PERIOD: u32 = 420;

/// The scene's session applier, the client binary's `sessions` (`crates/app/README.md`, "The
/// mover of the local world"). A join spawns [`SCENE_ROWS`] rows in the join's region, each
/// under a key from `World::allocate_net_id`, with an [`Orbit`] round the origin at four
/// metres, of one period and phases a seventh of it apart, a `Transform` that the first tick's
/// `orbit` places, and a `MeshInstance` of the mesh `1`. Each join spawns them again; a leave
/// does nothing. No row is claimed or anchored.
#[must_use]
pub fn scene() -> Box<dyn SessionApplier> {
    Box::new(Scene)
}

struct Scene;

impl SessionApplier for Scene {
    fn apply(&mut self, world: &mut World, _player: PlayerId, event: SessionEvent) -> Result<()> {
        let SessionEvent::Join { region } = event else { return Ok(()) };
        for row in 0..SCENE_ROWS {
            let key = StableKey::Net(world.allocate_net_id()?);
            let orbit = Orbit {
                centre: Vec3::ZERO,
                radius: Scalar::from_i32(4),
                period: SCENE_PERIOD,
                phase: row * (SCENE_PERIOD / SCENE_ROWS),
            };
            let mesh = MeshInstance::new(AssetRef::new(AssetId::from_bits(1)));
            world.spawn(key, region, (Transform::at(Vec3::ZERO), mesh, orbit))?;
        }
        Ok(())
    }
}
