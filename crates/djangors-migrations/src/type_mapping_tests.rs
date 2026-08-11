use super::*;

#[test]
fn file_field_maps_to_the_same_sql_type_as_text() {
    let file_field_sql = sql_type_for(
        &FieldKind::FileField,
        None,
        false,
        "attachment",
        Dialect::Postgres,
    )
    .unwrap();
    let text_sql = sql_type_for(&FieldKind::Text, None, false, "body", Dialect::Postgres).unwrap();
    assert_eq!(file_field_sql, text_sql);
    assert_eq!(file_field_sql, "TEXT");
}

#[test]
fn sqlite_auto_primary_key_yields_integer_primary_key_autoincrement() {
    let sql_int = sql_type_for(&FieldKind::Integer, None, true, "id", Dialect::Sqlite).unwrap();
    assert_eq!(sql_int, "INTEGER PRIMARY KEY AUTOINCREMENT");

    let sql_bigint = sql_type_for(&FieldKind::BigInt, None, true, "id", Dialect::Sqlite).unwrap();
    assert_eq!(sql_bigint, "INTEGER PRIMARY KEY AUTOINCREMENT");
}
