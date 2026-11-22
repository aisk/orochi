"""Misuse of the demo module: each marked line must be reported by the type
checker with the error code after `expect:` (`run.ps1 -TypeCheck`)."""

import typing

import demo
import demo.geometry
from demo import Basket, Color, Point

demo.add(1.5)  # expect: arg-type
demo.add(a="1")  # expect: arg-type
demo.push_twice((1, 2), 3)  # expect: arg-type
demo.count_lines(b"bytes")  # expect: arg-type
demo.read_text(1)  # expect: arg-type
text: int = demo.count_lines("x") + demo.favorite()  # expect: operator

p = Point(1, 2)
p.x = "no"  # expect: assignment
p.z = 1.0  # expect: attr-defined
label: str = p.norm()  # expect: assignment
key: typing.Hashable = p  # expect: assignment

b = Basket()
b.items.append(1)  # expect: attr-defined
copy: list[int] = b.items  # expect: assignment
b.items = ["x"]  # expect: list-item
b.table["k"] = [1]  # expect: index
b.tags = b.items  # expect: assignment

green: Color = Color.Green
Color()  # expect: call-arg
Color.Red = Color.Green  # expect: misc
wrong: Point = demo.favorite()  # expect: assignment
demo.geometry.area("1")  # expect: arg-type


class Sub(Point):  # expect: misc
    pass
