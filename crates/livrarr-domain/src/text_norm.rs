use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;

use crate::title_cleanup::clean_title;

const TITLE_STOPWORDS: &[&str] = &["a", "an", "the", "of", "and", "in", "on", "for", "to"];
const AUTHOR_SUFFIX_STOPWORDS: &[&str] = &["jr", "sr", "iii", "iv"];

pub fn title_tokens(raw: &str) -> HashSet<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return HashSet::new();
    }

    let cleaned = clean_title(trimmed);
    let normalized = strip_combining_marks(&cleaned);
    let lowered = normalized.to_lowercase();

    lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty() && s.chars().count() >= 2)
        .filter(|s| !TITLE_STOPWORDS.contains(s))
        .map(String::from)
        .collect()
}

pub fn author_tokens(raw: &str) -> HashSet<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return HashSet::new();
    }

    let normalized = strip_combining_marks(trimmed);
    let lowered = normalized.to_lowercase();

    lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .filter(|s| !AUTHOR_SUFFIX_STOPWORDS.contains(s))
        .map(String::from)
        .collect()
}

pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    intersection as f64 / union as f64
}

fn strip_combining_marks(s: &str) -> String {
    s.nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect()
}
