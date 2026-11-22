use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use orochi::prelude::*;
use orochi::{eval, exec};

fn round_trip<T>(value: T) -> Result<T>
where
    T: IntoObject + FromObject,
{
    value.into_object()?.to()
}

fn type_error<T: FromObject>(expr: &str) -> String {
    let err = eval(expr).unwrap().to::<T>().err().unwrap_or_else(|| panic!("`{expr}` converted"));
    assert!(err.is_instance_of("TypeError"), "{err}");
    err.message()
}

fn overflow<T: FromObject>(expr: &str) {
    let err = eval(expr).unwrap().to::<T>().err().unwrap_or_else(|| panic!("`{expr}` converted"));
    assert_eq!(err.type_name(), "OverflowError", "{expr}: {err}");
}

#[test]
fn integers() -> Result<()> {
    assert_eq!(round_trip(-5i8)?, -5);
    assert_eq!(round_trip(u64::MAX)?, u64::MAX);
    assert_eq!(round_trip(i128::MIN)?, i128::MIN);
    assert_eq!(round_trip(u128::MAX)?, u128::MAX);
    assert_eq!(round_trip(usize::MAX)?, usize::MAX);
    assert_eq!(eval("2 ** 40")?.to::<i64>()?, 1 << 40);

    // Out of range is always OverflowError, never a silent wrap or ValueError.
    overflow::<u8>("300");
    overflow::<u8>("-1");
    overflow::<u16>("-1");
    overflow::<u32>("-1");
    overflow::<u64>("-1");
    overflow::<u128>("-1");
    overflow::<usize>("-1");
    overflow::<u128>("2 ** 128");
    overflow::<i128>("2 ** 127");
    overflow::<i128>("-(2 ** 127) - 1");
    overflow::<i64>("2 ** 63");
    overflow::<i8>("-129");

    // Strictness.
    assert_eq!(type_error::<i64>("1.0"), "expected int, got float");
    assert_eq!(type_error::<i64>("True"), "expected int, got bool");
    assert_eq!(type_error::<u64>("False"), "expected int, got bool");
    assert_eq!(type_error::<i64>("'1'"), "expected int, got str");
    assert_eq!(type_error::<i64>("int"), "expected int, got type");
    type_error::<u8>("None");

    // int subclasses and __index__ objects are integers.
    assert_eq!(eval("__import__('enum').IntEnum('E', 'A B').B")?.to::<i64>()?, 2);
    let scope = exec("class I:\n    def __index__(self):\n        return 7\ni = I()")?;
    assert_eq!(scope.get("i")?.to::<u64>()?, 7);
    Ok(())
}

#[test]
fn floats() -> Result<()> {
    assert_eq!(round_trip(1.5f64)?, 1.5);
    assert_eq!(round_trip(0.25f32)?, 0.25);
    assert!(round_trip(f64::NAN)?.is_nan());
    // int -> float is allowed.
    assert_eq!(eval("3")?.to::<f64>()?, 3.0);
    assert_eq!(type_error::<f64>("True"), "expected float, got bool");
    type_error::<f64>("'1.0'");
    type_error::<f64>("None");
    overflow::<f64>("10 ** 400");

    // f32: finite values that do not fit overflow; inf/nan pass through.
    overflow::<f32>("1e300");
    overflow::<f32>("-1e300");
    overflow::<f32>("10 ** 50");
    assert_eq!(eval("float('inf')")?.to::<f32>()?, f32::INFINITY);
    assert_eq!(eval("float('-inf')")?.to::<f32>()?, f32::NEG_INFINITY);
    assert!(eval("float('nan')")?.to::<f32>()?.is_nan());
    assert_eq!(eval("3.4e38")?.to::<f32>()?, 3.4e38f32);
    assert_eq!(eval("1e-50")?.to::<f32>()?, 0.0); // underflow rounds, as in C
    Ok(())
}

