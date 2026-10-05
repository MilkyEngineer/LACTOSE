//! The static boundary checks: the facts about files and manifests that architecture
//! sections 8.2, 8.3, 20 and 21 require and that need no compiler to verify.
//!
//! What clippy enforces through `clippy.toml` is not repeated here; this module checks that
//! the configuration clippy reads is the agreed one and has not been switched off wholesale.

use crate::policy::Policy;
use crate::repo::{Files, is_build_configuration, is_cargo_config};
use crate::tiers::TierFile;
use crate::workspace::{DepKind, Package, Workspace};
use crate::{Finding, POLICY_FILE};

/// The boundary lint configuration every boundary crate copies byte for byte.
pub const REFERENCE_CLIPPY: &str = "crates/core/clippy.toml";

/// The cargo configuration file that carries the x86-64 pin.
pub const CARGO_CONFIG: &str = ".cargo/config.toml";

/// The `[target]` key and the flag of the pin (architecture section 8.2).
const PIN_KEY: &str = r#"cfg(target_arch = "x86_64")"#;
const PIN_CPU: &str = "x86-64-v3";

/// The lints a boundary crate may only lift for one item at a time.
const BOUNDARY_LINTS: [&str; 2] = ["clippy::disallowed_methods", "clippy::disallowed_types"];

/// Runs every static boundary check.
#[must_use]
pub fn check(tiers: &TierFile, policy: &Policy, ws: &Workspace, files: &Files) -> Vec<Finding> {
    let mut findings = Vec::new();
    for package in &ws.packages {
        readme(package, files, &mut findings);
        unsafe_policy(policy, package, files, &mut findings);
        if tiers.in_boundary(&package.name) {
            clippy_copy(package, files, &mut findings);
            blanket_lint_exemptions(package, files, &mut findings);
            dependency_allowlist(tiers, policy, package, &mut findings);
        }
    }
    target_feature_pin(files, &mut findings);
    findings
}

/// Architecture section 20: each crate's `README.md` states responsibility, tier, boundary
/// status and unsafe status. Only its presence is checked.
fn readme(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let path = format!("{}/README.md", package.dir);
    if !files.contains(&path) {
        findings.push(Finding::new(path, format!("`{}` has no README.md", package.name)));
    }
}

/// Architecture section 21: `unsafe` is confined to the listed crates, each of which carries
/// `SAFETY.md`; every other crate forbids it in each library, proc-macro and binary root.
fn unsafe_policy(policy: &Policy, package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    if policy.unsafe_crates.contains(&package.name) {
        let path = format!("{}/SAFETY.md", package.dir);
        if !files.contains(&path) {
            findings.push(Finding::new(
                path,
                format!(
                    "`{}` is on the unsafe list (architecture section 21) and has no SAFETY.md",
                    package.name
                ),
            ));
        }
        return;
    }
    for root in &package.crate_roots {
        let forbids = files
            .text(root)
            .is_some_and(|source| inner_attributes(&source).iter().any(|a| forbids_unsafe(a)));
        if !forbids {
            findings.push(Finding::new(
                root,
                format!(
                    "`{}` is not on the unsafe list (architecture section 21), so this crate \
                     root needs `#![forbid(unsafe_code)]`",
                    package.name
                ),
            ));
        }
    }
}

/// Architecture section 8.3: one `clippy.toml` per boundary crate, all identical.
fn clippy_copy(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let path = format!("{}/clippy.toml", package.dir);
    let Some(reference) = files.bytes(REFERENCE_CLIPPY) else {
        findings.push(Finding::new(
            REFERENCE_CLIPPY,
            "the reference boundary lint configuration is missing",
        ));
        return;
    };
    match files.bytes(&path) {
        None => findings.push(Finding::new(
            path,
            format!(
                "boundary crate `{}` has no clippy.toml; copy {REFERENCE_CLIPPY}",
                package.name
            ),
        )),
        Some(copy) if copy != reference => findings.push(Finding::new(
            path,
            format!("differs from {REFERENCE_CLIPPY}; the two must be byte-identical"),
        )),
        Some(_) => {}
    }
}

