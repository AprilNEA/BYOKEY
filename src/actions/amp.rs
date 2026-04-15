use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum AmpAction {
    /// Inject the byokey proxy URL into Amp configuration.
    Inject {
        /// The proxy URL to inject (overrides all config-based resolution).
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
    let config = load_byokey_config();
    let resolved_url = config.amp.resolve_url(url.as_deref(), &config.host);

    let settings_path = byokey_config::AmpConfig::default_settings_path()
        .ok_or_else(|| anyhow::anyhow!("cannot determine HOME directory"))?;

    let extras = config.amp.inject(&resolved_url, &settings_path)?;

    println!("amp.url set to {resolved_url}");
    if extras > 0 {
        println!("merged {extras} extra setting(s) from byokey config");
    }
    println!("config: {}", settings_path.display());
    Ok(())
}

fn load_byokey_config() -> byokey_config::Config {
    let Ok(path) = byokey_daemon::paths::config_path() else {
        return byokey_config::Config::default();
    };
    if !path.exists() {
        return byokey_config::Config::default();
    }
    match byokey_config::Config::from_file(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "warning: failed to load byokey config at {}: {e}",
                path.display()
            );
            byokey_config::Config::default()
        }
    }
}
