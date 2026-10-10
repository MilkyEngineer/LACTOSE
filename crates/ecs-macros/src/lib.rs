//! Proc macros of `lactose-ecs`: `#[system]` and `#[finalize]`. Use them through that
//! crate, which re-exports them; the expansion names `::lactose_ecs` paths. `README.md`
//! states what each attribute accepts and emits.
//!
//! Foundation tier, outside the determinism boundary: a proc macro compiles for the host, so
//! this crate depends on no boundary crate and computes nothing that reaches a build output
//! other than tokens.

#![forbid(unsafe_code)]

use proc_macro::TokenStream;

mod expand;

/// Declares a system. See `lactose_ecs::system` and `README.md`.
#[proc_macro_attribute]
pub fn system(args: TokenStream, item: TokenStream) -> TokenStream {
    expand::system(args.into(), item.into()).into()
}

/// Declares a reduction's finishing step. See `lactose_ecs::finalize` and `README.md`.
#[proc_macro_attribute]
pub fn finalize(args: TokenStream, item: TokenStream) -> TokenStream {
    expand::finalize(args.into(), item.into()).into()
}
