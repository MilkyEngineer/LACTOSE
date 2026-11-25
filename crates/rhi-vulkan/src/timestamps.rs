//! Timestamps the GPU writes between whole passes, outside `lactose-rhi-api`'s contract:
//! a cost is reported and changes no signature of the facade (the milestone 2 plan's
//! decision on the native Metal spike), so `cargo xtask measure gpu-cost` reads a pass's GPU
//! time through this backend's own surface. A query pool of one queue role's timestamps,
//! reset on the host once made, each written by an encoder of that role outside any pass and
//! read back once its submit has completed.

use core::any::Any;
use core::time::Duration;
use std::sync::Arc;

use ash::vk;
use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{Encoder, QueueRole, Rule};

use crate::device::{Shared, VulkanDevice, alive};
use crate::encoder::VulkanEncoder;
use crate::raw::{Raw, driver, unsupported};

/// A query pool of timestamps, destroyed in the drop of its last count: the [`Timestamps`]'
/// and each encoder's that recorded a write into it, which lasts until its submit is seen
/// complete.
pub(crate) struct QueryObj {
    raw: Arc<Raw>,
    pool: vk::QueryPool,
    count: u32,
}

impl Drop for QueryObj {
    fn drop(&mut self) {
        if self.raw.leaking() {
            return;
        }
        // SAFETY: the last count of the pool is gone, so no encoder that recorded a write
        // into it holds it, and no submit that writes it is pending.
        unsafe { self.raw.device.destroy_query_pool(self.pool, None) };
    }
}

/// A pool of timestamps for the encoders of one queue role: each written by
/// [`write`](Self::write) into an encoder, outside any pass, when every command recorded
/// before it in the queue's order has completed, and read by [`read`](Self::read) in
/// nanoseconds of the GPU's clock. The queue keeps [`valid_bits`](Self::valid_bits) of each,
/// and none where it writes no timestamps.
///
/// Both M1 drivers may latch a timestamp at a boundary of a Metal pass near its write rather
/// than where the queue's order puts it. The layout the device test holds to a lagging
/// dispatch and a lagging render pass under both is each of the two in an encoder submitted
/// alone, the work's submit between them, each submit waiting on the one before; the pair
/// reads the work with the queue's handoffs between the three submits. And
/// MoltenVK makes a result available in the command buffer's completion handler, which may
/// run a moment after a wait sees the submit's signal, so a reader polls
/// [`read`](Self::read) until the timestamps it wrote are `Some`.
pub struct Timestamps {
    shared: Arc<Shared>,
    role: QueueRole,
    bits: u32,
    query: Arc<QueryObj>,
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
        let shared = &self.shared;
        alive(&shared.records())?;
        if count == 0 {
            return Err(Error::new(ErrorKind::InvalidData, "a pool of no timestamps".to_owned()));
        }
        let raw = &shared.raw;
        if !raw.host_query_reset {
            return Err(unsupported("timestamps on a device with no hostQueryReset"));
        }
        let bits = shared.queue(shared.queue_of(role)).timestamp_bits;
        let info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(count);
        // SAFETY: the device is live and the info valid: a timestamp pool of `count` above
        // zero.
        let pool = unsafe { raw.device.create_query_pool(&info, None) }
            .map_err(|e| driver(e, "vkCreateQueryPool"))?;
        let query = Arc::new(QueryObj { raw: Arc::clone(raw), pool, count });
        // SAFETY: `hostQueryReset` is on, the queries are the pool's, and no command buffer
        // names the pool, which is this call's alone.
        unsafe { raw.device.reset_query_pool(pool, 0, count) };
        Ok(Timestamps { shared: Arc::clone(shared), role, bits, query })
    }
}

impl Timestamps {
    /// How many bits of a timestamp the role's queue keeps valid, from its family's
    /// `timestampValidBits`: zero where it writes none.
    #[must_use]
    pub fn valid_bits(&self) -> u32 {
        self.bits
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
        let any: &mut dyn Any = encoder;
        let Some(encoder) = any.downcast_mut::<VulkanEncoder>() else {
            return Err(Rule::Encoding.refuse("a timestamp into an encoder of another backend"));
        };
        if !Arc::ptr_eq(&encoder.shared, &self.shared) {
            return Err(Rule::Encoding.refuse("a timestamp into an encoder of another device"));
        }
        if encoder.role != self.role {
            return Err(Rule::Encoding.refuse(format!(
                "a timestamp of the {:?} role into an encoder of the {:?} role",
                self.role, encoder.role
            )));
        }
        if encoder.submitted.is_some() {
            return Err(Rule::Encoding.refuse("a timestamp into an encoder submitted, not reset"));
        }
        if encoder.open() {
            return Err(Rule::Encoding.refuse("a timestamp inside a pass"));
        }
        if index >= self.query.count {
            return Err(Rule::Encoding
                .refuse(format!("timestamp {index} of a pool of {}", self.query.count)));
        }
        if self.bits == 0 || !encoder.recording() {
            return Ok(());
        }
        let device = &self.shared.raw.device;
        // SAFETY: the command buffer is recording, outside any pass, and this encoder's alone
        // (`&mut`); the query is the pool's, which the encoder holds from here until its
        // submit is seen complete; the role's queue keeps valid bits, so its family writes
        // timestamps.
        unsafe {
            device.cmd_reset_query_pool(encoder.command_buffer, self.query.pool, index, 1);
            device.cmd_write_timestamp2(
                encoder.command_buffer,
                vk::PipelineStageFlags2::ALL_COMMANDS,
                self.query.pool,
                index,
            );
        }
        if !encoder.timestamps.iter().any(|held| Arc::ptr_eq(held, &self.query)) {
            encoder.timestamps.push(Arc::clone(&self.query));
        }
        Ok(())
    }

