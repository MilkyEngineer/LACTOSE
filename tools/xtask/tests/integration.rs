//! The integration tests of `lactose-xtask` as one test binary (D125): each file beside this one is
//! a module of it, named after the file. `harness_stand_in.rs` and `heavy.rs` are targets of their
//! own, for the reasons `Cargo.toml` gives.

#![forbid(unsafe_code)]

mod boundary_lints;
mod vendor;
mod workspace;
