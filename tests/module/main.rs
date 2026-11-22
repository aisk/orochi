//! Integration tests for `#[orochi::module]`: a module is defined with the
//! macro, built in-process with `__orochi_module()`, and driven from Python
//! code through the embedded interpreter.

mod classes;
mod functions;
mod mylib_def;
mod semantics;
mod views;

use orochi::Scope;

pub use mylib_def::mylib;

/// A scope where `mylib` is imported; `run` executes Python test code in it.
pub fn scope() -> Scope {
    mylib::__orochi_module().expect("module builds");
    orochi::exec("import mylib").expect("import mylib")
}

/// Runs Python statements (typically `assert`s) in a fresh scope.
#[track_caller]
pub fn run(code: &str) {
    let scope = scope();
    if let Err(err) = scope.exec(code) {
        panic!("{err:#}");
    }
}
