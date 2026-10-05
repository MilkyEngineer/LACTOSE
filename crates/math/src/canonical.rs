//! Canonical bytes: the one sanctioned route from math values to hashes and serialised data.

use crate::{Mat3, Mat4, Quat, Scalar, ScalarBits, Transform, Vec2, Vec3, Vec4};

/// A value with one byte encoding per observable state.
///
/// The encoding is the value's scalars in declaration order, each as its
/// [canonical bits](Scalar::to_canonical_bits) in little-endian. Every NaN encodes to the same
/// bytes; negative and positive zero encode differently because they are different states.
/// The world hash, saves and replication encode through this trait and never through a
/// `bytemuck` byte view, whose NaN bytes differ between architectures.
///
/// On a little-endian target the canonical bytes of a value holding no NaN equal its memory
/// bytes, so a bulk copy of a column is canonical once [`canonical`](Self::canonical) has been
/// applied to its rows.
pub trait Canonical: Copy {
    /// The encoding, a byte array `[u8; N]`.
    type Bytes: Copy + AsRef<[u8]> + for<'a> TryFrom<&'a [u8]>;

    /// The value with every NaN replaced by the canonical NaN. Idempotent; no other scalar
    /// changes.
    #[must_use]
    fn canonical(self) -> Self;

    /// Encodes the value.
    #[must_use]
    fn to_canonical_bytes(self) -> Self::Bytes;

    /// Decodes a value. Total: every byte string of the right length is a value, and a NaN in
    /// any encoding decodes to the canonical NaN.
    #[must_use]
    fn from_canonical_bytes(bytes: Self::Bytes) -> Self;
}

impl Canonical for Scalar {
    type Bytes = [u8; Scalar::BYTES];

    fn canonical(self) -> Self {
        Scalar::canonical(self)
    }

    fn to_canonical_bytes(self) -> Self::Bytes {
        self.to_canonical_bits().to_le_bytes()
    }

    fn from_canonical_bytes(bytes: Self::Bytes) -> Self {
        Self::from_canonical_bits(ScalarBits::from_le_bytes(bytes))
    }
}

macro_rules! canonical_by_scalars {
    ($($ty:ty => $count:literal),* $(,)?) => {$(
        impl Canonical for $ty {
            type Bytes = [u8; $count * Scalar::BYTES];

            fn canonical(self) -> Self {
                Self::from_array(self.to_array().map(Scalar::canonical))
            }

            fn to_canonical_bytes(self) -> Self::Bytes {
                let mut bytes = [0; $count * Scalar::BYTES];
                for (chunk, scalar) in bytes.chunks_exact_mut(Scalar::BYTES).zip(self.to_array()) {
                    chunk.copy_from_slice(&scalar.to_canonical_bytes());
                }
                bytes
            }

            fn from_canonical_bytes(bytes: Self::Bytes) -> Self {
                let mut scalars = [Scalar::ZERO; $count];
                for (scalar, chunk) in scalars.iter_mut().zip(bytes.chunks_exact(Scalar::BYTES)) {
                    let mut one = [0; Scalar::BYTES];
                    one.copy_from_slice(chunk);
                    *scalar = Scalar::from_canonical_bytes(one);
                }
                Self::from_array(scalars)
            }
        }
    )*};
}

canonical_by_scalars! {
    Vec2 => 2,
    Vec3 => 3,
    Vec4 => 4,
    Quat => 4,
    Mat3 => 9,
    Mat4 => 16,
    Transform => 10,
}
