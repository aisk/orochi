//! Turning plain Rust functions and closures into Python callables.
//!
//! A function `Fn(A, B) -> R` can be registered with the builder when every
//! parameter type implements [`Arg`] and the return type implements [`Ret`];
//! [`Handler`] is implemented for all such functions with up to 12
//! parameters (`self` counts as one).
//!
//! The marker type parameter `M` of `Arg<M>` / `Ret<M>` / `Handler<M>` keeps
//! the implementations for the different parameter forms apart (so that,
//! for example, `&Point` and `&str` can have different implementations).
//! It is always inferred: write `Handler<M>` in a bound with a generic `M`.

use std::any::TypeId;
use std::borrow::Borrow;

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::{PyRef, PyRefMut};

use crate::class::Class;
use crate::convert::{FromObject, IntoObject};
use crate::error::Error;
use crate::object::Object;

/// The marker types of [`Arg`] and [`Ret`]; one per parameter or return form.
#[doc(hidden)]
pub mod marker {
    pub struct Value;
    pub struct Lent;
    pub struct OptLent;
    pub struct ClassValue;
    pub struct ClassRef;
    pub struct ClassMut;
    pub struct OptClassRef;
    pub struct Instance;
    pub struct OptInstance;
    pub struct InstanceResult;
}
use marker::*;

/// A parameter type of a function exported to Python.
///
/// Implemented for:
///
/// - every [`FromObject`] type: values such as `i64`, `String`, `Vec<T>`,
///   `Option<T>`, and the references [`Object`], [`List`](crate::List),
///   [`Dict`](crate::Dict), [`Set`](crate::Set), [`Ref<T>`](crate::Ref);
/// - `&T` where `T::Owned` is [`FromObject`]: borrows a converted copy
///   (`&str`, `&[T]`, `&Path`, `&HashMap<K, V>`, ...), and `Option<&T>`;
/// - for a [`Class`] `T`: `&T` and `&mut T` (borrow the instance itself),
///   `Option<&T>`, and `T` by value if it is `Clone` (a copy; only alone,
///   not inside `Option`, `Vec`, ...: use [`Ref<T>`](crate::Ref) there).
///
/// Conversions run first, then class instances are borrowed; a conflicting
/// borrow raises `RuntimeError`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be a parameter of a function exported to Python",
    label = "no conversion from a Python object",
    note = "parameters can be values (`FromObject` types: numbers, `String`, `Vec<T>`, ...), \
            orochi references (`Object`, `List<T>`, `Dict<K, V>`, `Set<T>`, `Ref<T>`), borrows of \
            converted values (`&str`, `&[T]`, `&Path`, ...), or `&T` / `&mut T` / `Option<&T>` of a \
            class",
    note = "a class taken by value is a copy and must implement `Clone`; it works only alone (not \
            as `Option<T>`, `Vec<T>`, ...): use `Ref<T>` there",
    note = "`&mut` works only for classes: to modify the caller's list, dict or set, take \
            `List<T>`, `Dict<K, V>` or `Set<T>`; other Python values are immutable, so return the \
            new value"
)]
pub trait Arg<M> {
    /// What is kept during the call: a converted value, or a class
    /// instance and, once borrowed, its borrow guard.
    #[doc(hidden)]
    type Holder<'py>;
    /// What the function receives, borrowing the holder.
    #[doc(hidden)]
    type Out<'a>;
    /// May the argument be omitted (it then defaults to `None`)?
    #[doc(hidden)]
    const OPTIONAL: bool = false;
    /// Converts the argument (`None` if it was omitted). May run Python code.
    #[doc(hidden)]
    fn hold<'py>(obj: Option<&Bound<'py, PyAny>>, name: &str) -> PyResult<Self::Holder<'py>>;
    /// Hands the argument to the function, borrowing class instances.
    #[doc(hidden)]
    fn get<'a, 'py: 'a>(holder: &'a mut Self::Holder<'py>) -> PyResult<Self::Out<'a>>;
    /// `T` for the forms of a class `T` that can be a method's first
    /// parameter: `&T`, `&mut T` and `T`.
    #[doc(hidden)]
    fn class() -> Option<TypeId> {
        None
    }
}

