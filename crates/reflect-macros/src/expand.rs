//! Expansion of `#[component]`, `#[resource]` and `#[value]`.
//!
//! The expansion names `::lactose_reflect` and `::core` only, contains no `unsafe`, and does
//! no layout arithmetic of its own: sizes and offsets are `size_of` and `offset_of!` calls
//! the compiler evaluates in the invoking crate. The component key is likewise hashed there,
//! by a `const fn` call, never here.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote, quote_spanned};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DataEnum, DeriveInput, Error, Expr, ExprLit, Fields, Ident, Index, Lit,
    LitInt, LitStr, Member, Meta, Result, Token, Type,
};

#[derive(Clone, Copy)]
pub(crate) enum Role {
    Component,
    Resource,
}

impl Role {
    fn attribute(self) -> &'static str {
        match self {
            Self::Component => "component",
            Self::Resource => "resource",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Component => "a component",
            Self::Resource => "a reflected resource",
        }
    }
}

/// Expands `#[component(...)]` or `#[resource(...)]`.
pub(crate) fn reflected(role: Role, args: TokenStream, item: TokenStream) -> TokenStream {
    try_reflected(role, args, item.clone()).unwrap_or_else(|error| failed(item, &error))
}

/// Expands `#[value]`.
pub(crate) fn value(args: TokenStream, item: TokenStream) -> TokenStream {
    try_value(args, item.clone()).unwrap_or_else(|error| failed(item, &error))
}

/// The errors, plus the item without its `#[field]` attributes so that uses of the type
/// elsewhere do not add unrelated errors.
fn failed(item: TokenStream, error: &Error) -> TokenStream {
    let mut out = error.to_compile_error();
    if let Ok(mut input) = syn::parse2::<DeriveInput>(item) {
        match &mut input.data {
            Data::Struct(data) => strip_field_attributes(&mut data.fields),
            Data::Enum(data) => {
                data.variants.iter_mut().for_each(|v| strip_field_attributes(&mut v.fields));
            }
            Data::Union(data) => {
                data.fields.named.iter_mut().for_each(|f| f.attrs.retain(|a| !is_field(a)));
            }
        }
        input.to_tokens(&mut out);
    }
    out
}

fn is_field(attribute: &Attribute) -> bool {
    attribute.path().is_ident("field")
}

fn strip_field_attributes(fields: &mut Fields) {
    fields.iter_mut().for_each(|field| field.attrs.retain(|a| !is_field(a)));
}

struct TypeArgs {
    key: LitStr,
    persistence: Ident,
    version: u32,
    replicated: bool,
    extracted: bool,
}

fn parse_type_args(role: Role, args: TokenStream) -> Result<TypeArgs> {
    let attribute = role.attribute();
    let usage = format!(
        "expected `#[{attribute}(key = \"crate::name\", <policy>)]` where <policy> is \
         `persistent`, `transient` or `derived`, optionally followed by `version = N`, \
         `replicated`, `extracted`"
    );
    let mut key = None;
    let mut persistence: Option<Ident> = None;
    let mut version = None;
    let mut replicated = false;
    let mut extracted = false;

    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(args)?;
    for meta in &metas {
        let repeated = || Error::new(meta.span(), "this argument is given twice");
        match meta {
            Meta::NameValue(pair) if pair.path.is_ident("key") => {
                let Expr::Lit(ExprLit { lit: Lit::Str(name), .. }) = &pair.value else {
                    return Err(Error::new(pair.value.span(), "the key is a string literal"));
                };
                if name.value().is_empty() {
                    return Err(Error::new(name.span(), "the key cannot be empty"));
                }
                if key.replace(name.clone()).is_some() {
                    return Err(repeated());
                }
            }
            Meta::NameValue(pair) if pair.path.is_ident("version") => {
                let Expr::Lit(ExprLit { lit: Lit::Int(number), .. }) = &pair.value else {
                    return Err(Error::new(pair.value.span(), "the version is an integer"));
                };
                if version.replace(number.base10_parse::<u32>()?).is_some() {
                    return Err(repeated());
                }
            }
            Meta::Path(path) if path.is_ident("replicated") => {
                if std::mem::replace(&mut replicated, true) {
                    return Err(repeated());
                }
            }
            Meta::Path(path) if path.is_ident("extracted") => {
                if std::mem::replace(&mut extracted, true) {
                    return Err(repeated());
                }
            }
            Meta::Path(path)
                if path.is_ident("persistent")
                    || path.is_ident("transient")
                    || path.is_ident("derived") =>
            {
                if persistence.is_some() {
                    return Err(Error::new(
                        path.span(),
                        "a type has one persistence policy: `persistent`, `transient` or `derived`",
                    ));
                }
                persistence = path.get_ident().cloned();
            }
            _ => return Err(Error::new(meta.span(), format!("unknown argument; {usage}"))),
        }
    }

    let key = key.ok_or_else(|| {
        Error::new(Span::call_site(), format!("missing `key = \"...\"`; {usage}"))
    })?;
    let persistence = persistence.ok_or_else(|| {
        Error::new(
            Span::call_site(),
            format!(
                "missing persistence policy: add `persistent`, `transient` or `derived` to \
                 `#[{attribute}(...)]`"
            ),
        )
    })?;
    Ok(TypeArgs { key, persistence, version: version.unwrap_or(1), replicated, extracted })
}

