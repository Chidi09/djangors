//! Django-style ORM with querysets and model metadata for Djangors.
//!
//! This crate defines runtime metadata types (`ModelMeta`, `FieldMeta`, etc.)
//! which describe database models. These metadata structures serve as the
//! single source of truth for migrations, serializers, admin interfaces, and more.

pub mod meta;

pub use meta::{
    all_registered_models, DefaultValue, FieldKind, FieldMeta, ForeignKey, IndexMeta, Model,
    ModelMeta, ModelRegistration, OnDelete, RelationKind, RelationMeta,
};

#[cfg(test)]
mod tests;
