//! The `#[orochi::module]` attribute macro. Use it through the `orochi`
//! crate; see the documentation of `orochi::module`.
//!
//! The macro keeps every item of the module as written (so it stays usable
//! from Rust) and adds glue: pyo3 `#[pyclass]` on exported types, plus
//! generated `#[pyfunction]` / `#[pymethods]` wrappers that convert arguments
//! and return values with orochi's `FromObject` / `IntoObject` through
//! `orochi::__private`.

use std::collections::{HashMap, HashSet};

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2, TokenTree};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Fields, FnArg, GenericArgument, Ident, ImplItem, Item, ItemEnum, ItemFn, ItemImpl,
    ItemMod, ItemStruct, LitStr, Pat, Path, PathArguments, ReturnType, Signature, Token, Type,
    Visibility,
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

const PY: &str = "::orochi::pyo3";

/// Hard keywords of Python 3, which cannot be used as attribute or
/// parameter names.
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
    let mut cx = Cx {
        modules: HashMap::new(),
        errors: Vec::new(),
    };
    let root_name = match name {
        Some(lit) => {
            check_python_name(&mut cx, &lit.value(), &lit, "module");
            lit.value()
        }
        None => {
            let name = item_mod.ident.unraw().to_string();
            check_python_name(&mut cx, &name, &item_mod.ident, "module");
            name
        }
    };
    collect_classes(&item_mod, Vec::new(), &mut cx.modules);
    let out = process_module(&mut cx, item_mod, &root_name, &[]);
    match cx.errors.into_iter().reduce(|mut a, b| {
        a.combine(b);
        a
    }) {
        Some(err) => Err(err),
        None => Ok(out.into_token_stream()),
    }
}

