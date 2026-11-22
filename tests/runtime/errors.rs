use orochi::prelude::*;
use orochi::{eval, exec};

#[test]
fn python_exception_details() {
    let scope = exec("def inner():\n    return 1 / 0\n\ndef outer():\n    return inner()").unwrap();
    let err = scope.get("outer").unwrap().call(()).unwrap_err();

    assert_eq!(err.type_name(), "ZeroDivisionError");
    assert_eq!(err.message(), "division by zero");
    assert!(err.is_instance_of("ZeroDivisionError"));
    assert!(err.is_instance_of("ArithmeticError"));
    assert!(err.is_instance_of("Exception"));
    assert!(!err.is_instance_of("ValueError"));
    assert!(err.is_instance(&orochi::builtins().attr("ZeroDivisionError").unwrap()));

    let tb = err.traceback().expect("raised exceptions have a traceback");
    assert!(tb.starts_with("Traceback (most recent call last):"), "{tb}");
    assert!(tb.contains("in inner"), "{tb}");
    assert!(tb.contains("in outer"), "{tb}");

    // Display: one line. Alternate: full rendering.
    assert_eq!(err.to_string(), "ZeroDivisionError: division by zero");
    let full = format!("{err:#}");
    assert!(full.starts_with("Traceback (most recent call last):"), "{full}");
    assert!(full.contains("in inner"), "{full}");
    assert!(full.ends_with("ZeroDivisionError: division by zero"), "{full}");

    let debug = format!("{err:?}");
    assert!(debug.contains("ZeroDivisionError") && debug.contains("division by zero"), "{debug}");
    assert!(debug.contains("in inner"), "{debug}");

    // The exception object itself.
    let value = err.value();
    assert_eq!(value.type_name(), "ZeroDivisionError");
    assert_eq!(value.attr("args").unwrap().to::<(String,)>().unwrap().0, "division by zero");
}

#[test]
fn display_variants() {
    // Chained exceptions appear only in the alternate form.
    let err = exec(
        "try:\n    {}['k']\nexcept KeyError as e:\n    raise ValueError('outer') from e",
    )
    .unwrap_err();
    assert_eq!(err.to_string(), "ValueError: outer");
    let full = format!("{err:#}");
    assert!(full.contains("KeyError: 'k'"), "{full}");
    assert!(full.contains("direct cause"), "{full}");

    // Empty message: just the type, like Python.
    let err = exec("raise StopIteration").unwrap_err();
    assert_eq!(err.to_string(), "StopIteration");

    // Never raised: alternate equals the short form.
    let err = Error::value_error("v");
    assert_eq!(format!("{err:#}"), "ValueError: v");
}

#[test]
fn rust_created_errors() {
    let cases = [
        (Error::value_error("v"), "ValueError"),
        (Error::type_error("t"), "TypeError"),
        (Error::runtime_error("r"), "RuntimeError"),
        (Error::key_error("k"), "KeyError"),
        (Error::index_error("i"), "IndexError"),
        (Error::attribute_error("a"), "AttributeError"),
        (Error::overflow_error("o"), "OverflowError"),
        (Error::new("NotImplementedError", "n"), "NotImplementedError"),
        (Error::new("UnicodeError", "u"), "UnicodeError"),
    ];
    for (err, name) in cases {
        assert_eq!(err.type_name(), name);
        assert!(err.is_instance_of(name));
        assert!(err.traceback().is_none());
        assert_eq!(err.to_string(), format!("{name}: {}", err.message()));
    }

    // KeyError's str() quotes the key, like Python.
    assert_eq!(Error::key_error("k").message(), "'k'");

    // Dotted path to an exception class in a module (imported on demand).
    let err = Error::new("concurrent.futures.CancelledError", "stop");
    assert_eq!(err.type_name(), "CancelledError");
    assert_eq!(err.message(), "stop");
    assert!(err.is_instance_of("concurrent.futures.CancelledError"));
}

#[test]
fn unknown_exception_name_is_type_error() {
    let err = Error::new("DefinitelyNotAnException", "x");
    assert_eq!(err.to_string(), "TypeError: 'DefinitelyNotAnException' is not an exception type");
    // Not a class, a class that is not an exception, a missing module.
    for name in ["len", "int", "no_such_module_xyz.Error"] {
        let err = Error::new(name, "x");
        assert_eq!(err.message(), format!("'{name}' is not an exception type"));
    }
}

#[test]
fn exception_needing_more_arguments_is_type_error() -> Result<()> {
    // JSONDecodeError(msg, doc, pos) needs three arguments; it used to turn
    // silently into a TypeError about the missing arguments.
    let err = Error::new("json.JSONDecodeError", "boom");
    assert_eq!(
        err.to_string(),
        "TypeError: exception type 'json.JSONDecodeError' cannot be constructed from a single message argument"
    );
    // The construction error is the cause.
    let cause = err.value().attr("__cause__")?;
    assert_eq!(cause.type_name(), "TypeError");
    assert!(cause.str()?.contains("missing"), "{cause}");

    let cls = orochi::builtins().attr("UnicodeDecodeError")?;
    let err = Error::from_type(&cls, "boom");
    assert!(err.is_instance_of("TypeError"));
    assert!(err.message().contains("'UnicodeDecodeError' cannot be constructed"), "{err}");
    assert_eq!(err.value().attr("__cause__")?.type_name(), "TypeError");
    Ok(())
}

