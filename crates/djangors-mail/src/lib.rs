#![deny(missing_docs)]
//! Email messages and pluggable delivery backends.

use async_trait::async_trait;
use lettre::{
    message::{header::ContentType, Mailbox, MultiPart, SinglePart},
    AsyncSmtpTransport, AsyncTransport, Message as LettreMessage, Tokio1Executor,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

/// An outgoing email message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Recipient email addresses.
    pub to: Vec<String>,
    /// Sender email address.
    pub from: String,
    /// Email subject line.
    pub subject: String,
    /// Plain text email body.
    pub body: String,
    /// Optional HTML formatted email body.
    pub html_body: Option<String>,
}

/// Errors returned by email backends during delivery or configuration.
#[derive(Error, Debug)]
pub enum MailError {
    /// Email sending failed.
    #[error("failed to send mail: {0}")]
    Send(String),
    /// Invalid mail backend configuration.
    #[error("invalid mail configuration: {0}")]
    Configuration(String),
}

/// Trait defining a pluggable email delivery backend.
#[async_trait]
pub trait MailBackend: Send + Sync {
    /// Delivers the specified email `message`.
    async fn send(&self, message: &Message) -> Result<(), MailError>;
}

/// Email backend that logs outgoing messages via `tracing::info`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConsoleBackend;

#[async_trait]
impl MailBackend for ConsoleBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        tracing::info!(to=?message.to, from=%message.from, subject=%message.subject, body=%message.body, html_body=?message.html_body, "ConsoleBackend sending mail");
        Ok(())
    }
}

/// Configuration parameters for SMTP mail delivery.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    /// Hostname or IP address of the SMTP server.
    pub host: String,
    /// Port number for the SMTP connection.
    pub port: u16,
    /// Optional username for SMTP authentication.
    pub username: Option<String>,
    /// Optional password for SMTP authentication.
    pub password: Option<String>,
    /// Whether to enforce TLS encryption.
    pub use_tls: bool,
}

impl SmtpConfig {
    /// Creates a new `SmtpConfig` targeting `host` on default port 587 with TLS enabled.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 587,
            username: None,
            password: None,
            use_tls: true,
        }
    }

    /// Sets the SMTP server port.
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the username and password for SMTP authentication.
    pub fn credentials(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }

    /// Configures whether TLS encryption should be required.
    pub fn use_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = use_tls;
        self
    }

    fn validate(&self) -> Result<(), MailError> {
        if self.host.trim().is_empty() {
            return Err(MailError::Configuration("SMTP host cannot be empty".into()));
        }
        if (self.username.is_some()) != (self.password.is_some()) {
            return Err(MailError::Configuration(
                "SMTP username and password must be supplied together".into(),
            ));
        }
        Ok(())
    }
}

/// SMTP email delivery backend using Lettre.
pub struct SmtpBackend {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpBackend {
    /// Constructs a new `SmtpBackend` from `config`.
    pub fn new(config: SmtpConfig) -> Result<Self, MailError> {
        config.validate()?;
        // `relay` uses implicit TLS; when TLS is disabled, `builder_dangerous` is explicit.
        // No TLS downgrade is performed when `use_tls` is true.
        let mut builder = if config.use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|e| MailError::Configuration(e.to_string()))?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
        };
        builder = builder.port(config.port);
        if let (Some(user), Some(pass)) = (config.username, config.password) {
            builder = builder.credentials(
                lettre::transport::smtp::authentication::Credentials::new(user, pass),
            );
        }
        Ok(Self {
            transport: builder.build(),
        })
    }

    fn build_message(message: &Message) -> Result<LettreMessage, MailError> {
        let from: Mailbox = message
            .from
            .parse()
            .map_err(|e| MailError::Send(format!("invalid sender: {e}")))?;
        let mut builder = LettreMessage::builder()
            .from(from)
            .subject(&message.subject);
        for to in &message.to {
            builder = builder.to(to
                .parse()
                .map_err(|e| MailError::Send(format!("invalid recipient: {e}")))?);
        }
        if let Some(html) = &message.html_body {
            builder
                .multipart(MultiPart::alternative_plain_html(
                    message.body.clone(),
                    html.clone(),
                ))
                .map_err(|e| MailError::Send(e.to_string()))
        } else {
            builder
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(message.body.clone()),
                )
                .map_err(|e| MailError::Send(e.to_string()))
        }
    }

    #[cfg(test)]
    fn serialize(message: &Message) -> Result<Vec<u8>, MailError> {
        Ok(Self::build_message(message)?.formatted())
    }
}

