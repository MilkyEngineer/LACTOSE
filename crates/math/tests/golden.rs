//! Golden vectors: the cross-architecture proof.
//!
//! `tests/golden/vectors.txt` holds, for every operation, input bit patterns and the canonical
//! bits of the results as computed on aarch64. CI runs this test on x86-64 Linux and Windows
//! too, so a pass there shows the three targets agree bit for bit.
//!
//! Each line is `operation|inputs|outputs`, the values comma-separated 16-digit hex. Inputs
//! are raw bit patterns, NaN sign and payload included, so the table also shows that no
//! payload reaches a result.
//!
//! To regenerate after adding an operation or a case, on aarch64:
//! `cargo nextest run -p lactose-math --run-ignored only -E 'test(bless)'`, then review the
//! diff. A changed line for an existing case is a determinism break, not a refresh.

#![forbid(unsafe_code)]

use std::fmt::Write as _;

use lactose_math::render::RenderPosition;
use lactose_math::{
    ExactSum, Mat3, Mat4, Max, Min, Quat, Reducer, Scalar, Transform, Vec2, Vec3, Vec4,
};

type Eval = fn(&[Scalar]) -> Vec<Scalar>;

/// How an operation's inputs are generated.
#[derive(Clone, Copy)]
enum Inputs {
    /// Every edge value, then random values.
    Unary,
    /// Every pair of a smaller edge set, then random pairs.
    Binary,
    /// Every triple of a still smaller edge set, then random triples.
    Ternary,
    /// Random tuples of this many scalars, mostly of moderate magnitude.
    Tuple(usize),
    /// Random finite tuples of this many scalars within the render grid's range.
    Finite(usize),
    /// Every unary edge, then arbitrary 64-bit patterns, for operations that read the input
    /// as an integer.
    Bits,
    /// Lists of varying length, with cancellation and special values, this many rounds.
    List(usize),
}

struct Operation {
    name: &'static str,
    inputs: Inputs,
    eval: Eval,
}

const fn op(name: &'static str, inputs: Inputs, eval: Eval) -> Operation {
    Operation { name, inputs, eval }
}

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

fn flag(value: bool) -> Scalar {
    if value { Scalar::ONE } else { Scalar::ZERO }
}

fn vec2(v: &[Scalar]) -> Vec2 {
    Vec2::new(v[0], v[1])
}

