# lactose-io safety

The library contains no `unsafe` and forbids it: `#![forbid(unsafe_code)]` is the first
attribute of `src/lib.rs`, and of every test crate root but one. Architecture section 21
lists `io` for "platform completion APIs", which are the io_uring and IOCP backends of
section 4, and neither is written yet. The reference backend, the blocking pool, is safe
code over `std::fs` and `std::os::unix::fs::FileExt` or `std::os::windows::fs::FileExt`.

## The tests' allocator

`tests/requests.rs` denies `unsafe_code` and allows it in one module, `largest`: an
`unsafe impl GlobalAlloc` under the tagged allocator that records the largest block asked
for, so a test can show a request above the cap allocates nothing near it. Each of its
four methods records a size in an atomic and forwards its arguments unchanged to
`std::alloc::System`, returning what `System` returns. The invariant: every block this
allocator hands out or takes back is `System`'s, with the very layout the caller gave, so
`System`'s guarantees are its own and the caller's obligations are passed to `System`
intact. Recording a size reads the layout and touches no block. Its own test runs under
Miri with `MIRIFLAGS=-Zmiri-tree-borrows cargo +nightly miri test -p lactose-io --test
requests -- the_tests_allocator`; the other tests there call into zstd's C library and the
file system, which Miri does not run. Tree Borrows is needed for `lactose_core::TaggedAlloc`
above it, whatever the allocator under it (`crates/core/SAFETY.md`, `alloc`).

When a completion backend lands, it lives in its own module under `src/backend/` and the
crate-level `forbid` becomes `#![deny(unsafe_code)]` with `#![allow(unsafe_code)]` on that
module alone, so the reference backend and the public surface stay provably safe. Every
`unsafe` block carries a `// SAFETY:` comment, and this file then carries the invariants
they rely on: that a submitted buffer stays allocated and unmoved until the kernel reports
the operation complete, however the request's future was dropped, and that a completion is
matched to its request before any byte of the buffer is read.
