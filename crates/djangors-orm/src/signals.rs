//! Model lifecycle signals system for Djangors ORM.
//!
//! Provides a generic pub/sub primitive (`ModelSignal<T>`) used for model-level
//! lifecycle signals (`pre_save`, `post_save`, `pre_delete`, `post_delete`).
//!
//! This is a duplicate of the `Signal<T>` pattern from `djangors-core/src/signals.rs`,
//! replicated here because `djangors-core` depends on `djangors-orm` (not the reverse),
//! so `djangors-orm` cannot import `djangors_core::signals::Signal<T>`.

use crate::expr::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

/// Payload type for model lifecycle signals.
///
/// Each element is a `(field_name, current_value)` pair, built from
/// `Model::field_values()`. This avoids requiring `Self: Clone` on any model.
pub type ModelSignalPayload = Vec<(&'static str, Value)>;

type BoxedFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

type CallbackFn<T> = Arc<dyn Fn(T) -> BoxedFuture + Send + Sync>;

/// A generic signal that async callbacks can subscribe to.
///
/// Signals are typed, meaning they fire with a specific payload type `T`.
/// Callbacks are registered using `connect` and executed concurrently when `send` is called.
/// Panic isolation is guaranteed: a panic in one subscriber callback will not affect other
/// subscribers or the main execution flow.
pub struct ModelSignal<T: Clone + Send + Sync + 'static> {
    subscribers: RwLock<Vec<CallbackFn<T>>>,
}

impl<T: Clone + Send + Sync + 'static> Default for ModelSignal<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + Sync + 'static> ModelSignal<T> {
    /// Create a new, empty signal.
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Register an async callback to run whenever this signal fires.
    ///
    /// Callbacks run concurrently when the signal is fired. Their panics
    /// are isolated and do not affect the sender or other callbacks.
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
    /// Waits for all callbacks to complete concurrently (using `tokio::spawn` and joining
    /// the handles) while isolating each callback's panics so one failing subscriber does
    /// not disrupt the caller or other subscribers.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_model_signal_basic() {
        let signal = ModelSignal::<String>::new();
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
    async fn test_model_signal_panic_isolation() {
        let signal = ModelSignal::<()>::new();
        let run_indicator = Arc::new(AtomicUsize::new(0));

        signal.connect(|_| async move {
            panic!("intended panic in subscriber");
        });

        let indicator = run_indicator.clone();
        signal.connect(move |_| {
            let indicator = indicator.clone();
            async move {
                indicator.fetch_add(1, Ordering::SeqCst);
            }
        });

        signal.send(()).await;
        assert_eq!(run_indicator.load(Ordering::SeqCst), 1);
    }
}
