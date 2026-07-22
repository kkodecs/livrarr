use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

/// Hard cap on distinct keys resident in one `KeyedMutex` instance at once
/// (D3 / PRINCIPLES.md §5: no in-memory collection may assume its keyspace
/// is small enough). Enforced structurally via `capacity` below — every
/// resident key holds exactly one of `MAX_ACTIVE_KEYS` permits, so the map
/// can never exceed this count no matter how many distinct keys are ever
/// requested over the instance's lifetime.
const MAX_ACTIVE_KEYS: usize = 256;

/// One key's resident state: its per-key lock, plus the capacity permit
/// that key is currently spending. Dropping the tuple (on removal from the
/// map) drops the permit, returning capacity to `KeyedMutex::capacity`.
struct Entry {
    lock: Arc<Mutex<()>>,
    _permit: OwnedSemaphorePermit,
}

pub struct KeyedMutex<K> {
    map: Arc<Mutex<HashMap<K, Entry>>>,
    /// Bounds the number of distinct resident keys to `MAX_ACTIVE_KEYS`. A
    /// caller for a brand-new key acquires one permit before it can insert;
    /// an existing key never touches this semaphore at all.
    capacity: Arc<Semaphore>,
}

impl<K: Eq + Hash + Clone> KeyedMutex<K> {
    pub fn new() -> Self {
        Self {
            map: Arc::new(Mutex::new(HashMap::new())),
            capacity: Arc::new(Semaphore::new(MAX_ACTIVE_KEYS)),
        }
    }

    /// Acquire the per-key lock, creating the key's entry on first use.
    ///
    /// An EXISTING key is always immediately accessible: it never competes
    /// for capacity, no matter how full the map is. A genuinely NEW key,
    /// when the map already holds `MAX_ACTIVE_KEYS` distinct keys, waits
    /// here for a capacity permit — and that wait never holds the map
    /// lock, so other callers (including the guard releases whose
    /// opportunistic prune is what will eventually free a permit) are
    /// never blocked by it. A permit only ever becomes available because
    /// some other key was already pruned, so "prune before insert" holds
    /// by construction — there is no separate sweep step needed here.
    pub async fn lock(&self, key: K) -> KeyedMutexGuard<K> {
        // Fast path: the key already exists — no capacity wait, ever.
        let existing = {
            let map = self.map.lock().await;
            map.get(&key).map(|entry| Arc::clone(&entry.lock))
        };
        let lock = match existing {
            Some(lock) => lock,
            None => self.insert_new_key(key.clone()).await,
        };

        let guard = lock.lock_owned().await;
        KeyedMutexGuard {
            guard: Some(guard),
            key,
            map: Arc::clone(&self.map),
        }
    }

    /// Slow path for a candidate new key: wait for a capacity permit
    /// (without holding the map lock), then re-check under the map lock —
    /// a concurrent caller may have created (or created and released) the
    /// same key while this one waited.
    async fn insert_new_key(&self, key: K) -> Arc<Mutex<()>> {
        let permit = Arc::clone(&self.capacity)
            .acquire_owned()
            .await
            .expect("capacity semaphore is never closed");

        let mut map = self.map.lock().await;
        match map.get(&key) {
            Some(existing) => {
                // Lost the race: another caller already holds this key's
                // slot. Our permit is redundant — drop it, returning
                // capacity immediately rather than leaking it until this
                // guard eventually releases.
                drop(permit);
                Arc::clone(&existing.lock)
            }
            None => {
                let lock = Arc::new(Mutex::new(()));
                map.insert(
                    key,
                    Entry {
                        lock: Arc::clone(&lock),
                        _permit: permit,
                    },
                );
                lock
            }
        }
    }

    /// Remove entries where the map holds the only reference (no active
    /// waiters/holders). Explicit backstop only: opportunistic per-guard
    /// pruning (`Drop for KeyedMutexGuard`) already does this on every
    /// release, so a healthy instance rarely needs this called at all.
    pub async fn sweep(&self) {
        let mut map = self.map.lock().await;
        map.retain(|_, entry| Arc::strong_count(&entry.lock) > 1);
    }
}

impl<K: Eq + Hash + Clone> Default for KeyedMutex<K> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl<K: Eq + Hash + Clone> KeyedMutex<K> {
    /// Test-only: current number of distinct resident keys.
    async fn active_key_count_for_tests(&self) -> usize {
        self.map.lock().await.len()
    }
}

pub struct KeyedMutexGuard<K: Eq + Hash + Clone> {
    guard: Option<OwnedMutexGuard<()>>,
    key: K,
    map: Arc<Mutex<HashMap<K, Entry>>>,
}

