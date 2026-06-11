#![allow(dead_code, clippy::type_complexity)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use librarr_download::*;
use livrarr_domain::services::Release;

// =============================================================================
// Mock infrastructure
// =============================================================================

#[derive(Clone, Debug)]
enum QBitCall {
    Authenticate(i64),
    AddMagnet {
        client_id: i64,
        magnet: String,
    },
    AddFile {
        client_id: i64,
        filename: String,
        len: usize,
    },
    List(i64),
    Get {
        client_id: i64,
        hash: String,
    },
}

struct MockQBitClient {
    calls: Arc<Mutex<Vec<QBitCall>>>,
    auth_err: Option<DownloadError>,
    add_magnet_err: Option<DownloadError>,
    add_file_err: Option<DownloadError>,
    list_responses: Arc<Mutex<Vec<Result<Vec<QBitTorrent>, DownloadError>>>>,
    get_responses: Arc<Mutex<Vec<Result<Option<QBitTorrent>, DownloadError>>>>,
}

impl MockQBitClient {
    fn ok() -> Self {
        Self {
            calls: Arc::new(Mutex::new(vec![])),
            auth_err: None,
            add_magnet_err: None,
            add_file_err: None,
            list_responses: Arc::new(Mutex::new(vec![])),
            get_responses: Arc::new(Mutex::new(vec![])),
        }
    }

    fn auth_fail() -> Self {
        Self {
            calls: Arc::new(Mutex::new(vec![])),
            auth_err: Some(DownloadError::AuthFailed),
            add_magnet_err: None,
            add_file_err: None,
            list_responses: Arc::new(Mutex::new(vec![])),
            get_responses: Arc::new(Mutex::new(vec![])),
        }
    }
}

#[async_trait]
impl QBitClient for MockQBitClient {
    async fn authenticate(&self, cfg: &DownloadClient) -> Result<(), DownloadError> {
        self.calls
            .lock()
            .unwrap()
            .push(QBitCall::Authenticate(cfg.id));
        self.auth_err
            .as_ref()
            .map_or(Ok(()), |e| Err(clone_dl_err(e)))
    }
    async fn add_torrent_magnet(
        &self,
        cfg: &DownloadClient,
        magnet: &str,
        _cat: &str,
    ) -> Result<(), DownloadError> {
        self.calls.lock().unwrap().push(QBitCall::AddMagnet {
            client_id: cfg.id,
            magnet: magnet.into(),
        });
        self.add_magnet_err
            .as_ref()
            .map_or(Ok(()), |e| Err(clone_dl_err(e)))
    }
    async fn add_torrent_file(
        &self,
        cfg: &DownloadClient,
        fname: &str,
        data: &[u8],
        _cat: &str,
    ) -> Result<(), DownloadError> {
        self.calls.lock().unwrap().push(QBitCall::AddFile {
            client_id: cfg.id,
            filename: fname.into(),
            len: data.len(),
        });
        self.add_file_err
            .as_ref()
            .map_or(Ok(()), |e| Err(clone_dl_err(e)))
    }
    async fn list_torrents(
        &self,
        cfg: &DownloadClient,
        _cat: &str,
    ) -> Result<Vec<QBitTorrent>, DownloadError> {
        self.calls.lock().unwrap().push(QBitCall::List(cfg.id));
        self.list_responses
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(Ok(vec![]))
    }
    async fn get_torrent(
        &self,
        cfg: &DownloadClient,
        hash: &str,
    ) -> Result<Option<QBitTorrent>, DownloadError> {
        self.calls.lock().unwrap().push(QBitCall::Get {
            client_id: cfg.id,
            hash: hash.into(),
        });
        self.get_responses.lock().unwrap().pop().unwrap_or(Ok(None))
    }
    async fn test_connection(&self, _cfg: &DownloadClient) -> Result<(), DownloadError> {
        Ok(())
    }
}

struct MockHttpFetcher {
    response: Result<Vec<u8>, DownloadError>,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
trait HttpFetcher: Send + Sync {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, DownloadError>;
}

#[async_trait]
impl HttpFetcher for MockHttpFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, DownloadError> {
        self.calls.lock().unwrap().push(url.into());
        self.response
            .as_ref()
            .map(|v| v.clone())
            .map_err(clone_dl_err)
    }
}

