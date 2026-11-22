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
        try:
            mylib.add()
        except TypeError as e:
            assert "'a'" in str(e), e
        else:
            raise AssertionError("missing argument accepted")
        try:
            mylib.add(1, c=2)
        except TypeError:
            pass
        else:
            raise AssertionError("unknown keyword accepted")
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
        assert mylib.VERSION == "1.2.3"
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
        except RuntimeError as e:
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
        try:
            mylib.count_lines(b"bytes")
        except TypeError as e:
            assert "argument 'text'" in str(e), e
        else:
            raise AssertionError("bytes accepted as &str")
    "#);
}

#[test]
fn mut_vec_writes_back_in_place() {
    run(r#"
        xs = [1]
        alias = xs
        assert mylib.push_twice(xs, 5) is None
        assert xs == [1, 5, 5], xs
        assert xs is alias

        # By value is a copy.
        ys = [1]
        assert mylib.copy_back(ys) == [1]
        assert mylib.copy_back(ys) is not ys

        # The handle type also works in place.
        zs = [0]
        mylib.push_handle(zs, 2)
        assert zs == [0, 2, 2]
    "#);
}

#[test]
fn mut_vec_writes_back_on_error() {
    run(r#"
        xs = [1, 2]
        try:
            mylib.push_checked(xs, 3, limit=2)
        except ValueError:
            pass
        else:
            raise AssertionError("no error")
        assert xs == [1, 2, 3], xs
        mylib.push_checked(xs, 4, 10)
        assert xs == [1, 2, 3, 4]
    "#);
}

#[test]
fn mut_vec_requires_a_list() {
    run(r#"
        for bad in ((1, 2), range(3), "ab"):
            try:
                mylib.push_twice(bad, 1)
            except TypeError as e:
                assert str(e).startswith("argument 'xs': expected list"), e
            else:
                raise AssertionError(f"{bad!r} accepted")
        try:
            mylib.push_twice(["a"], 1)
        except TypeError as e:
            assert "argument 'xs'" in str(e), e
        else:
            raise AssertionError("bad element accepted")
    "#);
}

#[test]
fn mut_maps_and_sets_write_back_in_place() {
    run(r#"
        d = {"a": 1}
        alias = d
        mylib.bump(d, "a")
        mylib.bump(d, "b")
        assert d == {"a": 2, "b": 1} and d is alias

        m = {3: "3"}
        mylib.sorted_insert(m, 1)
        assert m == {1: "1", 3: "3"}

        s = {"x"}
        alias = s
        assert mylib.add_tag(s, "y") is True
        assert mylib.add_tag(s, "y") is False
        assert s == {"x", "y"} and s is alias

        for bad in (frozenset(), [], {}):
            try:
                mylib.add_tag(bad, "z")
            except TypeError as e:
                assert "expected set" in str(e), e
            else:
                raise AssertionError(f"{bad!r} accepted")
        try:
            mylib.bump([], "a")
        except TypeError as e:
            assert "expected dict" in str(e), e
        else:
            raise AssertionError("list accepted as dict")
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
    let mut xs = vec![1];
    mylib::push_twice(&mut xs, 3);
    assert_eq!(xs, [1, 3, 3]);
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
