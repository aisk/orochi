"""Live views of container fields of orochi classes.

Reading a `pub` field of type `Vec<T>`, `HashMap`/`BTreeMap` or
`HashSet`/`BTreeSet` returns one of these views instead of a copy. A view is
bound to (instance, field): every operation copies the field out as a real
`list`/`dict`/`set` (through the field's getter), applies the operation to
that copy and, for mutating operations, stores the copy back through the
field's setter. The setter converts strictly, so an element of the wrong type
raises `TypeError` at once and leaves the field unchanged.

An element read through a view that is itself a `list`/`dict`/`set` is
returned as a child view that addresses its position (index or key) in the
parent, so `obj.rows[0].append(1)` changes the field too.

This module is an implementation detail of orochi.
"""

from collections.abc import MutableMapping, MutableSequence, MutableSet, Set
import copy as _copy
import operator as _operator

__all__ = ["View", "ListView", "DictView", "SetView"]


def _plain(value):
    """The current builtin contents of a view, or `value` itself."""
    if isinstance(value, View):
        return value._get()
    return value


class View:
    __slots__ = ("_get_raw", "_set_raw", "_key")
    _builtin = object

    def __init__(self, get, set, key):
        self._get_raw = get
        self._set_raw = set
        self._key = key

    def _get(self):
        value = self._get_raw()
        if type(value) is not self._builtin:
            raise TypeError(
                f"the viewed field is no longer a {self._builtin.__name__} "
                f"(it is {type(value).__name__})"
            )
        return value

    def _mutate(self, operation):
        value = self._get()
        result = operation(value)
        self._set_raw(value)
        return result

    def _child(self, key, value):
        kind = _VIEW_TYPES.get(type(value))
        if kind is None:
            return value

        def get():
            return self._get()[key]

        def set(new):
            self._mutate(lambda c: c.__setitem__(key, new))

        return kind(get, set, self._key + (key,))

    def copy(self):
        """A real builtin copy of the current contents."""
        return self._get().copy()

    __copy__ = copy

    def __deepcopy__(self, memo):
        return _copy.deepcopy(self._get(), memo)

    def __reduce__(self):
        return (self._builtin, (self.copy(),))

    def __repr__(self):
        return repr(self._get())

    def __str__(self):
        return str(self._get())

    def __len__(self):
        return len(self._get())

    def __bool__(self):
        return bool(self._get())

    def __eq__(self, other):
        return self._get() == _plain(other)

    def __ne__(self, other):
        return self._get() != _plain(other)

    __hash__ = None


class ListView(View, MutableSequence):
    """A live view of a `Vec<T>` field; behaves like a `list`."""

    __slots__ = ()
    _builtin = list
    __hash__ = None

    def _index(self, index, length):
        index = _operator.index(index)
        return index + length if index < 0 else index

    def __getitem__(self, index):
        value = self._get()
        if isinstance(index, slice):
            return value[index]
        item = value[index]
        return self._child(self._index(index, len(value)), item)

    def __setitem__(self, index, value):
        if isinstance(index, slice):
            value = [_plain(v) for v in value]
        else:
            value = _plain(value)
        self._mutate(lambda c: c.__setitem__(index, value))

    def __delitem__(self, index):
        self._mutate(lambda c: c.__delitem__(index))

    def __iter__(self):
        for index, item in enumerate(self._get()):
            yield self._child(index, item)

    def __reversed__(self):
        value = self._get()
        for index in range(len(value) - 1, -1, -1):
            yield self._child(index, value[index])

    def __contains__(self, item):
        return _plain(item) in self._get()

    def __lt__(self, other):
        return self._get() < _plain(other)

    def __le__(self, other):
        return self._get() <= _plain(other)

    def __gt__(self, other):
        return self._get() > _plain(other)

    def __ge__(self, other):
        return self._get() >= _plain(other)

    def __add__(self, other):
        return self._get() + _plain(other)

    def __radd__(self, other):
        return _plain(other) + self._get()

    def __mul__(self, n):
        return self._get() * n

    __rmul__ = __mul__

    def __iadd__(self, other):
        self.extend(other)
        return self

    def __imul__(self, n):
        self._mutate(lambda c: c.__imul__(n))
        return self

    def append(self, value):
        value = _plain(value)
        self._mutate(lambda c: c.append(value))

    def extend(self, values):
        values = [_plain(v) for v in values]
        self._mutate(lambda c: c.extend(values))

    def insert(self, index, value):
        value = _plain(value)
        self._mutate(lambda c: c.insert(index, value))

    def pop(self, index=-1):
        return self._mutate(lambda c: c.pop(index))

    def remove(self, value):
        value = _plain(value)
        self._mutate(lambda c: c.remove(value))

    def clear(self):
        self._mutate(lambda c: c.clear())

    def reverse(self):
        self._mutate(lambda c: c.reverse())

    def sort(self, *, key=None, reverse=False):
        self._mutate(lambda c: c.sort(key=key, reverse=reverse))

    def index(self, value, *args):
        return self._get().index(_plain(value), *args)

    def count(self, value):
        return self._get().count(_plain(value))


