//! Module-level functions, parameters passing and error mapping.

use crate::{mylib, run, scope};

#[test]
fn keyword_arguments_and_defaults() {
    run(r#"
        assert mylib.add(1) == 2
        assert mylib.add(1, 5) == 6
        assert mylib.add(a=1, b=2) == 3
        assert mylib.add(b=10, a=1) == 11
        assert mylib.add(1, None) == 2
        assert mylib.add.__text_signature__ == "(a, b=None)"
    "#);
}

#[test]
fn docstrings() {
    run(r#"
        assert mylib.__doc__ == "A test library.", repr(mylib.__doc__)
        assert mylib.add.__doc__ == "Add two integers.", repr(mylib.add.__doc__)
        assert mylib.geometry.__doc__ == "Geometry helpers."
        assert mylib.geometry.area.__doc__ == "Area of a circle."
        assert mylib.Point.__doc__.startswith("A 2D point."), mylib.Point.__doc__
        assert mylib.Point.x.__doc__ == "Horizontal coordinate."
        assert mylib.Point.norm.__doc__ == "Euclidean norm."
        assert mylib.add.__module__ == "mylib"
        assert mylib.Point.__module__ == "mylib"
        assert mylib.geometry.Square.__module__ == "mylib.geometry"
    "#);
}

#[test]
fn strict_conversions() {
    run(r#"
        for bad in (True, 1.5, "1", [1]):
            try:
                mylib.add(bad)
            except TypeError as e:
                assert str(e).startswith("argument 'a': expected int"), e
            else:
                raise AssertionError(f"add({bad!r}) accepted")
        try:
            mylib.add(2**70)
        except OverflowError:
            pass
        else:
            raise AssertionError("overflow accepted")
        try:
            mylib.add(1, b=False)
        except TypeError as e:
            assert "argument 'b'" in str(e), e
        else:
            raise AssertionError("bool accepted for b")
    "#);
}

#[test]
fn constants_and_private_items() {
    run(r#"
        assert mylib.VERSION == "1.2.3" and mylib.LIMIT == 10
        for name in ("helper", "crate_only", "MyError", "private_mod", "internal"):
            assert not hasattr(mylib, name), name
        assert not hasattr(mylib.Point, "internal")
        assert not hasattr(mylib.Point(1, 2), "cache")
        assert not hasattr(mylib.Point, "new")
    "#);
}

#[test]
fn error_mapping() {
    run(r#"
        try:
            mylib.read_text("definitely/not/here.txt")
        except FileNotFoundError:
            pass
        else:
            raise AssertionError("no error")
        try:
            mylib.fail("x")
        except LookupError as e:
            assert str(e) == "my error: x", e
        else:
            raise AssertionError("no error")
    "#);
}

#[test]
fn panics_raise() {
    run(r#"
        try:
            mylib.boom()
        except BaseException as e:
            assert type(e).__name__ == "PanicException", type(e)
            assert "boom from Rust" in str(e)
        else:
            raise AssertionError("no error")
    "#);
}

#[test]
fn shared_borrow_parameters() {
    run(r#"
        import pathlib
        assert mylib.count_lines("a\nb\nc") == 3
        assert mylib.total([1.5, 2, 3]) == 6.5
        assert mylib.total((1.0, 2.0)) == 3.0
        assert mylib.file_name(pathlib.Path("x") / "y.txt") == "y.txt"
        assert mylib.file_name("a/b.rs") == "b.rs"
        assert mylib.lookup({"a": 1}, "a") == 1
        assert mylib.lookup({"a": 1}, key="b") is None
        assert mylib.greet() == "hello world"
        assert mylib.greet("you") == "hello you"
        assert mylib.greet(None) == "hello world"
        assert mylib.first_word("  hi there") == "hi"
        assert mylib.nothing() is None
        assert mylib.nested_sum([{"a": [1, 2]}, {"b": [3]}]) == 6
        assert mylib.many(1, 2, 3) == 1230 and mylib.many(1, 2, c=3, d=4) == 1234
        try:
            mylib.count_lines(b"bytes")
        except TypeError as e:
            assert "argument 'text'" in str(e), e
        else:
            raise AssertionError("bytes accepted as &str")
    "#);
}

#[test]
fn values_are_copies_and_handles_are_references() {
    run(r#"
        # By value is a copy, in both directions; the returned list is a new,
        # mutable list owned by the caller.
        ys = [1]
        out = mylib.copy_back(ys)
        assert out == [1] and out is not ys and type(out) is list
        out.append(2)
        assert ys == [1]

        # A `List<i64>` parameter is the caller's list itself.
        zs = [0]
        same = zs
        mylib.push_handle(zs, 2)
        assert zs == [0, 2, 2] and zs is same
        try:
            mylib.push_handle((0,), 1)
        except TypeError as e:
            assert str(e).startswith("argument 'xs': expected list"), e
        else:
            raise AssertionError("tuple accepted as a List")

        # Maps convert from any mapping; sets from a set or frozenset.
        import types
        assert mylib.lookup(types.MappingProxyType({"a": 1}), "a") == 1
        try:
            mylib.lookup([("a", 1)], "a")
        except TypeError as e:
            assert str(e).startswith("argument 'map': expected a mapping, got list"), e
        else:
            raise AssertionError("list accepted as a mapping")
    "#);
}

#[test]
fn submodule() {
    run(r#"
        import math, sys
        assert abs(mylib.geometry.area(1) - math.pi) < 1e-12
        from mylib import geometry
        assert geometry is mylib.geometry
        assert sys.modules["mylib.geometry"] is geometry
        import mylib.geometry as g
        assert g.Square(2).side == 2
    "#);
}

#[test]
fn items_remain_rust_callable() {
    assert_eq!(mylib::add(1, None), 2);
    assert_eq!(mylib::add(1, Some(2)), 3);
    assert_eq!(mylib::count_lines("a\nb"), 2);
    assert_eq!(mylib::copy_back(vec![1, 3]), [1, 3]);
    assert_eq!(mylib::geometry::area(0.0), 0.0);
    assert_eq!(mylib::crate_only(), 7);
    let p = mylib::Point::new(3.0, 4.0);
    assert_eq!(p.norm(), 5.0);
    assert_eq!(mylib::Point::DIMENSIONS, 2);
}

#[test]
fn module_object_is_cached_and_registered() {
    let a = mylib::__orochi_module().unwrap();
    let b = mylib::__orochi_module().unwrap();
    assert!(a.is(&b));
    let s = scope();
    assert!(s.eval("mylib").unwrap().is(&a));
}
