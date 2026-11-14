//! The milestone 1 proof of the assembly (`README.md`, "Proof for milestone 1"), one test
//! binary: every test asserts on counts, orders, bytes and hashes, never on timing, and
//! every pacer is driven by a stand-in clock (instants computed from one base, never read
//! between frames). The headless app's rows (a run is a function of its frames, the save
//! round trip through a file, the chaos log at shutdown) and the SOUR test are in
//! `tests/headless.rs`.

#![forbid(unsafe_code)]

mod client;
mod common;
mod registration;
mod server;
mod store;
