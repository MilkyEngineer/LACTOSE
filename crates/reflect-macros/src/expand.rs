//! Expansion of `#[component]`, `#[resource]` and `#[value]`.
//!
//! The expansion names the reflect crate (`::lactose_reflect` unless `crate = "..."` says
//! otherwise) and `::core` only, and does no layout arithmetic of its own: sizes and offsets
//! are `size_of` and `offset_of!` calls the compiler evaluates in the invoking crate. The
//! component key is likewise hashed there, by a `const fn` call, never here.
//!
//! The expansion of a struct contains one `unsafe impl`, of `CheckedBitPattern`; see
//! [`checked_bit_pattern`] and `SAFETY.md` for why it is written here and why it is sound.
//! Everything else is safe code. Checks on attributes exist to give the author a clear
//! error; soundness never rests on them.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DataEnum, DeriveInput, Error, Fields, Ident, Index, LitInt, LitStr, Member,
    Path, Result, Token, Type, bracketed,
};

#[derive(Clone, Copy)]
pub(crate) enum Role {
    Component,
    Resource,
}

/// Which attribute is being expanded.
#[derive(Clone, Copy)]
enum Target {
    Reflected(Role),
    Value,
}

impl Target {
    fn attribute(self) -> &'static str {
        match self {
            Self::Reflected(Role::Component) => "component",
            Self::Reflected(Role::Resource) => "resource",
            Self::Value => "value",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Reflected(Role::Component) => "a component",
            Self::Reflected(Role::Resource) => "a reflected resource",
            Self::Value => "a `#[value]` type",
        }
    }

    fn usage(self) -> String {
        match self {
            Self::Reflected(_) => format!(
                "expected `#[{}(key = \"crate::name\", <policy>)]` where <policy> is \
                 `persistent`, `transient` or `derived`, optionally followed by `version = N`, \
                 `replicated`, `extracted`, `retired = [N, ...]`, `crate = \"path\"`",
                self.attribute()
            ),
            Self::Value => {
                "expected `#[value]`, optionally with `retired = [N, ...]` or `crate = \"path\"`"
                    .to_owned()
            }
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

/// Whether `attribute` is `#[name]`, in plain or raw (`r#name`) spelling. The compiler
/// treats both alike, so every check here must.
fn is_named(attribute: &Attribute, name: &str) -> bool {
    attribute.path().get_ident().is_some_and(|ident| ident.unraw() == name)
}

fn is_field(attribute: &Attribute) -> bool {
    is_named(attribute, "field")
}

fn strip_field_attributes(fields: &mut Fields) {
    fields.iter_mut().for_each(|field| field.attrs.retain(|a| !is_field(a)));
}

/// The first `#[cfg]` or `#[cfg_attr]` among `attributes`.
fn conditional(attributes: &[Attribute]) -> Option<&Attribute> {
    attributes.iter().find(|a| is_named(a, "cfg") || is_named(a, "cfg_attr"))
}

struct Args {
    /// Path of the reflect crate, as tokens and as the text `bytemuck`'s derives take.
    krate: TokenStream,
    bytemuck: String,
    key: Option<LitStr>,
    persistence: Option<Ident>,
    version: u32,
    replicated: bool,
    extracted: bool,
    retired: Vec<(u16, Span)>,
}

fn parse_args(target: Target, args: TokenStream) -> Result<Args> {
    let reflected = matches!(target, Target::Reflected(_));
    let parser = |input: ParseStream| -> Result<Args> {
        let mut krate: Option<LitStr> = None;
        let mut key = None;
        let mut persistence: Option<Ident> = None;
        let mut version = None;
        let mut replicated = false;
        let mut extracted = false;
        let mut retired: Option<Vec<(u16, Span)>> = None;

        while !input.is_empty() {
            let (name, span) = if input.peek(Token![crate]) {
                ("crate".to_owned(), input.parse::<Token![crate]>()?.span)
            } else {
                let ident: Ident = input.parse()?;
                (ident.to_string(), ident.span())
            };
            let repeated = || Error::new(span, format!("`{name}` is given twice"));
            match name.as_str() {
                "crate" => {
                    input.parse::<Token![=]>()?;
                    if krate.replace(input.parse()?).is_some() {
                        return Err(repeated());
                    }
                }
                "retired" => {
                    input.parse::<Token![=]>()?;
                    let list;
                    bracketed!(list in input);
                    let mut ids: Vec<(u16, Span)> = Vec::new();
                    for literal in Punctuated::<LitInt, Token![,]>::parse_terminated(&list)? {
                        let id = parse_field_id(&literal)?;
                        if ids.iter().any(|(seen, _)| *seen == id) {
                            return Err(Error::new(
                                literal.span(),
                                format!("field ID {id} is retired twice"),
                            ));
                        }
                        ids.push((id, literal.span()));
                    }
                    if retired.replace(ids).is_some() {
                        return Err(repeated());
                    }
                }
                "key" if reflected => {
                    input.parse::<Token![=]>()?;
                    let literal: LitStr = input.parse()?;
                    if literal.value().is_empty() {
                        return Err(Error::new(literal.span(), "the key cannot be empty"));
                    }
                    if key.replace(literal).is_some() {
                        return Err(repeated());
                    }
                }
                "version" if reflected => {
                    input.parse::<Token![=]>()?;
                    let number = input.parse::<LitInt>()?.base10_parse::<u32>()?;
                    if version.replace(number).is_some() {
                        return Err(repeated());
                    }
                }
                "replicated" if reflected => {
                    if std::mem::replace(&mut replicated, true) {
                        return Err(repeated());
                    }
                }
                "extracted" if reflected => {
                    if std::mem::replace(&mut extracted, true) {
                        return Err(repeated());
                    }
                }
                "persistent" | "transient" | "derived" if reflected => {
                    if persistence.replace(Ident::new(&name, span)).is_some() {
                        return Err(Error::new(
                            span,
                            "a type has one persistence policy: `persistent`, `transient` or \
                             `derived`",
                        ));
                    }
                }
                _ => {
                    return Err(Error::new(
                        span,
                        format!("unknown argument `{name}`; {}", target.usage()),
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        let (krate, bytemuck) = match krate {
            Some(literal) => (
                literal.parse::<Path>()?.into_token_stream(),
                format!("{}::bytemuck", literal.value()),
            ),
            None => (quote!(::lactose_reflect), "::lactose_reflect::bytemuck".to_owned()),
        };
        Ok(Args {
            krate,
            bytemuck,
            key,
            persistence,
            version: version.unwrap_or(1),
            replicated,
            extracted,
            retired: retired.unwrap_or_default(),
        })
    };
    parser.parse2(args)
}

fn parse_field_id(literal: &LitInt) -> Result<u16> {
    literal
        .base10_parse::<u16>()
        .map_err(|_| Error::new(literal.span(), "a field ID is an integer from 0 to 65535"))
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
fn take_fields(fields: &mut Fields, retired: &[(u16, Span)]) -> Result<Vec<FieldSpec>> {
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
            Some(ident) => (Member::Named(ident.clone()), ident.unraw().to_string()),
            None => (Member::Unnamed(Index::from(index)), index.to_string()),
        };

        if let Some(attribute) = conditional(&field.attrs) {
            fail(Error::new(
                attribute.path().span(),
                format!(
                    "field `{name}` cannot be conditional: a `#[cfg]` would give builds \
                     different layouts and schemas under one key"
                ),
            ));
        }

        let mut id = None;
        let mut padding = false;
        let mut malformed = false;
        for attribute in field.attrs.iter().filter(|a| is_field(a)) {
            let parsed = attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("id") {
                    let parsed = parse_field_id(&meta.value()?.parse()?)?;
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
                if retired.iter().any(|(retired, _)| *retired == id) {
                    fail(Error::new(
                        span,
                        format!(
                            "field `{name}` uses the retired field ID {id}: a retired ID is \
                             never given to another field"
                        ),
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
fn check_item(input: &DeriveInput, target: Target, args: &Args) -> Result<TokenStream> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(Error::new(
            input.generics.span(),
            format!(
                "{} cannot be generic: its key and layout must be one fixed thing \
                 (architecture section 6); declare one type per instantiation",
                target.noun()
            ),
        ));
    }
    if let Some(attribute) = input.attrs.iter().find(|a| is_named(a, "cfg_attr")) {
        return Err(Error::new(
            attribute.path().span(),
            format!(
                "`#[cfg_attr]` cannot be applied to {}: its attributes must be the same in \
                 every build",
                target.noun()
            ),
        ));
    }
    let mut has_clone = false;
    let mut has_copy = false;
    for attribute in &input.attrs {
        if is_named(attribute, "derive") {
            attribute.parse_nested_meta(|meta| {
                let last =
                    meta.path.segments.last().map(|segment| segment.ident.unraw().to_string());
                has_clone |= last.as_deref() == Some("Clone");
                has_copy |= last.as_deref() == Some("Copy");
                Ok(())
            })?;
        }
    }
    let krate = &args.krate;
    let clone = (!has_clone).then(|| quote!(::core::clone::Clone,));
    let copy = (!has_copy).then(|| quote!(::core::marker::Copy,));
    Ok(quote!(#clone #copy #krate::bytemuck::NoUninit))
}

/// The `CheckedBitPattern` impl of the struct `name`, and the checks that make it sound.
///
/// `bytemuck`'s own derive cannot be used: for a struct it derives `AnyBitPattern` on a
/// generated type without passing on `#[bytemuck(crate = ...)]`, so on the pinned version it
/// compiles only in a crate that depends on `bytemuck` under that name.
///
/// What this macro parsed is not evidence about the type: an attribute below it can rewrite
/// the struct afterwards, and attributes can be spelled in ways a parser misses. The
/// expansion therefore proves the impl against the type the compiler ends up with.
/// `crates/reflect-macros/SAFETY.md` gives the argument; in short, the constant assertions
/// and `__same_fields` below fail to compile unless the final type is a struct with exactly
/// the recorded fields, of exactly the recorded types, at the offsets of `__Bits`, with the
/// size and alignment of `__Bits`.
fn checked_bit_pattern(name: &Ident, fields: &[FieldSpec], args: &Args) -> TokenStream {
    let Args { krate, bytemuck, .. } = args;
    let bits: Vec<Ident> = (0..fields.len())
        .map(|index| Ident::new(&format!("f{index}"), Span::call_site()))
        .collect();
    let types: Vec<&Type> = fields.iter().map(|field| &field.ty).collect();
    let members: Vec<&Member> = fields.iter().map(|field| &field.member).collect();
    let changed = format!(
        "`{name}` is not the struct its attribute read: another attribute changed its \
         representation or its fields"
    );
    quote! {
        #[derive(::core::clone::Clone, ::core::marker::Copy, #krate::bytemuck::AnyBitPattern)]
        #[bytemuck(crate = #bytemuck)]
        #[repr(C)]
        pub struct __Bits {
            #(#bits: <#types as #krate::bytemuck::CheckedBitPattern>::Bits,)*
        }

        const _: () = ::core::assert!(
            ::core::mem::size_of::<#name>() == ::core::mem::size_of::<__Bits>()
                && ::core::mem::align_of::<#name>() == ::core::mem::align_of::<__Bits>(),
            #changed
        );
        #(
            const _: () = ::core::assert!(
                ::core::mem::offset_of!(#name, #members) == ::core::mem::offset_of!(__Bits, #bits),
                #changed
            );
        )*

        fn __type_of<T>(_: &T) -> ::core::marker::PhantomData<T> {
            ::core::marker::PhantomData
        }

        // Never called. It compiles only if the struct has exactly these fields (the pattern
        // has no `..`) and each has exactly the recorded type. The field's type is inferred
        // into `PhantomData<T>` first, with no expected type in sight, and only then compared:
        // passing the binding where the recorded type is expected would let it coerce.
        #[allow(dead_code, unused_variables)]
        fn __same_fields(value: &#name) {
            let #name { #(#members: #bits,)* } = value;
            #(
                let witness = __type_of(#bits);
                let _: ::core::marker::PhantomData<#types> = witness;
            )*
        }

        unsafe impl #krate::bytemuck::CheckedBitPattern for #name {
            type Bits = __Bits;

            #[inline]
            #[allow(unused_variables)]
            fn is_valid_bit_pattern(bits: &__Bits) -> bool {
                true #(&& <#types as #krate::bytemuck::CheckedBitPattern>::is_valid_bit_pattern(
                    &{ bits.#bits }
                ))*
            }
        }
    }
}

fn reject_repr(input: &DeriveInput) -> Result<()> {
    match input.attrs.iter().find(|a| is_named(a, "repr")) {
        Some(repr) => Err(Error::new(
            repr.path().span(),
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
    /// One `FieldId` expression per retired ID, comma-terminated.
    retired: TokenStream,
    /// Statements of a `visit` body.
    visits: TokenStream,
    /// Statements of a `visit_references` body.
    reference_visits: TokenStream,
}

/// With `own_ids` each visit reports the field's own ID, as a component does. Without it
/// each visit reports the `id` parameter in scope, the ID of the field that holds a
/// `#[value]` struct.
fn struct_parts(name: &Ident, fields: &[FieldSpec], args: &Args, own_ids: bool) -> StructParts {
    let krate = &args.krate;
    let mut parts = StructParts {
        layout_checks: TokenStream::new(),
        field_infos: TokenStream::new(),
        retired: TokenStream::new(),
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
                let field_id = quote!(#krate::__private::FieldId(#id));
                parts.field_infos.extend(quote_spanned! {type_span=>
                    #krate::FieldInfo::new::<#ty>(#field_id, #field_name, #offset),
                });
                let reported = if own_ids { field_id } else { quote!(id) };
                parts.visits.extend(quote! {
                    #krate::Field::visit(&self.#member, #reported, visitor);
                });
                parts.reference_visits.extend(quote! {
                    #krate::Field::visit_references(&mut self.#member, #reported, visitor);
                });
            }
            None => parts.field_infos.extend(quote_spanned! {type_span=>
                #krate::FieldInfo::padding::<#ty>(#field_name, #offset),
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
    for (id, _) in &args.retired {
        parts.retired.extend(quote!(#krate::__private::FieldId(#id),));
    }
    parts
}

fn try_reflected(role: Role, args: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let target = Target::Reflected(role);
    let mut input: DeriveInput = syn::parse2(item)?;
    let args = parse_args(target, args)?;
    let derives = check_item(&input, target, &args)?;
    let name = input.ident.clone();

    reject_repr(&input)?;
    let Data::Struct(data) = &mut input.data else {
        return Err(Error::new(
            name.span(),
            format!(
                "{} is a struct; mark an enum `#[value]` and hold it in a field",
                target.noun()
            ),
        ));
    };
    let key = args.key.as_ref().ok_or_else(|| {
        Error::new(Span::call_site(), format!("missing `key = \"...\"`; {}", target.usage()))
    })?;
    let persistence = args.persistence.as_ref().ok_or_else(|| {
        Error::new(
            Span::call_site(),
            format!(
                "missing persistence policy: add `persistent`, `transient` or `derived` to \
                 `#[{}(...)]`",
                target.attribute()
            ),
        )
    })?;
    let fields = take_fields(&mut data.fields, &args.retired)?;
    let StructParts { layout_checks, field_infos, retired, visits, reference_visits } =
        struct_parts(&name, &fields, &args, true);
    let checked = checked_bit_pattern(&name, &fields, &args);

    let persistence = match persistence.to_string().as_str() {
        "persistent" => quote!(Persistent),
        "transient" => quote!(Transient),
        _ => quote!(Derived),
    };
    let role = match role {
        Role::Component => quote!(Component),
        Role::Resource => quote!(Resource),
    };
    let Args { krate, bytemuck, version, replicated, extracted, .. } = &args;

    // The layout checks come first so that their message precedes the one `bytemuck`'s
    // derive gives for the same padding.
    Ok(quote! {
        #layout_checks

        #[derive(#derives)]
        #[bytemuck(crate = #bytemuck)]
        #[repr(C)]
        #input

        const _: () = {
            #checked

            const __FIELDS: &[#krate::FieldInfo] = &[#field_infos];
            const __SCHEMA: #krate::Schema<#name> = #krate::Schema::new(
                #key,
                #krate::TypeRole::#role,
                #krate::Persistence::#persistence,
                #version,
                #krate::Tracking { replicated: #replicated, extracted: #extracted },
                __FIELDS,
                &[#retired],
            );

            impl #krate::Reflect for #name {
                const KEY: #krate::__private::ComponentKey =
                    #krate::__private::ComponentKey::from_name(#key);
                const SCHEMA: &'static #krate::Schema<Self> = &__SCHEMA;

                #[allow(unused_variables)]
                fn visit<V: #krate::FieldVisitor>(&self, visitor: &mut V) {
                    #visits
                }

                #[allow(unused_variables)]
                fn visit_references<V: #krate::ReferenceVisitor>(&mut self, visitor: &mut V) {
                    #reference_visits
                }
            }

            impl #krate::#role for #name {}
        };
    })
}

fn try_value(args: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let target = Target::Value;
    let mut input: DeriveInput = syn::parse2(item)?;
    let args = parse_args(target, args)?;
    let mut derives = check_item(&input, target, &args)?;
    let krate = &args.krate;
    let bytemuck = &args.bytemuck;
    let name = input.ident.clone();
    let rust_name = name.unraw().to_string();
    let mut before = TokenStream::new();

    let implementation = match &mut input.data {
        Data::Struct(data) => {
            let fields = take_fields(&mut data.fields, &args.retired)?;
            reject_repr(&input)?;
            let StructParts { layout_checks, field_infos, retired, visits, reference_visits } =
                struct_parts(&name, &fields, &args, false);
            before = layout_checks;
            input.attrs.push(syn::parse_quote!(#[repr(C)]));
            let checked = checked_bit_pattern(&name, &fields, &args);
            quote! {
                #checked

                const __FIELDS: &[#krate::FieldInfo] = &[#field_infos];
                const __LAYOUT: #krate::StructInfo =
                    #krate::StructInfo::new::<#name>(#rust_name, __FIELDS, &[#retired]);

                impl #krate::Field for #name {
                    const KIND: #krate::FieldKind = #krate::FieldKind::Struct(&__LAYOUT);
                    const NAME: &'static str = #rust_name;

                    #[allow(unused_variables)]
                    fn visit<V: #krate::FieldVisitor>(
                        &self,
                        id: #krate::__private::FieldId,
                        visitor: &mut V,
                    ) {
                        #visits
                    }

                    #[allow(unused_variables)]
                    fn visit_references<V: #krate::ReferenceVisitor>(
                        &mut self,
                        id: #krate::__private::FieldId,
                        visitor: &mut V,
                    ) {
                        #reference_visits
                    }
                }
            }
        }
        Data::Enum(data) => {
            if let Some((_, span)) = args.retired.first() {
                return Err(Error::new(*span, "`retired` lists field IDs; an enum has none"));
            }
            let info = enum_variants(&input.attrs, &name, data, krate, &rust_name)?;
            // A fieldless enum's derive names no second crate path, so it is used as it is.
            derives.extend(quote!(, #krate::bytemuck::CheckedBitPattern));
            quote! {
                #info

                impl #krate::Field for #name {
                    const KIND: #krate::FieldKind = #krate::FieldKind::Enum(&__INFO);
                    const NAME: &'static str = #rust_name;

                    fn visit<V: #krate::FieldVisitor>(
                        &self,
                        id: #krate::__private::FieldId,
                        visitor: &mut V,
                    ) {
                        #krate::FieldVisitor::bytes(visitor, id, #krate::bytemuck::bytes_of(self));
                    }

                    fn visit_references<V: #krate::ReferenceVisitor>(
                        &mut self,
                        _: #krate::__private::FieldId,
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
        #before

        #[derive(#derives)]
        #[bytemuck(crate = #bytemuck)]
        #input

        const _: () = {
            #implementation
        };
    })
}

/// Checks a `#[value]` enum and returns its `__INFO` constant with the checks that hold
/// `bytemuck`'s derives to the final type.
fn enum_variants(
    attrs: &[Attribute],
    name: &Ident,
    data: &DataEnum,
    krate: &TokenStream,
    rust_name: &str,
) -> Result<TokenStream> {
    let mut width: Option<Ident> = None;
    for attribute in attrs.iter().filter(|a| is_named(a, "repr")) {
        attribute.parse_nested_meta(|meta| {
            let ident = meta.path.get_ident().map(IdentExt::unraw);
            match ident {
                Some(ident)
                    if (ident == "u8" || ident == "u16" || ident == "u32") && width.is_none() =>
                {
                    width = Some(ident);
                    Ok(())
                }
                _ => Err(meta.error(
                    "the only representation of a `#[value]` enum is its width: `u8`, `u16` \
                     or `u32`, once",
                )),
            }
        })?;
    }
    let Some(width) = width else {
        return Err(Error::new(
            name.span(),
            "a `#[value]` enum states its width: add `#[repr(u8)]`, `#[repr(u16)]` or \
             `#[repr(u32)]` below `#[value]`",
        ));
    };
    if data.variants.is_empty() {
        return Err(Error::new(name.span(), "a `#[value]` enum has at least one variant"));
    }

    let changed = format!(
        "`{name}` is not the enum its attribute read: another attribute changed its \
         representation or its variants"
    );
    let mut variants = TokenStream::new();
    let mut arms = TokenStream::new();
    // `bytemuck`'s derives decide validity and padding from the tokens they were given. These
    // checks hold them to the final type: it has the width's size and alignment, exactly the
    // recorded variants, and the recorded discriminants. See `SAFETY.md`.
    let mut checks = quote! {
        const _: () = ::core::assert!(
            ::core::mem::size_of::<#name>() == ::core::mem::size_of::<#width>()
                && ::core::mem::align_of::<#name>() == ::core::mem::align_of::<#width>(),
            #changed
        );
    };
    for variant in &data.variants {
        if let Some(attribute) = conditional(&variant.attrs) {
            return Err(Error::new(
                attribute.path().span(),
                format!(
                    "variant `{}` cannot be conditional: a `#[cfg]` would give builds different \
                     schemas",
                    variant.ident
                ),
            ));
        }
        if !matches!(variant.fields, Fields::Unit) {
            return Err(Error::new(
                variant.fields.span(),
                "data-carrying enums are not reflected: `#[value]` enums are fieldless; hold \
                 the data in fields beside the enum",
            ));
        }
        let Some((_, discriminant)) = &variant.discriminant else {
            return Err(Error::new(
                variant.span(),
                format!(
                    "variant `{}` needs an explicit discriminant (`{} = N`): its value is \
                     part of the save format",
                    variant.ident, variant.ident
                ),
            ));
        };
        let ident = &variant.ident;
        let variant_name = ident.unraw().to_string();
        variants.extend(quote! {
            #krate::VariantInfo { name: #variant_name, value: #name::#ident as u32 },
        });
        arms.extend(quote!(#name::#ident => {}));
        checks.extend(quote! {
            const _: () = ::core::assert!(#name::#ident as #width == #discriminant, #changed);
        });
    }
    Ok(quote! {
        #checks

        // Never called. It compiles only if the enum has exactly these variants, all unit.
        #[allow(dead_code)]
        fn __same_variants(value: &#name) {
            match *value {
                #arms
            }
        }

        const __INFO: #krate::EnumInfo = #krate::EnumInfo {
            name: #rust_name,
            size: ::core::mem::size_of::<#name>(),
            variants: &[#variants],
        };
    })
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

    /// The idents that follow a path-root `::` in `expansion`.
    fn path_roots(expansion: &str) -> Vec<&str> {
        let tokens: Vec<&str> = expansion.split_whitespace().collect();
        let mut roots = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            // A path root is `::` that does not follow a path segment or `>`.
            let after_segment = index > 0
                && tokens[index - 1]
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '>');
            if *token == "::" && !after_segment {
                roots.push(tokens[index + 1]);
            }
        }
        roots
    }

    fn sample_expansions(extra: TokenStream) -> [String; 4] {
        [
            component(
                quote!(key = "t::a", persistent, version = 2, replicated, retired = [9], #extra),
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
                quote!(key = "t::r", transient, #extra),
                quote!(
                    struct R(#[field(id = 1)] u64);
                ),
            )
            .to_string(),
            value(
                quote!(retired = [2], #extra),
                quote!(
                    struct V {
                        #[field(id = 1)]
                        x: u32,
                    }
                ),
            )
            .to_string(),
            value(
                extra,
                quote!(
                    #[repr(u8)]
                    enum E {
                        A = 0,
                    }
                ),
            )
            .to_string(),
        ]
    }

    #[test]
    fn expansion_names_only_the_reflect_crate_and_core() {
        for expansion in sample_expansions(TokenStream::new()) {
            assert!(!expansion.contains("compile_error"), "{expansion}");
            let roots = path_roots(&expansion);
            assert!(roots.contains(&"lactose_reflect"));
            for root in roots {
                assert!(matches!(root, "lactose_reflect" | "core"), "`::{root}` in {expansion}");
            }
            assert!(expansion.contains("\"::lactose_reflect::bytemuck\""));
        }
    }

    #[test]
    fn the_only_unsafe_in_an_expansion_is_the_checked_bit_pattern_impl_of_a_struct() {
        let [component, resource, value_struct, value_enum] = sample_expansions(TokenStream::new());
        for expansion in [component, resource, value_struct] {
            assert_eq!(expansion.matches("unsafe").count(), 1, "{expansion}");
            assert!(
                expansion.contains(
                    "unsafe impl :: lactose_reflect :: bytemuck :: CheckedBitPattern for"
                ),
                "{expansion}"
            );
        }
        assert!(!value_enum.contains("unsafe"), "{value_enum}");
    }

    #[test]
    fn crate_argument_replaces_every_reflect_path() {
        for expansion in sample_expansions(quote!(crate = "::lactose_ecs::reflect")) {
            assert!(!expansion.contains("compile_error"), "{expansion}");
            // The padding advice names `lactose_reflect::Padding` in prose; no path does.
            assert!(!expansion.contains(":: lactose_reflect"), "{expansion}");
            for root in path_roots(&expansion) {
                assert!(matches!(root, "lactose_ecs" | "core"), "`::{root}` in {expansion}");
            }
            assert!(expansion.contains("\"::lactose_ecs::reflect::bytemuck\""));
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
    fn retired_ids_cannot_be_reused_or_repeated() {
        let item = quote!(
            struct A {
                #[field(id = 3)]
                x: u32,
            }
        );
        let out = component(quote!(key = "t::a", persistent, retired = [1, 3]), item.clone());
        assert!(error_of(&out).contains("field `x` uses the retired field ID 3"), "{out}");
        let out = value(quote!(retired = [3]), item.clone()).to_string();
        assert!(error_of(&out).contains("field `x` uses the retired field ID 3"), "{out}");
        let out = component(quote!(key = "t::a", persistent, retired = [1, 1]), item);
        assert!(error_of(&out).contains("field ID 1 is retired twice"), "{out}");
        let out = value(
            quote!(retired = [1]),
            quote!(
                #[repr(u8)]
                enum E {
                    A = 0,
                }
            ),
        )
        .to_string();
        assert!(error_of(&out).contains("an enum has none"), "{out}");
    }

    #[test]
    fn conditional_fields_variants_and_attributes_are_rejected() {
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                struct A {
                    #[field(id = 1)]
                    x: u32,
                    #[cfg(feature = "big")]
                    #[field(id = 2)]
                    y: u32,
                }
            ),
        );
        assert!(error_of(&out).contains("field `y` cannot be conditional"), "{out}");
        let out = value(
            TokenStream::new(),
            quote!(
                struct V(
                    #[cfg_attr(test, allow(dead_code))]
                    #[field(id = 1)]
                    u32,
                );
            ),
        )
        .to_string();
        assert!(error_of(&out).contains("field `0` cannot be conditional"), "{out}");
        let out = value(
            TokenStream::new(),
            quote!(
                #[repr(u8)]
                enum E {
                    A = 0,
                    #[cfg(test)]
                    B = 1,
                }
            ),
        )
        .to_string();
        assert!(error_of(&out).contains("variant `B` cannot be conditional"), "{out}");
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                #[cfg_attr(all(), repr(align(8)))]
                struct A;
            ),
        );
        assert!(error_of(&out).contains("`#[cfg_attr]` cannot be applied to a component"), "{out}");
    }

    #[test]
    fn raw_spellings_of_attributes_are_recognised() {
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                #[r#repr(align(16))]
                struct A;
            ),
        );
        assert!(error_of(&out).contains("remove this `#[repr]`"), "{out}");
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                struct A {
                    #[r#cfg(test)]
                    #[r#field(id = 1)]
                    x: u32,
                }
            ),
        );
        assert!(error_of(&out).contains("field `x` cannot be conditional"), "{out}");
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                #[derive(r#Clone, Copy)]
                struct A {
                    #[r#field(id = 1)]
                    r#type: u32,
                }
            ),
        );
        assert!(!out.contains("compile_error"), "{out}");
        assert!(out.contains("; # [derive (:: lactose_reflect :: bytemuck :: NoUninit)]"), "{out}");
        assert!(!out.contains("field (id"), "{out}");
        assert!(out.contains("\"type\""), "{out}");
        let out = value(
            TokenStream::new(),
            quote!(
                #[repr(u8)]
                #[r#repr(align(4))]
                enum E {
                    A = 0,
                }
            ),
        )
        .to_string();
        assert!(error_of(&out).contains("the only representation of a `#[value]` enum"), "{out}");
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
    fn unknown_arguments_are_rejected_per_attribute() {
        let out = component(
            quote!(key = "t::a", persistent, saved),
            quote!(
                struct A;
            ),
        );
        assert!(error_of(&out).contains("unknown argument `saved`"), "{out}");
        let out = value(
            quote!(persistent),
            quote!(
                struct A;
            ),
        )
        .to_string();
        assert!(error_of(&out).contains("unknown argument `persistent`"), "{out}");
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
        let added = "# [derive (:: lactose_reflect :: bytemuck :: NoUninit)]";
        assert!(out.starts_with(added), "{out}");
        let out = component(
            quote!(key = "t::a", persistent),
            quote!(
                struct A;
            ),
        );
        let added = "# [derive (:: core :: clone :: Clone , :: core :: marker :: Copy , \
                     :: lactose_reflect :: bytemuck :: NoUninit)]";
        assert!(out.starts_with(added), "{out}");
    }
}
