use livrarr_http::HttpClient;

/// Validate that an ISBN string contains only digits and an optional trailing X.
/// Rejects any input that could be used for URL injection.
fn is_valid_isbn(isbn: &str) -> bool {
    if isbn.is_empty() {
        return false;
    }
    let bytes = isbn.as_bytes();
    let (body, last) = bytes.split_at(bytes.len() - 1);
    body.iter().all(|b| b.is_ascii_digit())
        && (last[0].is_ascii_digit() || last[0] == b'X' || last[0] == b'x')
}

/// Attempt to resolve a cover image URL from an ISBN using OpenLibrary.
/// Used for English works only.
pub async fn resolve_cover_by_isbn_ol(http: &HttpClient, isbn: Option<&str>) -> Option<String> {
    let isbn = isbn?;
    if !is_valid_isbn(isbn) {
        return None;
    }

    let ol_url = format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false");
    if let Ok(resp) = http.get(&ol_url).send().await {
        if resp.status().is_success() {
            return Some(format!(
                "https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg",
            ));
        }
    }

    None
}

/// Resolve a cover using the Amazon direct ISBN-to-image URL.
/// No API key, no scraping — just URL construction from ISBN-10.
/// Returns the URL directly (Amazon returns 200 for valid ISBNs with covers,
/// or a 1x1 pixel for missing ones — we check Content-Length to filter).
pub async fn resolve_cover_by_isbn_amazon(http: &HttpClient, isbn: Option<&str>) -> Option<String> {
    let isbn = isbn?;
    if !is_valid_isbn(isbn) {
        return None;
    }

    // Amazon needs ISBN-10. Convert ISBN-13 if needed.
    let isbn10 = if isbn.len() == 13 && isbn.starts_with("978") {
        isbn13_to_isbn10(isbn)?
    } else if isbn.len() == 10 {
        isbn.to_string()
    } else {
        return None;
    };

    let url =
        format!("https://images-na.ssl-images-amazon.com/images/P/{isbn10}.01._SCLZZZZZZZ_.jpg");

    if let Ok(resp) = http.get(&url).send().await {
        if resp.status().is_success() {
            // Amazon returns a tiny 1x1 GIF for missing covers — filter by Content-Length.
            // Real covers are >1KB.
            if let Some(len) = resp.content_length() {
                if len > 1000 {
                    return Some(url);
                }
            } else {
                // No Content-Length header — accept it (could be chunked)
                return Some(url);
            }
        }
    }

    None
}

/// Convert ISBN-13 (starting with 978) to ISBN-10.
fn isbn13_to_isbn10(isbn13: &str) -> Option<String> {
    if isbn13.len() != 13 || !isbn13.starts_with("978") {
        return None;
    }
    let body = &isbn13[3..12]; // 9 digits after "978", before check digit
    let sum: u32 = body
        .chars()
        .enumerate()
        .filter_map(|(i, c)| c.to_digit(10).map(|d| d * (10 - i as u32)))
        .sum();
    let check = (11 - (sum % 11)) % 11;
    let check_char = if check == 10 {
        'X'
    } else {
        char::from_digit(check, 10)?
    };
    Some(format!("{body}{check_char}"))
}

/// Resolve a cover using Casa del Libro's predictable ISBN-to-URL pattern.
/// Works for Spanish ISBNs (978-84-...) with very high hit rate.
pub async fn resolve_cover_by_isbn_casadellibro(
    http: &HttpClient,
    isbn: Option<&str>,
) -> Option<String> {
    let isbn = isbn?;
    if !is_valid_isbn(isbn) {
        return None;
    }
    let clean: String = isbn.chars().filter(|c| c.is_ascii_digit()).collect();
    if clean.len() != 13 {
        return None;
    }
    let last2 = &clean[11..13];
    let n = (clean.as_bytes()[12] - b'0') % 10; // last digit mod 10
    let url = format!("https://imagessl{n}.casadellibro.com/a/l/s5/{last2}/{clean}.webp");
    if let Ok(resp) = http.get(&url).send().await {
        if resp.status().is_success() {
            if let Some(len) = resp.content_length() {
                if len > 1000 {
                    return Some(url);
                }
            } else {
                return Some(url);
            }
        }
    }
    None
}

/// Resolve cover for foreign (non-English) works.
/// Chain: Casa del Libro ISBN → Amazon ISBN → nothing.
/// CdL covers are proxied through /api/v1/coverproxy to bypass their CDN browser-blocking.
pub async fn resolve_cover_foreign(http: &HttpClient, isbn: Option<&str>) -> Option<String> {
    if let Some(url) = resolve_cover_by_isbn_casadellibro(http, isbn).await {
        return Some(url);
    }
    resolve_cover_by_isbn_amazon(http, isbn).await
}

/// Resolve cover for English works (existing behavior).
/// Chain: OL ISBN → Amazon ISBN.
pub async fn resolve_cover_english(http: &HttpClient, isbn: Option<&str>) -> Option<String> {
    if let Some(url) = resolve_cover_by_isbn_ol(http, isbn).await {
        return Some(url);
    }
    resolve_cover_by_isbn_amazon(http, isbn).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbn_validation() {
        assert!(is_valid_isbn("9780306406157"));
        assert!(is_valid_isbn("012000030X"));
        assert!(is_valid_isbn("012000030x"));
        assert!(!is_valid_isbn(""));
        assert!(!is_valid_isbn("978-0-306-40615-7")); // hyphens rejected
        assert!(!is_valid_isbn("978030640615X7")); // X not at end
        assert!(!is_valid_isbn("../../../etc/passwd"));
        assert!(!is_valid_isbn("9780306406157&extra=inject"));
    }

    #[test]
    fn isbn13_to_isbn10_valid() {
        assert_eq!(
            isbn13_to_isbn10("9782070612758"),
            Some("2070612759".to_string())
        );
        assert_eq!(
            isbn13_to_isbn10("9783522202022"),
            Some("3522202023".to_string())
        );
    }

    #[test]
    fn isbn13_to_isbn10_with_x_check() {
        // ISBN-13 9780306406157 → ISBN-10 0306406152
        assert_eq!(
            isbn13_to_isbn10("9780306406157"),
            Some("0306406152".to_string())
        );
        // ISBN-13 9780120000302 → ISBN-10 012000030X (check digit = 10 → X)
        assert_eq!(
            isbn13_to_isbn10("9780120000302"),
            Some("012000030X".to_string())
        );
    }

    #[test]
    fn isbn13_to_isbn10_invalid() {
        assert_eq!(isbn13_to_isbn10("1234567890"), None); // too short
        assert_eq!(isbn13_to_isbn10("9791234567890"), None); // 979 prefix
    }
}
