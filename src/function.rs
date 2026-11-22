//! `orochi.function`, the Python object behind every exported function,
//! method, static method and property accessor, and the binder that maps
//! `(args, kwargs)` onto a Rust function's parameters.
//!
//! A `function` holds a Rust closure. Like a Python function it is a
//! descriptor: stored on a class, `obj.method` binds it to the instance
//! (a `types.MethodType`). Its `__name__`, `__qualname__`, `__module__`,
//! `__doc__` and `__text_signature__` live in its `__dict__`, which is what
//! `inspect.signature` and `help()` read.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyString, PyTuple, PyType};

use crate::handler::{BoundArgs, MAX_ARITY};

type Call = dyn Fn(&Bound<'_, PyTuple>, Option<&Bound<'_, PyDict>>) -> PyResult<Py<PyAny>>
    + Send
    + Sync;

#[pyclass(module = "orochi", name = "function", frozen, dict)]
pub(crate) struct Function {
    qualname: String,
    call: Box<Call>,
}

#[pymethods]
impl Function {
    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(
        &self,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        (self.call)(args, kwargs)
    }

    /// Binds to an instance like a Python function does.
    fn __get__(
        slf: &Bound<'_, Self>,
        obj: Option<&Bound<'_, PyAny>>,
        _owner: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        match obj {
            Some(obj) if !obj.is_none() => Ok(method_type(slf.py())?.call1((slf, obj))?.unbind()),
            _ => Ok(slf.clone().into_any().unbind()),
        }
    }

    fn __repr__(&self) -> String {
        format!("<function {}>", self.qualname)
    }

    /// Pickled by reference, like Python functions: `module.qualname`.
    fn __reduce__(&self) -> &str {
        &self.qualname
    }
}

/// `types.MethodType`, imported once. [`ClassBuilder`](crate::ClassBuilder)
/// imports it while the class is built: binding a method must not import,
/// since that fails while the interpreter shuts down (e.g. in a `__del__`).
pub(crate) fn method_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    static METHOD_TYPE: PyOnceLock<Py<PyType>> = PyOnceLock::new();
    METHOD_TYPE.import(py, "types", "MethodType")
}

/// Creates a `function` named `name` that binds its arguments with `sig`
/// and runs `call`.
pub(crate) fn new_function<'py, C>(
    py: Python<'py>,
    name: &str,
    module: &str,
    doc: &str,
    sig: Signature,
    call: C,
) -> PyResult<Bound<'py, PyAny>>
where
    C: for<'p> Fn(Python<'p>, &BoundArgs<'p>, &[String]) -> PyResult<Py<PyAny>> + Send + Sync + 'static,
{
    let text_signature = sig.text();
    let qualname = sig.qualname.clone();
    let function = Bound::new(
        py,
        Function {
            qualname: qualname.clone(),
            call: Box::new(move |args, kwargs| {
                let bound = sig.bind(args, kwargs)?;
                call(args.py(), &bound, &sig.names)
            }),
        },
    )?;
    function.setattr("__name__", name)?;
    function.setattr("__qualname__", qualname)?;
    function.setattr("__module__", module)?;
    function.setattr("__doc__", non_empty(doc))?;
    function.setattr("__text_signature__", text_signature)?;
    Ok(function.into_any())
}

/// `None` for an empty doc comment.
pub(crate) fn non_empty(doc: &str) -> Option<&str> {
    let doc = doc.trim();
    (!doc.is_empty()).then_some(doc)
}

/// The Python-visible signature of one function, method or constructor.
pub(crate) struct Signature {
    /// Used in error messages (`add()`, `Point.dist()`, `Point()`).
    pub(crate) qualname: String,
    /// The first parameter is the instance (`self`); counts and messages
    /// include it, as CPython's do.
    method: bool,
    /// The Rust parameter names, `self` first for methods.
    pub(crate) names: Vec<String>,
    /// The number of leading parameters without a default.
    required: usize,
    /// Keyword arguments are rejected (the names are made up).
    positional_only: bool,
}

impl Signature {
    /// `params` are the Python parameter names (without `self`), `optional`
    /// says for each Rust parameter whether it may be omitted. Fails on a
    /// count mismatch, too many parameters or a duplicate name.
    pub(crate) fn new(qualname: String, method: bool, params: &[&str], optional: &[bool]) -> PyResult<Self> {
        let first = usize::from(method);
        if params.len() + first != optional.len() {
            return Err(PyTypeError::new_err(format!(
                "{qualname}: {} parameter names given for a Rust function with {} parameters{}",
                params.len(),
                optional.len(),
                if method { " (including `self`)" } else { "" },
            )));
        }
        if optional.len() > MAX_ARITY {
            return Err(PyTypeError::new_err(format!("{qualname}: more than {MAX_ARITY} parameters")));
        }
        let mut names = Vec::with_capacity(optional.len());
        if method {
            names.push("self".to_owned());
        }
        for param in params {
            if names.iter().any(|n| n == param) {
                return Err(PyTypeError::new_err(format!("{qualname}: duplicate parameter name '{param}'")));
            }
            names.push((*param).to_owned());
        }
        // Only a trailing run of optional parameters gets a default (`None`).
        let required = optional.iter().rposition(|o| !o).map_or(0, |i| i + 1).max(first);
        Ok(Signature { qualname, method, names, required, positional_only: false })
    }