/// A boundary crate lifts a boundary lint with an `#[expect]` on the one item that needs it.
/// An inner attribute lifts it for a whole crate or module, which is what this rejects.
fn blanket_lint_exemptions(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let src = format!("{}/src/", package.dir);
    let sources = files
        .paths()
        .filter(|path| path.ends_with(".rs"))
        .filter(|path| path.starts_with(&src) || package.crate_roots.iter().any(|r| r == path));
    for path in sources {
        let Some(source) = files.text(path) else {
            continue;
        };
        for attribute in inner_attributes(&source) {
            let lifts = attribute.contains("allow(") || attribute.contains("expect(");
            for lint in BOUNDARY_LINTS {
                if lifts && attribute.contains(lint) {
                    findings.push(Finding::new(
                        path,
                        format!(
                            "`#![{attribute}]` lifts `{lint}` for a whole crate or module; use \
                             `#[expect({lint}, reason = \"...\")]` on the one item that needs it"
                        ),
                    ));
                }
            }
        }
    }
}

/// Architecture section 8.2: the edges out of the boundary are the ones the policy file names.
fn dependency_allowlist(
    tiers: &TierFile,
    policy: &Policy,
    package: &Package,
    findings: &mut Vec<Finding>,
) {
    let internal = &policy.boundary_internal;
    let third_party = policy.boundary_third_party.get(&package.name);
    for dep in &package.deps {
        let allowed = if dep.workspace {
            // The tier check owns workspace dev-dependencies.
            dep.kind == DepKind::Dev
                || tiers.in_boundary(&dep.name)
                || internal.any.contains(&dep.name)
                || internal.only.get(&package.name).is_some_and(|only| only.contains(&dep.name))
        } else {
            third_party.is_some_and(|list| {
                list.normal.contains(&dep.name)
                    || (dep.kind == DepKind::Dev && list.dev.contains(&dep.name))
            })
        };
        if allowed {
            continue;
        }
        let (what, table) = if dep.workspace {
            ("non-boundary crate", "[boundary_internal]".to_owned())
        } else {
            ("third-party crate", format!("[boundary_third_party.\"{}\"]", package.name))
        };
        let section = match dep.kind {
            DepKind::Normal => "depends",
            DepKind::Build => "build-depends",
            DepKind::Dev => "dev-depends",
        };
        findings.push(Finding::new(
            package.manifest(),
            format!(
                "boundary crate `{}` {section} on {what} `{}`, which {table} in {POLICY_FILE} \
                 does not allow",
                package.name, dep.name
            ),
        ));
    }
}

/// Architecture section 8.2: the x86-64-v3 pin is present, nothing replaces it through
/// `RUSTFLAGS`, and nothing selects another CPU.
///
/// Only whole-line `#` comments are skipped. A trailing comment is scanned as if it were
/// code, which errs towards a finding.
fn target_feature_pin(files: &Files, findings: &mut Vec<Finding>) {
    for path in files.paths().filter(|path| is_build_configuration(path)) {
        let Some(text) = files.text(path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let line_number = index + 1;
            if line.contains("RUSTFLAGS") {
                findings.push(Finding::new(
                    path,
                    format!(
                        "line {line_number} names RUSTFLAGS; the environment variable replaces \
                         the {PIN_CPU} pin in {CARGO_CONFIG} instead of adding to it"
                    ),
                ));
            }
            for cpu in target_cpus(line) {
                if cpu == "native" {
                    findings.push(Finding::new(
                        path,
                        format!("line {line_number} sets target-cpu=native, which is denied"),
                    ));
                } else if !(is_cargo_config(path) && cpu == PIN_CPU) {
                    findings.push(Finding::new(
                        path,
                        format!(
                            "line {line_number} sets target-cpu={cpu}; the only permitted \
                             setting is the {PIN_CPU} pin in a cargo configuration file"
                        ),
                    ));
                }
            }
        }
    }
    if !pin_is_present(files) {
        findings.push(Finding::new(
            CARGO_CONFIG,
            format!(
                "the x86-64 pin is missing: [target.'{PIN_KEY}'] must set rustflags \
                 [\"-C\", \"target-cpu={PIN_CPU}\"]"
            ),
        ));
    }
}

