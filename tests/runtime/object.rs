use orochi::prelude::*;
use orochi::{builtins, eval, exec, import};

#[test]
fn attributes() -> Result<()> {
    let scope = exec("class C:\n    pass\nc = C()")?;
    let c = scope.get("c")?;
    assert!(!c.has_attr("x")?);
    c.set_attr("x", 42)?;
    assert!(c.has_attr("x")?);
    assert_eq!(c.attr("x")?.to::<i64>()?, 42);
    assert_eq!(scope.eval("c.x")?.to::<i64>()?, 42);
    c.del_attr("x")?;
    assert!(!c.has_attr("x")?);
    assert!(c.del_attr("x").unwrap_err().is_instance_of("AttributeError"));

    let err = c.attr("missing").unwrap_err();
    assert!(err.is_instance_of("AttributeError"));
    Ok(())
}

#[test]
fn new_eq_contains() -> Result<()> {
    let xs = Object::new(vec![1, 2, 3])?;
    assert_eq!(xs.type_name(), "list");
    assert!(xs.eq(vec![1, 2, 3])?);
    assert!(!xs.eq(vec![1, 2])?);
    assert!(xs.contains(2)?);
    assert!(!xs.contains(5)?);
    assert!(eval("1")?.eq(1.0)?);
    assert!(Object::new("ab")?.contains("a")?);
    assert!(eval("{'k': 1}")?.contains("k")?);
    assert!(eval("5")?.contains(1).unwrap_err().is_instance_of("TypeError"));

    // `==` may raise; the error is returned.
    let scope = exec("class E:\n    def __eq__(self, o):\n        raise ValueError('no')\ne = E()")?;
    assert!(scope.get("e")?.eq(1).unwrap_err().is_instance_of("ValueError"));
    Ok(())
}

#[test]
fn calls() -> Result<()> {
    let scope = exec(
        "def f(*args, **kwargs):\n    return (args, sorted(kwargs.items()))\n\
         def zero():\n    return 'zero'",
    )?;
    let f = scope.get("f")?;

    assert_eq!(scope.get("zero")?.call(())?.to::<String>()?, "zero");

    let (args, kw): (Vec<i64>, Vec<(String, i64)>) = f.call((1, 2, 3))?.to()?;
    assert_eq!(args, [1, 2, 3]);
    assert!(kw.is_empty());

    let (args, kw): (Vec<String>, Vec<(String, i64)>) =
        f.call_kw("a", kwargs! { "x" => 1, "y" => 2 })?.to()?;
    assert_eq!(args, ["a"]);
    assert_eq!(kw, [("x".to_owned(), 1), ("y".to_owned(), 2)]);

    // Keyword arguments from a HashMap and from a Dict handle.
    let mut map = std::collections::HashMap::new();
    map.insert("k", 5);
    let (_, kw): (Object, Vec<(String, i64)>) = f.call_kw((), map)?.to()?;
    assert_eq!(kw, [("k".to_owned(), 5)]);

    let d: Dict<String, i64> = Dict::new();
    d.insert("z", 9)?;
    let (_, kw): (Object, Vec<(String, i64)>) = f.call_kw((), &d)?.to()?;
    assert_eq!(kw, [("z".to_owned(), 9)]);

    // Empty kwargs!.
    let (_, kw): (Object, Vec<(String, i64)>) = f.call_kw((), kwargs! {})?.to()?;
    assert!(kw.is_empty());
    Ok(())
}

#[test]
fn single_argument_calls() -> Result<()> {
    let ident = eval("lambda *a: a")?;
    let one = |args: Object| -> Result<Object> {
        let t: Vec<Object> = args.to()?;
        assert_eq!(t.len(), 1, "expected exactly one argument, got {args}");
        Ok(t[0].clone())
    };
    assert_eq!(one(ident.call(2.0)?)?.to::<f64>()?, 2.0);
    assert_eq!(one(ident.call(7u8)?)?.to::<u8>()?, 7);
    assert_eq!(one(ident.call(-7i128)?)?.to::<i128>()?, -7);
    assert!(one(ident.call(true)?)?.to::<bool>()?);
    assert_eq!(one(ident.call('c')?)?.to::<char>()?, 'c');
    assert_eq!(one(ident.call("x")?)?.to::<String>()?, "x");
    assert_eq!(one(ident.call("y".to_owned())?)?.to::<String>()?, "y");
    let z = "z".to_owned();
    assert_eq!(one(ident.call(&z)?)?.to::<String>()?, z);
    assert_eq!(one(ident.call(std::borrow::Cow::Borrowed("w"))?)?.to::<String>()?, "w");
    assert!(one(ident.call(None::<i64>)?)?.is_none());
    assert_eq!(one(ident.call(Some(3))?)?.to::<i64>()?, 3);

    let obj = eval("[1]")?;
    assert!(one(ident.call(&obj)?)?.is(&obj));
    assert!(one(ident.call(obj.clone())?)?.is(&obj));

    let l: List<i64> = List::new();
    assert!(one(ident.call(&l)?)?.is(l.as_object()));
    assert!(one(ident.call(l.clone())?)?.is(l.as_object()));
    let d: Dict<String, i64> = Dict::new();
    assert!(one(ident.call(&d)?)?.is(d.as_object()));
    let s: Set<i64> = Set::new();
    assert!(one(ident.call(&s)?)?.is(s.as_object()));

    // Tuples are argument lists; collections need a 1-tuple.
    assert_eq!(ident.call((1, 2))?.len()?, 2);
    assert_eq!(one(ident.call((vec![1, 2],))?)?.len()?, 2);

    // Methods too.
    assert_eq!(import("math")?.method("sqrt", 16.0)?.to::<f64>()?, 4.0);
    Ok(())
}

