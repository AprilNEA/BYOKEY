use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

const BASE_URL: &str = "ANTHROPIC_BASE_URL";
const API_KEY: &str = "ANTHROPIC_API_KEY";
const DISABLE_EXPERIMENTAL_BETAS: &str = "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS";
const COPILOT_MODEL_OVERRIDES: [(&str, &str); 3] = [
    ("claude-opus-5-5", "claude-opus-5.5"),
    ("claude-fable-5-1", "claude-fable-5.1"),
    ("claude-haiku-4-5-20251001", "claude-haiku-4.5"),
];

/// Settings merged into Claude Code by `byokey claude-code inject`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCodeConfig {
    /// Claude Code settings. `env` and `modelOverrides` are merged key by key.
    #[serde(default)]
    pub settings: HashMap<String, Value>,
}

impl ClaudeCodeConfig {
    /// Resolve Claude Code's base URL without appending an API path.
    ///
    /// Priority: CLI `--url` > `claude_code.settings.env.ANTHROPIC_BASE_URL`
    /// > `http://{host}:{port}`. Wildcard listen addresses become loopback addresses.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid settings `env`, a non-string configured
    /// base URL, or a URL without an HTTP(S) scheme and host.
    pub fn resolve_url(
        &self,
        explicit: Option<&str>,
        host: &str,
        port: u16,
    ) -> anyhow::Result<String> {
        let configured = self.extra_env()?.and_then(|env| env.get(BASE_URL));
        let url = if let Some(url) = explicit {
            url.to_owned()
        } else if let Some(value) = configured {
            value
                .as_str()
                .context("claude_code.settings.env.ANTHROPIC_BASE_URL must be a string")?
                .to_owned()
        } else {
            let host = host
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or(host);
            let host = match host.parse::<IpAddr>() {
                Ok(IpAddr::V4(address)) if address.is_unspecified() => "127.0.0.1".to_owned(),
                Ok(IpAddr::V6(address)) if address.is_unspecified() => "[::1]".to_owned(),
                Ok(IpAddr::V6(address)) => format!("[{address}]"),
                _ => host.to_owned(),
            };
            format!("http://{host}:{port}")
        };
        validate_url(&url)?;
        Ok(url)
    }

    /// Default settings path: `$CLAUDE_CONFIG_DIR/settings.json`, or
    /// `~/.claude/settings.json` when no configuration directory is specified.
    #[must_use]
    pub fn default_settings_path() -> Option<PathBuf> {
        settings_path_from_dirs(
            std::env::var_os("CLAUDE_CONFIG_DIR"),
            std::env::var_os("HOME"),
            std::env::var_os("USERPROFILE"),
        )
    }

