//! How many threads the pool runs.

use core::num::NonZero;

use lactose_task::{CoreTopology, Platform};

/// The pool's configuration.
///
/// The one setting is the thread count. Architecture section 9.7 sizes the server's pool at
/// half the cores, separate from the background workers, so a region publish never waits
/// behind another; [`IoConfig::for_platform`] is that rule, and a client uses the same
/// rule until a measurement says otherwise (`docs/verification.md`). `app` may set the
/// count directly with [`IoConfig::new`]. `#[non_exhaustive]`, so a later setting (a
/// backend, a priority) is a new field with a default and no caller's code changes.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IoConfig {
    /// Threads in the pool. Each runs one request at a time, from start to finish, so this
    /// is also the number of requests the pool runs at once.
    pub pool_threads: NonZero<u32>,
}

impl IoConfig {
    /// A pool of `pool_threads` threads, the count `app` chose itself.
    #[must_use]
    pub const fn new(pool_threads: NonZero<u32>) -> Self {
        Self { pool_threads }
    }

    /// Section 9.7's rule from the platform's topology: see [`IoConfig::for_topology`].
    #[must_use]
    pub fn for_platform(platform: &dyn Platform) -> Self {
        Self::for_topology(platform.topology())
    }

    /// Section 9.7's rule: half the cores, performance and efficiency cores together, an
    /// odd count rounded up, and at least one. On a 4+4 Apple M1 that is four threads; on
    /// a homogeneous sixteen-core server, which the topology reports as fifteen, eight;
    /// on a four-vCPU CI runner, one.
    #[must_use]
    pub fn for_topology(topology: CoreTopology) -> Self {
        let cores = topology.performance.saturating_add(topology.efficiency);
        let threads = cores.div_ceil(2).max(1);
        Self::new(NonZero::new(threads).expect("at least one"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_the_cores_rounded_up_at_least_one() {
        let threads = |performance, efficiency| {
            IoConfig::for_topology(CoreTopology { performance, efficiency }).pool_threads.get()
        };
        assert_eq!(threads(4, 4), 4);
        assert_eq!(threads(15, 0), 8);
        assert_eq!(threads(1, 0), 1);
        assert_eq!(threads(0, 0), 1);
        assert_eq!(threads(8, 16), 12);
        assert_eq!(threads(u32::MAX, 1), u32::MAX / 2 + 1);
    }
}
