use crate::DbError;

/// Whether the database answers a read through the live connection pool.
#[trait_variant::make(Send)]
pub trait DatabaseHealth: Send + Sync {
    /// Reads the schema table through the pool. Fails when the pool cannot
    /// hand out a connection or SQLite raises an error for the read.
    async fn check_database(&self) -> Result<(), DbError>;
}
