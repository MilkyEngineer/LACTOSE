# lactose-reflect

What the engine knows about every component and resource type.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | none in this crate, `#![forbid(unsafe_code)]`; the attributes it re-exports expand to one `unsafe impl` per struct, whose proof is in `crates/reflect-macros/SAFETY.md` |

## What it holds

| Item | Contents |
|---|---|
| `#[component]`, `#[resource]`, `#[value]` | the attributes that declare reflected types, re-exported from `lactose-reflect-macros` |
| `Reflect`, `Component`, `Resource` | the key, the schema and the two visitor entry points of a registered type |
| `Field`, `ScalarField`, `AssetReference`, `FieldKind` | the types allowed as fields, the kind and type tag each reports |
| `FieldVisitor`, `ReferenceVisitor` | statically dispatched visitors: one reads every field by kind, one rewrites references in place |
| `Schema<T>`, `TypeInfo`, `StructInfo`, `FieldInfo`, `EnumInfo` | descriptions built by the compiler: name, key, policy, version, tracking, retired IDs, and each field's ID, name, type tag, offset, size and kind |
| `TypeInfo::field_bytes`, `write_field`, `validate`, `zero_padding`, `decode` | access to a value's bytes by field ID, with validation that returns an error |
| `TypeInfo::write_schema` | the type's schema in one canonical byte encoding, what a protocol hash covers; the documentation states it byte for byte and `tests/schema_bytes.rs` pins a golden encoding |
| `Registry` | explicit registration, lookup by key and by Rust type, sorted iteration, a text dump of the schema |
| `Padding<N>` | explicit padding whose only value is zero |

## Rules a caller should know

- A component is a struct declared with `#[component(key = "...", <policy>)]`, written above
  any `#[derive]`. The policy (`persistent`, `transient` or `derived`) has no default.
- Every field has `#[field(id = N)]`. An ID is never reused: deleting a field moves its ID
  into `retired = [...]`, and the attribute and the registry both refuse a field that uses a
  retired ID.
- The attribute does not reorder fields. Layout is `#[repr(C)]` in declaration order, and
  implicit padding is a compile error that names the field. Order fields from the largest
  alignment down and close any gap with `#[field(padding)] _pad: Padding<N>`.
- Arrays are `lactose_core::PlainArray<T, N>`, never a bare `[T; N]`.
- Inside the determinism boundary a float is the `lactose_math` scalar, a `ScalarField`,
  which visitors receive through `FieldVisitor::scalar` with canonical bits. Raw `f32` and
  `f64` are fields of kind `Float` for presentation components and arrive through their own
  visitor methods; the world hasher refuses them.
- Components cannot be generic or data-carrying enums. An enum is a fieldless `#[value]`
  type held in a field.
- `TypeInfo::validate` and `decode` are the proof that bytes are a valid value: they use the
  type's own `CheckedBitPattern` and nothing a description states. `write_field` and
  `zero_padding` are decoding steps that follow the description; validate after them. The
  documentation of `TypeInfo::validate` states exactly what code with `unsafe` may rely on,
  and it applies only to a description taken from `T::SCHEMA` or looked up in the registry.
- `FieldKind::Scalar` and `FieldKind::AssetRef` mean that some crate implemented
  `ScalarField` or `AssetReference` for the type. Both traits are safe, so the world hasher
  chooses the scalar types it accepts by naming them and does not treat the kind as proof
  of canonical floats.
- This crate never hashes, copies or remaps. `lactose-sim` implements those as visitors, so
  nothing here is on a determinism path (architecture section 8.1).
- A crate registers its types in a function it writes, `registry.register::<T>()?` per type.
  There is no global registry.

## The schema dump

`Registry::dump` is the schema as text: every type with its key, version and policy, every
field with its ID, name, type tag and size, and every retired ID. Type tags distinguish
types that share a layout (`u64`, `NetId`, `Option<NetId>`). A CI check that compares the
dumps of two revisions can therefore decide the rule "an ID present in the old dump is
either still present with the same type tag or listed as retired", which is what rejecting
field-ID reuse means. That check is not part of this crate.

## Not here yet

The serde bridge the architecture lists under `reflect` arrives in milestone 1 with
`lactose-persist`, its first user. This crate has no serde dependency.
