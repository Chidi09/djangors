#![deny(missing_docs)]
//! Database backends and connection pool management for the Djangors web framework.
//!
//! This crate provides connection pooling, config-driven database setup,
//! and transaction support with explicit isolation levels, built on top of SQLx.

/// Connection pool and database configuration options.
pub mod config;
/// Core database connection wrapper and transaction helpers.
pub mod database;
/// Database errors and failure types.
pub mod error;

pub use config::DatabaseConfig;
#[doc(hidden)]
pub use database::{isolation_level_sql, BoxFuture};
pub use database::{Database, IsolationLevel};
pub use error::DbError;
