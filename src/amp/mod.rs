use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum AmpAction {
    /// Inject the byokey proxy URL into Amp configuration.
    Inject {
        /// The proxy URL to inject (default: http://localhost:8018).
        #[arg(long)]
        url: Option<String>,
    },
}

pub fn cmd_amp(action: AmpAction) -> Result<()> {
    match action {
        AmpAction::Inject { url } => cmd_amp_inject(url),
    }
}

fn cmd_amp_inject(url: Option<String>) -> Result<()> {
    let byokey_amp = load_byokey_amp_config();

    let resolved_url = url
        .or_else(|| {
            byokey_amp
                .settings
                .get("amp.url")
                .and_then(|v| v.as_str().map(String::from))
        })
        .unwrap_or_else(|| "http://localhost:8018/amp".to_string());
    let settings_path = amp_settings_path();

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut map: serde_json::Map<String, serde_json::Value> = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };

    for (k, v) in &byokey_amp.settings {
        map.insert(k.clone(), v.clone());
    }
    map.insert(
        "amp.url".to_string(),
        serde_json::Value::String(resolved_url.clone()),
    );

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))?;
    std::fs::write(&settings_path, format!("{json}\n"))?;
    println!("amp.url set to {resolved_url}");
    let extras = byokey_amp
        .settings
        .len()
        .saturating_sub(usize::from(byokey_amp.settings.contains_key("amp.url")));
    if extras > 0 {
        println!("merged {extras} extra setting(s) from byokey config");
    }
    println!("config: {}", settings_path.display());
    Ok(())
}

fn load_byokey_amp_config() -> byokey_config::AmpConfig {
    let Ok(path) = byokey_daemon::paths::config_path() else {
        return byokey_config::AmpConfig::default();
    };
    if !path.exists() {
        return byokey_config::AmpConfig::default();
    }
    match byokey_config::Config::from_file(&path) {
        Ok(c) => c.amp,
        Err(e) => {
            eprintln!(
                "warning: failed to load byokey config at {}: {e}",
                path.display()
            );
            byokey_config::AmpConfig::default()
        }
    }
}

fn amp_settings_path() -> std::path::PathBuf {
    byokey_daemon::paths::home_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".config")
        .join("amp")
        .join("settings.json")
}
