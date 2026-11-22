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
    let mut f = Some(f);
    if let Some(r) = Python::try_attach(|py| (f.take().expect("not called yet"))(py)) {
        return r;
    }
    ensure_interpreter();
    // If attaching is still impossible (the interpreter is finalizing, or we
    // are inside a GC traversal), pyo3 panics with a precise message.
    Python::attach(f.take().expect("not called by `try_attach`"))
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

/// Starts the interpreter if it is not initialized and `embed` allows it;
/// otherwise panics with an explanation. Only called when attaching failed:
/// `Py_IsInitialized` is already false while the interpreter finalizes, when
/// `__del__` methods may still call into orochi.
#[cold]
fn ensure_interpreter() {
    // SAFETY: `Py_IsInitialized` may be called at any time, even before the
    // interpreter is initialized or after it has been finalized.
    if unsafe { pyo3::ffi::Py_IsInitialized() } == 0 {
        if cfg!(feature = "embed") {
            Python::initialize();
        } else {
            panic!("{NOT_RUNNING}");
        }
    }
}
