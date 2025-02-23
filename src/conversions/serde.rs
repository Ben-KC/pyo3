#![cfg_attr(docsrs, doc(cfg(feature = "serde")))]
#![cfg(feature = "serde")]

//! Enables (de)serialization of [`Py`]`<T>` objects via [serde](https://docs.rs/serde).
//!
//! # Setup
//!
//! To use this feature, add this to your **`Cargo.toml`**:
//!
//! ```toml
//! [dependencies]
//! serde = "1.0"
// workaround for `extended_key_value_attributes`: https://github.com/rust-lang/rust/issues/82768#issuecomment-803935643
#![cfg_attr(docsrs, cfg_attr(docsrs, doc = concat!("pyo3 = { version = \"", env!("CARGO_PKG_VERSION"),  "\", features = [\"serde\"] }")))]
#![cfg_attr(
    not(docsrs),
    doc = "pyo3 = { version = \"*\", features = [\"serde\"] }"
)]
//! ```

use crate::{Py, PyAny, PyClass, Python};
use serde::{de, ser, Deserialize, Deserializer, Serialize, Serializer};

impl<T> Serialize for Py<T>
where
    T: Serialize + PyClass,
{
    fn serialize<S>(&self, serializer: S) -> Result<<S as Serializer>::Ok, <S as Serializer>::Error>
    where
        S: Serializer,
    {
        Python::with_gil(|py| {
            self.try_borrow(py)
                .map_err(|e| ser::Error::custom(e.to_string()))?
                .serialize(serializer)
        })
    }
}

impl<'de, T> Deserialize<'de> for Py<T>
where
    T: PyClass<BaseType = PyAny> + Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Py<T>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let deserialized = T::deserialize(deserializer)?;

        Python::with_gil(|py| {
            Py::new(py, deserialized).map_err(|e| de::Error::custom(e.to_string()))
        })
    }
}