/// The values of every `target-cpu=` on the line.
fn target_cpus(line: &str) -> impl Iterator<Item = &str> {
    line.split("target-cpu=").skip(1).map(|rest| {
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
            .unwrap_or(rest.len());
        &rest[..end]
    })
}

fn pin_is_present(files: &Files) -> bool {
    let Some(text) = files.text(CARGO_CONFIG) else {
        return false;
    };
    let Ok(config) = toml::from_str::<toml::Table>(&text) else {
        return false;
    };
    let flags = config
        .get("target")
        .and_then(|target| target.get(PIN_KEY))
        .and_then(|table| table.get("rustflags"));
    let tokens: Vec<&str> = match flags {
        Some(toml::Value::Array(items)) => items.iter().filter_map(toml::Value::as_str).collect(),
        Some(toml::Value::String(line)) => line.split_whitespace().collect(),
        _ => return false,
    };
    let pin = format!("target-cpu={PIN_CPU}");
    tokens.windows(2).any(|pair| pair[0] == "-C" && pair[1] == pin)
        || tokens.iter().any(|token| token.strip_prefix("-C") == Some(&pin))
}

/// Whether an inner attribute, as [`inner_attributes`] returns it, is `forbid(unsafe_code)`,
/// alone or among other lints. A `cfg_attr` does not count: it forbids only sometimes.
fn forbids_unsafe(attribute: &str) -> bool {
    attribute
        .strip_prefix("forbid(")
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(|lints| lints.split(',').any(|lint| lint == "unsafe_code"))
}

/// The inner attributes (`#![...]`) of a Rust source file, without the `#![` and `]` and with
/// all whitespace removed. Comments, string literals and character literals are skipped, so
/// an attribute that is only quoted or commented out is not returned.
fn inner_attributes(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
    let mut attributes = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if at(i + 1) == b'/' => {
                i = source[i..].find('\n').map_or(bytes.len(), |n| i + n);
            }
            b'/' if at(i + 1) == b'*' => i = skip_block_comment(bytes, i),
            b'"' => i = skip_string(bytes, i),
            // `r"…"`, and `br"…"` whose `b` was stepped over as an ordinary byte.
            b'r' if !is_ident(at(i.wrapping_sub(1)))
                || (at(i.wrapping_sub(1)) == b'b' && !is_ident(at(i.wrapping_sub(2)))) =>
            {
                i = skip_raw_string(bytes, i).unwrap_or(i + 1);
            }
            b'\'' => i = skip_char_or_lifetime(source, i),
            b'#' => {
                let mut j = skip_whitespace(bytes, i + 1);
                if at(j) == b'!' {
                    j = skip_whitespace(bytes, j + 1);
                    if at(j) == b'['
                        && let Some(end) = skip_brackets(bytes, j)
                    {
                        let body = &source[j + 1..end - 1];
                        attributes.push(body.chars().filter(|c| !c.is_whitespace()).collect());
                        i = end;
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    attributes
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn skip_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

/// `i` is at `/*`. Returns the index after the matching `*/`; block comments nest.
fn skip_block_comment(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1)) {
            (b'/', Some(b'*')) => {
                depth += 1;
                i += 2;
            }
            (b'*', Some(b'/')) => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    break;
                }
            }
            _ => i += 1,
        }
    }
    i
}

