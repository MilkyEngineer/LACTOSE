//! The integration tests of `lactose-task` as one test binary (D150): each file beside this one is
//! a module of it, named after the file. `common`, the support they share, is declared once here.
//! `idle_cpu.rs` and `spans.rs` are targets of their own, for the reasons `Cargo.toml` gives.

mod common;

mod abort;
mod block_in_place;
mod chaos;
mod graph;
mod jobs;
mod lanes;
mod scope;
mod stress;
mod tasks;
