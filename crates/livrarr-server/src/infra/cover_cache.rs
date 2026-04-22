use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct CoverProxyCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
}

struct CacheEntry {
    data: Vec<u8>,
    content_type: String,
    fetched_at: Instant,
}

const CACHE_TTL: Duration = Duration::from_secs(300);
const MAX_CACHE_ENTRIES: usize = 200;

impl CoverProxyCache {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get(&self, url: &str) -> Option<(Vec<u8>, String)> {
        let cache = self.entries.read().await;
        let entry = cache.get(url)?;
        if entry.fetched_at.elapsed() < CACHE_TTL {
            Some((entry.data.clone(), entry.content_type.clone()))
        } else {
            None
        }
    }

    pub async fn put(&self, url: String, data: Vec<u8>, content_type: String) {
        let mut cache = self.entries.write().await;
        if cache.len() >= MAX_CACHE_ENTRIES {
            cache.retain(|_, e| e.fetched_at.elapsed() < CACHE_TTL);
        }
        while cache.len() >= MAX_CACHE_ENTRIES {
            let oldest_key = cache
                .iter()
                .min_by_key(|(_, e)| e.fetched_at)
                .map(|(k, _)| k.clone());
            match oldest_key {
                Some(k) => {
                    cache.remove(&k);
                }
                None => break,
            }
        }
        cache.insert(
            url,
            CacheEntry {
                data,
                content_type,
                fetched_at: Instant::now(),
            },
        );
    }
}
