# Flash Messages (`djangors-contrib-messages`)

One-shot, session-backed notifications ("Profile updated successfully", "Payment declined") that you queue during one request and render during the next. Views add messages, and the page-rendering call site drains them into template context.

---

## Public API

| Item | Description |
| ---- | ----------- |
| `enum Level` | Severity, mirroring Django message levels: `Debug`, `Info`, `Success`, `Warning`, `Error`. |
| `struct Message` | A queued notification with public fields `level: Level` and `text: String`. Built with `Message::new(level, text)`. |
| `add(session, level, text)` | Pushes a message onto the session queue. |
| `take(session) -> Vec<Message>` | Consumes **and drains** the queue, returning it. A second call returns an empty vector. |
| `add_debug / add_info / add_success / add_warning / add_error` | Shorthands for `add` with a fixed `Level`. |

All of the above take `&djangors_sessions::Session`.

```rust,compile
# fn main() {
use djangors_contrib_messages as messages;
use djangors_contrib_messages::Level;
use djangors_sessions::Session;

let session = Session::new_empty();

messages::add(&session, Level::Info, "Sync started");
messages::add_debug(&session, "loaded 42 rows");
messages::add_success(&session, "Profile updated successfully!");
messages::add_warning(&session, "Storage nearly full");
messages::add_error(&session, "Upstream unreachable");

let queued = messages::take(&session);
assert_eq!(queued.len(), 5);

// take drains the queue; a second call finds nothing.
assert!(messages::take(&session).is_empty());
# }
```

---

## How it Works

Messages are stored under the `_djangors_messages` key in the request's session. The `add` helpers append to whatever is already queued and persist the whole list back; `take` reads the list and removes the key. Because of this drain-on-read, the classic POST-then-GET flow "add in the POST, render in the next GET" stays clean — the message appears exactly once.

```rust,illustrative
use djangors_contrib_messages as messages;
use djangors_core::{DjangorsError, Request, Response, StatusCode};
use djangors_sessions::Session;

async fn save_profile(req: Request) -> Result<Response, DjangorsError> {
    let session = req.require_state::<Session>()?;
    messages::add_success(session, "Profile updated successfully!");
    Ok(Response::redirect("/profile/"))
}

async fn profile(req: Request) -> Result<Response, DjangorsError> {
    let session = req.require_state::<Session>()?;
    let context = messages::take(session); // drains the queue for this request
    Ok(Response::html(
        StatusCode::OK,
        format!("<ul>{}</ul>", context.iter().map(|m| m.text.clone()).collect::<Vec<_>>().join("</li><li>")),
    ))
}
```

Because a handler reads messages before the response is built, `context` is available as template context in the GET request that follows the POST.

### Sessions Middleware

The `add`/`take` helpers operate on the `Session` handle, so requests must flow through the `SessionLayer` middleware (`djangors-sessions`) before they reach these handlers. See [Sessions and CSRF Protection](sessions.md) for how to construct and mount it.

> [!NOTE]
> Djangors sessions are **client-side signed cookies**: queued flash messages round-trip to the browser inside the cookie. Keep messages short — a single `Message` should be a sentence or two, not a rendered HTML fragment — so the cookie stays small and requests against it stay fast.
