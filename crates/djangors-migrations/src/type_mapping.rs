use crate::error::MigrationError;
use djangors_orm::{FieldKind, FieldMeta};

/// Maps an ORM [`FieldMeta`] definition to its corresponding PostgreSQL SQL type string.
pub fn field_meta_to_sql_type(field: &FieldMeta) -> Result<String, MigrationError> {
    sql_type_for(&field.kind, field.max_length, field.auto, field.name)
}

/// Maps a field's kind/max_length/auto to its corresponding PostgreSQL SQL type string.
///
/// This is the single source of truth for ORM-field-kind-to-SQL-type mapping, shared by both
/// full-table creation ([`field_meta_to_sql_type`]) and incremental `ALTER TABLE ADD COLUMN`
/// generation (`dj makemigrations`), so every [`FieldKind`] variant is mapped correctly in both
/// paths rather than one of them silently falling back to a wrong default.
pub fn sql_type_for(
    kind: &FieldKind,
    max_length: Option<u32>,
    auto: bool,
    field_name: &str,
) -> Result<String, MigrationError> {
    match kind {
        FieldKind::Char | FieldKind::Email | FieldKind::Url | FieldKind::Slug => {
            if let Some(n) = max_length {
                Ok(format!("VARCHAR({})", n))
            } else {
                Err(MigrationError::MissingMaxLength {
                    field: field_name.to_string(),
                })
            }
        }
        FieldKind::Text | FieldKind::FileField => Ok("TEXT".to_string()),
        FieldKind::Integer => {
            if auto {
                Ok("SERIAL".to_string())
            } else {
                Ok("INTEGER".to_string())
            }
        }
        FieldKind::BigInt => {
            if auto {
                Ok("BIGSERIAL".to_string())
            } else {
                Ok("BIGINT".to_string())
            }
        }
        FieldKind::Float => Ok("DOUBLE PRECISION".to_string()),
        FieldKind::Decimal {
            max_digits,
            decimal_places,
        } => Ok(format!("NUMERIC({}, {})", max_digits, decimal_places)),
        FieldKind::Boolean => Ok("BOOLEAN".to_string()),
        FieldKind::Date => Ok("DATE".to_string()),
        FieldKind::DateTime => Ok("TIMESTAMPTZ".to_string()),
        FieldKind::Time => Ok("TIME".to_string()),
        FieldKind::Duration => Ok("INTERVAL".to_string()),
        FieldKind::Uuid => Ok("UUID".to_string()),
        FieldKind::Ip => Ok("INET".to_string()),
        FieldKind::Binary => Ok("BYTEA".to_string()),
        FieldKind::Json => Ok("JSONB".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_field_maps_to_the_same_sql_type_as_text() {
        let file_field_sql =
            sql_type_for(&FieldKind::FileField, None, false, "attachment").unwrap();
        let text_sql = sql_type_for(&FieldKind::Text, None, false, "body").unwrap();
        assert_eq!(file_field_sql, text_sql);
        assert_eq!(file_field_sql, "TEXT");
    }
}
