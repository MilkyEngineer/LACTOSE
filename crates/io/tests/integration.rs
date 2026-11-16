//! The integration tests of `lactose-io` as one test binary (D150): each file beside this one is a
//! module of it, named after the file. `common`, the support they share, is declared once here.
//! `requests.rs` and `spans.rs` are targets of their own, for the reasons `Cargo.toml` gives.

#![forbid(unsafe_code)]

mod common;

mod sketch;
mod storage_full;
