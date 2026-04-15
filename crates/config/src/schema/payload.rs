use super::{Config, glob_match};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Rules for modifying request payloads based on model patterns.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PayloadRules {
    /// Set params only if they are missing from the request.
    #[serde(default)]
    pub default: Vec<PayloadRule>,
    /// Always override params, replacing any existing values.
    #[serde(default)]
    pub r#override: Vec<PayloadRule>,
    /// Remove specified fields from the request body.
    #[serde(default)]
    pub filter: Vec<PayloadFilterRule>,
}

/// A rule that sets or overrides JSON fields for matching models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadRule {
    /// Model name patterns (glob with `*`).
    pub models: Vec<String>,
    /// JSON path → value pairs to set.
    pub params: HashMap<String, serde_json::Value>,
}

/// A rule that removes JSON fields for matching models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadFilterRule {
    /// Model name patterns (glob with `*`).
    pub models: Vec<String>,
    /// JSON paths to remove.
    pub params: Vec<String>,
}

impl Config {
    /// Applies payload rules (default, override, filter) to a request body.
    ///
    /// - `default` rules: set a value only if the path does not already exist.
    /// - `override` rules: always set the value, replacing existing.
    /// - `filter` rules: remove the specified paths.
    #[must_use]
    pub fn apply_payload_rules(
        &self,
        mut body: serde_json::Value,
        model: &str,
    ) -> serde_json::Value {
        // Apply default rules: only set if missing.
        for rule in &self.payload.default {
            if rule.models.iter().any(|pat| glob_match(pat, model)) {
                for (path, value) in &rule.params {
                    if dot_path_get(&body, path).is_none() {
                        dot_path_set(&mut body, path, value.clone());
                    }
                }
            }
        }

        // Apply override rules: always set.
        for rule in &self.payload.r#override {
            if rule.models.iter().any(|pat| glob_match(pat, model)) {
                for (path, value) in &rule.params {
                    dot_path_set(&mut body, path, value.clone());
                }
            }
        }

        // Apply filter rules: remove paths.
        for rule in &self.payload.filter {
            if rule.models.iter().any(|pat| glob_match(pat, model)) {
                for path in &rule.params {
                    dot_path_remove(&mut body, path);
                }
            }
        }

        body
    }
}

/// Get a value at a dot-separated path (e.g. "a.b.c").
fn dot_path_get<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for key in path.split('.') {
        current = current.get(key)?;
    }
    Some(current)
}

/// Set a value at a dot-separated path, creating intermediate objects as needed.
fn dot_path_set(value: &mut serde_json::Value, path: &str, new_val: serde_json::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for &key in &parts[..parts.len() - 1] {
        if !current.is_object() {
            return;
        }
        let obj = current.as_object_mut().expect("checked is_object");
        if !obj.contains_key(key) {
            obj.insert(
                key.to_string(),
                serde_json::Value::Object(serde_json::Map::default()),
            );
        }
        current = obj.get_mut(key).expect("just inserted");
    }
    if let Some(obj) = current.as_object_mut() {
        obj.insert(parts[parts.len() - 1].to_string(), new_val);
    }
}

/// Remove a value at a dot-separated path.
fn dot_path_remove(value: &mut serde_json::Value, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for &key in &parts[..parts.len() - 1] {
        match current.get_mut(key) {
            Some(next) => current = next,
            None => return,
        }
    }
    if let Some(obj) = current.as_object_mut() {
        obj.remove(parts[parts.len() - 1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_yaml_payload_rules() {
        let yaml = r#"
payload:
  default:
    - models: ["gemini-*"]
      params:
        "generationConfig.thinkingConfig.thinkingBudget": 32768
  override:
    - models: ["gpt-*"]
      params:
        "reasoning.effort": "high"
  filter:
    - models: ["gemini-*"]
      params: ["generationConfig.responseJsonSchema"]
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.payload.default.len(), 1);
        assert_eq!(c.payload.r#override.len(), 1);
        assert_eq!(c.payload.filter.len(), 1);
        assert_eq!(c.payload.default[0].models, vec!["gemini-*"]);
    }

    #[test]
    fn test_apply_payload_default_sets_missing() {
        let yaml = r#"
payload:
  default:
    - models: ["gemini-*"]
      params:
        "generationConfig.thinkingConfig.thinkingBudget": 32768
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let body = serde_json::json!({"model": "gemini-2.0-flash"});
        let result = c.apply_payload_rules(body, "gemini-2.0-flash");
        assert_eq!(
            result["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            32768
        );
    }

    #[test]
    fn test_apply_payload_default_skips_existing() {
        let yaml = r#"
payload:
  default:
    - models: ["gemini-*"]
      params:
        "generationConfig.thinkingConfig.thinkingBudget": 32768
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let body = serde_json::json!({
            "model": "gemini-2.0-flash",
            "generationConfig": {"thinkingConfig": {"thinkingBudget": 8000}}
        });
        let result = c.apply_payload_rules(body, "gemini-2.0-flash");
        assert_eq!(
            result["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            8000
        );
    }

    #[test]
    fn test_apply_payload_override_replaces() {
        let yaml = r#"
payload:
  override:
    - models: ["gpt-*"]
      params:
        "reasoning.effort": "high"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let body = serde_json::json!({"model": "gpt-4o", "reasoning": {"effort": "low"}});
        let result = c.apply_payload_rules(body, "gpt-4o");
        assert_eq!(result["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_apply_payload_filter_removes() {
        let yaml = r#"
payload:
  filter:
    - models: ["gemini-*"]
      params: ["generationConfig.responseJsonSchema"]
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let body = serde_json::json!({
            "model": "gemini-2.0-flash",
            "generationConfig": {"responseJsonSchema": {}, "thinkingConfig": {}}
        });
        let result = c.apply_payload_rules(body, "gemini-2.0-flash");
        assert!(
            result["generationConfig"]
                .as_object()
                .unwrap()
                .get("responseJsonSchema")
                .is_none()
        );
        assert!(
            result["generationConfig"]
                .as_object()
                .unwrap()
                .get("thinkingConfig")
                .is_some()
        );
    }

    #[test]
    fn test_apply_payload_no_match() {
        let yaml = r#"
payload:
  override:
    - models: ["gpt-*"]
      params:
        "reasoning.effort": "high"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let body = serde_json::json!({"model": "claude-opus-4-5"});
        let result = c.apply_payload_rules(body.clone(), "claude-opus-4-5");
        assert_eq!(result, body);
    }

    #[test]
    fn test_dot_path_helpers() {
        let val = serde_json::json!({"a": {"b": {"c": 42}}});
        assert_eq!(dot_path_get(&val, "a.b.c"), Some(&serde_json::json!(42)));
        assert!(dot_path_get(&val, "a.b.d").is_none());
        assert!(dot_path_get(&val, "x.y").is_none());

        let mut val2 = serde_json::json!({"a": {}});
        dot_path_set(&mut val2, "a.b.c", serde_json::json!(99));
        assert_eq!(val2["a"]["b"]["c"], 99);

        let mut val3 = serde_json::json!({"a": {"b": 1, "c": 2}});
        dot_path_remove(&mut val3, "a.b");
        assert!(val3["a"].as_object().unwrap().get("b").is_none());
        assert_eq!(val3["a"]["c"], 2);
    }
}
