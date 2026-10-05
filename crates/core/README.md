# lactose-core

The vocabulary every other crate shares.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | yes, see `SAFETY.md` |

## What it holds

| Module | Contents |
|---|---|
| `id` | `EntityId`, `NetId`, `PredictionKey`, `StableKey`, `EntityRef`, `PlayerId`, `Tick`, `SystemKey`, `ComponentKey` (architecture section 7) |
| `hash` | `xxh64` as a `const fn`, so name-derived keys are computed at compile time |
| `map` | `FixedHashMap`: fixed hasher, no observable iteration order |
| `handle` | `Handle<T>` and `HandleTable<T>` |
| `arena` | `Arena`, the bump allocator behind per-job scratch memory and scope job storage |
| `alloc` | `TaggedAlloc`, `AllocTag` and `TagScope`: live memory accounted per subsystem |
| `error` | `Error`, `ErrorKind` and `Result` (architecture section 19). Each `ErrorKind` has a stable `u8` code for journals, saves and the wire |
| `thread_class` | `ThreadClass::enter` and `fp`: scheduling class and floating-point state for any thread |
| `typed_plain_data!` | declares a plain-data newtype with a compile-time-only type parameter, so a crate that forbids `unsafe` can define a typed reference field such as `AssetRef<T>` |

## Rules a caller should know

- `EntityId` has no ordering on purpose. Anything stored, ordered, sent or used to key
  randomness is a `StableKey`.
- A component field that refers to an entity is an `EntityRef`, the padding-free form of
  `Option<StableKey>`. Its all-zero value is the empty reference.
- `FixedHashMap` can only be traversed through `sorted()`.
- `Arena` never drops what it holds, so its typed methods take `Copy` types.
- Binaries install `TaggedAlloc` as the global allocator. A subsystem charges its allocations
  by holding `AllocTag::X.scope()`. The executor carries the tag across threads.
- Any thread that runs engine work calls `ThreadClass::enter` first. That includes threads the
  executor does not own.
