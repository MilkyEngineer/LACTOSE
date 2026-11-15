//! The asset cooker: a directory of authored sources cooked into one pack of the store's
//! format. At milestone 1 it cooks one kind, a sim skeleton from each glTF source, read by
//! the cooker's own glTF reader over serde_json.
//!
//! Leaf tier, outside the determinism boundary; the binary is `ricotta`. See `README.md`,
//! which states the command line, the source schema and the cooked pack set as the contract,
//! and `SAFETY.md`.

#![forbid(unsafe_code)]

mod codec;
mod cook;
mod gltf;
mod skeleton;
mod space;

pub use cook::{Published, cook, pack_set};
pub use skeleton::SkeletonProcessor;
