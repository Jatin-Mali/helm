use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;

use crate::troubleshoot::LlmPlanStatus;

pub const DEFAULT_PLAN_CACHE_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone)]
struct CachedPlanEntry {
    status: LlmPlanStatus,
    cached_at: Instant,
}

#[derive(Clone, Debug)]
pub struct PlanCache {
    ttl: Duration,
    entries: Arc<DashMap<String, CachedPlanEntry>>,
    in_flight: Arc<DashMap<String, ()>>,
}

impl Default for PlanCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(DEFAULT_PLAN_CACHE_TTL_SECS))
    }
}

impl PlanCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Arc::new(DashMap::new()),
            in_flight: Arc::new(DashMap::new()),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn get(&self, fingerprint: &str) -> Option<LlmPlanStatus> {
        let status = self.entries.get(fingerprint).and_then(|entry| {
            if entry.cached_at.elapsed() < self.ttl {
                Some(entry.status.clone())
            } else {
                None
            }
        });
        if status.is_none() {
            self.entries.remove_if(fingerprint, |_, entry| {
                entry.cached_at.elapsed() >= self.ttl
            });
        }
        status
    }

    pub fn insert(&self, fingerprint: impl Into<String>, status: LlmPlanStatus) {
        self.entries.insert(
            fingerprint.into(),
            CachedPlanEntry {
                status,
                cached_at: Instant::now(),
            },
        );
    }

    pub fn evict_expired(&self) {
        let expired = self
            .entries
            .iter()
            .filter_map(|entry| (entry.cached_at.elapsed() > self.ttl).then(|| entry.key().clone()))
            .collect::<Vec<_>>();
        for key in expired {
            self.entries.remove(&key);
        }
    }

    pub fn ensure_plan<F, Fut>(&self, fingerprint: String, generator: F) -> bool
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = LlmPlanStatus> + Send + 'static,
    {
        self.evict_expired();
        // Entry ownership is atomic across callers; status lookup alone is not a lock.
        let dashmap::mapref::entry::Entry::Vacant(owner) =
            self.in_flight.entry(fingerprint.clone())
        else {
            return false;
        };
        if let Some(status) = self.get(&fingerprint) {
            let reusable = matches!(status, LlmPlanStatus::Ready(_) | LlmPlanStatus::Loading)
                || matches!(status, LlmPlanStatus::RateLimited(seconds) if self.entries.get(&fingerprint).is_some_and(|e| e.cached_at.elapsed() < Duration::from_secs(seconds)));
            if reusable {
                return false;
            }
        }
        owner.insert(());
        self.insert(fingerprint.clone(), LlmPlanStatus::Loading);
        let cache = self.clone();
        tokio::spawn(async move {
            struct Release(PlanCache, String);
            impl Drop for Release {
                fn drop(&mut self) {
                    // Cancellation/panic must not strand a permanent loading record.
                    if matches!(self.0.get(&self.1), Some(LlmPlanStatus::Loading)) {
                        self.0.insert(self.1.clone(), LlmPlanStatus::Timeout);
                    }
                    self.0.in_flight.remove(&self.1);
                }
            }
            let _owner = Release(cache.clone(), fingerprint.clone());
            let status = tokio::time::timeout(Duration::from_secs(35), generator())
                .await
                .unwrap_or(LlmPlanStatus::Timeout);
            cache.insert(fingerprint, status);
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::troubleshoot::{LlmFixPlan, LlmNarrative};
    use proptest::prelude::*;

    fn ready_status() -> LlmPlanStatus {
        LlmPlanStatus::Ready(LlmFixPlan {
            narrative: LlmNarrative {
                text: "ready".to_owned(),
            },
            steps: Vec::new(),
            generated_at: 1,
        })
    }

    #[test]
    fn cache_hits_within_ttl() {
        let cache = PlanCache::new(Duration::from_secs(60));
        cache.insert("disk:/var", ready_status());
        assert!(matches!(
            cache.get("disk:/var"),
            Some(LlmPlanStatus::Ready(_))
        ));
    }

    #[test]
    fn expired_entries_are_evicted() {
        let cache = PlanCache::new(Duration::from_millis(1));
        cache.entries.insert(
            "disk:/var".to_owned(),
            CachedPlanEntry {
                status: ready_status(),
                cached_at: Instant::now() - Duration::from_secs(2),
            },
        );
        assert!(cache.get("disk:/var").is_none());
    }

    #[tokio::test]
    async fn ensure_plan_spawns_once_per_fingerprint() {
        let cache = PlanCache::new(Duration::from_secs(60));
        let first = cache.ensure_plan("finding-a".to_owned(), || async { ready_status() });
        let second = cache.ensure_plan("finding-a".to_owned(), || async { ready_status() });
        assert!(first);
        assert!(!second);

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(matches!(
            cache.get("finding-a"),
            Some(LlmPlanStatus::Ready(_))
        ));
    }

    #[tokio::test]
    async fn failed_generation_can_retry() {
        let cache = PlanCache::default();
        cache.insert("failed", LlmPlanStatus::Timeout);
        assert!(cache.ensure_plan("failed".into(), || async { ready_status() }));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(matches!(cache.get("failed"), Some(LlmPlanStatus::Ready(_))));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_generation_has_one_owner() {
        let cache = PlanCache::default();
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let mut jobs = Vec::new();
        for _ in 0..16 {
            let cache = cache.clone();
            let barrier = barrier.clone();
            jobs.push(tokio::task::spawn_blocking(move || {
                barrier.wait();
                cache.ensure_plan("same".into(), || async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    ready_status()
                }) as usize
            }));
        }
        let mut owners = 0;
        for job in jobs {
            owners += job.await.unwrap();
        }
        assert_eq!(owners, 1);
    }

    proptest! {
        #[test]
        fn property_plan_cache_ttl_behavior(
            fingerprint in "[a-z0-9:_/-]{1,32}",
            ttl_ms in 1u64..5_000,
            age_ms in 0u64..10_000,
        ) {
            let cache = PlanCache::new(Duration::from_millis(ttl_ms));
            cache.entries.insert(
                fingerprint.clone(),
                CachedPlanEntry {
                    status: ready_status(),
                    cached_at: Instant::now() - Duration::from_millis(age_ms),
                },
            );

            let hit = cache.get(&fingerprint).is_some();
            prop_assert_eq!(hit, age_ms < ttl_ms);
            if age_ms > ttl_ms {
                prop_assert!(!cache.entries.contains_key(&fingerprint));
            }
        }
    }
}
