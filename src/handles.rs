//! Typed reference handles to Python containers: [`List`], [`Dict`], [`Set`].
//!
//! Unlike `Vec<T>` / `HashMap<K, V>` / `HashSet<T>`, which are copies, a
//! handle refers to the Python object itself: changes made through a handle
//! are visible to every other holder of the same object, in Rust or Python.
//! The container type is checked when the handle is created; elements are
//! converted to and from `T` on each access.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet};

use crate::convert::{FromObject, IntoObject, extract_bound, into_bound, snapshot, type_mismatch};
use crate::error::Result;
use crate::object::Object;
use crate::runtime::attach;

macro_rules! handle_common {
    ($name:ident <$($p:ident),+>, $pytype:ty, $pyname:literal, in: $in_hint:expr, out: $out_hint:expr, stored_in: $stored_in_hint:expr) => {
        impl<$($p),+> $name<$($p),+> {
            /// The handle as an untyped [`Object`] (same Python object).
            pub fn as_object(&self) -> &Object {
                &self.obj
            }

            /// Converts the handle into an untyped [`Object`] (same Python
            /// object).
            pub fn into_inner(self) -> Object {
                self.obj
            }

            /// Identity check: do both handles refer to the same Python object?
            pub fn is(&self, other: &Self) -> bool {
                self.obj.is(&other.obj)
            }

            fn wrap(obj: Object) -> Self {
                $name { obj, _types: PhantomData }
            }
        }

        impl<$($p),+> Clone for $name<$($p),+> {
            /// Clones the handle; both handles refer to the same Python object.
            fn clone(&self) -> Self {
                Self::wrap(self.obj.clone())
            }
        }

        impl<$($p),+> fmt::Display for $name<$($p),+> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.obj, f)
            }
        }

        impl<$($p),+> fmt::Debug for $name<$($p),+> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&self.obj, f)
            }
        }

        impl<$($p),+> From<$name<$($p),+>> for Object {
            fn from(handle: $name<$($p),+>) -> Object {
                handle.obj
            }
        }

        /// Passes the referenced object itself (no copy).
        impl<$($p: IntoObject),+> IntoObject for $name<$($p),+> {
            fn into_object(self) -> Result<Object> {
                Ok(self.obj)
            }
            fn output_hint() -> String {
                $out_hint
            }
            fn __input_hint() -> String {
                $stored_in_hint
            }
        }

        /// Passes the referenced object itself (no copy).
        impl<$($p: IntoObject),+> IntoObject for &$name<$($p),+> {
            fn into_object(self) -> Result<Object> {
                Ok(self.obj.clone())
            }
            fn output_hint() -> String {
                $out_hint
            }
        }

        #[doc = concat!("Accepts a `", $pyname, "` (or subclass) without copying it. Elements are not checked until accessed.")]
        impl<$($p: FromObject),+> FromObject for $name<$($p),+> {
            const REFERENCES_PYTHON: bool = true;
            const __HANDLE_OF: Option<&'static str> = Some($pyname);
            fn from_object(obj: &Object) -> Result<Self> {
                attach(|py| {
                    let b = obj.bind(py);
                    if b.is_instance_of::<$pytype>() {
                        Ok(Self::wrap(obj.clone()))
                    } else {
                        Err(type_mismatch($pyname, b))
                    }
                })
            }
            fn input_hint() -> String {
                $in_hint
            }
        }
    };
}

// ---------------------------------------------------------------- List

/// A live reference to a Python `list` whose elements convert to `T`.
///
/// ```
/// use orochi::List;
///
/// let scope = orochi::exec("xs = [1, 2]")?;
/// let xs: List<i64> = scope.get("xs")?.to()?;
/// xs.push(3)?;
/// // The change is visible from Python:
/// assert_eq!(scope.eval("sum(xs)")?.to::<i64>()?, 6);
/// assert_eq!(xs.to_vec()?, [1, 2, 3]);
/// # Ok::<(), orochi::Error>(())
/// ```
pub struct List<T> {
    obj: Object,
    _types: PhantomData<fn() -> T>,
}

handle_common!(List<T>, PyList, "list",
    in: format!("list[{}]", T::input_hint()),
    out: format!("list[{}]", T::output_hint()),
    stored_in: format!("list[{}]", T::__input_hint()));

impl<T> List<T> {
    /// Creates a new empty Python `list`.
    pub fn new() -> Self {
        attach(|py| Self::wrap(Object::from_bound(PyList::empty(py).into_any())))
    }

    /// Python's `len(list)`.
    pub fn len(&self) -> usize {
        attach(|py| self.bound(py).len())
    }

    /// Returns `true` if the list is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes all elements.
    pub fn clear(&self) -> Result<()> {
        self.obj.method("clear", ()).map(drop)
    }

    fn bound<'py>(&self, py: Python<'py>) -> Bound<'py, PyList> {
        // The type was checked on construction.
        self.obj.bind(py).clone().cast_into::<PyList>().expect("List handle holds a list")
    }
}

