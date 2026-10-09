//! `byokey route`: which provider serves each Anthropic model.
//!
//! The routes live in the config file's `anthropic.routes` section, which the running
//! server reloads on change. Listing asks the server, which knows what each
//! signed-in provider offers.

use anyhow::{Context as _, Result, bail};
use byokey_auth::AuthManager;
use byokey_config::{Config, Routes};
use byokey_proto::byokey::routes::RouteSource;
use byokey_proto::client::ManagementClient;
use byokey_types::{ClaudeFamily, ClaudeModel, ProviderId};
use clap::{Args, Subcommand};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Subcommand, Debug)]
pub enum RouteAction {
    /// Serve a model, a family or every other model from a provider.
    Set {
        #[command(flatten)]
        target: Target,
        /// `claude`, `copilot`, `cursor`, or a configured Messages provider.
        provider: String,
    },
    /// Remove a route; its models fall back to the next broader one.
    Unset {
        #[command(flatten)]
        target: Target,
    },
}

/// What a route applies to. A model's own route beats its family's, which
/// beats the default.
#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
pub struct Target {
    /// One model, by Anthropic's id (e.g. `claude-opus-5-5`).
    #[arg(long, value_name = "MODEL")]
    model: Option<ClaudeModel>,
    /// A model family: `fable` (or `mythos`), `opus`, `sonnet` or `haiku`.
    #[arg(long, value_name = "FAMILY")]
    family: Option<ClaudeFamily>,
    /// Every model without a model or family route.
    #[arg(long)]
    default: bool,
}

/// The one route a [`Target`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteKey {
    Model(ClaudeModel),
    Family(ClaudeFamily),
    Default,
}

impl From<Target> for RouteKey {
    /// clap's group guarantees exactly one of the fields.
    fn from(t: Target) -> Self {
        match (t.model, t.family) {
            (Some(model), _) => Self::Model(model),
            (None, Some(family)) => Self::Family(family),
            (None, None) => Self::Default,
        }
    }
}

/// Run `action`, or list the routes without one. `db` is the token store
/// that tells whether a routed provider is signed in.
pub async fn cmd_route(
    action: Option<RouteAction>,
    config: Option<PathBuf>,
    url: Option<String>,
    db: Option<PathBuf>,
) -> Result<()> {
    let path = match config {
        Some(path) => path,
        None => byokey_daemon::paths::config_path()?,
    };
    let (key, provider) = match action {
        None => return list(&path, url).await,
        Some(RouteAction::Set { target, provider }) => (target.into(), Some(provider)),
        Some(RouteAction::Unset { target }) => (target.into(), None),
    };
    let settings = read(&path)?;
    let mut config: Config = serde_json::from_value(Value::Object(settings.clone()))
        .with_context(|| format!("invalid config in {}", path.display()))?;
    config.validate()?;
    apply(&mut config.anthropic.routes, key, provider.clone());
    config.validate()?;
    save(&path, settings, &config.anthropic.routes)?;
    if let Some(provider) = provider {
        let config = load(&path)?;
        let store = Arc::new(crate::open_store(db).await?);
        let auth = AuthManager::new(store, reqwest::Client::new());
        if let Some(reason) = unusable(&auth, &config, &provider).await {
            eprintln!("warning: {reason}; requests on this route will fail until then");
        }
    }
    Ok(())
}

/// Why `provider` cannot serve requests, if it cannot: disabled, or neither
/// signed in nor given an API key. The server lists no model from it.
pub(crate) async fn unusable(
    auth: &AuthManager,
    config: &Config,
    provider: &str,
) -> Option<String> {
    let pc = config.providers.get(provider);
    if pc.is_some_and(|c| !c.enabled) {
        return Some(format!("{provider} is disabled in the config"));
    }
    let Ok(builtin) = provider.parse::<ProviderId>() else {
        return None; // Custom providers use configured headers, not stored logins.
    };
    if pc.is_some_and(|c| c.api_key.is_some()) || auth.is_authenticated(builtin).await {
        return None;
    }
    Some(format!(
        "{provider} is not signed in; run `byokey login {provider}`"
    ))
}

/// Route `key` to `provider`, or remove its route when `None`.
fn apply(routes: &mut Routes, key: RouteKey, provider: Option<String>) {
    match (key, provider) {
        (RouteKey::Model(m), Some(p)) => {
            routes.models.insert(m, p);
        }
        (RouteKey::Model(m), None) => {
            routes.models.remove(&m);
        }
        (RouteKey::Family(f), Some(p)) => {
            routes.families.insert(f, p);
        }
        (RouteKey::Family(f), None) => {
            routes.families.remove(&f);
        }
        (RouteKey::Default, p) => routes.default = p,
    }
}

fn load(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    Config::from_file(path).with_context(|| format!("load BYOKEY config {}", path.display()))
}

