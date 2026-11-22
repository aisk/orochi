//! # orochi
//!
//! An easy-to-use layer over [pyo3](https://pyo3.rs) for writing Python
//! extension modules in Rust, in the spirit of pybind11 and nanobind.
//! Simplicity of the API is the goal; raw performance is not.
//!
//! You never deal with `Python<'py>` tokens, `Bound<'py, T>` or pyo3
//! attributes: with [`#[orochi::module]`](module) a Rust `mod` is the Python
//! module and `pub` means exported.
//!
//! ```
//! #[orochi::module]
//! mod mylib {
//!     /// Add two integers.
//!     pub fn add(a: i64, b: Option<i64>) -> i64 {
//!         a + b.unwrap_or(1)
//!     }
//!
//!     pub fn push_twice(xs: &mut Vec<i64>, v: i64) {
//!         xs.push(v);
//!         xs.push(v);
//!     }
//!
//!     #[derive(Debug, Clone, PartialEq)]
//!     pub struct Point {
//!         pub x: f64,
//!         pub y: f64,
//!     }
//!
//!     impl Point {
//!         pub fn new(x: f64, y: f64) -> Self {
//!             Point { x, y }
//!         }
//!         pub fn norm(&self) -> f64 {
//!             self.x.hypot(self.y)
//!         }
//!     }
//! }
//! # mylib::__orochi_module()?; // what Python's `import mylib` does
//! # orochi::exec(r#"
//! #     import mylib
//! #     assert mylib.add(1) == 2 and mylib.add(a=1, b=2) == 3
//! #     xs = [1]
//! #     mylib.push_twice(xs, 5)
//! #     assert xs == [1, 5, 5]
//! #     p = mylib.Point(3, 4)
//! #     assert p.norm() == 5.0 and p == mylib.Point(3, 4)
//! # "#)?;
//! # Ok::<(), orochi::Error>(())
//! ```
//!
//! See [`module`] for the rules (parameters follow Rust's ownership: values
//! are copies, `&T` is read-only, `&mut Vec<T>` changes the caller's list),
//! and the README for building the extension.
//!
//! ## Calling Python
//!
//! The same types work for calling Python from Rust, inside an extension or
//! in a program embedding Python. Every handle ([`Object`], [`List`],
//! [`Dict`], [`Set`], [`Ref`]) is `Clone + Send + Sync + 'static` and
//! attaches to the interpreter on its own.
//!
//! ```
//! use orochi::prelude::*;
//!
//! let json = orochi::import("json")?;
//! let data = json.method("loads", r#"{"a": [1, 2, 3]}"#)?;
//! let a: Vec<i64> = data.item("a")?.to()?;
//! assert_eq!(a, [1, 2, 3]);
//!
//! let text = json.method_kw("dumps", &data, kwargs! { "sort_keys" => true })?;
//! assert_eq!(text.to::<String>()?, r#"{"a": [1, 2, 3]}"#);
//! # Ok::<(), orochi::Error>(())
//! ```
//!
//! ## Conversions
//!
//! [`IntoObject`] and [`FromObject`] convert between Rust and Python. Owned
//! Rust types (`i64`, `String`, `Vec<T>`, `HashMap<K, V>`, ...) are copies;
//! [`Object`], the typed handles and [`Ref`] are live references. Conversions are
//! strict: a `float` is not an integer, a `bool` is not a number and a `str`
//! is not a list.
//!
//! ## Embedding
//!
//! Inside an extension module the interpreter is already running. To use
//! orochi from a plain Rust program (or its tests), enable the `embed`
//! feature, which starts the interpreter on first use. Without it, using
//! orochi before an interpreter exists panics with a message saying so.
//!
//! ## Threads and locks
//!
//! Every operation attaches the calling thread to the interpreter (taking
//! the GIL on non-free-threaded builds) for its duration. Do not hold a Rust
//! lock (`Mutex`, `RwLock`, ...) while calling into Python if another thread
//! may take that same lock while it is attached to Python: thread A holds the
//! mutex and waits for the GIL, thread B holds the GIL (e.g. inside a
//! function called from Python) and waits for the mutex, and both block
//! forever. Release the lock first, or only take it while not calling Python.

mod args;
mod callable;
mod class;
mod convert;
mod error;
mod eval;
mod handles;
mod object;
#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;
mod runtime;
mod stubs;

pub use args::{Args, IntoArgs, IntoKwargs, Kwargs};
#[doc(hidden)]
pub use callable::IntoPyFunction;
pub use class::{Class, Ref, RefGuard, RefMutGuard};
pub use convert::{FromObject, IntoObject};
pub use error::{Error, Result};
pub use eval::{Scope, builtins, eval, exec, import};
pub use handles::{Dict, List, Set};
pub use object::{Iter, Object};
pub use stubs::{Stubs, write_stubs};

#[doc(hidden)]
pub use pyo3;

