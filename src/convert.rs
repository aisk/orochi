//! Conversions between Rust values and Python objects.
//!
//! [`IntoObject`] turns a Rust value into a Python object, [`FromObject`]
//! turns a Python object into a Rust value. Owned Rust types (`i64`,
//! `String`, `Vec<T>`, `HashMap<K, V>`, ...) are *copies*: changing the Rust
//! value afterwards does not affect Python and vice versa. Use [`Object`] or
//! the typed handles ([`List`](crate::List), [`Dict`](crate::Dict),
//! [`Set`](crate::Set)) to keep a live reference instead.
//!
//! Conversions are strict: a `float` is not accepted where an integer is
//! expected, `bool` is not accepted as a number, and a `str` is not accepted
//! as a sequence. An `int` *is* accepted where a float is expected. Values
//! that do not fit the Rust type raise `OverflowError`.
//!
//! | Rust | Python (into) | Python accepted (from) |
//! |------|---------------|------------------------|
//! | integers | `int` | `int` or any `__index__` object, not `bool` |
//! | `f32`, `f64` | `float` | `float` or integer as above |
//! | `bool` | `bool` | `bool` only |
//! | `String`, `&str`, `Cow<str>`, `char` | `str` | `str` (`char`: length 1) |
//! | `()` | `None` | `None` |
//! | `Option<T>` | `T` or `None` | `T` or `None` |
//! | `Vec<T>`, `&[T]`, `[T; N]` | `list` | any sequence except `str`, `bytes`, `bytearray`, `memoryview` |
//! | `Vec<u8>`, `&[u8]`, `[u8; N]` | `bytes` | `bytes`, `bytearray`, or a sequence of ints |
//! | `HashMap`, `BTreeMap` | `dict` | any `collections.abc.Mapping` |
//! | `HashSet`, `BTreeSet` | `set` | `set` or `frozenset` |
//! | tuples (1 to 12) | `tuple` | `tuple` of the same length |
//! | `PathBuf`, `&Path` | `pathlib.Path` | `str` or `os.PathLike` |
//! | `Box<T>` | as `T` | as `T` |
//!
//! `memoryview` (and other buffer objects) are rejected; buffer support is
//! not part of orochi yet.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, Hash};
use std::path::{Path, PathBuf};

use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyByteArray, PyBytes, PyDict, PyFloat, PyFrozenSet, PyInt, PyList, PyMapping,
    PyMemoryView, PySequence, PySet, PyString, PyTuple,
};

use crate::error::{Error, Result};
use crate::object::Object;
use crate::runtime::attach;

/// Conversion of a Rust value into a Python object.
///
/// Implement this for your own types by building the object with orochi's
/// API, e.g. returning a `dict` or calling a Python class.
///
/// ```
/// use orochi::{IntoObject, Object};
///
/// struct Point { x: f64, y: f64 }
///
/// impl IntoObject for Point {
///     fn into_object(self) -> orochi::Result<Object> {
///         (self.x, self.y).into_object()
///     }
/// }
///
/// let p = Point { x: 1.0, y: 2.0 }.into_object()?;
/// assert_eq!(p.to_string(), "(1.0, 2.0)");
/// # Ok::<(), orochi::Error>(())
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be converted into a Python object",
    label = "does not implement `orochi::IntoObject`"
)]
pub trait IntoObject {
    /// Performs the conversion.
    fn into_object(self) -> Result<Object>;

    /// Converts a `Vec<Self>`: a `list`. Overridden by `u8` to produce
    /// `bytes`.
    #[doc(hidden)]
    fn __vec_into_object(items: Vec<Self>) -> Result<Object>
    where
        Self: Sized,
    {
        attach(|py| {
            let items = items
                .into_iter()
                .map(|item| into_bound(py, item))
                .collect::<Result<Vec<_>>>()?;
            Ok(Object::from_bound(PyList::new(py, items)?.into_any()))
        })
    }
}

/// Conversion of a Python object into a Rust value.
///
/// ```
/// use orochi::{FromObject, Object};
///
/// struct Celsius(f64);
///
/// impl FromObject for Celsius {
///     fn from_object(obj: &Object) -> orochi::Result<Self> {
///         Ok(Celsius(obj.to::<f64>()?))
///     }
/// }
///
/// let c: Celsius = orochi::eval("21.5")?.to()?;
/// assert_eq!(c.0, 21.5);
/// # Ok::<(), orochi::Error>(())
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be converted from a Python object",
    label = "does not implement `orochi::FromObject`",
    note = "implement `orochi::FromObject` for `{Self}`, or take a type that already \
            implements it (numbers, `String`, `Vec<T>`, maps, `Object`, ...)"
)]
pub trait FromObject: Sized {
    /// Performs the conversion. Fails with `TypeError` if the object has the
    /// wrong type, or with the error Python raised (e.g. `OverflowError`).
    fn from_object(obj: &Object) -> Result<Self>;

