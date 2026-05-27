//! Cancellation registry for in-progress analysis jobs.
//!
//! Provides a shared registry of cancellation tokens keyed by job_id.
//! The pipeline registers a token when starting and checks it between batches.
//! The `cancel_analysis` command looks up and triggers the token.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// A cancellation token that can be checked by the pipeline and triggered by the cancel command.
#[derive(Debug, Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a new token in the non-cancelled state.
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Check whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Trigger cancellation. Returns `true` if this call actually transitioned
    /// the token from non-cancelled to cancelled (i.e., it wasn't already cancelled).
    pub fn cancel(&self) -> bool {
        // swap returns the previous value; if it was false, we successfully cancelled
        !self.cancelled.swap(true, Ordering::Relaxed)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Global registry mapping job_id → CancellationToken.
///
/// The pipeline registers its token on start and removes it when done.
/// The cancel command looks up and triggers the token.
#[derive(Debug, Clone)]
pub struct CancellationRegistry {
    tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl CancellationRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a cancellation token for a job. Returns the token for the pipeline to check.
    pub async fn register(&self, job_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        let mut map = self.tokens.lock().await;
        map.insert(job_id.to_string(), token.clone());
        token
    }

    /// Trigger cancellation for a job. Returns `true` if the job was found and cancelled,
    /// `false` if the job_id was not found (already finished or never started).
    pub async fn cancel(&self, job_id: &str) -> bool {
        let map = self.tokens.lock().await;
        if let Some(token) = map.get(job_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Remove a token from the registry (called when the pipeline finishes or is cancelled).
    pub async fn unregister(&self, job_id: &str) {
        let mut map = self.tokens.lock().await;
        map.remove(job_id);
    }

    /// Check if a job is currently registered (i.e., running).
    pub async fn is_registered(&self, job_id: &str) -> bool {
        let map = self.tokens.lock().await;
        map.contains_key(job_id)
    }
}

impl Default for CancellationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton instance of the cancellation registry.
/// Using a lazy static with tokio Mutex for async-safe access.
static GLOBAL_REGISTRY: std::sync::OnceLock<CancellationRegistry> = std::sync::OnceLock::new();

/// Get the global cancellation registry instance.
pub fn global_registry() -> &'static CancellationRegistry {
    GLOBAL_REGISTRY.get_or_init(CancellationRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_initial_state() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_token_cancel() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());

        // First cancel should return true (successfully transitioned)
        assert!(token.cancel());
        assert!(token.is_cancelled());

        // Second cancel should return false (already cancelled)
        assert!(!token.cancel());
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_token_clone_shares_state() {
        let token1 = CancellationToken::new();
        let token2 = token1.clone();

        assert!(!token1.is_cancelled());
        assert!(!token2.is_cancelled());

        token1.cancel();

        assert!(token1.is_cancelled());
        assert!(token2.is_cancelled());
    }

    #[tokio::test]
    async fn test_registry_register_and_cancel() {
        let registry = CancellationRegistry::new();

        let token = registry.register("job-1").await;
        assert!(!token.is_cancelled());
        assert!(registry.is_registered("job-1").await);

        // Cancel via registry
        let cancelled = registry.cancel("job-1").await;
        assert!(cancelled);
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn test_registry_cancel_unknown_job() {
        let registry = CancellationRegistry::new();

        // Cancelling a non-existent job returns false
        let cancelled = registry.cancel("nonexistent").await;
        assert!(!cancelled);
    }

    #[tokio::test]
    async fn test_registry_unregister() {
        let registry = CancellationRegistry::new();

        let _token = registry.register("job-2").await;
        assert!(registry.is_registered("job-2").await);

        registry.unregister("job-2").await;
        assert!(!registry.is_registered("job-2").await);

        // Cancel after unregister should return false
        let cancelled = registry.cancel("job-2").await;
        assert!(!cancelled);
    }

    #[tokio::test]
    async fn test_registry_multiple_jobs() {
        let registry = CancellationRegistry::new();

        let token_a = registry.register("job-a").await;
        let token_b = registry.register("job-b").await;

        // Cancel only job-a
        registry.cancel("job-a").await;

        assert!(token_a.is_cancelled());
        assert!(!token_b.is_cancelled());
    }

    #[test]
    fn test_global_registry_is_singleton() {
        let r1 = global_registry();
        let r2 = global_registry();
        // Both should point to the same instance
        assert!(std::ptr::eq(r1, r2));
    }
}
