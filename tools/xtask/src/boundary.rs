//! The static boundary checks: the facts about files and manifests that architecture
//! sections 8.2, 8.3, 20 and 21 require and that need no compiler to verify.
//!
//! What clippy enforces through `clippy.toml` is not repeated here. This module checks that
//! the configuration clippy reads is the agreed one, and that none of the ways of switching
//! it off is in use: a lint attribute on a whole crate or module, an `allow` on an item,
//! another configuration file, a `[lints]` table, or a flag in a workflow or script.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::policy::Policy;
use crate::repo::{Files, is_build_configuration, is_cargo_config, unread_cargo_configuration};
use crate::rust_source::{Attribute, Declaration, Impl, Items, attributes, items};
use crate::tiers::{TierFile, turning_on};
use crate::workspace::{DepKind, DepSource, Package, Workspace};
use crate::{ARCHITECTURE, Finding, POLICY_FILE, TIER_FILE};

/// The boundary lint configuration every boundary crate copies byte for byte.
pub const REFERENCE_CLIPPY: &str = "crates/core/clippy.toml";

/// The cargo configuration file that carries the x86-64 pin.
pub const CARGO_CONFIG: &str = ".cargo/config.toml";

/// The workspace manifest.
const ROOT_MANIFEST: &str = "Cargo.toml";

/// The harness protocol, which every simulation test includes by `#[path]` (section 8.3).
const HARNESS_PROTOCOL: &str = "tools/xtask/src/harness/protocol.rs";

/// The trait that declares an asset kind the simulation reads (D99).
const SIMULATION_ASSET: &str = "SimulationAsset";

/// The trait that decodes an asset kind from cooked bytes (D99).
const LOADER: &str = "Loader";

/// The traits of `lactose_sim` through which the input sync point writes what enters a tick
/// into the world: players' joins and leaves, their inputs, and staged state.
const APPLYING_TRAITS: [&str; 3] = ["SessionApplier", "InputApplier", "StagedState"];

/// The trait of `lactose_reflect` that describes a type a world stores: its key, its fields
/// and its persistence policy. `#[component]` and `#[resource]` write it; written by hand, it
/// declares a component or a resource that no attribute shows.
const DESCRIBING_TRAIT: &str = "Reflect";

/// The `[target]` key and the flags of the pin (architecture section 8.2).
const PIN_KEY: &str = r#"cfg(target_arch = "x86_64")"#;
const PIN_CPU: &str = "x86-64-v3";
const PIN_FLAGS: [&str; 2] = ["-C", "target-cpu=x86-64-v3"];

/// Every entry of the reference lint file that is neither a thread route nor in
/// [`NOT_THREAD_ROUTES`], in the file's order: the float methods and the trait routes to them,
/// the executor's worker index and the methods of its re-export, files, the environment,
/// subprocesses and the network, CPUID, the types, and the macros of run-time CPU feature
/// detection.
/// The check requires the reference's entries to be exactly this list, [`THREAD_ROUTES`] and
/// [`NOT_THREAD_ROUTES`], each in the table [`pinned_table`] names, so a ban deleted from the
/// reference and every copy is reported, and so is one moved to another table, where clippy
/// no longer applies it, or one added without being pinned here. `allow-invalid` keeps clippy
/// quiet about an entry whatever happens to it, and no test uses most of them.
const REQUIRED_LINT_PATHS: [&str; 279] = [
    "f32::min",
    "f32::max",
    "f32::clamp",
    "f32::powi",
    "f32::total_cmp",
    "f32::to_bits",
    "f32::copysign",
    "f32::is_sign_negative",
    "f32::sin",
    "f32::cos",
    "f32::tan",
    "f32::asin",
    "f32::acos",
    "f32::atan",
    "f32::atan2",
    "f32::sin_cos",
    "f32::sinh",
    "f32::cosh",
    "f32::tanh",
    "f32::exp",
    "f32::exp2",
    "f32::exp_m1",
    "f32::ln",
    "f32::ln_1p",
    "f32::log",
    "f32::log2",
    "f32::log10",
    "f32::powf",
    "f32::cbrt",
    "f32::hypot",
    "f64::min",
    "f64::max",
    "f64::clamp",
    "f64::powi",
    "f64::total_cmp",
    "f64::to_bits",
    "f64::copysign",
    "f64::is_sign_negative",
    "f64::sin",
    "f64::cos",
    "f64::tan",
    "f64::asin",
    "f64::acos",
    "f64::atan",
    "f64::atan2",
    "f64::sin_cos",
    "f64::sinh",
    "f64::cosh",
    "f64::tanh",
    "f64::exp",
    "f64::exp2",
    "f64::exp_m1",
    "f64::ln",
    "f64::ln_1p",
    "f64::log",
    "f64::log2",
    "f64::log10",
    "f64::powf",
    "f64::cbrt",
    "f64::hypot",
    "f32::algebraic_add",
    "f32::algebraic_sub",
    "f32::algebraic_mul",
    "f32::algebraic_div",
    "f32::algebraic_rem",
    "f32::to_le_bytes",
    "f32::to_be_bytes",
    "f32::to_ne_bytes",
    "f32::abs_sub",
    "f64::algebraic_add",
    "f64::algebraic_sub",
    "f64::algebraic_mul",
    "f64::algebraic_div",
    "f64::algebraic_rem",
    "f64::to_le_bytes",
    "f64::to_be_bytes",
    "f64::to_ne_bytes",
    "f64::abs_sub",
    "f32::is_sign_positive",
    "f32::rem_euclid",
    "f32::div_euclid",
    "f32::mul_add",
    "f64::is_sign_positive",
    "f64::rem_euclid",
    "f64::div_euclid",
    "f64::mul_add",
    "f32::asinh",
    "f32::acosh",
    "f32::atanh",
    "f64::asinh",
    "f64::acosh",
    "f64::atanh",
    "simba::scalar::RealField::max",
    "simba::scalar::RealField::min",
    "simba::scalar::RealField::copysign",
    "simba::scalar::RealField::is_sign_positive",
    "simba::scalar::RealField::is_sign_negative",
    "simba::simd::SimdRealField::simd_copysign",
    "simba::simd::SimdSigned::is_simd_positive",
    "simba::simd::SimdSigned::is_simd_negative",
    "num_traits::sign::Signed::is_positive",
    "num_traits::sign::Signed::is_negative",
    "glam::Vec2::copysign",
    "glam::Vec2::is_negative_bitmask",
    "glam::Vec2::is_negative_mask",
    "glam::Vec2::rem_euclid",
    "glam::Vec2::div_euclid",
    "glam::Vec3::copysign",
    "glam::Vec3::is_negative_bitmask",
    "glam::Vec3::is_negative_mask",
    "glam::Vec3::rem_euclid",
    "glam::Vec3::div_euclid",
    "glam::Vec3A::copysign",
    "glam::Vec3A::is_negative_bitmask",
    "glam::Vec3A::is_negative_mask",
    "glam::Vec3A::rem_euclid",
    "glam::Vec3A::div_euclid",
    "glam::Vec4::copysign",
    "glam::Vec4::is_negative_bitmask",
    "glam::Vec4::is_negative_mask",
    "glam::Vec4::rem_euclid",
    "glam::Vec4::div_euclid",
    "glam::DVec2::copysign",
    "glam::DVec2::is_negative_bitmask",
    "glam::DVec2::is_negative_mask",
    "glam::DVec2::rem_euclid",
    "glam::DVec2::div_euclid",
    "glam::DVec3::copysign",
    "glam::DVec3::is_negative_bitmask",
    "glam::DVec3::is_negative_mask",
    "glam::DVec3::rem_euclid",
    "glam::DVec3::div_euclid",
    "glam::DVec4::copysign",
    "glam::DVec4::is_negative_bitmask",
    "glam::DVec4::is_negative_mask",
    "glam::DVec4::rem_euclid",
    "glam::DVec4::div_euclid",
    "num_complex::Complex::cis",
    "num_complex::Complex::norm",
    "num_complex::Complex::arg",
    "num_complex::Complex::to_polar",
    "num_complex::Complex::from_polar",
    "num_complex::Complex::exp",
    "num_complex::Complex::ln",
    "num_complex::Complex::sqrt",
    "num_complex::Complex::cbrt",
    "num_complex::Complex::powf",
    "num_complex::Complex::log",
    "num_complex::Complex::powc",
    "num_complex::Complex::expf",
    "num_complex::Complex::sin",
    "num_complex::Complex::cos",
    "num_complex::Complex::tan",
    "num_complex::Complex::asin",
    "num_complex::Complex::acos",
    "num_complex::Complex::atan",
    "num_complex::Complex::sinh",
    "num_complex::Complex::cosh",
    "num_complex::Complex::tanh",
    "num_complex::Complex::asinh",
    "num_complex::Complex::acosh",
    "num_complex::Complex::atanh",
    "num_complex::Complex::finv",
    "num_complex::Complex::fdiv",
    "num_complex::Complex::exp2",
    "num_complex::Complex::log2",
    "num_complex::Complex::log10",
    "glamx::MatExt::symmetric_eigen",
    "glamx::MatExt::symmetric_eigenvalues",
    "glamx::MatExt::svd",
    "approx::UlpsEq::ulps_eq",
    "approx::UlpsEq::ulps_ne",
    "lactose_task::JobContext::slot",
    "lactose_task::ChildContext::slot",
    "lactose_task::Executor::frame_worker_count",
    "lactose_task::Executor::slot_count",
    "lactose_task::Executor::spawn",
    "lactose_task::Executor::run_graph",
    "lactose_task::Executor::run_job",
    "lactose_task::Executor::sleep_until",
    "lactose_task::Executor::execution_log",
    "lactose_task::Executor::chaos_config",
    "lactose_task::Platform::topology",
    "lactose_task::Platform::hardware_threads",
    "std::fs::canonicalize",
    "std::fs::copy",
    "std::fs::create_dir",
    "std::fs::create_dir_all",
    "std::fs::exists",
    "std::fs::hard_link",
    "std::fs::metadata",
    "std::fs::read",
    "std::fs::read_dir",
    "std::fs::read_link",
    "std::fs::read_to_string",
    "std::fs::remove_dir",
    "std::fs::remove_dir_all",
    "std::fs::remove_file",
    "std::fs::rename",
    "std::fs::set_permissions",
    "std::fs::set_times",
    "std::fs::set_times_nofollow",
    "std::fs::soft_link",
    "std::fs::symlink_metadata",
    "std::fs::write",
    "std::path::Path::canonicalize",
    "std::path::Path::exists",
    "std::path::Path::is_dir",
    "std::path::Path::is_file",
    "std::path::Path::is_symlink",
    "std::path::Path::metadata",
    "std::path::Path::read_dir",
    "std::path::Path::read_link",
    "std::path::Path::symlink_metadata",
    "std::path::Path::try_exists",
    "std::os::unix::fs::chown",
    "std::os::unix::fs::chroot",
    "std::os::unix::fs::lchown",
    "std::os::unix::fs::symlink",
    "std::os::windows::fs::symlink_dir",
    "std::os::windows::fs::symlink_file",
    "std::env::args",
    "std::env::args_os",
    "std::env::current_dir",
    "std::env::current_exe",
    "std::env::home_dir",
    "std::env::join_paths",
    "std::env::remove_var",
    "std::env::set_current_dir",
    "std::env::set_var",
    "std::env::split_paths",
    "std::env::temp_dir",
    "std::env::var",
    "std::env::var_os",
    "std::env::vars",
    "std::env::vars_os",
    "std::path::absolute",
    "std::io::stdin",
    "std::io::IsTerminal::is_terminal",
    "std::process::id",
    "std::os::unix::process::parent_id",
    "std::backtrace::Backtrace::capture",
    "std::net::ToSocketAddrs::to_socket_addrs",
    "core::arch::x86_64::__cpuid",
    "core::arch::x86_64::__cpuid_count",
    "core::arch::x86_64::__get_cpuid_max",
    "std::time::Instant",
    "std::time::SystemTime",
    "std::collections::HashMap",
    "std::collections::HashSet",
    "std::hash::RandomState",
    "hashbrown::HashMap",
    "hashbrown::HashSet",
    "num_traits::Float",
    "num_traits::real::Real",
    "num_complex::Complex",
    "glamx::SymmetricEigen3",
    "glamx::SymmetricEigen3A",
    "glamx::DSymmetricEigen3",
    "glamx::Svd2",
    "glamx::DSvd2",
    "glamx::Svd3",
    "glamx::Svd3A",
    "glamx::DSvd3",
    "simba::simd::WideF32x4",
    "simba::simd::WideF32x8",
    "simba::simd::WideF64x4",
    "std::fs::File",
    "std::fs::OpenOptions",
    "std::fs::DirBuilder",
    "std::net::TcpListener",
    "std::net::TcpStream",
    "std::net::UdpSocket",
    "std::os::unix::net::UnixDatagram",
    "std::os::unix::net::UnixListener",
    "std::os::unix::net::UnixStream",
    "std::process::Command",
    "std::arch::is_x86_feature_detected",
    "std::arch::is_aarch64_feature_detected",
];

