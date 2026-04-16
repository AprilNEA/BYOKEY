//! Structured representation of Ampcode balance information.
//!
//! The API returns a human-readable `displayText` string. This module
//! provides [`BalanceInfo`] which parses that string into typed fields.

use crate::error::{AmpcodeError, Result};

/// The plan type inferred from the balance display text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Plan {
    /// Amp Free tier with replenishing balance.
    Free,
    /// Individual/paid credits (no replenishment).
    IndividualCredits,
    /// Unknown or future plan type.
    Unknown(String),
}

/// Structured balance information parsed from `displayText`.
///
/// Constructed via [`BalanceInfo::parse`].
#[derive(Debug, Clone)]
pub struct BalanceInfo {
    /// The raw `displayText` string from the API, preserved for display.
    pub display_text: String,
    /// Username shown in the display text, if present.
    pub user: Option<String>,
    /// Inferred plan type.
    pub plan: Plan,
    /// Current remaining balance in USD.
    pub remaining_dollars: Option<f64>,
    /// Total balance for the period in USD.
    pub total_dollars: Option<f64>,
    /// Hourly replenishment rate in USD. `None` for non-free plans.
    pub replenish_rate_dollars: Option<f64>,
    /// Remaining credit dollars for "Individual credits" plans.
    pub credits_dollars: Option<f64>,
    /// Bonus percentage active on the account (e.g. `20` for 20%).
    pub bonus_percent: Option<u32>,
    /// Days remaining for the active bonus.
    pub bonus_days_remaining: Option<u32>,
}

impl BalanceInfo {
    /// Parse a `displayText` string from the `userDisplayBalanceInfo` API response.
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::BalanceParse`] if the text does not match any
    /// known format.
    pub fn parse(display_text: impl Into<String>) -> Result<Self> {
        let text = display_text.into();
        // The API may return multi-line text:
        //   "Signed in as user@example.com\nAmp Free: $4.23/$10.00 remaining ..."
        //   "Signed in as user@example.com\nIndividual credits: $42 remaining ..."
        // Normalize to single-line by joining user line with content line.
        let normalized = normalize_display_text(&text);
        if let Some(info) = try_parse_free(&normalized) {
            return Ok(info.with_display_text(&text));
        }
        if let Some(info) = try_parse_individual_credits(&normalized) {
            return Ok(info.with_display_text(&text));
        }
        if let Some(info) = try_parse_bare_credits(&normalized) {
            return Ok(info.with_display_text(&text));
        }
        Err(AmpcodeError::BalanceParse(text))
    }
}

impl BalanceInfo {
    fn with_display_text(mut self, text: &str) -> Self {
        self.display_text = text.to_string();
        self
    }
}

// ── Private parsing helpers ───────────────────────────────────────────────────

/// Normalize multi-line display text into a single line.
///
/// The API returns `"Signed in as user\nIndividual credits: ..."` — we
/// join lines with a space so the existing parsers can match.
/// Also strips trailing ` - https://...` URL suffixes.
fn normalize_display_text(text: &str) -> String {
    let joined = text.lines().map(str::trim).collect::<Vec<_>>().join(" ");
    // Strip trailing " - https://..." settings link.
    if let Some(pos) = joined.find(" - https://") {
        joined[..pos].to_string()
    } else {
        joined
    }
}

/// Parse a dollar string like `"$4.23"`, `"-$0.03"`, or `"10"` into `f64`.
fn parse_dollars(s: &str) -> Option<f64> {
    let s = s.trim();
    let (neg, s) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };
    let s = s.strip_prefix('$').unwrap_or(s);
    let val: f64 = s.parse().ok()?;
    Some(if neg { -val } else { val })
}

/// Parse bonus fragment like `"+20% bonus for 3 more days"`.
fn parse_bonus(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().strip_prefix('+').unwrap_or(s.trim());
    let (pct_str, rest) = s.split_once('%')?;
    let pct: u32 = pct_str.trim().parse().ok()?;
    let days_str = rest.trim().strip_prefix("bonus for ")?;
    let (days_part, _) = days_str.split_once(' ')?;
    let days: u32 = days_part.parse().ok()?;
    Some((pct, days))
}

/// Free tier: `"Signed in as <user> Amp Free: $<rem>/$<total> remaining
/// (replenishes +$<rate>/hour[ +N% bonus for N more days])"`.
fn try_parse_free(text: &str) -> Option<BalanceInfo> {
    let rest = text.strip_prefix("Signed in as ")?;
    let (user_part, balance_part) = rest.split_once(" Amp Free: ")?;
    let (amounts_str, paren_raw) = balance_part.split_once(" remaining (")?;
    let paren_content = paren_raw.strip_suffix(')')?;

    let (rem_str, total_str) = amounts_str.split_once('/')?;
    let remaining_dollars = parse_dollars(rem_str);
    let total_dollars = parse_dollars(total_str);

    // Split replenishment rate from optional bonus.
    // Format: "replenishes +$0.50/hour" or "replenishes +$0.50/hour +20% bonus for 3 more days"
    let (replenish_part, bonus_part) = split_rate_and_bonus(paren_content);

    let rate_str = replenish_part
        .strip_prefix("replenishes +")?
        .split('/')
        .next()?;
    let replenish_rate_dollars = parse_dollars(rate_str);

    let (bonus_percent, bonus_days_remaining) = if bonus_part.is_empty() {
        (None, None)
    } else {
        parse_bonus(bonus_part).map_or((None, None), |(p, d)| (Some(p), Some(d)))
    };

    Some(BalanceInfo {
        display_text: text.to_string(),
        user: Some(user_part.to_string()),
        plan: Plan::Free,
        remaining_dollars,
        total_dollars,
        replenish_rate_dollars,
        credits_dollars: None,
        bonus_percent,
        bonus_days_remaining,
    })
}

