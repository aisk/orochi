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
//! Rust types (`i64`, `String`, `Vec<T>`, ...) are converted copies, orochi's
//! types ([`Object`], [`List`], [`Dict`], [`Set`], [`Ref`]) are the Python
//! objects themselves, and the module's own classes are borrowed.
//!
//! **The [reference] is the complete description**: every rule for
//! parameters, return values and classes, the conversion table, what the
//! macro generates, and what is not supported yet. The
//! [README](https://github.com/aisk/orochi/blob/master/README.md) explains
//! how to build an extension module.
//!
//! Where to start in the API:
//!
//! - [`module`]: the attribute macro.
//! - [`Object`], [`List`], [`Dict`], [`Set`], [`Ref`]: references to Python
//!   objects; [`IntoObject`] / [`FromObject`]: conversions of Rust values.
//! - [`import`], [`eval`], [`exec`], [`Scope`], [`Object::call`],
//!   [`kwargs!`], [`Object::from_fn`]: calling Python from Rust, inside an
//!   extension or in a program that embeds Python (with the `embed`
//!   feature).
//! - [`Error`]: a Python exception.
//! - [`ModuleBuilder`], [`ClassBuilder`]: writing a module by hand, without
//!   the macro.
//!
//! Features: `macros` (default) provides [`module`]; `embed` starts the
//! interpreter on first use, for programs and tests that embed Python.
//!
//! [reference]: https://github.com/aisk/orochi/blob/master/docs/reference.md

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
/// Items with plain `pub` visibility are exported; private and `pub(crate)`
/// items stay Rust-only. Doc comments become docstrings.
///
/// - `pub fn`: a function. Its parameter names are the Python names;
///   trailing `Option` parameters default to `None`.
/// - `pub struct`: a class. In its `impl` blocks, `pub fn new` is the
///   constructor, the other `pub fn`s are methods (with `self`) or static
///   methods, and `pub const`s are class attributes; `pub` fields are
///   properties (see [`Field`]). `Debug`, `Display`, `PartialEq` and `Hash`
///   give `__repr__`, `__str__`, `__eq__` and `__hash__`.
/// - `pub const` / `pub static`: a module attribute.
/// - inline `pub mod name { ... }`: the submodule `parent.name`.
///
/// The Python module name is the Rust module name;
/// `#[orochi::module(name = "other")]` sets another. The items stay
/// ordinary Rust items. The macro generates plain [`ModuleBuilder`] calls,
/// the extension's `PyInit_<name>` entry point, and `mylib::__orochi_module()`,
/// which builds the module in-process for tests ([`load_module`]).
///
/// The [reference] has the full rules for parameters, return values and
/// classes, the generated code, and the list of what is not supported yet.
///
/// [reference]: https://github.com/aisk/orochi/blob/master/docs/reference.md
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

/// Compiles and runs the examples of the reference as doctests.
#[cfg(doctest)]
#[doc = include_str!("../docs/reference.md")]
struct ReferenceDoctests;
