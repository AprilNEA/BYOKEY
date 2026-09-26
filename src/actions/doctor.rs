//! `byokey doctor`: check that the server, its providers, Claude Code and
//! Claude Desktop are wired up, and say what to run when something is not.

use anyhow::Result;
use byokey_auth::AuthManager;
use byokey_daemon::process::ServerStatus;
use byokey_types::ProviderId;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A check's outcome.
enum Outcome {
    Ok(String),
    Warn(String),
    Fail(String),
}

impl Outcome {
    fn print(&self, name: &str) {
        let (mark, detail) = match self {
            Self::Ok(d) => ("ok  ", d),
            Self::Warn(d) => ("warn", d),
            Self::Fail(d) => ("FAIL", d),
        };
        println!("[{mark}] {name:<16} {detail}");
    }
}

/// Run every check and print one line per check. Exits non-zero when any
/// check fails.
pub async fn cmd_doctor(url: Option<String>, db: Option<PathBuf>) -> Result<()> {
    let config = super::claude::load_config(None)?;
    let url = url.unwrap_or_else(|| super::claude::local_url(&config.host, config.port));
    let http = wreq::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut failed = false;
    let mut report = |name: &str, outcome: Outcome| {
        failed |= matches!(outcome, Outcome::Fail(_));
        outcome.print(name);
    };

    report("server", server());
    report("service", service());
    let reachable = reachable(&http, &url).await;
    let up = matches!(reachable, Outcome::Ok(_));
    report("listener", reachable);
    if up {
        report("models", models(&http, &url).await);
        report("count_tokens", count_tokens(&http, &url).await);
    }

    let store = Arc::new(crate::open_store(db).await?);
    let auth = AuthManager::new(store, wreq::Client::new());
    for provider in ProviderId::all() {
        if let Some(outcome) = account(&auth, &config, provider).await {
            report(&provider.to_string(), outcome);
        }
    }
    report("claude code", claude_code(&url));
    if let Some(outcome) = claude_desktop(&url) {
        report("claude desktop", outcome);
    }

    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn server() -> Outcome {
    match byokey_daemon::process::status() {
        Ok(ServerStatus::Running { pid }) => Outcome::Ok(format!("running (pid {pid})")),
        Ok(ServerStatus::Stale { pid }) => {
            Outcome::Warn(format!("pid file for {pid} is stale; run `byokey start`"))
        }
        Ok(ServerStatus::Stopped) | Err(_) => {
            Outcome::Warn("not running under byokey's control socket".into())
        }
    }
}

fn service() -> Outcome {
    match byokey_daemon::service::status() {
        Ok(s) if s.running => Outcome::Ok(format!("{} service running", s.backend)),
        Ok(s) if s.installed => Outcome::Warn(format!(
            "{} service installed but stopped; run `byokey service start`",
            s.backend
        )),
        Ok(_) if brew_service() => Outcome::Ok("managed by `brew services`".into()),
        Ok(s) => Outcome::Ok(format!("no {} service (optional)", s.backend)),
        Err(e) => Outcome::Warn(format!("cannot query the service manager: {e}")),
    }
}

/// Whether Homebrew's `brew services` runs byokey under its own label.
fn brew_service() -> bool {
    cfg!(target_os = "macos")
        && std::env::home_dir().is_some_and(|home| {
            home.join("Library/LaunchAgents/sh.brew.byokey.plist")
                .exists()
        })
}

async fn reachable(http: &wreq::Client, url: &str) -> Outcome {
    let started = Instant::now();
    match http.get(format!("{url}/v1/models")).send().await {
        Ok(r) if r.status().is_success() => Outcome::Ok(format!(
            "{url} answers in {} ms",
            started.elapsed().as_millis()
        )),
        Ok(r) => Outcome::Fail(format!("{url} answered HTTP {}", r.status())),
        Err(e) => Outcome::Fail(format!(
            "{url} is not reachable ({}); start it with `byokey start`",
            redact(&e.to_string())
        )),
    }
}

async fn models(http: &wreq::Client, url: &str) -> Outcome {
    let body: Value = match http.get(format!("{url}/v1/models?limit=1000")).send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => return Outcome::Fail(redact(&e.to_string())),
    };
    let ids: Vec<&str> = body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str())
        .collect();
    let claude = ids
        .iter()
        .filter(|id| id.to_ascii_lowercase().contains("claude"))
        .count();
    if ids.is_empty() {
        Outcome::Fail("no models listed".into())
    } else {
        Outcome::Ok(format!("{} models, {claude} for Claude Code", ids.len()))
    }
}

