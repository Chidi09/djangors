//! The `Tenant` and `TenantMembership` models.

use djangors_macros::Model;
use djangors_orm::ForeignKey;

/// A tenant (an organization/school/bank-branch/seller account - deliberately generic, see the
/// design doc for why). Applications name their own domain concept on top of this.
#[derive(Model, Debug, Clone)]
#[djangors(
    app = "djangors_contrib_tenancy",
    table_name = "djangors_tenancy_tenant"
)]
pub struct Tenant {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Human-readable tenant name.
    #[djangors(max_length = 255)]
    pub name: String,
    /// URL-safe unique identifier for this tenant.
    #[djangors(unique, max_length = 100)]
    pub slug: String,
    /// Whether this tenant is currently active (an inactive tenant's users should be denied
    /// access - enforcing that is left to the application, this field just records the state).
    pub is_active: bool,
    /// When this tenant was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Links a user to a tenant with a role. A user may have more than one membership row (belonging
/// to multiple tenants) - `unique_together` on (user, tenant) prevents duplicate membership rows
/// for the same pair, not duplicate tenants for a user overall.
#[derive(Model, Debug, Clone)]
#[djangors(
    app = "djangors_contrib_tenancy",
    table_name = "djangors_tenancy_membership",
    unique_together = [["user", "tenant"]]
)]
pub struct TenantMembership {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// The user who is a member of `tenant`.
    #[djangors(foreign_key(on_delete = "cascade"))]
    pub user: ForeignKey<djangors_auth::User>,
    /// The tenant `user` belongs to.
    #[djangors(foreign_key(on_delete = "cascade"))]
    pub tenant: ForeignKey<Tenant>,
    /// Application-defined role string within this tenant (e.g. "admin", "teacher", "student") -
    /// deliberately a plain string, not an enum, matching how role/permission strings are handled
    /// elsewhere in this codebase rather than inventing a new fixed vocabulary the framework would
    /// have to own.
    #[djangors(max_length = 50)]
    pub role: String,
    /// When this membership was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}
