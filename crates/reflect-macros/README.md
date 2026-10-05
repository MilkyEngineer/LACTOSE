# lactose-reflect-macros

The attribute macros of `lactose-reflect`: `#[component]`, `#[resource]` and `#[value]`.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | none in this crate, `#![forbid(unsafe_code)]`; its expansion contains one `unsafe impl`, whose proof is in `SAFETY.md` |

## Rules a caller should know

- Depend on `lactose-reflect` and use the macros through it. The expansion names
  `::lactose_reflect` and `::core` paths only, so the invoking crate needs no other
  dependency. A crate that re-exports the macros passes `crate = "path"`.
- This crate depends on no boundary crate, because a proc macro compiles for the host
  (architecture section 8.2). It therefore computes no key and no layout: the expansion calls
  `ComponentKey::from_name`, `size_of` and `offset_of!`, which the compiler evaluates for the
  target in the invoking crate.
- Errors the macro can see (a missing, repeated or retired field ID, a missing policy, a
  generic type, a conditional field) are reported on the offending tokens. Errors only the
  compiler can see (padding, a type that is not a field) are constant assertions and trait
  bounds in the expansion, worded for the author.

## The one `unsafe impl` in the expansion

For a struct the expansion implements `bytemuck::CheckedBitPattern` itself. `bytemuck`'s
derive cannot be used: on the pinned version it derives `AnyBitPattern` on a generated type
without passing on the crate path, so it compiles only where `bytemuck` is a direct
dependency. `SAFETY.md` gives the argument for the impl. It does not rest on anything the
macro parsed: the expansion carries constant assertions and a never-called function that
fail to compile unless the final type has the recorded fields, types, offsets, size and
alignment. `#[value]` enums keep `bytemuck`'s derives and get the equivalent checks.

The `unsafe_code` lint does not apply to code a proc macro generates, so the expansion
compiles in a crate with `#![forbid(unsafe_code)]`, as the `bytemuck` derives do.
`crates/reflect/tests/ui/control.rs` is compiled that way by the test suite.