    /// May a parameter of this type be omitted in a call from Python? It is
    /// then converted from `None`. `true` for `Option<T>`: a trailing
    /// `Option` parameter defaults to `None`.
    const OPTIONAL: bool = false;

    /// Converts to `Vec<Self>`. Overridden by `u8` to accept `bytes`.
    #[doc(hidden)]
    fn __vec_from_object(obj: &Object) -> Result<Vec<Self>> {
        vec_from_sequence(obj, "a sequence")
    }
}

/// Builds the standard "wrong type" error.
pub(crate) fn type_mismatch(expected: &str, obj: &Bound<'_, PyAny>) -> Error {
    let got = obj
        .get_type()
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "<unknown>".to_owned());
    Error::type_error(format!("expected {expected}, got {got}"))
}

/// Converts a borrowed pyo3 object with `T`'s `FromObject` implementation.
pub(crate) fn extract_bound<T: FromObject>(obj: &Bound<'_, PyAny>) -> Result<T> {
    T::from_object(&Object::from_bound(obj.clone()))
}

/// Converts `value` into a pyo3 bound object.
pub(crate) fn into_bound<'py, T: IntoObject>(py: Python<'py>, value: T) -> Result<Bound<'py, PyAny>> {
    Ok(value.into_object()?.into_py().into_bound(py))
}

/// Copies the elements of any iterable into a new `list` (Python's
/// `list(obj)`), so that later conversions cannot observe concurrent changes.
pub(crate) fn snapshot<'py>(obj: &Bound<'py, PyAny>) -> Result<Bound<'py, PyList>> {
    let py = obj.py();
    let list = py.get_type::<PyList>().call1((obj,))?;
    Ok(list.cast_into::<PyList>()?)
}

/// The default `Vec<T>` conversion: any sequence except text, bytes and
/// buffers. The length is not trusted for preallocation (a lazy sequence like
/// `range(10**13)` would otherwise abort the process).
fn vec_from_sequence<T: FromObject>(obj: &Object, expected: &str) -> Result<Vec<T>> {
    attach(|py| {
        let b = obj.bind(py);
        let excluded = b.is_instance_of::<PyString>()
            || b.is_instance_of::<PyBytes>()
            || b.is_instance_of::<PyByteArray>()
            || b.is_instance_of::<PyMemoryView>();
        if excluded || b.cast::<PySequence>().is_err() {
            return Err(type_mismatch(expected, b));
        }
        let mut out = Vec::new();
        for item in b.try_iter()? {
            out.push(extract_bound::<T>(&item?)?);
        }
        Ok(out)
    })
}

/// `int`, or anything whose type implements `__index__`, but never `bool`.
fn is_int_like(obj: &Bound<'_, PyAny>) -> bool {
    if obj.is_instance_of::<PyBool>() {
        return false;
    }
    // Special methods are looked up on the type, as Python does (so that the
    // class `int` itself is not mistaken for an integer).
    obj.is_instance_of::<PyInt>()
        || (!obj.is_instance_of::<PyFloat>()
            && obj.get_type().hasattr("__index__").unwrap_or(false))
}

/// Python's `operator.index(obj)`: the exact `int` value of an integer-like
/// object.
fn as_exact_int<'py>(obj: &Bound<'py, PyAny>) -> Result<Bound<'py, PyAny>> {
    // The common case needs no call at all.
    if obj.is_exact_instance_of::<pyo3::types::PyInt>() {
        return Ok(obj.clone());
    }
    static INDEX: pyo3::sync::PyOnceLock<Py<PyAny>> = pyo3::sync::PyOnceLock::new();
    let index = INDEX.import(obj.py(), "operator", "index")?;
    Ok(index.call1((obj,))?)
}

