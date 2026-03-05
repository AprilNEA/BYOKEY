mod bundle;
mod patch;

use anyhow::Result;
use std::path::PathBuf;

pub fn cmd_amp_inject(url: Option<String>) -> Result<()> {
    let url = url.unwrap_or_else(|| "http://localhost:8018/amp".to_string());
    let settings_path = amp_settings_path();

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Read existing settings or start with an empty object.
    let mut map: serde_json::Map<String, serde_json::Value> = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };

    map.insert(
        "amp.url".to_string(),
        serde_json::Value::String(url.clone()),
    );

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))?;
    std::fs::write(&settings_path, format!("{json}\n"))?;
    println!("amp.url set to {url}");
    println!("config: {}", settings_path.display());
    Ok(())
}

pub fn cmd_amp_disable_ads(paths: Vec<PathBuf>, restore: bool) -> Result<()> {
    let bundles = if paths.is_empty() {
        println!("searching for amp bundle...");
        let found = bundle::find_amp_bundles();
        if found.is_empty() {
            println!("no amp bundle found — falling back to hide_free_tier");
            set_hide_free_tier(true)?;
            return Ok(());
        }
        for p in &found {
            println!("  found: {}", p.display());
        }
        found
    } else {
        paths
    };

    if restore {
        for bundle_path in &bundles {
            println!("\nrestoring: {}", bundle_path.display());
            patch::amp_restore(bundle_path)?;
        }
        println!("\nrestart amp / reload editor window to apply.");
        return Ok(());
    }

    let mut any_patched = false;
    let mut all_failed = true;

    for bundle_path in &bundles {
        println!("\npatching: {}", bundle_path.display());

        let data = match std::fs::read(bundle_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  ERROR reading file: {e}");
                continue;
            }
        };
        println!("  size: {} bytes", data.len());

        match patch::amp_patch(&data) {
            Ok(Some(patched)) => {
                let bak = bundle_path.with_extension("js.bak");
                if !bak.exists() {
                    std::fs::copy(bundle_path, &bak)?;
                    println!("  backup saved: {}", bak.display());
                }
                std::fs::write(bundle_path, patched)?;
                println!("  patched successfully");
                patch::resign_adhoc(bundle_path);
                any_patched = true;
                all_failed = false;
            }
            Ok(None) => {
                println!("  already patched — skipping");
                all_failed = false;
            }
            Err(e) => eprintln!("  ERROR: {e}"),
        }
    }

    if all_failed {
        println!("\nbinary patch failed — enabling hide_free_tier as fallback");
        set_hide_free_tier(true)?;
    } else if any_patched {
        println!("\nrestart amp / reload editor window to apply.");
    }

    Ok(())
}

/// Enable or disable `amp.hide_free_tier` in the byokey config file.
fn set_hide_free_tier(enabled: bool) -> Result<()> {
    let config_path = byokey_daemon::paths::config_path()?;

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut map: serde_json::Map<String, serde_json::Value> = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };

    // Ensure `amp` object exists.
    let amp = map
        .entry("amp")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let Some(obj) = amp.as_object_mut() {
        obj.insert(
            "hide_free_tier".to_string(),
            serde_json::Value::Bool(enabled),
        );
    }

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))?;
    std::fs::write(&config_path, format!("{json}\n"))?;
    println!(
        "  amp.hide_free_tier = {enabled} in {}",
        config_path.display()
    );
    Ok(())
}

fn amp_settings_path() -> PathBuf {
    byokey_daemon::paths::home_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".amp")
        .join("settings.json")
}