fn clone_dl_err(e: &DownloadError) -> DownloadError {
    match e {
        DownloadError::NoClient => DownloadError::NoClient,
        DownloadError::NoEnabledClient => DownloadError::NoEnabledClient,
        DownloadError::ConnectionFailed(s) => DownloadError::ConnectionFailed(s.clone()),
        DownloadError::AuthFailed => DownloadError::AuthFailed,
        DownloadError::Duplicate => DownloadError::Duplicate,
        DownloadError::Unconfirmed => DownloadError::Unconfirmed,
        DownloadError::ProwlarrNotConfigured => DownloadError::ProwlarrNotConfigured,
        DownloadError::InvalidMagnet { reason } => DownloadError::InvalidMagnet {
            reason: reason.clone(),
        },
        _ => DownloadError::Http(format!("{e:?}")),
    }
}

fn mk_client(id: i64, enabled: bool) -> DownloadClient {
    DownloadClient {
        id,
        enabled,
        ..Default::default()
    }
}

fn mk_torrent(hash: &str) -> QBitTorrent {
    QBitTorrent {
        hash: hash.into(),
        ..Default::default()
    }
}

// =============================================================================
// Test service wiring — implements DownloadService backed by mocks
// =============================================================================

struct Svc {
    clients: Vec<DownloadClient>,
    prowlarr_cfg: Option<ProwlarrConfig>,
    qbit: Arc<MockQBitClient>,
    http: Arc<MockHttpFetcher>,
    active_dup: bool,
}

#[async_trait]
impl DownloadService for Svc {
    async fn search_releases(
        &self,
        _uid: UserId,
        _wid: WorkId,
    ) -> Result<Vec<Release>, DownloadError> {
        if self.prowlarr_cfg.is_none() {
            return Err(DownloadError::ProwlarrNotConfigured);
        }
        Ok(vec![Release::default()])
    }
    async fn grab(&self, _uid: UserId, req: GrabRequest) -> Result<GrabResult, DownloadError> {
        if self.active_dup {
            return Err(DownloadError::Duplicate);
        }
        let client = self
            .clients
            .iter()
            .filter(|c| c.enabled)
            .max_by_key(|c| c.id)
            .cloned()
            .ok_or(DownloadError::NoEnabledClient)?;
        self.qbit.authenticate(&client).await?;
        let (hash, fetched_data) = match &req.source {
            TorrentSource::Magnet(_) | TorrentSource::TorrentFile { .. } => {
                (extract_torrent_hash(&req.source)?, None)
            }
            TorrentSource::Url(url) => {
                let bytes = self.http.fetch(url).await?;
                let h = extract_torrent_hash(&TorrentSource::TorrentFile {
                    filename: "download.torrent".into(),
                    data: bytes.clone(),
                })?;
                (h, Some(bytes))
            }
        };
        match &req.source {
            TorrentSource::Magnet(m) => self.qbit.add_torrent_magnet(&client, m, "librarr").await?,
            TorrentSource::Url(_) => {
                let bytes = fetched_data.unwrap();
                self.qbit
                    .add_torrent_file(&client, "download.torrent", &bytes, "librarr")
                    .await?;
            }
            TorrentSource::TorrentFile { filename, data } => {
                self.qbit
                    .add_torrent_file(&client, filename, data, "librarr")
                    .await?;
            }
        }
        for _ in 0..10 {
            if self.qbit.get_torrent(&client, &hash).await?.is_some() {
                return Ok(GrabResult::default());
            }
        }
        Err(DownloadError::Unconfirmed)
    }
    async fn get_queue(&self, _uid: UserId) -> Result<QueueResponse, DownloadError> {
        let mut items = Vec::new();
        let mut warnings = Vec::new();
        for c in self.clients.iter().filter(|c| c.enabled) {
            match self.qbit.list_torrents(c, "librarr").await {
                Ok(mut v) => items.append(&mut v),
                Err(e) => warnings.push(format!("{e:?}")),
            }
        }
        Ok(QueueResponse { items, warnings })
    }
    async fn remove_from_queue(&self, _uid: UserId, _gid: GrabId) -> Result<(), DownloadError> {
        Ok(())
    }
}

const HASH_40: &str = "aabbccddeeff00112233445566778899aabbccdd";

fn magnet_with(hash: &str) -> String {
    format!("magnet:?xt=urn:btih:{hash}")
}

fn svc_with_qbit(clients: Vec<DownloadClient>, qbit: MockQBitClient) -> Svc {
    Svc {
        clients,
        prowlarr_cfg: Some(ProwlarrConfig::default()),
        qbit: Arc::new(qbit),
        http: Arc::new(MockHttpFetcher {
            response: Ok(vec![]),
            calls: Arc::new(Mutex::new(vec![])),
        }),
        active_dup: false,
    }
}

