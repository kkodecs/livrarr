//! ARCHIVED: LLM-based filename parsing.
//!
//! Replaced by the deterministic matching engine (M1+M2+M3) in 2026-04-12.
//! Kept for potential future resurrection as an opt-in feature for users
//! with local LLM models or who accept the privacy tradeoff.
//!
//! To re-enable: integrate as an optional M0 method in the matching pipeline,
//! gated behind a user setting. Never send filenames to external APIs by default.
//!
//! This module is NOT compiled — it exists as reference code only.

#![allow(dead_code, unused_imports)]

// ---------------------------------------------------------------------------
// LLM filename parsing (archived)
// ---------------------------------------------------------------------------

/// Returns (parsed_files_by_index, sort_order).
/// sort_order is the LLM's recommended display order as a vec of original indices.
async fn llm_parse_filenames(
    state: &AppState,
    filenames: &[String],
) -> (Vec<Option<ParsedFile>>, Vec<usize>) {
    let default_order: Vec<usize> = (0..filenames.len()).collect();

    let cfg: livrarr_db::MetadataConfig = match state.db.get_metadata_config().await {
        Ok(c) => c,
        Err(_) => return (vec![None; filenames.len()], default_order),
    };

    let endpoint = match cfg.llm_endpoint.as_deref().filter(|s| !s.is_empty()) {
        Some(e) => e.to_string(),
        None => return (vec![None; filenames.len()], (0..filenames.len()).collect()),
    };
    let api_key = match cfg.llm_api_key.as_deref().filter(|s| !s.is_empty()) {
        Some(k) => k.to_string(),
        None => return (vec![None; filenames.len()], (0..filenames.len()).collect()),
    };
    let model = match cfg.llm_model.as_deref().filter(|s| !s.is_empty()) {
        Some(m) => m.to_string(),
        None => return (vec![None; filenames.len()], (0..filenames.len()).collect()),
    };

    let mut all_parsed: Vec<Option<ParsedFile>> = vec![None; filenames.len()];
    let mut sort_order: Vec<usize> = Vec::new();

    // Process in batches.
    for chunk_start in (0..filenames.len()).step_by(LLM_BATCH_SIZE) {
        let chunk_end = (chunk_start + LLM_BATCH_SIZE).min(filenames.len());
        let chunk = &filenames[chunk_start..chunk_end];

        match llm_parse_batch(&state.http_client, &endpoint, &api_key, &model, chunk).await {
            Some(parsed) => {
                for (idx, p) in parsed {
                    let abs_idx = chunk_start + idx;
                    if abs_idx < all_parsed.len() {
                        all_parsed[abs_idx] = Some(p);
                        sort_order.push(abs_idx);
                    }
                }
            }
            None => {
                warn!(
                    "manual import: LLM batch failed for files {}-{}",
                    chunk_start, chunk_end
                );
            }
        }
    }

    // Add any files the LLM didn't return (failed parse) at the end.
    for i in 0..filenames.len() {
        if !sort_order.contains(&i) {
            sort_order.push(i);
        }
    }

    (all_parsed, sort_order)
}

async fn llm_parse_batch(
    http: &livrarr_http::HttpClient,
    endpoint: &str,
    api_key: &str,
    model: &str,
    filenames: &[String],
) -> Option<Vec<(usize, ParsedFile)>> {
    let mut listing = String::new();
    for (i, name) in filenames.iter().enumerate() {
        listing.push_str(&format!("{i}: \"{name}\"\n"));
    }

    let prompt = format!(
        "These are ebook/audiobook filenames:\n\n\
         {listing}\n\
         Extract the author name, title, and series info from each filename.\n\
         Order the results in the most logical way for a reader — \
         group by author, then by series order, then standalone works alphabetically.\n\n\
         Return a JSON array. Each entry: {{\"idx\": <original index>, \"author\": \"<author name>\", \
         \"title\": \"<book title>\", \"series\": \"<series name or null>\", \
         \"position\": <number or null>}}\n\n\
         Return ONLY the JSON array, no other text."
    );

    let url = format!(
        "{}chat/completions",
        endpoint.trim_end_matches('/').to_owned() + "/"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 4000,
        "temperature": 0.0,
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    let answer = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    // Robust JSON extraction: find first '[' and last ']' to handle preamble text.
    let start = answer.find('[').unwrap_or(0);
    let end = answer.rfind(']').map(|e| e + 1).unwrap_or(answer.len());
    let json_str = if start < end {
        &answer[start..end]
    } else {
        answer
    };

    let entries: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;

    let parsed: Vec<(usize, ParsedFile)> = entries
        .iter()
        .filter_map(|entry| {
            let idx = entry.get("idx")?.as_u64()? as usize;
            let author = entry.get("author")?.as_str()?.to_string();
            let title = entry.get("title")?.as_str()?.to_string();
            let series = entry
                .get("series")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let position = entry.get("position").and_then(|v| v.as_f64());

            Some((
                idx,
                ParsedFile {
                    author,
                    title,
                    series,
                    series_position: position,
                },
            ))
        })
        .collect();

    if parsed.is_empty() {
        return None;
    }

    Some(parsed)
}
