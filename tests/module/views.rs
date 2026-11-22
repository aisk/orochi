//! Container fields read as live views.

use crate::run;

#[test]
fn list_field_view_is_live() {
    run(r#"
        import collections.abc, copy, pickle
        b = mylib.Bag()
        b.items.append(1)
        b.items.extend([2, 3])
        assert b.total() == 6
        v = b.items
        assert v == [1, 2, 3] and [1, 2, 3] == v and v != [1]
        assert v == b.items
        assert repr(v) == "[1, 2, 3]" and str(v) == "[1, 2, 3]"
        assert len(v) == 3 and 2 in v and 9 not in v
        assert list(v) == [1, 2, 3] and type(list(v)) is list
        assert type(v.copy()) is list and type(copy.copy(v)) is list
        assert pickle.loads(pickle.dumps(v)) == [1, 2, 3]
        assert isinstance(v, collections.abc.MutableSequence) and not isinstance(v, list)
        try:
            hash(v)
        except TypeError:
            pass
        else:
            raise AssertionError("view is hashable")

        v[0] = 5
        assert b.items == [5, 2, 3]
        v[-1] = 9
        assert b.items == [5, 2, 9] and v[-1] == 9
        del v[0]
        assert b.items == [2, 9]
        v[0:1] = [7, 8]
        assert b.items == [7, 8, 9]
        assert v[1:] == [8, 9] and type(v[1:]) is list
        del v[::2]
        assert b.items == [8]
        v += [1, 2]
        assert b.items == [8, 1, 2]
        b.items += [3]
        assert b.items == [8, 1, 2, 3]
        v.sort()
        assert b.items == [1, 2, 3, 8]
        v.sort(reverse=True)
        v.reverse()
        v.insert(0, 0)
        assert b.items == [0, 1, 2, 3, 8]
        assert v.pop() == 8 and v.pop(0) == 0
        v.remove(2)
        assert b.items == [1, 3]
        assert v.index(3) == 1 and v.count(1) == 1
        assert v + [4] == [1, 3, 4] and [0] + v == [0, 1, 3]
        assert v * 2 == [1, 3, 1, 3] and v < [2] and v >= [1, 3]
        assert list(reversed(v)) == [3, 1]
        assert b.total() == 4

        # Wrong elements fail at once and leave the field unchanged.
        for op in (lambda: v.append("x"), lambda: v.extend([5, "x"]),
                   lambda: v.insert(0, 1.5), lambda: v.__setitem__(0, None),
                   lambda: v.__setitem__(slice(0, 1), ["a"])):
            try:
                op()
            except TypeError as e:
                assert "attribute 'items'" in str(e), e
            else:
                raise AssertionError("wrong element accepted")
            assert b.items == [1, 3], b.items

        # Whole-field assignment; an older view sees the new contents.
        b.items = [4, 5]
        assert v == [4, 5] and b.total() == 9
        v.clear()
        assert b.items == [] and not v
    "#);
}

#[test]
fn nested_views_are_live() {
    run(r#"
        b = mylib.Bag()
        b.rows = [[1], [2]]
        b.rows[0].append(5)
        assert b.rows == [[1, 5], [2]] and b.row_len(0) == 2
        b.rows[-1].append(3)
        assert b.rows == [[1, 5], [2, 3]]
        for row in b.rows:
            row.append(0)
        assert b.rows == [[1, 5, 0], [2, 3, 0]]
        try:
            b.rows[1].append("x")
        except TypeError:
            pass
        else:
            raise AssertionError("wrong nested element accepted")
        assert b.rows == [[1, 5, 0], [2, 3, 0]]

        # A child view addresses its position in the parent.
        first = b.rows[0]
        b.rows.insert(0, [9])
        assert first == [9]

        b.table["k"] = [1]
        b.table["k"].append(2)
        assert b.table_len("k") == 2
        b.table.setdefault("n", []).append(7)
        assert b.table == {"k": [1, 2], "n": [7]}
        assert b.table.copy() == {"k": [1, 2], "n": [7]}
        assert type(b.table.copy()["k"]) is list
    "#);
}

#[test]
fn dict_and_set_views() {
    run(r#"
        import collections.abc
        b = mylib.Bag()
        t = b.counts
        t["a"] = 1
        t.update({"b": 2}, c=3)
        assert b.counts == {"a": 1, "b": 2, "c": 3}
        assert repr(t) == repr(t.copy()) and str(t) == str(t.copy())
        assert "a" in t and t.get("a") == 1 and t.get("q") is None and t.get("q", 0) == 0
        assert sorted(t.keys()) == ["a", "b", "c"] and sorted(t.values()) == [1, 2, 3]
        assert dict(t) == {"a": 1, "b": 2, "c": 3} and type(dict(t)) is dict
        assert t.pop("c") == 3 and t.pop("zz", None) is None
        del t["b"]
        assert b.counts == {"a": 1}
        t |= {"d": 4}
        assert b.counts == {"a": 1, "d": 4}
        assert t | {"e": 5} == {"a": 1, "d": 4, "e": 5} and type(t | {}) is dict
        assert isinstance(t, collections.abc.MutableMapping) and not isinstance(t, dict)
        for op in (lambda: t.__setitem__("x", "1"), lambda: t.__setitem__(1, 1),
                   lambda: t.update(x=1.5)):
            try:
                op()
            except TypeError:
                pass
            else:
                raise AssertionError("wrong entry accepted")
            assert b.counts == {"a": 1, "d": 4}
        assert t.popitem() in (("a", 1), ("d", 4)) and len(b.counts) == 1
        t.clear()
        assert b.counts == {}

        s = b.tags
        s.add("x")
        s |= {"y"}
        s.update(["z"])
        assert b.tag_list() == ["x", "y", "z"]
        s.discard("x")
        s.remove("z")
        assert b.tags == {"y"} and {"y"} == s and "y" in s
        assert s | {"q"} == {"y", "q"} and type(s | {"q"}) is set
        assert {"q"} | s == {"y", "q"} and s & {"y"} == {"y"} and s - {"y"} == set()
        assert s.union(["w"]) == {"y", "w"} and s.issubset({"y", "w"})
        assert set(s) == {"y"} and type(s.copy()) is set
        try:
            s.add(1)
        except TypeError:
            pass
        else:
            raise AssertionError("int accepted in set of str")
        assert b.tags == {"y"}
        s -= {"y"}
        assert b.tag_list() == [] and len(s) == 0
    "#);
}

#[test]
fn other_fields_stay_copies() {
    run(r#"
        b = mylib.Bag()
        assert b.maybe is None
        b.maybe = [1]
        b.maybe.append(2)
        assert b.maybe == [1, 2]
        b.maybe = None
        assert b.maybe is None

        f = b.fixed
        assert type(f) is list and f == [0, 0]
        f.append(1)
        assert b.fixed == [0, 0]

        assert b.data == b"" and type(b.data) is bytes
        b.label = "a"
        assert b.label == "a" and type(b.label) is str
    "#);
}

#[test]
fn views_are_accepted_as_arguments() {
    run(r#"
        b = mylib.Bag()
        b.items = [1, 2]
        assert mylib.copy_back(b.items) == [1, 2]
        assert mylib.total(b.items) == 3.0
        mylib.push_twice(b.items, 7)
        assert b.items == [1, 2, 7, 7] and b.total() == 17
        mylib.extend_from(b.items, b.items)
        assert b.items == [1, 2, 7, 7, 1, 2, 7, 7]
        try:
            mylib.merge_into(b.items, b.items)
        except RuntimeError as e:
            assert "Already borrowed" in str(e), e
        else:
            raise AssertionError("aliased views accepted")

        mylib.bump(b.counts, "a")
        mylib.bump(b.counts, "a")
        assert b.counts == {"a": 2}
        assert mylib.lookup(b.counts, "a") == 2
        assert mylib.add_tag(b.tags, "q") is True
        assert b.tag_list() == ["q"]

        b.rows = [[1, 2]]
        mylib.push_twice(b.rows[0], 3)
        assert b.rows == [[1, 2, 3, 3]]

        # Assigning a view to another field copies its contents.
        c = mylib.Bag()
        c.items = b.items
        c.items.append(0)
        assert b.items[-1] == 7 and c.items[-1] == 0
        c.counts = b.counts
        assert c.counts == {"a": 2}
    "#);
}

#[test]
fn views_respect_borrows() {
    run(r#"
        b = mylib.Bag()
        def touch():
            b.items.append(1)
        try:
            b.with_callback(touch)
        except RuntimeError as e:
            assert "borrow" in str(e).lower(), e
        else:
            raise AssertionError("view used during a mutable borrow")
        assert b.items == []
    "#);
}

#[test]
fn views_follow_the_rust_type() {
    run(r#"
        lst = [1]
        s = mylib.Shelf(lst)
        # Handles and `Object` fields are returned as they are.
        assert s.handle is lst
        s.any = [1, [2]]
        assert type(s.any) is list and s.any is s.any
        s.any.append(3)
        assert s.any == [1, [2], 3]
        # An array is a plain list; its `Vec` elements are live views.
        g = s.grid
        assert type(g) is list and g == [[], []]
        g[0].append(1)
        s.grid[1].extend([2, 3])
        assert s.grid == [[1], [2, 3]]
        g.append([9])
        assert len(s.grid) == 2
        try:
            g[0].append("x")
        except TypeError:
            pass
        else:
            raise AssertionError("wrong element accepted")
        # Elements that are arrays are plain lists (copies).
        s.pairs = [[1, 2]]
        p = s.pairs[0]
        assert type(p) is list and p == [1, 2]
        p[0] = 5
        assert s.pairs == [[1, 2]]
        # `Option<Box<Vec<T>>>`.
        assert s.boxed is None
        s.boxed = [1]
        s.boxed.append(2)
        assert s.boxed == [1, 2]
        # A `Ref` field is the referenced object.
        pt = mylib.Point(1, 2)
        c = mylib.Circle(pt, 1.0)
        assert c.center is pt
    "#);
}

#[test]
fn slices_of_views_are_lists_of_child_views() {
    run(r#"
        import collections.abc
        b = mylib.Bag()
        b.rows = [[1], [2], [3]]
        part = b.rows[1:]
        assert type(part) is list and part == [[2], [3]]
        assert all(isinstance(r, collections.abc.MutableSequence) and type(r) is not list for r in part)
        part[0].append(9)
        b.rows[::-1][0].append(8)
        assert b.rows == [[1], [2, 9], [3, 8]]
        b.items = [1, 2, 3]
        assert b.items[1:] == [2, 3] and all(type(x) is int for x in b.items[1:])
    "#);
}

#[test]
fn views_as_values_are_converted_from_plain_contents() {
    run(r#"
        b = mylib.Bag()
        b.rows = [[1], [2]]
        b.items = [5]
        x = mylib.identity(b.items)
        assert type(x) is list and x == [5]
        x.append(6)
        assert b.items == [5]
        rows = mylib.objects(b.rows)
        assert [type(r) for r in rows] == [list, list] and rows == [[1], [2]]
        h = mylib.Holder()
        h.obj = b.rows
        assert type(h.obj) is list and type(h.obj[0]) is list
        s = mylib.Shelf([])
        s.any = b.counts
        assert type(s.any) is dict
    "#);
}

#[test]
fn views_are_not_handles() {
    run(r#"
        b = mylib.Bag()
        b.items = [1]
        try:
            mylib.push_handle(b.items, 2)
        except TypeError as e:
            message = str(e)
            assert message.startswith("argument 'xs': expected list, got ListView"), message
            assert "&mut Vec<T>" in message and "list(view)" in message, message
        else:
            raise AssertionError("view passed as a List handle")
        assert b.items == [1]
        mylib.push_handle(list(b.items), 2)
        s = mylib.Shelf([])
        try:
            s.handle = b.items
        except TypeError as e:
            assert str(e).startswith("attribute 'handle': expected list"), e
        else:
            raise AssertionError("view assigned to a List handle field")
        try:
            mylib.push_handle(b.counts, 2)
        except TypeError as e:
            assert str(e) == "argument 'xs': expected list, got DictView", e
        else:
            raise AssertionError("dict view passed as a List handle")
    "#);
}

#[test]
fn nested_mut_arguments_conflict() {
    run(r#"
        b = mylib.Bag()
        b.rows = [[1]]
        for call in (lambda: mylib.refill(b, b.items),
                     lambda: mylib.nest(b.rows, b.rows[0]),
                     lambda: mylib.nest(b.rows, b.rows[0:1][0])):
            try:
                call()
            except RuntimeError as e:
                assert "Already borrowed" in str(e), e
            else:
                raise AssertionError("overlapping &mut arguments accepted")
        mylib.nest(b.rows, b.items)
        other = mylib.Bag()
        mylib.refill(b, other.items)
        assert b.rows == [[1], []] and other.items == [1]
    "#);
}

#[test]
fn view_mutations_are_atomic_across_threads() {
    run(r#"
        import sys, threading
        b = mylib.Bag()
        old = sys.getswitchinterval()
        sys.setswitchinterval(1e-6)
        try:
            def work():
                for i in range(300):
                    b.items.append(i)
                    b.counts["n"] = b.counts.get("n", 0)  # a read and a write
                    b.tags.add(str(i))
            threads = [threading.Thread(target=work) for _ in range(4)]
            for t in threads:
                t.start()
            for t in threads:
                t.join()
        finally:
            sys.setswitchinterval(old)
        assert len(b.items) == 1200, len(b.items)
        assert len(b.tags) == 300
    "#);
}

#[test]
fn views_module_is_registered_under_a_versioned_name() {
    run(r#"
        import sys
        v = mylib.Bag().items
        name = type(v).__module__
        assert name.startswith("orochi._views_") and len(name) == len("orochi._views_") + 16, name
        module = sys.modules[name]
        assert module._TAG == name[len("orochi._views_"):]
        assert "orochi._views" not in sys.modules
    "#);
}

#[test]
fn view_operators_defer_to_other_types() {
    run(r#"
        b = mylib.Bag()
        b.items = [1, 2]
        b.counts = {"a": 1}
        b.tags = {"x"}
        assert b.items.__eq__((1, 2)) is NotImplemented and b.items != (1, 2)
        assert b.items.__lt__((1,)) is NotImplemented
        assert b.items.__add__((3,)) is NotImplemented
        assert b.items.__mul__("x") is NotImplemented
        assert b.counts.__eq__([("a", 1)]) is NotImplemented
        assert b.counts.__or__([("b", 2)]) is NotImplemented
        assert b.tags.__eq__(["x"]) is NotImplemented and b.tags == frozenset({"x"})
        for op in (lambda: b.items < (1,), lambda: b.items + (3,), lambda: b.counts | [("b", 2)]):
            try:
                op()
            except TypeError:
                pass
            else:
                raise AssertionError("operator with an unrelated type succeeded")
        # A type that knows about lists gets its turn.
        class Anything:
            def __eq__(self, other):
                return "yes"
            def __radd__(self, other):
                return "added"
        assert (b.items == Anything()) == "yes" and b.items + Anything() == "added"
        assert type(b.counts).fromkeys("ab", 0) == {"a": 0, "b": 0}
    "#);
}

#[test]
fn field_views_typed_like_the_stubs() {
    // What the stubs promise for views (checked by mypy and pyright in
    // `run.ps1 -TypeCheck`) also works at runtime.
    run(r#"
        b = mylib.Bag()
        other = mylib.Bag()
        other.items = [3]
        other.table = {"t": [1]}
        mylib.push_twice(b.items, 7)
        b.table = other.table
        b.items += [1, 2]
        b.items.sort()
        assert b.items.copy() == [1, 2, 7, 7] and type(b.items.copy()) is list
        b.items = other.items
        b.rows.append((1, 2))
        b.rows.append(b.items)
        assert b.rows == [[1, 2], [3]]
        assert b.table == {"t": [1]} and type(b.table.copy()["t"]) is list
        b.tags = other.tags
        b.maybe = other.items
        assert b.maybe == [3]
    "#);
}