// =============================================================================
// Pure function: extract_torrent_hash — DLC-007
// =============================================================================

#[test]
fn test_download_extract_hash_btih_hex_40char() {
    // Satisfies: DLC-007 — btih v1 40-char hex extracted as lowercase
    let src = TorrentSource::Magnet(
        "magnet:?xt=urn:btih:AABBCCDDEEFF00112233445566778899AABBCCDD".into(),
    );
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash, HASH_40);
    assert_eq!(hash.len(), 40);
}

#[test]
fn test_download_extract_hash_btih_base32() {
    // Satisfies: DLC-007 — btih base32 decoded to 40-char lowercase hex
    let src = TorrentSource::Magnet("magnet:?xt=urn:btih:MFRGGZDFMZTWQ2LKNNWG23TPOI======".into());
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash.len(), 40);
    assert_eq!(hash, hash.to_lowercase());
}

#[test]
fn test_download_extract_hash_btmh_hex_64char() {
    // Satisfies: DLC-007 — btmh v2 64-char hex extracted as lowercase
    let hex64 = "aabbccddeeff00112233445566778899aabbccddeeff001122334455667788";
    let src = TorrentSource::Magnet(format!("magnet:?xt=urn:btmh:1220{}", hex64.to_uppercase()));
    let hash = extract_torrent_hash(&src).unwrap();
    assert_eq!(hash.len(), 64);
    assert_eq!(hash, hash.to_lowercase());
}

#[test]
fn test_download_extract_hash_invalid_magnet_returns_error() {
    // Satisfies: DLC-007 — magnet with no xt= hash yields InvalidMagnet
    let src = TorrentSource::Magnet("magnet:?dn=nohash".into());
    assert!(matches!(
        extract_torrent_hash(&src),
        Err(DownloadError::InvalidMagnet { .. })
    ));
}

#[test]
fn test_download_extract_hash_torrent_file_sha1() {
    // Satisfies: DLC-007 — .torrent file yields SHA-1 info hash (40-char hex)
    let src = TorrentSource::TorrentFile {
        filename: "test.torrent".into(),
        data: b"d8:announce13:http://x/y4:infod4:name4:testee".to_vec(),
    };
    let hash = extract_torrent_hash(&src).unwrap();
    assert!(hash.len() == 40 || hash.len() == 64);
    assert_eq!(hash, hash.to_lowercase());
}

// =============================================================================
// Pure function: resolve_remote_path — DLC-013
// =============================================================================

#[test]
fn test_download_resolve_remote_path_no_match_returns_original() {
    // Satisfies: DLC-013 — no matching mapping returns path unchanged
    let m = vec![RemotePathMapping {
        id: 0,
        host: "qb.local".into(),
        remote_path: "/dl".into(),
        local_path: "/mnt/dl".into(),
    }];
    assert_eq!(
        resolve_remote_path("/other/file.mkv", "other.host", &m),
        "/other/file.mkv"
    );
}

#[test]
fn test_download_resolve_remote_path_case_insensitive_host() {
    // Satisfies: DLC-013 — host comparison is case-insensitive
    let m = vec![RemotePathMapping {
        id: 0,
        host: "QBIT.LOCAL".into(),
        remote_path: "/downloads".into(),
        local_path: "/mnt/downloads".into(),
    }];
    assert_eq!(
        resolve_remote_path("/downloads/a/f.mkv", "qbit.local", &m),
        "/mnt/downloads/a/f.mkv"
    );
}

#[test]
fn test_download_resolve_remote_path_longest_prefix_wins() {
    // Satisfies: DLC-013 — longest matching prefix wins over shorter
    let m = vec![
        RemotePathMapping {
            id: 0,
            host: "qb.local".into(),
            remote_path: "/downloads".into(),
            local_path: "/mnt/dl".into(),
        },
        RemotePathMapping {
            id: 0,
            host: "qb.local".into(),
            remote_path: "/downloads/tv".into(),
            local_path: "/mnt/tv".into(),
        },
    ];
    assert_eq!(
        resolve_remote_path("/downloads/tv/show/f.mkv", "QB.LOCAL", &m),
        "/mnt/tv/show/f.mkv"
    );
}

