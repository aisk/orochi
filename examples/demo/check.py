"""Exercises the demo extension module; run by run.ps1.

    python check.py [STUBS_DIR]

With STUBS_DIR, the module's type stubs are also written there (for
`run.ps1 -TypeCheck`); otherwise they are only generated and checked.
"""

import ast
import math
import pathlib
import subprocess
import sys
import tempfile

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

# A value field reads as an immutable snapshot; a `List<T>` field is the
# Python list itself.
b = demo.Basket()
b.tags.append("x")
assert b.tags == ["x"] and b.tags is b.tags
b.items = [1, 2]
b.add_item(3)
assert b.items == (1, 2, 3) and type(b.items) is tuple
for mutate in (lambda: b.items.append(4), lambda: b.table.__setitem__("k", [1])):
    try:
        mutate()
    except (AttributeError, TypeError):
        pass
    else:
        raise AssertionError("a snapshot was changed")
items = list(b.items)
items.append(4)
assert b.items == (1, 2, 3)
b.table = {"k": [1, 2]}
assert dict(b.table) == {"k": (1, 2)} and b.count() == 6
other = demo.Basket()
other.table = b.table
other.items = b.items
assert dict(other.table) == {"k": (1, 2)} and other.items == (1, 2, 3)
try:
    b.items = ["x"]
except TypeError:
    pass
else:
    raise AssertionError("str accepted in Vec<i64>")
assert b.items == (1, 2, 3)
for bad_ctor in (lambda: demo.Color(), lambda: demo.Color(1)):
    try:
        bad_ctor()
    except TypeError:
        pass
    else:
        raise AssertionError("Color constructed")
assert demo.favorite() is demo.Color.Red and demo.Color.Red != demo.Color.Green

assert abs(demo.geometry.area(1) - math.pi) < 1e-12
assert not hasattr(demo, "helper")

# Type stubs: generated from the real extension module.
assert "_orochi_stubs" not in demo.__all__
assert demo._orochi_stubs.__text_signature__ == "(out_dir=None)"
stubs = demo._orochi_stubs()
assert sorted(stubs) == ["demo/__init__.pyi", "demo/geometry.pyi"], sorted(stubs)
for path, text in stubs.items():
    ast.parse(text, filename=path)
    compile(text, path, "exec")  # also rejects e.g. duplicate parameters
init = stubs["demo/__init__.pyi"]
for line in [
    '"""Demo extension module built with orochi."""',
    "from . import geometry as geometry",
    "def add(a: int, b: int | None = None) -> int:",
    "def read_text(path: str | os.PathLike[str]) -> str: ...",
    "def push_twice(xs: list[int], v: int) -> None:",
    "class Point:",
    "    def __new__(cls, x: float, y: float) -> Point: ...",
    "    def scale(self, k: float) -> Point: ...",
    "    __hash__: typing.ClassVar[None]  # type: ignore[assignment]",
    "    def items(self) -> tuple[int, ...]:",
    "    def items(self, value: collections.abc.Sequence[int]) -> None: ...",
    "    def table(self) -> types.MappingProxyType[str, tuple[int, ...]]:",
    "    def tags(self) -> list[str]:",
    "    Red: typing.Final[Color]",
    "    def __new__(cls, _not_constructible: typing.NoReturn, /) -> Color: ...",
    "def favorite() -> Color: ...",
]:
    assert line in init.splitlines(), f"missing in demo/__init__.pyi: {line!r}\n{init}"
assert "_orochi_stubs" not in init
assert "def area(r: float) -> float:" in stubs["demo/geometry.pyi"]
out_dir = sys.argv[1] if len(sys.argv) > 1 else tempfile.mkdtemp()
assert demo._orochi_stubs(out_dir=pathlib.Path(out_dir)) == stubs
for path, text in stubs.items():
    assert (pathlib.Path(out_dir) / path).read_text(encoding="utf-8") == text

# A `__del__` that calls the extension while the interpreter shuts down (the
# cycle is collected after `Py_IsInitialized()` already returns false).
SHUTDOWN = """
import demo

class Cycle:
    def __init__(self):
        self.me = self

    def __del__(self):
        print("lines:", demo.count_lines("a\\nb"))

c = Cycle()
"""
done = subprocess.run([sys.executable, "-c", SHUTDOWN], capture_output=True, text=True)
assert done.returncode == 0 and "panicked" not in done.stderr, done
assert "lines: 2" in done.stdout, done

print("demo extension module OK:", demo.__file__)
