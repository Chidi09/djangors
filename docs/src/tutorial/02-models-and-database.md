# Tutorial Part 2: Models and Database Setup

In Part 2, we define our database models, configure database connectivity, add business logic methods to models, and inspect CLI database management tools.

> [!NOTE]
> All code in this part corresponds directly to [`examples/polls/src/models.rs`](file:///root/dev/Rango/examples/polls/src/models.rs).

---

## 1. Defining Models

In Djangors, models are plain Rust `struct`s decorated with `#[derive(Model)]` and procedural attributes under `#[djangors(...)]`.

Create `src/models.rs` and add the `Question` and `Choice` models:

```rust
use chrono::{DateTime, Utc};
use djangors_macros::Model;
use djangors_orm::ForeignKey;

#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_question", ordering = "-pub_date")]
pub struct Question {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 200)]
    pub question_text: String,

    #[djangors(verbose_name = "date published", db_index)]
    pub pub_date: DateTime<Utc>,
}

impl Question {
    /// Not a framework feature — an ordinary inherent method, exactly like
    /// Django's `was_published_recently()` on the tutorial's `Question` model.
    /// Proves model structs stay plain Rust structs you can add real methods to.
    pub fn was_published_recently(&self) -> bool {
        self.pub_date > Utc::now() - chrono::Duration::days(1)
    }
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

---

## 2. Understanding Model Attributes

Djangors macro attributes control ORM schema generation and behavior:

- `app = "polls"`: Associates the model with the `polls` app.
- `table_name = "polls_question"`: Explicitly maps the database table name.
- `ordering = "-pub_date"`: Sets default query ordering (descending by publication date).
- `#[djangors(primary_key, auto)]`: Designates an auto-incrementing primary key.
- `#[djangors(foreign_key(on_delete = "cascade", related_name = "choices"))]`: Wraps foreign key relationship to `Question` with cascade deletion.

---

## 3. Database Connection & Schema Management

Database initialization happens in `main.rs` via `djangors_db::Database::connect(&config)`.

Djangors provides CLI commands for database migrations and interactive database shells:

```bash
# Generate migration files based on model metadata changes
dj makemigrations

# Apply migrations to update the database schema
dj migrate

# Open an interactive PostgreSQL shell (uses psql under the hood)
dj dbshell
```

In test suites and custom environments, raw DDL or programmatic migrations set up tables for isolation:

```sql
CREATE TABLE polls_question (
    id BIGSERIAL PRIMARY KEY,
    question_text VARCHAR(200) NOT NULL,
    pub_date TIMESTAMPTZ NOT NULL
);

CREATE TABLE polls_choice (
    id BIGSERIAL PRIMARY KEY,
    question BIGINT NOT NULL REFERENCES polls_question(id) ON DELETE CASCADE,
    choice_text VARCHAR(200) NOT NULL,
    votes INTEGER NOT NULL DEFAULT 0
);
```

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **Rust Structs, Not Python Classes**: Djangors models rely on `#[derive(Model)]` rather than inheriting from a base `models.Model` Python class.
> - **Typed Foreign Keys**: Foreign keys use `ForeignKey<T>` structs rather than direct field assignment.
> - **Inherent Methods**: Business logic methods like `was_published_recently()` are standard Rust `impl` blocks on the struct.
> - **Schema Migration Tooling**: `dj makemigrations` and `dj migrate` execute schema migrations, while integration tests directly run SQL DDL statements (`CREATE TABLE`) for predictable setup.

---

## Verification & Execution

To check model definitions and schema integrity, run `dj check`:

```bash
dj check
```

You can also run the interactive Rust REPL shell (requires `evcxr` via `cargo install evcxr_repl`):

```bash
dj shell
```

Inside the REPL, you can load your project crate directly:
```rust
:dep my_app = { path = "." }
use my_app::models::*;
```
