//! The asset cooker: a directory of authored sources cooked into one pack of the store's
//! format. At milestone 1 it cooks one kind, a sim skeleton from each glTF source, read by
//! the cooker's own glTF reader over serde_json.
//!
//! Leaf tier, outside the determinism boundary; the binary is `ricotta`. See `README.md`,
//! which states the command line and the source schema as the contract, and `SAFETY.md`.
//! Which packs a process mounts, and in which order, is `lactose_asset::pack_set`.

#![forbid(unsafe_code)]

mod codec;
mod cook;
mod gltf;
mod json;
mod skeleton;
mod space;

pub use cook::{Published, cook};
pub use skeleton::SkeletonProcessor;
