use std::time::Duration;

use livrarr_domain::services::*;
use livrarr_domain::*;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrAutocompleteBook {
    author: Option<GrAutocompleteAuthor>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrAutocompleteAuthor {
    #[serde(deserialize_with = "de_stringish_id")]
    id: String,
    name: String,
    #[serde(default)]
    profile_url: String,
    #[allow(dead_code)] // deserialized from Goodreads but not consumed
    #[serde(default)]
    is_goodreads_author: bool,
}

fn de_stringish_id<'de, D>(de: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Id {
        Num(i64),
        Str(String),
    }
    use serde::Deserialize as _;
    match Id::deserialize(de)? {
        Id::Num(n) => Ok(n.to_string()),
        Id::Str(s) => Ok(s),
    }
}

pub(super) async fn resolve_gr_candidates_json<F: HttpFetcher>(
    fetcher: &F,
    author_name: &str,
) -> Vec<GrAuthorCandidateView> {
    let clean_name: String = author_name
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let url = format!(
        "https://www.goodreads.com/book/auto_complete?format=json&q={}",
        urlencoding::encode(clean_name.trim())
    );

    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: Duration::from_secs(10),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 512 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Browser,
        priority: RequestPriority::Normal,
    };

    let resp = match fetcher.fetch(req).await {
        Ok(r) if r.status == 200 => r,
        _ => return Vec::new(),
    };

    let items: Vec<GrAutocompleteBook> = match serde_json::from_slice(&resp.body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for item in items {
        let Some(author) = item.author else {
            continue;
        };
        if author.name.trim().is_empty() {
            continue;
        }
        if !seen.insert(author.id.clone()) {
            continue;
        }
        out.push(GrAuthorCandidateView {
            gr_key: author.id,
            name: author.name,
            profile_url: author.profile_url,
        });
    }

    out
}
