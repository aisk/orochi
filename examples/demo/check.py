"""Exercises the demo extension module; run by run.py."""

import inspect
import math
import pickle
import subprocess
import sys

import demo
import demo.geometry

assert demo.__doc__ == "Demo extension module built with orochi."
assert demo.add(1) == 2 and demo.add(a=1, b=5) == 6
assert str(inspect.signature(demo.add)) == "(a, b=None)"
assert demo.add.__module__ == "demo" and demo.add.__doc__ == "Add two integers; `b` defaults to 1."
assert pickle.loads(pickle.dumps(demo.add)) is demo.add
for bad in (True, 1.5):
    try:
        demo.add(bad)
    except TypeError as e:
        assert str(e).startswith("argument 'a': expected int"), e
    else:
        raise AssertionError(f"add({bad!r}) accepted")
try:
    demo.add()
except TypeError as e:
    assert str(e) == "add() missing 1 required positional argument: 'a'", e

assert demo.count_lines("a\nb") == 2
assert demo.word_counts("a b a") == {"a": 2, "b": 1}
try:
    demo.read_text("does/not/exist")
except FileNotFoundError:
    pass
else:
    raise AssertionError("no error")

xs = [1]
same = xs
demo.push_twice(xs, 7)
assert xs == [1, 7, 7] and xs is same

P = demo.Point
p = P(3, 4)
assert p.norm() == 5.0
assert p.scale(2) is None and (p.x, p.y) == (6.0, 8.0)
p.y = 4
assert p == P(6, 4) and p != P(0, 0)
assert repr(p) == "Point { x: 6.0, y: 4.0 }" and str(p) == "(6, 4)"
assert type(p).__module__ == "demo" and P.norm.__qualname__ == "Point.norm"
assert str(inspect.signature(P)) == "(x, y)"
assert P.__doc__ == "A 2D point.\n\nCreates a point from its coordinates.", P.__doc__
assert str(inspect.signature(P.dist)) == "(self, /, other)"
assert p.dist(P(6, 0)) == 4.0
try:
    p.dist(1)
except TypeError as e:
    assert str(e) == "argument 'other': expected Point, got int", e
else:
    raise AssertionError("int accepted")

# A `List<T>` field is the Python list itself; other values are copies.
b = demo.Basket()
b.tags.append("x")
assert b.tags == ["x"] and b.tags is b.tags
b.label = "fruit"
assert b.label == "fruit"
b.add_item(3)
assert b.items() == [3] and b.count() == 2
try:
    b.label = 1
except TypeError as e:
    assert str(e).startswith("attribute 'label': expected str"), e
else:
    raise AssertionError("int accepted for String")

assert abs(demo.geometry.area(1) - math.pi) < 1e-12
assert demo.geometry.__name__ == "demo.geometry"
assert not hasattr(demo, "helper")

assert demo.ticket(3).id == 3

# Instances must only be created through the constructor (pyo3's own
# `tp_new`): every attempt to bypass it ends in a Python exception or a
# fully constructed object, never in a crash. Each runs in its own process.
BYPASS = [
    ("object.__new__(demo.Basket)", "TypeError"),
    ("object.__new__(demo.Point).norm()", "TypeError"),
    ("object.__new__(demo.Ticket)", "TypeError"),
    ("demo.Ticket()", "TypeError"),
    ("del demo.Basket.__new__; b = demo.Basket(); b.add_item(1); print(b.tags, b.items()); del b", None),
    ("del demo.Point.__new__; print(demo.Point(3, 4).norm())", None),
    ("demo.Basket.__new__ = object.__new__; b = demo.Basket(); print(b.tags, b.count()); del b", None),
    ("demo.Point.__new__ = object.__new__; print(demo.Point(3, 4).norm())", None),
]
for code, error in BYPASS:
    done = subprocess.run([sys.executable, "-c", f"import demo\n{code}"], capture_output=True, text=True)
    if error is None:
        assert done.returncode == 0, (code, done)
    else:
        assert done.returncode == 1 and f"\n{error}: " in done.stderr, (code, done)

# A `__del__` that calls the extension while the interpreter shuts down (the
# cycle is collected after `Py_IsInitialized()` already returns false).
SHUTDOWN = """
import demo

class Cycle:
    def __init__(self):
        self.me = self

    def __del__(self):
        print("lines:", demo.count_lines("a\\nb"), demo.Point(3, 4).norm())

c = Cycle()
"""
done = subprocess.run([sys.executable, "-c", SHUTDOWN], capture_output=True, text=True)
assert done.returncode == 0 and "panicked" not in done.stderr, done
assert "lines: 2 5.0" in done.stdout, done

print("demo extension module OK:", demo.__file__)
