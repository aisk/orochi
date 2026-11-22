//! The single place where orochi attaches to the Python interpreter.

use std::panic::{AssertUnwindSafe, catch_unwind};

use pyo3::Python;

/// Attaches the current thread to the interpreter and runs `f`.
///
/// If no interpreter is running, it is started when the `embed` feature is
/// enabled; otherwise this panics with an explanation. (Inside an extension
/// module the interpreter always runs.)
pub(crate) fn attach<F, R>(f: F) -> R
where
    F: for<'py> FnOnce(Python<'py>) -> R,
{
    if Python::try_attach(|_| ()).is_none() {
        ensure_interpreter();
    }
    // If attaching is still impossible (the interpreter is finalizing, or we
    // are inside a GC traversal), pyo3 panics with a precise message.
    Python::attach(f)
}

/// Like [`attach`], but never panics: returns `None` if the interpreter is not
/// available or if `f` panics (including a Python `PanicException` resumed by
/// pyo3). Used by formatting impls, which must not panic.
pub(crate) fn try_attach<F, R>(f: F) -> Option<R>
where
    F: for<'py> FnOnce(Python<'py>) -> R,
{
    catch_unwind(AssertUnwindSafe(|| Python::try_attach(f)))
        .ok()
        .flatten()
}

const NOT_RUNNING: &str = "orochi: the Python interpreter is not running. Inside a Python \
    extension module it always is; to use orochi from a standalone Rust program or its tests, \
    enable the `embed` feature of orochi.";

#[derive(Debug, PartialEq)]
enum Startup {
    /// Initialized: let `Python::attach` proceed (it reports finalization or
    /// GC-traversal states itself).
    Proceed,
    /// Not initialized, but `embed` allows starting it.
    Initialize,
    /// Not initialized and not allowed to start it.
    Fail(&'static str),
}

fn startup(initialized: bool, embed: bool) -> Startup {
    match (initialized, embed) {
        (true, _) => Startup::Proceed,
        (false, true) => Startup::Initialize,
        (false, false) => Startup::Fail(NOT_RUNNING),
    }
}

#[cold]
fn ensure_interpreter() {
    // SAFETY: `Py_IsInitialized` may be called at any time, even before the
    // interpreter is initialized or after it has been finalized.
    let initialized = unsafe { pyo3::ffi::Py_IsInitialized() } != 0;
    match startup(initialized, cfg!(feature = "embed")) {
        Startup::Proceed => {}
        Startup::Initialize => Python::initialize(),
        Startup::Fail(message) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_decision() {
        assert_eq!(startup(true, false), Startup::Proceed);
        assert_eq!(startup(true, true), Startup::Proceed);
        assert_eq!(startup(false, true), Startup::Initialize);
        match startup(false, false) {
            Startup::Fail(msg) => {
                assert!(msg.contains("enable the `embed` feature of orochi"));
                assert!(!msg.contains("auto-initialize"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
