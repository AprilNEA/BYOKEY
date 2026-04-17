//! Credential routing — API key selection with configurable strategy and per-key state machine.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Strategy used to select the next available key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RoutingStrategy {
    /// Rotate through keys evenly (default).
    #[default]
    RoundRobin,
    /// Always pick the first ready key (prefer earlier entries).
    FillFirst,
}

/// Per-key lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyState {
    /// Key is available for use.
    Ready,
    /// Key is temporarily unavailable until the given instant.
    Cooldown { until: Instant },
    /// Key hit a hard error (e.g. 403) and should not be retried automatically.
    Blocked,
    /// Key has been administratively disabled.
    Disabled,
}

/// A credential router with per-key state machine, configurable strategy, and
/// optional retry cap.
pub struct CredentialRouter {
    /// API keys available for rotation (immutable after construction).
    keys: Vec<String>,
    /// Per-key state, indexed in parallel with `keys`.
    states: Mutex<Vec<KeyState>>,
    /// Atomic counter for round-robin selection.
    index: AtomicUsize,
    /// How long a key stays in cooldown after an error.
    cooldown_duration: Duration,
    /// Selection strategy.
    strategy: RoutingStrategy,
    /// Optional cap on the number of keys to attempt.
    max_retry: Option<usize>,
}

impl CredentialRouter {
    /// Creates a new router with the given keys and cooldown duration.
    ///
    /// All keys start in [`KeyState::Ready`] with priority 0.
    ///
    /// # Panics
    ///
    /// Panics if `keys` is empty.
    #[must_use]
    pub fn new(keys: Vec<String>, cooldown_duration: Duration) -> Self {
        assert!(
            !keys.is_empty(),
            "CredentialRouter requires at least one key"
        );
        let len = keys.len();
        Self {
            keys,
            states: Mutex::new(vec![KeyState::Ready; len]),
            index: AtomicUsize::new(0),
            cooldown_duration,
            strategy: RoutingStrategy::default(),
            max_retry: None,
        }
    }

    /// Builder: set the routing strategy.
    #[must_use]
    pub fn with_strategy(mut self, strategy: RoutingStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Builder: set the maximum number of keys to try before giving up.
    #[must_use]
    pub fn with_max_retry(mut self, max: usize) -> Self {
        self.max_retry = Some(max);
        self
    }

    /// Returns the total number of configured keys (regardless of state).
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Returns `true` if there are no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Returns the configured maximum retry count, if any.
    #[must_use]
    pub fn max_retry(&self) -> Option<usize> {
        self.max_retry
    }

    /// Selects the next available API key, respecting the configured strategy.
    ///
    /// 1. Auto-promotes expired [`KeyState::Cooldown`] entries back to
    ///    [`KeyState::Ready`].
    /// 2. Filters to `Ready` entries only.
    /// 3. Applies the strategy:
    ///    - [`RoundRobin`](RoutingStrategy::RoundRobin): uses the atomic index.
    ///    - [`FillFirst`](RoutingStrategy::FillFirst): returns the first ready
    ///      key.
    ///
    /// Returns `None` if no key is currently ready.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn next_key(&self) -> Option<&str> {
        let len = self.keys.len();
        let now = Instant::now();
        let mut states = self.states.lock().expect("states lock");

        // Step 1: auto-promote expired cooldowns.
        for state in states.iter_mut() {
            if let KeyState::Cooldown { until } = *state
                && now >= until
            {
                *state = KeyState::Ready;
            }
        }

        // Step 2+3: select based on strategy.
        match self.strategy {
            RoutingStrategy::RoundRobin => {
                let start = self.index.fetch_add(1, Ordering::Relaxed) % len;
                for i in 0..len {
                    let idx = (start + i) % len;
                    if states[idx] == KeyState::Ready {
                        return Some(&self.keys[idx]);
                    }
                }
                None
            }
            RoutingStrategy::FillFirst => {
                for (idx, state) in states.iter().enumerate() {
                    if *state == KeyState::Ready {
                        return Some(&self.keys[idx]);
                    }
                }
                None
            }
        }
    }

