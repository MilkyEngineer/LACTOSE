//! The integration tests of `lactose-ecs` as one test binary (D125): each file beside this one is a
//! module of it, named after the file. `common`, the support they share, is declared once here.
//! `alloc.rs`, `channel_skip.rs`, `ownership.rs` and `units.rs` are targets of their own, for the
//! reasons `Cargo.toml` gives.

mod common;

mod binds;
mod bulk;
mod change;
mod claims;
mod commands;
mod compile_fail;
mod erased;
mod erased_rows;
mod events;
mod fork;
mod layout;
mod obligation;
mod pass;
mod predicted;
mod query;
mod regions;
mod regressions;
mod regressions_two;
mod reload;
mod schedule_errors;
mod snapshot;
mod structure;
mod systems;
mod wrap;
mod wrap_program;
