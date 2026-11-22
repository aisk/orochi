//! The runtime builder that `#[orochi::module]` calls: [`ModuleBuilder`],
//! [`ClassBuilder`] and [`construct`].

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher, Hash};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyModule, PyTuple, PyType};

use crate::class::{Class, Ref};
use crate::convert::{FromObject, IntoObject};
use crate::error::Result;
use crate::function::{Signature, new_function, non_empty};
use crate::handler::{Handler, IntoInstance, Ret, convert_named, instance};
use crate::handles::{Dict, List, Set};
use crate::object::Object;
use crate::runtime::attach;

/// Builds a Python module: adds functions, constants, classes and
/// submodules to it.
///
/// `#[orochi::module]` generates an `init` function that makes these calls
/// (see [`module`](crate::module)); it can also be written by hand. The
/// extension's entry point is a `#[pymodule]` that runs it with
/// [`init_module`]; [`load_module`] builds the same module in-process.
///
/// ```
/// use orochi::pyo3::prelude::*;
/// use orochi::pyo3::types::{PyDict, PyModule, PyTuple};
/// use orochi::{ModuleBuilder, Result};
///
/// /// A counter.
/// // `module` gives the class its full name, `handmade.Counter`.
/// #[pyclass(crate = "orochi::pyo3", module = "handmade")]
/// #[derive(Debug, PartialEq)]
/// struct Counter {
///     count: i64,
/// }
///
/// // One `#[new]` that runs the constructor registered with the builder.
/// #[pymethods(crate = "orochi::pyo3")]
/// impl Counter {
///     #[new]
///     #[pyo3(signature = (*args, **kwargs), text_signature = None)]
///     fn py_new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
///         orochi::construct(args, kwargs)
///     }
/// }
///
/// fn init(m: &ModuleBuilder<'_>) -> Result<()> {
///     m.doc("Built by hand.")?
///         .function("add", &["a", "b"], "Add two integers.", |a: i64, b: Option<i64>| {
///             a + b.unwrap_or(1)
///         })?
///         .constant("VERSION", "1.0")?
///         .submodule("text", "", |t| {
///             t.function("shout", &["s"], "", |s: &str| s.to_uppercase())?;
///             Ok(())
///         })?;
///     // Each `ClassBuilder` call returns the builder again.
///     m.class::<Counter>()?
///         .constructor(&["start"], "Starts at `start`, or 0.", |start: Option<i64>| Counter {
///             count: start.unwrap_or(0),
///         })?
///         .method("incr", &[], "Count one more.", |c: &mut Counter| c.count += 1)?
///         .field("count", "", |c: &Counter| &c.count, |c: &mut Counter| &mut c.count)?
///         .repr_debug()?
///         .eq()?;
///     Ok(())
/// }
///
/// // The extension's entry point, `PyInit_handmade`.
/// #[pymodule(crate = "orochi::pyo3", name = "handmade")]
/// fn handmade_module(module: &Bound<'_, PyModule>) -> PyResult<()> {
///     orochi::init_module(module, init)
/// }
///
/// // In a test: the same module, without loading the compiled library.
/// orochi::load_module("handmade", init)?;
/// orochi::exec(r#"
///     import handmade, handmade.text, inspect
///     assert handmade.add(1) == 2 and handmade.add(a=1, b=5) == 6
///     assert handmade.text.shout("hi") == "HI" and handmade.VERSION == "1.0"
///     c = handmade.Counter()
///     c.incr(); c.incr()
///     assert c.count == 2 and c == handmade.Counter(2)
///     assert repr(c) == "Counter { count: 2 }"
///     assert str(inspect.signature(handmade.Counter)) == "(start=None)"
///     assert handmade.Counter.__doc__ == "A counter.\n\nStarts at `start`, or 0."
/// "#)?;
/// # Ok::<(), orochi::Error>(())
/// ```
pub struct ModuleBuilder<'py> {
    module: Bound<'py, PyModule>,
    name: String,
}

