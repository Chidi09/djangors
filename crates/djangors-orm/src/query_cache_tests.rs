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
