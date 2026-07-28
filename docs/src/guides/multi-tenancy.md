# Multi-tenancy (`djangors-contrib-tenancy`)

`djangors-contrib-tenancy` provides row-level multi-tenancy: a shared database schema where
tenant-scoped tables carry a foreign key to a `Tenant`, and every query against them is
automatically and *verifiably* restricted to the current request's tenant. See
`docs/design/12.1-multi-tenancy.md` in the repository for the full design rationale, including why
this model was chosen over schema-per-tenant or database-per-tenant.

## `Tenant` and `TenantMembership`

```rust,compile
use djangors_contrib_tenancy::{Tenant, TenantMembership};

fn describe(tenant: &Tenant, membership: &TenantMembership) -> String {
    format!("{} ({})", tenant.name, membership.role)
}
```

`Tenant` is deliberately generic (`name`/`slug`/`is_active`). It represents a school, a bank
branch, or a seller account equally well; your application names its own domain concept on top.

`TenantMembership` links a user to a tenant with a role, and a user may hold **more than one**
membership (belonging to several tenants). That's more general than simply putting a `tenant_id`
directly on your user model, and it doesn't require modifying `djangors-auth`'s `User` struct at
all.

## Resolving the current tenant (`TenantResolutionLayer`)

A request's tenant is never trusted from a client-supplied header alone. `TenantResolutionLayer`
reads the `X-Tenant-Id` header, then verifies the authenticated user actually has a real
`TenantMembership` row for that tenant before accepting it. A forged or stale header for a
tenant the user doesn't belong to is silently rejected (no `CurrentTenant` gets set at all, which
downstream scoping then treats as unauthorized), never treated as a default/fallback tenant.

```rust,illustrative
use djangors_contrib_tenancy::TenantResolutionLayer;
use djangors_orm::djangors_db::Database;

fn build_layer(db: Database) -> TenantResolutionLayer<impl Fn(&hyper::Request<hyper::body::Incoming>) -> Option<i64> + Clone> {
    // The extractor closure reads whatever your own upstream auth middleware already stored on
    // the request (e.g. an authenticated user id extension) - this crate doesn't assume a
    // specific auth setup.
    TenantResolutionLayer::new(db, |req| {
        req.extensions().get::<CurrentUserId>().map(|u| u.0)
    })
}

#[derive(Clone, Copy)]
struct CurrentUserId(i64);
```

## Scoping a model to the current tenant

`djangors-rest` already ships the enforcement primitive this crate builds on: the `Scoped` trait,
which `ScopedViewSet<M>` *requires*. A model without a `scope()` implementation simply cannot be
used with `ScopedViewSet` at all, a compile-time guarantee rather than a convention someone has to
remember. `tenant_scope()` is a one-line helper for writing that implementation:

```rust,compile
use djangors_contrib_tenancy::tenant_scope;
use djangors_core::{Request, error::DjangorsError};
use djangors_macros::Model;
use djangors_orm::QuerySet;
use djangors_rest::Scoped;

#[derive(Model, Debug, Clone, sqlx::FromRow)]
#[djangors(app = "myapp", table_name = "myapp_schoolclass")]
struct SchoolClass {
    #[djangors(primary_key, auto)]
    id: i64,
    #[djangors(foreign_key(on_delete = "cascade"))]
    tenant: djangors_orm::ForeignKey<djangors_contrib_tenancy::Tenant>,
    #[djangors(max_length = 100)]
    name: String,
}

impl Scoped for SchoolClass {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        tenant_scope(req, qs, "tenant")
    }
}
```

Every `ScopedViewSet::<SchoolClass>` request now only ever sees rows belonging to the current,
membership-verified tenant. A user for tenant A genuinely cannot see tenant B's classes, list
them, or fetch one by id.

## What this doesn't cover yet

`djangors-admin` has no tenant-scoping integration yet: an admin user for one tenant can browse
another tenant's rows in the generated admin UI. This is the design doc's own flagged
highest-value follow-up, not yet built. Schema-per-tenant/database-per-tenant support,
subdomain-based tenant resolution (v1 is `X-Tenant-Id`-header only), and automatic Postgres
Row-Level Security policy generation are also deliberately out of scope for this first slice.
