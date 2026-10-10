//! The deterministic simulation (architecture section 4, `sim`): what turns a
//! [`World`](lactose_ecs::World), a tick [`Schedule`](lactose_ecs::Schedule) and an
//! [`Executor`](lactose_ecs::Executor) into a simulation that ticks, hashes, loads and
//! unloads regions, and applies at the input sync point what enters the boundary.
//!
//! Foundation tier, inside the determinism boundary. `README.md` is the contract: it states
//! what the signatures cannot, and what a caller may rely on.
//!
//! # Status
//!
//! A facade: every public signature is final and every body that decides anything is
//! `todo!()`. The implementation lane replaces the bodies behind this surface and removes
//! the crate-level `allow` with the last `todo!()`.
//!
//! # Where to start
//!
//! - A [`TypeTable`] registers every component and resource type once, for the registry and
//!   for the world hash; a generated registration function fills it.
//! - [`SimulationBuilder`] makes a [`Simulation`] for one
//!   [`WorldKind`](lactose_ecs::WorldKind): the world, what setup functions insert into it,
//!   the tick schedule composed from the four [`Stage`]s every registration function adds
//!   to, the [`Pass`] schedules run between ticks, and the region lifecycle.
//! - [`Simulation::step`] runs one tick from a [`TickInput`], the unit of everything that
//!   enters the boundary; [`Simulation::run_passes`] runs a frame's passes.
//! - [`Simulation::world_hash`] and [`Hashers`] are the world hash of architecture section
//!   8.3; [`RegionDigest`] is its field-level diff.
//! - [`harness`] is what a simulation test under SOUR uses; [`journal`] names the replay
//!   types.
//!
//! ```no_run
//! use lactose_ecs::{Executor, WorldKind};
//! use lactose_sim::{SimulationBuilder, TickInput, TypeTable};
//!
//! # fn main() -> lactose_core::Result<()> {
//! let executor = Executor::builder().build()?;
//! let mut simulation = SimulationBuilder::new(WorldKind::Server, TypeTable::new()).build()?;
//! // The timer root in `app` paces this loop with `TickPacer`; a step itself reads no clock.
//! for _ in 0..3 {
//!     simulation = executor.block_on(simulation.step(&executor, TickInput::default()))?;
//! }
//! simulation = executor.block_on(simulation.run_passes(&executor))?;
//! let hash = simulation.hash()?;
//! # let _ = hash;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every body is `todo!()` until the implementation lane replaces it, and \
              the fields and parameters it will use are declared now"
)]

pub mod harness;
mod hash;
mod input;
pub mod journal;
mod region;
mod schedule;
mod simulation;
mod systems;
mod time;

pub use hash::{Differences, DigestRows, FieldDifference, Hashers, RegionDigest, TypeTable};
pub use input::{
    InputApplier, PlayerSample, RegionBundle, SessionApplier, SessionEvent, StagedState, TickInput,
};
pub use region::{
    AssetFailureEvent, LoadFailure, RegionAtlas, RegionCell, RegionControl, RegionExport,
    RegionLifecycle, RegionPolicy, RegionRequest, SessionFailure, StreamingAnchor, Within,
};
pub use schedule::{Pass, Registration, Stage, StageBuilder, sets};
pub use simulation::{
    PassRun, RunError, SavePoint, SavePointRun, Simulation, SimulationBuilder, Step,
};
pub use time::{CurrentTick, TICK_RATE_HZ, TickDelta, TimeScale};

/// The thread-safety the contract promises, pinned so the implementation cannot lose it
/// without a compile error: the timer root is a `'static` task that owns the simulation
/// and drives its runs, a tick input and a bundle are assembled on other threads and
/// handed over, a registration is filled by functions handed across threads, the hashers
/// are shared by every chunk job, and a digest, an export and a journal travel.
const _: () = {
    const fn send<T: Send>() {}
    const fn send_sync<T: Send + Sync>() {}
    send::<Simulation>();
    send::<SimulationBuilder>();
    send::<Registration>();
    send::<Step<'_>>();
    send::<PassRun<'_>>();
    send::<SavePointRun<'_>>();
    send::<RunError>();
    send_sync::<TickInput>();
    send_sync::<RegionBundle>();
    send_sync::<SavePoint>();
    send_sync::<RegionExport>();
    send_sync::<LoadFailure>();
    send_sync::<Hashers>();
    send_sync::<TypeTable>();
    send_sync::<RegionDigest>();
    send_sync::<RegionAtlas>();
    send_sync::<RegionLifecycle>();
    send_sync::<journal::Journal>();
};