/// Exact token counting through whichever backend serves Claude Code.
async fn count_tokens(http: &wreq::Client, url: &str) -> Outcome {
    let body = json!({
        "model": "claude-sonnet-5",
        "messages": [{"role": "user", "content": "ping"}],
    });
    match http
        .post(format!("{url}/v1/messages/count_tokens"))
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let n = r
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v["input_tokens"].as_u64());
            Outcome::Ok(format!(
                "exact counts available ({} tokens for a probe)",
                n.unwrap_or(0)
            ))
        }
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            Outcome::Warn(format!(
                "HTTP {status}: {}; Claude Code will estimate",
                truncate(&redact(&text), 160)
            ))
        }
        Err(e) => Outcome::Warn(redact(&e.to_string())),
    }
}

/// A provider's login state, or `None` for providers with nothing set up.
async fn account(
    auth: &AuthManager,
    config: &byokey_config::Config,
    provider: &ProviderId,
) -> Option<Outcome> {
    let pc = config.providers.get(provider);
    if pc.is_some_and(|c| !c.enabled) {
        return None;
    }
    if pc.is_some_and(|c| c.api_key.is_some()) {
        return Some(Outcome::Ok("API key in config".into()));
    }
    let accounts = auth.list_accounts(provider).await.unwrap_or_default();
    if accounts.is_empty() {
        return None;
    }
    let n = accounts.len();
    let accounts = if n == 1 {
        "1 account".to_owned()
    } else {
        format!("{n} accounts")
    };
    Some(if auth.is_authenticated(provider).await {
        Outcome::Ok(format!("{accounts}, signed in"))
    } else {
        Outcome::Fail(format!(
            "{accounts}, token expired; run `byokey login {provider}`"
        ))
    })
}

/// Whether plain `claude` already points at BYOKEY.
fn claude_code(url: &str) -> Outcome {
    let Some(path) = super::claude::default_settings_path() else {
        return Outcome::Warn("cannot locate ~/.claude/settings.json".into());
    };
    let base = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|s| s["env"]["ANTHROPIC_BASE_URL"].as_str().map(str::to_owned));
    match base {
        Some(base) if base.trim_end_matches('/') == url => {
            Outcome::Ok("`claude` uses BYOKEY (settings.json)".into())
        }
        Some(base) => Outcome::Warn(format!(
            "`claude` points at {base}, not {url}; `byokey claude start` still works"
        )),
        None => {
            Outcome::Ok("use `byokey claude start`, or `byokey claude inject` for `claude`".into())
        }
    }
}

/// Whether Claude Desktop's third-party profile points at BYOKEY. `None`
/// where Desktop is not installed, or when the profile cannot be read.
fn claude_desktop(url: &str) -> Option<Outcome> {
    use super::claude_desktop::DesktopState;
    let state = match super::claude_desktop::state() {
        Ok(DesktopState::NotInstalled) => return None,
        Ok(state) => state,
        Err(e) => {
            return Some(Outcome::Warn(format!(
                "cannot read the Desktop profile: {e}"
            )));
        }
    };
    Some(match state {
        DesktopState::NotInstalled => return None,
        DesktopState::Unconfigured => {
            Outcome::Ok("not set up; `byokey claude desktop` opens one against BYOKEY".into())
        }
        DesktopState::OtherApplied => Outcome::Warn(
            "the third-party profile applies another gateway; run `byokey claude desktop`".into(),
        ),
        DesktopState::Applied {
            url: applied,
            running,
        } => {
            let where_ = if running { "open" } else { "not open" };
            if applied.trim_end_matches('/') == url {
                Outcome::Ok(format!("third-party profile uses BYOKEY ({where_})"))
            } else {
                Outcome::Warn(format!(
                    "third-party profile points at {applied}, not {url}; run `byokey claude desktop`"
                ))
            }
        }
    })
}

/// Mask credentials that may appear in upstream error text.
fn redact(text: &str) -> String {
    text.split_inclusive(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}'))
        .map(|word| {
            let token = word.trim_end_matches(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}')
            });
            let tail = &word[token.len()..];
            let secret = [
                "gho_",
                "ghu_",
                "ghp_",
                "ghs_",
                "ghr_",
                "github_pat_",
                "sk-",
                "crsr_",
                "eyJ",
            ]
            .iter()
            .any(|p| token.starts_with(p) && token.len() > p.len() + 8);
            if secret {
                format!("<redacted>{tail}")
            } else {
                word.to_owned()
            }
        })
        .collect()
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_masks_known_token_shapes_only() {
        let text = r#"auth failed for gho_abcdefghijklmnop, key "sk-ant-0123456789abcdef" ok"#;
        assert_eq!(
            redact(text),
            r#"auth failed for <redacted>, key "<redacted>" ok"#
        );
        assert_eq!(redact("sk- is short, gho_x too"), "sk- is short, gho_x too");
    }

    #[test]
    fn truncate_counts_characters() {
        assert_eq!(truncate("héllo", 3), "hél…");
        assert_eq!(truncate("hi", 3), "hi");
    }
}