/// The pinned entries that belong under `disallowed-macros`: run-time CPU feature detection
/// (section 8.2, D28). Each is named under `std::arch`, which clippy resolves to the same
/// macro as the root path `std::is_x86_feature_detected!`; the aarch64 one has no root path.
const PINNED_MACROS: [&str; 2] =
    ["std::arch::is_x86_feature_detected", "std::arch::is_aarch64_feature_detected"];

/// The pinned entries that belong under `disallowed-types`, in the reference's order; every
/// other pinned entry outside [`PINNED_MACROS`] belongs under `disallowed-methods`.
const PINNED_TYPES: [&str; 37] = [
    "std::time::Instant",
    "std::time::SystemTime",
    "std::collections::HashMap",
    "std::collections::HashSet",
    "std::hash::RandomState",
    "hashbrown::HashMap",
    "hashbrown::HashSet",
    "num_traits::Float",
    "num_traits::real::Real",
    "num_complex::Complex",
    "glamx::SymmetricEigen3",
    "glamx::SymmetricEigen3A",
    "glamx::DSymmetricEigen3",
    "glamx::Svd2",
    "glamx::DSvd2",
    "glamx::Svd3",
    "glamx::Svd3A",
    "glamx::DSvd3",
    "simba::simd::WideF32x4",
    "simba::simd::WideF32x8",
    "simba::simd::WideF64x4",
    "std::thread::LocalKey",
    "rayon::iter::plumbing::Producer",
    "rayon::iter::plumbing::UnindexedProducer",
    "rayon::iter::plumbing::ProducerCallback",
    "rayon::iter::plumbing::Consumer",
    "rayon::iter::plumbing::UnindexedConsumer",
    "std::fs::File",
    "std::fs::OpenOptions",
    "std::fs::DirBuilder",
    "std::net::TcpListener",
    "std::net::TcpStream",
    "std::net::UdpSocket",
    "std::os::unix::net::UnixDatagram",
    "std::os::unix::net::UnixListener",
    "std::os::unix::net::UnixStream",
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
/// index, Rapier's multibody paths into `matrixmultiply`, the Rapier and parry angles that call
/// the platform library, the core count and thread-local state.
const NOT_THREAD_ROUTES: [&str; 11] = [
    "rayon::current_thread_index",
    "rayon::ThreadPool::current_thread_index",
    "rapier3d_f64::dynamics::Multibody::inverse_kinematics",
    "rapier3d_f64::dynamics::Multibody::inverse_kinematics_delta",
    "rapier3d_f64::dynamics::Multibody::inverse_kinematics_delta_with_jacobian",
    "rapier3d_f64::dynamics::MultibodyJointSet::insert",
    "rapier3d_f64::dynamics::MultibodyJointSet::insert_kinematic",
    "rapier3d_f64::dynamics::RevoluteJoint::angle",
    "parry3d_f64::math::VectorExt::angle",
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

/// The lint tables of a lint file.
const LINT_TABLES: [&str; 3] = ["disallowed-methods", "disallowed-types", "disallowed-macros"];

/// Lint levels that leave a lint weaker than `deny`.
const WEAK_LEVELS: [&str; 3] = ["allow", "expect", "warn"];

/// Whether `lint` is one of the lints that carry the boundary rules, or a group that contains
/// them. `clippy::disallowed_methods`, `clippy::disallowed_types` and
/// `clippy::disallowed_macros` are in `clippy::style` and `clippy::all`, and CI turns every
/// warning into an error through `warnings`. Clippy still honours the first two lints' old
/// names, `disallowed_method` and `disallowed_type`.
fn is_boundary_lint(lint: &str) -> bool {
    matches!(
        lint.replace('-', "_").as_str(),
        "clippy::disallowed_methods"
            | "clippy::disallowed_types"
            | "clippy::disallowed_macros"
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
    unsafe_list(policy, files, &mut findings);
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
            unnamed_allowlist_entries(tiers, package, &mut findings);
        }
    }
    findings.extend(stray_configs.into_iter().map(|path| {
        Finding::new(
            path,
            "clippy reads `.clippy.toml` in preference to `clippy.toml`, so this file replaces \
             the boundary lint configuration of every crate at or below it",
        )
    }));
    harness_packages(policy, ws, files, &mut findings);
    simulation_assets(tiers, ws, files, &mut findings);
    registered_outside_the_lint_file(tiers, policy, ws, files, &mut findings);
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

/// Architecture section 21's table and `[unsafe_crates]` name the same crates. The section is
/// found by its title, not its number; the table is the first one in it, and each row's
/// first cell names its crates in backticks, by directory under `crates/` or by a path with
/// a `/` (`tools/cook`), each `lactose-` and its last segment as a package.
fn unsafe_list(policy: &Policy, files: &Files, findings: &mut Vec<Finding>) {
    const TITLE: &str = "Unsafe policy";
    let Some(text) = files.text(ARCHITECTURE) else {
        findings.push(Finding::new(
            ARCHITECTURE,
            format!("is missing, so [unsafe_crates] in {POLICY_FILE} cannot be checked against it"),
        ));
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    let heading = lines.iter().position(|line| {
        line.strip_prefix("## ").and_then(|title| title.split_once(". ")).is_some_and(
            |(number, title)| {
                !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) && title == TITLE
            },
        )
    });
    let Some(heading) = heading else {
        findings.push(Finding::new(
            ARCHITECTURE,
            format!(
                "has no `## <n>. {TITLE}` section, so [unsafe_crates] in {POLICY_FILE} cannot be \
                 checked against it"
            ),
        ));
        return;
    };
    let section = lines[heading + 1..].iter().take_while(|line| !line.starts_with("## "));
    let table: Vec<(usize, &str)> = section
        .enumerate()
        .map(|(at, line)| (heading + 2 + at, line.trim()))
        .skip_while(|(_, line)| !line.starts_with('|'))
        .take_while(|(_, line)| line.starts_with('|'))
        .collect();
    // A header row and its separator, then the rows.
    if table.len() < 3 {
        findings.push(Finding::new(
            ARCHITECTURE,
            format!(
                "line {}: the {TITLE} section has no table of crates, so [unsafe_crates] in \
                 {POLICY_FILE} cannot be checked against it",
                heading + 1
            ),
        ));
        return;
    }
    let mut listed = BTreeSet::new();
    for &(line, row) in &table[2..] {
        let cell = row.trim_start_matches('|').split('|').next().unwrap_or("");
        let names: Vec<&str> = cell.split('`').skip(1).step_by(2).collect();
        if names.is_empty() {
            findings.push(Finding::new(
                ARCHITECTURE,
                format!("line {line}: a row of the {TITLE} table names no crate in backticks"),
            ));
        }
        for name in names {
            let role = name.rsplit('/').next().unwrap_or(name);
            listed.insert(format!("lactose-{role}"));
        }
    }
    let number = lines[heading].trim_start_matches("## ").split('.').next().unwrap_or("");
    let policy_list: BTreeSet<String> = policy.unsafe_crates.iter().cloned().collect();
    for name in listed.difference(&policy_list) {
        findings.push(Finding::new(
            POLICY_FILE,
            format!(
                "[unsafe_crates] lacks `{name}`, which the table of architecture section \
                 {number} lists; the two change in the same commit"
            ),
        ));
    }
    for name in policy_list.difference(&listed) {
        findings.push(Finding::new(
            POLICY_FILE,
            format!(
                "[unsafe_crates] lists `{name}`, which the table of architecture section \
                 {number} does not; the two change in the same commit"
            ),
        ));
    }
}

/// Whether an attribute body is `forbid(unsafe_code)`, alone or among other lints. A
/// `cfg_attr` does not count: it forbids only sometimes.
fn forbids_unsafe(body: &str) -> bool {
    body.strip_prefix("forbid(")
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(|lints| lints.split(',').any(|lint| lint == "unsafe_code"))
}

/// Whether one of the three lists pins `path`.
fn is_pinned(path: &str) -> bool {
    REQUIRED_LINT_PATHS.contains(&path)
        || THREAD_ROUTES.contains(&path)
        || NOT_THREAD_ROUTES.contains(&path)
}

/// The lint table a pinned path belongs in.
fn pinned_table(path: &str) -> &'static str {
    if PINNED_TYPES.contains(&path) {
        "disallowed-types"
    } else if PINNED_MACROS.contains(&path) {
        "disallowed-macros"
    } else {
        "disallowed-methods"
    }
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
    // Each entry with its table: clippy reads a path only in the table it belongs to, so an
    // entry moved to the other one bans nothing, as if deleted.
    let banned: BTreeSet<(&str, &str)> = LINT_TABLES
        .iter()
        .filter_map(|table| Some((*table, config.get(*table)?.as_array()?)))
        .flat_map(|(table, entries)| {
            entries
                .iter()
                .filter_map(move |e| Some((table, e.as_str().or_else(|| e.get("path")?.as_str())?)))
        })
        .collect();
    // The reference's entries are exactly the three lists, each in its table: none can be
    // dropped from it and every copy at once or moved to another table, and none added there
    // without being pinned here.
    for (table, path) in &banned {
        let wanted = pinned_table(path);
        if is_pinned(path) && *table != wanted {
            findings.push(Finding::new(
                REFERENCE_CLIPPY,
                format!(
                    "lists `{path}` under `{table}`, where clippy does not apply it; it belongs \
                     under `{wanted}`"
                ),
            ));
        }
    }
    let missing = |path: &&&str| !banned.contains(&(pinned_table(path), **path));
    for required in REQUIRED_LINT_PATHS.iter().chain(&NOT_THREAD_ROUTES).filter(missing) {
        findings.push(Finding::new(
            REFERENCE_CLIPPY,
            format!(
                "no longer disallows `{required}` under `{}`, which architecture section 8.2 \
                 bans inside the boundary",
                pinned_table(required)
            ),
        ));
    }
    for route in THREAD_ROUTES.iter().filter(missing) {
        findings.push(Finding::new(
            REFERENCE_CLIPPY,
            format!(
                "no longer disallows `{route}` under `{}`, a route to a thread outside an \
                 executor, which architecture sections 8.2 and 16 ban inside the boundary",
                pinned_table(route)
            ),
        ));
    }
    let unpinned: BTreeSet<&str> =
        banned.iter().map(|(_, path)| *path).filter(|path| !is_pinned(path)).collect();
    for path in unpinned {
        let message = if THREAD_PREFIXES.iter().any(|prefix| path.starts_with(prefix)) {
            format!(
                "disallows `{path}`, which tools/xtask/src/boundary.rs lists neither in \
                 `THREAD_ROUTES` nor in `NOT_THREAD_ROUTES`; list it in the one it belongs to"
            )
        } else {
            format!(
                "disallows `{path}`, which tools/xtask/src/boundary.rs does not pin; add it to \
                 `REQUIRED_LINT_PATHS` there, in the reference's order, so that dropping it is \
                 caught"
            )
        };
        findings.push(Finding::new(REFERENCE_CLIPPY, message));
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

/// Every source file a package compiles, as far as the text shows, with its attributes: its
/// own sources, then each repository file a `#[path]` in one of them names, at any depth,
/// each once. A `#[path]` that names no repository file is left for the caller to report.
fn compiled_sources(package: &Package, files: &Files) -> Vec<(String, Vec<Attribute>)> {
    let mut queue: VecDeque<String> =
        package_sources(package, files).into_iter().map(str::to_owned).collect();
    let mut seen: BTreeSet<String> = queue.iter().cloned().collect();
    let mut sources = Vec::new();
    while let Some(path) = queue.pop_front() {
        let Some(source) = files.text(&path) else {
            continue;
        };
        let found = attributes(&source);
        for named in found.iter().filter_map(|attribute| attribute.path.as_ref()) {
            if let Some(file) = module_file(&path, named).filter(|file| files.contains(file))
                && seen.insert(file.clone())
            {
                queue.push_back(file);
            }
        }
        sources.push((path, found));
    }
    sources
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
    for (path, attributes) in compiled_sources(package, files) {
        for attribute in attributes {
            if let Some(named) = &attribute.path
                && !module_file(&path, named).is_some_and(|file| files.contains(&file))
            {
                findings.push(Finding::new(
                    &path,
                    format!(
                        "line {}: `#[path]` names `{named}`, which is not a source file in the \
                         repository, so the lint attributes of {kind} `{}` in it cannot be \
                         checked",
                        attribute.line, package.name
                    ),
                ));
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
                         `disallowed_types` or `disallowed_macros`) on the one item that needs \
                         it",
                        attribute.line, package.name
                    ),
                ));
            }
        }
    }
}

