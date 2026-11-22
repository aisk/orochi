//! Integration tests for orochi's runtime layer.
//!
//! All tests share one embedded interpreter (one test binary keeps link time
//! and disk usage down).

mod conversions;
mod errors;
mod evaluation;
mod handles;
mod object;
