#![cfg_attr(feature = "nightly", feature(specialization))]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    docsrs, // rustdoc:: is not supported on msrv
    deny(
        invalid_doc_attributes,
        rustdoc::broken_intra_doc_links,
        rustdoc::bare_urls
    )
)]

//! Rust bindings to the Python interpreter.
//!
//! PyO3 can be used to write native Python modules or run Python code and modules from Rust.
//!
//! See [the guide] for a detailed introduction.
//!
//! # PyO3's object types
//!
//! PyO3 has several core types that you should familiarize yourself with:
//!
//! ## The Python<'py> object
//!
//! Holding the [global interpreter lock] (GIL) is modeled with the [`Python<'py>`](crate::Python)
//! token. All APIs that require that the GIL is held require this token as proof that you really
//! are holding the GIL. It can be explicitly acquired and is also implicitly acquired by PyO3 as
//! it wraps Rust functions and structs into Python functions and objects.
//!
//! ## The GIL-dependent types
//!
//! For example `&`[`PyAny`]. These are only ever seen as references, with a lifetime that is only
//! valid for as long as the GIL is held, which is why using them doesn't require a
//! [`Python<'py>`](crate::Python) token. The underlying Python object, if mutable, can be mutated
//! through any reference.
//!
//! See the [guide][types] for an explanation of the different Python object types.
//!
//! ## The GIL-independent types
//!
//! When wrapped in [`Py`]`<...>`, like with [`Py`]`<`[`PyAny`]`>` or [`Py`]`<SomePyClass>`, Python
//! objects no longer have a limited lifetime which makes them easier to store in structs and pass
//! between functions. However, you cannot do much with them without a
//! [`Python<'py>`](crate::Python) token, for which you’d need to reacquire the GIL.
//!
//! ## PyErr
//!
//! The vast majority of operations in this library will return [`PyResult<...>`](PyResult).
//! This is an alias for the type `Result<..., PyErr>`.
//!
//! A `PyErr` represents a Python exception. A `PyErr` returned to Python code will be raised as a
//! Python exception. Errors from `PyO3` itself are also exposed as Python exceptions.
//!
//! # Feature flags
//!
//! PyO3 uses [feature flags] to enable you to opt-in to additional functionality.For a detailed
//! description, see the [Features chapter of the guide].
//!
//! ## Default feature flags
//!
//! The following features are turned on by default:
//! - `macros`: Enables various macros, including all the attribute macros.
//!
//! ## Optional feature flags
//!
//! The following features customize PyO3's behavior:
//!
//! - `abi3`: Restricts PyO3's API to a subset of the full Python API which is guaranteed by
//! [PEP 384] to be forward-compatible with future Python versions.
//! - `auto-initialize`: Changes [`Python::with_gil`] and [`Python::acquire_gil`] to automatically
//! initialize the Python interpreter if needed.
//! - `extension-module`: This will tell the linker to keep the Python symbols unresolved, so that
//! your module can also be used with statically linked Python interpreters. Use this feature when
//! building an extension module.
//! - `multiple-pymethods`: Enables the use of multiple [`#[pymethods]`](macro@crate::pymethods)
//! blocks per [`#[pyclass]`](macro@crate::pyclass). This adds a dependency on the [inventory]
//! crate, which is not supported on all platforms.
//!
//! The following features enable interactions with other crates in the Rust ecosystem:
//
//! - [`hashbrown`]: Enables conversions between Python objects and [hashbrown]'s [`HashMap`] and
//! [`HashSet`] types.
//! - [`indexmap`]: Enables conversions between Python dictionary and [indexmap]'s [`IndexMap`].
//! - [`num-bigint`]: Enables conversions between Python objects and [num-bigint]'s [`BigInt`] and
//! [`BigUint`] types.
//! - [`num-complex`]: Enables conversions between Python objects and [num-complex]'s [`Complex`]
//!  type.
//! - [`serde`]: Allows implementing [serde]'s [`Serialize`] and [`Deserialize`] traits for
//! [`Py`]`<T>` for all `T` that implement [`Serialize`] and [`Deserialize`].
//!
//! ## Unstable features
//!
//! - `nightly`: Gates some optimizations that rely on
//! [`#![feature(specialization)]`](https://github.com/rust-lang/rfcs/blob/master/text/1210-impl-specialization.md),
//! for which you'd also need nightly Rust. You should not use this feature.
//
//! ## `rustc` environment flags
//!
//! PyO3 uses `rustc`'s `--cfg` flags to enable or disable code used for different Python versions.
//! If you want to do this for your own crate, you can do so with the [`pyo3-build-config`] crate.
//!
//! - `Py_3_6`, `Py_3_7`, `Py_3_8`, `Py_3_9`, `Py_3_10`: Marks code that is only enabled when
//!  compiling for a given minimum Python version.
//! - `Py_LIMITED_API`: Marks code enabled when the `abi3` feature flag is enabled.
//! - `PyPy` - Marks code enabled when compiling for PyPy.
//!
//! # Minimum supported Rust and Python versions
//!
//! PyO3 supports the following software versions:
//!   - Python 3.6 and up (CPython and PyPy)
//!   - Rust 1.41 and up
//!
//! # Example: Building a native Python module
//!
//! PyO3 can be used to generate a native Python module. The easiest way to try this out for the
//! first time is to use [`maturin`]. `maturin` is a tool for building and publishing Rust-based
//! Python packages with minimal configuration. The following steps set up some files for an example
//! Python module, install `maturin`, and then show how build and import the Python module.
//!
//! First, create a new folder (let's call it `string_sum`) containing the following two files:
//!
//! **`Cargo.toml`**
//!
//! ```toml
//! [package]
//! name = "string-sum"
//! version = "0.1.0"
//! edition = "2018"
//!
//! [lib]
//! name = "string_sum"
//! # "cdylib" is necessary to produce a shared library for Python to import from.
//! #
//! # Downstream Rust code (including code in `bin/`, `examples/`, and `tests/`) will not be able
//! # to `use string_sum;` unless the "rlib" or "lib" crate type is also included, e.g.:
//! # crate-type = ["cdylib", "rlib"]
//! crate-type = ["cdylib"]
//!
//! [dependencies.pyo3]
// workaround for `extended_key_value_attributes`: https://github.com/rust-lang/rust/issues/82768#issuecomment-803935643
#![cfg_attr(docsrs, cfg_attr(docsrs, doc = concat!("version = \"", env!("CARGO_PKG_VERSION"),  "\"")))]
#![cfg_attr(not(docsrs), doc = "version = \"*\"")]
//! features = ["extension-module"]
//! ```
//!
//! **`src/lib.rs`**
//!
//! ```rust
//! use pyo3::prelude::*;
//!
//! /// Formats the sum of two numbers as string.
//! #[pyfunction]
//! fn sum_as_string(a: usize, b: usize) -> PyResult<String> {
//!     Ok((a + b).to_string())
//! }
//!
//! /// A Python module implemented in Rust.
//! #[pymodule]
//! fn string_sum(py: Python, m: &PyModule) -> PyResult<()> {
//!     m.add_function(wrap_pyfunction!(sum_as_string, m)?)?;
//!
//!     Ok(())
//! }
//! ```
//!
//! With those two files in place, now `maturin` needs to be installed. This can be done using
//! Python's package manager `pip`. First, load up a new Python `virtualenv`, and install `maturin`
//! into it:
//!
//! ```bash
//! $ cd string_sum
//! $ python -m venv .env
//! $ source .env/bin/activate
//! $ pip install maturin
//! ```
//!
//! Now build and execute the module:
//!
//! ```bash
//! $ maturin develop
//! # lots of progress output as maturin runs the compilation...
//! $ python
//! >>> import string_sum
//! >>> string_sum.sum_as_string(5, 20)
//! '25'
//! ```
//!
//! As well as with `maturin`, it is possible to build using [setuptools-rust] or
//! [manually][manual_builds]. Both offer more flexibility than `maturin` but require further
//! configuration.
//!
//! # Example: Using Python from Rust
//!
//! To embed Python into a Rust binary, you need to ensure that your Python installation contains a
//! shared library. The following steps demonstrate how to ensure this (for Ubuntu), and then give
//! some example code which runs an embedded Python interpreter.
//!
//! To install the Python shared library on Ubuntu:
//!
//! ```bash
//! sudo apt install python3-dev
//! ```
//!
//! Start a new project with `cargo new` and add  `pyo3` to the `Cargo.toml` like this:
//!
//! ```toml
//! [dependencies.pyo3]
// workaround for `extended_key_value_attributes`: https://github.com/rust-lang/rust/issues/82768#issuecomment-803935643
#![cfg_attr(docsrs, cfg_attr(docsrs, doc = concat!("version = \"", env!("CARGO_PKG_VERSION"),  "\"")))]
#![cfg_attr(not(docsrs), doc = "version = \"*\"")]
//! # this is necessary to automatically initialize the Python interpreter
//! features = ["auto-initialize"]
//! ```
//!
//! Example program displaying the value of `sys.version` and the current user name:
//!
//! ```rust
//! use pyo3::prelude::*;
//! use pyo3::types::IntoPyDict;
//!
//! fn main() -> PyResult<()> {
//!     Python::with_gil(|py| {
//!         let sys = py.import("sys")?;
//!         let version: String = sys.get("version")?.extract()?;
//!
//!         let locals = [("os", py.import("os")?)].into_py_dict(py);
//!         let code = "os.getenv('USER') or os.getenv('USERNAME') or 'Unknown'";
//!         let user: String = py.eval(code, None, Some(&locals))?.extract()?;
//!
//!         println!("Hello {}, I'm Python {}", user, version);
//!         Ok(())
//!     })
//! }
//! ```
//!
//! The guide has [a section][calling_rust] with lots of examples about this topic.
//!
//! # Other Examples
//!
//! The PyO3 [README](https://github.com/PyO3/pyo3#readme) contains quick-start examples for both
//! using [Rust from Python] and [Python from Rust].
//!
//! The PyO3 repository's [examples subdirectory]
//! contains some basic packages to demonstrate usage of PyO3.
//!
//! There are many projects using PyO3 - see a list of some at
//! <https://github.com/PyO3/pyo3#examples>.
//!
//! [inventory]: https://docs.rs/inventory
//! [`HashMap`]: https://docs.rs/hashbrown/latest/hashbrown/struct.HashMap.html
//! [`HashSet`]: https://docs.rs/hashbrown/latest/hashbrown/struct.HashSet.html
//! [`IndexMap`]: https://docs.rs/indexmap/latest/indexmap/map/struct.IndexMap.html
//! [`BigInt`]: https://docs.rs/num-bigint/latest/num_bigint/struct.BigInt.html
//! [`BigUint`]: https://docs.rs/num-bigint/latest/num_bigint/struct.BigUint.html
//! [`Complex`]: https://docs.rs/num-complex/latest/num_complex/struct.Complex.html
//! [`Deserialize`]: https://docs.rs/serde/latest/serde/trait.Deserialize.html
//! [`Serialize`]: https://docs.rs/serde/latest/serde/trait.Serialize.html
//! [`hashbrown`]: ./hashbrown/index.html
//! [`indexmap`]: <./indexmap/index.html>
//! [`maturin`]: https://github.com/PyO3/maturin "Build and publish crates with pyo3, rust-cpython and cffi bindings as well as rust binaries as python packages"
//! [`num-bigint`]: ./num_bigint/index.html
//! [`num-complex`]: ./num_complex/index.html
//! [`pyo3-build-config`]: https://docs.rs/pyo3-build-config
//! [`serde`]: <./serde/index.html>
//! [calling_rust]: https://pyo3.rs/latest/python_from_rust.html "Calling Python from Rust - PyO3 user guide"
//! [examples subdirectory]: https://github.com/PyO3/pyo3/tree/main/examples
//! [feature flags]: https://doc.rust-lang.org/cargo/reference/features.html "Features - The Cargo Book"
//! [global interpreter lock]: https://docs.python.org/3/glossary.html#term-global-interpreter-lock
//! [hashbrown]: https://docs.rs/hashbrown
//! [indexmap]: https://docs.rs/indexmap
//! [manual_builds]: https://pyo3.rs/latest/building_and_distribution.html#manual-builds "Manual builds - Building and Distribution - PyO3 user guide"
//! [num-bigint]: https://docs.rs/num-bigint
//! [num-complex]: https://docs.rs/num-complex
//! [serde]: https://docs.rs/serde
//! [setuptools-rust]: https://github.com/PyO3/setuptools-rust "Setuptools plugin for Rust extensions"
//! [the guide]: https://pyo3.rs "PyO3 user guide"
//! [types]: https://pyo3.rs/latest/types.html "GIL lifetimes, mutability and Python object types"
//! [PEP 384]: https://www.python.org/dev/peps/pep-0384 "PEP 384 -- Defining a Stable ABI"
//! [Python from Rust]: https://github.com/PyO3/pyo3#using-python-from-rust
//! [Rust from Python]: https://github.com/PyO3/pyo3#using-rust-from-python
//! [Features chapter of the guide]: https://pyo3.rs/latest/features.html#features-reference "Features Reference - PyO3 user guide"
pub use crate::class::*;
pub use crate::conversion::{
    AsPyPointer, FromPyObject, FromPyPointer, IntoPy, IntoPyPointer, PyTryFrom, PyTryInto,
    ToBorrowedObject, ToPyObject,
};
pub use crate::err::{PyDowncastError, PyErr, PyErrArguments, PyResult};
#[cfg(not(PyPy))]
#[cfg_attr(docsrs, doc(cfg(not(PyPy))))]
pub use crate::gil::{prepare_freethreaded_python, with_embedded_python_interpreter};
pub use crate::gil::{GILGuard, GILPool};
pub use crate::instance::{Py, PyNativeType, PyObject};
pub use crate::pycell::{PyCell, PyRef, PyRefMut};
pub use crate::pyclass::PyClass;
pub use crate::pyclass_init::PyClassInitializer;
pub use crate::python::{Python, PythonVersionInfo};
pub use crate::type_object::PyTypeInfo;
pub use crate::types::PyAny;

