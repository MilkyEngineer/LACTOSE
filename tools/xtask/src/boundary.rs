//! The static boundary checks: the facts about files and manifests that architecture
//! sections 8.2, 8.3, 20 and 21 require and that need no compiler to verify.
//!
//! What clippy enforces through `clippy.toml` is not repeated here. This module checks that
//! the configuration clippy reads is the agreed one, and that none of the ways of switching
//! it off is in use: a lint attribute on a whole crate or module, an `allow` on an item,
//! another configuration file, a `[lints]` table, or a flag in a workflow or script.

use std::collections::{BTreeSet, VecDeque};

use crate::policy::Policy;
use crate::repo::{Files, is_build_configuration, is_cargo_config, unread_cargo_configuration};
use crate::rust_source::attributes;
use crate::tiers::TierFile;
use crate::workspace::{DepKind, DepSource, Package, Workspace};
use crate::{Finding, TIER_FILE};

/// The boundary lint configuration every boundary crate copies byte for byte.
pub const REFERENCE_CLIPPY: &str = "crates/core/clippy.toml";

/// The cargo configuration file that carries the x86-64 pin.
pub const CARGO_CONFIG: &str = ".cargo/config.toml";

/// The workspace manifest.
const ROOT_MANIFEST: &str = "Cargo.toml";

/// The `[target]` key and the flags of the pin (architecture section 8.2).
const PIN_KEY: &str = r#"cfg(target_arch = "x86_64")"#;
const PIN_CPU: &str = "x86-64-v3";
const PIN_FLAGS: [&str; 2] = ["-C", "target-cpu=x86-64-v3"];

/// Entries the reference lint file must keep, one or two from each family of rule in
/// section 8.2, so a file that was emptied and then copied everywhere is still caught. The
/// executor's core count and rayon's worker index are here, not in [`THREAD_ROUTES`]: they
/// start no thread, and no test uses them.
const REQUIRED_LINT_PATHS: [&str; 17] = [
    "f32::min",
    "f64::sin",
    "f64::to_bits",
    "f64::powi",
    "std::fs::read",
    "std::env::var",
    "lactose_task::Platform::topology",
    "lactose_task::Platform::hardware_threads",
    "rayon::current_thread_index",
    "rayon::ThreadPool::current_thread_index",
    "std::time::Instant",
    "std::collections::HashMap",
    "std::hash::RandomState",
    "std::thread::LocalKey",
    "std::fs::File",
    "std::net::TcpStream",
    "std::process::Command",
];

/// Every route to a thread outside an executor that the reference lint file bans (sections 5.1,
/// 8.2 and 16): the standard library's, the executor's platform, and the solver pool's and
/// rayon's global pool's, among them the public functions of the pinned Rapier and parry found
/// to run parallel work outside `PhysicsPipeline::step` and the traits a parallel iterator of a
/// crate's own is written with. Each is required, not a sample: no test uses most of them, and
/// `allow-invalid` keeps clippy quiet about an entry whatever happens to it. A unit test holds
/// this list to the reference's entries that say a boundary crate creates no thread or cite
/// section 16, and the check holds every entry under [`THREAD_PREFIXES`] to this list or
/// [`NOT_THREAD_ROUTES`], so an entry added there is pinned here too.
const THREAD_ROUTES: [&str; 104] = [
    "lactose_task::Executor::platform",
    "lactose_task::Platform::spawn_thread",
    "std::thread::spawn",
    "std::thread::scope",
    "std::thread::Scope::spawn",
    "std::thread::Builder::spawn",
    "std::thread::Builder::spawn_scoped",
    "std::thread::Builder::spawn_unchecked",
    "rayon::ThreadPoolBuilder::build",
    "rayon::ThreadPoolBuilder::build_global",
    "rayon::ThreadPoolBuilder::build_scoped",
    "rayon::ThreadPool::new",
    "rapier3d_f64::pipeline::PhysicsWorld::configure_thread_pool",
    "rapier3d_f64::pipeline::PhysicsPipeline::configure_thread_pool",
    "rapier3d_f64::pipeline::PhysicsWorld::clear_thread_pool",
    "rapier3d_f64::pipeline::PhysicsPipeline::clear_thread_pool",
    "rapier3d_f64::pipeline::PhysicsWorld::detect_collisions",
    "rapier3d_f64::pipeline::CollisionPipeline::step",
    "rapier3d_f64::geometry::BroadPhaseBvh::update",
    "rapier3d_f64::dynamics::CCDSolver::find_first_impact",
    "rapier3d_f64::dynamics::CCDSolver::solve_continuous",
    "rapier3d_f64::dynamics::SoftBodySet::sync_particle_positions",
    "rapier3d_f64::dynamics::SoftBodyBuilder::volumetric",
    "rapier3d_f64::dynamics::SoftBodyBuilder::volumetric_with",
    "parry3d_f64::partitioning::Bvh::refit_parallel",
    "parry3d_f64::partitioning::Bvh::refit_without_resolve_parallel",
    "parry3d_f64::partitioning::Bvh::insert_or_update_batch_partially_parallel",
    "parry3d_f64::partitioning::Bvh::traverse_bvtt_single_tree_parallel",
    "parry3d_f64::query::PersistentQueryDispatcher::contact_manifolds",
    "parry3d_f64::query::details::contact_manifolds_composite_shape_shape",
    "parry3d_f64::query::details::contact_manifolds_trimesh_shape",
    "parry3d_f64::query::details::contact_manifolds_trimesh_shape_shapes",
    "parry3d_f64::transformation::volume_mesh",
    "rayon::spawn",
    "rayon::spawn_fifo",
    "rayon::join",
    "rayon::join_context",
    "rayon::scope",
    "rayon::scope_fifo",
    "rayon::in_place_scope",
    "rayon::in_place_scope_fifo",
    "rayon::broadcast",
    "rayon::spawn_broadcast",
    "rayon::current_num_threads",
    "rayon::iter::empty",
    "rayon::iter::once",
    "rayon::iter::repeat",
    "rayon::iter::repeat_n",
    "rayon::iter::repeatn",
    "rayon::iter::split",
    "rayon::iter::walk_tree",
    "rayon::iter::walk_tree_prefix",
    "rayon::iter::walk_tree_postfix",
    "rayon::iter::plumbing::bridge",
    "rayon::iter::plumbing::bridge_producer_consumer",
    "rayon::iter::plumbing::bridge_unindexed",
    "rayon::iter::IntoParallelIterator::into_par_iter",
    "rayon::iter::IntoParallelRefIterator::par_iter",
    "rayon::iter::IntoParallelRefMutIterator::par_iter_mut",
    "rayon::iter::ParallelBridge::par_bridge",
    "rayon::iter::ParallelExtend::par_extend",
    "rayon::iter::FromParallelIterator::from_par_iter",
    "rayon::iter::ParallelDrainFull::par_drain",
    "rayon::iter::ParallelDrainRange::par_drain",
    "rayon::slice::ParallelSlice::par_split",
    "rayon::slice::ParallelSlice::par_split_inclusive",
    "rayon::slice::ParallelSlice::par_windows",
    "rayon::slice::ParallelSlice::par_array_windows",
    "rayon::slice::ParallelSlice::par_chunks",
    "rayon::slice::ParallelSlice::par_chunks_exact",
    "rayon::slice::ParallelSlice::par_rchunks",
    "rayon::slice::ParallelSlice::par_rchunks_exact",
    "rayon::slice::ParallelSlice::par_chunk_by",
    "rayon::slice::ParallelSliceMut::par_split_mut",
    "rayon::slice::ParallelSliceMut::par_split_inclusive_mut",
    "rayon::slice::ParallelSliceMut::par_chunks_mut",
    "rayon::slice::ParallelSliceMut::par_chunks_exact_mut",
    "rayon::slice::ParallelSliceMut::par_rchunks_mut",
    "rayon::slice::ParallelSliceMut::par_rchunks_exact_mut",
    "rayon::slice::ParallelSliceMut::par_sort",
    "rayon::slice::ParallelSliceMut::par_sort_by",
    "rayon::slice::ParallelSliceMut::par_sort_by_key",
    "rayon::slice::ParallelSliceMut::par_sort_by_cached_key",
    "rayon::slice::ParallelSliceMut::par_sort_unstable",
    "rayon::slice::ParallelSliceMut::par_sort_unstable_by",
    "rayon::slice::ParallelSliceMut::par_sort_unstable_by_key",
    "rayon::slice::ParallelSliceMut::par_chunk_by_mut",
    "rayon::str::ParallelString::par_chars",
    "rayon::str::ParallelString::par_char_indices",
    "rayon::str::ParallelString::par_bytes",
    "rayon::str::ParallelString::par_encode_utf16",
    "rayon::str::ParallelString::par_split",
    "rayon::str::ParallelString::par_split_inclusive",
    "rayon::str::ParallelString::par_split_terminator",
    "rayon::str::ParallelString::par_lines",
    "rayon::str::ParallelString::par_split_whitespace",
    "rayon::str::ParallelString::par_split_ascii_whitespace",
    "rayon::str::ParallelString::par_matches",
    "rayon::str::ParallelString::par_match_indices",
    "rayon::iter::plumbing::Producer",
    "rayon::iter::plumbing::UnindexedProducer",
    "rayon::iter::plumbing::ProducerCallback",
    "rayon::iter::plumbing::Consumer",
    "rayon::iter::plumbing::UnindexedConsumer",
];

/// The path prefixes of the thread routes: the check holds every reference entry under one to
/// [`THREAD_ROUTES`] or [`NOT_THREAD_ROUTES`], whatever its reason says.
const THREAD_PREFIXES: [&str; 4] = ["rayon::", "rapier3d_f64::", "parry3d_f64::", "std::thread::"];

/// The reference's entries under [`THREAD_PREFIXES`] that start no thread: rayon's worker
/// index, Rapier's multibody paths into `matrixmultiply`, the core count and thread-local state.
const NOT_THREAD_ROUTES: [&str; 9] = [
    "rayon::current_thread_index",
    "rayon::ThreadPool::current_thread_index",
    "rapier3d_f64::dynamics::Multibody::inverse_kinematics",
    "rapier3d_f64::dynamics::Multibody::inverse_kinematics_delta",
    "rapier3d_f64::dynamics::Multibody::inverse_kinematics_delta_with_jacobian",
    "rapier3d_f64::dynamics::MultibodyJointSet::insert",
    "rapier3d_f64::dynamics::MultibodyJointSet::insert_kinematic",
    "std::thread::available_parallelism",
    "std::thread::LocalKey",
];

