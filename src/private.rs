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

use std::convert::Infallible;
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
pub use pyo3::sync::PyOnceLock;
use pyo3::types::{PyCFunction, PyDict, PyModule};
use pyo3::{PyClassGuard, PyClassGuardMut};

use crate::class::{Class, Ref, RefGuard, RefMutGuard};
use crate::convert::{FromObject, IntoObject};
use crate::error::{Error, Result};
use crate::object::Object;
use crate::runtime::attach;

pub use pyo3;

/// Return type of generated wrappers.
pub type PyResultObject = PyResult<Py<PyAny>>;

// ---------------------------------------------------------------- arguments

/// Converts a required argument.
pub fn arg<T: FromObject>(value: Py<PyAny>, what: &str) -> PyResult<T> {
    T::from_object(&Object::from_py(value)).map_err(|err| contextualize(err, what))
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
}

/// Parameters of any other type: `&T` lends a temporary `T::Owned`
/// converted with `FromObject`; `&mut T` is a compile error
/// ([`MutBorrowable`]).
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
    pub fn hold_mut(&self, _value: Py<PyAny>, what: &str) -> PyResult<Infallible>
    where
        T: MutBorrowable,
    {
        // Not reached: `MutBorrowable` has no implementations.
        Err(PyTypeError::new_err(format!("{what} cannot be borrowed mutably")))
    }
    pub fn get_mut<'a>(&self, holder: &'a mut Infallible) -> PyResult<&'a mut T> {
        match *holder {}
    }
}

/// Only class instances can be lent as `&mut`; this trait has no
/// implementations and only exists for its message.
#[diagnostic::on_unimplemented(
    message = "`&mut {Self}` cannot be a parameter of a function exported to Python",
    label = "only classes of an `#[orochi::module]` can be borrowed with `&mut`",
    note = "to modify the caller's list, dict or set, take `List<T>`, `Dict<K, V>` or `Set<T>` \
            (or `Object`): they refer to the caller's object, so they need no `&mut`",
    note = "other Python values, such as `str`, `int` and tuples, are immutable: return the new \
            value instead"
)]
pub trait MutBorrowable {}

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

/// Field read for getters: a clone of the field, converted with
/// `IntoObject::__into_frozen` (Rust containers become immutable snapshots;
/// a handle or `Ref` is the object it refers to).
pub fn get_field<C: Class, T: Clone + IntoObject>(
    slf: &Bound<'_, C>,
    field: impl FnOnce(&C) -> &T,
) -> PyResultObject {
    let value = field(&*borrow(slf)?).clone();
    Ok(value.__into_frozen()?.into_py())
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

    use crate::class::Class;
    use crate::convert::{FromObject, IntoObject};

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

    pub use crate::convert::optional_hint as optional;

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

    // `&T`, `&mut T` and `Option<&T>` parameters:
    // `(&&&Hint::<T>::new()).borrowed()`.

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

    // Field getters: `(&&Hint::<T>::new()).frozen()`.

    pub trait FrozenHint {
        fn frozen(&self) -> String;
    }

    impl<T: IntoObject> FrozenHint for &Hint<T> {
        fn frozen(&self) -> String {
            T::__frozen_hint()
        }
    }

    pub trait FrozenHintAny {
        fn frozen(&self) -> String;
    }

    impl<T: ?Sized> FrozenHintAny for Hint<T> {
        fn frozen(&self) -> String {
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
