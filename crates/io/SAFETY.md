# lactose-io safety

This crate contains no `unsafe` and forbids it: `#![forbid(unsafe_code)]` is the first
attribute of every crate root. Architecture section 21 lists `io` for "platform completion
APIs", which are the io_uring and IOCP backends of section 4, and neither is written yet.
The reference backend, the blocking pool, is safe code over `std::fs` and
`std::os::unix::fs::FileExt` or `std::os::windows::fs::FileExt`.

When a completion backend lands, it lives in its own module under `src/backend/` and the
crate-level `forbid` becomes `#![deny(unsafe_code)]` with `#![allow(unsafe_code)]` on that
module alone, so the reference backend and the public surface stay provably safe. Every
`unsafe` block carries a `// SAFETY:` comment, and this file then carries the invariants
they rely on: that a submitted buffer stays allocated and unmoved until the kernel reports
the operation complete, however the request's future was dropped, and that a completion is
matched to its request before any byte of the buffer is read.
