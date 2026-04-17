//! Per-provider account selection backed by the `loadwise` load-balancer.
//!
//! An [`AccountSelector`] owns a set of [`AccountNode`]s (each representing
//! one OAuth account or API key) plus a [`Strategy`] and answers the
//! question *"which account should this next request use?"*. Unlike the
//! static [`CredentialRouter`](crate::CredentialRouter), the strategy is
//! chosen per-(provider, family) pair at runtime and can rotate, stick,
//! weight, or score candidates however the user configures.
//!
//! This is the foundation for the configurable `RoutingPolicy` RPC — the
//! selector is built from a policy entry and swapped atomically when the
//! user changes the policy.

use loadwise_core::{Node, SelectionContext, Strategy, Weighted};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// A single account in a selector's pool.
#[derive(Debug, Clone)]
pub struct AccountNode {
    /// Unique account identifier (e.g. the OAuth `account_id` or API-key
    /// label).
    pub account_id: String,
    /// Optional relative weight for weighted strategies. Defaults to `1`.
    pub weight: u32,
}

impl AccountNode {
    #[must_use]
    pub fn new(account_id: impl Into<String>) -> Self {
        Self {
            account_id: account_id.into(),
            weight: 1,
        }
    }

    #[must_use]
    pub fn with_weight(mut self, weight: u32) -> Self {
        self.weight = weight.max(1);
        self
    }
}

impl Node for AccountNode {
    type Id = String;
    fn id(&self) -> &String {
        &self.account_id
    }
}

impl Weighted for AccountNode {
    fn weight(&self) -> u32 {
        self.weight
    }
}

/// Config-friendly strategy kind, mapped to a concrete `loadwise` strategy
/// via [`AccountSelector::new`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    /// Rotate evenly through all accounts.
    #[default]
    RoundRobin,
    /// Nginx smooth weighted round-robin.
    WeightedRoundRobin,
    /// Uniform random selection.
    Random,
    /// Random selection biased by weight.
    WeightedRandom,
    /// Prefer the first account; skip on repeated failures.
    ///
    /// Implemented as a thin wrapper over round-robin with the first node
    /// preferred — mirrors the existing `CredentialRouter` `FillFirst`
    /// semantics without introducing a new loadwise strategy.
    Priority,
}

/// A single resolved (provider, family) → strategy routing rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPolicy {
    /// Provider this policy applies to (e.g. `"claude"`).
    pub provider: String,
    /// Optional family context (e.g. `"claude"`, `"codex"`, `"gemini"`).
    /// When `None` the policy applies to every request for the provider.
    #[serde(default)]
    pub family: Option<String>,
    /// Strategy to use when picking from the account pool.
    #[serde(default)]
    pub strategy: StrategyKind,
    /// Accounts that participate in this pool. Empty means "use every
    /// configured account for the provider".
    #[serde(default)]
    pub accounts: Vec<String>,
    /// Optional per-account weights (`account_id` → weight). Unset accounts
    /// default to weight `1`.
    #[serde(default)]
    pub weights: std::collections::HashMap<String, u32>,
}

/// Owns a strategy + a node pool and picks accounts on demand.
///
/// Use [`AccountSelector::rebuild`] when the account pool changes (add or
/// remove OAuth accounts) so the strategy fingerprints stay consistent.
pub struct AccountSelector {
    nodes: Vec<AccountNode>,
    strategy: Box<dyn Strategy<AccountNode> + Send + Sync>,
    last_picked: Mutex<Option<String>>,
}

impl AccountSelector {
    /// Build a selector from a policy and the current account pool.
    ///
    /// The `accounts` slice should list every account currently available
    /// for the provider; `policy.accounts` is then used to filter if
    /// non-empty.
    #[must_use]
    pub fn new(policy: &RoutingPolicy, available_accounts: &[&str]) -> Self {
        let pool: Vec<AccountNode> = available_accounts
            .iter()
            .filter(|id| policy.accounts.is_empty() || policy.accounts.iter().any(|a| a == *id))
            .map(|id| {
                let weight = policy.weights.get(*id).copied().unwrap_or(1);
                AccountNode::new(*id).with_weight(weight)
            })
            .collect();
        Self {
            strategy: build_strategy(policy.strategy),
            nodes: pool,
            last_picked: Mutex::new(None),
        }
    }

    /// Number of accounts in the pool.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Pick the next account to serve a request, or `None` when the pool
    /// is empty. The returned `String` is the chosen `account_id`.
    #[must_use]
    pub fn pick(&self, ctx: &SelectionContext) -> Option<String> {
        let idx = self.strategy.select(&self.nodes, ctx)?;
        let picked = self.nodes.get(idx)?.account_id.clone();
        if let Ok(mut last) = self.last_picked.lock() {
            *last = Some(picked.clone());
        }
        Some(picked)
    }

