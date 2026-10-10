//! The build check of D121: a prediction-world system may name only components the server
//! corrects. A persistent component that is not replicated is never sent, so a prediction
//! system that reads it simulates from state no correction ever touches and diverges
//! silently; the check refuses the system at build rather than let the spike find it. An
//! exclusive system shows no table, so one that runs in the prediction world must be an
//! engine's, named here.

use lactose_core::Result;
use lactose_ecs::{SystemDescriptor, SystemSet};
use lactose_reflect::{Persistence, Registry, TypeInfo};

/// The sets an exclusive system whose `worlds` include the prediction world may be in:
/// the engine's exclusive systems that run there, each reviewed for what it reads through
/// `&mut World`. `sim`'s input sync point (`sim::sets::INPUT`), `input`'s writer
/// declaration (`input::sets::APPLY`), and `physics-api`'s sync and write-back
/// (`physics_api::sets::SYNC`, `physics_api::sets::WRITE_BACK`). An engine crate that adds
/// an exclusive prediction-world system extends this list in the same change; a game
/// crate never runs one there.
pub static EXCLUSIVE_SETS: [&SystemSet; 4] = [
    &lactose_sim::sets::INPUT,
    &lactose_input::sets::APPLY,
    &lactose_physics_api::sets::SYNC,
    &lactose_physics_api::sets::WRITE_BACK,
];

/// Whether a prediction-world system may name the component `info` describes: it is
/// replicated (the server sends it exact for a claimed entity and dequantised for a
/// proxy), transient (written before it is read every tick, from this tick's state) or
/// derived (rebuilt, never simulation state). A persistent component that is not
/// replicated is refused: `OwnedBy` and `PredictedOrigin` among them. Resources are not
/// judged here: a world resource is `derived` by `sim`'s build rule, and a region
/// resource is corrected only by the relevance message, which the README names as a gap.
#[must_use]
pub fn prediction_may_name(info: &TypeInfo) -> bool {
    info.persistence() != Persistence::Persistent || info.tracking().replicated
}

/// Checks every system of `systems` whose `worlds` include the prediction world: every
/// component its access table or its finalize's access table names, of any kind (`&T`,
/// `Mut<T>`, `Option<&T>`, `Option<Mut<T>>`, `With<T>`, `Without<T>`, `Changed<T>`,
/// `writes = [T]`), must satisfy [`prediction_may_name`] under `registry`; and an
/// exclusive system, whose `&mut World` shows no table, must be in one of
/// [`EXCLUSIVE_SETS`], compared by the set's name. Fails with `InvalidData` naming the
/// first system and component, or the first exclusive system, that do not, or a component
/// the registry does not know. A system whose `worlds` exclude the prediction world is not
/// judged, whatever it names.
///
/// What `app` hands to the simulation builder for the prediction world as its check
/// function, so a refused system is a build error (`crates/predict/README.md`, "The build
/// check"); also what a test or the spike calls on a list of descriptors.
pub fn check_prediction_reads(
    registry: &Registry,
    systems: &[&'static SystemDescriptor],
) -> Result<()> {
    todo!()
}
