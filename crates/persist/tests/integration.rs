//! The integration tests of `lactose-persist` as one test binary (D125): each file beside this one
//! is a module of it, named after the file. `common`, `glue` and `variants`, the support they
//! share, are declared once here. `container.rs` is a target of its own, for the reason
//! `Cargo.toml` gives.

#![forbid(unsafe_code)]

mod common;
mod glue;
mod variants;

mod binds;
mod diff;
mod format;
mod migrate;
mod physics;
mod rebase;
mod records;
mod reload;
mod roundtrip;
mod schema;
mod sketch;
