//! Credential routing — round-robin API key selection with error cooldown.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// A round-robin credential router with per-key cooldown.
pub struct CredentialRouter {
    /// API keys available for rotation.
    keys: Vec<String>,
    /// Atomic counter for round-robin selection.
    index: AtomicUsize,
    /// Per-key cooldown state: `Some(until)` means the key is cooled down.
    cooldowns: Mutex<Vec<Option<Instant>>>,
    /// How long a key stays in cooldown after an error.
    cooldown_duration: Duration,
}

impl CredentialRouter {
    /// Creates a new router with the given keys and cooldown duration.
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
            index: AtomicUsize::new(0),
            cooldowns: Mutex::new(vec![None; len]),
            cooldown_duration,
        }
    }

    /// Returns the number of configured keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Returns `true` if there are no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Selects the next available API key using round-robin, skipping cooled-down keys.
    ///
    /// Returns `None` if all keys are currently in cooldown.
    ///
    /// # Panics
    ///
    /// Panics if the internal cooldown mutex is poisoned.
    pub fn next_key(&self) -> Option<&str> {
        let len = self.keys.len();
        let start = self.index.fetch_add(1, Ordering::Relaxed) % len;
        let now = Instant::now();
        let cooldowns = self.cooldowns.lock().expect("cooldown lock");

        for i in 0..len {
            let idx = (start + i) % len;
            if cooldowns[idx].is_some_and(|until| now < until) {
                continue;
            }
            return Some(&self.keys[idx]);
        }
        None // all keys in cooldown
    }

    /// Marks a key as having encountered an error, placing it in cooldown.
    ///
    /// # Panics
    ///
    /// Panics if the internal cooldown mutex is poisoned.
    pub fn mark_error(&self, key: &str) {
        if let Some(idx) = self.keys.iter().position(|k| k == key) {
            let mut cooldowns = self.cooldowns.lock().expect("cooldown lock");
            cooldowns[idx] = Some(Instant::now() + self.cooldown_duration);
        }
    }

    /// Clears the cooldown for a specific key.
    ///
    /// # Panics
    ///
    /// Panics if the internal cooldown mutex is poisoned.
    pub fn clear_cooldown(&self, key: &str) {
        if let Some(idx) = self.keys.iter().position(|k| k == key) {
            let mut cooldowns = self.cooldowns.lock().expect("cooldown lock");
            cooldowns[idx] = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_round_robin() {
        let router = CredentialRouter::new(
            vec!["key-a".into(), "key-b".into(), "key-c".into()],
            Duration::from_secs(60),
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
        let router = CredentialRouter::new(
            vec!["key-a".into(), "key-b".into()],
            Duration::from_secs(60),
        );
        // Cool down key-a
        router.mark_error("key-a");
        // Should skip key-a and return key-b
        let k = router.next_key().unwrap();
        assert_eq!(k, "key-b");
    }

    #[test]
    fn test_all_cooled_returns_none() {
        let router = CredentialRouter::new(
            vec!["key-a".into(), "key-b".into()],
            Duration::from_secs(60),
        );
        router.mark_error("key-a");
        router.mark_error("key-b");
        assert!(router.next_key().is_none());
    }

    #[test]
    fn test_clear_cooldown() {
        let router = CredentialRouter::new(vec!["key-a".into()], Duration::from_secs(60));
        router.mark_error("key-a");
        assert!(router.next_key().is_none());
        router.clear_cooldown("key-a");
        assert!(router.next_key().is_some());
    }

    #[test]
    fn test_single_key() {
        let router = CredentialRouter::new(vec!["only-key".into()], Duration::from_secs(60));
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
}
