#![allow(dead_code)]

use librarr_download::*;

// =============================================================================
// extract_torrent_hash — edge cases beyond behavioral tests
// =============================================================================

#[test]
fn extract_hash_btih_hex_mixed_case_is_normalized() {
    // Mixed-case hex in btih should be lowercased.
    let src = TorrentSource::Magnet(
        "magnet:?xt=urn:btih:0123456789ABCDEF0123456789abcdef01234567".into(),
    );
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash, "0123456789abcdef0123456789abcdef01234567");
}

#[test]
fn extract_hash_btih_base32_mixed_case_valid_length() {
    // Valid 32-char base32 with mixed case — uppercased to valid base32 before decode.
    let src = TorrentSource::Magnet("magnet:?xt=urn:btih:mfrggzdfmztwq2lknnwg23tpoi".into());
    let result = extract_torrent_hash(&src);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 40);
}

#[test]
fn extract_hash_btih_base32_invalid_characters_error() {
    let src = TorrentSource::Magnet("magnet:?xt=urn:btih:!!!!".into());
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidMagnet { .. })
    ));
}

#[test]
fn extract_hash_btih_base32_truncated_is_zero_padded() {
    // Short base32 input — implementation pads decoded bytes with zeros to 20 bytes.
    let src = TorrentSource::Magnet("magnet:?xt=urn:btih:MY======".into());
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash.len(), 40);
    // "MY" base32 decodes to [0x66], then zero-padded to 20 bytes
    assert!(hash.starts_with("66"));
    assert!(hash.ends_with("00"));
}

#[test]
fn extract_hash_magnet_missing_xt_with_other_params_errors() {
    let src =
        TorrentSource::Magnet("magnet:?dn=Book&tr=udp://tracker.example.com:80/announce".into());
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidMagnet { .. })
    ));
}

#[test]
fn extract_hash_magnet_xt_not_first_param_is_found() {
    // xt= appearing after other parameters should still be found.
    let src = TorrentSource::Magnet(
        "magnet:?dn=Book&xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(),
    );
    assert_eq!(
        extract_torrent_hash(&src).unwrap(),
        "0123456789abcdef0123456789abcdef01234567"
    );
}

#[test]
fn extract_hash_magnet_xt_with_extra_ampersands() {
    // Repeated separators should not break parsing.
    let src = TorrentSource::Magnet(
        "magnet:?dn=Book&&xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(),
    );
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash, "0123456789abcdef0123456789abcdef01234567");
}

#[test]
fn extract_hash_url_source_always_errors() {
    let src = TorrentSource::Url("https://example.com/file.torrent".into());
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidMagnet { .. })
    ));
}

#[test]
fn extract_hash_magnet_xt_with_unknown_urn_scheme_errors() {
    // xt= present but not btih or btmh.
    let src = TorrentSource::Magnet("magnet:?xt=urn:other:abcdef".into());
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidMagnet { .. })
    ));
}

#[test]
fn extract_hash_btmh_without_1220_prefix() {
    // btmh without the "12" multihash prefix — implementation strips "12" if present.
    let hex64 = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
    let src = TorrentSource::Magnet(format!("magnet:?xt=urn:btmh:{hex64}"));
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash, hex64);
}

#[test]
fn extract_hash_empty_magnet_errors() {
    let src = TorrentSource::Magnet("".into());
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidMagnet { .. })
    ));
}

// =============================================================================
// Torrent file / bencode edge cases
// =============================================================================

#[test]
fn extract_hash_torrent_file_nested_dict() {
    let data = b"d4:infod3:foo3:bar4:nestd3:abc3:defee".to_vec();
    let src = TorrentSource::TorrentFile {
        filename: "nested.torrent".into(),
        data,
    };
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash.len(), 40);
}

#[test]
fn extract_hash_torrent_file_missing_info_dict_errors() {
    let data = b"d3:foo3:bare".to_vec();
    let src = TorrentSource::TorrentFile {
        filename: "missing-info.torrent".into(),
        data,
    };
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidTorrentFile { .. })
    ));
}

#[test]
fn extract_hash_torrent_file_truncated_info_dict_errors() {
    let data = b"d4:infod3:foo3:bar".to_vec();
    let src = TorrentSource::TorrentFile {
        filename: "truncated.torrent".into(),
        data,
    };
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidTorrentFile { .. })
    ));
}

#[test]
fn extract_hash_torrent_file_empty_data_errors() {
    let src = TorrentSource::TorrentFile {
        filename: "empty.torrent".into(),
        data: vec![],
    };
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidTorrentFile { .. })
    ));
}

#[test]
fn extract_hash_torrent_file_info_with_integer_value() {
    // Info dict containing an integer value.
    let data = b"d4:infod6:lengthi12345eee".to_vec();
    let src = TorrentSource::TorrentFile {
        filename: "int.torrent".into(),
        data,
    };
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash.len(), 40);
}

#[test]
fn extract_hash_torrent_file_info_with_list_value() {
    // Info dict containing a list value.
    let data = b"d4:infod5:filesl3:fooeee".to_vec();
    let src = TorrentSource::TorrentFile {
        filename: "list.torrent".into(),
        data,
    };
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash.len(), 40);
}

// =============================================================================
// resolve_remote_path — edge cases
// =============================================================================

#[test]
fn resolve_remote_path_empty_mappings_returns_original() {
    assert_eq!(
        resolve_remote_path("/mnt/books/Title.epub", "host", &[]),
        "/mnt/books/Title.epub"
    );
}

