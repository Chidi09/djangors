# Signals

Djangors features a pub-sub (publish-subscribe) signals subsystem allowing decoupled parts of your application to react to framework lifecycle events.

---

## Request Signals

Request signals are defined in `djangors-core` and track the processing lifecycle of HTTP requests.

### Built-in Request Signals

Djangors exposes two global request-level signals:

* **`REQUEST_STARTED`**: Fires when an HTTP request begins processing. Fired with a `RequestStarted` payload.
* **`REQUEST_FINISHED`**: Fires when an HTTP request completes (either successfully or with an error). Fired with a `RequestFinished` payload.

### Subscribing to Request Signals

To connect to a request signal, call `.connect()` and pass an async callback closure:

```rust,illustrative
use djangors::signals::{REQUEST_STARTED, REQUEST_FINISHED, RequestStarted, RequestFinished};

// Connect to REQUEST_STARTED
REQUEST_STARTED.connect(|payload: RequestStarted| async move {
    println!("HTTP request started: {} {}", payload.method, payload.path);
});

// Connect to REQUEST_FINISHED
REQUEST_FINISHED.connect(|payload: RequestFinished| async move {
    println!(
        "HTTP request finished: {} {} with status {}",
        payload.method, payload.path, payload.status
    );
});
```

---

## ORM Model Lifecycle Signals

Models deriving `#[derive(Model)]` automatically implement and expose four model-level lifecycle signals. These signals are static methods on your model struct:

* **`pre_save_signal()`**: Fires immediately before a model is saved or updated in the database (before the INSERT/UPDATE SQL query executes).
* **`post_save_signal()`**: Fires immediately after a model is saved or updated in the database.
* **`pre_delete_signal()`**: Fires immediately before a model row is deleted from the database.
* **`post_delete_signal()`**: Fires immediately after a model row is deleted from the database.

### The Signal Payload

Model signals carry a `ModelSignalPayload` which represents the model's field values:

```rust,illustrative
pub type ModelSignalPayload = Vec<(&'static str, Value)>;
```

Each element is a `(field_name, field_value)` tuple. This payload structure avoids requiring model types to implement `Clone` or `Sync`.

### Subscribing to Model Signals

To connect to a model's lifecycle signal:

```rust,illustrative
use my_app::models::Question;
use djangors::orm::signals::ModelSignalPayload;

Question::pre_save_signal().connect(|payload: ModelSignalPayload| async move {
    println!("Saving question model...");
    for (field_name, value) in payload {
        println!("  {field_name} = {value:?}");
    }
});
```

---

## Technical Design Details

### Async Execution and Concurrency
When a signal fires:
1. It retrieves all connected subscriber closures.
2. It executes all subscriber futures **concurrently** (spawning each callback inside its own `tokio::spawn` worker).
3. The sender waits for all spawned callbacks to complete.

### Panic Isolation
Signals in Djangors provide strict panic isolation. A panic or unhandled error inside a subscriber callback will **never**:
* Prevent other subscribers from executing.
* Halt the main request handler or transaction.

Instead, the system catches the panic, logs a trace to `stderr` (e.g. `Signal handler panicked: ...`), and safely continues executing.
