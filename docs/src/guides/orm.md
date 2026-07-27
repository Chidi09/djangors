# Object-Relational Mapping (ORM)

`djangors-orm` provides Django-style querysets, model metadata, expression macros, and database integration for PostgreSQL (via `sqlx`).

## Defining Models

Models are Rust structs decorated with `#[derive(Model)]` from `djangors-macros`.

```rust,compile
# use djangors_orm::Model;
use djangors_macros::Model;
use djangors_orm::ForeignKey;
use chrono::{DateTime, Utc};

#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_question", ordering = ["-pub_date"])]
pub struct Question {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 200)] pub question_text: String,

    pub pub_date: DateTime<Utc>,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_choice")]
pub struct Choice {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "choices"))]
    pub question: ForeignKey<Question>,

    #[djangors(max_length = 200)]
    pub choice_text: String,

    #[djangors(default = 0)]
    pub votes: i32,
}
```

### Model Attributes (`#[djangors(...)]`)

#### Struct-Level Attributes
- **`app = "app_label"`** *(required)*: Specifies the application label (e.g., `"polls"`).
- **`table_name = "custom_table"`** *(optional)*: Database table name. Defaults to `{app_label}_{snake_case_struct_name}`.
- **`ordering = ["field", "-field"]`** *(optional)*: Default ordering for querysets when `.order_by()` is not called. Prefix `-` denotes descending.
- **`unique_together = [["field1", "field2"]]`** *(optional)*: Defines composite unique constraints.

#### Field-Level Attributes
- **`primary_key`**: Marks the single primary key field. Automatically implies `unique` and `db_index`. Every model must have exactly one primary key field.
- **`auto`**: Marks auto-incrementing / database-generated fields (such as `BIGSERIAL`). Ignored during `save()` insertions.
- **`max_length = N`**: Sets maximum string length. Valid only on `String` fields (maps to `FieldKind::Char`). `String` without `max_length` maps to `FieldKind::Text`.
- **`default = value`**: Sets default field value (`Text`, `I64`, `F64`, or `Bool`).
- **`unique`**: Adds a `UNIQUE` constraint to the column.
- **`db_index`**: Creates a database index on the column.
- **`verbose_name = "..."`**, **`help_text = "..."`**: Descriptive labels for admin/forms metadata.
- **`column = "col_name"`**: Overrides the database column name if different from the struct field name.
- **`foreign_key(on_delete = "...", related_name = "...")`**: Configures foreign key behavior. `on_delete` options: `"cascade"` (default), `"protect"`, `"set_null"`, `"restrict"`, `"do_nothing"`.

### Supported Field Types
- Integer: `i32` (`FieldKind::Integer`), `i64` (`FieldKind::BigInt`)
- Float: `f32`, `f64` (`FieldKind::Float`)
- String: `String` (`FieldKind::Char` or `FieldKind::Text`)
- Boolean: `bool` (`FieldKind::Boolean`)
- Datetime: `chrono::DateTime<chrono::Utc>` (`FieldKind::DateTime`)
- Foreign Key: `ForeignKey<TargetModel>`
- Nullable: `Option<T>` for any of the above types

> [!NOTE]
> Types like `NaiveDate`, `NaiveTime`, `Duration`, `Uuid`, and `Decimal` are defined in `FieldKind` for schema metadata, but currently trigger a compile error on `Model` derive for save/update operations.

---

## QuerySet Methods

Access querysets using `Model::objects()` (or `QuerySet::<T>::new()`).

### Filtering and Ordering
- **`.filter(q!(field = value, ...))`**: Applies filter expressions combined with `AND`.
- **`.filter_or_icontains(&["field1", "field2"], "term")`**: Generates case-insensitive `ILIKE %term%` queries across specified text fields combined with `OR`.
- **`.filter_datetime_range(field, gte, lt)`**: Filters datetime fields in the half-open interval `[gte, lt)`.
- **`.order_by("field")` / `.order_by("-field")`**: Orders results. Prefix `-` means `DESC`.
- **`.limit(n)` / `.offset(n)`**: Paginates results using standard SQL `LIMIT` and `OFFSET`.

### Execution Methods
- **`.all(db).await`**: Executes select query and returns `Vec<T>`.
- **`.get(db).await`**: Fetches a single object. Returns `Ok(T)`, `Err(OrmError::NotFound)` if 0 rows returned, or `Err(OrmError::MultipleObjectsReturned)` if >1 row.
- **`.first(db).await`**: Returns `Result<Option<T>, OrmError>` (`LIMIT 1`).
- **`.exists(db).await`**: Returns `Result<bool, OrmError>` check (`SELECT 1 ... LIMIT 1`).
- **`.count(db).await`**: Returns `Result<i64, OrmError>` count (`SELECT COUNT(*) ...`).
- **`.aggregate(db, vec![AggExpr::...]).await`**: Evaluates aggregate functions: `AggExpr::Count`, `AggExpr::Sum`, `AggExpr::Avg`, `AggExpr::Min`, `AggExpr::Max`.
- **`.update(db, set!(...)).await`**: Performs bulk updates. Returns `Result<u64, OrmError>` (rows affected count).
- **`QuerySet::<T>::delete_by_pk(db, pk).await`**: Static helper deleting a row by its primary key.
- **`.select_related::<R>(db, "relation_field").await`**: Eagerly loads related model `R` alongside `T` to avoid N+1 queries.

---

## Expressions & Macros

### `q!` Macro
Constructs lookup filter expressions:
```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q, Model};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let qs = Question::objects().filter(q!(question_text = "What is your name?"))?;
# Ok(())
# }
```

### `set!` Macro & `F` Expressions
Constructs update assignment lists, supporting `F` expressions for in-database atomic updates:
```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q, Model};
# async fn run_update(db: &djangors_db::Database, choice_id: i64) -> Result<(), Box<dyn std::error::Error>> {
use djangors_orm::{set, F};

// Increment votes by 1 atomically in SQL: UPDATE polls_choice SET votes = votes + 1 WHERE ...
Choice::objects()
    .filter(q!(id = choice_id))?
    .update(db, set!(votes = F("votes") + 1))
    .await?;
# Ok(())
# }
```

---

## Model Instance Persistence Methods

Models derived with `#[derive(Model)]` implement instance CRUD operations:
- **`model.save(db).await`**: Inserts a new row into the database, ignoring `auto` fields, and returns a fresh model instance with database-assigned primary key values.
- **`model.update(db).await`**: Updates all non-primary key fields of an existing row matching `model.id`. Returns `Err(OrmError::NotFound)` if no row matched.
- **`model.delete(db).await`**: Deletes the row matching `model.id`. Returns `Err(OrmError::NotFound)` if no row was deleted.

---

## Relationships & Many-to-Many

Foreign keys are stored in `ForeignKey<T>` wrapper fields (e.g. `#[djangors(foreign_key(on_delete = "cascade", related_name = "choices"))] pub question: ForeignKey<Question>`). Foreign keys access `.id` directly.

> [!NOTE]
> Direct many-to-many relationship querying via implicit junction table abstraction is not implemented in `djangors-orm`. Explicit join models (such as `UserGroup`, `GroupPermission`, and `UserPermission` in `djangors-auth`) are defined and queried directly as models with foreign keys.
