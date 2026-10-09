//! Proc macros of `lactose-ecs`: `#[system]` and `#[finalize]`. Use them through that
//! crate, which re-exports them; the expansion names `::lactose_ecs` paths. `README.md`
//! states what each attribute accepts and emits.
//!
//! Foundation tier, outside the determinism boundary: a proc macro compiles for the host, so
//! this crate depends on no boundary crate and computes nothing that reaches a build output
//! other than tokens.
//!
//! # Status
//!
//! Facade: both attributes are declared and documented, and each expands to a compile error
//! that says so, until the implementation lane writes the expansion.

#![forbid(unsafe_code)]

use proc_macro::TokenStream;

/// Declares a system. See `lactose_ecs::system` and `README.md`.
#[proc_macro_attribute]
pub fn system(args: TokenStream, item: TokenStream) -> TokenStream {
    let _ = (args, item);
    r#"::core::compile_error!("`#[system]` is not implemented yet: lactose-ecs is a facade (crates/ecs/README.md)");"#
        .parse()
        .expect("a literal token stream parses")
}

/// Declares a reduction's finishing step. See `lactose_ecs::finalize` and `README.md`.
#[proc_macro_attribute]
pub fn finalize(args: TokenStream, item: TokenStream) -> TokenStream {
    let _ = (args, item);
    r#"::core::compile_error!("`#[finalize]` is not implemented yet: lactose-ecs is a facade (crates/ecs/README.md)");"#
        .parse()
        .expect("a literal token stream parses")
}
