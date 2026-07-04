use crate::ApiError;

pub(crate) fn qbit_base_url(client: &livrarr_domain::DownloadClient) -> String {
    if client.host.starts_with("http://") || client.host.starts_with("https://") {
        let url_base = client.url_base.as_deref().unwrap_or("");
        return format!("{}{url_base}", client.host.trim_end_matches('/'));
    }

    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    if client.port == 80 || client.port == 443 {
        format!("{scheme}://{}{url_base}", client.host)
    } else {
        format!("{scheme}://{}:{}{url_base}", client.host, client.port)
    }
}

pub(crate) async fn qbit_login(
    http_client: &livrarr_http::HttpClient,
    base_url: &str,
    client: &livrarr_domain::DownloadClient,
) -> Result<String, ApiError> {
    let username = client.username.as_deref().unwrap_or("");
    let password = client.password.as_deref().unwrap_or("");

    if username.is_empty() && password.is_empty() {
        return Ok(String::new());
    }

    let login_url = format!("{base_url}/api/v2/auth/login");
    let resp = http_client
        .post(&login_url)
        .form(&[("username", username), ("password", password)])
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("qBittorrent login failed: {e}")))?;

    let auth_cookie = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            let cookie = s.split(';').next()?.trim();

            let name = cookie.split('=').next()?;
            if name == "SID" || name == "QBT_SID" || name.starts_with("QBT_SID_") {
                Some(cookie.to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    if auth_cookie.is_empty() {
        let body = resp.text().await.unwrap_or_default();
        if body.contains("Fails") {
            return Err(ApiError::BadGateway(
                "qBittorrent authentication failed".into(),
            ));
        }
        return Err(ApiError::BadGateway(
            "qBittorrent login failed: empty session ID".into(),
        ));
    }

    Ok(auth_cookie)
}
