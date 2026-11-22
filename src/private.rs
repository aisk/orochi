//! Support code for `#[orochi::module]`. Everything here is an implementation
//! detail of the generated code and may change at any time.
//!
//! The macro expands the user's module into pyo3's own macros. Generated
//! wrappers take every Python argument as a plain `Py<PyAny>` and convert it
//! here with orochi's strict [`FromObject`] rules; return values go through
//! [`IntoObject`].
//!
//! A few conversions depend on the concrete type in a way Rust cannot express
//! with one trait impl (e.g. "`Result<T, E>` where `E` converts into
//! [`Error`], otherwise where `E: Display`"). The generated code resolves them
//! with *autoref specialization*: it calls a method on `&&&&Spec<T>`, and the
//! most specific trait implemented for some level of references wins. This
//! works because the generated code always names concrete types.

use std::ffi::CString;
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
pub use pyo3::sync::PyOnceLock;
use pyo3::types::{PyCFunction, PyDict, PyList, PyModule, PySet, PyType};
use pyo3::{PyClassGuard, PyClassGuardMut};

use crate::class::{Class, Ref, RefGuard, RefMutGuard};
pub use crate::convert::ViewShape;
use crate::convert::{FromObject, IntoObject};
use crate::error::{Error, Result};
use crate::object::Object;
use crate::runtime::attach;

pub use pyo3;

/// Return type of generated wrappers.
pub type PyResultObject = PyResult<Py<PyAny>>;

// ---------------------------------------------------------------- container views

const VIEWS_SOURCE: &str = include_str!("views.py");

/// FNV-1a hash of a text, at compile time.
const fn fnv1a(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    hash
}

/// The views module of this orochi version and its `View` base class.
struct Views {
    module: Py<PyModule>,
    view: Py<PyType>,
}

static VIEWS: PyOnceLock<Views> = PyOnceLock::new();

/// Name prefix of the views modules in `sys.modules`.
const VIEWS_PREFIX: &str = "orochi._views_";

/// The Python module with the live container views (`views.py`), loaded on
/// first use. It is registered in `sys.modules` under a name tagged with a
/// hash of its source, and reused from there when another extension built
/// with the same orochi loaded it first, so that both recognize each other's
/// views.
fn views(py: Python<'_>) -> PyResult<&Views> {
    VIEWS.get_or_try_init(py, || -> PyResult<Views> {
        let tag = format!("{:016x}", fnv1a(VIEWS_SOURCE));
        let module = load_views(py, &format!("{VIEWS_PREFIX}{tag}"), &tag, VIEWS_SOURCE)?;
        let view = module.bind(py).getattr("View")?.cast_into::<PyType>()?.unbind();
        Ok(Views { module, view })
    })
}

/// `sys.modules[name]` if it is a views module tagged `tag`; otherwise the
/// views module built from `source`, tagged and registered as `name`
/// (replacing an incompatible module of that name).
fn load_views(py: Python<'_>, name: &str, tag: &str, source: &str) -> PyResult<Py<PyModule>> {
    let modules = py.import("sys")?.getattr("modules")?;
    if let Ok(existing) = modules.get_item(name)
        && let Ok(module) = existing.cast::<PyModule>()
        && let Ok(existing_tag) = module.getattr("_TAG")
        && existing_tag.eq(tag)?
    {
        return Ok(module.clone().unbind());
    }
    let code = CString::new(source).expect("no NUL in views.py");
    let module_name = CString::new(name).expect("no NUL in the module name");
    // `from_code` would execute into a module already registered as `name`.
    if modules.contains(name)? {
        modules.del_item(name)?;
    }
    // Registers the module in `sys.modules` as `name`.
    let module = PyModule::from_code(py, &code, c"orochi/views.py", &module_name)?;
    module.setattr("_TAG", tag)?;
    modules.set_item(name, &module)?;
    Ok(module.unbind())
}

/// Whether `obj` is one of orochi's container views.
fn is_view(obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = obj.py();
    let views = match VIEWS.get(py) {
        Some(views) => views,
        // This extension has created no view yet; one from another extension
        // built with the same orochi still counts.
        None => {
            let from_views = obj
                .get_type()
                .getattr("__module__")
                .is_ok_and(|m| m.extract::<&str>().is_ok_and(|m| m.starts_with(VIEWS_PREFIX)));
            if !from_views {
                return Ok(false);
            }
            views(py)?
        }
    };
    obj.is_instance(views.view.bind(py))
}

/// The Python form of a [`ViewShape`] (see `views.py`).
fn shape_object<'py>(py: Python<'py>, shape: &ViewShape) -> PyResult<Bound<'py, PyAny>> {
    let pair = |kind: &str, inner: Option<&ViewShape>| -> PyResult<Bound<'py, PyAny>> {
        let inner = match inner {
            Some(inner) => shape_object(py, inner)?,
            None => py.None().into_bound(py),
        };
        Ok((kind, inner).into_pyobject(py)?.into_any())
    };
    match shape {
        ViewShape::Plain => Ok(py.None().into_bound(py)),
        ViewShape::List(inner) => pair("list", Some(inner)),
        ViewShape::Dict(inner) => pair("dict", Some(inner)),
        ViewShape::Set => pair("set", None),
        ViewShape::Array(inner) => pair("array", Some(inner)),
    }
}

/// Reads the field through `get`; the positions `shape` marks are returned
/// as live views bound to the instance and the field.
fn live_field<C: Class>(
    slf: &Bound<'_, C>,
    name: &'static str,
    shape: &ViewShape,
    get: RawGet,
    set: RawSet,
) -> PyResultObject {
    let py = slf.py();
    let value = get(slf.as_any())?;
    if *shape == ViewShape::Plain || value.is_none(py) {
        return Ok(value);
    }
    let owner: Py<PyAny> = slf.clone().into_any().unbind();
    let key = (owner.as_ptr() as usize, name);
    let owner_get = owner.clone_ref(py);
    let getter = PyCFunction::new_closure(py, None, None, move |args, _| -> PyResultObject {
        get(owner_get.bind(args.py()))
    })?;
    let setter = PyCFunction::new_closure(py, None, None, move |args, _| -> PyResult<()> {
        let value = args.get_item(0)?.unbind();
        set(owner.bind(args.py()), value)
    })?;
    Ok(views(py)?
        .module
        .bind(py)
        .getattr("_wrap")?
        .call1((shape_object(py, shape)?, value, getter, setter, key))?
        .unbind())
}

