//! `tools/xtask/ci-policy.toml`: the data the checks hold beyond `crate-tiers.toml`.
//!
//! The file's own comments say what each table means and which architecture section it
//! transcribes.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::POLICY_FILE;

/// A list of crate or package names.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
struct Names {
    crates: Vec<String>,
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
struct Packages {
    packages: Vec<String>,
}

/// The non-boundary workspace crates a boundary crate may depend on.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct InternalAllowlist {
    /// Permitted for every boundary crate.
    pub any: Vec<String>,
    /// Permitted for the named boundary crate alone.
    #[serde(default)]
    pub only: BTreeMap<String, Vec<String>>,
}

/// The third-party crates one boundary crate may depend on.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct ThirdPartyAllowlist {
    /// Permitted as a normal, build or dev dependency.
    #[serde(default)]
    pub normal: Vec<String>,
    /// Permitted as a dev-dependency only.
    #[serde(default)]
    pub dev: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    unsafe_crates: Names,
    boundary_internal: InternalAllowlist,
    #[serde(default)]
    boundary_third_party: BTreeMap<String, ThirdPartyAllowlist>,
    feature_roots: Packages,
    sour: Packages,
}

/// The parsed policy file.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Policy {
    /// The crates that may contain `unsafe` (architecture section 21).
    pub unsafe_crates: Vec<String>,
    /// The boundary allowlist for workspace crates (architecture section 8.2).
    pub boundary_internal: InternalAllowlist,
    /// The boundary allowlist for third-party crates, keyed by boundary crate.
    pub boundary_third_party: BTreeMap<String, ThirdPartyAllowlist>,
    /// The packages whose boundary feature trees must match (architecture section 8.3).
    pub feature_roots: Vec<String>,
    /// The packages whose tests run under SOUR.
    pub sour_packages: Vec<String>,
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
            boundary_internal: raw.boundary_internal,
            boundary_third_party: raw.boundary_third_party,
            feature_roots: raw.feature_roots.packages,
            sour_packages: raw.sour.packages,
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
        assert!(policy.boundary_internal.only["lactose-ecs"].contains(&"lactose-task".to_owned()));
        assert_eq!(policy.boundary_third_party["lactose-math"].normal, ["libm"]);
    }

    #[test]
    fn an_unknown_table_is_an_error() {
        let text = format!("{}\n[surprise]\nx = 1\n", include_str!("../ci-policy.toml"));
        assert!(Policy::parse(&text).unwrap_err().contains("surprise"));
    }
}