fn vec3(v: &[Scalar]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

fn vec4(v: &[Scalar]) -> Vec4 {
    Vec4::new(v[0], v[1], v[2], v[3])
}

fn quat(v: &[Scalar]) -> Quat {
    Quat::new(v[0], v[1], v[2], v[3])
}

fn mat3(v: &[Scalar]) -> Mat3 {
    Mat3::from_array(v[..9].try_into().unwrap())
}

fn mat4(v: &[Scalar]) -> Mat4 {
    Mat4::from_array(v[..16].try_into().unwrap())
}

fn transform(v: &[Scalar]) -> Transform {
    Transform::new(vec3(v), quat(&v[3..]).normalize(), vec3(&v[7..]))
}

fn operations() -> Vec<Operation> {
    use Inputs::{Binary, Bits, Finite, List, Ternary, Tuple, Unary};
    vec![
        // Scalar, one operand.
        op("sqrt", Unary, |v| vec![v[0].sqrt()]),
        op("sin", Unary, |v| vec![v[0].sin()]),
        op("cos", Unary, |v| vec![v[0].cos()]),
        op("sin_cos", Unary, |v| {
            let (sin, cos) = v[0].sin_cos();
            vec![sin, cos]
        }),
        op("tan", Unary, |v| vec![v[0].tan()]),
        op("asin", Unary, |v| vec![v[0].asin()]),
        op("acos", Unary, |v| vec![v[0].acos()]),
        op("atan", Unary, |v| vec![v[0].atan()]),
        op("sinh", Unary, |v| vec![v[0].sinh()]),
        op("cosh", Unary, |v| vec![v[0].cosh()]),
        op("tanh", Unary, |v| vec![v[0].tanh()]),
        op("exp", Unary, |v| vec![v[0].exp()]),
        op("exp2", Unary, |v| vec![v[0].exp2()]),
        op("exp_m1", Unary, |v| vec![v[0].exp_m1()]),
        op("ln", Unary, |v| vec![v[0].ln()]),
        op("ln_1p", Unary, |v| vec![v[0].ln_1p()]),
        op("log2", Unary, |v| vec![v[0].log2()]),
        op("log10", Unary, |v| vec![v[0].log10()]),
        op("cbrt", Unary, |v| vec![v[0].cbrt()]),
        op("floor", Unary, |v| vec![v[0].floor()]),
        op("ceil", Unary, |v| vec![v[0].ceil()]),
        op("trunc", Unary, |v| vec![v[0].trunc()]),
        op("round", Unary, |v| vec![v[0].round()]),
        op("abs", Unary, |v| vec![v[0].abs()]),
        op("sign", Unary, |v| vec![v[0].sign()]),
        op("neg", Unary, |v| vec![-v[0]]),
        op("recip", Unary, |v| vec![v[0].recip()]),
        op("to_radians", Unary, |v| vec![v[0].to_radians()]),
        op("to_degrees", Unary, |v| vec![v[0].to_degrees()]),
        op("to_i32", Unary, |v| vec![Scalar::from_i32(v[0].to_i32())]),
        op("to_i64", Unary, |v| vec![Scalar::from_i64(v[0].to_i64())]),
        op("to_f32", Unary, |v| vec![Scalar::from_f32(v[0].to_f32())]),
        op("canonical_round_trip", Unary, |v| {
            vec![Scalar::from_canonical_bits(v[0].to_canonical_bits())]
        }),
        op("powi_extreme", Unary, |v| {
            [-1, 31, 64, 1000, -1074, 65_535, i32::MAX, i32::MIN].map(|n| v[0].powi(n)).to_vec()
        }),
        // Integer and narrow-float sources. The input word is read as an integer; non-NaN
        // patterns pass through `to_canonical_bits` unchanged.
        op("unit_from_bits", Bits, |v| vec![Scalar::unit_from_bits(v[0].to_canonical_bits())]),
        op("from_u64", Bits, |v| vec![Scalar::from_u64(v[0].to_canonical_bits())]),
        op("from_i64", Bits, |v| vec![Scalar::from_i64(v[0].to_canonical_bits() as i64)]),
        op("from_u32", Bits, |v| vec![Scalar::from_u32(v[0].to_canonical_bits() as u32)]),
        op("from_i32", Bits, |v| vec![Scalar::from_i32(v[0].to_canonical_bits() as i32)]),
        op("from_f32", Bits, |v| {
            let narrow = f32::from_bits(v[0].to_canonical_bits() as u32);
            vec![Scalar::from_f32(narrow), Scalar::from_f32(Scalar::from_f32(narrow).to_f32())]
        }),
        op("powi", Unary, |v| {
            vec![v[0].powi(0), v[0].powi(2), v[0].powi(3), v[0].powi(-3), v[0].powi(17)]
        }),
        op("quat_from_rotation", Unary, |v| {
            let mut out = Vec::new();
            out.extend(Quat::from_rotation_x(v[0]).to_array());
            out.extend(Quat::from_rotation_y(v[0]).to_array());
            out.extend(Quat::from_rotation_z(v[0]).to_array());
            out
        }),
        // Scalar, two operands.
        op("add", Binary, |v| vec![v[0] + v[1]]),
        op("sub", Binary, |v| vec![v[0] - v[1]]),
        op("mul", Binary, |v| vec![v[0] * v[1]]),
        op("div", Binary, |v| vec![v[0] / v[1]]),
        op("rem", Binary, |v| vec![v[0] % v[1]]),
        op("rem_euclid", Binary, |v| vec![v[0].rem_euclid(v[1])]),
        op("atan2", Binary, |v| vec![v[0].atan2(v[1])]),
        op("powf", Binary, |v| vec![v[0].powf(v[1])]),
        op("hypot", Binary, |v| vec![v[0].hypot(v[1])]),
        op("min", Binary, |v| vec![v[0].min(v[1])]),
        op("max", Binary, |v| vec![v[0].max(v[1])]),
        op("compare", Binary, |v| {
            vec![flag(v[0] < v[1]), flag(v[0] == v[1]), flag(v[0] > v[1]), flag(v[0] <= v[1])]
        }),
        // Scalar, three operands.
        op("mul_add", Ternary, |v| vec![v[0].mul_add(v[1], v[2])]),
        op("clamp", Ternary, |v| vec![v[0].clamp(v[1], v[2])]),
        op("lerp", Ternary, |v| vec![v[0].lerp(v[1], v[2])]),
        // Vectors.
        op("vec2_products", Tuple(4), |v| {
            let (a, b) = (vec2(v), vec2(&v[2..]));
            vec![a.dot(b), a.perp_dot(b), a.length(), a.distance(b)]
        }),
        op("vec2_normalize", Tuple(2), |v| vec2(v).normalize().to_array().to_vec()),
        op("vec2_lerp", Tuple(5), |v| vec2(v).lerp(vec2(&v[2..]), v[4]).to_array().to_vec()),
        op("vec3_dot_cross", Tuple(6), |v| {
            let (a, b) = (vec3(v), vec3(&v[3..]));
            let cross = a.cross(b);
            vec![a.dot(b), cross.x, cross.y, cross.z]
        }),
        op("vec3_length", Tuple(3), |v| vec![vec3(v).length_squared(), vec3(v).length()]),
        op("vec3_distance", Tuple(6), |v| vec![vec3(v).distance(vec3(&v[3..]))]),
        op("vec3_normalize", Tuple(3), |v| {
            let mut out = vec3(v).normalize().to_array().to_vec();
            out.push(flag(vec3(v).try_normalize().is_some()));
            out
        }),
        op("vec3_lerp", Tuple(7), |v| vec3(v).lerp(vec3(&v[3..]), v[6]).to_array().to_vec()),
        op("vec3_select", Tuple(9), |v| {
            let (a, b, c) = (vec3(v), vec3(&v[3..]), vec3(&v[6..]));
            let mut out = a.min(b).to_array().to_vec();
            out.extend(a.max(b).to_array());
            out.extend(a.clamp(b, c).to_array());
            out.extend(a.abs().to_array());
            out
        }),
        op("vec3_arithmetic", Tuple(7), |v| {
            let (a, b) = (vec3(v), vec3(&v[3..]));
            let mut out = (a + b).to_array().to_vec();
            out.extend((a - b).to_array());
            out.extend((a * b).to_array());
            out.extend((a * v[6]).to_array());
            out.extend((a / v[6]).to_array());
            out
        }),
        op("vec4_dot", Tuple(8), |v| vec![vec4(v).dot(vec4(&v[4..])), vec4(v).length()]),
        op("vec4_normalize", Tuple(4), |v| vec4(v).normalize().to_array().to_vec()),
        // Quaternions.
        op("quat_mul", Tuple(8), |v| (quat(v) * quat(&v[4..])).to_array().to_vec()),
        op("quat_rotate", Tuple(7), |v| (quat(v).normalize() * vec3(&v[4..])).to_array().to_vec()),
        op("quat_normalize", Tuple(4), |v| {
            let mut out = quat(v).normalize().to_array().to_vec();
            out.push(quat(v).length());
            out
        }),
        op("quat_inverse", Tuple(4), |v| {
            let mut out = quat(v).inverse().to_array().to_vec();
            out.extend(quat(v).conjugate().to_array());
            out
        }),
        op("quat_slerp", Tuple(9), |v| {
            let (a, b) = (quat(v).normalize(), quat(&v[4..]).normalize());
            a.slerp(b, v[8]).to_array().to_vec()
        }),
        op("quat_slerp_near", Tuple(9), |v| {
            // The end is a small perturbation of the start, to reach the linear branch.
            let a = quat(v).normalize();
            let nudge = quat(&v[4..]).normalize();
            let b = Quat::from_array([
                a.x + nudge.x * s(0.001),
                a.y + nudge.y * s(0.001),
                a.z + nudge.z * s(0.001),
                a.w + nudge.w * s(0.001),
            ])
            .normalize();
            a.slerp(b, v[8]).to_array().to_vec()
        }),
        op("quat_nlerp", Tuple(9), |v| {
            let (a, b) = (quat(v).normalize(), quat(&v[4..]).normalize());
            a.nlerp(b, v[8]).to_array().to_vec()
        }),
        op("quat_from_axis_angle", Tuple(4), |v| {
            Quat::from_axis_angle(vec3(v).normalize(), v[3]).to_array().to_vec()
        }),
        op("quat_to_mat3", Tuple(4), |v| Mat3::from_quat(quat(v).normalize()).to_array().to_vec()),
        op("quat_from_mat3", Tuple(4), |v| {
            Quat::from_mat3(Mat3::from_quat(quat(v).normalize())).to_array().to_vec()
        }),
        op("quat_from_mat3_raw", Tuple(9), |v| Quat::from_mat3(mat3(v)).to_array().to_vec()),
        op("quat_canonical_sign", Tuple(4), |v| {
            let mut out = quat(v).canonical_sign().to_array().to_vec();
            // With leading zeros, so the later coefficients decide.
            out.extend(Quat::new(v[0], v[1], v[2], Scalar::ZERO).canonical_sign().to_array());
            out.extend(
                Quat::new(-Scalar::ZERO, v[1], v[2], -Scalar::ZERO).canonical_sign().to_array(),
            );
            out
        }),
        // Matrices.
        op("mat3_mul", Tuple(18), |v| (mat3(v) * mat3(&v[9..])).to_array().to_vec()),
        op("mat3_mul_vec", Tuple(12), |v| (mat3(v) * vec3(&v[9..])).to_array().to_vec()),
        op("mat3_inverse", Tuple(9), |v| {
            let inverse = mat3(v).inverse();
            let mut out = vec![mat3(v).determinant(), flag(inverse.is_some())];
            out.extend(inverse.unwrap_or(Mat3::ZERO).to_array());
            out.extend(mat3(v).transpose().to_array());
            out
        }),
        op("mat4_mul", Tuple(32), |v| (mat4(v) * mat4(&v[16..])).to_array().to_vec()),
        op("mat4_mul_vec", Tuple(20), |v| (mat4(v) * vec4(&v[16..])).to_array().to_vec()),
        op("mat4_inverse", Tuple(16), |v| {
            let inverse = mat4(v).inverse();
            let mut out = vec![mat4(v).determinant(), flag(inverse.is_some())];
            out.extend(inverse.unwrap_or(Mat4::ZERO).to_array());
            out
        }),
        op("mat4_transform", Tuple(19), |v| {
            let mut out = mat4(v).transform_point3(vec3(&v[16..])).to_array().to_vec();
            out.extend(mat4(v).transform_vector3(vec3(&v[16..])).to_array());
            out
        }),
        op("mat4_from_scale_rotation_translation", Tuple(10), |v| {
            let t = transform(v);
            Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.translation)
                .to_array()
                .to_vec()
        }),
        // Transforms.
        op("transform_point", Tuple(13), |v| {
            let t = transform(v);
            let mut out = t.transform_point(vec3(&v[10..])).to_array().to_vec();
            out.extend(t.transform_vector(vec3(&v[10..])).to_array());
            out
        }),
        op("transform_compose_uniform", Tuple(20), |v| {
            let mut parent = transform(v);
            parent.scale = Vec3::splat(parent.scale.x);
            parent.compose_uniform(transform(&v[10..])).to_array().to_vec()
        }),
        // Reductions.
        op("exact_sum", List(8), |v| {
            let mut sum = ExactSum::<Scalar>::identity();
            for &item in v {
                sum.add(item);
            }
            vec![sum.finish()]
        }),
        op("exact_sum_vec3", List(2), |v| {
            let mut sum = ExactSum::<Vec3>::identity();
            for item in v.as_chunks::<3>().0 {
                sum.add(vec3(item));
            }
            sum.finish().to_array().to_vec()
        }),
        op("min_max_reduce", List(2), |v| {
            let (mut min, mut max) = (Min::<Scalar>::identity(), Max::<Scalar>::identity());
            for &item in v {
                min.add(item);
                max.add(item);
            }
            vec![min.finish().unwrap_or(Scalar::ZERO), max.finish().unwrap_or(Scalar::ZERO)]
        }),
        // Render space. The outputs are widened exactly, so they compare as scalars.
        op("render_from_sim", Finite(3), |v| {
            let position = RenderPosition::from_sim(vec3(v));
            let [x, y, z] = position.cell;
            let origins = [
                position.cell,
                [0; 3],
                [x.wrapping_sub(3), 7, z],
                [x.wrapping_add(1000), y.wrapping_sub(1000), -z],
                [i32::MIN, i32::MAX, 1],
            ];
            let mut out = Vec::new();
            out.extend(position.cell.map(Scalar::from_i32));
            out.extend(position.offset.map(Scalar::from_f32));
            for origin in origins {
                out.extend(position.relative_to(origin).map(Scalar::from_f32));
            }
            out
        }),
    ]
}

