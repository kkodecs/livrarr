use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    CreateHistoryEventDbRequest, DbError, EventType, HistoryDb, HistoryEvent, HistoryFilter,
};

fn row_to_history_event(row: sqlx::sqlite::SqliteRow) -> Result<HistoryEvent, DbError> {
    let event_type_str: String = row
        .try_get("event_type")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let data_str: String = row.try_get("data").map_err(|e| DbError::Io(Box::new(e)))?;
    let date_str: String = row.try_get("date").map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(HistoryEvent {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        work_id: row
            .try_get::<Option<i64>, _>("work_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        event_type: parse_event_type(&event_type_str)?,
        data: serde_json::from_str(&data_str).map_err(|e| DbError::Io(Box::new(e)))?,
        date: parse_dt(&date_str)?,
    })
}

fn parse_event_type(s: &str) -> Result<EventType, DbError> {
    match s {
        "grabbed" => Ok(EventType::Grabbed),
        "downloadCompleted" => Ok(EventType::DownloadCompleted),
        "downloadFailed" => Ok(EventType::DownloadFailed),
        "imported" => Ok(EventType::Imported),
        "importFailed" => Ok(EventType::ImportFailed),
        "enriched" => Ok(EventType::Enriched),
        "enrichmentFailed" => Ok(EventType::EnrichmentFailed),
        "tagWritten" => Ok(EventType::TagWritten),
        "tagWriteFailed" => Ok(EventType::TagWriteFailed),
        "fileDeleted" => Ok(EventType::FileDeleted),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown event type: {s}"),
        }),
    }
}

fn event_type_str(t: EventType) -> &'static str {
    match t {
        EventType::Grabbed => "grabbed",
        EventType::DownloadCompleted => "downloadCompleted",
        EventType::DownloadFailed => "downloadFailed",
        EventType::Imported => "imported",
        EventType::ImportFailed => "importFailed",
        EventType::Enriched => "enriched",
        EventType::EnrichmentFailed => "enrichmentFailed",
        EventType::TagWritten => "tagWritten",
        EventType::TagWriteFailed => "tagWriteFailed",
        EventType::FileDeleted => "fileDeleted",
    }
}

impl HistoryDb for SqliteDb {
    async fn list_history(
        &self,
        user_id: crate::UserId,
        filter: HistoryFilter,
    ) -> Result<Vec<HistoryEvent>, DbError> {
        let mut query = "SELECT * FROM history WHERE user_id = ?".to_string();
        let mut binds: Vec<String> = vec![];

        if let Some(event_type) = filter.event_type {
            query.push_str(" AND event_type = ?");
            binds.push(event_type_str(event_type).to_string());
        }
        if let Some(work_id) = filter.work_id {
            query.push_str(" AND work_id = ?");
            binds.push(work_id.to_string());
        }
        if let Some(start_date) = filter.start_date {
            query.push_str(" AND date >= ?");
            binds.push(start_date.to_rfc3339());
        }
        if let Some(end_date) = filter.end_date {
            query.push_str(" AND date <= ?");
            binds.push(end_date.to_rfc3339());
        }
        query.push_str(" ORDER BY id DESC");

        let mut q = sqlx::query(&query).bind(user_id);
        for b in &binds {
            q = q.bind(b);
        }

        let rows = q.fetch_all(self.pool()).await.map_err(map_db_err)?;
        rows.into_iter().map(row_to_history_event).collect()
    }

    async fn create_history_event(&self, req: CreateHistoryEventDbRequest) -> Result<(), DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        let event_type_s = event_type_str(req.event_type);
        let data_str = serde_json::to_string(&req.data).map_err(|e| DbError::Io(Box::new(e)))?;

        sqlx::query(
            "INSERT INTO history (user_id, work_id, event_type, data, date) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(req.user_id)
        .bind(req.work_id)
        .bind(event_type_s)
        .bind(&data_str)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }
}
