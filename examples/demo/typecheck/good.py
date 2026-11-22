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
b.tags.append("x")  # a `List<str>` field is the Python list itself
demo.push_twice([1], 7)
b.items = [1, 2]  # a value field is replaced as a whole
b.items = b.items  # its snapshot is accepted back
b.table = {"k": [1, 2]}
b.table = other.table
first: int = b.items[0]
items: tuple[int, ...] = b.items
mutable: list[int] = list(b.items)
row: tuple[int, ...] = b.table["k"]
tags: list[str] = b.tags

c: Color = demo.favorite()
red = c is Color.Red or c == Color.Green
area: float = demo.geometry.area(1.0) + demo.geometry.area(r=2)
