//! A fixed array that is plain data whenever its element is.
//!
//! `bytemuck` implements `NoUninit` and `CheckedBitPattern` for `[T; N]` only when `T` accepts
//! every bit pattern. An array of references, enums or `bool` is therefore not plain data on
//! its own, and no crate outside this one may write the `unsafe impl` that says it is.

use core::ops::{Deref, DerefMut};

use bytemuck::{AnyBitPattern, CheckedBitPattern, NoUninit, Zeroable};

/// `[T; N]` as a component field for any plain-data `T`, including element types with invalid
/// bit patterns such as [`EntityRef`](crate::EntityRef), `bool` and fieldless enums. It
/// dereferences to the array, so it indexes and iterates like one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct PlainArray<T, const N: usize>(pub [T; N]);

impl<T: Default + Copy, const N: usize> Default for PlainArray<T, N> {
    fn default() -> Self {
        Self([T::default(); N])
    }
}

impl<T, const N: usize> Deref for PlainArray<T, N> {
    type Target = [T; N];

    fn deref(&self) -> &[T; N] {
        &self.0
    }
}

impl<T, const N: usize> DerefMut for PlainArray<T, N> {
    fn deref_mut(&mut self) -> &mut [T; N] {
        &mut self.0
    }
}

impl<T, const N: usize> From<[T; N]> for PlainArray<T, N> {
    fn from(elements: [T; N]) -> Self {
        Self(elements)
    }
}

// SAFETY: `PlainArray` is `repr(transparent)` over `[T; N]`. An array places its elements
// back to back with no padding between or after them, so it has an uninitialised byte only
// if `T` has one, and `T: NoUninit` has none.
unsafe impl<T: NoUninit, const N: usize> NoUninit for PlainArray<T, N> {}

/// The any-bit-pattern form of a [`PlainArray`], which [`CheckedBitPattern`] validates.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PlainArrayBits<T: CheckedBitPattern, const N: usize>([T::Bits; N]);

// `bytemuck`'s derive formats the `Bits` of every field, and `T::Bits` need not be `Debug`.
impl<T: CheckedBitPattern, const N: usize> core::fmt::Debug for PlainArrayBits<T, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PlainArrayBits").finish_non_exhaustive()
    }
}

// SAFETY: `repr(transparent)` over an array of `T::Bits`, and `T::Bits: AnyBitPattern` is
// valid when all its bytes are zero.
unsafe impl<T: CheckedBitPattern, const N: usize> Zeroable for PlainArrayBits<T, N> {}
// SAFETY: as above; every bit pattern of each `T::Bits` element is valid, so every bit
// pattern of the array is.
unsafe impl<T: CheckedBitPattern + 'static, const N: usize> AnyBitPattern for PlainArrayBits<T, N> {}

// SAFETY: `T::Bits` has the layout of `T`, so `PlainArrayBits<T, N>` has the layout of
// `PlainArray<T, N>`. The array is valid exactly when every element is.
unsafe impl<T: CheckedBitPattern + 'static, const N: usize> CheckedBitPattern for PlainArray<T, N> {
    type Bits = PlainArrayBits<T, N>;

    fn is_valid_bit_pattern(bits: &Self::Bits) -> bool {
        bits.0.iter().all(T::is_valid_bit_pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityRef, NetId, StableKey};

    #[test]
    fn layout_matches_the_plain_array() {
        assert_eq!(size_of::<PlainArray<EntityRef, 3>>(), size_of::<[EntityRef; 3]>());
        assert_eq!(align_of::<PlainArray<EntityRef, 3>>(), align_of::<[EntityRef; 3]>());
        assert_eq!(size_of::<PlainArray<bool, 0>>(), 0);
    }

    #[test]
    fn behaves_like_the_array_it_wraps() {
        let mut array = PlainArray::from([1u8, 2, 3]);
        array[1] = 7;
        assert_eq!(array.iter().sum::<u8>(), 11);
        assert_eq!(array.len(), 3);
        assert_eq!(PlainArray::<bool, 2>::default(), PlainArray([false; 2]));
    }

    #[test]
    fn bytes_round_trip_through_an_unaligned_buffer() {
        let target = EntityRef::new(StableKey::Net(NetId::cooked(7)));
        let array = PlainArray([EntityRef::NONE, target]);
        let mut buffer = vec![0u8; 1];
        buffer.extend_from_slice(bytemuck::bytes_of(&array));
        let back: PlainArray<EntityRef, 2> =
            bytemuck::checked::try_pod_read_unaligned(&buffer[1..]).unwrap();
        assert_eq!(back, array);
    }

    #[test]
    fn one_invalid_element_rejects_the_array() {
        assert!(
            bytemuck::checked::try_pod_read_unaligned::<PlainArray<bool, 3>>(&[1, 0, 1]).is_ok()
        );
        assert!(
            bytemuck::checked::try_pod_read_unaligned::<PlainArray<bool, 3>>(&[1, 2, 1]).is_err()
        );
        // A placeholder reference is never valid outside a command buffer.
        let placeholder = PlainArray([EntityRef::new(StableKey::Placeholder(1))]);
        let bytes = bytemuck::bytes_of(&placeholder);
        assert!(
            bytemuck::checked::try_pod_read_unaligned::<PlainArray<EntityRef, 1>>(bytes).is_err()
        );
    }
}
