//! Proc macros of `lactose-reflect`. Use them through that crate, which re-exports them and
//! documents what they accept; the expansion names `::lactose_reflect` paths.
//!
//! Foundation tier, outside the determinism boundary: a proc macro compiles for the host, so
//! this crate depends on no boundary crate and computes nothing that reaches a build output
//! other than tokens. See `README.md`.

#![forbid(unsafe_code)]

use proc_macro::TokenStream;

mod expand;

/// Declares a component. See `lactose_reflect::component`.
#[proc_macro_attribute]
pub fn component(args: TokenStream, item: TokenStream) -> TokenStream {
    expand::reflected(expand::Role::Component, args.into(), item.into()).into()
}

/// Declares a reflected resource. See `lactose_reflect::resource`.
#[proc_macro_attribute]
pub fn resource(args: TokenStream, item: TokenStream) -> TokenStream {
    expand::reflected(expand::Role::Resource, args.into(), item.into()).into()
}

/// Declares a struct or fieldless enum usable as a field. See `lactose_reflect::value`.
#[proc_macro_attribute]
pub fn value(args: TokenStream, item: TokenStream) -> TokenStream {
    expand::value(args.into(), item.into()).into()
}