class DictView(View, MutableMapping):
    """A live view of a `HashMap`/`BTreeMap` field; behaves like a `dict`."""

    __slots__ = ()
    _builtin = dict
    __hash__ = None

    def __getitem__(self, key):
        return self._child(key, self._get()[key])

    def __setitem__(self, key, value):
        value = _plain(value)
        self._mutate(lambda c: c.__setitem__(key, value))

    def __delitem__(self, key):
        self._mutate(lambda c: c.__delitem__(key))

    def __iter__(self):
        return iter(list(self._get()))

    def __reversed__(self):
        return reversed(list(self._get()))

    def __contains__(self, key):
        return key in self._get()

    def get(self, key, default=None):
        value = self._get()
        if key in value:
            return self._child(key, value[key])
        return default

    def pop(self, key, *default):
        return self._mutate(lambda c: c.pop(key, *default))

    def popitem(self):
        return self._mutate(lambda c: c.popitem())

    def setdefault(self, key, default=None):
        if key not in self._get():
            default = _plain(default)
            self._mutate(lambda c: c.__setitem__(key, default))
        return self[key]

    def update(self, *args, **kwargs):
        new = dict(*args, **kwargs)
        new = {k: _plain(v) for k, v in new.items()}
        self._mutate(lambda c: c.update(new))

    def clear(self):
        self._mutate(lambda c: c.clear())

    def __or__(self, other):
        return self._get() | _plain(other)

    def __ror__(self, other):
        return _plain(other) | self._get()

    def __ior__(self, other):
        self.update(_plain(other))
        return self


class SetView(View, MutableSet):
    """A live view of a `HashSet`/`BTreeSet` field; behaves like a `set`."""

    __slots__ = ()
    _builtin = set
    __hash__ = None

    @classmethod
    def _from_iterable(cls, iterable):
        return set(iterable)

    def __iter__(self):
        return iter(list(self._get()))

    def __contains__(self, value):
        return value in self._get()

    def add(self, value):
        self._mutate(lambda c: c.add(value))

    def discard(self, value):
        self._mutate(lambda c: c.discard(value))

    def remove(self, value):
        self._mutate(lambda c: c.remove(value))

    def pop(self):
        return self._mutate(lambda c: c.pop())

    def clear(self):
        self._mutate(lambda c: c.clear())

    def update(self, *others):
        others = [_plain(o) for o in others]
        self._mutate(lambda c: c.update(*others))

    def difference_update(self, *others):
        others = [_plain(o) for o in others]
        self._mutate(lambda c: c.difference_update(*others))

    def intersection_update(self, *others):
        others = [_plain(o) for o in others]
        self._mutate(lambda c: c.intersection_update(*others))

    def symmetric_difference_update(self, other):
        other = _plain(other)
        self._mutate(lambda c: c.symmetric_difference_update(other))

    def union(self, *others):
        return self._get().union(*[_plain(o) for o in others])

    def intersection(self, *others):
        return self._get().intersection(*[_plain(o) for o in others])

    def difference(self, *others):
        return self._get().difference(*[_plain(o) for o in others])

    def symmetric_difference(self, other):
        return self._get().symmetric_difference(_plain(other))

    def issubset(self, other):
        return self._get().issubset(_plain(other))

    def issuperset(self, other):
        return self._get().issuperset(_plain(other))

    def isdisjoint(self, other):
        return self._get().isdisjoint(_plain(other))

    def __or__(self, other):
        other = _as_set(other)
        return NotImplemented if other is NotImplemented else self._get() | other

    def __and__(self, other):
        other = _as_set(other)
        return NotImplemented if other is NotImplemented else self._get() & other

    def __sub__(self, other):
        other = _as_set(other)
        return NotImplemented if other is NotImplemented else self._get() - other

    def __xor__(self, other):
        other = _as_set(other)
        return NotImplemented if other is NotImplemented else self._get() ^ other

    def __rsub__(self, other):
        other = _as_set(other)
        return NotImplemented if other is NotImplemented else other - self._get()

    __ror__ = __or__
    __rand__ = __and__
    __rxor__ = __xor__

    def _inplace(self, other, operation):
        other = _as_set(other)
        if other is NotImplemented:
            return NotImplemented
        self._mutate(lambda c: operation(c, other))
        return self

    def __ior__(self, other):
        return self._inplace(other, set.__ior__)

    def __iand__(self, other):
        return self._inplace(other, set.__iand__)

    def __isub__(self, other):
        return self._inplace(other, set.__isub__)

    def __ixor__(self, other):
        return self._inplace(other, set.__ixor__)

    def _compare(self, other, operation):
        other = _as_set(other)
        return NotImplemented if other is NotImplemented else operation(self._get(), other)

    def __lt__(self, other):
        return self._compare(other, _operator.lt)

    def __le__(self, other):
        return self._compare(other, _operator.le)

    def __gt__(self, other):
        return self._compare(other, _operator.gt)

    def __ge__(self, other):
        return self._compare(other, _operator.ge)


def _as_set(other):
    """`other` as a builtin set for set operators, or `NotImplemented`."""
    other = _plain(other)
    if isinstance(other, (set, frozenset)):
        return other
    if isinstance(other, Set):
        return set(other)
    return NotImplemented

_VIEW_TYPES = {list: ListView, dict: DictView, set: SetView}


def alias_key(obj):
    """What a `&mut` argument refers to: a view's field position, otherwise
    the object's identity."""
    if isinstance(obj, View):
        return obj._key
    return id(obj)