/// Turns an inline Rust module into a Python extension module.
///
/// ```
/// #[orochi::module]
/// mod mylib {
///     //! The module docstring.
///     use orochi::prelude::*;
///
///     /// Becomes `mylib.add`, with this docstring.
///     pub fn add(a: i64, b: Option<i64>) -> i64 {
///         a + b.unwrap_or(1)
///     }
///
///     fn helper() {} // private: Rust-only
///
///     #[derive(Debug, Clone, PartialEq)]
///     pub struct Point { pub x: f64, pub y: f64 }
///
///     impl Point {
///         pub fn new(x: f64, y: f64) -> Self { Point { x, y } }
///         pub fn norm(&self) -> f64 { self.x.hypot(self.y) }
///     }
///
///     pub mod geometry {
///         pub fn area(r: f64) -> f64 { std::f64::consts::PI * r * r }
///     }
/// }
/// # mylib::__orochi_module()?;
/// # orochi::exec("import mylib\nassert mylib.add(2) == 3\nassert not hasattr(mylib, 'helper')")?;
/// # Ok::<(), orochi::Error>(())
/// ```
///
/// The attribute generates the `PyInit_<name>` entry point Python looks for
/// when it imports the compiled library (built as a `cdylib`). The Python
/// module name is the Rust module name; `#[orochi::module(name = "other")]`
/// overrides it. All items stay ordinary Rust items, callable from Rust.
///
/// # What is exported
///
/// Items with plain `pub` visibility; private and `pub(crate)` items are not
/// exported. Doc comments become `__doc__`.
///
/// - `pub fn` becomes a function. Parameter names are the Python names, so
///   arguments can be passed by keyword; trailing `Option<T>` parameters
///   default to `None`.
/// - `pub struct` becomes a class. In its inherent `impl` blocks, `pub fn new`
///   without `self` is the constructor (it returns `Self` or
///   `Result<Self, E>`), other `pub fn`s with `self` / `&self` / `&mut self`
///   are methods, those without are static methods, and `pub const`s are
///   class attributes. `pub` fields are read/write attributes. Reading one
///   returns a copy of the field, except that fields of type `Vec<T>`,
///   `HashMap`/`BTreeMap` or `HashSet`/`BTreeSet` (also in `Option`) read as
///   live views: `obj.items.append(1)` changes the struct, an element of the
///   wrong type raises `TypeError` at once, nested containers are live too,
///   and `list(view)` / `dict(view)` / `set(view)` / `view.copy()` give a
///   builtin copy. Derived or implemented `Debug`, `Display`, `PartialEq` and
///   `Hash` provide `__repr__`, `__str__`, `__eq__` and `__hash__` (with
///   `PartialEq` but without `Hash`, the class is unhashable, as in Python;
///   `Hash` without `PartialEq` is a compile error). Classes must be
///   `Send + Sync`.
/// - A `pub enum` whose variants have no fields becomes a class whose
///   members are singleton attributes (`f() is Color.Red`); members compare
///   and hash by variant, and only members convert back into the enum.
/// - `pub const` / `pub static` become module attributes (a copy of the
///   value).
/// - `pub mod name { ... }` becomes the submodule `parent.name` (also
///   importable as `import parent.name`).
///
/// # Parameters
///
/// Parameters follow Rust's ownership rules:
///
/// - **By value** (`i64`, `String`, `Vec<T>`, `HashMap<K, V>`, `PathBuf`, a
///   class): the function gets its own value, converted with [`FromObject`].
///   A class taken by value is a copy and must implement `Clone`.
/// - **`&T`** (`&str`, `&[T]`, `&Path`, `&HashMap<K, V>`, `&Point`,
///   `Option<&str>`, ...): read-only access during the call. Class instances
///   are borrowed in place; other values are converted into a temporary.
/// - **`&mut T`**: changes are visible to the caller. A class instance is
///   borrowed mutably in place. `&mut Vec<T>`, `&mut HashMap` /
///   `&mut BTreeMap` and `&mut HashSet` / `&mut BTreeSet` require a `list`,
///   `dict` or `set` argument, which is converted into a temporary and, after
///   the function returns (`Ok` or `Err`, but not after a panic), written back
///   into the same Python object. Other `&mut` parameters (`&mut String`,
///   `&mut i64`, ...) are compile errors: Python `str` and `int` objects are
///   immutable, so return the new value instead.
///
///   Write-back changes only what changed: an equal list is left alone, a
///   changed one is replaced as a whole (`xs[:] = new`); a dict or set gets
///   the removed entries deleted and new or changed (by `==`) ones assigned,
///   new dict keys in the Rust map's order (sorted for `BTreeMap`, arbitrary
///   for `HashMap`). Changed elements are converted in both directions: after
///   a call that changes a `&mut Vec<Point>`, the list holds new `Point`
///   objects (use `Vec<Ref<Point>>` or `List<..>` to keep their identity),
///   and changes made to the same container by other Python code during the
///   call are overwritten. The same container passed to two `&mut`
///   parameters raises `RuntimeError`.
/// - The handle types [`Object`], [`List`], [`Dict`], [`Set`] and [`Ref`]
///   refer to the caller's object itself and can be kept beyond the call.
///
/// Conversions are strict (see [`FromObject`]): `add(True)` and `add(1.5)`
/// raise `TypeError`.
///
/// Instance borrows are checked at runtime: a method taking `&mut self`
/// called with `other` being the same object as `self` raises
/// `RuntimeError` instead of aliasing.
///
/// ```
/// #[orochi::module]
/// mod lists {
///     use std::collections::HashMap;
///
///     pub fn push_twice(xs: &mut Vec<i64>, v: i64) {
///         xs.push(v);
///         xs.push(v);
///     }
///
///     pub fn bump(counts: &mut HashMap<String, i64>, key: &str) {
///         *counts.entry(key.to_owned()).or_insert(0) += 1;
///     }
/// }
/// # lists::__orochi_module()?;
/// # orochi::exec(r#"
/// #     import lists
/// #     xs = [1]; same = xs
/// #     lists.push_twice(xs, 2)
/// #     assert xs == [1, 2, 2] and xs is same
/// #     d = {}
/// #     lists.bump(d, "a")
/// #     assert d == {"a": 1}
/// # "#)?;
/// # Ok::<(), orochi::Error>(())
/// ```
///
/// # Return values
///
/// Owned values become new Python objects ([`IntoObject`]). A method
/// returning `&Self` / `&mut Self` (also inside `Result`) returns `self`'s
/// own Python object, so calls can be chained; if the reference points to a
/// different instance, it is copied for a `Clone` class and raises
/// `RuntimeError` otherwise. Other references (`&str`, `&[T]`, `&Point`) are
/// copied. `Result<T, E>` raises its error:
/// `E: Into<Error>` (e.g. `std::io::Error`, which maps to the matching
/// `OSError` subclass) raises that exception, any other `E: Display` raises
/// `RuntimeError(e.to_string())`. A panic raises
/// `pyo3_runtime.PanicException`.
///
/// # Sharing objects: `Ref<T>`
///
/// A `pub` field cannot have another class as its type by value (it would be
/// read as a copy, so `c.center.x = 5` would do nothing); use [`Ref<T>`] to
/// share one instance. See [`Ref`].
///
/// # Type stubs
///
/// The attribute also describes the module for `.pyi` stubs: write them from
/// Rust with `orochi::write_stubs(&mylib::__orochi_stubs(), dir)` (see
/// [`Stubs`]), or from Python with `mylib._orochi_stubs(dir)`, a hidden
/// function of the root module that returns `{relative path: text}` and,
/// given a directory, writes the files there (replacing stubs of the other
/// layout, single file or package, that orochi generated before).
///
/// # Not supported
///
/// These are compile errors on `pub` items (make the item private to keep it
/// Rust-only): generic parameters and explicit lifetimes, `async`, `unsafe`
/// and `extern` functions, `impl Trait` parameters or returns, parameters
/// that are patterns, special methods written by hand (`__len__`, ...),
/// tuple structs with `pub` fields, enums with data, `pub use`, `pub trait`,
/// `pub type`, `pub static mut`, non-inline `pub mod`, Python keywords as
/// names (`r#in`; `r#type` is fine), names starting with `_orochi` or
/// `__orochi` (reserved for orochi), `#[cfg]` on parameters or on the
/// `Debug`/`Display`/`PartialEq`/`Hash`/`Clone` impls of a class. Items
/// produced by `macro_rules!` macros inside the module are not exported (the
/// attribute only sees the macro call).
#[cfg(feature = "macros")]
pub use orochi_macros::module;

/// The most commonly used items: `use orochi::prelude::*;`.
pub mod prelude {
    pub use crate::{
        Args, Dict, Error, FromObject, IntoObject, List, Object, Ref, Result, Scope, Set, kwargs,
    };
}

/// Builds keyword arguments for [`Object::call_kw`] / [`Object::method_kw`].
///
/// Keys are anything convertible to `String`, values anything implementing
/// [`IntoObject`]. A repeated key raises `TypeError` when the call is made.
///
/// ```
/// use orochi::kwargs;
///
/// let dict = orochi::builtins().attr("dict")?;
/// let d = dict.call_kw((), kwargs! { "x" => 1, "y" => "two" })?;
/// assert_eq!(d.to_string(), "{'x': 1, 'y': 'two'}");
/// # Ok::<(), orochi::Error>(())
/// ```
#[macro_export]
macro_rules! kwargs {
    () => {
        $crate::Kwargs::new()
    };
    ($($key:expr => $value:expr),+ $(,)?) => {
        $crate::Kwargs::new()$(.arg($key, $value))+
    };
}

/// Compiles and runs the README example as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
