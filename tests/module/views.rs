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
