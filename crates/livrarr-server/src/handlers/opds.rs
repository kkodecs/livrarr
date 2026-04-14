use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;
use livrarr_db::{AuthorDb, LibraryItemDb, WorkDb};
use livrarr_domain::{LibraryItem, User, Work};

// ---------------------------------------------------------------------------
// OPDS Basic Auth
// ---------------------------------------------------------------------------

/// Extract user from HTTP Basic Auth (username + password).
/// Returns 401 with WWW-Authenticate header on failure.
async fn basic_auth(state: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    let unauthorized = || {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"Livrarr OPDS\"")],
            "Unauthorized",
        )
            .into_response()
    };

    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauthorized)?;

    if !auth_header.starts_with("Basic ") {
        return Err(unauthorized());
    }

    use data_encoding::BASE64;
    let decoded = BASE64
        .decode(auth_header[6..].trim().as_bytes())
        .map_err(|_| unauthorized())?;
    let creds = String::from_utf8(decoded).map_err(|_| unauthorized())?;
    let (username, password) = creds.split_once(':').ok_or_else(unauthorized)?;

    use crate::AuthService;
    state
        .auth_service
        .verify_credentials(username, password)
        .await
        .map_err(|_| unauthorized())
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "mobi" => "application/x-mobipocket-ebook",
        "azw3" => "application/x-mobi8-ebook",
        "m4b" | "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

const ATOM_NS: &str = "http://www.w3.org/2005/Atom";
const DC_NS: &str = "http://purl.org/dc/terms/";
const OPDS_NS: &str = "http://opds-spec.org/2010/catalog";
const PAGE_SIZE: usize = 20;

fn xml_response(body: String) -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/atom+xml;profile=opds-catalog;charset=utf-8",
        )],
        body,
    )
        .into_response()
}

fn feed_header(id: &str, title: &str, self_href: &str) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="{ATOM_NS}" xmlns:dc="{DC_NS}" xmlns:opds="{OPDS_NS}">
  <id>{id}</id>
  <title>{title}</title>
  <updated>{now}</updated>
  <link rel="self" href="{self_href}" type="application/atom+xml;profile=opds-catalog"/>
  <link rel="start" href="/opds/" type="application/atom+xml;profile=opds-catalog"/>
  <link rel="search" href="/opds/osd" type="application/opensearchdescription+xml"/>
"#
    )
}

fn nav_entry(title: &str, href: &str, content: &str) -> String {
    let id = xml_escape(href);
    let now = chrono::Utc::now().to_rfc3339();
    format!(
        r#"  <entry>
    <title>{title}</title>
    <id>{id}</id>
    <updated>{now}</updated>
    <content type="text">{content}</content>
    <link rel="subsection" href="{href}" type="application/atom+xml;profile=opds-catalog"/>
  </entry>
"#,
        title = xml_escape(title),
        content = xml_escape(content),
    )
}

