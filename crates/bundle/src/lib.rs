//! The region bundle format shared by the cooker, the asset store, saves and the network:
//! cooked identity, [`Baseline`] rows and [`Overlay`] records, with one canonical binary
//! encoding each.
//!
//! Foundation tier, inside the determinism boundary. See `README.md`.
//!
//! # Encoding
//!
//! Both documents share a frame:
//!
//! | Bytes | Contents |
//! |---|---|
//! | 4 | magic: `LBAS` for a baseline, `LOVL` for an overlay |
//! | 2 | format version, currently 1 |
//! | 2 | zero |
//! | ... | the body, laid out as [`Baseline`] and [`Overlay`] describe |
//! | 8 | xxh64, seed zero, of every preceding byte |
//!
//! Every integer is little-endian and every count or length is a `u32`. A document has
//! exactly one encoding, and a decoder accepts nothing else, so the trailing hash is both
//! an integrity check and the document's content hash. Compression is applied outside, by
//! `io` and `persist`.
//!
//! Component and field bytes are opaque here: this crate stores them under their
//! [`ComponentKey`] and [`FieldId`](lactose_core::FieldId) and
//! never interprets them.

#![forbid(unsafe_code)]

mod baseline;
mod codec;
mod identity;
mod overlay;

pub use baseline::{Archetype, Baseline, BaselineRow, Column};
pub use identity::{PlacementGuid, cooked_net_id};
use lactose_core::ComponentKey;
pub use overlay::{Overlay, Record};

/// The largest component value, and so the largest field value, a document may carry, in
/// bytes. A component row must fit one 16 KiB chunk (architecture section 6).
pub const MAX_COMPONENT_BYTES: u32 = 16 * 1024;

/// The bytes of one component of one entity, exactly as the component is laid out in
/// memory.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ComponentData {
    /// The component.
    pub component: ComponentKey,
    /// The schema version of the component when the bytes were written.
    pub schema_version: u32,
    /// The component's bytes, at most [`MAX_COMPONENT_BYTES`] of them.
    pub bytes: Vec<u8>,
}
