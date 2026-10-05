# Unsafe in the expansion of lactose-reflect-macros

This crate contains no `unsafe` and is `#![forbid(unsafe_code)]`. Its expansion, which lands
in every crate that declares a component, a resource or a `#[value]` struct, contains one
`unsafe impl`. This file is the argument for it. The `unsafe_code` lint does not apply to
code a proc macro generates, so the expansion compiles in a crate that forbids `unsafe`, as
the `bytemuck` derives do.

## The rule the argument follows

Nothing the macro parsed is evidence about the type. An attribute placed below ours runs
after it and can rewrite the struct, and an attribute can be spelled in ways a parser does
not recognise (`#[r#repr(align(16))]`). The macro's checks on attributes exist to give the
author a clear error. Soundness rests only on items in the expansion that the compiler
checks against the final type, and each claim below names the item that establishes it.

## `unsafe impl CheckedBitPattern` for a struct `T`

`bytemuck::CheckedBitPattern` requires that `T::Bits` has the layout of `T` and that
`is_valid_bit_pattern` returns true only for bit patterns that are valid values of `T`.

The expansion defines `__Bits`, a `#[repr(C)]` struct with one field `fN` per recorded field,
of type `<F as CheckedBitPattern>::Bits` for the recorded field type `F`, and proves:

1. `T` is a struct with exactly the recorded fields. `__same_fields` destructures `&T` with a
   struct pattern that names every recorded field and has no `..`, so it fails to compile
   if `T` is not a struct, lacks one of the fields, or has another.
2. Each field has exactly the recorded type. `__same_fields` passes each binding to
   `__type_of`, which infers the field's type `F` into a `PhantomData<F>` with no expected
   type in scope, and then assigns that value to a `PhantomData` of the recorded type. The
   assignment admits no coercion, so a retyped field fails to compile even if the new type
   dereferences to the old.
3. `T` and `__Bits` have the same size and alignment: a constant assertion compares
   `size_of` and `align_of`.
4. Every field of `T` is at the offset of its counterpart in `__Bits`: one constant
   assertion per field compares `offset_of!`.
5. Each `fN` has the size and alignment of its field, by the field type's own
   `CheckedBitPattern` contract.
6. `__Bits` is `AnyBitPattern`: `bytemuck`'s derive is applied to it, and it is defined by
   this expansion inside an anonymous constant, where no other attribute can reach it.
7. `T` has no padding: the `NoUninit` derive asserts that the size of `T` is the sum of the
   sizes of the recorded field types, which by 1 and 2 are its field types.

From 1 to 5, `__Bits` has the layout of `T` field for field. From 1, 2 and 7, a bit pattern
is a valid `T` exactly when the bytes of each field are a valid value of that field's type,
and `is_valid_bit_pattern` is the conjunction of the fields' own checks, which read each
`fN` at the offset of its field by 4.

The representation of `T` is not an assumption. The macro adds `#[repr(C)]`, but if anything
changes the representation the assertions of 3, 4 and 7 either still hold, in which case the
argument goes through, or fail to compile.

## `#[value]` enums

For a fieldless enum the expansion applies `bytemuck`'s `NoUninit` and `CheckedBitPattern`
derives and contains no `unsafe` of its own. Those derives decide padding and validity from
the tokens they were given, so the expansion holds them to the final type `E` with declared
width `uN`:

- a constant assertion that `E` has the size and alignment of `uN`, so it has no padding
  and its `Bits` type, the integer, has its layout;
- `__same_variants`, a match on `E` with one arm per recorded variant, each a unit pattern,
  and no wildcard, so it fails to compile if a variant was added, removed or given fields;
- one constant assertion per variant that `E::V as uN` equals the recorded discriminant.

Together these make the set of valid bit patterns of `E` the set the derive accepts.

## What is tested

`crates/reflect/tests/compile_fail.rs` compiles each of these as a case that must fail: raw
spellings of `repr` and `cfg_attr` on a struct and on an enum, and attribute macros below
ours that retype a field (to an unrelated type, and to one that dereferences to the
recorded type), raise the alignment of a struct or an enum, add a variant, and
swap discriminants. Its control case compiles every form of declaration in a crate with
`#![forbid(unsafe_code)]` that depends on `lactose-reflect` alone.
