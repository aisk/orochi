//! Classes: construction, fields, methods, special methods, borrowing and
//! `Ref<T>` identity.

use orochi::Ref;

use crate::{mylib, run, scope};

#[test]
fn construction_methods_and_properties() {
    run(r#"
        P = mylib.Point
        p = P(3, 4)
        assert p.norm() == 5.0
        assert P(x=1, y=2).x == 1.0
        assert P.__text_signature__ == "(x, y)", P.__text_signature__
        p.x = 6
        assert p.x == 6.0 and isinstance(p.x, float)
        p.scale(0.5)
        assert (p.x, p.y) == (3.0, 2.0)
        o = P.origin()
        assert (o.x, o.y) == (0.0, 0.0)
        assert p.dist(P(3, 0)) == 2.0
        assert p.dist(other=P(3, 0)) == 2.0
        assert P.DIMENSIONS == 2
        q = p.swapped()
        assert (q.x, q.y) == (2.0, 3.0) and (p.x, p.y) == (3.0, 2.0)
        assert p.cached_norm() == p.norm()
        try:
            p.x = "no"
        except TypeError as e:
            assert str(e).startswith("attribute 'x': expected float"), e
        else:
            raise AssertionError("str accepted for float field")
        try:
            del p.x
        except (AttributeError, TypeError):
            pass
        else:
            raise AssertionError("field deleted")
        try:
            P(True, 1)
        except TypeError as e:
            assert "argument 'x'" in str(e)
        else:
            raise AssertionError("bool accepted")
    "#);
}

#[test]
fn method_chaining_returns_self() {
    run(r#"
        p = mylib.Point(1, 1)
        r = p.scale(2).scale(3)
        assert r is p
        assert (p.x, p.y) == (6.0, 6.0)
        assert p.checked_scale(2) is p
        try:
            p.checked_scale(0)
        except ValueError:
            pass
        else:
            raise AssertionError("no error")
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

        # By value is a copy (Point is Clone); Counter is not Clone, so it can
        # only be passed by reference.
        try:
            mylib.incr_counter(a)
        except TypeError as e:
            assert "expected mylib.Counter, got Point" in str(e), e
        else:
            raise AssertionError("wrong class accepted")
    "#);
}

#[test]
fn borrow_conflicts_raise() {
    run(r#"
        p = mylib.Point(1, 1)
        assert p.dist(p) == 0.0       # two shared borrows are fine
        try:
            p.absorb(p)                # &mut self + &Point on the same object
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
            assert "expected mylib.Point" in str(e), e
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
    assert!(p.borrow().is_err());
    drop(guard);
    s.exec("assert p.x == 1.0").unwrap();

    // Conversions: a Ref comes back from Python as the same object; a
    // by-value class is a copy.
    let back: Ref<mylib::Point> = s.get("p").unwrap().to().unwrap();
    assert!(back.is(&p));
    let copy: mylib::Point = s.get("p").unwrap().to().unwrap();
    assert_eq!(copy, mylib::Point::new(1.0, 7.0));
    assert!(s.get("c").unwrap().to::<Ref<mylib::Point>>().is_err());
}

#[test]
fn enums() {
    run(r#"
        C = mylib.Color
        assert C.Red == C.Red and C.Red != C.Green
        assert mylib.is_red(C.Red) and not mylib.is_red(C.Green)
        assert repr(C.Green) == "Color.Green", repr(C.Green)
        assert len({C.Red, C.Red, C.Green}) == 2
    "#);
}
