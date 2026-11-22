//! The dynamic [`Object`] handle.

use std::fmt;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyIterator, PyTuple};

use crate::args::{IntoArgs, IntoKwargs};
use crate::convert::{FromObject, IntoObject};
use crate::error::{Error, Result};
use crate::runtime::{attach, try_attach};

/// A reference to any Python object.
///
/// `Object` owns a strong reference, has no lifetime and is
/// `Clone + Send + Sync + 'static`, so it can be stored in structs, sent to
/// other threads and kept for as long as needed. Every method attaches to the
/// Python interpreter internally.
///
/// `Display` is Python's `str()`, `Debug` is Python's `repr()`; neither ever
/// panics (a placeholder is printed if Python fails).
///
/// ```
/// use orochi::Object;
///
/// let math = orochi::import("math")?;
/// let pi: f64 = math.attr("pi")?.to()?;
/// assert!((pi - std::f64::consts::PI).abs() < 1e-12);
///
/// let root = math.method("sqrt", 16)?;
/// assert_eq!(root.to::<f64>()?, 4.0);
/// assert_eq!(root.to_string(), "4.0");
/// # Ok::<(), orochi::Error>(())
/// ```
///
/// # Interpreter lifetime
///
/// Methods panic if the interpreter is not running (see the `embed`
/// feature) or is shutting down. Objects kept in Rust `static`s or in
/// threads that outlive the interpreter must not be used after Python has
/// begun finalization; dropping them then is harmless.
pub struct Object(Py<PyAny>);

impl Object {
    /// Converts any Rust value into a Python object; shorthand for
    /// [`IntoObject::into_object`].
    ///
    /// ```
    /// let xs = orochi::Object::new(vec![1, 2, 3])?;
    /// assert_eq!(xs.len()?, 3);
    /// # Ok::<(), orochi::Error>(())
    /// ```
    pub fn new(value: impl IntoObject) -> Result<Object> {
        value.into_object()
    }

    /// Python's `None`.
    pub fn none() -> Object {
        attach(|py| Object(py.None()))
    }

    /// Wraps a Rust closure as a Python callable.
    ///
    /// The closure may take up to 8 arguments of any [`FromObject`] type and
    /// return any [`IntoObject`] type (including [`Result`](crate::Result)).
    /// Arguments are converted strictly; a wrong argument count or keyword
    /// arguments raise `TypeError`, and a failed argument conversion raises
    /// the conversion error with a note naming the argument.
    ///
    /// A Rust panic inside the closure does not unwind through Python: it is
    /// raised as `pyo3_runtime.PanicException` (a `BaseException`), and if
    /// that exception propagates back into Rust the panic resumes there.
    ///
    /// ```
    /// use orochi::Object;
    ///
    /// let add = Object::from_fn(|a: i64, b: i64| a + b);
    /// assert_eq!(add.call((1, 2))?.to::<i64>()?, 3);
    ///
    /// let checked = Object::from_fn(|x: f64| -> orochi::Result<f64> {
    ///     if x < 0.0 {
    ///         return Err(orochi::Error::value_error("negative"));
    ///     }
    ///     Ok(x.sqrt())
    /// });
    /// assert!(checked.call(-1.0).unwrap_err().is_instance_of("ValueError"));
    /// # Ok::<(), orochi::Error>(())
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if Python fails to allocate the function object.
    pub fn from_fn<F, Marker>(f: F) -> Object
    where
        F: crate::callable::IntoPyFunction<Marker>,
    {
        crate::callable::make_function(f)
    }

    /// Reads attribute `name` (Python's `obj.name`).
    pub fn attr(&self, name: &str) -> Result<Object> {
        attach(|py| Ok(Object::from_bound(self.bind(py).getattr(name)?)))
    }

    /// Sets attribute `name` (Python's `obj.name = value`).
    pub fn set_attr(&self, name: &str, value: impl IntoObject) -> Result<()> {
        let value = value.into_object()?;
        attach(|py| Ok(self.bind(py).setattr(name, value.bind(py))?))
    }

    /// Returns `true` if the object has attribute `name` (Python's
    /// `hasattr`).
    pub fn has_attr(&self, name: &str) -> Result<bool> {
        attach(|py| Ok(self.bind(py).hasattr(name)?))
    }

    /// Deletes attribute `name` (Python's `del obj.name`).
    pub fn del_attr(&self, name: &str) -> Result<()> {
        attach(|py| Ok(self.bind(py).delattr(name)?))
    }

    /// Calls the object.
    ///
    /// `args` is `()` for no arguments, a single plain value such as `2.0`,
    /// `"x"` or an `&Object` for one argument, or a tuple `(a, b, ...)` for
    /// several. Collections must be wrapped in a tuple: `(vec,)`. See
    /// [`IntoArgs`].
    pub fn call(&self, args: impl IntoArgs) -> Result<Object> {
        self.call_impl(args.into_args()?, None)
    }