#[test]
fn resolve_remote_path_empty_remote_prefix_does_not_match() {
    // Empty remote_path has length 0; implementation requires len > best_len (initially 0).
    // So empty prefix never wins.
    let mappings = vec![RemotePathMapping {
        id: 1,
        host: "host".into(),
        remote_path: "".into(),
        local_path: "/local".into(),
    }];
    assert_eq!(
        resolve_remote_path("/anything/here", "host", &mappings),
        "/anything/here"
    );
}

#[test]
fn resolve_remote_path_empty_path_with_empty_prefix_no_match() {
    // Empty prefix doesn't match — 0 > 0 is false.
    let mappings = vec![RemotePathMapping {
        id: 1,
        host: "host".into(),
        remote_path: "".into(),
        local_path: "/local".into(),
    }];
    assert_eq!(resolve_remote_path("", "HOST", &mappings), "");
}

#[test]
fn resolve_remote_path_same_length_prefix_first_wins() {
    // Equal-length competing prefixes — implementation keeps first (strict >).
    let mappings = vec![
        RemotePathMapping {
            id: 1,
            host: "host".into(),
            remote_path: "/mnt".into(),
            local_path: "/local1".into(),
        },
        RemotePathMapping {
            id: 2,
            host: "host".into(),
            remote_path: "/mnt".into(),
            local_path: "/local2".into(),
        },
    ];
    assert_eq!(
        resolve_remote_path("/mnt/title.epub", "host", &mappings),
        "/local1/title.epub"
    );
}

#[test]
fn resolve_remote_path_empty_host_matches_empty_client() {
    let mappings = vec![RemotePathMapping {
        id: 1,
        host: "".into(),
        remote_path: "/mnt".into(),
        local_path: "/local".into(),
    }];
    assert_eq!(
        resolve_remote_path("/mnt/file", "", &mappings),
        "/local/file"
    );
}

// =============================================================================
// map_qbit_state — edge cases
// =============================================================================

#[test]
fn map_qbit_state_empty_string_maps_to_warning() {
    assert_eq!(map_qbit_state(""), QueueStatus::Warning);
}

#[test]
fn map_qbit_state_unknown_state_maps_to_warning() {
    assert_eq!(map_qbit_state("some-new-state"), QueueStatus::Warning);
}

#[test]
fn map_qbit_state_case_sensitive_not_normalized() {
    // Uppercase should NOT match — qBit states are case-sensitive.
    assert_eq!(map_qbit_state("DOWNLOADING"), QueueStatus::Warning);
    assert_eq!(map_qbit_state("Error"), QueueStatus::Warning);
}

#[test]
fn map_qbit_state_additional_known_states() {
    // States in the implementation not covered in behavioral tests.
    assert_eq!(map_qbit_state("forcedDL"), QueueStatus::Downloading);
    assert_eq!(map_qbit_state("queuedDL"), QueueStatus::Queued);
    assert_eq!(map_qbit_state("checkingDL"), QueueStatus::Queued);
    assert_eq!(map_qbit_state("checkingResumeData"), QueueStatus::Queued);
    assert_eq!(map_qbit_state("moving"), QueueStatus::Warning);
    assert_eq!(map_qbit_state("unknown"), QueueStatus::Warning);
}

// =============================================================================
// normalize_eta — boundary value tests
// =============================================================================
// Effective rules: v < 0 → None, v >= 8_640_000 → None, otherwise Some(v).
// Rule 3 (v > 365*86400 = 31_536_000) is dead code because 31_536_000 > 8_640_000,
// so rule 2 already catches everything rule 3 would.
// Valid range: [0, 8_639_999].

#[test]
fn normalize_eta_zero_is_preserved() {
    assert_eq!(normalize_eta(Some(0)), Some(0));
}

#[test]
fn normalize_eta_one_is_preserved() {
    assert_eq!(normalize_eta(Some(1)), Some(1));
}

#[test]
fn normalize_eta_max_valid_is_8639999() {
    // Largest valid value: 8,639,999 (just below sentinel).
    assert_eq!(normalize_eta(Some(8_639_999)), Some(8_639_999));
}

#[test]
fn normalize_eta_exactly_sentinel_is_none() {
    assert_eq!(normalize_eta(Some(8_640_000)), None);
}

#[test]
fn normalize_eta_365_days_is_none() {
    // 365 * 86400 = 31,536,000 which is >= 8,640,000 → caught by rule 2.
    assert_eq!(normalize_eta(Some(365 * 86400)), None);
}

#[test]
fn normalize_eta_i64_max_is_none() {
    assert_eq!(normalize_eta(Some(i64::MAX)), None);
}

#[test]
fn normalize_eta_i64_min_is_none() {
    assert_eq!(normalize_eta(Some(i64::MIN)), None);
}

#[test]
fn normalize_eta_rule3_is_dead_code() {
    // Rule 3 checks v > 31,536,000, but rule 2 (v >= 8,640,000) catches all such values first.
    // This test documents the dead code: no value passes rule 2 but fails rule 3.
    // Every value in [0, 8,639,999] is <= 8,639,999 < 31,536,000, so rule 3 never triggers.
    for v in [8_639_999, 100_000, 3600, 1] {
        // All below sentinel (8,640,000) → valid.
        assert_eq!(normalize_eta(Some(v)), Some(v), "expected Some({v})");
    }
}
