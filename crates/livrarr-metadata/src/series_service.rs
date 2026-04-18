use livrarr_db::SeriesDb;
use livrarr_domain::services::*;
use livrarr_domain::*;

pub struct SeriesServiceImpl<D> {
    db: D,
}

impl<D> SeriesServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

impl<D> SeriesService for SeriesServiceImpl<D>
where
    D: SeriesDb + Send + Sync,
{
    async fn list(&self, user_id: UserId) -> Result<Vec<Series>, SeriesServiceError> {
        self.db
            .list_all_series(user_id)
            .await
            .map_err(SeriesServiceError::Db)
    }

    async fn get(&self, user_id: UserId, series_id: i64) -> Result<Series, SeriesServiceError> {
        let series = self
            .db
            .get_series(series_id)
            .await
            .map_err(SeriesServiceError::Db)?
            .ok_or(SeriesServiceError::NotFound)?;

        if series.user_id != user_id {
            return Err(SeriesServiceError::NotFound);
        }

        Ok(series)
    }

    async fn refresh(
        &self,
        _user_id: UserId,
        _series_id: i64,
    ) -> Result<Series, SeriesServiceError> {
        // Requires Goodreads provider integration
        todo!("series refresh via Goodreads")
    }

    async fn monitor(
        &self,
        user_id: UserId,
        series_id: i64,
        monitored: bool,
    ) -> Result<Series, SeriesServiceError> {
        let series = self.get(user_id, series_id).await?;

        self.db
            .update_series_flags(series.id, monitored, monitored)
            .await
            .map_err(SeriesServiceError::Db)
    }

    async fn update(
        &self,
        user_id: UserId,
        series_id: i64,
        _title: Option<String>,
    ) -> Result<Series, SeriesServiceError> {
        // SeriesDb doesn't have a title update method — series names come from Goodreads.
        // For now, just return the existing series after verifying ownership.
        self.get(user_id, series_id).await
    }
}
