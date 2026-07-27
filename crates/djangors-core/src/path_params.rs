use std::collections::HashMap;
use std::str::FromStr;

use crate::error::DjangorsError;

/// A map of named path parameters extracted from a matched route.
///
/// Parameters are stored as strings and can be retrieved with `.get()` for
/// string access or `.get_as::<T>()` for typed access.
#[derive(Debug, Clone)]
pub struct PathParams {
    params: HashMap<String, String>,
}

impl Default for PathParams {
    fn default() -> Self {
        Self::new()
    }
}

impl PathParams {
    /// Creates a new empty `PathParams` collection.
    pub fn new() -> Self {
        PathParams {
            params: HashMap::new(),
        }
    }

    /// Insert a path parameter value.
    pub fn insert(&mut self, key: &str, value: &str) {
        self.params.insert(key.to_string(), value.to_string());
    }

    /// Get a path parameter as a string slice.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    /// Get a path parameter parsed as the requested type `T`.
    ///
    /// Returns `DjangorsError::BadRequest` if the parameter is missing or
    /// cannot be parsed as `T`.
    pub fn get_as<T: FromStr>(&self, key: &str) -> Result<T, DjangorsError> {
        let val = self
            .params
            .get(key)
            .ok_or_else(|| DjangorsError::BadRequest(format!("missing path parameter: {key}")))?;
        val.parse::<T>().map_err(|_| {
            DjangorsError::BadRequest(format!("invalid value for path parameter '{key}': {val}"))
        })
    }
}