// ---------------------------------------------------------------- arguments

/// Converts a required argument. An orochi container view is converted from
/// its (plain) contents, except into a handle type (`List`, ...), which
/// would silently refer to a detached copy.
pub fn arg<T: FromObject>(value: Py<PyAny>, what: &str) -> PyResult<T> {
    let obj = Object::from_py(value);
    let plain = attach(|py| -> PyResult<Option<Object>> {
        let b = obj.bind(py);
        if !is_view(b)? {
            return Ok(None);
        }
        if let Some(builtin) = T::__HANDLE_OF {
            return Err(contextualize(view_to_handle_error(b, builtin)?, what));
        }
        Ok(Some(Object::from_bound(b.call_method0("_get")?)))
    })?;
    let obj = plain.unwrap_or(obj);
    T::from_object(&obj).map_err(|err| contextualize(err, what))
}

/// The error for a view passed where a handle to a `builtin` is expected.
fn view_to_handle_error(view: &Bound<'_, PyAny>, builtin: &str) -> PyResult<Error> {
    let kind: String = view.getattr("_builtin")?.getattr("__name__")?.extract()?;
    if kind != builtin {
        return Ok(crate::convert::type_mismatch(builtin, view));
    }
    let rust = match builtin {
        "list" => "&mut Vec<T>",
        "dict" => "&mut HashMap<K, V>",
        _ => "&mut HashSet<T>",
    };
    let got = view.get_type().name()?;
    Ok(Error::type_error(format!(
        "expected {builtin}, got {got}, a live view of a field, which is not a {builtin} \
         object; to change the field in place, the parameter must be `{rust}` instead of a \
         handle, or pass `{builtin}(view)` to pass a copy"
    )))
}

/// Converts an argument that defaults to `None` when omitted.
pub fn opt_arg<T: FromObject>(value: Option<Py<PyAny>>, what: &str) -> PyResult<T> {
    match value {
        Some(value) => arg(value, what),
        None => T::from_object(&Object::none()).map_err(|err| contextualize(err, what)),
    }
}

/// Says which argument (or attribute) failed to convert, like pyo3 does: a
/// `TypeError` is re-raised as `TypeError("argument 'x': ...")` with the
/// original as its cause; other exceptions (e.g. `OverflowError`) keep their
/// type and get a note.
fn contextualize(err: Error, what: &str) -> PyErr {
    attach(|py| {
        let err = err.into_pyerr();
        if err.is_instance_of::<PyTypeError>(py) {
            let message = err
                .value(py)
                .str()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let new = PyTypeError::new_err(format!("{what}: {message}"));
            new.set_cause(py, Some(err));
            new
        } else {
            let value = err.value(py);
            if value.hasattr("add_note").unwrap_or(false) {
                let _ = value.call_method1("add_note", (format!("while converting {what}"),));
            }
            err
        }
    })
}

/// Rejects the same object (or the same field, through views) passed to two
/// `&mut` parameters, like pyo3's "Already borrowed" for class instances. A
/// view also conflicts with a view of a position inside it, and with the
/// instance whose field it views.
pub fn check_aliasing(args: &[(&Py<PyAny>, &str)]) -> PyResult<()> {
    attach(|py| {
        let module = views(py)?.module.bind(py);
        let alias_key = module.getattr("alias_key")?;
        let aliased = module.getattr("aliased")?;
        let mut keys: Vec<Bound<'_, PyAny>> = Vec::with_capacity(args.len());
        for (obj, _) in args {
            keys.push(alias_key.call1((obj.bind(py),))?);
        }
        for (j, key) in keys.iter().enumerate() {
            for (i, earlier) in keys[..j].iter().enumerate() {
                if aliased.call1((key, earlier))?.is_truthy()? {
                    return Err(PyRuntimeError::new_err(format!(
                        "Already borrowed: {} refers to the same object as {}, which is \
                         already lent mutably to this call",
                        args[j].1, args[i].1
                    )));
                }
            }
        }
        Ok(())
    })
}

/// Selects how a `&T` / `&mut T` parameter is produced from a Python object.
pub struct ParamSpec<T: ?Sized>(PhantomData<fn() -> *const T>);

impl<T: ?Sized> ParamSpec<T> {
    pub fn new() -> Self {
        ParamSpec(PhantomData)
    }
}

impl<T: ?Sized> Default for ParamSpec<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// `&T` / `&mut T` where `T` is a class: borrows the Python instance itself.
pub trait ClassParamKind<T> {
    fn param_kind(&self) -> ClassParam<T>;
}

impl<T: Class> ClassParamKind<T> for &ParamSpec<T> {
    fn param_kind(&self) -> ClassParam<T> {
        ClassParam(PhantomData)
    }
}

/// Any other `&T`: converts into `T::Owned` and lends it (`&str` from a
/// `String`, `&[i64]` from a `Vec<i64>`, `&Object` from an `Object`, ...).
pub trait OwnedParamKind<T: ?Sized> {
    fn param_kind(&self) -> OwnedParam<T>;
}

impl<T: ?Sized> OwnedParamKind<T> for ParamSpec<T> {
    fn param_kind(&self) -> OwnedParam<T> {
        OwnedParam(PhantomData)
    }
}

/// Classes whose instances may be lent as `&mut` (structs; enum members are
/// shared singletons).
#[diagnostic::on_unimplemented(
    message = "`&mut {Self}` cannot be a parameter of a function exported to Python",
    label = "the members of an exported enum are shared singletons",
    note = "take the enum by value (`{Self}`) or as `&{Self}`, and return the new value"
)]
pub trait MutableClass: Class {}

/// Parameters of class type borrow the Python instance itself (pyo3's
/// runtime borrow checking; a conflict raises `RuntimeError`).
pub struct ClassParam<T>(PhantomData<fn() -> T>);

