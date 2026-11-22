//! Wrapping Rust closures as Python callables ([`Object::from_fn`]).

use pyo3::prelude::*;
use pyo3::types::{PyCFunction, PyDict, PyTuple};

use crate::convert::{FromObject, IntoObject};
use crate::error::{Error, Result};
use crate::object::Object;
use crate::runtime::attach;

/// A Rust closure callable from Python.
///
/// Implemented for `Fn(A1, ..., An) -> R` (n <= 8) where every `Ai` is
/// [`FromObject`] and `R` is [`IntoObject`]. `Marker` only disambiguates the
/// implementations; it is inferred.
#[doc(hidden)]
pub trait IntoPyFunction<Marker>: Send + Sync + 'static {
    /// Number of positional parameters.
    const ARITY: usize;

    /// Calls the closure; `args.len()` equals `ARITY`.
    fn invoke(&self, args: &[Object]) -> Result<Object>;
}

macro_rules! fn_impls {
    ($len:expr; $($arg:ident $idx:tt),*) => {
        impl<F, R, $($arg),*> IntoPyFunction<(R, $($arg,)*)> for F
        where
            F: Fn($($arg),*) -> R + Send + Sync + 'static,
            R: IntoObject,
            $($arg: FromObject,)*
        {
            const ARITY: usize = $len;

            #[allow(unused_variables)]
            fn invoke(&self, args: &[Object]) -> Result<Object> {
                (self)($(convert_arg::<$arg>(args, $idx)?),*).into_object()
            }
        }
    };
}

fn_impls!(0;);
fn_impls!(1; A 0);
fn_impls!(2; A 0, B 1);
fn_impls!(3; A 0, B 1, C 2);
fn_impls!(4; A 0, B 1, C 2, D 3);
fn_impls!(5; A 0, B 1, C 2, D 3, E 4);
fn_impls!(6; A 0, B 1, C 2, D 3, E 4, G 5);
fn_impls!(7; A 0, B 1, C 2, D 3, E 4, G 5, H 6);
fn_impls!(8; A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7);

/// Converts argument `index`. On failure the original exception is kept
/// (type, traceback and cause intact) and a note naming the argument is
/// added to it.
fn convert_arg<T: FromObject>(args: &[Object], index: usize) -> Result<T> {
    T::from_object(&args[index]).map_err(|err| add_context(err, index))
}

fn add_context(err: Error, index: usize) -> Error {
    let note = format!("while converting argument {} of a Rust function", index + 1);
    attach(|py| {
        let value = err.as_pyerr().value(py);
        if value.hasattr("add_note").unwrap_or(false) {
            // Python 3.11+: `BaseException.add_note` (PEP 678).
            let _ = value.call_method1("add_note", (note,));
            err
        } else {
            // Python 3.9/3.10: raise a contextual exception of the same type
            // with the original as `__cause__`; if that type cannot be built
            // from one message, keep the original unchanged.
            let message = format!("{}: {}", note, err.message());
            match value.get_type().call1((message,)) {
                Ok(contextual) => {
                    let contextual = PyErr::from_value(contextual);
                    contextual.set_cause(py, Some(err.into_pyerr()));
                    contextual.into()
                }
                Err(_) => err,
            }
        }
    })
}

/// Builds the Python function object behind [`Object::from_fn`].
pub(crate) fn make_function<F, Marker>(f: F) -> Object
where
    F: IntoPyFunction<Marker>,
{
    let closure = move |args: &Bound<'_, PyTuple>,
                        kwargs: Option<&Bound<'_, PyDict>>|
          -> PyResult<Py<PyAny>> {
        if kwargs.is_some_and(|kw| !kw.is_empty()) {
            return Err(Error::type_error("this function takes no keyword arguments").into());
        }
        if args.len() != F::ARITY {
            return Err(Error::type_error(format!(
                "this function takes {} positional argument{} but {} {} given",
                F::ARITY,
                if F::ARITY == 1 { "" } else { "s" },
                args.len(),
                if args.len() == 1 { "was" } else { "were" },
            ))
            .into());
        }
        let args: Vec<Object> = args.iter().map(Object::from_bound).collect();
        Ok(f.invoke(&args)?.into_py())
    };
    attach(|py| {
        let func = PyCFunction::new_closure(py, Some(c"orochi_function"), None, closure)
            .expect("failed to create a Python function object");
        Object::from_bound(func.into_any())
    })
}
