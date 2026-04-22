#[derive(Debug, thiserror::Error)]
pub enum EmailServiceError {
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Send(String),
}

#[trait_variant::make(Send)]
pub trait EmailService: Send + Sync {
    async fn send_test(&self) -> Result<(), EmailServiceError>;
    async fn send_file(
        &self,
        file_bytes: Vec<u8>,
        filename: &str,
        extension: &str,
    ) -> Result<(), EmailServiceError>;
}
