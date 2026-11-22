//! # orochi
//!
//! An easy-to-use layer over [pyo3](https://pyo3.rs) for writing Python
//! extension modules in Rust, in the spirit of pybind11 and nanobind.
//! A simple API is the goal; raw performance is not.
//!
//! With [`#[orochi::module]`](module) a Rust `mod` is the Python module and
//! `pub` means exported:
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
//! The attribute only reads names, signatures and doc comments; it
//! generates plain calls to [`ModuleBuilder`] and [`ClassBuilder`], which
//! can also be written by hand. See [`module`] for the rules and the
//! generated code, and the README for building the extension.
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
//! [`Object`], the typed handles and [`Ref`] are live references.
//! Conversions are strict: a `float` is not an integer, a `bool` is not a
//! number and a `str` is not a list.
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
mod builder;
mod class;
mod convert;
mod error;
mod eval;
mod function;
mod handler;
mod handles;
mod object;
mod runtime;

pub use args::{Args, IntoArgs, IntoKwargs, Kwargs};
pub use builder::{ClassBuilder, Field, ModuleBuilder, construct, init_module, load_module};
pub use class::{Class, Ref, RefGuard, RefMutGuard};
pub use convert::{FromObject, IntoObject};
pub use error::{Error, Result};
pub use eval::{Scope, builtins, eval, exec, import};
pub use handler::{Arg, Handler, IntoInstance, Ret};
pub use handles::{Dict, List, Set};
pub use object::{Iter, Object};

/// The pyo3 version orochi is built on, for `#[pyclass]` and `#[pymodule]`
/// in hand-written modules (`#[orochi::pyo3::pyclass(crate = "orochi::pyo3")]`).
pub use pyo3;