/// Runs `init` on a module created by pyo3's `#[pymodule]`: the body of the
/// extension's entry point.
pub fn init_module(
    module: &Bound<'_, PyModule>,
    init: impl FnOnce(&ModuleBuilder<'_>) -> Result<()>,
) -> PyResult<()> {
    let builder = ModuleBuilder { module: module.clone(), name: module.name()?.to_string() };
    Ok(init(&builder)?)
}

/// Builds module `name` in-process with `init` and registers it in
/// `sys.modules` (so that `import name` finds it), unless a module of that
/// name is already registered; returns the module.
///
/// This is how an extension module is used without loading the compiled
/// library, e.g. in tests that embed Python: `#[orochi::module] mod mylib`
/// generates `mylib::__orochi_module()`, which calls this.
pub fn load_module(name: &str, init: impl FnOnce(&ModuleBuilder<'_>) -> Result<()>) -> Result<Object> {
    // `init` may release the GIL (an import does), so another thread could
    // start building the same module meanwhile: a Python lock, which
    // releases the GIL while it waits, lets only one thread build at a time.
    static LOCK: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
    attach(|py| {
        let lock = LOCK.get_or_try_init(py, || py.import("threading")?.getattr("RLock")?.call0().map(Bound::unbind))?;
        lock.bind(py).call_method0("acquire")?;
        // Released on every way out, including a panic in `init`.
        let _release = Release(lock.bind(py));
        let modules = py.import("sys")?.getattr("modules")?;
        if let Ok(module) = modules.get_item(name) {
            return Ok(Object::from_bound(module));
        }
        let module = PyModule::new(py, name)?;
        init(&ModuleBuilder { module: module.clone(), name: name.to_owned() })?;
        modules.set_item(name, &module)?;
        Ok(Object::from_bound(module.into_any()))
    })
}

/// Releases a held Python lock when dropped.
struct Release<'a, 'py>(&'a Bound<'py, PyAny>);

impl Drop for Release<'_, '_> {
    fn drop(&mut self) {
        let _ = self.0.call_method0("release");
    }
}

/// The body of a class's `#[new]`: runs the constructor registered with
/// [`ClassBuilder::constructor`] (see [`ModuleBuilder`] for a complete
/// example).
///
/// ```ignore
/// #[pymethods(crate = "orochi::pyo3")]
/// impl Point {
///     #[new]
///     #[pyo3(signature = (*args, **kwargs), text_signature = None)]
///     fn py_new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
///         orochi::construct(args, kwargs)
///     }
/// }
/// ```
///
/// The class's own `tp_new` stays pyo3's, so Python cannot create an
/// instance without running the constructor (`object.__new__(Point)`
/// raises). `text_signature = None` leaves the signature to
/// [`ClassBuilder::constructor`], which sets `__signature__`.
pub fn construct<T: Class>(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<T> {
    let new = constructors().get(&TypeId::of::<T>()).cloned();
    let Some(new) = new else {
        return Err(PyTypeError::new_err(format!(
            "cannot create '{}' instances: no constructor was registered with `ClassBuilder::constructor`",
            T::NAME
        )));
    };
    let new = new.downcast_ref::<Box<Constructor<T>>>().expect("registered under the TypeId of T");
    new(args, kwargs)
}

/// A constructor of class `T`, as [`construct`] calls it.
type Constructor<T> = dyn Fn(&Bound<'_, PyTuple>, Option<&Bound<'_, PyDict>>) -> PyResult<T> + Send + Sync;

/// The constructor of each class (a `Box<Constructor<T>>`) by the class's
/// `TypeId`. A `#[new]` is a plain function that cannot hold a closure, so
/// [`construct`] looks the closure up here.
fn constructors() -> MutexGuard<'static, BTreeMap<TypeId, Arc<dyn Any + Send + Sync>>> {
    static CONSTRUCTORS: Mutex<BTreeMap<TypeId, Arc<dyn Any + Send + Sync>>> = Mutex::new(BTreeMap::new());
    CONSTRUCTORS.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<'py> ModuleBuilder<'py> {
    fn py(&self) -> Python<'py> {
        self.module.py()
    }

    /// The module being built.
    pub fn module(&self) -> Object {
        Object::from_bound(self.module.clone().into_any())
    }

    /// Sets the module docstring (`__doc__`); an empty doc is ignored.
    pub fn doc(&self, doc: &str) -> Result<&Self> {
        if let Some(doc) = non_empty(doc) {
            self.module.setattr("__doc__", doc)?;
        }
        Ok(self)
    }

    /// Adds function `name`: `params` names the parameters (they can be
    /// passed by keyword; trailing `Option`s default to `None`), `doc` is its
    /// docstring.
    pub fn function<M, R, F>(&self, name: &str, params: &[&str], doc: &str, f: F) -> Result<&Self>
    where
        F: Handler<M>,
        F::Output: Ret<R>,
    {
        let sig = Signature::new(name.to_owned(), false, params, F::OPTIONAL)?;
        self.module.add(name, typed_function(self.py(), name, &self.name, doc, sig, f)?)?;
        Ok(self)
    }

    /// Adds module attribute `name` (a constant).
    pub fn constant<M>(&self, name: &str, value: impl Ret<M>) -> Result<&Self> {
        self.module.add(name, value.ret(self.py())?)?;
        Ok(self)
    }

    /// Adds class `T` under its Rust name and returns its builder.
    pub fn class<T: Class>(&self) -> Result<ClassBuilder<'py, T>> {
        // Methods bind through `types.MethodType`; import it now.
        crate::function::method_type(self.py())?;
        let ty = self.py().get_type::<T>();
        ty.setattr("__module__", &self.name)?;
        self.module.add(T::NAME, &ty)?;
        Ok(ClassBuilder { qualname: ty.qualname()?.to_string(), ty, module: self.name.clone(), _class: PhantomData })
    }

    /// Adds submodule `name`, built by `init`; it is also importable as
    /// `import parent.name`.
    pub fn submodule(
        &self,
        name: &str,
        doc: &str,
        init: impl FnOnce(&ModuleBuilder<'py>) -> Result<()>,
    ) -> Result<&Self> {
        let full_name = format!("{}.{name}", self.name);
        let module = PyModule::new(self.py(), &full_name)?;
        let builder = ModuleBuilder { module, name: full_name };
        builder.doc(doc)?;
        init(&builder)?;
        self.module.add(name, &builder.module)?;
        self.py().import("sys")?.getattr("modules")?.set_item(&builder.name, &builder.module)?;
        Ok(self)
    }
}

/// A `function` calling the typed Rust function `f`.
fn typed_function<'py, M, R, F>(
    py: Python<'py>,
    name: &str,
    module: &str,
    doc: &str,
    sig: Signature,
    f: F,
) -> PyResult<Bound<'py, PyAny>>
where
    F: Handler<M>,
    F::Output: Ret<R>,
{
    new_function(py, name, module, doc, sig, move |py, args, names| f.call(args, names)?.ret(py))
}

/// Builds a class: attaches methods, properties and special methods to a
/// `#[pyclass]` type. Created by [`ModuleBuilder::class`]; see there for an
/// example. Like [`ModuleBuilder`]'s, every method returns a reference to
/// the builder, so calls can be chained.
pub struct ClassBuilder<'py, T> {
    ty: Bound<'py, PyType>,
    module: String,
    qualname: String,
    _class: PhantomData<fn() -> T>,
}