    /// Last account returned by [`pick`](Self::pick), if any.
    #[must_use]
    pub fn last_picked(&self) -> Option<String> {
        self.last_picked.lock().ok().and_then(|g| g.clone())
    }
}

impl From<byokey_config::PolicyStrategyKind> for StrategyKind {
    fn from(value: byokey_config::PolicyStrategyKind) -> Self {
        use byokey_config::PolicyStrategyKind as P;
        match value {
            P::RoundRobin => Self::RoundRobin,
            P::WeightedRoundRobin => Self::WeightedRoundRobin,
            P::Random => Self::Random,
            P::WeightedRandom => Self::WeightedRandom,
            P::Priority => Self::Priority,
        }
    }
}

impl From<&byokey_config::RoutingPolicyEntry> for RoutingPolicy {
    fn from(entry: &byokey_config::RoutingPolicyEntry) -> Self {
        Self {
            provider: entry.provider.to_string(),
            family: entry.family.clone(),
            strategy: entry.strategy.into(),
            accounts: entry.accounts.clone(),
            weights: entry.weights.clone(),
        }
    }
}

/// Map a [`StrategyKind`] to a concrete `loadwise` strategy.
fn build_strategy(kind: StrategyKind) -> Box<dyn Strategy<AccountNode> + Send + Sync> {
    use loadwise_core::strategy::{Random, RoundRobin, WeightedRandom, WeightedRoundRobin};
    match kind {
        StrategyKind::RoundRobin | StrategyKind::Priority => Box::new(RoundRobin::new()),
        StrategyKind::WeightedRoundRobin => Box::new(WeightedRoundRobin::new()),
        StrategyKind::Random => Box::new(Random::new()),
        StrategyKind::WeightedRandom => Box::new(WeightedRandom::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(strategy: StrategyKind, accounts: &[&str]) -> RoutingPolicy {
        RoutingPolicy {
            provider: "claude".into(),
            family: None,
            strategy,
            accounts: accounts.iter().map(|s| (*s).to_string()).collect(),
            weights: std::collections::HashMap::default(),
        }
    }

    #[test]
    fn round_robin_cycles_through_pool() {
        let p = policy(StrategyKind::RoundRobin, &[]);
        let sel = AccountSelector::new(&p, &["a", "b", "c"]);
        let ctx = SelectionContext::default();
        assert_eq!(sel.pick(&ctx).as_deref(), Some("a"));
        assert_eq!(sel.pick(&ctx).as_deref(), Some("b"));
        assert_eq!(sel.pick(&ctx).as_deref(), Some("c"));
        assert_eq!(sel.pick(&ctx).as_deref(), Some("a"));
    }

    #[test]
    fn accounts_filter_restricts_pool() {
        let p = policy(StrategyKind::RoundRobin, &["a", "c"]);
        let sel = AccountSelector::new(&p, &["a", "b", "c"]);
        assert_eq!(sel.len(), 2);
        let ctx = SelectionContext::default();
        assert_eq!(sel.pick(&ctx).as_deref(), Some("a"));
        assert_eq!(sel.pick(&ctx).as_deref(), Some("c"));
    }

    #[test]
    fn empty_pool_returns_none() {
        let p = policy(StrategyKind::RoundRobin, &[]);
        let sel = AccountSelector::new(&p, &[]);
        assert!(sel.pick(&SelectionContext::default()).is_none());
    }

    #[test]
    fn last_picked_tracks_selection() {
        let p = policy(StrategyKind::RoundRobin, &[]);
        let sel = AccountSelector::new(&p, &["x", "y"]);
        assert_eq!(sel.last_picked(), None);
        let _ = sel.pick(&SelectionContext::default());
        assert_eq!(sel.last_picked().as_deref(), Some("x"));
    }

    #[test]
    fn weighted_round_robin_respects_weights() {
        let mut p = policy(StrategyKind::WeightedRoundRobin, &[]);
        p.weights.insert("a".into(), 3);
        p.weights.insert("b".into(), 1);
        let sel = AccountSelector::new(&p, &["a", "b"]);
        let ctx = SelectionContext::default();
        let picks: Vec<String> = (0..4).filter_map(|_| sel.pick(&ctx)).collect();
        let a = picks.iter().filter(|p| p.as_str() == "a").count();
        let b = picks.iter().filter(|p| p.as_str() == "b").count();
        assert_eq!(a, 3, "a should win 3 of 4 picks");
        assert_eq!(b, 1, "b should win 1 of 4 picks");
    }
}