impl<T> Default for List<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: FromObject> List<T> {
    /// Returns element `index`, or `IndexError` if out of range.
    pub fn get(&self, index: usize) -> Result<T> {
        attach(|py| extract_bound(&self.bound(py).get_item(index)?))
    }

    /// Removes and returns the last element, or `None` if the list is empty.
    pub fn pop(&self) -> Result<Option<T>> {
        match self.obj.method("pop", ()) {
            Ok(item) => item.to::<T>().map(Some),
            Err(err) if err.is_instance_of("IndexError") => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Iterates over the elements, converting each one.
    ///
    /// Iteration goes through Python's list iterator, so it observes
    /// concurrent modifications the way a Python `for` loop would.
    pub fn iter(&self) -> Result<impl Iterator<Item = Result<T>> + use<T>> {
        Ok(self.obj.iter()?.map(|item| item?.to::<T>()))
    }

    /// Copies the current elements into a `Vec`.
    pub fn to_vec(&self) -> Result<Vec<T>> {
        self.iter()?.collect()
    }
}

impl<T: IntoObject> List<T> {
    /// Appends an element.
    pub fn push(&self, value: impl Into<T>) -> Result<()> {
        attach(|py| Ok(self.bound(py).append(into_bound(py, value.into())?)?))
    }

    /// Replaces element `index`, or `IndexError` if out of range.
    pub fn set(&self, index: usize, value: impl Into<T>) -> Result<()> {
        attach(|py| Ok(self.bound(py).set_item(index, into_bound(py, value.into())?)?))
    }

    /// Inserts an element before `index` (like Python's `list.insert`).
    pub fn insert(&self, index: usize, value: impl Into<T>) -> Result<()> {
        attach(|py| Ok(self.bound(py).insert(index, into_bound(py, value.into())?)?))
    }

    /// Creates a new Python `list` holding the given elements.
    pub fn from_vec(values: Vec<T>) -> Result<Self> {
        // Built element by element: `Vec<u8>::into_object` would give `bytes`.
        let list = Self::new();
        for value in values {
            list.push(value)?;
        }
        Ok(list)
    }
}

// ---------------------------------------------------------------- Dict

/// A live reference to a Python `dict` whose keys convert to `K` and values
/// to `V`.
///
/// ```
/// use orochi::Dict;
///
/// let d: Dict<String, i64> = Dict::new();
/// d.insert("a", 1)?;
/// let alias = d.clone(); // same Python dict
/// alias.insert("b", 2)?;
/// assert_eq!(d.len(), 2);
/// assert_eq!(d.get("b")?, Some(2));
/// assert_eq!(d.get("zzz")?, None);
/// # Ok::<(), orochi::Error>(())
/// ```
pub struct Dict<K, V> {
    obj: Object,
    _types: PhantomData<fn() -> (K, V)>,
}

handle_common!(Dict<K, V>, PyDict, "dict",
    in: format!("dict[{}, {}]", K::input_hint(), V::input_hint()),
    out: format!("dict[{}, {}]", K::output_hint(), V::output_hint()),
    stored_in: format!("dict[{}, {}]", K::__input_hint(), V::__input_hint()));

impl<K, V> Dict<K, V> {
    /// Creates a new empty Python `dict`.
    pub fn new() -> Self {
        attach(|py| Self::wrap(Object::from_bound(PyDict::new(py).into_any())))
    }

    /// Python's `len(dict)`.
    pub fn len(&self) -> usize {
        attach(|py| self.bound(py).len())
    }

    /// Returns `true` if the dict is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes all entries.
    pub fn clear(&self) -> Result<()> {
        attach(|py| self.bound(py).clear());
        Ok(())
    }

    fn bound<'py>(&self, py: Python<'py>) -> Bound<'py, PyDict> {
        // The type was checked on construction.
        self.obj.bind(py).clone().cast_into::<PyDict>().expect("Dict handle holds a dict")
    }
}

impl<K, V> Default for Dict<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: IntoObject, V> Dict<K, V> {
    /// Python's `key in dict`.
    pub fn contains(&self, key: impl Into<K>) -> Result<bool> {
        attach(|py| Ok(self.bound(py).contains(into_bound(py, key.into())?)?))
    }
}

impl<K: IntoObject, V: FromObject> Dict<K, V> {
    /// Returns the value for `key`, or `None` if the key is absent.
    pub fn get(&self, key: impl Into<K>) -> Result<Option<V>> {
        attach(|py| match self.bound(py).get_item(into_bound(py, key.into())?)? {
            Some(v) => extract_bound(&v).map(Some),
            None => Ok(None),
        })
    }

