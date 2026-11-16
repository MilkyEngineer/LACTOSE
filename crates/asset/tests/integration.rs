//! The integration tests of `lactose-asset` as one test binary (D150): each file beside this one is
//! a module of it, named after the file. `common`, the support they share, is declared once here.
//! `drop.rs`, `failures.rs`, `index.rs`, `locking.rs`, `mounts.rs`, `pack.rs`, `reads.rs` and
//! `spans.rs` are targets of their own, for the reasons `Cargo.toml` gives.

#![forbid(unsafe_code)]

mod common;

mod baseline;
mod conformance;
mod generation;
mod pack_set;
mod settling;
mod sketch;