impl<T: Class> ClassParam<T> {
    pub fn hold(&self, value: Py<PyAny>, what: &str) -> PyResult<Ref<T>> {
        arg::<Ref<T>>(value, what)
    }
    pub fn hold_opt(&self, value: Option<Py<PyAny>>, what: &str) -> PyResult<Option<Ref<T>>> {
        opt_arg::<Option<Ref<T>>>(value, what)
    }
    pub fn hold_mut(&self, value: Py<PyAny>, what: &str) -> PyResult<Ref<T>>
    where
        T: MutableClass,
    {
        self.hold(value, what)
    }
    pub fn get<'a>(&self, holder: &'a Ref<T>) -> PyResult<RefGuard<'a, T>> {
        Ok(holder.borrow()?)
    }
    pub fn get_opt<'a>(&self, holder: &'a Option<Ref<T>>) -> PyResult<Option<RefGuard<'a, T>>> {
        holder.as_ref().map(|h| self.get(h)).transpose()
    }
    pub fn get_mut<'a>(&self, holder: &'a mut Ref<T>) -> PyResult<RefMutGuard<'a, T>> {
        Ok(holder.borrow_mut()?)
    }
    /// Nothing to write back: the instance was modified in place.
    pub fn write_back(&self, _holder: Ref<T>) -> PyResult<()> {
        Ok(())
    }
}

/// Parameters of any other type: `&T` lends a temporary `T::Owned`
/// converted with `FromObject`; `&mut T` additionally writes the temporary
/// back into the caller's container after the call ([`WriteBack`]).
pub struct OwnedParam<T: ?Sized>(PhantomData<fn() -> *const T>);

impl<T: ?Sized> OwnedParam<T> {
    // The owned type is a method parameter `O` rather than the projection
    // `T::Owned`, so that a `T` without a conversion is reported as a missing
    // `Clone`/`FromObject` implementation, not as an unusable method.
    pub fn hold<O>(&self, value: Py<PyAny>, what: &str) -> PyResult<O>
    where
        T: ToOwned<Owned = O>,
        O: FromObject,
    {
        arg::<O>(value, what)
    }
    pub fn hold_opt<O>(&self, value: Option<Py<PyAny>>, what: &str) -> PyResult<Option<O>>
    where
        T: ToOwned<Owned = O>,
        O: FromObject,
    {
        opt_arg::<Option<O>>(value, what)
    }
    pub fn get<'a, O: std::borrow::Borrow<T>>(&self, holder: &'a O) -> PyResult<&'a T> {
        Ok(holder.borrow())
    }
    pub fn get_opt<'a, O: std::borrow::Borrow<T>>(
        &self,
        holder: &'a Option<O>,
    ) -> PyResult<Option<&'a T>> {
        Ok(holder.as_ref().map(std::borrow::Borrow::borrow))
    }

    pub fn hold_mut(&self, value: Py<PyAny>, what: &str) -> PyResult<MutHolder<T>>
    where
        T: WriteBack,
    {
        let obj = Object::from_py(value);
        // The caller's container itself, or the contents of a view (written
        // back through the view).
        let target = attach(|py| -> PyResult<Option<(Object, Option<Object>)>> {
            let b = obj.bind(py);
            if T::accepts(b) {
                return Ok(Some((Object::from_bound(b.clone()), None)));
            }
            if is_view(b)?
                && let contents = b.call_method0("_get")?
                && T::accepts(&contents)
            {
                return Ok(Some((
                    Object::from_bound(contents),
                    Some(Object::from_bound(b.clone())),
                )));
            }
            Ok(None)
        })?;
        let Some((target, view)) = target else {
            let err = crate::convert::type_mismatch_object(T::EXPECTED, &obj);
            return Err(contextualize(err, what));
        };
        let value = T::from_object(&target).map_err(|err| contextualize(err, what))?;
        Ok(MutHolder {
            target,
            view,
            value,
            write: T::write_back,
        })
    }
    pub fn get_mut<'a>(&self, holder: &'a mut MutHolder<T>) -> PyResult<&'a mut T>
    where
        T: Sized,
    {
        Ok(&mut holder.value)
    }
    pub fn write_back(&self, holder: MutHolder<T>) -> PyResult<()>
    where
        T: Sized,
    {
        let changed = (holder.write)(holder.value, &holder.target)?;
        if changed && let Some(view) = &holder.view {
            attach(|py| {
                view.bind(py)
                    .call_method1("_set_raw", (holder.target.bind(py),))
                    .map(drop)
            })?;
        }
        Ok(())
    }
}

/// The caller's container (or a view's contents) and the temporary lent as
/// `&mut T`.
pub struct MutHolder<T> {
    target: Object,
    view: Option<Object>,
    value: T,
    write: fn(T, &Object) -> Result<bool>,
}

