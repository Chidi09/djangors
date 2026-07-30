use crate::error::MigrationError;
use crate::operation::{ColumnDef, ForeignKeyRef, Operation};
use crate::type_mapping::field_meta_to_sql_type;
use djangors_orm::{all_registered_models, ModelMeta};
use std::collections::{HashMap, HashSet};

/// Builds a sequence of migration DDL operations to create tables for all registered models.
pub fn build_create_all_plan() -> Result<Vec<Operation>, MigrationError> {
    // 1. Get all models
    let models: Vec<&'static ModelMeta> = all_registered_models().collect();

    // Map struct_name -> ModelMeta
    let mut models_by_struct = HashMap::new();
    for &model in &models {
        models_by_struct.insert(model.struct_name, model);
    }

    // 2. Topological sort
    let mut visited = HashSet::new();
    let mut visiting = HashSet::new();
    let mut ordered_models = Vec::new();

    for &model in &models {
        if !visited.contains(model.struct_name) {
            dfs(
                model,
                &models_by_struct,
                &mut visiting,
                &mut visited,
                &mut ordered_models,
            )?;
        }
    }

    // 3. Build plan
    let mut operations = Vec::new();
    for model in ordered_models {
        let mut columns = Vec::new();

        // Standard fields
        for field in model.fields {
            let sql_type = field_meta_to_sql_type(field, djangors_db::Dialect::Postgres)?;
            columns.push(ColumnDef {
                name: field.column_name.to_string(),
                sql_type,
                nullable: field.nullable,
                primary_key: field.primary_key,
                unique: field.unique,
                default_sql: None,
                references: None,
            });
        }

        // Relation fields
        for relation in model.relations {
            let target_meta = (relation.target)();
            let target_pk = target_meta
                .fields
                .iter()
                .find(|f| f.primary_key)
                .ok_or_else(|| {
                    MigrationError::Database(djangors_db::DbError::ConnectionFailed(format!(
                        "Target model {} has no primary key field",
                        target_meta.struct_name
                    )))
                })?;

            let mut fk_field_meta = *target_pk;
            fk_field_meta.auto = false;
            let sql_type = field_meta_to_sql_type(&fk_field_meta, djangors_db::Dialect::Postgres)?;

            let on_delete_str = match relation.on_delete {
                djangors_orm::OnDelete::Cascade => "CASCADE",
                djangors_orm::OnDelete::Protect | djangors_orm::OnDelete::Restrict => "RESTRICT",
                djangors_orm::OnDelete::SetNull => "SET NULL",
                djangors_orm::OnDelete::DoNothing => "NO ACTION",
            };

            let nullable = matches!(relation.on_delete, djangors_orm::OnDelete::SetNull);

            columns.push(ColumnDef {
                name: relation.field_name.to_string(),
                sql_type,
                nullable,
                primary_key: false,
                unique: false,
                default_sql: None,
                references: Some(ForeignKeyRef {
                    table: target_meta.table_name.to_string(),
                    column: target_pk.column_name.to_string(),
                    on_delete: on_delete_str.to_string(),
                }),
            });
        }

        operations.push(Operation::CreateTable {
            table_name: model.table_name.to_string(),
            columns,
        });
    }

    Ok(operations)
}

/// Builds the initial plan from metadata emitted by a project's binary.
pub fn build_create_plan_from_snapshots(
    snapshots: &[djangors_orm::ModelSnapshot],
) -> Result<Vec<Operation>, MigrationError> {
    let mut by_name = HashMap::new();
    for s in snapshots {
        by_name.insert(s.struct_name.clone(), s);
    }
    let mut visited = HashSet::new();
    let mut ordered = Vec::new();

    fn visit<'a>(
        m: &'a djangors_orm::ModelSnapshot,
        by_name: &HashMap<String, &'a djangors_orm::ModelSnapshot>,
        visited: &mut HashSet<String>,
        ordered: &mut Vec<&'a djangors_orm::ModelSnapshot>,
    ) {
        if visited.contains(&m.struct_name) {
            return;
        }
        visited.insert(m.struct_name.clone());
        for r in &m.relations {
            if let Some(target) = by_name.get(r.target_struct.as_str()) {
                visit(target, by_name, visited, ordered);
            }
        }
        ordered.push(m);
    }

    for m in snapshots {
        visit(m, &by_name, &mut visited, &mut ordered);
    }
    ordered
        .into_iter()
        .map(|m| {
            let mut columns = Vec::new();
            for f in &m.fields {
                columns.push(ColumnDef {
                    name: f.column_name.clone(),
                    sql_type: crate::type_mapping::sql_type_for(
                        &f.kind,
                        f.max_length,
                        f.auto,
                        &f.name,
                        djangors_db::Dialect::Postgres,
                    )?,
                    nullable: f.nullable,
                    primary_key: f.primary_key,
                    unique: f.unique,
                    default_sql: None,
                    references: None,
                });
            }
            for r in &m.relations {
                let target = by_name.get(r.target_struct.as_str()).ok_or_else(|| {
                    MigrationError::Database(djangors_db::DbError::ConnectionFailed(format!(
                        "unknown relation target {}",
                        r.target_struct
                    )))
                })?;
                let pk = target
                    .fields
                    .iter()
                    .find(|f| f.primary_key)
                    .ok_or_else(|| {
                        MigrationError::Database(djangors_db::DbError::ConnectionFailed(
                            "relation target has no primary key".into(),
                        ))
                    })?;
                let mut fk = pk.clone();
                fk.auto = false;
                let on_delete = match r.on_delete {
                    djangors_orm::OnDelete::Cascade => "CASCADE",
                    djangors_orm::OnDelete::SetNull => "SET NULL",
                    djangors_orm::OnDelete::DoNothing => "NO ACTION",
                    _ => "RESTRICT",
                };
                columns.push(ColumnDef {
                    name: r.field_name.clone(),
                    sql_type: crate::type_mapping::sql_type_for(
                        &fk.kind,
                        fk.max_length,
                        fk.auto,
                        &fk.name,
                        djangors_db::Dialect::Postgres,
                    )?,
                    nullable: matches!(r.on_delete, djangors_orm::OnDelete::SetNull),
                    primary_key: false,
                    unique: false,
                    default_sql: None,
                    references: Some(ForeignKeyRef {
                        table: target.table_name.clone(),
                        column: pk.column_name.clone(),
                        on_delete: on_delete.into(),
                    }),
                });
            }
            Ok(Operation::CreateTable {
                table_name: m.table_name.clone(),
                columns,
            })
        })
        .collect()
}

fn dfs(
    meta: &'static ModelMeta,
    models_by_struct: &HashMap<&str, &'static ModelMeta>,
    visiting: &mut HashSet<&str>,
    visited: &mut HashSet<&str>,
    order: &mut Vec<&'static ModelMeta>,
) -> Result<(), MigrationError> {
    let struct_name = meta.struct_name;
    if visited.contains(struct_name) {
        return Ok(());
    }
    if visiting.contains(struct_name) {
        let mut cycle_models: Vec<String> = visiting.iter().map(|&s| s.to_string()).collect();
        cycle_models.sort();
        return Err(MigrationError::CyclicDependency {
            models: cycle_models,
        });
    }

    visiting.insert(struct_name);

    for relation in meta.relations {
        let target_meta = (relation.target)();
        if let Some(&dep_meta) = models_by_struct.get(target_meta.struct_name) {
            dfs(dep_meta, models_by_struct, visiting, visited, order)?;
        }
    }

    visiting.remove(struct_name);
    visited.insert(struct_name);
    order.push(meta);

    Ok(())
}
