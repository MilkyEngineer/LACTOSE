//! The one integration test binary of `lactose-renderer` (D150): every other file in this
//! directory is a module declared here, so the package links one test executable.

#![forbid(unsafe_code)]

mod camera;
mod common;
mod consumers;
mod cost;
mod device;
mod frames;
mod sequences;
mod slots;
mod values;
mod writers;
