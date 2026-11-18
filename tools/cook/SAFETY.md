# lactose-cook safety

The package contains no `unsafe` and forbids it: `#![forbid(unsafe_code)]` is the first
attribute of `src/lib.rs`, of `src/main.rs` and of every test crate root. Architecture
section 21 lists `tools/cook` for the FFI to Slang and compression libraries; neither is
called from here. Slang runs as a subprocess, the pinned `slangc` and on macOS Apple's
`metal` through `xcrun`, through `std::process::Command`: the cooker links neither, and
reads back only their output files and streams. The one compression the cooker does is
zstd's, inside `lactose-asset`'s `PackBuilder`, through the `zstd` crate's safe interface.

When an FFI binding lands, it lives in its own module and the crate-level `forbid` becomes
`#![deny(unsafe_code)]` with `#![allow(unsafe_code)]` on that module alone, so the reader,
the processors and the pack writing stay provably safe. Every `unsafe` block then carries a
`// SAFETY:` comment, and this file the invariants they rely on.
