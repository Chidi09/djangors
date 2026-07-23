# How to Send Email

## Problem
You want to send plain-text or HTML emails in your Djangors application using SMTP, or write outgoing messages to local disk during development.

## Solution
Use `djangors_mail::Message` and configure a backend implementing the `MailBackend` trait, such as `SmtpBackend`, `FileBackend`, or `ConsoleBackend`.

## Code Example

### 1. Sending via SMTP (Production)

```rust
use djangors_mail::{Message, SmtpConfig, SmtpBackend, MailBackend};

pub async fn send_welcome_email(user_email: &str) -> Result<(), djangors_mail::MailError> {
    // Construct email message
    let message = Message {
        to: vec![user_email.to_string()],
        from: "noreply@example.com".to_string(),
        subject: "Welcome to our platform!".to_string(),
        body: "Hello, welcome to our service!".to_string(),
        html_body: Some("<h1>Hello!</h1><p>Welcome to our service!</p>".to_string()),
    };

    // Configure SMTP backend
    let config = SmtpConfig::new("smtp.example.com")
        .port(587)
        .credentials("smtp_user", "smtp_pass")
        .use_tls(true);

    let backend = SmtpBackend::new(config)?;
    backend.send(&message).await?;

    Ok(())
}
```

### 2. Saving Emails to Disk (Development)

```rust
use djangors_mail::{Message, FileBackend, MailBackend};
use std::path::PathBuf;

pub async fn send_dev_email(message: &Message) -> Result<(), djangors_mail::MailError> {
    // Writes emails as `.eml` files in the specified directory
    let backend = FileBackend::new(PathBuf::from("/tmp/app_emails"));
    backend.send(message).await?;
    Ok(())
}
```
