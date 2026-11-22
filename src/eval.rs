//! Free functions for reaching Python: [`import`], [`builtins`], [`eval`],
//! [`exec`], and the [`Scope`] namespace they run in.

use std::ffi::CString;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::convert::IntoObject;
use crate::error::{Error, Result};
use crate::handles::Dict;
use crate::object::Object;
use crate::runtime::attach;

/// Imports a module (Python's `importlib.import_module(name)`).
///
/// ```
/// let os = orochi::import("os.path")?;
/// let joined: String = os.method("join", ("a", "b"))?.to()?; // two arguments
/// assert!(joined == "a/b" || joined == "a\\b");
/// # Ok::<(), orochi::Error>(())
/// ```
pub fn import(name: &str) -> Result<Object> {
    attach(|py| Ok(Object::from_bound(py.import(name)?.into_any())))
}

/// The `builtins` module (`len`, `int`, `print`, ...).
///
/// ```
/// let len = orochi::builtins().attr("len")?;
/// assert_eq!(len.call("abc")?.to::<usize>()?, 3);
/// # Ok::<(), orochi::Error>(())
/// ```
pub fn builtins() -> Object {
    import("builtins").expect("the builtins module is always importable")
}

/// Evaluates a Python expression in a fresh, empty [`Scope`].
///
/// ```
/// let v: Vec<i64> = orochi::eval("[i * i for i in range(4)]")?.to()?;
/// assert_eq!(v, [0, 1, 4, 9]);
/// # Ok::<(), orochi::Error>(())
/// ```
pub fn eval(expr: &str) -> Result<Object> {
    Scope::new().eval(expr)
}

/// Executes Python statements in a fresh [`Scope`] and returns it, so the
/// names they defined can be read back.
///
/// ```
/// let scope = orochi::exec("
/// def square(x):
///     return x * x
/// answer = square(6) + 6
/// ")?;
/// assert_eq!(scope.get("answer")?.to::<i64>()?, 42);
/// let square = scope.get("square")?;
/// assert_eq!(square.call(3)?.to::<i64>()?, 9);
/// # Ok::<(), orochi::Error>(())
/// ```
pub fn exec(code: &str) -> Result<Scope> {
    let scope = Scope::new();
    scope.exec(code)?;
    Ok(scope)
}

/// A Python global namespace (a `dict`) for running code with
/// [`Scope::eval`] / [`Scope::exec`].
///
/// Cloning a `Scope` yields another handle to the same namespace.
///
/// ```
/// use orochi::Scope;
///
/// let scope = Scope::new();
/// scope.set("base", 10)?;
/// scope.exec("result = base * 2")?;
/// assert_eq!(scope.get("result")?.to::<i64>()?, 20);
/// assert_eq!(scope.eval("result + 1")?.to::<i64>()?, 21);
/// # Ok::<(), orochi::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct Scope {
    globals: Dict<String, Object>,
}

impl Scope {
    /// Creates an empty namespace (Python adds `__builtins__` on first use).
    pub fn new() -> Self {
        Scope { globals: Dict::new() }
    }

    /// Evaluates an expression in this namespace.
    ///
    /// The source is dedented (like `textwrap.dedent`) and surrounding blank
    /// lines are removed, so indented multi-line literals work.
    pub fn eval(&self, expr: &str) -> Result<Object> {
        let code = to_cstring(dedent(expr)?.trim())?;
        attach(|py| {
            let globals = self.dict(py);
            Ok(Object::from_bound(py.eval(&code, Some(&globals), None)?))
        })
    }

    /// Executes statements in this namespace.
    ///
    /// The source is dedented (like `textwrap.dedent`), so code can be
    /// written as an indented Rust string literal:
    ///
    /// ```
    /// let scope = orochi::Scope::new();
    /// scope.exec(
    ///     r#"
    ///     def double(x):
    ///         return 2 * x
    ///     y = double(21)
    ///     "#,
    /// )?;
    /// assert_eq!(scope.get("y")?.to::<i64>()?, 42);
    /// # Ok::<(), orochi::Error>(())
    /// ```
    pub fn exec(&self, code: &str) -> Result<()> {
        let code = to_cstring(&dedent(code)?)?;
        attach(|py| {
            let globals = self.dict(py);
            Ok(py.run(&code, Some(&globals), None)?)
        })
    }

    /// Reads a global name; raises `NameError` if it is not defined.
    pub fn get(&self, name: &str) -> Result<Object> {
        attach(|py| match self.dict(py).get_item(name)? {
            Some(v) => Ok(Object::from_bound(v)),
            None => Err(Error::new("NameError", format!("name '{name}' is not defined"))),
        })
    }

    /// Defines or replaces a global name.
    pub fn set(&self, name: &str, value: impl IntoObject) -> Result<()> {
        self.globals.insert(name, value.into_object()?)
    }

    /// Returns `true` if the global name is defined.
    pub fn contains(&self, name: &str) -> Result<bool> {
        self.globals.contains(name)
    }

    /// The underlying globals `dict`.
    pub fn globals(&self) -> &Dict<String, Object> {
        &self.globals
    }

    fn dict<'py>(&self, py: Python<'py>) -> Bound<'py, PyDict> {
        self.globals
            .as_object()
            .bind(py)
            .clone()
            .cast_into::<PyDict>()
            .expect("Scope holds a dict")
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

/// Python's `textwrap.dedent`.
fn dedent(code: &str) -> Result<String> {
    attach(|py| {
        let text = py.import("textwrap")?.call_method1("dedent", (code,))?;
        Ok(text.extract::<String>()?)
    })
}

fn to_cstring(code: &str) -> Result<CString> {
    CString::new(code).map_err(|_| Error::value_error("source code must not contain NUL bytes"))
}