/// The entries of the reference lint file, by path prefix, that keep the outside world out:
/// files, the process environment, subprocesses, the network and the standard library's
/// thread creation (section 8.2). Every crate of `[boundary_internal] any` that does not carry the
/// whole file carries these, as written.
const OUTSIDE_WORLD: [&str; 17] = [
    "std::fs::",
    "std::path::",
    "std::os::unix::fs::",
    "std::os::windows::fs::",
    "std::env::",
    "std::io::stdin",
    "std::io::IsTerminal::",
    "std::process::",
    "std::os::unix::process::",
    "std::thread::available_parallelism",
    "std::backtrace::",
    "std::net::",
    "std::os::unix::net::",
    "std::thread::spawn",
    "std::thread::scope",
    "std::thread::Scope::",
    "std::thread::Builder::",
];

/// The two lint tables of a lint file.
const LINT_TABLES: [&str; 2] = ["disallowed-methods", "disallowed-types"];

/// Lint levels that leave a lint weaker than `deny`.
const WEAK_LEVELS: [&str; 3] = ["allow", "expect", "warn"];

/// Whether `lint` is one of the lints that carry the boundary rules, or a group that contains
/// them. `clippy::disallowed_methods` and `clippy::disallowed_types` are in `clippy::style`
/// and `clippy::all`, and CI turns every warning into an error through `warnings`. Clippy
/// still honours the lints' old names, `disallowed_method` and `disallowed_type`.
fn is_boundary_lint(lint: &str) -> bool {
    matches!(
        lint.replace('-', "_").as_str(),
        "clippy::disallowed_methods"
            | "clippy::disallowed_types"
            | "clippy::disallowed_method"
            | "clippy::disallowed_type"
            | "clippy::all"
            | "clippy::style"
            | "warnings"
    )
}

/// Runs every static boundary check.
#[must_use]
pub fn check(tiers: &TierFile, policy: &Policy, ws: &Workspace, files: &Files) -> Vec<Finding> {
    let mut findings = Vec::new();
    reference_lint_file(files, &mut findings);
    let mut stray_configs = BTreeSet::new();
    for package in &ws.packages {
        readme(package, files, &mut findings);
        unsafe_policy(policy, package, files, &mut findings);
        let boundary = tiers.in_boundary(&package.name);
        let carrier = boundary || policy.lint_file_carriers.contains(&package.name);
        // Compiled into every boundary crate, so held to the bans that keep the outside world
        // out (section 8.2), but not to the float, executor or solver-pool ones.
        let internal = !carrier && tiers.boundary_internal.any.contains(&package.name);
        if carrier {
            lint_file_copy(package, files, &mut findings);
        } else if internal {
            outside_world_lint_file(package, files, &mut findings);
        }
        if carrier || internal {
            let kind = if boundary {
                "boundary crate"
            } else if carrier {
                "[lint_file_carriers] crate"
            } else {
                "[boundary_internal] crate"
            };
            stray_lint_files(package, files, &mut stray_configs);
            manifest_lints(package, files, &mut findings);
            lint_attributes(package, kind, files, &mut findings);
        }
        if boundary {
            dependency_allowlist(tiers, ws, package, &mut findings);
        }
    }
    findings.extend(stray_configs.into_iter().map(|path| {
        Finding::new(
            path,
            "clippy reads `.clippy.toml` in preference to `clippy.toml`, so this file replaces \
             the boundary lint configuration of every crate at or below it",
        )
    }));
    workspace_lints(files, &mut findings);
    build_configuration(files, &mut findings);
    cargo_configs(files, &mut findings);
    findings.extend(unread_cargo_configuration(files));
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
/// `SAFETY.md`. Every other crate forbids it in every crate root it has, with an inner
/// attribute ahead of the first item: one further down applies to a nested module only.
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
        let forbids = files.text(root).is_some_and(|source| {
            attributes(&source).iter().any(|a| a.leading && forbids_unsafe(&a.body))
        });
        if !forbids {
            findings.push(Finding::new(
                root,
                format!(
                    "`{}` is not on the unsafe list (architecture section 21), so this crate \
                     root needs `#![forbid(unsafe_code)]` before its first item",
                    package.name
                ),
            ));
        }
    }
}

/// Whether an attribute body is `forbid(unsafe_code)`, alone or among other lints. A
/// `cfg_attr` does not count: it forbids only sometimes.
fn forbids_unsafe(body: &str) -> bool {
    body.strip_prefix("forbid(")
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(|lints| lints.split(',').any(|lint| lint == "unsafe_code"))
}

/// The reference lint file exists and still bans what section 8.2 bans.
fn reference_lint_file(files: &Files, findings: &mut Vec<Finding>) {
    let Some(text) = files.text(REFERENCE_CLIPPY) else {
        findings.push(Finding::new(
            REFERENCE_CLIPPY,
            "the reference boundary lint configuration is missing",
        ));
        return;
    };
    let config = match toml::from_str::<toml::Table>(&text) {
        Ok(config) => config,
        Err(e) => {
            findings.push(Finding::new(REFERENCE_CLIPPY, format!("not valid TOML: {e}")));
            return;
        }
    };
    let banned: BTreeSet<&str> = LINT_TABLES
        .iter()
        .filter_map(|key| config.get(*key).and_then(toml::Value::as_array))
        .flatten()
        .filter_map(|entry| entry.as_str().or_else(|| entry.get("path")?.as_str()))
        .collect();
    for required in REQUIRED_LINT_PATHS {
        if !banned.contains(required) {
            findings.push(Finding::new(
                REFERENCE_CLIPPY,
                format!(
                    "no longer disallows `{required}`, which architecture section 8.2 bans \
                     inside the boundary"
                ),
            ));
        }
    }
    for route in THREAD_ROUTES.iter().filter(|route| !banned.contains(*route)) {
        findings.push(Finding::new(
            REFERENCE_CLIPPY,
            format!(
                "no longer disallows `{route}`, a route to a thread outside an executor, which \
                 architecture sections 8.2 and 16 ban inside the boundary"
            ),
        ));
    }
    let unpinned = banned.iter().filter(|path| {
        THREAD_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
            && !THREAD_ROUTES.contains(path)
            && !NOT_THREAD_ROUTES.contains(path)
    });
    for path in unpinned {
        findings.push(Finding::new(
            REFERENCE_CLIPPY,
            format!(
                "disallows `{path}`, which tools/xtask/src/boundary.rs lists neither in \
                 `THREAD_ROUTES` nor in `NOT_THREAD_ROUTES`; list it in the one it belongs to"
            ),
        ));
    }
}