/// `i` is at an opening `"`. Returns the index after the closing one.
fn skip_string(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// `i` is at an `r` that may open a raw string (`r"…"`, `r#"…"#`). Returns the index after
/// the literal, or `None` if no raw string starts here.
fn skip_raw_string(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while bytes.get(j) == Some(&b'#') {
        j += 1;
    }
    let hashes = j - (i + 1);
    if bytes.get(j) != Some(&b'"') {
        return None;
    }
    j += 1;
    while j < bytes.len() {
        if bytes[j] == b'"'
            && bytes[j + 1..].iter().take(hashes).filter(|b| **b == b'#').count() == hashes
        {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(bytes.len())
}

/// `i` is at a `'`. Returns the index after a character literal, or after the quote alone if
/// it starts a lifetime.
fn skip_char_or_lifetime(source: &str, i: usize) -> usize {
    let rest = &source[i + 1..];
    if let Some(escaped) = rest.strip_prefix('\\') {
        // The character after the backslash may itself be a quote; the literal closes at the
        // next quote after it.
        let mut chars = escaped.char_indices().skip(1);
        return chars.find(|&(_, c)| c == '\'').map_or(source.len(), |(n, _)| i + 2 + n + 1);
    }
    match rest.chars().next() {
        Some(c) if rest[c.len_utf8()..].starts_with('\'') => i + 1 + c.len_utf8() + 1,
        _ => i + 1,
    }
}

/// `i` is at `[`. Returns the index after the matching `]`, skipping string literals, or
/// `None` if the bracket never closes.
fn skip_brackets(bytes: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::{package, workspace};
    use DepKind::{Build, Dev, Normal};

    const TIERS: &str = r#"
        [foundation]
        crates = ["lactose-core", "lactose-math", "lactose-task", "lactose-ecs", "lactose-reflect",
                  "lactose-ecs-macros", "lactose-io"]
        [backend]
        crates = []
        [leaf]
        crates = ["lactose-xtask"]
        [game]
        crates = []
        [boundary]
        crates = ["lactose-core", "lactose-math", "lactose-ecs"]
        [server_denylist]
        crates = []
    "#;

    const PIN: &str = "[target.'cfg(target_arch = \"x86_64\")']\nrustflags = [\"-C\", \"target-cpu=x86-64-v3\"]\n";
    const FORBID: &str = "//! Docs.\n#![forbid(unsafe_code)]\n";

    fn policy() -> Policy {
        Policy::parse(include_str!("../ci-policy.toml")).unwrap()
    }

    /// A conforming repository: `core` (unsafe, boundary), `math` (boundary), `task` (unsafe).
    fn conforming() -> (Workspace, Files) {
        let ws = workspace(vec![
            package("lactose-core", &[("bytemuck", Normal), ("xxhash-rust", Dev)]),
            package("lactose-math", &[("lactose-core", Normal), ("libm", Normal)]),
            package("lactose-task", &[("lactose-core", Normal), ("crossbeam-deque", Normal)]),
        ]);
        let mut files = Files::default();
        files.insert(CARGO_CONFIG, PIN);
        files.insert(REFERENCE_CLIPPY, "disallowed-types = []\n");
        files.insert("crates/math/clippy.toml", "disallowed-types = []\n");
        for role in ["core", "math", "task"] {
            files.insert(format!("crates/{role}/README.md"), "# readme\n");
        }
        files.insert("crates/core/SAFETY.md", "# safety\n");
        files.insert("crates/task/SAFETY.md", "# safety\n");
        files.insert("crates/core/src/lib.rs", "pub fn f() {}\n");
        files.insert("crates/task/src/lib.rs", "pub fn f() {}\n");
        files.insert("crates/math/src/lib.rs", FORBID);
        (ws, files)
    }

    fn run(ws: &Workspace, files: &Files) -> Vec<String> {
        let tiers = TierFile::parse(TIERS).unwrap();
        check(&tiers, &policy(), ws, files).iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_conforming_repository_has_no_findings() {
        let (ws, files) = conforming();
        assert_eq!(run(&ws, &files), [] as [&str; 0]);
    }

    #[test]
    fn missing_documents_are_reported() {
        let (ws, files) = conforming();
        let mut without = Files::default();
        for path in files.paths() {
            if path != "crates/task/SAFETY.md" && path != "crates/math/README.md" {
                without.insert(path, files.bytes(path).unwrap());
            }
        }
        assert_eq!(
            run(&ws, &without),
            [
                "crates/math/README.md: `lactose-math` has no README.md",
                "crates/task/SAFETY.md: `lactose-task` is on the unsafe list (architecture \
                 section 21) and has no SAFETY.md",
            ]
        );
    }

    #[test]
    fn a_crate_off_the_unsafe_list_must_forbid_unsafe() {
        let (ws, mut files) = conforming();
        for source in [
            "pub fn f() {}\n",
            "// #![forbid(unsafe_code)]\n",
            "/* #![forbid(unsafe_code)] */\n",
            "const S: &str = \"#![forbid(unsafe_code)]\";\n",
            "const S: &str = r#\"#![forbid(unsafe_code)]\"#;\n",
            "#![deny(unsafe_code)]\n",
            "#![cfg_attr(not(test), forbid(unsafe_code))]\n",
            "#[forbid(unsafe_code)]\nmod m {}\n",
        ] {
            files.insert("crates/math/src/lib.rs", source);
            let found = run(&ws, &files);
            assert_eq!(found.len(), 1, "{source:?} gave {found:?}");
            assert!(
                found[0].starts_with(
                    "crates/math/src/lib.rs: `lactose-math` is not on the unsafe list"
                )
            );
        }
        for source in [
            "#![forbid(unsafe_code)]\n",
            "#! [ forbid( unsafe_code ) ]\n",
            "#![forbid(missing_docs, unsafe_code)]\n",
            "//! Docs with a ' quote.\n#![allow(dead_code, reason = \"a ] bracket\")]\n#![forbid(unsafe_code)]\n",
            "/* nested /* comment */ still */ #![forbid(unsafe_code)]\nfn f<'a>(c: &'a str) -> char { '\\'' }\n",
        ] {
            files.insert("crates/math/src/lib.rs", source);
            assert_eq!(run(&ws, &files), [] as [&str; 0], "{source:?}");
        }
    }

    #[test]
    fn every_crate_root_is_checked() {
        let (mut ws, mut files) = conforming();
        ws.packages[1].crate_roots.push("crates/math/src/main.rs".to_owned());
        files.insert("crates/math/src/main.rs", "fn main() {}\n");
        let found = run(&ws, &files);
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with("crates/math/src/main.rs: "));
    }

    #[test]
    fn the_boundary_lint_configuration_must_be_an_exact_copy() {
        let (ws, mut files) = conforming();
        files.insert("crates/math/clippy.toml", "disallowed-types = []\n\n");
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/clippy.toml: differs from crates/core/clippy.toml; the two must be byte-identical"
            ]
        );

        let (ws, files) = conforming();
        let mut without = Files::default();
        for path in files.paths().filter(|p| *p != "crates/math/clippy.toml") {
            without.insert(path, files.bytes(path).unwrap());
        }
        assert_eq!(
            run(&ws, &without),
            [
                "crates/math/clippy.toml: boundary crate `lactose-math` has no clippy.toml; copy crates/core/clippy.toml"
            ]
        );
    }

    #[test]
    fn a_boundary_lint_may_not_be_lifted_wholesale() {
        let (ws, mut files) = conforming();
        files.insert(
            "crates/math/src/lib.rs",
            "#![forbid(unsafe_code)]\n#![allow(clippy::disallowed_methods)]\nmod trig;\n",
        );
        files.insert(
            "crates/math/src/trig.rs",
            "#![cfg_attr(not(test), expect(clippy::disallowed_types, reason = \"x\"))]\n",
        );
        // Outside the boundary the same attribute is nobody's business here.
        files.insert("crates/task/src/lib.rs", "#![allow(clippy::disallowed_methods)]\n");
        let found = run(&ws, &files);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(
            found[0].starts_with(
                "crates/math/src/lib.rs: `#![allow(clippy::disallowed_methods)]` lifts"
            )
        );
        assert!(found[1].starts_with("crates/math/src/trig.rs: "));

        // The sanctioned form: an outer attribute on one item.
        files.insert(
            "crates/math/src/lib.rs",
            "#![forbid(unsafe_code)]\n#[expect(clippy::disallowed_methods, reason = \"x\")]\nfn f() {}\n",
        );
        files.insert("crates/math/src/trig.rs", "");
        assert_eq!(run(&ws, &files), [] as [&str; 0]);
    }

    #[test]
    fn boundary_dependencies_must_be_on_the_allowlist() {
        let (_, files) = conforming();
        let mut files = files;
        files.insert("crates/ecs/README.md", "");
        files.insert("crates/ecs/SAFETY.md", "");
        files.insert("crates/ecs/clippy.toml", "disallowed-types = []\n");
        let ws = workspace(vec![
            package(
                "lactose-core",
                &[("bytemuck", Normal), ("xxhash-rust", Normal), ("rand", Dev)],
            ),
            package(
                "lactose-math",
                &[
                    ("libm", Build),
                    ("bytemuck", Normal),
                    ("lactose-task", Normal),
                    ("lactose-io", Dev),
                ],
            ),
            package(
                "lactose-ecs",
                &[
                    ("lactose-core", Normal),
                    ("lactose-task", Normal),
                    ("lactose-ecs-macros", Normal),
                    ("lactose-reflect", Normal),
                    ("lactose-io", Normal),
                ],
            ),
        ]);
        let found = run(&ws, &files);
        let policy = "tools/xtask/ci-policy.toml does not allow";
        assert_eq!(
            found,
            [
                format!(
                    "crates/core/Cargo.toml: boundary crate `lactose-core` depends on third-party crate `xxhash-rust`, which [boundary_third_party.\"lactose-core\"] in {policy}"
                ),
                format!(
                    "crates/core/Cargo.toml: boundary crate `lactose-core` dev-depends on third-party crate `rand`, which [boundary_third_party.\"lactose-core\"] in {policy}"
                ),
                format!(
                    "crates/ecs/Cargo.toml: boundary crate `lactose-ecs` depends on non-boundary crate `lactose-io`, which [boundary_internal] in {policy}"
                ),
                format!(
                    "crates/math/Cargo.toml: boundary crate `lactose-math` depends on third-party crate `bytemuck`, which [boundary_third_party.\"lactose-math\"] in {policy}"
                ),
                format!(
                    "crates/math/Cargo.toml: boundary crate `lactose-math` depends on non-boundary crate `lactose-task`, which [boundary_internal] in {policy}"
                ),
            ]
        );
    }

    #[test]
    fn the_pin_must_be_present_and_unchallenged() {
        let (ws, mut files) = conforming();
        files.insert(
            CARGO_CONFIG,
            "# rustflags = [\"-C\", \"target-cpu=x86-64-v3\"]\n[alias]\nx = \"run\"\n",
        );
        assert_eq!(
            run(&ws, &files),
            [
                ".cargo/config.toml: the x86-64 pin is missing: [target.'cfg(target_arch = \"x86_64\")'] must set rustflags [\"-C\", \"target-cpu=x86-64-v3\"]"
            ]
        );

        for accepted in [
            PIN.to_owned(),
            PIN.replace("[\"-C\", \"target-cpu=x86-64-v3\"]", "[\"-Ctarget-cpu=x86-64-v3\"]"),
            PIN.replace("[\"-C\", \"target-cpu=x86-64-v3\"]", "\"-C target-cpu=x86-64-v3\""),
        ] {
            files.insert(CARGO_CONFIG, accepted.clone());
            assert_eq!(run(&ws, &files), [] as [&str; 0], "{accepted}");
        }

        files.insert(
            ".github/workflows/ci.yml",
            "env:\n  # Never set RUSTFLAGS here.\n  RUSTFLAGS: -D warnings\njobs:\n  a:\n    run: cargo rustc -- -C target-cpu=native\n",
        );
        files.insert(
            "scripts/bench.sh",
            "CARGO_ENCODED_RUSTFLAGS=x cargo bench\ncargo rustc -- -Ctarget-cpu=haswell\n",
        );
        files.insert(
            "tools/cook/.cargo/config.toml",
            "[build]\nrustflags = [\"-C\", \"target-cpu=znver4\"]\n",
        );
        // Files that are neither configuration nor scripts are not scanned.
        files.insert("docs/notes.toml", "x = \"RUSTFLAGS target-cpu=native\"\n");
        assert_eq!(
            run(&ws, &files),
            [
                ".github/workflows/ci.yml: line 3 names RUSTFLAGS; the environment variable replaces the x86-64-v3 pin in .cargo/config.toml instead of adding to it",
                ".github/workflows/ci.yml: line 6 sets target-cpu=native, which is denied",
                "scripts/bench.sh: line 1 names RUSTFLAGS; the environment variable replaces the x86-64-v3 pin in .cargo/config.toml instead of adding to it",
                "scripts/bench.sh: line 2 sets target-cpu=haswell; the only permitted setting is the x86-64-v3 pin in a cargo configuration file",
                "tools/cook/.cargo/config.toml: line 2 sets target-cpu=znver4; the only permitted setting is the x86-64-v3 pin in a cargo configuration file",
            ]
        );
    }
}