#[test]
fn dynamic_args() -> Result<()> {
    let ident = eval("lambda *a: a")?;
    let args: Args = vec![1, 2, 3].into_iter().collect();
    assert_eq!(args.len(), 3);
    assert_eq!(ident.call(args)?.to::<Vec<i64>>()?, [1, 2, 3]);

    let objs = vec![eval("'a'")?, eval("None")?];
    let args: Args = objs.into_iter().collect();
    assert_eq!(ident.call(args)?.to_string(), "('a', None)");

    let mut args = Args::new().arg(1).arg("two");
    args.push(3.0);
    args.extend([4, 5]);
    assert_eq!(ident.call(args)?.to_string(), "(1, 'two', 3.0, 4, 5)");
    assert!(Args::new().is_empty());

    // A failed conversion is reported by the call.
    let mut bad = std::collections::HashMap::new();
    bad.insert(vec![1], 1);
    let err = ident.call(Args::new().arg(bad)).unwrap_err();
    assert!(err.is_instance_of("TypeError"));
    Ok(())
}

#[test]
fn duplicate_keyword_arguments() -> Result<()> {
    let f = eval("lambda **kw: kw")?;
    let err = f.call_kw((), kwargs! { "a" => 1, "a" => 2 }).unwrap_err();
    assert!(err.is_instance_of("TypeError"));
    assert_eq!(err.message(), "got multiple values for keyword argument 'a'");
    assert_eq!(f.call_kw((), kwargs! { "a" => 1, "b" => 2 })?.len()?, 2);
    Ok(())
}

#[test]
fn methods() -> Result<()> {
    let s = "a,b,c".into_object()?;
    let parts: Vec<String> = s.method("split", ",")?.to()?;
    assert_eq!(parts, ["a", "b", "c"]);
    assert_eq!(s.method("upper", ())?.to::<String>()?, "A,B,C");

    let parts: Vec<String> = s.method_kw("split", (), kwargs! { "sep" => ",", "maxsplit" => 1 })?.to()?;
    assert_eq!(parts, ["a", "b,c"]);
    Ok(())
}

#[test]
fn items() -> Result<()> {
    let d = eval("{'a': 1}")?;
    assert_eq!(d.item("a")?.to::<i64>()?, 1);
    d.set_item("b", 2)?;
    assert_eq!(d.item("b")?.to::<i64>()?, 2);
    d.del_item("a")?;
    assert!(d.item("a").unwrap_err().is_instance_of("KeyError"));

    let l = eval("[10, 20]")?;
    assert_eq!(l.item(1)?.to::<i64>()?, 20);
    assert!(l.item(5).unwrap_err().is_instance_of("IndexError"));
    Ok(())
}

#[test]
fn iteration_and_len() -> Result<()> {
    let l = eval("[1, 2, 3]")?;
    assert_eq!(l.len()?, 3);
    assert!(!l.is_empty()?);
    assert!(eval("[]")?.is_empty()?);
    let items: Vec<i64> = l.iter()?.map(|o| o?.to::<i64>()).collect::<Result<_>>()?;
    assert_eq!(items, [1, 2, 3]);

    // Errors raised by __next__ are yielded.
    let scope = exec("def gen():\n    yield 1\n    raise ValueError('boom')")?;
    let mut it = scope.get("gen")?.call(())?.iter()?;
    assert_eq!(it.next().unwrap()?.to::<i64>()?, 1);
    assert!(it.next().unwrap().unwrap_err().is_instance_of("ValueError"));

    // Not iterable / no len.
    assert!(eval("5")?.iter().unwrap_err().is_instance_of("TypeError"));
    assert!(eval("5")?.len().unwrap_err().is_instance_of("TypeError"));
    Ok(())
}

