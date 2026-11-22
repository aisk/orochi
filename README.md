# orochi

An ease-of-use-first layer over [pyo3](https://pyo3.rs) for writing Python
extension modules in Rust, in the spirit of pybind11 / nanobind. A simple API
is the goal; performance is not.

No `Python<'py>`, no `Bound<'py, T>`, no pyo3 attributes: a Rust `mod` is the
Python module, and `pub` means exported.

```rust
#[orochi::module]
mod mylib {
    //! My first extension module.
    use orochi::prelude::*;
    use std::path::PathBuf;

    /// Add two integers.
    pub fn add(a: i64, b: Option<i64>) -> i64 {
        a + b.unwrap_or(1)
    }

    pub fn count_lines(text: &str) -> usize {
        text.lines().count()
    }

    pub fn total(xs: &[f64]) -> f64 {
        xs.iter().sum()
    }

    pub fn push_twice(xs: List<i64>, v: i64) -> Result<()> {
        xs.push(v)?;                                // the caller's list itself
        xs.push(v)
    }

    pub fn read_text(path: PathBuf) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn helper() {} // private: not exported

    /// A 2D point.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Point {
        pub x: f64,
        pub y: f64,
        cache: Option<f64>, // private field: not visible from Python
    }

    impl Point {
        pub fn new(x: f64, y: f64) -> Self {      // the constructor
            Point { x, y, cache: None }
        }
        pub fn norm(&self) -> f64 {                 // a method
            self.x.hypot(self.y)
        }
        pub fn scale(&mut self, k: f64) -> &mut Self {
            self.x *= k;
            self.y *= k;
            self                                    // returns the same object
        }
        pub fn origin() -> Self {                   // no `self`: a static method
            Point::new(0.0, 0.0)
        }
        pub fn dist(&self, other: &Point) -> f64 {
            (self.x - other.x).hypot(self.y - other.y)
        }
    }

    impl std::fmt::Display for Point {              // becomes `str(p)`
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "({}, {})", self.x, self.y)
        }
    }

    pub enum Color {                                // members: Color.Red, Color.Green
        Red,
        Green,
    }

    pub fn favorite() -> Color {
        Color::Red
    }

    pub mod geometry {                              // the submodule `mylib.geometry`
        pub fn area(r: f64) -> f64 {
            std::f64::consts::PI * r * r
        }
    }
}
```

```python
import mylib

mylib.add(1)                 # 2
mylib.add(a=1, b=2)          # 3: real parameter names, `Option` parameters default to None
mylib.add(True)              # TypeError: argument 'a': expected int, got bool
mylib.read_text("missing")   # FileNotFoundError

xs = [1]
mylib.push_twice(xs, 5)      # xs == [1, 5, 5]: a `List<i64>` is the caller's list

p = mylib.Point(3, 4)
p.norm()                     # 5.0
p.x = 6
p.scale(2).scale(0.5) is p   # True
p == mylib.Point(6, 4)       # True (`PartialEq`)
repr(p)                      # 'Point { x: 6.0, y: 4.0, cache: None }' (`Debug`)
str(p)                       # '(6, 4)' (`Display`)
mylib.geometry.area(1.0)
```

The items stay ordinary Rust: `mylib::add(1, None)` and
`mylib::Point::new(1.0, 2.0)` work as before.

## The rules

**`pub` means exported.** Plain `pub` functions, structs, fieldless enums,
constants, statics and inline `pub mod`s are exported; private and
`pub(crate)` items are not. Doc comments become `__doc__`.

Everything else follows from three rules:

1. **Rust types are values.** `i64`, `String`, `Vec<T>`, `HashMap<K, V>`,
   `PathBuf`, tuples, ... are converted (copied) on the way in and on the way
   out. `&str`, `&[T]`, `&Path`, `&HashMap<K, V>`, `Option<&T>` parameters
   borrow that converted copy: nothing the function does is visible to the
   caller. A returned `Vec<T>` is a new `list`, a `HashMap` a new `dict` in
   arbitrary order (use `BTreeMap` for a sorted, stable order).
2. **orochi types are references.** `Object`, `List<T>`, `Dict<K, V>`,
   `Set<T>` and `Ref<T>` refer to the caller's Python object itself: take a
   `List<T>` to modify the caller's list, or to keep an object's identity.
3. **Your own classes** (`pub struct`s of the module): `&Point`,
   `&mut Point`, `&self` and `&mut self` borrow the instance itself (a
   conflicting borrow raises `RuntimeError`); by value is a copy (the class
   must be `Clone`); `Ref<Point>` is a handle. Any other Python type is an
   `Object`.

`&mut` of anything but a class is a compile error. To modify the caller's
list, dict or set, take `List<T>` / `Dict<K, V>` / `Set<T>`, which need no
`&mut`; other Python values (`str`, `int`, tuples, ...) are immutable, so
return the new value.
Trailing `Option` parameters default to `None`.

Fields follow the same rules:

```rust
# use orochi::List;
pub struct Basket {
    pub items: Vec<i64>,    // value: Python reads an immutable snapshot (a tuple); assign to replace
    pub tags: List<String>, // reference: a real Python list, mutable in place
}
```

```python
b.tags.append("x")   # changes the basket's list
b.items = [1, 2]     # replaces the value
b.items              # (1, 2)
b.items.append(3)    # AttributeError: 'tuple' object has no attribute 'append'
```

The `Vec` lives in the Rust struct, so Python can only be handed a snapshot
of it: `Vec<T>` and arrays read as a `tuple`, sets as a `frozenset`, maps as
a read-only `types.MappingProxyType`, also nested (a `Vec<u8>` reads as
`bytes`), so a mutation fails instead of changing a copy. A snapshot is not a
list: `b.items == [1, 2]` is `False`, compare with `(1, 2)`.
`list(b.items)`, `dict(b.table)` and `set(b.tags)` give mutable copies (also
what `json` and `pickle` need). References inside a value pass through
(`Vec<Ref<Point>>` reads as a tuple of the instances), and a type with a
hand-written `IntoObject` is handed over exactly as it builds it. A class
held by value in a field is a compile error (`pub center: Point`,
`pub points: Vec<Point>`): use `Ref<Point>` / `Vec<Ref<Point>>`, so that
`c.center is p`. Enum members are immutable singletons and fine by value.

Conversions are strict: a `float` is not an `int`, a `bool` is not a number,
a `str` is not a list (see `orochi::IntoObject` / `orochi::FromObject`).
Wrong arguments raise `TypeError`.

**Return values.** A method returning `&Self` or `&mut Self` returns the same
object (so calls can be chained); a reference to a *different* instance is
copied when the class is `Clone` and raises `RuntimeError` otherwise. Other
references (`&str`, `&[T]`, ...) are copied. `Result<T, E>` raises when it is
`Err`: `std::io::Error` maps to the matching `OSError` subclass,
`orochi::Error` to its own exception, and any other `E: Display` to
`RuntimeError`. A panic raises `PanicException` instead of aborting.

**Classes.** A `pub struct` is a class. `pub fn new` (without `self`) is the
constructor, other `pub fn`s with `self` are methods, those without are static
methods, `pub const`s in the `impl` are class attributes, and `pub` fields are
attributes. `Debug`, `Display`, `PartialEq` and `Hash` (derived or
implemented) become `__repr__`, `__str__`, `__eq__` and `__hash__`; a class
with `PartialEq` but no `Hash` is unhashable, like in Python, and `Hash`
without `PartialEq` is a compile error.

A fieldless `pub enum` is a class whose members are attributes: they are
singletons (`mylib.favorite() is mylib.Color.Red`), compare and hash by
member whatever the enum derives, and only members are accepted as arguments.
The class is not iterable or callable (`list(Color)` and `Color(1)` raise
`TypeError`). Rust returns the member objects that existed when the class was
created, even if `Color.Red` is later reassigned in Python; the stubs mark
members `Final`, so type checkers reject such an assignment, but the runtime
does not.

Unsupported items (generic or `async` functions, `impl Trait` parameters,
explicit lifetimes, enums with data, `pub use`, `pub trait`, Python keywords
such as `r#in` as names, names starting with `_orochi` or `__orochi`, which
orochi reserves, ...) are compile errors when `pub`; make them private to keep
them Rust-only. Items produced by a `macro_rules!` macro inside the
module are not exported: the attribute only sees the macro call, not the
items it expands to.

## Building an extension module

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
orochi = "..."
```

Build it with [maturin](https://www.maturin.rs) (`maturin develop`), or by
hand: `cargo build`, then copy the library to a directory on Python's path as
`mylib.pyd` (Windows) or `mylib.so` (Linux/macOS; set
`PYO3_BUILD_EXTENSION_MODULE=1` when building, as maturin does). The file name
must match the module name. `examples/demo` is a complete example; on Windows
`examples\demo\run.ps1` builds it and imports it with Python.

## Type stubs

Every `#[orochi::module]` can write `.pyi` stubs for itself, so that editors
and type checkers understand it like a typed Python library. With the built
module:

```sh
python -c "import mylib; mylib._orochi_stubs('typings')"
```

This writes `typings/mylib.pyi`, or a stub package (`typings/mylib/__init__.pyi`,
`typings/mylib/geometry.pyi`, ...) when the module has submodules, and removes
the stubs of the other layout that orochi generated earlier (files it did not
write are kept); `mylib._orochi_stubs()` without a directory only returns
`{relative path: text}`. Pyright and Pylance read `typings/` by default; for
mypy set `mypy_path = typings`. `_orochi_stubs` itself is hidden: it is not in
`__all__` nor in the stubs.

To ship the stubs with the module (PEP 561), either:

- with maturin and a module without submodules, put `mylib.pyi` beside
  `Cargo.toml`: maturin packages it as `mylib/__init__.pyi` with a `py.typed`
  marker. Stubs of submodules are not picked up this way.
- publish a separate stub-only package whose files are the stub package in a
  directory named `mylib-stubs/` (`mylib-stubs/__init__.pyi`,
  `mylib-stubs/geometry.pyi`, ...); type checkers prefer it to the module.

Both were checked with mypy 2.4.1 and pyright 1.1.407 against
`examples/demo` installed from a wheel built by `maturin build` (maturin
1.9.6), with `mylib-stubs/` copied into the environment's `site-packages`;
building and publishing the stub-only distribution itself was not tested.

The same from Rust, e.g. in a test of the extension crate (no interpreter is
started, although the test binary links Python like any other):

```rust,no_run
#[orochi::module]
mod mylib {
    pub fn add(a: i64, b: Option<i64>) -> i64 {
        a + b.unwrap_or(1)
    }
}

#[test]
fn write_stubs() {
    orochi::write_stubs(&mylib::__orochi_stubs(), "typings").unwrap();
}
```

Stubs include docstrings, parameter names and `None` defaults, and the
types the conversions really accept and return: `Vec<T>` parameters take any
`collections.abc.Sequence[T]`, `HashMap<K, V>` any
`collections.abc.Mapping[K, V]` (also a field's `MappingProxyType`);
`Result<T, E>` returns `T` and `&mut Self`
the class. A field is a property typed with what reading it returns
(`tuple[int, ...]` for a `Vec<i64>`) and a setter typed with what assigning
accepts. Classes are `@typing.final` with `__new__` from `new` (a class
without `new`, and an enum, gets a `__new__` that no call matches); enum
members are `Final` attributes of their class.

Stubs are looser than the runtime in places: an integer or string type only
says `int` / `str` (not the range of a `u8`, say), and a custom conversion is
only as precise as its hint. In one place they are stricter: a map whose keys
are paths (`HashMap<PathBuf, V>`) accepts `Mapping[str | os.PathLike[str], V]`
but reads back with `pathlib.Path` keys, and since `Mapping` is invariant in
its key type, a type checker rejects passing the read-back mapping in again
(`b.by_path = b.by_path`) although it works at runtime. A custom `FromObject` / `IntoObject` hint is used
as written: spell names fully qualified (`typing.Any`,
`numpy.typing.NDArray[numpy.float64]`) and the stub imports their modules.

## Embedding Python

orochi also works the other way round, calling Python from a Rust program.
That needs the `embed` feature
(`orochi = { version = "...", features = ["embed"] }`):

```rust
use orochi::prelude::*;

fn main() -> orochi::Result<()> {
    // Call into Python modules. A single argument can be passed bare;
    // several go in a tuple.
    let json = orochi::import("json")?;
    let data = json.method("loads", r#"{"xs": [1, 2, 3]}"#)?;
    let xs: Vec<i64> = data.item("xs")?.to()?;          // strict, copying conversion
    let text = json.method_kw("dumps", &data, kwargs! { "indent" => 2 })?;
    println!("{xs:?}\n{text}");

    // Live references: changes are visible on both sides.
    let scope = orochi::exec(
        r#"
        names = ['a']
        "#,
    )?;
    let names: List<String> = scope.get("names")?.to()?;
    names.push("b")?;
    assert_eq!(scope.eval("len(names)")?.to::<usize>()?, 2);

    // Rust closures as Python callables.
    scope.set("add", Object::from_fn(|a: i64, b: i64| a + b))?;
    assert_eq!(scope.eval("add(40, 2)")?.to::<i64>()?, 42);

    // Python exceptions are `orochi::Error`.
    let err = orochi::eval("1 / 0").unwrap_err();
    assert!(err.is_instance_of("ZeroDivisionError"));
    println!("{err}");   // ZeroDivisionError: division by zero
    println!("{err:#}"); // full traceback
    Ok(())
}
```

## Features

- `macros` (default): the `#[orochi::module]` attribute.
- `embed`: start the Python interpreter automatically on first use. Enable it
  for standalone Rust programs that embed Python; leave it off for extension
  modules (the interpreter is already running there).

## Caveat: locks

Do not hold a Rust lock while calling into Python if another thread may take
the same lock while attached to Python (for example inside a Rust function
called from Python): one thread waits for the GIL while holding the lock, the
other holds the GIL and waits for the lock, and both hang.

## Development

```powershell
# The test binaries embed Python, so python3XX.dll must be on PATH (Windows).
$env:PATH = "<python install dir>;$env:PATH"
cargo test --workspace
cargo clippy --workspace --all-targets
.\examples\demo\run.ps1   # builds the demo extension and imports it with Python
                          # (`py -3` or `python`; or pass -Python <python.exe>)
.\examples\demo\run.ps1 -TypeCheck   # also checks its stubs with mypy (needs uv):
                                     # stubtest, typecheck/good.py, typecheck/bad.py
```

`PYO3_PYTHON` can be set to select the interpreter used at build time.
