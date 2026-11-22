# orochi reference

The complete description of orochi: every rule, the supported types, and what
is not supported yet. The [README](../README.md) has a one-minute
introduction and explains how to build an extension module; the rustdoc
(`cargo doc --open`) documents each item of the API.

- [Exporting a module](#exporting-a-module)
- [Functions and parameters](#functions-and-parameters)
- [Classes](#classes)
- [Return values and errors](#return-values-and-errors)
- [Conversions](#conversions)
- [Reference handles](#reference-handles)
- [Calling Python from Rust](#calling-python-from-rust)
- [What the macro generates](#what-the-macro-generates)
- [Writing a module by hand](#writing-a-module-by-hand)
- [Testing a module in-process](#testing-a-module-in-process)
- [pyo3 interop](#pyo3-interop)
- [Features](#features)
- [Threads, locks and interpreter lifetime](#threads-locks-and-interpreter-lifetime)
- [Not supported (yet)](#not-supported-yet)

The Python examples are written as interactive sessions (`>>>`), run against
the Rust module shown just before them.

## Exporting a module

`#[orochi::module]` goes on an inline module (`mod name { ... }`) and turns
it into a Python extension module:

```rust
#[orochi::module]
mod geo {
    //! Geometry helpers.

    pub const ORIGIN: (f64, f64) = (0.0, 0.0);

    /// Distance between two points.
    pub fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
        (a.0 - b.0).hypot(a.1 - b.1)
    }

    fn helper() {} // private: Rust only

    pub mod units {
        //! Unit conversions.
        pub fn km(meters: f64) -> f64 {
            meters / 1000.0
        }
    }
}
```

```python
>>> import geo, geo.units
>>> geo.__doc__, geo.dist.__doc__
('Geometry helpers.', 'Distance between two points.')
>>> geo.dist(geo.ORIGIN, (3, 4))
5.0
>>> geo.units.km(1500)
1.5
>>> hasattr(geo, "helper")
False
```

**`pub` means exported.** Only items with plain `pub` visibility are seen by
Python; private, `pub(crate)`, `pub(super)` and `pub(in ...)` items stay
Rust-only. A private module (`mod x { ... }`) is not looked at at all, so
its `pub` items are plain Rust too.

| Rust item | Python |
|-----------|--------|
| `pub fn` | a function ([Functions and parameters](#functions-and-parameters)) |
| `pub struct` | a class ([Classes](#classes)) |
| `pub const`, `pub static` | a module attribute: a copy of the value (made with `Clone`), converted like a [return value](#return-values-and-errors) |
| inline `pub mod name { ... }` | the submodule `parent.name`, also registered in `sys.modules`, so `import parent.name` works |

Any other `pub` item (`pub enum`, `pub use`, `pub trait`, ...) is a compile
error rather than being silently left out; see
[Not supported (yet)](#not-supported-yet). Make it private to keep it
Rust-only.

**Names.** The Python name of each item is its Rust name, without the raw
identifier prefix (`pub fn r#type` is `type`). The module's name is the Rust
module's name; `#[orochi::module(name = "other")]` sets another (it must be
an identifier). A Python keyword (`in`, `class`, `None`, ...) cannot be the
name of an exported item or parameter, two exported items cannot share a
Python name in one namespace (a field and a method of the same name, say),
and names starting with `__orochi` are reserved for the generated code, for
every item of the module, private ones included.

**Docstrings.** Doc comments become `__doc__`: `///` on functions, structs,
methods and fields; `//!` inside the module (or `///` on it) for the module
and for submodules. `#[doc = ...]` attributes such as
`#![doc = include_str!("README.md")]` work too. A class's docstring is the
struct's doc comment, followed by a blank line and the doc comment of its
`new`. Constants have no docstring.

**The items stay Rust.** `geo::dist(...)` and `geo::units::km(...)` are still
ordinary Rust functions. The macro adds pyo3's `#[pyclass]` to each
`pub struct` and generates registration code next to the items (see
[What the macro generates](#what-the-macro-generates)); it does not change
the items themselves.

**`#[cfg]`** on an item, an `impl` block, a method or a field applies to the
Python side as well: the item is exported only when it is compiled.

## Functions and parameters

### Signatures

Parameter names are the Python names, so every argument can be passed by
position or by keyword. Trailing parameters of type `Option<...>` may be
omitted and then are `None` (an `Option` parameter followed by a required one
must be passed, possibly as `None`). There are no other defaults. Signatures
show in `inspect.signature` and `help()`, and argument errors use CPython's
wording:

```rust
#[orochi::module]
mod calc {
    pub fn add(a: i64, b: Option<i64>) -> i64 {
        a + b.unwrap_or(1)
    }
}
```

```python
>>> import calc, inspect
>>> calc.add(1), calc.add(1, 5), calc.add(b=10, a=1), calc.add(1, None)
(2, 6, 11, 2)
>>> inspect.signature(calc.add)
<Signature (a, b=None)>
>>> calc.add()
Traceback (most recent call last):
TypeError: add() missing 1 required positional argument: 'a'
>>> calc.add(1, c=2)
Traceback (most recent call last):
TypeError: add() got an unexpected keyword argument 'c'
```

A function can have at most 12 parameters (`self` included), and each
parameter must be a plain name (`x` or `mut x`, not a pattern such as
`(a, b)`).

### The three rules

What a parameter type means follows from three rules.

**1. Rust types are values.** A parameter of a Rust type (`i64`, `String`,
`Vec<T>`, `HashMap<K, V>`, `PathBuf`, tuples, `Option<T>`, ...: any
[`FromObject`](#conversions) type) receives a converted copy of the argument.
Nothing the function does to it is visible to the caller. Borrowed forms
`&T` and `Option<&T>`, where `T::Owned` is such a type, borrow that converted
copy: `&str` (a `String`), `&[T]` (a `Vec<T>`), `&Path` (a `PathBuf`),
`&HashMap<K, V>`, `Option<&str>`, ... They are only a more convenient
spelling of the owned type.

**2. orochi types are references.** [`Object`](#reference-handles),
`List<T>`, `Dict<K, V>`, `Set<T>` and `Ref<T>` receive the caller's Python
object itself: changes made through them are visible to the caller, and the
object keeps its identity. The container type is checked when the call is
made (`List<T>` accepts a `list` or a subclass of it, not a `tuple`); the
elements are converted to `T` on each access.

**3. Your classes are borrowed.** For a class of the module, `&Point`,
`&mut Point` and `Option<&Point>` (and the receivers `&self` and
`&mut self`) borrow the instance inside the Python object, without copying
it: a change through `&mut` is seen by every holder of the object. The
argument must be an instance of that class (or `None` for
`Option<&Point>`). Taking a class by value (`p: Point`, or a `self`
receiver) gives the function a copy, so the class must implement `Clone`.
`Ref<Point>` is rule 2: a handle to the instance, which the function can
keep (store in a field, return).

```rust
#[orochi::module]
mod rules {
    use orochi::prelude::*;

    pub fn total(xs: &[f64]) -> f64 {
        xs.iter().sum()
    }

    pub fn sorted(mut xs: Vec<i64>) -> Vec<i64> {
        xs.sort();
        xs
    }

    pub fn sort_in_place(xs: List<i64>) -> Result<()> {
        let mut sorted = xs.to_vec()?;
        sorted.sort();
        for (i, x) in sorted.into_iter().enumerate() {
            xs.set(i, x)?;
        }
        Ok(())
    }

    #[derive(Clone)]
    pub struct Counter {
        pub n: i64,
    }

    impl Counter {
        pub fn new() -> Self {
            Counter { n: 0 }
        }

        pub fn absorb(&mut self, other: &Counter) {
            self.n += other.n;
        }
    }

    pub fn bump(c: &mut Counter) {
        c.n += 1;
    }

    pub fn bumped(mut c: Counter) -> Counter {
        c.n += 1;
        c
    }
}
```

```python
>>> import rules
>>> rules.total((1, 2.5))
3.5
>>> xs = [3, 1, 2]
>>> rules.sorted(xs), xs
([1, 2, 3], [3, 1, 2])
>>> rules.sort_in_place(xs); xs
[1, 2, 3]
>>> c = rules.Counter()
>>> rules.bump(c); c.n
1
>>> rules.bumped(c).n, c.n
(2, 1)
```

**`&mut` is only for classes.** `&mut Vec<i64>`, `&mut String` and the like
are compile errors: the function would change a converted copy and the
change would be silently lost. Take `List<T>`, `Dict<K, V>` or `Set<T>` to
modify the caller's container; other Python values (numbers, strings,
tuples) are immutable, so return the new value instead.

**A class by value only works alone.** `p: Point` is fine, but not
`Option<Point>`, `Vec<Point>` or `(Point, i64)`: use `Option<&Point>`,
`Vec<Ref<Point>>` or `Option<Ref<Point>>` there.

### Strict conversions

Conversions do not coerce: a `float` is not an `int`, a `bool` is not a
number, a `str` is not a list of characters, `bytes` is not a `str` (the
[conversion table](#conversions) has the details). A failed conversion names
the argument:

- a `TypeError` becomes `TypeError("argument 'a': ...")`, with the original
  exception as its `__cause__`;
- any other exception (an `OverflowError` for an integer that does not fit,
  say) keeps its type and gets the note `while converting argument 'a'`
  (on Python 3.11+; before that, the message is prefixed with it instead).

```python
>>> calc.add(True)
Traceback (most recent call last):
TypeError: argument 'a': expected int, got bool
>>> calc.add(1.0)
Traceback (most recent call last):
TypeError: argument 'a': expected int, got float
```

### Borrowing

All arguments are converted first; then the class instances are borrowed for
the duration of the call, and released before the return value is
converted. Borrows follow `RefCell` rules: any number of shared borrows
(`&Point`), or a single mutable one (`&mut Point`). A conflicting borrow
raises `RuntimeError` instead of producing aliased `&mut` references:

```python
>>> c.absorb(c)    # `&mut self` and `&Counter` of the same object
Traceback (most recent call last):
RuntimeError: Already mutably borrowed
```

The same check applies while Rust code holds a borrow of the instance
through [`Ref::borrow_mut`](#reference-handles).

## Classes

A `pub struct` is a class; its inherent `impl` blocks in the same module
provide the constructor, methods, static methods and class attributes, and
`Debug`, `Display`, `PartialEq` and `Hash` provide special methods.

```rust
#[orochi::module]
mod shapes {
    /// A 2D point.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Point {
        /// Horizontal coordinate.
        pub x: f64,
        pub y: f64,
    }

    impl Point {
        pub const DIMENSIONS: usize = 2;

        /// `y` defaults to 0.
        pub fn new(x: f64, y: Option<f64>) -> Self {
            Point { x, y: y.unwrap_or(0.0) }
        }

        pub fn norm(&self) -> f64 {
            self.x.hypot(self.y)
        }

        pub fn scale(&mut self, k: f64) {
            self.x *= k;
            self.y *= k;
        }

        pub fn swapped(self) -> Point {
            Point { x: self.y, y: self.x }
        }

        pub fn origin() -> Point {
            Point::new(0.0, None)
        }
    }

    impl std::fmt::Display for Point {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "({}, {})", self.x, self.y)
        }
    }
}
```

```python
>>> from shapes import Point
>>> p = Point(3, 4)
>>> p.norm(), p.x, Point.DIMENSIONS
(5.0, 3.0, 2)
>>> p.scale(2); p
Point { x: 6.0, y: 8.0 }
>>> str(p.swapped()), Point.origin() == Point(0)
('(8, 6)', True)
>>> Point.__doc__
'A 2D point.\n\n`y` defaults to 0.'
>>> import inspect; inspect.signature(Point)
<Signature (x, y=None)>
```

A class's struct cannot have generic parameters or lifetimes, and must be
`Send + Sync`; only named fields can be `pub`. Its `__module__` is the
full name of the module that defines it (`mylib.geometry`), its
`__qualname__` the struct's name.

### Constructor

`pub fn new` without `self` is the constructor, `Point(...)` in Python. It
returns `Self` (or the struct's name) or `Result<Self, E>` with
`E: Into<orochi::Error>`; its parameters follow the same rules as a
function's, and its signature is the class's. A class has at most one
`pub fn new`. A `pub fn new` that returns anything else is a compile error
(give it another name: it becomes a static method).

A class without a `pub fn new` cannot be created from Python
(`TypeError: cannot create 'mylib.Token' instances`); its instances come from
Rust functions that return one. Python cannot skip the constructor either:
`object.__new__(Point)` raises `TypeError`. `new` itself is not an attribute
of the class.

### Methods and static methods

- `pub fn` with `&self` or `&mut self` is a method; it borrows the instance
  (rule 3). With `self` (or `mut self`) it gets a copy, so the class must
  be `Clone`. Other receivers (`self: Box<Self>`, ...) are not supported.
- `pub fn` without `self` (other than `new`) is a static method, callable on
  the class and on instances.
- `pub const` in the `impl` is a class attribute, a copy of the value. It can
  be an instance of the class itself (`pub const UNIT: Point = ...`, for a
  `Clone` class).

Methods bind like Python functions: `p.norm()` and `Point.norm(p)` both work,
`inspect.signature(Point.dist)` is `(self, /, other)`, and `self` cannot be
passed by keyword. Private functions and constants of the `impl` stay
Rust-only.

### Fields

A `pub` field (with a name) is a read/write property, documented by the
field's doc comment. Reading it returns a copy of a value, or the object
itself for a reference type; assigning converts strictly
(`TypeError: attribute 'x': expected float, got str`). Fields cannot be
deleted. Private fields are invisible to Python.

**The `Field` rule.** A `pub` field must have one of these types:

- integers, `f32`, `f64`, `bool`, `char`, `String`, `PathBuf`;
- `Object`, `List<T>`, `Dict<K, V>`, `Set<T>`, `Ref<T>`;
- `Option` of any of these.

Any other type is a compile error. A `pub items: Vec<i64>` field would hand
Python a new list on every read, so `obj.items.append(3)` would silently
change a copy. Use `List<i64>` for a list Python can modify in place, or
keep the field private and write methods. If a copy really is what you want
for your own type, implement the marker trait `orochi::Field` for it (it must
be `FromObject + IntoObject + Clone`).

### Special methods

| Rust | Python |
|------|--------|
| `Debug` | `__repr__` |
| `Display` | `__str__` |
| `PartialEq` | `__eq__` (and `!=`); comparing with another type gives `NotImplemented`, so `p == 3` is `False` |
| `Hash` | `__hash__` |

They are recognized by name, in `#[derive(...)]` on the struct or in an
`impl Debug for Point` (any path, such as `std::fmt::Display`) in the
module. `impl PartialEq<f64> for Point` does not count: it says nothing
about comparing two points. Without `Debug`, `repr` is Python's default
`<mylib.Point object at ...>`.

Equality and hashing follow Python's rules for `__eq__` and `__hash__`:

| The class implements | `==` | `hash()` |
|----------------------|------|----------|
| neither | identity | by identity |
| `PartialEq` | by value | `TypeError`: unhashable, like a Python class that defines only `__eq__` |
| `PartialEq` and `Hash` | by value | by value |
| `Hash` only | compile error | |

No other special methods are generated, and writing one by hand
(`pub fn __len__`) is a compile error; see
[Not supported (yet)](#not-supported-yet).

### Classes in submodules

Each module's structs are classes of that module. An `impl` block with `pub`
items must be in the module that defines the struct. Functions can take and
return the classes of other modules (`super::Point`,
`Vec<Ref<super::Point>>`).

## Return values and errors

A function, method or static method can return:

- any [`IntoObject`](#conversions) type: a new Python object for a value
  (`i64`, `String`, `Vec<T>`, maps, tuples, ...), or the object itself for
  a reference (`Object`, `List<T>`, `Ref<T>`, ...), including tuples and
  collections of them (`Vec<Ref<Point>>`);
- `()`, or nothing: `None`;
- `Option<T>` of those: the value or `None`;
- a class of the module by value, as `Point`, `Option<Point>` or
  `Result<Point, E>`: a new instance. Not inside other types
  (`Vec<Point>`, `(Point, Point)`, `Result<Option<Point>, E>`): create the
  instances with `Ref::new` and return `Ref<Point>`s there;
- `Result<T, E>` of any of these with `E: Into<orochi::Error>`: `Err` raises
  the error as a Python exception. `orochi::Result<T>` is
  `Result<T, orochi::Error>`.

Borrowed values cannot be returned (a compile error): return `String`, not
`&str`, `Vec<T>`, not `&[T]`; `&'static` values such as `&'static str` are
fine. A method that modifies `self` returns `()`: Python still holds the
instance.

### Errors

`orochi::Error` is a Python exception. Create one with
`Error::value_error(msg)`, `Error::type_error`, `Error::runtime_error`,
`Error::key_error`, `Error::index_error`, `Error::attribute_error`,
`Error::overflow_error`, `Error::new("LookupError", msg)` (any builtin
exception name or dotted `module.Class` path) or `Error::from_type(class,
msg)`. Your own error types convert with a `From` impl:

```rust
#[orochi::module]
mod store {
    use orochi::prelude::*;
    use std::path::PathBuf;

    pub fn read_text(path: PathBuf) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn parse(text: &str) -> std::result::Result<i64, std::num::ParseIntError> {
        text.trim().parse()
    }

    pub fn sqrt(x: f64) -> Result<f64> {
        if x < 0.0 {
            return Err(Error::value_error("negative"));
        }
        Ok(x.sqrt())
    }

    struct NotFound(String);

    impl From<NotFound> for Error {
        fn from(err: NotFound) -> Error {
            Error::key_error(err.0)
        }
    }

    fn lookup(key: &str) -> std::result::Result<i64, NotFound> {
        Err(NotFound(key.to_owned()))
    }

    pub fn find(key: &str) -> Result<i64> {
        Ok(lookup(key)?)
    }

    pub fn boom() {
        panic!("boom");
    }
}
```

```python
>>> import store
>>> try:
...     store.read_text("no/such/file")
... except FileNotFoundError:
...     print("not found")
not found
>>> store.parse("x")
Traceback (most recent call last):
ValueError: invalid digit found in string
>>> store.sqrt(-1)
Traceback (most recent call last):
ValueError: negative
>>> store.find("a")
Traceback (most recent call last):
KeyError: 'a'
```

Standard error types convert as pyo3 converts them:

| Rust error | Python exception |
|------------|------------------|
| `std::io::Error` | the matching `OSError` subclass (`FileNotFoundError`, `PermissionError`, ...), else `OSError` |
| `std::string::FromUtf8Error` | `UnicodeDecodeError` |
| `std::str::Utf8Error` | `ValueError` |
| `ParseIntError`, `ParseFloatError`, `ParseBoolError`, `ParseCharError`, `TryFromIntError`, `TryFromSliceError`, `std::ffi::NulError` | `ValueError` |
| `std::fmt::Error` | `RuntimeError` |
| pyo3's `PyErr` | itself |

**Panics** do not unwind into Python: a panic raises
`pyo3_runtime.PanicException` with the panic message. It derives from
`BaseException`, so `except Exception` does not catch it. If it propagates
back into Rust (through an orochi call), the panic resumes there.

```python
>>> store.boom()
Traceback (most recent call last):
pyo3_runtime.PanicException: boom
```

## Conversions

`IntoObject` converts Rust values into Python objects (return values, call
arguments, constants, fields), `FromObject` converts Python objects into
Rust values (parameters, `Object::to`, field assignments).

| Rust | Python (into) | Python accepted (from) |
|------|---------------|------------------------|
| `i8` ... `i128`, `isize`, `u8` ... `u128`, `usize` | `int` | `int` or any object with `__index__`, not `bool` |
| `f32`, `f64` | `float` | `float`, or an integer as above |
| `bool` | `bool` | `bool` only |
| `String`, `&str`, `Cow<str>` | `str` | `str` |
| `char` | `str` | `str` of length 1 |
| `()` | `None` | `None` |
| `Option<T>` | `T` or `None` | `T` or `None` |
| `Vec<T>`, `&[T]`, `[T; N]` | `list` | any sequence except `str`, `bytes`, `bytearray`, `memoryview` (`[T; N]`: of length `N`) |
| `Vec<u8>`, `&[u8]`, `[u8; N]` | `bytes` | `bytes`, `bytearray`, or a sequence of integers |
| `HashMap<K, V>`, `BTreeMap<K, V>` | `dict` | any `collections.abc.Mapping` |
| `HashSet<T>`, `BTreeSet<T>` | `set` | `set` or `frozenset` |
| tuples of 1 to 12 elements | `tuple` | `tuple` of the same length |
| `PathBuf`, `&Path` | `pathlib.Path` | `str` or `os.PathLike` |
| `Box<T>` | as `T` | as `T` |
| `Object` | the object | any object |
| `List<T>`, `Dict<K, V>`, `Set<T>` | the object | a `list`, `dict`, `set` (or subclass) |
| `Ref<T>` | the instance | an instance of class `T` |

- A value that does not fit the Rust type raises `OverflowError` (also for a
  negative value into an unsigned type, and for a finite `float` out of
  `f32`'s range); a `char` from a string of another length raises
  `ValueError`; every other mismatch raises `TypeError`
  (`expected int, got str`).
- An `int` is accepted where a float is expected; nothing else is coerced.
- Maps and sets are converted from a snapshot of their items, so Python code
  run during the conversion (an element's `__index__`, say) cannot
  invalidate it; sequences are iterated like a Python `for` loop.
- References (`&str`, `&String`, `&Path`, `&PathBuf`, `&[T]`, `&Vec<T>`,
  `&Object`, `&List<T>`, ...) convert into Python like the owned types; as
  parameters, see [rule 1](#the-three-rules).
- Implement `IntoObject` / `FromObject` for your own types by building or
  reading Python objects with orochi's API (examples in the rustdoc of the
  two traits).

## Reference handles

`Object`, `List<T>`, `Dict<K, V>`, `Set<T>` and `Ref<T>` refer to a Python
object. They have no lifetime, are `Clone + Send + Sync + 'static` (a clone
refers to the same object), and can be stored in structs and fields, sent to
other threads and kept as long as needed; each operation attaches to the
interpreter by itself. `Display` is Python's `str()`, `Debug` its `repr()`,
and neither panics. `is(&other)` is Python's `is`; `Object::from(handle)` (or
`.into()`) gives the same object as an `Object`.

| Handle | Created with | Main operations |
|--------|--------------|-----------------|
| `Object` (any object) | `Object::new(value)`, `Object::none()`, `Object::from_fn(closure)`, `orochi::eval`, ... | `attr`, `set_attr`, `has_attr`, `del_attr`, `call`, `call_kw`, `method`, `method_kw`, `item`, `set_item`, `del_item`, `contains`, `eq`, `to::<T>()`, `iter`, `len`, `is_empty`, `is_truthy`, `is_none`, `is_instance`, `type_object`, `type_name`, `repr`, `str` |
| `List<T>` (a `list`) | `List::new()`, `List::from_vec(v)` | `len`, `is_empty`, `get`, `set`, `push`, `insert`, `pop`, `clear`, `iter`, `to_vec`, `as_object`, `into_inner` |
| `Dict<K, V>` (a `dict`) | `Dict::new()` | `len`, `is_empty`, `get`, `insert`, `remove`, `contains`, `clear`, `keys`, `values`, `items`, `to_hashmap`, `as_object`, `into_inner` |
| `Set<T>` (a `set`) | `Set::new()` | `len`, `is_empty`, `add`, `remove`, `contains`, `clear`, `to_vec`, `to_hashset`, `as_object`, `into_inner` |
| `Ref<T>` (an instance of class `T`) | `Ref::new(value)` | `borrow`, `borrow_mut`, `to_object` |

The typed handles check the container type when they are created and convert
elements on each access (`get` on a `List<i64>` fails for an element that is
not an `int`). `List::get` and `List::set` raise `IndexError` out of range;
`Dict::get`, `Dict::remove` and `List::pop` return `None` for a missing
entry.

**`Ref<T>`** refers to an instance of a class: Python variables, other `Ref`s
and `Ref` fields all see the same data. `borrow()` and `borrow_mut()` return
guards that dereference to `&T` / `&mut T`, with `RefCell` rules: any number
of shared borrows or one mutable borrow, a conflict failing with
`RuntimeError` instead of blocking. While Rust holds a guard, Python calls
that need a conflicting borrow fail the same way. Python's garbage collector
does not see references held in Rust fields, so a cycle of `Ref`s is never
freed.

```rust
#[orochi::module]
mod scene {
    use orochi::prelude::*;

    pub struct Point {
        pub x: f64,
        pub y: f64,
    }

    pub struct Circle {
        pub center: Ref<Point>,
        pub radius: f64,
    }

    impl Circle {
        pub fn new(center: Ref<Point>, radius: f64) -> Self {
            Circle { center, radius }
        }
    }
}

fn main() -> orochi::Result<()> {
    use orochi::Ref;
    use scene::{Circle, Point};

    let p = Ref::new(Point { x: 1.0, y: 2.0 })?;
    let c = Ref::new(Circle::new(p.clone(), 3.0))?;
    p.borrow_mut()?.x = 5.0;
    assert_eq!(c.borrow()?.center.borrow()?.x, 5.0); // the same object
    assert!(c.borrow()?.center.is(&p));
    Ok(())
}
```

## Calling Python from Rust

The same types call Python from Rust: inside an exported function (the
interpreter is running), or in a Rust program that embeds Python (with the
[`embed` feature](#features)).

- `orochi::import("json")` imports a module; `orochi::builtins()` is the
  `builtins` module.
- `orochi::eval(expr)` evaluates an expression in a fresh namespace;
  `orochi::exec(code)` runs statements in a fresh namespace and returns it as
  a `Scope`. A `Scope` (`Scope::new()`) is a global namespace with `eval`,
  `exec`, `get` (`NameError` if undefined), `set`, `contains` and `globals`.
  Source code is dedented first, so it can be an indented Rust string
  literal.
- `obj.call(args)`, `obj.method(name, args)`, and `call_kw` / `method_kw`
  with keyword arguments.
- `.to::<T>()` converts a result to a Rust value (a copy) or a handle (a
  reference).

**Positional arguments** (`IntoArgs`):

- `()` for none;
- one plain value passed bare: numbers, `bool`, `char`, strings, paths,
  `Object` / `&Object`, the handles (`List`, `Dict`, `Set`, `Ref`, and
  references to them), and `Option<T>`;
- a tuple `(a, b, ...)` of up to 12 `IntoObject` values; a single
  collection must be wrapped in one, `(vec,)`, since a bare `Vec` would be
  ambiguous;
- `orochi::Args` for a list built at run time (`Args::new().arg(1)`,
  `collect()`).

**Keyword arguments** (`IntoKwargs`): `kwargs! { "key" => value, ... }`
(which builds an `orochi::Kwargs`), `HashMap<String, V>`, `HashMap<&str, V>`
and `Dict<String, V>`. A repeated keyword raises `TypeError` when the call is
made.

```rust
use orochi::prelude::*;

fn main() -> orochi::Result<()> {
    let scope = orochi::exec(
        r#"
        def greet(name, punct="!"):
            return f"hello {name}{punct}"
        names = ["ann"]
        "#,
    )?;
    let greet = scope.get("greet")?;
    assert_eq!(greet.call("bob")?.to::<String>()?, "hello bob!");
    let text = greet.call_kw(("bob",), kwargs! { "punct" => "?" })?;
    assert_eq!(text.to::<String>()?, "hello bob?");

    // A handle is the Python list itself.
    let names: List<String> = scope.get("names")?.to()?;
    names.push("bob")?;
    assert_eq!(scope.eval("len(names)")?.to::<usize>()?, 2);

    let json = orochi::import("json")?;
    let text = json.method_kw("dumps", (vec![1, 2],), kwargs! { "separators" => (",", ":") })?;
    assert_eq!(text.to::<String>()?, "[1,2]");
    Ok(())
}
```

### Rust closures as Python callables

`Object::from_fn(closure)` wraps a Rust closure as a Python callable. Its
parameters and return type follow the rules of exported functions (up to 12
parameters, values, references, `&T` / `&mut T` of a class, `Result`
returns, panics raised as `PanicException`), except that the parameters
have no names: they are positional-only and called `arg1`, `arg2`, ... in
error messages. Trailing `Option` parameters may be omitted.

```rust
use orochi::Object;

fn main() -> orochi::Result<()> {
    let scope = orochi::Scope::new();
    scope.set("add", Object::from_fn(|a: i64, b: Option<i64>| a + b.unwrap_or(1)))?;
    assert_eq!(scope.eval("add(40, 2) + add(-1)")?.to::<i64>()?, 42);
    let err = scope.eval("add('x')").unwrap_err();
    assert_eq!(err.to_string(), "TypeError: argument 'arg1': expected int, got str");
    Ok(())
}
```

### The Error type

Every fallible operation returns `orochi::Result<T>`, whose error is a
Python exception:

- `err.type_name()` (`"ZeroDivisionError"`), `err.message()` (Python's
  `str(exc)`), `err.traceback()`, `err.value()` (the exception object);
- `err.is_instance_of("OSError")` (a builtin name or a dotted path to an
  already imported module; subclasses match, like `except`) and
  `err.is_instance(&class)`;
- `Display` prints `TypeName: message`, the alternate form `{:#}` the full
  Python rendering with traceback, notes and chained exceptions; `Debug`
  includes the traceback;
- `Error` implements `std::error::Error`, so it works with `?` into
  `Box<dyn Error>`;
- constructors as listed in [Errors](#errors); `Error::from_value(&obj)`
  wraps an exception instance.

## What the macro generates

`#[orochi::module]` reads only names, signatures, derives and doc comments;
all behaviour is in the library. For this module:

```rust
#[orochi::module]
mod mylib {
    //! The module docstring.

    /// Add two integers.
    pub fn add(a: i64, b: Option<i64>) -> i64 {
        a + b.unwrap_or(1)
    }

    #[derive(Debug, PartialEq)]
    pub struct Point {
        pub x: f64,
        pub y: f64,
    }

    impl Point {
        pub fn new(x: f64, y: f64) -> Self {
            Point { x, y }
        }
        pub fn norm(&self) -> f64 {
            self.x.hypot(self.y)
        }
    }

    pub mod geometry {}
}
```

it keeps every item as written and adds, roughly (paths shortened, doc
strings simplified):

```rust,ignore
#[pyclass(crate = "::orochi::pyo3", module = "mylib")]   // on the struct
pub struct Point { /* ... */ }

#[pymethods(crate = "::orochi::pyo3")]
impl Point {
    #[new]
    #[pyo3(signature = (*args, **kwargs), text_signature = None)]
    fn __orochi_new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        ::orochi::construct(args, kwargs)
    }
}

/// Registers this module's items (in every module and submodule).
pub fn __orochi_init(__orochi_m: &::orochi::ModuleBuilder<'_>) -> ::orochi::Result<()> {
    __orochi_m.doc("The module docstring.")?;
    {
        let __orochi_c = __orochi_m.class::<Point>()?;
        __orochi_c.field("x", "", |s: &Point| &s.x, |s: &mut Point| &mut s.x)?;
        __orochi_c.field("y", "", |s: &Point| &s.y, |s: &mut Point| &mut s.y)?;
        __orochi_c.constructor(&["x", "y"], "", Point::new)?;
        __orochi_c.method("norm", &[], "", Point::norm)?;
        __orochi_c.repr_debug()?;
        __orochi_c.eq()?;
    }
    __orochi_m.function("add", &["a", "b"], "Add two integers.", self::add)?;
    __orochi_m.submodule("geometry", "", self::geometry::__orochi_init)?;
    Ok(())
}

// In the root module only:
#[pymodule(crate = "::orochi::pyo3", name = "mylib", gil_used = true)]
fn __orochi_pymodule(module: &Bound<'_, PyModule>) -> PyResult<()> {
    ::orochi::init_module(module, __orochi_init)   // `PyInit_mylib`
}

pub fn __orochi_module() -> ::orochi::Result<::orochi::Object> {
    ::orochi::load_module("mylib", __orochi_init)
}
```

plus, for each parameter and return type, a compile-time check that reports
a type without a conversion on that type. The classes are registered first,
in source order (fields, then the items of the `impl` blocks, then the
special methods), followed by the functions, constants and submodules in
source order. The `#[new]` exists only for a class with a `pub fn new`.

## Writing a module by hand

The builder that the macro calls is public, for modules the macro cannot
express or for classes written with pyo3's own attributes:

- `ModuleBuilder`: `doc`, `function(name, params, doc, f)`,
  `constant(name, value)`, `class::<T>()`, `submodule(name, doc, init)`,
  `module()`.
- `ClassBuilder<T>`: `doc`, `constructor(params, doc, new)`,
  `method(name, params, doc, f)` (the first parameter of `f` is `&T`,
  `&mut T` or `T`), `static_method`, `constant`, `field` (a `Field`),
  `property` (getter and setter closures), `readonly`, `repr_debug`,
  `str_display`, `eq`, `hash`, `class()`.

`f` is any function or closure whose parameter and return types follow the
rules above, with at most 12 parameters; closures may capture state
(`Send + Sync + 'static`). `params` names the parameters (without `self`); a
wrong count, a duplicate name or a method whose first parameter is not the
class fails with `TypeError` when the module is built. The registration
methods return the builder again, so calls chain.

A class is any pyo3 `#[pyclass]` that is not `frozen` and is `Send + Sync`.
To be constructible from Python it needs a `#[new]` that runs the
constructor registered with `ClassBuilder::constructor`, through
`orochi::construct`. Keeping pyo3's `#[new]` means Python cannot create an
instance without running the constructor; `text_signature = None` leaves the
signature to `constructor`, which sets `__signature__`.

The extension's entry point is a `#[pymodule]` that calls
`orochi::init_module(module, init)`; `orochi::load_module(name, init)` builds
the same module in-process (see the next section).

```rust
use orochi::pyo3::prelude::*;
use orochi::pyo3::types::{PyDict, PyModule, PyTuple};
use orochi::{ModuleBuilder, Result};

/// Counts.
#[pyclass(crate = "orochi::pyo3", module = "counting")]
#[derive(Debug)]
struct Counter {
    count: i64,
}

#[pymethods(crate = "orochi::pyo3")]
impl Counter {
    #[new]
    #[pyo3(signature = (*args, **kwargs), text_signature = None)]
    fn py_new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        orochi::construct(args, kwargs)
    }
}

fn init(m: &ModuleBuilder<'_>) -> Result<()> {
    let step = 2;
    m.doc("Counting, by hand.")?
        .function("double", &["x"], "", move |x: i64| x * step)?;
    m.class::<Counter>()?
        .constructor(&["start"], "", |start: Option<i64>| Counter { count: start.unwrap_or(0) })?
        .method("incr", &[], "Count one more.", |c: &mut Counter| c.count += 1)?
        .field("count", "", |c: &Counter| &c.count, |c: &mut Counter| &mut c.count)?
        .repr_debug()?;
    Ok(())
}

// The entry point, `PyInit_counting`.
#[pymodule(crate = "orochi::pyo3", name = "counting")]
fn counting_module(module: &Bound<'_, PyModule>) -> PyResult<()> {
    orochi::init_module(module, init)
}

fn main() -> orochi::Result<()> {
    orochi::load_module("counting", init)?;
    orochi::exec(
        r#"
        import counting
        c = counting.Counter(5)
        c.incr()
        assert c.count == 6 and counting.double(c.count) == 12
        assert repr(c) == "Counter { count: 6 }"
        "#,
    )?;
    Ok(())
}
```

Use `#[pyclass(module = "...")]` to give the class its full name.

## Testing a module in-process

`#[orochi::module] mod mylib` also generates `mylib::__orochi_module()`,
which builds the module in the running process with
`orochi::load_module("mylib", ...)` and registers it in `sys.modules`, so
that Python code can `import mylib` without the compiled extension. A second
call returns the registered module. With the `embed` feature (for example
through a dev-dependency on orochi), tests can drive the module from Python:

```rust
#[orochi::module]
mod tested {
    pub fn add(a: i64, b: i64) -> i64 {
        a + b
    }
}

fn main() -> orochi::Result<()> {
    tested::__orochi_module()?; // what `import tested` would load
    orochi::exec(
        r#"
        import tested
        assert tested.add(1, 2) == 3
        "#,
    )?;
    Ok(())
}
```

## pyo3 interop

orochi is built on pyo3, re-exported as `orochi::pyo3` (use
`crate = "orochi::pyo3"` in pyo3's attributes, as above). Hand-written pyo3
code converts with `From` / `Into`:

- `Object` from `Py<PyAny>` or `Bound<'py, PyAny>`, and `Py<PyAny>` from
  `Object`; `object.as_ref()` borrows the `Py<PyAny>`, and `.bind(py)` on it
  gives the `Bound`;
- `orochi::Error` from and into `PyErr` (so `?` works both ways);
  `error.as_ref()` borrows the `PyErr`;
- any `#[pyclass]` that is not `frozen` and is `Send + Sync` is an orochi
  class: it works with `ClassBuilder`, `Ref<T>` and `&T` / `&mut T`
  parameters.

## Features

- `macros` (default): the `#[orochi::module]` attribute.
- `embed`: start the Python interpreter automatically on first use. Enable
  it for Rust programs (and tests) that embed Python; leave it off for
  extension modules, where the interpreter is already running. Without it,
  using orochi when no interpreter is running panics with a message saying
  so.

## Threads, locks and interpreter lifetime

Every orochi operation attaches the calling thread to the interpreter (taking
the GIL on non-free-threaded builds) for its duration, so handles can be
used from any thread. Modules made with `#[orochi::module]` declare that they
need the GIL (`gil_used = true`): on a free-threaded Python, importing one
re-enables the GIL.

**Locks.** Do not hold a Rust lock (`Mutex`, `RwLock`, ...) while calling
into Python if another thread may take the same lock while attached to
Python (for example inside a Rust function called from Python): thread A
holds the lock and waits for the GIL, thread B holds the GIL and waits for
the lock, and both block forever. Release the lock first, or take it only
while not calling Python.

**Interpreter lifetime.** Using a handle panics if the interpreter is not
running (see [Features](#features)) or is shutting down. Objects kept in
Rust `static`s or in threads that outlive the interpreter must not be used
after Python has begun finalization; dropping them then is harmless.
Formatting a handle or an error (`Display`, `Debug`) never panics.

## Not supported (yet)

**Compile errors** on exported items (make the item private to keep it
Rust-only):

- items: `pub enum`, `pub union`, `pub trait`, `pub type`, `pub use`,
  `pub extern crate`, `pub static mut`, a `pub mod` that is not inline
  (`pub mod name;`), generic `pub const`s;
- functions and methods: generic parameters or explicit lifetimes,
  `where` clauses, `async`, `unsafe`, `extern` and variadic functions, more
  than 12 parameters (`self` included), parameters that are patterns,
  `#[cfg]` or `#[cfg_attr]` on a parameter, `impl Trait` in a parameter or
  return type, explicit lifetimes in types (`'_` and `'static` are fine),
  returning borrowed values (other than `&'static`), receivers other than
  `&self`, `&mut self` and `self`, special methods written by hand
  (`__len__`, ...), a second `pub fn new` in a class, a `pub fn new` that
  does not return `Self` or `Result<Self, E>`;
- classes: generic structs or structs with lifetimes, `pub` fields of tuple
  structs, `pub` fields whose type is not a [`Field`](#fields) (`Vec<T>`,
  maps, classes by value, ...), `Hash` without `PartialEq`, generic `impl`
  blocks with `pub` items, an `impl` block with `pub` items for a struct of
  another module, `#[cfg]` on the `Debug` / `Display` / `PartialEq` / `Hash`
  impls of a class;
- names: Python keywords (`r#in`), names starting with `__orochi` (for every
  item of the module), two exported items with the same Python name in one
  namespace;
- types: parameter and return types without a conversion, `&mut` of anything
  but a class, classes by value inside other types (`Option<Point>` or
  `Vec<Point>` parameters, `Vec<Point>`, `(Point, Point)` or
  `Result<Option<Point>, E>` returns: use `Ref<Point>`).

**Silently not seen** by the macro: items produced by `macro_rules!` macros
inside the module, derives behind `#[cfg_attr(..., derive(...))]`, and the
items of private modules.

**Missing features:** `.pyi` type stubs; subclassing exported classes in
Python, and inheritance between classes; keyword-only parameters, `*args`
and `**kwargs`; defaults other than `None`; special methods other than
`__repr__`, `__str__`, `__eq__` and `__hash__` (no ordering, operators,
`__len__`, iteration, context managers, ...); computed properties with the
macro (`ClassBuilder::property` / `readonly` by hand); enums; `async`
functions; the buffer protocol (`memoryview` and other buffers are
rejected); free-threaded Python without the GIL.