struct FieldSpec {
    member: Member,
    name: String,
    ty: Type,
    /// `None` for a `#[field(padding)]` field.
    id: Option<u16>,
    span: Span,
}

/// Reads and removes the `#[field(...)]` attribute of every field.
fn take_fields(fields: &mut Fields) -> Result<Vec<FieldSpec>> {
    let mut specs: Vec<FieldSpec> = Vec::new();
    let mut errors: Option<Error> = None;
    let mut fail = |error: Error| match &mut errors {
        Some(errors) => errors.combine(error),
        None => errors = Some(error),
    };

    for (index, field) in fields.iter_mut().enumerate() {
        // The name, or the type of a tuple field: a span that starts at the field's attributes
        // would underline only their first token.
        let span = field.ident.as_ref().map_or_else(|| field.ty.span(), Ident::span);
        let (member, name) = match &field.ident {
            Some(ident) => (Member::Named(ident.clone()), ident.to_string()),
            None => (Member::Unnamed(Index::from(index)), index.to_string()),
        };

        let mut id = None;
        let mut padding = false;
        let mut malformed = false;
        for attribute in field.attrs.iter().filter(|a| is_field(a)) {
            let parsed = attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("id") {
                    let number: LitInt = meta.value()?.parse()?;
                    let parsed = number.base10_parse::<u16>().map_err(|_| {
                        Error::new(number.span(), "a field ID is an integer from 0 to 65535")
                    })?;
                    if id.replace(parsed).is_some() {
                        return Err(meta.error("a field has one ID"));
                    }
                    Ok(())
                } else if meta.path.is_ident("padding") {
                    padding = true;
                    Ok(())
                } else {
                    Err(meta.error("expected `#[field(id = N)]` or `#[field(padding)]`"))
                }
            });
            if let Err(error) = parsed {
                malformed = true;
                fail(error);
            }
        }
        field.attrs.retain(|a| !is_field(a));

        if malformed {
            continue;
        }
        match (id, padding) {
            (Some(_), true) => {
                fail(Error::new(span, format!("padding field `{name}` cannot have a field ID")));
            }
            (None, false) => fail(Error::new(
                span,
                format!(
                    "field `{name}` has no field ID: add `#[field(id = N)]` with an ID this \
                     type has never used, or `#[field(padding)]` if it is padding"
                ),
            )),
            (Some(id), false) => {
                if let Some(first) = specs.iter().find(|spec| spec.id == Some(id)) {
                    fail(Error::new(
                        span,
                        format!("field ID {id} is already used by field `{}`", first.name),
                    ));
                }
            }
            (None, true) => {}
        }
        specs.push(FieldSpec { member, name, ty: field.ty.clone(), id, span });
    }

    match errors {
        Some(errors) => Err(errors),
        None => Ok(specs),
    }
}