// Input generation. Everything is integer arithmetic on bit patterns, so the generated cases
// are the same on every target without relying on the code under test.

struct Rng(u64);

impl Rng {
    /// Seeds from the operation name, so adding an operation leaves the others' cases alone.
    fn for_operation(name: &str) -> Self {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in name.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(hash)
    }

    /// SplitMix64.
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// A finite value with a random sign and fraction and a binary exponent in
    /// `[low, high]`.
    fn with_exponent(&mut self, low: i64, high: i64) -> u64 {
        let exponent = (1023 + low + self.below((high - low + 1) as u64) as i64) as u64;
        let bits = self.next();
        (bits & 0x800f_ffff_ffff_ffff) | (exponent << 52)
    }

    /// Magnitude between about a thousandth and sixteen.
    fn moderate(&mut self) -> u64 {
        self.with_exponent(-10, 3)
    }

    /// Magnitude between about a sixteenth and sixteen thousand: world coordinates, speeds
    /// and timers.
    fn gameplay(&mut self) -> u64 {
        self.with_exponent(-4, 13)
    }

    /// Any of: moderate, widely scaled, a small multiple of one half, or any bit pattern at
    /// all (subnormals, infinities and NaNs with payloads included).
    fn any(&mut self) -> u64 {
        match self.below(4) {
            0 => self.moderate(),
            1 => self.with_exponent(-70, 70),
            2 => (Scalar::from_i32(self.below(41) as i32 - 20) * Scalar::HALF).to_canonical_bits(),
            _ => self.next(),
        }
    }
}

