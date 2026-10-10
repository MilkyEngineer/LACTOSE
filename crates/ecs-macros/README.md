# lactose-ecs-macros

The attribute macros of `lactose-ecs`: `#[system]` and `#[finalize]`.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside; carries the boundary `clippy.toml` because its expansion compiles inside boundary crates |
| Unsafe | none, `#![forbid(unsafe_code)]`; the expansion calls two `unsafe` constructors of `lactose-ecs`, whose proof is in `crates/ecs/SAFETY.md` |

`crates/ecs/README.md`, under "The `#[system]` attribute", is the specification of what each
attribute accepts and emits, and lists the hidden items the expansion names.

## Rules a caller should know

- Depend on `lactose-ecs` and use the attributes through it. The expansion names
  `::lactose_ecs` and `::core` paths only.
- This crate depends on no boundary crate, because a proc macro compiles for the host
  (architecture section 8.2). The expansion computes no key: it writes
  `SystemDescriptor::new(name, ..)`, and `SystemKey::from_name` runs in the invoking crate.
- Errors the macro can see (a missing `name`, an unknown world, `exclusive` without
  `&mut World` first, `&mut World` without `exclusive`, `reduce` on a function that returns
  nothing, a return value without `reduce`, a generic or `async` function) are reported on
  the offending tokens. Errors only the compiler can see are trait bounds and name lookups
  in the expansion: a parameter type that is not a `SystemParam` is reported at the type
  as "`T` is not a system parameter", a missing finalize as "cannot find value
  `__finalize_f`".

## What the expansion is

For `#[system(..)] fn f(p0: T0, p1: T1)`:

- `f` itself, with every identifier binding made `mut` and `#[allow(unused_mut)]` added.
- `pub static F: ::lactose_ecs::SystemDescriptor`, documented, whose initializer holds the
  access table `[<T0 as SystemParam>::ACCESS, ..]` (each entry spanned at its type), a
  static of the `after` descriptors, the entry point
  `fn __run(invocation) { f(fetch(invocation, 0), fetch(invocation, 1)) }`, for a reduction
  `reduce::<R>(invocation, f(..))` and an entry point that finishes the reducer and calls
  `__finalize_f`, and one `unsafe { SystemDescriptor::new(..) }` spanned at the call site.

For `#[finalize(f)] fn g(out: O, p1: T1)`: `g` itself, as above, and
`const __finalize_f: Finalize<O>` with `g`'s visibility, holding the table of `p1..` and an
entry point that calls `g(out, fetch(invocation, 0), ..)`, built by one
`unsafe { Finalize::new(..) }` spanned at the call site.

`crates/ecs/tests/compile_fail.rs` checks every error above and that a crate with
`#![forbid(unsafe_code)]` and `#![deny(warnings)]` compiles every kind of system.
