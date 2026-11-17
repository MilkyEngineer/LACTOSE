//! The crate's integration tests, one binary (D150): what the facade can hold before any
//! backend exists. `consumers` compiles each consumer's use of the surface; `tables`
//! runs the few bodies that are real.

#![forbid(unsafe_code)]

mod consumers;
mod tables;
