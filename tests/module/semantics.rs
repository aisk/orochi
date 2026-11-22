//! Return-reference identity, enums, names and other generated-code details.

use crate::run;

#[test]
fn self_reference_returns_check_the_instance() {
    run(r#"
        n = mylib.Node(1)
        assert n.me() is n                    # `-> &Node`: the same instance
        assert n.pick(True) is n              # `Result<&Self>`
        for call in (lambda: n.shared(), lambda: n.pick(False)):
            try:
                call()
            except RuntimeError as e:
                assert "different instance" in str(e) and "not Clone" in str(e), e
            else:
                raise AssertionError("reference to another instance accepted")
        assert n.enabled() is True
        assert not hasattr(n, "disabled")

        # A `Clone` class returns a copy of the other instance.
        p = mylib.Point(1, 2)
        o = p.origin_ref()
        assert o is not p and o == mylib.Point(0, 0)
        o.x = 5
        assert p.origin_ref() == mylib.Point(0, 0)
    "#);
}

#[test]
fn generated_names_do_not_collide_with_parameters() {
    run(r#"
        assert mylib.scale(2.5) == 5.0
        assert mylib.scale(scale=1) == 2.0
        assert mylib.reserved_names(1, 2, 3, 4, 5, 6) == 21
        assert mylib.reserved_names(py=1, slf=1, args=1, kwargs=1, module=1, type=1) == 6
        assert mylib._under() == 1
    "#);
}

#[test]
fn text_signatures_show_none_defaults() {
    run(r#"
        assert mylib.add.__text_signature__ == "(a, b=None)", mylib.add.__text_signature__
        assert mylib.greet.__text_signature__ == "(name=None)"
        assert mylib.Point.dist.__text_signature__ == "($self, other)", mylib.Point.dist.__text_signature__
    "#);
}

#[test]
fn enum_members_are_singletons() {
    run(r#"
        C = mylib.Color
        assert mylib.favorite() is C.Red
        assert mylib.favorite() is mylib.favorite()
        assert C.Red is C.Red
        assert hash(C.Red) == hash(mylib.favorite())

        S = mylib.Shade
        assert S.Light == S.Light and S.Light != S.Dark
        assert len({S.Light, S.Light, S.Dark}) == 2
        assert S.Light.inverted() is S.Dark
        assert S.Dark.is_dark() and not S.Light.is_dark()
        assert mylib.darkest([S.Light, S.Dark]) is S.Dark
        assert mylib.darkest([S.Light]) is None
        assert (S.Light == C.Red) is False

        # Only members of the class are accepted.
        for bad in (0, 1, "Red", S.Light, None):
            try:
                mylib.is_red(bad)
            except TypeError as e:
                assert "expected mylib.Color" in str(e), e
            else:
                raise AssertionError(f"{bad!r} accepted as Color")

        # Not iterable, not callable: there are no other members to make.
        try:
            list(C)
        except TypeError:
            pass
        else:
            raise AssertionError("iterable")
        try:
            C(1)
        except TypeError:
            pass
        else:
            raise AssertionError("callable")
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
        assert s.b == (1.0, 2.0)
    "#);
}

#[test]
fn field_setter_drops_the_old_value_after_the_borrow() {
    run(r#"
        seen = []
        class Watch:
            def __del__(self):
                seen.append(h.count)
        h = mylib.Holder()
        h.count = 3
        h.obj = Watch()
        h.obj = None
        assert seen == [3], seen
    "#);
}
