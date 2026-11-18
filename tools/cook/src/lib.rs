//! The asset cooker: a directory of authored sources cooked into one pack of the store's
//! format. It cooks two kinds: a sim skeleton from each glTF source, read by the cooker's own
//! glTF reader over serde_json, and at milestone 2 a shader for each entry point of a Slang
//! source and each target, compiled by the pinned `slangc` and on macOS Apple's `metal`, and
//! checked on what they emitted.
//!
//! Leaf tier, outside the determinism boundary; the binary is `ricotta`. See `README.md`,
//! which states the command line and the source schema as the contract, and `SAFETY.md`.
//! Which packs a process mounts, and in which order, is `lactose_asset::pack_set`.

#![forbid(unsafe_code)]

mod codec;
mod cook;
mod gltf;
mod json;
mod msl;
mod semantics;
mod shader;
mod skeleton;
mod space;
mod spirv;

pub use cook::{Published, SHADER_KIND, Shaders, cook};
pub use shader::Slang;
pub use skeleton::SkeletonProcessor;
