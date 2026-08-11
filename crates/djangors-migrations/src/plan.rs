use crate::error::MigrationError;
use crate::operation::{ColumnDef, ForeignKeyRef, Operation};
use crate::type_mapping::field_meta_to_sql_type;
use djangors_db::Dialect;
use djangors_orm::{all_registered_models, ModelMeta};
use std::collections::{HashMap, HashSet};

/// Builds a sequence of migration DDL operations to create tables for all registered models.
pub fn build_create_all_plan(dialect: Dialect) -> Result<Vec<Operation>, MigrationError> {
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
            let sql_type = field_meta_to_sql_type(field, dialect)?;
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
            // ManyToMany is not a column: its join table is emitted separately below.
            if relation.kind == djangors_orm::RelationKind::ManyToMany {
                continue;
            }
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
            let sql_type = field_meta_to_sql_type(&fk_field_meta, dialect)?;

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
                unique: relation.kind == djangors_orm::RelationKind::OneToOne,
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
            check_constraints: build_check_constraints(model),
        });

        for index in model.indexes {
            if index.fields.is_empty() {
                continue;
            }
            let columns: Vec<String> = index.fields.iter().map(|f| f.to_string()).collect();
            operations.push(Operation::CreateIndex {
                index_name: index.name.to_string(),
                table_name: model.table_name.to_string(),
                columns,
            });
        }

        // Emit join tables for every many-to-many relation on this model.
        for relation in model.relations {
            if relation.kind != djangors_orm::RelationKind::ManyToMany {
                continue;
            }
            let target_meta = (relation.target)();
            let source_pk = model.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                MigrationError::Database(djangors_db::DbError::ConnectionFailed(
                    "M2M source model has no primary key".into(),
                ))
            })?;
            let target_pk = target_meta
                .fields
                .iter()
                .find(|f| f.primary_key)
                .ok_or_else(|| {
                    MigrationError::Database(djangors_db::DbError::ConnectionFailed(
                        "M2M target model has no primary key".into(),
                    ))
                })?;

            let source_col = format!("{}_id", model.struct_name.to_lowercase());
            let target_col = format!("{}_id", target_meta.struct_name.to_lowercase());
            let source_type = field_meta_to_sql_type(source_pk, dialect)?;
            let target_type = field_meta_to_sql_type(target_pk, dialect)?;

            let through = djangors_orm::m2m_through_table(model, relation);
            operations.push(Operation::CreateTable {
                table_name: through,
                columns: vec![
                    ColumnDef {
                        name: source_col,
                        sql_type: source_type,
                        nullable: false,
                        primary_key: false,
                        unique: false,
                        default_sql: None,
                        references: Some(ForeignKeyRef {
                            table: model.table_name.to_string(),
                            column: source_pk.column_name.to_string(),
                            on_delete: "CASCADE".to_string(),
                        }),
                    },
                    ColumnDef {
                        name: target_col,
                        sql_type: target_type,
                        nullable: false,
                        primary_key: false,
                        unique: false,
                        default_sql: None,
                        references: Some(ForeignKeyRef {
                            table: target_meta.table_name.to_string(),
                            column: target_pk.column_name.to_string(),
                            on_delete: "CASCADE".to_string(),
                        }),
                    },
                ],
                check_constraints: vec![],
            });
        }
    }

    Ok(operations)
}

/// Builds the initial plan from metadata emitted by a project's binary.
pub fn build_create_plan_from_snapshots(
    snapshots: &[djangors_orm::ModelSnapshot],
    dialect: Dialect,
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
    Ok(ordered
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
                        dialect,
                    )?,
                    nullable: f.nullable,
                    primary_key: f.primary_key,
                    unique: f.unique,
                    default_sql: None,
                    references: None,
                });
            }
            for r in &m.relations {
                if r.kind == djangors_orm::RelationKind::ManyToMany {
                    continue;
                }
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
                        dialect,
                    )?,
                    nullable: matches!(r.on_delete, djangors_orm::OnDelete::SetNull),
                    primary_key: false,
                    unique: r.kind == djangors_orm::RelationKind::OneToOne,
                    default_sql: None,
                    references: Some(ForeignKeyRef {
                        table: target.table_name.clone(),
                        column: pk.column_name.clone(),
                        on_delete: on_delete.into(),
                    }),
                });
            }
            let mut ops = vec![Operation::CreateTable {
                table_name: m.table_name.clone(),
                columns,
                check_constraints: vec![],
            }];
            for i in &m.indexes {
                if i.fields.is_empty() {
                    continue;
                }
                ops.push(Operation::CreateIndex {
                    index_name: i.name.clone(),
                    table_name: m.table_name.clone(),
                    columns: i.fields.clone(),
                });
            }
            for r in &m.relations {
                if r.kind != djangors_orm::RelationKind::ManyToMany {
                    continue;
                }
                let target = by_name.get(r.target_struct.as_str()).ok_or_else(|| {
                    MigrationError::Database(djangors_db::DbError::ConnectionFailed(format!(
                        "unknown M2M target {}",
                        r.target_struct
                    )))
                })?;
                let source_pk = m.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                    MigrationError::Database(djangors_db::DbError::ConnectionFailed(
                        "M2M source snapshot has no primary key".into(),
                    ))
                })?;
                let target_pk = target
                    .fields
                    .iter()
                    .find(|f| f.primary_key)
                    .ok_or_else(|| {
                        MigrationError::Database(djangors_db::DbError::ConnectionFailed(
                            "M2M target snapshot has no primary key".into(),
                        ))
                    })?;
                let through = format!("{}_{}", m.table_name, r.field_name);
                ops.push(Operation::CreateTable {
                    table_name: through,
                    columns: vec![
                        ColumnDef {
                            name: format!("{}_id", m.struct_name.to_lowercase()),
                            sql_type: crate::type_mapping::sql_type_for(
                                &source_pk.kind,
                                source_pk.max_length,
                                false,
                                &source_pk.name,
                                dialect,
                            )?,
                            nullable: false,
                            primary_key: false,
                            unique: false,
                            default_sql: None,
                            references: Some(ForeignKeyRef {
                                table: m.table_name.clone(),
                                column: source_pk.column_name.clone(),
                                on_delete: "CASCADE".into(),
                            }),
                        },
                        ColumnDef {
                            name: format!("{}_id", target.struct_name.to_lowercase()),
                            sql_type: crate::type_mapping::sql_type_for(
                                &target_pk.kind,
                                target_pk.max_length,
                                false,
                                &target_pk.name,
                                dialect,
                            )?,
                            nullable: false,
                            primary_key: false,
                            unique: false,
                            default_sql: None,
                            references: Some(ForeignKeyRef {
                                table: target.table_name.clone(),
                                column: target_pk.column_name.clone(),
                                on_delete: "CASCADE".into(),
                            }),
                        },
                    ],
                    check_constraints: vec![],
                });
            }
            Ok(ops)
        })
        .collect::<Result<Vec<Vec<Operation>>, MigrationError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<Operation>>())
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

