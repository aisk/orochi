//! Call arguments: [`IntoArgs`] / [`Args`] for positional and [`IntoKwargs`]
//! / [`Kwargs`] for keyword arguments.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::convert::IntoObject;
use crate::error::{Error, Result};
use crate::handles::{Dict, List, Set};
use crate::object::Object;

/// Positional call arguments.
///
/// - `()` means no arguments.
/// - A tuple `(a, b, ...)` (up to 12 elements) is an argument list; one
///   argument in tuple form needs a trailing comma: `(x,)`.
/// - A single plain value is one argument: numbers, `bool`, `char`, strings,
///   `Object` / `&Object`, the handles (`List`, `Dict`, `Set`, and references
///   to them) and `Option<T>` of those.
/// - Collections (`Vec`, `HashMap`, slices, ...) are *not* accepted bare, to
///   avoid ambiguity; write `(vec,)`.
/// - [`Args`] holds an argument list whose length is only known at runtime.
///
/// ```
/// let len = orochi::builtins().attr("len")?;
/// assert_eq!(len.call("abc")?.to::<usize>()?, 3);
/// assert_eq!(len.call((vec![1, 2],))?.to::<usize>()?, 2);
///
/// let max = orochi::builtins().attr("max")?;
/// assert_eq!(max.call((1, 5, 3))?.to::<i64>()?, 5);
/// let args: orochi::Args = (1..=4).collect();
/// assert_eq!(max.call(args)?.to::<i64>()?, 4);
/// # Ok::<(), orochi::Error>(())
/// ```
pub trait IntoArgs {
    /// Converts every argument to a Python object.
    fn into_args(self) -> Result<Vec<Object>>;
}

impl IntoArgs for () {
    fn into_args(self) -> Result<Vec<Object>> {
        Ok(Vec::new())
    }
}

macro_rules! args_impls {
    ($($name:ident $idx:tt),+) => {
        impl<$($name: IntoObject),+> IntoArgs for ($($name,)+) {
            fn into_args(self) -> Result<Vec<Object>> {
                Ok(vec![$(self.$idx.into_object()?),+])
            }
        }
    };
}

args_impls!(A 0);
args_impls!(A 0, B 1);
args_impls!(A 0, B 1, C 2);
args_impls!(A 0, B 1, C 2, D 3);
args_impls!(A 0, B 1, C 2, D 3, E 4);
args_impls!(A 0, B 1, C 2, D 3, E 4, F 5);
args_impls!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
args_impls!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
args_impls!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8);
args_impls!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9);
args_impls!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10);
args_impls!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11);

/// A single value as the only argument.
macro_rules! single_arg_impls {
    ($($t:ty),* $(,)?) => {$(
        impl IntoArgs for $t {
            fn into_args(self) -> Result<Vec<Object>> {
                Ok(vec![self.into_object()?])
            }
        }
    )*};
}

single_arg_impls!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char,
    &str, String, &String, Cow<'_, str>, Object, &Object,
);

impl<T: IntoObject> IntoArgs for Option<T> {
    fn into_args(self) -> Result<Vec<Object>> {
        Ok(vec![self.into_object()?])
    }
}

macro_rules! handle_arg_impls {
    ($($handle:ident <$($p:ident),+>),*) => {$(
        impl<$($p: IntoObject),+> IntoArgs for $handle<$($p),+> {
            fn into_args(self) -> Result<Vec<Object>> {
                Ok(vec![self.into_object()?])
            }
        }
        impl<$($p: IntoObject),+> IntoArgs for &$handle<$($p),+> {
            fn into_args(self) -> Result<Vec<Object>> {
                Ok(vec![self.into_object()?])
            }
        }
    )*};
}

handle_arg_impls!(List<T>, Dict<K, V>, Set<T>);

/// A positional argument list built at runtime.
///
/// Values are converted when added; a conversion error is kept and reported
/// by the call that uses the `Args`.
///
/// ```
/// use orochi::Args;
///
/// let mut args = Args::new().arg("a").arg(1);
/// args.push(vec![2.5]);
/// let f = orochi::eval("lambda *a: a")?;
/// assert_eq!(f.call(args)?.to_string(), "('a', 1, [2.5])");
/// # Ok::<(), orochi::Error>(())
/// ```
#[derive(Default)]
pub struct Args {
    items: Vec<Object>,
    error: Option<Error>,
}