impl<'py, T: Class> ClassBuilder<'py, T> {
    fn py(&self) -> Python<'py> {
        self.ty.py()
    }

    fn function<M, R, F>(&self, name: &str, doc: &str, method: bool, params: &[&str], f: F) -> PyResult<Bound<'py, PyAny>>
    where
        F: Handler<M>,
        F::Output: Ret<R>,
    {
        let qualname = format!("{}.{name}", self.qualname);
        if method && F::receiver() != Some(TypeId::of::<T>()) {
            return Err(PyTypeError::new_err(format!(
                "{qualname}: the first parameter of a method must be `&{0}`, `&mut {0}` or `{0}`",
                T::NAME
            )));
        }
        let sig = Signature::new(qualname, method, params, F::OPTIONAL)?;
        typed_function(self.py(), name, &self.module, doc, sig, f)
    }

    /// The class object.
    pub fn class(&self) -> Object {
        Object::from_bound(self.ty.clone().into_any())
    }

    /// Sets the class docstring (`__doc__`); an empty doc is ignored.
    /// (`#[pyclass]` already uses the struct's doc comment.)
    pub fn doc(&self, doc: &str) -> Result<&Self> {
        if let Some(doc) = non_empty(doc) {
            self.ty.setattr("__doc__", doc)?;
        }
        Ok(self)
    }

    /// Sets the constructor, which the class's `#[new]` runs through
    /// [`construct`]: `params` names the parameters of `new` (as for a
    /// function), which returns `T` or `Result<T, E>`. Sets the class's
    /// `__signature__` and, the first time, appends `doc` to its docstring
    /// after a blank line.
    pub fn constructor<M, F>(&self, params: &[&str], doc: &str, new: F) -> Result<&Self>
    where
        F: Handler<M>,
        F::Output: IntoInstance<T>,
    {
        let sig = Signature::new(self.qualname.clone(), false, params, F::OPTIONAL)?;
        self.ty.setattr("__signature__", sig.to_inspect(self.py())?)?;
        let new: Box<Constructor<T>> =
            Box::new(move |args, kwargs| new.call(&sig.bind(args, kwargs)?, &sig.names)?.into_instance());
        // A replaced constructor is dropped after the table is unlocked: its
        // drop may run Python code.
        let replaced = constructors().insert(TypeId::of::<T>(), Arc::new(new));
        if let (None, Some(doc)) = (&replaced, non_empty(doc)) {
            let doc = match self.ty.getattr("__doc__")?.extract::<Option<String>>()? {
                Some(class_doc) if !class_doc.is_empty() => format!("{class_doc}\n\n{doc}"),
                _ => doc.to_owned(),
            };
            self.ty.setattr("__doc__", doc)?;
        }
        Ok(self)
    }

    /// Adds method `name`: the first parameter of `f` is `&T` or `&mut T`
    /// (or `T` for a `Clone` class), otherwise this fails; `params` names
    /// the others.
    pub fn method<M, R, F>(&self, name: &str, params: &[&str], doc: &str, f: F) -> Result<&Self>
    where
        F: Handler<M>,
        F::Output: Ret<R>,
    {
        let function = self.function(name, doc, true, params, f)?;
        self.ty.setattr(name, function)?;
        Ok(self)
    }

    /// Adds static method `name` (`f` has no `self`).
    pub fn static_method<M, R, F>(&self, name: &str, params: &[&str], doc: &str, f: F) -> Result<&Self>
    where
        F: Handler<M>,
        F::Output: Ret<R>,
    {
        let function = self.function(name, doc, false, params, f)?;
        let static_method = self.py().import("builtins")?.getattr("staticmethod")?.call1((function,))?;
        self.ty.setattr(name, static_method)?;
        Ok(self)
    }

    /// Adds class attribute `name` (a constant).
    pub fn constant<M>(&self, name: &str, value: impl Ret<M>) -> Result<&Self> {
        self.ty.setattr(name, value.ret(self.py())?)?;
        Ok(self)
    }

    /// Adds the read/write property `name`. `set` may return a value (such
    /// as the one it replaced); it is dropped after the instance is released.
    pub fn property<G, R, S, V, D>(&self, name: &str, doc: &str, get: G, set: S) -> Result<&Self>
    where
        G: Fn(&T) -> R + Send + Sync + 'static,
        R: IntoObject,
        S: Fn(&mut T, V) -> D + Send + Sync + 'static,
        V: FromObject,
    {
        let fget = self.getter(name, get)?;
        let what = format!("attribute '{name}'");
        let sig = Signature::new(format!("{}.{name}", self.qualname), true, &["value"], &[false, false])?;
        let fset = new_function(self.py(), name, &self.module, "", sig, move |py, args, _| {
            let [Some(slf), Some(value), ..] = args else { unreachable!("the binder checks the count") };
            let instance = instance::<T>(slf, "self")?;
            let value: V = convert_named(value, || what.clone())?;
            let old = set(&mut *instance.try_borrow_mut()?, value);
            drop(old);
            Ok(py.None())
        })?;
        self.add_property(name, doc, fget, Some(fset))
    }

    /// Adds the read-only property `name`.
    pub fn readonly<G, R>(&self, name: &str, doc: &str, get: G) -> Result<&Self>
    where
        G: Fn(&T) -> R + Send + Sync + 'static,
        R: IntoObject,
    {
        let fget = self.getter(name, get)?;
        self.add_property(name, doc, fget, None)
    }

    /// Adds the read/write property `name` for a field: `get` and `get_mut`
    /// point to the field. Reading copies the value, assigning replaces it.
    /// The field type must be a [`Field`].
    pub fn field<V: Field>(
        &self,
        name: &str,
        doc: &str,
        get: impl Fn(&T) -> &V + Send + Sync + 'static,
        get_mut: impl Fn(&mut T) -> &mut V + Send + Sync + 'static,
    ) -> Result<&Self> {
        self.property(name, doc, move |slf| get(slf).clone(), move |slf, value| std::mem::replace(get_mut(slf), value))
    }

    fn getter<G, R>(&self, name: &str, get: G) -> PyResult<Bound<'py, PyAny>>
    where
        G: Fn(&T) -> R + Send + Sync + 'static,
        R: IntoObject,
    {
        // The value is converted after the instance is released.
        self.function(name, "", true, &[], move |slf: &T| get(slf))
    }

    fn add_property(&self, name: &str, doc: &str, fget: Bound<'py, PyAny>, fset: Option<Bound<'py, PyAny>>) -> Result<&Self> {
        let property = self.py().import("builtins")?.getattr("property")?;
        let property = property.call1((fget, fset, None::<()>, non_empty(doc)))?;
        self.ty.setattr(name, property)?;
        Ok(self)
    }

    /// `__repr__` from `Debug`.
    pub fn repr_debug(&self) -> Result<&Self>
    where
        T: std::fmt::Debug,
    {
        self.method("__repr__", &[], "", |slf: &T| format!("{slf:?}"))
    }

    /// `__str__` from `Display`.
    pub fn str_display(&self) -> Result<&Self>
    where
        T: std::fmt::Display,
    {
        self.method("__str__", &[], "", |slf: &T| slf.to_string())
    }

    /// `__eq__` (and `!=`) from `PartialEq`; comparing with another type is
    /// `NotImplemented`. Like defining `__eq__` in Python, this makes the
    /// class unhashable (`__hash__ = None`) unless it has its own `__hash__`
    /// (from [`hash`](Self::hash), called before or after this one).
    pub fn eq(&self) -> Result<&Self>
    where
        T: PartialEq,
    {
        self.method("__eq__", &["other"], "", |slf: &T, other: Object| -> Result<Object> {
            attach(|py| match other.bind(py).cast::<T>() {
                Ok(other) => (*slf == *other.try_borrow().map_err(PyErr::from)?).into_object(),
                Err(_) => Ok(Object::from_py(py.NotImplemented())),
            })
        })?;
        // Only the class's own `__dict__`: an inherited `__hash__` does not
        // count, as in Python.
        if !self.ty.getattr("__dict__")?.contains("__hash__")? {
            self.ty.setattr("__hash__", self.py().None())?;
        }
        Ok(self)
    }

    /// `__hash__` from `Hash`; with [`eq`](Self::eq), in either order, the
    /// class is hashable.
    pub fn hash(&self) -> Result<&Self>
    where
        T: Hash,
    {
        self.method("__hash__", &[], "", |slf: &T| {
            BuildHasherDefault::<DefaultHasher>::default().hash_one(slf) as i64
        })
    }
}

