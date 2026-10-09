//! Normalisation of Anthropic Messages request bodies before they go
//! upstream: empty system blocks, Claude Code's billing header, thinking
//! settings the API rejects, the `[1m]` model suffix and beta headers.

use axum::http::HeaderMap;
use byokey_types::ThinkingCapability;
use serde_json::Value;

/// Default thinking budget (tokens) for `Auto` mode on legacy Claude models
/// that require an explicit `budget_tokens` value with `thinking.type: "enabled"`.
const DEFAULT_AUTO_BUDGET: u32 = 10_000;

/// Claude Code's billing header carries a `cch=<hash>;` segment that changes
/// between requests. Every change invalidates the upstream prompt cache for
/// the whole system prompt, so it is pinned to one value.
const STABLE_CCH: &str = "cch=00000;";

/// Pin the `cch=` segment of a Claude Code billing header, if `text` is one.
fn stabilize_billing_header(text: &str) -> Option<String> {
    if !text.starts_with("x-anthropic-billing-header:") {
        return None;
    }
    let start = text.find("cch=")?;
    let end = start + text[start..].find(';')? + 1;
    if &text[start..end] == STABLE_CCH {
        return None;
    }
    Some(format!("{}{STABLE_CCH}{}", &text[..start], &text[end..]))
}

/// Strip empty system content to prevent "text content blocks must be non-empty" API error.
///
/// Handles both string (`"system": ""`) and array forms
/// (`"system": [{"type": "text", "text": ""}]`). Also pins the `cch=`
/// segment of Claude Code's billing header so it stops busting the prompt
/// cache.
pub(super) fn sanitize_system(body: &mut Value) {
    match body.get_mut("system") {
        Some(Value::String(s)) => {
            if let Some(fixed) = stabilize_billing_header(s) {
                *s = fixed;
            }
        }
        Some(Value::Array(arr)) => {
            for text in arr.iter_mut().filter_map(|b| b.get_mut("text")) {
                if let Some(fixed) = text.as_str().and_then(stabilize_billing_header) {
                    *text = Value::String(fixed);
                }
            }
        }
        _ => {}
    }
    let dominated_by_empty = match body.get("system") {
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(arr)) => arr.iter().all(|block| {
            block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(str::is_empty)
        }),
        _ => false,
    };

    if dominated_by_empty {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("system");
        }
        return;
    }

    // Filter individual empty text blocks from an array that has some non-empty blocks.
    if let Some(arr) = body.get_mut("system").and_then(Value::as_array_mut) {
        arr.retain(|block| {
            !block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(str::is_empty)
        });
    }
}

/// Sanitize thinking configuration before sending to the Anthropic API.
///
/// Two cases require intervention:
///
/// 1. **`tool_choice` conflict** — the API rejects `thinking` when `tool_choice.type`
///    is `"any"` or `"tool"`. Strip all thinking-related fields.
///    Aligned with upstream `disableThinkingIfToolChoiceForced`.
///
/// 2. **`thinking.type: "auto"`** — not a valid Anthropic API value (returns 400).
///    Instead of stripping (which silently disables thinking), translate based on
///    model capability:
///    - Hybrid (4.6): `"auto"` → `"adaptive"` — let Claude decide thinking depth.
///    - `BudgetOnly` (legacy): `"auto"` → `"enabled"` + default budget.
///    - No thinking support: strip entirely.
pub(super) fn sanitize_thinking(body: &mut Value) {
    let forced_tool = body
        .get("tool_choice")
        .and_then(|tc| tc.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| t == "any" || t == "tool");

    if forced_tool {
        strip_thinking_fields(body);
        return;
    }

    let is_auto = body
        .get("thinking")
        .and_then(|th| th.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| t == "auto");

    if is_auto {
        let model = body.get("model").and_then(Value::as_str).unwrap_or("");
        match byokey_provider::thinking_capability(model) {
            Some(ThinkingCapability::Hybrid) => {
                // 4.6 models: "auto" semantically means "let the model decide".
                body["thinking"] = serde_json::json!({"type": "adaptive"});
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("output_config");
                }
            }
            Some(_) => {
                // Legacy models: "enabled" requires budget_tokens; use default.
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": DEFAULT_AUTO_BUDGET
                });
            }
            None => {
                // Model has no thinking support — strip to avoid API error.
                strip_thinking_fields(body);
            }
        }
    }

    // Anthropic rejects temperature != 1 when thinking is active.
    normalize_temperature_for_thinking(body);
}

