//! In-memory usage statistics for request/token tracking, with optional
//! persistent backing via [`UsageStore`].

use byokey_types::{UsageRecord, UsageStore};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

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

/// Combines in-memory [`UsageStats`] with an optional persistent [`UsageStore`].
///
/// Every `record_*` call updates the in-memory counters immediately and, if a
/// store is configured, sends the record to a single background task that
/// batches writes to reduce spawn overhead and `SQLite` write contention.
pub struct UsageRecorder {
    stats: UsageStats,
    store: Option<Arc<dyn UsageStore>>,
    sender: Option<mpsc::UnboundedSender<UsageRecord>>,
}

impl UsageRecorder {
    /// Creates a new recorder, optionally backed by a persistent store.
    ///
    /// When a store is provided a background flush loop is spawned that drains
    /// records from an mpsc channel in micro-batches (up to 64 at a time).
    #[must_use]
    pub fn new(store: Option<Arc<dyn UsageStore>>) -> Self {
        let sender = store.as_ref().map(|s| {
            let (tx, rx) = mpsc::unbounded_channel::<UsageRecord>();
            let flush_store = Arc::clone(s);
            tokio::spawn(Self::flush_loop(flush_store, rx));
            tx
        });
        Self {
            stats: UsageStats::new(),
            store,
            sender,
        }
    }

    /// Background loop that drains the record channel in micro-batches.
    async fn flush_loop(store: Arc<dyn UsageStore>, mut rx: mpsc::UnboundedReceiver<UsageRecord>) {
        const BATCH_CAP: usize = 64;
        let mut buf: Vec<UsageRecord> = Vec::with_capacity(BATCH_CAP);

        while let Some(record) = rx.recv().await {
            buf.push(record);

            // Drain any additional records already queued without blocking.
            while buf.len() < BATCH_CAP {
                match rx.try_recv() {
                    Ok(r) => buf.push(r),
                    Err(_) => break,
                }
            }

            for record in buf.drain(..) {
                if let Err(e) = store.record(&record).await {
                    tracing::warn!(error = %e, "failed to persist usage record");
                }
            }
        }
    }

    /// Record a successful request with token counts.
    pub fn record_success(
        &self,
        model: &str,
        provider: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) {
        self.stats
            .record_success(model, input_tokens, output_tokens);
        self.persist(model, provider, input_tokens, output_tokens, true);
    }

    /// Record a failed request.
    pub fn record_failure(&self, model: &str, provider: &str) {
        self.stats.record_failure(model);
        self.persist(model, provider, 0, 0, false);
    }

    /// Take a snapshot of in-memory stats.
    #[must_use]
    pub fn snapshot(&self) -> UsageSnapshot {
        self.stats.snapshot()
    }

    /// Pre-load cumulative counters from historical totals (e.g. on startup).
    pub fn preload(&self, model: &str, requests: u64, input_tokens: u64, output_tokens: u64) {
        self.stats
            .total_requests
            .fetch_add(requests, Ordering::Relaxed);
        self.stats
            .success_requests
            .fetch_add(requests, Ordering::Relaxed);
        self.stats
            .input_tokens
            .fetch_add(input_tokens, Ordering::Relaxed);
        self.stats
            .output_tokens
            .fetch_add(output_tokens, Ordering::Relaxed);

        if let Ok(mut map) = self.stats.model_counts.lock() {
            let entry = map.entry(model.to_string()).or_default();
            entry.requests += requests;
            entry.success += requests;
            entry.input_tokens += input_tokens;
            entry.output_tokens += output_tokens;
        }
    }

    /// Returns a reference to the backing store (if configured).
    pub fn store(&self) -> Option<&Arc<dyn UsageStore>> {
        self.store.as_ref()
    }

    fn persist(
        &self,
        model: &str,
        provider: &str,
        input_tokens: u64,
        output_tokens: u64,
        success: bool,
    ) {
        if let Some(sender) = &self.sender {
            let record = UsageRecord {
                model: model.to_string(),
                provider: provider.to_string(),
                input_tokens,
                output_tokens,
                success,
            };
            let _ = sender.send(record);
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
