use crate::error::MigrationError;
use djangors_orm::{FieldKind, FieldMeta};

/// Maps an ORM [`FieldMeta`] definition to its corresponding PostgreSQL SQL type string.
pub fn field_meta_to_sql_type(field: &FieldMeta) -> Result<String, MigrationError> {
    match &field.kind {
        FieldKind::Char | FieldKind::Email | FieldKind::Url | FieldKind::Slug => {
            if let Some(n) = field.max_length {
                Ok(format!("VARCHAR({})", n))
            } else {
                Err(MigrationError::MissingMaxLength {
                    field: field.name.to_string(),
                })
            }
        }
        FieldKind::Text => Ok("TEXT".to_string()),
        FieldKind::Integer => {
            if field.auto {
                Ok("SERIAL".to_string())
            } else {
                Ok("INTEGER".to_string())
            }
        }
        FieldKind::BigInt => {
            if field.auto {
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
