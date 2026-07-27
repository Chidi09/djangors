//! Typed, asynchronous signals system for Djangors.
//!
//! This module provides a publish-subscribe pattern allowing decoupled parts of the
//! application to react to framework lifecycle events.
//!
//! Built-in signals:
//! - `REQUEST_STARTED`: Fires right as a request begins matching.
//! - `REQUEST_FINISHED`: Fires when a request completes (either successfully or with an error).
//!
//! Analogous to Django's built-in signals (`request_started`, `request_finished`).
//! Model-lifecycle signals (`pre_save`, `post_save`, etc.) will be added once an ORM is available.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, RwLock};

/// Type alias for the boxed callback future returned by subscribers.
pub type BoxedFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Type alias for signal callback functions.
#[allow(clippy::type_complexity)]
pub type CallbackFn<T> = Arc<dyn Fn(T) -> BoxedFuture + Send + Sync>;

/// A generic signal that async callbacks can subscribe to.
///
/// Signals are typed, meaning they fire with a specific payload type `T`.
/// Callbacks are registered using `connect` and executed concurrently when `send` is called.
/// Panic isolation is guaranteed: a panic in one subscriber callback will not affect other
/// subscribers or the main execution flow.
pub struct Signal<T: Clone + Send + Sync + 'static> {
    subscribers: RwLock<Vec<CallbackFn<T>>>,
}

impl<T: Clone + Send + Sync + 'static> Default for Signal<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + Sync + 'static> Signal<T> {
    /// Create a new, empty signal.
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Register an async callback to run whenever this signal fires.
    ///
    /// Callbacks run concurrently when the signal is fired. Their errors or panics
    /// are isolated and do not affect the sender or other callbacks.
    ///
    /// # Example
    /// ```rust
    /// # use djangors_core::signals::REQUEST_STARTED;
    /// REQUEST_STARTED.connect(|payload| async move {
    ///     println!("Request started: {} {}", payload.method, payload.path);
    /// });
    /// ```
    pub fn connect<F, Fut>(&self, callback: F)
    where
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let boxed_cb = move |payload: T| -> BoxedFuture { Box::pin(callback(payload)) };
        self.subscribers.write().unwrap().push(Arc::new(boxed_cb));
    }

    /// Fire the signal, invoking all connected callbacks with a clone of `payload`.
    ///
    /// Waits for all callbacks to complete concurrently (using `tokio::spawn` and joining the handles)
    /// while isolating each callback's panics so one failing subscriber does not disrupt
    /// the request handling or other subscribers.
    pub async fn send(&self, payload: T) {
        let callbacks = {
            let read_guard = self.subscribers.read().unwrap();
            read_guard.clone()
        };

        let mut join_handles = Vec::new();
        for cb in callbacks {
            let payload_clone = payload.clone();
            let cb_clone = cb.clone();
            let join_handle = tokio::spawn(async move {
                let fut = cb_clone(payload_clone);
                fut.await;
            });
            join_handles.push(join_handle);
        }

        for join_handle in join_handles {
            if let Err(err) = join_handle.await {
                if err.is_panic() {
                    let payload = err.into_panic();
                    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic message".to_string()
                    };
                    eprintln!("Signal handler panicked: {}", msg);
                } else {
                    eprintln!("Signal handler failed to execute: {}", err);
                }
            }
        }
    }
}

/// Payload for the [`REQUEST_STARTED`] signal.
#[derive(Debug, Clone)]
pub struct RequestStarted {
    /// The HTTP method (e.g. "GET", "POST").
    pub method: String,
    /// The URI path of the request (e.g. "/users/123").
    pub path: String,
}

/// Payload for the [`REQUEST_FINISHED`] signal.
#[derive(Debug, Clone)]
pub struct RequestFinished {
    /// The HTTP method (e.g. "GET", "POST").
    pub method: String,
    /// The URI path of the request (e.g. "/users/123").
    pub path: String,
    /// The resulting HTTP status code of the response (e.g. 200, 404).
    pub status: u16,
}

/// Global signal fired when an HTTP request begins processing.
pub static REQUEST_STARTED: LazyLock<Signal<RequestStarted>> = LazyLock::new(Signal::new);

/// Global signal fired when an HTTP request finishes processing.
pub static REQUEST_FINISHED: LazyLock<Signal<RequestFinished>> = LazyLock::new(Signal::new);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_signal_basic() {
        let signal = Signal::<String>::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let counter_clone1 = counter.clone();
        signal.connect(move |val| {
            let counter = counter_clone1.clone();
            async move {
                assert_eq!(val, "hello");
                counter.fetch_add(1, Ordering::SeqCst);
            }
        });

        let counter_clone2 = counter.clone();
        signal.connect(move |val| {
            let counter = counter_clone2.clone();
            async move {
                assert_eq!(val, "hello");
                counter.fetch_add(2, Ordering::SeqCst);
            }
        });

        signal.send("hello".to_string()).await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_signal_panic_isolation() {
        let signal = Signal::<()>::new();
        let run_indicator = Arc::new(AtomicUsize::new(0));

        // First subscriber panics
        signal.connect(|_| async move {
            panic!("intended panic in subscriber");
        });

        // Second subscriber should still run successfully
        let indicator = run_indicator.clone();
        signal.connect(move |_| {
            let indicator = indicator.clone();
            async move {
                indicator.fetch_add(1, Ordering::SeqCst);
            }
        });

        signal.send(()).await;
        // Verify the second one ran
        assert_eq!(run_indicator.load(Ordering::SeqCst), 1);
    }
}
