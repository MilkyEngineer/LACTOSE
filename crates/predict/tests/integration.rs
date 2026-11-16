//! The integration tests of `lactose-predict` as one test binary (D125): each file beside this one
//! is a module of it, named after the file. `common`, the support they share, is declared once
//! here. A file whose name is a codename is a module named for what it holds (CLAUDE.md, Names):
//! `recorded` is `sour.rs`.

#![forbid(unsafe_code)]

mod common;

mod check;
mod claims;
mod consumers;
mod corrections;
mod cost;
mod hook;
mod logs;
mod physics;
mod presentation;
#[path = "sour.rs"]
mod recorded;
mod regions;
mod resync;
mod ring;
mod substitution;
