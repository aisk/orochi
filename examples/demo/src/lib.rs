//! A small extension module built with orochi. `run.ps1` builds it, copies
//! the DLL to `target/pyd/demo.pyd` and runs `check.py` against it with a
//! real Python interpreter.

#[orochi::module]
mod demo {
    #![doc = concat!("Demo extension module ", "built with orochi.")]
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

    /// Appends `v` twice, in place.
    pub fn push_twice(xs: &mut Vec<i64>, v: i64) {
        xs.push(v);
        xs.push(v);
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
        pub fn new(x: f64, y: f64) -> Self {
            Point { x, y }
        }

        pub fn norm(&self) -> f64 {
            self.x.hypot(self.y)
        }

        pub fn scale(&mut self, k: f64) -> &mut Self {
            self.x *= k;
            self.y *= k;
            self
        }
    }

    impl std::fmt::Display for Point {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "({}, {})", self.x, self.y)
        }
    }

    /// Container fields are live views from Python.
    #[derive(Default)]
    pub struct Basket {
        pub items: Vec<String>,
        pub table: HashMap<String, Vec<i64>>,
    }

    impl Basket {
        pub fn new() -> Self {
            Basket::default()
        }

        pub fn count(&self) -> usize {
            self.items.len() + self.table.values().map(Vec::len).sum::<usize>()
        }
    }

    pub enum Color {
        Red,
        Green,
    }

    pub fn favorite() -> Color {
        Color::Red
    }

    pub mod geometry {
        /// Area of a circle of radius `r`.
        pub fn area(r: f64) -> f64 {
            std::f64::consts::PI * r * r
        }
    }
}