/// Split `"replenishes +$0.50/hour +20% bonus for 3 more days"` into
/// the rate portion and the bonus portion.
fn split_rate_and_bonus(s: &str) -> (&str, &str) {
    // Find " +" where the part after "+" contains "%" (bonus indicator).
    // We search backwards to avoid matching the "+$" in the rate itself.
    let bytes = s.as_bytes();
    for i in (1..s.len()).rev() {
        if bytes[i] == b'+' && bytes[i - 1] == b' ' && s[i..].contains('%') {
            return (s[..i - 1].trim(), &s[i..]);
        }
    }
    (s, "")
}

/// Individual credits: `"Signed in as <user> Individual credits: $<amount> remaining"`.
fn try_parse_individual_credits(text: &str) -> Option<BalanceInfo> {
    let rest = text.strip_prefix("Signed in as ")?;
    let (user_part, credits_part) = rest.split_once(" Individual credits: ")?;
    let credits_str = credits_part.strip_suffix(" remaining")?;
    let credits_dollars = parse_dollars(credits_str);

    Some(BalanceInfo {
        display_text: text.to_string(),
        user: Some(user_part.to_string()),
        plan: Plan::IndividualCredits,
        remaining_dollars: None,
        total_dollars: None,
        replenish_rate_dollars: None,
        credits_dollars,
        bonus_percent: None,
        bonus_days_remaining: None,
    })
}

/// Bare credits: `"$<amount> remaining"`.
fn try_parse_bare_credits(text: &str) -> Option<BalanceInfo> {
    let credits_str = text.strip_suffix(" remaining")?;
    let credits_dollars = parse_dollars(credits_str)?;

    Some(BalanceInfo {
        display_text: text.to_string(),
        user: None,
        plan: Plan::IndividualCredits,
        remaining_dollars: None,
        total_dollars: None,
        replenish_rate_dollars: None,
        credits_dollars: Some(credits_dollars),
        bonus_percent: None,
        bonus_days_remaining: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_tier_basic() {
        let text = "Signed in as alice@example.com Amp Free: $4.23/$10.00 remaining (replenishes +$0.50/hour)";
        let info = BalanceInfo::parse(text).unwrap();
        assert_eq!(info.user.as_deref(), Some("alice@example.com"));
        assert_eq!(info.plan, Plan::Free);
        assert!((info.remaining_dollars.unwrap() - 4.23).abs() < f64::EPSILON);
        assert!((info.total_dollars.unwrap() - 10.0).abs() < f64::EPSILON);
        assert!((info.replenish_rate_dollars.unwrap() - 0.5).abs() < f64::EPSILON);
        assert!(info.credits_dollars.is_none());
        assert!(info.bonus_percent.is_none());
        assert!(info.bonus_days_remaining.is_none());
    }

    #[test]
    fn free_tier_with_bonus() {
        let text = "Signed in as bob@test.io Amp Free: $7.50/$10.00 remaining (replenishes +$0.42/hour +20% bonus for 3 more days)";
        let info = BalanceInfo::parse(text).unwrap();
        assert_eq!(info.plan, Plan::Free);
        assert!((info.remaining_dollars.unwrap() - 7.5).abs() < f64::EPSILON);
        assert!((info.replenish_rate_dollars.unwrap() - 0.42).abs() < f64::EPSILON);
        assert_eq!(info.bonus_percent, Some(20));
        assert_eq!(info.bonus_days_remaining, Some(3));
    }

    #[test]
    fn individual_credits() {
        let text = "Signed in as bob@example.com Individual credits: $42.00 remaining";
        let info = BalanceInfo::parse(text).unwrap();
        assert_eq!(info.user.as_deref(), Some("bob@example.com"));
        assert_eq!(info.plan, Plan::IndividualCredits);
        assert!((info.credits_dollars.unwrap() - 42.0).abs() < f64::EPSILON);
        assert!(info.remaining_dollars.is_none());
    }

    #[test]
    fn bare_credits() {
        let text = "$5.00 remaining";
        let info = BalanceInfo::parse(text).unwrap();
        assert!(info.user.is_none());
        assert_eq!(info.plan, Plan::IndividualCredits);
        assert!((info.credits_dollars.unwrap() - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unknown_format_is_error() {
        let text = "Some future format we don't know about";
        let err = BalanceInfo::parse(text).unwrap_err();
        assert!(matches!(err, AmpcodeError::BalanceParse(_)));
    }

    #[test]
    fn parse_dollars_variants() {
        assert!((parse_dollars("$4.23").unwrap() - 4.23).abs() < f64::EPSILON);
        assert!((parse_dollars("$10").unwrap() - 10.0).abs() < f64::EPSILON);
        assert!((parse_dollars("0.50").unwrap() - 0.5).abs() < f64::EPSILON);
        assert!(parse_dollars("").is_none());
        assert!(parse_dollars("not-a-number").is_none());
    }

    #[test]
    fn display_text_preserved() {
        let text = "$99.99 remaining";
        let info = BalanceInfo::parse(text).unwrap();
        assert_eq!(info.display_text, text);
    }
}
