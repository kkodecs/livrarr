use crate::{MetadataError, ProviderSearchResult};
use quick_xml::events::Event;
use quick_xml::Reader;

// =============================================================================
// Shared XML helpers
// =============================================================================

/// Extract text content between current position and the closing tag.
fn read_text(reader: &mut Reader<&[u8]>) -> String {
    let mut buf = Vec::new();
    let mut text = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(e)) => {
                if let Ok(s) = e.unescape() {
                    text.push_str(&s);
                }
            }
            Ok(Event::CData(e)) => {
                if let Ok(s) = std::str::from_utf8(e.as_ref()) {
                    text.push_str(s);
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    text.trim().to_string()
}

/// Parse an SRU response and extract records using a per-record parser.
/// Handles both inline XML and entity-escaped recordData (string packing).
fn parse_sru_records<F>(
    xml: &[u8],
    record_tag: &[u8],
    mut parse_record: F,
) -> Result<Vec<ProviderSearchResult>, MetadataError>
where
    F: FnMut(&[u8]) -> Option<ProviderSearchResult>,
{
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut results = Vec::new();
    let mut record_depth: u32 = 0;
    let mut record_buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.local_name().as_ref() == record_tag => {
                record_depth += 1;
                if record_depth == 1 {
                    record_buf.clear();
                    record_buf.extend_from_slice(b"<");
                    record_buf.extend_from_slice(record_tag);
                    record_buf.extend_from_slice(b">");
                } else {
                    let mut writer = quick_xml::Writer::new(&mut record_buf);
                    let _ = writer.write_event(Event::Start(e.clone()));
                }
            }
            Ok(Event::End(ref e)) if e.local_name().as_ref() == record_tag && record_depth > 0 => {
                record_depth -= 1;
                if record_depth == 0 {
                    record_buf.extend_from_slice(b"</");
                    record_buf.extend_from_slice(record_tag);
                    record_buf.extend_from_slice(b">");
                    // Some SRU servers (e.g. NDL) use string packing — recordData
                    // contains entity-escaped XML (&lt;dc:title&gt;). Detect and unescape.
                    let parse_buf = if record_buf.windows(4).any(|w| w == b"&lt;") {
                        let s = String::from_utf8_lossy(&record_buf)
                            .replace("&lt;", "<")
                            .replace("&gt;", ">")
                            .replace("&amp;", "&")
                            .replace("&quot;", "\"")
                            .replace("&apos;", "'");
                        s.into_bytes()
                    } else {
                        record_buf.clone()
                    };
                    if let Some(result) = parse_record(&parse_buf) {
                        results.push(result);
                    }
                } else {
                    // Inner record close — capture as content
                    let mut writer = quick_xml::Writer::new(&mut record_buf);
                    let _ = writer.write_event(Event::End(e.clone()));
                }
            }
            Ok(Event::Eof) => break,
            Ok(event) if record_depth > 0 => {
                // Capture raw XML bytes for the record
                let mut writer = quick_xml::Writer::new(&mut record_buf);
                let _ = writer.write_event(event);
            }
            Err(e) => {
                return Err(MetadataError::InvalidResponse(format!(
                    "XML parse error: {e}"
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(results)
}

// =============================================================================
// Dublin Core parser — shared by KB and NDL
// =============================================================================

fn parse_dublin_core_record(
    xml: &[u8],
    source: &str,
    source_type: &str,
    language: &str,
) -> Option<ProviderSearchResult> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut title = None;
    let mut author = None;
    let mut year = None;
    let mut isbn = None;
    let mut publisher = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = std::str::from_utf8(local.as_ref()).unwrap_or("");
                match name {
                    "title" => {
                        let t = read_text(&mut reader);
                        if title.is_none() && !t.is_empty() {
                            title = Some(t);
                        }
                    }
                    // DC: creator. RDF/DNB: preferredName (nested in creator).
                    "creator" | "preferredName" => {
                        let a = read_text(&mut reader);
                        if author.is_none() && !a.is_empty() {
                            author = Some(a);
                        }
                    }
                    // DNB RDF: rdau:P60327 = statement of responsibility (author string)
                    "P60327" => {
                        let a = read_text(&mut reader);
                        if author.is_none() && !a.is_empty() {
                            author = Some(a);
                        }
                    }
                    "date" | "issued" => {
                        let d = read_text(&mut reader);
                        if year.is_none() {
                            year = extract_year(&d);
                        }
                    }
                    // DC: identifier. Also match bibo:isbn13/isbn10 from RDF.
                    "identifier" | "isbn13" | "isbn10" => {
                        let id = read_text(&mut reader);
                        if isbn.is_none() {
                            isbn = extract_isbn(&id);
                        }
                    }
                    "publisher" => {
                        let p = read_text(&mut reader);
                        if publisher.is_none() && !p.is_empty() {
                            publisher = Some(p);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }

    let title = title?;
    Some(ProviderSearchResult {
        provider_key: isbn.clone().unwrap_or_default(),
        title,
        author_name: author,
        year,
        cover_url: None,
        isbn,
        publisher,
        source: source.to_string(),
        source_type: source_type.to_string(),
        language: language.to_string(),
        detail_url: None,
    })
}

// =============================================================================
// MARC XML parser — used by BNE (MARC21) and BnF (UNIMARC/InterXMarc)
// =============================================================================

struct MarcRecord {
    datafields: Vec<MarcDatafield>,
}

struct MarcDatafield {
    tag: String,
    subfields: Vec<(char, String)>,
}

fn parse_marc_xml(xml: &[u8]) -> MarcRecord {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut datafields = Vec::new();
    let mut current_tag = String::new();
    let mut current_subfields = Vec::new();
    let mut current_subfield_code: Option<char>;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local_name = e.local_name();
                let local = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                if local == "datafield" {
                    current_tag = e
                        .attributes()
                        .filter_map(|a| a.ok())
                        .find(|a| a.key.as_ref() == b"tag")
                        .and_then(|a| std::str::from_utf8(&a.value).ok().map(|s| s.to_string()))
                        .unwrap_or_default();
                    current_subfields.clear();
                } else if local == "subfield" {
                    current_subfield_code = e
                        .attributes()
                        .filter_map(|a| a.ok())
                        .find(|a| a.key.as_ref() == b"code")
                        .and_then(|a| {
                            std::str::from_utf8(&a.value)
                                .ok()
                                .and_then(|s| s.chars().next())
                        });
                    let text = read_text(&mut reader);
                    if let Some(code) = current_subfield_code {
                        current_subfields.push((code, text));
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = e.local_name();
                let local = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                if local == "datafield" && !current_tag.is_empty() {
                    datafields.push(MarcDatafield {
                        tag: current_tag.clone(),
                        subfields: current_subfields.clone(),
                    });
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }

    MarcRecord { datafields }
}

impl MarcRecord {
    fn subfield(&self, tag: &str, code: char) -> Option<&str> {
        self.datafields
            .iter()
            .find(|df| df.tag == tag)
            .and_then(|df| {
                df.subfields
                    .iter()
                    .find(|(c, _)| *c == code)
                    .map(|(_, v)| v.as_str())
            })
    }
}

// =============================================================================
// Utility functions
// =============================================================================

fn extract_year(s: &str) -> Option<i32> {
    // Try to find a 4-digit year in the string
    let re_like: Vec<char> = s.chars().collect();
    for i in 0..re_like.len().saturating_sub(3) {
        if re_like[i].is_ascii_digit()
            && re_like[i + 1].is_ascii_digit()
            && re_like[i + 2].is_ascii_digit()
            && re_like[i + 3].is_ascii_digit()
        {
            let year_str: String = re_like[i..i + 4].iter().collect();
            if let Ok(y) = year_str.parse::<i32>() {
                if (1000..=2100).contains(&y) {
                    return Some(y);
                }
            }
        }
    }
    None
}

fn extract_isbn(s: &str) -> Option<String> {
    // Extract ISBN-13 or ISBN-10 from a string
    let digits: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X')
        .collect();
    if (digits.len() == 13 && digits.starts_with("978")) || digits.len() == 10 {
        Some(digits)
    } else {
        None
    }
}

// =============================================================================
// BNE Field Mapper (Spain — MARC21)
// =============================================================================

pub struct BneFieldMapper;

impl super::FieldMapper for BneFieldMapper {
    fn name(&self) -> &str {
        "BNE"
    }

    fn parse_search_results(&self, xml: &[u8]) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        parse_sru_records(xml, b"record", |record_xml| {
            let marc = parse_marc_xml(record_xml);

            let title = marc
                .subfield("245", 'a')?
                .trim_end_matches(['/', ':'])
                .trim()
                .to_string();
            let author = marc
                .subfield("100", 'a')
                .map(|s| s.trim_end_matches(',').trim().to_string());
            let year = marc.subfield("260", 'c').and_then(extract_year);
            let isbn = marc.subfield("020", 'a').and_then(extract_isbn);
            let publisher = marc
                .subfield("260", 'b')
                .map(|s| s.trim_end_matches([',', ';']).trim().to_string());

            Some(ProviderSearchResult {
                provider_key: isbn.clone().unwrap_or_default(),
                title,
                author_name: author,
                year,
                cover_url: None,
                isbn,
                publisher,
                source: "BNE".to_string(),
                source_type: "api".to_string(),
                language: "es".to_string(),
                detail_url: None,
            })
        })
    }
}

// =============================================================================
// BnF Field Mapper (France — UNIMARC/InterXMarc)
// =============================================================================

pub struct BnfFieldMapper;

impl super::FieldMapper for BnfFieldMapper {
    fn name(&self) -> &str {
        "BnF"
    }

    fn parse_search_results(&self, xml: &[u8]) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        parse_sru_records(xml, b"record", |record_xml| {
            let marc = parse_marc_xml(record_xml);

            // UNIMARC fields
            let title = marc.subfield("200", 'a')?.trim().to_string();
            let author = marc.subfield("700", 'a').map(|last| {
                let first = marc.subfield("700", 'b').unwrap_or("");
                if first.is_empty() {
                    last.trim_end_matches(',').trim().to_string()
                } else {
                    format!(
                        "{} {}",
                        first.trim_end_matches(',').trim(),
                        last.trim_end_matches(',').trim()
                    )
                }
            });
            let year = marc.subfield("210", 'd').and_then(extract_year);
            let isbn = marc.subfield("010", 'a').and_then(extract_isbn);
            let publisher = marc.subfield("210", 'c').map(|s| s.trim().to_string());

            Some(ProviderSearchResult {
                provider_key: isbn.clone().unwrap_or_default(),
                title,
                author_name: author,
                year,
                cover_url: None,
                isbn,
                publisher,
                source: "BnF".to_string(),
                source_type: "api".to_string(),
                language: "fr".to_string(),
                detail_url: None,
            })
        })
    }
}

// =============================================================================
// DNB Field Mapper (Germany — RDF/Dublin Core)
// =============================================================================

pub struct DnbFieldMapper;

impl super::FieldMapper for DnbFieldMapper {
    fn name(&self) -> &str {
        "DNB"
    }

    fn parse_search_results(&self, xml: &[u8]) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        // DNB returns RDF with Dublin Core elements
        parse_sru_records(xml, b"record", |record_xml| {
            parse_dublin_core_record(record_xml, "DNB", "api", "de")
        })
    }
}

// =============================================================================
// KB Field Mapper (Netherlands — Dublin Core)
// =============================================================================

pub struct KbFieldMapper;

impl super::FieldMapper for KbFieldMapper {
    fn name(&self) -> &str {
        "KB"
    }

    fn parse_search_results(&self, xml: &[u8]) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        parse_sru_records(xml, b"record", |record_xml| {
            parse_dublin_core_record(record_xml, "KB", "api", "nl")
        })
    }
}

// =============================================================================
// NDL Field Mapper (Japan — DC-NDL / Dublin Core)
// =============================================================================

pub struct NdlFieldMapper;

impl super::FieldMapper for NdlFieldMapper {
    fn name(&self) -> &str {
        "NDL"
    }

    fn parse_search_results(&self, xml: &[u8]) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        parse_sru_records(xml, b"record", |record_xml| {
            parse_dublin_core_record(record_xml, "NDL", "api", "ja")
        })
    }
}
