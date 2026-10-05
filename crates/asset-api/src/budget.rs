//! Streaming budget and priority values.
//!
//! These are the units the streaming system (milestone 3) is configured in. They describe
//! limits; they decide nothing. The numbers are chosen with the memory budgets of
//! architecture section 22.1.

/// How urgently a load is wanted. A store serves a higher priority first when its budgets
/// bind; among equal priorities the order is the store's own. The default is the lowest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct LoadPriority(pub u8);

/// A limit on the memory loaded assets may occupy.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MemoryBudget {
    /// Bytes of loaded asset data resident at once, summed over every published version
    /// the store holds.
    pub resident_bytes: u64,
}

/// A limit on the IO a store may issue for loading.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IoBudget {
    /// Bytes read from storage per second of real time, measured before decompression.
    pub bytes_per_second: u64,
    /// Read requests outstanding at once.
    pub requests_in_flight: u32,
}

/// The budgets one streaming configuration imposes on a store.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StreamingBudget {
    /// The memory limit.
    pub memory: MemoryBudget,
    /// The IO limit.
    pub io: IoBudget,
}