#[test]
fn bools() -> Result<()> {
    assert!(round_trip(true)?);
    assert!(!round_trip(false)?);
    assert_eq!(true.into_object()?.type_name(), "bool");
    assert_eq!(type_error::<bool>("1"), "expected bool, got int");
    type_error::<bool>("None");
    type_error::<bool>("''");
    Ok(())
}

#[test]
fn strings() -> Result<()> {
    assert_eq!(round_trip("héllo ✓".to_owned())?, "héllo ✓");
    assert_eq!("abc".into_object()?.to::<String>()?, "abc");
    assert_eq!((&"x".to_owned()).into_object()?.to::<String>()?, "x");
    assert_eq!(type_error::<String>("b'abc'"), "expected str, got bytes");
    type_error::<String>("1");

    assert_eq!(round_trip('λ')?, 'λ');
    assert!(eval("'ab'")?.to::<char>().unwrap_err().is_instance_of("ValueError"));

    let owned: Cow<'static, str> = round_trip(Cow::Borrowed("cow"))?;
    assert!(matches!(owned, Cow::Owned(ref s) if s == "cow"));
    type_error::<Cow<'static, str>>("1");
    Ok(())
}

#[test]
fn none_and_option() -> Result<()> {
    assert!(().into_object()?.is_none());
    eval("None")?.to::<()>()?;
    type_error::<()>("0");

    assert_eq!(round_trip(Some(3i64))?, Some(3));
    assert_eq!(round_trip(None::<i64>)?, None);
    assert!(None::<i64>.into_object()?.is_none());
    // Option does not make the inner conversion lenient.
    type_error::<Option<i64>>("1.5");
    Ok(())
}

#[test]
fn boxes() -> Result<()> {
    assert_eq!(*round_trip(Box::new(5i64))?, 5);
    assert_eq!(<Box<Vec<i64>> as FromObject>::input_hint(), "collections.abc.Sequence[int]");
    type_error::<Box<i64>>("'x'");
    Ok(())
}

#[test]
fn vectors() -> Result<()> {
    assert_eq!(round_trip(vec![1i64, 2, 3])?, [1, 2, 3]);
    let obj = vec!["a", "b"].into_object()?;
    assert_eq!(obj.type_name(), "list");
    assert_eq!([1i32, 2].as_slice().into_object()?.to::<Vec<i32>>()?, [1, 2]);
    let v = vec![3i64, 4];
    assert_eq!((&v).into_object()?.type_name(), "list");
    assert_eq!((&v).into_object()?.to::<Vec<i64>>()?, v);

    // Any non-text sequence is accepted.
    assert_eq!(eval("(1, 2)")?.to::<Vec<i64>>()?, [1, 2]);
    assert_eq!(eval("range(3)")?.to::<Vec<i64>>()?, [0, 1, 2]);

    // Strictness: str/bytes/buffers are not sequences here; elements are strict.
    type_error::<Vec<String>>("'abc'");
    type_error::<Vec<i64>>("b'abc'");
    type_error::<Vec<i64>>("bytearray(b'abc')");
    type_error::<Vec<i64>>("memoryview(b'abc')");
    type_error::<Vec<i64>>("{1: 2}");
    type_error::<Vec<i64>>("{1, 2}");
    assert_eq!(type_error::<Vec<i64>>("[1, 2.5]"), "expected int, got float");

    // Nested.
    assert_eq!(round_trip(vec![vec![1i64], vec![]])?, vec![vec![1], vec![]]);

    // A copy: Rust-side changes do not reach Python.
    let py_list = eval("[1]")?;
    let mut v: Vec<i64> = py_list.to()?;
    v.push(2);
    assert_eq!(py_list.len()?, 1);
    Ok(())
}

