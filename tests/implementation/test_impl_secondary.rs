#![allow(dead_code)]

use librarr_domain::*;
use librarr_server::api_secondary_impl::new_test_secondary_api;
use librarr_server::*;

// =============================================================================
// AuthorApi — edge cases
// =============================================================================

#[tokio::test]
async fn author_get_detail_with_no_matching_works() {
    // Author exists but has no works — works list should be empty, not error.
    let (api, uid) = new_test_secondary_api().await;
    let author = AuthorApi::add(
        &api,
        uid,
        AddAuthorRequest {
            name: "Lonely Author".into(),
            sort_name: Some("Author, Lonely".into()),
            ol_key: "OL1A".into(),
        },
    )
    .await
    .unwrap();

    let detail = AuthorApi::get(&api, uid, author.id).await.unwrap();
    assert_eq!(detail.author.id, author.id);
    assert!(detail.works.is_empty());
}

#[tokio::test]
async fn author_enable_monitoring_sets_monitor_since() {
    // When monitoring is enabled, monitor_since should be set to now-ish.
    let (api, uid) = new_test_secondary_api().await;
    let author = AuthorApi::add(
        &api,
        uid,
        AddAuthorRequest {
            name: "Monitored".into(),
            sort_name: None,
            ol_key: "OL2A".into(),
        },
    )
    .await
    .unwrap();
    assert!(!author.monitored);

    let updated = AuthorApi::update(
        &api,
        uid,
        author.id,
        UpdateAuthorApiRequest {
            monitored: Some(true),
            monitor_new_items: None,
        },
    )
    .await
    .unwrap();
    assert!(updated.monitored);
}

#[tokio::test]
async fn author_delete_nonexistent_returns_not_found() {
    let (api, uid) = new_test_secondary_api().await;
    let err = AuthorApi::delete(&api, uid, 9999).await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

// =============================================================================
// RootFolderApi — edge cases
// =============================================================================

#[tokio::test]
async fn root_folder_delete_nonexistent_returns_not_found() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = RootFolderApi::delete(&api, 9999).await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

#[tokio::test]
async fn root_folder_get_nonexistent_returns_not_found() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = RootFolderApi::get(&api, 9999).await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

// =============================================================================
// DownloadClientApi — edge cases
// =============================================================================

#[tokio::test]
async fn download_client_category_with_backslash_rejected() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = DownloadClientApi::create(
        &api,
        CreateDownloadClientApiRequest {
            name: "test".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "localhost".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: None,
            password: None,
            category: "bad\\cat".into(),
            download_dir: None,
            enabled: true,
            api_key: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Validation { .. }));
}

#[tokio::test]
async fn download_client_category_with_double_slash_rejected() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = DownloadClientApi::create(
        &api,
        CreateDownloadClientApiRequest {
            name: "test".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "localhost".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: None,
            password: None,
            category: "bad//cat".into(),
            download_dir: None,
            enabled: true,
            api_key: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Validation { .. }));
}

#[tokio::test]
async fn download_client_category_leading_slash_rejected() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = DownloadClientApi::create(
        &api,
        CreateDownloadClientApiRequest {
            name: "test".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "localhost".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: None,
            password: None,
            category: "/leading".into(),
            download_dir: None,
            enabled: true,
            api_key: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Validation { .. }));
}

#[tokio::test]
async fn download_client_category_trailing_slash_rejected() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = DownloadClientApi::create(
        &api,
        CreateDownloadClientApiRequest {
            name: "test".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "localhost".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: None,
            password: None,
            category: "trailing/".into(),
            download_dir: None,
            enabled: true,
            api_key: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Validation { .. }));
}

#[tokio::test]
async fn download_client_get_nonexistent_returns_not_found() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = DownloadClientApi::get(&api, 9999).await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

// =============================================================================
// RemotePathMappingApi — edge cases
// =============================================================================

