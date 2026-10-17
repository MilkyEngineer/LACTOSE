//! The physics backend over Rapier (`rapier3d-f64`, patched: `vendor/README.md`).
//!
//! [`backend`] is the one public item: it takes the solver pool `app` builds and returns the
//! [`PhysicsBackend`] the `physics-api` sync drives. Everything the trait's documentation and
//! `crates/physics-api/README.md` state of a backend holds here; what this crate adds (where
//! the mass comes from, how keys reach Rapier, the snapshot bytes) is in `README.md`.
//!
//! Every call records into a bookkeeping keyed by `StableKey`, and the next step applies it
//! to the solver in ascending key order before it steps, so the solver's state, and with it
//! every step and snapshot byte, does not depend on the order of the calls between two
//! steps. It does depend on the row history (every create, remove, write and command, in
//! order): a body's results depend on every body the world holds or held, far-away awake
//! bodies that touch only each other included, and bit equality holds only between worlds
//! whose row histories are identical.

#![forbid(unsafe_code)]

mod backend;
mod book;
mod convert;
mod query;
mod snapshot;

use std::sync::Arc;

use lactose_core::ThreadClass;
use lactose_physics_api::PhysicsBackend;

/// The backend, stepping on `pool`. Each of the pool's threads enters
/// [`ThreadClass::PhysicsSolver`] here, which sets its floating-point state; a thread the
/// operating system refuses the scheduling class is logged and runs at its default
/// priority. Rapier's parallel work runs on the pool and on nothing else, and a step can wait
/// for every thread of it: the pool is the backend's alone, no job on it may block outside
/// rayon, and it must not be built with `use_current_thread`.
#[must_use]
pub fn backend(pool: rayon::ThreadPool) -> Box<dyn PhysicsBackend> {
    pool.broadcast(|context| {
        if let Err(error) = ThreadClass::PhysicsSolver.enter() {
            tracing::warn!(
                target: backend::BACKEND_TARGET,
                thread = context.index(),
                %error,
                "a solver thread runs without its scheduling class",
            );
        }
    });
    Box::new(backend::Rapier::new(Arc::new(pool)))
}