/// Converts argument `name` with [`FromObject`] (`None` if omitted: only
/// optional parameters may be, and they accept `None`).
fn convert<T: FromObject>(obj: Option<&Bound<'_, PyAny>>, name: &str) -> PyResult<T> {
    match obj {
        Some(obj) => convert_named(obj, || format!("argument '{name}'")),
        None => T::from_object(&Object::none()).map_err(Error::into_pyerr),
    }
}

/// Converts `obj` with [`FromObject`], naming it in the error with `what()`
/// (`argument 'x'`): a `TypeError` becomes `TypeError("argument 'x': ...")`
/// with the original as its cause, other exceptions get a note.
///
/// This is the one place that says which argument failed to convert.
pub(crate) fn convert_named<T: FromObject>(
    obj: &Bound<'_, PyAny>,
    what: impl FnOnce() -> String,
) -> PyResult<T> {
    T::from_object(&Object::from_bound(obj.clone())).map_err(|err| {
        let (py, err, what) = (obj.py(), err.into_pyerr(), what());
        if err.is_instance_of::<PyTypeError>(py) {
            let new = PyTypeError::new_err(format!("{what}: {}", message(py, &err)));
            new.set_cause(py, Some(err));
            new
        } else {
            add_note(py, err, format!("while converting {what}"))
        }
    })
}

/// Adds `note` to `err` with `BaseException.add_note` (Python 3.11+).
/// Before 3.11, raises a new exception of the same type, `"{note}: {message}"`,
/// with `err` as its cause; if that type cannot be built from one message,
/// `err` is kept unchanged.
fn add_note(py: Python<'_>, err: PyErr, note: String) -> PyErr {
    let value = err.value(py).clone();
    if py.version_info() >= (3, 11) {
        let _ = value.call_method1("add_note", (note,));
        return err;
    }
    match value.get_type().call1((format!("{note}: {}", message(py, &err)),)) {
        Ok(new) => {
            let new = PyErr::from_value(new);
            new.set_cause(py, Some(err));
            new
        }
        Err(_) => err,
    }
}

/// `str(err)`, or an empty string if that fails.
fn message(py: Python<'_>, err: &PyErr) -> String {
    err.value(py).str().map(|s| s.to_string()).unwrap_or_default()
}

/// The instance of class `T` passed as argument `name`.
pub(crate) fn instance<'py, T: Class>(obj: &Bound<'py, PyAny>, name: &str) -> PyResult<Bound<'py, T>> {
    match obj.cast::<T>() {
        Ok(instance) => Ok(instance.clone()),
        Err(_) => Err(PyTypeError::new_err(format!(
            "argument '{name}': expected {}, got {}",
            T::NAME,
            obj.get_type().name()?
        ))),
    }
}

fn required<'a, 'py>(obj: Option<&'a Bound<'py, PyAny>>) -> &'a Bound<'py, PyAny> {
    obj.expect("the binder supplies every required argument")
}

impl<T: FromObject> Arg<Value> for T {
    type Holder<'py> = Option<T>;
    type Out<'a> = T;
    const OPTIONAL: bool = T::OPTIONAL;
    fn hold<'py>(obj: Option<&Bound<'py, PyAny>>, name: &str) -> PyResult<Option<T>> {
        convert(obj, name).map(Some)
    }
    fn get<'a, 'py: 'a>(holder: &'a mut Option<T>) -> PyResult<T> {
        Ok(holder.take().expect("each argument is used once"))
    }
}

impl<T: ?Sized + ToOwned + 'static> Arg<Lent> for &T
where
    T::Owned: FromObject,
{
    type Holder<'py> = T::Owned;
    type Out<'a> = &'a T;
    fn hold<'py>(obj: Option<&Bound<'py, PyAny>>, name: &str) -> PyResult<T::Owned> {
        convert(obj, name)
    }
    fn get<'a, 'py: 'a>(holder: &'a mut T::Owned) -> PyResult<&'a T> {
        Ok((*holder).borrow())
    }
}

