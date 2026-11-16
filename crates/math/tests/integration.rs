//! The integration tests of `lactose-math` as one test binary (D125): each file beside this one is
//! a module of it, named after the file. `common`, the support they share, is declared once here.

#![forbid(unsafe_code)]

mod common;

mod golden;
mod reduce;
mod select;
mod types;