const NEGATIVE_NAN_WITH_PAYLOAD: u64 = 0xfff8_0000_0000_1234;
const SIGNALLING_NAN: u64 = 0x7ff0_0000_0000_0001;
const SMALLEST_SUBNORMAL: u64 = 0x0000_0000_0000_0001;
const LARGEST_SUBNORMAL: u64 = 0x000f_ffff_ffff_ffff;

fn unary_edges() -> Vec<u64> {
    let mut edges = vec![
        SMALLEST_SUBNORMAL,
        SMALLEST_SUBNORMAL | (1 << 63),
        LARGEST_SUBNORMAL,
        Scalar::NAN.to_canonical_bits(),
        NEGATIVE_NAN_WITH_PAYLOAD,
        SIGNALLING_NAN,
        0x3fef_ffff_ffff_ffff, // just below one
        0x3ff0_0000_0000_0001, // just above one
        0xffff_ffff_ffff_ffff, // unit_from_bits: the top of the range
        0x0000_0000_0000_07ff, // unit_from_bits: discarded low bits only
    ];
    let values = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        1.5,
        -1.5,
        2.0,
        2.5,
        -2.5,
        3.0,
        -8.0,
        27.0,
        1e-300,
        1e300,
        -1e300,
        1e22,
        1e10,
        1e5,
        9007199254740992.0,
        2147483647.5,
        -2147483648.5,
        4294967296.0,
        9.3e18,
        709.78,
        710.0,
        -745.2,
        1024.0,
        -1075.0,
        3.5e38,
        core::f64::consts::PI,
        -core::f64::consts::PI,
        core::f64::consts::FRAC_PI_2,
        core::f64::consts::FRAC_PI_4,
        core::f64::consts::E,
        1.2246467991473532e-16,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::MIN,
        f64::EPSILON,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    edges.extend(values.map(|value| s(value).to_canonical_bits()));
    edges
}

