//! The integration tests of `lactose-asset` as one test binary (D125): each file beside this one is
//! a module of it, named after the file. `common`, the support they share, is declared once here.
//! `drop.rs` and `index.rs` are targets of their own, for the reason `Cargo.toml` gives.

#![forbid(unsafe_code)]

mod common;

mod baseline;
mod conformance;
mod failures;
mod generation;
mod locking;
mod mounts;
mod pack;
mod pack_set;
mod reads;
mod settling;
mod sketch;
mod spans;
