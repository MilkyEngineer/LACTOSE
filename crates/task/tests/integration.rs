//! The integration tests of `lactose-task` as one test binary (D125): each file beside this one is
//! a module of it, named after the file. `common`, the support they share, is declared once here.

mod common;

mod abort;
mod block_in_place;
mod chaos;
mod graph;
mod idle_cpu;
mod lanes;
mod scope;
mod spans;
mod stress;
mod tasks;
