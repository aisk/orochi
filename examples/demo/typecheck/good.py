"""Correct use of the demo module: must type-check cleanly (`run.ps1 -TypeCheck`)."""

import pathlib

import demo
import demo.geometry
from demo import Basket, Color, Point

total: int = demo.add(1) + demo.add(a=1, b=None)
lines: int = demo.count_lines("a\nb")
xs: list[int] = [1]
demo.push_twice(xs, 7)
counts: dict[str, int] = demo.word_counts("a b a")


def read(path: pathlib.Path) -> str:
    return demo.read_text(path) + demo.read_text("relative/name.txt")


p = Point(3, 4)
chained: Point = p.scale(2).scale(0.5)
norm: float = p.norm()
p.x = 6  # an int is fine where a float is expected
same = p == Point(6, 4)

b = Basket()
other = Basket()
b.items.append("apple")
b.items += ["pear", "fig"]
b.items.sort()
b.table.setdefault("k", []).append(1)
b.table["k"] += [2]
demo.push_twice(b.numbers, 7)  # a field's view works where `&mut Vec` is expected
b.items = ["plum"]
b.table = {"k": [1, 2]}
b.table = other.table  # copies the other basket's table
b.numbers = other.numbers
first: str = b.items[0]
plain: list[str] = b.items.copy()
table: dict[str, list[int]] = b.table.copy()

c: Color = demo.favorite()
red = c is Color.Red or c == Color.Green
area: float = demo.geometry.area(1.0) + demo.geometry.area(r=2)