/// Extracts an integer of type `T`, raising `OverflowError` (never
/// `ValueError`) when the value does not fit.
fn extract_int<T>(obj: &Object, unsigned: bool) -> Result<T>
where
    T: for<'a, 'py> pyo3::FromPyObject<'a, 'py>,
    for<'a, 'py> <T as pyo3::FromPyObject<'a, 'py>>::Error: Into<PyErr>,
{
    attach(|py| {
        let b = obj.bind(py);
        if !is_int_like(b) {
            return Err(type_mismatch("int", b));
        }
        let value = as_exact_int(b)?;
        if unsigned && value.lt(0)? {
            return Err(Error::overflow_error("can't convert negative int to unsigned"));
        }
        value.extract::<T>().map_err(|err| {
            let err: Error = err.into().into();
            if err.is_instance_of("OverflowError") {
                err
            } else {
                Error::overflow_error(format!("int too large to convert: {}", err.message()))
            }
        })
    })
}

/// Extracts a finite-or-not `f64` from a `float` or integer-like object.
fn extract_f64(obj: &Object) -> Result<f64> {
    attach(|py| {
        let b = obj.bind(py);
        if !(b.is_instance_of::<PyFloat>() || is_int_like(b)) {
            return Err(type_mismatch("float", b));
        }
        Ok(b.extract::<f64>()?)
    })
}

// ---------------------------------------------------------------- Object

impl IntoObject for Object {
    fn into_object(self) -> Result<Object> {
        Ok(self)
    }
}

impl IntoObject for &Object {
    fn into_object(self) -> Result<Object> {
        Ok(self.clone())
    }
}

impl FromObject for Object {
    fn from_object(obj: &Object) -> Result<Self> {
        Ok(obj.clone())
    }
}

/// `Result` can be returned from functions; an `Err` is propagated as a
/// Python exception.
impl<T: IntoObject, E: Into<Error>> IntoObject for std::result::Result<T, E> {
    fn into_object(self) -> Result<Object> {
        self.map_err(Into::into)?.into_object()
    }
}

/// Converts the boxed value.
impl<T: IntoObject> IntoObject for Box<T> {
    fn into_object(self) -> Result<Object> {
        (*self).into_object()
    }
}

/// Converts like `T`.
impl<T: FromObject> FromObject for Box<T> {
    fn from_object(obj: &Object) -> Result<Self> {
        T::from_object(obj).map(Box::new)
    }
}

// ---------------------------------------------------------------- numbers

macro_rules! int_impls {
    ($unsigned:expr; $($t:ty),*) => {$(
        impl IntoObject for $t {
            fn into_object(self) -> Result<Object> {
                attach(|py| Ok(Object::from_bound(self.into_bound_py_any(py)?)))
            }
        }

        impl FromObject for $t {
            fn from_object(obj: &Object) -> Result<Self> {
                extract_int::<$t>(obj, $unsigned)
            }
        }
    )*};
}

int_impls!(false; i8, i16, i32, i64, i128, isize);
int_impls!(true; u16, u32, u64, u128, usize);

/// `u8` behaves like the other integers, except that `Vec<u8>` (and `&[u8]`,
/// `[u8; N]`) is Python `bytes`.
impl IntoObject for u8 {
    fn into_object(self) -> Result<Object> {
        attach(|py| Ok(Object::from_bound(self.into_bound_py_any(py)?)))
    }
    fn __vec_into_object(items: Vec<Self>) -> Result<Object> {
        attach(|py| Ok(Object::from_bound(PyBytes::new(py, &items).into_any())))
    }
}

impl FromObject for u8 {
    fn from_object(obj: &Object) -> Result<Self> {
        extract_int::<u8>(obj, true)
    }
    fn __vec_from_object(obj: &Object) -> Result<Vec<Self>> {
        let direct = attach(|py| {
            let b = obj.bind(py);
            if let Ok(bytes) = b.cast::<PyBytes>() {
                Some(bytes.as_bytes().to_vec())
            } else if let Ok(array) = b.cast::<PyByteArray>() {
                Some(array.to_vec())
            } else {
                None
            }
        });
        match direct {
            Some(v) => Ok(v),
            None => vec_from_sequence(obj, "bytes, bytearray or a sequence of int"),
        }
    }
}

impl IntoObject for f64 {
    fn into_object(self) -> Result<Object> {
        attach(|py| Ok(Object::from_bound(self.into_bound_py_any(py)?)))
    }
}

impl FromObject for f64 {
    fn from_object(obj: &Object) -> Result<Self> {
        extract_f64(obj)
    }
}

impl IntoObject for f32 {
    fn into_object(self) -> Result<Object> {
        f64::from(self).into_object()
    }
}