    /// Calls the object with positional and keyword arguments.
    ///
    /// ```
    /// use orochi::kwargs;
    ///
    /// let int = orochi::builtins().attr("int")?;
    /// let n = int.call_kw("ff", kwargs! { "base" => 16 })?;
    /// assert_eq!(n.to::<i64>()?, 255);
    /// # Ok::<(), orochi::Error>(())
    /// ```
    pub fn call_kw(&self, args: impl IntoArgs, kwargs: impl IntoKwargs) -> Result<Object> {
        let args = args.into_args()?;
        let kwargs = kwargs.into_kwargs()?;
        self.call_impl(args, Some(kwargs))
    }

    /// Calls method `name` (Python's `obj.name(*args)`).
    pub fn method(&self, name: &str, args: impl IntoArgs) -> Result<Object> {
        self.attr(name)?.call(args)
    }

    /// Calls method `name` with positional and keyword arguments.
    pub fn method_kw(
        &self,
        name: &str,
        args: impl IntoArgs,
        kwargs: impl IntoKwargs,
    ) -> Result<Object> {
        self.attr(name)?.call_kw(args, kwargs)
    }

    fn call_impl(&self, args: Vec<Object>, kwargs: Option<Vec<(String, Object)>>) -> Result<Object> {
        attach(|py| {
            let args = PyTuple::new(py, args.iter().map(|a| a.bind(py)))?;
            let kwargs = match kwargs {
                Some(pairs) => {
                    let dict = PyDict::new(py);
                    for (k, v) in &pairs {
                        if dict.contains(k)? {
                            return Err(Error::type_error(format!(
                                "got multiple values for keyword argument '{k}'"
                            )));
                        }
                        dict.set_item(k, v.bind(py))?;
                    }
                    Some(dict)
                }
                None => None,
            };
            Ok(Object::from_bound(self.bind(py).call(args, kwargs.as_ref())?))
        })
    }

    /// Reads `obj[key]`.
    pub fn item(&self, key: impl IntoObject) -> Result<Object> {
        let key = key.into_object()?;
        attach(|py| Ok(Object::from_bound(self.bind(py).get_item(key.bind(py))?)))
    }

    /// Performs `obj[key] = value`.
    pub fn set_item(&self, key: impl IntoObject, value: impl IntoObject) -> Result<()> {
        let key = key.into_object()?;
        let value = value.into_object()?;
        attach(|py| Ok(self.bind(py).set_item(key.bind(py), value.bind(py))?))
    }

    /// Performs `del obj[key]`.
    pub fn del_item(&self, key: impl IntoObject) -> Result<()> {
        let key = key.into_object()?;
        attach(|py| Ok(self.bind(py).del_item(key.bind(py))?))
    }

    /// Python's `item in obj`.
    pub fn contains(&self, item: impl IntoObject) -> Result<bool> {
        let item = item.into_object()?;
        attach(|py| Ok(self.bind(py).contains(item.bind(py))?))
    }

    /// Python's `obj == other`, interpreted as a truth value.
    ///
    /// ```
    /// let x = orochi::eval("[1, 2]")?;
    /// assert!(x.eq(vec![1, 2])?);
    /// assert!(orochi::eval("1")?.eq(1.0)?);
    /// # Ok::<(), orochi::Error>(())
    /// ```
    pub fn eq(&self, other: impl IntoObject) -> Result<bool> {
        let other = other.into_object()?;
        attach(|py| Ok(self.bind(py).eq(other.bind(py))?))
    }

    /// Converts the object to a Rust value (see [`FromObject`]).
    ///
    /// ```
    /// let v: Vec<i64> = orochi::eval("[1, 2, 3]")?.to()?;
    /// assert_eq!(v, [1, 2, 3]);
    /// assert!(orochi::eval("1.5")?.to::<i64>().is_err());
    /// # Ok::<(), orochi::Error>(())
    /// ```
    pub fn to<T: FromObject>(&self) -> Result<T> {
        T::from_object(self)
    }

    /// Starts iterating the object (Python's `iter(obj)`).
    ///
    /// ```
    /// let mut total = 0;
    /// for item in orochi::eval("range(4)")?.iter()? {
    ///     total += item?.to::<i64>()?;
    /// }
    /// assert_eq!(total, 6);
    /// # Ok::<(), orochi::Error>(())
    /// ```
    pub fn iter(&self) -> Result<Iter> {
        attach(|py| {
            let it = PyIterator::from_object(self.bind(py))?;
            Ok(Iter(it.unbind()))
        })
    }

    /// Python's `len(obj)`.
    pub fn len(&self) -> Result<usize> {
        attach(|py| Ok(self.bind(py).len()?))
    }

