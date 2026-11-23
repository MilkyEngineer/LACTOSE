//! The integration tests of `lactose-platform` as one test binary (D150): each file beside
//! this one is a module of it, named after the file. None opens a window; those that do are
//! `tests/window.rs`, a binary of its own.

#![forbid(unsafe_code)]

mod consumers;
mod present;
mod threads;
mod values;
