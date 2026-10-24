# Unsafe in lactose-core

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. The crate's tests run under Miri.

## `arena`

- A chunk is allocated with `std::alloc::alloc` and referred to only through the raw pointer
  kept in its `Chunk`. No reference to a whole chunk is ever formed, so handing out a new
  allocation never invalidates an earlier one.
- `used` never exceeds the last chunk's size, and an allocation is carved from
  `[start, end)` with `end <= size`.
- Returned borrows are tied to `&self`. `reset` and `drop` take `&mut self`, so no borrow
  survives them.
- `Arena` is `Send` and not `Sync`: it owns its memory outright and its bookkeeping is
  unsynchronised.

## `alloc`

- The inner allocator always sees the caller's layout extended by one trailing byte. The
  returned pointer is the inner allocator's own, so it keeps the requested alignment.
- The tag is the byte at `ptr + layout.size()`. `dealloc` and `realloc` read it and rebuild
  the extended layout from the same `layout` the caller passes back.
- `realloc` validates the new extended layout before calling the inner `realloc`, reads the
  tag first because a shrink may discard it, and rewrites it at the new end.
- The current-tag thread-local is const-initialised and has no destructor, so reading it
  inside the allocator cannot allocate or fail during thread teardown.
- A test binary that installs `TaggedAlloc` as its global allocator runs under Miri only
  with `-Zmiri-tree-borrows`: `dealloc` and `realloc` read the tag trailer through the
  caller's pointer, outside the range of its pointee, which Stacked Borrows forbids and Tree
  Borrows allows.

## `id`

- `NetId` is `repr(transparent)` over `NonZeroU64`, which is what the `ZeroableInOption` and
  `PodInOption` impls assert.
- `EntityRef` is four `u32` words. Every bit pattern is memory-valid; `CheckedBitPattern`
  only narrows to canonical encodings, and `EntityRef::get` treats anything else as empty.

## `array`

- `PlainArray<T, N>` and `PlainArrayBits<T, N>` are `repr(transparent)` over `[T; N]` and
  `[T::Bits; N]`. An array has no padding of its own, so each is plain data exactly when its
  element is, and the array is valid exactly when every element is.
- That `PlainArrayBits<T, N>` has the layout of `PlainArray<T, N>` relies on `T::Bits` having
  the size and alignment of `T`, which is `T`'s own `CheckedBitPattern` contract.

## `typed_plain_data!`

- The macro expands to `unsafe impl`s of `Zeroable` and `Pod` in the calling crate, which may
  forbid `unsafe` itself; this file and the comments in `typed.rs` are their justification.
- The macro writes the struct as well as the impls: `repr(transparent)` over the named inner
  type plus a `PhantomData<fn() -> T>`. Its grammar accepts doc comments, a visibility, a
  name, one parameter and one inner type, and nothing else: no attribute (so no `repr`, also
  not through `cfg_attr`), no second field. A caller cannot change the representation or
  supply the impls for a struct of its own, so the layout argument holds for every
  expansion. `compile_fail` doctests on the macro pin each refusal; they run under
  `cargo test --doc`, not under nextest.
- The impls are bounded on the inner type being `Zeroable` and `Pod`. The marker is zero-sized
  with alignment one and owns no `T`, so the struct has the inner type's size, alignment and
  bit validity for every `T: 'static`.

## `thread_class::fp`

- The inline assembly reads or writes MXCSR (x86-64) or FPCR (aarch64) and nothing else.
- `set_deterministic` only ever moves the register towards the IEEE default state that Rust
  assumes. The one function that leaves that state is test-only, and its test restores the
  default before running any floating-point code.

## `thread_class::os`

- Each call (`pthread_set_qos_class_self_np`, `setpriority`, `SetThreadPriority`) takes plain
  values and affects only the calling thread.