fn binary_edges() -> Vec<u64> {
    let mut edges = vec![NEGATIVE_NAN_WITH_PAYLOAD, SMALLEST_SUBNORMAL];
    let values = [0.0, -0.0, 1.0, -1.0, 0.5, -3.0, 1e300, f64::INFINITY, f64::NEG_INFINITY];
    edges.extend(values.map(|value| s(value).to_canonical_bits()));
    edges
}

fn ternary_edges() -> Vec<u64> {
    let mut edges = vec![NEGATIVE_NAN_WITH_PAYLOAD];
    let values = [0.0, -0.0, 1.0, f64::INFINITY];
    edges.extend(values.map(|value| s(value).to_canonical_bits()));
    edges
}

fn cases(operation: &Operation) -> Vec<Vec<u64>> {
    let mut rng = Rng::for_operation(operation.name);
    let mut cases = Vec::new();
    match operation.inputs {
        Inputs::Unary => {
            cases.extend(unary_edges().into_iter().map(|edge| vec![edge]));
            cases.extend((0..100).map(|_| vec![rng.moderate()]));
            cases.extend((0..100).map(|_| vec![rng.gameplay()]));
            cases.extend((0..40).map(|_| vec![rng.any()]));
        }
        Inputs::Bits => {
            cases.extend(unary_edges().into_iter().map(|edge| vec![edge]));
            cases.extend((0..96).map(|_| vec![rng.next()]));
            // Small integers, and values one either side of a rounding boundary.
            cases.extend((0..32).map(|_| vec![rng.below(1 << 20)]));
            for base in [1u64 << 53, 1 << 63, u64::MAX - (1 << 10), (1 << 63) - (1 << 9)] {
                cases.extend((0..4u64).map(|step| vec![base.wrapping_add(step * 511)]));
            }
        }
        Inputs::Binary => {
            let edges = binary_edges();
            for &a in &edges {
                cases.extend(edges.iter().map(|&b| vec![a, b]));
            }
            cases.extend((0..100).map(|_| vec![rng.moderate(), rng.moderate()]));
            cases.extend((0..100).map(|_| vec![rng.gameplay(), rng.gameplay()]));
            cases.extend((0..40).map(|_| vec![rng.any(), rng.any()]));
        }
        Inputs::Ternary => {
            let edges = ternary_edges();
            for &a in &edges {
                for &b in &edges {
                    cases.extend(edges.iter().map(|&c| vec![a, b, c]));
                }
            }
            cases.extend((0..60).map(|_| vec![rng.moderate(), rng.moderate(), rng.moderate()]));
            cases.extend((0..60).map(|_| vec![rng.gameplay(), rng.gameplay(), rng.gameplay()]));
            cases.extend((0..24).map(|_| vec![rng.any(), rng.any(), rng.any()]));
        }
        Inputs::Tuple(arity) => {
            cases.extend((0..8).map(|_| (0..arity).map(|_| rng.moderate()).collect()));
            cases.extend((0..3).map(|_| (0..arity).map(|_| rng.with_exponent(-70, 70)).collect()));
            cases.extend((0..3).map(|_| (0..arity).map(|_| rng.any()).collect()));
            cases.push(vec![0; arity]);
            cases.push(vec![1 << 63; arity]);
        }
        Inputs::Finite(arity) => {
            cases.extend((0..12).map(|_| (0..arity).map(|_| rng.moderate()).collect()));
            cases.extend((0..24).map(|_| (0..arity).map(|_| rng.with_exponent(-40, 30)).collect()));
            cases.push(vec![0; arity]);
            cases.push(vec![s(-32.0).to_canonical_bits(); arity]);
            cases.push(vec![s(31.999999999999996).to_canonical_bits(); arity]);
            cases.push(vec![s(1e12).to_canonical_bits(); arity]);
        }
        Inputs::List(rounds) => {
            let lengths = [0, 3, 6, 9, 30, 60, 120];
            for length in (0..rounds).flat_map(|_| lengths) {
                // Moderate values; widely scaled values; and values that cancel exactly.
                cases.push((0..length).map(|_| rng.moderate()).collect());
                cases.push((0..length).map(|_| rng.with_exponent(-1000, 1000)).collect());
                let mut cancelling: Vec<u64> =
                    (0..length / 3).map(|_| rng.with_exponent(-300, 300)).collect();
                let negated: Vec<u64> = cancelling.iter().map(|bits| bits ^ (1 << 63)).collect();
                cancelling.extend((0..length / 3).map(|_| rng.moderate()));
                cancelling.extend(negated);
                cases.push(cancelling);
                cases.push((0..length).map(|_| rng.any()).collect());
            }
            let max = Scalar::MAX.to_canonical_bits();
            cases.push(vec![max, max, max ^ (1 << 63)]);
            cases.push(vec![max, max, s(1.0).to_canonical_bits()]);
            cases.push(vec![SMALLEST_SUBNORMAL; 3]);
            cases.push(vec![1 << 63; 3]);
        }
    }
    cases
}

