use byokey_types::ProviderId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_true() -> bool {
    true
}

/// Configuration for a single API key entry within a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    /// The API key value.
    pub api_key: String,
    /// Optional label for identification in logs.
    #[serde(default)]
    pub label: Option<String>,
}

/// Configuration for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Single API key (shorthand, takes precedence for simple configs).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Multiple API keys with round-robin routing.
    #[serde(default)]
    pub api_keys: Vec<ApiKeyEntry>,
    /// Whether this provider is enabled (defaults to `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Always route requests to this provider instead (e.g. `backend: copilot`
    /// lets Gemini requests go through GitHub Copilot).
    #[serde(default)]
    pub backend: Option<ProviderId>,
    /// Fallback provider to use when the primary provider fails.
    #[serde(default)]
    pub fallback: Option<ProviderId>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            api_keys: Vec::new(),
            enabled: true,
            backend: None,
            fallback: None,
        }
    }
}

impl ProviderConfig {
    /// Returns all configured API keys (merging `api_key` and `api_keys`).
    /// If `api_key` is set, it's treated as the first entry.
    #[must_use]
    pub fn all_api_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = Vec::new();
        if let Some(ref key) = self.api_key {
            keys.push(key.as_str());
        }
        for entry in &self.api_keys {
            keys.push(entry.api_key.as_str());
        }
        keys
    }
}

fn default_port() -> u16 {
    8018
}
fn default_host() -> String {
    "127.0.0.1".to_string()
}

/// `AmpCode` 管理代理配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AmpConfig {
    /// 设置后，byokey 进入"共享代理"模式：
    /// 客户端的 Authorization / X-Api-Key 头会被剥离，
    /// 改为注入此 key（同时设置 `Authorization: Bearer {key}` 和 `X-Api-Key: {key}`）。
    /// 不设置则保持 BYOK 透传行为（默认）。
    #[serde(default)]
    pub upstream_key: Option<String>,
    /// 拦截 `getUserFreeTierStatus` 响应，将 `canUseAmpFree` 和
    /// `isDailyGrantEnabled` 改为 `false`，隐藏免费层提示（默认关闭）。
    #[serde(default)]
    pub hide_free_tier: bool,
}

/// A single model alias mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAlias {
    /// Original model name to map from.
    pub name: String,
    /// Alias name to expose.
    pub alias: String,
    /// If true, expose both the original name and the alias.
    #[serde(default)]
    pub fork: bool,
}

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Listen port (defaults to 8018).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Listen address (defaults to `127.0.0.1`).
    #[serde(default = "default_host")]
    pub host: String,
    /// Provider configuration map.
    #[serde(default)]
    pub providers: HashMap<ProviderId, ProviderConfig>,
    /// `AmpCode` 管理代理配置。
    #[serde(default)]
    pub amp: AmpConfig,
    /// Global upstream proxy URL (e.g. "socks5://user:pass@host:port").
    /// All upstream requests will go through this proxy.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Model alias mappings per provider.
    #[serde(default)]
    pub model_alias: HashMap<ProviderId, Vec<ModelAlias>>,
    /// Models to exclude from the /v1/models listing, per provider.
    /// Supports glob patterns (e.g. "claude-3-*", "*-thinking").
    #[serde(default)]
    pub excluded_models: HashMap<ProviderId, Vec<String>>,
    /// Streaming SSE configuration.
    #[serde(default)]
    pub streaming: StreamingConfig,
    /// TLS configuration for HTTPS serving.
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    /// Payload rules for modifying request bodies.
    #[serde(default)]
    pub payload: PayloadRules,
}

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

/// TLS configuration for serving HTTPS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Enable TLS (defaults to false if the section is present but this is omitted).
    #[serde(default)]
    pub enable: bool,
    /// Path to the PEM-encoded certificate file.
    pub cert: String,
    /// Path to the PEM-encoded private key file.
    pub key: String,
}

fn default_keepalive_seconds() -> u64 {
    15
}
fn default_bootstrap_retries() -> u32 {
    1
}
fn default_nonstream_keepalive_interval() -> u64 {
    30
}