/// Rejects what no reflected type may have, and returns what the macro must add to the
/// derive list.
fn check_item(input: &DeriveInput, what: &str) -> Result<TokenStream> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(Error::new(
            input.generics.span(),
            format!(
                "{what} cannot be generic: its key and layout must be one fixed thing \
                 (architecture section 6); declare one type per instantiation"
            ),
        ));
    }
    let mut has_clone = false;
    let mut has_copy = false;
    for attribute in &input.attrs {
        if attribute.path().is_ident("derive") {
            attribute.parse_nested_meta(|meta| {
                let last = meta.path.segments.last().map(|segment| segment.ident.to_string());
                has_clone |= last.as_deref() == Some("Clone");
                has_copy |= last.as_deref() == Some("Copy");
                Ok(())
            })?;
        }
    }
    let clone = (!has_clone).then(|| quote!(::core::clone::Clone,));
    let copy = (!has_copy).then(|| quote!(::core::marker::Copy,));
    Ok(quote! {
        #clone #copy
        ::lactose_reflect::bytemuck::NoUninit,
        ::lactose_reflect::bytemuck::CheckedBitPattern
    })
}

fn reject_repr(input: &DeriveInput) -> Result<()> {
    match input.attrs.iter().find(|a| a.path().is_ident("repr")) {
        Some(repr) => Err(Error::new(
            repr.span(),
            "remove this `#[repr]`: the attribute lays the struct out as `#[repr(C)]`",
        )),
        None => Ok(()),
    }
}

/// The parts of an expansion that depend on the fields of a struct.
struct StructParts {
    /// Constant assertions that no field is preceded or followed by implicit padding.
    layout_checks: TokenStream,
    /// One `FieldInfo` expression per field, comma-terminated.
    field_infos: TokenStream,
    /// Statements of a `visit` body.
    visits: TokenStream,
    /// Statements of a `visit_references` body.
    reference_visits: TokenStream,
}

