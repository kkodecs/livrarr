use crate::{CoverCandidate, CoverMediaType, UserId, WorkId};

#[derive(Debug)]
pub enum CoverServiceError {
    NotFound,
    InvalidCandidate(String),
    UploadValidation(String),
    Internal(String),
}

impl std::fmt::Display for CoverServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "work not found"),
            Self::InvalidCandidate(msg) => write!(f, "invalid candidate: {msg}"),
            Self::UploadValidation(msg) => write!(f, "upload validation: {msg}"),
            Self::Internal(msg) => write!(f, "cover service error: {msg}"),
        }
    }
}

impl std::error::Error for CoverServiceError {}

#[trait_variant::make(Send)]
pub trait CoverService: Send + Sync {
    async fn fetch_alternatives(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<CoverCandidate>, CoverServiceError>;

    async fn select_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        candidate_id: &str,
        media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError>;

    async fn upload_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        data: &[u8],
        media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError>;
}
