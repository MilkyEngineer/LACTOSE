//! The reference lint file, `crates/core/clippy.toml`, read by clippy itself over a probe crate
//! in a temporary workspace that names the solver's math directly.
//!
//! The routes of D119 through the solver's math crates are banned by path with
//! `allow-invalid`, which silences a path that does not resolve; so a typo, or a path that
//! moves in an upgrade, would ban nothing and say nothing. This proves each such route is
//! refused by the def ids clippy resolves, at the versions the repository's lock holds, which
//! are in the registry cache wherever this package is built (the physics backend compiles
//! them). The same probe proves the run-time CPU feature detection of the target it is built
//! for refused under `disallowed-macros` (section 8.2), by the root path as by `std::arch`, and
//! on x86-64 CPUID read directly, by its `core` path, its `std::arch` re-export, a rename and
//! `__get_cpuid_max`.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| env!("CARGO").to_owned())
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{}: {e}", parent.display()));
    }
    fs::write(path, contents).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// Each line of the probe that clippy must refuse, and the path its finding names: one line
/// per route of D119 banned by trait path, inherent method of a dependency or type, and the
/// feature detection macro of the target. A line whose path is empty compiles to nothing on
/// this target.
const REFUSED: [(&str, &str); 106] = [
    ("use num_traits::Float;", "num_traits::Float"),
    ("pub fn generic<T: num_traits::real::Real>(x: T) -> T { x.sin() }", "num_traits::real::Real"),
    (
        "pub fn qualified(x: f64) -> f64 { simba::scalar::RealField::max(x, 0.0) }",
        "simba::scalar::RealField::max",
    ),
    (
        "pub fn real_min(x: f64) -> f64 { simba::scalar::RealField::min(x, 0.0) }",
        "simba::scalar::RealField::min",
    ),
    (
        "pub fn real_copysign(x: f64) -> f64 { <f64 as simba::scalar::RealField>::copysign(x, x) }",
        "simba::scalar::RealField::copysign",
    ),
    (
        "pub fn bounded<T: simba::scalar::RealField>(x: T) -> bool { x.is_sign_negative() }",
        "simba::scalar::RealField::is_sign_negative",
    ),
    (
        "pub fn real_positive<T: simba::scalar::RealField>(x: T) -> bool { x.is_sign_positive() }",
        "simba::scalar::RealField::is_sign_positive",
    ),
    (
        "pub fn lanes<T: simba::simd::SimdRealField>(x: T, y: T) -> T { x.simd_copysign(y) }",
        "simba::simd::SimdRealField::simd_copysign",
    ),
    (
        "pub fn simd_positive(x: f64) -> bool { simba::simd::SimdSigned::is_simd_positive(&x) }",
        "simba::simd::SimdSigned::is_simd_positive",
    ),
    (
        "pub fn simd_negative<T: simba::simd::SimdSigned>(x: T) -> T::SimdBool { x.is_simd_negative() }",
        "simba::simd::SimdSigned::is_simd_negative",
    ),
    (
        "pub fn signed_positive<T: simba::scalar::RealField>(x: T) -> bool { x.is_positive() }",
        "num_traits::sign::Signed::is_positive",
    ),
    (
        "pub fn signed_negative<T: simba::scalar::RealField>(x: T) -> bool { x.is_negative() }",
        "num_traits::sign::Signed::is_negative",
    ),
    ("pub fn f32_asinh(x: f32) -> f32 { x.asinh() }", "f32::asinh"),
    ("pub fn f32_acosh(x: f32) -> f32 { x.acosh() }", "f32::acosh"),
    ("pub fn f32_atanh(x: f32) -> f32 { x.atanh() }", "f32::atanh"),
    ("pub fn f64_asinh(x: f64) -> f64 { x.asinh() }", "f64::asinh"),
    ("pub fn f64_acosh(x: f64) -> f64 { x.acosh() }", "f64::acosh"),
    ("pub fn f64_atanh(x: f64) -> f64 { x.atanh() }", "f64::atanh"),
    ("pub fn vec2_copysign(v: glam::Vec2) { let _ = v.copysign(v); }", "glam::Vec2::copysign"),
    (
        "pub fn vec2_is_negative_bitmask(v: glam::Vec2) { let _ = v.is_negative_bitmask(); }",
        "glam::Vec2::is_negative_bitmask",
    ),
    (
        "pub fn vec2_is_negative_mask(v: glam::Vec2) { let _ = v.is_negative_mask(); }",
        "glam::Vec2::is_negative_mask",
    ),
    (
        "pub fn vec2_rem_euclid(v: glam::Vec2) { let _ = v.rem_euclid(v); }",
        "glam::Vec2::rem_euclid",
    ),
    (
        "pub fn vec2_div_euclid(v: glam::Vec2) { let _ = v.div_euclid(v); }",
        "glam::Vec2::div_euclid",
    ),
    ("pub fn vec3_copysign(v: glam::Vec3) { let _ = v.copysign(v); }", "glam::Vec3::copysign"),
    (
        "pub fn vec3_is_negative_bitmask(v: glam::Vec3) { let _ = v.is_negative_bitmask(); }",
        "glam::Vec3::is_negative_bitmask",
    ),
    (
        "pub fn vec3_is_negative_mask(v: glam::Vec3) { let _ = v.is_negative_mask(); }",
        "glam::Vec3::is_negative_mask",
    ),
    (
        "pub fn vec3_rem_euclid(v: glam::Vec3) { let _ = v.rem_euclid(v); }",
        "glam::Vec3::rem_euclid",
    ),
    (
        "pub fn vec3_div_euclid(v: glam::Vec3) { let _ = v.div_euclid(v); }",
        "glam::Vec3::div_euclid",
    ),
    ("pub fn vec3a_copysign(v: glam::Vec3A) { let _ = v.copysign(v); }", "glam::Vec3A::copysign"),
    (
        "pub fn vec3a_is_negative_bitmask(v: glam::Vec3A) { let _ = v.is_negative_bitmask(); }",
        "glam::Vec3A::is_negative_bitmask",
    ),
    (
        "pub fn vec3a_is_negative_mask(v: glam::Vec3A) { let _ = v.is_negative_mask(); }",
        "glam::Vec3A::is_negative_mask",
    ),
    (
        "pub fn vec3a_rem_euclid(v: glam::Vec3A) { let _ = v.rem_euclid(v); }",
        "glam::Vec3A::rem_euclid",
    ),
    (
        "pub fn vec3a_div_euclid(v: glam::Vec3A) { let _ = v.div_euclid(v); }",
        "glam::Vec3A::div_euclid",
    ),
    ("pub fn vec4_copysign(v: glam::Vec4) { let _ = v.copysign(v); }", "glam::Vec4::copysign"),
    (
        "pub fn vec4_is_negative_bitmask(v: glam::Vec4) { let _ = v.is_negative_bitmask(); }",
        "glam::Vec4::is_negative_bitmask",
    ),
    (
        "pub fn vec4_is_negative_mask(v: glam::Vec4) { let _ = v.is_negative_mask(); }",
        "glam::Vec4::is_negative_mask",
    ),
    (
        "pub fn vec4_rem_euclid(v: glam::Vec4) { let _ = v.rem_euclid(v); }",
        "glam::Vec4::rem_euclid",
    ),
    (
        "pub fn vec4_div_euclid(v: glam::Vec4) { let _ = v.div_euclid(v); }",
        "glam::Vec4::div_euclid",
    ),
    ("pub fn dvec2_copysign(v: glam::DVec2) { let _ = v.copysign(v); }", "glam::DVec2::copysign"),
    (
        "pub fn dvec2_is_negative_bitmask(v: glam::DVec2) { let _ = v.is_negative_bitmask(); }",
        "glam::DVec2::is_negative_bitmask",
    ),
    (
        "pub fn dvec2_is_negative_mask(v: glam::DVec2) { let _ = v.is_negative_mask(); }",
        "glam::DVec2::is_negative_mask",
    ),
    (
        "pub fn dvec2_rem_euclid(v: glam::DVec2) { let _ = v.rem_euclid(v); }",
        "glam::DVec2::rem_euclid",
    ),
    (
        "pub fn dvec2_div_euclid(v: glam::DVec2) { let _ = v.div_euclid(v); }",
        "glam::DVec2::div_euclid",
    ),
    ("pub fn dvec3_copysign(v: glam::DVec3) { let _ = v.copysign(v); }", "glam::DVec3::copysign"),
    (
        "pub fn dvec3_is_negative_bitmask(v: glam::DVec3) { let _ = v.is_negative_bitmask(); }",
        "glam::DVec3::is_negative_bitmask",
    ),
    (
        "pub fn dvec3_is_negative_mask(v: glam::DVec3) { let _ = v.is_negative_mask(); }",
        "glam::DVec3::is_negative_mask",
    ),
    (
        "pub fn dvec3_rem_euclid(v: glam::DVec3) { let _ = v.rem_euclid(v); }",
        "glam::DVec3::rem_euclid",
    ),
    (
        "pub fn dvec3_div_euclid(v: glam::DVec3) { let _ = v.div_euclid(v); }",
        "glam::DVec3::div_euclid",
    ),
    ("pub fn dvec4_copysign(v: glam::DVec4) { let _ = v.copysign(v); }", "glam::DVec4::copysign"),
    (
        "pub fn dvec4_is_negative_bitmask(v: glam::DVec4) { let _ = v.is_negative_bitmask(); }",
        "glam::DVec4::is_negative_bitmask",
    ),
    (
        "pub fn dvec4_is_negative_mask(v: glam::DVec4) { let _ = v.is_negative_mask(); }",
        "glam::DVec4::is_negative_mask",
    ),
    (
        "pub fn dvec4_rem_euclid(v: glam::DVec4) { let _ = v.rem_euclid(v); }",
        "glam::DVec4::rem_euclid",
    ),
    (
        "pub fn dvec4_div_euclid(v: glam::DVec4) { let _ = v.div_euclid(v); }",
        "glam::DVec4::div_euclid",
    ),
    (
        "pub fn complex_cis(x: f64) { let _ = num_complex::Complex::cis(x); }",
        "num_complex::Complex::cis",
    ),
    ("pub fn complex_norm(x: f64) { let _ = z(x).norm(); }", "num_complex::Complex::norm"),
    ("pub fn complex_arg(x: f64) { let _ = z(x).arg(); }", "num_complex::Complex::arg"),
    (
        "pub fn complex_to_polar(x: f64) { let _ = z(x).to_polar(); }",
        "num_complex::Complex::to_polar",
    ),
    (
        "pub fn complex_from_polar(x: f64) { let _ = num_complex::Complex::from_polar(x, x); }",
        "num_complex::Complex::from_polar",
    ),
    ("pub fn complex_exp(x: f64) { let _ = z(x).exp(); }", "num_complex::Complex::exp"),
    ("pub fn complex_ln(x: f64) { let _ = z(x).ln(); }", "num_complex::Complex::ln"),
    ("pub fn complex_sqrt(x: f64) { let _ = z(x).sqrt(); }", "num_complex::Complex::sqrt"),
    ("pub fn complex_cbrt(x: f64) { let _ = z(x).cbrt(); }", "num_complex::Complex::cbrt"),
    ("pub fn complex_powf(x: f64) { let _ = z(x).powf(x); }", "num_complex::Complex::powf"),
    ("pub fn complex_log(x: f64) { let _ = z(x).log(x); }", "num_complex::Complex::log"),
    ("pub fn complex_powc(x: f64) { let _ = z(x).powc(z(x)); }", "num_complex::Complex::powc"),
    ("pub fn complex_expf(x: f64) { let _ = z(x).expf(x); }", "num_complex::Complex::expf"),
    ("pub fn complex_sin(x: f64) { let _ = z(x).sin(); }", "num_complex::Complex::sin"),
    ("pub fn complex_cos(x: f64) { let _ = z(x).cos(); }", "num_complex::Complex::cos"),
    ("pub fn complex_tan(x: f64) { let _ = z(x).tan(); }", "num_complex::Complex::tan"),
    ("pub fn complex_asin(x: f64) { let _ = z(x).asin(); }", "num_complex::Complex::asin"),
    ("pub fn complex_acos(x: f64) { let _ = z(x).acos(); }", "num_complex::Complex::acos"),
    ("pub fn complex_atan(x: f64) { let _ = z(x).atan(); }", "num_complex::Complex::atan"),
    ("pub fn complex_sinh(x: f64) { let _ = z(x).sinh(); }", "num_complex::Complex::sinh"),
    ("pub fn complex_cosh(x: f64) { let _ = z(x).cosh(); }", "num_complex::Complex::cosh"),
    ("pub fn complex_tanh(x: f64) { let _ = z(x).tanh(); }", "num_complex::Complex::tanh"),
    ("pub fn complex_asinh(x: f64) { let _ = z(x).asinh(); }", "num_complex::Complex::asinh"),
    ("pub fn complex_acosh(x: f64) { let _ = z(x).acosh(); }", "num_complex::Complex::acosh"),
    ("pub fn complex_atanh(x: f64) { let _ = z(x).atanh(); }", "num_complex::Complex::atanh"),
    ("pub fn complex_finv(x: f64) { let _ = z(x).finv(); }", "num_complex::Complex::finv"),
    ("pub fn complex_fdiv(x: f64) { let _ = z(x).fdiv(z(x)); }", "num_complex::Complex::fdiv"),
    ("pub fn complex_exp2(x: f64) { let _ = z(x).exp2(); }", "num_complex::Complex::exp2"),
    ("pub fn complex_log2(x: f64) { let _ = z(x).log2(); }", "num_complex::Complex::log2"),
    ("pub fn complex_log10(x: f64) { let _ = z(x).log10(); }", "num_complex::Complex::log10"),
    (
        "pub fn mat_symmetric_eigen(m: glamx::DMat3) { let _ = m.symmetric_eigen(); }",
        "glamx::MatExt::symmetric_eigen",
    ),
    (
        "pub fn mat_symmetric_eigenvalues(m: glamx::DMat3) { let _ = m.symmetric_eigenvalues(); }",
        "glamx::MatExt::symmetric_eigenvalues",
    ),
    ("pub fn mat_svd(m: glamx::DMat3) { let _ = m.svd(); }", "glamx::MatExt::svd"),
    (
        "pub fn ulps_eq(x: f64) -> bool { approx::UlpsEq::ulps_eq(&x, &x, 0.0, 0) }",
        "approx::UlpsEq::ulps_eq",
    ),
    (
        "pub fn ulps_ne(x: f64) -> bool { approx::UlpsEq::ulps_ne(&x, &x, 0.0, 0) }",
        "approx::UlpsEq::ulps_ne",
    ),
    ("pub fn complex(_: num_complex::Complex<f32>) {}", "num_complex::Complex"),
    ("pub fn symmetriceigen3(_: glamx::SymmetricEigen3) {}", "glamx::SymmetricEigen3"),
    ("pub fn symmetriceigen3a(_: glamx::SymmetricEigen3A) {}", "glamx::SymmetricEigen3A"),
    ("pub fn dsymmetriceigen3(_: glamx::DSymmetricEigen3) {}", "glamx::DSymmetricEigen3"),
    ("pub fn svd2(_: glamx::Svd2) {}", "glamx::Svd2"),
    ("pub fn dsvd2(_: glamx::DSvd2) {}", "glamx::DSvd2"),
    ("pub fn svd3(_: glamx::Svd3) {}", "glamx::Svd3"),
    ("pub fn svd3a(_: glamx::Svd3A) {}", "glamx::Svd3A"),
    ("pub fn dsvd3(_: glamx::DSvd3) {}", "glamx::DSvd3"),
    ("pub fn widef32x4(_: simba::simd::WideF32x4) {}", "simba::simd::WideF32x4"),
    ("pub fn widef32x8(_: simba::simd::WideF32x8) {}", "simba::simd::WideF32x8"),
    ("pub fn widef64x4(_: simba::simd::WideF64x4) {}", "simba::simd::WideF64x4"),
    // The control: the inherent method the routes reach is refused as it always was.
    ("pub fn control(x: f64) -> f64 { x.max(0.0) }", "f64::max"),
    // Each CI platform is x86-64 or aarch64, and each target has only its own macro: the line
    // calls the x86 one by its root path, which clippy reports under `std::arch`.
    (
        "#[cfg(target_arch = \"x86_64\")] pub fn detect() -> bool { std::is_x86_feature_detected!(\"avx2\") } \
         #[cfg(target_arch = \"aarch64\")] pub fn detect() -> bool { std::arch::is_aarch64_feature_detected!(\"sve\") }",
        if cfg!(target_arch = "x86_64") {
            "std::arch::is_x86_feature_detected"
        } else {
            "std::arch::is_aarch64_feature_detected"
        },
    ),
    // CPUID exists only on x86-64, where clippy reports each spelling by the `core` path. On
    // aarch64 the lines compile to nothing, and `allow-invalid` keeps the bans quiet there.
    (
        "#[cfg(target_arch = \"x86_64\")] pub fn leaf1() -> u32 { core::arch::x86_64::__cpuid(1).ecx }",
        if X86_64 { "core::arch::x86_64::__cpuid" } else { "" },
    ),
    (
        "#[cfg(target_arch = \"x86_64\")] pub fn leaf7() -> u32 { std::arch::x86_64::__cpuid_count(7, 0).ebx }",
        if X86_64 { "core::arch::x86_64::__cpuid_count" } else { "" },
    ),
    (
        "#[cfg(target_arch = \"x86_64\")] pub fn renamed() -> u32 { use std::arch::x86_64::__cpuid as id; id(0).eax }",
        if X86_64 { "core::arch::x86_64::__cpuid" } else { "" },
    ),
    (
        "#[cfg(target_arch = \"x86_64\")] pub fn max7() -> u32 { core::arch::x86_64::__get_cpuid_max(7).1 }",
        if X86_64 { "core::arch::x86_64::__get_cpuid_max" } else { "" },
    ),
];

