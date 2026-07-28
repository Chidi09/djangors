#![deny(missing_docs)]
//! Multi-tenancy support for Djangors: a shared-schema, row-level-scoping model (not
//! schema-per-tenant or database-per-tenant - see `docs/design/12.1-multi-tenancy.md` for the full
//! rationale). Ships a `Tenant` model, a `TenantMembership` join model (a user can belong to more
//! than one tenant), a `TenantResolutionLayer` middleware that resolves and verifies the current
//! request's tenant, and a `tenant_scope()` helper for one-line `djangors_rest::Scoped`
//! implementations.

mod middleware;
mod models;
mod scope;

pub use middleware::{CurrentTenant, TenantResolutionLayer};
pub use models::{Tenant, TenantMembership};
pub use scope::tenant_scope;

#[cfg(test)]
mod tests;