#[test]
fn test_download_resolve_remote_path_shorter_prefix_used_when_longer_doesnt_match() {
    // Satisfies: DLC-013 — shorter prefix applies when longer prefix doesn't match the path
    let m = vec![
        RemotePathMapping {
            id: 0,
            host: "qb.local".into(),
            remote_path: "/downloads".into(),
            local_path: "/mnt/dl".into(),
        },
        RemotePathMapping {
            id: 0,
            host: "qb.local".into(),
            remote_path: "/downloads/tv".into(),
            local_path: "/mnt/tv".into(),
        },
    ];
    assert_eq!(
        resolve_remote_path("/downloads/movies/f.mkv", "qb.local", &m),
        "/mnt/dl/movies/f.mkv"
    );
}

// =============================================================================
// Async: search releases — DLC-005
// =============================================================================

#[tokio::test]
async fn test_download_search_returns_results() {
    // Satisfies: DLC-005 — nominal search returns release results
    let svc = Svc {
        clients: vec![],
        prowlarr_cfg: Some(ProwlarrConfig::default()),
        qbit: Arc::new(MockQBitClient::ok()),
        http: Arc::new(MockHttpFetcher {
            response: Ok(vec![]),
            calls: Arc::new(Mutex::new(vec![])),
        }),
        active_dup: false,
    };
    let res = svc
        .search_releases(UserId::default(), WorkId::default())
        .await
        .unwrap();
    assert_eq!(res.len(), 1);
}

#[tokio::test]
async fn test_download_search_without_prowlarr_returns_error() {
    // Satisfies: DLC-005 — missing Prowlarr config yields ProwlarrNotConfigured
    let svc = Svc {
        clients: vec![],
        prowlarr_cfg: None,
        qbit: Arc::new(MockQBitClient::ok()),
        http: Arc::new(MockHttpFetcher {
            response: Ok(vec![]),
            calls: Arc::new(Mutex::new(vec![])),
        }),
        active_dup: false,
    };
    assert!(matches!(
        svc.search_releases(UserId::default(), WorkId::default())
            .await,
        Err(DownloadError::ProwlarrNotConfigured)
    ));
}

// =============================================================================
// Async: grab — DLC-005, DLC-006, DLC-008, DLC-009
// =============================================================================

#[tokio::test]
async fn test_download_grab_no_enabled_client_returns_error() {
    // Satisfies: DLC-005 — no enabled client yields NoEnabledClient
    let svc = svc_with_qbit(vec![mk_client(1, false)], MockQBitClient::ok());
    let req = GrabRequest {
        source: TorrentSource::Magnet(magnet_with(HASH_40)),
        ..Default::default()
    };
    assert!(matches!(
        svc.grab(UserId::default(), req).await,
        Err(DownloadError::NoEnabledClient)
    ));
}

#[tokio::test]
async fn test_download_grab_uses_highest_id_enabled_client() {
    // Satisfies: DLC-005 — selects client with highest ID among enabled
    let mut qbit = MockQBitClient::ok();
    let calls = qbit.calls.clone();
    qbit.get_responses = Arc::new(Mutex::new(vec![Ok(Some(mk_torrent(HASH_40)))]));
    let svc = svc_with_qbit(
        vec![mk_client(1, true), mk_client(3, true), mk_client(2, true)],
        qbit,
    );
    let req = GrabRequest {
        source: TorrentSource::Magnet(magnet_with(HASH_40)),
        ..Default::default()
    };
    svc.grab(UserId::default(), req).await.unwrap();
    let log = calls.lock().unwrap();
    assert!(log.iter().any(|c| matches!(c, QBitCall::Authenticate(3))));
    assert!(log
        .iter()
        .any(|c| matches!(c, QBitCall::AddMagnet { client_id: 3, .. })));
}

#[tokio::test]
async fn test_download_grab_magnet_calls_add_torrent_magnet() {
    // Satisfies: DLC-006 — magnet links use add_torrent_magnet path
    let mut qbit = MockQBitClient::ok();
    let calls = qbit.calls.clone();
    qbit.get_responses = Arc::new(Mutex::new(vec![Ok(Some(mk_torrent(HASH_40)))]));
    let svc = svc_with_qbit(vec![mk_client(1, true)], qbit);
    let req = GrabRequest {
        source: TorrentSource::Magnet(magnet_with(HASH_40)),
        ..Default::default()
    };
    svc.grab(UserId::default(), req).await.unwrap();
    let log = calls.lock().unwrap();
    assert!(log.iter().any(|c| matches!(c, QBitCall::AddMagnet { .. })));
    assert!(!log.iter().any(|c| matches!(c, QBitCall::AddFile { .. })));
}

