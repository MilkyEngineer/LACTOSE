//! Streaming priority and budget values.
//!
//! These are inputs to the residency and budget methods that milestone 3 adds to the store
//! traits. They describe limits; they decide nothing. Only shapes that the architecture
//! already determines are here: an IO budget is not, and is added with the streaming system
//! that measures what it needs.

/// How urgently a load is wanted. A store serves a higher priority first when its budgets
/// bind; among equal priorities the order is the store's own. The default is the lowest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct LoadPriority(pub u8);

/// A limit on the memory loaded assets may occupy (architecture section 22.1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MemoryBudget {
    /// Bytes of loaded asset data resident at once, summed over every published version
    /// the store holds.
    pub resident_bytes: u64,
}