/// The JSON config file at `path`, or an empty one when there is none.
fn read(path: &Path) -> Result<Map<String, Value>> {
    if path.extension().is_none_or(|e| e != "json") {
        bail!(
            "`byokey route` edits JSON config files only; set `anthropic.routes` in {} by hand",
            path.display()
        );
    }
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid JSON in {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// Write `settings` with `routes` to `path`.
fn save(path: &Path, mut settings: Map<String, Value>, routes: &Routes) -> Result<()> {
    let anthropic = settings
        .entry("anthropic")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("Anthropic settings validated before editing");
    if *routes == Routes::default() {
        anthropic.remove("routes");
    } else {
        anthropic.insert("routes".into(), serde_json::to_value(routes)?);
    }
    if anthropic.is_empty() {
        settings.remove("anthropic");
    }
    let mut bytes = serde_json::to_vec_pretty(&settings)?;
    bytes.push(b'\n');
    super::claude::write_atomic(path, &bytes)
        .with_context(|| format!("write {}", path.display()))?;
    // Checks the file still loads, and that the server can reload it.
    load(path)?;
    println!("routes saved to {}", path.display());
    Ok(())
}

async fn list(path: &Path, url: Option<String>) -> Result<()> {
    let config = load(path)?;
    let url = url.unwrap_or_else(|| super::claude::local_url(&config.host, config.port));
    let uri = url
        .parse()
        .with_context(|| format!("invalid BYOKEY URL {url}"))?;
    let listing = ManagementClient::local_http(uri)
        .list_routes()
        .await
        .with_context(|| {
            format!("ask BYOKEY at {url} for its routes; start it with `byokey start`")
        })?;
    println!(
        "{:<20} {:<10} {:<14} OFFERED BY",
        "MODEL", "PROVIDER", "ROUTE"
    );
    for m in &listing.models {
        let source = match m.source.as_known() {
            Some(RouteSource::ROUTE_SOURCE_MODEL) => "--model".to_owned(),
            Some(RouteSource::ROUTE_SOURCE_FAMILY) => {
                let family = m.model.split('-').nth(1).unwrap_or_default();
                format!("--family {family}")
            }
            Some(RouteSource::ROUTE_SOURCE_DEFAULT) => "--default".to_owned(),
            _ => "unset".to_owned(),
        };
        let offered = if m.offered_by.is_empty() {
            "-".to_owned()
        } else {
            m.offered_by.join(" ")
        };
        let unlisted = if m.offered_by.contains(&m.provider) {
            ""
        } else {
            "   (not offered: not listed)"
        };
        println!(
            "{:<20} {:<10} {source:<14} {offered}{unlisted}",
            m.model, m.provider
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn model(id: &str) -> ClaudeModel {
        id.parse().unwrap()
    }

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        action: RouteAction,
    }

    fn parse(args: &[&str]) -> Result<RouteAction, clap::Error> {
        Cli::try_parse_from(std::iter::once("route").chain(args.iter().copied())).map(|c| c.action)
    }

    #[test]
    fn a_route_names_exactly_one_target() {
        let key = |args: &[&str]| match parse(args).unwrap() {
            RouteAction::Set { target, .. } | RouteAction::Unset { target } => {
                RouteKey::from(target)
            }
        };
        assert_eq!(
            key(&["set", "--model", "claude-opus-5.5", "cursor"]),
            RouteKey::Model(model("claude-opus-5-5"))
        );
        assert_eq!(
            key(&["set", "--family", "opus", "cursor"]),
            RouteKey::Family(ClaudeFamily::Opus)
        );
        assert_eq!(key(&["unset", "--default"]), RouteKey::Default);
        for args in [
            &["set", "claude", "copilot"][..],
            &["set", "--default", "--family", "opus", "copilot"],
            &["set", "--family", "claude-opus", "copilot"],
            &["set", "--model", "gpt-5.4", "copilot"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[tokio::test]
    async fn actions_keep_catalog_providers_and_other_protocol_routes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"port":9000,"providers":{"copilot":{"display_name":"GitHub"}},"anthropic":{"catalog":{"name_format":"{{ model }} / {{ provider }}"}},"responses":{"routes":{"default":"copilot"}}}"#,
        )
        .unwrap();
        let db = dir.path().join("tokens.db");
        let run = |args: &[&str]| {
            cmd_route(
                Some(parse(args).unwrap()),
                Some(path.clone()),
                None,
                Some(db.clone()),
            )
        };
        run(&["set", "--default", "copilot"]).await.unwrap();
        run(&["set", "--family", "opus", "cursor"]).await.unwrap();
        run(&["set", "--model", "claude-opus-5-5", "claude"])
            .await
            .unwrap();
        let config = load(&path).unwrap();
        assert_eq!(config.port, 9000);
        assert_eq!(
            config.anthropic.routes,
            Routes {
                default: Some("copilot".into()),
                families: [(ClaudeFamily::Opus, "cursor".into())].into(),
                models: [(model("claude-opus-5-5"), "claude".into())].into(),
            },
            "`claude` pins a model to Anthropic"
        );
        assert_eq!(
            config.providers["copilot"].display_name.as_deref(),
            Some("GitHub")
        );
        assert_eq!(config.responses.routes.default, "copilot");

        for args in [
            &["unset", "--model", "claude-opus-5-5"][..],
            &["unset", "--family", "opus"],
            &["unset", "--default"],
        ] {
            run(args).await.unwrap();
        }
        let settings: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            settings["anthropic"].get("routes").is_none(),
            "empty routes are removed"
        );
        assert_eq!(
            settings["anthropic"]["catalog"]["name_format"],
            "{{ model }} / {{ provider }}"
        );
        assert!(
            settings.get("routes").is_none(),
            "root routes are never written"
        );

        let yaml = dir.path().join("settings.yaml");
        let unset = parse(&["unset", "--default"]).unwrap();
        assert!(
            cmd_route(Some(unset), Some(yaml), None, None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_unknown_provider_does_not_replace_a_valid_route() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = r#"{"anthropic":{"routes":{"default":"copilot"}}}"#;
        std::fs::write(&path, original).unwrap();

        let result = cmd_route(
            Some(parse(&["set", "--default", "unconfigured-router"]).unwrap()),
            Some(path.clone()),
            None,
            None,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }

    #[tokio::test]
    async fn invalid_configuration_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = r#"{"providers":{"claude":{"backend":"copilot"}}}"#;
        std::fs::write(&path, original).unwrap();

        let result = cmd_route(
            Some(parse(&["unset", "--default"]).unwrap()),
            Some(path.clone()),
            None,
            None,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }
}
