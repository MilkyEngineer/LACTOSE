# lactose-reflect

What the engine knows about every component and resource type.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

## What it holds

| Item | Contents |
|---|---|
| `#[component]`, `#[resource]`, `#[value]` | the attributes that declare reflected types, re-exported from `lactose-reflect-macros` |
| `Reflect`, `Component`, `Resource` | the key, the description and the two visitor entry points of a registered type |
| `Field`, `FieldKind` | the types allowed as fields and the kind each reports |
| `FieldVisitor`, `ReferenceVisitor` | statically dispatched visitors: one reads every field by kind, one rewrites references in place |
| `TypeInfo`, `StructInfo`, `FieldInfo`, `EnumInfo` | descriptions built by the compiler: name, key, policy, version, tracking, and each field's ID, name, offset, size and kind |
| `TypeInfo::field_bytes`, `write_field`, `validate`, `zero_padding`, `decode` | access to a value's bytes by field ID, with validation that returns an error |
| `Registry` | explicit registration, lookup by key and by Rust type, sorted iteration, a text dump of the schema |
| `Padding<N>` | explicit padding whose only value is zero |

## Rules a caller should know

- A component is a struct declared with `#[component(key = "...", <policy>)]`. The policy
  (`persistent`, `transient` or `derived`) has no default. Every field has
  `#[field(id = N)]`; an ID is never reused, including after the field is deleted.
- The attribute does not reorder fields. Layout is `#[repr(C)]` in declaration order, and
  implicit padding is a compile error that names the field. Order fields from the largest
  alignment down and close any gap with `#[field(padding)] _pad: Padding<N>`.
- Arrays are `lactose_core::PlainArray<T, N>`, never a bare `[T; N]`.
- Raw `f32` and `f64` are fields of kind `Float`, for presentation components. Inside the
  determinism boundary floats are `lactose_math` types, whose kind is `Scalar`; a boundary
  crate that finds `Float` in a simulation component's description should refuse it.
- Components cannot be generic or data-carrying enums. An enum is a fieldless `#[value]`
  type held in a field.
- This crate never hashes, copies or remaps. `lactose-sim` implements those as visitors, so
  nothing here is on a determinism path (architecture section 8.1).
- A crate registers its types in a function it writes, `registry.register::<T>()?` per type.
  There is no global registry. Registration checks the description against the type.
- `Registry::dump` is the schema as text. A CI check that diffs it between revisions is what
  rejects field-ID reuse; that check is not part of this crate.

## Not here yet

The serde bridge the architecture lists under `reflect` arrives in milestone 1 with
`lactose-persist`, its first user. This crate has no serde dependency.
