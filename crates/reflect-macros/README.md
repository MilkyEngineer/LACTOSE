# lactose-reflect-macros

The attribute macros of `lactose-reflect`: `#[component]`, `#[resource]` and `#[value]`.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

## Rules a caller should know

- Depend on `lactose-reflect` and use the macros through it. The expansion names
  `::lactose_reflect` and `::core` paths only.
- The expansion contains no `unsafe`, so it compiles in a crate that forbids it. The plain-data
  impls come from the `bytemuck` derives the expansion applies.
- This crate depends on no boundary crate, because a proc macro compiles for the host
  (architecture section 8.2). It therefore computes no key and no layout: the expansion calls
  `ComponentKey::from_name`, `size_of` and `offset_of!`, which the compiler evaluates for the
  target in the invoking crate.
- Errors the macro can see (a missing or repeated field ID, a missing policy, a generic type)
  are reported on the offending tokens. Errors only the compiler can see (padding, a type
  that is not a field) are constant assertions and trait bounds in the expansion, worded for
  the author.
