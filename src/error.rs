//! The [`Error`] type: a Python exception carried through Rust code.

use std::fmt;

use pyo3::exceptions::{
    PyAttributeError, PyBaseException, PyIndexError, PyKeyError, PyRuntimeError, PyTypeError,
    PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::{PyString, PyType};

use crate::object::Object;
use crate::runtime::{attach, try_attach};

/// Result alias used throughout orochi.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A Python exception.
///
/// Every fallible orochi operation returns this type. It either wraps an
/// exception raised by Python code, or one created in Rust (for example with
/// [`Error::value_error`]) that will be raised when it crosses back into
/// Python.
///
/// `Display` prints one line, `ExceptionType: message`. The alternate form
/// `{:#}` prints the full Python rendering: traceback, notes and chained
/// exceptions. `Debug` includes the traceback too.
///
/// For hand-written pyo3 code, `Error` converts to and from pyo3's `PyErr`
/// with `From` / `Into` (so `?` works both ways), and `as_ref()` borrows the
/// `PyErr`.
///
/// ```
/// let err = orochi::eval("1 / 0").unwrap_err();
/// assert!(err.is_instance_of("ZeroDivisionError"));
/// assert!(err.is_instance_of("ArithmeticError"));
/// assert_eq!(err.type_name(), "ZeroDivisionError");
/// assert_eq!(err.message(), "division by zero");
/// assert_eq!(err.to_string(), "ZeroDivisionError: division by zero");
/// assert!(format!("{err:#}").starts_with("Traceback (most recent call last):"));
/// ```
pub struct Error(PyErr);

impl Error {
    /// Creates an exception of the given type with a message.
    ///
    /// `exception_type` is either the name of a builtin exception
    /// (`"ValueError"`, `"KeyError"`, ...) or a dotted path to an exception
    /// class in an importable module (the module is imported if needed).
    ///
    /// Returns a `TypeError` instead if `exception_type` does not name an
    /// exception class (a misspelled name, say), or if the class cannot be
    /// constructed from a single message argument (for example
    /// `UnicodeDecodeError`, which needs five; the construction error is the
    /// `TypeError`'s cause). Use [`Error::from_value`] for exceptions that
    /// need other arguments.
    ///
    /// ```
    /// let err = orochi::Error::new("LookupError", "not here");
    /// assert_eq!(err.to_string(), "LookupError: not here");
    ///
    /// let err = orochi::Error::new("concurrent.futures.CancelledError", "stop");
    /// assert_eq!(err.type_name(), "CancelledError");
    ///
    /// let err = orochi::Error::new("LookupErorr", "typo");
    /// assert_eq!(err.to_string(), "TypeError: 'LookupErorr' is not an exception type");
    /// ```
    pub fn new(exception_type: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        attach(|py| match resolve_exception_type(py, exception_type, true) {
            Some(ty) => instantiate(&ty, message, exception_type),
            None => Error::type_error(format!("'{exception_type}' is not an exception type")),
        })
    }

    /// Creates an exception from a Python exception class object (for
    /// example a custom exception class defined in Python).
    ///
    /// Returns a `TypeError` instead if `exception_type` is not an exception
    /// class, or if it cannot be constructed from a single message argument
    /// (see [`Error::new`]).
    pub fn from_type(exception_type: &Object, message: impl Into<String>) -> Self {
        let message = message.into();
        attach(|py| {
            let ty = exception_type.bind(py);
            match ty.cast::<PyType>() {
                Ok(ty) if ty.is_subclass_of::<PyBaseException>().unwrap_or(false) => {
                    let name = ty.name().map(|n| n.to_string()).unwrap_or_default();
                    instantiate(ty, message, &name)
                }
                _ => Error::type_error(format!(
                    "exceptions must derive from BaseException, not {}",
                    exception_type.type_name()
                )),
            }
        })
    }

    /// Wraps an exception instance (e.g. one constructed with
    /// [`Object::call`]). Returns a `TypeError` if `value` is not an
    /// exception instance.
    pub fn from_value(value: &Object) -> Self {
        attach(|py| Error(PyErr::from_value(value.bind(py).clone())))
    }

    /// Creates a `ValueError`.
    pub fn value_error(message: impl Into<String>) -> Self {
        Error(PyValueError::new_err(message.into()))
    }

    /// Creates a `TypeError`.
    pub fn type_error(message: impl Into<String>) -> Self {
        Error(PyTypeError::new_err(message.into()))
    }

    /// Creates a `RuntimeError`.
    pub fn runtime_error(message: impl Into<String>) -> Self {
        Error(PyRuntimeError::new_err(message.into()))
    }

    /// Creates a `KeyError`.
    pub fn key_error(message: impl Into<String>) -> Self {
        Error(PyKeyError::new_err(message.into()))
    }

    /// Creates an `IndexError`.
    pub fn index_error(message: impl Into<String>) -> Self {
        Error(PyIndexError::new_err(message.into()))
    }

    /// Creates an `AttributeError`.
    pub fn attribute_error(message: impl Into<String>) -> Self {
        Error(PyAttributeError::new_err(message.into()))
    }

    /// Creates an `OverflowError`.
    pub fn overflow_error(message: impl Into<String>) -> Self {
        Error(pyo3::exceptions::PyOverflowError::new_err(message.into()))
    }

    /// The exception class name, e.g. `"ZeroDivisionError"`.
    pub fn type_name(&self) -> String {
        attach(|py| self.type_name_in(py))
    }

    fn type_name_in(&self, py: Python<'_>) -> String {
        self.0
            .get_type(py)
            .name()
            .map(|n| n.to_string())
            .unwrap_or_else(|_| "<unknown>".to_owned())
    }

    /// The exception message, i.e. Python's `str(exception)`.
    pub fn message(&self) -> String {
        attach(|py| self.message_in(py))
    }

    fn message_in(&self, py: Python<'_>) -> String {
        self.0
            .value(py)
            .str()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "<unprintable exception>".to_owned())
    }

    /// The formatted traceback (`Traceback (most recent call last): ...`),
    /// or `None` if the exception was never raised in Python.
    pub fn traceback(&self) -> Option<String> {
        attach(|py| {
            use pyo3::types::PyTracebackMethods;
            self.0.traceback(py).and_then(|tb| tb.format().ok())
        })
    }

    /// Returns `true` if this exception is an instance of the named exception
    /// class (subclasses match too, like Python's `except`).
    ///
    /// `exception_type` is a builtin exception name or a dotted
    /// `module.Class` path. A dotted path is looked up among the *already
    /// imported* modules only (nothing is imported as a side effect); if the
    /// module is not loaded, no exception can be an instance of its classes
    /// and this returns `false`. A name that does not resolve to an exception
    /// class (a misspelled name, say) also returns `false`.
    pub fn is_instance_of(&self, exception_type: &str) -> bool {
        attach(|py| match resolve_exception_type(py, exception_type, false) {
            Some(ty) => self.0.is_instance(py, ty.as_any()),
            None => false,
        })
    }

    /// Returns `true` if this exception is an instance of the given exception
    /// class object.
    pub fn is_instance(&self, exception_type: &Object) -> bool {
        attach(|py| self.0.is_instance(py, exception_type.bind(py)))
    }

    /// The exception instance as an [`Object`] (e.g. to read attributes such
    /// as `errno`, `args` or `__notes__`).
    pub fn value(&self) -> Object {
        attach(|py| Object::from_bound(self.0.value(py).clone().into_any()))
    }

    /// Unwraps the underlying pyo3 error.
    pub(crate) fn into_pyerr(self) -> PyErr {
        self.0
    }
}

