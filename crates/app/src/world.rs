//! What the assembly decides for a world beyond what the game registers, and where a
//! server world starts.

use lactose_ecs::LayoutConfig;
use lactose_physics_api::PhysicsConfig;
use lactose_sim::{RegionControl, RegionPolicy};

use crate::runtime::LOG_RETENTION;
use crate::save::SavedWorld;

/// What the assembly decides for one world: the storage layout, who decides which regions
/// are loaded, the region policy, the physics configuration and the log retention. The
/// registration, the appliers, the hooks and the check are not here: the assembly supplies
/// them from the world's kind (`README.md`, "Registration").
#[derive(Clone, Debug)]
pub struct WorldConfig {
    /// The storage layout: the default in production, SOUR's layout mode under test
    /// (`lactose_sim::harness::layout_mode`). A no-op for results.
    pub layout: LayoutConfig,
    /// `Anchored` for a server with an atlas and pinned regions; `Directed` for a headless
    /// run, which loads what its frames carry, and for every client world.
    pub regions: RegionControl,
    /// The region policy, read under `Anchored` only; the overload ladder turns its
    /// activation radius (not in this crate yet).
    pub policy: RegionPolicy,
    /// Gravity and the reserved workers: the solver pool is sized from the reservation
    /// (`README.md`, "The executor and the solver pool").
    pub physics: PhysicsConfig,
    /// `SimulationBuilder::log_retention`, at least [`CATCH_UP_CAP`](crate::CATCH_UP_CAP);
    /// a smaller value is a build error of kind `InvalidData`.
    pub log_retention: u32,
}

impl WorldConfig {
    /// The headless configuration: the default layout, `Directed` regions, a policy of
    /// radii one, two and three (unread under `Directed`, valid should the control change),
    /// `PhysicsConfig::default()` (Earth gravity, one reserved worker) and
    /// [`LOG_RETENTION`]. Real, not `todo!()`: it is a value.
    #[must_use]
    pub fn headless() -> Self {
        Self {
            layout: LayoutConfig::default(),
            regions: RegionControl::Directed,
            policy: RegionPolicy { activation_radius: 1, streaming_radius: 2, release_radius: 3 },
            physics: PhysicsConfig::default(),
            log_retention: LOG_RETENTION,
        }
    }
}

/// Where a server world starts.
#[derive(Clone, Debug)]
pub enum Start {
    /// Tick zero and the counter's first value; every region enters through the frames.
    Fresh,
    /// From a save ([`load`](crate::load), or a [`SavedWorld`] a save point produced): the
    /// saved tick and counter into the builder, every saved region into the server's store
    /// (`Server::kept`), from which the first step loads every one under `Directed` and the
    /// lifecycle's `Load` requests bring each back under `Anchored` as players join, so no
    /// saved region is lost; every player joins again through `sessions` (`README.md`,
    /// "Saves and loads").
    Loaded(SavedWorld),
}