const X86_64: bool = cfg!(target_arch = "x86_64");

/// What the lines of [`REFUSED`] call, after them.
const HELPERS: &str = "use glamx::MatExt as _;\n\
                       fn z(x: f64) -> num_complex::Complex<f64> { num_complex::Complex::new(x, x) }\n";

#[test]
fn the_routes_to_the_platform_float_library_are_refused() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("boundary-lints");
    let ws = base.join("ws");
    if ws.exists() {
        fs::remove_dir_all(&ws).unwrap_or_else(|e| panic!("{}: {e}", ws.display()));
    }
    write(
        &ws.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
         [workspace]\n\n[dependencies]\napprox = \"0.5\"\nglam = \"0.33\"\nglamx = \"0.3\"\n\
         num-complex = \"0.4\"\nnum-traits = \"0.2\"\nsimba = { version = \"0.10\", features = [\"wide\"] }\n",
    );
    // The repository's lock, so cargo keeps its versions and drops what the probe does not use.
    let lock = fs::read_to_string(repository.join("Cargo.lock")).unwrap();
    write(&ws.join("Cargo.lock"), &lock);
    let reference = fs::read_to_string(repository.join("crates/core/clippy.toml")).unwrap();
    write(&ws.join("clippy.toml"), &reference);
    let source: Vec<&str> = REFUSED.iter().map(|(line, _)| *line).collect();
    write(&ws.join("src/lib.rs"), &format!("{}\n{HELPERS}", source.join("\n")));

    let output = Command::new(cargo())
        .args(["clippy", "--offline", "--quiet", "--message-format", "short", "--"])
        .args(["-D", "clippy::disallowed_methods", "-D", "clippy::disallowed_types"])
        .args(["-D", "clippy::disallowed_macros"])
        .current_dir(&ws)
        .env("CARGO_TARGET_DIR", base.join("target"))
        .output()
        .unwrap_or_else(|e| panic!("cannot run cargo clippy: {e}"));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "clippy refused nothing:\n{text}");
    for (at, (line, path)) in REFUSED.iter().enumerate().filter(|(_, (_, path))| !path.is_empty()) {
        // Windows writes the path with a backslash.
        let finding = text.lines().map(|finding| finding.replace('\\', "/")).any(|finding| {
            finding.starts_with(&format!("src/lib.rs:{}:", at + 1))
                && finding.contains("disallowed")
                && finding.contains(&format!("`{path}`"))
        });
        assert!(finding, "line {}, `{line}`, is not refused for `{path}`:\n{text}", at + 1);
    }
}