#[test]
fn huge_lazy_sequence_does_not_abort() -> Result<()> {
    // `len()` is 10**13: must not be used to preallocate. Element conversion
    // fails at the first bad element, long before memory runs out.
    let scope = exec(
        "class Big:\n    def __len__(self):\n        return 10 ** 13\n    \
         def __getitem__(self, i):\n        if i < 3:\n            return i\n        \
         if i == 3:\n            return 'stop'\n        raise IndexError(i)\n\
         import collections.abc\ncollections.abc.Sequence.register(Big)\nbig = Big()",
    )?;
    let err = scope.get("big")?.to::<Vec<i64>>().unwrap_err();
    assert!(err.is_instance_of("TypeError"));
    Ok(())
}

#[test]
fn bytes() -> Result<()> {
    let obj = vec![1u8, 2, 255].into_object()?;
    assert_eq!(obj.type_name(), "bytes");
    assert!(obj.eq(eval("b'\\x01\\x02\\xff'")?)?);
    assert_eq!(b"hi".as_slice().into_object()?.type_name(), "bytes");
    assert_eq!((&vec![0u8]).into_object()?.type_name(), "bytes");
    assert_eq!([7u8; 3].into_object()?.type_name(), "bytes");

    assert_eq!(eval("b'abc'")?.to::<Vec<u8>>()?, b"abc");
    assert_eq!(eval("bytearray(b'xy')")?.to::<Vec<u8>>()?, b"xy");
    assert_eq!(eval("[1, 2]")?.to::<Vec<u8>>()?, [1, 2]);
    assert_eq!(eval("b'abcd'")?.to::<[u8; 4]>()?, *b"abcd");

    // memoryview is rejected (no buffer support), as are str and bad ints.
    assert_eq!(
        type_error::<Vec<u8>>("memoryview(b'abc')"),
        "expected bytes, bytearray or a sequence of int, got memoryview"
    );
    type_error::<Vec<u8>>("'abc'");
    assert!(eval("[256]")?.to::<Vec<u8>>().unwrap_err().is_instance_of("OverflowError"));

    // A List<u8> handle is still a list.
    let l = List::from_vec(vec![1u8, 2])?;
    assert_eq!(l.as_object().type_name(), "list");

    assert_eq!(<Vec<u8> as IntoObject>::output_hint(), "bytes");
    assert_eq!(<[u8; 4] as IntoObject>::output_hint(), "bytes");
    assert_eq!(
        <Vec<u8> as FromObject>::input_hint(),
        "bytes | bytearray | collections.abc.Sequence[int]"
    );
    assert_eq!(<Vec<Vec<u8>> as IntoObject>::output_hint(), "list[bytes]");
    Ok(())
}

#[test]
fn arrays() -> Result<()> {
    assert_eq!(round_trip([1i64, 2, 3])?, [1, 2, 3]);
    assert_eq!([1i64, 2].into_object()?.type_name(), "list");
    assert_eq!(eval("(1, 2)")?.to::<[i64; 2]>()?, [1, 2]);
    assert_eq!(
        type_error::<[i64; 2]>("[1, 2, 3]"),
        "expected a sequence of length 2, got length 3"
    );
    assert_eq!(<[f64; 3] as FromObject>::input_hint(), "collections.abc.Sequence[float]");
    Ok(())
}

#[test]
fn paths() -> Result<()> {
    let p = PathBuf::from("some/dir/file.txt");
    let obj = p.clone().into_object()?;
    assert_eq!(obj.type_name().trim_start_matches("Windows").trim_start_matches("Posix"), "Path");
    assert!(obj.is_instance(&orochi::import("pathlib")?.attr("Path")?)?);
    assert_eq!(obj.to::<PathBuf>()?, p);
    assert_eq!(Path::new("x").into_object()?.attr("name")?.to::<String>()?, "x");
    assert_eq!((&p).into_object()?.to::<PathBuf>()?, p);

    assert_eq!(eval("'a/b'")?.to::<PathBuf>()?, PathBuf::from("a/b"));
    let scope = exec("class P:\n    def __fspath__(self):\n        return 'from/fspath'\np = P()")?;
    assert_eq!(scope.get("p")?.to::<PathBuf>()?, PathBuf::from("from/fspath"));
    assert_eq!(type_error::<PathBuf>("1"), "expected str or os.PathLike, got int");
    type_error::<PathBuf>("b'bytes'");

    assert_eq!(<PathBuf as FromObject>::input_hint(), "str | os.PathLike[str]");
    assert_eq!(<PathBuf as IntoObject>::output_hint(), "pathlib.Path");
    assert_eq!(<&Path as IntoObject>::output_hint(), "pathlib.Path");
    Ok(())
}