    /// Marks a key as having encountered an error, placing it in cooldown
    /// for the default duration.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn mark_error(&self, key: &str) {
        if let Some(idx) = self.keys.iter().position(|k| k == key) {
            let mut states = self.states.lock().expect("states lock");
            states[idx] = KeyState::Cooldown {
                until: Instant::now() + self.cooldown_duration,
            };
        }
    }

    /// Marks a key as having encountered an error, placing it in cooldown
    /// for the specified delay (e.g. from a server-provided `Retry-After` value).
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn mark_error_with_delay(&self, key: &str, delay: Duration) {
        if let Some(idx) = self.keys.iter().position(|k| k == key) {
            let mut states = self.states.lock().expect("states lock");
            states[idx] = KeyState::Cooldown {
                until: Instant::now() + delay,
            };
        }
    }

    /// Marks a key as blocked (hard error, e.g. 403 forbidden).
    ///
    /// Blocked keys are never auto-promoted; use [`clear_cooldown`](Self::clear_cooldown)
    /// to manually restore them.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn mark_blocked(&self, key: &str) {
        if let Some(idx) = self.keys.iter().position(|k| k == key) {
            let mut states = self.states.lock().expect("states lock");
            states[idx] = KeyState::Blocked;
        }
    }

    /// Marks a key as disabled (administratively removed from rotation).
    ///
    /// Disabled keys are never auto-promoted; use [`clear_cooldown`](Self::clear_cooldown)
    /// to manually restore them.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn mark_disabled(&self, key: &str) {
        if let Some(idx) = self.keys.iter().position(|k| k == key) {
            let mut states = self.states.lock().expect("states lock");
            states[idx] = KeyState::Disabled;
        }
    }

    /// Clears any non-ready state for a specific key, setting it back to
    /// [`KeyState::Ready`].
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn clear_cooldown(&self, key: &str) {
        if let Some(idx) = self.keys.iter().position(|k| k == key) {
            let mut states = self.states.lock().expect("states lock");
            states[idx] = KeyState::Ready;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ───────────────────────────────────────────────
    // Existing tests (unchanged)
    // ───────────────────────────────────────────────

    #[test]
    fn test_round_robin() {
        let router = CredentialRouter::new(
            vec!["key-a".into(), "key-b".into(), "key-c".into()],
            Duration::from_mins(1),
        );
        let k1 = router.next_key().unwrap().to_string();
        let k2 = router.next_key().unwrap().to_string();
        let k3 = router.next_key().unwrap().to_string();
        let k4 = router.next_key().unwrap().to_string();
        // Round-robin cycles
        assert_eq!(k1, "key-a");
        assert_eq!(k2, "key-b");
        assert_eq!(k3, "key-c");
        assert_eq!(k4, "key-a");
    }

    #[test]
    fn test_cooldown_skips_key() {
        let router =
            CredentialRouter::new(vec!["key-a".into(), "key-b".into()], Duration::from_mins(1));
        // Cool down key-a
        router.mark_error("key-a");
        // Should skip key-a and return key-b
        let k = router.next_key().unwrap();
        assert_eq!(k, "key-b");
    }

    #[test]
    fn test_all_cooled_returns_none() {
        let router =
            CredentialRouter::new(vec!["key-a".into(), "key-b".into()], Duration::from_mins(1));
        router.mark_error("key-a");
        router.mark_error("key-b");
        assert!(router.next_key().is_none());
    }

    #[test]
    fn test_clear_cooldown() {
        let router = CredentialRouter::new(vec!["key-a".into()], Duration::from_mins(1));
        router.mark_error("key-a");
        assert!(router.next_key().is_none());
        router.clear_cooldown("key-a");
        assert!(router.next_key().is_some());
    }

    #[test]
    fn test_single_key() {
        let router = CredentialRouter::new(vec!["only-key".into()], Duration::from_mins(1));
        assert_eq!(router.next_key().unwrap(), "only-key");
        assert_eq!(router.next_key().unwrap(), "only-key");
    }

    #[test]
    fn test_len() {
        let router = CredentialRouter::new(vec!["a".into(), "b".into()], Duration::from_secs(1));
        assert_eq!(router.len(), 2);
        assert!(!router.is_empty());
    }

    #[test]
    #[should_panic(expected = "at least one key")]
    fn test_empty_keys_panics() {
        let _ = CredentialRouter::new(vec![], Duration::from_secs(1));
    }

    // ───────────────────────────────────────────────
    // New tests
    // ───────────────────────────────────────────────

    #[test]
    fn test_fill_first_always_picks_first_ready() {
        let router = CredentialRouter::new(
            vec!["key-a".into(), "key-b".into(), "key-c".into()],
            Duration::from_mins(1),
        )
        .with_strategy(RoutingStrategy::FillFirst);

        // Should always return key-a when all are ready.
        assert_eq!(router.next_key().unwrap(), "key-a");
        assert_eq!(router.next_key().unwrap(), "key-a");
        assert_eq!(router.next_key().unwrap(), "key-a");
    }

    #[test]
    fn test_fill_first_skips_cooled() {
        let router = CredentialRouter::new(
            vec!["key-a".into(), "key-b".into(), "key-c".into()],
            Duration::from_mins(1),
        )
        .with_strategy(RoutingStrategy::FillFirst);

        router.mark_error("key-a");
        assert_eq!(router.next_key().unwrap(), "key-b");

        router.mark_error("key-b");
        assert_eq!(router.next_key().unwrap(), "key-c");

        router.mark_error("key-c");
        assert!(router.next_key().is_none());
    }

    #[test]
    fn test_cooldown_auto_promotion() {
        // Use a tiny cooldown so it expires immediately.
        let router = CredentialRouter::new(vec!["key-a".into()], Duration::from_millis(1));

        router.mark_error("key-a");
        // Wait for the cooldown to expire.
        std::thread::sleep(Duration::from_millis(5));
        // Should auto-promote back to Ready.
        assert_eq!(router.next_key().unwrap(), "key-a");
    }

    #[test]
    fn test_blocked_not_auto_promoted() {
        let router = CredentialRouter::new(
            vec!["key-a".into(), "key-b".into()],
            Duration::from_millis(1),
        );

        router.mark_blocked("key-a");
        std::thread::sleep(Duration::from_millis(5));
        // Blocked keys are never auto-promoted.
        assert_eq!(router.next_key().unwrap(), "key-b");
    }

    #[test]
    fn test_disabled_not_auto_promoted() {
        let router = CredentialRouter::new(
            vec!["key-a".into(), "key-b".into()],
            Duration::from_millis(1),
        );

        router.mark_disabled("key-a");
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(router.next_key().unwrap(), "key-b");
    }

    #[test]
    fn test_clear_cooldown_restores_blocked() {
        let router = CredentialRouter::new(vec!["key-a".into()], Duration::from_mins(1));
        router.mark_blocked("key-a");
        assert!(router.next_key().is_none());
        router.clear_cooldown("key-a");
        assert_eq!(router.next_key().unwrap(), "key-a");
    }

    #[test]
    fn test_clear_cooldown_restores_disabled() {
        let router = CredentialRouter::new(vec!["key-a".into()], Duration::from_mins(1));
        router.mark_disabled("key-a");
        assert!(router.next_key().is_none());
        router.clear_cooldown("key-a");
        assert_eq!(router.next_key().unwrap(), "key-a");
    }

    #[test]
    fn test_max_retry_configuration() {
        let router = CredentialRouter::new(
            vec!["a".into(), "b".into(), "c".into()],
            Duration::from_secs(1),
        );
        assert!(router.max_retry().is_none());

        let router = router.with_max_retry(2);
        assert_eq!(router.max_retry(), Some(2));
    }

    #[test]
    fn test_default_strategy_is_round_robin() {
        let router = CredentialRouter::new(vec!["a".into()], Duration::from_secs(1));
        // Default strategy should behave as round-robin (covered by existing tests).
        assert_eq!(router.next_key().unwrap(), "a");
    }

    #[test]
    fn test_with_strategy_builder() {
        let router = CredentialRouter::new(vec!["a".into()], Duration::from_secs(1))
            .with_strategy(RoutingStrategy::FillFirst);
        assert_eq!(router.next_key().unwrap(), "a");
    }

    #[test]
    fn test_all_blocked_returns_none() {
        let router =
            CredentialRouter::new(vec!["key-a".into(), "key-b".into()], Duration::from_mins(1));
        router.mark_blocked("key-a");
        router.mark_blocked("key-b");
        assert!(router.next_key().is_none());
    }

    #[test]
    fn test_mixed_states() {
        let router = CredentialRouter::new(
            vec![
                "key-a".into(),
                "key-b".into(),
                "key-c".into(),
                "key-d".into(),
            ],
            Duration::from_mins(1),
        )
        .with_strategy(RoutingStrategy::FillFirst);

        router.mark_disabled("key-a");
        router.mark_blocked("key-b");
        router.mark_error("key-c");
        // Only key-d should be available.
        assert_eq!(router.next_key().unwrap(), "key-d");
    }
}
