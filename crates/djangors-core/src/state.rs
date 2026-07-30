use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// A type-erased, per-app container for shared state (e.g. a database
/// connection pool) that handlers can retrieve via [`Request::state`](crate::request::Request::state).
/// Cheaply cloneable — cloning an `AppState` clones the underlying `Arc`s,
/// not the contained values.
#[derive(Clone, Default, Debug)]
pub struct AppState {
    inner: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl AppState {
    /// Create a new, empty `AppState`.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(HashMap::new()),
        }
    }

    /// Insert a value of type `T`, returning a new `AppState` (builder-style
    /// — since the inner map is behind an `Arc`, inserting requires building
    /// a new map; this is fine, state is configured once at app startup,
    /// not per-request).
    pub fn insert<T: Send + Sync + 'static>(self, value: T) -> Self {
        let mut map = (*self.inner).clone();
        map.insert(TypeId::of::<T>(), Arc::new(value));
        Self {
            inner: Arc::new(map),
        }
    }

    /// Retrieve a previously-inserted value of type `T`, if any.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.inner
            .get(&TypeId::of::<T>())
            .and_then(|val| (**val).downcast_ref::<T>())
    }

    /// Whether a value of type `T` has been inserted.
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.inner.contains_key(&TypeId::of::<T>())
    }

    /// Number of distinct types held.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether no state has been attached.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Fold `other` into this state, keeping `self`'s value whenever both hold
    /// the same type.
    ///
    /// Used when mounting a sub-router: the parent's configuration wins, and
    /// anything the parent has not set is inherited from the child rather than
    /// being silently lost.
    pub fn merge(self, other: &AppState) -> Self {
        if other.is_empty() {
            return self;
        }
        let mut map = (*self.inner).clone();
        for (type_id, value) in other.inner.iter() {
            map.entry(*type_id).or_insert_with(|| value.clone());
        }
        Self {
            inner: Arc::new(map),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_roundtrip() {
        let state = AppState::new().insert(42i32).insert("hello".to_string());

        assert_eq!(state.get::<i32>(), Some(&42i32));
        assert_eq!(state.get::<String>(), Some(&"hello".to_string()));
        assert_eq!(state.get::<bool>(), None);
    }
}
