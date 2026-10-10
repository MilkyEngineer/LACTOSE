//! Expansion of `#[system]` and `#[finalize]`.
//!
//! The expansion names `::lactose_ecs` and `::core` only and computes no key: the system
//! key is hashed in the invoking crate by `SystemDescriptor::new`, a `const fn`. Each
//! attribute writes one access table, from the parameter types and `#[system]`'s `writes`
//! list, and the entry point that fetches the same parameters in the same order; the
//! `unsafe` blocks that hand both to `lactose-ecs` are spanned at the call site, so the
//! invoking crate keeps `#![forbid(unsafe_code)]` (`crates/ecs/SAFETY.md`).

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Error, FnArg, Ident, ItemFn, LitStr, Pat, Path, Result, ReturnType, Token, Type, bracketed,
};

const USAGE: &str = "expected `#[system(name = \"crate::name\", ...)]` with, in any order, \
                     `worlds = [server, client, prediction]`, `in_set = PATH`, \
                     `after = f` or `after = [f, g]`, `writes = [T, ..]`, `reduce = R` and \
                     `exclusive`";

/// The errors, then the function as written, so its uses elsewhere add nothing.
fn failed(item: TokenStream, error: &Error) -> TokenStream {
    let mut out = error.to_compile_error();
    out.extend(item);
    out
}

struct SystemArgs {
    name: Option<LitStr>,
    worlds: Option<Vec<Ident>>,
    in_set: Option<Path>,
    after: Vec<Path>,
    writes: Vec<Type>,
    reduce: Option<Type>,
    exclusive: bool,
}

