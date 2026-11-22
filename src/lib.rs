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
//!     use orochi::prelude::*;
//!
//!     /// Add two integers.
//!     pub fn add(a: i64, b: Option<i64>) -> i64 {
//!         a + b.unwrap_or(1)
//!     }
//!
//!     pub fn push_twice(xs: List<i64>, v: i64) -> Result<()> {
//!         xs.push(v)?;
//!         xs.push(v)
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
//! See [`module`] for the rules (Rust types are values, orochi types such as
//! [`List`] are references, your own classes are borrowed), and the README
//! for building the extension.
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
///   class attributes. `pub` fields are read/write attributes (see
///   [Fields](#fields)). Derived or implemented `Debug`, `Display`,
///   `PartialEq` and `Hash` provide `__repr__`, `__str__`, `__eq__` and
///   `__hash__` (with `PartialEq` but without `Hash`, the class is
///   unhashable, as in Python; `Hash` without `PartialEq` is a compile
///   error). Classes must be `Send + Sync`.
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
/// - **Rust types are values**: `i64`, `String`, `Vec<T>`, `HashMap<K, V>`,
///   `PathBuf`, tuples, ... are converted with [`FromObject`] into the
///   function's own value. `&str`, `&[T]`, `&Path`, `&HashMap<K, V>` and
///   `Option<&T>` borrow such a converted value; nothing the function does is
///   visible to the caller.
/// - **orochi types are references**: [`Object`], [`List`], [`Dict`], [`Set`]
///   and [`Ref`] refer to the caller's object itself, so a function taking
///   `List<T>` can modify the caller's list, and can keep it beyond the call.
/// - **Classes of the module** are borrowed in place as `&Point` /
///   `&mut Point` (a conflicting borrow, e.g. `p.absorb(p)` with
///   `&mut self` and `&Point`, raises `RuntimeError`); by value, a class is
///   a copy and must implement `Clone`.
///
/// `&mut` of any other type is a compile error. To modify the caller's list,
/// dict or set, take `List<T>` / `Dict<K, V>` / `Set<T>`, which need no
/// `&mut`; other Python values (`str`, `int`, tuples, ...) are immutable, so
/// return the new value. A `HashMap`/`BTreeMap` parameter accepts any
/// `collections.abc.Mapping`.
///
/// Conversions are strict (see [`FromObject`]): `add(True)` and `add(1.5)`
/// raise `TypeError`.
///
/// ```
/// #[orochi::module]
/// mod lists {
///     use orochi::prelude::*;
///
///     pub fn push_twice(xs: List<i64>, v: i64) -> Result<()> {
///         xs.push(v)?;
///         xs.push(v)
///     }
///
///     pub fn doubled(xs: Vec<i64>) -> Vec<i64> {
///         xs.into_iter().map(|x| 2 * x).collect()
///     }
/// }
/// # lists::__orochi_module()?;
/// # orochi::exec(r#"
/// #     import lists
/// #     xs = [1]; same = xs
/// #     lists.push_twice(xs, 2)
/// #     assert xs == [1, 2, 2] and xs is same
/// #     assert lists.doubled(xs) == [2, 4, 4] and xs == [1, 2, 2]
/// # "#)?;
/// # Ok::<(), orochi::Error>(())
/// ```
///
/// # Fields
///
/// A `pub` field follows the same rules. A field of a Rust type is a value:
/// reading it gives an immutable snapshot (`Vec<T>` and arrays a `tuple`,
/// sets a `frozenset`, maps a read-only `types.MappingProxyType`, also
/// nested; `Vec<u8>` is `bytes`), so `obj.items.append(1)` raises instead of
/// changing a copy; assigning replaces the whole value. A snapshot does not
/// compare equal to a list (`obj.items == [1]` is `False`); `list(...)`,
/// `dict(...)` and `set(...)` give mutable copies (e.g. for `json`). Orochi
/// references inside the value pass through, and a type with a hand-written
/// [`IntoObject`] is handed over exactly as it builds it. A field of an orochi
/// type is the Python object itself, mutable in place. A field cannot hold
/// another class by value (it would be read as a copy, so `c.center.x = 5`
/// would do nothing); use [`Ref<T>`] (`Vec<Ref<T>>`, ...) to share instances.
/// Enums are fine by value: their members are immutable singletons.
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
