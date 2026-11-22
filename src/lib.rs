//! # orochi
//!
//! An easy-to-use layer over [pyo3](https://pyo3.rs) for working with Python
//! from Rust, in the spirit of pybind11 and nanobind. Simplicity of the API is
//! the goal; raw performance is not.
//!
//! You never deal with `Python<'py>` tokens, `Bound<'py, T>` or lifetimes:
//! every handle ([`Object`], [`List`], [`Dict`], [`Set`]) is
//! `Clone + Send + Sync + 'static` and attaches to the interpreter on its own.
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
//! [`Object`] and the typed handles are live references. Conversions are
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
mod convert;
mod error;
mod eval;
mod handles;
mod object;
mod runtime;

pub use args::{Args, IntoArgs, IntoKwargs, Kwargs};
#[doc(hidden)]
pub use callable::IntoPyFunction;
pub use convert::{FromObject, IntoObject};
pub use error::{Error, Result};
pub use eval::{Scope, builtins, eval, exec, import};
pub use handles::{Dict, List, Set};
pub use object::{Iter, Object};

#[doc(hidden)]
pub use pyo3;

/// The most commonly used items: `use orochi::prelude::*;`.
pub mod prelude {
    pub use crate::{
        Args, Dict, Error, FromObject, IntoObject, List, Object, Result, Scope, Set, kwargs,
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
