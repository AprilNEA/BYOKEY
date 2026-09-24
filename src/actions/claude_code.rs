//! `byokey claude-code inject`: point Claude Code at BYOKEY.

use anyhow::{Context as _, Result, bail};
use byokey_config::Config;
use clap::{Args, Subcommand};
use serde_json::{Map, Value};
use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

const BASE_URL: &str = "ANTHROPIC_BASE_URL";
const AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";
/// BYOKEY ignores client credentials, but Claude Code needs one to be set.
/// `ANTHROPIC_AUTH_TOKEN` does so without the approval prompt that
/// `ANTHROPIC_API_KEY` triggers.
const PLACEHOLDER_TOKEN: &str = "byokey";

#[derive(Subcommand, Debug)]
pub enum ClaudeCodeAction {
    /// Point Claude Code at BYOKEY, keeping its other settings.
    Inject(InjectArgs),
}

#[derive(Args, Debug)]
pub struct InjectArgs {
    /// BYOKEY configuration file [default: ~/.config/byokey/settings.json].
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Claude Code settings file
    /// [default: $CLAUDE_CONFIG_DIR/settings.json or ~/.claude/settings.json].
    #[arg(long, value_name = "FILE")]
    settings: Option<PathBuf>,
    /// BYOKEY base URL, without `/v1` [default: the configured listen address].
    #[arg(long)]
    url: Option<String>,
}

pub fn cmd_claude_code(action: ClaudeCodeAction) -> Result<()> {
    match action {
        ClaudeCodeAction::Inject(args) => inject(args),
    }
}

fn inject(args: InjectArgs) -> Result<()> {
    let config = load_config(args.config)?;
    let extras = &config.claude_code.settings;
    let url = match args.url {
        Some(url) => url,
        None => match configured_url(extras)? {
            Some(url) => url.to_owned(),
            None => local_url(&config.host, config.port),
        },
    };
    validate_url(&url)?;

    let path = match args.settings {
        Some(path) => path,
        None => default_settings_path().context("cannot locate the Claude Code settings")?,
    };
    // Write through a symlink (e.g. from a dotfiles manager) to its target.
    let path = if path.is_symlink() {
        path.canonicalize()?
    } else {
        path
    };
    let settings = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid JSON in {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };

    let merged = merge(settings, extras, &url)?;
    let mut bytes = serde_json::to_vec_pretty(&merged)?;
    bytes.push(b'\n');
    write_atomic(&path, &bytes).with_context(|| format!("write {}", path.display()))?;

    println!("Claude Code now uses BYOKEY at {url}: {}", path.display());
    println!("Restart Claude Code to apply the settings.");
    Ok(())
}

fn load_config(explicit: Option<PathBuf>) -> Result<Config> {
    let path = match explicit {
        Some(path) => path,
        None => {
            let path = byokey_daemon::paths::config_path()?;
            if !path.exists() {
                return Ok(Config::default());
            }
            path
        }
    };
    Config::from_file(&path).with_context(|| format!("load BYOKEY config {}", path.display()))
}

/// Merge BYOKEY's connection settings and the configured `extras` into
/// Claude Code's `settings`, keeping everything else.
fn merge(
    mut settings: Map<String, Value>,
    extras: &Map<String, Value>,
    url: &str,
) -> Result<Map<String, Value>> {
    let extra_env = extra_env(extras)?;
    for (key, value) in extras {
        if key != "env" {
            settings.insert(key.clone(), value.clone());
        }
    }

    let Value::Object(env) = settings
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()))
    else {
        bail!("Claude Code settings `env` must be an object");
    };
    env.insert(AUTH_TOKEN.to_owned(), PLACEHOLDER_TOKEN.into());
    if let Some(extra_env) = extra_env {
        env.extend(extra_env.clone());
    }
    env.insert(BASE_URL.to_owned(), url.into());
    Ok(settings)
}

/// `claude_code.settings.env`, which Claude Code requires to hold strings.
fn extra_env(extras: &Map<String, Value>) -> Result<Option<&Map<String, Value>>> {
    let Some(env) = extras.get("env") else {
        return Ok(None);
    };
    let env = env
        .as_object()
        .context("claude_code.settings.env must be an object")?;
    if let Some((key, _)) = env.iter().find(|(_, value)| !value.is_string()) {
        bail!("claude_code.settings.env.{key} must be a string");
    }
    Ok(Some(env))
}

fn configured_url(extras: &Map<String, Value>) -> Result<Option<&str>> {
    Ok(extra_env(extras)?.and_then(|env| env.get(BASE_URL)?.as_str()))
}