#[test]
fn maps_and_sets() -> Result<()> {
    let mut m = HashMap::new();
    m.insert("a".to_owned(), 1.5f64);
    m.insert("b".to_owned(), 2.0);
    let obj = m.clone().into_object()?;
    assert_eq!(obj.type_name(), "dict");
    assert_eq!(obj.to::<HashMap<String, f64>>()?, m);
    assert_eq!(
        type_error::<HashMap<String, i64>>("[('a', 1)]"),
        "expected a mapping, got list"
    );
    // Any `collections.abc.Mapping` converts, not only a `dict`.
    let proxy = eval("__import__('types').MappingProxyType({'a': 1.5, 'b': 2.0})")?;
    assert_eq!(proxy.to::<HashMap<String, f64>>()?, m);
    type_error::<HashMap<String, i64>>("{'a': 1.5}");
    type_error::<HashMap<i64, i64>>("{'a': 1}");
    // `dict` subclasses are read from the dict itself, not an overridden
    // `items()`; other mappings through `items()`, whose entries must be
    // pairs.
    let scope = exec(
        "import collections\n\
         class Liar(dict):\n    def items(self): return [('x', 99)]\n\
         class Odd(collections.abc.Mapping):\n\
         \x20   def __init__(self, items): self._items = items\n\
         \x20   def __getitem__(self, k): raise KeyError(k)\n\
         \x20   def __iter__(self): return iter(())\n\
         \x20   def __len__(self): return 0\n\
         \x20   def items(self): return self._items\n",
    )?;
    let a1 = HashMap::from([("a".to_owned(), 1i64)]);
    let map = |expr: &str| scope.eval(expr)?.to::<HashMap<String, i64>>();
    assert_eq!(map("collections.OrderedDict(a=1)")?, a1);
    assert_eq!(map("collections.UserDict(a=1)")?, a1);
    assert_eq!(map("Liar(a=1)")?, a1);
    assert_eq!(map("Odd([('a', 1)])")?, a1);
    for bad in ["Odd([('a', 1, 2)])", "Odd(['ab'])", "Odd([['a', 1]])"] {
        let err = map(bad).unwrap_err();
        assert!(err.is_instance_of("TypeError"), "{err}");
        assert!(err.message().starts_with("expected a (key, value) tuple, got "), "{err}");
    }

    let s: HashSet<i64> = [1, 2, 3].into_iter().collect();
    let obj = s.clone().into_object()?;
    assert_eq!(obj.type_name(), "set");
    assert_eq!(obj.to::<HashSet<i64>>()?, s);
    assert_eq!(eval("frozenset([1])")?.to::<HashSet<i64>>()?, HashSet::from([1]));
    type_error::<HashSet<i64>>("[1, 2]");

    // Unhashable keys are a Python TypeError at conversion time.
    let mut bad = HashMap::new();
    bad.insert(vec![1i64], 1i64);
    assert!(bad.into_object().unwrap_err().is_instance_of("TypeError"));
    Ok(())
}

