//! A small extension module built with orochi. `run.py` builds it, copies
//! the library to `target/pyd/` (`demo.pyd` or `demo.so`) and runs
//! `check.py` against it with a real Python interpreter.

#[orochi::module]
mod demo {
    #![doc = concat!("Demo extension module ", "built with orochi.")]
    use orochi::prelude::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Add two integers; `b` defaults to 1.
    pub fn add(a: i64, b: Option<i64>) -> i64 {
        a + b.unwrap_or(1)
    }

    /// Number of lines in `text`.
    pub fn count_lines(text: &str) -> usize {
        text.lines().count()
    }

    pub fn read_text(path: PathBuf) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    /// Appends `v` twice to the caller's list.
    pub fn push_twice(xs: List<i64>, v: i64) -> Result<()> {
        xs.push(v)?;
        xs.push(v)
    }

    pub fn word_counts(text: &str) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for word in text.split_whitespace() {
            *counts.entry(word.to_owned()).or_insert(0) += 1;
        }
        counts
    }

    /// A 2D point.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Point {
        pub x: f64,
        pub y: f64,
    }

    impl Point {
        /// Creates a point from its coordinates.
        pub fn new(x: f64, y: f64) -> Self {
            Point { x, y }
        }

        pub fn norm(&self) -> f64 {
            self.x.hypot(self.y)
        }

        pub fn scale(&mut self, k: f64) {
            self.x *= k;
            self.y *= k;
        }

        pub fn dist(&self, other: &Point) -> f64 {
            (self.x - other.x).hypot(self.y - other.y)
        }
    }

    impl std::fmt::Display for Point {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "({}, {})", self.x, self.y)
        }
    }

    /// A reference field next to a private value field.
    #[derive(Default)]
    pub struct Basket {
        /// A reference: a real Python list, mutable in place.
        pub tags: List<String>,
        /// A simple value: copied in and out.
        pub label: String,
        items: Vec<i64>,
    }

    impl Basket {
        pub fn new() -> Self {
            Basket::default()
        }

        pub fn add_item(&mut self, item: i64) {
            self.items.push(item);
        }

        pub fn items(&self) -> Vec<i64> {
            self.items.clone()
        }

        pub fn count(&self) -> usize {
            self.items.len() + self.tags.len()
        }
    }

    /// No constructor: instances only come from Rust.
    pub struct Ticket {
        pub id: i64,
    }

    pub fn ticket(id: i64) -> Ticket {
        Ticket { id }
    }

    pub mod geometry {
        /// Area of a circle of radius `r`.
        pub fn area(r: f64) -> f64 {
            std::f64::consts::PI * r * r
        }
    }
}