    /// Each timestamp, in nanoseconds of the GPU's clock (its ticks, masked to the valid
    /// bits, times the device's `timestampPeriod`), as its last write the GPU has
    /// completed left it; `None` for one unwritten, one the GPU has reset for a write it has
    /// not yet completed or not yet made available, and every one on a queue with no valid
    /// bits. Once it reads `Some` after the submit that wrote it is seen complete, a
    /// timestamp is that write's. Waits for nothing.
    ///
    /// A timestamp written again reads its previous write's value, as available, until the
    /// GPU runs the reset the new write recorded, which is a command of its submit: a `Some`
    /// read before that submit is seen complete may be the last round's. Read a reused pool
    /// only after its submit's signal is seen. [`elapsed`](Self::elapsed) gives the GPU time
    /// between two.
    ///
    /// # Errors
    ///
    /// A lost device's error; a driver's failure as its kind.
    pub fn read(&self) -> Result<Vec<Option<u64>>> {
        alive(&self.shared.records())?;
        let count = self.query.count as usize;
        if self.bits == 0 {
            return Ok(vec![None; count]);
        }
        let mut data = vec![[0_u64; 2]; count];
        let flags = vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WITH_AVAILABILITY;
        let device = &self.shared.raw.device;
        // SAFETY: the pool lives (`query`), its queries were reset on the host at its making,
        // and `data` holds a value and its availability, 64 bits each, for every query.
        let read = unsafe { device.get_query_pool_results(self.query.pool, 0, &mut data, flags) };
        match read {
            Ok(()) | Err(vk::Result::NOT_READY) => {}
            Err(error) => {
                self.shared.lost_on(error);
                return Err(driver(error, "vkGetQueryPoolResults"));
            }
        }
        let mask = if self.bits >= 64 { u64::MAX } else { (1_u64 << self.bits) - 1 };
        let scale = self.scale();
        Ok(data
            .iter()
            .map(|&[ticks, available]| {
                let nanos = (u128::from(ticks & mask) * scale) >> 32;
                (available != 0).then(|| u64::try_from(nanos).unwrap_or(u64::MAX))
            })
            .collect())
    }

    /// The GPU time from `earlier` to `later`, two values [`read`](Self::read) gave: their
    /// difference modulo the clock's range, the valid bits' scaled to nanoseconds, so a pair
    /// the clock wrapped between reads its span and not zero. To a nanosecond of the ticks'
    /// span, while less than one whole range of the clock lies between the two writes.
    #[must_use]
    pub fn elapsed(&self, earlier: u64, later: u64) -> Duration {
        Duration::from_nanos(elapsed_nanos(earlier, later, self.bits, self.scale()))
    }

    /// The device's `timestampPeriod` in 32.32 fixed point, so that a tick count of any size
    /// scales with no loss of its low bits, as an `f64` product would past 2^53, and a
    /// period of one gives the ticks themselves.
    fn scale(&self) -> u128 {
        let period = f64::from(self.shared.raw.properties.limits.timestamp_period);
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scale = (period * 4_294_967_296.0).round() as u128;
        scale
    }
}

/// `later` less `earlier`, nanoseconds of a clock of `bits` valid bits whose ticks scale by
/// `scale` (32.32), modulo the clock's whole range so scaled; a range of 64 bits where the
/// queue keeps none or all, so that no range is zero.
fn elapsed_nanos(earlier: u64, later: u64, bits: u32, scale: u128) -> u64 {
    let range = if bits == 0 || bits >= 64 { 1_u128 << 64 } else { 1_u128 << bits };
    let span = ((range * scale) >> 32).max(1);
    let (earlier, later) = (u128::from(earlier) % span, u128::from(later) % span);
    let nanos = if later >= earlier { later - earlier } else { later + span - earlier };
    u64::try_from(nanos).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::elapsed_nanos;

    /// A period of one, and of 41.666…, in 32.32.
    const ONE: u128 = 1 << 32;
    const SLOW: u128 = 178_956_970_667;

    #[test]
    fn a_pair_the_clock_wrapped_between_reads_its_span() {
        // Eight bits of ticks of a nanosecond: from 250 to 4 is ten ticks past the wrap.
        assert_eq!(elapsed_nanos(250, 4, 8, ONE), 10);
        assert_eq!(elapsed_nanos(4, 250, 8, ONE), 246);
        assert_eq!(elapsed_nanos(7, 7, 8, ONE), 0);
        // All 64 bits: a wrap of the whole `u64`.
        assert_eq!(elapsed_nanos(u64::MAX - 1, 3, 64, ONE), 5);
        // Ticks of 41.666… ns: tick 254 reads 10,583 ns and tick 2 reads 83 ns, and the
        // range is 10,666 ns: four ticks, to a nanosecond of their 166.
        let tick = |ticks: u128| u64::try_from((ticks * SLOW) >> 32).unwrap();
        let span = elapsed_nanos(tick(254), tick(2), 8, SLOW);
        assert!((166..=167).contains(&span), "{span}");
    }
}
