//! The integration tests of `lactose-reflect` as one test binary (D150): each file beside this one
//! is a module of it, named after the file.

#![forbid(unsafe_code)]

mod compile_fail;
mod reflect;
mod schema_bytes;
