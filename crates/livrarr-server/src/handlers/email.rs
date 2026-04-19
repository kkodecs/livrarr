//! Shared email sending utilities for Send to Kindle / eReader.

use lettre::message::{header::ContentType, Attachment, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use livrarr_domain::settings::EmailConfig;

/// Accepted file extensions for email delivery.
pub const ACCEPTED_EXTENSIONS: &[&str] =
    &["epub", "pdf", "docx", "doc", "rtf", "htm", "html", "txt"];

/// Maximum file size for email attachments (50 MB).
pub const MAX_EMAIL_SIZE: i64 = 50 * 1024 * 1024;

/// Build an SMTP transport from an EmailConfig.
fn build_transport(
    cfg: &EmailConfig,
    creds: Credentials,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let mailer = match cfg.encryption.as_str() {
        "ssl" => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.smtp_host)
            .map_err(|e| format!("SMTP relay error: {e}"))?
            .port(cfg.smtp_port as u16)
            .credentials(creds)
            .build(),
        "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)
            .map_err(|e| format!("SMTP STARTTLS error: {e}"))?
            .port(cfg.smtp_port as u16)
            .credentials(creds)
            .build(),
        _ => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host)
            .port(cfg.smtp_port as u16)
            .credentials(creds)
            .build(),
    };
    Ok(mailer)
}

/// Validate email config has all required fields. Returns (from_address, recipient, password, username).
pub fn validate_config(cfg: &EmailConfig) -> Result<(String, String, String, String), String> {
    let from_address = cfg
        .from_address
        .clone()
        .ok_or("Email 'From Address' not configured")?;
    let recipient = cfg
        .recipient_email
        .clone()
        .ok_or("Kindle email not configured")?;
    let password = cfg.password.clone().ok_or("SMTP password not configured")?;
    let username = cfg.username.clone().unwrap_or_else(|| from_address.clone());
    Ok((from_address, recipient, password, username))
}

/// Generate a proper Message-ID using the sender's domain.
fn make_message_id(from_address: &str) -> String {
    let sender_domain = from_address.split('@').nth(1).unwrap_or("livrarr.local");
    format!(
        "<{}.{}@{}>",
        chrono::Utc::now().timestamp(),
        uuid::Uuid::new_v4(),
        sender_domain
    )
}

/// MIME type for a file extension.
pub fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "rtf" => "application/rtf",
        "htm" | "html" => "text/html",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Send a test email (no attachment) to verify SMTP connectivity.
pub async fn send_test(cfg: &EmailConfig) -> Result<(), String> {
    let (from_address, recipient, password, username) = validate_config(cfg)?;
    let message_id = make_message_id(&from_address);

    let email = Message::builder()
        .message_id(Some(message_id))
        .from(
            from_address
                .parse()
                .map_err(|e| format!("Invalid 'From' address: {e}"))?,
        )
        .to(recipient
            .parse()
            .map_err(|e| format!("Invalid recipient address: {e}"))?)
        .subject("Livrarr Test Email")
        .singlepart(SinglePart::plain(String::from(
            "This is a test email from Livrarr. If you received this, your email configuration is working correctly.",
        )))
        .map_err(|e| format!("Failed to build email: {e}"))?;

    let creds = Credentials::new(username, password);
    let mailer = build_transport(cfg, creds)?;

    mailer
        .send(email)
        .await
        .map_err(|e| format!("Failed to send: {e}"))?;

    Ok(())
}

/// Send a file as an email attachment.
pub async fn send_file(
    cfg: &EmailConfig,
    file_bytes: Vec<u8>,
    filename: &str,
    ext: &str,
) -> Result<(), String> {
    let (from_address, recipient, password, username) = validate_config(cfg)?;
    let message_id = make_message_id(&from_address);
    let mime = mime_for_ext(ext);

    let attachment = Attachment::new(filename.to_string())
        .body(file_bytes, mime.parse().unwrap_or(ContentType::TEXT_PLAIN));

    let email = Message::builder()
        .message_id(Some(message_id))
        .from(
            from_address
                .parse()
                .map_err(|e| format!("Invalid 'From' address: {e}"))?,
        )
        .to(recipient
            .parse()
            .map_err(|e| format!("Invalid recipient address: {e}"))?)
        .subject(String::new())
        .multipart(
            MultiPart::mixed()
                .singlepart(SinglePart::plain(String::from("Sent from Livrarr")))
                .singlepart(attachment),
        )
        .map_err(|e| format!("Failed to build email: {e}"))?;

    let creds = Credentials::new(username, password);
    let mailer = build_transport(cfg, creds)?;

    mailer
        .send(email)
        .await
        .map_err(|e| format!("Failed to send: {e}"))?;

    Ok(())
}