#[test]
fn from_value_for_complex_exceptions() -> Result<()> {
    let cls = orochi::import("json")?.attr("JSONDecodeError")?;
    let value = cls.call(("bad", "doc", 1))?;
    let err = Error::from_value(&value);
    assert_eq!(err.type_name(), "JSONDecodeError");
    assert!(err.is_instance_of("ValueError"));
    assert!(err.is_instance_of("json.JSONDecodeError"));
    assert!(Error::from_value(&eval("1")?).is_instance_of("TypeError"));
    Ok(())
}

#[test]
fn is_instance_of_non_exception_is_false() {
    let err = Error::value_error("x");
    // Misspelled, not a class, a class that is not an exception.
    for name in ["ValueErorr", "len", "int", "builtins.int"] {
        assert!(!err.is_instance_of(name), "{name}");
    }
}

#[test]
fn is_instance_of_does_not_import() -> Result<()> {
    let modules = orochi::import("sys")?.attr("modules")?;
    // A stdlib module that nothing in this test binary imports.
    assert!(!modules.contains("xml.dom.minidom")?);
    assert!(!Error::value_error("x").is_instance_of("xml.dom.minidom.NoSuchError"));
    assert!(!modules.contains("xml.dom.minidom")?, "is_instance_of imported a module");

    // Loaded module, unknown attribute or not an exception class.
    orochi::import("json")?;
    assert!(!Error::value_error("x").is_instance_of("json.JSONDecodeErorr"));
    assert!(!Error::value_error("x").is_instance_of("json.JSONDecoder"));
    Ok(())
}

#[test]
fn custom_exception_class() -> Result<()> {
    let scope = exec("class MyError(ValueError):\n    pass")?;
    let cls = scope.get("MyError")?;
    let err = Error::from_type(&cls, "custom");
    assert_eq!(err.type_name(), "MyError");
    assert!(err.is_instance(&cls));
    assert!(err.is_instance_of("ValueError"));

    // Not an exception class -> TypeError.
    let err = Error::from_type(&orochi::builtins().attr("int")?, "x");
    assert!(err.is_instance_of("TypeError"));
    Ok(())
}

#[test]
fn errors_raise_into_python() -> Result<()> {
    let scope = orochi::Scope::new();
    scope.set(
        "fail",
        Object::from_fn(|| -> Result<()> { Err(Error::new("PermissionError", "denied")) }),
    )?;
    scope.exec(
        "try:\n    fail()\nexcept PermissionError as e:\n    caught = str(e)\nelse:\n    caught = None",
    )?;
    assert_eq!(scope.get("caught")?.to::<String>()?, "denied");
    Ok(())
}

#[test]
fn std_error_mapping() {
    use std::io::{Error as IoError, ErrorKind};
    let err: Error = IoError::new(ErrorKind::NotFound, "gone").into();
    assert!(err.is_instance_of("FileNotFoundError"));
    assert!(err.is_instance_of("OSError"));

    let err: Error = IoError::new(ErrorKind::PermissionDenied, "no").into();
    assert!(err.is_instance_of("PermissionError"));

    let err: Error = IoError::other("misc").into();
    assert_eq!(err.type_name(), "OSError");

    let real = std::fs::read("this/file/does/not/exist").unwrap_err();
    let err: Error = real.into();
    assert!(err.is_instance_of("FileNotFoundError"));

    let err: Error = "x".parse::<i32>().unwrap_err().into();
    assert!(err.is_instance_of("ValueError"));
    let err: Error = "x".parse::<f64>().unwrap_err().into();
    assert!(err.is_instance_of("ValueError"));
    let err: Error = u8::try_from(300i32).unwrap_err().into();
    assert!(err.is_instance_of("ValueError"));
    let err: Error = "maybe".parse::<bool>().unwrap_err().into();
    assert!(err.is_instance_of("ValueError"));
    let err: Error = "ab".parse::<char>().unwrap_err().into();
    assert!(err.is_instance_of("ValueError"));
    let err: Error = <[u8; 2]>::try_from(&[1u8][..]).unwrap_err().into();
    assert!(err.is_instance_of("ValueError"));
    let err: Error = std::ffi::CString::new("a\0b").unwrap_err().into();
    assert!(err.is_instance_of("ValueError"));

    let bad = vec![0xffu8, 0xfe];
    let err: Error = String::from_utf8(bad.clone()).unwrap_err().into();
    assert!(err.is_instance_of("UnicodeDecodeError"));
    let err: Error = std::str::from_utf8(&bad).unwrap_err().into();
    assert!(err.is_instance_of("ValueError"));

    let err: Error = std::fmt::Error.into();
    assert!(err.is_instance_of("RuntimeError"));
}

#[test]
fn pyerr_round_trip_and_std_error() {
    let err = eval("[][0]").unwrap_err();
    let pyerr: orochi::pyo3::PyErr = err.into();
    let back: Error = pyerr.into();
    assert!(back.is_instance_of("IndexError"));

    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(back);
    assert_eq!(boxed.to_string(), "IndexError: list index out of range");

    // `?` works in functions returning other error types via Box<dyn Error>.
    fn uses_question_mark() -> std::result::Result<i64, Box<dyn std::error::Error>> {
        Ok(eval("int('x')")?.to::<i64>()?)
    }
    assert!(uses_question_mark().unwrap_err().to_string().contains("ValueError"));
}

#[test]
fn syntax_error() {
    let err = eval("1 +").unwrap_err();
    assert!(err.is_instance_of("SyntaxError"));
    let err = exec("x = \0").unwrap_err();
    assert!(err.is_instance_of("ValueError"));
}
