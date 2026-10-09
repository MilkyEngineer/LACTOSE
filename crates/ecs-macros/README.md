# lactose-ecs-macros

The attribute macros of `lactose-ecs`: `#[system]` and `#[finalize]`.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside; carries the boundary `clippy.toml` because its expansion compiles inside boundary crates |
| Unsafe | none, `#![forbid(unsafe_code)]`; the expansion contains none either |

**Status: facade.** Both attributes exist and expand to a compile error naming this status.
`crates/ecs/README.md`, under "The `#[system]` attribute", is the specification of what each
accepts and emits; the implementation lane writes the expansion against it.

## Rules a caller should know

- Depend on `lactose-ecs` and use the attributes through it. The expansion names
  `::lactose_ecs` and `::core` paths only.
- This crate depends on no boundary crate, because a proc macro compiles for the host
  (architecture section 8.2). The expansion computes no key: it writes
  `SystemDescriptor::new(name, ..)`, and `SystemKey::from_name` runs in the invoking crate.
- Errors the macro can see (a missing `name`, an unknown world, `exclusive` without
  `&mut World`, `reduce` on a function that returns nothing) are reported on the offending
  tokens. Errors only the compiler can see (a parameter type that is not a `SystemParam`, a
  missing finalize function) are trait bounds and name lookups in the expansion, worded for
  the author where the expansion can word them.