#[async_trait]
impl MailBackend for SmtpBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        self.transport
            .send(Self::build_message(message)?)
            .await
            .map(|_| ())
            .map_err(|e| MailError::Send(e.to_string()))
    }
}

/// Email backend that writes outgoing messages as `.eml` files to a target directory.
#[derive(Debug, Clone)]
pub struct FileBackend {
    directory: PathBuf,
}

impl FileBackend {
    /// Creates a new `FileBackend` that outputs emails into `directory`.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    fn format(message: &Message) -> String {
        format!(
            "From: {}\nTo: {}\nSubject: {}\n\n{}{}",
            message.from,
            message.to.join(", "),
            message.subject,
            message.body,
            message
                .html_body
                .as_ref()
                .map(|h| format!("\n\n[HTML body]\n{h}"))
                .unwrap_or_default()
        )
    }
}

#[async_trait]
impl MailBackend for FileBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        tokio::fs::create_dir_all(&self.directory)
            .await
            .map_err(|e| MailError::Send(e.to_string()))?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = self.directory.join(format!("{}.eml", nanos));
        tokio::fs::write(path, Self::format(message))
            .await
            .map_err(|e| MailError::Send(e.to_string()))
    }
}

/// An in-memory email backend for testing that records sent messages.
#[derive(Debug, Clone, Default)]
pub struct InMemoryBackend {
    messages: Arc<Mutex<Vec<Message>>>,
}

impl InMemoryBackend {
    /// Returns a vector clone of all email messages sent through this backend.
    pub fn sent_messages(&self) -> Vec<Message> {
        self.messages
            .lock()
            .expect("in-memory mail mutex poisoned")
            .clone()
    }
}
#[async_trait]
impl MailBackend for InMemoryBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        self.messages
            .lock()
            .map_err(|_| MailError::Send("in-memory mail mutex poisoned".into()))?
            .push(message.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message() -> Message {
        Message {
            to: vec!["recipient@example.com".into()],
            from: "sender@example.com".into(),
            subject: "Subject".into(),
            body: "plain body".into(),
            html_body: Some("<p>html body</p>".into()),
        }
    }
    #[tokio::test]
    async fn console_send() {
        ConsoleBackend.send(&message()).await.unwrap();
    }
    #[tokio::test]
    async fn in_memory_send_and_inspect() {
        let b = InMemoryBackend::default();
        b.send(&message()).await.unwrap();
        assert_eq!(b.sent_messages(), vec![message()]);
    }
    #[tokio::test]
    async fn file_send_can_be_read_back() {
        let dir = std::env::temp_dir().join(format!("djangors-mail-{}", std::process::id()));
        let b = FileBackend::new(&dir);
        b.send(&message()).await.unwrap();
        let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
        let p = entries.next_entry().await.unwrap().unwrap().path();
        let text = tokio::fs::read_to_string(p).await.unwrap();
        assert!(text.contains("plain body") && text.contains("html body"));
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
    #[test]
    fn smtp_config_rejects_empty_host() {
        assert!(SmtpBackend::new(SmtpConfig::new(" ")).is_err());
    }
    #[test]
    fn html_message_is_multipart_alternative() {
        let raw = SmtpBackend::serialize(&message()).unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(text.contains("multipart/alternative"));
        assert!(text.contains("plain body") && text.contains("html body"));
    }
}