/// The address a local client reaches BYOKEY's listener at: a wildcard bind
/// address becomes loopback.
fn local_url(host: &str, port: u16) -> String {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host = match host.parse::<IpAddr>() {
        Ok(ip) if ip.is_unspecified() && ip.is_ipv4() => "127.0.0.1".to_owned(),
        Ok(ip) if ip.is_unspecified() => "[::1]".to_owned(),
        Ok(IpAddr::V6(ip)) => format!("[{ip}]"),
        _ => host.to_owned(),
    };
    format!("http://{host}:{port}")
}

fn validate_url(url: &str) -> Result<()> {
    let uri: wreq::Uri = url
        .parse()
        .with_context(|| format!("invalid base URL {url}"))?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.host().is_none() {
        bail!("base URL must be an http(s) URL with a host: {url}");
    }
    Ok(())
}

fn default_settings_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    std::env::home_dir().map(|home| home.join(".claude").join("settings.json"))
}

/// Replace `path` atomically, keeping its permissions.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    if let Ok(metadata) = std::fs::metadata(path) {
        file.as_file().set_permissions(metadata.permissions())?;
    }
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => unreachable!("test fixture is an object"),
        }
    }

    #[test]
    fn merge_points_at_byokey_and_keeps_other_settings() {
        let settings = object(json!({
            "model": "claude-opus-5-5",
            "permissions": {"allow": ["Read"]},
            "env": {"KEEP": "1", "ANTHROPIC_BASE_URL": "https://old.example"}
        }));
        let merged = merge(settings, &Map::new(), "http://127.0.0.1:8018").unwrap();
        assert_eq!(
            Value::Object(merged),
            json!({
                "model": "claude-opus-5-5",
                "permissions": {"allow": ["Read"]},
                "env": {
                    "KEEP": "1",
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8018",
                    "ANTHROPIC_AUTH_TOKEN": "byokey"
                }
            })
        );
    }

    #[test]
    fn configured_extras_override_per_key_and_env_variable() {
        let settings = object(json!({"model": "old", "env": {"KEEP": "1", "X": "old"}}));
        let extras = object(json!({
            "model": "new",
            "env": {"X": "new", "ANTHROPIC_AUTH_TOKEN": "gateway-token"}
        }));
        let merged = Value::Object(merge(settings, &extras, "http://h:1").unwrap());
        assert_eq!(merged["model"], "new");
        assert_eq!(merged["env"]["KEEP"], "1");
        assert_eq!(merged["env"]["X"], "new");
        assert_eq!(merged["env"]["ANTHROPIC_AUTH_TOKEN"], "gateway-token");
    }

    #[test]
    fn configured_base_url_is_the_default_url() {
        let extras = object(json!({"env": {"ANTHROPIC_BASE_URL": "https://gw.example"}}));
        assert_eq!(configured_url(&extras).unwrap(), Some("https://gw.example"));
        assert_eq!(configured_url(&Map::new()).unwrap(), None);
    }

    #[test]
    fn malformed_env_is_rejected() {
        let settings = object(json!({"env": "not an object"}));
        assert!(merge(settings, &Map::new(), "http://h:1").is_err());
        let extras = object(json!({"env": {"X": 1}}));
        assert!(merge(Map::new(), &extras, "http://h:1").is_err());
    }

    #[test]
    fn wildcard_listen_addresses_become_loopback() {
        assert_eq!(local_url("0.0.0.0", 8018), "http://127.0.0.1:8018");
        assert_eq!(local_url("::", 8018), "http://[::1]:8018");
        assert_eq!(local_url("[::]", 8018), "http://[::1]:8018");
        assert_eq!(local_url("2001:db8::1", 8018), "http://[2001:db8::1]:8018");
        assert_eq!(local_url("localhost", 8018), "http://localhost:8018");
    }

    #[test]
    fn only_http_urls_with_a_host_are_accepted() {
        assert!(validate_url("http://127.0.0.1:8018").is_ok());
        assert!(validate_url("https://gw.example/proxy").is_ok());
        for bad in ["localhost:8018", "ftp://host", "http:///path", ""] {
            assert!(validate_url(bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn write_keeps_permissions_symlink_and_key_order() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.json");
        let link = dir.path().join("settings.json");
        std::fs::write(&target, r#"{"zeta": 1, "alpha": 2}"#).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, &link).unwrap();

        let settings: Map<String, Value> =
            serde_json::from_slice(&std::fs::read(&link).unwrap()).unwrap();
        let merged = merge(settings, &Map::new(), "http://h:1").unwrap();
        write_atomic(
            &link.canonicalize().unwrap(),
            &serde_json::to_vec(&merged).unwrap(),
        )
        .unwrap();

        assert!(link.is_symlink());
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let written = std::fs::read_to_string(&target).unwrap();
        assert!(written.find("zeta") < written.find("alpha"), "{written}");
    }
}
