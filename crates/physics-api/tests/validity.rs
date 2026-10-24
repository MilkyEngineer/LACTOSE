//! The validity run against the test backend (`README.md`, "Validity"): the fake applies
//! this crate's rules and nothing of its own, so the run that holds
//! `lactose-physics-rapier` to them holds the fake too, and a test written against the
//! fake sees what a real backend does with a refused value.

#![forbid(unsafe_code)]

mod common;

use common::fake::{CallLog, Fake};
use lactose_physics_api::conformance::validity;

#[test]
fn the_fake_holds_to_the_validity_rules_under_every_pool_size() {
    for threads in [1, 4] {
        let fake = Fake::new(CallLog::default()).with_threads(threads);
        validity(Box::new(fake)).unwrap_or_else(|error| panic!("{threads} threads: {error}"));
    }
}
