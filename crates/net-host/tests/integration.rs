//! The integration tests of `lactose-net-host` as one test binary (D125): each file beside this one
//! is a module of it, named after the file. `common`, the support they share, is declared once
//! here. `interpolation_alloc.rs` is a target of its own, for the reason `Cargo.toml` gives. A file
//! whose name is a codename is a module named for what it holds (CLAUDE.md, Names): `recorded` is
//! `sour.rs`.

#![forbid(unsafe_code)]

mod common;

mod consumers;
mod handles;
mod handshake;
mod interest;
mod interpolation;
mod quant;
#[path = "sour.rs"]
mod recorded;
mod replication;