    /// Returns `true` if `len(obj) == 0`.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Python's `bool(obj)`.
    pub fn is_truthy(&self) -> Result<bool> {
        attach(|py| Ok(self.bind(py).is_truthy()?))
    }

    /// Returns `true` if the object is `None`.
    pub fn is_none(&self) -> bool {
        attach(|py| self.bind(py).is_none())
    }

    /// Python's `isinstance(obj, ty)`. `ty` may be a class or a tuple of
    /// classes.
    pub fn is_instance(&self, ty: &Object) -> Result<bool> {
        attach(|py| Ok(self.bind(py).is_instance(ty.bind(py))?))
    }

    /// Identity check (Python's `obj is other`).
    pub fn is(&self, other: &Object) -> bool {
        self.0.is(&other.0)
    }

    /// The object's type (Python's `type(obj)`).
    pub fn type_object(&self) -> Object {
        attach(|py| Object::from_bound(self.bind(py).get_type().into_any()))
    }

    /// The name of the object's type, e.g. `"int"` or `"list"`.
    pub fn type_name(&self) -> String {
        attach(|py| type_name_of(self.bind(py)))
    }

    /// Python's `repr(obj)`.
    pub fn repr(&self) -> Result<String> {
        attach(|py| Ok(self.bind(py).repr()?.to_string()))
    }

    /// Python's `str(obj)`.
    pub fn str(&self) -> Result<String> {
        attach(|py| Ok(self.bind(py).str()?.to_string()))
    }

    // ---- pyo3 interop (used by orochi's own layers) ----

    /// Wraps a pyo3 bound reference.
    #[doc(hidden)]
    pub fn from_bound(obj: Bound<'_, PyAny>) -> Object {
        Object(obj.unbind())
    }

    /// Wraps a pyo3 owned reference.
    #[doc(hidden)]
    pub fn from_py(obj: Py<PyAny>) -> Object {
        Object(obj)
    }

    /// Borrows the object as a pyo3 bound reference.
    #[doc(hidden)]
    pub fn bind<'py>(&self, py: Python<'py>) -> &Bound<'py, PyAny> {
        self.0.bind(py)
    }

    /// Borrows the underlying pyo3 reference.
    #[doc(hidden)]
    pub fn as_py(&self) -> &Py<PyAny> {
        &self.0
    }

    /// Unwraps the underlying pyo3 reference.
    #[doc(hidden)]
    pub fn into_py(self) -> Py<PyAny> {
        self.0
    }

    /// Formats with `str()` or `repr()` without ever panicking.
    fn format_with(&self, f: &mut fmt::Formatter<'_>, use_repr: bool) -> fmt::Result {
        let text = try_attach(|py| {
            let b = self.bind(py);
            let s = if use_repr { b.repr() } else { b.str() };
            s.map(|s| s.to_string()).map_err(|_| type_name_of(b))
        });
        match text {
            Some(Ok(s)) => f.write_str(&s),
            Some(Err(ty)) if use_repr => write!(f, "<unrepresentable {ty} object>"),
            Some(Err(ty)) => write!(f, "<unprintable {ty} object>"),
            // The interpreter is unavailable, or formatting raised a
            // `PanicException` (which pyo3 resumes as a Rust panic).
            None if use_repr => f.write_str("<unrepresentable object>"),
            None => f.write_str("<unprintable object>"),
        }
    }
}

fn type_name_of(obj: &Bound<'_, PyAny>) -> String {
    obj.get_type()
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "<unknown>".to_owned())
}

impl Clone for Object {
    fn clone(&self) -> Self {
        attach(|py| Object(self.0.clone_ref(py)))
    }
}

impl From<Py<PyAny>> for Object {
    fn from(obj: Py<PyAny>) -> Self {
        Object(obj)
    }
}

impl<'py> From<Bound<'py, PyAny>> for Object {
    fn from(obj: Bound<'py, PyAny>) -> Self {
        Object(obj.unbind())
    }
}

impl From<Object> for Py<PyAny> {
    fn from(obj: Object) -> Self {
        obj.0
    }
}

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format_with(f, false)
    }
}

impl fmt::Debug for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format_with(f, true)
    }
}

/// Iterator over a Python iterable, created by [`Object::iter`].
///
/// Each step yields `Ok(item)` or the `Err` raised by Python's `__next__`.
pub struct Iter(Py<PyIterator>);

impl fmt::Debug for Iter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let repr = try_attach(|py| self.0.bind(py).repr().map(|r| r.to_string()).ok()).flatten();
        f.debug_tuple("Iter").field(&repr.unwrap_or_default()).finish()
    }
}

impl Iterator for Iter {
    type Item = Result<Object>;

    fn next(&mut self) -> Option<Self::Item> {
        attach(|py| {
            let mut it = self.0.bind(py).clone();
            it.next()
                .map(|item| item.map(Object::from_bound).map_err(Into::into))
        })
    }
}
