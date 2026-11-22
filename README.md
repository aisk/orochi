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
        pub fn scale(&mut self, k: f64) {           // modifies the instance
            self.x *= k;
            self.y *= k;
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
p.scale(0.5)                 # p is now (3, 2)
p == mylib.Point(3, 2)       # True (`PartialEq`)
repr(p)                      # 'Point { x: 3.0, y: 2.0, cache: None }' (`Debug`)
str(p)                       # '(3, 2)' (`Display`)
mylib.geometry.area(1.0)
help(mylib.Point.dist)       # dist(self, /, other)
```

The items stay ordinary Rust: `mylib::add(1, None)` and
`mylib::Point::new(1.0, 2.0)` work as before.

## The rules

**`pub` means exported.** Plain `pub` functions, structs, constants, statics
and inline `pub mod`s are exported; private and `pub(crate)` items are not.
Doc comments become `__doc__`. In a struct's `impl`, `pub fn new` is the
constructor, `pub fn`s with `self` are methods, those without are static
methods and `pub const`s are class attributes. `Debug`, `Display`,
`PartialEq` and `Hash` (derived or implemented) become `__repr__`,
`__str__`, `__eq__` and `__hash__`.

Everything else follows from three rules:

1. **Rust types are values.** `i64`, `String`, `Vec<T>`, `HashMap<K, V>`,
   `PathBuf`, tuples, ... are converted (copied) on the way in and on the way
   out. `&str`, `&[T]`, `&Path`, `&HashMap<K, V>` and `Option<&str>`
   parameters borrow that converted copy: nothing the function does is
   visible to the caller.
2. **orochi types are references.** `Object`, `List<T>`, `Dict<K, V>`,
   `Set<T>` and `Ref<T>` refer to the caller's Python object itself: take a
   `List<T>` to modify the caller's list, or to keep an object's identity.
3. **Your own classes are borrowed.** `&self`, `&mut self`, `&Point`,
   `&mut Point` and `Option<&Point>` borrow the instance itself; a
   conflicting borrow (`p.absorb(p)` with `&mut self` and `&Point`) raises
   `RuntimeError`. By value (`p: Point`) is a copy, so the class must be
   `Clone`; `Ref<Point>` is a handle.

`&mut` of anything but a class is a compile error: take `List<T>`,
`Dict<K, V>` or `Set<T>` to modify the caller's container; other Python
values are immutable, so return the new value. Trailing `Option` parameters
default to `None`. Conversions are strict: a `float` is not an `int`, a
`bool` is not a number, a `str` is not a list, and a wrong argument raises
`TypeError` naming the parameter.

**Fields.** A `pub` field is a read/write property if its type is a simple
value (numbers, `bool`, `char`, `String`, `PathBuf`) or an orochi reference
(`Object`, `List<T>`, `Dict<K, V>`, `Set<T>`, `Ref<T>`), or an `Option` of
those. Any other type is a compile error: a `pub items: Vec<i64>` would
hand Python a copy that silently ignores `b.items.append(3)`. Use
`List<i64>` for a list Python can modify, or keep the field private and
write methods.

**Return values.** Any convertible value (numbers, `String`, `Vec<T>`,
maps, `Object`, `List<T>`, `Ref<T>`, ...), `()` (`None`), and `Option` /
`Result` of those; a class by value (a new instance) as `Point`,
`Option<Point>` or `Result<Point, E>`. `Result<T, E>` needs
`E: Into<orochi::Error>` and raises on `Err` (`std::io::Error` maps to the
matching `OSError` subclass). Borrowed values cannot be returned: return
`String`, not `&str`; a method that modifies `self` returns `()`. A panic
raises `PanicException` instead of aborting.

## What the macro generates

`#[orochi::module]` only reads names, signatures and doc comments. It adds
pyo3's `#[pyclass]` to each `pub struct`, a `#[new]` calling
`orochi::construct` for `pub fn new` (so Python cannot create an instance
without it), and plain calls to `orochi::ModuleBuilder` /
`orochi::ClassBuilder`, roughly:

```rust,ignore
#[pymethods]
impl Point {
    #[new]
    #[pyo3(signature = (*args, **kwargs), text_signature = None)]
    fn __orochi_new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        orochi::construct(args, kwargs)
    }
}

pub fn __orochi_init(__orochi_m: &orochi::ModuleBuilder<'_>) -> orochi::Result<()> {
    __orochi_m.doc("My first extension module.")?;
    let __orochi_c = __orochi_m.class::<Point>()?;
    __orochi_c.constructor(&["x", "y"], "", Point::new)?;
    __orochi_c.field("x", "", |s: &Point| &s.x, |s: &mut Point| &mut s.x)?;
    __orochi_c.method("norm", &[], "", Point::norm)?;
    __orochi_c.method("dist", &["other"], "", Point::dist)?;
    __orochi_c.static_method("origin", &[], "", Point::origin)?;
    __orochi_c.repr_debug()?;
    __orochi_c.str_display()?;
    __orochi_c.eq()?;
    __orochi_m.function("add", &["a", "b"], "Add two integers.", self::add)?;
    __orochi_m.submodule("geometry", "", self::geometry::__orochi_init)?;
    Ok(())
}
```

plus a `#[pymodule]` entry point calling it. The same builder works by hand
with a `#[pyclass]` of your own; the `ModuleBuilder` documentation shows a
complete hand-written module. `Debug`, `Display`, `PartialEq` and `Hash` are
recognized by name, in `#[derive(...)]` or in `impl ... for` the class.

## Not supported (yet)

Compile errors on `pub` items (make the item private to keep it Rust-only):
enums, generic or `async` functions, more than 12 parameters,
`impl Trait`, explicit lifetimes (`'_` is fine), returning borrowed values,
hand-written special methods (`__len__`, ...), `pub` fields of other types
than above, `pub use`, `pub trait`, an `impl` with `pub` items for a struct
of another module, Python keywords as names (`r#in`), names starting with
`__orochi`.

Classes by value work alone, in `Option` or in `Result` as return values,
and alone as parameters; not as `Vec<Point>`, `(Point, Point)` or
`Result<Option<Point>, E>` returns, nor as `Option<Point>` or `Vec<Point>`
parameters: use `Ref<Point>` there (`Vec<Ref<Point>>`, `Option<Ref<Point>>`).

Not seen by the macro: items produced by a `macro_rules!` macro inside the
module, and derives behind `#[cfg_attr(..., derive(...))]`. Also missing:
`.pyi` type stubs, subclassing, keyword-only and `*args` parameters,
defaults other than `None`, buffers.

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
must match the module name. `examples/demo` is a complete example:
`python examples/demo/run.py` builds it this way and imports it with Python.

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

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
python examples/demo/run.py   # builds the demo extension and imports it with Python
```

The test binaries embed Python, so the Python shared library must be findable
at run time:

- Windows: put the directory containing `python3XX.dll` on `PATH`. The
  Microsoft Store `python.exe` (under `WindowsApps`) is only a shim; find the
  real directory with `python -c "import sys; print(sys.base_prefix)"`.
- Linux: add `python3 -c "import sysconfig; print(sysconfig.get_config_var('LIBDIR'))"`
  to `LD_LIBRARY_PATH` if `libpython3.X.so` is not in a system directory.
- macOS: likewise with `DYLD_LIBRARY_PATH`, if needed.

`PYO3_PYTHON` can be set to select the interpreter used at build time;
`run.py` sets it to the Python that runs it.

## License

BSD 3-Clause; see [LICENSE](LICENSE).
