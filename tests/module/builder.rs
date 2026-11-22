//! A module written by hand with the builder API, without the macro.

use orochi::pyo3::prelude::*;
use orochi::pyo3::types::{PyDict, PyTuple};
use orochi::{ClassBuilder, List, ModuleBuilder, Result};

/// A bank account.
#[pyclass(crate = "orochi::pyo3", module = "bank")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Account {
    owner: String,
    cents: i64,
}

impl std::fmt::Display for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.owner, self.cents)
    }
}

#[pymethods(crate = "orochi::pyo3")]
impl Account {
    #[new]
    #[pyo3(signature = (*args, **kwargs), text_signature = "(owner)")]
    fn py_new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        orochi::construct(&["owner"], |owner: String| Account { owner, cents: 0 }, args, kwargs)
    }
}

/// Not Clone, with a constructor that can fail.
#[pyclass(crate = "orochi::pyo3", module = "bank")]
struct Vault;

#[pymethods(crate = "orochi::pyo3")]
impl Vault {
    #[new]
    #[pyo3(signature = (*args, **kwargs))]
    fn py_new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        orochi::construct(&["code"], |code: i64| -> Result<Vault> {
            if code == 42 { Ok(Vault) } else { Err(orochi::Error::value_error("wrong code")) }
        }, args, kwargs)
    }
}

fn init(m: &ModuleBuilder<'_>) -> Result<()> {
    m.doc("Built by hand.")?;
    m.constant("ANSWER", 42)?;
    // Closures work as well as functions.
    let offset = 10;
    m.function("shift", &["x"], "Adds ten.", move |x: i64| x + offset)?;
    m.function("fill", &["xs", "n"], "", |xs: List<i64>, n: Option<usize>| -> Result<()> {
        for i in 0..n.unwrap_or(2) {
            xs.push(i as i64)?;
        }
        Ok(())
    })?;
    let c: ClassBuilder<'_, Account> = m.class::<Account>()?;

    c.method("deposit", &["cents"], "Adds money.", |a: &mut Account, cents: i64| -> Result<()> {
        if cents <= 0 {
            return Err(orochi::Error::value_error("deposit must be positive"));
        }
        a.cents += cents;
        Ok(())
    })?;
    c.method("transfer", &["to", "cents"], "", |from: &mut Account, to: &mut Account, cents: i64| {
        from.cents -= cents;
        to.cents += cents;
    })?;
    c.static_method("bank", &[], "", || "orochi bank")?;
    c.constant("CURRENCY", "EUR")?;
    c.readonly("cents", "Balance in cents.", |a: &Account| a.cents)?;
    c.property(
        "owner",
        "",
        |a: &Account| a.owner.clone(),
        |a: &mut Account, owner: String| a.owner = owner,
    )?;
    c.repr_debug()?.str_display()?.eq()?.hash()?;
    // Chaining from the temporary, and binding the result.
    let v = m.class::<Vault>()?.static_method("hint", &[], "", || 42)?;
    v.constant("NAME", "vault")?;
    m.submodule("util", "Helpers.", |sub| {
        sub.function("double", &["x"], "", |x: f64| x * 2.0)?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn hand_written_module() {
    orochi::load_module("bank", init).unwrap();
    crate::run(
        r#"
        import bank, inspect
        assert bank.__doc__ == "Built by hand." and bank.ANSWER == 42
        assert bank.shift(1) == 11 and bank.shift.__doc__ == "Adds ten."
        xs = []
        bank.fill(xs)
        bank.fill(xs, n=1)
        assert xs == [0, 1, 0]
        A = bank.Account
        assert A.__doc__ == "A bank account." and A.__module__ == "bank"
        assert str(inspect.signature(A)) == "(owner)"
        a, b = A("ann"), A("bob")
        a.deposit(100)
        a.transfer(b, 30)
        assert (a.cents, b.cents) == (70, 30)
        try:
            a.transfer(a, 1)
        except RuntimeError:
            pass
        else:
            raise AssertionError("conflicting borrow accepted")
        try:
            a.deposit(0)
        except ValueError as e:
            assert str(e) == "deposit must be positive"
        try:
            a.cents = 5
        except AttributeError:
            pass
        else:
            raise AssertionError("read-only property assigned")
        a.owner = "amy"
        assert str(a) == "amy: 70" and repr(a) == 'Account { owner: "amy", cents: 70 }'
        assert a == A("amy") or a != A("amy")
        assert hash(A("x")) == hash(A("x"))
        assert A.bank() == "orochi bank" and A.CURRENCY == "EUR"
        assert A.deposit.__qualname__ == "Account.deposit"
        assert str(inspect.signature(A.transfer)) == "(self, /, to, cents)"
        import bank.util
        assert bank.util.double(2) == 4.0 and bank.util.__doc__ == "Helpers."
        assert bank.Vault(code=42) is not None and bank.Vault.hint() == 42 and bank.Vault.NAME == "vault"
        try:
            bank.Vault(1)
        except ValueError as e:
            assert str(e) == "wrong code"
        try:
            bank.Vault()
        except TypeError as e:
            assert str(e) == "Vault() missing 1 required positional argument: 'code'", e
        try:
            object.__new__(bank.Account)
        except TypeError as e:
            assert "is not safe" in str(e), e
        else:
            raise AssertionError("object.__new__ created an instance")
    "#,
    );
}

/// Only used to check build-time errors.
#[pyclass(crate = "orochi::pyo3")]
struct Probe;

#[test]
fn bad_signatures_fail_at_build_time() {
    let message = |init: fn(&ModuleBuilder<'_>) -> Result<()>| orochi::load_module("bad", init).unwrap_err().message();
    let err = message(|m| {
        m.function("f", &["a", "a"], "", |a: i64, b: i64| a + b)?;
        Ok(())
    });
    assert!(err.contains("duplicate parameter name 'a'"), "{err}");
    let err = message(|m| {
        m.class::<Probe>()?.method("m", &["x"], "", |x: i64| x)?;
        Ok(())
    });
    assert!(err.contains("Probe.m: the first parameter of a method must be `&Probe`"), "{err}");
    let err = message(|m| {
        m.class::<Probe>()?.method("m", &["other"], "", |x: Option<&Probe>| x.is_some())?;
        Ok(())
    });
    assert!(err.contains("first parameter of a method"), "{err}");
}

#[test]
fn parameter_name_count_must_match() {
    let err = orochi::load_module("bad_arity", |m| {
        m.function("f", &["a"], "", |a: i64, b: i64| a + b)?;
        Ok(())
    })
    .unwrap_err();
    assert!(err.is_instance_of("TypeError"), "{err}");
    assert!(err.message().contains("1 parameter names given for a Rust function with 2 parameters"), "{err}");
}
