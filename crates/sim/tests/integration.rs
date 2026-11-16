//! The integration tests of `lactose-sim` as one test binary (D150): each file beside this one is a
//! module of it, named after the file. `common` and `glue`, the support they share, are declared
//! once here. A file whose name is a codename is a module named for what it holds (CLAUDE.md,
//! Names): `recorded` is `sour.rs` and `readme_glue` is `sour_glue.rs`.

#![forbid(unsafe_code)]

mod common;
mod glue;

mod check;
mod consumers;
mod cost;
mod hash;
mod layout;
mod persist;
#[path = "sour_glue.rs"]
mod readme_glue;
#[path = "sour.rs"]
mod recorded;
mod refusals;
mod regions;
mod runs;
mod sync_point;
