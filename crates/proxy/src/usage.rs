//! In-memory usage statistics for request/token tracking.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global request/token counters.
#[derive(Default)]
pub struct UsageStats {
    /// Total requests received.
    pub total_requests: AtomicU64,
    /// Successful requests (2xx from upstream).
    pub success_requests: AtomicU64,
    /// Failed requests (non-2xx or internal error).
    pub failure_requests: AtomicU64,
    /// Total input tokens across all requests.
    pub input_tokens: AtomicU64,
    /// Total output tokens across all requests.
    pub output_tokens: AtomicU64,
    /// Per-model request counts.
    model_counts: Mutex<HashMap<String, ModelStats>>,
}

/// Per-model usage counters.
#[derive(Default, Clone, Serialize)]
pub struct ModelStats {
    pub requests: u64,
    pub success: u64,
    pub failure: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// JSON-serializable snapshot of current usage.
#[derive(Serialize)]
pub struct UsageSnapshot {
    pub total_requests: u64,
    pub success_requests: u64,
    pub failure_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub models: HashMap<String, ModelStats>,
}

impl UsageStats {
    /// Creates a new empty stats tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful request with optional token counts.
    pub fn record_success(&self, model: &str, input_tokens: u64, output_tokens: u64) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.success_requests.fetch_add(1, Ordering::Relaxed);
        self.input_tokens.fetch_add(input_tokens, Ordering::Relaxed);
        self.output_tokens
            .fetch_add(output_tokens, Ordering::Relaxed);

        if let Ok(mut map) = self.model_counts.lock() {
            let entry = map.entry(model.to_string()).or_default();
            entry.requests += 1;
            entry.success += 1;
            entry.input_tokens += input_tokens;
            entry.output_tokens += output_tokens;
        }
    }

    /// Record a failed request.
    pub fn record_failure(&self, model: &str) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.failure_requests.fetch_add(1, Ordering::Relaxed);

        if let Ok(mut map) = self.model_counts.lock() {
            let entry = map.entry(model.to_string()).or_default();
            entry.requests += 1;
            entry.failure += 1;
        }
    }

    /// Take a JSON-serializable snapshot of current stats.
    #[must_use]
    pub fn snapshot(&self) -> UsageSnapshot {
        let models = self
            .model_counts
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        UsageSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            success_requests: self.success_requests.load(Ordering::Relaxed),
            failure_requests: self.failure_requests.load(Ordering::Relaxed),
            input_tokens: self.input_tokens.load(Ordering::Relaxed),
            output_tokens: self.output_tokens.load(Ordering::Relaxed),
            models,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_success() {
        let stats = UsageStats::new();
        stats.record_success("claude-opus-4-5", 100, 200);
        stats.record_success("claude-opus-4-5", 50, 100);
        stats.record_success("gpt-4o", 80, 150);

        let snap = stats.snapshot();
        assert_eq!(snap.total_requests, 3);
        assert_eq!(snap.success_requests, 3);
        assert_eq!(snap.failure_requests, 0);
        assert_eq!(snap.input_tokens, 230);
        assert_eq!(snap.output_tokens, 450);

        let claude = &snap.models["claude-opus-4-5"];
        assert_eq!(claude.requests, 2);
        assert_eq!(claude.success, 2);
        assert_eq!(claude.input_tokens, 150);
        assert_eq!(claude.output_tokens, 300);
    }

    #[test]
    fn test_record_failure() {
        let stats = UsageStats::new();
        stats.record_failure("gpt-4o");
        stats.record_success("gpt-4o", 10, 20);

        let snap = stats.snapshot();
        assert_eq!(snap.total_requests, 2);
        assert_eq!(snap.success_requests, 1);
        assert_eq!(snap.failure_requests, 1);

        let model = &snap.models["gpt-4o"];
        assert_eq!(model.requests, 2);
        assert_eq!(model.failure, 1);
        assert_eq!(model.success, 1);
    }

    #[test]
    fn test_snapshot_empty() {
        let stats = UsageStats::new();
        let snap = stats.snapshot();
        assert_eq!(snap.total_requests, 0);
        assert!(snap.models.is_empty());
    }
}
