use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

pub struct KeyedMutex<K> {
    map: Mutex<HashMap<K, Arc<Mutex<()>>>>,
}

impl<K: Eq + Hash + Clone> KeyedMutex<K> {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    pub async fn lock(&self, key: K) -> KeyedMutexGuard {
        let entry = {
            let mut map = self.map.lock().await;
            map.entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let guard = Arc::clone(&entry).lock_owned().await;
        KeyedMutexGuard { _guard: guard }
    }

    /// Remove entries where the map holds the only reference (no active waiters/holders).
    /// Call periodically from a background task.
    pub async fn sweep(&self) {
        let mut map = self.map.lock().await;
        map.retain(|_, arc| Arc::strong_count(arc) > 1);
    }
}

impl<K: Eq + Hash + Clone> Default for KeyedMutex<K> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct KeyedMutexGuard {
    _guard: OwnedMutexGuard<()>,
}
