//! The integration tests of `lactose-present-api` as one test binary (D150): each file beside
//! this one is a module of it, named after the file.

#![forbid(unsafe_code)]

mod common;
mod compile_fail;
mod consumers;
mod exchange;
mod extract;
mod frame;
mod registration;
mod rollback;
mod view;