/// Containers that can be lent as `&mut T`: converted in, then written back
/// into the same Python object (identity preserved) after the call.
#[diagnostic::on_unimplemented(
    message = "`&mut {Self}` cannot be a parameter of a function exported to Python",
    label = "Python cannot see changes made through this `&mut`",
    note = "`&mut` parameters work for `Vec<T>` (a `list`), `HashMap`/`BTreeMap` (a `dict`), \
            `HashSet`/`BTreeSet` (a `set`) and orochi classes: changes are written back into \
            the caller's object",
    note = "Python `str`, `int`, `float`, `bool`, `bytes` and `tuple` objects are immutable; \
            return the new value instead"
)]
pub trait WriteBack: FromObject {
    /// Python type name for error messages.
    const EXPECTED: &'static str;
    /// Whether `obj` is a container this type can be written back into.
    fn accepts(obj: &Bound<'_, PyAny>) -> bool;
    /// Updates `obj` in place to hold `value`; returns whether anything
    /// changed.
    fn write_back(value: Self, obj: &Object) -> Result<bool>;
    /// Type hint of the accepted argument: only the builtin container itself
    /// can be written back (`list[int]`, not `Sequence[int]`), or a live
    /// view of a field (`list[int] | ListView[int, int, int]`). Elements
    /// have their output hints: they are what the caller reads back.
    fn mut_hint() -> String;
}

/// `plain`, or `plain | <view hint>` if `T` is read as a view.
fn or_view<T: IntoObject>(plain: String) -> String {
    match T::__shape() {
        ViewShape::Plain => plain,
        _ => format!("{plain} | {}", T::__view_hint()),
    }
}

impl<T: FromObject + IntoObject> WriteBack for Vec<T> {
    const EXPECTED: &'static str = "list";
    fn accepts(obj: &Bound<'_, PyAny>) -> bool {
        obj.is_instance_of::<PyList>()
    }
    fn mut_hint() -> String {
        or_view::<Self>(format!("list[{}]", T::output_hint()))
    }
    fn write_back(value: Self, obj: &Object) -> Result<bool> {
        // `xs[:] = new`, unless the contents are element-wise equal (a
        // `Vec<u8>` converts to `bytes`, which never equals a list and is
        // fine to assign).
        let new = value.into_object()?;
        attach(|py| {
            let b = obj.bind(py);
            let new = new.bind(py);
            if b.eq(new)? {
                return Ok(false);
            }
            b.set_item(pyo3::types::PySlice::full(py), new)?;
            Ok(true)
        })
    }
}

/// Makes `obj` equal to `new` with the fewest changes: removes the keys that
/// are gone, then assigns new or changed (by Python `==`) entries in `new`'s
/// order.
fn write_back_dict(obj: &Object, new: Object) -> Result<bool> {
    attach(|py| {
        let old = obj.bind(py).cast::<PyDict>().map_err(PyErr::from)?;
        let new = new.bind(py).cast::<PyDict>().map_err(PyErr::from)?;
        let mut changed = false;
        // `keys()` is a new list: safe to delete while iterating it.
        for key in old.keys().iter() {
            if !new.contains(&key)? {
                old.del_item(&key)?;
                changed = true;
            }
        }
        for (key, value) in new.iter() {
            let same = match old.get_item(&key)? {
                Some(current) => current.eq(&value)?,
                None => false,
            };
            if !same {
                old.set_item(&key, &value)?;
                changed = true;
            }
        }
        Ok(changed)
    })
}

/// Makes `obj` equal to `new`: discards the elements that are gone, adds the
/// new ones.
fn write_back_set(obj: &Object, new: Object) -> Result<bool> {
    attach(|py| {
        let old = obj.bind(py).cast::<PySet>().map_err(PyErr::from)?;
        let new = new.bind(py).cast::<PySet>().map_err(PyErr::from)?;
        let mut changed = false;
        for item in crate::convert::snapshot(old.as_any())?.iter() {
            if !new.contains(&item)? {
                old.discard(&item)?;
                changed = true;
            }
        }
        for item in new.iter() {
            if !old.contains(&item)? {
                old.add(&item)?;
                changed = true;
            }
        }
        Ok(changed)
    })
}

impl<K, V, S> WriteBack for std::collections::HashMap<K, V, S>
where
    K: FromObject + IntoObject + Eq + Hash,
    V: FromObject + IntoObject,
    S: std::hash::BuildHasher + Default,
{
    const EXPECTED: &'static str = "dict";
    fn accepts(obj: &Bound<'_, PyAny>) -> bool {
        obj.is_instance_of::<PyDict>()
    }
    fn mut_hint() -> String {
        or_view::<Self>(format!("dict[{}, {}]", K::output_hint(), V::output_hint()))
    }
    fn write_back(value: Self, obj: &Object) -> Result<bool> {
        write_back_dict(obj, value.into_object()?)
    }
}

impl<K, V> WriteBack for std::collections::BTreeMap<K, V>
where
    K: FromObject + IntoObject + Ord,
    V: FromObject + IntoObject,
{
    const EXPECTED: &'static str = "dict";
    fn accepts(obj: &Bound<'_, PyAny>) -> bool {
        obj.is_instance_of::<PyDict>()
    }
    fn mut_hint() -> String {
        or_view::<Self>(format!("dict[{}, {}]", K::output_hint(), V::output_hint()))
    }
    fn write_back(value: Self, obj: &Object) -> Result<bool> {
        write_back_dict(obj, value.into_object()?)
    }
}

impl<T, S> WriteBack for std::collections::HashSet<T, S>
where
    T: FromObject + IntoObject + Eq + Hash,
    S: std::hash::BuildHasher + Default,
{
    const EXPECTED: &'static str = "set";
    fn accepts(obj: &Bound<'_, PyAny>) -> bool {
        obj.is_instance_of::<PySet>()
    }
    fn mut_hint() -> String {
        or_view::<Self>(format!("set[{}]", T::output_hint()))
    }
    fn write_back(value: Self, obj: &Object) -> Result<bool> {
        write_back_set(obj, value.into_object()?)
    }
}

impl<T: FromObject + IntoObject + Ord> WriteBack for std::collections::BTreeSet<T> {
    const EXPECTED: &'static str = "set";
    fn accepts(obj: &Bound<'_, PyAny>) -> bool {
        obj.is_instance_of::<PySet>()
    }
    fn mut_hint() -> String {
        or_view::<Self>(format!("set[{}]", T::output_hint()))
    }
    fn write_back(value: Self, obj: &Object) -> Result<bool> {
        write_back_set(obj, value.into_object()?)
    }
}

// ---------------------------------------------------------------- return values

/// Error types of exported functions must be `Display` or `Into<Error>`;
/// this trait has no implementations and only exists for its message.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be the error type of a function exported to Python",
    label = "implements neither `std::fmt::Display` nor `Into<orochi::Error>`",
    note = "implement `std::fmt::Display` (the error is raised as `RuntimeError(str(e))`), or \
            `From<{Self}> for orochi::Error` to choose the Python exception"
)]
pub trait PythonError {}

/// Selects how a function's return value becomes a Python object.
pub struct RetSpec<T>(PhantomData<fn() -> T>);

impl<T> RetSpec<T> {
    pub fn of(_: &T) -> Self {
        RetSpec(PhantomData)
    }
}

/// Anything implementing [`IntoObject`], including `Result<T, E>` with
/// `E: Into<Error>`.
pub trait RetIntoKind {
    fn ret_kind(&self) -> RetInto;
}

impl<T: IntoObject> RetIntoKind for &&&RetSpec<T> {
    fn ret_kind(&self) -> RetInto {
        RetInto
    }
}

/// `Result<T, E>` with any `E: Display`: raises `RuntimeError(e.to_string())`.
pub trait RetDisplayKind {
    fn ret_kind(&self) -> RetDisplay;
}

impl<T: IntoObject, E: Display> RetDisplayKind for &&RetSpec<std::result::Result<T, E>> {
    fn ret_kind(&self) -> RetDisplay {
        RetDisplay
    }
}