fn build_check_constraints(model: &'static ModelMeta) -> Vec<String> {
    let mut check_constraints = Vec::new();
    for field in model.fields {
        if !field.choices.is_empty() {
            let col = field.column_name;
            let table = model.table_name;
            let choice_vals: Vec<String> = field
                .choices
                .iter()
                .map(|(v, _)| format!("'{}'", v.replace('\'', "''")))
                .collect();
            let check = format!(
                "CONSTRAINT chk_{}_{} CHECK (\"{}\" IN ({}))",
                table,
                col,
                col,
                choice_vals.join(", ")
            );
            check_constraints.push(check);
        }
    }
    check_constraints
}

/// Compares two model snapshots and returns incremental DDL operations to bring
/// the database from `previous` to `current`. Django's `makemigrations` logic.
///
/// Detects new tables (CreateTable), new columns (AddColumn), dropped columns
/// (DropColumn), and column type changes (AlterColumnType — Postgres only).
pub fn detect_changes(
    previous: &[djangors_orm::ModelSnapshot],
    current: &[djangors_orm::ModelSnapshot],
    dialect: Dialect,
) -> Result<Vec<Operation>, MigrationError> {
    let prev: std::collections::HashMap<&str, &djangors_orm::ModelSnapshot> = previous
        .iter()
        .map(|s| (s.struct_name.as_str(), s))
        .collect();
    let curr: std::collections::HashMap<&str, &djangors_orm::ModelSnapshot> = current
        .iter()
        .map(|s| (s.struct_name.as_str(), s))
        .collect();

    let mut ops = Vec::new();

    for (name, snap) in &curr {
        if let Some(prev_snap) = prev.get(name) {
            let mut dropped = Vec::new();
            for pf in &prev_snap.fields {
                if !snap.fields.iter().any(|f| f.name == pf.name) {
                    dropped.push(pf.clone());
                }
            }
            for df in &dropped {
                ops.push(Operation::DropColumn {
                    table_name: snap.table_name.clone(),
                    column_name: df.column_name.clone(),
                });
            }

            for cf in &snap.fields {
                if let Some(pf) = prev_snap.fields.iter().find(|p| p.name == cf.name) {
                    if cf.kind != pf.kind || cf.max_length != pf.max_length {
                        let sql_type = crate::type_mapping::sql_type_for(
                            &cf.kind,
                            cf.max_length,
                            cf.auto,
                            &cf.name,
                            dialect,
                        )?;
                        ops.push(Operation::AlterColumnType {
                            table_name: snap.table_name.clone(),
                            column_name: cf.column_name.clone(),
                            new_sql_type: sql_type,
                        });
                    }
                } else {
                    let sql_type = crate::type_mapping::sql_type_for(
                        &cf.kind,
                        cf.max_length,
                        cf.auto,
                        &cf.name,
                        dialect,
                    )?;
                    ops.push(Operation::AddColumn {
                        table_name: snap.table_name.clone(),
                        column: ColumnDef {
                            name: cf.column_name.clone(),
                            sql_type,
                            nullable: cf.nullable,
                            primary_key: cf.primary_key,
                            unique: cf.unique,
                            default_sql: None,
                            references: None,
                        },
                    });
                }
            }
        } else {
            // New table: generate CreateTable from the live build_plan logic.
            let mut columns = Vec::new();
            for f in &snap.fields {
                columns.push(ColumnDef {
                    name: f.column_name.clone(),
                    sql_type: crate::type_mapping::sql_type_for(
                        &f.kind,
                        f.max_length,
                        f.auto,
                        &f.name,
                        dialect,
                    )?,
                    nullable: f.nullable,
                    primary_key: f.primary_key,
                    unique: f.unique,
                    default_sql: None,
                    references: None,
                });
            }
            ops.push(Operation::CreateTable {
                table_name: snap.table_name.clone(),
                columns,
                check_constraints: vec![],
            });
        }
    }

    Ok(ops)
}
