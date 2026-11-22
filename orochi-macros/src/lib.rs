//! The `#[orochi::module]` attribute macro. Use it through the `orochi`
//! crate; see the documentation of `orochi::module`.
//!
//! The macro reads the module's `pub` items and keeps them as written, adds
//! pyo3's `#[pyclass]` to each `pub struct`, and generates an
//! `__orochi_init` function per module that registers everything with
//! `orochi::ModuleBuilder` / `orochi::ClassBuilder`, plus the `#[pymodule]`
//! entry point. All behaviour lives in the `orochi` library.

use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2, TokenTree};
use quote::{ToTokens, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, FnArg, GenericArgument, Ident, ImplItem, Item, ItemImpl, ItemMod,
    ItemStruct, LitStr, Pat, Path, PathArguments, ReturnType, Signature, Token, Type, Visibility,
};

/// Turns an inline Rust module into a Python extension module.
///
/// Use it as `#[orochi::module]`; the documentation is in the `orochi` crate.
#[proc_macro_attribute]
pub fn module(args: TokenStream, input: TokenStream) -> TokenStream {
    let original: TokenStream2 = input.clone().into();
    match expand(args.into(), input.into()) {
        Ok(tokens) => tokens.into(),
        Err(err) => {
            // Keep the user's items so that the only errors shown are ours.
            let err = err.to_compile_error();
            quote!(#original #err).into()
        }
    }
}

/// Hard keywords of Python 3, which cannot be attribute or parameter names.
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

fn expand(args: TokenStream2, input: TokenStream2) -> syn::Result<TokenStream2> {
    let name = parse_args(args)?;
    let item_mod: ItemMod = syn::parse2(input)?;
    if item_mod.content.is_none() {
        return Err(syn::Error::new_spanned(
            &item_mod,
            "#[orochi::module] needs an inline module: `mod name { ... }`",
        ));
    }
    let mut cx = Cx { errors: Vec::new() };
    let root_name = match name {
        Some(lit) => lit.value(),
        None => {
            let name = item_mod.ident.unraw().to_string();
            check_python_name(&mut cx, &name, &item_mod.ident, "module");
            name
        }
    };
    let mut out = process_module(&mut cx, item_mod, &root_name);
    if let Some(err) = cx.errors.into_iter().reduce(|mut a, b| {
        a.combine(b);
        a
    }) {
        return Err(err);
    }
    // The entry point Python calls when it imports the compiled library,
    // and an in-process loader for tests and embedding. `gil_used = true`:
    // the code has not been audited for the free-threaded build.
    let entry = quote! {
        #[::orochi::pyo3::pymodule(crate = "::orochi::pyo3", name = #root_name, gil_used = true)]
        fn __orochi_pymodule(
            module: &::orochi::pyo3::Bound<'_, ::orochi::pyo3::types::PyModule>,
        ) -> ::orochi::pyo3::PyResult<()> {
            ::orochi::init_module(module, __orochi_init)
        }

        /// Builds this module in-process and registers it in `sys.modules`
        /// (Python itself loads the extension through `PyInit_<name>`).
        #[doc(hidden)]
        pub fn __orochi_module() -> ::orochi::Result<::orochi::Object> {
            ::orochi::load_module(#root_name, __orochi_init)
        }
    };
    if let Some((_, items)) = &mut out.content {
        items.push(Item::Verbatim(entry));
    }
    Ok(out.into_token_stream())
}

/// `#[orochi::module]` or `#[orochi::module(name = "pyname")]`.
fn parse_args(args: TokenStream2) -> syn::Result<Option<LitStr>> {
    let mut name = None;
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("name") {
            let lit: LitStr = meta.value()?.parse()?;
            if syn::parse_str::<Ident>(&lit.value()).is_err() {
                return Err(syn::Error::new_spanned(&lit, "the module name must be an identifier"));
            }
            name = Some(lit);
            Ok(())
        } else {
            Err(meta.error("unknown argument; the only supported argument is `name = \"...\"`"))
        }
    });
    syn::parse::Parser::parse2(parser, args)?;
    Ok(name)
}

struct Cx {
    errors: Vec<syn::Error>,
}

impl Cx {
    fn error(&mut self, tokens: impl ToTokens, message: impl std::fmt::Display) {
        self.errors.push(syn::Error::new_spanned(tokens, message));
    }
}

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// Rejects Python keywords as exported names (`mylib.in` is a syntax error
/// in Python).
fn check_python_name(cx: &mut Cx, name: &str, tokens: impl ToTokens, what: &str) {
    if PYTHON_KEYWORDS.contains(&name) {
        cx.error(
            tokens,
            format!(
                "`{name}` is a Python keyword and cannot be the name of an exported {what}; \
                 rename it, or make the item private"
            ),
        );
    }
}

/// Python-visible names in one namespace, to reject duplicates.
#[derive(Default)]
struct Names(HashSet<String>);

impl Names {
    fn add(&mut self, cx: &mut Cx, name: &str, tokens: impl ToTokens, what: &str) {
        if !self.0.insert(name.to_owned()) {
            cx.error(tokens, format!("`{name}` is defined twice in this Python {what}"));
        }
    }
}

