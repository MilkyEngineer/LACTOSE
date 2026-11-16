//! The integration tests of `lactose-app` as one test binary (D125): each file beside this one is a
//! module of it, named after the file. `protocol`, the support they share, is declared once here.
//! `proof/` is a target of its own, for the reason `Cargo.toml` gives.

#![forbid(unsafe_code)]

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "the boundary lints these expectations name are not configured outside the boundary"
)]
#[allow(dead_code, reason = "the runner's module as written; a test uses what it needs")]
mod protocol;

mod cost;
mod headless;
mod schema_record;
mod spike;