#[test]
fn predicates() -> Result<()> {
    let none = Object::none();
    assert!(none.is_none());
    assert!(!eval("0")?.is_none());
    assert_eq!(none.type_name(), "NoneType");

    let int = builtins().attr("int")?;
    let five = eval("5")?;
    assert!(five.is_instance(&int)?);
    assert!(!five.is_instance(&builtins().attr("str")?)?);
    assert!(five.type_object().is(&int));
    assert_eq!(five.type_name(), "int");
    assert!(five.is_instance(&five).is_err());

    let a = eval("[]")?;
    let b = a.clone();
    assert!(a.is(&b));
    assert!(!a.is(&eval("[]")?));
    assert!(Object::none().is(&Object::none()));

    assert!(eval("[1]")?.is_truthy()?);
    assert!(!eval("''")?.is_truthy()?);
    Ok(())
}

#[test]
fn display_and_debug() -> Result<()> {
    let s = "hi".into_object()?;
    assert_eq!(s.to_string(), "hi");
    assert_eq!(format!("{s:?}"), "'hi'");
    assert_eq!(s.repr()?, "'hi'");
    assert_eq!(s.str()?, "hi");

    let scope = exec(
        "class Bad:\n    def __str__(self):\n        raise RuntimeError()\n    \
         def __repr__(self):\n        raise RuntimeError()\nb = Bad()",
    )?;
    let b = scope.get("b")?;
    assert_eq!(b.to_string(), "<unprintable Bad object>");
    assert_eq!(format!("{b:?}"), "<unrepresentable Bad object>");
    Ok(())
}

#[test]
fn formatting_never_panics() -> Result<()> {
    // `__str__` / `__repr__` raising a PanicException (from a panicking Rust
    // callback): pyo3 resumes the panic in Rust; formatting must swallow it.
    let scope = Scope::new();
    scope.set("boom", Object::from_fn(|| -> String { panic!("formatting panic") }))?;
    scope.exec(
        "class P:\n    def __str__(self):\n        return boom()\n    \
         def __repr__(self):\n        return boom()\np = P()",
    )?;
    let p = scope.get("p")?;
    assert_eq!(p.to_string(), "<unprintable object>");
    assert_eq!(format!("{p:?}"), "<unrepresentable object>");
    // The non-formatting API still reports (resumes) the panic.
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.str())).is_err());
    Ok(())
}

#[test]
fn from_fn_round_trip() -> Result<()> {
    let add = Object::from_fn(|a: i64, b: i64| a + b);
    assert_eq!(add.call((2, 3))?.to::<i64>()?, 5);

    // Called from Python code.
    let scope = Scope::new();
    scope.set("add", &add)?;
    scope.set("greet", Object::from_fn(|name: String| format!("hello {name}")))?;
    scope.set("nothing", Object::from_fn(|| ()))?;
    scope.set(
        "summarize",
        Object::from_fn(|xs: Vec<f64>, scale: Option<f64>| {
            xs.iter().sum::<f64>() * scale.unwrap_or(1.0)
        }),
    )?;
    assert_eq!(scope.eval("add(40, 2)")?.to::<i64>()?, 42);
    assert_eq!(scope.eval("greet('py')")?.to::<String>()?, "hello py");
    assert!(scope.eval("nothing()")?.is_none());
    assert_eq!(scope.eval("summarize([1, 2.5], None)")?.to::<f64>()?, 3.5);
    assert_eq!(scope.eval("summarize((1, 2), 10)")?.to::<f64>()?, 30.0);
    assert_eq!(scope.eval("list(map(add, [1, 2], [10, 20]))")?.to::<Vec<i64>>()?, [11, 22]);

    // Captured state and many arguments.
    let offset = 100;
    let many = Object::from_fn(move |a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32| {
        offset + a + b + c + d + e + f + g + h
    });
    assert_eq!(many.call((1, 1, 1, 1, 1, 1, 1, 1))?.to::<i32>()?, 108);

    // Objects pass through by reference.
    let append = Object::from_fn(|l: List<i64>, v: i64| l.push(v));
    let l = eval("[]")?;
    append.call((&l, 7))?;
    assert_eq!(l.to::<Vec<i64>>()?, [7]);

    // Returning a Result with a std error type.
    let parse = Object::from_fn(|s: String| s.parse::<i64>());
    assert_eq!(parse.call("12")?.to::<i64>()?, 12);
    assert!(parse.call("x").unwrap_err().is_instance_of("ValueError"));
    Ok(())
}