/// Streaming SSE configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingConfig {
    /// SSE keepalive interval in seconds (sends empty comment lines).
    #[serde(default = "default_keepalive_seconds")]
    pub keepalive_seconds: u64,
    /// Number of retries before the first byte arrives.
    #[serde(default = "default_bootstrap_retries")]
    pub bootstrap_retries: u32,
    /// Non-streaming request keepalive interval in seconds.
    #[serde(default = "default_nonstream_keepalive_interval")]
    pub nonstream_keepalive_interval: u64,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            keepalive_seconds: default_keepalive_seconds(),
            bootstrap_retries: default_bootstrap_retries(),
            nonstream_keepalive_interval: default_nonstream_keepalive_interval(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            providers: HashMap::new(),
            amp: AmpConfig::default(),
            proxy_url: None,
            model_alias: HashMap::new(),
            excluded_models: HashMap::new(),
            streaming: StreamingConfig::default(),
            tls: None,
            payload: PayloadRules::default(),
        }
    }
}

/// Simple glob matching with `*` wildcard support.
fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        text.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        text.starts_with(prefix)
    } else {
        pattern == text
    }
}

impl Config {
    /// Parses configuration from a YAML string, merged with defaults.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the YAML is invalid or extraction fails.
    #[allow(clippy::result_large_err)]
    pub fn from_yaml(yaml: &str) -> Result<Self, figment::Error> {
        use figment::{
            Figment,
            providers::{Format as _, Serialized, Yaml},
        };
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Yaml::string(yaml))
            .extract()
    }

    /// Loads configuration from a file path, merged with defaults.
    ///
    /// The file format is determined by the file extension:
    /// `.json` uses JSON, everything else uses YAML.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the file cannot be read or parsed.
    #[allow(clippy::result_large_err)]
    pub fn from_file(path: &std::path::Path) -> Result<Self, figment::Error> {
        use figment::{
            Figment,
            providers::{Format as _, Json, Serialized, Yaml},
        };
        let base = Figment::from(Serialized::defaults(Config::default()));
        let figment = if path.extension().is_some_and(|e| e == "json") {
            base.merge(Json::file(path))
        } else {
            base.merge(Yaml::file(path))
        };
        figment.extract()
    }

    /// Resolves a model alias back to the original model name.
    /// If the input is not an alias, returns it unchanged.
    #[must_use]
    pub fn resolve_alias(&self, model: &str) -> String {
        for aliases in self.model_alias.values() {
            for alias in aliases {
                if alias.alias == model {
                    return alias.name.clone();
                }
            }
        }
        model.to_string()
    }

    /// Returns true if the model matches any excluded pattern for its provider.
    #[must_use]
    pub fn is_model_excluded(&self, provider: &ProviderId, model: &str) -> bool {
        if let Some(patterns) = self.excluded_models.get(provider) {
            for pattern in patterns {
                if glob_match(pattern, model) {
                    return true;
                }
            }
        }
        false
    }

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

    const SAMPLE_YAML: &str = r#"
port: 9000
host: "0.0.0.0"
providers:
  claude:
    api_key: "sk-ant-test"
    enabled: true
  codex:
    enabled: false
