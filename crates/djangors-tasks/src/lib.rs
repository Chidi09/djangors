#![deny(missing_docs)]
//! Background task queue, `#[task]` attribute macro, and worker loop for Djangors.

extern crate self as djangors_tasks;

use djangors_macros::Model;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

fn parse_schedule(expression: &str) -> Result<cron::Schedule, String> {
    if expression.split_whitespace().count() != 5 {
        return Err("cron expression must contain exactly five fields".into());
    }
    cron::Schedule::from_str(&format!("0 {expression}")).map_err(|e| e.to_string())
}

pub use djangors_macros::task;
pub use inventory;
pub use serde_json;

/// Boxed pinned sendable future returned by background task handlers.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Error types for task queue operations and task execution.
#[derive(thiserror::Error, Debug)]
pub enum TaskError {
    /// A database operation failed.
    #[error("Database error: {0}")]
    Db(#[from] djangors_db::DbError),

    /// An ORM operation failed.
    #[error("ORM error: {0}")]
    Orm(#[from] djangors_orm::OrmError),

    /// A JSON serialization or deserialization error occurred.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Task payload deserialization failed.
    #[error("Task payload deserialization failed: {0}")]
    PayloadDeserialization(String),

    /// Task serialization failed.
    #[error("Task serialization failed: {0}")]
    Serialization(String),

    /// Requested task handler was not found in the inventory registry.
    #[error("Task handler not found: {0}")]
    TaskNotFound(String),

    /// Task execution encountered an error.
    #[error("Task execution failed: {0}")]
    TaskExecution(String),

    /// Task execution panicked.
    #[error("Task panicked: {0}")]
    TaskPanicked(String),
}

/// Registration record for background tasks registered via the `#[task]` macro.
pub struct TaskRegistration {
    /// Unique identifier name of the task.
    pub name: &'static str,
    /// Function pointer executing the task handler logic given JSON payload value.
    pub handler: fn(serde_json::Value) -> BoxFuture<'static, Result<(), TaskError>>,
}

inventory::collect!(TaskRegistration);

/// Database model representing a task in the queue table.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_tasks", table_name = "djangors_task_queue")]
pub struct QueuedTask {
    /// Primary key task identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Name of the registered task handler.
    #[djangors(max_length = 255)]
    pub task_name: String,
    /// JSON payload parameter string.
    pub payload: String,
    /// Status string (`"pending"`, `"running"`, `"completed"`, `"failed"`).
    #[djangors(max_length = 50)]
    pub status: String,
    /// Number of execution attempts attempted so far.
    pub attempts: i32,
    /// Maximum allowed execution attempts before marking failed.
    pub max_attempts: i32,
    /// Record creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Earliest scheduled execution timestamp.
    pub scheduled_at: chrono::DateTime<chrono::Utc>,
    /// Last error or panic message if execution failed.
    pub error_message: Option<String>,
}

/// Database model representing a task scheduled on a recurring cron expression.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_tasks", table_name = "djangors_recurring_task")]
pub struct RecurringTask {
    /// Primary key recurring task identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Name of the registered task handler.
    pub task_name: String,
    /// JSON payload parameter string.
    pub payload: String,
    /// Standard five-field cron expression.
    pub cron_expr: String,
    /// Next scheduled execution timestamp.
    pub next_run_at: chrono::DateTime<chrono::Utc>,
    /// Previous scheduled execution timestamp.
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this recurring task is active.
    pub enabled: bool,
}

/// Creates the database table `djangors_task_queue` if it does not already exist.
pub async fn create_task_table(db: &djangors_db::Database) -> Result<(), djangors_db::DbError> {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS djangors_task_queue (
        id BIGSERIAL PRIMARY KEY,
        task_name VARCHAR(255) NOT NULL,
        payload TEXT NOT NULL,
        status VARCHAR(50) NOT NULL,
        attempts INT NOT NULL DEFAULT 0,
        max_attempts INT NOT NULL DEFAULT 3,
        created_at TIMESTAMPTZ NOT NULL,
        scheduled_at TIMESTAMPTZ NOT NULL,
        error_message TEXT
    );
    "#;
    sqlx::query(sql)
        .execute(db.pool())
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

/// Creates the database table for recurring tasks if it does not already exist.
pub async fn create_recurring_task_table(
    db: &djangors_db::Database,
) -> Result<(), djangors_db::DbError> {
    let sql = r#"CREATE TABLE IF NOT EXISTS djangors_recurring_task (
        id BIGSERIAL PRIMARY KEY,
        task_name TEXT NOT NULL,
        payload TEXT NOT NULL,
        cron_expr TEXT NOT NULL,
        next_run_at TIMESTAMPTZ NOT NULL,
        last_run_at TIMESTAMPTZ,
        enabled BOOLEAN NOT NULL DEFAULT TRUE
    );"#;
    sqlx::query(sql)
        .execute(db.pool())
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

/// Registers a recurring task after immediately validating its standard cron expression.
pub async fn register_recurring(
    db: &djangors_db::Database,
    task_name: &str,
    payload: &impl serde::Serialize,
    cron_expr: &str,
) -> Result<i64, TaskError> {
    let schedule = parse_schedule(cron_expr)
        .map_err(|e| TaskError::Serialization(format!("invalid cron expression: {e}")))?;
    let next_run_at = schedule.upcoming(chrono::Utc).next().ok_or_else(|| {
        TaskError::Serialization("cron expression has no future occurrence".into())
    })?;
    let payload =
        serde_json::to_string(payload).map_err(|e| TaskError::Serialization(e.to_string()))?;
    let row = sqlx::query("INSERT INTO djangors_recurring_task (task_name, payload, cron_expr, next_run_at, last_run_at, enabled) VALUES ($1, $2, $3, $4, NULL, TRUE) RETURNING id")
        .bind(task_name).bind(payload).bind(cron_expr).bind(next_run_at)
        .fetch_one(db.pool()).await.map_err(djangors_db::DbError::QueryFailed)?;
    Ok(sqlx::Row::get(&row, "id"))
}

/// Enqueues due recurring tasks atomically while holding row locks, returning the enqueue count.
pub async fn tick_recurring_tasks(db: &djangors_db::Database) -> Result<usize, TaskError> {
    db.transaction(|tx| Box::pin(async move {
        let now = chrono::Utc::now();
        // A tick spans several statements (cron evaluation, enqueue, and
        // schedule advancement).  Serialize that claim-and-advance sequence
        // at the database level so READ COMMITTED cannot let another tick
        // observe the same due row between those statements.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('djangors.tick_recurring_tasks', 0))")
            .execute(&mut *tx)
            .await
            .map_err(djangors_db::DbError::QueryFailed)?;
        let rows = sqlx::query("SELECT id, task_name, payload, cron_expr, next_run_at FROM djangors_recurring_task WHERE enabled = true AND next_run_at <= $1 ORDER BY next_run_at, id FOR UPDATE SKIP LOCKED")
            .bind(now).fetch_all(&mut *tx).await.map_err(djangors_db::DbError::QueryFailed)?;
        let mut count = 0;
        for row in rows {
            let id: i64 = sqlx::Row::get(&row, "id");
            let task_name: String = sqlx::Row::get(&row, "task_name");
            let payload: String = sqlx::Row::get(&row, "payload");
            let expr: String = sqlx::Row::get(&row, "cron_expr");
            let previous: chrono::DateTime<chrono::Utc> = sqlx::Row::get(&row, "next_run_at");
            let schedule = parse_schedule(&expr).map_err(|e| djangors_db::DbError::QueryFailed(sqlx::Error::Protocol(e)))?;
            let next = schedule.after(&previous).next().ok_or_else(|| djangors_db::DbError::QueryFailed(sqlx::Error::Protocol("cron has no next occurrence".into())))?;
            sqlx::query("INSERT INTO djangors_task_queue (task_name, payload, status, attempts, max_attempts, created_at, scheduled_at, error_message) VALUES ($1, $2, 'pending', 0, 3, $3, $3, NULL)")
                .bind(task_name).bind(payload).bind(previous).execute(&mut *tx).await.map_err(djangors_db::DbError::QueryFailed)?;
            sqlx::query("UPDATE djangors_recurring_task SET last_run_at = $1, next_run_at = $2 WHERE id = $3")
                .bind(previous).bind(next).bind(id).execute(&mut *tx).await.map_err(djangors_db::DbError::QueryFailed)?;
            count += 1;
        }
        Ok::<usize, djangors_db::DbError>(count)
    })).await.map_err(TaskError::Db)
}

/// Enqueues a task for immediate execution.
pub async fn enqueue(
    db: &djangors_db::Database,
    task_name: &str,
    payload: &impl serde::Serialize,
) -> Result<i64, TaskError> {
    enqueue_scheduled(db, task_name, payload, chrono::Utc::now()).await
}

/// Enqueues a task with a specific scheduled_at time.
pub async fn enqueue_scheduled(
    db: &djangors_db::Database,
    task_name: &str,
    payload: &impl serde::Serialize,
    scheduled_at: chrono::DateTime<chrono::Utc>,
) -> Result<i64, TaskError> {
    let payload_str =
        serde_json::to_string(payload).map_err(|e| TaskError::Serialization(e.to_string()))?;
    let task = QueuedTask {
        id: 0,
        task_name: task_name.to_string(),
        payload: payload_str,
        status: "pending".to_string(),
        attempts: 0,
        max_attempts: 3,
        created_at: chrono::Utc::now(),
        scheduled_at,
        error_message: None,
    };
    let saved = task.save(db).await?;
    Ok(saved.id)
}

/// Atomically claims the next pending, due task using `SELECT ... FOR UPDATE SKIP LOCKED`
/// within a single transaction.
pub async fn claim_next_task(db: &djangors_db::Database) -> Result<Option<QueuedTask>, TaskError> {
    let claimed = db
        .transaction(|tx| {
            Box::pin(async move {
                let now = chrono::Utc::now();
                let row = sqlx::query(
                    "SELECT id, task_name, payload, status, attempts, max_attempts, created_at, scheduled_at, error_message \
                     FROM djangors_task_queue \
                     WHERE status = 'pending' AND scheduled_at <= $1 \
                     ORDER BY scheduled_at ASC, id ASC \
                     FOR UPDATE SKIP LOCKED \
                     LIMIT 1"
                )
                .bind(now)
                .fetch_optional(&mut *tx)
                .await
                .map_err(djangors_db::DbError::QueryFailed)?;

                let row = match row {
                    Some(r) => r,
                    None => return Ok::<Option<QueuedTask>, djangors_db::DbError>(None),
                };

                let mut task = QueuedTask::from_row(&row)
                    .map_err(|e| djangors_db::DbError::QueryFailed(sqlx::Error::Decode(Box::new(e))))?;

                task.status = "running".to_string();
                task.attempts += 1;

                sqlx::query(
                    "UPDATE djangors_task_queue SET status = 'running', attempts = $1 WHERE id = $2"
                )
                .bind(task.attempts)
                .bind(task.id)
                .execute(&mut *tx)
                .await
                .map_err(djangors_db::DbError::QueryFailed)?;

                Ok(Some(task))
            })
        })
        .await?;

    Ok(claimed)
}

async fn mark_task_completed(db: &djangors_db::Database, task_id: i64) -> Result<(), TaskError> {
    sqlx::query(
        "UPDATE djangors_task_queue SET status = 'completed', error_message = NULL WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

async fn mark_task_failed(
    db: &djangors_db::Database,
    task_id: i64,
    error_message: &str,
    attempts: i32,
    max_attempts: i32,
) -> Result<(), TaskError> {
    let next_status = if attempts < max_attempts {
        "pending"
    } else {
        "failed"
    };
    sqlx::query("UPDATE djangors_task_queue SET status = $1, error_message = $2 WHERE id = $3")
        .bind(next_status)
        .bind(error_message)
        .bind(task_id)
        .execute(db.pool())
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

/// Worker that claims and executes background tasks from the queue.
pub struct Worker {
    db: djangors_db::Database,
    poll_interval: std::time::Duration,
    recurring_tick_interval: Option<std::time::Duration>,
}

impl Worker {
    /// Creates a new `Worker` backed by database `db`.
    pub fn new(db: djangors_db::Database) -> Self {
        Self {
            db,
            poll_interval: std::time::Duration::from_secs(1),
            recurring_tick_interval: None,
        }
    }

    /// Configures how often due recurring tasks are enqueued.
    pub fn with_recurring_tick_interval(mut self, interval: std::time::Duration) -> Self {
        self.recurring_tick_interval = Some(interval);
        self
    }

    /// Configures the polling interval for checking due tasks when idle.
    pub fn with_poll_interval(mut self, interval: std::time::Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Runs the worker loop indefinitely, processing tasks as they become due.
    pub async fn run(&self) -> ! {
        let mut last_recurring_tick = tokio::time::Instant::now();
        loop {
            if let Some(interval) = self.recurring_tick_interval {
                if last_recurring_tick.elapsed() >= interval {
                    let _ = tick_recurring_tasks(&self.db).await;
                    last_recurring_tick = tokio::time::Instant::now();
                }
            }
            match self.run_once().await {
                Ok(true) => {}
                Ok(false) => {
                    tokio::time::sleep(self.poll_interval).await;
                }
                Err(_) => {
                    tokio::time::sleep(self.poll_interval).await;
                }
            }
        }
    }

    /// Claims and attempts to execute a single task from the queue.
    pub async fn run_once(&self) -> Result<bool, TaskError> {
        let task = match claim_next_task(&self.db).await? {
            Some(t) => t,
            None => return Ok(false),
        };

        let handler = inventory::iter::<TaskRegistration>()
            .find(|reg| reg.name == task.task_name)
            .map(|reg| reg.handler);

        let handler = match handler {
            Some(h) => h,
            None => {
                let err_msg = format!("Task handler '{}' not found in registry", task.task_name);
                mark_task_failed(
                    &self.db,
                    task.id,
                    &err_msg,
                    task.attempts,
                    task.max_attempts,
                )
                .await?;
                return Ok(true);
            }
        };

        let payload_json: serde_json::Value = match serde_json::from_str(&task.payload) {
            Ok(v) => v,
            Err(e) => {
                let err_msg = format!("Failed to parse payload JSON: {}", e);
                mark_task_failed(
                    &self.db,
                    task.id,
                    &err_msg,
                    task.attempts,
                    task.max_attempts,
                )
                .await?;
                return Ok(true);
            }
        };

        let join_handle = tokio::spawn(async move { handler(payload_json).await });

        match join_handle.await {
            Ok(Ok(())) => {
                mark_task_completed(&self.db, task.id).await?;
            }
            Ok(Err(task_err)) => {
                let err_msg = task_err.to_string();
                mark_task_failed(
                    &self.db,
                    task.id,
                    &err_msg,
                    task.attempts,
                    task.max_attempts,
                )
                .await?;
            }
            Err(join_err) => {
                let err_msg = if join_err.is_panic() {
                    let payload = join_err.into_panic();
                    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic message".to_string()
                    };
                    format!("Task panicked: {}", msg)
                } else {
                    format!("Task execution failed: {}", join_err)
                };
                mark_task_failed(
                    &self.db,
                    task.id,
                    &err_msg,
                    task.attempts,
                    task.max_attempts,
                )
                .await?;
            }
        }

        Ok(true)
    }
}

/// Registers the `QueuedTask` model with an admin site instance.
/// Registers `QueuedTask` with the given admin site for real add/change/delete/changelist
/// views. `status` is a plain text field (not a boolean), so it can't be used as a
/// `list_filter` entry yet — this ORM's `list_filter` only supports Boolean fields today
/// (choices-based filtering is a separate, not-yet-built feature, tracked in
/// `docs/design/phase-5-roadmap.md`'s deferred-items ledger). `search_fields` gives staff a
/// working way to find tasks by name/status in the meantime.
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register_with::<QueuedTask>(djangors_admin::ModelAdminConfig {
        list_display: Some(&["id", "task_name", "status", "attempts", "scheduled_at"]),
        search_fields: Some(&["task_name", "status"]),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    static TEST_TASK_EXECUTED: AtomicBool = AtomicBool::new(false);

    // All DB-touching tests below share the single real `djangors_test` database and a
    // fixed `djangors_task_queue` table name (djangors-test's TestDatabase does not yet
    // provide per-test isolation - see docs/design's TestDatabase rollback/fixtures item).
    // `cargo test` runs `#[tokio::test]` functions concurrently by default, so without this
    // lock two tests racing on the same table produce nondeterministic row counts. Every
    // test that touches `djangors_task_queue` must acquire this guard before doing so.
    static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[derive(serde::Serialize, serde::Deserialize)]
    struct SamplePayload {
        message: String,
    }

    #[task]
    async fn sample_task_fn(payload: SamplePayload) -> Result<(), TaskError> {
        if payload.message == "hello_tasks" {
            TEST_TASK_EXECUTED.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    #[task(name = "custom_registered_task")]
    async fn custom_task_fn(payload: SamplePayload) -> Result<(), TaskError> {
        if payload.message == "custom" {
            TEST_TASK_EXECUTED.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    #[task]
    async fn failing_task_fn(_payload: SamplePayload) -> Result<(), TaskError> {
        Err(TaskError::TaskExecution("simulated error".to_string()))
    }

    #[task]
    async fn panicking_task_fn(_payload: SamplePayload) -> Result<(), TaskError> {
        panic!("simulated task panic!");
    }

    #[test]
    fn test_task_macro_discovery_and_execution() {
        let mut found = false;
        for reg in inventory::iter::<TaskRegistration>() {
            if reg.name == "sample_task_fn" {
                found = true;
                TEST_TASK_EXECUTED.store(false, Ordering::SeqCst);
                let payload = serde_json::json!({ "message": "hello_tasks" });
                let fut = (reg.handler)(payload);
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(fut)
                    .unwrap();
                assert!(TEST_TASK_EXECUTED.load(Ordering::SeqCst));
            }
        }
        assert!(found, "sample_task_fn should be discovered via inventory");
    }

    #[test]
    fn test_custom_name_macro() {
        let found =
            inventory::iter::<TaskRegistration>().any(|reg| reg.name == "custom_registered_task");
        assert!(
            found,
            "custom_registered_task should be registered with explicit name"
        );
    }

    #[test]
    fn test_admin_registration() {
        let site = djangors_admin::AdminSite::new();
        register_admin(&site);
    }

    #[tokio::test]
    async fn test_concurrent_claim_skip_locked() -> Result<(), Box<dyn std::error::Error>> {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()), // Skip if no test DB available
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_task_table(db).await?;

        // Clear queue
        sqlx::query("DELETE FROM djangors_task_queue")
            .execute(db.pool())
            .await?;

        // Enqueue ONE task
        let task_id = enqueue(
            db,
            "sample_task_fn",
            &SamplePayload {
                message: "hello_tasks".into(),
            },
        )
        .await?;

        // Spawn two concurrent claims against the same row
        let db1 = db.clone();
        let db2 = db.clone();

        let handle1 = tokio::spawn(async move { claim_next_task(&db1).await });
        let handle2 = tokio::spawn(async move { claim_next_task(&db2).await });

        let (res1, res2) = tokio::join!(handle1, handle2);
        let claim1 = res1??;
        let claim2 = res2??;

        // Exactly one should get Some(task_id), the other None
        let claimed_count =
            (if claim1.is_some() { 1 } else { 0 }) + (if claim2.is_some() { 1 } else { 0 });
        assert_eq!(
            claimed_count, 1,
            "Exactly one worker should claim the single pending task"
        );

        let claimed_task = claim1.or(claim2).unwrap();
        assert_eq!(claimed_task.id, task_id);
        assert_eq!(claimed_task.status, "running");
        assert_eq!(claimed_task.attempts, 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_worker_run_once_success() -> Result<(), Box<dyn std::error::Error>> {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()),
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_task_table(db).await?;

        sqlx::query("DELETE FROM djangors_task_queue")
            .execute(db.pool())
            .await?;

        TEST_TASK_EXECUTED.store(false, Ordering::SeqCst);
        let task_id = enqueue(
            db,
            "sample_task_fn",
            &SamplePayload {
                message: "hello_tasks".into(),
            },
        )
        .await?;

        let worker = Worker::new(db.clone());
        let claimed = worker.run_once().await?;
        assert!(claimed);

        assert!(TEST_TASK_EXECUTED.load(Ordering::SeqCst));

        // Check task status in DB
        let row = sqlx::query("SELECT status, attempts FROM djangors_task_queue WHERE id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await?;

        let status: String = sqlx::Row::get(&row, "status");
        let attempts: i32 = sqlx::Row::get(&row, "attempts");

        assert_eq!(status, "completed");
        assert_eq!(attempts, 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_worker_retry_under_and_at_max_attempts() -> Result<(), Box<dyn std::error::Error>>
    {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()),
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_task_table(db).await?;

        sqlx::query("DELETE FROM djangors_task_queue")
            .execute(db.pool())
            .await?;

        let task_id = enqueue(
            db,
            "failing_task_fn",
            &SamplePayload {
                message: "fail".into(),
            },
        )
        .await?;

        let worker = Worker::new(db.clone());

        // 1st attempt: attempts = 0 -> 1. max_attempts = 3. 1 < 3 => status requeued to 'pending'
        let claimed = worker.run_once().await?;
        assert!(claimed);

        let row1 = sqlx::query(
            "SELECT status, attempts, error_message FROM djangors_task_queue WHERE id = $1",
        )
        .bind(task_id)
        .fetch_one(db.pool())
        .await?;
        let status1: String = sqlx::Row::get(&row1, "status");
        let attempts1: i32 = sqlx::Row::get(&row1, "attempts");
        let err1: Option<String> = sqlx::Row::get(&row1, "error_message");

        assert_eq!(status1, "pending");
        assert_eq!(attempts1, 1);
        assert!(err1.unwrap().contains("simulated error"));

        // 2nd attempt: attempts = 1 -> 2. 2 < 3 => status requeued to 'pending'
        let claimed2 = worker.run_once().await?;
        assert!(claimed2);

        let row2 = sqlx::query("SELECT status, attempts FROM djangors_task_queue WHERE id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await?;
        let status2: String = sqlx::Row::get(&row2, "status");
        let attempts2: i32 = sqlx::Row::get(&row2, "attempts");
        assert_eq!(status2, "pending");
        assert_eq!(attempts2, 2);

        // 3rd attempt: attempts = 2 -> 3. 3 >= 3 => status set to 'failed' permanently
        let claimed3 = worker.run_once().await?;
        assert!(claimed3);

        let row3 = sqlx::query("SELECT status, attempts FROM djangors_task_queue WHERE id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await?;
        let status3: String = sqlx::Row::get(&row3, "status");
        let attempts3: i32 = sqlx::Row::get(&row3, "attempts");
        assert_eq!(status3, "failed");
        assert_eq!(attempts3, 3);

        // 4th run_once should claim nothing because status is now 'failed'
        let claimed4 = worker.run_once().await?;
        assert!(!claimed4);

        Ok(())
    }

    #[tokio::test]
    async fn test_worker_panic_isolation() -> Result<(), Box<dyn std::error::Error>> {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()),
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_task_table(db).await?;

        sqlx::query("DELETE FROM djangors_task_queue")
            .execute(db.pool())
            .await?;

        let task_id = enqueue(
            db,
            "panicking_task_fn",
            &SamplePayload {
                message: "panic".into(),
            },
        )
        .await?;

        let worker = Worker::new(db.clone());

        // Worker should NOT panic or crash when running panicking task
        let claimed = worker.run_once().await?;
        assert!(claimed);

        let row =
            sqlx::query("SELECT status, error_message FROM djangors_task_queue WHERE id = $1")
                .bind(task_id)
                .fetch_one(db.pool())
                .await?;
        let status: String = sqlx::Row::get(&row, "status");
        let err_msg: Option<String> = sqlx::Row::get(&row, "error_message");

        assert_eq!(status, "pending");
        assert!(err_msg
            .unwrap()
            .contains("Task panicked: simulated task panic!"));

        Ok(())
    }

    #[tokio::test]
    async fn test_register_recurring_rejects_invalid_cron_expression(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()),
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_recurring_task_table(db).await?;
        sqlx::query("DELETE FROM djangors_recurring_task")
            .execute(db.pool())
            .await?;

        let bad_syntax = register_recurring(
            db,
            "sample_task_fn",
            &SamplePayload {
                message: "x".into(),
            },
            "not a cron expression",
        )
        .await;
        assert!(matches!(bad_syntax, Err(TaskError::Serialization(_))));

        // Wrong field count (4 instead of the required standard 5).
        let wrong_fields = register_recurring(
            db,
            "sample_task_fn",
            &SamplePayload {
                message: "x".into(),
            },
            "* * * *",
        )
        .await;
        assert!(matches!(wrong_fields, Err(TaskError::Serialization(_))));

        Ok(())
    }

    #[tokio::test]
    async fn test_disabled_recurring_task_never_enqueues() -> Result<(), Box<dyn std::error::Error>>
    {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()),
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_recurring_task_table(db).await?;
        sqlx::query("DELETE FROM djangors_recurring_task")
            .execute(db.pool())
            .await?;
        create_task_table(db).await?;

        let name = "sample_task_fn_disabled_test";
        let recurring_id = register_recurring(
            db,
            name,
            &SamplePayload {
                message: "disabled".into(),
            },
            "*/5 * * * *",
        )
        .await?;

        // Mark it disabled and force it overdue.
        sqlx::query(
            "UPDATE djangors_recurring_task SET enabled = false, next_run_at = $1 WHERE id = $2",
        )
        .bind(chrono::Utc::now() - chrono::Duration::minutes(20))
        .bind(recurring_id)
        .execute(db.pool())
        .await?;

        let enqueued = tick_recurring_tasks(db).await?;
        assert_eq!(
            enqueued, 0,
            "a disabled recurring task must never enqueue, even overdue"
        );

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM djangors_task_queue WHERE task_name = $1")
                .bind(name)
                .fetch_one(db.pool())
                .await?;
        assert_eq!(count, 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_recurring_task_advances_from_previous_scheduled_time_not_now(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()),
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_recurring_task_table(db).await?;
        sqlx::query("DELETE FROM djangors_recurring_task")
            .execute(db.pool())
            .await?;
        create_task_table(db).await?;

        let name = "sample_task_fn_advance_test";
        let recurring_id = register_recurring(
            db,
            name,
            &SamplePayload {
                message: "advance".into(),
            },
            "*/5 * * * *",
        )
        .await?;

        // Force it 20 minutes overdue (well past several missed 5-minute occurrences).
        let overdue_since = chrono::Utc::now() - chrono::Duration::minutes(20);
        sqlx::query("UPDATE djangors_recurring_task SET next_run_at = $1 WHERE id = $2")
            .bind(overdue_since)
            .bind(recurring_id)
            .execute(db.pool())
            .await?;

        let enqueued = tick_recurring_tasks(db).await?;
        assert_eq!(enqueued, 1);

        let new_next_run_at: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT next_run_at FROM djangors_recurring_task WHERE id = $1")
                .bind(recurring_id)
                .fetch_one(db.pool())
                .await?;

        // If next_run_at had incorrectly been advanced from `now()` instead of the recorded
        // (overdue) `next_run_at`, the new value would land in the future, close to now() + up
        // to 5 minutes. Advancing correctly from the overdue timestamp instead catches up from
        // where it left off, landing well in the past relative to now.
        assert!(
            new_next_run_at < chrono::Utc::now(),
            "next_run_at must advance from the previous overdue schedule, not from now() \
             (got {new_next_run_at}, which is not in the past as expected)"
        );
        assert!(new_next_run_at > overdue_since);

        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_tick_recurring_tasks_dual_claim_race() -> Result<(), Box<dyn std::error::Error>> {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()),
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_recurring_task_table(db).await?;
        sqlx::query("DELETE FROM djangors_recurring_task")
            .execute(db.pool())
            .await?;
        create_task_table(db).await?;
        sqlx::query("DELETE FROM djangors_task_queue WHERE task_name = 'sample_task_fn_race_test'")
            .execute(db.pool())
            .await?;

        let name = "sample_task_fn_race_test";
        let recurring_id = register_recurring(
            db,
            name,
            &SamplePayload {
                message: "race".into(),
            },
            "*/5 * * * *",
        )
        .await?;
        sqlx::query("UPDATE djangors_recurring_task SET next_run_at = $1 WHERE id = $2")
            .bind(chrono::Utc::now() - chrono::Duration::minutes(1))
            .bind(recurring_id)
            .execute(db.pool())
            .await?;

        let db1 = db.clone();
        let db2 = db.clone();
        let handle1 = tokio::spawn(async move { tick_recurring_tasks(&db1).await });
        let handle2 = tokio::spawn(async move { tick_recurring_tasks(&db2).await });
        let (res1, res2) = tokio::join!(handle1, handle2);
        let enqueued1 = res1??;
        let enqueued2 = res2??;

        assert_eq!(
            enqueued1 + enqueued2,
            1,
            "exactly one of the two concurrent ticks must enqueue the single due row, never zero or two"
        );

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM djangors_task_queue WHERE task_name = $1")
                .bind(name)
                .fetch_one(db.pool())
                .await?;
        assert_eq!(count, 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_recurring_task_end_to_end_enqueue_and_execute(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let test_db = match djangors_test::TestDatabase::connect().await {
            Ok(db) => db,
            Err(_) => return Ok(()),
        };
        let _guard = TEST_DB_LOCK.lock().await;
        let db = test_db.database();
        create_recurring_task_table(db).await?;
        sqlx::query("DELETE FROM djangors_recurring_task")
            .execute(db.pool())
            .await?;
        create_task_table(db).await?;
        sqlx::query("DELETE FROM djangors_task_queue")
            .execute(db.pool())
            .await?;

        TEST_TASK_EXECUTED.store(false, Ordering::SeqCst);

        let recurring_id = register_recurring(
            db,
            "sample_task_fn",
            &SamplePayload {
                message: "hello_tasks".into(),
            },
            "*/5 * * * *",
        )
        .await?;
        sqlx::query("UPDATE djangors_recurring_task SET next_run_at = $1 WHERE id = $2")
            .bind(chrono::Utc::now() - chrono::Duration::minutes(1))
            .bind(recurring_id)
            .execute(db.pool())
            .await?;

        let enqueued = tick_recurring_tasks(db).await?;
        assert_eq!(enqueued, 1);

        let queued_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM djangors_task_queue WHERE task_name = 'sample_task_fn'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(queued_count, 1);

        // Now prove the two systems compose: the existing Worker/claim_next_task machinery
        // actually picks up and executes the task the recurring tick enqueued.
        let worker = Worker::new(db.clone());
        let claimed = worker.run_once().await?;
        assert!(claimed);
        assert!(
            TEST_TASK_EXECUTED.load(Ordering::SeqCst),
            "the task enqueued by tick_recurring_tasks must actually execute via the normal worker"
        );

        Ok(())
    }
}
