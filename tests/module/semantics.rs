//! Argument binding, error wording, introspection and other details of the
//! generated Python objects.

use crate::run;

#[test]
fn argument_errors_use_cpython_wording() {
    run(r#"
        def error(call):
            try:
                call()
            except TypeError as e:
                return str(e)
            raise AssertionError("no error")

        P = mylib.Point
        p = P(1, 2)
        cases = [
            (lambda: mylib.add(), "add() missing 1 required positional argument: 'a'"),
            (lambda: mylib.many(1), "many() missing 2 required positional arguments: 'b' and 'c'"),
            (lambda: mylib.many(), "many() missing 3 required positional arguments: 'a', 'b', and 'c'"),
            (lambda: mylib.add(1, 2, 3), "add() takes from 1 to 2 positional arguments but 3 were given"),
            (lambda: mylib.count_lines("a", "b"), "count_lines() takes 1 positional argument but 2 were given"),
            (lambda: mylib.nothing(1), "nothing() takes 0 positional arguments but 1 was given"),
            (lambda: mylib.add(1, c=2), "add() got an unexpected keyword argument 'c'"),
            (lambda: mylib.add(1, a=2), "add() got multiple values for argument 'a'"),
            (lambda: p.dist(), "Point.dist() missing 1 required positional argument: 'other'"),
            (lambda: p.dist(p, p), "Point.dist() takes 2 positional arguments but 3 were given"),
            (lambda: p.dist(self=p, other=p), "Point.dist() got an unexpected keyword argument 'self'"),
            (lambda: P.dist(), "unbound method Point.dist() needs an argument"),
            (lambda: P(1), "Point() missing 1 required positional argument: 'y'"),
            (lambda: P(1, 2, 3), "Point() takes 2 positional arguments but 3 were given"),
            (lambda: P.origin(1), "Point.origin() takes 0 positional arguments but 1 was given"),
            (lambda: mylib.geometry.area(), "area() missing 1 required positional argument: 'r'"),
        ]
        for call, expected in cases:
            got = error(call)
            assert got == expected, (got, expected)

        # Conversion errors name the argument and keep the cause.
        try:
            mylib.add(1, b="2")
        except TypeError as e:
            assert str(e) == "argument 'b': expected int, got str", e
            assert e.__cause__ is not None
        try:
            mylib.add(2**70)
        except OverflowError as e:
            assert "while converting argument 'a'" in "".join(e.__notes__), e.__notes__
        try:
            P.dist(1, p)
        except TypeError as e:
            assert str(e) == "argument 'self': expected Point, got int", e
    "#);
}

#[test]
fn names_and_docstrings() {
    run(r#"
        assert mylib.__name__ == "mylib"
        assert mylib.__doc__ == "A test library.", repr(mylib.__doc__)
        assert mylib.geometry.__name__ == "mylib.geometry"
        assert mylib.geometry.__doc__ == "Geometry helpers."
        assert mylib.geometry.solids.__name__ == "mylib.geometry.solids"
        assert mylib.geometry.solids.__doc__ == "Nested submodule."

        f = mylib.add
        assert (f.__name__, f.__qualname__, f.__module__) == ("add", "add", "mylib")
        assert f.__doc__ == "Add two integers."
        assert mylib.nothing.__doc__ is None
        assert repr(f) == "<function add>"
        a = mylib.geometry.area
        assert (a.__qualname__, a.__module__, a.__doc__) == ("area", "mylib.geometry", "Area of a circle.")

        P = mylib.Point
        assert (P.__name__, P.__qualname__, P.__module__) == ("Point", "Point", "mylib")
        assert P.__doc__.startswith("A 2D point."), P.__doc__
        m = P.norm
        assert (m.__name__, m.__qualname__, m.__module__) == ("norm", "Point.norm", "mylib")
        assert m.__doc__ == "Euclidean norm."
        assert P.origin.__qualname__ == "Point.origin"
        assert P(1, 2).norm.__self__ is not None
        assert P(1, 2).norm.__func__ is m
        S = mylib.geometry.Square
        assert S.__module__ == "mylib.geometry"
        assert mylib.geometry.solids.Square is not S
        assert mylib.geometry.solids.Cube.__module__ == "mylib.geometry.solids"
    "#);
}

#[test]
fn inspect_signature_and_help() {
    run(r#"
        import inspect, pydoc
        sig = lambda f: str(inspect.signature(f))
        assert sig(mylib.add) == "(a, b=None)", sig(mylib.add)
        assert sig(mylib.greet) == "(name=None)"
        assert sig(mylib.nothing) == "()"
        assert sig(mylib.Point.dist) == "(self, /, other)", sig(mylib.Point.dist)
        assert sig(mylib.Point(1, 2).dist) == "(other)", sig(mylib.Point(1, 2).dist)
        assert sig(mylib.Point.origin) == "()", sig(mylib.Point.origin)
        assert sig(mylib.Point) == "(x, y)", sig(mylib.Point)
        assert sig(mylib.Tag) == "(name)", sig(mylib.Tag)
        # An alias of `Option` defaults to `None` in the signature as in the binder.
        assert sig(mylib.Span) == "(stop, start=None)", sig(mylib.Span)
        assert mylib.Span(3).start == 0 and mylib.Span(stop=3, start=1).start == 1
        p = inspect.signature(mylib.add).parameters
        assert p["b"].default is None and p["a"].default is inspect.Parameter.empty

        text = pydoc.render_doc(mylib.Point, renderer=pydoc.plaintext)
        assert "norm(self, /)" in text and "Euclidean norm." in text, text
        assert "dist(self, /, other)" in text, text
        assert "origin()" in text, text
        assert "Point(x, y)" in text, text
        assert "Horizontal coordinate." in text, text
        text = pydoc.render_doc(mylib, renderer=pydoc.plaintext)
        assert "add(a, b=None)" in text and "Add two integers." in text, text
    "#);
}

#[test]
fn constructors_keep_pyo3_tp_new() {
    run(r#"
        import pydoc
        P = mylib.Point
        assert P.__doc__ == "A 2D point.\n\nCreates a point from its coordinates.", P.__doc__
        # Without a struct doc, the class doc is the constructor's.
        assert mylib.Span.__doc__ == "Counts from `start` (0 if omitted) to `stop`.", mylib.Span.__doc__
        assert "Creates a point" in pydoc.render_doc(P, renderer=pydoc.plaintext)
        for cls in (P, mylib.Token, mylib.Counter):
            try:
                object.__new__(cls)
            except TypeError as e:
                assert "is not safe" in str(e) or "cannot create" in str(e), e
            else:
                raise AssertionError(f"object.__new__({cls.__name__}) created an instance")
        try:
            mylib.add(**{1: 2})
        except TypeError as e:
            assert str(e) == "keywords must be strings", e
        try:
            P(1, **{2: 3})
        except TypeError as e:
            assert str(e) == "keywords must be strings", e
    "#);
}

#[test]
fn generated_bindings_do_not_capture_user_names() {
    run(r#"
        h = mylib.hygiene
        assert h.m == 1 and h.c == 2 and h.P(3).get() == 6
        assert type(h.inner.make_m()) is h.inner.m and h.inner.make_c().v == 1
        assert mylib.cow_len("abc") == 3
    "#);
}

#[test]
fn functions_pickle_by_reference() {
    run(r#"
        import pickle
        assert pickle.loads(pickle.dumps(mylib.add)) is mylib.add
        assert pickle.loads(pickle.dumps(mylib.geometry.area)) is mylib.geometry.area
    "#);
}

#[test]
fn generated_names_do_not_collide_with_parameters() {
    run(r#"
        assert mylib.scale(2.5) == 5.0
        assert mylib.scale(scale=1) == 2.0
        assert mylib.reserved_names(1, 2, 3, 4, 5, 6, 7) == 28
        assert mylib.reserved_names(py=1, slf=1, args=1, kwargs=1, m=1, c=1, type=1) == 7
        assert mylib._under() == 1
        s = mylib.Shadow()
        assert s.str == "" and s.typing is None and s.int() == 0 and s.list(["a"]) == ["a"]
    "#);
}

#[test]
fn cfg_gated_impl_blocks() {
    run(r#"
        n = mylib.Node(1)
        assert n.enabled() is True
        assert not hasattr(n, "disabled")
    "#);
}

#[test]
fn trait_impls_with_other_types_are_ignored() {
    run(r#"
        m = mylib.Meters(1)
        # `impl PartialEq<f64> for Meters` does not make instances comparable.
        assert m == m and m != mylib.Meters(1)
        hash(m)
    "#);
}

#[test]
fn same_named_types_in_submodules_are_not_classes() {
    run(r#"
        s = mylib.geometry.Segment((0, 0), (1, 2))
        assert s.ends() == ((0.0, 0.0), (1.0, 2.0))
        cube = mylib.geometry.solids.Cube(mylib.geometry.Square(2))
        assert type(cube.face()) is mylib.geometry.solids.Square and cube.face().side == 2.0
        assert cube.corner() == mylib.Point(0, 0)
        assert cube.corner(mylib.Point(1, 1)) == mylib.Point(1, 1)
    "#);
}
