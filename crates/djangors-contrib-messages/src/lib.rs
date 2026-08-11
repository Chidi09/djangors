#![deny(missing_docs)]
//! # djangors-contrib-messages
//!
//! Per-session flash-message queue for the Djangors web framework.
//!
//! Provides one-shot notifications (e.g., `"Profile updated successfully"`) stored in the request session.
//! Views add messages with [`add`] (or shorthands [`add_success`], [`add_error`], etc.), and the rendering code
//! consumes them with [`take`].
//!
//! ## Integration Example
//!
//! Page rendering call sites thread [`take`] into their template context:
//!
//! ```rust,ignore
//! use djangors_contrib_messages as messages;
//! use djangors_sessions::Session;
//! use serde::Serialize;
//!
//! fn handle_request(session: &Session) {
//!     // In a view/handler:
//!     messages::add_success(session, "Profile updated successfully!");
//! }
//!
//! #[derive(Serialize)]
//! struct PageContext {
//!     messages: Vec<messages::Message>,
//! }
//!
//! fn render_response(session: &Session) {
//!     // Before rendering:
//!     let context = PageContext {
//!         messages: messages::take(session),
//!     };
//!     // Render template with `context`...
//! }
//! ```

use djangors_sessions::Session;
use serde::{Deserialize, Serialize};

const SESSION_KEY: &str = "_djangors_messages";

/// Severity level of a flash message, matching Django message levels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Level {
    /// Development/debug informational message.
    Debug,
    /// Standard informational notification.
    Info,
    /// Action completed successfully.
    Success,
    /// Non-fatal warning or advisory.
    Warning,
    /// Operation failed or critical error message.
    Error,
}

/// A queued flash notification message containing a severity level and message string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    /// Severity level of the message.
    pub level: Level,
    /// Text body of the message.
    pub text: String,
}

impl Message {
    /// Creates a new `Message` with the given level and text.
    pub fn new(level: Level, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}

/// Pushes a new message onto the session's queued message list.
pub fn add(session: &Session, level: Level, text: impl Into<String>) {
    let mut messages: Vec<Message> = session.get(SESSION_KEY).unwrap_or_default();
    messages.push(Message::new(level, text));
    session.set(SESSION_KEY, messages);
}

/// Consumes and clears all queued messages from the session.
/// A second call in the same session returns an empty vector.
pub fn take(session: &Session) -> Vec<Message> {
    let messages: Vec<Message> = session.get(SESSION_KEY).unwrap_or_default();
    session.remove(SESSION_KEY);
    messages
}

/// Convenience helper to add a Debug-level message.
pub fn add_debug(session: &Session, text: impl Into<String>) {
    add(session, Level::Debug, text);
}

/// Convenience helper to add an Info-level message.
pub fn add_info(session: &Session, text: impl Into<String>) {
    add(session, Level::Info, text);
}

/// Convenience helper to add a Success-level message.
pub fn add_success(session: &Session, text: impl Into<String>) {
    add(session, Level::Success, text);
}

/// Convenience helper to add a Warning-level message.
pub fn add_warning(session: &Session, text: impl Into<String>) {
    add(session, Level::Warning, text);
}

/// Convenience helper to add an Error-level message.
pub fn add_error(session: &Session, text: impl Into<String>) {
    add(session, Level::Error, text);
}

#[cfg(test)]
mod tests;
