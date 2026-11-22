# orochi

An ease-of-use-first layer over [pyo3](https://pyo3.rs) for working with
Python from Rust, in the spirit of pybind11 / nanobind. A simple API is the
goal; performance is not.

No `Python<'py>`, no `Bound<'py, T>`, no lifetimes: `Object`, `List<T>`,
`Dict<K, V>` and `Set<T>` are `Clone + Send + Sync + 'static` handles that
attach to the interpreter by themselves.

The example below is a standalone program embedding Python, so it needs the
`embed` feature (`orochi = { version = "...", features = ["embed"] }`):

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
cargo test
cargo clippy --all-targets
```

`PYO3_PYTHON` can be set to select the interpreter used at build time.