    /// Removes `key` and returns its value, or `None` if the key was absent.
    pub fn remove(&self, key: impl Into<K>) -> Result<Option<V>> {
        attach(|py| {
            // `dict.pop(key, sentinel)` is a single atomic operation.
            let sentinel = py.import("builtins")?.getattr("object")?.call0()?;
            let key = into_bound(py, key.into())?;
            let value = self.bound(py).call_method1("pop", (key, &sentinel))?;
            if value.is(&sentinel) { Ok(None) } else { extract_bound(&value).map(Some) }
        })
    }
}

impl<K: IntoObject, V: IntoObject> Dict<K, V> {
    /// Python's `dict[key] = value`.
    pub fn insert(&self, key: impl Into<K>, value: impl Into<V>) -> Result<()> {
        attach(|py| {
            Ok(self
                .bound(py)
                .set_item(into_bound(py, key.into())?, into_bound(py, value.into())?)?)
        })
    }
}

impl<K: FromObject, V> Dict<K, V> {
    /// Snapshot of the keys, in insertion order.
    pub fn keys(&self) -> Result<Vec<K>> {
        attach(|py| self.bound(py).keys().iter().map(|k| extract_bound(&k)).collect())
    }
}

impl<K, V: FromObject> Dict<K, V> {
    /// Snapshot of the values, in insertion order.
    pub fn values(&self) -> Result<Vec<V>> {
        attach(|py| self.bound(py).values().iter().map(|v| extract_bound(&v)).collect())
    }
}

impl<K: FromObject, V: FromObject> Dict<K, V> {
    /// Snapshot of the entries, in insertion order.
    pub fn items(&self) -> Result<Vec<(K, V)>> {
        attach(|py| {
            // `items()` is a snapshot list; converting entries may run Python
            // code that changes the dict.
            self.bound(py)
                .items()
                .iter()
                .map(|item| {
                    let (k, v) = item.extract::<(Bound<'_, PyAny>, Bound<'_, PyAny>)>()?;
                    Ok((extract_bound(&k)?, extract_bound(&v)?))
                })
                .collect()
        })
    }

    /// Copies the current entries into a `HashMap`.
    pub fn to_hashmap(&self) -> Result<HashMap<K, V>>
    where
        K: Eq + Hash,
    {
        self.obj.to()
    }
}

// ---------------------------------------------------------------- Set

/// A live reference to a Python `set` whose elements convert to `T`.
///
/// ```
/// use orochi::Set;
///
/// let s: Set<String> = Set::new();
/// s.add("x")?;
/// s.add("x")?;
/// assert_eq!(s.len(), 1);
/// assert!(s.contains("x")?);
/// assert!(s.remove("x")?);
/// assert!(!s.remove("x")?);
/// # Ok::<(), orochi::Error>(())
/// ```
pub struct Set<T> {
    obj: Object,
    _types: PhantomData<fn() -> T>,
}

handle_common!(Set<T>, PySet, "set",
    in: format!("set[{}]", T::input_hint()),
    out: format!("set[{}]", T::output_hint()),
    stored_in: format!("set[{}]", T::__input_hint()));

impl<T> Set<T> {
    /// Creates a new empty Python `set`.
    pub fn new() -> Self {
        attach(|py| {
            let set = PySet::empty(py).expect("failed to create a Python set");
            Self::wrap(Object::from_bound(set.into_any()))
        })
    }

    /// Python's `len(set)`.
    pub fn len(&self) -> usize {
        attach(|py| self.bound(py).len())
    }

    /// Returns `true` if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes all elements.
    pub fn clear(&self) -> Result<()> {
        attach(|py| self.bound(py).clear());
        Ok(())
    }

    fn bound<'py>(&self, py: Python<'py>) -> Bound<'py, PySet> {
        // The type was checked on construction.
        self.obj.bind(py).clone().cast_into::<PySet>().expect("Set handle holds a set")
    }
}

impl<T> Default for Set<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: IntoObject> Set<T> {
    /// Adds an element (no-op if already present).
    pub fn add(&self, value: impl Into<T>) -> Result<()> {
        attach(|py| Ok(self.bound(py).add(into_bound(py, value.into())?)?))
    }

    /// Removes an element; returns whether it was present.
    pub fn remove(&self, value: impl Into<T>) -> Result<bool> {
        attach(|py| Ok(self.bound(py).discard(into_bound(py, value.into())?)?))
    }

    /// Python's `value in set`.
    pub fn contains(&self, value: impl Into<T>) -> Result<bool> {
        attach(|py| Ok(self.bound(py).contains(into_bound(py, value.into())?)?))
    }
}

impl<T: FromObject> Set<T> {
    /// Snapshot of the elements as a `Vec` (in Python's iteration order).
    pub fn to_vec(&self) -> Result<Vec<T>> {
        attach(|py| snapshot(self.bound(py).as_any())?.iter().map(|v| extract_bound(&v)).collect())
    }

    /// Copies the current elements into a `HashSet`.
    pub fn to_hashset(&self) -> Result<HashSet<T>>
    where
        T: Eq + Hash,
    {
        self.obj.to()
    }
}