    /// Merge settings while preserving unrelated entries and model selection.
    ///
    /// Always sets the base URL and a placeholder API key. When
    /// `disable_experimental_betas` is true, also disables experimental beta
    /// features for upstream compatibility. Otherwise that setting is preserved.
    /// With `copilot_backend`, adds `modelOverrides` defaults for Copilot model
    /// IDs, preserving existing overrides. Explicit overrides in `settings` take
    /// precedence over both existing entries and defaults. Model selection is
    /// unchanged unless explicitly present in `settings`.
    /// See <https://code.claude.com/docs/en/model-config#override-model-ids-per-version>.
    ///
    /// Writes atomically, preserving existing file permissions and following
    /// existing symlinks. Returns the number of additional settings merged,
    /// counting each environment variable and model override separately.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL, source file, or extra settings are invalid,
    /// or if the file cannot be read or written. Invalid input is never replaced.
    pub fn inject(
        &self,
        resolved_url: &str,
        settings_path: &Path,
        disable_experimental_betas: bool,
        copilot_backend: bool,
    ) -> anyhow::Result<usize> {
        let (path, content, extras) = self.prepare_injection(
            resolved_url,
            settings_path,
            disable_experimental_betas,
            copilot_backend,
        )?;
        let parent = path.parent().filter(|path| !path.as_os_str().is_empty());
        let parent = parent.unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        match std::fs::metadata(&path) {
            Ok(metadata) => temporary
                .as_file()
                .set_permissions(metadata.permissions())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("cannot read Claude Code settings permissions");
            }
        }
        temporary.write_all(&content)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&path)
            .with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(extras)
    }

    /// Validate an injection without creating or changing files.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL, existing settings, or extra settings are
    /// invalid, or if the existing settings cannot be read.
    pub fn validate_injection(
        &self,
        resolved_url: &str,
        settings_path: &Path,
        disable_experimental_betas: bool,
        copilot_backend: bool,
    ) -> anyhow::Result<()> {
        self.prepare_injection(
            resolved_url,
            settings_path,
            disable_experimental_betas,
            copilot_backend,
        )?;
        Ok(())
    }

    fn prepare_injection(
        &self,
        resolved_url: &str,
        settings_path: &Path,
        disable_experimental_betas: bool,
        copilot_backend: bool,
    ) -> anyhow::Result<(PathBuf, Vec<u8>, usize)> {
        validate_url(resolved_url)?;
        let extra_env = self.extra_env()?;
        if let Some(env) = extra_env {
            for (key, value) in env {
                if !value.is_string() {
                    bail!("claude_code.settings.env.{key} must be a string");
                }
            }
        }

        let path = match std::fs::symlink_metadata(settings_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => settings_path
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", settings_path.display()))?,
            Ok(_) => settings_path.to_owned(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => settings_path.to_owned(),
            Err(error) => return Err(error).context("cannot inspect Claude Code settings"),
        };
        let mut settings = match std::fs::read(&path) {
            Ok(content) => {
                let value: Value = serde_json::from_slice(&content)
                    .with_context(|| format!("invalid JSON in {}", path.display()))?;
                match value {
                    Value::Object(settings) => settings,
                    _ => bail!("Claude Code settings must contain a JSON object"),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(error) => return Err(error).context("cannot read Claude Code settings"),
        };
        let mut env = match settings.remove("env") {
            Some(Value::Object(env)) => env,
            Some(_) => bail!("Claude Code settings.env must be a JSON object"),
            None => Map::new(),
        };

        let mut extras = self.merge_model_overrides(&mut settings, copilot_backend)?;
        for (key, value) in &self.settings {
            if key != "env" && key != "modelOverrides" {
                settings.insert(key.clone(), value.clone());
                extras += 1;
            }
        }
        if let Some(extra_env) = extra_env {
            for (key, value) in extra_env {
                env.insert(key.clone(), value.clone());
                if key != BASE_URL
                    && key != API_KEY
                    && !(disable_experimental_betas && key == DISABLE_EXPERIMENTAL_BETAS)
                {
                    extras += 1;
                }
            }
        }
        env.insert(BASE_URL.to_owned(), Value::String(resolved_url.to_owned()));
        env.insert(API_KEY.to_owned(), Value::String("byokey-local".to_owned()));
        if disable_experimental_betas {
            env.insert(
                DISABLE_EXPERIMENTAL_BETAS.to_owned(),
                Value::String("1".to_owned()),
            );
        }
        settings.insert("env".to_owned(), Value::Object(env));
        let mut content = serde_json::to_vec_pretty(&settings)?;
        content.push(b'\n');

        Ok((path, content, extras))
    }

    fn merge_model_overrides(
        &self,
        settings: &mut Map<String, Value>,
        copilot_backend: bool,
    ) -> anyhow::Result<usize> {
        let existing = model_overrides_object(
            settings.get("modelOverrides"),
            "Claude Code settings.modelOverrides",
        )?;
        let extra = model_overrides_object(
            self.settings.get("modelOverrides"),
            "claude_code.settings.modelOverrides",
        )?;
        if existing.is_none() && extra.is_none() && !copilot_backend {
            return Ok(0);
        }
        let mut overrides = Map::new();
        if copilot_backend {
            for (model, target) in COPILOT_MODEL_OVERRIDES {
                overrides.insert(model.to_owned(), Value::String(target.to_owned()));
            }
        }
        if let Some(existing) = existing {
            overrides.extend(existing.clone());
        }
        let count = extra.map_or(0, Map::len);
        if let Some(extra) = extra {
            overrides.extend(extra.clone());
        }
        settings.insert("modelOverrides".to_owned(), Value::Object(overrides));
        Ok(count)
    }

    fn extra_env(&self) -> anyhow::Result<Option<&Map<String, Value>>> {
        match self.settings.get("env") {
            Some(Value::Object(env)) => Ok(Some(env)),
            Some(_) => bail!("claude_code.settings.env must be a JSON object"),
            None => Ok(None),
        }
    }
}

fn model_overrides_object<'a>(
    value: Option<&'a Value>,
    label: &str,
) -> anyhow::Result<Option<&'a Map<String, Value>>> {
    let overrides = match value {
        Some(Value::Object(overrides)) => overrides,
        Some(_) => bail!("{label} must be a JSON object"),
        None => return Ok(None),
    };
    for (model, target) in overrides {
        if !target.is_string() {
            bail!("{label}.{model} must be a string");
        }
    }
    Ok(Some(overrides))
}