/// `Result<T, E>` with an unusable `E`: reports [`PythonError`].
pub trait RetBadErrorKind {
    fn ret_kind(&self) -> RetBadError;
}

impl<T: IntoObject, E> RetBadErrorKind for &RetSpec<std::result::Result<T, E>> {
    fn ret_kind(&self) -> RetBadError {
        RetBadError
    }
}

/// Fallback whose `ret` requires `IntoObject`, so a non-convertible type is
/// reported as a missing `IntoObject` implementation.
pub trait RetFallbackKind {
    fn ret_kind(&self) -> RetInto;
}

impl<T> RetFallbackKind for RetSpec<T> {
    fn ret_kind(&self) -> RetInto {
        RetInto
    }
}

pub struct RetInto;

impl RetInto {
    pub fn ret<T: IntoObject>(&self, value: T) -> PyResultObject {
        Ok(value.into_object()?.into_py())
    }
}

pub struct RetDisplay;

impl RetDisplay {
    pub fn ret<T: IntoObject, E: Display>(
        &self,
        value: std::result::Result<T, E>,
    ) -> PyResultObject {
        match value {
            Ok(v) => Ok(v.into_object()?.into_py()),
            Err(e) => Err(Error::runtime_error(e.to_string()).into()),
        }
    }
}

pub struct RetBadError;

impl RetBadError {
    pub fn ret<T, E: PythonError>(&self, _value: std::result::Result<T, E>) -> PyResultObject {
        unreachable!("`PythonError` has no implementations")
    }
}

/// A method returning `&Self` / `&mut Self` returned a reference to another
/// instance: a `Clone` class returns a copy, otherwise this is an error.
pub struct OtherInstance<C>(PhantomData<fn() -> C>);

impl<C> OtherInstance<C> {
    pub fn new() -> Self {
        OtherInstance(PhantomData)
    }
}

impl<C> Default for OtherInstance<C> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait OtherCloneKind {
    fn other_kind(&self) -> OtherClone;
}

impl<C: Class + Clone> OtherCloneKind for &OtherInstance<C> {
    fn other_kind(&self) -> OtherClone {
        OtherClone
    }
}

pub trait OtherRejectKind {
    fn other_kind(&self) -> OtherReject;
}

impl<C: Class> OtherRejectKind for OtherInstance<C> {
    fn other_kind(&self) -> OtherReject {
        OtherReject
    }
}

pub struct OtherClone;

impl OtherClone {
    pub fn convert<C: Class + Clone>(&self, value: &C, _what: &str) -> PyResultObject {
        Ok(class_into_object(value.clone())?.into_py())
    }
}

pub struct OtherReject;

impl OtherReject {
    pub fn convert<C: Class>(&self, _value: &C, what: &str) -> PyResultObject {
        Err(PyRuntimeError::new_err(format!(
            "{what} returned a reference to a different instance and the class is not Clone, so \
             it cannot be returned to Python (derive `Clone` to return a copy, or return a \
             `Ref<{name}>`)",
            name = C::PYTHON_NAME.rsplit('.').next().unwrap_or(C::PYTHON_NAME)
        )))
    }
}

// ---------------------------------------------------------------- constructors

/// Selects how a constructor's return value (`Self` or `Result<Self, E>`)
/// becomes the new instance.
pub struct CtorSpec<T>(PhantomData<fn() -> T>);

impl<T> CtorSpec<T> {
    pub fn of(_: &T) -> Self {
        CtorSpec(PhantomData)
    }
}

pub trait CtorResultKind {
    fn ctor_kind(&self) -> CtorResult;
}

impl<T, E: Into<Error>> CtorResultKind for &&&CtorSpec<std::result::Result<T, E>> {
    fn ctor_kind(&self) -> CtorResult {
        CtorResult
    }
}

pub trait CtorDisplayKind {
    fn ctor_kind(&self) -> CtorDisplay;
}

impl<T, E: Display> CtorDisplayKind for &&CtorSpec<std::result::Result<T, E>> {
    fn ctor_kind(&self) -> CtorDisplay {
        CtorDisplay
    }
}

pub trait CtorBadErrorKind {
    fn ctor_kind(&self) -> CtorBadError;
}

impl<T, E> CtorBadErrorKind for &CtorSpec<std::result::Result<T, E>> {
    fn ctor_kind(&self) -> CtorBadError {
        CtorBadError
    }
}

pub trait CtorPlainKind {
    fn ctor_kind(&self) -> CtorPlain;
}

impl<T> CtorPlainKind for CtorSpec<T> {
    fn ctor_kind(&self) -> CtorPlain {
        CtorPlain
    }
}

pub struct CtorResult;

impl CtorResult {
    pub fn ctor<T, E: Into<Error>>(&self, value: std::result::Result<T, E>) -> PyResult<T> {
        value.map_err(|e| e.into().into_pyerr())
    }
}

pub struct CtorDisplay;

impl CtorDisplay {
    pub fn ctor<T, E: Display>(&self, value: std::result::Result<T, E>) -> PyResult<T> {
        value.map_err(|e| Error::runtime_error(e.to_string()).into_pyerr())
    }
}

pub struct CtorBadError;

impl CtorBadError {
    pub fn ctor<T, E: PythonError>(&self, _value: std::result::Result<T, E>) -> PyResult<T> {
        unreachable!("`PythonError` has no implementations")
    }
}

pub struct CtorPlain;

impl CtorPlain {
    pub fn ctor<T>(&self, value: T) -> PyResult<T> {
        Ok(value)
    }
}

// ---------------------------------------------------------------- classes

/// Borrows `self` of a method immutably.
pub fn borrow<'a, C: Class>(slf: &'a Bound<'_, C>) -> PyResult<PyClassGuard<'a, C>> {
    Ok(PyClassGuard::try_from(slf)?)
}

/// Borrows `self` of a method mutably.
pub fn borrow_mut<'a, C: Class>(slf: &'a Bound<'_, C>) -> PyResult<PyClassGuardMut<'a, C>> {
    Ok(PyClassGuardMut::try_from(slf)?)
}

/// The Python object of a method's `self` (for methods returning `&Self` /
/// `&mut Self`, which return the same object to allow chaining).
pub fn self_object<C: Class>(slf: &Bound<'_, C>) -> Py<PyAny> {
    slf.clone().into_any().unbind()
}

