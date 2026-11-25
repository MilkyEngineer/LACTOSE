//! Timestamps the GPU writes between whole passes, outside `lactose-rhi-api`'s contract:
//! a cost is reported and changes no signature of the facade (the milestone 2 plan's
//! decision on the native Metal spike), so `cargo xtask measure gpu-cost` reads a pass's GPU
//! time through this backend's own surface. A query pool of one queue role's timestamps,
//! reset on the host once made, each written by an encoder of that role outside any pass and
//! read back once its submit has completed.

use std::sync::Arc;

use lactose_core::Result;
use lactose_rhi_api::{Encoder, QueueRole};

use crate::device::{Shared, VulkanDevice};

/// A pool of timestamps for the encoders of one queue role: each written by
/// [`write`](Self::write) into an encoder, outside any pass, when every command recorded
/// before it in the queue's order has completed, and read by [`read`](Self::read) in
/// nanoseconds of the GPU's clock. Two timestamps of the same pool, written around whole
/// passes, give the GPU time between them; the queue keeps [`valid_bits`](Self::valid_bits)
/// of each, and none where it writes no timestamps.
pub struct Timestamps {
    shared: Arc<Shared>,
}

impl VulkanDevice {
    /// A pool of `count` timestamps for encoders of `role`, each reset and unwritten. On a
    /// queue whose family keeps no valid bits the pool writes nothing and reads `None`.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` for a count of zero; `ErrorKind::Unsupported` where the
    /// device does not offer `hostQueryReset`, which resets the pool on the host; a lost
    /// device's error; a driver's failure as its kind.
    pub fn create_timestamps(&self, role: QueueRole, count: u32) -> Result<Timestamps> {
        let _ = (role, count);
        todo!()
    }
}

impl Timestamps {
    /// How many bits of a timestamp the role's queue keeps valid, from its family's
    /// `timestampValidBits`: zero where it writes none.
    #[must_use]
    pub fn valid_bits(&self) -> u32 {
        todo!()
    }

    /// Records into `encoder` the write of timestamp `index` once every command before it
    /// in its queue's order has completed, after resetting it, so that a pool is written
    /// again frame after frame. The encoder holds the pool until its submit completes. On a
    /// queue with no valid bits it records nothing, and in an encoder that has found a
    /// breach nothing, as the encoder's own calls do.
    ///
    /// # Errors
    ///
    /// `Rule::Encoding` for an encoder of another backend, of another device or of another
    /// role, one inside a pass, one taken by a submit and not reset since, and an index past
    /// the pool's count.
    pub fn write(&self, encoder: &mut dyn Encoder, index: u32) -> Result<()> {
        let _ = (encoder, index, &self.shared);
        todo!()
    }

    /// Each timestamp, in nanoseconds of the GPU's clock (its ticks, masked to the valid
    /// bits, times the device's `timestampPeriod`), where its last write has completed;
    /// `None` for one unwritten, written by a submit not yet complete, or on a queue with no
    /// valid bits. Waits for nothing. A difference of two is the GPU time between them while
    /// the clock does not wrap past the valid bits between their writes.
    ///
    /// # Errors
    ///
    /// A lost device's error; a driver's failure as its kind.
    pub fn read(&self) -> Result<Vec<Option<u64>>> {
        todo!()
    }
}