/// Architecture section 8.3: `[harness]` lists exactly the packages that compile the harness
/// protocol, by `#[path]` from their tests. A package that records hashes but is not listed is
/// never launched by `sour run`, so its simulation is never compared; a listed package that
/// does not compile the protocol records nothing. The protocol's own package is neither.
fn harness_packages(policy: &Policy, ws: &Workspace, files: &Files, findings: &mut Vec<Finding>) {
    let listed = &policy.harness_packages;
    for name in listed.iter().filter(|name| ws.package(name).is_none()) {
        findings.push(Finding::new(
            POLICY_FILE,
            format!("[harness] `packages` lists `{name}`, which is not a workspace package"),
        ));
    }
    for package in &ws.packages {
        if HARNESS_PROTOCOL.starts_with(&format!("{}/", package.dir)) {
            continue;
        }
        let records =
            compiled_sources(package, files).iter().any(|(path, _)| path == HARNESS_PROTOCOL);
        let message = match (records, listed.contains(&package.name)) {
            (true, false) => format!(
                "`{}` compiles the harness protocol ({HARNESS_PROTOCOL}), so its tests record \
                 hashes, but [harness] `packages` in {POLICY_FILE} does not list it and `cargo \
                 xtask sour run` never launches them; add \"{}\" to that list",
                package.name, package.name
            ),
            (false, true) => format!(
                "`{}` is listed in [harness] `packages` of {POLICY_FILE}, but no source of it \
                 includes the harness protocol ({HARNESS_PROTOCOL}) by `#[path]`, so it records \
                 nothing; remove it from that list, or record through the protocol",
                package.name
            ),
            _ => continue,
        };
        findings.push(Finding::new(package.manifest(), message));
    }
}

/// D99: a kind the simulation reads is declared, by `impl SimulationAsset`, only in a
/// boundary crate, and decoded only there: a `Loader` outside the boundary whose `type Asset`
/// names a kind a boundary crate declares or a type parameter of the implementation, or is
/// not written in its body, is reported, so the decoder stays under section 8.2.
///
/// Shipped code is read, from the root of each library, proc-macro and binary target,
/// wherever it lives, through every out-of-line `mod` (by `#[path]`, by the file layout, and
/// inside inline modules, which add their name or their own `#[path]` to the directory) and
/// every `include!` of a literal path, skipping what a `cfg` compiles only for tests or a
/// `[dev_only_features]` feature. A file is read again when a shipped route reaches it after
/// a test-only one, and a file under `src/` that no route reaches is read as shipped, so a
/// module the walk cannot place errs towards a finding.
///
/// Names are compared as written, by their last path segment, of the self type for a
/// qualified path `<T as Trait>::Name`: a kind brought in under another name (`use ... as`, a
/// type alias, a projection such as `<Self as Decodes>::Kind`) or a loader a macro writes is
/// not seen, and a type outside the boundary that shares a kind's name is reported.
fn simulation_assets(tiers: &TierFile, ws: &Workspace, files: &Files, findings: &mut Vec<Finding>) {
    let shipped = |package: &Package| shipped(tiers, package, files).impls;
    let (inside, outside): (Vec<&Package>, Vec<&Package>) =
        ws.packages.iter().partition(|package| tiers.in_boundary(&package.name));
    let kinds: BTreeSet<String> = inside
        .into_iter()
        .flat_map(shipped)
        .filter(|(_, found)| found.trait_name == SIMULATION_ASSET && !found.self_type.is_empty())
        .map(|(_, found)| found.self_type)
        .collect();
    for package in outside {
        for (file, found) in shipped(package) {
            let line = found.line;
            let message = if found.trait_name == SIMULATION_ASSET {
                format!(
                    "line {line}: `impl {SIMULATION_ASSET}` in `{}`, which is not a boundary \
                     crate; a kind the simulation reads is declared in a boundary crate, where \
                     architecture section 8.2 holds its loader (D99), or only in tests",
                    package.name
                )
            } else if found.trait_name == LOADER {
                let Some(decoded) = loads_for_the_simulation(&found, &kinds) else {
                    continue;
                };
                format!(
                    "line {line}: `impl {LOADER}` in `{}`, which is not a boundary crate, \
                     decodes {decoded}; the loader of a kind the simulation reads sits in a \
                     boundary crate, where architecture section 8.2 holds it (D99), or only in \
                     tests",
                    package.name
                )
            } else {
                continue;
            };
            findings.push(Finding::new(file, message));
        }
    }
}

/// Project review 2, X1: what a world registers and runs is written under the boundary lint
/// file. A crate that carries none, neither a boundary crate nor one of `[lint_file_carriers]`,
/// ships no `#[system]`, no `#[component]` or `#[resource]` that is not `derived`, and no
/// implementation of a trait through which the simulation applies what enters a tick
/// ([`APPLYING_TRAITS`]), and no hand-written [`DESCRIBING_TRAIT`]; each is reported where it
/// stands, unless only tests compile it.
///
/// The shipped code is read as [`simulation_assets`] reads it, and attributes and traits by
/// their last path segment as written: one brought in under another name, produced by a macro
/// the crate's source does not spell out (another crate's macro or derive), or inside a
/// `cfg_attr`, is not seen.
fn registered_outside_the_lint_file(
    tiers: &TierFile,
    policy: &Policy,
    ws: &Workspace,
    files: &Files,
    findings: &mut Vec<Finding>,
) {
    let unlinted = ws.packages.iter().filter(|package| {
        !tiers.in_boundary(&package.name) && !policy.lint_file_carriers.contains(&package.name)
    });
    for package in unlinted {
        let Shipped { impls, declarations } = shipped(tiers, package, files);
        let declared =
            declarations.into_iter().filter(|(_, found)| !found.derived).map(|(file, found)| {
                let what = match found.attribute {
                    "system" => "`#[system]`".to_owned(),
                    attribute => format!("`#[{attribute}]` that is not `derived`"),
                };
                (file, found.line, what)
            });
        let (described, impls): (Vec<_>, Vec<_>) =
            impls.into_iter().partition(|(_, found)| found.trait_name == DESCRIBING_TRAIT);
        let applied = impls
            .into_iter()
            .filter(|(_, found)| APPLYING_TRAITS.contains(&found.trait_name.as_str()))
            .map(|(file, found)| (file, found.line, format!("`impl {}`", found.trait_name)));
        for (file, line, what) in declared.chain(applied) {
            findings.push(Finding::new(
                file,
                format!(
                    "line {line}: {what} in `{}`, which carries no boundary lint file; what a \
                     world registers and runs is written in a boundary crate or a crate of \
                     [lint_file_carriers] in {POLICY_FILE}, under the lints of architecture \
                     section 8.2, or only in tests",
                    package.name
                ),
            ));
        }
        for (file, found) in described {
            findings.push(Finding::new(
                file,
                format!(
                    "line {}: a hand-written `impl {DESCRIBING_TRAIT}` in `{}`, which carries no \
                     boundary lint file; it declares a component or a resource that no attribute \
                     shows, and what a world stores is described in a boundary crate or a crate \
                     of [lint_file_carriers] in {POLICY_FILE}, under the lints of architecture \
                     section 8.2, or only in tests",
                    found.line, package.name
                ),
            ));
        }
    }
}

/// What a loader outside the boundary decodes, said for a finding, if it may be a kind the
/// simulation reads: its `type Asset` names one of `kinds` by its last path segment, names a
/// type parameter of the implementation, or cannot be read.
fn loads_for_the_simulation(found: &Impl, kinds: &BTreeSet<String>) -> Option<String> {
    let Some((_, asset)) = found.types.iter().find(|(name, _)| name == "Asset") else {
        return Some(format!(
            "a kind its text does not show (no `type Asset = ...;` in its body), which may be \
             one a boundary crate declares a `{SIMULATION_ASSET}`"
        ));
    };
    let words = || asset.split(|c: char| !c.is_ascii_alphanumeric() && c != '_');
    if let Some(parameter) = words().find(|word| found.generics.iter().any(|g| g == word)) {
        return Some(format!(
            "`{asset}`, which names its type parameter `{parameter}`, so it may decode a kind \
             a boundary crate declares a `{SIMULATION_ASSET}`"
        ));
    }
    // The type the path names: of `<T as Trait>::Name`, its self type `T`.
    let outer = match asset.strip_prefix('<') {
        Some(qualified) => qualified.split(" as ").next().unwrap_or_default(),
        None => asset.as_str(),
    };
    let outer = outer.split('<').next().unwrap_or_default();
    let last = outer.rsplit("::").next().unwrap_or_default();
    kinds
        .contains(last)
        .then(|| format!("`{asset}`, which a boundary crate declares a `{SIMULATION_ASSET}`"))
}

