//! Minimal mail support for Djangors.
//!
//! # WARNING: Minimal Scope (Phase 4 v1)
//!
//! This crate is a minimal, console-only slice pull-forward needed for Phase 4 password reset testing.
//! It is NOT the full Phase 7 `djangors-mail` deliverable, which will include SMTP, TLS, file,
//! in-memory test backends, and HTML-multipart messages.
//!
//! Do not use this for production SMTP email delivery.

use async_trait::async_trait;
use thiserror::Error;

/// A simple, plain-text email message.
#[derive(Debug, Clone)]
pub struct Message {
    /// List of recipient email addresses.
    pub to: Vec<String>,
    /// The sender's email address.
    pub from: String,
    /// The subject line of the email.
    pub subject: String,
    /// The plain text body of the email.
    pub body: String,
}

/// Errors returned by mail backends.
#[derive(Error, Debug)]
pub enum MailError {
    /// Failed to send the mail message.
    #[error("failed to send mail: {0}")]
    Send(String),
}

/// A backend capable of sending email messages.
#[async_trait]
pub trait MailBackend: Send + Sync {
    /// Sends the given message.
    async fn send(&self, message: &Message) -> Result<(), MailError>;
}

/// A mail backend that outputs messages to the console using the tracing subscriber.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConsoleBackend;

#[async_trait]
impl MailBackend for ConsoleBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        tracing::info!(
            to = ?message.to,
            from = %message.from,
            subject = %message.subject,
            body = %message.body,
            "ConsoleBackend sending mail"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_console_backend_send_does_not_panic_or_error() {
        let msg = Message {
            to: vec!["recipient@example.com".to_string()],
            from: "sender@example.com".to_string(),
            subject: "Reset Password".to_string(),
            body: "Click here to reset: https://example.com/reset/token".to_string(),
        };

        let backend = ConsoleBackend;
        let result = backend.send(&msg).await;
        assert!(result.is_ok(), "ConsoleBackend::send failed: {:?}", result);
    }
}
