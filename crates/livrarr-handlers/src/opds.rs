use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::context::AppContext;
use crate::ApiError;
use livrarr_domain::services::{
    AuthorService, FileService, ManualImportService, WorkFilter, WorkService,
};
use livrarr_domain::{LibraryItem, User, Work};

// ---------------------------------------------------------------------------
// OPDS Basic Auth
// ---------------------------------------------------------------------------

async fn basic_auth<S: AppContext>(state: &S, headers: &HeaderMap) -> Result<User, Response> {
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

    use crate::types::auth::AuthService;
    state
        .auth_service()
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

pub async fn root<S: AppContext>(
    State(state): State<S>,
    headers: HeaderMap,
) -> Result<Response, Response> {
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

pub async fn recent<S: AppContext>(
    State(state): State<S>,
    headers: HeaderMap,
    Query(pq): Query<PageQuery>,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;
    let page = pq.page.unwrap_or(1).max(1);

    let view = state
        .work_service()
        .list_paginated(user.id, page as u32, PAGE_SIZE as u32)
        .await
        .map_err(api_err_to_response)?;

    let mut body = feed_header(
        "urn:livrarr:opds:recent",
        "Recent Additions",
        "/opds/recent",
    );
    body.push_str(&pagination_links("/opds/recent", page, view.total as usize));

    for wv in &view.works {
        if !wv.library_items.is_empty() {
            body.push_str(&work_entry(&wv.work, &wv.library_items));
        }
    }

    body.push_str("</feed>\n");
    Ok(xml_response(body))
}

pub async fn author_list<S: AppContext>(
    State(state): State<S>,
    headers: HeaderMap,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;

    let authors = state
        .author_service()
        .list(user.id)
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

pub async fn author_works<S: AppContext>(
    State(state): State<S>,
    headers: HeaderMap,
    Path(author_id): Path<i64>,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;

    let author = state
        .author_service()
        .get(user.id, author_id)
        .await
        .map_err(api_err_to_response)?;

    let works = state
        .work_service()
        .list(
            user.id,
            WorkFilter {
                author_id: Some(author_id),
                monitored: None,
                enrichment_status: None,
                media_type: None,
                sort_by: None,
                sort_dir: None,
            },
        )
        .await
        .map_err(api_err_to_response)?;

    let work_ids: Vec<_> = works.iter().map(|w| w.id).collect();
    let items = state
        .manual_import_service()
        .list_library_items_by_work_ids(user.id, &work_ids)
        .await
        .map_err(api_err_to_response)?;

    let href = format!("/opds/author/{}", author_id);
    let mut body = feed_header(
        &format!("urn:livrarr:opds:author:{}", author_id),
        &format!("Works by {}", author.name),
        &href,
    );

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

pub async fn search<S: AppContext>(
    State(state): State<S>,
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

    let all_works = state
        .work_service()
        .list(
            user.id,
            WorkFilter {
                author_id: None,
                monitored: None,
                enrichment_status: None,
                media_type: None,
                sort_by: None,
                sort_dir: None,
            },
        )
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
        .manual_import_service()
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

pub async fn opensearch<S: AppContext>(
    State(_state): State<S>,
    _headers: HeaderMap,
) -> Result<Response, Response> {
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

pub async fn cover<S: AppContext>(
    State(state): State<S>,
    headers: HeaderMap,
    Path(work_id): Path<i64>,
) -> Result<Response, Response> {
    let _user = basic_auth(&state, &headers).await?;

    let cover_path = state
        .data_dir()
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

pub async fn download<S: AppContext>(
    State(state): State<S>,
    headers: HeaderMap,
    Path(item_id): Path<i64>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, Response> {
    let user = basic_auth(&state, &headers).await?;

    let path = state
        .file_service()
        .resolve_path(user.id, item_id)
        .await
        .map_err(|e| api_err_to_response(ApiError::from(e)))?;

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
