//! `byokey claude desktop`: run Claude Desktop against BYOKEY.
//!
//! Claude Desktop has a third-party inference mode with its own data
//! directory (`Claude-3p`): the entry its `configLibrary` applies names the
//! inference provider, and `deploymentMode` in that directory's
//! `claude_desktop_config.json` picks the mode at launch (`"3p"` or `"1p"`).
//! BYOKEY adds a gateway entry at its address and switches the mode; the
//! official profile and its sign-in are never touched. Other library
//! entries are kept.

use anyhow::{Context as _, Result, bail};
use clap::ValueEnum;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::claude::{Target, ensure_reachable, write_atomic};

const APP: &str = "/Applications/Claude.app";
const BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
/// Fixed id of the library entry BYOKEY owns, so rewrites replace it. Claude
/// Desktop accepts only UUID-shaped ids.
const ENTRY_ID: &str = "6279746b-6579-4000-8000-000000000001";

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum Mode {
    /// Claude Desktop in third-party mode, against BYOKEY.
    #[default]
    Byokey,
    /// Claude Desktop with its official sign-in.
    Official,
}

#[derive(clap::Args, Debug)]
pub struct DesktopArgs {
    #[command(flatten)]
    target: Target,
    /// Which profile to launch.
    #[arg(value_enum, default_value_t)]
    mode: Mode,
}

pub fn desktop(args: DesktopArgs) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("`byokey claude desktop` supports macOS only");
    }
    if !Path::new(APP).exists() {
        bail!("Claude Desktop is not installed at {APP}");
    }
    let profile = profile_dir()?;
    match args.mode {
        Mode::Byokey => {
            let (_, url) = args.target.resolve()?;
            ensure_reachable(&url)?;
            write_profile(&profile, &url)?;
            relaunch()?;
            println!("Claude Desktop now uses BYOKEY at {url}");
        }
        Mode::Official => {
            set_mode(&profile, "1p")?;
            relaunch()?;
            println!("Claude Desktop is back on its official sign-in");
        }
    }
    Ok(())
}

/// Claude Desktop's third-party data directory.
fn profile_dir() -> Result<PathBuf> {
    let home = std::env::home_dir().context("cannot locate the home directory")?;
    Ok(home.join("Library/Application Support/Claude-3p"))
}

/// The gateway configuration Claude Desktop reads in third-party mode.
fn gateway_config(url: &str) -> Value {
    json!({
        "inferenceProvider": "gateway",
        "inferenceGatewayBaseUrl": url,
        "inferenceCredentialKind": "static",
        // BYOKEY ignores client credentials, but the gateway needs one.
        "inferenceGatewayApiKey": "byokey",
        "inferenceGatewayAuthScheme": "bearer",
        "modelDiscoveryEnabled": true,
    })
}

/// Point `profile`'s config library at BYOKEY, keeping other entries.
fn write_profile(profile: &Path, url: &str) -> Result<()> {
    let library = profile.join("configLibrary");
    let meta_path = library.join("_meta.json");
    let mut meta = read_object(&meta_path)?;
    let entries = meta
        .entry("entries")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .context("configLibrary/_meta.json: `entries` must be an array")?;
    if !entries.iter().any(|e| e["id"] == ENTRY_ID) {
        entries.push(json!({"id": ENTRY_ID, "name": "BYOKEY", "provider": "gateway"}));
    }
    meta.insert("appliedId".into(), ENTRY_ID.into());

    write_json(
        &library.join(format!("{ENTRY_ID}.json")),
        &gateway_config(url),
    )?;
    write_json(&meta_path, &Value::Object(meta))?;
    set_mode(profile, "3p")
}

/// Persist the launch mode: `"3p"` applies the library entry, `"1p"` keeps
/// the official sign-in.
fn set_mode(profile: &Path, mode: &str) -> Result<()> {
    let path = profile.join("claude_desktop_config.json");
    let mut desktop = read_object(&path)?;
    desktop.insert("deploymentMode".into(), mode.into());
    write_json(&path, &Value::Object(desktop))
}

fn read_object(path: &Path) -> Result<Map<String, Value>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid JSON in {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes).with_context(|| format!("write {}", path.display()))
}

/// Quit a running Claude Desktop so it rereads its mode, then open it.
fn relaunch() -> Result<()> {
    if running() {
        let script = format!("tell application id \"{BUNDLE_ID}\" to quit");
        Command::new("osascript")
            .args(["-e", &script])
            .status()
            .context("quit Claude Desktop")?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while running() {
            if Instant::now() > deadline {
                bail!("Claude Desktop did not quit; quit it and run this again");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    let status = Command::new("open")
        .args(["-b", BUNDLE_ID])
        .status()
        .context("open Claude Desktop")?;
    if !status.success() {
        bail!("`open` failed to launch Claude Desktop ({status})");
    }
    Ok(())
}

fn running() -> bool {
    Command::new("pgrep")
        .args(["-x", "Claude"])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_points_the_config_library_at_byokey_and_keeps_other_entries() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("configLibrary");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(
            library.join("_meta.json"),
            r#"{"appliedId":"other","entries":[{"id":"other","name":"Mine"}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("claude_desktop_config.json"),
            r#"{"deploymentMode":"1p","keep":1}"#,
        )
        .unwrap();

        write_profile(dir.path(), "http://127.0.0.1:8018").unwrap();
        write_profile(dir.path(), "http://127.0.0.1:9000").unwrap();

        let meta = read_object(&library.join("_meta.json")).unwrap();
        assert_eq!(meta["appliedId"], ENTRY_ID);
        let ids: Vec<&Value> = meta["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| &e["id"])
            .collect();
        assert_eq!(ids, [&json!("other"), &json!(ENTRY_ID)]);
        let entry = read_object(&library.join(format!("{ENTRY_ID}.json"))).unwrap();
        assert_eq!(entry["inferenceGatewayBaseUrl"], "http://127.0.0.1:9000");
        let desktop = read_object(&dir.path().join("claude_desktop_config.json")).unwrap();
        assert_eq!(desktop["deploymentMode"], "3p");
        assert_eq!(desktop["keep"], 1);
    }
}
