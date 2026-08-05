# Object-Level Permissions (generic foreign keys)

`djangors-contrib-guardian` plus `djangors-contrib-contenttypes` give you Django-guardian-style
object-level permissions. The first stores one permission row per `(user, codename, object)`; the
second provides the generic foreign key that lets those rows point at **any** model — no
permissions table per model needed.

## The problem: model-level permissions can't target a single row

`djangors-auth`'s `has_perm(db, user_id, codename)` and `is_superuser()` answer only one question:
*does this user hold `codename` on the whole model?* They can say "Alice can change documents", so
the guard passes for every `Document` row. They cannot express:

- "Alice can edit document #42 but not document #17."
- "Bob may read only the documents someone shared with him."

Object-level permissions solve this with a tiny extra table: one row per `(user, codename,
object)`. A check becomes "model-level permission **or** an object row exists", which is exactly
what `has_perm_for_object` computes.

## Why `contenttypes` is the enabling primitive

A naive fix for object permissions is a join table per model (`document_permissions`,
`image_permissions`, ...). That doesn't scale, and every new model needs a new table with a new
service function. A generic foreign key avoids all of it: instead of a real foreign key to a
specific table, you store `(content_type_id, object_id)`, where `content_type_id` points at a row
saying *which* model it is (`app_label`, `model_name`).

```rust,illustrative
use djangors_macros::Model;

#[derive(Model, Debug, Clone)]
#[djangors(
    app = "djangors_contrib_contenttypes",
    table_name = "djangors_content_type",
    unique_together = [["app_label", "model_name"]]
)]
pub struct ContentType {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 100)]
    pub app_label: String,
    #[djangors(max_length = 100)]
    pub model_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenericForeignKey {
    pub content_type_id: i64,
    pub object_id: i64,
}
```

`ObjectPermission` uses exactly this shape, but stored inline: it carries `app_label`,
`model_name`, and `object_id` directly so the lookup needs no extra join. That one table can name a
`Document`, an `Image`, or anything else a permission should target.

## The `ObjectPermission` model

`ObjectPermission` (`#[derive(Model)]`, table `djangors_object_permission`) records one grant:

- `user`: `ForeignKey<djangors_auth::User>` — who holds the permission.
- `permission`: `ForeignKey<djangors_auth::Permission>` — which codename (must already exist in
  `auth_permission`).
- `app_label` (`max_length = 100`) and `model_name` (`max_length = 100`) — the target model.
- `object_id: i64` — the target row's primary key.

Calling `grant_object_permission` twice for the same `(user, codename, object)` is idempotent; it
returns the existing row instead of inserting a duplicate.

## Granting, checking, and revoking

```rust,compile
# async fn grant_check_revoke() -> Result<(), Box<dyn std::error::Error>> {
# let db = djangors_db::Database::connect(&djangors_db::config::DatabaseConfig::new("postgres://postgres:postgres@localhost/djangors_test")).await.unwrap();
use djangors_contrib_guardian::{grant_object_permission, has_perm_for_object, revoke_object_permission};

// Grant: upsert a row granting user 7 "change_document" on Document 42.
grant_object_permission(&db, 7, "change_document", "documents", "Document", 42).await?;

// Check: `true` if model-level has_perm allows it, OR an object row exists for
// this exact (user, codename, app_label, model_name, object_id).
let allowed = has_perm_for_object(&db, 7, "change_document", "documents", "Document", 42).await?;

// Revoke: deletes the row, returning `true`; `false` when nothing matched
// (or the codename isn't registered in `auth_permission`).
revoke_object_permission(&db, 7, "change_document", "documents", "Document", 42).await?;
# Ok(())
# }
```

`has_perm_for_object` first delegates to `djangors_auth::has_perm`, so a user with the model-level
permission (or via group) is automatically allowed on every object — object rows only *add*
restrictions-turned-grants, never remove them.

## Content-type bookkeeping

`djangors_contrib_contenttypes` keeps the `djangors_content_type` table in sync and turns generic
keys in both directions:

```rust,illustrative
use djangors_contrib_contenttypes::ContentTypeError;

// Registers one row per registered model (run at startup). Returns how many were written.
async fn bootstrap(db: &djangors_db::Database) -> Result<usize, ContentTypeError> {
    djangors_contrib_contenttypes::sync_content_types(db).await
}

// Build a generic key for a compile-time-known model, creating its ContentType row lazily.
async fn key_for_typed(db: &djangors_db::Database, doc_id: i64) -> Result<djangors_contrib_contenttypes::GenericForeignKey, ContentTypeError> {
    djangors_contrib_contenttypes::generic_key_for::<Document>(db, doc_id).await
}

// Reverse lookup: ContentTypeError::NotFound(id) if the row is missing.
async fn resolve(db: &djangors_db::Database, ct_id: i64) -> Result<(String, String), ContentTypeError> {
    djangors_contrib_contenttypes::resolve_content_type(db, ct_id).await
}
```

