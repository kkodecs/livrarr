use crate::DbError;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Db(DbError),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Internal(String),
}

impl From<DbError> for ServiceError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::NotFound { .. } => ServiceError::NotFound,
            other => ServiceError::Db(other),
        }
    }
}
