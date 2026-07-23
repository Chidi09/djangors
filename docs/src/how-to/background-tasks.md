# How to Execute Background Tasks

## Problem
You want to offload long-running work (such as sending emails, processing uploads, or generating reports) to an asynchronous background task queue.

## Solution
Annotate an async task handler with `#[task]` from `djangors_tasks`. Enqueue tasks into the database queue with `enqueue()`, and run the background task consumer using `Worker`.

## Code Example

```rust
use serde::{Deserialize, Serialize};
use djangors_tasks::{task, enqueue, Worker, TaskError};
use djangors_db::Database;

// 1. Define task payload struct deriving Serde
#[derive(Serialize, Deserialize, Debug)]
pub struct SendReportPayload {
    pub user_id: i64,
    pub report_name: String,
}

// 2. Define background task handler function
#[task]
pub async fn send_report_task(payload: SendReportPayload) -> Result<(), TaskError> {
    println!("Processing report '{}' for user ID {}", payload.report_name, payload.user_id);
    // Task execution logic here...
    Ok(())
}

// 3. Enqueue task from a request handler or service
pub async fn trigger_report_generation(db: &Database, user_id: i64) -> Result<i64, TaskError> {
    let payload = SendReportPayload {
        user_id,
        report_name: "Monthly Audit".to_string(),
    };
    
    // Enqueue task for immediate background execution
    let task_id = enqueue(db, "send_report_task", &payload).await?;
    Ok(task_id)
}

// 4. Run the worker loop in a background thread or service
pub async fn start_background_worker(db: Database) {
    let worker = Worker::new(db).with_poll_interval(std::time::Duration::from_secs(2));
    worker.run().await; // Loops continuously processing claimed tasks
}
```
