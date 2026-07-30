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
- **`.filter(q!(field = value, ...))`**: Applies filter expressions combined with `AND`. Accepts a whole `Q`-style tree, so `OR` and `NOT` compose (see [Combining filters](#combining-filters-or-and-not)).
- **`.exclude(q!(...))`**: The negation of `.filter()` — Django's `.exclude()`.
- **`.filter_or_icontains(&["field1", "field2"], "term")`**: Generates case-insensitive `ILIKE %term%` queries across specified text fields combined with `OR`.
- **`.filter_datetime_range(field, gte, lt)`**: Filters datetime fields in the half-open interval `[gte, lt)`.
- **`.order_by("field")` / `.order_by("-field")`**: Orders results. Prefix `-` means `DESC`.
- **`.limit(n)` / `.offset(n)`**: Paginates results using standard SQL `LIMIT` and `OFFSET`.
- **`.debug_sql()` / `.debug_params()`**: The `SELECT` this queryset would run and the parameters it would bind, for debugging and tests — Django's `str(queryset.query)`. Placeholders stay as `$1`, `$2`, … rather than being interpolated, so the output is never a runnable statement.

### Lookup Suffixes

Append a lookup to a field name to change the comparison. Without one, `=` is used.

| Suffix | SQL | Notes |
| --- | --- | --- |
| *(none)* / `__eq` | `=` | |
| `__ne` | `<>` | |
| `__lt` `__lte` `__gt` `__gte` | `<` `<=` `>` `>=` | |
| `__contains` / `__icontains` | `LIKE` / `ILIKE` `%v%` | |
| `__startswith` / `__endswith` | `LIKE 'v%'` / `LIKE '%v'` | |
| `__iexact` | `ILIKE 'v'` | Case-insensitive, no wildcards |
| `__in` | `IN (...)` | Takes a `Vec`; an empty one compiles to `FALSE`, never invalid `IN ()` |
| `__isnull` | `IS NULL` / `IS NOT NULL` | `= false` inverts it; binds no parameter |
| `__regex` / `__iregex` | `~` / `~*` | POSIX regular expression |

An unrecognised suffix is treated as part of the field name, so a typo surfaces
as `OrmError::FieldNotFound` rather than silently becoming an equality test.

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

### Combining filters: OR, AND, NOT

A `q!(...)` produces a value you can combine with `|` (OR), `&` (AND), and `!`
(NOT), which is how Django's `Q` objects spell the same thing. Successive
`.filter()` calls are still `AND`ed together.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q, Model};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
// WHERE (votes = 0 OR votes > 100) AND NOT (choice_text = 'spam')
let qs = Choice::objects()
    .filter(q!(votes = 0i32) | q!(votes__gt = 100i32))?
    .filter(!q!(choice_text = "spam"))?;

// `.exclude()` is the same as filtering on a negation.
let qs = Choice::objects().exclude(q!(votes = 0i32))?;
# let _ = qs;
# Ok(())
# }
```

### Comparing two columns: `q_f!`

Where `q!` compares a column to a bound value, `q_f!` compares two columns on
the same row — `F()` on the filter side. The lookup suffix rides on the
left-hand field, as it does in Django.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q_f, Model};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
// WHERE "id" <> "votes"  — neither side is a bind parameter.
let qs = Choice::objects().filter(q_f!(id__ne votes))?;
# let _ = qs;
# Ok(())
# }
```

### Grouping: `annotate` and `values`

`.annotate()` is Django's `.values(...).annotate(...)`: it groups by the given
fields and computes aggregates per group, returning `GroupRow`s rather than
model instances.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{Model, aggregate::AggExpr};
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
let rows = Choice::objects()
    .annotate(db, &["question"], vec![("total", AggExpr::Sum { field: "votes" })])
    .await?;

for row in &rows {
    let question_id = row.key("question");
    let total = row.get("total");
    println!("{question_id:?} => {total:?}");
}
# Ok(())
# }
```

Ordering is dropped for grouped queries, because ordering by a column that is
not in the `GROUP BY` is not valid SQL; sort the returned rows in Rust instead.

`.values()` and `.values_list()` select a projection instead of whole models,
which is useful when a table has columns you do not want to pay to fetch:

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::Model;
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
let rows = Question::objects().values(db, &["id", "question_text"]).await?;
let ids = Question::objects().values_list(db, "id").await?;
# let _ = (rows, ids);
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