/// Architecture section 8.3: one `clippy.toml` per boundary crate, all identical.
fn lint_file_copy(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let path = format!("{}/clippy.toml", package.dir);
    let Some(reference) = files.bytes(REFERENCE_CLIPPY) else {
        return;
    };
    match files.bytes(&path) {
        None => findings.push(Finding::new(
            path,
            format!(
                "`{}` must carry the boundary lint file; copy {REFERENCE_CLIPPY}",
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

/// A crate of `[boundary_internal] any` compiles into every boundary crate, so it carries a
/// `clippy.toml` whose entries under [`OUTSIDE_WORLD`] are exactly the reference's, each as
/// written there.
fn outside_world_lint_file(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let path = format!("{}/clippy.toml", package.dir);
    // A reference that is missing or malformed is reported on its own.
    let Some(reference) = files.text(REFERENCE_CLIPPY).and_then(|t| toml::from_str(&t).ok()) else {
        return;
    };
    let Some(text) = files.text(&path) else {
        findings.push(Finding::new(
            path,
            format!(
                "`{}` is in [boundary_internal] `any` of {TIER_FILE}, so it compiles into every \
                 boundary crate and must carry a clippy.toml with the entries of \
                 {REFERENCE_CLIPPY} that ban files, the environment, subprocesses, the \
                 network and thread creation",
                package.name
            ),
        ));
        return;
    };
    let config = match toml::from_str::<toml::Table>(&text) {
        Ok(config) => config,
        Err(e) => {
            findings.push(Finding::new(path, format!("not valid TOML: {e}")));
            return;
        }
    };
    for table in LINT_TABLES {
        let wanted = outside_world_entries(&reference, table);
        let carried = outside_world_entries(&config, table);
        for (name, _) in wanted.iter().filter(|entry| !carried.contains(entry)) {
            findings.push(Finding::new(
                &path,
                format!(
                    "`{table}` lacks the entry for `{name}` that {REFERENCE_CLIPPY} has; copy it \
                     as written there"
                ),
            ));
        }
        for (name, _) in carried.iter().filter(|entry| !wanted.contains(entry)) {
            findings.push(Finding::new(
                &path,
                format!(
                    "`{table}` has an entry for `{name}` that {REFERENCE_CLIPPY} does not have \
                     as written; the two must agree on every file, environment, subprocess, \
                     network and thread entry"
                ),
            ));
        }
    }
}

/// The entries of one lint table of `config` whose paths are under [`OUTSIDE_WORLD`], with
/// their paths.
fn outside_world_entries<'a>(
    config: &'a toml::Table,
    table: &str,
) -> Vec<(&'a str, &'a toml::Value)> {
    config
        .get(table)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| Some((entry.as_str().or_else(|| entry.get("path")?.as_str())?, entry)))
        .filter(|(path, _)| OUTSIDE_WORLD.iter().any(|prefix| path.starts_with(prefix)))
        .collect()
}

/// Collects every `.clippy.toml` in the package directory or a directory above it.
fn stray_lint_files(package: &Package, files: &Files, found: &mut BTreeSet<String>) {
    let mut dir = Some(package.dir.as_str());
    while let Some(at) = dir {
        let path =
            if at.is_empty() { ".clippy.toml".to_owned() } else { format!("{at}/.clippy.toml") };
        if files.contains(&path) {
            found.insert(path);
        }
        dir = match at.rsplit_once('/') {
            Some((parent, _)) => Some(parent),
            None if at.is_empty() => None,
            None => Some(""),
        };
    }
}

/// The Rust sources of a package: every `.rs` file under its directory that is not inside a
/// nested package.
fn package_sources<'a>(package: &Package, files: &'a Files) -> Vec<&'a str> {
    let prefix = format!("{}/", package.dir);
    files
        .paths()
        .filter(|path| path.ends_with(".rs"))
        .filter(|path| {
            let Some(inside) = path.strip_prefix(&prefix) else {
                return false;
            };
            // Any directory between the package and the file that has a manifest of its own.
            !inside
                .match_indices('/')
                .any(|(at, _)| files.contains(&format!("{prefix}{}/Cargo.toml", &inside[..at])))
        })
        .collect()
}

/// The boundary lints an attribute body weakens, each as `level(lint)`.
fn weakened_lints(body: &str) -> Vec<String> {
    let mut found = Vec::new();
    for level in WEAK_LEVELS {
        let open = format!("{level}(");
        for (at, _) in body.match_indices(&open) {
            let standalone = at == 0
                || !body.as_bytes()[at - 1].is_ascii_alphanumeric()
                    && body.as_bytes()[at - 1] != b'_';
            if !standalone {
                continue;
            }
            // The arguments run to the matching parenthesis.
            let arguments = &body[at + open.len()..];
            let mut depth = 1usize;
            let end = arguments.find(|c: char| {
                depth = match c {
                    '(' => depth + 1,
                    ')' => depth - 1,
                    _ => depth,
                };
                depth == 0
            });
            for lint in arguments[..end.unwrap_or(arguments.len())].split(',') {
                if is_boundary_lint(lint) {
                    found.push(format!("{level}({lint})"));
                }
            }
        }
    }
    found
}

/// A crate that carries boundary lints lifts one with an `#[expect]` on the one item that
/// needs it, which fails once the item no longer does. This rejects every other way: `allow`
/// or `warn` anywhere, and an `expect` in an inner attribute or on a `mod`, where it covers a
/// whole crate or module. It reads every source file of the package and every file a
/// `#[path]` in them names, wherever that lives, since the file is compiled into the crate
/// (the SOUR protocol is, from `tools/xtask`). `kind` names the crate in the findings.
fn lint_attributes(package: &Package, kind: &str, files: &Files, findings: &mut Vec<Finding>) {
    let mut queue: VecDeque<String> =
        package_sources(package, files).into_iter().map(str::to_owned).collect();
    let mut seen: BTreeSet<String> = queue.iter().cloned().collect();
    while let Some(path) = queue.pop_front() {
        let Some(source) = files.text(&path) else {
            continue;
        };
        for attribute in attributes(&source) {
            if let Some(named) = &attribute.path {
                match module_file(&path, named).filter(|file| files.contains(file)) {
                    Some(file) => {
                        if seen.insert(file.clone()) {
                            queue.push_back(file);
                        }
                    }
                    None => findings.push(Finding::new(
                        &path,
                        format!(
                            "line {}: `#[path]` names `{named}`, which is not a source file in \
                             the repository, so the lint attributes of {kind} `{}` in it cannot \
                             be checked",
                            attribute.line, package.name
                        ),
                    )),
                }
            }
            for weakened in weakened_lints(&attribute.body) {
                let what = if attribute.inner {
                    "applies to a whole crate or module of"
                } else if attribute.on_module {
                    "applies to a module of"
                } else if weakened.starts_with("expect(") {
                    continue;
                } else {
                    "never goes stale, on an item of"
                };
                findings.push(Finding::new(
                    &path,
                    format!(
                        "line {}: `{weakened}` {what} {kind} `{}`; the only sanctioned form is \
                         `#[expect(clippy::disallowed_methods, reason = \"...\")]` (or \
                         `disallowed_types`) on the one item that needs it",
                        attribute.line, package.name
                    ),
                ));
            }
        }
    }
}

/// The repository path of the file that `#[path = "named"]` in the file `from` names: relative
/// to the directory of `from`, as for every `mod` outside an inline module block. `None` for
/// a path that is absolute or leaves the repository.
fn module_file(from: &str, named: &str) -> Option<String> {
    if named.starts_with(['/', '\\']) || named.get(1..2) == Some(":") {
        return None;
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for part in named.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// The level of each boundary lint that a `[lints]`-shaped table sets, as (lint, level).
fn lint_table_levels(table: Option<&toml::Value>) -> Vec<(String, String)> {
    let mut levels = Vec::new();
    for (tool, prefix) in [("rust", ""), ("clippy", "clippy::")] {
        let Some(lints) = table.and_then(|t| t.get(tool)).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, value) in lints {
            let lint = format!("{prefix}{name}");
            let level = value.as_str().or_else(|| value.get("level")?.as_str());
            if let Some(level) = level.filter(|_| is_boundary_lint(&lint)) {
                levels.push((lint, level.to_owned()));
            }
        }
    }
    levels
}

fn level_rank(level: &str) -> u8 {
    match level {
        "allow" => 0,
        "warn" => 1,
        "deny" => 2,
        _ => 3,
    }
}

fn manifest_table(files: &Files, path: &str) -> Option<toml::Table> {
    toml::from_str(&files.text(path)?).ok()
}

/// `[workspace.lints]` may not set a boundary lint below `warn`, the level at which CI's
/// `-D warnings` still fails the build.
fn workspace_lints(files: &Files, findings: &mut Vec<Finding>) {
    let Some(manifest) = manifest_table(files, ROOT_MANIFEST) else {
        return;
    };
    let table = manifest.get("workspace").and_then(|w| w.get("lints"));
    for (lint, level) in lint_table_levels(table) {
        if level_rank(&level) < level_rank("warn") {
            findings.push(Finding::new(
                ROOT_MANIFEST,
                format!(
                    "[workspace.lints] sets `{lint}` to `{level}`, which switches the boundary \
                     lints off for every crate that inherits it"
                ),
            ));
        }
    }
}

/// A crate that carries the boundary lint file may not set a boundary lint in its own
/// `[lints]` table below the workspace's level (`warn` where the workspace says nothing).
fn manifest_lints(package: &Package, files: &Files, findings: &mut Vec<Finding>) {
    let workspace = manifest_table(files, ROOT_MANIFEST);
    let workspace_levels =
        lint_table_levels(workspace.as_ref().and_then(|m| m.get("workspace")?.get("lints")));
    let Some(manifest) = manifest_table(files, &package.manifest()) else {
        return;
    };
    for (lint, level) in lint_table_levels(manifest.get("lints")) {
        let floor = workspace_levels
            .iter()
            .find(|(name, _)| *name == lint)
            .map_or(1, |(_, level)| level_rank(level).max(1));
        if level_rank(&level) < floor {
            findings.push(Finding::new(
                package.manifest(),
                format!(
                    "[lints] sets `{lint}` to `{level}`, weaker than the workspace default; \
                     `{}` carries the boundary lints",
                    package.name
                ),
            ));
        }
    }
}

/// Architecture section 8.2: the edges out of the boundary are the ones `crate-tiers.toml`
/// names, directly and at any depth.
fn dependency_allowlist(
    tiers: &TierFile,
    ws: &Workspace,
    package: &Package,
    findings: &mut Vec<Finding>,
) {
    let internal = &tiers.boundary_internal;
    let third_party = &tiers.boundary_third_party;
    let direct = third_party.direct.get(&package.name);
    // The workspace crates `package` may compile, whether it names them or reaches them.
    let may_compile = |name: &str| {
        tiers.in_boundary(name)
            || internal.any.iter().any(|c| c == name)
            || internal.only.get(&package.name).is_some_and(|only| only.iter().any(|c| c == name))
    };
    // The workspace crates its tests may compile besides those. Section 8.2's "directly or at
    // any depth" holds for a test too: it runs under SOUR.
    let test_may_compile = |name: &str| {
        may_compile(name)
            || internal.dev.iter().any(|c| c == name)
            || tiers.test_support.iter().any(|c| c == name)
    };
    // The workspace crates reported so far, so that what a test reaches is not named twice.
    let mut reported = BTreeSet::new();
    for dep in &package.deps {
        // A name the tier file lists is a workspace crate wherever it is fetched from.
        let is_internal = dep.source == DepSource::Member || tiers.tier(&dep.name).is_some();
        let allowed = if is_internal {
            may_compile(&dep.name) || dep.kind == DepKind::Dev && test_may_compile(&dep.name)
        } else {
            direct.is_some_and(|list| {
                list.normal.contains(&dep.name)
                    || (dep.kind == DepKind::Dev && list.dev.contains(&dep.name))
            })
        };
        if allowed {
            continue;
        }
        if is_internal {
            reported.insert(dep.name.clone());
        }
        let (what, table) = if is_internal {
            ("non-boundary crate", "[boundary_internal]".to_owned())
        } else {
            ("third-party crate", format!("[boundary_third_party.direct] \"{}\"", package.name))
        };
        let section = match dep.kind {
            DepKind::Normal => "depends",
            DepKind::Build => "build-depends",
            DepKind::Dev => "dev-depends",
        };
        findings.push(Finding::new(
            package.manifest(),
            format!(
                "boundary crate `{}` {section} on {what} `{}`, which {table} in {TIER_FILE} \
                 does not allow",
                package.name, dep.name
            ),
        ));
    }

    // The workspace crates it reaches through crates outside the boundary, at any depth: from
    // the crate itself, then from each crate outside the boundary that its tests name, against
    // what a test may compile. A boundary crate on the way is not passed through: its own
    // reach is judged as its own, against its own `only` list (section 8.2: `ecs` compiles
    // `task`, and every crate that depends on `ecs` therefore does). Cargo never compiles a
    // dependency's dev-dependencies, so the crate's own dev edges are the only ones followed.
    let workspace_crate = |name: &str| tiers.tier(name).is_some() || ws.package(name).is_some();
    let mut roots = vec![(package.name.as_str(), false)];
    roots.extend(
        package
            .deps
            .iter()
            .filter(|dep| {
                dep.kind == DepKind::Dev
                    && dep.name != package.name
                    && workspace_crate(&dep.name)
                    && !tiers.in_boundary(&dep.name)
                    && test_may_compile(&dep.name)
            })
            .map(|dep| (dep.name.as_str(), true)),
    );
    for (root, test) in roots {
        let allowed = |name: &str| if test { test_may_compile(name) } else { may_compile(name) };
        let through =
            |name: &str| workspace_crate(name) && !tiers.in_boundary(name) && allowed(name);
        for chain in ws.reach(root, &through) {
            let reached = chain.last().map_or("", String::as_str);
            // The crate's direct edges were judged above, in the words they always had.
            let direct = !test && chain.len() == 2;
            if direct || !workspace_crate(reached) || allowed(reached) {
                continue;
            }
            // What the crate itself reaches is reported however else it is named, and `reach`
            // names each crate once; what a test reaches only if nothing before named it.
            if !reported.insert(reached.to_owned()) && test {
                continue;
            }
            let (whose, route) = if test {
                (" in its tests", format!("{} -> {}", package.name, chain.join(" -> ")))
            } else {
                ("", chain.join(" -> "))
            };
            findings.push(Finding::new(
                package.manifest(),
                format!(
                    "boundary crate `{}` reaches non-boundary crate `{reached}`{whose} \
                     ({route}), which [boundary_internal] in {TIER_FILE} does not allow at any \
                     depth",
                    package.name
                ),
            ));
        }
    }

    // Everything the crate ends up compiling. Its own direct dependencies were judged above.
    let named: BTreeSet<&str> =
        package.deps.iter().filter(|d| d.kind.ships()).map(|d| d.name.as_str()).collect();
    for (name, chain) in ws.third_party_closure(&package.name) {
        if !named.contains(name.as_str()) && !third_party.vetted(&name) {
            findings.push(Finding::new(
                package.manifest(),
                format!(
                    "boundary crate `{}` compiles third-party crate `{name}` ({}), which is on \
                     no `normal` list and not in `transitive` of [boundary_third_party] in \
                     {TIER_FILE}",
                    package.name,
                    chain.join(" -> ")
                ),
            ));
        }
    }
}

/// The lint named by each `-A`/`--allow` flag on the line.
fn allowed_by_flag(line: &str) -> Vec<&str> {
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '[' | ']'))
        .filter(|word| !word.is_empty())
        .collect();
    let mut lints = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let lint = if *word == "-A" || *word == "--allow" {
            words.get(index + 1).copied()
        } else {
            word.strip_prefix("--allow=").or_else(|| word.strip_prefix("-A"))
        };
        lints.extend(lint.filter(|lint| {
            is_boundary_lint(lint) || lint.replace('-', "_").starts_with("clippy::disallowed")
        }));
    }
    lints
}