impl<T: ?Sized + ToOwned + 'static> Arg<OptLent> for Option<&T>
where
    T::Owned: FromObject,
{
    type Holder<'py> = Option<T::Owned>;
    type Out<'a> = Option<&'a T>;
    const OPTIONAL: bool = true;
    fn hold<'py>(obj: Option<&Bound<'py, PyAny>>, name: &str) -> PyResult<Option<T::Owned>> {
        convert(obj, name)
    }
    fn get<'a, 'py: 'a>(holder: &'a mut Option<T::Owned>) -> PyResult<Option<&'a T>> {
        Ok(holder.as_ref().map(Borrow::borrow))
    }
}

impl<T: Class + Clone> Arg<ClassValue> for T {
    type Holder<'py> = Bound<'py, T>;
    type Out<'a> = T;
    fn hold<'py>(obj: Option<&Bound<'py, PyAny>>, name: &str) -> PyResult<Bound<'py, T>> {
        instance(required(obj), name)
    }
    fn get<'a, 'py: 'a>(holder: &'a mut Bound<'py, T>) -> PyResult<T> {
        Ok(holder.try_borrow()?.clone())
    }
    fn class() -> Option<TypeId> {
        Some(TypeId::of::<T>())
    }
}

impl<T: Class> Arg<ClassRef> for &T {
    type Holder<'py> = (Bound<'py, T>, Option<PyRef<'py, T>>);
    type Out<'a> = &'a T;
    fn hold<'py>(obj: Option<&Bound<'py, PyAny>>, name: &str) -> PyResult<Self::Holder<'py>> {
        Ok((instance(required(obj), name)?, None))
    }
    fn get<'a, 'py: 'a>(holder: &'a mut Self::Holder<'py>) -> PyResult<&'a T> {
        let guard = holder.1.insert(holder.0.try_borrow()?);
        Ok(&**guard)
    }
    fn class() -> Option<TypeId> {
        Some(TypeId::of::<T>())
    }
}

impl<T: Class> Arg<ClassMut> for &mut T {
    type Holder<'py> = (Bound<'py, T>, Option<PyRefMut<'py, T>>);
    type Out<'a> = &'a mut T;
    fn hold<'py>(obj: Option<&Bound<'py, PyAny>>, name: &str) -> PyResult<Self::Holder<'py>> {
        Ok((instance(required(obj), name)?, None))
    }
    fn get<'a, 'py: 'a>(holder: &'a mut Self::Holder<'py>) -> PyResult<&'a mut T> {
        let guard = holder.1.insert(holder.0.try_borrow_mut()?);
        Ok(&mut **guard)
    }
    fn class() -> Option<TypeId> {
        Some(TypeId::of::<T>())
    }
}

impl<T: Class> Arg<OptClassRef> for Option<&T> {
    type Holder<'py> = (Option<Bound<'py, T>>, Option<PyRef<'py, T>>);
    type Out<'a> = Option<&'a T>;
    const OPTIONAL: bool = true;
    fn hold<'py>(obj: Option<&Bound<'py, PyAny>>, name: &str) -> PyResult<Self::Holder<'py>> {
        let instance = match obj {
            Some(obj) if !obj.is_none() => Some(instance(obj, name)?),
            _ => None,
        };
        Ok((instance, None))
    }
    fn get<'a, 'py: 'a>(holder: &'a mut Self::Holder<'py>) -> PyResult<Option<&'a T>> {
        match &holder.0 {
            Some(instance) => Ok(Some(&**holder.1.insert(instance.try_borrow()?))),
            None => Ok(None),
        }
    }
}

