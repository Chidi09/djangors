# How to Stream Server-Sent Events (SSE)

## Problem
You want to stream real-time updates or events to web browsers over Server-Sent Events (SSE).

## Solution
Create a streaming response using `StreamingResponse::sse(stream)` or register an SSE endpoint on a `Router` using `.sse(path, handler)`.

## Code Example

```rust
use async_stream::stream;
use tokio_stream::StreamExt;
use djangors_core::{Request, PathParams, DjangorsError, Router, DjangorsSettings, Djangors};
use djangors_core::sse::StreamingResponse;

// 1. Define SSE streaming handler
async fn live_events_handler(
    _req: Request,
    _params: PathParams,
) -> Result<StreamingResponse, DjangorsError> {
    // Generate an async stream of String events
    let event_stream = stream! {
        for i in 1..=5 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            yield format!("Event #{}", i);
        }
    };

    Ok(StreamingResponse::sse(event_stream))
}

// 2. Register endpoint on Router
fn build_sse_router() -> Router {
    Router::new().sse("/events", live_events_handler)
}

// 3. Start server
#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    let settings = DjangorsSettings::default();
    let app = Djangors::new(settings, build_sse_router());
    
    // Note: Must use standard serve()/run() loop
    app.run().await
}
```

### ⚠️ Real v1 Limitations & Constraints
- **Stream Format**: `StreamingResponse::sse` automatically formats each yielded string as an SSE data frame (`data: {item}\n\n`) and applies headers: `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `Connection: keep-alive`.
- **Server Execution Path**: Streaming responses produce dynamic `BoxBody<Bytes, Infallible>` chunked streams. In v1, SSE streaming endpoints only work when served via the standard `serve()` / `run()` server loop, not custom `run_service()` middleware stacks (which expect fixed `Full<Bytes>` response bodies).
