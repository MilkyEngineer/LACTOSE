//! The integration tests of `lactose-net-api` as one test binary (D150): each file beside this one
//! is a module of it, named after the file. `common`, the support they share, is declared once
//! here. A file whose name is a codename is a module named for what it holds (CLAUDE.md, Names):
//! `recorded` is `sour.rs`.

#![forbid(unsafe_code)]

mod common;

mod consumers;
mod fold;
mod gather;
#[path = "sour.rs"]
mod recorded;
mod relevance;
mod staged;
