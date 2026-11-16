//! The integration tests of `lactose-input-host` as one test binary (D125): each file beside this
//! one is a module of it, named after the file.

#![forbid(unsafe_code)]

mod consumers;
mod golden;
mod latest;
mod quantise;
