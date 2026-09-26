//! `byokey claude desktop`: run Claude Desktop against BYOKEY.
//!
//! Claude Desktop has a third-party inference mode with its own data
//! directory (`Claude-3p`): the entry its `configLibrary` applies names the
//! inference provider, and `deploymentMode` in that directory's
//! `claude_desktop_config.json` picks the mode at launch (`"3p"` or `"1p"`).
//! The two modes use separate data directories, so one instance of each can
//! run side by side.
//!
//! Desktop has no per-launch mode switch, only that persisted value. BYOKEY
//! adds a gateway entry at its address, sets the mode to `3p`, opens a new
//! instance, and sets it back to `1p` once that instance reports third-party
//! mode in its log. The official profile and its sign-in are never touched.
//!
//! The reset does not hold: the third-party instance writes its whole config
//! back, `3p` included, whenever it saves settings. While it runs, a cold
//! launch of Desktop (the official one closed) can therefore open in
//! third-party mode; the command warns about this.

use anyhow::{Context as _, Result, bail};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::claude::{Target, ensure_reachable, write_atomic};

const APP: &str = "/Applications/Claude.app";
const BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
/// Logged by a Desktop instance once it has started in third-party mode.
const READY: &str = "3P mode active";
const LAUNCH_TIMEOUT: Duration = Duration::from_mins(1);
/// Fixed id of the library entry BYOKEY owns, so rewrites replace it. Claude
/// Desktop accepts only UUID-shaped ids.
const ENTRY_ID: &str = "6279746b-6579-4000-8000-000000000001";

#[derive(clap::Args, Debug)]
pub struct DesktopArgs {
    #[command(flatten)]
    target: Target,
}

pub fn desktop(args: DesktopArgs) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("`byokey claude desktop` supports macOS only");
    }
    if !Path::new(APP).exists() {
        bail!("Claude Desktop is not installed at {APP}");
    }
    let url = args.target.resolve()?.url;
    ensure_reachable(&url)?;
    let profile = profile_dir()?;
    if third_party_running(&profile) {
        bail!("a BYOKEY Claude Desktop is already open; quit it to open one with new settings");
    }
    // Undo a `3p` the previous BYOKEY instance wrote back, whatever happens next.
    set_mode(&profile, "1p")?;
    let log = log_path()?;
    let offset = std::fs::metadata(&log).map_or(0, |m| m.len());

    write_profile(&profile, &url)?;
    let launched = launch(&log, offset);
    // Whatever happened, try to keep a normal launch official.
    set_mode(&profile, "1p")?;
    launched?;
    println!("Opened Claude Desktop against BYOKEY at {url}, alongside the official one");
    eprintln!(
        "warning: while the BYOKEY Claude Desktop runs, it may switch Desktop's saved \
         mode back to BYOKEY. If the official Claude Desktop is closed, opening it from \
         the Dock or Spotlight can then start the BYOKEY one instead. Run \
         `byokey claude desktop` again after quitting the BYOKEY one to restore it."
    );
    Ok(())
}

fn home() -> Result<PathBuf> {
    std::env::home_dir().context("cannot locate the home directory")
}

/// Claude Desktop's third-party data directory.
fn profile_dir() -> Result<PathBuf> {
    Ok(home()?.join("Library/Application Support/Claude-3p"))
}

/// How Claude Desktop's third-party profile stands with respect to BYOKEY.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DesktopState {
    /// Claude Desktop is not installed.
    NotInstalled,
    /// The profile has no BYOKEY entry; `byokey claude desktop` was never run.
    Unconfigured,
    /// BYOKEY's entry exists but another entry is applied.
    OtherApplied,
    /// BYOKEY's entry is applied and points at `url`.
    Applied { url: String, running: bool },
}

/// Inspect the third-party profile without changing it.
pub(crate) fn state() -> Result<DesktopState> {
    if !cfg!(target_os = "macos") || !Path::new(APP).exists() {
        return Ok(DesktopState::NotInstalled);
    }
    let profile = profile_dir()?;
    inspect(&profile, third_party_running(&profile))
}