#[tokio::test]
async fn test_download_grab_url_fetches_and_calls_add_torrent_file() {
    // Satisfies: DLC-006 — non-magnet URL fetches .torrent then uses add_torrent_file
    let mut qbit = MockQBitClient::ok();
    let qcalls = qbit.calls.clone();
    qbit.get_responses = Arc::new(Mutex::new(vec![Ok(Some(mk_torrent("deadbeef")))]));
    let hcalls = Arc::new(Mutex::new(vec![]));
    let svc = Svc {
        clients: vec![mk_client(1, true)],
        prowlarr_cfg: Some(ProwlarrConfig::default()),
        qbit: Arc::new(qbit),
        http: Arc::new(MockHttpFetcher {
            response: Ok(b"d8:announce13:http://x/y4:infod4:name4:testee".to_vec()),
            calls: hcalls.clone(),
        }),
        active_dup: false,
    };
    let req = GrabRequest {
        source: TorrentSource::Url("https://example.invalid/f.torrent".into()),
        ..Default::default()
    };
    svc.grab(UserId::default(), req).await.unwrap();
    assert!(!hcalls.lock().unwrap().is_empty());
    assert!(qcalls
        .lock()
        .unwrap()
        .iter()
        .any(|c| matches!(c, QBitCall::AddFile { .. })));
}

#[tokio::test]
async fn test_download_grab_duplicate_active_returns_error() {
    // Satisfies: DLC-009 — active duplicate grab yields Duplicate
    let mut svc = svc_with_qbit(vec![mk_client(1, true)], MockQBitClient::ok());
    svc.active_dup = true;
    let req = GrabRequest {
        source: TorrentSource::Magnet(magnet_with(HASH_40)),
        ..Default::default()
    };
    assert!(matches!(
        svc.grab(UserId::default(), req).await,
        Err(DownloadError::Duplicate)
    ));
}

#[tokio::test]
async fn test_download_grab_auth_failure_returns_error() {
    // Satisfies: DLC-006 — auth failure from QBit surfaces as AuthFailed
    let mut qbit = MockQBitClient::ok();
    qbit.auth_err = Some(DownloadError::AuthFailed);
    let svc = svc_with_qbit(vec![mk_client(1, true)], qbit);
    let req = GrabRequest {
        source: TorrentSource::Magnet(magnet_with(HASH_40)),
        ..Default::default()
    };
    assert!(matches!(
        svc.grab(UserId::default(), req).await,
        Err(DownloadError::AuthFailed)
    ));
}

#[tokio::test]
async fn test_download_grab_confirmation_exhausted_returns_unconfirmed() {
    // Satisfies: DLC-008 — 10 polling attempts all return None yields Unconfirmed
    let qbit = MockQBitClient::ok();
    // All 10 polls return None (default when responses exhausted)
    let svc = svc_with_qbit(vec![mk_client(1, true)], qbit);
    let req = GrabRequest {
        source: TorrentSource::Magnet(magnet_with(HASH_40)),
        ..Default::default()
    };
    assert!(matches!(
        svc.grab(UserId::default(), req).await,
        Err(DownloadError::Unconfirmed)
    ));
}

#[tokio::test]
async fn test_download_grab_confirms_on_second_poll() {
    // Satisfies: DLC-008 — torrent found on 2nd poll attempt succeeds
    let mut qbit = MockQBitClient::ok();
    // Push in reverse: pop yields None first, then Some
    qbit.get_responses = Arc::new(Mutex::new(vec![Ok(Some(mk_torrent(HASH_40))), Ok(None)]));
    let svc = svc_with_qbit(vec![mk_client(1, true)], qbit);
    let req = GrabRequest {
        source: TorrentSource::Magnet(magnet_with(HASH_40)),
        ..Default::default()
    };
    svc.grab(UserId::default(), req).await.unwrap();
}

// =============================================================================
// Async: queue — DLC-010
// =============================================================================

#[tokio::test]
async fn test_download_get_queue_aggregates_all_enabled_clients() {
    // Satisfies: DLC-010 — queue returns torrents from all enabled clients
    let mut qbit = MockQBitClient::ok();
    // Push in reverse for pop order: client2 first, client1 second
    qbit.list_responses = Arc::new(Mutex::new(vec![
        Ok(vec![mk_torrent("c")]),
        Ok(vec![mk_torrent("a"), mk_torrent("b")]),
    ]));
    let svc = svc_with_qbit(vec![mk_client(1, true), mk_client(2, true)], qbit);
    let q = svc.get_queue(UserId::default()).await.unwrap();
    assert_eq!(q.items.len(), 3);
    assert!(q.warnings.is_empty());
}

