//! The integration tests of `lactose-present-api` as one test binary (D150): each file beside
//! this one is a module of it, named after the file. A file whose name is a codename is a
//! module named for what it holds (CLAUDE.md, Names): `recorded` is `sour.rs`.

#![forbid(unsafe_code)]

mod common;
mod compile_fail;
mod consumers;
mod exchange;
mod extract;
mod frame;
#[path = "sour.rs"]
mod recorded;
mod registration;
mod rollback;
mod view;
