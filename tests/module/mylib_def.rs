//! The module under test.

#[orochi::module]
pub mod mylib {
    #![doc = concat!("A test ", "library.")]
    use orochi::prelude::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    pub const VERSION: &str = "1.2.3";
    pub static LIMIT: i64 = 10;


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

    pub fn first_word(text: &str) -> String {
        text.split_whitespace().next().unwrap_or("").to_owned()
    }

    pub fn nothing() {}

    /// The handle type: refers to the caller's list itself.
    pub fn push_handle(xs: List<i64>, v: i64) -> Result<()> {
        xs.push(v)?;
        xs.push(v)
    }

    pub fn copy_back(xs: Vec<i64>) -> Vec<i64> {
        xs
    }

    pub fn nested_sum(rows: Vec<HashMap<String, Vec<i64>>>) -> i64 {
        rows.iter().flat_map(|row| row.values()).flatten().sum()
    }

    pub fn many(a: i64, b: i64, c: i64, d: Option<i64>) -> i64 {
        a * 1000 + b * 100 + c * 10 + d.unwrap_or(0)
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
        m: i64,
        c: i64,
        r#type: i64,
    ) -> i64 {
        py + slf + args + kwargs + m + c + r#type
    }

    /// A leading underscore must not trigger `non_snake_case` in generated code.
    pub fn _under() -> i64 {
        1
    }

    #[allow(dead_code)]
    fn helper() {}

    pub(crate) fn crate_only() -> i64 {
        7
    }

    /// An error type converted into a Python exception by its `From` impl.
    #[derive(Debug)]
    pub(crate) struct MyError(String);

    impl From<MyError> for Error {
        fn from(e: MyError) -> Error {
            Error::new("LookupError", format!("my error: {}", e.0))
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
        /// A class attribute that is an instance.
        pub const UNIT: Point = Point { x: 1.0, y: 0.0, cache: None };

        /// Creates a point from its coordinates.
        pub fn new(x: f64, y: f64) -> Self {
            Point { x, y, cache: None }
        }

        /// Euclidean norm.
        pub fn norm(&self) -> f64 {
            (self.x * self.x + self.y * self.y).sqrt()
        }

        pub fn scale(&mut self, k: f64) {
            self.x *= k;
            self.y *= k;
            self.cache = None;
        }

        pub fn checked_scale(&mut self, k: f64) -> Result<()> {
            if k == 0.0 {
                return Err(Error::value_error("k must not be zero"));
            }
            self.scale(k);
            Ok(())
        }

        pub fn origin() -> Self {
            Point::new(0.0, 0.0)
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
        pub fn absorb(&mut self, other: &Self) {
            self.x += other.x;
            self.y += other.y;
        }

        /// By value: a copy of the instance.
        pub fn swapped(self) -> Point {
            Point::new(self.y, self.x)
        }

        pub fn cached_norm(&mut self) -> f64 {
            *self
                .cache
                .get_or_insert_with(|| (self.x * self.x + self.y * self.y).sqrt())
        }

        pub fn nearest(&self, others: Vec<Ref<Point>>) -> Result<Option<Ref<Point>>> {
            let mut best: Option<(f64, Ref<Point>)> = None;
            for p in others {
                let d = self.dist(&*p.borrow()?);
                if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                    best = Some((d, p));
                }
            }
            Ok(best.map(|(_, p)| p))
        }

        #[allow(dead_code)]
        fn internal(&self) {}
    }

