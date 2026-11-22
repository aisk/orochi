//! Classes: the [`Class`] trait and the [`Ref`] handle.

use std::fmt;
use std::ops::{Deref, DerefMut};

use pyo3::prelude::*;
use pyo3::pyclass::boolean_struct::False;
use pyo3::{PyClassGuard, PyClassGuardMut, PyClassInitializer};

use crate::convert::{FromObject, IntoObject, type_mismatch};
use crate::error::Result;
use crate::object::Object;
use crate::runtime::attach;

/// A Rust type exposed to Python as a class.
///
/// Implemented automatically for every type with pyo3's `#[pyclass]`
/// (`#[orochi::module]` adds it to each `pub struct`) that is not `frozen`
/// and is `Send + Sync`. The class gets its constructor, methods and
/// properties from [`ClassBuilder`](crate::ClassBuilder); Python runs the
/// constructor through a `#[new]` that calls [`construct`](crate::construct).
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a Python class",
    label = "not a `pub struct` of an `#[orochi::module]` (or a `#[pyclass]` type)",
    note = "`&mut` parameters work only for classes: to modify the caller's list, dict or set, \
            take `List<T>`, `Dict<K, V>` or `Set<T>`; other Python values are immutable, so \
            return the new value"
)]
pub trait Class:
    pyo3::PyClass<Frozen = False> + Into<PyClassInitializer<Self>> + Send + Sync + 'static
{
}

impl<T> Class for T where
    T: pyo3::PyClass<Frozen = False> + Into<PyClassInitializer<T>> + Send + Sync + 'static
{
}

/// A reference to an instance of a [`Class`] living in Python.
///
/// Taking a class by value copies it (`p: Point` is a new, independent
/// `Point`, and needs `Clone`). A `Ref<Point>` instead refers to one Python
/// object: every `Ref` to it, and every Python variable bound to it, sees
/// the same data. Use it for fields, parameters and return values that must
/// keep identity:
///
/// ```
/// #[orochi::module]
/// mod shapes {
///     use orochi::prelude::*;
///
///     pub struct Point { pub x: f64, pub y: f64 }
///
///     pub struct Circle { pub center: Ref<Point>, pub radius: f64 }
///
///     impl Circle {
///         pub fn new(center: Ref<Point>, radius: f64) -> Self {
///             Circle { center, radius }
///         }
///     }
/// }
///
/// use orochi::Ref;
/// use shapes::{Circle, Point};
///
/// let p = Ref::new(Point { x: 1.0, y: 2.0 })?;
/// let c = Ref::new(Circle::new(p.clone(), 3.0))?;
/// p.borrow_mut()?.x = 5.0;
/// assert_eq!(c.borrow()?.center.borrow()?.x, 5.0); // same object
/// assert!(c.borrow()?.center.is(&p));
/// # Ok::<(), orochi::Error>(())
/// ```
///
/// # Borrowing
///
/// [`borrow`](Ref::borrow) and [`borrow_mut`](Ref::borrow_mut) follow
/// `RefCell` rules at runtime: any number of shared borrows, or one mutable
/// borrow. A conflicting borrow fails with a `RuntimeError` instead of
/// blocking. The same check guards calls from Python, so calling a method
/// while a [`RefMutGuard`] is alive raises in Python as well.
///
/// Python's garbage collector does not see references held in Rust fields,
/// so a cycle of `Ref`s (an object referring to itself) is never freed.
pub struct Ref<T: Class>(Py<T>);

impl<T: Class> Ref<T> {
    /// Moves `value` into a new Python object.
    pub fn new(value: T) -> Result<Ref<T>> {
        attach(|py| Ok(Ref(Py::new(py, value)?)))
    }

    /// Borrows the instance immutably. Fails with `RuntimeError` if it is
    /// currently borrowed mutably.
    pub fn borrow(&self) -> Result<RefGuard<'_, T>> {
        attach(|py| {
            let guard = PyClassGuard::<T>::extract(self.0.as_any().bind_borrowed(py))
                .map_err(PyErr::from)?;
            Ok(RefGuard(guard))
        })
    }

    /// Borrows the instance mutably. Fails with `RuntimeError` if it is
    /// currently borrowed (mutably or not).
    pub fn borrow_mut(&self) -> Result<RefMutGuard<'_, T>> {
        attach(|py| {
            let guard = PyClassGuardMut::<T>::extract(self.0.as_any().bind_borrowed(py))
                .map_err(PyErr::from)?;
            Ok(RefMutGuard(guard))
        })
    }

    /// Identity check: do both handles refer to the same Python object?
    pub fn is(&self, other: &Ref<T>) -> bool {
        self.0.as_ptr() == other.0.as_ptr()
    }

    /// The instance as an untyped [`Object`] (same Python object).
    pub fn to_object(&self) -> Object {
        self.clone().into()
    }
}

impl<T: Class> Clone for Ref<T> {
    /// Clones the handle; both handles refer to the same Python object.
    fn clone(&self) -> Self {
        attach(|py| Ref(self.0.clone_ref(py)))
    }
}

impl<T: Class> From<Ref<T>> for Object {
    fn from(r: Ref<T>) -> Object {
        Object::from_py(r.0.into_any())
    }
}

/// Python's `str()` of the instance.
impl<T: Class> fmt::Display for Ref<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_object(), f)
    }
}

/// Python's `repr()` of the instance.
impl<T: Class> fmt::Debug for Ref<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_object(), f)
    }
}

/// Passes the referenced object itself (no copy).
impl<T: Class> IntoObject for Ref<T> {
    fn into_object(self) -> Result<Object> {
        Ok(self.into())
    }
}

/// Passes the referenced object itself (no copy).
impl<T: Class> IntoObject for &Ref<T> {
    fn into_object(self) -> Result<Object> {
        Ok(self.clone().into())
    }
}

/// Accepts an instance of `T` without copying it.
impl<T: Class> FromObject for Ref<T> {
    fn from_object(obj: &Object) -> Result<Self> {
        attach(|py| {
            let b = obj.bind(py);
            match b.cast::<T>() {
                Ok(instance) => Ok(Ref(instance.clone().unbind())),
                Err(_) => Err(type_mismatch(T::NAME, b)),
            }
        })
    }
}

/// A shared borrow of a class instance, created by [`Ref::borrow`].
pub struct RefGuard<'a, T: Class>(PyClassGuard<'a, T>);

impl<T: Class> Deref for RefGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Class + fmt::Debug> fmt::Debug for RefGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// A mutable borrow of a class instance, created by [`Ref::borrow_mut`].
pub struct RefMutGuard<'a, T: Class>(PyClassGuardMut<'a, T>);

impl<T: Class> Deref for RefMutGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Class> DerefMut for RefMutGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: Class + fmt::Debug> fmt::Debug for RefMutGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