fn validate_url(url: &str) -> anyhow::Result<()> {
    let uri: http::Uri = url.parse().context("invalid Claude Code base URL")?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri.host().is_none_or(str::is_empty)
        || uri.query().is_some()
        || uri
            .authority()
            .is_some_and(|authority| authority.as_str().contains('@'))
        || url.contains('#')
    {
        bail!(
            "Claude Code base URL must be an HTTP(S) URL with a host and no credentials, query string, or fragment"
        );
    }
    Ok(())
}

fn settings_path_from_dirs(
    config_dir: Option<OsString>,
    home: Option<OsString>,
    userprofile: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(directory) = config_dir.filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(directory).join("settings.json"));
    }
    home.filter(|value| !value.is_empty())
        .or_else(|| userprofile.filter(|value| !value.is_empty()))
        .map(|directory| {
            PathBuf::from(directory)
                .join(".claude")
                .join("settings.json")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use serde_json::json;

    fn config(settings: Value) -> ClaudeCodeConfig {
        ClaudeCodeConfig {
            settings: serde_json::from_value(settings).unwrap(),
        }
    }

    fn read_settings(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn config_defaults_and_json_yaml_deserialization() {
        assert!(Config::default().claude_code.settings.is_empty());
        assert!(
            serde_json::from_str::<Config>("{}")
                .unwrap()
                .claude_code
                .settings
                .is_empty()
        );
        assert!(
            Config::from_yaml("port: 9000")
                .unwrap()
                .claude_code
                .settings
                .is_empty()
        );
        let settings =
            json!({"env": {"ANTHROPIC_BASE_URL": "https://gateway.example"}, "model": "sonnet"});
        let json_config: Config =
            serde_json::from_value(json!({"claude_code": {"settings": settings}})).unwrap();
        let yaml_config = Config::from_yaml(
            "claude_code:\n  settings:\n    env:\n      ANTHROPIC_BASE_URL: https://gateway.example\n    model: sonnet\n",
        )
        .unwrap();
        assert_eq!(
            json_config.claude_code.settings,
            yaml_config.claude_code.settings
        );
    }

    #[test]
    fn url_precedence_and_no_api_suffix() {
        let configured =
            config(json!({"env": {"ANTHROPIC_BASE_URL": "https://gateway.example/proxy"}}));
        assert_eq!(
            configured
                .resolve_url(Some("http://override:9000"), "localhost", 8018)
                .unwrap(),
            "http://override:9000"
        );
        assert_eq!(
            configured.resolve_url(None, "localhost", 8018).unwrap(),
            "https://gateway.example/proxy"
        );
        assert_eq!(
            ClaudeCodeConfig::default()
                .resolve_url(None, "localhost", 9000)
                .unwrap(),
            "http://localhost:9000"
        );
    }

    #[test]
    fn urls_use_connectable_wildcard_addresses_and_bracket_ipv6() {
        let cfg = ClaudeCodeConfig::default();
        for (host, expected) in [
            ("0.0.0.0", "http://127.0.0.1:8018"),
            ("::", "http://[::1]:8018"),
            ("[::]", "http://[::1]:8018"),
            ("::1", "http://[::1]:8018"),
            ("[2001:db8::1]", "http://[2001:db8::1]:8018"),
        ] {
            assert_eq!(cfg.resolve_url(None, host, 8018).unwrap(), expected);
        }
    }

    #[test]
    fn invalid_urls_are_rejected() {
        let cfg = ClaudeCodeConfig::default();
        for url in [
            "",
            "localhost:8018",
            "ftp://localhost",
            "http:///",
            "http://local host",
            "http://localhost?key=x",
            "http://localhost/#fragment",
            "http://user:pass@localhost",
        ] {
            assert!(
                cfg.resolve_url(Some(url), "localhost", 8018).is_err(),
                "{url}"
            );
        }
        assert!(
            config(json!({"env": {"ANTHROPIC_BASE_URL": 42}}))
                .resolve_url(None, "localhost", 8018)
                .is_err()
        );
    }

    #[test]
    fn settings_path_honors_config_dir_then_home_then_userprofile() {
        assert_eq!(
            settings_path_from_dirs(
                Some("custom".into()),
                Some("home".into()),
                Some("profile".into())
            ),
            Some(PathBuf::from("custom/settings.json"))
        );
        assert_eq!(
            settings_path_from_dirs(None, Some("home".into()), Some("profile".into())),
            Some(PathBuf::from("home/.claude/settings.json"))
        );
        assert_eq!(
            settings_path_from_dirs(Some("".into()), Some("".into()), Some("profile".into())),
            Some(PathBuf::from("profile/.claude/settings.json"))
        );
        assert_eq!(settings_path_from_dirs(None, None, None), None);
    }

    #[test]
    fn injection_merges_env_and_preserves_model_and_unrelated_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, serde_json::to_vec(&json!({
            "model": "my-model",
            "permissions": {"allow": ["Read"]},
            "env": {"KEEP": "yes", "ANTHROPIC_MODEL": "my-model", "ANTHROPIC_API_KEY": "old", "UPDATE": "old"}
        })).unwrap()).unwrap();
        let cfg = config(json!({
            "effortLevel": "high",
            "env": {"UPDATE": "new", "ANTHROPIC_BASE_URL": "http://unused", "ANTHROPIC_API_KEY": "ignored"}
        }));
        assert_eq!(
            cfg.inject("http://127.0.0.1:8018", &path, true, false)
                .unwrap(),
            2
        );
        let actual = read_settings(&path);
        assert_eq!(
            actual,
            json!({
                "model": "my-model",
                "permissions": {"allow": ["Read"]},
                "effortLevel": "high",
                "env": {"KEEP": "yes", "UPDATE": "new", "ANTHROPIC_MODEL": "my-model",
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8018", "ANTHROPIC_API_KEY": "byokey-local",
                    "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS": "1"}
            })
        );
    }

    #[test]
    fn injection_creates_minimal_settings_and_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claude/settings.json");
        let cfg = ClaudeCodeConfig::default();
        assert_eq!(
            cfg.inject("http://localhost:8018", &path, false, false)
                .unwrap(),
            0
        );
        assert_eq!(
            read_settings(&path),
            json!({"env": {
                "ANTHROPIC_BASE_URL": "http://localhost:8018", "ANTHROPIC_API_KEY": "byokey-local"
            }})
        );
        let before = std::fs::read(&path).unwrap();
        cfg.inject("http://localhost:8018", &path, false, false)
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn copilot_model_overrides_are_added_and_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let cfg = ClaudeCodeConfig::default();
        assert_eq!(
            cfg.inject("http://localhost:8018", &path, true, true)
                .unwrap(),
            0
        );
        assert_eq!(
            read_settings(&path)["modelOverrides"],
            json!({
                "claude-opus-5-5": "claude-opus-5.5",
                "claude-fable-5-1": "claude-fable-5.1",
                "claude-haiku-4-5-20251001": "claude-haiku-4.5"
            })
        );
        assert!(read_settings(&path).get("model").is_none());
        let before = std::fs::read(&path).unwrap();
        cfg.inject("http://localhost:8018", &path, true, true)
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn model_overrides_merge_with_explicit_then_existing_then_default_precedence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let original = json!({
            "model": "claude-fable-5-1[1m]",
            "permissions": {"allow": ["Read"]},
            "env": {"KEEP": "yes"},
            "modelOverrides": {
                "claude-opus-5-5": "existing-opus",
                "claude-fable-5-1": "existing-fable",
                "custom": "existing-custom"
            }
        });
        let cfg = config(json!({"modelOverrides": {
            "claude-opus-5-5": "explicit-opus", "extra": "explicit-extra"
        }}));
        for copilot_backend in [false, true] {
            std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
            assert_eq!(
                cfg.inject("http://localhost:8018", &path, false, copilot_backend)
                    .unwrap(),
                2
            );
            let actual = read_settings(&path);
            let mut expected = json!({
                "claude-opus-5-5": "explicit-opus",
                "claude-fable-5-1": "existing-fable",
                "custom": "existing-custom",
                "extra": "explicit-extra"
            });
            if copilot_backend {
                expected["claude-haiku-4-5-20251001"] = json!("claude-haiku-4.5");
            }
            assert_eq!(actual["modelOverrides"], expected);
            assert_eq!(actual["model"], original["model"]);
            assert_eq!(actual["permissions"], original["permissions"]);
            assert_eq!(actual["env"]["KEEP"], "yes");
        }
    }

    #[test]
    fn generic_injection_does_not_add_model_overrides_even_with_beta_flag() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for disable_experimental_betas in [false, true] {
            ClaudeCodeConfig::default()
                .inject(
                    "http://localhost:8018",
                    &path,
                    disable_experimental_betas,
                    false,
                )
                .unwrap();
            assert!(read_settings(&path).get("modelOverrides").is_none());
        }
    }

    #[test]
    fn invalid_model_overrides_fail_preflight_without_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for invalid in [
            json!(null),
            json!([]),
            json!("invalid"),
            json!({"opus": 42}),
        ] {
            for copilot_backend in [false, true] {
                let existing = json!({"model": "keep", "modelOverrides": invalid});
                let original = serde_json::to_vec(&existing).unwrap();
                std::fs::write(&path, &original).unwrap();
                // Even explicit valid overrides must not hide invalid existing data.
                let cfg = config(json!({"modelOverrides": {"opus": "valid"}}));
                assert!(
                    cfg.validate_injection("http://localhost:8018", &path, true, copilot_backend)
                        .is_err()
                );
                assert!(
                    cfg.inject("http://localhost:8018", &path, true, copilot_backend)
                        .is_err()
                );
                assert_eq!(std::fs::read(&path).unwrap(), original);

                let original = b"{\"model\":\"keep\"}\n";
                std::fs::write(&path, original).unwrap();
                let cfg = config(json!({"modelOverrides": invalid}));
                assert!(
                    cfg.validate_injection("http://localhost:8018", &path, true, copilot_backend)
                        .is_err()
                );
                assert!(
                    cfg.inject("http://localhost:8018", &path, true, copilot_backend)
                        .is_err()
                );
                assert_eq!(std::fs::read(&path).unwrap(), original);
            }
        }
    }

    #[test]
    fn compatibility_flag_is_preserved_when_not_requested() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let cfg = config(json!({"env": {"CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS": "0"}}));
        assert_eq!(
            cfg.inject("http://localhost:8018", &path, false, false)
                .unwrap(),
            1
        );
        assert_eq!(read_settings(&path)["env"][DISABLE_EXPERIMENTAL_BETAS], "0");
        ClaudeCodeConfig::default()
            .inject("http://localhost:8018", &path, false, false)
            .unwrap();
        assert_eq!(read_settings(&path)["env"][DISABLE_EXPERIMENTAL_BETAS], "0");
        cfg.inject("http://localhost:8018", &path, true, false)
            .unwrap();
        assert_eq!(read_settings(&path)["env"][DISABLE_EXPERIMENTAL_BETAS], "1");
    }

    #[test]
    fn explicit_extra_model_settings_override_existing_selections() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"model":"old","env":{"ANTHROPIC_MODEL":"old"}}"#).unwrap();
        config(json!({"model": "new", "env": {"ANTHROPIC_MODEL": "new"}}))
            .inject("http://localhost:8018", &path, false, false)
            .unwrap();
        assert_eq!(read_settings(&path)["model"], "new");
        assert_eq!(read_settings(&path)["env"]["ANTHROPIC_MODEL"], "new");
    }

    #[test]
    fn invalid_existing_files_are_preserved_byte_for_byte() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for original in [
            "invalid JSON\n",
            "[]\n",
            "null\n",
            r#"{"env":"invalid","model":"keep"}"#,
            r#"{"env":null}"#,
        ] {
            std::fs::write(&path, original).unwrap();
            assert!(
                ClaudeCodeConfig::default()
                    .inject("http://localhost:8018", &path, true, false)
                    .is_err()
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[test]
    fn validation_does_not_create_settings_or_parent_directories() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claude/settings.json");
        ClaudeCodeConfig::default()
            .validate_injection("http://localhost:8018", &path, true, false)
            .unwrap();
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn invalid_extras_do_not_modify_existing_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let original = "{\"model\":\"keep\"}\n";
        std::fs::write(&path, original).unwrap();
        for settings in [
            json!({"env": null}),
            json!({"env": []}),
            json!({"env": {"BAD": 1}}),
        ] {
            assert!(
                config(settings)
                    .inject("http://localhost:8018", &path, false, false)
                    .is_err()
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[cfg(unix)]
    #[test]
    fn injection_preserves_permissions_and_symlink_target() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.json");
        let path = directory.path().join("settings.json");
        std::fs::write(&target, "{}").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &path).unwrap();
        ClaudeCodeConfig::default()
            .inject("http://localhost:8018", &path, true, false)
            .unwrap();
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            read_settings(&target)["env"][BASE_URL],
            "http://localhost:8018"
        );
    }
}