/// `self` by value for a method of a `Clone` class: a copy of the instance.
#[diagnostic::on_unimplemented(
    message = "methods taking `self` by value need `{Self}` to implement `Clone`",
    label = "Python calls this method on a copy of the instance",
    note = "add `#[derive(Clone)]` to `{Self}`, or take `&self` / `&mut self`"
)]
pub trait SelfByValue: Sized {
    fn copy_self(&self) -> Self;
}

impl<T: Clone> SelfByValue for T {
    fn copy_self(&self) -> Self {
        self.clone()
    }
}

/// The copy of `self` for a method taking `self` by value (the anchor of
/// the error message for a class that is not `Clone`).
pub fn self_by_value<C: Class + SelfByValue>(slf: &Bound<'_, C>) -> PyResult<C> {
    Ok(borrow(slf)?.copy_self())
}

/// Field read for getters: a copy of the field (a `Ref` field returns the
/// referenced object itself).
pub fn get_field<C: Class, T: Clone + IntoObject>(
    slf: &Bound<'_, C>,
    field: impl FnOnce(&C) -> &T,
) -> PyResultObject {
    let value = field(&*borrow(slf)?).clone();
    Ok(value.into_object()?.into_py())
}

/// Field setter: converts first, then stores under a mutable borrow; the old
/// value is dropped after the borrow is released (its `Drop` may run Python
/// code that touches the instance).
pub fn set_field<C: Class, T: FromObject>(
    slf: &Bound<'_, C>,
    value: Py<PyAny>,
    what: &str,
    field: impl FnOnce(&mut C) -> &mut T,
) -> PyResult<()> {
    let value = arg::<T>(value, what)?;
    let old = {
        let mut guard = borrow_mut(slf)?;
        std::mem::replace(field(&mut *guard), value)
    };
    drop(old);
    Ok(())
}

/// Reads a field of the instance `obj` (for views).
pub type RawGet = for<'a, 'py> fn(&'a Bound<'py, PyAny>) -> PyResultObject;
/// Assigns a field of the instance `obj` (for views).
pub type RawSet = for<'a, 'py> fn(&'a Bound<'py, PyAny>, Py<PyAny>) -> PyResult<()>;

pub fn raw_get<C: Class, T: Clone + IntoObject>(
    obj: &Bound<'_, PyAny>,
    field: impl FnOnce(&C) -> &T,
) -> PyResultObject {
    get_field(obj.cast::<C>()?, field)
}

pub fn raw_set<C: Class, T: FromObject>(
    obj: &Bound<'_, PyAny>,
    value: Py<PyAny>,
    what: &str,
    field: impl FnOnce(&mut C) -> &mut T,
) -> PyResult<()> {
    set_field(obj.cast::<C>()?, value, what, field)
}


/// Selects how a field getter returns the field.
pub struct FieldSpec<T>(PhantomData<fn() -> T>);

impl<T> FieldSpec<T> {
    pub fn new() -> Self {
        FieldSpec(PhantomData)
    }
}

impl<T> Default for FieldSpec<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Fields of a convertible type: container positions (by the type's
/// [`ViewShape`]) are read as live views.
pub trait LiveFieldKind<T> {
    fn field_kind(&self) -> LiveField<T>;
}

impl<T: IntoObject> LiveFieldKind<T> for &FieldSpec<T> {
    fn field_kind(&self) -> LiveField<T> {
        LiveField(PhantomData)
    }
}

pub trait CopyFieldKind {
    fn field_kind(&self) -> CopyField;
}

impl<T> CopyFieldKind for FieldSpec<T> {
    fn field_kind(&self) -> CopyField {
        CopyField
    }
}

pub struct LiveField<T>(PhantomData<fn() -> T>);

impl<T: IntoObject> LiveField<T> {
    pub fn get<C: Class>(
        &self,
        slf: &Bound<'_, C>,
        name: &'static str,
        get: RawGet,
        set: RawSet,
    ) -> PyResultObject {
        live_field(slf, name, &T::__shape(), get, set)
    }
}

pub struct CopyField;

impl CopyField {
    pub fn get<C: Class>(
        &self,
        slf: &Bound<'_, C>,
        _name: &'static str,
        get: RawGet,
        _set: RawSet,
    ) -> PyResultObject {
        get(slf.as_any())
    }
}

/// `IntoObject` for a class: moves the value into a new Python object.
pub fn class_into_object<C: Class>(value: C) -> Result<Object> {
    Ok(Ref::new(value)?.into())
}

/// `FromObject` for a `Clone` class: copies the instance.
pub fn class_from_object<C: Class + Clone>(obj: &Object) -> Result<C> {
    let r = Ref::<C>::from_object(obj)?;
    let copy = r.borrow()?.clone();
    Ok(copy)
}

/// The member objects of an enum class, by variant index (`None` for a
/// variant compiled out with `#[cfg]`).
pub type EnumMembers = PyOnceLock<Vec<Option<Py<PyAny>>>>;

/// Reads the member singletons of the enum class `C` (named `names`, in
/// variant order) into `cache`, once. The generated module init calls this
/// right after creating the class, so later conversions return these
/// objects even if the class attributes are reassigned.
pub fn enum_members<'a, C: Class>(
    py: Python<'_>,
    cache: &'a EnumMembers,
    names: &[&str],
) -> PyResult<&'a [Option<Py<PyAny>>]> {
    let members = cache.get_or_try_init(py, || -> PyResult<_> {
        let class = py.get_type::<C>();
        Ok(names
            .iter()
            .map(|name| class.getattr(*name).ok().map(Bound::unbind))
            .collect())
    })?;
    Ok(members)
}

/// The `__orochi_members` function of an enum: its cached members.
pub type MembersFn = for<'py> fn(Python<'py>) -> PyResult<&'static [Option<Py<PyAny>>]>;

/// `IntoObject` for an enum: the cached member singleton of variant `index`.
pub fn enum_member(
    members: MembersFn,
    index: usize,
) -> Result<Object> {
    attach(|py| match members(py)?.get(index) {
        Some(Some(member)) => Ok(Object::from_py(member.clone_ref(py))),
        _ => Err(Error::runtime_error("enum member missing from its class")),
    })
}

