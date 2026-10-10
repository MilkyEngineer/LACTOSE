//! The physics backend over Rapier (`rapier3d-f64`, patched: `vendor/README.md`).
//!
//! [`backend`] is the one public item: it takes the solver pool `app` builds and returns the
//! [`PhysicsBackend`] the `physics-api` sync drives. Everything the trait's documentation and
//! `crates/physics-api/README.md` state of a backend holds here; what this crate adds (where
//! the mass comes from, how keys reach Rapier, the snapshot bytes) is in `README.md`.
//!
//! Every call records into a bookkeeping keyed by `StableKey`, and the next step applies it
//! to the solver in ascending key order before it steps, so the solver's state, and with it
//! every step and snapshot byte, depends on the rows alone and not on the order the calls
//! came in.

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
/// priority. The pool is the backend's alone: Rapier's parallel work runs on it and on
/// nothing else.
#[must_use]
pub fn backend(pool: rayon::ThreadPool) -> Box<dyn PhysicsBackend> {
    pool.broadcast(|context| {
        if let Err(error) = ThreadClass::PhysicsSolver.enter() {
            tracing::warn!(
                target: backend::TARGET,
                thread = context.index(),
                %error,
                "a solver thread runs without its scheduling class",
            );
        }
    });
    Box::new(backend::Rapier::new(Arc::new(pool)))
}