/// Architecture section 8.2, for every file that can decide how cargo and clippy are invoked
/// (cargo configuration, workflows, scripts): nothing sets rust flags through the environment,
/// selects a CPU or a target feature, points clippy at another configuration, or allows a
/// boundary lint from the command line.
///
/// rustc reads `-C target_cpu` and `-C target-cpu` alike, so underscores are folded before
/// matching. Only whole-line `#` comments are skipped; a trailing comment is scanned as if it
/// were code, which errs towards a finding.
fn build_configuration(files: &Files, findings: &mut Vec<Finding>) {
    for path in files.paths().filter(|path| is_build_configuration(path)) {
        let Some(text) = files.text(path) else {
            continue;
        };
        let cargo_config = is_cargo_config(path);
        for (index, line) in text.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let mut report = |message: String| {
                findings.push(Finding::new(path, format!("line {}: {message}", index + 1)));
            };
            // In a cargo configuration file the `rustflags` key is judged by structure, in
            // `cargo_configs`; anywhere else the word in any spelling is a way to set them
            // (`--config build.rustflags=...`, the environment variables).
            let names_flags = if cargo_config {
                line.contains("RUSTFLAGS")
            } else {
                line.to_ascii_uppercase().replace('-', "_").contains("RUSTFLAGS")
            };
            if names_flags {
                report(format!(
                    "names RUSTFLAGS; rust flags from the environment or the command line \
                     replace the {PIN_CPU} pin in {CARGO_CONFIG} instead of adding to it"
                ));
            }
            let folded = line.replace('_', "-");
            for (at, _) in folded.match_indices("target-cpu") {
                let value = folded[at + "target-cpu".len()..]
                    .strip_prefix('=')
                    .map(|rest| {
                        let end = rest
                            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.')))
                            .unwrap_or(rest.len());
                        &rest[..end]
                    })
                    .unwrap_or_default();
                if !(cargo_config && value == PIN_CPU) {
                    report(format!(
                        "selects a target CPU (`target-cpu` `{value}`); the only permitted \
                         setting is the {PIN_CPU} pin in {CARGO_CONFIG}"
                    ));
                }
            }
            if folded.contains("target-feature") {
                report(
                    "sets `target-feature`; target features come from the pinned target CPU only"
                        .to_owned(),
                );
            }
            if line.contains("CLIPPY_CONF_DIR") {
                report(
                    "names CLIPPY_CONF_DIR, which points clippy away from each crate's \
                     clippy.toml"
                        .to_owned(),
                );
            }
            for lint in allowed_by_flag(line) {
                report(format!(
                    "allows `{lint}` from the command line, which switches the boundary lints off"
                ));
            }
        }
    }
}

/// Every `rustflags` key of a TOML value, as a dotted path with its value.
fn rustflags_keys<'a>(
    value: &'a toml::Value,
    path: &mut Vec<&'a str>,
    found: &mut Vec<(String, &'a toml::Value)>,
) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        path.push(key);
        if key == "rustflags" {
            found.push((path.join("."), child));
        } else {
            rustflags_keys(child, path, found);
        }
        path.pop();
    }
}

