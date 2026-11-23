//! The crate's integration tests, one binary (D150): what the facade can hold before
//! `graph-impl` gives it bodies. `consumers` compiles each consumer's use of the surface;
//! `values` runs the few bodies that are real and holds `README.md` to the surface.

#![forbid(unsafe_code)]

mod cache;
mod common;
mod consumers;
mod queues;
mod recording;
mod transients;
mod transitions;
mod values;
