use anyhow::{Context as _, Result, bail};
use byokey_config::{ClaudeCodeConfig, Config};
use byokey_types::ProviderId;
use clap::{Args, Subcommand};
use serde_json::{Map, Value};
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum ClaudeCodeAction {
    /// Point Claude Code at BYOKEY while preserving model and other settings.
    Inject(InjectArgs),
}

#[derive(Args, Debug)]
pub struct InjectArgs {
    /// BYOKEY configuration file (JSON or YAML; defaults to ~/.config/byokey/settings.json).
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Claude Code settings file (defaults to $CLAUDE_CONFIG_DIR/settings.json or ~/.claude/settings.json).
    #[arg(long, value_name = "FILE")]
    settings: Option<PathBuf>,
    /// Proxy base URL, without /v1 (overrides configured URL and listen address).
    #[arg(long)]
    url: Option<String>,
    /// Also route Claude Messages through Copilot and add Claude Code model aliases.
    #[arg(long, value_parser = ["copilot"], conflicts_with = "url")]
    backend: Option<String>,
    /// Disable experimental betas (automatic for a configured Copilot backend without --url).
    #[arg(long)]
    disable_experimental_betas: bool,
}

pub fn cmd_claude_code(action: ClaudeCodeAction) -> Result<()> {
    match action {
        ClaudeCodeAction::Inject(args) => inject(args),
    }
}

fn inject(args: InjectArgs) -> Result<()> {
    let config_path = args
        .config
        .clone()
        .map_or_else(byokey_daemon::paths::config_path, Ok)?;
    let mut config = load_config(&config_path, args.config.is_some(), args.backend.is_some())?;
    let settings_path = args.settings.map_or_else(
        || {
            ClaudeCodeConfig::default_settings_path()
                .ok_or_else(|| anyhow::anyhow!("cannot determine Claude Code settings directory"))
        },
        Ok,
    )?;
    if same_file(&config_path, &settings_path) {
        bail!("BYOKEY configuration and Claude Code settings must be different files");
    }

    let backend_update = if args.backend.is_some() {
        let update = BackendUpdate::prepare(&config_path)?;
        config
            .providers
            .entry(ProviderId::Claude)
            .or_default()
            .backend = Some(ProviderId::Copilot);
        Some(update)
    } else {
        None
    };
    // An explicit URL can point at a different gateway with its own backend.
    let copilot_backend = args.url.is_none()
        && config
            .providers
            .get(&ProviderId::Claude)
            .is_some_and(|provider| provider.backend == Some(ProviderId::Copilot));
    let disable_betas = args.disable_experimental_betas || copilot_backend;
    let url = config
        .claude_code
        .resolve_url(args.url.as_deref(), &config.host, config.port)?;

    // Validate both documents before changing either one. In particular, a
    // malformed Claude settings file must not leave behind a backend change.
    config
        .claude_code
        .validate_injection(&url, &settings_path, disable_betas, copilot_backend)?;
    if let Some(update) = &backend_update {
        update.apply()?;
    }
    let injected = config
        .claude_code
        .inject(&url, &settings_path, disable_betas, copilot_backend);
    let extras = match injected {
        Ok(extras) => extras,
        Err(error) => {
            if let Some(update) = &backend_update {
                update.rollback().context(
                    "Claude Code injection failed and BYOKEY configuration could not be restored",
                )?;
            }
            return Err(error);
        }
    };

    println!("ANTHROPIC_BASE_URL set to {url}");
    println!("ANTHROPIC_API_KEY set to a local placeholder");
    if disable_betas {
        println!("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS set to 1");
    }
    if copilot_backend {
        println!("Copilot model aliases merged into modelOverrides");
    }
    if extras > 0 {
        println!("merged {extras} extra setting(s) from claude_code.settings");
    }
    println!("config: {}", settings_path.display());
    if backend_update.is_some() {
        println!(
            "Claude Messages backend set to copilot in {}",
            config_path.display()
        );
        println!("Authenticate with `byokey login copilot` if needed.");
        println!(
            "Start BYOKEY, or restart an existing server with: byokey restart --config {}",
            shell_quote(&config_path)
        );
    }
    warn_auth_conflicts(&settings_path)?;
    println!("Restart Claude Code to apply the settings.");
    Ok(())
}