#[test]
fn btree_collections() -> Result<()> {
    let m = BTreeMap::from([(2i64, "b".to_owned()), (1, "a".to_owned())]);
    let obj = m.clone().into_object()?;
    assert_eq!(obj.to_string(), "{1: 'a', 2: 'b'}");
    assert_eq!(obj.to::<BTreeMap<i64, String>>()?, m);
    type_error::<BTreeMap<i64, String>>("[(1, 'a')]");

    let s = BTreeSet::from([3i64, 1, 2]);
    let obj = s.clone().into_object()?;
    assert_eq!(obj.type_name(), "set");
    assert_eq!(obj.to::<BTreeSet<i64>>()?, s);
    assert_eq!(eval("frozenset([5])")?.to::<BTreeSet<i64>>()?, BTreeSet::from([5]));
    assert_eq!(<BTreeMap<String, i64> as IntoObject>::output_hint(), "dict[str, int]");
    assert_eq!(<BTreeSet<i64> as FromObject>::input_hint(), "set[int] | frozenset[int]");
    Ok(())
}

#[test]
fn containers_mutated_during_conversion() -> Result<()> {
    // Converting an element runs Python code (`__index__`) that mutates the
    // container being converted. This must not panic; the snapshot taken
    // before converting is used.
    let scope = exec(
        "d = {}\ns = set()\n\
         class Evil:\n    def __index__(self):\n        \
         d.clear(); d.update({i: i for i in range(100)})\n        \
         s.clear(); s.update(range(100))\n        return 1\n\
         d.update({1: Evil(), 2: Evil()})\n\
         s.update({Evil(), Evil()})",
    )?;
    let d = scope.get("d")?;
    let m: HashMap<i64, i64> = d.to()?;
    assert_eq!(m, HashMap::from([(1, 1), (2, 1)]));
    scope.exec("d.clear(); d.update({1: Evil(), 2: Evil()})")?;
    let m: BTreeMap<i64, i64> = d.to()?;
    assert_eq!(m.len(), 2);
    scope.exec("d.clear(); d.update({1: Evil(), 2: Evil()})")?;
    let handle: Dict<i64, i64> = d.to()?;
    assert_eq!(handle.items()?.len(), 2);
    scope.exec("d.clear(); d.update({1: Evil(), 2: Evil()})")?;
    assert_eq!(handle.to_hashmap()?.len(), 2);

    // (The dict conversions above also rewrote `s`; refill it.)
    scope.exec("s.clear(); s.update({Evil(), Evil()})")?;
    let s = scope.get("s")?;
    let v: HashSet<i64> = s.to()?;
    assert_eq!(v, HashSet::from([1]));
    scope.exec("s.clear(); s.update({Evil(), Evil()})")?;
    let h: Set<i64> = s.to()?;
    assert_eq!(h.to_vec()?, [1, 1]);
    Ok(())
}

#[test]
fn tuples() -> Result<()> {
    assert_eq!(round_trip((1i64,))?, (1,));
    assert_eq!(round_trip((1i64, "a".to_owned(), 2.5f64))?, (1, "a".to_owned(), 2.5));
    let t12 = (1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, 10u8, 11u8, 12u8);
    assert_eq!(round_trip(t12)?, t12);
    assert_eq!((1, 2).into_object()?.type_name(), "tuple");

    type_error::<(i64, i64)>("[1, 2]");
    assert_eq!(type_error::<(i64, i64)>("(1, 2, 3)"), "expected a tuple of length 2, got length 3");
    type_error::<(i64, String)>("(1, 2)");
    Ok(())
}

#[test]
fn objects() -> Result<()> {
    let o = eval("object()")?;
    let o2: Object = o.clone().into_object()?;
    assert!(o.is(&o2));
    assert!(o.to::<Object>()?.is(&o));
    assert!((&o).into_object()?.is(&o));
    Ok(())
}

#[test]
fn result_into_object() -> Result<()> {
    let ok: Result<i64> = Ok(1);
    assert_eq!(ok.into_object()?.to::<i64>()?, 1);
    let err: Result<i64> = Err(Error::key_error("k"));
    assert!(err.into_object().unwrap_err().is_instance_of("KeyError"));

    // Any error convertible into orochi::Error.
    let io: std::result::Result<i64, std::io::Error> =
        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "nope"));
    assert!(io.into_object().unwrap_err().is_instance_of("FileNotFoundError"));
    let parsed: std::result::Result<i64, std::num::ParseIntError> = "5".parse();
    assert_eq!(parsed.into_object()?.to::<i64>()?, 5);
    assert_eq!(<std::result::Result<String, std::fmt::Error> as IntoObject>::output_hint(), "str");
    Ok(())
}