/// A return type of a function exported to Python.
///
/// Implemented for every [`IntoObject`] type (numbers, `String`, `()`,
/// `Vec<T>`, maps, [`Object`], [`Ref<T>`](crate::Ref), `Option<T>` and
/// `Result<T, E>` with `E: Into<orochi::Error>` of those, ...), and for a
/// [`Class`] `T` by value (a new instance): `T`, `Option<T>` and
/// `Result<T, E>`. Not for collections of classes by value (`Vec<T>`) or
/// `Result<Option<T>, E>`: return `Vec<Ref<T>>` / `Result<Option<Ref<T>>>`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be returned to Python",
    label = "no conversion into a Python object",
    note = "return an `IntoObject` type (numbers, `String`, `Vec<T>`, `Object`, `List<T>`, `Ref<T>`, \
            `Option`/`Result` of those, ...), or a class by value as `T`, `Option<T>` or \
            `Result<T, E>`; for anything else holding a class (`Vec<T>`, `Result<Option<T>, E>`), \
            create the instances with `Ref::new` and return `Ref<T>`s",
    note = "the error type of a `Result` must convert into `orochi::Error` (`std::io::Error` and \
            `orochi::Error` do; implement `From<YourError> for orochi::Error` for your own)",
    note = "borrowed values cannot be returned: return `String` instead of `&str`, `Vec<T>` \
            instead of `&[T]`, and `()` from methods that modify `self`"
)]
pub trait Ret<M> {
    #[doc(hidden)]
    fn ret(self, py: Python<'_>) -> PyResult<Py<PyAny>>;
}

impl<T: IntoObject> Ret<Value> for T {
    fn ret(self, _py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.into_object()?.into_py())
    }
}

impl<T: Class> Ret<Instance> for T {
    fn ret(self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(Py::new(py, self)?.into_any())
    }
}

#[diagnostic::do_not_recommend]
impl<T: Class> Ret<OptInstance> for Option<T> {
    fn ret(self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self {
            Some(value) => value.ret(py),
            None => Ok(py.None()),
        }
    }
}

#[diagnostic::do_not_recommend]
impl<T: Class, E: Into<Error>> Ret<InstanceResult> for std::result::Result<T, E> {
    fn ret(self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self {
            Ok(value) => value.ret(py),
            Err(err) => Err(err.into().into_pyerr()),
        }
    }
}

/// What a constructor of class `T` may return: `T` or `Result<T, E>` with
/// `E: Into<orochi::Error>`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be returned by a constructor of `{T}`",
    label = "a constructor returns `Self` or `Result<Self, E>`",
    note = "the error type must convert into `orochi::Error`; to export a function that returns \
            something else, give it another name than `new` (it becomes a static method)"
)]
pub trait IntoInstance<T> {
    #[doc(hidden)]
    fn into_instance(self) -> PyResult<T>;
}

impl<T: Class> IntoInstance<T> for T {
    fn into_instance(self) -> PyResult<T> {
        Ok(self)
    }
}

#[diagnostic::do_not_recommend]
impl<T: Class, E: Into<Error>> IntoInstance<T> for std::result::Result<T, E> {
    fn into_instance(self) -> PyResult<T> {
        self.map_err(|err| err.into().into_pyerr())
    }
}

/// The most parameters a [`Handler`] can have (`self` included). The macro
/// crate checks the same limit (`MAX_PARAMS` in `orochi-macros`).
pub(crate) const MAX_ARITY: usize = 12;

/// The arguments of one call, bound to the function's parameters (`None`
/// for an omitted optional one).
pub(crate) type BoundArgs<'py> = [Option<Bound<'py, PyAny>>; MAX_ARITY];

/// A Rust function or closure callable from Python: every parameter type is
/// an [`Arg`], and there are at most 12 parameters (`self` included). What
/// it may return depends on the use ([`Ret`] for functions and methods,
/// [`IntoInstance`] for constructors). The marker `M` is inferred.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be called from Python",
    label = "a parameter type has no Python conversion",
    note = "see `orochi::Arg` for the supported parameter types; at most 12 parameters are supported"
)]
pub trait Handler<M>: Send + Sync + 'static {
    /// The return type.
    type Output;
    /// For each parameter: may it be omitted?
    #[doc(hidden)]
    const OPTIONAL: &'static [bool];
    /// The class of the first parameter if it is `&T`, `&mut T` or `T` of a
    /// class (see [`Arg`]).
    #[doc(hidden)]
    fn receiver() -> Option<TypeId>;
    /// Calls the function with bound arguments, named `names` in errors.
    #[doc(hidden)]
    fn call(&self, args: &BoundArgs<'_>, names: &[String]) -> PyResult<Self::Output>;
}