fn load_config(path: &Path, explicit: bool, create: bool) -> Result<Config> {
    if path.exists() {
        Config::from_file(path).with_context(|| format!("load BYOKEY config {}", path.display()))
    } else if explicit && !create {
        bail!("BYOKEY config does not exist: {}", path.display());
    } else {
        Ok(Config::default())
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || destination_path(a)
            .zip(destination_path(b))
            .is_some_and(|(a, b)| a == b)
}

fn destination_path(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            component => {
                resolved.push(component.as_os_str());
                // Resolve existing ancestors before interpreting later `..`
                // components, which may otherwise traverse the wrong symlink
                // parent. Missing components are normalized until an existing
                // ancestor is reached again.
                if let Ok(canonical) = resolved.canonicalize() {
                    resolved = canonical;
                }
            }
        }
    }
    Some(resolved)
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn warn_auth_conflicts(path: &Path) -> Result<()> {
    let settings: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut conflicts = Vec::new();
    for name in ["ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"] {
        if settings["env"][name]
            .as_str()
            .is_some_and(|value| !value.is_empty())
            || std::env::var_os(name).is_some_and(|value| !value.is_empty())
        {
            conflicts.push(name);
        }
    }
    if settings["apiKeyHelper"]
        .as_str()
        .is_some_and(|value| !value.is_empty())
    {
        conflicts.push("apiKeyHelper");
    }
    if !conflicts.is_empty() {
        eprintln!(
            "note: existing {} preserved; Claude Code may report conflicting credentials alongside ANTHROPIC_API_KEY",
            conflicts.join(", ")
        );
    }
    Ok(())
}

struct BackendUpdate {
    path: PathBuf,
    original: Option<Vec<u8>>,
    contents: Vec<u8>,
}

impl BackendUpdate {
    fn prepare(path: &Path) -> Result<Self> {
        // Preserve symlinks used by dotfile managers, including their target's
        // extension-independent format (the selected config path chooses it).
        let target = match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => path.canonicalize()?,
            Ok(_) => path.to_path_buf(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
            Err(error) => return Err(error.into()),
        };
        let original = match std::fs::read(&target) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let json = path
            .extension()
            .is_some_and(|extension| extension == "json");
        let mut document: Value = match original.as_deref() {
            Some(contents) if json => serde_json::from_slice(contents)?,
            Some(contents) => serde_yaml::from_slice(contents)?,
            None => Value::Object(Map::new()),
        };
        let root = document
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("BYOKEY configuration must be an object"))?;
        let providers = object_entry(root, "providers")?;
        let claude = object_entry(providers, "claude")?;
        claude.insert("backend".into(), Value::String("copilot".into()));
        let contents = if json {
            format!("{}\n", serde_json::to_string_pretty(&document)?).into_bytes()
        } else {
            serde_yaml::to_string(&document)?.into_bytes()
        };
        Ok(Self {
            path: target,
            original,
            contents,
        })
    }

    fn apply(&self) -> Result<()> {
        atomic_write(&self.path, &self.contents)
            .with_context(|| format!("save BYOKEY config {}", self.path.display()))
    }

    fn rollback(&self) -> Result<()> {
        if let Some(original) = &self.original {
            atomic_write(&self.path, original)
        } else {
            std::fs::remove_file(&self.path).map_err(Into::into)
        }
    }
}

fn object_entry<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>> {
    object
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("BYOKEY configuration {key} must be an object"))
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if let Ok(metadata) = std::fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary.write_all(contents)?;
    temporary.flush()?;
    temporary.persist(path)?;
    Ok(())
}
