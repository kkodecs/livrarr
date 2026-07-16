use std::time::Duration;

use livrarr_db::SeriesCacheEntry;
use livrarr_domain::services::*;

pub(super) async fn llm_clean_series_list<L: LlmCaller + Send + Sync>(
    llm: &L,
    author_name: &str,
    entries: &[SeriesCacheEntry],
) -> Option<Vec<SeriesCacheEntry>> {
    use std::collections::HashMap;

    if entries.is_empty() {
        return None;
    }

    let mut listing = String::new();
    for (i, e) in entries.iter().enumerate() {
        listing.push_str(&format!("{}: \"{}\" ({} books)\n", i, e.name, e.book_count));
    }

    let user_template = format!(
        "These are book series attributed to \"{author_name}\" from Goodreads:\n\n\
         {listing}\n\
         Clean up this list:\n\
         1. REMOVE series by a different person who shares the same name\n\
         2. REMOVE box sets and omnibus editions that repackage books from other series\n\
         3. REMOVE series where this author only contributed a foreword, introduction, or single story\n\
         4. Keep the author's own original series\n\
         5. Fix capitalization: use standard Title Case (e.g. \"night angel\" → \"Night Angel\")\n\n\
         Return a JSON array of objects for series to KEEP: [{{\"i\": 0, \"name\": \"Corrected Name\"}}, ...]\n\
         If the name is already correct, use the original name.\n\
         Return ONLY the JSON array, no other text."
    );

    let mut context = HashMap::new();
    context.insert(LlmField::AuthorName, LlmValue::Text(author_name.into()));
    context.insert(LlmField::BibliographyHtml, LlmValue::Text(listing));

    let req = LlmCallRequest {
        system_template: "You are a librarian assistant. Clean up book series lists.".to_string(),
        user_template,
        context,
        allowed_fields: &[LlmField::AuthorName, LlmField::BibliographyHtml],
        timeout: Duration::from_secs(15),
        purpose: LlmPurpose::BibliographyCleanup,
    };

    let resp = llm.call(req).await.ok()?;

    let json_str = crate::strip_llm_fences(&resp.content);

    #[derive(serde::Deserialize)]
    struct KeepEntry {
        i: usize,
        name: Option<String>,
    }

    let cleaned: Vec<SeriesCacheEntry> =
        if let Ok(entries_with_names) = serde_json::from_str::<Vec<KeepEntry>>(json_str) {
            entries_with_names
                .into_iter()
                .filter_map(|ke| {
                    entries.get(ke.i).map(|orig| SeriesCacheEntry {
                        name: ke.name.unwrap_or_else(|| orig.name.clone()),
                        gr_key: orig.gr_key.clone(),
                        book_count: orig.book_count,
                        language: orig.language.clone(),
                    })
                })
                .collect()
        } else if let Ok(indices) = serde_json::from_str::<Vec<usize>>(json_str) {
            indices
                .into_iter()
                .filter_map(|i| entries.get(i).cloned())
                .collect()
        } else {
            return None;
        };

    if cleaned.is_empty() {
        return None;
    }

    Some(cleaned)
}