/// A finite value outside `f32`'s range raises `OverflowError`; `inf` and
/// `nan` pass through. Precision is rounded as usual.
impl FromObject for f32 {
    fn from_object(obj: &Object) -> Result<Self> {
        let v = extract_f64(obj)?;
        let narrow = v as f32;
        if v.is_finite() && narrow.is_infinite() {
            return Err(Error::overflow_error("float too large to convert to f32"));
        }
        Ok(narrow)
    }
}

impl IntoObject for bool {
    fn into_object(self) -> Result<Object> {
        attach(|py| Ok(Object::from_bound(self.into_bound_py_any(py)?)))
    }
}

impl FromObject for bool {
    fn from_object(obj: &Object) -> Result<Self> {
        attach(|py| {
            let b = obj.bind(py);
            match b.cast::<PyBool>() {
                Ok(v) => Ok(v.is_true()),
                Err(_) => Err(type_mismatch("bool", b)),
            }
        })
    }
}

// ---------------------------------------------------------------- strings

impl IntoObject for &str {
    fn into_object(self) -> Result<Object> {
        attach(|py| Ok(Object::from_bound(PyString::new(py, self).into_any())))
    }
}

impl IntoObject for String {
    fn into_object(self) -> Result<Object> {
        self.as_str().into_object()
    }
}

impl IntoObject for &String {
    fn into_object(self) -> Result<Object> {
        self.as_str().into_object()
    }
}

impl IntoObject for Cow<'_, str> {
    fn into_object(self) -> Result<Object> {
        self.as_ref().into_object()
    }
}

impl FromObject for String {
    fn from_object(obj: &Object) -> Result<Self> {
        attach(|py| {
            let b = obj.bind(py);
            match b.cast::<PyString>() {
                Ok(s) => Ok(s.to_cow()?.into_owned()),
                Err(_) => Err(type_mismatch("str", b)),
            }
        })
    }
}

/// Always produces `Cow::Owned`.
impl FromObject for Cow<'static, str> {
    fn from_object(obj: &Object) -> Result<Self> {
        String::from_object(obj).map(Cow::Owned)
    }
}

impl IntoObject for char {
    fn into_object(self) -> Result<Object> {
        self.encode_utf8(&mut [0; 4]).into_object()
    }
}

/// Accepts a `str` of length 1.
impl FromObject for char {
    fn from_object(obj: &Object) -> Result<Self> {
        let s = String::from_object(obj)?;
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(Error::value_error(format!(
                "expected a string of length 1, got length {}",
                s.chars().count()
            ))),
        }
    }
}

// ---------------------------------------------------------------- paths

/// Becomes a `pathlib.Path`.
impl IntoObject for PathBuf {
    fn into_object(self) -> Result<Object> {
        self.as_path().into_object()
    }
}

/// Becomes a `pathlib.Path`.
impl IntoObject for &PathBuf {
    fn into_object(self) -> Result<Object> {
        self.as_path().into_object()
    }
}

/// Becomes a `pathlib.Path`.
impl IntoObject for &Path {
    fn into_object(self) -> Result<Object> {
        attach(|py| Ok(Object::from_bound(self.into_bound_py_any(py)?)))
    }
}

/// Accepts a `str` or an `os.PathLike` object (such as `pathlib.Path`).
impl FromObject for PathBuf {
    fn from_object(obj: &Object) -> Result<Self> {
        attach(|py| {
            let b = obj.bind(py);
            let path_like =
                b.is_instance_of::<PyString>() || b.get_type().hasattr("__fspath__").unwrap_or(false);
            if !path_like {
                return Err(type_mismatch("str or os.PathLike", b));
            }
            Ok(b.extract::<PathBuf>()?)
        })
    }
}

// ---------------------------------------------------------------- None / Option

/// `()` is Python's `None`.
impl IntoObject for () {
    fn into_object(self) -> Result<Object> {
        Ok(Object::none())
    }
}

/// Accepts only `None`.
impl FromObject for () {
    fn from_object(obj: &Object) -> Result<Self> {
        attach(|py| {
            let b = obj.bind(py);
            if b.is_none() { Ok(()) } else { Err(type_mismatch("None", b)) }
        })
    }
}

/// `None` maps to `None`, `Some(v)` to `v`.
impl<T: IntoObject> IntoObject for Option<T> {
    fn into_object(self) -> Result<Object> {
        match self {
            Some(v) => v.into_object(),
            None => Ok(Object::none()),
        }
    }
}

