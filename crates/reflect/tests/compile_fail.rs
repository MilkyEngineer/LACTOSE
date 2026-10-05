//! What must not compile, and what the author is told. The expected text is exact, so a
//! toolchain bump regenerates it: `TRYBUILD=overwrite cargo test -p lactose-reflect --test
//! compile_fail`, then read the diff. The macro's own messages are also asserted, by
//! substring, in the unit tests of `lactose-reflect-macros`.

#[test]
fn rejected_declarations() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
