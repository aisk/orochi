"""Exercises the demo extension module; run by run.ps1."""

import math

import demo
import demo.geometry

assert demo.__doc__ == "Demo extension module built with orochi."
assert demo.add(1) == 2 and demo.add(a=1, b=5) == 6
assert demo.add.__text_signature__ == "(a, b=None)"
for bad in (True, 1.5):
    try:
        demo.add(bad)
    except TypeError:
        pass
    else:
        raise AssertionError(f"add({bad!r}) accepted")

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

p = demo.Point(3, 4)
assert p.norm() == 5.0
assert p.scale(2).scale(0.5) is p
p.x = 6
assert p == demo.Point(6, 4) and p != demo.Point(0, 0)
assert repr(p) == "Point { x: 6.0, y: 4.0 }" and str(p) == "(6, 4)"
assert type(p).__module__ == "demo"

b = demo.Basket()
b.items.append("apple")
b.table.setdefault("k", []).append(1)
b.table["k"].append(2)
assert b.count() == 3 and b.items == ["apple"] and b.table == {"k": [1, 2]}
try:
    b.items.append(1)
except TypeError:
    pass
else:
    raise AssertionError("int accepted in Vec<String>")
assert list(b.items) == ["apple"]
assert demo.favorite() is demo.Color.Red and demo.Color.Red != demo.Color.Green

assert abs(demo.geometry.area(1) - math.pi) < 1e-12
assert not hasattr(demo, "helper")

print("demo extension module OK:", demo.__file__)