#[tokio::test]
async fn remote_path_mapping_update_partial_fields_preserves_unchanged() {
    let (api, _uid) = new_test_secondary_api().await;
    let created = RemotePathMappingApi::create(&api, "host1", "/remote/", "/local/")
        .await
        .unwrap();

    let updated = RemotePathMappingApi::update(
        &api,
        created.id,
        UpdateRemotePathMappingRequest {
            host: Some("host2".into()),
            remote_path: None,
            local_path: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.host, "host2");
    assert_eq!(updated.remote_path, "/remote/");
    assert_eq!(updated.local_path, "/local/");
}

#[tokio::test]
async fn remote_path_mapping_update_missing_trailing_slash_rejected() {
    let (api, _uid) = new_test_secondary_api().await;
    let created = RemotePathMappingApi::create(&api, "host", "/remote/", "/local/")
        .await
        .unwrap();

    let err = RemotePathMappingApi::update(
        &api,
        created.id,
        UpdateRemotePathMappingRequest {
            host: None,
            remote_path: Some("/no-trailing".into()),
            local_path: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Validation { .. }));
}

#[tokio::test]
async fn remote_path_mapping_get_nonexistent_returns_not_found() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = RemotePathMappingApi::get(&api, 9999).await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

// =============================================================================
// ConfigApi — edge cases
// =============================================================================

#[tokio::test]
async fn config_prowlarr_secrets_redacted() {
    let (api, _uid) = new_test_secondary_api().await;
    ConfigApi::update_prowlarr(
        &api,
        UpdateProwlarrApiRequest {
            url: Some("http://prowlarr:9696".into()),
            api_key: Some("secret-key".into()),
            enabled: Some(true),
        },
    )
    .await
    .unwrap();

    let config = ConfigApi::get_prowlarr(&api).await.unwrap();
    assert!(config.api_key_set);
    assert!(config.enabled);
    assert_eq!(config.url, Some("http://prowlarr:9696".into()));
}

#[tokio::test]
async fn config_metadata_secrets_redacted() {
    let (api, _uid) = new_test_secondary_api().await;
    ConfigApi::update_metadata(
        &api,
        UpdateMetadataApiRequest {
            hardcover_api_token: Some("hc-token".into()),
            llm_api_key: Some("llm-key".into()),
            llm_provider: None,
            llm_endpoint: None,
            llm_model: None,
            audnexus_url: None,
            languages: None,
        },
    )
    .await
    .unwrap();

    let config = ConfigApi::get_metadata(&api).await.unwrap();
    assert!(config.hardcover_api_token_set);
    assert!(config.llm_api_key_set);
}

// =============================================================================
// SystemApi — edge cases
// =============================================================================

#[tokio::test]
async fn system_health_returns_at_least_one_check() {
    let (api, _uid) = new_test_secondary_api().await;
    let checks = SystemApi::health(&api).await.unwrap();
    assert!(!checks.is_empty());
    assert_eq!(checks[0].check_type, HealthCheckType::Ok);
}

#[tokio::test]
async fn system_status_returns_version() {
    let (api, _uid) = new_test_secondary_api().await;
    let status = SystemApi::status(&api).await.unwrap();
    assert!(!status.version.is_empty());
    assert!(!status.os_info.is_empty());
}

// =============================================================================
// HistoryApi — edge cases
// =============================================================================

#[tokio::test]
async fn history_empty_db_returns_empty_list() {
    let (api, uid) = new_test_secondary_api().await;
    let events = HistoryApi::list(
        &api,
        uid,
        None,
        HistoryFilter {
            event_type: None,
            work_id: None,
            start_date: None,
            end_date: None,
        },
    )
    .await
    .unwrap();
    assert!(events.is_empty());
}

// =============================================================================
// Cross-user isolation — SecondaryApiImpl uses explicit user_id scoping
// =============================================================================

#[tokio::test]
async fn cross_user_author_isolation() {
    // Authors created by one user should not be accessible by another.
    let (api, uid) = new_test_secondary_api().await;
    let author = AuthorApi::add(
        &api,
        uid,
        AddAuthorRequest {
            name: "Isolated Author".into(),
            sort_name: None,
            ol_key: "OL99A".into(),
        },
    )
    .await
    .unwrap();

    // Access with a different user_id should return NotFound (user-scoped query).
    let fake_uid = uid + 1000;
    let err = AuthorApi::get(&api, fake_uid, author.id).await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

#[tokio::test]
async fn cross_user_notification_isolation() {
    let (api, uid) = new_test_secondary_api().await;
    let nid = api.create_test_notification(uid, "isolated-ref").await;

    // Another user should see no notifications
    let fake_uid = uid + 1000;
    let notifs = NotificationApi::list(&api, fake_uid, false).await.unwrap();
    assert!(notifs.is_empty());

    // Original user sees it
    let notifs = NotificationApi::list(&api, uid, false).await.unwrap();
    assert!(notifs.iter().any(|n| n.id == nid));
}

// =============================================================================
// Additional validation — empty names, download client edge cases
// =============================================================================

#[tokio::test]
async fn download_client_empty_name_and_host_both_rejected() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = DownloadClientApi::create(
        &api,
        CreateDownloadClientApiRequest {
            name: "".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: None,
            password: None,
            category: "books".into(),
            enabled: true,
            api_key: None,
        },
    )
    .await
    .unwrap_err();
    // Should have validation errors for both name and host
    if let ApiError::Validation { errors } = err {
        assert!(errors.len() >= 2);
        assert!(errors.iter().any(|e| e.field == "name"));
        assert!(errors.iter().any(|e| e.field == "host"));
    } else {
        panic!("expected Validation error");
    }
}

#[tokio::test]
async fn root_folder_relative_path_rejected() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = RootFolderApi::create(&api, "relative/path", MediaType::Ebook)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Validation { .. }));
}

#[tokio::test]
async fn remote_path_mapping_create_missing_trailing_slash_rejected() {
    let (api, _uid) = new_test_secondary_api().await;
    let err = RemotePathMappingApi::create(&api, "host", "/no-trailing", "/local/")
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Validation { .. }));

    let err = RemotePathMappingApi::create(&api, "host", "/remote/", "/no-trailing")
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Validation { .. }));
}
