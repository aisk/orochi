use std::collections::{HashMap, HashSet};

use orochi::prelude::*;
use orochi::{eval, exec};

#[test]
fn list_basics() -> Result<()> {
    let l: List<i64> = List::new();
    assert!(l.is_empty());
    l.push(1)?;
    l.push(2i32)?; // anything Into<i64>
    l.insert(0, 0)?;
    assert_eq!(l.len(), 3);
    assert_eq!(l.to_vec()?, [0, 1, 2]);
    assert_eq!(l.get(2)?, 2);
    l.set(2, 20)?;
    assert_eq!(l.get(2)?, 20);
    assert!(l.get(3).unwrap_err().is_instance_of("IndexError"));
    assert!(l.set(9, 0).unwrap_err().is_instance_of("IndexError"));
    assert_eq!(l.iter()?.collect::<Result<Vec<_>>>()?, [0, 1, 20]);
    assert_eq!(l.pop()?, Some(20));
    l.clear()?;
    assert_eq!(l.pop()?, None);

    let l = List::from_vec(vec!["a".to_owned()])?;
    assert_eq!(l.to_string(), "['a']");
    assert_eq!(format!("{l:?}"), "['a']");
    Ok(())
}

#[test]
fn list_reference_semantics() -> Result<()> {
    let scope = exec("xs = [1, 2]")?;
    let xs: List<i64> = scope.get("xs")?.to()?;

    // Rust -> Python.
    xs.push(3)?;
    assert_eq!(scope.eval("xs")?.to::<Vec<i64>>()?, [1, 2, 3]);

    // Python -> Rust.
    scope.exec("xs.append(4)")?;
    assert_eq!(xs.len(), 4);

    // Another handle to the same list.
    let alias: List<i64> = scope.get("xs")?.to()?;
    alias.set(0, 100)?;
    assert_eq!(xs.get(0)?, 100);
    assert!(xs.is(&alias));
    assert!(xs.as_object().is(&scope.get("xs")?));

    // Passing the handle to Python passes the same object.
    let append = scope.eval("lambda l: l.append(5)")?;
    append.call(&xs)?;
    assert_eq!(alias.len(), 5);
    let obj: Object = xs.clone().into();
    assert!(obj.is(alias.as_object()));
    assert!(xs.into_inner().is(&obj));
    Ok(())
}

#[test]
fn list_type_checks() -> Result<()> {
    let err = eval("(1, 2)")?.to::<List<i64>>().unwrap_err();
    assert!(err.is_instance_of("TypeError"));
    assert_eq!(err.message(), "expected list, got tuple");
    assert!(eval("'ab'")?.to::<List<String>>().is_err());

    // Elements are converted lazily, on access.
    let l: List<i64> = eval("[1, 'two']")?.to()?;
    assert_eq!(l.get(0)?, 1);
    assert!(l.get(1).unwrap_err().is_instance_of("TypeError"));
    assert!(l.to_vec().is_err());

    // list subclasses are accepted.
    let scope = exec("class L(list):\n    pass\nl = L([1])")?;
    assert_eq!(scope.get("l")?.to::<List<i64>>()?.len(), 1);
    Ok(())
}

#[test]
fn dict_basics_and_references() -> Result<()> {
    let scope = exec("d = {'a': 1}")?;
    let d: Dict<String, i64> = scope.get("d")?.to()?;
    assert_eq!(d.get("a")?, Some(1));
    assert_eq!(d.get("b")?, None);
    assert!(d.contains("a")?);

    d.insert("b", 2)?;
    assert_eq!(scope.eval("d['b']")?.to::<i64>()?, 2);
    scope.exec("d['c'] = 3")?;
    assert_eq!(d.len(), 3);
    assert_eq!(d.keys()?, ["a", "b", "c"]);
    assert_eq!(d.values()?, [1, 2, 3]);
    assert_eq!(d.items()?[2], ("c".to_owned(), 3));
    assert_eq!(d.to_hashmap()?, HashMap::from([("a".into(), 1), ("b".into(), 2), ("c".into(), 3)]));

    let alias = d.clone();
    assert_eq!(alias.remove("a")?, Some(1));
    assert_eq!(alias.remove("a")?, None);
    assert!(!d.contains("a")?);
    assert!(!scope.eval("'a' in d")?.to::<bool>()?);

    d.clear()?;
    assert!(alias.is_empty());

    // Typed value access is strict.
    scope.exec("d['x'] = 1.5")?;
    assert!(d.get("x").unwrap_err().is_instance_of("TypeError"));

    assert!(eval("[]")?.to::<Dict<String, i64>>().unwrap_err().is_instance_of("TypeError"));
    Ok(())
}

#[test]
fn dict_with_object_values() -> Result<()> {
    let d: Dict<String, Object> = Dict::default();
    d.insert("list", eval("[1]")?)?;
    let inner = d.get("list")?.unwrap();
    inner.method("append", (2,))?;
    assert_eq!(d.get("list")?.unwrap().to::<Vec<i64>>()?, [1, 2]);
    Ok(())
}

#[test]
fn set_basics_and_references() -> Result<()> {
    let scope = exec("s = {1}")?;
    let s: Set<i64> = scope.get("s")?.to()?;
    assert!(s.contains(1)?);
    s.add(2)?;
    assert!(scope.eval("2 in s")?.to::<bool>()?);
    scope.exec("s.add(3)")?;
    assert_eq!(s.len(), 3);
    assert_eq!(s.to_hashset()?, HashSet::from([1, 2, 3]));
    let mut v = s.to_vec()?;
    v.sort();
    assert_eq!(v, [1, 2, 3]);
    assert!(s.remove(1)?);
    assert!(!s.remove(1)?);
    s.clear()?;
    assert!(s.is_empty());

    // frozenset is immutable and therefore not a Set handle.
    assert!(eval("frozenset()")?.to::<Set<i64>>().unwrap_err().is_instance_of("TypeError"));
    Ok(())
}

#[test]
fn handles_are_send_sync() -> Result<()> {
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<Object>();
    assert_send_sync::<List<i64>>();
    assert_send_sync::<Dict<String, Object>>();
    assert_send_sync::<Set<String>>();
    assert_send_sync::<Error>();
    assert_send_sync::<Scope>();

    let l: List<i64> = List::new();
    let l2 = l.clone();
    std::thread::spawn(move || l2.push(1)).join().unwrap()?;
    assert_eq!(l.to_vec()?, [1]);
    Ok(())
}
