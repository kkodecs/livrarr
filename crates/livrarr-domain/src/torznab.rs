use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, Default)]
pub struct TorznabItem {
    pub title: String,
    pub guid: String,
    pub download_url: String,
    pub size: i64,
    pub seeders: Option<i32>,
    pub leechers: Option<i32>,
    pub publish_date: Option<String>,
    pub categories: Vec<i32>,
    pub enclosure_type: Option<String>,
}

pub enum TorznabParseResult {
    Items(Vec<TorznabItem>),
    Error { code: i32, description: String },
}

pub fn parse_torznab_xml(xml: &[u8]) -> Result<TorznabParseResult, String> {
    let xml_str = std::str::from_utf8(xml).map_err(|e| format!("invalid UTF-8: {e}"))?;
    let mut reader = Reader::from_str(xml_str);
    reader.config_mut().trim_text(true);

    let mut items = Vec::new();
    let mut in_item = false;
    let mut item = TorznabItem::default();
    let mut current_tag: Vec<u8> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(ref event @ (Event::Start(_) | Event::Empty(_))) => {
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let local = e.local_name();
                let is_start = matches!(event, Event::Start(_));

                match local.as_ref() {
                    b"error" => {
                        let code = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.local_name().as_ref() == b"code")
                            .and_then(|a| a.unescape_value().ok()?.parse::<i32>().ok())
                            .unwrap_or(0);
                        let desc = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.local_name().as_ref() == b"description")
                            .and_then(|a| a.unescape_value().ok().map(|v| v.to_string()))
                            .unwrap_or_default();
                        return Ok(TorznabParseResult::Error {
                            code,
                            description: desc,
                        });
                    }
                    b"item" if is_start => {
                        in_item = true;
                        item = TorznabItem::default();
                    }
                    b"enclosure" if in_item => {
                        for attr in e.attributes().flatten() {
                            match attr.key.local_name().as_ref() {
                                b"url" => {
                                    if let Ok(val) = attr.unescape_value() {
                                        item.download_url = val.into_owned();
                                    }
                                }
                                b"length" if item.size == 0 => {
                                    if let Ok(val) = attr.unescape_value() {
                                        item.size = val.parse().unwrap_or(0);
                                    }
                                }
                                b"type" => {
                                    if let Ok(val) = attr.unescape_value() {
                                        item.enclosure_type = Some(val.into_owned());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    b"attr" if in_item => {
                        let mut attr_name = String::new();
                        let mut attr_value = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.local_name().as_ref() {
                                b"name" => {
                                    if let Ok(v) = attr.unescape_value() {
                                        attr_name = v.into_owned();
                                    }
                                }
                                b"value" => {
                                    if let Ok(v) = attr.unescape_value() {
                                        attr_value = v.into_owned();
                                    }
                                }
                                _ => {}
                            }
                        }
                        match attr_name.as_str() {
                            "seeders" => item.seeders = attr_value.parse().ok(),
                            "peers" | "leechers" => {
                                if item.leechers.is_none() {
                                    item.leechers = attr_value.parse().ok();
                                }
                            }
                            "size" if item.size == 0 => {
                                item.size = attr_value.parse().unwrap_or(0);
                            }
                            "category" => {
                                if let Ok(cat) = attr_value.parse::<i32>() {
                                    item.categories.push(cat);
                                }
                            }
                            _ => {}
                        }
                    }
                    _ if in_item && is_start => {
                        current_tag = local.as_ref().to_vec();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) if in_item => {
                if let Ok(text) = e.unescape() {
                    handle_xml_text(&text, &current_tag, &mut item);
                }
            }
            Ok(Event::CData(ref e)) if in_item => {
                if let Ok(text) = std::str::from_utf8(e.as_ref()) {
                    handle_xml_text(text, &current_tag, &mut item);
                }
            }
            Ok(Event::End(ref e)) => {
                if e.local_name().as_ref() == b"item" && in_item {
                    in_item = false;
                    items.push(std::mem::take(&mut item));
                }
                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    Ok(TorznabParseResult::Items(items))
}

#[inline(always)]
fn handle_xml_text(text: &str, current_tag: &[u8], item: &mut TorznabItem) {
    match current_tag {
        b"title" => item.title.push_str(text),
        b"guid" => item.guid.push_str(text),
        b"link" if item.download_url.is_empty() => item.download_url.push_str(text),
        b"size" if item.size == 0 => {
            item.size = text.parse().unwrap_or(0);
        }
        b"pubDate" => {
            item.publish_date
                .get_or_insert_with(String::new)
                .push_str(text);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_entities_are_decoded() {
        let xml = br#"<?xml version="1.0"?>
<rss><channel>
<item>
  <title>Test Book</title>
  <guid>abc123</guid>
  <enclosure url="https://example.com/dl?id=1&amp;token=xyz&amp;cat=7020" length="1024" type="application/x-bittorrent"/>
</item>
</channel></rss>"#;
        let result = parse_torznab_xml(xml).unwrap();
        let items = match result {
            TorznabParseResult::Items(items) => items,
            TorznabParseResult::Error { .. } => panic!("unexpected error"),
        };
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].download_url,
            "https://example.com/dl?id=1&token=xyz&cat=7020"
        );
    }

    #[test]
    fn cdata_title_is_extracted() {
        let xml = br#"<?xml version="1.0"?>
<rss><channel>
<item>
  <title><![CDATA[A Book & More]]></title>
  <guid>def456</guid>
  <link>https://example.com/dl</link>
</item>
</channel></rss>"#;
        let result = parse_torznab_xml(xml).unwrap();
        let items = match result {
            TorznabParseResult::Items(items) => items,
            TorznabParseResult::Error { .. } => panic!("unexpected error"),
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "A Book & More");
        assert_eq!(items[0].download_url, "https://example.com/dl");
    }

    #[test]
    fn enclosure_type_nzb_detected() {
        let xml = br#"<?xml version="1.0"?>
<rss><channel>
<item>
  <title>Usenet Book</title>
  <guid>nzb001</guid>
  <enclosure url="https://nzb.example.com/get/123" length="5000" type="application/x-nzb"/>
</item>
</channel></rss>"#;
        let result = parse_torznab_xml(xml).unwrap();
        let items = match result {
            TorznabParseResult::Items(items) => items,
            TorznabParseResult::Error { .. } => panic!("unexpected error"),
        };
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].enclosure_type.as_deref(),
            Some("application/x-nzb")
        );
        assert_eq!(items[0].size, 5000);
    }

    #[test]
    fn optional_fields_missing() {
        let xml = br#"<?xml version="1.0"?>
<rss><channel>
<item>
  <title>Minimal</title>
  <guid>min001</guid>
  <link>https://example.com/dl</link>
</item>
</channel></rss>"#;
        let result = parse_torznab_xml(xml).unwrap();
        let items = match result {
            TorznabParseResult::Items(items) => items,
            TorznabParseResult::Error { .. } => panic!("unexpected error"),
        };
        assert_eq!(items.len(), 1);
        assert!(items[0].seeders.is_none());
        assert!(items[0].leechers.is_none());
        assert!(items[0].publish_date.is_none());
        assert_eq!(items[0].size, 0);
    }

    #[test]
    fn error_response_parsed() {
        let xml = br#"<?xml version="1.0"?>
<error code="100" description="Incorrect user credentials"/>"#;
        let result = parse_torznab_xml(xml).unwrap();
        match result {
            TorznabParseResult::Error { code, description } => {
                assert_eq!(code, 100);
                assert_eq!(description, "Incorrect user credentials");
            }
            TorznabParseResult::Items(_) => panic!("expected error"),
        }
    }

    #[test]
    fn empty_feed() {
        let xml = br#"<?xml version="1.0"?>
<rss><channel></channel></rss>"#;
        let result = parse_torznab_xml(xml).unwrap();
        let items = match result {
            TorznabParseResult::Items(items) => items,
            TorznabParseResult::Error { .. } => panic!("unexpected error"),
        };
        assert!(items.is_empty());
    }

    #[test]
    fn malformed_xml_returns_err() {
        let xml = b"<rss><channel><item><title>broken</channel></rss>";
        let result = parse_torznab_xml(xml);
        assert!(result.is_err());
    }

    #[test]
    fn torznab_attrs_parsed() {
        let xml = br#"<?xml version="1.0"?>
<rss><channel>
<item>
  <title>Full Item</title>
  <guid>full001</guid>
  <enclosure url="https://example.com/dl" length="2048" type="application/x-bittorrent"/>
  <pubDate>Sat, 19 Apr 2026 12:00:00 +0000</pubDate>
  <torznab:attr name="seeders" value="42"/>
  <torznab:attr name="peers" value="10"/>
  <torznab:attr name="category" value="7020"/>
  <torznab:attr name="category" value="3030"/>
</item>
</channel></rss>"#;
        let result = parse_torznab_xml(xml).unwrap();
        let items = match result {
            TorznabParseResult::Items(items) => items,
            TorznabParseResult::Error { .. } => panic!("unexpected error"),
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].seeders, Some(42));
        assert_eq!(items[0].leechers, Some(10));
        assert_eq!(items[0].categories, vec![7020, 3030]);
        assert_eq!(
            items[0].publish_date.as_deref(),
            Some("Sat, 19 Apr 2026 12:00:00 +0000")
        );
    }

    #[test]
    fn incomplete_items_are_returned_for_caller_filtering() {
        let xml = br#"<?xml version="1.0"?>
<rss><channel>
<item>
  <title>No GUID</title>
  <link>https://example.com/dl</link>
</item>
<item>
  <title>No URL</title>
  <guid>nourl001</guid>
</item>
<item>
  <title>Valid</title>
  <guid>valid001</guid>
  <link>https://example.com/dl</link>
</item>
</channel></rss>"#;
        let result = parse_torznab_xml(xml).unwrap();
        let items = match result {
            TorznabParseResult::Items(items) => items,
            TorznabParseResult::Error { .. } => panic!("unexpected error"),
        };
        assert_eq!(items.len(), 3);
        assert!(items[0].guid.is_empty());
        assert!(items[1].download_url.is_empty());
        assert_eq!(items[2].guid, "valid001");
    }
}