/// What a package ships, by [`shipped`].
#[derive(Default)]
struct Shipped {
    /// Each trait implementation, with its file.
    impls: Vec<(String, Impl)>,
    /// Each declaring attribute, with its file.
    declarations: Vec<(String, Declaration)>,
}

/// Every trait implementation and declaring attribute `package` ships, with its file, each
/// once: the walk of [`simulation_assets`] over the package's module tree, without the
/// features `[dev_only_features]` lists for it.
fn shipped(tiers: &TierFile, package: &Package, files: &Files) -> Shipped {
    let listed = tiers.dev_only_features.get(&package.name).map_or(&[][..], Vec::as_slice);
    let features: Vec<String> =
        turning_on(package, listed).into_iter().map(str::to_owned).collect();
    let features = &features[..];
    // (file, the directory its out-of-line modules sit in, whether only tests compile it)
    let mut queue: VecDeque<(String, String, bool)> = package
        .shipped_roots
        .iter()
        .map(|root| (root.clone(), parent(root).to_owned(), false))
        .collect();
    // Each file read, under the directory it was read with, and whether it was read as
    // shipped: a file reached again as shipped after a test-only route is read again.
    let mut read: BTreeMap<(String, String), bool> = BTreeMap::new();
    let mut impls: BTreeMap<(String, usize), Impl> = BTreeMap::new();
    let mut declarations: BTreeMap<(String, usize), Declaration> = BTreeMap::new();
    let mut sweep = true;
    loop {
        while let Some((file, dir, test_only)) = queue.pop_front() {
            let key = (file, dir);
            if read.get(&key).is_some_and(|&shipped| shipped || test_only) {
                continue;
            }
            read.insert(key.clone(), !test_only);
            let (file, dir) = key;
            let Some(source) = files.text(&file) else {
                continue;
            };
            let items = items(&source, features);
            for (child, child_dir, gated) in children(&file, &dir, &items, files) {
                queue.push_back((child, child_dir, test_only || gated));
            }
            if test_only {
                continue;
            }
            for implementation in items.impls.into_iter().filter(|i| !i.test_only) {
                impls.insert((file.clone(), implementation.line), implementation);
            }
            for declaration in items.declarations.into_iter().filter(|d| !d.test_only) {
                declarations.insert((file.clone(), declaration.line), declaration);
            }
        }
        if !std::mem::take(&mut sweep) {
            break;
        }
        let src = format!("{}/src/", package.dir);
        for path in package_sources(package, files) {
            if path.starts_with(&src) && !read.keys().any(|(file, _)| file == path) {
                let dir = match path.strip_suffix("/mod.rs") {
                    Some(dir) => dir.to_owned(),
                    None => path.trim_end_matches(".rs").to_owned(),
                };
                queue.push_back((path.to_owned(), dir, false));
            }
        }
    }
    Shipped {
        impls: impls.into_iter().map(|((file, _), found)| (file, found)).collect(),
        declarations: declarations.into_iter().map(|((file, _), found)| (file, found)).collect(),
    }
}

/// A boundary crate's `direct` lists name only crates its manifest names. A crate it only
/// reaches through others is vetted on `transitive`, and an entry its manifest does not name
/// is a permission waiting to be used by the next manifest edit.
fn unnamed_allowlist_entries(tiers: &TierFile, package: &Package, findings: &mut Vec<Finding>) {
    let Some(direct) = tiers.boundary_third_party.direct.get(&package.name) else {
        return;
    };
    for (list, names) in [("normal", &direct.normal), ("dev", &direct.dev)] {
        for name in names.iter().filter(|name| !package.deps.iter().any(|d| d.name == **name)) {
            findings.push(Finding::new(
                TIER_FILE,
                format!(
                    "[boundary_third_party.direct] \"{}\" has `{name}` on its `{list}` list, but \
                     the crate's manifest does not name it; remove it there, and put a crate \
                     the boundary crate reaches only through others on `transitive`",
                    package.name
                ),
            ));
        }
    }
}

/// The files that the out-of-line modules and the literal `include!`s of `file` name, read
/// with its own out-of-line modules in the directory `dir`, as rustc resolves them: each with
/// the directory its own modules sit in and whether a `cfg` on its declaration holds only for
/// tests (see [`items`]). A module whose file is in neither place it may be is left out.
pub(crate) fn children(
    file: &str,
    dir: &str,
    items: &Items,
    files: &Files,
) -> Vec<(String, String, bool)> {
    let mut found = Vec::new();
    for module in &items.modules {
        // The directory of the innermost inline module around the declaration: each is a
        // directory of its name under the one around it, or the one its `#[path]` names
        // relative to it. At the top of a file that is the file's own directory, without the
        // one a `name.rs` adds for its modules.
        let mut base = Some(dir.to_owned());
        for (depth, inline) in module.inline.iter().enumerate() {
            base = match &inline.path {
                Some(named) if depth == 0 => joined(parent(file), named),
                Some(named) => base.and_then(|base| joined(&base, named)),
                None => base.map(|base| format!("{base}/{}", inline.name)),
            };
        }
        let Some(base) = base else {
            continue;
        };
        let child = match &module.path {
            // Outside an inline module a `#[path]` is relative to the file's own directory,
            // inside one to the module's.
            Some(named) if module.inline.is_empty() => module_file(file, named),
            Some(named) => joined(&base, named),
            None => {
                [format!("{base}/{}.rs", module.name), format!("{base}/{}/mod.rs", module.name)]
                    .into_iter()
                    .find(|candidate| files.contains(candidate))
            }
        };
        // A `#[path]` file holds its modules beside it, as a `mod.rs` does; a `name.rs` in the
        // directory `name`, which is where `name/mod.rs` holds them.
        let child_dir = match &module.path {
            Some(_) => child.as_deref().map(|child| parent(child).to_owned()),
            None => Some(format!("{base}/{}", module.name)),
        };
        if let (Some(child), Some(child_dir)) = (child, child_dir) {
            found.push((child, child_dir, module.test_only));
        }
    }
    // An included file is part of the module it sits in, and holds its own modules beside it.
    for include in &items.includes {
        if let Some(child) = module_file(file, &include.path) {
            let child_dir = parent(&child).to_owned();
            found.push((child, child_dir, include.test_only));
        }
    }
    found
}

/// The repository path of the file that `#[path = "named"]` in the file `from` names: relative
/// to the directory of `from`, as for every `mod` outside an inline module block. `None` for
/// a path that is absolute or leaves the repository.
pub(crate) fn module_file(from: &str, named: &str) -> Option<String> {
    joined(parent(from), named)
}

/// The directory of a repository path, `""` at the root.
pub(crate) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The repository path that `named` names relative to the directory `dir`; `None` for a path
/// that is absolute or leaves the repository.
fn joined(dir: &str, named: &str) -> Option<String> {
    if named.starts_with(['/', '\\']) || named.get(1..2) == Some(":") {
        return None;
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|part| !part.is_empty()).collect();
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
/// (cargo configuration, workflows, scripts): nothing sets rust or rustdoc flags through the
/// environment, selects a CPU or a target feature, points clippy at another configuration, or
/// allows a boundary lint from the command line.
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
            // In a cargo configuration file the `rustflags` and `rustdocflags` keys are judged
            // by structure, in `cargo_configs`; anywhere else either word in any spelling is a
            // way to set them (`--config build.rustflags=...`, the environment variables).
            let folded_upper = line.to_ascii_uppercase().replace('-', "_");
            for (variable, what) in [("RUSTFLAGS", "rust"), ("RUSTDOCFLAGS", "rustdoc")] {
                let names = if cargo_config {
                    line.contains(variable)
                } else {
                    folded_upper.contains(variable)
                };
                if names {
                    report(format!(
                        "names {variable}; {what} flags from the environment or the command \
                         line replace the {PIN_CPU} pin in {CARGO_CONFIG} instead of adding to it"
                    ));
                }
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

/// The cargo configuration keys that set flags, for rustc and for rustdoc. Cargo does not pass
/// `rustflags` to rustdoc, so a doctest is pinned only by `rustdocflags`.
const FLAG_KEYS: [&str; 2] = ["rustflags", "rustdocflags"];

/// Every key of [`FLAG_KEYS`] in a TOML value, as a dotted path with its value.
fn flag_keys<'a>(
    value: &'a toml::Value,
    path: &mut Vec<&'a str>,
    found: &mut Vec<(String, &'a toml::Value)>,
) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        path.push(key);
        if FLAG_KEYS.contains(&key.as_str()) {
            found.push((path.join("."), child));
        } else {
            flag_keys(child, path, found);
        }
        path.pop();
    }
}

