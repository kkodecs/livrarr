/// Language configuration and supported language definitions.
pub struct LanguageInfo {
    pub code: &'static str,
    pub english_name: &'static str,
    pub provider_name: &'static str,
    pub provider_type: &'static str, // "api" or "llm"
    pub requires_llm: bool,
}

/// All supported languages, in display order (English first, then alphabetical by English name).
pub const SUPPORTED_LANGUAGES: &[LanguageInfo] = &[
    LanguageInfo {
        code: "en",
        english_name: "English",
        provider_name: "OpenLibrary + Hardcover",
        provider_type: "api",
        requires_llm: false,
    },
    LanguageInfo {
        code: "nl",
        english_name: "Dutch",
        provider_name: "Web Search",
        provider_type: "llm",
        requires_llm: true,
    },
    LanguageInfo {
        code: "fr",
        english_name: "French",
        provider_name: "Web Search",
        provider_type: "llm",
        requires_llm: true,
    },
    LanguageInfo {
        code: "de",
        english_name: "German",
        provider_name: "Web Search",
        provider_type: "llm",
        requires_llm: true,
    },
    LanguageInfo {
        code: "it",
        english_name: "Italian",
        provider_name: "Web Search",
        provider_type: "llm",
        requires_llm: true,
    },
    LanguageInfo {
        code: "ja",
        english_name: "Japanese",
        provider_name: "Web Search",
        provider_type: "llm",
        requires_llm: true,
    },
    LanguageInfo {
        code: "ko",
        english_name: "Korean",
        provider_name: "Web Search",
        provider_type: "llm",
        requires_llm: true,
    },
    LanguageInfo {
        code: "pl",
        english_name: "Polish",
        provider_name: "lubimyczytac.pl",
        provider_type: "llm",
        requires_llm: true,
    },
    LanguageInfo {
        code: "es",
        english_name: "Spanish",
        provider_name: "Web Search",
        provider_type: "llm",
        requires_llm: true,
    },
];

pub fn is_supported_language(code: &str) -> bool {
    SUPPORTED_LANGUAGES.iter().any(|l| l.code == code)
}

pub fn language_requires_llm(code: &str) -> bool {
    SUPPORTED_LANGUAGES
        .iter()
        .find(|l| l.code == code)
        .map(|l| l.requires_llm)
        .unwrap_or(false)
}

pub fn get_language_info(code: &str) -> Option<&'static LanguageInfo> {
    SUPPORTED_LANGUAGES.iter().find(|l| l.code == code)
}

/// Validate and normalize a languages array before persisting.
/// - Lowercases all codes
/// - Removes duplicates (preserving order)
/// - Rejects unsupported codes
/// - Ensures "en" is present (injects at position 0 if missing)
/// - Strips LLM-dependent languages if LLM is not configured
pub fn validate_languages(
    languages: &[String],
    llm_configured: bool,
) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    for code in languages {
        let code = code.trim().to_lowercase();
        if code.is_empty() {
            continue;
        }
        if !is_supported_language(&code) {
            return Err(format!("unsupported language code: {code}"));
        }
        if !llm_configured && language_requires_llm(&code) {
            // Auto-strip LLM languages when LLM not configured
            continue;
        }
        if seen.insert(code.clone()) {
            result.push(code);
        }
    }

    // Ensure "en" is always present
    if !result.contains(&"en".to_string()) {
        result.insert(0, "en".to_string());
    }

    Ok(result)
}

/// Read-time repair for legacy data. Ensures "en" is present.
pub fn repair_languages_on_read(languages: &mut Vec<String>) {
    if !languages.contains(&"en".to_string()) {
        languages.insert(0, "en".to_string());
    }
}

/// Returns true if LLM is fully configured (enabled + endpoint + key + model all non-empty).
pub fn is_llm_configured(
    llm_enabled: bool,
    llm_endpoint: Option<&str>,
    llm_api_key: Option<&str>,
    llm_model: Option<&str>,
) -> bool {
    llm_enabled
        && llm_endpoint.is_some_and(|s| !s.is_empty())
        && llm_api_key.is_some_and(|s| !s.is_empty())
        && llm_model.is_some_and(|s| !s.is_empty())
}

pub fn is_foreign_source(metadata_source: Option<&str>) -> bool {
    livrarr_domain::is_foreign_source(metadata_source)
}

pub fn is_foreign_work(metadata_source: Option<&str>, language: Option<&str>) -> bool {
    is_foreign_source(metadata_source) || is_non_english(language)
}

fn is_non_english(language: Option<&str>) -> bool {
    match language.map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if matches!(s.as_str(), "en" | "eng" | "english") => false,
        Some(s) if !s.is_empty() => true,
        _ => false,
    }
}