`sync_content_types` is idempotent (`ON CONFLICT DO NOTHING`), and `generic_key_for` inserts the
row on demand, so you can call either freely at startup without worrying about duplicates.

## A combined worked example

Put it together. At startup, synchronize both content types and permissions; in handlers, grant,
guard, log, and revoke.

```rust,illustrative
use djangors_macros::Model;
use djangors_orm::{ForeignKey, Model, q};

#[derive(Model, Debug, Clone)]
#[djangors(app = "documents", table_name = "documents_document")]
struct Document {
    #[djangors(primary_key, auto)]
    id: i64,
    owner: ForeignKey<djangors_auth::User>,
    #[djangors(max_length = 200)]
    title: String,
    body: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "activity", table_name = "activity_feed_entry")]
struct ActivityFeedEntry {
    #[djangors(primary_key, auto)]
    id: i64,
    actor: ForeignKey<djangors_auth::User>,
    content_type_id: i64,
    object_id: i64,
    #[djangors(max_length = 200)]
    message: String,
}

async fn startup(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
    djangors_contrib_contenttypes::sync_content_types(db).await?;
    djangors_auth::sync_permissions(db).await?;
    Ok(())
}

async fn share_document(
    db: &djangors_db::Database,
    owner_id: i64,
    user_id: i64,
    doc_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Generic foreign key: the feed entry can point at any model, not just this one.
    let gk = djangors_contrib_contenttypes::generic_key_for::<Document>(db, doc_id).await?;

    djangors_contrib_guardian::grant_object_permission(
        db, user_id, "change_document", "documents", "Document", doc_id,
    )
    .await?;

    ActivityFeedEntry {
        id: 0,
        actor: ForeignKey::new(owner_id),
        content_type_id: gk.content_type_id,
        object_id: gk.object_id,
        message: format!("shared document {doc_id}"),
    }
    .save(db)
    .await?;
    Ok(())
}

async fn edit_document(db: &djangors_db::Database, user_id: i64, doc_id: i64) -> Result<(), Box<dyn std::error::Error>> {
    let allowed = djangors_contrib_guardian::has_perm_for_object(
        db, user_id, "change_document", "documents", "Document", doc_id,
    )
    .await?;
    if !allowed {
        return Err("You cannot edit this document".into());
    }
    let doc: Option<Document> = Document::objects()
        .filter(q!(id = doc_id))
        .first(db)
        .await?; // Some(_) here: we already know the object exists
    Ok(())
}

async fn unshare_document(
    db: &djangors_db::Database,
    user_id: i64,
    doc_id: i64,
) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(djangors_contrib_guardian::revoke_object_permission(
        db, user_id, "change_document", "documents", "Document", doc_id,
    )
    .await?)
}
```

Later, when you render an `ActivityFeedEntry`, resolve its generic key back to a model:
`resolve_content_type(db, entry.content_type_id)?` yields `(app_label, model_name)`. That pair
matches the codename format below, so you can also answer "what can this user do with *this* feed
object?".

## Prerequisites

> [!NOTE]
> Object-level permissions are layered on `djangors-auth`: a `change_document` codename must
> already exist as a `djangors_auth::Permission` row or both `grant_object_permission` fails to
> find it and `revoke_object_permission` silently returns `false`. Run
> `dj createpermissions` (or `djangors_auth::sync_permissions`) first — see
> [Authentication and Permissions](auth.md).

- Add `djangors-contrib-guardian` and `djangors-contrib-contenttypes` to your `Cargo.toml`.
- Run `dj makemigrations && dj migrate` so `djangors_object_permission` and
  `djangors_content_type` exist.
- Run `dj createpermissions` so codenames like `change_document` exist before granting.
- Call `sync_content_types(&db)` at startup (in `main` before serving requests) so the generic
  foreign key machinery has stable, predictable IDs.

## Codename format

Codename follows the same `{action}_{model}` contract as `djangors-auth`'s generated permission
codes (`view`, `add`, `change`, `delete` + the model's Rust struct name, lowercased as-is):
`change_document`, `view_document`, `delete_document`. The target lookup uses `app_label` +
`model_name` (e.g. `("documents", "Document")`), and `djangors_auth::sync_permissions`/`dj
createpermissions` build exactly these codenames from registered model metadata, so the two layers
align with no custom naming.