impl<T: FromObject> FromObject for Option<T> {
    const OPTIONAL: bool = true;
    fn from_object(obj: &Object) -> Result<Self> {
        if obj.is_none() { Ok(None) } else { T::from_object(obj).map(Some) }
    }
}

// ---------------------------------------------------------------- sequences

/// Becomes a new `list` (`bytes` for `Vec<u8>`).
impl<T: IntoObject> IntoObject for Vec<T> {
    fn into_object(self) -> Result<Object> {
        T::__vec_into_object(self)
    }
}

/// Becomes a new `list` (`bytes` for `&[u8]`).
impl<T: IntoObject + Clone> IntoObject for &[T] {
    fn into_object(self) -> Result<Object> {
        T::__vec_into_object(self.to_vec())
    }
}

/// Becomes a new `list` (`bytes` for `&Vec<u8>`).
impl<T: IntoObject + Clone> IntoObject for &Vec<T> {
    fn into_object(self) -> Result<Object> {
        self.as_slice().into_object()
    }
}

/// Becomes a new `list` (`bytes` for `[u8; N]`).
impl<T: IntoObject, const N: usize> IntoObject for [T; N] {
    fn into_object(self) -> Result<Object> {
        T::__vec_into_object(Vec::from(self))
    }
}

/// Accepts any sequence (`list`, `tuple`, `range`, ...) except `str`,
/// `bytes`, `bytearray` and `memoryview`; elements are converted strictly.
/// `Vec<u8>` additionally accepts `bytes` and `bytearray`.
impl<T: FromObject> FromObject for Vec<T> {
    fn from_object(obj: &Object) -> Result<Self> {
        T::__vec_from_object(obj)
    }
}

/// Accepts what `Vec<T>` accepts, with exactly `N` elements.
impl<T: FromObject, const N: usize> FromObject for [T; N] {
    fn from_object(obj: &Object) -> Result<Self> {
        let items = T::__vec_from_object(obj)?;
        let len = items.len();
        items.try_into().map_err(|_| {
            Error::type_error(format!("expected a sequence of length {N}, got length {len}"))
        })
    }
}

// ---------------------------------------------------------------- mappings / sets

fn dict_from_pairs<K: IntoObject, V: IntoObject>(
    pairs: impl IntoIterator<Item = (K, V)>,
) -> Result<Object> {
    attach(|py| {
        let dict = PyDict::new(py);
        for (k, v) in pairs {
            dict.set_item(into_bound(py, k)?, into_bound(py, v)?)?;
        }
        Ok(Object::from_bound(dict.into_any()))
    })
}

/// Converts a `dict` or any other `collections.abc.Mapping` into key/value
/// pairs from a snapshot of its items.
fn pairs_from_mapping<K: FromObject, V: FromObject>(
    obj: &Object,
    mut sink: impl FnMut(K, V),
) -> Result<()> {
    attach(|py| {
        let b = obj.bind(py);
        // Both are new lists, so converting the entries (which may run
        // arbitrary Python code) cannot invalidate the iteration. A `dict`
        // (or subclass) is read from its own storage, without calling an
        // overridden `items()`; another mapping through its `items()`.
        let items = if let Ok(dict) = b.cast::<PyDict>() {
            dict.items()
        } else if let Ok(mapping) = b.cast::<PyMapping>() {
            mapping.items()?
        } else {
            return Err(type_mismatch("a mapping", b));
        };
        for item in items.iter() {
            let pair = match item.cast::<PyTuple>() {
                Ok(pair) if pair.len() == 2 => pair,
                _ => return Err(type_mismatch("a (key, value) tuple", &item)),
            };
            sink(extract_bound(&pair.get_item(0)?)?, extract_bound(&pair.get_item(1)?)?);
        }
        Ok(())
    })
}

fn set_from_items<T: IntoObject>(items: impl IntoIterator<Item = T>) -> Result<Object> {
    attach(|py| {
        let set = PySet::empty(py)?;
        for item in items {
            set.add(into_bound(py, item)?)?;
        }
        Ok(Object::from_bound(set.into_any()))
    })
}

/// Converts a `set`/`frozenset` from a snapshot of its elements.
fn items_from_set<T: FromObject>(obj: &Object, mut sink: impl FnMut(T)) -> Result<()> {
    attach(|py| {
        let b = obj.bind(py);
        if !(b.is_instance_of::<PySet>() || b.is_instance_of::<PyFrozenSet>()) {
            return Err(type_mismatch("set", b));
        }
        for item in snapshot(b)?.iter() {
            sink(extract_bound(&item)?);
        }
        Ok(())
    })
}

