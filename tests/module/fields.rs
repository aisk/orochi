//! `pub` fields: a Rust value reads as an immutable snapshot, an orochi
//! reference type as the Python object itself.

use crate::run;

#[test]
fn value_fields_read_as_immutable_snapshots() {
    run(r#"
        import pathlib, types
        b = mylib.Bag()
        b.push(1)
        b.push(2)
        assert type(b.items) is tuple and b.items == (1, 2)
        assert type(b.rows) is tuple and b.rows == ((1,), (2,))
        assert all(type(r) is tuple for r in b.rows)
        assert type(b.table) is types.MappingProxyType
        assert dict(b.table) == {"1": (1,), "2": (2,)} and type(b.table["1"]) is tuple
        assert type(b.tags) is frozenset and b.tags == {"1", "2"}
        b.sets = {(1, 2)}
        assert type(b.sets) is frozenset and b.sets == frozenset({(1, 2)})
        assert b.maybe is None and b.boxed is None
        b.maybe = [3]
        b.boxed = [4]
        assert b.maybe == (3,) and b.boxed == (4,)
        assert type(b.fixed) is tuple and b.fixed == (0, 0)
        b.pair = (1, [2, 3])
        assert b.pair == (1, (2, 3)) and type(b.pair[1]) is tuple
        assert b.data == b"" and type(b.data) is bytes
        b.data = b"ab"
        assert b.data == b"ab"
        assert type(b.label) is str and type(b.path) is type(pathlib.Path())
        # Each read is a new snapshot of the current value.
        assert b.items is not b.items
        b.dicts = [{"a": 1}]
        assert type(b.dicts) is tuple and type(b.dicts[0]) is types.MappingProxyType
        assert dict(b.dicts[0]) == {"a": 1}
        b.opts = [1, None]
        assert b.opts == (1, None)
        b.blobs = [b"a", b""]
        assert b.blobs == (b"a", b"") and type(b.blobs[0]) is bytes
        b.by_path = {"a.txt": 1}
        (key,) = b.by_path
        assert isinstance(key, pathlib.Path) and b.by_path[key] == 1
        b.by_path = b.by_path
        assert dict(b.by_path) == {pathlib.Path("a.txt"): 1}
    "#);
}

#[test]
fn enum_fields_read_as_members() {
    run(r#"
        b = mylib.Bag()
        assert b.color is None and b.colors == () and dict(b.palette) == {}
        b.color = mylib.Color.Green
        b.colors = [mylib.Color.Red, mylib.Color.Green]
        b.palette = {"sky": mylib.Color.Green}
        assert b.color is mylib.Color.Green
        assert b.colors[0] is mylib.Color.Red and b.colors[1] is mylib.Color.Green
        assert b.palette["sky"] is mylib.Color.Green
    "#);
}

#[test]
fn nested_snapshots_are_accepted_by_parameters() {
    run(r#"
        b = mylib.Bag()
        b.table = {"a": [1, 2], "b": [3]}
        rows = (b.table, b.table)
        assert mylib.nested_sum(rows) == 12 and mylib.nested_len(rows) == 4
        assert mylib.nested_sum([b.table]) == 6 and mylib.nested_len([b.table]) == 2
    "#);
}

#[test]
fn snapshots_cannot_be_changed() {
    run(r#"
        b = mylib.Bag()
        b.push(1)
        attempts = [
            lambda: b.items.append(3),
            lambda: b.items.__setitem__(0, 5),
            lambda: b.rows[0].append(1),
            lambda: b.table.__setitem__("k", [1]),
            lambda: b.table["1"].append(2),
            lambda: b.table.pop("1"),
            lambda: b.counts.update(a=1),
            lambda: b.tags.add("x"),
            lambda: b.tags.discard("1"),
        ]
        for attempt in attempts:
            try:
                attempt()
            except (AttributeError, TypeError):
                pass
            else:
                raise AssertionError("a snapshot was changed")
        assert b.items == (1,) and b.rows == ((1,),) and dict(b.table) == {"1": (1,)}
        assert b.tags == {"1"} and dict(b.counts) == {}
        # `list(...)` gives a mutable copy that is independent of the field.
        items = list(b.items)
        items.append(2)
        assert b.items == (1,)
    "#);
}

#[test]
fn reads_follow_rust_changes_and_assignment() {
    run(r#"
        b = mylib.Bag()
        before = b.items
        b.push(7)
        assert before == () and b.items == (7,) and b.total() == 7
        b.items = [1, 2]
        assert b.items == (1, 2) and b.total() == 3
        # Wrong elements raise and leave the field unchanged.
        for bad in (["x"], [1.5], "12", None):
            try:
                b.items = bad
            except TypeError as e:
                assert str(e).startswith("attribute 'items'"), e
            else:
                raise AssertionError(f"{bad!r} accepted")
            assert b.items == (1, 2)
        try:
            b.table = {"k": ["x"]}
        except TypeError:
            pass
        else:
            raise AssertionError("wrong nested element accepted")
        assert dict(b.table) == {"7": (7,)}
    "#);
}

#[test]
fn snapshots_round_trip() {
    run(r#"
        b = mylib.Bag()
        b.push(1)
        b.push(2)
        b.counts = {"a": 1}
        b.sets = {(1, 2)}
        b.items = b.items
        b.rows = b.rows
        b.table = b.table
        b.counts = b.counts
        b.tags = b.tags
        b.sets = b.sets
        b.fixed = b.fixed
        b.pair = b.pair
        assert b.items == (1, 2) and b.rows == ((1,), (2,))
        assert dict(b.table) == {"1": (1,), "2": (2,)} and dict(b.counts) == {"a": 1}
        assert b.tags == {"1", "2"} and b.sets == {(1, 2)}
        other = mylib.Bag()
        other.table = b.table
        assert dict(other.table) == dict(b.table)
        # Snapshots are accepted where the Rust type is converted.
        assert mylib.copy_back(b.items) == [1, 2]
        assert mylib.lookup(b.counts, "a") == 1
        assert mylib.total(b.items) == 3.0
    "#);
}

#[test]
fn references_inside_value_containers_pass_through() {
    run(r#"
        b = mylib.Bag()
        xs = [1]
        o = {"any": "object"}
        p = mylib.Point(1, 2)
        b.handles = [xs]
        b.objects = [o, [2]]
        b.points = [p]
        assert type(b.handles) is tuple and b.handles[0] is xs
        b.handles[0].append(2)
        assert xs == [1, 2]
        assert b.objects[0] is o and type(b.objects[1]) is list
        assert b.points[0] is p
        b.points[0].x = 5
        assert p.x == 5
        b.refs = {"p": p}
        assert b.refs["p"] is p
        assert b.maybe_list is None
        b.maybe_list = xs
        assert b.maybe_list is xs
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
        # A `Ref` field is the referenced object.
        pt = mylib.Point(1, 2)
        c = mylib.Circle(pt, 1.0)
        assert c.center is pt
    "#);
}