/// Support code for `#[orochi::module]`; not a public API.
#[doc(hidden)]
pub mod __private {
    pub use crate::handler::{assert_arg, assert_new, assert_ret, marker};
}

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
/// The Python module name is the Rust module name;
/// `#[orochi::module(name = "other")]` overrides it. All items stay ordinary
/// Rust items, callable from Rust.
///
/// # What is exported
///
/// Items with plain `pub` visibility; private and `pub(crate)` items are
/// not. Doc comments become `__doc__`.
///
/// - `pub fn` is a function. Its parameter names are the Python names, so
///   arguments can be passed by keyword; trailing `Option` parameters
///   default to `None`.
/// - `pub struct` is a class. In its inherent `impl` blocks, `pub fn new`
///   without `self` is the constructor (returning `Self` or
///   `Result<Self, E>`), other `pub fn`s with `&self` / `&mut self` (or
///   `self` for a `Clone` class) are methods, those without `self` are
///   static methods, and `pub const`s are class attributes. `pub` fields are
///   read/write properties (see [`Field`]). Derived or implemented `Debug`,
///   `Display`, `PartialEq` and `Hash` give `__repr__`, `__str__`, `__eq__`
///   and `__hash__` (with `PartialEq` but without `Hash`, the class is
///   unhashable, as in Python). Classes must be `Send + Sync`.
/// - `pub const` / `pub static` are module attributes (a copy of the value).
/// - `pub mod name { ... }` is the submodule `parent.name` (also importable
///   as `import parent.name`).
///
/// # The three rules
///
/// 1. **Rust types are values**: `i64`, `String`, `Vec<T>`, `HashMap<K, V>`,
///    `PathBuf`, tuples, ... are converted with [`FromObject`] /
///    [`IntoObject`]; nothing the function does to them is visible to the
///    caller. `&str`, `&[T]`, `&Path`, `&HashMap<K, V>` and `Option<&str>`
///    parameters borrow such a converted copy.
/// 2. **orochi types are references**: [`Object`], [`List`], [`Dict`],
///    [`Set`] and [`Ref`] refer to the caller's object itself, so a function
///    taking `List<T>` can modify the caller's list.
/// 3. **Classes of the module are borrowed**: `&self`, `&mut self`,
///    `&Point`, `&mut Point` and `Option<&Point>` borrow the instance itself;
///    a conflicting borrow (e.g. `p.absorb(p)` with `&mut self` and
///    `&Point`) raises `RuntimeError`. A class by value (`p: Point`) is a
///    copy and needs `Clone`.
///
/// `&mut` of anything but a class is a compile error: take `List<T>`,
/// `Dict<K, V>` or `Set<T>` to modify the caller's container, and return
/// new values otherwise. Conversions are strict (`add(True)` and
/// `add(1.5)` raise `TypeError`, naming the argument).
///
/// # Return values
///
/// - any [`IntoObject`] type: a new Python object, or the object itself for
///   a reference type ([`Object`], [`List`], [`Ref<T>`](Ref), ...);
///   `Option` and `Result` of those, `()` (`None`), tuples and collections
///   of those (`Vec<Ref<Point>>`);
/// - a class by value (a new instance) as `T`, `Option<T>` or
///   `Result<T, E>`.
///
/// `Result<_, E>` needs `E: Into<orochi::Error>` (`std::io::Error` maps to
/// the matching `OSError` subclass) and raises on `Err`. Borrowed values
/// cannot be returned: return `String`, not `&str`; a method that modifies
/// `self` returns `()`. A panic raises `pyo3_runtime.PanicException`.
///
/// # What it generates
///
/// Each `pub struct` gets pyo3's `#[pyclass]` and, if it has `pub fn new`,
/// a `#[pymethods]` block with a `#[new]` that calls [`construct`] (so the
/// instance is created only by `new`). Every module gets an
/// `__orochi_init` function of [`ModuleBuilder`] calls, the root module a
/// `#[pymodule]` (the `PyInit_<name>` entry point) that runs it, and
/// `__orochi_module()`, which builds the module in-process
/// ([`load_module`]) for tests and embedding. For the example above:
///
/// ```ignore
/// #[::orochi::pyo3::pymethods(crate = "::orochi::pyo3")]
/// impl Point {
///     #[new]
///     #[pyo3(signature = (*args, **kwargs), text_signature = "(x, y)")]
///     fn __orochi_new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
///         ::orochi::construct(&["x", "y"], Point::new, args, kwargs)
///     }
/// }
///
/// pub fn __orochi_init(__orochi_m: &::orochi::ModuleBuilder<'_>) -> ::orochi::Result<()> {
///     __orochi_m.doc("The module docstring.")?;
///     let __orochi_c = __orochi_m.class::<Point>()?;
///     __orochi_c.field("x", "", |s: &Point| &s.x, |s: &mut Point| &mut s.x)?;
///     __orochi_c.field("y", "", |s: &Point| &s.y, |s: &mut Point| &mut s.y)?;
///     __orochi_c.method("norm", &[], "", Point::norm)?;
///     __orochi_c.repr_debug()?;
///     __orochi_c.eq()?;
///     __orochi_m.function("add", &["a", "b"], "Becomes `mylib.add`, with this docstring.", self::add)?;
///     __orochi_m.submodule("geometry", "", self::geometry::__orochi_init)?;
///     Ok(())
/// }
/// ```
///
/// (plus compile-time checks that report a parameter or return type
/// without a conversion on that type). The doc comment of `new` is appended
/// to the class doc. `Debug`, `Display`, `PartialEq` and `Hash` are
/// recognized by name, in `#[derive(...)]` or `impl ... for` the class.
///
/// # Not supported (yet)
///
/// Compile errors on `pub` items (make the item private to keep it
/// Rust-only): enums, generic parameters and explicit lifetimes (`'_` is
/// fine), `async`, `unsafe` and `extern` functions, more than 12
/// parameters, `impl Trait`, parameters that are patterns, returning
/// borrowed values, special methods written by hand (`__len__`, ...), tuple
/// structs with `pub` fields, `pub` fields that are not a [`Field`]
/// (`Vec<T>`, maps, classes by value), `pub use`, `pub trait`, `pub type`,
/// `pub static mut`, non-inline `pub mod`, `impl` blocks with `pub` items
/// for a struct of another module, Python keywords as names (`r#in`), names
/// starting with `__orochi`, `#[cfg]` on parameters or on the
/// `Debug`/`Display`/`PartialEq`/`Hash` impls of a class.
///
/// Classes by value only work alone, in `Option` or in `Result` as return
/// values, and alone as parameters: not `Vec<Point>`, `(Point, Point)` or
/// `Result<Option<Point>, E>` returns, nor `Option<Point>` or `Vec<Point>`
/// parameters; use `Ref<Point>` there.
///
/// Silently not seen: items produced by `macro_rules!` macros inside the
/// module, and derives behind `#[cfg_attr(..., derive(...))]`. There are no
/// `.pyi` stubs, no subclassing, no keyword-only or `*args` parameters, and
/// no Rust-side defaults other than `None`.
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
