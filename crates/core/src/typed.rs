//! Plain-data newtypes that carry a compile-time-only type parameter.

/// Declares a plain-data newtype over `$inner` with a type parameter that exists only at
/// compile time, such as a typed reference field.
///
/// `bytemuck`'s derives accept a generic struct only when its parameters are themselves
/// plain data, which a marker parameter never is. This macro writes the struct itself, so
/// the layout the `unsafe impl`s rely on holds for every invocation, and the declaring crate
/// stays `#![forbid(unsafe_code)]`.
///
/// The struct has two private fields, `value: $inner` and `marker: PhantomData<fn() -> T>`,
/// and is `Copy`, [`Zeroable`](bytemuck::Zeroable) and [`Pod`](bytemuck::Pod) for every
/// `T: 'static`. `$inner` must be `Pod`; anything else fails to compile.
///
/// ```
/// lactose_core::typed_plain_data! {
///     /// A typed slot number.
///     pub struct Slot<T>(u32);
/// }
/// struct Mesh;
/// let slot: Slot<Mesh> = bytemuck::cast(7u32);
/// assert_eq!(bytemuck::bytes_of(&slot), &7u32.to_ne_bytes());
/// ```
///
/// # What the macro refuses
///
/// The safety argument rests on the caller being unable to change the struct. Only doc
/// comments are accepted in front of it, so no `repr`, directly or through `cfg_attr`, can
/// reach it:
///
/// ```compile_fail
/// lactose_core::typed_plain_data! {
///     #[repr(C)]
///     pub struct Slot<T>(u32);
/// }
/// ```
///
/// ```compile_fail
/// lactose_core::typed_plain_data! {
///     #[cfg_attr(all(), repr(align(64)))]
///     pub struct Slot<T>(u8);
/// }
/// ```
///
/// There is exactly one inner field:
///
/// ```compile_fail
/// lactose_core::typed_plain_data! {
///     pub struct Slot<T>(u32, u8);
/// }
/// ```
///
/// An inner type that is not `Pod` is rejected where the struct is declared:
///
/// ```compile_fail
/// lactose_core::typed_plain_data! {
///     pub struct Name<T>(String);
/// }
/// ```
///
/// The parameter is a marker and cannot be the inner type, because the struct is `Copy`
/// for every parameter:
///
/// ```compile_fail
/// lactose_core::typed_plain_data! {
///     pub struct Wrapper<T>(T);
/// }
/// ```
#[macro_export]
macro_rules! typed_plain_data {
    ($(#[doc = $doc:expr])* $vis:vis struct $name:ident<$param:ident>($inner:ty);) => {
        $(#[doc = $doc])*
        #[repr(transparent)]
        $vis struct $name<$param> {
            value: $inner,
            marker: ::core::marker::PhantomData<fn() -> $param>,
        }

        impl<$param> ::core::clone::Clone for $name<$param> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<$param> ::core::marker::Copy for $name<$param> {}

        // SAFETY: the struct is `repr(transparent)` over `$inner`, and its only other field
        // is a zero-sized `PhantomData` of alignment one, so it has exactly the layout and
        // bit validity of `$inner`, which the bound requires to be `Zeroable`.
        unsafe impl<$param: 'static> $crate::__bytemuck::Zeroable for $name<$param> where
            $inner: $crate::__bytemuck::Zeroable
        {
        }

        // SAFETY: as above, with `$inner: Pod`. The struct is `Copy` and `'static`, has no
        // padding and no invalid bit pattern because `$inner` has none, and the marker
        // owns no `$param`.
        unsafe impl<$param: 'static> $crate::__bytemuck::Pod for $name<$param> where
            $inner: $crate::__bytemuck::Pod
        {
        }
    };
}

#[cfg(test)]
mod tests {
    typed_plain_data! {
        /// A key typed by what it names.
        struct Key<T>(u64);
    }

    // Deliberately not plain data: the parameter is never stored.
    struct Named(#[expect(dead_code, reason = "only the type matters")] String);

    #[test]
    fn typed_newtype_is_plain_data_for_any_parameter() {
        let key = Key::<Named> { value: 0x0102_0304_0506_0708, marker: core::marker::PhantomData };
        assert_eq!(size_of::<Key<Named>>(), size_of::<u64>());
        assert_eq!(align_of::<Key<Named>>(), align_of::<u64>());
        assert_eq!(bytemuck::cast::<Key<Named>, u64>(key), 0x0102_0304_0506_0708);
        let zero: Key<Named> = bytemuck::Zeroable::zeroed();
        assert_eq!(zero.value, 0);
        let back: Key<Named> = bytemuck::pod_read_unaligned(&9u64.to_ne_bytes());
        assert_eq!(back.value, 9);
        fn assert_plain<
            T: Copy + Send + Sync + bytemuck::NoUninit + bytemuck::CheckedBitPattern,
        >() {
        }
        assert_plain::<Key<Named>>();
    }
}