fn work_entry(work: &Work, items: &[LibraryItem]) -> String {
    let id = format!("urn:livrarr:work:{}", work.id);
    let title = xml_escape(&work.title);
    let author = xml_escape(&work.author_name);
    let updated = work.added_at.to_rfc3339();
    let desc = work
        .description
        .as_deref()
        .map(xml_escape)
        .unwrap_or_default();
    let lang = work
        .language
        .as_deref()
        .map(|l| format!("  <dc:language>{}</dc:language>\n", xml_escape(l)))
        .unwrap_or_default();

    let cover_link = format!(
        r#"  <link rel="http://opds-spec.org/image" href="/opds/cover/{}" type="image/jpeg"/>"#,
        work.id
    );

    let acq_links: String = items
        .iter()
        .map(|item| {
            let ext = item
                .path
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_lowercase();
            let mime = mime_for_ext(&ext);
            format!(
                r#"  <link rel="http://opds-spec.org/acquisition" href="/opds/download/{}" type="{}" length="{}"/>"#,
                item.id, mime, item.file_size
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"  <entry>
    <title>{title}</title>
    <id>{id}</id>
    <updated>{updated}</updated>
    <author><name>{author}</name></author>
    <content type="text">{desc}</content>
{lang}{cover_link}
{acq_links}
  </entry>
"#
    )
}

fn pagination_links(base_href: &str, page: usize, total: usize) -> String {
    let mut links = String::new();
    if page > 1 {
        let sep = if base_href.contains('?') { "&" } else { "?" };
        links.push_str(&format!(
            r#"  <link rel="previous" href="{base_href}{sep}page={}" type="application/atom+xml;profile=opds-catalog"/>"#,
            page - 1
        ));
        links.push('\n');
    }
    let total_pages = total.div_ceil(PAGE_SIZE);
    if page < total_pages {
        let sep = if base_href.contains('?') { "&" } else { "?" };
        links.push_str(&format!(
            r#"  <link rel="next" href="{base_href}{sep}page={}" type="application/atom+xml;profile=opds-catalog"/>"#,
            page + 1
        ));
        links.push('\n');
    }
    links
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct PageQuery {
    pub page: Option<usize>,
}

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub page: Option<usize>,
}

/// GET /opds/ — root navigation catalog
pub async fn root(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, Response> {
    let _user = basic_auth(&state, &headers).await?;

    let mut body = feed_header("urn:livrarr:opds:root", "Livrarr OPDS Catalog", "/opds/");
    body.push_str(&nav_entry(
        "Recent Additions",
        "/opds/recent",
        "Recently added books",
    ));
    body.push_str(&nav_entry("Authors", "/opds/author", "Browse by author"));
    body.push_str(&nav_entry("Search", "/opds/search", "Search the library"));
    body.push_str("</feed>\n");

    Ok(xml_response(body))
}

/// GET /opds/recent — acquisition feed of newest items
pub async fn recent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(pq): Query<PageQuery>,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;
    let page = pq.page.unwrap_or(1).max(1);

    let (works, total) = state
        .db
        .list_works_paginated(user.id, page as u32, PAGE_SIZE as u32)
        .await
        .map_err(api_err_to_response)?;

    let work_ids: Vec<_> = works.iter().map(|w| w.id).collect();
    let items = state
        .db
        .list_library_items_by_work_ids(user.id, &work_ids)
        .await
        .map_err(api_err_to_response)?;

    let mut body = feed_header(
        "urn:livrarr:opds:recent",
        "Recent Additions",
        "/opds/recent",
    );
    body.push_str(&pagination_links("/opds/recent", page, total as usize));

    for work in &works {
        let work_items: Vec<_> = items
            .iter()
            .filter(|i| i.work_id == work.id)
            .cloned()
            .collect();
        if !work_items.is_empty() {
            body.push_str(&work_entry(work, &work_items));
        }
    }

    body.push_str("</feed>\n");
    Ok(xml_response(body))
}

/// GET /opds/author — navigation feed listing authors
pub async fn author_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;

    let authors = state
        .db
        .list_authors(user.id)
        .await
        .map_err(api_err_to_response)?;

    let mut body = feed_header("urn:livrarr:opds:authors", "Authors", "/opds/author");

    for author in &authors {
        body.push_str(&nav_entry(
            &author.name,
            &format!("/opds/author/{}", author.id),
            &format!("Works by {}", author.name),
        ));
    }

    body.push_str("</feed>\n");
    Ok(xml_response(body))
}

/// GET /opds/author/:id — acquisition feed for author's works
pub async fn author_works(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(author_id): Path<i64>,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;

    let author = state
        .db
        .get_author(user.id, author_id)
        .await
        .map_err(api_err_to_response)?;

    let works = state
        .db
        .list_works(user.id)
        .await
        .map_err(api_err_to_response)?;

    let author_works: Vec<_> = works
        .iter()
        .filter(|w| w.author_id == Some(author_id))
        .collect();

    let work_ids: Vec<_> = author_works.iter().map(|w| w.id).collect();
    let items = state
        .db
        .list_library_items_by_work_ids(user.id, &work_ids)
        .await
        .map_err(api_err_to_response)?;

    let href = format!("/opds/author/{}", author_id);
    let mut body = feed_header(
        &format!("urn:livrarr:opds:author:{}", author_id),
        &format!("Works by {}", author.name),
        &href,
    );

    for work in &author_works {
        let work_items: Vec<_> = items
            .iter()
            .filter(|i| i.work_id == work.id)
            .cloned()
            .collect();
        if !work_items.is_empty() {
            body.push_str(&work_entry(work, &work_items));
        }
    }

    body.push_str("</feed>\n");
    Ok(xml_response(body))
}

/// GET /opds/search?q= — acquisition feed with search results
pub async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(sq): Query<SearchQuery>,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;
    let query = sq.q.unwrap_or_default();
    let page = sq.page.unwrap_or(1).max(1);

    if query.is_empty() {
        let mut body = feed_header("urn:livrarr:opds:search", "Search", "/opds/search");
        body.push_str("</feed>\n");
        return Ok(xml_response(body));
    }

    // Simple search: filter works by title or author name containing the query.
    let all_works = state
        .db
        .list_works(user.id)
        .await
        .map_err(api_err_to_response)?;

    let query_lower = query.to_lowercase();
    let matching: Vec<_> = all_works
        .into_iter()
        .filter(|w| {
            w.title.to_lowercase().contains(&query_lower)
                || w.author_name.to_lowercase().contains(&query_lower)
        })
        .collect();

    let total = matching.len();
    let start = (page - 1) * PAGE_SIZE;
    let page_works: Vec<_> = matching.into_iter().skip(start).take(PAGE_SIZE).collect();

    let work_ids: Vec<_> = page_works.iter().map(|w| w.id).collect();
    let items = state
        .db
        .list_library_items_by_work_ids(user.id, &work_ids)
        .await
        .map_err(api_err_to_response)?;

    let search_href = format!("/opds/search?q={}", urlencoding::encode(&query));
    let mut body = feed_header(
        "urn:livrarr:opds:search",
        &format!("Search: {}", xml_escape(&query)),
        &search_href,
    );
    body.push_str(&pagination_links(&search_href, page, total));

    for work in &page_works {
        let work_items: Vec<_> = items
            .iter()
            .filter(|i| i.work_id == work.id)
            .cloned()
            .collect();
        if !work_items.is_empty() {
            body.push_str(&work_entry(work, &work_items));
        }
    }

    body.push_str("</feed>\n");
    Ok(xml_response(body))
}

