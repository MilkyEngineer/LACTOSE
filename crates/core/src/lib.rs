//! Shared vocabulary of the engine: identity keys, handles, deterministic maps, arenas, the
//! tagged allocator, the error taxonomy and thread classes.
//!
//! Foundation tier, inside the determinism boundary. See `README.md` and `SAFETY.md`.

pub mod alloc;
pub mod arena;
pub mod array;
pub mod error;
pub mod handle;
pub mod hash;
pub mod id;
pub mod map;
pub mod thread_class;

pub use alloc::{AllocTag, TagScope, TagStats, TaggedAlloc};
pub use arena::Arena;
pub use array::PlainArray;
pub use error::{Error, ErrorKind, Result};
pub use handle::{Handle, HandleTable};
pub use hash::xxh64;
pub use id::{
    ComponentKey, EntityId, EntityRef, FieldId, NetId, PlayerId, PredictionKey, StableKey,
    SystemKey, Tick,
};
pub use map::{FixedHashMap, FixedHasher};
pub use thread_class::ThreadClass;
