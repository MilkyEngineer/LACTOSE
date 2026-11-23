//! The crate's integration tests, one binary (D150). `cache`, `queues`, `recording`,
//! `transients` and `transitions` hold the graph on null (`README.md`, "What `graph-impl`
//! proves on null"); `consumers` compiles each consumer's use of the surface; `values` holds
//! the plain values and `README.md` to the surface. `device` runs the graph on a Vulkan
//! device: a plain run ignores its tests, and `cargo xtask gpu` runs them.

#![forbid(unsafe_code)]

mod cache;
mod common;
mod consumers;
mod device;
mod queues;
mod recording;
mod transients;
mod transitions;
mod values;