/// Becomes a new `dict`.
impl<K: IntoObject, V: IntoObject, S> IntoObject for HashMap<K, V, S> {
    fn into_object(self) -> Result<Object> {
        dict_from_pairs(self)
    }
}

/// Accepts any `collections.abc.Mapping` (`dict`, `types.MappingProxyType`,
/// ...).
impl<K, V, S> FromObject for HashMap<K, V, S>
where
    K: FromObject + Eq + Hash,
    V: FromObject,
    S: BuildHasher + Default,
{
    fn from_object(obj: &Object) -> Result<Self> {
        let mut out = HashMap::with_hasher(S::default());
        pairs_from_mapping(obj, |k, v| {
            out.insert(k, v);
        })?;
        Ok(out)
    }
}

/// Becomes a new `dict` (in key order).
impl<K: IntoObject, V: IntoObject> IntoObject for BTreeMap<K, V> {
    fn into_object(self) -> Result<Object> {
        dict_from_pairs(self)
    }
}

/// Accepts any `collections.abc.Mapping` (`dict`, `types.MappingProxyType`,
/// ...).
impl<K: FromObject + Ord, V: FromObject> FromObject for BTreeMap<K, V> {
    fn from_object(obj: &Object) -> Result<Self> {
        let mut out = BTreeMap::new();
        pairs_from_mapping(obj, |k, v| {
            out.insert(k, v);
        })?;
        Ok(out)
    }
}

/// Becomes a new `set`.
impl<T: IntoObject, S> IntoObject for HashSet<T, S> {
    fn into_object(self) -> Result<Object> {
        set_from_items(self)
    }
}

/// Accepts a `set` or `frozenset` (or subclasses).
impl<T, S> FromObject for HashSet<T, S>
where
    T: FromObject + Eq + Hash,
    S: BuildHasher + Default,
{
    fn from_object(obj: &Object) -> Result<Self> {
        let mut out = HashSet::with_hasher(S::default());
        items_from_set(obj, |v| {
            out.insert(v);
        })?;
        Ok(out)
    }
}

/// Becomes a new `set`.
impl<T: IntoObject> IntoObject for BTreeSet<T> {
    fn into_object(self) -> Result<Object> {
        set_from_items(self)
    }
}

/// Accepts a `set` or `frozenset` (or subclasses).
impl<T: FromObject + Ord> FromObject for BTreeSet<T> {
    fn from_object(obj: &Object) -> Result<Self> {
        let mut out = BTreeSet::new();
        items_from_set(obj, |v| {
            out.insert(v);
        })?;
        Ok(out)
    }
}

// ---------------------------------------------------------------- tuples

macro_rules! tuple_impls {
    ($len:expr; $($name:ident $idx:tt),+) => {
        /// Becomes a new `tuple`.
        impl<$($name: IntoObject),+> IntoObject for ($($name,)+) {
            fn into_object(self) -> Result<Object> {
                attach(|py| {
                    let items = [$(into_bound(py, self.$idx)?),+];
                    Ok(Object::from_bound(PyTuple::new(py, items)?.into_any()))
                })
            }
        }

        /// Accepts a `tuple` (or subclass) of exactly this length.
        impl<$($name: FromObject),+> FromObject for ($($name,)+) {
            fn from_object(obj: &Object) -> Result<Self> {
                attach(|py| {
                    let b = obj.bind(py);
                    let t = b.cast::<PyTuple>().map_err(|_| type_mismatch("tuple", b))?;
                    if t.len() != $len {
                        return Err(Error::type_error(format!(
                            "expected a tuple of length {}, got length {}",
                            $len,
                            t.len()
                        )));
                    }
                    Ok(($(extract_bound::<$name>(&t.get_item($idx)?)?,)+))
                })
            }
        }
    };
}

tuple_impls!(1; A 0);
tuple_impls!(2; A 0, B 1);
tuple_impls!(3; A 0, B 1, C 2);
tuple_impls!(4; A 0, B 1, C 2, D 3);
tuple_impls!(5; A 0, B 1, C 2, D 3, E 4);
tuple_impls!(6; A 0, B 1, C 2, D 3, E 4, F 5);
tuple_impls!(7; A 0, B 1, C 2, D 3, E 4, F 5, G 6);
tuple_impls!(8; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
tuple_impls!(9; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8);
tuple_impls!(10; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9);
tuple_impls!(11; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10);
tuple_impls!(12; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11);