/// A type that can be a `pub` field of a class (a read/write property).
///
/// Reading a field copies its value, so it must be a simple value or one of
/// orochi's reference types: numbers, `bool`, `char`, `String`, `PathBuf`,
/// [`Object`], [`List<T>`], [`Dict<K, V>`], [`Set<T>`], [`Ref<T>`], and
/// `Option`s of those. A `Vec<T>` field, say, would be read as a new list
/// on every access, so `obj.items.append(1)` would silently change a copy.
///
/// Implement it for your own [`FromObject`] + [`IntoObject`] + `Clone` type
/// if reading a copy is what you want.
///
/// ```compile_fail
/// #[orochi::module]
/// mod basket {
///     pub struct Basket {
///         pub items: Vec<i64>, // error: use `List<i64>`, or a private field and methods
///     }
/// }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be the type of a `pub` field of a Python class",
    label = "reading this field from Python would return a copy, so changes to it would be lost",
    note = "use an orochi reference type (`List<T>` for a `Vec<T>`, `Dict<K, V>` for a map, \
            `Set<T>` for a set, `Ref<T>` for another class), which Python reads as the object itself",
    note = "or make the field private and write methods that read and change it"
)]
pub trait Field: FromObject + IntoObject + Clone {}

macro_rules! fields {
    ($($t:ty),*) => {$(impl Field for $t {})*};
}

fields!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char);
fields!(String, PathBuf, Object);

impl<T: Field> Field for Option<T> {}
impl<T: FromObject + IntoObject> Field for List<T> {}
impl<K: FromObject + IntoObject, V: FromObject + IntoObject> Field for Dict<K, V> {}
impl<T: FromObject + IntoObject> Field for Set<T> {}
impl<T: Class> Field for Ref<T> {}