/// Resolves a builtin exception name or a dotted `module.Class` path to an
/// exception class. Modules are imported only if `import` is set; otherwise
/// they are taken from `sys.modules`.
fn resolve_exception_type<'py>(
    py: Python<'py>,
    name: &str,
    import: bool,
) -> Option<Bound<'py, PyType>> {
    let candidate = match name.rsplit_once('.') {
        Some((module, attr)) => {
            let module = if import {
                py.import(module).ok()?.into_any()
            } else {
                py.import("sys").ok()?.getattr("modules").ok()?.get_item(module).ok()?
            };
            module.getattr(attr).ok()?
        }
        None => py.import("builtins").ok()?.getattr(name).ok()?,
    };
    let ty = candidate.cast_into::<PyType>().ok()?;
    if ty.is_subclass_of::<PyBaseException>().ok()? { Some(ty) } else { None }
}

/// Instantiates `ty(message)` eagerly, so that a class that cannot be built
/// from one argument is reported here (as a `TypeError` caused by the
/// construction error) and not replaced by a `TypeError` when the error is
/// first used.
fn instantiate(ty: &Bound<'_, PyType>, message: String, name: &str) -> Error {
    match ty.call1((message,)) {
        Ok(value) => Error(PyErr::from_value(value)),
        Err(err) => {
            let py = ty.py();
            let error = PyTypeError::new_err(format!(
                "exception type '{name}' cannot be constructed from a single message argument"
            ));
            error.set_cause(py, Some(err));
            Error(error)
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate()
            && let Some(Ok(text)) = try_attach(|py| render_full(py, &self.0))
        {
            return f.write_str(&text);
        }
        match try_attach(|py| (self.type_name_in(py), self.message_in(py))) {
            Some((ty, msg)) if msg.is_empty() => f.write_str(&ty),
            Some((ty, msg)) => write!(f, "{ty}: {msg}"),
            None => f.write_str("<Python exception (interpreter unavailable)>"),
        }
    }
}

/// Python's full rendering (traceback, notes, chained exceptions). Uses the
/// three-argument form of `format_exception`, which works on Python 3.9+.
fn render_full(py: Python<'_>, err: &PyErr) -> PyResult<String> {
    let value = err.value(py);
    let lines = py.import("traceback")?.call_method1(
        "format_exception",
        (value.get_type(), value, value.getattr("__traceback__")?),
    )?;
    let text = PyString::new(py, "").call_method1("join", (lines,))?;
    Ok(text.extract::<String>()?.trim_end().to_owned())
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `unwrap()` prints `Debug`, so include everything useful.
        let details = try_attach(|py| {
            use pyo3::types::PyTracebackMethods;
            (
                self.type_name_in(py),
                self.message_in(py),
                self.0.traceback(py).and_then(|tb| tb.format().ok()),
            )
        });
        match details {
            Some((ty, msg, tb)) => f
                .debug_struct("Error")
                .field("type", &ty)
                .field("message", &msg)
                .field("traceback", &tb)
                .finish(),
            None => f.write_str("Error(<interpreter unavailable>)"),
        }
    }
}