/// Force `temperature` to `1` when thinking is enabled/adaptive/auto.
///
/// Anthropic API returns 400 if temperature is set to anything other than 1
/// while a thinking mode is active. When thinking was stripped (e.g. by
/// `tool_choice` conflict), we leave temperature as-is so non-thinking requests
/// keep their original sampling behaviour.
fn normalize_temperature_for_thinking(body: &mut Value) {
    let thinking_active = body
        .get("thinking")
        .and_then(|th| th.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| matches!(t, "enabled" | "adaptive" | "auto"));

    if !thinking_active {
        return;
    }

    match body.get("temperature") {
        // temperature == 1 is already valid; no temperature field is fine too.
        None => {}
        Some(v) if v.as_f64() == Some(1.0) => {}
        Some(_) => {
            body["temperature"] = serde_json::json!(1);
        }
    }
}

/// Returns `true` if a Claude thinking block signature looks valid.
///
/// Valid Anthropic-generated signatures start with `E` or `R` (after
/// stripping an optional `<prefix>#` cache key). Thinking blocks a client
/// carried over from another vendor's model use a different format and
/// would be rejected by the Claude API if forwarded.
fn has_valid_claude_signature(sig: &str) -> bool {
    let sig = sig.trim();
    if sig.is_empty() {
        return false;
    }
    let core = if let Some(idx) = sig.find('#') {
        sig[idx + 1..].trim()
    } else {
        sig
    };
    if core.is_empty() {
        return false;
    }
    matches!(core.as_bytes()[0], b'E' | b'R')
}

/// Strip thinking blocks with non-Anthropic signatures from
/// `messages[].content[]` so they don't trip the Claude API on the way out.
pub(super) fn strip_invalid_thinking_signatures(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for msg in messages {
        let Some(content) = msg.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        content.retain(|block| {
            if block.get("type").and_then(Value::as_str) != Some("thinking") {
                return true;
            }
            let sig = block.get("signature").and_then(Value::as_str).unwrap_or("");
            has_valid_claude_signature(sig)
        });
    }
}

/// Remove thinking-related fields and associated adaptive controls.
fn strip_thinking_fields(body: &mut Value) {
    if let Some(obj) = body.as_object_mut() {
        obj.remove("thinking");
        if let Some(oc) = obj.get_mut("output_config").and_then(Value::as_object_mut) {
            oc.remove("effort");
            if oc.is_empty() {
                obj.remove("output_config");
            }
        }
    }
}

/// Beta that unlocks the 1M-token context window on Anthropic's API.
pub(super) const CONTEXT_1M_BETA: &str = "context-1m-2025-08-07";

/// Claude Code and Claude Desktop pick a model's 1M-context variant by
/// appending `[1m]` to its id, and Claude Desktop sends that spelling to a
/// gateway as-is. Upstreams reject it, so it is taken off `body.model`.
/// Returns whether it was there, so the caller can ask for the long context
/// in the upstream's own terms. Runs before anything that looks the model up.
pub(super) fn take_long_context_suffix(body: &mut Value) -> bool {
    let Some(bare) = body
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.strip_suffix("[1m]"))
    else {
        return false;
    };
    body["model"] = Value::String(bare.to_owned());
    true
}