/// `#[orochi::module]` or `#[orochi::module(name = "pyname")]`.
fn parse_args(args: TokenStream2) -> syn::Result<Option<LitStr>> {
    if args.is_empty() {
        return Ok(None);
    }
    let mut name = None;
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("name") {
            let lit: LitStr = meta.value()?.parse()?;
            if syn::parse_str::<Ident>(&lit.value()).is_err() {
                return Err(syn::Error::new_spanned(
                    &lit,
                    "the module name must be an identifier",
                ));
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
    /// Exported structs of each module, by module path relative to the root
    /// (`[]` is the root, `["geometry"]` its submodule). Enums are not
    /// included: their members are immutable singletons, fine in a field.
    modules: HashMap<Vec<String>, HashSet<String>>,
    errors: Vec<syn::Error>,
}

impl Cx {
    fn error(&mut self, tokens: impl ToTokens, message: impl std::fmt::Display) {
        self.errors.push(syn::Error::new_spanned(tokens, message));
    }

    /// The struct class `path` names when written inside module `current`: a
    /// struct of that module (`P`, `self::P`), or of a module reached with
    /// `super::` / submodule names. `None` for anything else.
    fn resolve_class(&self, current: &[String], path: &Path) -> Option<String> {
        if path.leading_colon.is_some() {
            return None;
        }
        let segments: Vec<_> = path.segments.iter().collect();
        let (last, dirs) = segments.split_last()?;
        let mut dir = current.to_vec();
        for seg in dirs.iter() {
            if !matches!(seg.arguments, PathArguments::None) {
                return None;
            }
            let name = seg.ident.unraw().to_string();
            match name.as_str() {
                "self" => {}
                "super" => {
                    dir.pop()?;
                }
                "crate" | "Self" => return None,
                _ => dir.push(name),
            }
        }
        if !matches!(last.arguments, PathArguments::None) {
            return None;
        }
        let name = last.ident.unraw().to_string();
        self.modules
            .get(&dir)
            .is_some_and(|classes| classes.contains(&name))
            .then_some(name)
    }
}

fn collect_classes(
    item_mod: &ItemMod,
    path: Vec<String>,
    out: &mut HashMap<Vec<String>, HashSet<String>>,
) {
    let mut classes = HashSet::new();
    for item in item_mod.content.iter().flat_map(|(_, items)| items) {
        match item {
            Item::Struct(s) if is_pub(&s.vis) => {
                classes.insert(s.ident.unraw().to_string());
            }
            Item::Mod(m) if is_pub(&m.vis) => {
                let mut sub = path.clone();
                sub.push(m.ident.unraw().to_string());
                collect_classes(m, sub, out);
            }
            _ => {}
        }
    }
    out.insert(path, classes);
}

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// Rejects Python keywords as exported names (`r#in` would be unusable from
/// Python: `mylib.in` is a syntax error), and names reserved for orochi
/// (`_orochi_stubs` is the module's stub helper, `__orochi_*` generated code).
fn check_python_name(cx: &mut Cx, name: &str, tokens: impl ToTokens, what: &str) {
    if name.starts_with("_orochi") || name.starts_with("__orochi") {
        cx.error(
            tokens,
            format!(
                "`{name}` cannot be the name of an exported {what}: names starting with \
                 `_orochi` or `__orochi` are reserved for orochi; rename it, or make the item \
                 private"
            ),
        );
    } else if PYTHON_KEYWORDS.contains(&name) {
        cx.error(
            tokens,
            format!(
                "`{name}` is a Python keyword and cannot be the name of an exported {what}; \
                 rename it, or make the item private"
            ),
        );
    }
}

// ---------------------------------------------------------------- modules

/// Python-visible names in one namespace, to reject duplicates.
#[derive(Default)]
struct Names(HashMap<String, ()>);

impl Names {
    fn add(&mut self, cx: &mut Cx, name: &str, tokens: impl ToTokens, what: &str) {
        if self.0.insert(name.to_owned(), ()).is_some() {
            cx.error(
                tokens,
                format!("`{name}` is defined twice in this Python {what}"),
            );
        }
    }
}

struct Class {
    ident: Ident,
    py_name: String,
    /// Unit variants (with their `#[cfg]`s) of an exported enum.
    variants: Option<Vec<(Ident, Vec<Attribute>)>>,
    cfg: Vec<Attribute>,
    clone: bool,
    debug: bool,
    display: bool,
    eq: bool,
    hash: bool,
    has_ctor: bool,
    names: Names,
    /// Items of the generated `#[pymethods]` block.
    methods: Vec<TokenStream2>,
    /// Doc comments, as a `&'static str` expression.
    doc: TokenStream2,
    /// Statements adding members to `__orochi_class` in the generated stub
    /// description.
    stub_members: Vec<TokenStream2>,
}

impl Class {
    fn is_enum(&self) -> bool {
        self.variants.is_some()
    }
}

fn process_module(cx: &mut Cx, mut item_mod: ItemMod, full_name: &str, path: &[String]) -> ItemMod {
    let is_root = path.is_empty();
    let (brace, items) = item_mod.content.take().expect("inline module");
    let mut names = Names::default();
    let mut classes: Vec<Class> = Vec::new();
    let mut out_items: Vec<Item> = Vec::new();
    let mut glue: Vec<TokenStream2> = Vec::new();
    let mut init: Vec<TokenStream2> = Vec::new();
    let mut wrappers: HashSet<String> = HashSet::new();
    // Statements describing the module for `.pyi` stubs, in source order.
    let mut stub_items: Vec<TokenStream2> = Vec::new();
    let hint = quote!(::orochi::__private::hint::Hint);

    // Classes first, so that impl blocks anywhere in the module find them.
    let mut items_pending = Vec::new();
    for item in items {
        match item {
            Item::Struct(s) if is_pub(&s.vis) => {
                let (s, class) = process_struct(cx, s, full_name, path);
                names.add(cx, &class.py_name, &s.ident, "module");
                classes.push(class);
                items_pending.push(Item::Struct(s));
            }
            Item::Enum(e) if is_pub(&e.vis) => {
                let (e, class) = process_enum(cx, e, full_name);
                names.add(cx, &class.py_name, &e.ident, "module");
                classes.push(class);
                items_pending.push(Item::Enum(e));
            }
            other => items_pending.push(other),
        }
    }
    // Trait impls tell which special methods to generate.
    for item in &items_pending {
        if let Item::Impl(imp) = item
            && let Some((trait_path, _)) = &imp.trait_
            && let Some(class) = impl_target(&mut classes, &imp.self_ty)
        {
            let Some(last) = trait_path.segments.last() else {
                continue;
            };
            let name = last.ident.to_string();
            if !["Clone", "Debug", "Display", "PartialEq", "Hash"].contains(&name.as_str()) {
                continue;
            }
            // `impl PartialEq<i64> for P` says nothing about `P == P`.
            let about_self = match &last.arguments {
                PathArguments::None => true,
                PathArguments::AngleBracketed(args) => {
                    args.args.len() == 1
                        && matches!(&args.args[0], GenericArgument::Type(t)
                            if is_self_type(t, Some(&class.ident)))
                }
                PathArguments::Parenthesized(_) => false,
            };
            if !about_self {
                continue;
            }
            if !cfg_attrs(&imp.attrs).is_empty() {
                cx.error(
                    &imp.attrs[0],
                    format!(
                        "`#[cfg]` on `impl {name} for {}` is not supported inside \
                         #[orochi::module]: the generated Python special methods depend on it; \
                         put the `#[cfg]` on the type, or implement the trait unconditionally",
                        class.ident
                    ),
                );
                continue;
            }
            match name.as_str() {
                "Clone" => class.clone = true,
                "Debug" => class.debug = true,
                "Display" => class.display = true,
                "PartialEq" => class.eq = true,
                "Hash" => class.hash = true,
                _ => {}
            }
        }
    }
    for class in &classes {
        if !class.is_enum() && class.hash && !class.eq {
            cx.error(
                &class.ident,
                format!(
                    "`{}` implements `Hash` but not `PartialEq`: Python uses `__hash__` together \
                     with `__eq__`, so derive or implement `PartialEq` (and `Eq`) too",
                    class.ident
                ),
            );
        }
    }

    for item in items_pending {
        match item {
            Item::Fn(f) if is_pub(&f.vis) => {
                let py_name = f.sig.ident.unraw().to_string();
                check_python_name(cx, &py_name, &f.sig.ident, "function");
                names.add(cx, &py_name, &f.sig.ident, "module");
                let wrapper = wrapper_ident("fn", &f.sig.ident, &mut wrappers);
                if let Some((tokens, stub)) = function_wrapper(cx, &f, &wrapper) {
                    let cfg = cfg_attrs(&f.attrs);
                    glue.push(tokens);
                    stub_items.push(quote!(#(#cfg)* __orochi_module.function(#stub);));
                    init.push(quote! {
                        #(#cfg)*
                        ::orochi::__private::add_function(
                            __orochi_module,
                            ::orochi::pyo3::wrap_pyfunction!(#wrapper, __orochi_module)?,
                        )?;
                    });
                }
                out_items.push(Item::Fn(f));
            }
            Item::Impl(imp) => {
                if imp.trait_.is_none()
                    && let Some(class) = impl_target(&mut classes, &imp.self_ty)
                {
                    process_inherent_impl(cx, &imp, class);
                }
                out_items.push(Item::Impl(imp));
            }
            Item::Mod(m) if is_pub(&m.vis) => {
                if m.content.is_none() {
                    cx.error(
                        &m,
                        "a `pub mod` inside #[orochi::module] becomes a Python submodule and must \
                         be inline (`pub mod name { ... }`); make it private to keep it Rust-only",
                    );
                    out_items.push(Item::Mod(m));
                    continue;
                }
                let name = m.ident.unraw().to_string();
                check_python_name(cx, &name, &m.ident, "submodule");
                names.add(cx, &name, &m.ident, "module");
                let sub_full = format!("{full_name}.{name}");
                let doc = doc_expr(&m.attrs);
                let ident = m.ident.clone();
                let cfg = cfg_attrs(&m.attrs);
                init.push(quote! {
                    #(#cfg)*
                    ::orochi::__private::add_submodule(
                        __orochi_module, #name, #sub_full, #doc, #ident::__orochi_init,
                    )?;
                });
                stub_items.push(quote! {
                    #(#cfg)*
                    __orochi_module.module(#ident::__orochi_stub_module());
                });
                let mut sub_path = path.to_vec();
                sub_path.push(name);
                out_items.push(Item::Mod(process_module(cx, m, &sub_full, &sub_path)));
            }
            Item::Const(c) if is_pub(&c.vis) => {
                if !c.generics.params.is_empty() {
                    cx.error(
                        &c.generics,
                        "generic constants cannot be exported to Python",
                    );
                }
                let name = c.ident.unraw().to_string();
                check_python_name(cx, &name, &c.ident, "constant");
                names.add(cx, &name, &c.ident, "module");
                let ident = &c.ident;
                let cfg = cfg_attrs(&c.attrs);
                let ty = &c.ty;
                stub_items.push(quote! {
                    #(#cfg)*
                    __orochi_module.constant(#name, (&&&#hint::<#ty>::new()).output());
                });
                init.push(quote_spanned! {c.ty.span()=>
                    #(#cfg)*
                    ::orochi::__private::add_value(__orochi_module, #name, &#ident)?;
                });
                out_items.push(Item::Const(c));
            }
            Item::Static(s) if is_pub(&s.vis) => {
                if let syn::StaticMutability::Mut(_) = s.mutability {
                    cx.error(
                        &s,
                        "`pub static mut` cannot be exported to Python; make it private",
                    );
                } else {
                    let name = s.ident.unraw().to_string();
                    check_python_name(cx, &name, &s.ident, "static");
                    names.add(cx, &name, &s.ident, "module");
                    let ident = &s.ident;
                    let cfg = cfg_attrs(&s.attrs);
                    let ty = &s.ty;
                    stub_items.push(quote! {
                        #(#cfg)*
                        __orochi_module.constant(#name, (&&&#hint::<#ty>::new()).output());
                    });
                    init.push(quote_spanned! {s.ty.span()=>
                        #(#cfg)*
                        ::orochi::__private::add_value(__orochi_module, #name, &#ident)?;
                    });
                }
                out_items.push(Item::Static(s));
            }
            Item::Use(u) if is_pub(&u.vis) => {
                cx.error(
                    &u,
                    "`pub use` inside #[orochi::module] would mean re-exporting to Python, which \
                     is not supported; use a private `use` (or `pub(crate) use`), or wrap the \
                     item in a `pub fn`",
                );
                out_items.push(Item::Use(u));
            }
            Item::Trait(t) if is_pub(&t.vis) => {
                cx.error(
                    &t.ident,
                    "traits cannot be exported to Python; make the trait private or \
                     `pub(crate)`, or define it outside the #[orochi::module]",
                );
                out_items.push(Item::Trait(t));
            }
            Item::Type(t) if is_pub(&t.vis) => {
                cx.error(
                    &t.ident,
                    "type aliases cannot be exported to Python; make the alias private or \
                     `pub(crate)`",
                );
                out_items.push(Item::Type(t));
            }
            Item::Union(u) if is_pub(&u.vis) => {
                cx.error(
                    &u.ident,
                    "unions cannot be exported to Python; make it private",
                );
                out_items.push(Item::Union(u));
            }
            Item::Struct(s) if is_pub(&s.vis) => {
                let cfg = cfg_attrs(&s.attrs);
                let ident = &s.ident;
                stub_items.push(quote! {
                    #(#cfg)*
                    __orochi_module.class(#ident::__orochi_stub_class());
                });
                out_items.push(Item::Struct(s));
            }
            Item::Enum(e) if is_pub(&e.vis) => {
                let cfg = cfg_attrs(&e.attrs);
                let ident = &e.ident;
                stub_items.push(quote! {
                    #(#cfg)*
                    __orochi_module.class(#ident::__orochi_stub_class());
                });
                out_items.push(Item::Enum(e));
            }
            Item::ExternCrate(e) if is_pub(&e.vis) => {
                cx.error(
                    &e,
                    "`pub extern crate` cannot be exported to Python; make it private",
                );
                out_items.push(Item::ExternCrate(e));
            }
            other => out_items.push(other),
        }
    }

    for class in classes {
        let ident = &class.ident;
        let cfg = &class.cfg;
        init.push(quote! {
            #(#cfg)*
            ::orochi::__private::add_class::<#ident>(__orochi_module)?;
        });
        if class.is_enum() {
            init.push(quote! {
                #(#cfg)*
                #ident::__orochi_members(__orochi_module.py())?;
            });
        }
        glue.push(class_glue(class, full_name));
    }

    let doc = doc_expr(&item_mod.attrs);
    let short_name = full_name.rsplit('.').next().unwrap_or(full_name);
    let uses = hint_uses();
    let stub_module = quote! {
        /// Describes this module for `.pyi` stubs.
        #[doc(hidden)]
        #[allow(unused_mut, clippy::all)]
        pub fn __orochi_stub_module() -> ::orochi::__private::stub::Module {
            #uses
            let mut __orochi_module = ::orochi::__private::stub::Module::new(#short_name, #doc);
            #(#stub_items)*
            __orochi_module
        }
    };
    let mut entry = TokenStream2::new();
    if is_root {
        // The hidden Python-side accessor of the stubs.
        init.push(quote! {
            ::orochi::__private::add_stubs_helper(__orochi_module, __orochi_stubs)?;
        });
        // Inner doc comments (`//!`) become outer ones on the init function.
        let docs = item_mod
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .map(|a| {
                let mut a = a.clone();
                a.style = syn::AttrStyle::Outer;
                a
            });
        // `gil_used = true`: the generated code has not been audited for the
        // free-threaded build, so ask it to keep the GIL enabled.
        entry = quote! {
            #[::orochi::pyo3::pymodule(crate = #PY, name = #full_name, gil_used = true)]
            #(#docs)*
            fn __orochi_pymodule(
                __orochi_module: &::orochi::pyo3::Bound<'_, ::orochi::pyo3::types::PyModule>,
            ) -> ::orochi::pyo3::PyResult<()> {
                __orochi_init(__orochi_module)
            }

            /// Builds this module in-process and registers it in `sys.modules`
            /// (for embedding and tests; Python itself loads the extension through
            /// `PyInit_<name>`). Later calls return the same module object.
            #[doc(hidden)]
            pub fn __orochi_module() -> ::orochi::Result<::orochi::Object> {
                static CELL: ::orochi::__private::PyOnceLock<
                    ::orochi::pyo3::Py<::orochi::pyo3::types::PyModule>,
                > = ::orochi::__private::PyOnceLock::new();
                ::orochi::__private::module_object(&CELL, #full_name, #doc, __orochi_init)
            }

            /// The `.pyi` type stubs of this module and its submodules; write
            /// them with `orochi::write_stubs`.
            #[doc(hidden)]
            pub fn __orochi_stubs() -> ::orochi::Stubs {
                ::orochi::__private::stub::stubs(__orochi_stub_module())
            }
        };
    }

    let glue_items: Vec<Item> = glue
        .into_iter()
        .chain([
            quote! {
                #[doc(hidden)]
                #[allow(unused_variables)]
                pub fn __orochi_init(
                    __orochi_module: &::orochi::pyo3::Bound<'_, ::orochi::pyo3::types::PyModule>,
                ) -> ::orochi::pyo3::PyResult<()> {
                    #(#init)*
                    ::core::result::Result::Ok(())
                }
            },
            stub_module,
            entry,
        ])
        .map(Item::Verbatim)
        .collect();
    out_items.extend(glue_items);
    item_mod.content = Some((brace, out_items));
    item_mod
}

/// A snake_case wrapper name: pyo3 derives a module name from the wrapper of
/// a `#[pyfunction]`, where our `#[allow(non_snake_case)]` does not reach,
/// so `_under` or `a__b` must not produce `__orochi_fn__under`.
fn wrapper_ident(prefix: &str, ident: &Ident, used: &mut HashSet<String>) -> Ident {
    let name = ident.unraw().to_string();
    let parts: Vec<&str> = name.split('_').filter(|p| !p.is_empty()).collect();
    let base = format!("__orochi_{prefix}_{}", parts.join("_").to_lowercase());
    let mut candidate = base.clone();
    let mut n = 1;
    while !used.insert(candidate.clone()) {
        n += 1;
        candidate = format!("{base}_{n}");
    }
    Ident::new(&candidate, ident.span())
}

/// The class an `impl` block is for, if its self type names a class of this
/// module (`Point` or `self::Point`).
fn impl_target<'a>(classes: &'a mut [Class], self_ty: &Type) -> Option<&'a mut Class> {
    classes
        .iter_mut()
        .find(|c| is_self_type(self_ty, Some(&c.ident)) && !is_self_keyword(self_ty))
}

fn is_self_keyword(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp) if tp.path.is_ident("Self"))
}

// ---------------------------------------------------------------- classes

fn derives(attrs: &[Attribute]) -> HashSet<String> {
    let mut out = HashSet::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("derive")) {
        if let Ok(paths) = attr.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated) {
            for path in paths {
                if let Some(seg) = path.segments.last() {
                    out.insert(seg.ident.to_string());
                }
            }
        }
    }
    out
}

fn new_class(
    cx: &mut Cx,
    ident: &Ident,
    attrs: &[Attribute],
    variants: Option<Vec<(Ident, Vec<Attribute>)>>,
) -> Class {
    let d = derives(attrs);
    let py_name = ident.unraw().to_string();
    check_python_name(cx, &py_name, ident, "class");
    Class {
        ident: ident.clone(),
        py_name,
        variants,
        cfg: cfg_attrs(attrs),
        clone: d.contains("Clone"),
        debug: d.contains("Debug"),
        display: false,
        eq: d.contains("PartialEq"),
        hash: d.contains("Hash"),
        has_ctor: false,
        names: Names::default(),
        methods: Vec::new(),
        doc: doc_expr(attrs),
        stub_members: Vec::new(),
    }
}

fn pyclass_attr(class: &Class, full_name: &str) -> Attribute {
    let name = &class.py_name;
    syn::parse_quote! {
        #[::orochi::pyo3::pyclass(crate = #PY, name = #name, module = #full_name)]
    }
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

fn process_struct(
    cx: &mut Cx,
    mut s: ItemStruct,
    full_name: &str,
    path: &[String],
) -> (ItemStruct, Class) {
    reject_generics(cx, &s.generics, "a struct");
    let mut class = new_class(cx, &s.ident, &s.attrs, None);
    match &s.fields {
        Fields::Named(named) => {
            for field in &named.named {
                if !is_pub(&field.vis) {
                    continue;
                }
                let ident = field.ident.as_ref().expect("named field");
                let py_name = ident.unraw().to_string();
                check_python_name(cx, &py_name, ident, "field");
                class.names.add(cx, &py_name, ident, "class");
                check_field_type(cx, &field.ty, path);
                let ty = &field.ty;
                let docs = doc_attrs(&field.attrs);
                let cfg = cfg_attrs(&field.attrs);
                let doc = doc_expr(&field.attrs);
                class.stub_members.push(quote! {
                    #(#cfg)*
                    __orochi_class.property(
                        #py_name,
                        #doc,
                        (&&::orochi::__private::hint::Hint::<#ty>::new()).frozen(),
                        (&&::orochi::__private::hint::Hint::<#ty>::new()).input(),
                    );
                });
                let getter = format_ident!("__orochi_get_{}", ident.unraw());
                let setter = format_ident!("__orochi_set_{}", ident.unraw());
                let what = format!("attribute '{py_name}'");
                class.methods.push(quote_spanned! {ty.span()=>
                    #(#cfg)*
                    #(#docs)*
                    #[getter]
                    #[pyo3(name = #py_name)]
                    fn #getter(
                        __orochi_self: &::orochi::pyo3::Bound<'_, Self>,
                    ) -> ::orochi::__private::PyResultObject {
                        ::orochi::__private::get_field(__orochi_self, |s: &Self| &s.#ident)
                    }

                    #(#cfg)*
                    #[setter]
                    #[pyo3(name = #py_name)]
                    fn #setter(
                        __orochi_self: &::orochi::pyo3::Bound<'_, Self>,
                        value: ::orochi::pyo3::Py<::orochi::pyo3::PyAny>,
                    ) -> ::orochi::pyo3::PyResult<()> {
                        ::orochi::__private::set_field(
                            __orochi_self, value, #what, |s: &mut Self| &mut s.#ident,
                        )
                    }
                });
            }
        }
        Fields::Unnamed(unnamed) => {
            for field in &unnamed.unnamed {
                if is_pub(&field.vis) {
                    cx.error(
                        field,
                        "fields of a tuple struct cannot be exposed to Python (they have no \
                         name); use a struct with named fields, or make this field private",
                    );
                }
            }
        }
        Fields::Unit => {}
    }
    s.attrs.insert(0, pyclass_attr(&class, full_name));
    (s, class)
}

/// A `pub` field that holds a struct class of this invocation by value (directly or
/// inside `Vec`, `Option`, a map, ...) would be read from Python as copies, so
/// `obj.field.x = ...` would silently change a copy.
fn check_field_type(cx: &mut Cx, ty: &Type, path: &[String]) {
    let mut suggestion = ty.clone();
    let mut found = Vec::new();
    wrap_classes(cx, path, &mut suggestion, &mut found);
    let Some(name) = found.first() else { return };
    let message = if matches!(ty, Type::Path(tp) if cx.resolve_class(path, &tp.path).is_some()) {
        format!(
            "a `pub` field of class type `{name}` would be read as a copy from Python, so \
             `obj.field.x = ...` would silently change the copy; use `Ref<{name}>` to share \
             one `{name}` object, or make the field private"
        )
    } else {
        format!(
            "this `pub` field holds the class `{name}` by value, so Python would only see \
             copies of the `{name}` objects and changes to them would be lost; use `{}` to \
             share the objects, or make the field private",
            type_text(&suggestion)
        )
    };
    cx.error(ty, message);
}

/// `ty` as written in source, roughly (`Vec<Ref<P>>`, not `Vec < Ref < P > >`).
fn type_text(ty: &Type) -> String {
    let mut text = ty.to_token_stream().to_string();
    for (from, to) in [
        (" :: ", "::"),
        (":: ", "::"),
        (" < ", "<"),
        ("< ", "<"),
        (" <", "<"),
        (" >", ">"),
        (" ,", ","),
        ("& ", "&"),
    ] {
        text = text.replace(from, to);
    }
    text
}

/// Replaces every class held by value in `ty` with `Ref<Class>`, collecting
/// the class names.
fn wrap_classes(cx: &Cx, path: &[String], ty: &mut Type, found: &mut Vec<String>) {
    match ty {
        Type::Path(tp) if tp.qself.is_none() => {
            if let Some(name) = cx.resolve_class(path, &tp.path) {
                found.push(name);
                let inner = tp.clone();
                *ty = syn::parse_quote!(Ref<#inner>);
                return;
            }
            if tp.path.segments.last().is_some_and(|s| s.ident == "Ref") {
                return;
            }
            for seg in tp.path.segments.iter_mut() {
                if let PathArguments::AngleBracketed(args) = &mut seg.arguments {
                    for arg in args.args.iter_mut() {
                        if let GenericArgument::Type(t) = arg {
                            wrap_classes(cx, path, t, found);
                        }
                    }
                }
            }
        }
        Type::Array(a) => wrap_classes(cx, path, &mut a.elem, found),
        Type::Slice(s) => wrap_classes(cx, path, &mut s.elem, found),
        Type::Paren(p) => wrap_classes(cx, path, &mut p.elem, found),
        Type::Group(g) => wrap_classes(cx, path, &mut g.elem, found),
        Type::Tuple(t) => {
            for elem in t.elems.iter_mut() {
                wrap_classes(cx, path, elem, found);
            }
        }
        _ => {}
    }
}

fn process_enum(cx: &mut Cx, mut e: ItemEnum, full_name: &str) -> (ItemEnum, Class) {
    reject_generics(cx, &e.generics, "an enum");
    let mut variants = Vec::new();
    for variant in &e.variants {
        if !matches!(variant.fields, Fields::Unit) {
            cx.error(
                variant,
                "only enums whose variants have no fields can be exported to Python; make the \
                 enum private, or wrap it in a `pub struct`",
            );
        }
        check_python_name(
            cx,
            &variant.ident.unraw().to_string(),
            &variant.ident,
            "enum member",
        );
        variants.push((variant.ident.clone(), cfg_attrs(&variant.attrs)));
    }
    if e.variants.is_empty() {
        cx.error(
            &e.ident,
            "an enum exported to Python needs at least one variant",
        );
    }
    let class = new_class(cx, &e.ident, &e.attrs, Some(variants));
    e.attrs.insert(0, pyclass_attr(&class, full_name));
    (e, class)
}

fn process_inherent_impl(cx: &mut Cx, imp: &ItemImpl, class: &mut Class) {
    let exported = imp.items.iter().any(|item| match item {
        ImplItem::Fn(f) => is_pub(&f.vis),
        ImplItem::Const(c) => is_pub(&c.vis),
        _ => false,
    });
    if exported {
        reject_generics(cx, &imp.generics, "an impl block with `pub` items");
    }
    // A `#[cfg]` on the impl block applies to every generated wrapper.
    let impl_cfg = cfg_attrs(&imp.attrs);
    for item in &imp.items {
        match item {
            ImplItem::Fn(f) if is_pub(&f.vis) => {
                let cfg = impl_cfg
                    .iter()
                    .chain(&cfg_attrs(&f.attrs))
                    .cloned()
                    .collect::<Vec<_>>();
                let is_ctor = f.sig.ident == "new" && f.sig.receiver().is_none();
                if is_ctor {
                    if class.has_ctor {
                        cx.error(&f.sig.ident, "a class can have only one `pub fn new`");
                    }
                    class.has_ctor = true;
                    check_ctor_return(cx, &f.sig, &class.ident);
                } else {
                    let py_name = f.sig.ident.unraw().to_string();
                    check_python_name(cx, &py_name, &f.sig.ident, "method");
                    class.names.add(cx, &py_name, &f.sig.ident, "class");
                }
                let kind = if is_ctor {
                    Kind::Ctor
                } else if f.sig.receiver().is_some() {
                    Kind::Method
                } else {
                    Kind::Static
                };
                if let Some((tokens, stub)) = method_wrapper(cx, &f.sig, &f.attrs, kind, class) {
                    class.methods.push(quote!(#(#cfg)* #tokens));
                    let add = match kind {
                        Kind::Ctor => quote!(init),
                        Kind::Static => quote!(static_method),
                        _ => quote!(method),
                    };
                    class
                        .stub_members
                        .push(quote!(#(#cfg)* __orochi_class.#add(#stub);));
                }
            }
            ImplItem::Const(c) if is_pub(&c.vis) => {
                let py_name = c.ident.unraw().to_string();
                check_python_name(cx, &py_name, &c.ident, "constant");
                class.names.add(cx, &py_name, &c.ident, "class");
                let ident = &c.ident;
                let func = format_ident!("__orochi_const_{}", ident.unraw());
                let cfg = impl_cfg
                    .iter()
                    .chain(&cfg_attrs(&c.attrs))
                    .cloned()
                    .collect::<Vec<_>>();
                let cty = &c.ty;
                class.stub_members.push(quote! {
                    #(#cfg)*
                    __orochi_class.attribute(
                        #py_name,
                        (&&&::orochi::__private::hint::Hint::<#cty>::new()).output(),
                    );
                });
                class.methods.push(quote_spanned! {c.ty.span()=>
                    #(#cfg)*
                    #[classattr]
                    #[pyo3(name = #py_name)]
                    fn #func() -> ::orochi::__private::PyResultObject {
                        ::orochi::__private::value(&Self::#ident)
                    }
                });
            }
            _ => {}
        }
    }
}

fn check_ctor_return(cx: &mut Cx, sig: &Signature, class: &Ident) {
    let ok = match &sig.output {
        ReturnType::Default => false,
        ReturnType::Type(_, ty) => match &**ty {
            Type::Path(tp) => tp
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "Self" || s.ident == *class || s.ident == "Result"),
            _ => false,
        },
    };
    if !ok {
        cx.error(
            &sig.ident,
            "`pub fn new` without `self` becomes the Python constructor and must return `Self` \
             or `Result<Self, E>`; rename it to export it as a static method instead",
        );
    }
}

fn class_glue(class: Class, full_name: &str) -> TokenStream2 {
    let ident = &class.ident;
    let cfg = &class.cfg;
    let qualified = format!("{full_name}.{}", class.py_name);
    let stub = class_stub(&class);
    let mut methods = class.methods;
    let extra: TokenStream2;
    if let Some(variants) = &class.variants {
        // Enum members are singletons stored on the class; equality and
        // hashing go by member, whatever the enum derives.
        let names: Vec<String> = variants
            .iter()
            .map(|(v, _)| v.unraw().to_string())
            .collect();
        let vars: Vec<&Ident> = variants.iter().map(|(v, _)| v).collect();
        let cfgs: Vec<&Vec<Attribute>> = variants.iter().map(|(_, c)| c).collect();
        let index: Vec<usize> = (0..variants.len()).collect();
        let index_fn = quote! {
            fn index(value: &#ident) -> usize {
                match *value {
                    #( #(#cfgs)* #ident::#vars => #index, )*
                }
            }
        };
        methods.push(quote! {
            fn __eq__(&self, other: &Self) -> bool {
                #index_fn
                index(self) == index(other)
            }

            fn __hash__(&self) -> u64 {
                #index_fn
                ::orochi::__private::hash(&index(self))
            }
        });
        extra = quote! {
            #(#cfg)*
            impl ::orochi::IntoObject for #ident {
                fn into_object(self) -> ::orochi::Result<::orochi::Object> {
                    ::orochi::IntoObject::into_object(&self)
                }
                fn output_hint() -> ::std::string::String {
                    ::std::string::String::from(#qualified)
                }
            }

            #(#cfg)*
            impl #ident {
                /// The member singletons of the Python class, by variant index,
                /// read once when the class is created.
                #[doc(hidden)]
                fn __orochi_members(
                    py: ::orochi::pyo3::Python<'_>,
                ) -> ::orochi::pyo3::PyResult<
                    &'static [::core::option::Option<::orochi::pyo3::Py<::orochi::pyo3::PyAny>>],
                > {
                    static MEMBERS: ::orochi::__private::EnumMembers =
                        ::orochi::__private::EnumMembers::new();
                    ::orochi::__private::enum_members::<Self>(py, &MEMBERS, &[#(#names),*])
                }
            }

            /// The member singleton stored on the Python class.
            #(#cfg)*
            impl ::orochi::IntoObject for &#ident {
                fn into_object(self) -> ::orochi::Result<::orochi::Object> {
                    ::orochi::__private::enum_member(
                        #ident::__orochi_members,
                        match *self {
                            #( #(#cfgs)* #ident::#vars => #index, )*
                        },
                    )
                }
                fn output_hint() -> ::std::string::String {
                    ::std::string::String::from(#qualified)
                }
            }

            /// Accepts only members of the Python class.
            #(#cfg)*
            impl ::orochi::FromObject for #ident {
                fn from_object(obj: &::orochi::Object) -> ::orochi::Result<Self> {
                    ::orochi::__private::enum_from_object(obj, |value: &Self| match *value {
                        #( #(#cfgs)* #ident::#vars => #ident::#vars, )*
                    })
                }
                fn input_hint() -> ::std::string::String {
                    ::std::string::String::from(#qualified)
                }
            }
        };
    } else {
        if class.debug {
            methods.push(quote! {
                fn __repr__(&self) -> ::std::string::String {
                    ::std::format!("{:?}", self)
                }
            });
        }
        if class.eq {
            methods.push(quote! {
                fn __eq__(&self, other: &Self) -> bool {
                    self == other
                }
            });
        }
        if class.hash {
            methods.push(quote! {
                fn __hash__(&self) -> u64 {
                    ::orochi::__private::hash(self)
                }
            });
        } else if class.eq {
            // Python semantics: a class defining `__eq__` without `__hash__` is
            // unhashable.
            methods.push(quote! {
                #[classattr]
                const __hash__: ::core::option::Option<::orochi::pyo3::Py<::orochi::pyo3::PyAny>> =
                    ::core::option::Option::None;
            });
        }
        let from_object = class.clone.then(|| {
            quote! {
                #(#cfg)*
                impl ::orochi::FromObject for #ident {
                    fn from_object(obj: &::orochi::Object) -> ::orochi::Result<Self> {
                        ::orochi::__private::class_from_object(obj)
                    }
                    fn input_hint() -> ::std::string::String {
                        ::std::string::String::from(#qualified)
                    }
                }

                /// Converts a copy of the referenced value into a new Python object.
                #(#cfg)*
                impl ::orochi::IntoObject for &#ident {
                    fn into_object(self) -> ::orochi::Result<::orochi::Object> {
                        ::orochi::__private::class_into_object(::core::clone::Clone::clone(self))
                    }
                    fn output_hint() -> ::std::string::String {
                        ::std::string::String::from(#qualified)
                    }
                }
            }
        });
        extra = quote! {
            #(#cfg)*
            impl ::orochi::IntoObject for #ident {
                fn into_object(self) -> ::orochi::Result<::orochi::Object> {
                    ::orochi::__private::class_into_object(self)
                }
                fn output_hint() -> ::std::string::String {
                    ::std::string::String::from(#qualified)
                }
            }

            #(#cfg)*
            impl ::orochi::__private::MutableClass for #ident {}

            #from_object
        };
    }
    if class.display {
        methods.push(quote! {
            fn __str__(&self) -> ::std::string::String {
                ::std::string::ToString::to_string(self)
            }
        });
    }
    quote! {
        #(#cfg)*
        impl ::orochi::Class for #ident {
            const PYTHON_NAME: &'static str = #qualified;
        }

        #extra

        #(#cfg)*
        #[::orochi::pyo3::pymethods]
        #[pyo3(crate = #PY)]
        #[allow(non_snake_case, unused_mut, clippy::all)]
        impl #ident {
            #(#methods)*
        }

        #stub
    }
}

/// The `__orochi_stub_class()` function describing the class for `.pyi`
/// stubs.
fn class_stub(class: &Class) -> TokenStream2 {
    let ident = &class.ident;
    let cfg = &class.cfg;
    let name = &class.py_name;
    let doc = &class.doc;
    // Mirrors the `__hash__ = None` class attribute generated below.
    let unhashable = !class.is_enum() && class.eq && !class.hash;
    let constructible = class.has_ctor;
    let variants = class.variants.iter().flatten().map(|(v, cfgs)| {
        let v = v.unraw().to_string();
        quote!(#(#cfgs)* __orochi_class.variant(#v);)
    });
    let members = &class.stub_members;
    let uses = hint_uses();
    quote! {
        #(#cfg)*
        impl #ident {
            #[allow(unused_mut, clippy::all)]
            fn __orochi_stub_class() -> ::orochi::__private::stub::Class {
                #uses
                let mut __orochi_class =
                    ::orochi::__private::stub::Class::new(#name, #doc, #unhashable, #constructible);
                #(#variants)*
                #(#members)*
                __orochi_class
            }
        }
    }
}

/// Brings the hint-selection traits into scope (anonymously, so that they
/// cannot shadow the user's names).
fn hint_uses() -> TokenStream2 {
    quote! {
        #[allow(unused_imports)]
        use ::orochi::__private::hint::{
            BorrowedClassHint as _, BorrowedHint as _, BorrowedHintAny as _, FrozenHint as _,
            FrozenHintAny as _, InputHint as _, InputHintAny as _, OutputHint as _,
            OutputHintAny as _, OutputResultHint as _,
        };
    }
}

/// A `::orochi::__private::stub::Function` expression for an analyzed
/// function or method.
fn stub_function(a: &Analyzed, sig: &Signature, attrs: &[Attribute], kind: Kind) -> TokenStream2 {
    let name = sig.ident.unraw().to_string();
    let doc = doc_expr(attrs);
    let hint = quote!(::orochi::__private::hint::Hint);
    let params = a.params.iter().map(|p| {
        let name = p.ident.unraw().to_string();
        let ty = &p.ty;
        let hint = match p.pass {
            Pass::Value => quote!((&&#hint::<#ty>::new()).input()),
            Pass::Ref | Pass::RefMut => quote!((&&&#hint::<#ty>::new()).borrowed()),
            Pass::OptRef => quote! {
                ::orochi::__private::hint::optional((&&&#hint::<#ty>::new()).borrowed())
            },
        };
        let default_none = p.default_none;
        quote!(::orochi::__private::stub::Param::new(#name, #hint, #default_none))
    });
    let none = quote!(::std::string::String::from("None"));
    let ret = match (kind, a.ret, &sig.output) {
        (Kind::Ctor, _, _) => none,
        // The class is final, so its own name is exactly `Self`.
        (_, Ret::SelfRef | Ret::SelfRefResult, _) => {
            quote!(::std::string::String::from(
                <Self as ::orochi::Class>::PYTHON_NAME
            ))
        }
        (_, Ret::Convert, ReturnType::Default) => none,
        (_, Ret::Convert, ReturnType::Type(_, ty)) => quote!((&&&#hint::<#ty>::new()).output()),
    };
    quote! {
        ::orochi::__private::stub::Function::new(#name, #doc, ::std::vec![#(#params),*], #ret)
    }
}

// ---------------------------------------------------------------- functions

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Function,
    Method,
    Static,
    Ctor,
}

#[derive(Clone, Copy, PartialEq)]
enum Pass {
    Value,
    Ref,
    RefMut,
    /// `Option<&T>`.
    OptRef,
}

struct Param {
    ident: Ident,
    /// The full type for `Value`, the referent for the others.
    ty: Type,
    pass: Pass,
    default_none: bool,
    span: Span,
}

#[derive(Clone, Copy, PartialEq)]
enum Receiver {
    Ref,
    RefMut,
    Value,
}

#[derive(Clone, Copy, PartialEq)]
enum Ret {
    Convert,
    /// `&Self` / `&mut Self`: returns the same Python object.
    SelfRef,
    /// `Result<&Self, E>` / `Result<&mut Self, E>`.
    SelfRefResult,
}

struct Analyzed {
    receiver: Option<(Receiver, Span)>,
    params: Vec<Param>,
    ret: Ret,
    ret_span: Span,
}

fn analyze(cx: &mut Cx, sig: &Signature, kind: Kind, class: Option<&Class>) -> Option<Analyzed> {
    let before = cx.errors.len();
    let what = match kind {
        Kind::Function => "the function",
        _ => "the method",
    };
    if let Some(t) = &sig.asyncness {
        cx.error(
            t,
            format!("`async fn` cannot be exported to Python yet; make {what} private"),
        );
    }
    if let syn::Safety::Unsafe(t) = &sig.safety {
        cx.error(
            t,
            format!("`unsafe fn` cannot be exported to Python; make {what} private"),
        );
    }
    if let Some(abi) = &sig.abi {
        cx.error(
            abi,
            format!("`extern` functions cannot be exported to Python; make {what} private"),
        );
    }
    if let Some(v) = &sig.variadic {
        cx.error(v, "variadic functions cannot be exported to Python");
    }
    reject_generics(
        cx,
        &sig.generics,
        if kind == Kind::Function {
            "a function"
        } else {
            "a method"
        },
    );
    let py_name = sig.ident.unraw().to_string();
    if kind != Kind::Ctor && py_name.starts_with("__") && py_name.ends_with("__") {
        cx.error(
            &sig.ident,
            "special methods (`__name__`) are not supported yet; orochi generates `__repr__`, \
             `__str__`, `__eq__` and `__hash__` from `Debug`, `Display`, `PartialEq` and `Hash`",
        );
    }

    let mut receiver = None;
    let mut params = Vec::new();
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(r) => {
                let kind = match &r.kind {
                    syn::ReceiverKind::Reference(_, lifetime, mutability) => {
                        if let Some(lt) = lifetime {
                            cx.error(
                                lt,
                                "explicit lifetimes are not supported in exported methods",
                            );
                        }
                        if mutability.is_some() {
                            Receiver::RefMut
                        } else {
                            Receiver::Ref
                        }
                    }
                    syn::ReceiverKind::Value => Receiver::Value,
                    _ => {
                        cx.error(
                            r,
                            "only `&self`, `&mut self` and `self` receivers are supported in \
                             exported methods",
                        );
                        Receiver::Ref
                    }
                };
                if kind == Receiver::RefMut
                    && let Some(class) = class
                    && class.is_enum()
                {
                    cx.error(
                        r,
                        format!(
                            "methods of an exported enum cannot take `&mut self`: the members \
                             of `{}` are shared singletons in Python; take `self` or `&self` and \
                             return the new value",
                            class.ident
                        ),
                    );
                }
                receiver = Some((kind, r.span()));
            }
            FnArg::Typed(pt) => {
                if let Some(attr) = pt
                    .attrs
                    .iter()
                    .find(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
                {
                    cx.error(
                        attr,
                        "`#[cfg]` on a parameter of an exported function is not supported (the \
                         Python signature would change with it); put the `#[cfg]` on the whole \
                         function and write one version per configuration",
                    );
                }
                let ident = match &*pt.pat {
                    Pat::Ident(pi) if pi.by_ref.is_none() && pi.subpat.is_none() => {
                        pi.ident.clone()
                    }
                    other => {
                        cx.error(
                            other,
                            "parameters of exported functions must be plain names (they become \
                             Python keyword argument names); destructure inside the body",
                        );
                        continue;
                    }
                };
                let name = ident.unraw().to_string();
                check_python_name(cx, &name, &ident, "parameter");
                check_type(cx, &pt.ty, "parameter");
                let (ty, pass) = match &*pt.ty {
                    Type::Reference(r) => (
                        (*r.elem).clone(),
                        if r.mutability.is_some() { Pass::RefMut } else { Pass::Ref },
                    ),
                    ty => match option_inner(ty) {
                        Some(Type::Reference(r)) if r.mutability.is_none() => {
                            ((*r.elem).clone(), Pass::OptRef)
                        }
                        Some(Type::Reference(r)) => {
                            cx.error(r, "`Option<&mut T>` parameters are not supported");
                            continue;
                        }
                        _ => (ty.clone(), Pass::Value),
                    },
                };
                params.push(Param {
                    ident,
                    ty,
                    pass,
                    default_none: false,
                    span: pt.ty.span(),
                });
            }
        }
    }
    // Trailing `Option<T>` parameters default to `None`.
    let types: Vec<&Type> = sig
        .inputs
        .iter()
        .filter_map(|i| match i {
            FnArg::Typed(pt) => Some(&*pt.ty),
            _ => None,
        })
        .collect();
    if types.len() == params.len() {
        for (param, ty) in params.iter_mut().zip(&types).rev() {
            if option_inner(ty).is_none() {
                break;
            }
            param.default_none = true;
        }
    }

    let (ret, ret_span) = match &sig.output {
        ReturnType::Default => (Ret::Convert, sig.ident.span()),
        ReturnType::Type(_, ty) => {
            check_type(cx, ty, "return type");
            let by_ref = matches!(receiver, Some((Receiver::Ref | Receiver::RefMut, _)));
            let class_ident = class.map(|c| &c.ident);
            let is_self_ref = |ty: &Type| {
                by_ref && matches!(ty, Type::Reference(r) if is_self_type(&r.elem, class_ident))
            };
            let ret = if is_self_ref(ty) {
                Ret::SelfRef
            } else if result_ok_type(ty).is_some_and(is_self_ref) {
                Ret::SelfRefResult
            } else {
                Ret::Convert
            };
            (ret, ty.span())
        }
    };
    (cx.errors.len() == before).then_some(Analyzed {
        receiver,
        params,
        ret,
        ret_span,
    })
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
                TokenTree::Ident(i) if after_quote && i != "static" => {
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
                "explicit lifetimes are not supported in the {what} of an exported function; \
                 use elided references (`&str`, `&[T]`, `&Point`) or owned types"
            ),
        )),
        None => {}
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

/// `T` for `Option<T>` (by the last path segment).
fn option_inner(ty: &Type) -> Option<&Type> {
    generic_arg(ty, "Option")
}

/// `T` for `Result<T, ..>` (by the last path segment).
fn result_ok_type(ty: &Type) -> Option<&Type> {
    generic_arg(ty, "Result")
}

fn generic_arg<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(tp) = ty else { return None };
    if tp.qself.is_some() {
        return None;
    }
    let last = tp.path.segments.last()?;
    if last.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    }
}

/// The `#[pyfunction]` wrapper of `f`, and its stub description.
fn function_wrapper(
    cx: &mut Cx,
    f: &ItemFn,
    wrapper: &Ident,
) -> Option<(TokenStream2, TokenStream2)> {
    let a = analyze(cx, &f.sig, Kind::Function, None)?;
    let stub = stub_function(&a, &f.sig, &f.attrs, Kind::Function);
    let ident = &f.sig.ident;
    let py_name = ident.unraw().to_string();
    let docs = doc_attrs(&f.attrs);
    let cfg = cfg_attrs(&f.attrs);
    let parts = Parts::new(&a.params);
    let signature = &parts.signature;
    let wrapper_args = &parts.wrapper_args;
    let text_sig = parts.text_signature(None);
    // A path, so that a parameter named like the function cannot shadow it.
    let body = body(
        &a,
        &parts,
        quote!(self::#ident),
        Kind::Function,
        &py_name,
        false,
    );
    let tokens = quote! {
        #(#cfg)*
        #[::orochi::pyo3::pyfunction(crate = #PY, name = #py_name, signature = (#(#signature),*), text_signature = #text_sig)]
        #(#docs)*
        #[allow(non_snake_case, unused_mut, clippy::all)]
        fn #wrapper(#(#wrapper_args),*) -> ::orochi::__private::PyResultObject {
            #body
        }
    };
    Some((tokens, stub))
}

fn method_wrapper(
    cx: &mut Cx,
    sig: &Signature,
    attrs: &[Attribute],
    kind: Kind,
    class: &Class,
) -> Option<(TokenStream2, TokenStream2)> {
    let a = analyze(cx, sig, kind, Some(class))?;
    let stub = stub_function(&a, sig, attrs, kind);
    let ident = &sig.ident;
    let py_name = ident.unraw().to_string();
    let what = format!("{}.{py_name}()", class.py_name);
    let docs = doc_attrs(attrs);
    let parts = Parts::new(&a.params);
    let signature = &parts.signature;
    let mut wrapper_args = parts.wrapper_args.clone();
    let text_sig = parts.text_signature(None);
    let body = body(
        &a,
        &parts,
        quote!(Self::#ident),
        kind,
        &what,
        class.is_enum(),
    );
    let tokens = match kind {
        Kind::Ctor => quote! {
            #[new]
            #[pyo3(signature = (#(#signature),*), text_signature = #text_sig)]
            #(#docs)*
            fn __orochi_new(#(#wrapper_args),*) -> ::orochi::pyo3::PyResult<Self> {
                #body
            }
        },
        Kind::Static => {
            let wrapper = format_ident!("__orochi_static_{}", ident.unraw());
            quote! {
                #[staticmethod]
                #[pyo3(name = #py_name, signature = (#(#signature),*), text_signature = #text_sig)]
                #(#docs)*
                fn #wrapper(#(#wrapper_args),*) -> ::orochi::__private::PyResultObject {
                    #body
                }
            }
        }
        _ => {
            let wrapper = format_ident!("__orochi_method_{}", ident.unraw());
            wrapper_args.insert(0, quote!(__orochi_self: &::orochi::pyo3::Bound<'_, Self>));
            let text_sig = parts.text_signature(Some("$self"));
            quote! {
                #[pyo3(name = #py_name, signature = (#(#signature),*), text_signature = #text_sig)]
                #(#docs)*
                fn #wrapper(#(#wrapper_args),*) -> ::orochi::__private::PyResultObject {
                    #body
                }
            }
        }
    };
    Some((tokens, stub))
}

/// Per-parameter pieces of a wrapper.
struct Parts {
    signature: Vec<TokenStream2>,
    /// Python parameter list for `__text_signature__`.
    py_params: Vec<String>,
    wrapper_args: Vec<TokenStream2>,
    /// Conversions done before borrowing anything.
    holds: Vec<TokenStream2>,
    /// Borrows of class instances / temporaries, inside the call scope.
    guards: Vec<TokenStream2>,
    call_args: Vec<TokenStream2>,
}

impl Parts {
    fn new(params: &[Param]) -> Parts {
        let mut p = Parts {
            signature: Vec::new(),
            py_params: Vec::new(),
            wrapper_args: Vec::new(),
            holds: Vec::new(),
            guards: Vec::new(),
            call_args: Vec::new(),
        };
        let pyany = quote!(::orochi::pyo3::Py<::orochi::pyo3::PyAny>);
        for param in params {
            let id = &param.ident;
            let ty = &param.ty;
            let span = param.span;
            let what = format!("argument '{}'", id.unraw());
            let base = id.unraw();
            let k = format_ident!("__orochi_k_{}", base);
            let h = format_ident!("__orochi_h_{}", base);
            let g = format_ident!("__orochi_g_{}", base);
            let py_name = id.unraw().to_string();
            if param.default_none {
                p.py_params.push(format!("{py_name}=None"));
                p.signature.push(quote!(#id = ::core::option::Option::None));
                p.wrapper_args
                    .push(quote!(#id: ::core::option::Option<#pyany>));
            } else {
                p.py_params.push(py_name);
                p.signature.push(quote!(#id));
                p.wrapper_args.push(quote!(#id: #pyany));
            }
            let spec = quote_spanned! {span=>
                let #k = (&&::orochi::__private::ParamSpec::<#ty>::new()).param_kind();
            };
            match param.pass {
                Pass::Value => {
                    let convert = if param.default_none {
                        quote_spanned!(span=> opt_arg)
                    } else {
                        quote_spanned!(span=> arg)
                    };
                    p.holds.push(quote_spanned! {span=>
                        let #id: #ty = ::orochi::__private::#convert(#id, #what)?;
                    });
                    p.call_args.push(quote!(#id));
                }
                Pass::Ref => {
                    p.holds
                        .push(quote_spanned! {span=> #spec let #h = #k.hold(#id, #what)?; });
                    p.guards
                        .push(quote_spanned! {span=> let #g = #k.get(&#h)?; });
                    p.call_args.push(quote!(&*#g));
                }
                Pass::OptRef => {
                    let hold = if param.default_none {
                        quote_spanned!(span=> #k.hold_opt(#id, #what)?)
                    } else {
                        quote_spanned!(span=> #k.hold_opt(::core::option::Option::Some(#id), #what)?)
                    };
                    p.holds.push(quote_spanned! {span=> #spec let #h = #hold; });
                    p.guards
                        .push(quote_spanned! {span=> let #g = #k.get_opt(&#h)?; });
                    p.call_args
                        .push(quote!(::core::option::Option::as_deref(&#g)));
                }
                Pass::RefMut => {
                    p.holds.push(
                        quote_spanned! {span=> #spec let mut #h = #k.hold_mut(#id, #what)?; },
                    );
                    p.guards
                        .push(quote_spanned! {span=> let mut #g = #k.get_mut(&mut #h)?; });
                    p.call_args.push(quote!(&mut *#g));
                }
            }
        }
        p
    }

    /// `(a, b=None)`, spelled out because pyo3 renders the fully qualified
    /// `::core::option::Option::None` default as `...`.
    fn text_signature(&self, receiver: Option<&str>) -> String {
        let params: Vec<&str> = receiver
            .into_iter()
            .chain(self.py_params.iter().map(String::as_str))
            .collect();
        format!("({})", params.join(", "))
    }
}

/// The wrapper body: convert arguments, borrow, call, convert the result,
/// release borrows. `self` by value is a copy of the instance (for an enum,
/// of the member, without `Clone`).
fn body(
    a: &Analyzed,
    parts: &Parts,
    callee: TokenStream2,
    kind: Kind,
    what: &str,
    is_enum: bool,
) -> TokenStream2 {
    let holds = &parts.holds;
    let guards = &parts.guards;
    let call_args = &parts.call_args;
    let (self_borrow, self_arg) = match a.receiver {
        None => (quote!(), None),
        Some((Receiver::Ref, _)) => (
            quote!(let __orochi_guard = ::orochi::__private::borrow(__orochi_self)?;),
            Some(quote!(&*__orochi_guard)),
        ),
        Some((Receiver::RefMut, _)) => (
            quote!(let mut __orochi_guard = ::orochi::__private::borrow_mut(__orochi_self)?;),
            Some(quote!(&mut *__orochi_guard)),
        ),
        Some((Receiver::Value, span)) => (
            if is_enum {
                quote_spanned! {span=>
                    let __orochi_guard: Self = ::orochi::__private::enum_self(__orochi_self)?;
                }
            } else {
                quote_spanned! {span=>
                    let __orochi_guard: Self = ::orochi::__private::self_by_value(__orochi_self)?;
                }
            },
            Some(quote!(__orochi_guard)),
        ),
    };
    let args = self_arg.into_iter().chain(call_args.iter().cloned());
    let span = a.ret_span;
    let same_instance = quote! {{
        let __orochi_ret: &Self = &*__orochi_ret;
        if ::core::ptr::eq(__orochi_ret, __orochi_self_ptr) {
            ::core::result::Result::Ok(::orochi::__private::self_object(__orochi_self))
        } else {
            (&&::orochi::__private::OtherInstance::<Self>::new())
                .other_kind()
                .convert(__orochi_ret, #what)
        }
    }};
    let convert = if kind == Kind::Ctor {
        quote_spanned! {span=>
            (&&&&::orochi::__private::CtorSpec::of(&__orochi_ret)).ctor_kind().ctor(__orochi_ret)
        }
    } else {
        match a.ret {
            Ret::Convert => quote_spanned! {span=>
                (&&&&::orochi::__private::RetSpec::of(&__orochi_ret)).ret_kind().ret(__orochi_ret)
            },
            Ret::SelfRef => same_instance,
            Ret::SelfRefResult => {
                let err = quote_spanned! {span=>
                    (&&&&::orochi::__private::RetSpec::of(&__orochi_err))
                        .ret_kind()
                        .ret(__orochi_err)
                };
                quote! {
                    match __orochi_ret {
                        ::core::result::Result::Ok(__orochi_ret) => #same_instance,
                        ::core::result::Result::Err(__orochi_err) => {
                            let __orochi_err: ::core::result::Result<(), _> =
                                ::core::result::Result::Err(__orochi_err);
                            #err
                        }
                    }
                }
            }
        }
    };
    let self_ptr = (kind != Kind::Ctor && a.ret != Ret::Convert)
        .then(|| quote!(let __orochi_self_ptr: *const Self = &*__orochi_guard;));
    quote! {
        #[allow(unused_imports)]
        use ::orochi::__private::{
            ClassParamKind as _, CtorBadErrorKind as _, CtorDisplayKind as _, CtorPlainKind as _,
            CtorResultKind as _, OtherCloneKind as _, OtherRejectKind as _, OwnedParamKind as _,
            RetBadErrorKind as _, RetDisplayKind as _, RetFallbackKind as _, RetIntoKind as _,
        };
        #(#holds)*
        #self_borrow
        #self_ptr
        #(#guards)*
        let __orochi_ret = #callee(#(#args),*);
        #convert
    }
}

// ---------------------------------------------------------------- attributes

fn doc_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .cloned()
        .collect()
}

fn cfg_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .cloned()
        .collect()
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
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) => {
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
    fn errors(args: TokenStream2, input: TokenStream2) -> Vec<String> {
        match expand(args, input) {
            Ok(_) => Vec::new(),
            Err(err) => err.into_iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn orochi_names_are_reserved() {
        let cases = [
            (quote!(), quote!(mod m { pub fn _orochi_stubs() {} }), "_orochi_stubs"),
            (quote!(), quote!(mod m { pub fn __orochi_init() {} }), "__orochi_init"),
            (quote!(), quote!(mod m { pub const _orochi: i64 = 1; }), "_orochi"),
            (quote!(), quote!(mod m { pub static __orochi_x: i64 = 1; }), "__orochi_x"),
            (quote!(), quote!(mod m { pub struct _orochiThing; }), "_orochiThing"),
            (quote!(), quote!(mod m { pub enum E { _orochi_member } }), "_orochi_member"),
            (quote!(), quote!(mod m { pub mod _orochi_sub {} }), "_orochi_sub"),
            (quote!(), quote!(mod _orochi_mod {}), "_orochi_mod"),
            (quote!(name = "_orochi_lib"), quote!(mod m {}), "_orochi_lib"),
            (quote!(), quote!(mod m { pub fn f(_orochi_x: i64) {} }), "_orochi_x"),
            (quote!(), quote!(mod m { pub fn f(__orochi_y: i64) {} }), "__orochi_y"),
            (
                quote!(),
                quote!(mod m { pub struct S { pub _orochi_field: i64 } }),
                "_orochi_field",
            ),
            (
                quote!(),
                quote!(mod m { pub struct S; impl S { pub fn _orochi_m(&self) {} } }),
                "_orochi_m",
            ),
            (
                quote!(),
                quote!(mod m { pub struct S; impl S { pub const _orochi_C: i64 = 1; } }),
                "_orochi_C",
            ),
        ];
        for (args, input, name) in cases {
            let errors = errors(args, input);
            let expected = format!(
                "`{name}` cannot be the name of an exported"
            );
            assert!(
                errors.len() == 1 && errors[0].starts_with(&expected),
                "{name}: {errors:?}"
            );
            assert!(errors[0].contains("reserved for orochi"), "{errors:?}");
        }
        // Only the prefix is reserved.
        assert!(errors(quote!(), quote!(mod m { pub fn orochi_stubs() {} pub fn _orochi2() {} })).len() == 1);
        assert!(errors(quote!(), quote!(mod m { pub fn orochi_stubs() {} pub fn x_orochi() {} })).is_empty());
    }
}