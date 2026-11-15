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
//! | 2 | format version, currently 2 |
//! | 2 | zero |
//! | ... | the body, laid out as [`Baseline`] and [`Overlay`] describe |
//! | 8 | xxh64, seed zero, of every preceding byte |
//!
//! Every integer is little-endian and every count is a `u32`. A document has exactly one
//! encoding, and a decoder accepts nothing else, so the trailing hash is both an integrity
//! check and the document's content hash. Compression is applied outside, by `io` and
//! `persist`.
//!
//! A decoder refuses every other format version with `ErrorKind::VersionMismatch`. Format 2
//! added the layout digest to each schema ([`ComponentSchema::layout_digest`]); a format-1
//! document is refused, and is re-cooked or re-saved.
//!
//! Component and field bytes are opaque here: this crate stores them under their
//! [`ComponentKey`] and [`FieldId`](lactose_core::FieldId), with the [`ComponentSchema`]
//! that says how large they are and where each field lies, and never interprets them.
//! [`write_table`] and [`read_table`] are the schema table's encoding, for a container
//! that carries schemas beside its documents.

#![forbid(unsafe_code)]

mod baseline;
mod codec;
mod identity;
mod overlay;
mod schema;

pub use baseline::{Archetype, Baseline, BaselineRow, Column};
pub use identity::{AuthoredGuid, cooked_net_id};
use lactose_core::ComponentKey;
pub use overlay::{Overlay, Record};
pub use schema::{ComponentSchema, FieldLayout, read_table, write_table};

/// The largest component value a document may carry, in bytes. A component row must fit
/// one 16 KiB chunk (architecture section 6).
pub const MAX_COMPONENT_BYTES: u32 = 16 * 1024;

/// The largest encoded document, in bytes: 256 MiB. A decoder refuses anything longer
/// before reading it, and a constructor refuses to build what no decoder would accept.
///
/// Decoding allocates at most eight times the encoded length (see `README.md`), so this
/// is also a bound on what decoding an untrusted document can cost. A caller that expects
/// far smaller documents, such as `net` receiving an overlay, should apply its own smaller
/// limit to the byte length before calling `decode`.
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024 * 1024;

/// The bytes of one component of one entity, exactly as the component is laid out in
/// memory. Their length is the element size of the component's [`ComponentSchema`] in the
/// same document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ComponentData {
    /// The component.
    pub component: ComponentKey,
    /// The component's bytes.
    pub bytes: Vec<u8>,
}
