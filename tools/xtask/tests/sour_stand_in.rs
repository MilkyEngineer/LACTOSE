//! The stand-in simulation tests that `cargo xtask sour self-test` launches through nextest,
//! once per run configuration, exactly as it will launch real simulation tests.
//!
//! Under a plain `cargo nextest run` there is no session, so each runs once in the production
//! configuration and records nothing. Neither asserts anything about its hashes: comparing
//! them across configurations is the runner's job, and the flawed one must reach it.

#![forbid(unsafe_code)]

use lactose_xtask::sour::protocol::Session;
use lactose_xtask::sour::stand_in::{self, Flaw};

#[test]
fn sound() {
    let session = Session::from_env().expect("the SOUR environment is malformed");
    stand_in::run(&session, stand_in::SOUND_TEST, Flaw::None).expect("recording failed");
}

#[test]
fn order_dependent() {
    let session = Session::from_env().expect("the SOUR environment is malformed");
    stand_in::run(&session, stand_in::FLAWED_TEST, Flaw::OrderDependent).expect("recording failed");
}
