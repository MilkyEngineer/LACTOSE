//! The stand-in simulation tests that `cargo xtask sour self-test` launches through nextest,
//! once per run configuration, exactly as it will launch real simulation tests.
//!
//! Under a plain `cargo nextest run` there is no session, so each runs once in the production
//! configuration and records nothing. None asserts anything about its hashes: comparing them
//! across configurations is the runner's job, and the flawed ones must reach it.
//!
//! Each test is named after its entry in `stand_in::STAND_INS`, which is how the self-test
//! selects it.

#![forbid(unsafe_code)]

use lactose_xtask::harness::protocol::Session;
use lactose_xtask::harness::stand_in::{self, Flaw};

fn run(flaw: Flaw) {
    let session = Session::from_env().expect("the SOUR environment is malformed");
    stand_in::run(&session, stand_in::test_name(flaw), flaw).expect("recording failed");
}

#[test]
fn sound() {
    run(Flaw::None);
}

#[test]
fn worker_order() {
    run(Flaw::WorkerOrder);
}

#[test]
fn layout_order() {
    run(Flaw::LayoutOrder);
}

#[test]
fn steal_order() {
    run(Flaw::StealOrder);
}

#[test]
fn unrepeatable() {
    run(Flaw::Unrepeatable);
}

#[test]
fn ignores_configuration() {
    run(Flaw::IgnoresConfiguration);
}
