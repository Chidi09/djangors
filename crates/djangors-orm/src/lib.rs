#![deny(missing_docs)]
//! Django-style ORM with querysets and model metadata for Djangors.
//!
//! This crate defines runtime metadata types (`ModelMeta`, `FieldMeta`, etc.)
//! which describe database models. These metadata structures serve as the
//! single source of truth for migrations, serializers, admin interfaces, and more.

/// Aggregate function definitions and result types.
pub mod aggregate;
/// Error types and row-mapping trait for ORM operations.
pub mod error;
/// Query expression tree nodes, operators, and macros.
pub mod expr;
/// Model runtime metadata structures and field definitions.
pub mod meta;
/// Type-safe fluent QuerySet query builder.
pub mod queryset;
/// Model lifecycle signals.
pub mod signals;

pub use djangors_db;
pub use djangors_forms;
pub use inventory;
pub use sqlx;

pub use meta::{
    all_registered_models, DefaultValue, FieldKind, FieldMeta, FieldSnapshot, ForeignKey,
    IndexMeta, Model, ModelMeta, ModelRegistration, ModelSnapshot, OnDelete, RelationKind,
    RelationMeta, RelationSnapshot, SnapshotDefault,
};

pub use aggregate::{AggExpr, AggResult};
pub use error::{FromRow, OrmError};
pub use expr::{
    ArithOp, CompareOp, Expr, IntoSetExpr, SetExpr, UnresolvedCompare, UnresolvedExpr, Value, F,
};
pub use queryset::{prefetch_related, QuerySet};

#[cfg(test)]
#[allow(unused_extern_crates)]
extern crate self as djangors_orm;

#[cfg(test)]
mod tests;