    /// A function whose parameters have no names (a Rust closure): they
    /// are called `arg1`, `arg2`, ... in messages and can only be passed
    /// positionally.
    pub(crate) fn positional_only(qualname: String, optional: &[bool]) -> PyResult<Self> {
        let names: Vec<String> = (1..=optional.len()).map(|i| format!("arg{i}")).collect();
        let params: Vec<&str> = names.iter().map(String::as_str).collect();
        Ok(Signature { positional_only: true, ..Signature::new(qualname, false, &params, optional)? })
    }


    /// `__text_signature__`: `(a, b=None)`, `($self, k)`, `(arg1, /)`.
    fn text(&self) -> String {
        let first = usize::from(self.method);
        let mut parts = Vec::with_capacity(self.names.len());
        if self.method {
            parts.push("$self".to_owned());
        }
        for (i, name) in self.names.iter().enumerate().skip(first) {
            parts.push(if i < self.required { name.clone() } else { format!("{name}=None") });
        }
        if self.positional_only && !parts.is_empty() {
            parts.push("/".to_owned());
        }
        format!("({})", parts.join(", "))
    }

    /// The same signature as an `inspect.Signature`, for a class's
    /// `__signature__` (a class's `__text_signature__` cannot be set).
    pub(crate) fn to_inspect<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inspect = py.import("inspect")?;
        let parameter = inspect.getattr("Parameter")?;
        let kind = parameter.getattr("POSITIONAL_OR_KEYWORD")?;
        let mut params = Vec::with_capacity(self.names.len());
        for (i, name) in self.names.iter().enumerate().skip(usize::from(self.method)) {
            let kwargs = PyDict::new(py);
            if i >= self.required {
                kwargs.set_item("default", py.None())?;
            }
            params.push(parameter.call((name, &kind), Some(&kwargs))?);
        }
        inspect.getattr("Signature")?.call1((params,))
    }

    /// Binds one call's arguments to the parameters, raising `TypeError`
    /// with CPython's wording when they do not fit.
    pub(crate) fn bind<'py>(
        &self,
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<BoundArgs<'py>> {
        if self.positional_only && kwargs.is_some_and(|kw| !kw.is_empty()) {
            return Err(PyTypeError::new_err(format!("{}() takes no keyword arguments", self.qualname)));
        }
        let given = args.len();
        let first = usize::from(self.method);
        if given == 0 && self.method {
            return Err(PyTypeError::new_err(format!("unbound method {}() needs an argument", self.qualname)));
        }
        if given > self.names.len() {
            return Err(self.too_many(given));
        }
        let mut out = BoundArgs::default();
        for (slot, arg) in out.iter_mut().zip(args.iter()) {
            *slot = Some(arg);
        }
        if let Some(kwargs) = kwargs {
            for (key, value) in kwargs.iter() {
                let Ok(key) = key.cast_into::<PyString>() else {
                    return Err(PyTypeError::new_err("keywords must be strings"));
                };
                let key = key.to_str()?;
                let index = self.names[first..].iter().position(|p| p == key).map(|i| i + first);
                match index {
                    Some(i) if out[i].is_some() => {
                        return Err(PyTypeError::new_err(format!(
                            "{}() got multiple values for argument '{key}'",
                            self.qualname
                        )));
                    }
                    Some(i) => out[i] = Some(value),
                    None => {
                        return Err(PyTypeError::new_err(format!(
                            "{}() got an unexpected keyword argument '{key}'",
                            self.qualname
                        )));
                    }
                }
            }
        }
        let missing: Vec<&str> =
            (first..self.required).filter(|&i| out[i].is_none()).map(|i| self.names[i].as_str()).collect();
        if !missing.is_empty() {
            return Err(self.missing(&missing));
        }
        Ok(out)
    }

    fn too_many(&self, given: usize) -> PyErr {
        let n = self.names.len();
        let takes = if self.required == n {
            format!("{n} positional argument{}", plural(n))
        } else {
            format!("from {} to {n} positional arguments", self.required)
        };
        let was = if given == 1 { "was" } else { "were" };
        PyTypeError::new_err(format!("{}() takes {takes} but {given} {was} given", self.qualname))
    }

    fn missing(&self, names: &[&str]) -> PyErr {
        let quoted: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
        let list = match quoted.as_slice() {
            [one] => one.clone(),
            [a, b] => format!("{a} and {b}"),
            [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
            [] => String::new(),
        };
        PyTypeError::new_err(format!(
            "{}() missing {} required positional argument{}: {list}",
            self.qualname,
            names.len(),
            plural(names.len())
        ))
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
