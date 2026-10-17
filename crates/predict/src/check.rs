//! The build check of D121: a prediction-world system may name only components the server
//! corrects. A persistent component that is not replicated is never sent, so a prediction
//! system that reads it simulates from state no correction ever touches and diverges
//! silently; the check refuses the system at build rather than let the spike find it. An
//! exclusive system shows no table, so one that runs in the prediction world must be an
//! engine's, named here by system key.

use lactose_core::{Error, ErrorKind, Result, SystemKey};
use lactose_ecs::{AccessTarget, SystemDescriptor, WorldKind};
use lactose_reflect::{Persistence, Registry, TypeInfo};

/// The engine's exclusive systems that run in the prediction world, by system key: the
/// xxh64 of the declared name, which the schedule rejects a collision of, so a look-alike
/// set name or system passes nothing. Each is reviewed for what it reads through
/// `&mut World`: `sim`'s six input-sync-point systems (hidden plumbing `crates/sim/
/// README.md` names; renaming one is a change to this list in the same commit), `input`'s
/// writer declaration, and `physics-api`'s sync and write-back. An engine crate that adds
/// an exclusive prediction-world system extends this list in the same change; a game crate
/// never runs one there. Test-support scenes are not shipped and are not listed:
/// `physics-api`'s conformance scene has an exclusive `input_tick` system, and a test of
/// this crate that builds that scene does not run the check.
pub const EXCLUSIVE_SYSTEMS: [SystemKey; 9] = [
    SystemKey::from_name("sim::begin_tick"),
    SystemKey::from_name("sim::apply_frame_regions"),
    SystemKey::from_name("sim::apply_sessions"),
    SystemKey::from_name("sim::apply_inputs"),
    SystemKey::from_name("sim::apply_staged_state"),
    SystemKey::from_name("sim::apply_region_policy"),
    SystemKey::from_name("input::player_input_writer"),
    SystemKey::from_name("physics_api::sync"),
    SystemKey::from_name("physics_api::write_back"),
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
/// exclusive system, whose `&mut World` shows no table, must have its
/// `SystemDescriptor::key` in [`EXCLUSIVE_SYSTEMS`]. Fails with `InvalidData` naming the
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
    let refused = |message: String| Err(Error::new(ErrorKind::InvalidData, message));
    for system in systems.iter().filter(|system| system.worlds().contains(WorldKind::Prediction)) {
        if system.is_exclusive() && !EXCLUSIVE_SYSTEMS.contains(&system.key()) {
            return refused(format!(
                "`{}` is an exclusive system that runs in the prediction world and is not one \
                 of the engine's (`lactose_predict::EXCLUSIVE_SYSTEMS`): its `&mut World` \
                 can read what the server never corrects",
                system.name()
            ));
        }
        for access in system.access().iter().chain(system.finalize_access()) {
            let AccessTarget::Component(component) = access.target() else { continue };
            let Some(info) = registry.get(component) else {
                return refused(format!(
                    "`{}` names a component the registry does not know ({:#018x})",
                    system.name(),
                    component.0
                ));
            };
            if !prediction_may_name(info) {
                return refused(format!(
                    "`{}` runs in the prediction world and names `{}`, a persistent component \
                     that is not replicated: the server never sends it, so no correction \
                     reaches what the system reads",
                    system.name(),
                    info.name()
                ));
            }
        }
    }
    Ok(())
}