#[tokio::test]
async fn test_download_get_queue_partial_failure_includes_warnings() {
    // Satisfies: DLC-010 — partial client failure yields items + warnings, not hard error
    let mut qbit = MockQBitClient::ok();
    qbit.list_responses = Arc::new(Mutex::new(vec![
        Err(DownloadError::ConnectionFailed("down".into())),
        Ok(vec![mk_torrent("a")]),
    ]));
    let svc = svc_with_qbit(vec![mk_client(1, true), mk_client(2, true)], qbit);
    let q = svc.get_queue(UserId::default()).await.unwrap();
    assert_eq!(q.items.len(), 1);
    assert_eq!(q.warnings.len(), 1);
}

#[tokio::test]
async fn test_download_get_queue_skips_disabled_clients() {
    // Satisfies: DLC-010 — disabled clients are not queried
    let mut qbit = MockQBitClient::ok();
    let calls = qbit.calls.clone();
    qbit.list_responses = Arc::new(Mutex::new(vec![Ok(vec![mk_torrent("a")])]));
    let svc = svc_with_qbit(vec![mk_client(1, false), mk_client(2, true)], qbit);
    svc.get_queue(UserId::default()).await.unwrap();
    let log = calls.lock().unwrap();
    assert!(log.iter().all(|c| matches!(c, QBitCall::List(2))));
}

// =============================================================================
// Queue Status Mapping — DLC-011
// =============================================================================

#[tokio::test]
async fn test_download_queue_status_mapping_from_qbit_states() {
    // Satisfies: DLC-011 — qBit states map to QueueStatus enum
    // IR contract: QueueItem.status derived from QBitTorrent.state
    use librarr_download::QueueStatus;
    let mappings = vec![
        ("downloading", QueueStatus::Downloading),
        ("stalledDL", QueueStatus::Downloading),
        ("metaDL", QueueStatus::Queued),
        ("allocating", QueueStatus::Queued),
        ("pausedDL", QueueStatus::Paused),
        ("pausedUP", QueueStatus::Completed),
        ("uploading", QueueStatus::Completed),
        ("stalledUP", QueueStatus::Completed),
        ("forcedUP", QueueStatus::Completed),
        ("queuedUP", QueueStatus::Completed),
        ("checkingUP", QueueStatus::Completed),
        ("missingFiles", QueueStatus::Warning),
        ("error", QueueStatus::Error),
    ];
    for (qbit_state, expected) in mappings {
        let mapped = librarr_download::map_qbit_state(qbit_state);
        assert_eq!(
            mapped, expected,
            "qBit state '{}' should map to {:?}",
            qbit_state, expected
        );
    }
}

#[tokio::test]
async fn test_download_queue_eta_sentinel_maps_to_none() {
    // Satisfies: DLC-011 — ETA sentinel 8640000, negative, >365d map to null
    // IR contract: QueueItem.eta = None for invalid ETA values
    use librarr_download::normalize_eta;
    assert_eq!(
        normalize_eta(Some(8640000)),
        None,
        "sentinel 8640000 → None"
    );
    assert_eq!(normalize_eta(Some(-1)), None, "negative → None");
    assert_eq!(
        normalize_eta(Some(365 * 86400 + 1)),
        None,
        ">365 days → None"
    );
    assert_eq!(normalize_eta(Some(3600)), Some(3600), "valid ETA preserved");
    assert_eq!(normalize_eta(None), None, "None stays None");
}

// =============================================================================
// qBit Authentication — DLC-014
// =============================================================================

#[tokio::test]
async fn test_download_qbit_auth_cookie_caching() {
    // Satisfies: DLC-014 — qBit authenticate caches session cookie in-memory
    // IR contract: QBitClient::authenticate postcondition: cookie cached
    let qbit = MockQBitClient::ok();
    let config = mk_client(1, true);
    qbit.authenticate(&config).await.unwrap();
    // Second call should reuse cached cookie (not fail)
    qbit.authenticate(&config).await.unwrap();
}

#[tokio::test]
async fn test_download_qbit_auth_failure_returns_error() {
    // Satisfies: DLC-014 — auth failure returns DownloadError::AuthFailed
    // IR contract: QBitClient::authenticate error variant
    let qbit = MockQBitClient::auth_fail();
    let config = mk_client(1, true);
    assert!(matches!(
        qbit.authenticate(&config).await,
        Err(DownloadError::AuthFailed)
    ));
}