fn evaluate(operation: &Operation, inputs: &[u64]) -> Vec<u64> {
    // `f64::from_bits` keeps NaN sign and payload, unlike `Scalar::from_canonical_bits`.
    let scalars: Vec<Scalar> = inputs.iter().map(|&bits| s(f64::from_bits(bits))).collect();
    (operation.eval)(&scalars).into_iter().map(Scalar::to_canonical_bits).collect()
}

fn hex(values: &[u64]) -> String {
    let words: Vec<String> = values.iter().map(|value| format!("{value:016x}")).collect();
    words.join(",")
}

fn parse_hex(field: &str) -> Vec<u64> {
    if field.is_empty() {
        return Vec::new();
    }
    field.split(',').map(|word| u64::from_str_radix(word, 16).expect("hex word")).collect()
}

#[cfg(target_arch = "aarch64")]
fn table_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/vectors.txt")
}

const TABLE: &str = include_str!("golden/vectors.txt");

struct Line<'a> {
    name: &'a str,
    inputs: Vec<u64>,
    outputs: Vec<u64>,
}

fn table() -> Vec<Line<'static>> {
    TABLE
        .lines()
        .map(|line| {
            let mut fields = line.split('|');
            let name = fields.next().expect("operation name");
            let inputs = parse_hex(fields.next().expect("inputs"));
            let outputs = parse_hex(fields.next().expect("outputs"));
            Line { name, inputs, outputs }
        })
        .collect()
}

