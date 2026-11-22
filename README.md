# orochi

An ease-of-use-first layer over [pyo3](https://pyo3.rs) for writing Python
extension modules in Rust, in the spirit of pybind11 / nanobind. No
`Python<'py>`, no `Bound<'py, T>`, no pyo3 attributes: a Rust `mod` is the
Python module, `pub` means exported, and plain Rust signatures say how values
cross the boundary. A simple API is the goal; performance is not.

```rust
#[orochi::module]
mod mylib {
    //! My first extension module.
    use orochi::prelude::*;

    /// Add two integers.
    pub fn add(a: i64, b: Option<i64>) -> i64 {
        a + b.unwrap_or(1)
    }

    /// Append `v` to the caller's list, twice.
    pub fn push_twice(xs: List<i64>, v: i64) -> Result<()> {
        xs.push(v)?;
        xs.push(v)
    }

    fn helper() {} // private: not exported

    /// A 2D point.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Point {
        pub x: f64,
        pub y: f64,
    }

    impl Point {
        pub fn new(x: f64, y: f64) -> Self { // the constructor
            Point { x, y }
        }
        pub fn norm(&self) -> f64 {            // a method
            self.x.hypot(self.y)
        }
        pub fn scale(&mut self, k: f64) {      // modifies the instance
            self.x *= k;
            self.y *= k;
        }
    }
}
```

```python
import mylib

mylib.add(1)               # 2: a trailing `Option` parameter defaults to None
mylib.add(a=1, b=2)        # 3: parameters keep their Rust names
mylib.add(1.5)             # TypeError: argument 'a': expected int, got float

xs = [1]
mylib.push_twice(xs, 5)    # xs == [1, 5, 5]: a `List<i64>` is the caller's list

p = mylib.Point(3, 4)
p.norm()                   # 5.0
p.x = 6                    # `pub` fields are properties
p.scale(0.5)
p == mylib.Point(3, 2)     # True (`PartialEq`)
repr(p)                    # 'Point { x: 3.0, y: 2.0 }' (`Debug`)
```

The items stay ordinary Rust: `mylib::add(1, None)` still works.

## The rules

`pub` items are exported: functions, structs (classes), constants and inline
submodules; in a struct's `impl`, `pub fn new` is the constructor and the
other `pub fn`s are methods. How a value crosses the boundary follows from
three rules:

1. **Rust types are values.** `i64`, `String`, `Vec<T>`, `HashMap<K, V>`,
   `PathBuf`, ... are converted (copied) on the way in and out, strictly:
   a `float` is not an `int`, a `bool` is not a number.
2. **orochi types are references.** `Object`, `List<T>`, `Dict<K, V>`,
   `Set<T>` and `Ref<T>` are the caller's Python object itself.
3. **Your classes are borrowed.** `&self`, `&mut self`, `&Point` and
   `&mut Point` borrow the Python instance itself.

**Everything else is in the [reference](docs/reference.md)**: all parameter
and return types, classes and special methods, errors, the conversion table,
calling Python from Rust, what the macro generates, writing a module by hand,
and what is not supported yet.

## Building an extension module

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
orochi = "..."
```

Build it with [maturin](https://www.maturin.rs) (`maturin develop`), or by
hand: `cargo build` (with `PYO3_BUILD_EXTENSION_MODULE=1` on Linux/macOS, as
maturin does), then copy the library to a directory on Python's path as
`mylib.pyd` (Windows) or `mylib.so` (Linux/macOS). The file name must match
the module name. [`examples/demo`](examples/demo) is a complete example:
`python examples/demo/run.py` builds it this way and imports it with Python.

## Calling Python from Rust

The same types work the other way round, inside an extension or in a Rust
program that embeds Python (enable the `embed` feature for that:
`orochi = { version = "...", features = ["embed"] }`):

```rust
use orochi::prelude::*;

fn main() -> orochi::Result<()> {
    let json = orochi::import("json")?;
    let data = json.method("loads", r#"{"xs": [1, 2, 3]}"#)?;
    let xs: Vec<i64> = data.item("xs")?.to()?;   // a copy
    let live: List<i64> = data.item("xs")?.to()?; // the list itself
    live.push(4)?;
    assert_eq!(xs, [1, 2, 3]);
    assert_eq!(json.method("dumps", &data)?.to::<String>()?, r#"{"xs": [1, 2, 3, 4]}"#);
    Ok(())
}
```

See [Calling Python from Rust](docs/reference.md#calling-python-from-rust) in
the reference.

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