/// Cargo configuration files set `rustflags` in exactly one place: the x86-64-v3 pin in
/// `.cargo/config.toml`. Any other `rustflags` key, in any table of any cargo configuration
/// file, is a finding, because cargo joins or replaces flags by rules nobody should have to
/// re-derive to know what the boundary was compiled with.
fn cargo_configs(files: &Files, findings: &mut Vec<Finding>) {
    let pin_path = format!("target.{PIN_KEY}.rustflags");
    let mut pinned = false;
    for path in files.paths().filter(|path| is_cargo_config(path)) {
        let Some(text) = files.text(path) else {
            continue;
        };
        let config = match toml::from_str::<toml::Value>(&text) {
            Ok(config) => config,
            Err(e) => {
                findings.push(Finding::new(path, format!("not valid TOML: {e}")));
                continue;
            }
        };
        let mut found = Vec::new();
        rustflags_keys(&config, &mut Vec::new(), &mut found);
        for (key, value) in found {
            let flags: Option<Vec<&str>> = value
                .as_array()
                .map(|items| items.iter().filter_map(toml::Value::as_str).collect());
            if path == CARGO_CONFIG && key == pin_path && flags.as_deref() == Some(&PIN_FLAGS[..]) {
                pinned = true;
            } else {
                findings.push(Finding::new(
                    path,
                    format!(
                        "sets `{key}`; the only rust flags permitted in a cargo configuration \
                         file are the pin, [target.'{PIN_KEY}'] rustflags = {PIN_FLAGS:?} in \
                         {CARGO_CONFIG}"
                    ),
                ));
            }
        }
    }
    if !pinned {
        findings.push(Finding::new(
            CARGO_CONFIG,
            format!(
                "the x86-64 pin is missing: [target.'{PIN_KEY}'] must set rustflags = \
                 {PIN_FLAGS:?}"
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiers::fixture::FILE as TIERS;
    use crate::workspace::fixture::{package, workspace, workspace_with};
    use DepKind::{Build, Dev, Normal};

    const PIN: &str = "[target.'cfg(target_arch = \"x86_64\")']\nrustflags = [\"-C\", \"target-cpu=x86-64-v3\"]\n";
    const FORBID: &str = "//! Docs.\n#![forbid(unsafe_code)]\n";
    const MANIFEST: &str = "[package]\nname = \"x\"\n[lints]\nworkspace = true\n";

    /// A lint file of the required entries and thread routes that `keep` keeps: the first ten
    /// required entries and every route are methods (the check reads both tables as one).
    fn lint_file_of(keep: impl Fn(&str) -> bool) -> String {
        let mut text = String::from("disallowed-methods = [\n");
        let methods = REQUIRED_LINT_PATHS[..10].iter().chain(&THREAD_ROUTES);
        for path in methods.filter(|path| keep(path)) {
            text.push_str(&format!("  {{ path = \"{path}\", reason = \"r\" }},\n"));
        }
        text.push_str("]\ndisallowed-types = [\n");
        for path in REQUIRED_LINT_PATHS[10..].iter().filter(|path| keep(path)) {
            text.push_str(&format!("  \"{path}\",\n"));
        }
        text.push_str("]\n");
        text
    }

    fn lint_file() -> String {
        lint_file_of(|_| true)
    }

    /// What a crate of `[boundary_internal] any` carries.
    fn outside_world_file() -> String {
        lint_file_of(|path| OUTSIDE_WORLD.iter().any(|prefix| path.starts_with(prefix)))
    }

    fn policy() -> Policy {
        Policy::parse(include_str!("../ci-policy.toml")).unwrap()
    }

    /// A conforming repository: `core` (unsafe, boundary), `math` (boundary), `task` (unsafe).
    fn conforming() -> (Workspace, Files) {
        let ws = workspace_with(
            vec![
                package("lactose-core", &[("bytemuck", Normal), ("xxhash-rust", Dev)]),
                package("lactose-math", &[("lactose-core", Normal), ("libm", Normal)]),
                package("lactose-task", &[("lactose-core", Normal), ("crossbeam-deque", Normal)]),
            ],
            &[("bytemuck", &["bytemuck_derive"]), ("crossbeam-deque", &["crossbeam-epoch"])],
        );
        let mut files = Files::default();
        files.insert(CARGO_CONFIG, PIN);
        files.insert(
            ROOT_MANIFEST,
            "[workspace]\n[workspace.lints.rust]\nmissing_docs = \"warn\"\n",
        );
        files.insert(REFERENCE_CLIPPY, lint_file());
        files.insert("crates/math/clippy.toml", lint_file());
        for role in ["core", "math", "task"] {
            files.insert(format!("crates/{role}/README.md"), "# readme\n");
            files.insert(format!("crates/{role}/Cargo.toml"), MANIFEST);
        }
        files.insert("crates/core/SAFETY.md", "# safety\n");
        files.insert("crates/task/SAFETY.md", "# safety\n");
        files.insert("crates/core/src/lib.rs", "pub fn f() {}\n");
        files.insert("crates/task/src/lib.rs", "pub fn f() {}\n");
        files.insert("crates/math/src/lib.rs", FORBID);
        (ws, files)
    }

    fn without(files: &Files, removed: &[&str]) -> Files {
        let mut kept = Files::default();
        for path in files.paths().filter(|path| !removed.contains(path)) {
            kept.insert(path, files.bytes(path).unwrap());
        }
        kept
    }

    fn run(ws: &Workspace, files: &Files) -> Vec<String> {
        let tiers = TierFile::parse(TIERS).unwrap();
        check(&tiers, &policy(), ws, files).iter().map(ToString::to_string).collect()
    }

    const NONE: [&str; 0] = [];

    #[test]
    fn a_conforming_repository_has_no_findings() {
        let (ws, files) = conforming();
        assert_eq!(run(&ws, &files), NONE);
    }

    #[test]
    fn missing_documents_are_reported() {
        let (ws, files) = conforming();
        let files = without(&files, &["crates/task/SAFETY.md", "crates/math/README.md"]);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/README.md: `lactose-math` has no README.md",
                "crates/task/SAFETY.md: `lactose-task` is on the unsafe list (architecture \
                 section 21) and has no SAFETY.md",
            ]
        );
    }

    #[test]
    fn a_crate_off_the_unsafe_list_must_forbid_unsafe_before_its_first_item() {
        let (ws, mut files) = conforming();
        for source in [
            "pub fn f() {}\n",
            "// #![forbid(unsafe_code)]\n",
            "const S: &str = \"#![forbid(unsafe_code)]\";\n",
            "#![deny(unsafe_code)]\n",
            "#![cfg_attr(not(test), forbid(unsafe_code))]\n",
            "#[forbid(unsafe_code)]\nmod m {}\n",
            // Review item 9: the attribute inside a nested module forbids nothing outside it.
            "mod quarantine {\n    #![forbid(unsafe_code)]\n}\npub unsafe fn f() {}\n",
            "pub unsafe fn f() {}\nmod m { #![forbid(unsafe_code)] }\n",
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
            "//! Docs.\n#![allow(dead_code, reason = \"a ] bracket\")]\n#![forbid(unsafe_code)]\nfn f() {}\n",
        ] {
            files.insert("crates/math/src/lib.rs", source);
            assert_eq!(run(&ws, &files), NONE, "{source:?}");
        }
    }

    #[test]
    fn every_crate_root_is_checked_tests_benches_examples_and_build_script_included() {
        // Review item 16.
        let (mut ws, mut files) = conforming();
        let roots = ["build.rs", "benches/b.rs", "examples/e.rs", "src/bin/tool.rs", "tests/t.rs"];
        for root in roots {
            ws.packages[1].crate_roots.push(format!("crates/math/{root}"));
            files.insert(format!("crates/math/{root}"), "fn main() {}\n");
        }
        let found = run(&ws, &files);
        assert_eq!(found.len(), roots.len(), "{found:#?}");
        for root in roots {
            assert!(
                found.iter().any(|f| f.starts_with(&format!("crates/math/{root}: "))),
                "{root}"
            );
            files.insert(format!("crates/math/{root}"), FORBID);
        }
        assert_eq!(run(&ws, &files), NONE);
    }

    #[test]
    fn the_boundary_lint_file_must_be_an_exact_copy() {
        let (ws, mut files) = conforming();
        files.insert("crates/math/clippy.toml", format!("{}\n", lint_file()));
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/clippy.toml: differs from crates/core/clippy.toml; the two must be byte-identical"
            ]
        );
        let files = without(&files, &["crates/math/clippy.toml"]);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/clippy.toml: `lactose-math` must carry the boundary lint file; copy crates/core/clippy.toml"
            ]
        );
    }

    #[test]
    fn a_proc_macro_crate_on_the_carrier_list_carries_the_lint_file_too() {
        // Review item 13.
        let (mut ws, mut files) = conforming();
        ws.packages.push(package("lactose-reflect-macros", &[("syn", Normal)]));
        files.insert("crates/reflect-macros/README.md", "");
        files.insert("crates/reflect-macros/Cargo.toml", MANIFEST);
        files.insert("crates/reflect-macros/src/lib.rs", FORBID);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/reflect-macros/clippy.toml: `lactose-reflect-macros` must carry the boundary lint file; copy crates/core/clippy.toml"
            ]
        );
        files.insert("crates/reflect-macros/clippy.toml", lint_file());
        assert_eq!(run(&ws, &files), NONE, "and its dependencies are not held to the allowlist");
        // Round 2, finding 2: nor may its sources switch the lints off.
        files.insert(
            "crates/reflect-macros/src/lib.rs",
            format!("{FORBID}#[allow(clippy::disallowed_methods)]\nfn f() {{}}\n"),
        );
        assert_eq!(
            run(&ws, &files),
            [
                "crates/reflect-macros/src/lib.rs: line 3: `allow(clippy::disallowed_methods)` never goes stale, on an item of [lint_file_carriers] crate `lactose-reflect-macros`; the only sanctioned form is `#[expect(clippy::disallowed_methods, reason = \"...\")]` (or `disallowed_types`) on the one item that needs it"
            ]
        );
    }

    #[test]
    fn a_weakened_reference_lint_file_is_caught_even_when_every_copy_matches() {
        // Review item 19.
        let (ws, mut files) = conforming();
        let weakened = lint_file()
            .replace("  \"std::time::Instant\",\n", "")
            .replace("  { path = \"f64::sin\", reason = \"r\" },\n", "");
        files.insert(REFERENCE_CLIPPY, weakened.clone());
        files.insert("crates/math/clippy.toml", weakened);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/core/clippy.toml: no longer disallows `f64::sin`, which architecture section 8.2 bans inside the boundary",
                "crates/core/clippy.toml: no longer disallows `std::time::Instant`, which architecture section 8.2 bans inside the boundary",
            ]
        );
    }

    #[test]
    fn every_thread_route_stays_banned_in_the_reference_and_in_what_every_boundary_crate_compiles()
    {
        // A boundary crate creates no thread outside an executor (architecture sections 8.2
        // and 16): the reference keeps every route, and a crate of `[boundary_internal] any`
        // carries the standard library's as written there.
        let (mut ws, mut files) = conforming();
        let entry = |path: &str| format!("  {{ path = \"{path}\", reason = \"r\" }},\n");
        for route in THREAD_ROUTES {
            let unbanned = lint_file().replace(&entry(route), "");
            assert_ne!(unbanned, lint_file(), "{route}");
            files.insert(REFERENCE_CLIPPY, unbanned.clone());
            files.insert("crates/math/clippy.toml", unbanned);
            assert_eq!(
                run(&ws, &files),
                [format!(
                    "crates/core/clippy.toml: no longer disallows `{route}`, a route to a thread outside an executor, which architecture sections 8.2 and 16 ban inside the boundary"
                )]
            );
        }
        files.insert(REFERENCE_CLIPPY, lint_file());
        files.insert("crates/math/clippy.toml", lint_file());
        ws.packages.push(package("lactose-reflect", &[("lactose-core", Normal)]));
        files.insert("crates/reflect/README.md", "");
        files.insert("crates/reflect/Cargo.toml", MANIFEST);
        files.insert("crates/reflect/src/lib.rs", FORBID);
        let std_routes: Vec<&str> =
            THREAD_ROUTES.into_iter().filter(|route| route.starts_with("std::")).collect();
        assert_eq!(std_routes.len(), 6);
        for route in std_routes {
            assert!(outside_world_file().contains(&entry(route)), "{route}");
            files.insert(
                "crates/reflect/clippy.toml",
                outside_world_file().replace(&entry(route), ""),
            );
            assert_eq!(
                run(&ws, &files),
                [format!(
                    "crates/reflect/clippy.toml: `disallowed-methods` lacks the entry for `{route}` that crates/core/clippy.toml has; copy it as written there"
                )]
            );
        }
        // The pool and executor routes are not the outside world's: an `any` crate cannot
        // name them.
        assert!(!outside_world_file().contains("rayon::"));
        assert!(!outside_world_file().contains("lactose_task::"));
    }

    #[test]
    fn the_thread_routes_are_the_real_references_thread_entries() {
        // Every entry of the reference that says a boundary crate creates no thread, or cites
        // section 16, is pinned by `THREAD_ROUTES`, and every route is such an entry.
        let reference: toml::Table =
            toml::from_str(include_str!("../../../crates/core/clippy.toml")).unwrap();
        let found: BTreeSet<&str> = LINT_TABLES
            .iter()
            .flat_map(|table| reference[*table].as_array().unwrap())
            .filter(|entry| {
                let reason = entry["reason"].as_str().unwrap();
                reason.contains("creates no thread")
                    || reason.starts_with("architecture section 16: ")
            })
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        let pinned: BTreeSet<&str> = THREAD_ROUTES.into_iter().collect();
        assert_eq!(pinned.len(), THREAD_ROUTES.len(), "a route is listed twice");
        assert_eq!(found, pinned);
    }

    #[test]
    fn an_entry_under_a_thread_prefix_is_a_listed_route_or_a_listed_non_route() {
        // Review round 2, M6: an adaptor banned with a reason that cites neither "creates no
        // thread" nor section 16 is still a route the check must pin.
        let (ws, mut files) = conforming();
        let added = "rayon::iter::IndexedParallelIterator::enumerate";
        let unpinned = lint_file().replace(
            "disallowed-methods = [\n",
            &format!("disallowed-methods = [\n  {{ path = \"{added}\", reason = \"r\" }},\n"),
        );
        files.insert(REFERENCE_CLIPPY, unpinned.clone());
        files.insert("crates/math/clippy.toml", unpinned);
        assert_eq!(
            run(&ws, &files),
            [format!(
                "crates/core/clippy.toml: disallows `{added}`, which tools/xtask/src/boundary.rs lists neither in `THREAD_ROUTES` nor in `NOT_THREAD_ROUTES`; list it in the one it belongs to"
            )]
        );
        // The non-routes are real entries of the reference, under a prefix, and no route.
        let reference: toml::Table =
            toml::from_str(include_str!("../../../crates/core/clippy.toml")).unwrap();
        let banned: BTreeSet<&str> = LINT_TABLES
            .iter()
            .flat_map(|table| reference[*table].as_array().unwrap())
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        for path in NOT_THREAD_ROUTES {
            assert!(banned.contains(path), "{path}");
            assert!(THREAD_PREFIXES.iter().any(|prefix| path.starts_with(prefix)), "{path}");
            assert!(!THREAD_ROUTES.contains(&path), "{path}");
        }
        for route in THREAD_ROUTES {
            assert!(
                THREAD_PREFIXES.iter().any(|prefix| route.starts_with(prefix))
                    || route.starts_with("lactose_task::"),
                "{route}"
            );
        }
    }

    #[test]
    fn the_real_reference_lint_file_has_the_required_entries() {
        let mut files = Files::default();
        files.insert(REFERENCE_CLIPPY, include_str!("../../../crates/core/clippy.toml"));
        let mut findings = Vec::new();
        reference_lint_file(&files, &mut findings);
        assert_eq!(findings, []);
    }

    #[test]
    fn a_boundary_lint_may_not_be_weakened_for_a_crate_or_a_module() {
        // Review item 1, attributes.
        let (ws, files) = conforming();
        let reported = [
            "#![allow(clippy::disallowed_methods)]",
            "#![expect(clippy::disallowed_types, reason = \"x\")]",
            "#![warn(clippy::disallowed_methods)]",
            "#![allow(clippy::all)]",
            "#![allow(clippy::style)]",
            "#![allow(warnings)]",
            "#![allow(dead_code, warnings)]",
            "#![cfg_attr(not(test), allow(clippy::all))]",
            "mod inner {\n    #![allow(clippy::disallowed_methods)]\n}",
            "#[allow(clippy::disallowed_methods)]\nmod trig {}",
            "#[allow(clippy::all)]\n/// Docs.\n#[cfg(test)]\npub(crate) mod trig;",
            "#[expect(clippy::disallowed_types, reason = \"x\")]\npub mod maps {}",
            // Review of the closure check, finding 4: on an item only `expect` goes stale.
            "#[allow(clippy::disallowed_methods)]\nfn f() {}",
            "#[warn(clippy::disallowed_types)]\nstruct S;",
            "#[cfg_attr(test, allow(clippy::all))]\nimpl S {}",
            // Round 2, finding 1: a comment in the list, and the lints' old names.
            "#[allow(clippy::disallowed_methods /* c */)]\nfn f() {}",
            "#[allow(\n    clippy::disallowed_types, // ]\n)]\nstruct S;",
            "#[allow(renamed_and_removed_lints, clippy::disallowed_method)]\nfn f() {}",
            "#[allow(renamed_and_removed_lints, clippy::disallowed_type)]\nfn f() {}",
        ];
        for attribute in reported {
            let mut files = files.clone();
            files.insert("crates/math/src/lib.rs", format!("{FORBID}{attribute}\n"));
            let found = run(&ws, &files);
            assert_eq!(found.len(), 1, "{attribute:?} gave {found:#?}");
            let line = if attribute.starts_with("mod inner") { 4 } else { 3 };
            let start = format!("crates/math/src/lib.rs: line {line}: `");
            assert!(found[0].starts_with(&start), "{}", found[0]);
        }
        let accepted = [
            // The sanctioned form, and attributes that weaken nothing.
            "#[expect(clippy::disallowed_methods, reason = \"x\")]\nfn f() {}",
            "#[expect(clippy::disallowed_types, reason = \"x\")]\nimpl S {}",
            "#![deny(warnings)]",
            "#![allow(dead_code, reason = \"not warnings, not clippy::all\")]",
            "#![allow(unused_warnings)]",
            "// #![allow(clippy::all)]",
        ];
        for attribute in accepted {
            let source = if attribute.starts_with("#!") || attribute.starts_with("//") {
                format!("{FORBID}{attribute}\n")
            } else {
                format!("{FORBID}struct S;\n{attribute}\n")
            };
            let mut files = files.clone();
            files.insert("crates/math/src/lib.rs", source);
            assert_eq!(run(&ws, &files), NONE, "{attribute:?}");
        }
        // Outside the boundary the same attribute is nobody's business here.
        let mut files = files.clone();
        files.insert("crates/task/src/lib.rs", "#![allow(clippy::all)]\n");
        assert_eq!(run(&ws, &files), NONE);
    }

    #[test]
    fn every_source_file_of_the_package_is_scanned_but_not_a_nested_package() {
        // Review item 10.
        let (ws, mut files) = conforming();
        let bad = "#![allow(clippy::disallowed_methods)]\n";
        for path in [
            "tests/t.rs",
            "benches/b.rs",
            "examples/e.rs",
            "build.rs",
            "src/a/b.rs",
            "generated/g.rs",
        ] {
            files.insert(format!("crates/math/{path}"), bad);
        }
        files.insert("crates/math/fuzz/Cargo.toml", "[package]\nname = \"fuzz\"\n");
        files.insert("crates/math/fuzz/src/main.rs", bad);
        let found = run(&ws, &files);
        assert_eq!(found.len(), 6, "{found:#?}");
        assert!(found.iter().all(|f| f.contains(": line 1: `allow(clippy::disallowed_methods)`")));
        assert!(!found.iter().any(|f| f.contains("fuzz")));
    }

    #[test]
    fn another_clippy_configuration_file_is_reported() {
        // Review item 1, `.clippy.toml`.
        let (ws, mut files) = conforming();
        files.insert(".clippy.toml", "");
        files.insert("crates/.clippy.toml", "");
        files.insert("crates/math/.clippy.toml", "");
        files.insert("crates/task/.clippy.toml", "");
        let found = run(&ws, &files);
        let paths: Vec<&str> = found.iter().map(|f| f.split(": ").next().unwrap()).collect();
        assert_eq!(paths, [".clippy.toml", "crates/.clippy.toml", "crates/math/.clippy.toml"]);
    }

    #[test]
    fn a_lints_table_may_not_weaken_a_boundary_lint() {
        // Review item 1, `[lints]`.
        let (ws, mut files) = conforming();
        files.insert(
            ROOT_MANIFEST,
            "[workspace.lints.clippy]\nall = { level = \"allow\", priority = -1 }\ndisallowed_types = \"deny\"\n[workspace.lints.rust]\nwarnings = \"allow\"\n",
        );
        files.insert(
            "crates/math/Cargo.toml",
            "[lints.clippy]\ndisallowed-methods = \"allow\"\ndisallowed_types = \"warn\"\nstyle = \"warn\"\n",
        );
        // Not a carrier of the boundary lints.
        files.insert("crates/task/Cargo.toml", "[lints.clippy]\nall = \"allow\"\n");
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/Cargo.toml: [lints] sets `clippy::disallowed-methods` to `allow`, weaker than the workspace default; `lactose-math` carries the boundary lints",
                "crates/math/Cargo.toml: [lints] sets `clippy::disallowed_types` to `warn`, weaker than the workspace default; `lactose-math` carries the boundary lints",
                "Cargo.toml: [workspace.lints] sets `warnings` to `allow`, which switches the boundary lints off for every crate that inherits it",
                "Cargo.toml: [workspace.lints] sets `clippy::all` to `allow`, which switches the boundary lints off for every crate that inherits it",
            ]
        );
    }

    #[test]
    fn boundary_dependencies_must_be_on_the_allowlist() {
        let (_, mut files) = conforming();
        files.insert("crates/ecs/README.md", "");
        files.insert("crates/ecs/SAFETY.md", "");
        files.insert("crates/ecs/Cargo.toml", MANIFEST);
        files.insert("crates/ecs/clippy.toml", lint_file());
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
                    ("lactose-math", Dev),
                    ("lactose-rhi-null", Dev),
                ],
            ),
            package(
                "lactose-ecs",
                &[
                    ("lactose-core", Normal),
                    ("lactose-task", Normal),
                    ("lactose-task", Dev),
                    ("lactose-ecs-macros", Normal),
                    ("lactose-reflect", Normal),
                    ("lactose-io", Normal),
                ],
            ),
        ]);
        let found = run(&ws, &files);
        let tail = "in crate-tiers.toml does not allow";
        assert_eq!(
            found,
            [
                format!("crates/core/Cargo.toml: boundary crate `lactose-core` depends on third-party crate `xxhash-rust`, which [boundary_third_party.direct] \"lactose-core\" {tail}"),
                format!("crates/core/Cargo.toml: boundary crate `lactose-core` dev-depends on third-party crate `rand`, which [boundary_third_party.direct] \"lactose-core\" {tail}"),
                format!("crates/ecs/Cargo.toml: boundary crate `lactose-ecs` depends on non-boundary crate `lactose-io`, which [boundary_internal] {tail}"),
                // What `core` wrongly names reaches everything that depends on `core`.
                "crates/ecs/Cargo.toml: boundary crate `lactose-ecs` compiles third-party crate `xxhash-rust` (lactose-ecs -> lactose-core -> xxhash-rust), which is on no `normal` list and not in `transitive` of [boundary_third_party] in crate-tiers.toml".to_owned(),
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` depends on third-party crate `bytemuck`, which [boundary_third_party.direct] \"lactose-math\" {tail}"),
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` depends on non-boundary crate `lactose-task`, which [boundary_internal] {tail}"),
                // Review of the closure check, finding 2: section 8.2's "directly or at any
                // depth" holds for tests. Itself and [test_support] are allowed.
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` dev-depends on non-boundary crate `lactose-io`, which [boundary_internal] {tail}"),
            ]
        );
    }

    #[test]
    fn what_a_boundary_crates_tests_name_is_held_to_the_allowlist_at_any_depth() {
        // Review of the closure check, finding 2: `[boundary_internal] dev` lets a test name
        // `task`, and what `task` compiles is a test's to compile too.
        let (_, mut files) = conforming();
        for role in ["io", "rhi-null", "future"] {
            files.insert(format!("crates/{role}/README.md"), "");
            files.insert(format!("crates/{role}/src/lib.rs"), FORBID);
        }
        files.insert("crates/io/SAFETY.md", "");
        let ws = workspace(vec![
            package("lactose-core", &[]),
            package(
                "lactose-math",
                &[
                    ("lactose-math", Dev),
                    ("lactose-task", Dev),
                    ("lactose-io", Dev),
                    ("lactose-rhi-null", Dev),
                ],
            ),
            package(
                "lactose-task",
                &[("lactose-core", Normal), ("lactose-future", Normal), ("lactose-io", Normal)],
            ),
            package("lactose-io", &[("lactose-net", Normal)]),
            // Round 2, R02: `task` is passed through from `rhi-null` too, as a test may.
            package("lactose-rhi-null", &[("lactose-renderer", Normal), ("lactose-task", Normal)]),
            package("lactose-future", &[]),
        ]);
        let tail = "which [boundary_internal] in crate-tiers.toml does not allow at any depth";
        assert_eq!(
            run(&ws, &files),
            [
                "crates/math/Cargo.toml: boundary crate `lactose-math` dev-depends on non-boundary crate `lactose-io`, which [boundary_internal] in crate-tiers.toml does not allow".to_owned(),
                // `io` is not named again through `task`, nor `net` behind it, nor `task` or
                // what it reaches through `rhi-null`.
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` reaches non-boundary crate `lactose-future` in its tests (lactose-math -> lactose-task -> lactose-future), {tail}"),
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` reaches non-boundary crate `lactose-renderer` in its tests (lactose-math -> lactose-rhi-null -> lactose-renderer), {tail}"),
            ]
        );
    }

    #[test]
    fn a_boundary_crate_may_not_reach_a_refused_workspace_crate_at_any_depth() {
        // Review of the io facade, D2: `lactose-io` added to an allowlisted foundation crate
        // reached every boundary crate above it unreported.
        let (_, mut files) = conforming();
        for role in ["ecs", "io"] {
            files.insert(format!("crates/{role}/README.md"), "");
            files.insert(format!("crates/{role}/SAFETY.md"), "");
        }
        files.insert("crates/ecs/Cargo.toml", MANIFEST);
        files.insert("crates/ecs/clippy.toml", lint_file());
        for role in ["reflect", "future"] {
            files.insert(format!("crates/{role}/README.md"), "");
            files.insert(format!("crates/{role}/src/lib.rs"), FORBID);
        }
        files.insert("crates/reflect/clippy.toml", outside_world_file());
        let ws = workspace(vec![
            package("lactose-core", &[]),
            package(
                "lactose-math",
                &[("lactose-reflect", Normal), ("lactose-ecs", Normal), ("lactose-io", Dev)],
            ),
            package(
                "lactose-ecs",
                &[
                    ("lactose-core", Normal),
                    ("lactose-task", Normal),
                    ("lactose-reflect", Normal),
                    ("lactose-io", Normal),
                ],
            ),
            package(
                "lactose-reflect",
                &[
                    ("lactose-core", Normal),
                    ("lactose-io", Normal),
                    ("lactose-future", Build),
                    ("lactose-app", Dev),
                ],
            ),
            package("lactose-io", &[("lactose-task", Normal), ("lactose-net", Normal)]),
            package("lactose-task", &[("lactose-core", Normal)]),
            package("lactose-future", &[]),
        ]);
        let tail = "which [boundary_internal] in crate-tiers.toml does not allow at any depth";
        assert_eq!(
            run(&ws, &files),
            [
                // The direct edge keeps its own words, and is not reported again at depth.
                "crates/ecs/Cargo.toml: boundary crate `lactose-ecs` depends on non-boundary crate `lactose-io`, which [boundary_internal] in crate-tiers.toml does not allow".to_owned(),
                // Build edges are followed; `task` is `ecs`'s own to compile.
                format!("crates/ecs/Cargo.toml: boundary crate `lactose-ecs` reaches non-boundary crate `lactose-future` (lactose-ecs -> lactose-reflect -> lactose-future), {tail}"),
                "crates/math/Cargo.toml: boundary crate `lactose-math` dev-depends on non-boundary crate `lactose-io`, which [boundary_internal] in crate-tiers.toml does not allow".to_owned(),
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` reaches non-boundary crate `lactose-future` (lactose-math -> lactose-reflect -> lactose-future), {tail}"),
                // The refused dev edge does not hide the shipping path. Nothing past a refused
                // crate is reported (`io` -> `net`), nor `task` through `ecs`, a boundary crate
                // whose own reach is judged as its own, nor a dev edge of `reflect`.
                format!("crates/math/Cargo.toml: boundary crate `lactose-math` reaches non-boundary crate `lactose-io` (lactose-math -> lactose-reflect -> lactose-io), {tail}"),
            ]
        );
    }

    #[test]
    fn what_a_crate_on_a_boundary_crates_own_list_reaches_is_judged_for_that_crate() {
        // Review of the closure check, finding 5: `task` is passed through for `ecs` alone,
        // so a refused crate behind it is `ecs`'s finding, and no crate above `ecs` repeats it.
        let (_, mut files) = conforming();
        files.insert("crates/ecs/README.md", "");
        files.insert("crates/ecs/SAFETY.md", "");
        files.insert("crates/ecs/Cargo.toml", MANIFEST);
        files.insert("crates/ecs/clippy.toml", lint_file());
        files.insert("crates/future/README.md", "");
        files.insert("crates/future/src/lib.rs", FORBID);
        // Round 2, R14: the same holds when only `math`'s tests name `ecs`. A boundary crate
        // is not a test root, though its tests may name `task`, which `ecs` passes through.
        for kind in [Normal, Dev] {
            let ws = workspace(vec![
                package("lactose-core", &[]),
                package("lactose-math", &[("lactose-ecs", kind)]),
                package("lactose-ecs", &[("lactose-core", Normal), ("lactose-task", Normal)]),
                package("lactose-task", &[("lactose-core", Normal), ("lactose-future", Normal)]),
                package("lactose-future", &[]),
            ]);
            assert_eq!(
                run(&ws, &files),
                [
                    "crates/ecs/Cargo.toml: boundary crate `lactose-ecs` reaches non-boundary crate `lactose-future` (lactose-ecs -> lactose-task -> lactose-future), which [boundary_internal] in crate-tiers.toml does not allow at any depth"
                ],
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_file_a_path_attribute_names_is_scanned_wherever_it_lives() {
        // Review of the closure check, finding 3: the SOUR protocol lives in `tools/xtask`
        // and is compiled into boundary crates' tests through `#[path]`.
        let (ws, mut files) = conforming();
        let protocol = "tools/xtask/src/harness/protocol.rs";
        files.insert(
            "crates/math/tests/sour.rs",
            format!(
                "{FORBID}#[path = \"../../../tools/xtask/src/harness/protocol.rs\"]\n\
                 #[expect(clippy::wrong_self_convention, reason = \"x\")]\nmod protocol;\n\
                 #[cfg_attr(unix, path = \"../../missing.rs\")]\nmod missing;\n\
                 #[path = \"../../../../outside.rs\"]\nmod outside;\n"
            ),
        );
        files.insert(
            protocol,
            "#![allow(clippy::disallowed_methods)]\n#[path = \"nested/inner.rs\"]\nmod inner;\n",
        );
        // A cycle back, and a file of the package itself, are scanned once.
        files.insert(
            "tools/xtask/src/harness/nested/inner.rs",
            "#[allow(clippy::disallowed_types)]\nfn f() {}\n#[path = \"../protocol.rs\"]\nmod back;\n\
             #[path = \"../../../../../crates/math/src/lib.rs\"]\nmod again;\n",
        );
        // Round 2, R05: `../../../../outside.rs` climbs out of the repository; it is not the
        // file at its root.
        files.insert("outside.rs", "#![allow(clippy::all)]\n");
        // A crate outside the boundary that includes the same file is not this check's.
        files.insert(
            "crates/task/tests/sour.rs",
            "#[path = \"../../../tools/xtask/src/harness/protocol.rs\"]\nmod protocol;\n",
        );
        let unread = "which is not a source file in the repository, so the lint attributes of \
                      boundary crate `lactose-math` in it cannot be checked";
        let sanctioned = "the only sanctioned form is `#[expect(clippy::disallowed_methods, \
                          reason = \"...\")]` (or `disallowed_types`) on the one item that \
                          needs it";
        assert_eq!(
            run(&ws, &files),
            [
                format!(
                    "crates/math/tests/sour.rs: line 6: `#[path]` names `../../missing.rs`, {unread}"
                ),
                format!(
                    "crates/math/tests/sour.rs: line 8: `#[path]` names `../../../../outside.rs`, {unread}"
                ),
                format!(
                    "tools/xtask/src/harness/protocol.rs: line 1: `allow(clippy::disallowed_methods)` applies to a whole crate or module of boundary crate `lactose-math`; {sanctioned}"
                ),
                format!(
                    "tools/xtask/src/harness/nested/inner.rs: line 1: `allow(clippy::disallowed_types)` never goes stale, on an item of boundary crate `lactose-math`; {sanctioned}"
                ),
            ]
        );
    }

    #[test]
    fn a_crate_every_boundary_crate_may_compile_carries_the_outside_world_bans() {
        // Review of the closure check, finding 6.
        let (mut ws, mut files) = conforming();
        ws.packages.push(package("lactose-reflect", &[("lactose-core", Normal)]));
        files.insert("crates/reflect/README.md", "");
        files.insert("crates/reflect/Cargo.toml", MANIFEST);
        files.insert("crates/reflect/src/lib.rs", FORBID);
        let path = "crates/reflect/clippy.toml";
        assert_eq!(
            run(&ws, &files),
            [
                "crates/reflect/clippy.toml: `lactose-reflect` is in [boundary_internal] `any` of crate-tiers.toml, so it compiles into every boundary crate and must carry a clippy.toml with the entries of crates/core/clippy.toml that ban files, the environment, subprocesses, the network and thread creation"
            ]
        );
        // The outside-world entries alone, or with the rest of the reference.
        for carried in [outside_world_file(), lint_file()] {
            files.insert(path, carried);
            assert_eq!(run(&ws, &files), NONE);
        }
        let reworded = outside_world_file()
            .replace("  { path = \"std::fs::read\", reason = \"r\" },\n", "")
            .replace(
                "{ path = \"std::env::var\", reason = \"r\" }",
                "{ path = \"std::env::var\", reason = \"s\" }",
            )
            .replace(
                "  \"std::fs::File\",\n",
                "  \"std::fs::File\",\n  \"std::net::UdpSocket\",\n",
            );
        files.insert(path, reworded);
        let table = |name: &str| format!("crates/reflect/clippy.toml: `disallowed-{name}`");
        assert_eq!(
            run(&ws, &files),
            [
                format!(
                    "{} lacks the entry for `std::fs::read` that crates/core/clippy.toml has; copy it as written there",
                    table("methods")
                ),
                format!(
                    "{} lacks the entry for `std::env::var` that crates/core/clippy.toml has; copy it as written there",
                    table("methods")
                ),
                format!(
                    "{} has an entry for `std::env::var` that crates/core/clippy.toml does not have as written; the two must agree on every file, environment, subprocess, network and thread entry",
                    table("methods")
                ),
                format!(
                    "{} has an entry for `std::net::UdpSocket` that crates/core/clippy.toml does not have as written; the two must agree on every file, environment, subprocess, network and thread entry",
                    table("types")
                ),
            ]
        );
        // The ways around a lint file are closed for it as for a boundary crate.
        files.insert(path, outside_world_file());
        files.insert("crates/reflect/.clippy.toml", "");
        files.insert(
            "crates/reflect/Cargo.toml",
            "[lints.clippy]\ndisallowed_methods = \"allow\"\n",
        );
        files.insert(
            "crates/reflect/src/lib.rs",
            format!("{FORBID}#![allow(clippy::disallowed_types)]\n"),
        );
        let found = run(&ws, &files);
        assert_eq!(found.len(), 3, "{found:#?}");
        assert!(found[0].starts_with("crates/reflect/Cargo.toml: [lints] sets"));
        assert!(found[1].contains(
            "`allow(clippy::disallowed_types)` applies to a whole crate or module of [boundary_internal] crate `lactose-reflect`"
        ));
        assert!(found[2].starts_with("crates/reflect/.clippy.toml: clippy reads"));
    }

    #[test]
    fn the_outside_world_entries_of_the_real_reference_are_its_section_8_2_entries() {
        // Every entry that keeps the outside world out cites section 8.2 and nothing else does,
        // so an entry added to one family is not missed by the crates that carry the families.
        let reference: toml::Table =
            toml::from_str(include_str!("../../../crates/core/clippy.toml")).unwrap();
        let mut families = 0;
        for table in LINT_TABLES {
            let entries = reference[table].as_array().unwrap();
            for entry in entries {
                let path = entry["path"].as_str().unwrap();
                let cites =
                    entry["reason"].as_str().unwrap().starts_with("architecture section 8.2: ");
                let family = OUTSIDE_WORLD.iter().any(|prefix| path.starts_with(prefix));
                assert_eq!(cites, family, "{path}");
                families += usize::from(family);
            }
        }
        assert!(families >= OUTSIDE_WORLD.len(), "{families}");
        for prefix in OUTSIDE_WORLD {
            let found = LINT_TABLES
                .iter()
                .flat_map(|t| reference[*t].as_array().unwrap())
                .any(|entry| entry["path"].as_str().unwrap().starts_with(prefix));
            assert!(found, "no entry under `{prefix}`");
        }
    }

    #[test]
    fn everything_a_boundary_crate_compiles_must_be_vetted() {
        // Review item 14: an allowlisted crate, and an allowlisted workspace crate, each
        // bringing in a crate nobody reviewed.
        let (_, mut files) = conforming();
        files.insert("crates/reflect/README.md", "");
        files.insert("crates/reflect/src/lib.rs", FORBID);
        files.insert("crates/reflect/clippy.toml", outside_world_file());
        let ws = workspace_with(
            vec![
                package("lactose-core", &[("bytemuck", Normal), ("lactose-reflect", Normal)]),
                package("lactose-reflect", &[("tracing", Normal), ("trybuild", Dev)]),
            ],
            &[("bytemuck", &["bytemuck_derive", "fast-float"]), ("tracing", &["tracing-core"])],
        );
        let found = run(&ws, &files);
        let tail = "which is on no `normal` list and not in `transitive` of [boundary_third_party] in crate-tiers.toml";
        assert_eq!(
            found,
            [
                format!(
                    "crates/core/Cargo.toml: boundary crate `lactose-core` compiles third-party crate `fast-float` (lactose-core -> bytemuck -> fast-float), {tail}"
                ),
                format!(
                    "crates/core/Cargo.toml: boundary crate `lactose-core` compiles third-party crate `tracing` (lactose-core -> lactose-reflect -> tracing), {tail}"
                ),
                format!(
                    "crates/core/Cargo.toml: boundary crate `lactose-core` compiles third-party crate `tracing-core` (lactose-core -> lactose-reflect -> tracing -> tracing-core), {tail}"
                ),
            ]
        );
    }

    #[test]
    fn the_pin_is_the_only_rust_flag_setting_in_any_cargo_configuration() {
        // Review item 2, cargo configuration files.
        let (ws, mut files) = conforming();
        files.insert(
            CARGO_CONFIG,
            "# rustflags = [\"-C\", \"target-cpu=x86-64-v3\"]\n[alias]\nx = \"run\"\n",
        );
        assert_eq!(
            run(&ws, &files),
            [
                ".cargo/config.toml: the x86-64 pin is missing: [target.'cfg(target_arch = \"x86_64\")'] must set rustflags = [\"-C\", \"target-cpu=x86-64-v3\"]"
            ]
        );

        let only = "the only rust flags permitted in a cargo configuration file are the pin, [target.'cfg(target_arch = \"x86_64\")'] rustflags = [\"-C\", \"target-cpu=x86-64-v3\"] in .cargo/config.toml";
        files.insert(
            CARGO_CONFIG,
            format!("{PIN}[build]\nrustflags = [\"-C\", \"target_feature=+avx512f\"]\n[target.x86_64-unknown-linux-gnu]\nrustflags = [\"-Ctarget_cpu=native\"]\n[env]\nCLIPPY_CONF_DIR = \"elsewhere\"\n"),
        );
        files.insert("tools/cook/.cargo/config.toml", PIN);
        assert_eq!(
            run(&ws, &files),
            [
                ".cargo/config.toml: line 4: sets `target-feature`; target features come from the pinned target CPU only".to_owned(),
                ".cargo/config.toml: line 6: selects a target CPU (`target-cpu` `native`); the only permitted setting is the x86-64-v3 pin in .cargo/config.toml".to_owned(),
                ".cargo/config.toml: line 8: names CLIPPY_CONF_DIR, which points clippy away from each crate's clippy.toml".to_owned(),
                format!(".cargo/config.toml: sets `build.rustflags`; {only}"),
                format!(".cargo/config.toml: sets `target.x86_64-unknown-linux-gnu.rustflags`; {only}"),
                format!("tools/cook/.cargo/config.toml: sets `target.cfg(target_arch = \"x86_64\").rustflags`; {only}"),
            ]
        );

        // The pin with anything added to it is not the pin.
        files.insert(CARGO_CONFIG, PIN.replace("\"]", "\", \"-C\", \"target-feature=+fma\"]"));
        let files = without(&files, &["tools/cook/.cargo/config.toml"]);
        let found = run(&ws, &files);
        assert_eq!(found.len(), 3, "{found:#?}");
        assert!(found[2].contains("the x86-64 pin is missing"));
    }

    #[test]
    fn a_cargo_configuration_no_check_reads_is_refused() {
        // D123: an include key and a linked configuration reach cargo past every check.
        let (ws, mut files) = conforming();
        files.insert(CARGO_CONFIG, format!("include = [\"pin.toml\"]\n{PIN}"));
        files.insert_link("tools/cook/.cargo");
        assert_eq!(
            run(&ws, &files),
            [
                ".cargo/config.toml: has an include key; cargo reads the included files, which no check does",
                "tools/cook/.cargo: is a symbolic link; cargo follows it, which no check does, so a cargo configuration file or directory is committed as itself",
            ]
        );
    }

    #[test]
    fn workflows_and_scripts_may_not_touch_flags_cpu_features_or_lints() {
        // Review items 1 and 2, build configuration.
        let (ws, mut files) = conforming();
        files.insert(
            ".github/workflows/ci.yml",
            "env:\n  # Never set RUSTFLAGS here.\n  RUSTFLAGS: -D warnings\njobs:\n  a:\n    run: cargo rustc -- -C target_cpu=native\n    run: cargo --config 'build.rustflags=[\"-Ctarget-feature=+avx2\"]' build\n    run: cargo clippy -- -A clippy::disallowed_methods -Aclippy::all --allow=warnings -A dead_code\n    env:\n      CLIPPY_CONF_DIR: /tmp\n",
        );
        files.insert(
            "scripts/bench.sh",
            "CARGO_ENCODED_RUSTFLAGS=x cargo bench\ncargo rustc -- --codegen target-cpu=haswell\n",
        );
        // Files that are neither configuration nor scripts are not scanned.
        files.insert("docs/notes.toml", "x = \"RUSTFLAGS target-cpu=native\"\n");
        let found = run(&ws, &files);
        let expect = [
            ".github/workflows/ci.yml: line 3: names RUSTFLAGS;",
            ".github/workflows/ci.yml: line 6: selects a target CPU (`target-cpu` `native`);",
            ".github/workflows/ci.yml: line 7: names RUSTFLAGS;",
            ".github/workflows/ci.yml: line 7: sets `target-feature`;",
            ".github/workflows/ci.yml: line 8: allows `clippy::disallowed_methods` from the command line",
            ".github/workflows/ci.yml: line 8: allows `clippy::all` from the command line",
            ".github/workflows/ci.yml: line 8: allows `warnings` from the command line",
            ".github/workflows/ci.yml: line 10: names CLIPPY_CONF_DIR,",
            "scripts/bench.sh: line 1: names RUSTFLAGS;",
            "scripts/bench.sh: line 2: selects a target CPU (`target-cpu` `haswell`);",
        ];
        assert_eq!(found.len(), expect.len(), "{found:#?}");
        for (found, expect) in found.iter().zip(expect) {
            assert!(found.starts_with(expect), "{found}\n  expected: {expect}");
        }
    }
}