#[cfg(feature = "macros")]
#[doc(hidden)]
pub use {
    indoc,    // Re-exported for py_run
    paste,    // Re-exported for wrap_function
    unindent, // Re-exported for py_run
};

#[cfg(all(feature = "macros", feature = "multiple-pymethods"))]
#[doc(hidden)]
pub use inventory; // Re-exported for `#[pyclass]` and `#[pymethods]` with `multiple-pymethods`.

#[macro_use]
mod internal_tricks;

pub mod buffer;
#[doc(hidden)]
pub mod callback;
pub mod class;
pub mod conversion;
mod conversions;
#[macro_use]
#[doc(hidden)]
pub mod derive_utils;
mod err;
pub mod exceptions;
pub mod ffi;
mod gil;
#[doc(hidden)]
pub mod impl_;
mod instance;
pub mod marshal;
pub mod once_cell;
pub mod panic;
pub mod prelude;
pub mod pycell;
pub mod pyclass;
pub mod pyclass_init;
pub mod pyclass_slots;
mod python;
pub mod type_object;
pub mod types;

pub use crate::conversions::*;

#[doc(hidden)]
#[deprecated(
    since = "0.15.0",
    note = "please import this with `use pyo3::...` or from the prelude instead"
)]
#[cfg(feature = "macros")]
pub mod proc_macro {
    pub use pyo3_macros::{pyclass, pyfunction, pymethods, pymodule, pyproto};
}

