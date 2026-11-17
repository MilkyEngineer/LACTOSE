//! The integration tests of `lactose-present-api` as one test binary (D150): each file beside
//! this one is a module of it, named after the file.

#![forbid(unsafe_code)]

mod common;
mod consumers;
mod extract;
mod registration;
