//! The build check of D121: a prediction-world system may name only components the server
//! corrects. A persistent component that is not replicated is never sent, so a prediction
//! system that reads it simulates from state no correction ever touches and diverges
//! silently; the check refuses the system at build rather than let the spike find it.

use lactose_core::Result;
use lactose_ecs::SystemDescriptor;
use lactose_reflect::{Persistence, Registry, TypeInfo};

/// Whether a prediction-world system may name the component `info` describes: it is
/// replicated (the server sends it exact for a claimed entity and dequantised for a
/// proxy), transient (written before it is read every tick, from this tick's state) or
/// derived (rebuilt, never simulation state). A persistent component that is not
/// replicated is refused. Resources are not judged here: a world resource is `derived` by
/// `sim`'s build rule, and a region resource is corrected only by the relevance message,
/// which the README names as a gap.
#[must_use]
pub fn prediction_may_name(info: &TypeInfo) -> bool {
    info.persistence() != Persistence::Persistent || info.tracking().replicated
}

/// Checks every system of `systems` whose `worlds` include the prediction world: every
/// component its access table or its finalize's access table names, of any kind (`&T`,
/// `Mut<T>`, `Option<&T>`, `Option<Mut<T>>`, `With<T>`, `Without<T>`, `Changed<T>`,
/// `writes = [T]`), must satisfy [`prediction_may_name`] under `registry`. Fails with
/// `InvalidData` naming the first system and component that do not, or a component the
/// registry does not know. A system whose `worlds` exclude the prediction world is not
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