#[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
#[cfg(feature = "macros")]
pub use pyo3_macros::{pyclass, pyfunction, pymethods, pymodule, pyproto, FromPyObject};

#[macro_use]
mod macros;

/// Test readme and user guide
#[cfg(doctest)]
pub mod doc_test {
    macro_rules! doctest_impl {
        ($doc:expr, $mod:ident) => {
            #[doc = $doc]
            mod $mod {}
        };
    }

    macro_rules! doctest {
        ($path:expr, $mod:ident) => {
            doctest_impl!(include_str!(concat!("../", $path)), $mod);
        };
    }

    doctest!("README.md", readme_md);
    doctest!("guide/src/advanced.md", guide_advanced_md);
    doctest!(
        "guide/src/building_and_distribution.md",
        guide_building_and_distribution_md
    );
    doctest!("guide/src/class.md", guide_class_md);
    doctest!("guide/src/class/protocols.md", guide_class_protocols_md);
    doctest!("guide/src/conversions.md", guide_conversions_md);
    doctest!(
        "guide/src/conversions/tables.md",
        guide_conversions_tables_md
    );
    doctest!(
        "guide/src/conversions/traits.md",
        guide_conversions_traits_md
    );
    doctest!("guide/src/debugging.md", guide_debugging_md);
    doctest!("guide/src/exception.md", guide_exception_md);
    doctest!("guide/src/function.md", guide_function_md);
    doctest!("guide/src/migration.md", guide_migration_md);
    doctest!("guide/src/module.md", guide_module_md);
    doctest!("guide/src/parallelism.md", guide_parallelism_md);
    doctest!("guide/src/python_from_rust.md", guide_python_from_rust_md);
    doctest!("guide/src/rust_cpython.md", guide_rust_cpython_md);
    doctest!("guide/src/trait_bounds.md", guide_trait_bounds_md);
    doctest!("guide/src/types.md", guide_types_md);
    doctest!("guide/src/faq.md", faq);

    // deliberate choice not to test guide/ecosystem because those pages depend on external crates
    // such as pyo3_asyncio.
}
