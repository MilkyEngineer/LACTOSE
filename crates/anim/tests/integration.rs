//! The integration tests of `lactose-anim` as one test binary (D125): each file beside this one is
//! a module of it, named after the file. `common`, the support they share, is declared once here. A
//! file whose name is a codename is a module named for what it holds (CLAUDE.md, Names): `recorded`
//! is `sour.rs`.

#![forbid(unsafe_code)]

mod common;

mod bones;
mod consumers;
mod cost;
mod evaluation;
mod events;
mod format;
mod machine;
mod phase;
mod prediction;
#[path = "sour.rs"]
mod recorded;
mod root_motion;
