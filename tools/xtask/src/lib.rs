//! The CI driver: the tier check, the static boundary checks, the feature-tree comparison, the
//! check of crates built from a patched copy and the mode runner of the determinism harness
//! (architecture sections 3, 8.2, 8.3 and 21).
//!
//! Leaf tier, outside the determinism boundary. See `README.md`.
//!
//! Every check is a pure function from parsed inputs to a list of [`Finding`]s. The binary in
//! `main.rs` only gathers the inputs (the tier file, `cargo metadata`, file contents) and
//! prints what the checks return. [`heavy`] is not a check: it bounds how many heavy
//! commands run at once on one machine.

#![forbid(unsafe_code)]

pub mod boundary;
pub mod features;
pub mod harness;
pub mod heavy;
pub mod policy;
pub mod repo;
mod rust_source;
pub mod tiers;
pub mod vendor;
pub mod workspace;

use std::fmt;

/// One violation of a rule, printed as `path: message`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Finding {
    /// The file the finding is about, relative to the repository root, with `/` separators.
    pub path: String,
    /// What is wrong and, where it is not obvious, what would make it right.
    pub message: String,
}

impl Finding {
    /// A finding about `path`.
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { path: path.into(), message: message.into() }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// The repository-relative path of the tier file, which holds every dependency rule.
pub const TIER_FILE: &str = "crate-tiers.toml";

/// The repository-relative path of the data the checks hold that is not a dependency rule.
pub const POLICY_FILE: &str = "tools/xtask/ci-policy.toml";
