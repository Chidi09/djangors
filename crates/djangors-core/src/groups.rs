use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::broadcast;
use tokio_stream::Stream;

const DEFAULT_GROUP_CAPACITY: usize = 1024;

/// A handle to a named broadcast channel group.
#[derive(Clone, Debug)]
pub struct GroupHandle {
    sender: broadcast::Sender<String>,
}

impl GroupHandle {
    /// Creates a new `GroupHandle` wrapping `sender`.
    pub fn new(sender: broadcast::Sender<String>) -> Self {
        Self { sender }
    }

    /// Broadcast a message to all current subscribers in this group.
    ///
    /// If there are no current subscribers, the message is dropped silently
    /// without error.
    pub fn send(&self, msg: impl Into<String>) {
        let _ = self.sender.send(msg.into());
    }

    /// Subscribe to messages broadcast to this group.
    ///
    /// Returns an async [`Stream`] of string messages.
    pub fn subscribe(&self) -> GroupStream {
        GroupStream {
            receiver: tokio_stream::wrappers::BroadcastStream::new(self.sender.subscribe()),
        }
    }

    /// Get the number of active receivers currently subscribed to this group.
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// An async stream of messages received from a group broadcast channel.
pub struct GroupStream {
    receiver: tokio_stream::wrappers::BroadcastStream<String>,
}

impl Stream for GroupStream {
    type Item = String;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.receiver).poll_next(cx) {
                Poll::Ready(Some(Ok(msg))) => return Poll::Ready(Some(msg)),
                Poll::Ready(Some(Err(
                    tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_),
                ))) => {
                    // Skip lagged error and poll for the next message
                    continue;
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// An in-process registry of named pub/sub broadcast groups.
///
/// Channels are lazily instantiated on first access via [`group`](Self::group).
#[derive(Clone)]
pub struct Groups {
    channels: Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
    capacity: usize,
}

impl Default for Groups {
    fn default() -> Self {
        Self::new()
    }
}

impl Groups {
    /// Create a new `Groups` registry with default channel capacity (1024).
    pub fn new() -> Self {
        Self {
            channels: Arc::new(Mutex::new(HashMap::new())),
            capacity: DEFAULT_GROUP_CAPACITY,
        }
    }

    /// Create a new `Groups` registry with a custom per-group channel capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            channels: Arc::new(Mutex::new(HashMap::new())),
            capacity,
        }
    }

    /// Get or lazily create a group handle for the given group name.
    pub fn group(&self, name: &str) -> GroupHandle {
        let mut map = self.channels.lock().expect("lock groups mutex");
        let sender = map
            .entry(name.to_string())
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(self.capacity);
                tx
            })
            .clone();
        GroupHandle::new(sender)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_groups_broadcast_with_no_receivers() {
        let groups = Groups::new();
        let handle = groups.group("events");
        // Should not panic or error even with 0 receivers
        handle.send("hello");
    }

    #[tokio::test]
    async fn test_groups_subscribe_and_receive() {
        use tokio_stream::StreamExt;

        let groups = Groups::new();
        let handle = groups.group("news");
        let mut stream = handle.subscribe();

        handle.send("breaking news");

        let msg = stream.next().await;
        assert_eq!(msg, Some("breaking news".to_string()));
    }
}