/// Cargo configuration files set flags in exactly one place: the x86-64-v3 pin in
/// `.cargo/config.toml`, as `rustflags` and as `rustdocflags` with the same flags. Any other
/// `rustflags` or `rustdocflags` key, in any table of any cargo configuration file, is a
/// finding, because cargo joins or replaces flags by rules nobody should have to re-derive to
/// know what the boundary was compiled with; so is either half of the pin missing.
fn cargo_configs(files: &Files, findings: &mut Vec<Finding>) {
    let mut pinned = [false; FLAG_KEYS.len()];
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
        flag_keys(&config, &mut Vec::new(), &mut found);
        for (key, value) in found {
            let flags: Option<Vec<&str>> = value
                .as_array()
                .map(|items| items.iter().filter_map(toml::Value::as_str).collect());
            let pin = FLAG_KEYS.iter().position(|name| key == format!("target.{PIN_KEY}.{name}"));
            match pin {
                Some(index) if path == CARGO_CONFIG && flags.as_deref() == Some(&PIN_FLAGS[..]) => {
                    pinned[index] = true;
                }
                _ => findings.push(Finding::new(
                    path,
                    format!(
                        "sets `{key}`; the only flags permitted in a cargo configuration file \
                         are the pin, [target.'{PIN_KEY}'] rustflags = {PIN_FLAGS:?} and \
                         rustdocflags the same, in {CARGO_CONFIG}"
                    ),
                )),
            }
        }
    }
    for (name, _) in FLAG_KEYS.iter().zip(pinned).filter(|(_, pinned)| !pinned) {
        findings.push(Finding::new(
            CARGO_CONFIG,
            format!(
                "the x86-64 pin is missing: [target.'{PIN_KEY}'] must set {name} = {PIN_FLAGS:?}"
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

    const PIN: &str = "[target.'cfg(target_arch = \"x86_64\")']\nrustflags = [\"-C\", \"target-cpu=x86-64-v3\"]\nrustdocflags = [\"-C\", \"target-cpu=x86-64-v3\"]\n";
    const FORBID: &str = "//! Docs.\n#![forbid(unsafe_code)]\n";
    const MANIFEST: &str = "[package]\nname = \"x\"\n[lints]\nworkspace = true\n";

    /// The line a fixture lint file holds for `path`: a type as a plain string, anything else
    /// as a table (the check reads every table, and both forms, as one).
    fn entry(path: &str) -> String {
        if pinned_table(path) == "disallowed-types" {
            format!("  \"{path}\",\n")
        } else {
            format!("  {{ path = \"{path}\", reason = \"r\" }},\n")
        }
    }

    /// A lint file of the pinned entries that `keep` keeps, each in its table.
    fn lint_file_of(keep: impl Fn(&str) -> bool) -> String {
        let pinned = || REQUIRED_LINT_PATHS.iter().chain(&THREAD_ROUTES).chain(&NOT_THREAD_ROUTES);
        let mut text = String::new();
        for table in LINT_TABLES {
            text.push_str(&format!("{table} = [\n"));
            for path in pinned().filter(|path| pinned_table(path) == table && keep(path)) {
                text.push_str(&entry(path));
            }
            text.push_str("]\n");
        }
        text
    }

    fn lint_file() -> String {
        lint_file_of(|_| true)
    }

    /// What a crate of `[boundary_internal] any` carries.
    fn outside_world_file() -> String {
        lint_file_of(|path| OUTSIDE_WORLD.iter().any(|prefix| path.starts_with(prefix)))
    }

    /// The real policy file with no harness packages, which a test adds where it wants them.
    fn policy() -> Policy {
        let policy = Policy::parse(include_str!("../ci-policy.toml")).unwrap();
        Policy { harness_packages: Vec::new(), ..policy }
    }

    /// An architecture document whose unsafe policy table is the policy file's list.
    fn architecture() -> String {
        let mut text = String::from(
            "# Architecture\n\n## 20. Agent workflow\n\n| Role | `x` |\n|---|---|\n| a | `b` |\n\n\
             ## 21. Unsafe policy\n\n| Crate | Why |\n|---|---|\n",
        );
        for name in policy().unsafe_crates {
            let role = name.strip_prefix("lactose-").unwrap();
            let cell = if role == "cook" { "tools/cook".to_owned() } else { role.to_owned() };
            text.push_str(&format!("| `{cell}` | FFI, `unsafe` |\n"));
        }
        text.push_str("\nEach carries `SAFETY.md`.\n\n## 22. Milestones\n\n| # | Name |\n|---|---|\n| 0 | `skeleton` |\n");
        text
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
        files.insert(ARCHITECTURE, architecture());
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
        run_with(&policy(), ws, files)
    }

    fn run_with(policy: &Policy, ws: &Workspace, files: &Files) -> Vec<String> {
        let tiers = TierFile::parse(TIERS).unwrap();
        check(&tiers, policy, ws, files).iter().map(ToString::to_string).collect()
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
                "crates/reflect-macros/src/lib.rs: line 3: `allow(clippy::disallowed_methods)` never goes stale, on an item of [lint_file_carriers] crate `lactose-reflect-macros`; the only sanctioned form is `#[expect(clippy::disallowed_methods, reason = \"...\")]` (or `disallowed_types` or `disallowed_macros`) on the one item that needs it"
            ]
        );
    }

    #[test]
    fn a_weakened_reference_lint_file_is_caught_even_when_every_copy_matches() {
        // Review item 19.
        let (ws, mut files) = conforming();
        let weakened =
            lint_file().replace("  \"std::time::Instant\",\n", "").replace(&entry("f64::sin"), "");
        files.insert(REFERENCE_CLIPPY, weakened.clone());
        files.insert("crates/math/clippy.toml", weakened);
        assert_eq!(
            run(&ws, &files),
            [
                "crates/core/clippy.toml: no longer disallows `f64::sin` under `disallowed-methods`, which architecture section 8.2 bans inside the boundary",
                "crates/core/clippy.toml: no longer disallows `std::time::Instant` under `disallowed-types`, which architecture section 8.2 bans inside the boundary",
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
        for route in THREAD_ROUTES {
            let unbanned = lint_file().replace(&entry(route), "");
            assert_ne!(unbanned, lint_file(), "{route}");
            files.insert(REFERENCE_CLIPPY, unbanned.clone());
            files.insert("crates/math/clippy.toml", unbanned);
            assert_eq!(
                run(&ws, &files),
                [format!(
                    "crates/core/clippy.toml: no longer disallows `{route}` under `{}`, a route to a thread outside an executor, which architecture sections 8.2 and 16 ban inside the boundary",
                    pinned_table(route)
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
    fn any_ban_deleted_from_the_reference_and_every_copy_is_reported() {
        // Review X3: every entry is pinned, not a sample of each family.
        let (ws, mut files) = conforming();
        for path in REQUIRED_LINT_PATHS.iter().chain(&NOT_THREAD_ROUTES) {
            let unbanned = lint_file().replace(&entry(path), "");
            assert_ne!(unbanned, lint_file(), "{path}");
            files.insert(REFERENCE_CLIPPY, unbanned.clone());
            files.insert("crates/math/clippy.toml", unbanned);
            assert_eq!(
                run(&ws, &files),
                [format!(
                    "crates/core/clippy.toml: no longer disallows `{path}` under `{}`, which architecture section 8.2 bans inside the boundary",
                    pinned_table(path)
                )]
            );
        }
    }

    #[test]
    fn any_ban_moved_to_another_table_in_the_reference_and_every_copy_is_reported() {
        // Review round 1, F2: clippy reads a path only in its own table, so a method under
        // `disallowed-types`, or a type or a macro under `disallowed-methods`, bans nothing.
        let (ws, mut files) = conforming();
        let pinned = REQUIRED_LINT_PATHS.iter().chain(&THREAD_ROUTES).chain(&NOT_THREAD_ROUTES);
        let moves = pinned.flat_map(|path| {
            let wanted = pinned_table(path);
            LINT_TABLES.into_iter().filter(move |t| *t != wanted).map(move |t| (path, wanted, t))
        });
        for (path, wanted, other) in moves {
            let moved = lint_file()
                .replace(&entry(path), "")
                .replace(&format!("{other} = [\n"), &format!("{other} = [\n{}", entry(path)));
            assert_ne!(moved, lint_file(), "{path}");
            files.insert(REFERENCE_CLIPPY, moved.clone());
            files.insert("crates/math/clippy.toml", moved);
            let found = run(&ws, &files);
            let misplaced = format!(
                "crates/core/clippy.toml: lists `{path}` under `{other}`, where clippy does not apply it; it belongs under `{wanted}`"
            );
            assert!(found.contains(&misplaced), "{path}: {found:?}");
            assert!(
                found.iter().any(|f| f.starts_with(&format!(
                    "crates/core/clippy.toml: no longer disallows `{path}` under `{wanted}`"
                ))),
                "{path}: {found:?}"
            );
            assert_eq!(found.len(), 2, "{path}: {found:?}");
        }
    }

    #[test]
    fn run_time_feature_detection_stays_banned_and_its_lint_stays_on() {
        // Project review 2, X3: section 8.2 bans run-time CPU feature detection that selects
        // numerically different paths, and the macros are pinned in their own table.
        let (ws, mut files) = conforming();
        let table =
            "disallowed-macros = [\n".to_owned() + &PINNED_MACROS.map(entry).concat() + "]\n";
        assert!(lint_file().contains(&table));
        let without = lint_file().replace(&table, "");
        files.insert(REFERENCE_CLIPPY, without.clone());
        files.insert("crates/math/clippy.toml", without);
        assert_eq!(
            run(&ws, &files),
            PINNED_MACROS.map(|path| format!(
                "crates/core/clippy.toml: no longer disallows `{path}` under `disallowed-macros`, which architecture section 8.2 bans inside the boundary"
            ))
        );
        let (ws, mut files) = conforming();
        files.insert(
            "crates/math/src/lib.rs",
            format!("{FORBID}#![allow(clippy::disallowed_macros)]\nfn f() {{}}\n"),
        );
        let found = run(&ws, &files);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with(
                "crates/math/src/lib.rs: line 3: `allow(clippy::disallowed_macros)` applies to a whole crate or module of boundary crate `lactose-math`"
            ),
            "{found:?}"
        );
    }

    #[test]
    fn a_ban_added_to_the_reference_without_a_pin_is_reported() {
        let (ws, mut files) = conforming();
        let added = "f64::exp10";
        let unpinned = lint_file()
            .replace("disallowed-types = [\n", &format!("disallowed-types = [\n  \"{added}\",\n"));
        files.insert(REFERENCE_CLIPPY, unpinned.clone());
        files.insert("crates/math/clippy.toml", unpinned);
        assert_eq!(
            run(&ws, &files),
            [format!(
                "crates/core/clippy.toml: disallows `{added}`, which tools/xtask/src/boundary.rs does not pin; add it to `REQUIRED_LINT_PATHS` there, in the reference's order, so that dropping it is caught"
            )]
        );
    }

    #[test]
    fn the_pinned_lists_are_disjoint_and_the_required_one_holds_no_thread_path() {
        let mut all = BTreeSet::new();
        for path in REQUIRED_LINT_PATHS.iter().chain(&THREAD_ROUTES).chain(&NOT_THREAD_ROUTES) {
            assert!(all.insert(*path), "`{path}` is pinned twice");
        }
        for path in PINNED_TYPES {
            assert!(is_pinned(path), "`{path}` is a pinned type that no list pins");
        }
        // An entry under a thread prefix is classified as a route or not, never just required.
        for path in REQUIRED_LINT_PATHS {
            assert!(!THREAD_PREFIXES.iter().any(|prefix| path.starts_with(prefix)), "{path}");
        }
        // Every family of item 1 of review X2 is pinned.
        for path in [
            "num_traits::Float",
            "num_traits::real::Real",
            "simba::scalar::RealField::max",
            "simba::scalar::RealField::min",
            "simba::scalar::RealField::copysign",
            "simba::simd::SimdRealField::simd_copysign",
            "simba::simd::SimdSigned::is_simd_positive",
            "f64::asinh",
        ] {
            assert!(REQUIRED_LINT_PATHS.contains(&path), "{path}");
        }
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
    fn an_allowlist_entry_the_manifest_does_not_name_is_reported() {
        // Review X2: the backend's list had grown the solver's math, which the backend names
        // only through Rapier's re-exports.
        let (_, files) = conforming();
        let ws = workspace_with(
            vec![
                package("lactose-core", &[("bytemuck", Normal)]),
                package("lactose-math", &[("lactose-core", Normal)]),
                package("lactose-task", &[("lactose-core", Normal), ("crossbeam-deque", Normal)]),
            ],
            &[("bytemuck", &["bytemuck_derive"]), ("crossbeam-deque", &["crossbeam-epoch"])],
        );
        assert_eq!(
            run(&ws, &files),
            [
                "crate-tiers.toml: [boundary_third_party.direct] \"lactose-core\" has `xxhash-rust` on its `dev` list, but the crate's manifest does not name it; remove it there, and put a crate the boundary crate reaches only through others on `transitive`",
                "crate-tiers.toml: [boundary_third_party.direct] \"lactose-math\" has `libm` on its `normal` list, but the crate's manifest does not name it; remove it there, and put a crate the boundary crate reaches only through others on `transitive`",
            ]
        );
    }

    #[test]
    fn the_physics_backend_may_not_name_the_solvers_math() {
        // Review X2, against the real tier file: the solver's math is vetted on `transitive`,
        // so naming it (num-traits' `Float` above all) is refused.
        let tiers = TierFile::parse(include_str!("../../../crate-tiers.toml")).unwrap();
        let backend = package(
            "lactose-physics-rapier",
            &[("rapier3d-f64", Normal), ("num-traits", Normal), ("simba", Normal), ("glam", Dev)],
        );
        let ws = workspace(vec![backend.clone()]);
        let mut findings = Vec::new();
        dependency_allowlist(&tiers, &ws, &backend, &mut findings);
        unnamed_allowlist_entries(&tiers, &backend, &mut findings);
        let found: Vec<String> = findings.iter().map(ToString::to_string).collect();
        let refused = |kind: &str, name: &str| {
            format!(
                "crates/physics-rapier/Cargo.toml: boundary crate `lactose-physics-rapier` {kind} on third-party crate `{name}`, which [boundary_third_party.direct] \"lactose-physics-rapier\" in crate-tiers.toml does not allow"
            )
        };
        let unnamed = |name: &str| {
            format!(
                "crate-tiers.toml: [boundary_third_party.direct] \"lactose-physics-rapier\" has `{name}` on its `normal` list, but the crate's manifest does not name it; remove it there, and put a crate the boundary crate reaches only through others on `transitive`"
            )
        };
        assert_eq!(
            found,
            [
                refused("depends", "num-traits"),
                refused("depends", "simba"),
                refused("dev-depends", "glam"),
                unnamed("bytemuck"),
                unnamed("postcard"),
                unnamed("rayon"),
                unnamed("serde"),
                unnamed("tracing"),
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
            package("lactose-core", &[("bytemuck", Normal), ("xxhash-rust", Dev)]),
            package(
                "lactose-math",
                &[
                    ("libm", Normal),
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
            package("lactose-core", &[("bytemuck", Normal), ("xxhash-rust", Dev)]),
            package(
                "lactose-math",
                &[
                    ("libm", Normal),
                    ("lactose-reflect", Normal),
                    ("lactose-ecs", Normal),
                    ("lactose-io", Dev),
                ],
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
                package("lactose-core", &[("bytemuck", Normal), ("xxhash-rust", Dev)]),
                package("lactose-math", &[("libm", Normal), ("lactose-ecs", kind)]),
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
                          reason = \"...\")]` (or `disallowed_types` or `disallowed_macros`) on \
                          the one item that needs it";
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
                // Both record through the protocol, and the fixture policy lists no package.
                unlisted_harness_package("math"),
                unlisted_harness_package("task"),
            ]
        );
    }

    fn unlisted_harness_package(role: &str) -> String {
        format!(
            "crates/{role}/Cargo.toml: `lactose-{role}` compiles the harness protocol \
             (tools/xtask/src/harness/protocol.rs), so its tests record hashes, but [harness] \
             `packages` in tools/xtask/ci-policy.toml does not list it and `cargo xtask sour \
             run` never launches them; add \"lactose-{role}\" to that list"
        )
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
            .replace("  \"std::fs::File\",\n", "  \"std::fs::File\",\n  \"std::net::Ipv4Addr\",\n");
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
                    "{} has an entry for `std::net::Ipv4Addr` that crates/core/clippy.toml does not have as written; the two must agree on every file, environment, subprocess, network and thread entry",
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
                package(
                    "lactose-core",
                    &[("bytemuck", Normal), ("xxhash-rust", Dev), ("lactose-reflect", Normal)],
                ),
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
                ".cargo/config.toml: the x86-64 pin is missing: [target.'cfg(target_arch = \"x86_64\")'] must set rustflags = [\"-C\", \"target-cpu=x86-64-v3\"]",
                ".cargo/config.toml: the x86-64 pin is missing: [target.'cfg(target_arch = \"x86_64\")'] must set rustdocflags = [\"-C\", \"target-cpu=x86-64-v3\"]",
            ]
        );

        let only = "the only flags permitted in a cargo configuration file are the pin, [target.'cfg(target_arch = \"x86_64\")'] rustflags = [\"-C\", \"target-cpu=x86-64-v3\"] and rustdocflags the same, in .cargo/config.toml";
        files.insert(
            CARGO_CONFIG,
            format!("{PIN}[build]\nrustflags = [\"-C\", \"target_feature=+avx512f\"]\n[target.x86_64-unknown-linux-gnu]\nrustflags = [\"-Ctarget_cpu=native\"]\n[env]\nCLIPPY_CONF_DIR = \"elsewhere\"\n"),
        );
        files.insert("tools/cook/.cargo/config.toml", PIN);
        assert_eq!(
            run(&ws, &files),
            [
                ".cargo/config.toml: line 5: sets `target-feature`; target features come from the pinned target CPU only".to_owned(),
                ".cargo/config.toml: line 7: selects a target CPU (`target-cpu` `native`); the only permitted setting is the x86-64-v3 pin in .cargo/config.toml".to_owned(),
                ".cargo/config.toml: line 9: names CLIPPY_CONF_DIR, which points clippy away from each crate's clippy.toml".to_owned(),
                format!(".cargo/config.toml: sets `build.rustflags`; {only}"),
                format!(".cargo/config.toml: sets `target.x86_64-unknown-linux-gnu.rustflags`; {only}"),
                format!("tools/cook/.cargo/config.toml: sets `target.cfg(target_arch = \"x86_64\").rustdocflags`; {only}"),
                format!("tools/cook/.cargo/config.toml: sets `target.cfg(target_arch = \"x86_64\").rustflags`; {only}"),
            ]
        );

        // The pin with anything added to it is not the pin.
        files.insert(CARGO_CONFIG, PIN.replace("\"]", "\", \"-C\", \"target-feature=+fma\"]"));
        let files = without(&files, &["tools/cook/.cargo/config.toml"]);
        let found = run(&ws, &files);
        assert_eq!(found.len(), 6, "{found:#?}");
        assert!(found[4].contains(
            "the x86-64 pin is missing: [target.'cfg(target_arch = \"x86_64\")'] must set rustflags"
        ));
        assert!(found[5].contains("the x86-64 pin is missing: [target.'cfg(target_arch = \"x86_64\")'] must set rustdocflags"));
    }

    #[test]
    fn doctests_are_pinned_too() {
        // Row 70 of docs/verification.md: cargo does not pass `rustflags` to rustdoc.
        let (ws, mut files) = conforming();
        let rustdoc = "rustdocflags = [\"-C\", \"target-cpu=x86-64-v3\"]\n";
        assert!(PIN.contains(rustdoc));
        files.insert(CARGO_CONFIG, PIN.replace(rustdoc, ""));
        assert_eq!(
            run(&ws, &files),
            [
                ".cargo/config.toml: the x86-64 pin is missing: [target.'cfg(target_arch = \"x86_64\")'] must set rustdocflags = [\"-C\", \"target-cpu=x86-64-v3\"]"
            ]
        );
        // Another value is not the pin, wherever it is set.
        files.insert(
            CARGO_CONFIG,
            PIN.replace(rustdoc, "rustdocflags = [\"-C\", \"target-cpu=native\"]\n"),
        );
        let found = run(&ws, &files);
        assert_eq!(found.len(), 3, "{found:#?}");
        assert!(found[1].starts_with(".cargo/config.toml: sets `target.cfg(target_arch = \"x86_64\").rustdocflags`; the only flags"), "{found:#?}");
        assert!(found[2].contains("must set rustdocflags"), "{found:#?}");
        files.insert(CARGO_CONFIG, format!("{PIN}[build]\nrustdocflags = [\"--cfg\", \"x\"]\n"));
        let found = run(&ws, &files);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(
            found[0].starts_with(".cargo/config.toml: sets `build.rustdocflags`;"),
            "{found:#?}"
        );
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
            "CARGO_ENCODED_RUSTFLAGS=x cargo bench\ncargo rustc -- --codegen target-cpu=haswell\nRUSTDOCFLAGS='-D warnings' cargo test --doc\n",
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
            "scripts/bench.sh: line 3: names RUSTDOCFLAGS; rustdoc flags",
        ];
        assert_eq!(found.len(), expect.len(), "{found:#?}");
        for (found, expect) in found.iter().zip(expect) {
            assert!(found.starts_with(expect), "{found}\n  expected: {expect}");
        }
    }

    #[test]
    fn the_harness_packages_are_the_packages_that_include_the_protocol() {
        // Review X4: a package dropped from the list records nothing, and nothing says so.
        let (ws, mut files) = conforming();
        let include = "#[path = \"../../../tools/xtask/src/harness/protocol.rs\"]\nmod protocol;\n";
        files.insert(HARNESS_PROTOCOL, "pub fn f() {}\n");
        files.insert("crates/math/tests/sour.rs", format!("{FORBID}{include}"));
        files.insert("crates/task/tests/sour.rs", include);
        let listing = |names: &[&str]| Policy {
            harness_packages: names.iter().map(|name| (*name).to_owned()).collect(),
            ..policy()
        };
        assert_eq!(run_with(&listing(&["lactose-math", "lactose-task"]), &ws, &files), NONE);
        assert_eq!(
            run_with(&listing(&["lactose-math"]), &ws, &files),
            [unlisted_harness_package("task")]
        );
        // Listed and recording nothing, or no package at all.
        let files = without(&files, &["crates/task/tests/sour.rs"]);
        assert_eq!(
            run_with(&listing(&["lactose-math", "lactose-task", "lactose-gone"]), &ws, &files),
            [
                "tools/xtask/ci-policy.toml: [harness] `packages` lists `lactose-gone`, which is not a workspace package",
                "crates/task/Cargo.toml: `lactose-task` is listed in [harness] `packages` of tools/xtask/ci-policy.toml, but no source of it includes the harness protocol (tools/xtask/src/harness/protocol.rs) by `#[path]`, so it records nothing; remove it from that list, or record through the protocol",
            ]
        );
    }

    #[test]
    fn the_unsafe_list_is_the_table_of_architecture_section_21() {
        // Review X5: the table and the list drift apart, and a crate's forbid goes with it.
        let (mut ws, mut files) = conforming();
        ws.packages.push(package("lactose-sim", &[("lactose-core", Normal)]));
        files.insert("crates/sim/README.md", "");
        files.insert("crates/sim/Cargo.toml", MANIFEST);
        files.insert("crates/sim/src/lib.rs", FORBID);
        assert_eq!(run(&ws, &files), NONE);
        files.insert(ARCHITECTURE, architecture().replace("| `tools/cook` |", "| `sim` |"));
        files.insert("crates/sim/src/lib.rs", "pub fn f() {}\n");
        let found = run(&ws, &files);
        assert_eq!(
            found[..2],
            [
                "tools/xtask/ci-policy.toml: [unsafe_crates] lacks `lactose-sim`, which the table of architecture section 21 lists; the two change in the same commit",
                "tools/xtask/ci-policy.toml: [unsafe_crates] lists `lactose-cook`, which the table of architecture section 21 does not; the two change in the same commit",
            ]
        );
        // The list still governs the forbid, so the crate without it is reported too.
        assert_eq!(found.len(), 3, "{found:#?}");
        assert!(
            found[2].starts_with("crates/sim/src/lib.rs: `lactose-sim` is not on the unsafe list")
        );

        // A document the check cannot read is a finding, not a pass.
        let cut = |from: &str, to: &str| {
            let text = architecture();
            let (head, rest) = text.split_once(from).unwrap();
            format!("{head}{to}{}", &rest[rest.find("\n## 22").unwrap()..])
        };
        let (ws, mut files) = conforming();
        for (text, expect) in [
            (
                cut("## 21. Unsafe policy", "## 21. Unsafe code\n"),
                "docs/architecture.md: has no `## <n>. Unsafe policy` section",
            ),
            (
                cut("| Crate |", "Prose only.\n"),
                "docs/architecture.md: line 9: the Unsafe policy section has no table of crates",
            ),
            (
                architecture().replace("| `core` |", "| core |"),
                "a row of the Unsafe policy table names no crate in backticks",
            ),
        ] {
            files.insert(ARCHITECTURE, text);
            let found = run(&ws, &files);
            assert!(found.first().is_some_and(|f| f.contains(expect)), "{found:#?}\n{expect}");
        }
        let files = without(&files, &[ARCHITECTURE]);
        assert_eq!(
            run(&ws, &files),
            [
                "docs/architecture.md: is missing, so [unsafe_crates] in tools/xtask/ci-policy.toml cannot be checked against it"
            ]
        );
    }

    #[test]
    fn a_simulation_asset_outside_the_boundary_is_reported() {
        // Row 66 of docs/verification.md: `task` is no boundary crate, `math` is.
        let (mut ws, mut files) = conforming();
        let declared = "pub struct Kind;\nimpl lactose_asset::SimulationAsset for Kind {}\n";
        files.insert("crates/task/src/kinds.rs", declared);
        files.insert("crates/task/src/lib.rs", "mod kinds;\n");
        let at = |file: &str, line: usize| {
            format!(
                "{file}: line {line}: `impl SimulationAsset` in `lactose-task`, which is not a boundary crate; a kind the simulation reads is declared in a boundary crate, where architecture section 8.2 holds its loader (D99), or only in tests"
            )
        };
        assert_eq!(run(&ws, &files), [at("crates/task/src/kinds.rs", 2)]);
        // A module under a nested directory, and one a `#[path]` names.
        files.insert(
            "crates/task/src/kinds.rs",
            "#[path = \"../extra/more.rs\"]\nmod more;\nmod deeper;\n",
        );
        files.insert("crates/task/extra/more.rs", declared);
        files.insert("crates/task/src/kinds/deeper/mod.rs", format!("\n{declared}"));
        assert_eq!(
            run(&ws, &files),
            [at("crates/task/extra/more.rs", 2), at("crates/task/src/kinds/deeper/mod.rs", 3)]
        );
        // A module declared inside an inline one sits in the inline module's directory, and a
        // file under `src/` that no route reaches is read as shipped.
        files.insert("crates/task/src/lib.rs", "mod kinds;\nmod outer {\n    mod inner;\n}\n");
        files.insert("crates/task/src/outer/inner.rs", declared);
        files.insert("crates/task/src/orphan.rs", declared);
        assert_eq!(
            run(&ws, &files),
            [
                at("crates/task/extra/more.rs", 2),
                at("crates/task/src/kinds/deeper/mod.rs", 3),
                at("crates/task/src/orphan.rs", 2),
                at("crates/task/src/outer/inner.rs", 2),
            ]
        );
        files.insert("crates/task/src/orphan.rs", format!("#![cfg(test)]\n{declared}"));
        files.insert("crates/task/src/outer/inner.rs", "");
        assert_eq!(
            run(&ws, &files),
            [at("crates/task/extra/more.rs", 2), at("crates/task/src/kinds/deeper/mod.rs", 3)]
        );

        // Compiled only for tests or a dev-only feature, outside `src/`, or in a boundary crate.
        let (_, mut files) = conforming();
        files.insert(
            "crates/task/src/lib.rs",
            format!(
                "#[cfg(test)]\nmod tests;\n#[cfg(test)]\nmod inline {{ {declared} }}\n\
                 #[cfg(feature = \"suite\")]\nmod suite;\n#[cfg(feature = \"alias\")]\nimpl SimulationAsset for Kind {{}}\n"
            ),
        );
        files.insert("crates/task/src/tests.rs", declared);
        files.insert("crates/task/src/suite.rs", declared);
        files.insert("crates/task/tests/kinds.rs", declared);
        files.insert("crates/task/benches/kinds.rs", declared);
        files.insert("crates/math/src/kinds.rs", declared);
        files.insert("crates/math/src/lib.rs", format!("{FORBID}mod kinds;\n"));
        ws.packages[2].crate_roots.push("crates/task/tests/kinds.rs".to_owned());
        ws.packages[2].features.insert("suite".to_owned(), Vec::new());
        ws.packages[2].features.insert("alias".to_owned(), vec!["suite".to_owned()]);
        // `alias` turns `suite` on, so it is held to the dev edges too.
        let tiers = TierFile::parse(&format!(
            "{TIERS}\n[dev_only_features]\n\"lactose-task\" = [\"suite\"]\n"
        ))
        .unwrap();
        let found: Vec<String> =
            check(&tiers, &policy(), &ws, &files).iter().map(ToString::to_string).collect();
        assert_eq!(found, NONE);
        // Without the table, the feature ships, and both of its impls are reported.
        assert_eq!(
            run(&ws, &files),
            [at("crates/task/src/lib.rs", 10), at("crates/task/src/suite.rs", 2)]
        );
    }

    #[test]
    fn what_a_world_registers_outside_every_boundary_lint_file_is_reported() {
        // Project review 2, X1: `task` carries no boundary lint file, `math` is a boundary crate.
        let (ws, mut files) = conforming();
        let game = "#[component(key = \"g::a\", persistent, replicated)]\nstruct A(u32);\n\
                    #[lactose_ecs::system(name = \"g::s\", worlds = [server])]\nfn s() {}\n\
                    #[resource(key = \"g::r\", transient)]\nstruct R;\n\
                    #[component(key = \"g::d\", derived)]\nstruct D;\n\
                    #[resource(key = \"g::e\", derived)]\nstruct E;\n\
                    impl SessionApplier for S {}\n\
                    impl lactose_sim::InputApplier for S {}\n\
                    impl<T> StagedState for H<T> {}\n\
                    impl Applier for S {}\n\
                    impl lactose_reflect::Reflect for A {}\n\
                    impl Reflected for A {}\n";
        files.insert("crates/task/src/lib.rs", "mod game;\n");
        files.insert("crates/task/src/game.rs", game);
        let at = |line: usize, what: &str| {
            format!(
                "crates/task/src/game.rs: line {line}: {what} in `lactose-task`, which carries no boundary lint file; what a world registers and runs is written in a boundary crate or a crate of [lint_file_carriers] in {POLICY_FILE}, under the lints of architecture section 8.2, or only in tests"
            )
        };
        assert_eq!(
            run(&ws, &files),
            [
                at(1, "`#[component]` that is not `derived`"),
                at(3, "`#[system]`"),
                at(5, "`#[resource]` that is not `derived`"),
                at(11, "`impl SessionApplier`"),
                at(12, "`impl InputApplier`"),
                at(13, "`impl StagedState`"),
                format!(
                    "crates/task/src/game.rs: line 15: a hand-written `impl Reflect` in `lactose-task`, which carries no boundary lint file; it declares a component or a resource that no attribute shows, and what a world stores is described in a boundary crate or a crate of [lint_file_carriers] in {POLICY_FILE}, under the lints of architecture section 8.2, or only in tests"
                ),
            ]
        );

        // Compiled only for tests, in a boundary crate, or in a lint-file carrier.
        let (ws, mut files) = conforming();
        files.insert(
            "crates/task/src/lib.rs",
            format!("#[cfg(test)]\nmod game;\n#[cfg(test)]\nmod inline {{\n{game}}}\n"),
        );
        files.insert("crates/task/src/game.rs", game);
        files.insert("crates/task/tests/game.rs", game);
        files.insert("crates/math/src/lib.rs", format!("{FORBID}{game}"));
        assert_eq!(run(&ws, &files), NONE);
        files.insert("crates/task/src/lib.rs", "mod game;\n");
        files.insert("crates/task/clippy.toml", lint_file());
        let mut carriers = policy();
        carriers.lint_file_carriers.push("lactose-task".to_owned());
        assert_eq!(run_with(&carriers, &ws, &files), NONE);
        let unlisted = run(&ws, &files);
        assert_eq!(unlisted.iter().filter(|f| f.contains("no boundary lint file")).count(), 7);
    }

    /// What row 66 reports for an `impl SimulationAsset` at `line` of `file` in `lactose-task`.
    fn declared_outside(file: &str, line: usize) -> String {
        format!(
            "{file}: line {line}: `impl SimulationAsset` in `lactose-task`, which is not a boundary crate; a kind the simulation reads is declared in a boundary crate, where architecture section 8.2 holds its loader (D99), or only in tests"
        )
    }

    #[test]
    fn every_shipped_route_to_a_simulation_asset_outside_the_boundary_is_followed() {
        // Review round 1, F3: the review's cases A to L, and the forms its fix names. Each
        // case is the files it adds to `lactose-task`, its shipped roots if not the library's,
        // and what is reported.
        let declared = "pub struct Kind;\nimpl lactose_asset_api::SimulationAsset for Kind {}\n";
        let in_lib = |before: &str| vec![("crates/task/src/lib.rs", format!("{before}{declared}"))];
        type Case =
            (&'static str, Vec<(&'static str, String)>, Vec<&'static str>, (&'static str, usize));
        let cases: Vec<Case> = vec![
            (
                "A: a test-only `#[path]` alias reaches the file first",
                vec![
                    ("crates/task/src/lib.rs", "#[cfg(test)]\n#[path = \"kinds.rs\"]\nmod test_kinds;\nmod kinds;\n".to_owned()),
                    ("crates/task/src/kinds.rs", declared.to_owned()),
                ],
                vec![],
                ("crates/task/src/kinds.rs", 2),
            ),
            (
                "A: the same file and directory, reached test-only and then shipped",
                vec![
                    ("crates/task/src/lib.rs", "#[cfg(test)]\n#[path = \"k/mod.rs\"]\nmod t;\nmod k;\n".to_owned()),
                    ("crates/task/src/k/mod.rs", declared.to_owned()),
                ],
                vec![],
                ("crates/task/src/k/mod.rs", 2),
            ),
            (
                "B: a test-only inline module's `mod kinds;` is its own file",
                vec![
                    ("crates/task/src/lib.rs", "#[cfg(test)]\nmod t {\n    mod kinds;\n}\nmod kinds;\n".to_owned()),
                    ("crates/task/src/kinds.rs", declared.to_owned()),
                    ("crates/task/src/t/kinds.rs", "\n".to_owned()),
                ],
                vec![],
                ("crates/task/src/kinds.rs", 2),
            ),
            (
                "B: a `#[path]` inside an inline module is relative to the module's directory",
                vec![
                    ("crates/task/src/lib.rs", "mod outer {\n    #[path = \"x.rs\"]\n    mod inner;\n}\n".to_owned()),
                    ("crates/task/src/outer/x.rs", declared.to_owned()),
                    ("crates/task/src/x.rs", "\n".to_owned()),
                ],
                vec![],
                ("crates/task/src/outer/x.rs", 2),
            ),
            (
                // Review round 2, R1: an inline module's `#[path]` names its directory.
                "B: an inline module's `#[path]` outside `src/`",
                vec![
                    ("crates/task/src/lib.rs", "#[path = \"../gen\"]\nmod g {\n    mod kinds;\n}\n".to_owned()),
                    ("crates/task/gen/kinds.rs", declared.to_owned()),
                ],
                vec![],
                ("crates/task/gen/kinds.rs", 2),
            ),
            (
                "B: an inline module's `#[path]` inside `src/`, with a test-only alias first",
                vec![
                    (
                        "crates/task/src/lib.rs",
                        "#[cfg(test)]\n#[path = \"other/kinds.rs\"]\nmod t;\n#[path = \"other\"]\nmod g {\n    mod kinds;\n}\n"
                            .to_owned(),
                    ),
                    ("crates/task/src/other/kinds.rs", declared.to_owned()),
                ],
                vec![],
                ("crates/task/src/other/kinds.rs", 2),
            ),
            (
                // As rustc places them: at the top of `a.rs` an inline module's `#[path]` is
                // relative to `src/`, not `src/a/`.
                "B: an inline module's `#[path]` at the top of a `name.rs`",
                vec![
                    (
                        "crates/task/src/lib.rs",
                        "#[cfg(test)]\n#[path = \"p/kinds.rs\"]\nmod t;\nmod a;\n".to_owned(),
                    ),
                    ("crates/task/src/a.rs", "#[path = \"p\"]\nmod g {\n    mod kinds;\n}\n".to_owned()),
                    ("crates/task/src/p/kinds.rs", declared.to_owned()),
                    ("crates/task/src/a/p/kinds.rs", "\n".to_owned()),
                ],
                vec![],
                ("crates/task/src/p/kinds.rs", 2),
            ),
            (
                // Under another inline module, relative to that module's directory.
                "B: an inline module's `#[path]` inside another inline module of a `name.rs`",
                vec![
                    (
                        "crates/task/src/lib.rs",
                        "#[cfg(test)]\n#[path = \"a/m/q/kinds.rs\"]\nmod t;\nmod a;\n".to_owned(),
                    ),
                    (
                        "crates/task/src/a.rs",
                        "mod m {\n    #[path = \"q\"]\n    mod g {\n        mod kinds;\n    }\n}\n".to_owned(),
                    ),
                    ("crates/task/src/a/m/q/kinds.rs", declared.to_owned()),
                    ("crates/task/src/q/kinds.rs", "\n".to_owned()),
                ],
                vec![],
                ("crates/task/src/a/m/q/kinds.rs", 2),
            ),
            (
                "C: a library root outside `src/`",
                vec![("crates/task/lib.rs", declared.to_owned())],
                vec!["crates/task/lib.rs"],
                ("crates/task/lib.rs", 2),
            ),
            (
                "C: a module of a library root outside `src/`",
                vec![
                    ("crates/task/lib.rs", "mod kinds;\n".to_owned()),
                    ("crates/task/kinds.rs", declared.to_owned()),
                ],
                vec!["crates/task/lib.rs"],
                ("crates/task/kinds.rs", 2),
            ),
            (
                "D: a binary root read first, with a test-only alias of a shipped module",
                vec![
                    ("crates/task/src/lib.rs", "mod kinds;\n".to_owned()),
                    ("crates/task/src/bin/tool.rs", "#[cfg(test)]\n#[path = \"../kinds.rs\"]\nmod k;\nfn main() {}\n".to_owned()),
                    ("crates/task/src/kinds.rs", declared.to_owned()),
                ],
                vec!["crates/task/src/bin/tool.rs", "crates/task/src/lib.rs"],
                ("crates/task/src/kinds.rs", 2),
            ),
            (
                "E: `include!` of a file outside `src/`",
                vec![
                    ("crates/task/src/lib.rs", "include!(\"../gen/kinds.rs\");\n".to_owned()),
                    ("crates/task/gen/kinds.rs", declared.to_owned()),
                ],
                vec![],
                ("crates/task/gen/kinds.rs", 2),
            ),
            (
                "E: a module of an included file sits beside it",
                vec![
                    ("crates/task/src/lib.rs", "mod a {\n    include!(\"../gen/kinds.rs\");\n}\n".to_owned()),
                    ("crates/task/gen/kinds.rs", "mod more;\n".to_owned()),
                    ("crates/task/gen/more.rs", declared.to_owned()),
                ],
                vec![],
                ("crates/task/gen/more.rs", 2),
            ),
            ("F: `cfg(not(test))`", in_lib("#[cfg(not(test))]\n"), vec![], ("crates/task/src/lib.rs", 3)),
            ("G: `cfg(any(test, feature))`", in_lib("#[cfg(any(test, feature = \"x\"))]\n"), vec![], ("crates/task/src/lib.rs", 3)),
            ("H: `cfg_attr(test, ...)`", in_lib("#[cfg_attr(test, allow(dead_code))]\n"), vec![], ("crates/task/src/lib.rs", 3)),
            ("I: `cfg(all(feature, not(test)))`", in_lib("#[cfg(all(feature = \"x\", not(test)))]\n"), vec![], ("crates/task/src/lib.rs", 3)),
            ("J: a gated item with a `;` in brackets before it", in_lib("#[cfg(test)]\nconst N: [u8; 2] = [0; 2];\n"), vec![], ("crates/task/src/lib.rs", 4)),
            ("K: a gated function before it", in_lib("#[cfg(test)]\nfn helper() {}\n"), vec![], ("crates/task/src/lib.rs", 4)),
            (
                "L: a generic header under a global path",
                vec![("crates/task/src/lib.rs", "pub struct K<T>(T);\nunsafe impl<T: Copy> ::lactose_asset_api::SimulationAsset for K<T> where T: Default {}\n".to_owned())],
                vec![],
                ("crates/task/src/lib.rs", 2),
            ),
        ];
        for (name, added, roots, (file, line)) in cases {
            let (mut ws, mut files) = conforming();
            if !roots.is_empty() {
                ws.packages[2].shipped_roots =
                    roots.iter().map(|root| (*root).to_owned()).collect();
                ws.packages[2].crate_roots = ws.packages[2].shipped_roots.clone();
            }
            for (path, text) in &added {
                files.insert(*path, text.as_str());
            }
            assert_eq!(run(&ws, &files), [declared_outside(file, line)], "{name}");
        }
    }

    #[test]
    fn a_loader_outside_the_boundary_for_a_kind_the_simulation_reads_is_reported() {
        // Review round 1, F4: the review's probe, a loader in a crate outside the boundary
        // (`task`) for a kind a boundary crate (`math`) declares, judged with the platform's
        // `sin`.
        let (ws, mut files) = conforming();
        files.insert(
            "crates/math/src/lib.rs",
            format!(
                "{FORBID}pub struct Clip;\nimpl lactose_asset_api::SimulationAsset for Clip {{}}\n\
                 pub struct ClipLoader;\nimpl Loader for ClipLoader {{\n    type Asset = Clip;\n}}\n\
                 #[cfg(test)]\nmod tests {{\n    impl SimulationAsset for Probe {{}}\n}}\n"
            ),
        );
        assert_eq!(run(&ws, &files), NONE, "the kind's own loader sits in its boundary crate");
        let probe = "//! Review probe (xtask-policy r1, row 66): a loader for a simulation kind, outside the boundary.\n\
                     use lactose_asset_api::{AssetId, Loader};\n\
                     use lactose_anim::{Clip, ClipLoader};\n\
                     use lactose_core::Result;\n\
                     \n\
                     /// Decodes a clip, then judges it with the platform's `sin`.\n\
                     pub struct PlatformClipLoader;\n\
                     \n\
                     impl Loader for PlatformClipLoader {\n\
                     \x20   type Asset = Clip;\n\
                     \n\
                     \x20   fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<Clip> {\n\
                     \x20       let clip = ClipLoader.load(bytes, dependencies)?;\n\
                     \x20       let judged = (bytes.len() as f64).sin();\n\
                     \x20       if judged > 2.0 { ClipLoader.load(&[], dependencies) } else { Ok(clip) }\n\
                     \x20   }\n\
                     }\n";
        files.insert("crates/task/src/lib.rs", "mod platform;\n");
        files.insert("crates/task/src/platform.rs", probe);
        let loader = |line: usize, decoded: &str| {
            format!(
                "crates/task/src/platform.rs: line {line}: `impl Loader` in `lactose-task`, which is not a boundary crate, decodes {decoded}; the loader of a kind the simulation reads sits in a boundary crate, where architecture section 8.2 holds it (D99), or only in tests"
            )
        };
        let a_kind =
            |asset: &str| format!("`{asset}`, which a boundary crate declares a `SimulationAsset`");
        assert_eq!(run(&ws, &files), [loader(9, &a_kind("Clip"))]);

        // Under any path, with a type parameter, and with no `type Asset` in sight.
        files.insert(
            "crates/task/src/platform.rs",
            "impl lactose_asset_api::Loader for A {\n    type Asset = lactose_math::Clip;\n}\n\
             impl<L: Loader> Loader for Wrapped<L> {\n    type Asset = L::Asset;\n}\n\
             impl<T> Loader for Any<T> where T: Asset {\n    type Asset = Vec<T>;\n}\n\
             impl Loader for Hidden {\n    asset_type!();\n}\n\
             impl<L: Loader> Loader for Qualified<L> {\n    type Asset = <L as Loader>::Asset;\n}\n\
             impl Loader for Projected {\n    type Asset = <lactose_math::Clip as Decoded>::Output;\n}\n",
        );
        assert_eq!(
            run(&ws, &files),
            [
                loader(1, &a_kind("lactose_math::Clip")),
                loader(
                    4,
                    "`L::Asset`, which names its type parameter `L`, so it may decode a kind a boundary crate declares a `SimulationAsset`"
                ),
                loader(
                    7,
                    "`Vec<T>`, which names its type parameter `T`, so it may decode a kind a boundary crate declares a `SimulationAsset`"
                ),
                loader(
                    10,
                    "a kind its text does not show (no `type Asset = ...;` in its body), which may be one a boundary crate declares a `SimulationAsset`"
                ),
                // Review round 2, R2: the words of a qualified path stay apart.
                loader(
                    13,
                    "`<L as Loader>::Asset`, which names its type parameter `L`, so it may decode a kind a boundary crate declares a `SimulationAsset`"
                ),
                loader(16, &a_kind("<lactose_math::Clip as Decoded>::Output")),
            ]
        );

        // A loader for a kind no boundary crate declares, one for a kind declared only in a
        // boundary crate's tests, and one compiled only for tests, are not.
        files.insert(
            "crates/task/src/platform.rs",
            "impl Loader for RegionBaselineLoader {\n    type Asset = RegionBaseline;\n}\n\
             impl Loader for ProbeLoader {\n    type Asset = Probe;\n}\n\
             #[cfg(test)]\nmod tests {\n    impl Loader for TestLoader {\n        type Asset = Clip;\n    }\n}\n\
             impl Other for B {\n    type Asset = Clip;\n}\n\
             impl Loader for Aliased {\n    type Asset = <Self as Decodes>::Kind;\n}\n",
        );
        // The last is the admitted limit: a projection names its type only through the trait.
        assert_eq!(run(&ws, &files), NONE);
    }
}
