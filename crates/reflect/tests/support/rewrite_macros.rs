//! Attribute macros that rewrite the item below `#[component]` or `#[value]` after that
//! attribute has read it. The compile-fail cases use them to show that the expansion checks
//! itself against the type the compiler ends up with, not against what it parsed.

use proc_macro::TokenStream;

/// Replaces every `u8` in the item by `bool`.
#[proc_macro_attribute]
pub fn u8_to_bool(_: TokenStream, item: TokenStream) -> TokenStream {
    item.to_string().replace("u8", "bool").parse().unwrap()
}

/// Replaces every `u8` in the item by `Wrapper`, which the case defines.
#[proc_macro_attribute]
pub fn u8_to_wrapper(_: TokenStream, item: TokenStream) -> TokenStream {
    item.to_string().replace("u8", "Wrapper").parse().unwrap()
}

/// Raises the item's alignment to 16.
#[proc_macro_attribute]
pub fn align16(_: TokenStream, item: TokenStream) -> TokenStream {
    format!("#[repr(align(16))] {item}").parse().unwrap()
}

/// Appends a variant to an enum.
#[proc_macro_attribute]
pub fn add_variant(_: TokenStream, item: TokenStream) -> TokenStream {
    let item = item.to_string();
    let end = item.rfind('}').unwrap();
    format!("{} , Added = 200 }}", &item[..end]).parse().unwrap()
}

/// Swaps the discriminants 0 and 1 of an enum.
#[proc_macro_attribute]
pub fn swap_discriminants(_: TokenStream, item: TokenStream) -> TokenStream {
    item.to_string().replace("= 0", "= 9").replace("= 1", "= 0").replace("= 9", "= 1").parse().unwrap()
}