/// Merge betas from the request body's `betas` array, the client's
/// `anthropic-beta` HTTP header and `extra` into the base beta string, then
/// strip the body field so the upstream API doesn't reject it as unknown.
pub(super) fn build_beta_header(
    body: &mut Value,
    client_headers: &HeaderMap,
    base: &str,
    extra: Option<&str>,
) -> String {
    let mut betas: Vec<&str> = base.split(',').filter(|beta| !beta.is_empty()).collect();
    if let Some(extra) = extra
        && !betas.contains(&extra)
    {
        betas.push(extra);
    }

    // Merge from client's `anthropic-beta` HTTP header (comma-separated).
    if let Some(hv) = client_headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
    {
        for token in hv.split(',') {
            let token = token.trim();
            if !token.is_empty() && !betas.contains(&token) {
                betas.push(token);
            }
        }
    }

    // Merge from body's `betas` array (BYOKEY client-to-proxy convention).
    if let Some(arr) = body.get("betas").and_then(Value::as_array) {
        for b in arr {
            if let Some(s) = b.as_str()
                && !s.is_empty()
                && !betas.contains(&s)
            {
                betas.push(s);
            }
        }
    }
    let betas = betas.join(",");
    // Strip `betas` — it's a client-to-proxy field, not a valid API field.
    if let Some(obj) = body.as_object_mut() {
        obj.remove("betas");
    }
    betas
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn long_context_suffix_comes_off_the_model_and_becomes_a_beta() {
        let mut body = json!({"model": "claude-sonnet-5[1m]", "betas": ["x-beta"]});
        let long_context = take_long_context_suffix(&mut body);
        assert!(long_context);
        assert_eq!(body["model"], "claude-sonnet-5");
        let beta = build_beta_header(
            &mut body,
            &HeaderMap::new(),
            "",
            long_context.then_some(CONTEXT_1M_BETA),
        );
        let betas: Vec<&str> = beta.split(',').collect();
        assert!(betas.contains(&CONTEXT_1M_BETA));
        assert!(betas.contains(&"x-beta"));
        assert!(body.get("betas").is_none());

        let mut body = json!({"model": "claude-sonnet-5"});
        assert!(!take_long_context_suffix(&mut body));
        assert_eq!(body["model"], "claude-sonnet-5");
        let beta = build_beta_header(&mut body, &HeaderMap::new(), "", None);
        assert!(!beta.contains(CONTEXT_1M_BETA));

        let mut body = json!({"max_tokens": 1});
        assert!(!take_long_context_suffix(&mut body), "no model at all");
    }

    #[test]
    fn the_billing_header_cch_segment_is_pinned() {
        let header =
            "x-anthropic-billing-header: cc_version=2.1.282.7f3a; cc_entrypoint=cli; cch=a1b2c;";
        let mut body = json!({
            "system": [
                {"type": "text", "text": header, "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "You are Claude Code."}
            ]
        });
        sanitize_system(&mut body);
        assert_eq!(
            body["system"][0]["text"],
            "x-anthropic-billing-header: cc_version=2.1.282.7f3a; cc_entrypoint=cli; cch=00000;"
        );
        assert_eq!(body["system"][1]["text"], "You are Claude Code.");

        let mut body = json!({"system": header});
        sanitize_system(&mut body);
        assert!(body["system"].as_str().unwrap().ends_with("cch=00000;"));

        // Not a billing header, or no cch: untouched.
        assert!(stabilize_billing_header("cch=zzz; something").is_none());
        assert!(stabilize_billing_header("x-anthropic-billing-header: cc_version=1;").is_none());
        assert!(stabilize_billing_header("x-anthropic-billing-header: cch=00000;").is_none());
    }

    // ── sanitize_thinking: tool_choice conflict ────────────────────────

    #[test]
    fn tool_choice_any_strips_thinking() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "enabled", "budget_tokens": 10000},
            "tool_choice": {"type": "any"},
            "output_config": {"effort": "high"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn tool_choice_tool_strips_thinking() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "adaptive"},
            "tool_choice": {"type": "tool", "name": "get_weather"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn tool_choice_auto_does_not_strip() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "adaptive"},
            "tool_choice": {"type": "auto"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    // ── sanitize_thinking: "auto" translation ──────────────────────────

    #[test]
    fn auto_on_hybrid_model_becomes_adaptive() {
        // claude-opus-5-5 is Hybrid → should translate to "adaptive".
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "auto"},
            "output_config": {"effort": "high"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");
        // output_config should be removed — adaptive picks its own effort.
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn auto_on_unknown_model_strips_thinking() {
        // Unknown model has no thinking support → strip entirely.
        let mut body = json!({
            "model": "gpt-4o",
            "thinking": {"type": "auto"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
    }

    // ── sanitize_thinking: valid types pass through ────────────────────

    #[test]
    fn enabled_type_passes_through() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "enabled", "budget_tokens": 8000}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8000);
    }

    #[test]
    fn adaptive_type_passes_through() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    #[test]
    fn no_thinking_field_is_noop() {
        let mut body = json!({"model": "claude-opus-5-5", "max_tokens": 1024});
        let expected = body.clone();
        sanitize_thinking(&mut body);
        assert_eq!(body, expected);
    }

    // ── strip_thinking_fields ──────────────────────────────────────────

    #[test]
    fn strip_cleans_output_config_effort() {
        let mut body = json!({
            "thinking": {"type": "enabled"},
            "output_config": {"effort": "high", "format": "json"}
        });
        strip_thinking_fields(&mut body);
        assert!(body.get("thinking").is_none());
        // "format" remains, only "effort" removed.
        assert!(body["output_config"].get("effort").is_none());
        assert_eq!(body["output_config"]["format"], "json");
    }

    #[test]
    fn strip_removes_empty_output_config() {
        let mut body = json!({
            "thinking": {"type": "enabled"},
            "output_config": {"effort": "high"}
        });
        strip_thinking_fields(&mut body);
        assert!(body.get("output_config").is_none());
    }

    // ── normalize_temperature_for_thinking ─────────────────────────────

    #[test]
    fn adaptive_thinking_coerces_temperature_to_one() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 0,
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn enabled_thinking_coerces_temperature_to_one() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 0.2,
            "thinking": {"type": "enabled", "budget_tokens": 2048}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn temperature_one_with_thinking_is_unchanged() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 1,
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn no_thinking_leaves_temperature_alone() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 0,
            "messages": [{"role": "user", "content": "hi"}]
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 0);
    }

    #[test]
    fn forced_tool_choice_strips_thinking_keeps_temperature() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 0,
            "thinking": {"type": "adaptive"},
            "tool_choice": {"type": "any"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
        // Temperature should remain at 0 — thinking was stripped.
        assert_eq!(body["temperature"], 0);
    }

    #[test]
    fn no_temperature_with_thinking_is_fine() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("temperature").is_none());
    }
}