// ---------------------------------------------------------------- modules

/// An exported struct and the builder calls collected for it.
struct Class {
    ident: Ident,
    cfg: Vec<Attribute>,
    debug: bool,
    display: bool,
    eq: bool,
    hash: bool,
    has_ctor: bool,
    /// The constructor's doc comment, appended to the class doc.
    ctor_doc: Vec<Attribute>,
    names: Names,
    calls: Vec<TokenStream2>,
    /// Items emitted next to the struct (the constructor's #[pymethods]).
    extra: Vec<TokenStream2>,
}

fn process_module(cx: &mut Cx, mut item_mod: ItemMod, path: &str) -> ItemMod {
    let (brace, items) = item_mod.content.take().expect("inline module");
    let mut names = Names::default();
    let mut classes: Vec<Class> = Vec::new();
    let mut calls: Vec<TokenStream2> = Vec::new();
    let mut out_items: Vec<Item> = Vec::new();

    // Classes first, so that impl blocks anywhere in the module find them.
    let items: Vec<Item> = items
        .into_iter()
        .map(|item| match item {
            Item::Struct(s) if is_pub(&s.vis) => {
                let (s, class) = process_struct(cx, s, path);
                names.add(cx, &class.ident.unraw().to_string(), &s.ident, "module");
                classes.push(class);
                Item::Struct(s)
            }
            other => other,
        })
        .collect();
    // Types defined here that are not classes: their impl blocks stay Rust-only.
    let local_types: HashSet<String> = items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(s) => Some(&s.ident),
            Item::Enum(e) => Some(&e.ident),
            Item::Union(u) => Some(&u.ident),
            Item::Type(t) => Some(&t.ident),
            _ => None,
        })
        .map(|i| i.to_string())
        .collect();
    for item in &items {
        check_reserved(cx, item);
        let Item::Impl(imp) = item else { continue };
        if let Some(class) = impl_target(&mut classes, &imp.self_ty) {
            if imp.trait_.is_some() {
                trait_impl(cx, imp, class);
            } else {
                inherent_impl(cx, imp, class);
            }
        } else if imp.trait_.is_none() && has_pub_items(imp) && !is_local_type(&imp.self_ty, &local_types) {
            cx.error(
                &imp.self_ty,
                format!(
                    "the `pub` items of this impl block are not exported: `{}` is not a struct \
                     of this module; put the impl block in the module that defines the struct",
                    imp.self_ty.to_token_stream()
                ),
            );
        }
    }
    // The constructor's doc comment goes to the class doc (shown by `help`).
    let mut items = items;
    for class in &classes {
        if class.ctor_doc.is_empty() {
            continue;
        }
        for item in &mut items {
            if let Item::Struct(s) = item
                && s.ident == class.ident
            {
                if s.attrs.iter().any(|a| a.path().is_ident("doc")) {
                    s.attrs.push(syn::parse_quote!(#[doc = ""]));
                }
                s.attrs.extend(class.ctor_doc.iter().cloned());
            }
        }
    }

    for item in items {
        match &item {
            Item::Fn(f) if is_pub(&f.vis) => {
                let name = f.sig.ident.unraw().to_string();
                check_python_name(cx, &name, &f.sig.ident, "function");
                names.add(cx, &name, &f.sig.ident, "module");
                if let Some((checks, call)) = function_call(cx, &f.sig, &f.attrs, Kind::Function, None) {
                    let cfg = cfg_attrs(&f.attrs);
                    calls.push(quote!(#(#cfg)* { #checks #call }));
                }
            }
            Item::Mod(m) if is_pub(&m.vis) => {
                if m.content.is_none() {
                    cx.error(
                        m,
                        "a `pub mod` inside #[orochi::module] becomes a Python submodule and must \
                         be inline (`pub mod name { ... }`); make it private to keep it Rust-only",
                    );
                } else {
                    let name = m.ident.unraw().to_string();
                    check_python_name(cx, &name, &m.ident, "submodule");
                    names.add(cx, &name, &m.ident, "module");
                    let (ident, doc, cfg) = (&m.ident, doc_expr(&m.attrs), cfg_attrs(&m.attrs));
                    calls.push(quote! {
                        #(#cfg)* __orochi_m.submodule(#name, #doc, self::#ident::__orochi_init)?;
                    });
                }
            }
            Item::Const(c) if is_pub(&c.vis) => {
                if !c.generics.params.is_empty() {
                    cx.error(&c.generics, "generic constants cannot be exported to Python");
                }
                calls.extend(constant(cx, &mut names, &c.ident, &c.attrs, &c.ty, quote!(__orochi_m), quote!(self)));
            }
            Item::Static(s) if is_pub(&s.vis) => {
                if let syn::StaticMutability::Mut(_) = s.mutability {
                    cx.error(s, "`pub static mut` cannot be exported to Python; make it private");
                } else {
                    calls.extend(constant(cx, &mut names, &s.ident, &s.attrs, &s.ty, quote!(__orochi_m), quote!(self)));
                }
            }
            Item::Enum(e) if is_pub(&e.vis) => cx.error(
                &e.ident,
                "enums cannot be exported to Python yet; make the enum private (or `pub(crate)`), \
                 or wrap it in a `pub struct`",
            ),
            Item::Use(u) if is_pub(&u.vis) => cx.error(
                u,
                "`pub use` inside #[orochi::module] would mean re-exporting to Python, which is \
                 not supported; use a private `use` (or `pub(crate) use`), or wrap the item in a \
                 `pub fn`",
            ),
            Item::Trait(t) if is_pub(&t.vis) => cx.error(
                &t.ident,
                "traits cannot be exported to Python; make the trait private or `pub(crate)`, or \
                 define it outside the #[orochi::module]",
            ),
            Item::Type(t) if is_pub(&t.vis) => cx.error(
                &t.ident,
                "type aliases cannot be exported to Python; make the alias private or `pub(crate)`",
            ),
            Item::Union(u) if is_pub(&u.vis) => {
                cx.error(&u.ident, "unions cannot be exported to Python; make it private")
            }
            Item::ExternCrate(e) if is_pub(&e.vis) => {
                cx.error(e, "`pub extern crate` cannot be exported to Python; make it private")
            }
            _ => {}
        }
        out_items.push(match item {
            Item::Mod(m) if is_pub(&m.vis) && m.content.is_some() => {
                let path = format!("{path}.{}", m.ident.unraw());
                Item::Mod(process_module(cx, m, &path))
            }
            other => other,
        });
    }

    for class in &mut classes {
        out_items.extend(class.extra.drain(..).map(Item::Verbatim));
    }
    let class_calls = classes.into_iter().map(|class| class_calls(cx, class));
    let doc = doc_expr(&item_mod.attrs);
    out_items.push(Item::Verbatim(quote! {
        /// Registers this module's items with the builder (generated by
        /// `#[orochi::module]`).
        #[doc(hidden)]
        #[allow(clippy::all)]
        pub fn __orochi_init(__orochi_m: &::orochi::ModuleBuilder<'_>) -> ::orochi::Result<()> {
            __orochi_m.doc(#doc)?;
            #(#class_calls)*
            #(#calls)*
            ::core::result::Result::Ok(())
        }
    }));
    item_mod.content = Some((brace, out_items));
    item_mod
}

/// `builder.constant("NAME", Clone::clone(&owner::NAME))?;`
fn constant(
    cx: &mut Cx,
    names: &mut Names,
    ident: &Ident,
    attrs: &[Attribute],
    ty: &Type,
    builder: TokenStream2,
    owner: TokenStream2,
) -> Option<TokenStream2> {
    let name = ident.unraw().to_string();
    check_python_name(cx, &name, ident, "constant");
    names.add(cx, &name, ident, "namespace");
    let cfg = cfg_attrs(attrs);
    Some(quote_spanned! {ty.span()=>
        #(#cfg)* #builder.constant(#name, ::core::clone::Clone::clone(&#owner::#ident))?;
    })
}

/// The class an `impl` block is for, if its self type names a class of this
/// module (`Point` or `self::Point`).
fn impl_target<'a>(classes: &'a mut [Class], self_ty: &Type) -> Option<&'a mut Class> {
    classes.iter_mut().find(|c| is_self_type(self_ty, Some(&c.ident)) && !is_self_keyword(self_ty))
}

fn has_pub_items(imp: &ItemImpl) -> bool {
    imp.items.iter().any(|item| match item {
        ImplItem::Fn(f) => is_pub(&f.vis),
        ImplItem::Const(c) => is_pub(&c.vis),
        _ => false,
    })
}

/// `X` or `self::X` for a type `X` defined in this module.
fn is_local_type(ty: &Type, local: &HashSet<String>) -> bool {
    let Type::Path(tp) = ty else { return false };
    let segments = &tp.path.segments;
    let last = match segments.len() {
        1 if tp.qself.is_none() && tp.path.leading_colon.is_none() => &segments[0],
        2 if segments[0].ident == "self" => &segments[1],
        _ => return false,
    };
    local.contains(&last.ident.to_string())
}

/// Names starting with `__orochi` are reserved for the generated code.
fn check_reserved(cx: &mut Cx, item: &Item) {
    let mut idents: Vec<&Ident> = match item {
        Item::Fn(f) => vec![&f.sig.ident],
        Item::Struct(s) => vec![&s.ident],
        Item::Enum(e) => vec![&e.ident],
        Item::Union(u) => vec![&u.ident],
        Item::Const(c) => vec![&c.ident],
        Item::Static(s) => vec![&s.ident],
        Item::Mod(m) => vec![&m.ident],
        Item::Type(t) => vec![&t.ident],
        Item::Trait(t) => vec![&t.ident],
        Item::Impl(imp) => imp
            .items
            .iter()
            .filter_map(|i| match i {
                ImplItem::Fn(f) => Some(&f.sig.ident),
                ImplItem::Const(c) => Some(&c.ident),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    idents.retain(|i| i.unraw().to_string().starts_with("__orochi"));
    for ident in idents {
        cx.error(ident, "names starting with `__orochi` are reserved for the code #[orochi::module] generates");
    }
}

fn is_self_keyword(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp) if tp.path.is_ident("Self"))
}

// ---------------------------------------------------------------- classes

fn derives(attrs: &[Attribute]) -> HashSet<String> {
    let mut out = HashSet::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("derive")) {
        if let Ok(paths) = attr.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated) {
            out.extend(paths.iter().filter_map(|p| p.segments.last()).map(|s| s.ident.to_string()));
        }
    }
    out
}

fn reject_generics(cx: &mut Cx, generics: &syn::Generics, what: &str) {
    if !generics.params.is_empty() || generics.where_clause.is_some() {
        cx.error(
            generics,
            format!(
                "{what} exported to Python cannot have generic parameters or lifetimes; make it \
                 private (or `pub(crate)`), or remove the generics"
            ),
        );
    }
}

/// Adds the bare `#[pyclass]` and collects the field properties.
fn process_struct(cx: &mut Cx, mut s: ItemStruct, module: &str) -> (ItemStruct, Class) {
    reject_generics(cx, &s.generics, "a struct");
    check_python_name(cx, &s.ident.unraw().to_string(), &s.ident, "class");
    let d = derives(&s.attrs);
    let mut class = Class {
        ident: s.ident.clone(),
        cfg: cfg_attrs(&s.attrs),
        debug: d.contains("Debug"),
        display: false,
        eq: d.contains("PartialEq"),
        hash: d.contains("Hash"),
        has_ctor: false,
        ctor_doc: Vec::new(),
        names: Names::default(),
        calls: Vec::new(),
        extra: Vec::new(),
    };
    let ident = &s.ident;
    for field in s.fields.iter().filter(|f| is_pub(&f.vis)) {
        let Some(field_ident) = &field.ident else {
            cx.error(
                field,
                "fields of a tuple struct cannot be exposed to Python (they have no name); use a \
                 struct with named fields, or make this field private",
            );
            continue;
        };
        let name = field_ident.unraw().to_string();
        check_python_name(cx, &name, field_ident, "field");
        class.names.add(cx, &name, field_ident, "class");
        let (doc, cfg) = (doc_expr(&field.attrs), cfg_attrs(&field.attrs));
        // Spanned on the field type, which reports a type that is not a
        // `Field` there.
        class.calls.push(quote_spanned! {field.ty.span()=>
            #(#cfg)* __orochi_c.field(
                #name,
                #doc,
                |__orochi_self: &#ident| &__orochi_self.#field_ident,
                |__orochi_self: &mut #ident| &mut __orochi_self.#field_ident,
            )?;
        });
    }
    s.attrs.insert(0, syn::parse_quote!(#[::orochi::pyo3::pyclass(crate = "::orochi::pyo3", module = #module)]));
    (s, class)
}

/// `impl Debug/Display/PartialEq/Hash for Class` give special methods.
fn trait_impl(cx: &mut Cx, imp: &ItemImpl, class: &mut Class) {
    let Some(last) = imp.trait_.as_ref().and_then(|(path, _)| path.segments.last().cloned()) else {
        return;
    };
    let name = last.ident.to_string();
    if !["Debug", "Display", "PartialEq", "Hash"].contains(&name.as_str()) {
        return;
    }
    // `impl PartialEq<i64> for P` says nothing about `P == P`.
    let about_self = match &last.arguments {
        PathArguments::None => true,
        PathArguments::AngleBracketed(args) => {
            args.args.len() == 1
                && matches!(&args.args[0], GenericArgument::Type(t) if is_self_type(t, Some(&class.ident)))
        }
        PathArguments::Parenthesized(_) => false,
    };
    if !about_self {
        return;
    }
    if let Some(cfg) = cfg_attrs(&imp.attrs).first() {
        cx.error(
            cfg,
            format!(
                "`#[cfg]` on `impl {name} for {}` is not supported inside #[orochi::module]: the \
                 Python special methods depend on it; put the `#[cfg]` on the type, or implement \
                 the trait unconditionally",
                class.ident
            ),
        );
        return;
    }
    match name.as_str() {
        "Debug" => class.debug = true,
        "Display" => class.display = true,
        "PartialEq" => class.eq = true,
        _ => class.hash = true,
    }
}

/// Constructor, methods, static methods and constants of a class.
fn inherent_impl(cx: &mut Cx, imp: &ItemImpl, class: &mut Class) {
    let exported = imp.items.iter().any(|item| match item {
        ImplItem::Fn(f) => is_pub(&f.vis),
        ImplItem::Const(c) => is_pub(&c.vis),
        _ => false,
    });
    if exported {
        reject_generics(cx, &imp.generics, "an impl block with `pub` items");
    }
    let impl_cfg = cfg_attrs(&imp.attrs);
    let ident = class.ident.clone();
    for item in &imp.items {
        match item {
            ImplItem::Fn(f) if is_pub(&f.vis) => {
                let kind = if f.sig.ident == "new" && f.sig.receiver().is_none() {
                    if class.has_ctor {
                        cx.error(&f.sig.ident, "a class can have only one `pub fn new`");
                    }
                    class.has_ctor = true;
                    Kind::Constructor
                } else {
                    let name = f.sig.ident.unraw().to_string();
                    check_python_name(cx, &name, &f.sig.ident, "method");
                    class.names.add(cx, &name, &f.sig.ident, "class");
                    if f.sig.receiver().is_some() { Kind::Method } else { Kind::Static }
                };
                if let Some((checks, call)) = function_call(cx, &f.sig, &f.attrs, kind, Some(&ident)) {
                    let cfg = impl_cfg.iter().chain(&cfg_attrs(&f.attrs)).cloned().collect::<Vec<_>>();
                    if kind == Kind::Constructor {
                        class.calls.push(quote!(#(#cfg)* { #checks }));
                        let class_cfg = &class.cfg;
                        class.extra.push(quote!(#(#class_cfg)* #(#cfg)* #call));
                        class.ctor_doc = f.attrs.iter().filter(|a| a.path().is_ident("doc")).cloned().collect();
                    } else {
                        class.calls.push(quote!(#(#cfg)* { #checks #call }));
                    }
                }
            }
            ImplItem::Const(c) if is_pub(&c.vis) => {
                let mut attrs = impl_cfg.clone();
                attrs.extend(c.attrs.iter().cloned());
                class.calls.extend(constant(cx, &mut class.names, &c.ident, &attrs, &c.ty, quote!(__orochi_c), quote!(#ident)));
            }
            _ => {}
        }
    }
}

/// The builder calls of one class, in a block of their own.
fn class_calls(cx: &mut Cx, class: Class) -> TokenStream2 {
    let Class { ident, cfg, debug, display, eq, hash, calls, .. } = class;
    if hash && !eq {
        cx.error(
            &ident,
            format!(
                "`{ident}` implements `Hash` but not `PartialEq`: Python uses `__hash__` together \
                 with `__eq__`, so derive or implement `PartialEq` (and `Eq`) too"
            ),
        );
    }
    let specials = [
        (debug, quote!(__orochi_c.repr_debug()?;)),
        (display, quote!(__orochi_c.str_display()?;)),
        (eq, quote!(__orochi_c.eq()?;)),
        (hash, quote!(__orochi_c.hash()?;)),
    ]
    .into_iter()
    .filter_map(|(on, call)| on.then_some(call));
    quote! {
        #(#cfg)*
        {
            let __orochi_c = __orochi_m.class::<#ident>()?;
            #(#calls)*
            #(#specials)*
        }
    }
}

// ---------------------------------------------------------------- functions

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Function,
    Method,
    Static,
    Constructor,
}

/// The most parameters `orochi::Handler` supports.
const MAX_PARAMS: usize = 12;

/// Validates an exported function or method and returns compile-time checks
/// of its parameter and return types, and its builder call (for a
/// constructor: the `#[pymethods]` block with its `#[new]`).
fn function_call(
    cx: &mut Cx,
    sig: &Signature,
    attrs: &[Attribute],
    kind: Kind,
    class: Option<&Ident>,
) -> Option<(TokenStream2, TokenStream2)> {
    let before = cx.errors.len();
    let what = if kind == Kind::Function { "the function" } else { "the method" };
    if let Some(t) = &sig.asyncness {
        cx.error(t, format!("`async fn` cannot be exported to Python yet; make {what} private"));
    }
    if let syn::Safety::Unsafe(t) = &sig.safety {
        cx.error(t, format!("`unsafe fn` cannot be exported to Python; make {what} private"));
    }
    if let Some(abi) = &sig.abi {
        cx.error(abi, format!("`extern` functions cannot be exported to Python; make {what} private"));
    }
    if let Some(v) = &sig.variadic {
        cx.error(v, "variadic functions cannot be exported to Python");
    }
    reject_generics(cx, &sig.generics, if kind == Kind::Function { "a function" } else { "a method" });
    let name = sig.ident.unraw().to_string();
    if kind != Kind::Constructor && name.starts_with("__") && name.ends_with("__") {
        cx.error(
            &sig.ident,
            "special methods (`__name__`) are not supported yet; orochi generates `__repr__`, \
             `__str__`, `__eq__` and `__hash__` from `Debug`, `Display`, `PartialEq` and `Hash`",
        );
    }

    let mut params = Vec::new();
    let mut checks = Vec::new();
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(r) => match &r.kind {
                syn::ReceiverKind::Reference(_, Some(lt), _) if lt.ident != "_" => {
                    cx.error(lt, "explicit lifetimes are not supported in exported methods")
                }
                syn::ReceiverKind::Reference(..) | syn::ReceiverKind::Value => {}
                _ => cx.error(
                    r,
                    "only `&self`, `&mut self` and `self` receivers are supported in exported methods",
                ),
            },
            FnArg::Typed(pt) => {
                if let Some(attr) =
                    pt.attrs.iter().find(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
                {
                    cx.error(
                        attr,
                        "`#[cfg]` on a parameter of an exported function is not supported (the \
                         Python signature would change with it); put the `#[cfg]` on the whole \
                         function and write one version per configuration",
                    );
                }
                let Pat::Ident(pi) = &*pt.pat else {
                    cx.error(
                        &pt.pat,
                        "parameters of exported functions must be plain names (they become Python \
                         keyword argument names); destructure inside the body",
                    );
                    continue;
                };
                let name = pi.ident.unraw().to_string();
                check_python_name(cx, &name, &pi.ident, "parameter");
                check_type(cx, &pt.ty, "parameter");
                params.push(name);
                let ty = replace_self(pt.ty.to_token_stream(), class);
                checks.push(quote_spanned! {pt.ty.span()=>
                    ::orochi::__private::assert_arg::<#ty, _>();
                });
            }
        }
    }
    if sig.inputs.len() > MAX_PARAMS {
        cx.error(
            &sig.inputs,
            format!(
                "an exported function can have at most {MAX_PARAMS} parameters (`self` included); \
                 group some of them in a class or a tuple"
            ),
        );
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        check_type(cx, ty, "return type");
        check_borrowed_return(cx, ty);
        let span = ty.span();
        let ty = replace_self(ty.to_token_stream(), class);
        checks.push(if kind == Kind::Constructor {
            quote_spanned!(span=> ::orochi::__private::assert_new::<#class, #ty>();)
        } else {
            quote_spanned!(span=> ::orochi::__private::assert_ret::<#ty, _>();)
        });
    } else if kind == Kind::Constructor {
        checks.push(quote_spanned!(sig.ident.span()=> ::orochi::__private::assert_new::<#class, ()>();));
    }
    if cx.errors.len() != before {
        return None;
    }

    let ident = &sig.ident;
    let doc = doc_expr(attrs);
    // Spanned on the name: an unsupported type is reported on that type by
    // the checks above, and once more here, on the function.
    let span = ident.span();
    let call = match (kind, class) {
        (Kind::Function, _) => {
            quote_spanned!(span=> __orochi_m.function(#name, &[#(#params),*], #doc, self::#ident)?;)
        }
        (Kind::Constructor, Some(c)) => {
            // `#[new]` keeps pyo3's own `tp_new`, so no instance exists
            // without running `new`.
            let construct = quote_spanned!(span=> ::orochi::construct::<Self, _, _>(&[#(#params),*], #c::#ident, args, kwargs));
            let text_signature = text_signature(sig, &params);
            return Some((
                quote!(#(#checks)*),
                quote! {
                    #[::orochi::pyo3::pymethods(crate = "::orochi::pyo3")]
                    impl #c {
                        #[new]
                        #[pyo3(signature = (*args, **kwargs), text_signature = #text_signature)]
                        fn __orochi_new(
                            args: &::orochi::pyo3::Bound<'_, ::orochi::pyo3::types::PyTuple>,
                            kwargs: ::core::option::Option<
                                &::orochi::pyo3::Bound<'_, ::orochi::pyo3::types::PyDict>,
                            >,
                        ) -> ::orochi::pyo3::PyResult<Self> {
                            #construct
                        }
                    }
                },
            ));
        }
        (Kind::Method, Some(c)) => {
            quote_spanned!(span=> __orochi_c.method(#name, &[#(#params),*], #doc, #c::#ident)?;)
        }
        (_, Some(c)) => {
            quote_spanned!(span=> __orochi_c.static_method(#name, &[#(#params),*], #doc, #c::#ident)?;)
        }
        (_, None) => unreachable!("methods have a class"),
    };
    Some((quote!(#(#checks)*), call))
}

/// The constructor's `__text_signature__`: a trailing run of parameters
/// whose type is spelled `Option<..>` defaults to `None` (what the binder
/// does for every `Option`; an alias of `Option` shows without default).
fn text_signature(sig: &Signature, params: &[String]) -> String {
    let optional: Vec<bool> = sig
        .inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(pt) => Some(matches!(&*pt.ty, Type::Path(tp)
                if tp.qself.is_none() && tp.path.segments.last().is_some_and(|s| s.ident == "Option"))),
            FnArg::Receiver(_) => None,
        })
        .collect();
    let required = optional.iter().rposition(|o| !o).map_or(0, |i| i + 1);
    let parts: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, p)| if i < required { p.clone() } else { format!("{p}=None") })
        .collect();
    format!("({})", parts.join(", "))
}

/// `Self` in a method's types, spelled as the class (the checks are
/// generated outside the `impl` block).
fn replace_self(tokens: TokenStream2, class: Option<&Ident>) -> TokenStream2 {
    let Some(class) = class else { return tokens };
    tokens
        .into_iter()
        .map(|tt| match tt {
            TokenTree::Ident(i) if i == "Self" => TokenTree::Ident(Ident::new(&class.to_string(), i.span())),
            TokenTree::Group(g) => {
                let mut new = proc_macro2::Group::new(g.delimiter(), replace_self(g.stream(), Some(class)));
                new.set_span(g.span());
                TokenTree::Group(new)
            }
            other => other,
        })
        .collect()
}

/// Rejects `impl Trait` and explicit lifetimes (other than `'static`).
fn check_type(cx: &mut Cx, ty: &Type, what: &str) {
    // Lifetimes are tokenized as `'` (joint punct) followed by an ident.
    let mut found: Option<(Span, &str)> = None;
    let mut stack = vec![ty.to_token_stream()];
    while let Some(stream) = stack.pop() {
        let mut after_quote = false;
        for tt in stream {
            match &tt {
                TokenTree::Group(g) => stack.push(g.stream()),
                TokenTree::Ident(i) if after_quote && i != "static" && i != "_" => {
                    found.get_or_insert((i.span(), "lifetime"));
                }
                TokenTree::Ident(i) if i == "impl" => {
                    found.get_or_insert((i.span(), "impl"));
                }
                _ => {}
            }
            after_quote = matches!(&tt, TokenTree::Punct(p) if p.as_char() == '\'');
        }
    }
    match found {
        Some((span, "impl")) => cx.errors.push(syn::Error::new(
            span,
            format!("`impl Trait` is not supported in the {what} of an exported function; use a concrete type"),
        )),
        Some((span, _)) => cx.errors.push(syn::Error::new(
            span,
            format!(
                "explicit lifetimes are not supported in the {what} of an exported function; use \
                 elided references (`&str`, `&[T]`, `&Point`) or owned types"
            ),
        )),
        None => {}
    }
}

/// A returned reference (other than `&'static`) would borrow from the
/// arguments, which Python cannot hold on to.
fn check_borrowed_return(cx: &mut Cx, ty: &Type) {
    let mut stack = vec![ty.to_token_stream()];
    while let Some(stream) = stack.pop() {
        let tokens: Vec<TokenTree> = stream.into_iter().collect();
        for (i, tt) in tokens.iter().enumerate() {
            match tt {
                TokenTree::Group(g) => stack.push(g.stream()),
                TokenTree::Punct(p) if p.as_char() == '&' => {
                    let is_static = matches!(tokens.get(i + 1), Some(TokenTree::Punct(q)) if q.as_char() == '\'')
                        && matches!(tokens.get(i + 2), Some(TokenTree::Ident(s)) if s == "static");
                    if !is_static {
                        cx.errors.push(syn::Error::new(
                            p.span(),
                            "an exported function cannot return a borrowed value: return an owned \
                             value (`String` instead of `&str`, `Vec<T>` instead of `&[T]`); \
                             a method that modifies `self` returns `()`, and Python still holds \
                             the instance",
                        ));
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

/// `Self`, or the class's own name (`Point` / `self::Point`).
fn is_self_type(ty: &Type, class: Option<&Ident>) -> bool {
    let Type::Path(tp) = ty else { return false };
    if tp.qself.is_some() || tp.path.leading_colon.is_some() {
        return false;
    }
    let segments = &tp.path.segments;
    let last = match segments.len() {
        1 => &segments[0],
        2 if segments[0].ident == "self" => &segments[1],
        _ => return false,
    };
    matches!(last.arguments, PathArguments::None)
        && (last.ident == "Self" || class.is_some_and(|c| last.ident == *c))
}

// ---------------------------------------------------------------- attributes

fn cfg_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs.iter().filter(|a| a.path().is_ident("cfg")).cloned().collect()
}

/// A `&'static str` expression with the text of the doc comments: literal
/// lines lose the conventional single leading space; other values (e.g.
/// `#![doc = include_str!("README.md")]`) are included as they are.
fn doc_expr(attrs: &[Attribute]) -> TokenStream2 {
    let mut parts = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("doc")) {
        let syn::Meta::NameValue(nv) = &attr.meta else {
            continue;
        };
        if !parts.is_empty() {
            parts.push(quote!("\n"));
        }
        match &nv.value {
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => {
                let text = s
                    .value()
                    .split('\n')
                    .map(|line| line.strip_prefix(' ').unwrap_or(line))
                    .collect::<Vec<_>>()
                    .join("\n");
                parts.push(quote!(#text));
            }
            other => parts.push(other.to_token_stream()),
        }
    }
    quote!(::core::concat!(#(#parts),*))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The error messages `#[orochi::module]` reports for `input`.
    fn errors(input: TokenStream2) -> Vec<String> {
        match expand(quote!(), input) {
            Ok(_) => Vec::new(),
            Err(err) => err.into_iter().map(|e| e.to_string()).collect(),
        }
    }

    #[track_caller]
    fn assert_error(input: TokenStream2, expected: &str) {
        let errors = errors(input);
        assert!(
            errors.len() == 1 && errors[0].contains(expected),
            "expected one error containing {expected:?}, got {errors:?}"
        );
    }

    #[test]
    fn unsupported_items_are_reported() {
        assert_error(quote!(mod m { pub enum E { A } }), "enums cannot be exported");
        assert_error(quote!(mod m { pub fn r#in() {} }), "`in` is a Python keyword");
        assert_error(quote!(mod m { pub fn f(r#in: i64) {} }), "`in` is a Python keyword");
        assert_error(quote!(mod m { pub fn f<T>(x: T) {} }), "cannot have generic parameters");
        assert_error(quote!(mod m { pub async fn f() {} }), "`async fn` cannot be exported");
        assert_error(quote!(mod m { pub fn f((a, b): (i64, i64)) {} }), "must be plain names");
        assert_error(quote!(mod m { pub fn f(x: impl Fn()) {} }), "`impl Trait` is not supported");
        assert_error(quote!(mod m { pub fn f(x: Cow<'a, str>) {} }), "explicit lifetimes");
        assert_error(quote!(mod m { pub struct S(pub i64); }), "tuple struct");
        assert_error(quote!(mod m { pub use std::fmt; }), "`pub use`");
        assert_error(quote!(mod m { pub mod sub; }), "must be inline");
        assert_error(
            quote!(mod m { #[derive(Hash)] pub struct S; }),
            "implements `Hash` but not `PartialEq`",
        );
        assert_error(
            quote!(mod m { pub struct S { pub x: i64 } impl S { pub fn x(&self) -> i64 { 1 } } }),
            "`x` is defined twice",
        );
        assert_error(
            quote!(mod m { pub struct S; impl S { pub fn __len__(&self) -> usize { 0 } } }),
            "special methods",
        );
        assert_error(quote!(mod m { pub fn __orochi_init() {} }), "reserved");
        assert_error(quote!(mod m { pub struct S; impl S { fn __orochi_new() {} } }), "reserved");
        assert_error(
            quote!(mod m { pub mod sub { pub struct Q; } impl sub::Q { pub fn new() -> Self { sub::Q } } }),
            "is not a struct of this module",
        );
        assert_error(quote!(mod m { use x::Q; impl Q { pub fn f(&self) {} } }), "is not a struct of this module");
        // Impl blocks of private types stay Rust-only.
        assert!(errors(quote!(mod m { struct Q; impl Q { pub fn f(&self) {} } })).is_empty());
        assert_error(
            quote!(mod m { pub fn f(a: i8, b: i8, c: i8, d: i8, e: i8, f: i8, g: i8, h: i8, i: i8, j: i8, k: i8, l: i8, m: i8) {} }),
            "at most 12 parameters",
        );
    }

    #[test]
    fn anonymous_lifetimes_are_accepted() {
        assert!(errors(quote!(mod m { pub fn f(x: Cow<'_, str>) -> usize { x.len() } })).is_empty());
        assert_error(quote!(mod m { pub fn f(x: &'_ str) -> &'_ str { x } }), "cannot return a borrowed value");
    }

    #[test]
    fn borrowed_returns_are_reported() {
        assert_error(quote!(mod m { pub fn f(s: &str) -> &str { s } }), "cannot return a borrowed value");
        assert_error(
            quote!(mod m { pub struct S; impl S { pub fn me(&mut self) -> &mut Self { self } } }),
            "cannot return a borrowed value",
        );
        assert_error(
            quote!(mod m { pub fn f(v: &[i64]) -> Option<&i64> { v.first() } }),
            "cannot return a borrowed value",
        );
        assert!(errors(quote!(mod m { pub fn f() -> &'static str { "" } })).is_empty());
    }

    #[test]
    fn supported_items_expand() {
        let out = expand(
            quote!(),
            quote!(mod m {
                /// Doc.
                pub fn add(a: i64, b: Option<i64>) -> i64 { a + b.unwrap_or(1) }
                /// A point.
                #[derive(Debug, PartialEq, Eq, Hash)]
                pub struct P { pub x: f64 }
                impl P {
                    pub const ZERO: f64 = 0.0;
                    /// Makes a P.
                    pub fn new(x: f64, y: Option<f64>) -> Self { P { x } }
                    pub fn get(&self, other: &Self) -> f64 { self.x + other.x }
                    pub fn origin() -> Self { P { x: 0.0 } }
                }
                pub mod sub { pub fn f() {} }
            }),
        )
        .unwrap()
        .to_string();
        for expected in [
            "pyclass (crate = \"::orochi::pyo3\" , module = \"m\")",
            "__orochi_m . function (\"add\" , & [\"a\" , \"b\"]",
            "# [new] # [pyo3 (signature = (* args , * * kwargs) , text_signature = \"(x, y=None)\")]",
            "fn __orochi_new",
            ":: orochi :: construct :: < Self , _ , _ > (& [\"x\" , \"y\"] , P :: new , args , kwargs)",
            "# [doc = \"\"] # [doc = r\" Makes a P.\"]",
            "__orochi_c . method (\"get\" , & [\"other\"]",
            "assert_arg :: < & P , _ >",
            "__orochi_c . static_method (\"origin\"",
            "__orochi_c . constant (\"ZERO\"",
            "__orochi_c . field (\"x\"",
            "__orochi_c . repr_debug () ? ; __orochi_c . eq () ? ; __orochi_c . hash () ? ;",
            "__orochi_m . submodule (\"sub\"",
            "pymodule (crate = \"::orochi::pyo3\" , name = \"m\"",
        ] {
            assert!(out.contains(expected), "missing {expected:?} in\n{out}");
        }
    }
}
