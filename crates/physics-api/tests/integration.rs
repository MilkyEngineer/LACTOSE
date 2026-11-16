//! The integration tests of `lactose-physics-api` as one test binary (D125): each file beside this
//! one is a module of it, named after the file. `common`, the support they share, is declared once
//! here. A file whose name is a codename is a module named for what it holds (CLAUDE.md, Names):
//! `recorded` is `sour.rs`.

#![forbid(unsafe_code)]

mod common;

mod bodies;
mod commands;
mod consumers;
mod cost;
mod events;
mod joints;
mod queries;
#[path = "sour.rs"]
mod recorded;
mod reload;
mod rows;
mod snapshot;
mod types;
mod validity;