#[test]
fn from_fn_errors() -> Result<()> {
    let add = Object::from_fn(|a: i64, b: i64| a + b);

    let err = add.call(1).unwrap_err();
    assert!(err.is_instance_of("TypeError"));
    assert!(err.message().contains("takes 2 positional arguments but 1 was given"), "{err}");

    let err = add.call_kw((1, 2), kwargs! { "c" => 3 }).unwrap_err();
    assert!(err.is_instance_of("TypeError"));

    let fallible = Object::from_fn(|x: i64| -> Result<i64> {
        if x < 0 {
            Err(Error::value_error("negative"))
        } else {
            Ok(x)
        }
    });
    assert_eq!(fallible.call(1)?.to::<i64>()?, 1);
    let err = fallible.call(-1).unwrap_err();
    assert!(err.is_instance_of("ValueError"));
    assert_eq!(err.message(), "negative");

    // A Python exception raised inside the closure propagates unchanged.
    let inner = Object::from_fn(|| eval("1 / 0"));
    assert!(inner.call(()).unwrap_err().is_instance_of("ZeroDivisionError"));

    // Panics become Python exceptions (catchable in Python) instead of
    // aborting; when the exception reaches Rust again, the panic resumes.
    let panics = Object::from_fn(|| -> i64 { panic!("oh no") });
    let scope = Scope::new();
    scope.set("panics", &panics)?;
    scope.exec(
        "try:\n    panics()\nexcept BaseException as e:\n    caught = (type(e).__name__, str(e))",
    )?;
    let caught: (String, String) = scope.get("caught")?.to()?;
    assert_eq!(caught, ("PanicException".to_owned(), "oh no".to_owned()));

    let resumed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| panics.call(())));
    assert!(resumed.is_err());
    Ok(())
}

#[test]
fn from_fn_argument_errors_keep_original() -> Result<()> {
    let notes = |err: &Error| -> Result<Vec<String>> { err.value().attr("__notes__")?.to() };

    // A TypeError from strict conversion keeps its message; context is a note.
    let add = Object::from_fn(|a: i64, b: i64| a + b);
    let err = add.call((1, "x")).unwrap_err();
    assert_eq!(err.type_name(), "TypeError");
    assert_eq!(err.message(), "expected int, got str");
    assert_eq!(notes(&err)?, ["while converting argument 2 of a Rust function"]);
    assert!(format!("{err:#}").contains("while converting argument 2"));

    // Other exception types get the same context and keep their type.
    let small = Object::from_fn(|x: u8| x);
    let err = small.call(300).unwrap_err();
    assert_eq!(err.type_name(), "OverflowError");
    assert_eq!(notes(&err)?, ["while converting argument 1 of a Rust function"]);

    // An exception raised by user code during conversion (here `__index__`)
    // is the very same object, with its traceback.
    let scope = exec(
        "class Boom(Exception):\n    pass\n\
         class I:\n    def __index__(self):\n        raise Boom('index failed')\n\
         i = I()",
    )?;
    scope.set("small", &small)?;
    scope.exec(
        "try:\n    small(i)\nexcept Boom as e:\n    caught = e\n    has_tb = e.__traceback__ is not None",
    )?;
    let caught = scope.get("caught")?;
    assert_eq!(caught.type_name(), "Boom");
    assert_eq!(caught.str()?, "index failed");
    assert!(scope.get("has_tb")?.to::<bool>()?);
    assert_eq!(caught.attr("__notes__")?.to::<Vec<String>>()?, ["while converting argument 1 of a Rust function"]);
    Ok(())
}

#[test]
fn cross_thread() -> Result<()> {
    let list = eval("[1, 2, 3]")?;
    let func = Object::from_fn(|x: i64| x * 2);

    let handles: Vec<_> = (0..4)
        .map(|i| {
            let list = list.clone();
            let func = func.clone();
            std::thread::spawn(move || -> Result<i64> {
                list.method("append", i)?;
                func.call(i)?.to::<i64>()
            })
        })
        .collect();
    let mut results: Vec<i64> = handles.into_iter().map(|h| h.join().unwrap()).collect::<Result<_>>()?;
    results.sort();
    assert_eq!(results, [0, 2, 4, 6]);
    assert_eq!(list.len()?, 7);

    // An object created on another thread and dropped on this one.
    let obj = std::thread::spawn(|| eval("{'from': 'thread'}")).join().unwrap()?;
    assert_eq!(obj.item("from")?.to::<String>()?, "thread");
    drop(obj);

    // An object dropped on a thread without ever attaching there.
    let obj = eval("object()")?;
    std::thread::spawn(move || drop(obj)).join().unwrap();
    Ok(())
}

#[test]
fn import_and_builtins() -> Result<()> {
    let math = import("math")?;
    assert_eq!(math.method("floor", 2.7)?.to::<i64>()?, 2);
    assert!(import("no_such_module_xyz").unwrap_err().is_instance_of("ModuleNotFoundError"));
    assert_eq!(builtins().attr("abs")?.call(-3)?.to::<i64>()?, 3);
    Ok(())
}
