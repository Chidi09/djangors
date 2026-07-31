# Schema Migrations

Djangors provides a robust, declarative database schema migration system inspired by Django's migration engine. It tracks changes to your Rust model definitions and generates database-agnostic SQL migrations that can be applied to both PostgreSQL and SQLite.

---

## The Workflow

The typical migration cycle involves two commands:

1. **`dj makemigrations`**: Inspects your current models, compares them with the previous schema state saved in `.schema_snapshot.json`, and generates a new SQL migration file.
2. **`dj migrate`**: Executes any pending migration files against the target database and records their names in the `djangors_migrations` history table.

---

## Migration Files and Markers

Migrations are written as plain SQL files under your project's migrations directory (usually `src/migrations/` or `migrations/`). 

Each file contains custom SQL comment markers to separate the forward (`up`) migration from the backward (`down`) rollback logic.

### Forward and Backward Actions
* **`-- up`**: Denotes the SQL statements to apply when running the migration forward.
* **`-- down`**: Denotes the SQL statements to run when reverting the migration.
* **`-- no-down`**: Explicitly marks a migration as non-reversible. If a rollback is attempted on a migration containing `-- no-down`, Djangors returns a `MigrationError::NonInvertible` error and aborts.

Example migration file (`0001_initial.sql`):
```sql
-- up
CREATE TABLE "polls_question" (
    "id" SERIAL PRIMARY KEY,
    "question_text" VARCHAR(200) NOT NULL,
    "pub_date" TIMESTAMPTZ NOT NULL
);

-- down
DROP TABLE "polls_question";
```

---

## Schema Snapshots

To detect schema changes without connecting to a live database, Djangors stores a JSON representation of your model structures in a file named `.schema_snapshot.json` inside your migrations directory. 

When you run `dj makemigrations`:
1. Djangors parses your compiled Rust model attributes (`#[derive(Model)]`).
2. It loads `.schema_snapshot.json` to see the last recorded schema state.
3. It performs a structural diff between the models and the snapshot.
4. Based on the diff, it generates the appropriate `Operation` steps and writes the SQL migration.
5. It updates `.schema_snapshot.json` with the new schema state.

---

## The `Operation` Enum

Internally, migrations are planned as a sequence of `Operation` steps. The `Operation` enum in `djangors-migrations` represents the supported DDL instructions.

```rust,illustrative
pub enum Operation {
    CreateTable {
        table_name: String,
        columns: Vec<ColumnDef>,
    },
    AddColumn {
        table_name: String,
        column: ColumnDef,
    },
    DropColumn {
        table_name: String,
        column_name: String,
    },
    AlterColumnType {
        table_name: String,
        column_name: String,
        new_sql_type: String,
    },
    RenameColumn {
        table_name: String,
        old_name: String,
        new_name: String,
    },
    DropTable {
        table_name: String,
    },
}
```

### Reversibility

Djangors divides operations into **reversible** (invertible) and **non-reversible** categories. 

Reversible operations can automatically generate the correct SQL for the `-- down` section when running `dj makemigrations`:

| Operation | Reversible? | Reverse Operation |
| --- | --- | --- |
| `CreateTable` | **Yes** | `DropTable` |
| `AddColumn` | **Yes** | `DropColumn` |
| `RenameColumn` | **Yes** | `RenameColumn` (with old and new names swapped) |
| `DropColumn` | **No** | None (data is lost, cannot be automatically reconstructed) |
| `DropTable` | **No** | None (table columns and data are lost) |
| `AlterColumnType` | **No** | None (reverting requires knowledge of the original type) |

If a migration contains a non-reversible operation, Djangors inserts the `-- no-down` marker in the generated SQL file.

---

## Migration History Tracking

Djangors tracks which migrations have been applied using the `djangors_migrations` table inside your database.

The schema of this history table is:
* **`id`**: Auto-incrementing primary key.
* **`name`**: The migration filename without the `.sql` extension (e.g. `0001_initial`).

When running `dj migrate`, the system:
1. Ensures the `djangors_migrations` table exists.
2. Queries the table for all applied migration names.
3. Scans your migrations directory for `.sql` files.
4. Executes the SQL inside the `-- up` section of any files not recorded in `djangors_migrations`.
5. Inserts the names of newly applied migrations into `djangors_migrations` inside a transaction.

---

## Rolling Back Migrations

To undo the most recently applied migrations, use the rollback command or call `rollback_from_dir` programmatically:

```bash
# Roll back the single most recent migration
dj migrate --rollback 1
```

During a rollback, the system:
1. Queries the most recently applied migrations from the `djangors_migrations` table.
2. Reads their corresponding `.sql` files from disk.
3. Asserts that the files do not contain the `-- no-down` marker.
4. Executes the SQL inside the `-- down` section.
5. Deletes the migration records from the `djangors_migrations` table.

---

## Migration Inspection & CLI Options

Djangors provides several commands and flags for inspecting and managing migration states:

### `dj sqlmigrate <app> <migration>`

Renders the SQL statements that a migration would execute without connecting to the database or modifying any state. The SQL output is dialect-correct based on `DATABASE_URL` (e.g. SQLite DDL when targeting SQLite, Postgres DDL when targeting PostgreSQL).

```bash
# Render SQL statements for migration 0001 in app polls
dj sqlmigrate polls 0001
```

### `dj showmigrations`

Lists every migration along with its applied state:

```bash
dj showmigrations
```

Output format:
```text
polls
 [X] 0001_initial
 [ ] 0002_add_field
```

* **`[X]`**: Migration is recorded in the `djangors_migrations` history table.
* **`[ ]`**: Migration exists on disk but has not been applied.
* **`[?]`**: Migration is recorded in the history table but missing from disk.

### `dj migrate --plan`

Prints the ordered list of migrations that would be applied without executing them:

```bash
dj migrate --plan
```

If the database is already up to date, it clearly reports that no migrations need to be applied.

### `dj migrate --fake`

Records migrations as applied in the `djangors_migrations` history table without executing their SQL DDL instructions:

```bash
dj migrate --fake
```

> [!WARNING]
> `--fake` can silently desynchronise the migration history table from the actual database schema. Use with caution when schema changes have been made out of band.

Note that `--fake`, `--plan`, and `--rollback` are mutually exclusive flags.