fn parse_system_args(args: TokenStream) -> Result<SystemArgs> {
    let parser = |input: ParseStream| -> Result<SystemArgs> {
        let mut parsed = SystemArgs {
            name: None,
            worlds: None,
            in_set: None,
            after: Vec::new(),
            writes: Vec::new(),
            reduce: None,
            exclusive: false,
        };
        let mut seen: Vec<String> = Vec::new();
        while !input.is_empty() {
            let ident = Ident::parse_any(input)?;
            let name = ident.unraw().to_string();
            if seen.contains(&name) {
                return Err(Error::new(ident.span(), format!("`{name}` is given twice")));
            }
            seen.push(name.clone());
            match name.as_str() {
                "name" => {
                    input.parse::<Token![=]>()?;
                    let literal: LitStr = input.parse()?;
                    if literal.value().is_empty() {
                        return Err(Error::new(literal.span(), "the name cannot be empty"));
                    }
                    parsed.name = Some(literal);
                }
                "worlds" => {
                    input.parse::<Token![=]>()?;
                    let list;
                    bracketed!(list in input);
                    let worlds = Punctuated::<Ident, Token![,]>::parse_terminated(&list)?;
                    for world in &worlds {
                        if !matches!(
                            world.unraw().to_string().as_str(),
                            "server" | "client" | "prediction"
                        ) {
                            return Err(Error::new(
                                world.span(),
                                format!(
                                    "unknown world `{world}`: expected `server`, `client` or \
                                     `prediction`"
                                ),
                            ));
                        }
                    }
                    parsed.worlds = Some(worlds.into_iter().collect());
                }
                "in_set" => {
                    input.parse::<Token![=]>()?;
                    parsed.in_set = Some(input.parse()?);
                }
                "after" => {
                    input.parse::<Token![=]>()?;
                    if input.peek(syn::token::Bracket) {
                        let list;
                        bracketed!(list in input);
                        parsed.after = Punctuated::<Path, Token![,]>::parse_terminated(&list)?
                            .into_iter()
                            .collect();
                    } else {
                        parsed.after = vec![input.parse()?];
                    }
                }
                "writes" => {
                    input.parse::<Token![=]>()?;
                    let list;
                    bracketed!(list in input);
                    parsed.writes = Punctuated::<Type, Token![,]>::parse_terminated(&list)?
                        .into_iter()
                        .collect();
                }
                "reduce" => {
                    input.parse::<Token![=]>()?;
                    parsed.reduce = Some(input.parse()?);
                }
                "exclusive" => parsed.exclusive = true,
                _ => {
                    return Err(Error::new(
                        ident.span(),
                        format!("unknown argument `{name}`; {USAGE}"),
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(parsed)
    };
    parser.parse2(args)
}

/// `apply_gravity` gives `APPLY_GRAVITY`.
fn descriptor_ident(function: &Ident) -> Ident {
    Ident::new(&function.unraw().to_string().to_uppercase(), function.span())
}

/// The path of the descriptor static of the system function at `path`: `m::f` gives `m::F`.
fn descriptor_path(path: &Path) -> Path {
    let mut path = path.clone();
    if let Some(last) = path.segments.last_mut() {
        last.ident = descriptor_ident(&last.ident);
    }
    path
}

/// Whether `ty` is written `&mut World`, the parameter an exclusive system takes first.
/// The macro cannot see through an alias; the schedule checks the table.
fn is_world(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => {
            reference.mutability.is_some()
                && matches!(&*reference.elem, Type::Path(path)
                    if path.path.segments.last().is_some_and(|segment| segment.ident == "World"))
        }
        _ => false,
    }
}

/// The parameter types of `function`, after making every identifier binding mutable.
fn parameters(function: &mut ItemFn) -> Result<Vec<Type>> {
    let mut types = Vec::new();
    for input in &mut function.sig.inputs {
        match input {
            FnArg::Receiver(receiver) => {
                return Err(Error::new(
                    receiver.span(),
                    "a system is a free function, not a method",
                ));
            }
            FnArg::Typed(typed) => {
                if let Pat::Ident(binding) = &mut *typed.pat
                    && binding.by_ref.is_none()
                {
                    binding.mutability = Some(Token![mut](binding.ident.span()));
                }
                types.push((*typed.ty).clone());
            }
        }
    }
    Ok(types)
}

fn check_signature(function: &ItemFn) -> Result<()> {
    if let Some(param) = function.sig.generics.type_params().next() {
        return Err(Error::new(param.span(), "a system cannot be generic"));
    }
    if let Some(param) = function.sig.generics.const_params().next() {
        return Err(Error::new(param.span(), "a system cannot be generic"));
    }
    if let Some(asyncness) = function.sig.asyncness {
        return Err(Error::new(asyncness.span(), "a system is not `async`"));
    }
    Ok(())
}

/// The access table: one `SystemParam::ACCESS` per parameter, each spanned at its type so
/// a type that is not a parameter is reported there, then one `Writes` entry per column of
/// `writes`, after every parameter so the fetch indices stay the parameters'.
fn access_table(types: &[Type], writes: &[Type]) -> TokenStream {
    let entries = types
        .iter()
        .map(|ty| quote_spanned!(ty.span()=> <#ty as ::lactose_ecs::SystemParam>::ACCESS));
    let writes = writes
        .iter()
        .map(|ty| quote_spanned!(ty.span()=> ::lactose_ecs::__private::writes::<#ty>()));
    quote!(&[#(#entries,)* #(#writes),*])
}

/// The call of `function` with every parameter fetched in order.
fn call(function: &Ident, types: &[Type]) -> TokenStream {
    let fetches =
        (0..types.len()).map(|index| quote!(::lactose_ecs::__private::fetch(__invocation, #index)));
    quote!(#function(#(#fetches),*))
}

/// Expands `#[system(...)]`.
pub(crate) fn system(args: TokenStream, item: TokenStream) -> TokenStream {
    try_system(args, item.clone()).unwrap_or_else(|error| failed(item, &error))
}

fn try_system(args: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let args = parse_system_args(args)?;
    let mut function: ItemFn = syn::parse2(item)?;
    check_signature(&function)?;
    let name = args.name.ok_or_else(|| {
        Error::new(Span::call_site(), format!("missing `name = \"crate::name\"`; {USAGE}"))
    })?;
    let types = parameters(&mut function)?;
    let ident = function.sig.ident.clone();

    if args.exclusive {
        if !types.first().is_some_and(is_world) {
            let span = types.first().map_or(function.sig.ident.span(), Spanned::span);
            return Err(Error::new(
                span,
                "an exclusive system takes `&mut World` as its first parameter",
            ));
        }
        if let Some(ty) = types.iter().skip(1).find(|ty| is_world(ty)) {
            return Err(Error::new(ty.span(), "an exclusive system takes `&mut World` once"));
        }
    } else if let Some(ty) = types.iter().find(|ty| is_world(ty)) {
        return Err(Error::new(
            ty.span(),
            "`&mut World` is the first parameter of an exclusive system: add `exclusive`",
        ));
    }
    match (&args.reduce, &function.sig.output) {
        (Some(reduce), ReturnType::Default) => {
            return Err(Error::new(
                reduce.span(),
                "a reduction returns one `R::Item` per entity: give the function a return type",
            ));
        }
        (Some(reduce), ReturnType::Type(..)) if args.exclusive => {
            return Err(Error::new(reduce.span(), "an exclusive system is not a reduction"));
        }
        (None, ReturnType::Type(_, ty)) => {
            return Err(Error::new(
                ty.span(),
                "a system returns nothing unless it is a reduction: add `reduce = R`",
            ));
        }
        _ => {}
    }

    let descriptor = descriptor_ident(&ident);
    let doc = format!("The system descriptor of `{}`, emitted by `#[system]`.", ident.unraw());
    let worlds = match &args.worlds {
        None => quote!(::lactose_ecs::WorldSet::SERVER.union(::lactose_ecs::WorldSet::PREDICTION)),
        Some(worlds) => {
            let sets = worlds.iter().map(|world| {
                let set = Ident::new(&world.unraw().to_string().to_uppercase(), world.span());
                quote!(.union(::lactose_ecs::WorldSet::#set))
            });
            quote!(::lactose_ecs::WorldSet::NONE #(#sets)*)
        }
    };
    let set = match &args.in_set {
        Some(path) => quote!(::core::option::Option::Some(&#path)),
        None => quote!(::core::option::Option::None),
    };
    let after = args.after.iter().map(descriptor_path);
    let after_count = args.after.len();
    let access = access_table(&types, &args.writes);
    let invoke = call(&ident, &types);
    let exclusive = args.exclusive;
    let finalize_name = format_ident!("__finalize_{}", ident.unraw());

    // The kernel entry point, the finalize entry point with its definition, the finalize
    // table, the reducer's operations and its name.
    let (run, finalize_fn, finalize, finalize_access, reducer, reduce_name) = match &args.reduce {
        Some(reduce) => (
            quote! {
                fn __run(__invocation: &mut ::lactose_ecs::__private::Invocation<'_, '_>) {
                    let __item = #invoke;
                    ::lactose_ecs::__private::reduce::<#reduce>(__invocation, __item);
                }
            },
            quote! {
                fn __finalize(__invocation: &mut ::lactose_ecs::__private::Invocation<'_, '_>) {
                    let __output = ::lactose_ecs::__private::finish::<#reduce>(__invocation);
                    unsafe { #finalize_name.call(__invocation, __output) }
                }
            },
            quote!(__finalize),
            quote!(#finalize_name.access()),
            quote!(::core::option::Option::Some(::lactose_ecs::__private::ReducerOps::of::<#reduce>())),
            quote!(::core::option::Option::Some(::core::stringify!(#reduce))),
        ),
        None => (
            quote! {
                fn __run(__invocation: &mut ::lactose_ecs::__private::Invocation<'_, '_>) {
                    #invoke;
                }
            },
            quote!(),
            quote!(::lactose_ecs::__private::Kernel::no_finalize),
            quote!(&[]),
            quote!(::core::option::Option::None),
            quote!(::core::option::Option::None),
        ),
    };

    function.attrs.push(syn::parse_quote!(#[allow(unused_mut)]));
    Ok(quote! {
        #function

        #[doc = #doc]
        pub static #descriptor: ::lactose_ecs::SystemDescriptor = {
            const __ACCESS: &[::lactose_ecs::Access] = #access;
            static __AFTER: [&::lactose_ecs::SystemDescriptor; #after_count] = [#(&#after),*];
            #run
            #finalize_fn
            unsafe {
                ::lactose_ecs::SystemDescriptor::new(
                    #name,
                    #worlds,
                    #set,
                    &__AFTER,
                    __ACCESS,
                    #finalize_access,
                    #exclusive,
                    #reduce_name,
                    ::lactose_ecs::__private::Kernel {
                        run: __run,
                        finalize: #finalize,
                        reducer: #reducer,
                    },
                )
            }
        };
    })
}

/// Expands `#[finalize(system_fn)]`.
pub(crate) fn finalize(args: TokenStream, item: TokenStream) -> TokenStream {
    try_finalize(args, item.clone()).unwrap_or_else(|error| failed(item, &error))
}

fn try_finalize(args: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let system: Ident = syn::parse2(args).map_err(|error| {
        Error::new(
            error.span(),
            "expected `#[finalize(system_fn)]`, naming the reduction's function",
        )
    })?;
    let mut function: ItemFn = syn::parse2(item)?;
    check_signature(&function)?;
    let types = parameters(&mut function)?;
    let Some(output) = types.first().cloned() else {
        return Err(Error::new(
            function.sig.ident.span(),
            "a finalize function takes the reduction's output as its first parameter",
        ));
    };
    if let ReturnType::Type(_, ty) = &function.sig.output {
        return Err(Error::new(ty.span(), "a finalize function returns nothing"));
    }
    if let Some(ty) = types.iter().find(|ty| is_world(ty)) {
        return Err(Error::new(ty.span(), "a finalize function cannot take `&mut World`"));
    }
    let params = &types[1..];
    let access = access_table(params, &[]);
    let ident = function.sig.ident.clone();
    let fetches = (0..params.len())
        .map(|index| quote!(::lactose_ecs::__private::fetch(__invocation, #index)));
    let name = format_ident!("__finalize_{}", system.unraw());
    let descriptor = descriptor_ident(&system);
    let stray = format!(
        "`#[finalize({})]` names a system that is not a reduction: add `reduce = R` to it",
        system.unraw()
    );
    let vis = &function.vis;
    function.attrs.push(syn::parse_quote!(#[allow(unused_mut)]));
    Ok(quote! {
        #function

        // The system the finalize names exists and is a reduction; otherwise nothing would
        // ever run this function.
        const _: () = ::core::assert!(#descriptor.reduce().is_some(), #stray);

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        #vis const #name: ::lactose_ecs::__private::Finalize<#output> = {
            const __ACCESS: &[::lactose_ecs::Access] = #access;
            fn __entry(
                __invocation: &mut ::lactose_ecs::__private::Invocation<'_, '_>,
                __output: #output,
            ) {
                #ident(__output, #(#fetches),*);
            }
            unsafe { ::lactose_ecs::__private::Finalize::new(__ACCESS, __entry) }
        };
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(args: &str, item: &str) -> String {
        system(args.parse().unwrap(), item.parse().unwrap()).to_string()
    }

    #[test]
    fn descriptor_is_named_in_upper_snake_case() {
        let expansion = expand("name = \"t::apply_gravity\"", "fn apply_gravity(v: Mut<V>) {}");
        assert!(expansion.contains("pub static APPLY_GRAVITY"), "{expansion}");
        assert!(expansion.contains("mut v : Mut < V >"), "{expansion}");
    }

    #[test]
    fn worlds_default_to_server_and_prediction() {
        let expansion = expand("name = \"t::f\"", "fn f() {}");
        assert!(expansion.contains("WorldSet :: SERVER . union"), "{expansion}");
        assert!(expansion.contains("WorldSet :: PREDICTION"), "{expansion}");
    }

    #[test]
    fn after_names_descriptor_statics() {
        let expansion = expand("name = \"t::f\", after = [g, m::h]", "fn f() {}");
        assert!(expansion.contains("& G"), "{expansion}");
        assert!(expansion.contains("& m :: H"), "{expansion}");
    }

    #[test]
    fn writes_follow_the_parameters_in_the_table() {
        let expansion = expand("name = \"t::f\", writes = [A, m::B]", "fn f(c: Commands) {}");
        let parameter = expansion.find("SystemParam > :: ACCESS").expect("the parameter entry");
        let first = expansion.find("writes :: < A >").expect("a `writes` entry for A");
        assert!(parameter < first, "{expansion}");
        assert!(expansion.contains("writes :: < m :: B >"), "{expansion}");
    }

    #[test]
    fn the_only_unsafe_is_the_descriptor_constructor_call() {
        let expansion = expand("name = \"t::f\"", "fn f(a: &A) {}");
        assert_eq!(expansion.matches("unsafe").count(), 1, "{expansion}");
        let reduction = expand("name = \"t::f\", reduce = R", "fn f(a: &A) -> u32 { 0 }");
        assert_eq!(reduction.matches("unsafe").count(), 2, "{reduction}");
    }

    #[test]
    fn errors_name_the_offending_tokens() {
        for (args, item, message) in [
            ("worlds = [server]", "fn f() {}", "missing `name"),
            ("name = \"t::f\", worlds = [servr]", "fn f() {}", "unknown world `servr`"),
            (
                "name = \"t::f\", exclusive",
                "fn f(c: Commands) {}",
                "takes `&mut World` as its first",
            ),
            ("name = \"t::f\"", "fn f(w: &mut World) {}", "add `exclusive`"),
            ("name = \"t::f\", reduce = R", "fn f(a: &A) {}", "give the function a return type"),
            ("name = \"t::f\"", "fn f(a: &A) -> u32 { 0 }", "add `reduce = R`"),
            ("name = \"t::f\", bogus", "fn f() {}", "unknown argument `bogus`"),
            ("name = \"t::f\"", "fn f<T>() {}", "cannot be generic"),
        ] {
            let expansion = expand(args, item);
            assert!(expansion.contains("compile_error"), "{args}: {expansion}");
            assert!(expansion.contains(message), "{args}: `{message}` not in {expansion}");
        }
    }

    #[test]
    fn finalize_pairs_by_name() {
        let expansion = finalize(
            "total_mass".parse().unwrap(),
            "fn store(sum: Scalar, out: ResMut<Total>) { out.0 = sum; }".parse().unwrap(),
        )
        .to_string();
        assert!(expansion.contains("const __finalize_total_mass"), "{expansion}");
        assert!(expansion.contains("Finalize < Scalar >"), "{expansion}");
        assert!(expansion.contains("fetch (__invocation , 0usize)"), "{expansion}");
        assert!(expansion.contains("TOTAL_MASS . reduce () . is_some ()"), "{expansion}");
    }
}
