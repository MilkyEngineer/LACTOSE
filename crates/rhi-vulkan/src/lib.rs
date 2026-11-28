//! The Vulkan backend of `lactose-rhi-api`, with no window: a device made against the
//! contract's required set on a Vulkan 1.3 driver with `VK_KHR_dynamic_rendering_local_read`
//! (Linux, Windows, and macOS through MoltenVK or KosmicKrisp), reporting what it offers of
//! the optional set.
//!
//! [`VulkanDevice::new`] makes a device. It checks every rule of the contract's "Every
//! backend" column before the driver call the rule guards, so a breach is refused as a
//! value and never reaches the driver; the rules null alone checks are the caller's to
//! keep, and the Khronos validation layer reports a breach of them.
//!
//! What a device counts as pending is completion: every submit to a queue signals the
//! queue's own timeline semaphore as its last operation, and the device reads that value
//! before each check (`lactose-rhi-api`'s README, "Queues by role").

#[cfg(test)]
mod canary;
mod convert;
mod device;
mod encoder;
mod heap;
mod holders;
mod pipeline;
mod raw;
mod records;
mod submit;
mod surface;
mod swapchain;
mod table;
mod timestamps;
mod wait;

use std::sync::{Mutex, MutexGuard, PoisonError};

pub use device::VulkanDevice;
pub use timestamps::Timestamps;

/// Takes a lock, through a poisoning: a test that fails while a lock is held reports its
/// own failure, not a panic in every call after it.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