/// Every line of the table evaluates to its recorded bits on this target.
#[test]
fn golden_vectors_match() {
    let operations = operations();
    let table = table();
    assert!(table.len() > 5_000, "the table is the cross-architecture proof; it must be broad");

    let mut report = String::new();
    let mut mismatches = 0;
    for line in &table {
        let operation = operations
            .iter()
            .find(|operation| operation.name == line.name)
            .unwrap_or_else(|| panic!("the table names an unknown operation `{}`", line.name));
        let actual = evaluate(operation, &line.inputs);
        if actual != line.outputs {
            mismatches += 1;
            let decimal: Vec<f64> = line.inputs.iter().map(|&bits| f64::from_bits(bits)).collect();
            writeln!(
                report,
                "{}\n  inputs:   {}\n            {decimal:?}\n  expected: {}\n  actual:   {}",
                line.name,
                hex(&line.inputs),
                hex(&line.outputs),
                hex(&actual),
            )
            .unwrap();
        }
    }
    assert!(
        mismatches == 0,
        "{mismatches} of {} golden vectors differ on this target:\n{report}",
        table.len()
    );
}

/// The table holds exactly the cases the generator produces, so no operation or case can be
/// dropped from the proof without this failing.
#[test]
fn golden_table_is_current() {
    let table = table();
    let mut expected = Vec::new();
    for operation in operations() {
        for inputs in cases(&operation) {
            expected.push((operation.name, inputs));
        }
    }
    assert_eq!(table.len(), expected.len(), "case count differs; regenerate the table");
    for (line, (name, inputs)) in table.iter().zip(&expected) {
        assert!(
            line.name == *name && line.inputs == *inputs,
            "the table is stale at `{}|{}`; the generator has `{name}|{}`",
            line.name,
            hex(&line.inputs),
            hex(inputs),
        );
    }
}

/// Rewrites the table from this machine's results; review the diff. aarch64 is the reference
/// architecture, so the generator does not exist in a build for any other.
#[cfg(target_arch = "aarch64")]
#[test]
#[ignore = "rewrites tests/golden/vectors.txt"]
fn bless() {
    let mut text = String::new();
    for operation in operations() {
        for inputs in cases(&operation) {
            let outputs = evaluate(&operation, &inputs);
            writeln!(text, "{}|{}|{}", operation.name, hex(&inputs), hex(&outputs)).unwrap();
        }
    }
    std::fs::write(table_path(), text).unwrap();
}