impl Args {
    /// Creates an empty argument list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an argument (builder style).
    pub fn arg(mut self, value: impl IntoObject) -> Self {
        self.push(value);
        self
    }

    /// Appends an argument.
    pub fn push(&mut self, value: impl IntoObject) {
        match value.into_object() {
            Ok(obj) => self.items.push(obj),
            Err(err) => {
                self.error.get_or_insert(err);
            }
        }
    }

    /// Number of arguments.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` if there are no arguments.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T: IntoObject> FromIterator<T> for Args {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut args = Args::new();
        iter.into_iter().for_each(|v| args.push(v));
        args
    }
}

impl<T: IntoObject> Extend<T> for Args {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        iter.into_iter().for_each(|v| self.push(v));
    }
}

impl IntoArgs for Args {
    fn into_args(self) -> Result<Vec<Object>> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(self.items),
        }
    }
}

/// Keyword call arguments.
///
/// Implemented for [`Kwargs`] (what [`kwargs!`](crate::kwargs) builds),
/// `HashMap<String, V>` / `HashMap<&str, V>` and [`Dict<String, V>`].
/// Passing the same keyword twice raises `TypeError` at call time.
pub trait IntoKwargs {
    /// Converts every value to a Python object.
    fn into_kwargs(self) -> Result<Vec<(String, Object)>>;
}

/// An ordered list of keyword arguments.
///
/// Usually built with [`kwargs!`](crate::kwargs). Values are converted when
/// added; a conversion error is kept and reported by the call that uses the
/// `Kwargs`.
///
/// ```
/// use orochi::Kwargs;
///
/// let kw = Kwargs::new().arg("sep", "-").arg("end", "");
/// let print = orochi::builtins().attr("print")?;
/// print.call_kw(("a", "b"), kw)?; // prints "a-b"
/// # Ok::<(), orochi::Error>(())
/// ```
#[derive(Default)]
pub struct Kwargs {
    items: Vec<(String, Object)>,
    error: Option<Error>,
}

impl Kwargs {
    /// Creates an empty argument list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a keyword argument (builder style).
    pub fn arg(mut self, key: impl Into<String>, value: impl IntoObject) -> Self {
        self.push(key, value);
        self
    }

    /// Adds a keyword argument.
    pub fn push(&mut self, key: impl Into<String>, value: impl IntoObject) {
        match value.into_object() {
            Ok(obj) => self.items.push((key.into(), obj)),
            Err(err) => {
                self.error.get_or_insert(err);
            }
        }
    }

    /// Number of arguments.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` if there are no arguments.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl IntoKwargs for Kwargs {
    fn into_kwargs(self) -> Result<Vec<(String, Object)>> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(self.items),
        }
    }
}

impl<V: IntoObject, S> IntoKwargs for HashMap<String, V, S> {
    fn into_kwargs(self) -> Result<Vec<(String, Object)>> {
        self.into_iter()
            .map(|(k, v)| Ok((k, v.into_object()?)))
            .collect()
    }
}

impl<V: IntoObject, S> IntoKwargs for HashMap<&str, V, S> {
    fn into_kwargs(self) -> Result<Vec<(String, Object)>> {
        self.into_iter()
            .map(|(k, v)| Ok((k.to_owned(), v.into_object()?)))
            .collect()
    }
}

impl<V> IntoKwargs for Dict<String, V> {
    fn into_kwargs(self) -> Result<Vec<(String, Object)>> {
        // Snapshot with `list(d.items())` before converting anything.
        let items = self.as_object().method("items", ())?;
        crate::builtins().attr("list")?.call(&items)?.to::<Vec<(String, Object)>>()
    }
}

impl<V> IntoKwargs for &Dict<String, V> {
    fn into_kwargs(self) -> Result<Vec<(String, Object)>> {
        self.clone().into_kwargs()
    }
}