impl std::error::Error for Error {}

impl From<PyErr> for Error {
    fn from(err: PyErr) -> Self {
        Error(err)
    }
}

impl From<Error> for PyErr {
    fn from(err: Error) -> Self {
        err.0
    }
}

impl AsRef<PyErr> for Error {
    fn as_ref(&self) -> &PyErr {
        &self.0
    }
}

impl<'a, 'py> From<pyo3::CastError<'a, 'py>> for Error {
    fn from(err: pyo3::CastError<'a, 'py>) -> Self {
        Error(err.into())
    }
}

impl<'py> From<pyo3::CastIntoError<'py>> for Error {
    fn from(err: pyo3::CastIntoError<'py>) -> Self {
        Error(err.into())
    }
}

/// Maps to the matching `OSError` subclass (`FileNotFoundError`,
/// `PermissionError`, ...), exactly like pyo3 does.
impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error(err.into())
    }
}

/// Errors mapped exactly as pyo3 maps them.
macro_rules! via_pyerr {
    ($($(#[$doc:meta])* $t:ty;)*) => {$(
        $(#[$doc])*
        impl From<$t> for Error {
            fn from(err: $t) -> Self {
                Error(err.into())
            }
        }
    )*};
}

via_pyerr! {
    /// Maps to `UnicodeDecodeError` (with the offending bytes).
    std::string::FromUtf8Error;
    /// Maps to `ValueError`.
    std::num::ParseIntError;
    /// Maps to `ValueError`.
    std::num::ParseFloatError;
    /// Maps to `ValueError`.
    std::num::TryFromIntError;
    /// Maps to `ValueError`.
    std::str::ParseBoolError;
    /// Maps to `ValueError`.
    std::char::ParseCharError;
    /// Maps to `ValueError`.
    std::array::TryFromSliceError;
    /// Maps to `ValueError`.
    std::ffi::NulError;
}

/// Maps to `ValueError` (`UnicodeDecodeError` is a subclass of it, but cannot
/// be built without the source bytes, which `Utf8Error` does not carry).
impl From<std::str::Utf8Error> for Error {
    fn from(err: std::str::Utf8Error) -> Self {
        Error::value_error(err.to_string())
    }
}

/// Maps to `RuntimeError`.
impl From<std::fmt::Error> for Error {
    fn from(err: std::fmt::Error) -> Self {
        Error::runtime_error(err.to_string())
    }
}

/// Never constructed; lets infallible conversions use `?`.
impl From<std::convert::Infallible> for Error {
    fn from(err: std::convert::Infallible) -> Self {
        match err {}
    }
}