/// `FromObject` for an enum: accepts only members of the class.
pub fn enum_from_object<C: Class>(obj: &Object, copy: impl FnOnce(&C) -> C) -> Result<C> {
    let r = Ref::<C>::from_object(obj)?;
    let guard = r.borrow()?;
    Ok(copy(&guard))
}

/// `self` by value for a method of an enum: the member itself.
pub fn enum_self<C: Class + FromObject>(slf: &Bound<'_, C>) -> PyResult<C> {
    Ok(C::from_object(&Object::from_bound(slf.as_any().clone()))?)
}

/// `__hash__` from `Hash`.
pub fn hash<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Value of a `pub const` / `pub static` (module or class attribute).
pub fn value<T: Clone + IntoObject>(value: &T) -> PyResultObject {
    Ok(value.clone().into_object()?.into_py())
}

// ---------------------------------------------------------------- modules

/// Signature of the generated per-module initialization function.
pub type ModuleInit = for<'py> fn(&Bound<'py, PyModule>) -> PyResult<()>;

pub fn add_function(
    module: &Bound<'_, PyModule>,
    function: Bound<'_, PyCFunction>,
) -> PyResult<()> {
    module.add_function(function)
}

pub fn add_class<C: Class>(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<C>()
}

pub fn add_value<T: Clone + IntoObject>(
    module: &Bound<'_, PyModule>,
    name: &str,
    v: &T,
) -> PyResult<()> {
    module.add(name, value(v)?)
}

/// Sets `__doc__` from the module's doc comments (trimmed).
fn set_doc(module: &Bound<'_, PyModule>, doc: &str) -> PyResult<()> {
    let doc = doc.trim();
    if !doc.is_empty() {
        module.setattr("__doc__", doc)?;
    }
    Ok(())
}

/// Creates submodule `full_name` (e.g. `"mylib.geometry"`), adds it to
/// `parent` as attribute `name` and registers it in `sys.modules` so that
/// `import mylib.geometry` works too.
pub fn add_submodule(
    parent: &Bound<'_, PyModule>,
    name: &str,
    full_name: &str,
    doc: &str,
    init: ModuleInit,
) -> PyResult<()> {
    let py = parent.py();
    let module = PyModule::new(py, full_name)?;
    set_doc(&module, doc)?;
    init(&module)?;
    parent.add(name, &module)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item(full_name, &module)?;
    Ok(())
}

/// Builds the module in-process (once; later calls return the same object)
/// and registers it in `sys.modules`. Used when the extension is not loaded
/// through `PyInit_<name>`, e.g. in tests that embed Python.
pub fn module_object(
    cell: &'static PyOnceLock<Py<PyModule>>,
    name: &str,
    doc: &str,
    init: ModuleInit,
) -> Result<Object> {
    attach(|py| {
        // `PyOnceLock` runs `init` once even when several threads race here,
        // without deadlocking against the interpreter.
        let module = cell.get_or_try_init(py, || -> PyResult<Py<PyModule>> {
            let module = PyModule::new(py, name)?;
            set_doc(&module, doc)?;
            init(&module)?;
            py.import("sys")?
                .getattr("modules")?
                .set_item(name, &module)?;
            Ok(module.unbind())
        })?;
        Ok(Object::from_py(module.clone_ref(py).into_any()))
    })
}

// ---------------------------------------------------------------- type stubs

/// The description of a module that `.pyi` stubs are rendered from.
pub mod stub {
    pub use crate::stubs::{Class, Function, Module, Param};

    /// The [`Stubs`](crate::Stubs) of a root module.
    pub fn stubs(root: Module) -> crate::Stubs {
        crate::Stubs::new(root)
    }
}

/// Type hints of parameters, return values and fields, selected with
/// autoref specialization like the conversions above: the generated code
/// calls e.g. `(&&&Hint::<T>::new()).borrowed()`, and the most specific
/// implementation that applies to `T` wins. Types without a conversion (the
/// wrapper reports those) fall back to `typing.Any` instead of repeating the
/// compile error.
pub mod hint {
    use std::marker::PhantomData;

    use super::WriteBack;
    use crate::class::Class;
    use crate::convert::{FromObject, IntoObject, ViewShape};

    const ANY: &str = "typing.Any";

    pub struct Hint<T: ?Sized>(PhantomData<fn() -> *const T>);

    impl<T: ?Sized> Hint<T> {
        pub fn new() -> Self {
            Hint(PhantomData)
        }
    }

    impl<T: ?Sized> Default for Hint<T> {
        fn default() -> Self {
            Self::new()
        }
    }

    /// `T | None`, without repeating `None` (the renderer dedupes unions).
    pub fn optional(hint: String) -> String {
        format!("{hint} | None")
    }

    // By-value parameters and field setters: `(&&Hint::<T>::new()).input()`.

    pub trait InputHint {
        fn input(&self) -> String;
    }

    impl<T: FromObject> InputHint for &Hint<T> {
        fn input(&self) -> String {
            T::input_hint()
        }
    }

    pub trait InputHintAny {
        fn input(&self) -> String;
    }

    impl<T: ?Sized> InputHintAny for Hint<T> {
        fn input(&self) -> String {
            ANY.to_owned()
        }
    }

    // `&T` and `Option<&T>` parameters: `(&&&Hint::<T>::new()).borrowed()`.

    pub trait BorrowedClassHint {
        fn borrowed(&self) -> String;
    }

    impl<T: Class> BorrowedClassHint for &&Hint<T> {
        fn borrowed(&self) -> String {
            T::PYTHON_NAME.to_owned()
        }
    }

    pub trait BorrowedHint {
        fn borrowed(&self) -> String;
    }

    impl<T: ?Sized + ToOwned> BorrowedHint for &Hint<T>
    where
        T::Owned: FromObject,
    {
        fn borrowed(&self) -> String {
            <T::Owned as FromObject>::input_hint()
        }
    }

    pub trait BorrowedHintAny {
        fn borrowed(&self) -> String;
    }

    impl<T: ?Sized> BorrowedHintAny for Hint<T> {
        fn borrowed(&self) -> String {
            ANY.to_owned()
        }
    }

    // `&mut T` parameters: `(&&&Hint::<T>::new()).mutable()`.

    pub trait MutableClassHint {
        fn mutable(&self) -> String;
    }

    impl<T: Class> MutableClassHint for &&Hint<T> {
        fn mutable(&self) -> String {
            T::PYTHON_NAME.to_owned()
        }
    }

