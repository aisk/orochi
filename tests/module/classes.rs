//! Classes: construction, methods, special methods, borrowing and `Ref<T>`
//! identity.

use orochi::Ref;

use crate::{mylib, run, scope};

#[test]
fn construction_methods_and_properties() {
    run(r#"
        P = mylib.Point
        p = P(3, 4)
        assert type(p) is P and isinstance(p, P)
        assert p.norm() == 5.0
        assert P(x=1, y=2).x == 1.0
        p.x = 6
        assert p.x == 6.0 and isinstance(p.x, float)
        assert p.scale(0.5) is None
        assert (p.x, p.y) == (3.0, 2.0)
        o = P.origin()
        assert (o.x, o.y) == (0.0, 0.0) and type(o) is P
        assert p.origin().x == 0.0         # static methods work on instances too
        assert p.dist(P(3, 0)) == 2.0
        assert p.dist(other=P(3, 0)) == 2.0
        assert P.dist(p, P(3, 0)) == 2.0   # unbound call
        assert P.DIMENSIONS == 2
        assert type(P.UNIT) is P and P.UNIT.x == 1.0
        assert p.cached_norm() == p.norm()
        try:
            P(True, 1)
        except TypeError as e:
            assert str(e).startswith("argument 'x': expected float, got bool"), e
        else:
            raise AssertionError("bool accepted")
        try:
            p.checked_scale(0)
        except ValueError as e:
            assert str(e) == "k must not be zero"
        else:
            raise AssertionError("no error")
    "#);
}

#[test]
fn class_return_values() {
    run(r#"
        P = mylib.Point
        assert type(mylib.maybe_point(True)) is P
        assert mylib.maybe_point(False) is None
        assert mylib.parse_point("1, 2") == P(1, 2)
        try:
            mylib.parse_point("x")
        except ValueError as e:
            assert "not a point" in str(e)
        else:
            raise AssertionError("no error")
        t = mylib.token(5)
        assert type(t).__name__ == "Token" and t.id == 5
        try:
            mylib.Token(1)
        except TypeError as e:
            assert str(e) == "cannot create 'mylib.Token' instances", e
        else:
            raise AssertionError("class without `new` constructed")
        s = mylib.unit_square()
        assert type(s) is mylib.geometry.Square and s.side == 1.0
    "#);
}

#[test]
fn special_methods() {
    run(r#"
        P = mylib.Point
        p = P(6, 4)
        assert p == P(6, 4)
        assert p != P(6, 5)
        assert (p == 3) is False
        assert p.__eq__(3) is NotImplemented
        assert repr(p) == "Point { x: 6.0, y: 4.0, cache: None }", repr(p)
        assert str(p) == "(6, 4)", str(p)
        try:
            hash(p)
        except TypeError:
            pass
        else:
            raise AssertionError("PartialEq without Hash must be unhashable")

        T = mylib.Tag
        assert hash(T("a")) == hash(T("a"))
        assert len({T("a"), T("a"), T("b")}) == 2
        try:
            T("")
        except ValueError as e:
            assert str(e) == "empty tag"
        else:
            raise AssertionError("no error")

        # No PartialEq: identity equality and hash, like plain Python objects.
        c = mylib.Counter()
        assert c == c and c != mylib.Counter()
        hash(c)
        # No Debug: the default repr.
        assert repr(c).startswith("<mylib.Counter object at"), repr(c)
    "#);
}

#[test]
fn class_parameters_borrow_the_instance() {
    run(r#"
        a = mylib.Point(1, 2)
        b = mylib.Point(0, 0)
        a.pull(b)
        assert (b.x, b.y) == (1.0, 2.0)

        c = mylib.Counter()
        assert mylib.incr_counter(c) == 1
        assert mylib.incr_counter(c) == 2
        assert c.count == 2 and mylib.read_counter(c) == 2
        try:
            mylib.incr_counter(a)
        except TypeError as e:
            assert str(e) == "argument 'c': expected Counter, got Point", e
        else:
            raise AssertionError("wrong class accepted")

        assert mylib.x_or_zero() == 0.0 and mylib.x_or_zero(None) == 0.0
        assert mylib.x_or_zero(a) == 1.0
        try:
            mylib.x_or_zero(1)
        except TypeError as e:
            assert str(e) == "argument 'p': expected Point, got int", e
        else:
            raise AssertionError("int accepted")
    "#);
}

#[test]
fn class_by_value_is_a_copy() {
    run(r#"
        p = mylib.Point(1, 2)
        q = mylib.mirrored(p)
        assert (q.x, p.x) == (-1.0, 1.0) and q is not p
        s = p.swapped()
        assert (s.x, s.y) == (2.0, 1.0) and (p.x, p.y) == (1.0, 2.0)
    "#);
}

#[test]
fn ref_parameters_and_returns_keep_identity() {
    run(r#"
        p = mylib.Point(1, 2)
        assert mylib.same(p) is p
        far, near = mylib.Point(9, 9), mylib.Point(1, 3)
        assert p.nearest([far, near]) is near
        assert p.nearest([]) is None
        c = mylib.geometry.centroid([mylib.Point(0, 0), mylib.Point(2, 4)])
        assert c == mylib.Point(1, 2)
        try:
            mylib.same(1)
        except TypeError as e:
            assert str(e).startswith("argument 'p': expected Point, got int"), e
        else:
            raise AssertionError("int accepted for Ref<Point>")
    "#);
}

#[test]
fn borrow_conflicts_raise() {
    run(r#"
        p = mylib.Point(1, 1)
        assert p.dist(p) == 0.0       # two shared borrows are fine
        try:
            p.absorb(p)                # &mut self + &Self on the same object
        except RuntimeError as e:
            assert "borrow" in str(e).lower(), e
        else:
            raise AssertionError("conflicting borrow accepted")
        try:
            p.pull(p)                  # &self + &mut Point
        except RuntimeError:
            pass
        else:
            raise AssertionError("conflicting borrow accepted")
        assert (p.x, p.y) == (1.0, 1.0)
        p.absorb(mylib.Point(1, 1))
        assert p.x == 2.0
        # `&self` and `Ref::borrow` of the same instance are both shared.
        assert p.nearest([p]) is p
    "#);
}

#[test]
fn ref_fields_share_identity() {
    run(r#"
        p = mylib.Point(1, 2)
        c = mylib.Circle(p, 3)
        assert c.center is p
        c.center.x = 5
        assert p.x == 5.0
        assert c.center_x() == 5.0
        q = mylib.Point(9, 9)
        c.center = q
        assert c.center is q
        try:
            c.center = 1
        except TypeError as e:
            assert str(e).startswith("attribute 'center': expected Point, got int"), e
        else:
            raise AssertionError("int accepted for Ref<Point>")
    "#);
}

#[test]
fn ref_from_rust() {
    let p = Ref::new(mylib::Point::new(1.0, 2.0)).unwrap();
    let c = Ref::new(mylib::Circle::new(p.clone(), 1.0)).unwrap();
    let s = scope();
    s.set("p", &p).unwrap();
    s.set("c", &c).unwrap();
    s.exec("c.center.y = 7\nassert c.center is p").unwrap();
    assert_eq!(p.borrow().unwrap().y, 7.0);
    assert!(c.borrow().unwrap().center.is(&p));

    // A live mutable borrow makes Python access fail instead of racing.
    let guard = p.borrow_mut().unwrap();
    let err = s.exec("p.x").unwrap_err();
    assert!(err.is_instance_of("RuntimeError"), "{err}");
    let err = s.exec("p.norm()").unwrap_err();
    assert!(err.is_instance_of("RuntimeError"), "{err}");
    assert!(p.borrow().is_err());
    drop(guard);
    s.exec("assert p.x == 1.0").unwrap();

    // A Ref comes back from Python as the same object.
    let back: Ref<mylib::Point> = s.get("p").unwrap().to().unwrap();
    assert!(back.is(&p));
    assert_eq!(*back.borrow().unwrap(), mylib::Point::new(1.0, 7.0));
    assert!(s.get("c").unwrap().to::<Ref<mylib::Point>>().is_err());
}
