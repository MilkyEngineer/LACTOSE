//! `tools/xtask/ci-policy.toml`: the data the checks hold that is not a dependency rule.
//!
//! The file's own comments say what each table means and which architecture section it
//! transcribes.

use serde::Deserialize;

use crate::POLICY_FILE;

#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
struct Crates {
    crates: Vec<String>,
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
struct Packages {
    packages: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    unsafe_crates: Crates,
    lint_file_carriers: Crates,
    feature_roots: Packages,
    harness: Packages,
}

/// The parsed policy file.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Policy {
    /// The crates that may contain `unsafe` (architecture section 21).
    pub unsafe_crates: Vec<String>,
    /// The crates outside the boundary that carry the boundary lint file.
    pub lint_file_carriers: Vec<String>,
    /// The shipped binaries, whose boundary feature trees must match (section 8.3).
    pub feature_roots: Vec<String>,
    /// The packages whose tests run under the determinism harness.
    pub harness_packages: Vec<String>,
}

impl Policy {
    /// Parses the text of the policy file.
    ///
    /// # Errors
    /// If the text is not TOML of the expected shape; unknown tables and keys are errors.
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawPolicy = toml::from_str(text).map_err(|e| format!("{POLICY_FILE}: {e}"))?;
        Ok(Self {
            unsafe_crates: raw.unsafe_crates.crates,
            lint_file_carriers: raw.lint_file_carriers.crates,
            feature_roots: raw.feature_roots.packages,
            harness_packages: raw.harness.packages,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checked_in_file_parses() {
        let policy = Policy::parse(include_str!("../ci-policy.toml")).unwrap();
        // Architecture section 21 lists nine crates.
        assert_eq!(policy.unsafe_crates.len(), 9);
        assert_eq!(policy.lint_file_carriers, ["lactose-reflect-macros", "lactose-ecs-macros"]);
    }

    #[test]
    fn an_unknown_table_is_an_error() {
        let text = format!("{}\n[surprise]\nx = 1\n", include_str!("../ci-policy.toml"));
        assert!(Policy::parse(&text).unwrap_err().contains("surprise"));
    }
}
