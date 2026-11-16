//! The integration tests of `lactose-xtask` as one test binary (D125): each file beside this one is
//! a module of it, named after the file. `harness_stand_in.rs` is a target of its own, for the
//! reason `Cargo.toml` gives.

#![forbid(unsafe_code)]

mod boundary_lints;
mod heavy;
mod vendor;
mod workspace;
