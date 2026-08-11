//! Process-local query result cache.
//!
//! This module implements an opt-in, in-memory cache for [`QuerySet`] results,
//! keyed by (model table, compiled SQL, bind parameters). It is intentionally
//! self-contained (no dependency on `djangors-cache`) so the ORM can offer
//! queryset caching without a dependency cycle.
//!
//! Entries expire after a configured TTL. Writes (save/update/delete) invalidate
//! every cached entry belonging to the affected model's table via
//! [`invalidate_table`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Entry {
    value: Box<dyn std::any::Any + Send + Sync>,
    expires_at: Option<Instant>,
}

struct CacheStore {
    entries: HashMap<String, Entry>,
    max_entries: usize,
}

static STORE: std::sync::LazyLock<Mutex<CacheStore>> = std::sync::LazyLock::new(|| {
    Mutex::new(CacheStore {
        entries: HashMap::new(),
        max_entries: 10_000,
    })
});

fn now() -> Instant {
    Instant::now()
}

/// Sets the maximum number of cached queryset entries across all models.
pub fn set_max_cache_entries(n: usize) {
    if let Ok(mut store) = STORE.lock() {
        store.max_entries = n;
        if store.entries.len() > n {
            evict(&mut store, n);
        }
    }
}

fn evict(store: &mut CacheStore, target: usize) {
    while store.entries.len() > target {
        if let Some(key) = store.entries.keys().next().map(|k| k.to_string()) {
            store.entries.remove(&key);
        } else {
            break;
        }
    }
}

fn find_key(table: &str, sql: &str, params: &[crate::expr::Value]) -> String {
    let param_str = params
        .iter()
        .map(|v| format!("{v:?}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{table}\u{0}{sql}\u{0}{param_str}")
}

/// Invalidates all cached results belonging to `table_name`.
///
/// Called automatically by model writes (save/update/delete) so stale rows are
/// never served after a mutation.
pub fn invalidate_table(table_name: &str) {
    if let Ok(mut store) = STORE.lock() {
        store
            .entries
            .retain(|key, _| !key.starts_with(&format!("{table_name}\u{0}")));
    }
}

/// Returns a cloned cached `Vec<T>` for the given query if present and valid.
pub(crate) fn get_cached<T>(table: &str, sql: &str, params: &[crate::expr::Value]) -> Option<Vec<T>>
where
    T: Clone + Send + Sync + 'static,
{
    let key = find_key(table, sql, params);
    let store = STORE.lock().ok()?;
    let entry = store.entries.get(&key)?;
    if entry.expires_at.is_none_or(|e| e > now()) && entry.value.downcast_ref::<Vec<T>>().is_some()
    {
        Some(entry.value.downcast_ref::<Vec<T>>()?.clone())
    } else {
        None
    }
}

/// Stores `value` under the given query for `ttl`.
pub(crate) fn put_cached<T>(
    table: &str,
    sql: &str,
    params: &[crate::expr::Value],
    value: Vec<T>,
    ttl: Option<Duration>,
) where
    T: Clone + Send + Sync + 'static,
{
    let key = find_key(table, sql, params);
    let entry = Entry {
        value: Box::new(value),
        expires_at: ttl.map(|t| now() + t),
    };
    if let Ok(mut store) = STORE.lock() {
        if !store.entries.contains_key(&key) && store.entries.len() >= store.max_entries {
            let target = store.max_entries.saturating_sub(1);
            evict(&mut store, target);
        }
        store.entries.insert(key, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip_and_invalidate() {
        let params = vec![crate::expr::Value::I64(1)];
        assert!(get_cached::<i64>("t", "SELECT 1", &params).is_none());

        put_cached("t", "SELECT 1", &params, vec![1i64, 2, 3], None);
        assert_eq!(
            get_cached::<i64>("t", "SELECT 1", &params),
            Some(vec![1, 2, 3])
        );

        // Different params -> separate key, no hit.
        let params2 = vec![crate::expr::Value::I64(2)];
        assert!(get_cached::<i64>("t", "SELECT 1", &params2).is_none());

        // Invalidation clears entries for the table.
        invalidate_table("t");
        assert!(get_cached::<i64>("t", "SELECT 1", &params).is_none());
    }

    #[test]
    fn cache_ttl_expiry() {
        let params = vec![];
        put_cached(
            "t",
            "SELECT 1",
            &params,
            vec![1i64],
            Some(Duration::from_millis(30)),
        );
        assert!(get_cached::<i64>("t", "SELECT 1", &params).is_some());
        std::thread::sleep(Duration::from_millis(60));
        assert!(get_cached::<i64>("t", "SELECT 1", &params).is_none());
    }
}
