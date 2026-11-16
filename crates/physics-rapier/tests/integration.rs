//! The integration tests of `lactose-physics-rapier` as one test binary (D125): each file beside
//! this one is a module of it, named after the file. `common` and `support`, the support they
//! share, are declared once here. `memory.rs` is a target of its own, for the reason `Cargo.toml`
//! gives. A file whose name is a codename is a module named for what it holds (CLAUDE.md, Names):
//! `recorded` is `sour.rs`.

#![forbid(unsafe_code)]

#[path = "../../physics-api/tests/common/mod.rs"]
mod common;
mod support;

mod backend;
mod conformance;
mod copy;
mod cost;
mod determinism;
mod events;
mod pool;
mod queries;
#[path = "sour.rs"]
mod recorded;
mod reload;
mod scenes;
mod snapshot;