macro_rules! first {
    () => {
        None
    };
    ($A:ident $M:ident $(, $RA:ident $RM:ident)*) => {
        <$A as Arg<$M>>::class()
    };
}

macro_rules! handler {
    ($($A:ident $M:ident $h:ident $o:ident $i:tt),*) => {
        impl<Func, R, $($A, $M),*> Handler<(R, $($M, $A),*)> for Func
        where
            // Lets the compiler infer the parameter types from the function...
            Func: Fn($($A),*) -> R,
            // ...and is what is called: each parameter borrows its holder
            // for the duration of the call.
            Func: for<'a> Fn($(<$A as Arg<$M>>::Out<'a>),*) -> R,
            Func: Send + Sync + 'static,
            $($A: Arg<$M>,)*
        {
            type Output = R;
            const OPTIONAL: &'static [bool] = &[$(<$A as Arg<$M>>::OPTIONAL),*];

            fn receiver() -> Option<TypeId> {
                first!($($A $M),*)
            }

            #[allow(unused_variables)]
            fn call(&self, args: &BoundArgs<'_>, names: &[String]) -> PyResult<R> {
                // Selects the higher-ranked `Fn` bound for the call.
                #[allow(clippy::too_many_arguments)]
                fn call<R, $($A),*>(f: &impl Fn($($A),*) -> R, $($o: $A),*) -> R {
                    f($($o),*)
                }
                // Convert everything first, then borrow class instances; the
                // borrows end before the result is converted.
                $(let mut $h = <$A as Arg<$M>>::hold(args[$i].as_ref(), &names[$i])?;)*
                $(let $o = <$A as Arg<$M>>::get(&mut $h)?;)*
                Ok(call(self, $($o),*))
            }
        }
    };
}

handler!();
handler!(A0 M0 h0 o0 0);
handler!(A0 M0 h0 o0 0, A1 M1 h1 o1 1);
handler!(A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2);
handler!(A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3);
handler!(A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3, A4 M4 h4 o4 4);
handler!(A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3, A4 M4 h4 o4 4, A5 M5 h5 o5 5);
handler!(
    A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3, A4 M4 h4 o4 4, A5 M5 h5 o5 5,
    A6 M6 h6 o6 6
);
handler!(
    A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3, A4 M4 h4 o4 4, A5 M5 h5 o5 5,
    A6 M6 h6 o6 6, A7 M7 h7 o7 7
);
handler!(
    A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3, A4 M4 h4 o4 4, A5 M5 h5 o5 5,
    A6 M6 h6 o6 6, A7 M7 h7 o7 7, A8 M8 h8 o8 8
);
handler!(
    A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3, A4 M4 h4 o4 4, A5 M5 h5 o5 5,
    A6 M6 h6 o6 6, A7 M7 h7 o7 7, A8 M8 h8 o8 8, A9 M9 h9 o9 9
);
handler!(
    A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3, A4 M4 h4 o4 4, A5 M5 h5 o5 5,
    A6 M6 h6 o6 6, A7 M7 h7 o7 7, A8 M8 h8 o8 8, A9 M9 h9 o9 9, A10 M10 h10 o10 10
);
handler!(
    A0 M0 h0 o0 0, A1 M1 h1 o1 1, A2 M2 h2 o2 2, A3 M3 h3 o3 3, A4 M4 h4 o4 4, A5 M5 h5 o5 5,
    A6 M6 h6 o6 6, A7 M7 h7 o7 7, A8 M8 h8 o8 8, A9 M9 h9 o9 9, A10 M10 h10 o10 10,
    A11 M11 h11 o11 11
);

/// Compile-time check emitted by `#[orochi::module]` for each parameter, so
/// that a type without a conversion is reported on that parameter.
#[doc(hidden)]
pub fn assert_arg<T: Arg<M>, M>() {}

/// Compile-time check emitted by `#[orochi::module]` for each return type.
#[doc(hidden)]
pub fn assert_ret<T: Ret<M>, M>() {}

/// Compile-time check emitted by `#[orochi::module]` for the return type
/// of a constructor of class `T`.
#[doc(hidden)]
pub fn assert_new<T, R: IntoInstance<T>>() {}