    pub trait MutableHint {
        fn mutable(&self) -> String;
    }

    impl<T: WriteBack> MutableHint for &Hint<T> {
        fn mutable(&self) -> String {
            T::mut_hint()
        }
    }

    pub trait MutableHintAny {
        fn mutable(&self) -> String;
    }

    impl<T: ?Sized> MutableHintAny for Hint<T> {
        fn mutable(&self) -> String {
            ANY.to_owned()
        }
    }

    // Return values and constants: `(&&&Hint::<T>::new()).output()`.
    // `Result<T, E>` is `T`, also when `E` is only `Display`.

    pub trait OutputResultHint {
        fn output(&self) -> String;
    }

    impl<T: IntoObject, E> OutputResultHint for &&Hint<std::result::Result<T, E>> {
        fn output(&self) -> String {
            T::output_hint()
        }
    }

    pub trait OutputHint {
        fn output(&self) -> String;
    }

    impl<T: IntoObject> OutputHint for &Hint<T> {
        fn output(&self) -> String {
            T::output_hint()
        }
    }

    pub trait OutputHintAny {
        fn output(&self) -> String;
    }

    impl<T: ?Sized> OutputHintAny for Hint<T> {
        fn output(&self) -> String {
            ANY.to_owned()
        }
    }

    // Field getters: `(&&Hint::<T>::new()).field()`; container positions are
    // read as live views.

    pub trait FieldHint {
        fn field(&self) -> String;
    }

    impl<T: IntoObject> FieldHint for &Hint<T> {
        fn field(&self) -> String {
            T::__view_hint()
        }
    }

    pub trait FieldHintAny {
        fn field(&self) -> String;
    }

    impl<T: ?Sized> FieldHintAny for Hint<T> {
        fn field(&self) -> String {
            ANY.to_owned()
        }
    }

    // Field setters: `(&&&Hint::<T>::new()).setter()`; a field read as a
    // view also accepts a view (of another instance, say).

    pub trait SetterViewHint {
        fn setter(&self) -> String;
    }

    impl<T: FromObject + IntoObject> SetterViewHint for &&Hint<T> {
        fn setter(&self) -> String {
            match T::__shape() {
                // An array reads as a plain `list`, which the input accepts.
                ViewShape::Plain | ViewShape::Array(_) => T::input_hint(),
                _ => format!("{} | {}", T::input_hint(), T::__view_hint()),
            }
        }
    }

    pub trait SetterHint {
        fn setter(&self) -> String;
    }

    impl<T: FromObject> SetterHint for &Hint<T> {
        fn setter(&self) -> String {
            T::input_hint()
        }
    }

    pub trait SetterHintAny {
        fn setter(&self) -> String;
    }

    impl<T: ?Sized> SetterHintAny for Hint<T> {
        fn setter(&self) -> String {
            ANY.to_owned()
        }
    }
}

/// Adds the hidden `_orochi_stubs(out_dir=None)` function to a root module:
/// it returns `{relative path: text}` of the module's `.pyi` stubs and, given
/// a directory, also writes them there.
pub fn add_stubs_helper(module: &Bound<'_, PyModule>, stubs: fn() -> crate::Stubs) -> PyResult<()> {
    let py = module.py();
    let function = PyCFunction::new_closure(
        py,
        Some(c"_orochi_stubs"),
        Some(
            c"_orochi_stubs(out_dir=None)\n--\n\n\
              Type stubs (.pyi) of this module, generated by orochi.\n\n\
              Returns {relative path: text}; if out_dir is given, also writes the files \
              there (e.g. out_dir/name.pyi, or out_dir/name/__init__.pyi and one file per \
              submodule).",
        ),
        move |args, kwargs| -> PyResultObject {
            let py = args.py();
            let mut out_dir: Option<Bound<'_, PyAny>> = match args.len() {
                0 => None,
                1 => Some(args.get_item(0)?),
                n => {
                    return Err(PyTypeError::new_err(format!(
                        "_orochi_stubs() takes at most 1 argument ({n} given)"
                    )));
                }
            };
            if let Some(kwargs) = kwargs {
                for (key, value) in kwargs.iter() {
                    if key.extract::<String>().ok().as_deref() != Some("out_dir")
                        || out_dir.is_some()
                    {
                        return Err(PyTypeError::new_err(format!(
                            "_orochi_stubs() got an unexpected keyword argument {key}"
                        )));
                    }
                    out_dir = Some(value);
                }
            }
            let stubs = stubs();
            if let Some(dir) = out_dir.filter(|d| !d.is_none()) {
                let dir: std::path::PathBuf = arg(dir.unbind(), "argument 'out_dir'")?;
                crate::stubs::write_stubs(&stubs, dir).map_err(Error::from)?;
            }
            let files = PyDict::new(py);
            for (path, text) in stubs.files() {
                files.set_item(path, text)?;
            }
            Ok(files.into_any().unbind())
        },
    )?;
    // Not `add`: that would list it in `__all__`.
    module.setattr("_orochi_stubs", function)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_module_is_reused_only_when_compatible() {
        attach(|py| -> PyResult<()> {
            let name = "orochi._views_unit_test";
            let modules = py.import("sys")?.getattr("modules")?;
            // Not loaded yet: a new module, tagged and registered.
            let first = load_views(py, name, "t1", VIEWS_SOURCE)?;
            let first = first.bind(py);
            assert!(modules.get_item(name)?.is(first));
            assert_eq!(first.getattr("_TAG")?.extract::<String>()?, "t1");
            assert!(first.hasattr("ListView")?);
            // Loaded with the same tag (by another extension): reused.
            assert!(load_views(py, name, "t1", VIEWS_SOURCE)?.bind(py).is(first));
            // Another tag, or something else under that name: replaced.
            let second = load_views(py, name, "t2", VIEWS_SOURCE)?;
            assert!(!second.bind(py).is(first));
            assert!(modules.get_item(name)?.is(second.bind(py)));
            modules.set_item(name, "not a module")?;
            let third = load_views(py, name, "t2", VIEWS_SOURCE)?;
            assert!(modules.get_item(name)?.is(third.bind(py)));
            modules.del_item(name)?;
            Ok(())
        })
        .unwrap();
    }
}