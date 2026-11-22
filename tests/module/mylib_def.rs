//! The module under test.

#[orochi::module]
pub mod mylib {
    #![doc = concat!("A test ", "library.")]
    use orochi::prelude::*;
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::path::{Path, PathBuf};

    pub const VERSION: &str = "1.2.3";

    /// Add two integers.
    pub fn add(a: i64, b: Option<i64>) -> i64 {
        a + b.unwrap_or(1)
    }

    pub fn read_text(path: PathBuf) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn count_lines(text: &str) -> usize {
        text.lines().count()
    }

    pub fn total(xs: &[f64]) -> f64 {
        xs.iter().sum()
    }

    pub fn file_name(path: &Path) -> Option<String> {
        path.file_name().map(|n| n.to_string_lossy().into_owned())
    }

    pub fn lookup(map: &HashMap<String, i64>, key: &str) -> Option<i64> {
        map.get(key).copied()
    }

    pub fn greet(name: Option<&str>) -> String {
        format!("hello {}", name.unwrap_or("world"))
    }

    pub fn first_word(text: &str) -> &str {
        text.split_whitespace().next().unwrap_or("")
    }

    pub fn push_twice(xs: &mut Vec<i64>, v: i64) {
        xs.push(v);
        xs.push(v);
    }

    /// Pushes `v`, then fails if the list is now longer than `limit`.
    pub fn push_checked(xs: &mut Vec<i64>, v: i64, limit: usize) -> Result<()> {
        xs.push(v);
        if xs.len() > limit {
            return Err(Error::value_error("too long"));
        }
        Ok(())
    }

    pub fn bump(counts: &mut HashMap<String, i64>, key: String) {
        *counts.entry(key).or_insert(0) += 1;
    }

    pub fn sorted_insert(m: &mut BTreeMap<i64, String>, k: i64) {
        m.insert(k, k.to_string());
    }

    pub fn add_tag(tags: &mut HashSet<String>, tag: &str) -> bool {
        tags.insert(tag.to_owned())
    }

    /// The handle type: refers to the caller's list beyond a copy.
    pub fn push_handle(xs: List<i64>, v: i64) -> Result<()> {
        xs.push(v)?;
        xs.push(v)
    }

    pub fn copy_back(xs: Vec<i64>) -> Vec<i64> {
        xs
    }

    #[allow(private_interfaces)]
    pub fn fail(kind: &str) -> std::result::Result<(), MyError> {
        Err(MyError(kind.to_owned()))
    }

    pub fn boom() {
        panic!("boom from Rust");
    }

    /// A parameter named like the function.
    pub fn scale(scale: f64) -> f64 {
        scale * 2.0
    }

    /// Parameter names that pyo3 or orochi might use internally.
    pub fn reserved_names(
        py: i64,
        slf: i64,
        args: i64,
        kwargs: i64,
        module: i64,
        r#type: i64,
    ) -> i64 {
        py + slf + args + kwargs + module + r#type
    }

    /// A leading underscore must not trigger `non_snake_case` in generated code.
    pub fn _under() -> i64 {
        1
    }

    pub fn touch_vec(xs: &mut Vec<String>) {
        let _ = xs;
    }

    pub fn touch_map(m: &mut HashMap<String, String>) {
        let _ = m;
    }

    pub fn touch_set(s: &mut HashSet<String>) {
        let _ = s;
    }

    pub fn set_value(m: &mut HashMap<String, String>, key: &str, value: &str) {
        m.insert(key.to_owned(), value.to_owned());
    }

    pub fn remove_key(m: &mut BTreeMap<String, String>, key: &str) {
        m.remove(key);
    }

    pub fn remove_elem(s: &mut HashSet<String>, elem: &str) {
        s.remove(elem);
    }

    pub fn merge_into(dst: &mut Vec<i64>, src: &mut Vec<i64>) {
        dst.append(src);
    }

    pub fn extend_from(dst: &mut Vec<i64>, src: &[i64]) {
        dst.extend_from_slice(src);
    }

    #[allow(dead_code)]
    fn helper() {}

    pub(crate) fn crate_only() -> i64 {
        7
    }

    /// An error type that only implements `Display`.
    #[derive(Debug)]
    pub(crate) struct MyError(String);

