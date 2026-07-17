//! Database backends and connection pool management for the Djangors web framework.
//!
//! This crate provides connection pooling, config-driven database setup,
//! and transaction support with explicit isolation levels, built on top of SQLx.

pub mod config;
pub mod database;
pub mod error;

pub use config::DatabaseConfig;
pub use database::{isolation_level_sql, BoxFuture, Database, IsolationLevel};
pub use error::DbError;