#[test]
fn type_hints() {
    assert_eq!(<i64 as FromObject>::input_hint(), "int");
    assert_eq!(<u8 as IntoObject>::output_hint(), "int");
    assert_eq!(<f32 as FromObject>::input_hint(), "float");
    assert_eq!(<bool as IntoObject>::output_hint(), "bool");
    assert_eq!(<String as FromObject>::input_hint(), "str");
    assert_eq!(<&str as IntoObject>::output_hint(), "str");
    assert_eq!(<Cow<'_, str> as IntoObject>::output_hint(), "str");
    assert_eq!(<() as IntoObject>::output_hint(), "None");
    assert_eq!(<Object as FromObject>::input_hint(), "typing.Any");
    assert_eq!(<Option<i64> as FromObject>::input_hint(), "int | None");
    assert_eq!(<Option<()> as FromObject>::input_hint(), "None");
    assert_eq!(<Option<()> as IntoObject>::output_hint(), "None");
    assert_eq!(<Option<Option<i64>> as FromObject>::input_hint(), "int | None");
    assert_eq!(<Vec<i64> as IntoObject>::output_hint(), "list[int]");
    assert_eq!(<Vec<i64> as FromObject>::input_hint(), "collections.abc.Sequence[int]");
    assert_eq!(<HashMap<String, f64> as IntoObject>::output_hint(), "dict[str, float]");
    assert_eq!(
        <BTreeMap<String, f64> as FromObject>::input_hint(),
        "collections.abc.Mapping[str, float]"
    );
    // Keys read back as `pathlib.Path`; `Mapping` is invariant in its keys.
    assert_eq!(
        <HashMap<PathBuf, i64> as FromObject>::input_hint(),
        "collections.abc.Mapping[str | os.PathLike[str], int] \
         | collections.abc.Mapping[pathlib.Path, int]"
    );
    assert_eq!(<HashSet<i64> as IntoObject>::output_hint(), "set[int]");
    assert_eq!(<HashSet<i64> as FromObject>::input_hint(), "set[int] | frozenset[int]");
    assert_eq!(<(i64, String) as FromObject>::input_hint(), "tuple[int, str]");
    assert_eq!(<Result<Vec<Object>> as IntoObject>::output_hint(), "list[typing.Any]");
    assert_eq!(<List<i64> as FromObject>::input_hint(), "list[int]");
    assert_eq!(<Dict<String, List<f64>> as IntoObject>::output_hint(), "dict[str, list[float]]");
    assert_eq!(<Set<String> as FromObject>::input_hint(), "set[str]");
}

#[test]
fn user_impls() -> Result<()> {
    #[derive(Debug, PartialEq)]
    struct Point {
        x: f64,
        y: f64,
    }
    impl IntoObject for Point {
        fn into_object(self) -> Result<Object> {
            (self.x, self.y).into_object()
        }
        fn output_hint() -> String {
            "tuple[float, float]".into()
        }
    }
    impl FromObject for Point {
        fn from_object(obj: &Object) -> Result<Self> {
            let (x, y) = obj.to()?;
            Ok(Point { x, y })
        }
        fn input_hint() -> String {
            "tuple[float, float]".into()
        }
    }
    let p = Point { x: 1.0, y: 2.0 };
    assert_eq!(round_trip(vec![p])?, [Point { x: 1.0, y: 2.0 }]);
    assert_eq!(<Vec<Point> as FromObject>::input_hint(), "collections.abc.Sequence[tuple[float, float]]");
    Ok(())
}