    impl std::fmt::Display for Point {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "({}, {})", self.x, self.y)
        }
    }

    /// By value: a copy.
    pub fn mirrored(mut p: Point) -> Point {
        p.x = -p.x;
        p
    }

    pub fn x_or_zero(p: Option<&Point>) -> f64 {
        p.map_or(0.0, |p| p.x)
    }

    /// `Ref<T>` in and out: the same instance.
    pub fn same(p: Ref<Point>) -> Ref<Point> {
        p
    }

    pub fn maybe_point(give: bool) -> Option<Point> {
        give.then(Point::origin)
    }

    pub fn parse_point(text: &str) -> Result<Point> {
        let (x, y) = text
            .split_once(',')
            .ok_or_else(|| Error::value_error(format!("not a point: {text:?}")))?;
        let num = |s: &str| s.trim().parse::<f64>().map_err(|e| Error::value_error(e.to_string()));
        Ok(Point::new(num(x)?, num(y)?))
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

    /// No constructor: instances only come from Rust.
    pub struct Token {
        pub id: i64,
    }

    pub fn token(id: i64) -> Token {
        Token { id }
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

    /// Methods in `#[cfg]`-gated impl blocks.
    pub struct Node {
        pub id: i64,
    }

    impl Node {
        pub fn new(id: i64) -> Self {
            Node { id }
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

    /// Returns its argument.
    pub fn identity(x: Object) -> Object {
        x
    }

    /// An anonymous lifetime is fine in a parameter.
    pub fn cow_len(s: std::borrow::Cow<'_, str>) -> usize {
        s.len()
    }

    pub fn identity2(x: Object) -> Object {
        x
    }

    pub fn objects(xs: Vec<Object>) -> Vec<Object> {
        xs
    }

    /// Simple value fields, and a value container kept private.
    #[derive(Default)]
    pub struct Bag {
        pub label: String,
        pub path: PathBuf,
        pub letter: char,
        pub flag: bool,
        pub ratio: f32,
        pub small: u8,
        pub maybe: Option<i64>,
        pub note: Option<String>,
        items: Vec<i64>,
    }

    impl Bag {
        pub fn new() -> Self {
            Bag::default()
        }

        pub fn items(&self) -> Vec<i64> {
            self.items.clone()
        }

        pub fn push(&mut self, v: i64) {
            self.items.push(v);
        }
    }

    /// Fields of orochi reference types are the Python objects themselves.
    pub struct Shelf {
        pub xs: List<i64>,
        pub map: Dict<String, i64>,
        pub members: Set<i64>,
        pub any: Object,
        pub maybe_list: Option<List<i64>>,
    }

    impl Shelf {
        pub fn new(xs: List<i64>) -> Self {
            Shelf {
                xs,
                map: Dict::new(),
                members: Set::new(),
                any: Object::none(),
                maybe_list: None,
            }
        }

        pub fn sizes(&self) -> (usize, usize, usize) {
            (self.xs.len(), self.map.len(), self.members.len())
        }
    }

    /// Member names that shadow builtins.
    #[derive(Default)]
    pub struct Shadow {
        pub str: String,
        pub typing: Option<Object>,
    }

    impl Shadow {
        pub fn new() -> Self {
            Shadow::default()
        }

        pub fn int(&self) -> i64 {
            0
        }

        pub fn list(&self, items: Vec<String>) -> Vec<String> {
            items
        }
    }

    /// A class of a submodule.
    pub fn unit_square() -> geometry::Square {
        geometry::Square::new(1.0)
    }

    pub mod geometry {
        #![doc = concat!("Geometry ", "helpers.")]
        use orochi::prelude::*;

        /// Not the root module's class `Point`.
        type Point = (f64, f64);

        /// Instances of the parent module's class.
        pub fn centroid(points: Vec<Ref<super::Point>>) -> Result<super::Point> {
            let n = points.len().max(1) as f64;
            let (mut x, mut y) = (0.0, 0.0);
            for p in &points {
                let p = p.borrow()?;
                x += p.x;
                y += p.y;
            }
            Ok(super::Point::new(x / n, y / n))
        }

        pub mod solids {
            //! Nested submodule.
            use orochi::prelude::*;

            /// Same name as `geometry.Square`.
            pub struct Square {
                pub side: f64,
            }

            pub struct Cube {
                pub base: Ref<super::Square>,
            }

            impl Cube {
                pub fn new(base: Ref<super::Square>) -> Self {
                    Cube { base }
                }

                pub fn face(&self) -> Result<Square> {
                    Ok(Square {
                        side: self.base.borrow()?.side,
                    })
                }

                pub fn corner(&self, at: Option<&super::super::Point>) -> super::super::Point {
                    at.cloned().unwrap_or_else(super::super::Point::origin)
                }
            }
        }

        pub struct Segment {
            a: Point,
            b: Point,
        }

        impl Segment {
            pub fn new(a: Point, b: Point) -> Self {
                Segment { a, b }
            }

            pub fn ends(&self) -> (Point, Point) {
                (self.a, self.b)
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

    /// Items named like bindings the generated code could use.
    #[allow(non_upper_case_globals, non_camel_case_types)]
    pub mod hygiene {
        pub static m: i64 = 1;
        pub const c: i64 = 2;

        pub struct P {
            pub x: i64,
        }

        impl P {
            pub fn new(x: i64) -> Self {
                P { x }
            }

            pub fn get(&self) -> i64 {
                self.x + m + c
            }
        }

        pub mod inner {
            pub struct m;

            pub struct c {
                pub v: i64,
            }

            pub fn make_m() -> m {
                m
            }

            pub fn make_c() -> c {
                c { v: 1 }
            }
        }
    }

    #[allow(dead_code)]
    mod private_mod {
        pub fn hidden() {}
    }
}
