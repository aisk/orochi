//! `pub` fields: simple values are copied in and out, orochi reference types
//! are the Python objects themselves.

use crate::run;

#[test]
fn value_fields_convert_strictly() {
    run(r#"
        import pathlib
        b = mylib.Bag()
        assert (b.label, b.letter, b.flag, b.ratio, b.small) == ("", "\0", False, 0.0, 0), (b.label, b.letter, b.flag, b.ratio, b.small)
        assert b.maybe is None and b.note is None
        b.label = "x"
        b.path = "a/b"
        b.letter = "z"
        b.flag = True
        b.ratio = 0.5
        b.small = 255
        b.maybe = 3
        b.note = "n"
        assert b.label == "x" and b.path == pathlib.Path("a/b") and b.letter == "z"
        assert b.flag is True and b.ratio == 0.5 and b.small == 255
        assert b.maybe == 3 and b.note == "n"
        b.maybe = None
        assert b.maybe is None
        for name, bad in (("label", 1), ("flag", 1), ("small", 256), ("small", -1), ("letter", "ab")):
            try:
                setattr(b, name, bad)
            except (TypeError, OverflowError, ValueError) as e:
                assert f"attribute '{name}'" in str(e) or "while converting attribute" in "".join(getattr(e, "__notes__", [])), (name, e)
            else:
                raise AssertionError(f"{name} = {bad!r} accepted")
        try:
            del b.label
        except (AttributeError, TypeError):
            pass
        else:
            raise AssertionError("field deleted")
        # Private fields are not visible; methods give access.
        assert callable(b.items)
        b.push(1)
        b.push(2)
        assert b.items() == [1, 2]
        assert mylib.Bag.label.__doc__ is None
        assert mylib.Point.x.__doc__ == "Horizontal coordinate."
    "#);
}

#[test]
fn reference_fields_are_the_python_objects() {
    run(r#"
        lst = [1]
        s = mylib.Shelf(lst)
        assert s.xs is lst and s.xs is s.xs
        assert s.map is s.map and s.members is s.members
        s.xs.append(2)
        s.map["a"] = 1
        s.members.add(3)
        assert s.sizes() == (2, 1, 1) and lst == [1, 2]
        s.any = [1, [2]]
        assert type(s.any) is list and s.any is s.any
        s.any.append(3)
        assert s.any == [1, [2], 3]
        try:
            s.xs = (1,)
        except TypeError as e:
            assert str(e).startswith("attribute 'xs': expected list"), e
        else:
            raise AssertionError("tuple accepted as a List")
        assert s.maybe_list is None
        s.maybe_list = lst
        assert s.maybe_list is lst
        # A `Ref` field is the referenced object.
        pt = mylib.Point(1, 2)
        c = mylib.Circle(pt, 1.0)
        assert c.center is pt
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

#[test]
fn field_access_respects_borrows() {
    let p = orochi::Ref::new(crate::mylib::Point::new(1.0, 2.0)).unwrap();
    let s = crate::scope();
    s.set("p", &p).unwrap();
    let guard = p.borrow().unwrap();
    s.exec("assert p.x == 1.0").unwrap(); // shared + shared
    let err = s.exec("p.x = 3").unwrap_err();
    assert!(err.is_instance_of("RuntimeError"), "{err}");
    drop(guard);
    s.exec("p.x = 3").unwrap();
    assert_eq!(p.borrow().unwrap().x, 3.0);
}
