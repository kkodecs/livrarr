use livrarr_db::{DownloadClientDb, GrabDb};
use livrarr_domain::services::{QueueService, QueueServiceError};
use livrarr_domain::{DownloadClient, Grab, GrabId, GrabStatus, QueueProgress, UserId};
use livrarr_http::HttpClient;

pub struct QueueServiceImpl<D> {
    db: D,
    http: HttpClient,
}

impl<D> QueueServiceImpl<D> {
    pub fn new(db: D, http: HttpClient) -> Self {
        Self { db, http }
    }
}

fn map_db_err(e: livrarr_domain::DbError) -> QueueServiceError {
    match e {
        livrarr_domain::DbError::NotFound { .. } => QueueServiceError::NotFound,
        other => QueueServiceError::Db(other),
    }
}

impl<D> QueueService for QueueServiceImpl<D>
where
    D: GrabDb + DownloadClientDb + Send + Sync + 'static,
{
    async fn list_grabs_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Grab>, i64), QueueServiceError> {
        self.db
            .list_grabs_paginated(user_id, page, per_page)
            .await
            .map_err(map_db_err)
    }

    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, QueueServiceError> {
        self.db.list_download_clients().await.map_err(map_db_err)
    }

    async fn try_set_importing(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<bool, QueueServiceError> {
        self.db
            .try_set_importing(user_id, grab_id)
            .await
            .map_err(map_db_err)
    }

    async fn update_grab_status(
        &self,
        user_id: UserId,
        grab_id: GrabId,
        status: GrabStatus,
        error: Option<&str>,
    ) -> Result<(), QueueServiceError> {
        self.db
            .update_grab_status(user_id, grab_id, status, error)
            .await
            .map_err(map_db_err)
    }

    async fn fetch_download_progress(
        &self,
        client: &DownloadClient,
        download_id: &str,
    ) -> Option<QueueProgress> {
        match client.client_type() {
            "sabnzbd" => fetch_sab_progress(&self.http, client, download_id).await,
            _ => fetch_qbit_progress(&self.http, client, download_id).await,
        }
    }
}

async fn fetch_qbit_progress(
    http: &HttpClient,
    client: &DownloadClient,
    hash: &str,
) -> Option<QueueProgress> {
    let base_url = livrarr_handlers::download_client::client_base_url(client);

    let username = client.username.as_deref().unwrap_or("");
    let password = client.password.as_deref().unwrap_or("");
    let mut sid: Option<String> = None;
    if !username.is_empty() || !password.is_empty() {
        let login_url = format!("{base_url}/api/v2/auth/login");
        let resp = http
            .post(&login_url)
            .form(&[("username", username), ("password", password)])
            .send()
            .await
            .ok()?;
        if let Some(cookie) = resp
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
        {
            if let Some(s) = cookie
                .split(';')
                .next()
                .and_then(|c| c.strip_prefix("SID="))
            {
                sid = Some(s.to_string());
            }
        }
    }

    let url = format!("{base_url}/api/v2/torrents/info");
    let mut req = http.get(&url).query(&[("hashes", hash)]);
    if let Some(ref s) = sid {
        req = req.header("Cookie", format!("SID={s}"));
    }
    let resp = req
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    let torrents: Vec<serde_json::Value> = resp.json().await.ok()?;
    let t = torrents.first()?;

    let progress = t.get("progress").and_then(|p| p.as_f64()).unwrap_or(0.0);
    let eta = t
        .get("eta")
        .and_then(|e| e.as_i64())
        .filter(|&e| e > 0 && e < 86400);
    let qstate = t.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");

    Some(QueueProgress {
        percent: (progress * 100.0).round(),
        eta,
        download_status: qstate.to_string(),
    })
}

async fn fetch_sab_progress(
    http: &HttpClient,
    client: &DownloadClient,
    nzo_id: &str,
) -> Option<QueueProgress> {
    let base_url = livrarr_handlers::download_client::client_base_url(client);
    let api_key = client.api_key.as_deref().unwrap_or("");

    let url = format!("{base_url}/api?mode=queue&apikey={api_key}&output=json");
    let resp = http
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    let body: serde_json::Value = resp.json().await.ok()?;
    let slot = body
        .get("queue")
        .and_then(|q| q.get("slots"))
        .and_then(|s| s.as_array())
        .and_then(|slots| {
            slots
                .iter()
                .find(|s| s.get("nzo_id").and_then(|n| n.as_str()) == Some(nzo_id))
        })?;

    let pct = slot
        .get("percentage")
        .and_then(|p| p.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let status = slot
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("Queued");
    let timeleft = slot
        .get("timeleft")
        .and_then(|t| t.as_str())
        .and_then(parse_sab_timeleft);

    Some(QueueProgress {
        percent: pct,
        eta: timeleft,
        download_status: status.to_string(),
    })
}

fn parse_sab_timeleft(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h: i64 = parts[0].parse().ok()?;
        let m: i64 = parts[1].parse().ok()?;
        let s: i64 = parts[2].parse().ok()?;
        let total = h * 3600 + m * 60 + s;
        if total > 0 && total < 86400 {
            Some(total)
        } else {
            None
        }
    } else {
        None
    }
}