/// With `own_ids` each visit reports the field's own ID, as a component does. Without it
/// each visit reports the `id` parameter in scope, the ID of the field that holds a
/// `#[value]` struct.
fn struct_parts(name: &Ident, fields: &[FieldSpec], own_ids: bool) -> StructParts {
    let mut parts = StructParts {
        layout_checks: TokenStream::new(),
        field_infos: TokenStream::new(),
        visits: TokenStream::new(),
        reference_visits: TokenStream::new(),
    };
    let how = "order the fields from the largest alignment to the smallest, or insert \
               `#[field(padding)] _pad: lactose_reflect::Padding<N>`";
    let mut previous_end = None;

    for field in fields {
        let FieldSpec { member, name: field_name, ty, id, span } = field;
        let offset = quote!(::core::mem::offset_of!(#name, #member));

        if let Some(previous_end) = &previous_end {
            let message =
                format!("`{name}` has padding before field `{field_name}`: {how} before it");
            parts.layout_checks.extend(quote_spanned! {*span=>
                const _: () = ::core::assert!(#offset == #previous_end, #message);
            });
        }
        previous_end = Some(quote!(#offset + ::core::mem::size_of::<#ty>()));

        let type_span = ty.span();
        match id {
            Some(id) => {
                let field_id = quote!(::lactose_reflect::__private::FieldId(#id));
                parts.field_infos.extend(quote_spanned! {type_span=>
                    ::lactose_reflect::FieldInfo::new::<#ty>(#field_id, #field_name, #offset),
                });
                let reported = if own_ids { field_id } else { quote!(id) };
                parts.visits.extend(quote! {
                    ::lactose_reflect::Field::visit(&self.#member, #reported, visitor);
                });
                parts.reference_visits.extend(quote! {
                    ::lactose_reflect::Field::visit_references(&mut self.#member, #reported, visitor);
                });
            }
            None => parts.field_infos.extend(quote_spanned! {type_span=>
                ::lactose_reflect::FieldInfo::padding::<#ty>(#field_name, #offset),
            }),
        }
    }

    if let (Some(previous_end), Some(last)) = (&previous_end, fields.last()) {
        let message =
            format!("`{name}` has padding after its last field `{}`: {how} at the end", last.name);
        parts.layout_checks.extend(quote_spanned! {last.span=>
            const _: () = ::core::assert!(::core::mem::size_of::<#name>() == #previous_end, #message);
        });
    }
    parts
}

fn try_reflected(role: Role, args: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let mut input: DeriveInput = syn::parse2(item)?;
    let args = parse_type_args(role, args)?;
    let derives = check_item(&input, role.noun())?;
    let name = input.ident.clone();

    reject_repr(&input)?;
    let Data::Struct(data) = &mut input.data else {
        return Err(Error::new(
            name.span(),
            format!("{} is a struct; mark an enum `#[value]` and hold it in a field", role.noun()),
        ));
    };
    let fields = take_fields(&mut data.fields)?;
    let StructParts { layout_checks, field_infos, visits, reference_visits } =
        struct_parts(&name, &fields, true);

    let TypeArgs { key, persistence, version, replicated, extracted } = args;
    let persistence = match persistence.to_string().as_str() {
        "persistent" => quote!(Persistent),
        "transient" => quote!(Transient),
        _ => quote!(Derived),
    };
    let (role, marker) = match role {
        Role::Component => (quote!(Component), quote!(Component)),
        Role::Resource => (quote!(Resource), quote!(Resource)),
    };
    let rust_name = name.to_string();

    Ok(quote! {
        #[derive(#derives)]
        #[bytemuck(crate = "::lactose_reflect::bytemuck")]
        #[repr(C)]
        #input

        const _: () = {
            #layout_checks

            const FIELDS: &[::lactose_reflect::FieldInfo] = &[#field_infos];
            const INFO: ::lactose_reflect::TypeInfo = ::lactose_reflect::TypeInfo::new(
                #key,
                ::lactose_reflect::TypeRole::#role,
                ::lactose_reflect::Persistence::#persistence,
                #version,
                ::lactose_reflect::Tracking { replicated: #replicated, extracted: #extracted },
                ::lactose_reflect::StructInfo::new::<#name>(#rust_name, FIELDS),
            );

            impl ::lactose_reflect::Reflect for #name {
                const KEY: ::lactose_reflect::__private::ComponentKey =
                    ::lactose_reflect::__private::ComponentKey::from_name(#key);
                const INFO: &'static ::lactose_reflect::TypeInfo = &INFO;

                #[allow(unused_variables)]
                fn visit<V: ::lactose_reflect::FieldVisitor>(&self, visitor: &mut V) {
                    #visits
                }

                #[allow(unused_variables)]
                fn visit_references<V: ::lactose_reflect::ReferenceVisitor>(
                    &mut self,
                    visitor: &mut V,
                ) {
                    #reference_visits
                }
            }

            impl ::lactose_reflect::#marker for #name {}
        };
    })
}

fn try_value(args: TokenStream, item: TokenStream) -> Result<TokenStream> {
    if !args.is_empty() {
        return Err(Error::new(args.span(), "`#[value]` takes no arguments"));
    }
    let mut input: DeriveInput = syn::parse2(item)?;
    let derives = check_item(&input, "a `#[value]` type")?;
    let name = input.ident.clone();
    let rust_name = name.to_string();

    let implementation = match &mut input.data {
        Data::Struct(data) => {
            let fields = take_fields(&mut data.fields)?;
            reject_repr(&input)?;
            let StructParts { layout_checks, field_infos, visits, reference_visits } =
                struct_parts(&name, &fields, false);
            input.attrs.push(syn::parse_quote!(#[repr(C)]));
            quote! {
                #layout_checks

                const FIELDS: &[::lactose_reflect::FieldInfo] = &[#field_infos];
                const LAYOUT: ::lactose_reflect::StructInfo =
                    ::lactose_reflect::StructInfo::new::<#name>(#rust_name, FIELDS);

                impl ::lactose_reflect::Field for #name {
                    const KIND: ::lactose_reflect::FieldKind =
                        ::lactose_reflect::FieldKind::Struct(&LAYOUT);

                    #[allow(unused_variables)]
                    fn visit<V: ::lactose_reflect::FieldVisitor>(
                        &self,
                        id: ::lactose_reflect::__private::FieldId,
                        visitor: &mut V,
                    ) {
                        #visits
                    }

                    #[allow(unused_variables)]
                    fn visit_references<V: ::lactose_reflect::ReferenceVisitor>(
                        &mut self,
                        id: ::lactose_reflect::__private::FieldId,
                        visitor: &mut V,
                    ) {
                        #reference_visits
                    }
                }
            }
        }
        Data::Enum(data) => {
            let variants = enum_variants(&input.attrs, &name, data)?;
            quote! {
                const INFO: ::lactose_reflect::EnumInfo = ::lactose_reflect::EnumInfo {
                    name: #rust_name,
                    size: ::core::mem::size_of::<#name>(),
                    variants: &[#variants],
                };

                impl ::lactose_reflect::Field for #name {
                    const KIND: ::lactose_reflect::FieldKind =
                        ::lactose_reflect::FieldKind::Enum(&INFO);

                    fn visit<V: ::lactose_reflect::FieldVisitor>(
                        &self,
                        id: ::lactose_reflect::__private::FieldId,
                        visitor: &mut V,
                    ) {
                        visitor.bytes(id, ::lactose_reflect::bytemuck::bytes_of(self));
                    }

                    fn visit_references<V: ::lactose_reflect::ReferenceVisitor>(
                        &mut self,
                        _: ::lactose_reflect::__private::FieldId,
                        _: &mut V,
                    ) {
                    }
                }
            }
        }
        Data::Union(_) => {
            return Err(Error::new(
                name.span(),
                "a `#[value]` type is a struct or a fieldless enum",
            ));
        }
    };

    Ok(quote! {
        #[derive(#derives)]
        #[bytemuck(crate = "::lactose_reflect::bytemuck")]
        #input

        const _: () = {
            #implementation
        };
    })
}

/// Checks a `#[value]` enum and returns one `VariantInfo` expression per variant.
fn enum_variants(attrs: &[Attribute], name: &Ident, data: &DataEnum) -> Result<TokenStream> {
    let mut width = None;
    for attribute in attrs.iter().filter(|a| a.path().is_ident("repr")) {
        attribute.parse_nested_meta(|meta| {
            width = meta.path.get_ident().map(Ident::to_string);
            Ok(())
        })?;
    }
    if !matches!(width.as_deref(), Some("u8" | "u16" | "u32")) {
        return Err(Error::new(
            name.span(),
            "a `#[value]` enum states its width: add `#[repr(u8)]`, `#[repr(u16)]` or \
             `#[repr(u32)]` below `#[value]`",
        ));
    }
    if data.variants.is_empty() {
        return Err(Error::new(name.span(), "a `#[value]` enum has at least one variant"));
    }

    let mut variants = TokenStream::new();
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(Error::new(
                variant.fields.span(),
                "data-carrying enums are not reflected: `#[value]` enums are fieldless; hold \
                 the data in fields beside the enum",
            ));
        }
        if variant.discriminant.is_none() {
            return Err(Error::new(
                variant.span(),
                format!(
                    "variant `{}` needs an explicit discriminant (`{} = N`): its value is \
                     part of the save format",
                    variant.ident, variant.ident
                ),
            ));
        }
        let ident = &variant.ident;
        let variant_name = ident.to_string();
        variants.extend(quote! {
            ::lactose_reflect::VariantInfo { name: #variant_name, value: #name::#ident as u32 },
        });
    }
    Ok(variants)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(args: TokenStream, item: TokenStream) -> String {
        reflected(Role::Component, args, item).to_string()
    }

    fn error_of(expansion: &str) -> &str {
        let start = expansion.find("compile_error").expect("expansion has no error");
        &expansion[start..]
    }

    #[test]
    fn expansion_names_only_the_reflect_crate_and_core() {
        let expansions = [
            component(
                quote!(key = "t::a", persistent, version = 2, replicated),
                quote!(
                    struct A {
                        #[field(id = 1)]
                        x: u32,
                        #[field(padding)]
                        _pad: Padding<4>,
                    }
                ),
            ),
            reflected(
                Role::Resource,
                quote!(key = "t::r", transient),
                quote!(
                    struct R(#[field(id = 1)] u64);
                ),
            )
            .to_string(),
            value(
                TokenStream::new(),
                quote!(
                    struct V {
                        #[field(id = 1)]
                        x: u32,
                    }
                ),
            )
            .to_string(),
            value(
                TokenStream::new(),
                quote!(
                    #[repr(u8)]
                    enum E {
                        A = 0,
                    }
                ),
            )
            .to_string(),
        ];
        for expansion in expansions {
            assert!(!expansion.contains("compile_error"), "{expansion}");
            assert!(!expansion.contains("unsafe"), "{expansion}");
            let tokens: Vec<&str> = expansion.split_whitespace().collect();
            let mut roots = 0;
            for (index, token) in tokens.iter().enumerate() {
                // A path root is `::` that does not follow a path segment or `>`.
                let after_segment = index > 0
                    && tokens[index - 1]
                        .chars()
                        .last()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '>');
                if *token == "::" && !after_segment {
                    roots += 1;
                    let root = tokens[index + 1];
                    assert!(
                        matches!(root, "lactose_reflect" | "core"),
                        "`::{root}` in {expansion}"
                    );
                }
            }
            assert!(roots > 0);
            assert!(expansion.contains("\"::lactose_reflect::bytemuck\""));
        }
    }

    #[test]
    fn missing_field_id_is_an_error() {
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                struct A {
                    x: u32,
                }
            ),
        );
        assert!(error_of(&out).contains("field `x` has no field ID"), "{out}");
    }

    #[test]
    fn duplicate_field_id_is_an_error() {
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                struct A {
                    #[field(id = 3)]
                    x: u32,
                    #[field(id = 3)]
                    y: u32,
                }
            ),
        );
        assert!(error_of(&out).contains("field ID 3 is already used by field `x`"), "{out}");
    }

    #[test]
    fn padding_field_cannot_have_an_id() {
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                struct A {
                    #[field(id = 1, padding)]
                    x: Padding<4>,
                }
            ),
        );
        assert!(error_of(&out).contains("padding field `x` cannot have a field ID"), "{out}");
    }

    #[test]
    fn policy_is_required_and_single() {
        let out = component(
            quote!(key = "t::a"),
            quote!(
                struct A;
            ),
        );
        assert!(error_of(&out).contains("missing persistence policy"), "{out}");
        let out = component(
            quote!(key = "t::a", persistent, derived),
            quote!(
                struct A;
            ),
        );
        assert!(error_of(&out).contains("one persistence policy"), "{out}");
    }

    #[test]
    fn key_is_required() {
        let out = component(
            quote!(persistent),
            quote!(
                struct A;
            ),
        );
        assert!(error_of(&out).contains("missing `key = \\\"...\\\"`"), "{out}");
        let out = component(
            quote!(key = "", persistent),
            quote!(
                struct A;
            ),
        );
        assert!(error_of(&out).contains("the key cannot be empty"), "{out}");
    }

    #[test]
    fn generics_reprs_and_enums_are_rejected() {
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                struct A<T>(T);
            ),
        );
        assert!(error_of(&out).contains("a component cannot be generic"), "{out}");
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                #[repr(packed)]
                struct A;
            ),
        );
        assert!(error_of(&out).contains("remove this `#[repr]`"), "{out}");
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                enum A {
                    B,
                }
            ),
        );
        assert!(error_of(&out).contains("a component is a struct"), "{out}");
    }

    #[test]
    fn value_enum_rules() {
        let out = value(
            TokenStream::new(),
            quote!(
                enum E {
                    A = 0,
                }
            ),
        )
        .to_string();
        assert!(error_of(&out).contains("states its width"), "{out}");
        let out = value(
            TokenStream::new(),
            quote!(
                #[repr(u8)]
                enum E {
                    A,
                }
            ),
        )
        .to_string();
        assert!(error_of(&out).contains("needs an explicit discriminant"), "{out}");
        let out = value(
            TokenStream::new(),
            quote!(
                #[repr(u8)]
                enum E {
                    A(u8) = 0,
                }
            ),
        )
        .to_string();
        assert!(error_of(&out).contains("data-carrying enums are not reflected"), "{out}");
    }

    #[test]
    fn a_failed_expansion_still_emits_the_item_without_field_attributes() {
        let out = component(
            quote!(key = "t::a"),
            quote!(
                struct A {
                    #[field(id = 1)]
                    x: u32,
                }
            ),
        );
        assert!(out.contains("struct A"), "{out}");
        assert!(!out.contains("field ("), "{out}");
    }

    #[test]
    fn existing_clone_and_copy_derives_are_not_repeated() {
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                #[derive(Clone, Copy, Debug)]
                struct A;
            ),
        );
        assert_eq!(out.matches("Clone").count(), 1, "{out}");
        assert_eq!(out.matches("Copy").count(), 1, "{out}");
    }
}
