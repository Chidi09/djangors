//! Django-style ORM with querysets and model metadata for Djangors.
//!
//! This crate defines runtime metadata types (`ModelMeta`, `FieldMeta`, etc.)
//! which describe database models. These metadata structures serve as the
//! single source of truth for migrations, serializers, admin interfaces, and more.

pub mod aggregate;
pub mod error;
pub mod expr;
pub mod meta;
pub mod queryset;

pub use djangors_db;
pub use inventory;
pub use sqlx;

pub use meta::{
    all_registered_models, DefaultValue, FieldKind, FieldMeta, ForeignKey, IndexMeta, Model,
    ModelMeta, ModelRegistration, OnDelete, RelationKind, RelationMeta,
};

pub use aggregate::{AggExpr, AggResult};
pub use error::{FromRow, OrmError};
pub use expr::{CompareOp, Expr, UnresolvedCompare, UnresolvedExpr, Value};
pub use queryset::QuerySet;

#[cfg(test)]
#[allow(unused_extern_crates)]
extern crate self as djangors_orm;

#[cfg(test)]
mod tests;
