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
    use std::collections::HashMap;
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

    pub fn push_twice(xs: &mut Vec<i64>, v: i64) {
        xs.push(v);
        xs.push(v);
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

    #[derive(Default)]
    pub struct Basket {
        pub items: Vec<String>,                     // reads as a live view of the Vec
        pub table: HashMap<String, Vec<i64>>,
        pub label: String,                          // reads as a copy
    }

    impl Basket {
        pub fn new() -> Self {
            Basket::default()
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
mylib.push_twice(xs, 5)      # xs == [1, 5, 5]: `&mut Vec` changes the caller's list

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

**Parameters mean what they mean in Rust.**

| Parameter | What the function gets |
|-----------|------------------------|
| `T` (`i64`, `String`, `Vec<T>`, `HashMap<K, V>`, `PathBuf`, a class) | its own value, converted from the argument. Nothing it does is visible to the caller. A class taken by value is a copy and must be `Clone`. |
| `&T` (`&str`, `&[T]`, `&Path`, `&HashMap<K, V>`, `&Point`, ...) | read-only access for the duration of the call |
| `&mut T` (`&mut Vec<T>`, `&mut HashMap`/`BTreeMap`, `&mut HashSet`/`BTreeSet`, `&mut Point`) | changes are visible to the caller: a class instance is borrowed in place; a container is copied in and written back into the *same* `list`/`dict`/`set` after the call (also when the function returns `Err`). Passing the same container to two `&mut` parameters raises `RuntimeError`, like a class instance borrowed twice. |
| `Option<T>` | `None` or a `T`; trailing `Option` parameters default to `None` |

`&mut String`, `&mut i64` etc. are compile errors: Python `str` and `int` are
immutable, so return the new value instead.

Write-back changes only what changed: a list is left alone when its contents
are still equal (`==`), and otherwise replaced as a whole (`xs[:] = new`); a
dict or set gets the removed keys deleted and the new or changed entries
assigned, so untouched entries keep their objects and a dict keeps its order.
New keys are appended in the Rust map's iteration order, which is sorted for
`BTreeMap` and arbitrary for `HashMap`. Caveats: changed elements are
converted both ways, so after a call that changes a `&mut Vec<Point>` the list
holds new `Point` objects (use `Vec<Ref<Point>>` or `List<..>` to keep their
identity), and changes made to the same container by other Python code during
the call are overwritten.

Conversions are strict: a `float` is not an `int`, a `bool` is not a number,
a `str` is not a list (see `orochi::IntoObject` / `orochi::FromObject`).
Wrong arguments raise `TypeError`.

**Return values** become new Python objects. A method returning `&Self` or
`&mut Self` returns the same object (so calls can be chained); if it returns a
reference to a *different* instance, that is copied when the class is `Clone`
and raises `RuntimeError` otherwise. Other references (`&str`, `&[T]`, ...)
are copied. `Result<T, E>` raises when it is
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

Reading a field gives a copy of its value, except that fields of container
type (`Vec<T>`, `HashMap`/`BTreeMap`, `HashSet`/`BTreeSet`, also inside
`Option`) read as *live views* that change the struct:

```python
b = mylib.Basket()
b.items.append("apple")          # changes the Rust `Vec`
b.table.setdefault("k", []).append(1)   # nested containers are live too
b.items.append(1)                # TypeError at once; the field is unchanged
b.items = ["pear"]               # assigning the whole field works as usual
plain = list(b.items)            # a real list, e.g. for json.dumps
```

A view supports what the builtin does (indexing and slices, `len`, `in`,
`==`, `repr`, the usual methods and operators) and is accepted wherever the
container is (`mylib.push_twice(b.items, 7)` updates the struct), but it is
not a `list`/`dict`/`set` instance: `list(view)`, `dict(view)`, `set(view)` or
`view.copy()` give a builtin copy (`copy()` also turns nested views into plain
containers). Each operation copies the field out and, if it changes it, back
in, so it is slow for large containers. A nested view addresses its position
(index or key) in its parent, so after the parent is restructured it follows
whatever is now at that position. Other fields (numbers, strings, tuples,
`PathBuf`, fixed-size arrays, ...) are plain copies: `b.label += "x"` works,
but mutating a copy, e.g. of an array field, does not change the struct.

A fieldless `pub enum` is a class whose members are attributes: they are
singletons (`mylib.favorite() is mylib.Color.Red`), compare and hash by
member whatever the enum derives, and only members are accepted as arguments.
The class is not iterable or callable (`list(Color)` and `Color(1)` raise
`TypeError`).

**Sharing Python objects.** For a value that must outlive the call or keep
its identity, use the handle types: `Object`, `List<T>`, `Dict<K, V>`,
`Set<T>`, and `Ref<T>` for class instances. A field holding another class must
be a `Ref` (`pub center: Ref<Point>`), so that `c.center is p` and
`c.center.x = 5` changes `p`; `pub center: Point` is a compile error, and so is
a class held by value inside a container (`pub points: Vec<Point>`; use
`Vec<Ref<Point>>`).

Unsupported items (generic or `async` functions, `impl Trait` parameters,
explicit lifetimes, enums with data, `pub use`, `pub trait`, Python keywords
such as `r#in` as names, ...) are compile errors when `pub`; make them private
to keep them Rust-only. Items produced by a `macro_rules!` macro inside the
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
```

`PYO3_PYTHON` can be set to select the interpreter used at build time.