fn inspect(profile: &Path, running: bool) -> Result<DesktopState> {
    let library = profile.join("configLibrary");
    let entry = read_object(&library.join(format!("{ENTRY_ID}.json")))?;
    let Some(url) = entry.get("inferenceGatewayBaseUrl").and_then(Value::as_str) else {
        return Ok(DesktopState::Unconfigured);
    };
    let meta = read_object(&library.join("_meta.json"))?;
    if meta.get("appliedId").and_then(Value::as_str) != Some(ENTRY_ID) {
        return Ok(DesktopState::OtherApplied);
    }
    Ok(DesktopState::Applied {
        url: url.to_owned(),
        running,
    })
}

/// Where a third-party-mode instance writes its main log.
fn log_path() -> Result<PathBuf> {
    Ok(home()?.join("Library/Logs/Claude-3p/main.log"))
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

/// Open a new Desktop instance and wait until it reports third-party mode in
/// `log` past `offset`.
fn launch(log: &Path, offset: u64) -> Result<()> {
    let status = Command::new("open")
        .args(["-n", "-b", BUNDLE_ID])
        .status()
        .context("open Claude Desktop")?;
    if !status.success() {
        bail!("`open` failed to launch Claude Desktop ({status})");
    }
    let deadline = Instant::now() + LAUNCH_TIMEOUT;
    while !logged_since(log, offset, READY) {
        if Instant::now() > deadline {
            bail!(
                "Claude Desktop did not start in third-party mode within {}s; see {}",
                LAUNCH_TIMEOUT.as_secs(),
                log.display()
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// Whether `needle` appears in what was appended to `log` after `offset`.
fn logged_since(log: &Path, offset: u64, needle: &str) -> bool {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let Ok(mut file) = std::fs::File::open(log) else {
        return false;
    };
    // A rotated (shorter) log starts over.
    let start = if file.metadata().is_ok_and(|m| m.len() < offset) {
        0
    } else {
        offset
    };
    let mut tail = Vec::new();
    file.seek(SeekFrom::Start(start)).is_ok()
        && file.read_to_end(&mut tail).is_ok()
        && String::from_utf8_lossy(&tail).contains(needle)
}

/// Whether a Desktop instance already runs on `profile`.
fn third_party_running(profile: &Path) -> bool {
    let Ok(out) = Command::new("pgrep").args(["-x", "Claude"]).output() else {
        return false;
    };
    let dir = format!("{}/", profile.display());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .any(|pid| {
            Command::new("lsof")
                .args(["-Fn", "-p", pid])
                .output()
                .is_ok_and(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .any(|l| l.starts_with('n') && l[1..].starts_with(&dir))
                })
        })
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

    #[test]
    fn inspection_reports_what_the_profile_applies() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            inspect(dir.path(), false).unwrap(),
            DesktopState::Unconfigured
        );

        write_profile(dir.path(), "http://127.0.0.1:8018").unwrap();
        assert_eq!(
            inspect(dir.path(), true).unwrap(),
            DesktopState::Applied {
                url: "http://127.0.0.1:8018".into(),
                running: true,
            }
        );

        let meta_path = dir.path().join("configLibrary/_meta.json");
        let mut meta = read_object(&meta_path).unwrap();
        meta.insert("appliedId".into(), "other".into());
        write_json(&meta_path, &Value::Object(meta)).unwrap();
        assert_eq!(
            inspect(dir.path(), false).unwrap(),
            DesktopState::OtherApplied
        );
    }

    #[test]
    fn readiness_is_read_only_from_new_log_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("main.log");
        std::fs::write(&log, "old: 3P mode active\n").unwrap();
        let offset = std::fs::metadata(&log).unwrap().len();
        assert!(!logged_since(&log, offset, READY));
        std::fs::write(
            &log,
            "old: 3P mode active\nnew: [custom-3p] 3P mode active\n",
        )
        .unwrap();
        assert!(logged_since(&log, offset, READY));
        // Rotated to a shorter file: read from the start.
        std::fs::write(&log, "3P mode active\n").unwrap();
        assert!(logged_since(&log, offset, READY));
        assert!(!logged_since(&dir.path().join("missing.log"), 0, READY));
    }
}