"#;

    #[test]
    fn test_default_config() {
        let c = Config::default();
        assert_eq!(c.port, 8018);
        assert_eq!(c.host, "127.0.0.1");
        assert!(c.providers.is_empty());
    }

    #[test]
    fn test_from_yaml_port_and_host() {
        let c = Config::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(c.port, 9000);
        assert_eq!(c.host, "0.0.0.0");
    }

    #[test]
    fn test_from_yaml_provider_api_key() {
        let c = Config::from_yaml(SAMPLE_YAML).unwrap();
        let claude = c.providers.get(&ProviderId::Claude).unwrap();
        assert_eq!(claude.api_key.as_deref(), Some("sk-ant-test"));
        assert!(claude.enabled);
    }

    #[test]
    fn test_from_yaml_provider_disabled() {
        let c = Config::from_yaml(SAMPLE_YAML).unwrap();
        let codex = c.providers.get(&ProviderId::Codex).unwrap();
        assert!(!codex.enabled);
        assert!(codex.api_key.is_none());
    }

    #[test]
    fn test_from_yaml_defaults_applied() {
        let c = Config::from_yaml("port: 1234").unwrap();
        assert_eq!(c.port, 1234);
        assert_eq!(c.host, "127.0.0.1"); // default preserved
    }

    #[test]
    fn test_provider_config_default_enabled() {
        let pc = ProviderConfig::default();
        assert!(pc.enabled);
        assert!(pc.api_key.is_none());
    }

    #[test]
    fn test_default_amp_upstream_key_is_none() {
        let c = Config::default();
        assert!(c.amp.upstream_key.is_none());
    }

    #[test]
    fn test_from_yaml_amp_upstream_key() {
        let yaml = r#"
amp:
  upstream_key: "amp-key-xxx"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.amp.upstream_key.as_deref(), Some("amp-key-xxx"));
    }

    #[test]
    fn test_from_yaml_amp_defaults_when_omitted() {
        let c = Config::from_yaml("port: 1234").unwrap();
        assert!(c.amp.upstream_key.is_none());
    }

    #[test]
    fn test_provider_config_backend_fallback_default_none() {
        let pc = ProviderConfig::default();
        assert!(pc.backend.is_none());
        assert!(pc.fallback.is_none());
    }

    #[test]
    fn test_from_yaml_backend_copilot() {
        let yaml = r"
providers:
  gemini:
    backend: copilot
";
        let c = Config::from_yaml(yaml).unwrap();
        let gemini = c.providers.get(&ProviderId::Gemini).unwrap();
        assert_eq!(gemini.backend, Some(ProviderId::Copilot));
        assert!(gemini.fallback.is_none());
    }

    #[test]
    fn test_from_yaml_fallback_copilot() {
        let yaml = r"
providers:
  gemini:
    fallback: copilot
";
        let c = Config::from_yaml(yaml).unwrap();
        let gemini = c.providers.get(&ProviderId::Gemini).unwrap();
        assert!(gemini.backend.is_none());
        assert_eq!(gemini.fallback, Some(ProviderId::Copilot));
    }

    #[test]
    fn test_from_yaml_api_keys() {
        let yaml = r#"
providers:
  claude:
    api_keys:
      - api_key: "sk-key1"
        label: "team-a"
      - api_key: "sk-key2"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let claude = c.providers.get(&ProviderId::Claude).unwrap();
        assert_eq!(claude.api_keys.len(), 2);
        assert_eq!(claude.api_keys[0].api_key, "sk-key1");
        assert_eq!(claude.api_keys[0].label.as_deref(), Some("team-a"));
        assert_eq!(claude.api_keys[1].api_key, "sk-key2");
        assert!(claude.api_keys[1].label.is_none());
    }

    #[test]
    fn test_all_api_keys_merges() {
        let pc = ProviderConfig {
            api_key: Some("single-key".into()),
            api_keys: vec![
                ApiKeyEntry {
                    api_key: "multi-1".into(),
                    label: None,
                },
                ApiKeyEntry {
                    api_key: "multi-2".into(),
                    label: None,
                },
            ],
            ..Default::default()
        };
        let keys = pc.all_api_keys();
        assert_eq!(keys, vec!["single-key", "multi-1", "multi-2"]);
    }

    #[test]
    fn test_all_api_keys_empty() {
        let pc = ProviderConfig::default();
        assert!(pc.all_api_keys().is_empty());
    }

    #[test]
    fn test_from_yaml_model_alias() {
        let yaml = r#"
model_alias:
  claude:
    - name: "claude-sonnet-4-5-20250929"
      alias: "cs4.5"
      fork: true
    - name: "claude-opus-4-5"
      alias: "co4.5"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let aliases = c.model_alias.get(&ProviderId::Claude).unwrap();
        assert_eq!(aliases.len(), 2);
        assert_eq!(aliases[0].alias, "cs4.5");
        assert!(aliases[0].fork);
        assert_eq!(aliases[1].alias, "co4.5");
        assert!(!aliases[1].fork);
    }

    #[test]
    fn test_from_yaml_excluded_models() {
        let yaml = r#"
excluded_models:
  claude:
    - "claude-3-*"
    - "*-thinking"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let excluded = c.excluded_models.get(&ProviderId::Claude).unwrap();
        assert_eq!(excluded.len(), 2);
    }

    #[test]
    fn test_resolve_alias() {
        let yaml = r#"
model_alias:
  claude:
    - name: "claude-sonnet-4-5-20250929"
      alias: "cs4.5"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.resolve_alias("cs4.5"), "claude-sonnet-4-5-20250929");
        assert_eq!(c.resolve_alias("unknown"), "unknown");
    }

    #[test]
    fn test_is_model_excluded() {
        let yaml = r#"
excluded_models:
  claude:
    - "claude-3-*"
    - "*-thinking"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert!(c.is_model_excluded(&ProviderId::Claude, "claude-3-opus"));
        assert!(c.is_model_excluded(&ProviderId::Claude, "anything-thinking"));
        assert!(!c.is_model_excluded(&ProviderId::Claude, "claude-opus-4-5"));
        assert!(!c.is_model_excluded(&ProviderId::Gemini, "claude-3-opus"));
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("claude-3-opus", "claude-3-opus"));
        assert!(!glob_match("claude-3-opus", "claude-3-sonnet"));
    }

    #[test]
    fn test_glob_match_star_prefix() {
        assert!(glob_match("*-thinking", "claude-thinking"));
        assert!(glob_match("*-thinking", "model-thinking"));
        assert!(!glob_match("*-thinking", "thinking-model"));
    }

    #[test]
    fn test_glob_match_star_suffix() {
        assert!(glob_match("claude-3-*", "claude-3-opus"));
        assert!(glob_match("claude-3-*", "claude-3-"));
        assert!(!glob_match("claude-3-*", "claude-4-opus"));
    }

    #[test]
    fn test_glob_match_star_only() {
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn test_default_config_alias_and_excluded_empty() {
        let c = Config::default();
        assert!(c.model_alias.is_empty());
        assert!(c.excluded_models.is_empty());
    }

    #[test]
    fn test_default_streaming_config() {
        let c = Config::default();
        assert_eq!(c.streaming.keepalive_seconds, 15);
        assert_eq!(c.streaming.bootstrap_retries, 1);
        assert_eq!(c.streaming.nonstream_keepalive_interval, 30);
    }

    #[test]
    fn test_from_yaml_streaming_config() {
        let yaml = r"
streaming:
  keepalive_seconds: 30
  bootstrap_retries: 3
  nonstream_keepalive_interval: 60
";
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.streaming.keepalive_seconds, 30);
        assert_eq!(c.streaming.bootstrap_retries, 3);
        assert_eq!(c.streaming.nonstream_keepalive_interval, 60);
    }

    #[test]
    fn test_from_yaml_streaming_partial() {
        let yaml = r"
streaming:
  keepalive_seconds: 20
";
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.streaming.keepalive_seconds, 20);
        assert_eq!(c.streaming.bootstrap_retries, 1);
        assert_eq!(c.streaming.nonstream_keepalive_interval, 30);
    }

    #[test]
    fn test_default_proxy_url_is_none() {
        let c = Config::default();
        assert!(c.proxy_url.is_none());
    }

    #[test]
    fn test_from_yaml_proxy_url() {
        let yaml = r#"
proxy_url: "socks5://user:pass@host:1080"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.proxy_url.as_deref(), Some("socks5://user:pass@host:1080"));
    }

    #[test]
    fn test_default_tls_is_none() {
        let c = Config::default();
        assert!(c.tls.is_none());
    }

    #[test]
    fn test_from_yaml_tls_config() {
        let yaml = r#"
tls:
  enable: true
  cert: "/path/to/cert.pem"
  key: "/path/to/key.pem"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let tls = c.tls.unwrap();
        assert!(tls.enable);
        assert_eq!(tls.cert, "/path/to/cert.pem");
        assert_eq!(tls.key, "/path/to/key.pem");
    }

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
