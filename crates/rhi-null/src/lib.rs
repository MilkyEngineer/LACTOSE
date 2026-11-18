//! The null backend of `lactose-rhi-api`: a device that keeps the state a caller carries
//! between calls and refuses every breach of the contract by the rule it breaks, under the
//! [`Capabilities`](lactose_rhi_api::Capabilities) it is created with, and draws nothing.
//! Test support: every consumer of the facade runs its tests against it, under one queue
//! and several, coherent memory and not, and each optional capability on and off.
//!
//! [`NullDevice::new`] makes a device; [`NullDevice::control`] reaches the two things the
//! contract leaves to a test, a device loss and a stall of presents; [`NullSurface`] is a
//! surface with no window, for a device that makes a swapchain.
//!
//! What null counts as pending is its own: a submit is pending until the caller has seen a
//! point it, a later submit to its queue, or a submit that waits for such a point, signals
//! (`lactose-rhi-api`'s README, "Queues by role"). A submit runs, its buffer-to-buffer
//! copies moving their bytes, as soon as its waits are reached.

#![forbid(unsafe_code)]

mod device;
mod encoder;
mod pipeline;
mod records;
mod submit;
mod swapchain;
mod table;
#[cfg(test)]
mod tests;

use std::sync::{Mutex, MutexGuard, PoisonError};

pub use device::{Control, NullDevice, NullSurface};

/// Takes a lock, through a poisoning: a test that fails while one of null's locks is held
/// reports its own failure, not a panic in every call after it.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
