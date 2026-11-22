use orochi::prelude::*;
use orochi::{eval, exec};

#[test]
fn eval_uses_fresh_scope() -> Result<()> {
    assert_eq!(eval("1 + 2")?.to::<i64>()?, 3);
    exec("leaked = 1")?;
    assert!(eval("leaked").unwrap_err().is_instance_of("NameError"));
    // Builtins are available.
    assert_eq!(eval("len('abcd')")?.to::<i64>()?, 4);
    Ok(())
}

#[test]
fn exec_returns_namespace() -> Result<()> {
    let scope = exec(
        "import math\n\
         from dataclasses import dataclass\n\
         @dataclass\n\
         class P:\n    x: int\n\
         p = P(3)\n\
         r = math.sqrt(p.x ** 2)",
    )?;
    assert_eq!(scope.get("r")?.to::<f64>()?, 3.0);
    assert_eq!(scope.get("p")?.attr("x")?.to::<i64>()?, 3);
    assert!(scope.contains("math")?);
    assert!(!scope.contains("nope")?);
    let err = scope.get("nope").unwrap_err();
    assert!(err.is_instance_of("NameError"));
    assert_eq!(err.message(), "name 'nope' is not defined");
    Ok(())
}

#[test]
fn indented_source_is_dedented() -> Result<()> {
    let scope = exec(
        r#"
        def f(x):
            if x:
                return "yes"
            return "no"

        result = f(1)
        "#,
    )?;
    assert_eq!(scope.get("result")?.to::<String>()?, "yes");

    scope.exec(
        "
            more = f(0)
        ",
    )?;
    assert_eq!(scope.get("more")?.to::<String>()?, "no");

    let v = eval(
        r#"
            [i * 2
             for i in range(3)]
        "#,
    )?;
    assert_eq!(v.to::<Vec<i64>>()?, [0, 2, 4]);
    assert_eq!(scope.eval("\n\n    more + '!'\n\n")?.to::<String>()?, "no!");

    // Genuinely inconsistent indentation is still an error.
    assert!(exec("  a = 1\n b = 2").unwrap_err().is_instance_of("IndentationError"));
    Ok(())
}

#[test]
fn scope_state() -> Result<()> {
    let scope = Scope::new();
    scope.set("xs", vec![1, 2, 3])?;
    scope.exec("total = sum(xs)")?;
    assert_eq!(scope.get("total")?.to::<i64>()?, 6);
    assert_eq!(scope.eval("total * 2")?.to::<i64>()?, 12);

    let same = scope.clone();
    same.exec("total = 0")?;
    assert_eq!(scope.get("total")?.to::<i64>()?, 0);
    assert_eq!(scope.globals().get("total")?.unwrap().to::<i64>()?, 0);
    Ok(())
}

#[test]
fn kwargs_macro() -> Result<()> {
    let kw = kwargs! { "a" => 1, "b" => "x", "c" => vec![1.5], };
    assert_eq!(kw.len(), 3);
    assert!(orochi::kwargs! {}.is_empty());

    let key = String::from("dyn");
    let d = orochi::builtins().attr("dict")?.call_kw((), kwargs! { key => None::<i64> })?;
    assert!(d.item("dyn")?.is_none());

    // A conversion error is reported by the call.
    let mut bad = std::collections::HashMap::new();
    bad.insert(vec![1], 1);
    let err = orochi::builtins().attr("dict")?.call_kw((), kwargs! { "m" => bad }).unwrap_err();
    assert!(err.is_instance_of("TypeError"));
    Ok(())
}