    impl std::fmt::Display for MyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "my error: {}", self.0)
        }
    }

    /// A 2D point.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Point {
        /// Horizontal coordinate.
        pub x: f64,
        pub y: f64,
        cache: Option<f64>,
    }

    impl Point {
        pub const DIMENSIONS: usize = 2;

        /// Creates a point.
        pub fn new(x: f64, y: f64) -> Self {
            Point { x, y, cache: None }
        }

        /// Euclidean norm.
        pub fn norm(&self) -> f64 {
            (self.x * self.x + self.y * self.y).sqrt()
        }

        pub fn scale(&mut self, k: f64) -> &mut Self {
            self.x *= k;
            self.y *= k;
            self.cache = None;
            self
        }

        pub fn checked_scale(&mut self, k: f64) -> Result<&mut Self> {
            if k == 0.0 {
                return Err(Error::value_error("k must not be zero"));
            }
            Ok(self.scale(k))
        }

        pub fn origin() -> Self {
            Point::new(0.0, 0.0)
        }

        /// A reference to another instance: returned as a copy.
        pub fn origin_ref(&self) -> &'static Point {
            &ORIGIN
        }

        pub fn dist(&self, other: &Point) -> f64 {
            ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
        }

        /// Moves `other` onto `self`.
        pub fn pull(&self, other: &mut Point) {
            other.x = self.x;
            other.y = self.y;
        }

        /// Adds `other` into `self` (conflicts when `other is self`).
        pub fn absorb(&mut self, other: &Point) {
            self.x += other.x;
            self.y += other.y;
        }

        pub fn swapped(self) -> Point {
            Point::new(self.y, self.x)
        }

        pub fn cached_norm(&mut self) -> f64 {
            *self
                .cache
                .get_or_insert_with(|| (self.x * self.x + self.y * self.y).sqrt())
        }

        #[allow(dead_code)]
        fn internal(&self) {}
    }

    impl std::fmt::Display for Point {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "({}, {})", self.x, self.y)
        }
    }

    /// Hashable, with fallible constructor.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct Tag {
        pub name: String,
    }

    impl Tag {
        pub fn new(name: String) -> Result<Self> {
            if name.is_empty() {
                return Err(Error::value_error("empty tag"));
            }
            Ok(Tag { name })
        }
    }

    /// Not `Clone`: can only be borrowed or shared.
    #[derive(Default)]
    pub struct Counter {
        pub count: i64,
    }

    impl Counter {
        pub fn new() -> Self {
            Counter { count: 0 }
        }

        pub fn incr(&mut self) -> i64 {
            self.count += 1;
            self.count
        }
    }

    pub fn incr_counter(c: &mut Counter) -> i64 {
        c.incr()
    }

    pub fn read_counter(c: &Counter) -> i64 {
        c.count
    }

    pub struct Circle {
        pub center: Ref<Point>,
        pub radius: f64,
    }

    impl Circle {
        pub fn new(center: Ref<Point>, radius: f64) -> Self {
            Circle { center, radius }
        }

        pub fn center_x(&self) -> Result<f64> {
            Ok(self.center.borrow()?.x)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Color {
        Red,
        Green,
    }

    pub fn is_red(c: Color) -> bool {
        c == Color::Red
    }

    pub fn favorite() -> Color {
        Color::Red
    }

    /// No derives at all: members still compare and hash, and convert.
    pub enum Shade {
        Light,
        Dark,
    }

    impl Shade {
        pub fn inverted(self) -> Shade {
            match self {
                Shade::Light => Shade::Dark,
                Shade::Dark => Shade::Light,
            }
        }

        pub fn is_dark(&self) -> bool {
            matches!(self, Shade::Dark)
        }
    }

    pub fn darkest(shades: Vec<Shade>) -> Option<Shade> {
        shades.into_iter().find(|s| s.is_dark())
    }

    /// Not `Clone`; methods return references to itself or to a static.
    pub struct Node {
        pub id: i64,
    }

    static SHARED_NODE: Node = Node { id: -1 };
    static ORIGIN: Point = Point {
        x: 0.0,
        y: 0.0,
        cache: None,
    };

    impl Node {
        pub fn new(id: i64) -> Self {
            Node { id }
        }

        pub fn me(&self) -> &Node {
            self
        }

        pub fn shared(&self) -> &'static Node {
            &SHARED_NODE
        }

        pub fn pick(&mut self, own: bool) -> Result<&Self> {
            Ok(if own { self } else { &SHARED_NODE })
        }
    }

    #[cfg(test)]
    impl Node {
        pub fn enabled(&self) -> bool {
            true
        }
    }

    #[cfg(not(test))]
    impl Node {
        pub fn disabled(&self) -> NotAType {
            unreachable!()
        }
    }

    /// Only comparable with plain numbers, so no `__eq__` is generated.
    pub struct Meters {
        pub value: f64,
    }

    impl Meters {
        pub fn new(value: f64) -> Self {
            Meters { value }
        }
    }

    impl PartialEq<f64> for Meters {
        fn eq(&self, other: &f64) -> bool {
            self.value == *other
        }
    }

    /// Holds a Python object, to observe when the old value of a field is
    /// dropped.
    pub struct Holder {
        pub obj: Object,
        pub count: i64,
    }

    #[allow(clippy::new_without_default)]
    impl Holder {
        pub fn new() -> Self {
            Holder {
                obj: Object::none(),
                count: 0,
            }
        }
    }

    /// Container fields read as live views.
    #[derive(Default)]
    pub struct Bag {
        pub items: Vec<i64>,
        pub rows: Vec<Vec<i64>>,
        pub table: HashMap<String, Vec<i64>>,
        pub counts: HashMap<String, i64>,
        pub tags: BTreeSet<String>,
        pub maybe: Option<Vec<i64>>,
        pub fixed: [i64; 2],
        pub data: Vec<u8>,
        pub label: String,
    }

    impl Bag {
        pub fn new() -> Self {
            Bag::default()
        }

        pub fn total(&self) -> i64 {
            self.items.iter().sum()
        }

        pub fn row_len(&self, i: usize) -> usize {
            self.rows[i].len()
        }

        pub fn table_len(&self, key: &str) -> Option<usize> {
            self.table.get(key).map(Vec::len)
        }

        pub fn tag_list(&self) -> Vec<String> {
            self.tags.iter().cloned().collect()
        }

        /// Calls `f` while `self` is mutably borrowed.
        pub fn with_callback(&mut self, f: Object) -> Result<Object> {
            f.call(())
        }
    }

    pub mod geometry {
        #![doc = concat!("Geometry ", "helpers.")]

        /// Not the root module's class `Point`.
        type Point = (f64, f64);

        pub struct Segment {
            pub a: Point,
            pub b: Point,
        }

        impl Segment {
            pub fn new(a: Point, b: Point) -> Self {
                Segment { a, b }
            }
        }

        /// Area of a circle.
        pub fn area(r: f64) -> f64 {
            std::f64::consts::PI * r * r
        }

        pub struct Square {
            pub side: f64,
        }

        impl Square {
            pub fn new(side: f64) -> Self {
                Square { side }
            }
        }
    }

    #[allow(dead_code)]
    mod private_mod {
        pub fn hidden() {}
    }
}