/// GET /opds/osd — OpenSearch descriptor
pub async fn opensearch(
    State(_state): State<AppState>,
    _headers: HeaderMap,
) -> Result<Response, Response> {
    // No auth required for the descriptor itself (some clients fetch it first).
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/">
  <ShortName>Livrarr</ShortName>
  <Description>Search the Livrarr library</Description>
  <Url type="application/atom+xml;profile=opds-catalog" template="/opds/search?q={searchTerms}"/>
</OpenSearchDescription>
"#
    .to_string();

    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/opensearchdescription+xml;charset=utf-8",
        )],
        body,
    )
        .into_response())
}

/// GET /opds/cover/:work_id — serve cover image under Basic Auth
pub async fn cover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(work_id): Path<i64>,
) -> Result<Response, Response> {
    let _user = basic_auth(&state, &headers).await?;

    // Reuse the mediacover logic: read from data_dir/covers/{work_id}.jpg
    let cover_path = state
        .data_dir
        .join("covers")
        .join(format!("{}.jpg", work_id));

    if !cover_path.exists() {
        return Err(StatusCode::NOT_FOUND.into_response());
    }

    let bytes = tokio::fs::read(&cover_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/jpeg")],
        bytes,
    )
        .into_response())
}

/// GET /opds/download/:library_item_id — serve file under Basic Auth
pub async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(item_id): Path<i64>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;

    let path = super::workfile::resolve_file_path(&state.db, user.id, item_id)
        .await
        .map_err(api_err_to_response)?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type = mime_for_ext(&ext);

    use tower::Service;
    use tower_http::services::ServeFile;
    let mut svc = ServeFile::new(&path);
    let resp = svc.call(req).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("File serve error: {e}"),
        )
            .into_response()
    })?;

    let (mut parts, body) = resp.into_response().into_parts();
    parts
        .headers
        .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    Ok(Response::from_parts(parts, body))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn api_err_to_response(e: impl std::fmt::Display) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response()
}