impl<K: Eq + Hash + Clone> Drop for KeyedMutexGuard<K> {
    fn drop(&mut self) {
        // Release the per-key lock first: until this drops, the guard's
        // own Arc clone keeps the entry's strong count above 1.
        self.guard.take();

        // Best-effort, non-blocking opportunistic prune. `Drop::drop` is
        // sync, so this never awaits — `try_lock` either wins immediately
        // or this release simply skips pruning (a busy map means someone
        // else is actively using it right now anyway; the explicit
        // `sweep()` remains as a backstop, and the next guard to release
        // against a free map gets another chance).
        if let Ok(mut map) = self.map.try_lock() {
            if let Some(entry) = map.get(&self.key) {
                if Arc::strong_count(&entry.lock) == 1 {
                    map.remove(&self.key);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn existing_key_always_remains_accessible() {
        let km: KeyedMutex<u32> = KeyedMutex::new();
        let g1 = km.lock(1).await;
        drop(g1);
        // Re-locking the same key after release must succeed immediately.
        let _g2 = km.lock(1).await;
        assert_eq!(km.active_key_count_for_tests().await, 1);
    }

    #[tokio::test]
    async fn releasing_a_guard_opportunistically_prunes_its_key() {
        let km: KeyedMutex<u32> = KeyedMutex::new();
        {
            let _guard = km.lock(1).await;
            assert_eq!(km.active_key_count_for_tests().await, 1);
        }
        // The guard dropped — its key must be pruned without an explicit
        // sweep() call.
        assert_eq!(km.active_key_count_for_tests().await, 0);
    }

    #[tokio::test]
    async fn a_key_with_an_active_holder_is_never_pruned_by_a_sibling_release() {
        let km: KeyedMutex<u32> = KeyedMutex::new();
        let held = km.lock(1).await;
        {
            let _other = km.lock(2).await;
        }
        // Key 2 released and pruned; key 1 is still held and must remain.
        assert_eq!(km.active_key_count_for_tests().await, 1);
        drop(held);
        assert_eq!(km.active_key_count_for_tests().await, 0);
    }

    #[tokio::test]
    async fn map_never_exceeds_the_hard_cap_under_churn() {
        let km: KeyedMutex<u32> = KeyedMutex::new();
        // Churn far more distinct keys through the instance than the cap —
        // each is locked and immediately released, so none are held
        // concurrently, but the map's OWN size must never exceed the cap
        // at any point we can observe it (a running scan below never
        // reads more than MAX_ACTIVE_KEYS resident at once).
        for key in 0..(MAX_ACTIVE_KEYS as u32) * 4 {
            let _guard = km.lock(key).await;
            assert!(km.active_key_count_for_tests().await <= MAX_ACTIVE_KEYS);
        }
        // Nothing is held after the loop — everything prunes back to zero.
        assert_eq!(km.active_key_count_for_tests().await, 0);
    }

    #[tokio::test]
    async fn a_new_key_waits_for_capacity_when_the_map_is_full_and_proceeds_once_freed() {
        let km: Arc<KeyedMutex<u32>> = Arc::new(KeyedMutex::new());

        // Saturate the map to the hard cap with held (unreleased) guards.
        let mut holders = Vec::new();
        for key in 0..(MAX_ACTIVE_KEYS as u32) {
            holders.push(km.lock(key).await);
        }
        assert_eq!(km.active_key_count_for_tests().await, MAX_ACTIVE_KEYS);

        // A brand-new key (never seen before) must wait — spawn it so the
        // test can prove it hasn't resolved yet, then free one slot and
        // confirm it proceeds.
        let waiter_km = Arc::clone(&km);
        let waiter = tokio::spawn(async move {
            let _guard = waiter_km.lock(MAX_ACTIVE_KEYS as u32).await;
        });

        // Give the waiter a chance to run; it must still be pending — the
        // map is at the hard cap and nothing has been released yet.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            !waiter.is_finished(),
            "a new key must wait when the map is at the hard cap"
        );

        // Free exactly one slot — the waiter must now be able to proceed.
        holders.pop();
        tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("waiter must complete once a slot frees")
            .unwrap();
    }

    #[tokio::test]
    async fn existing_key_lookup_is_never_blocked_by_a_new_key_waiting_for_capacity() {
        let km: Arc<KeyedMutex<u32>> = Arc::new(KeyedMutex::new());
        let mut holders = Vec::new();
        for key in 0..(MAX_ACTIVE_KEYS as u32) {
            holders.push(km.lock(key).await);
        }
        // Key 0 stays HELD (resident AND locked) throughout this test —
        // it must never be pruned here, or a second lock() for it would
        // take the new-key path instead of the one under test.
        let key0_holder = holders.remove(0);

        // A brand-new key blocks on capacity: all MAX_ACTIVE_KEYS slots
        // are still occupied (the remaining `holders` plus key 0).
        let waiter_km = Arc::clone(&km);
        let _capacity_waiter = tokio::spawn(async move {
            let _guard = waiter_km.lock(MAX_ACTIVE_KEYS as u32).await;
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        // A second caller for the ALREADY-RESIDENT key 0 must reach the
        // fast (existing-key) path — it only ever waits on key 0's own
        // per-key mutex, never on the capacity semaphore the spawned
        // waiter above is blocked on.
        let second_locker_km = Arc::clone(&km);
        let second_locker = tokio::spawn(async move {
            let _g = second_locker_km.lock(0u32).await;
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            !second_locker.is_finished(),
            "key 0 is still held by its original owner, so the second locker must still be \
             waiting on key 0's OWN per-key mutex (not stuck behind the capacity waiter)"
        );

        // Release key 0's original holder — the second locker must
        // unblock promptly, independent of whether the capacity waiter
        // (a genuinely different, still-unresolved key) ever does.
        drop(key0_holder);
        tokio::time::timeout(std::time::Duration::from_secs(5), second_locker)
            .await
            .expect(
                "an existing key's lock must not be gated by an unrelated new-key capacity wait",
            )
            .unwrap();
    }
}
