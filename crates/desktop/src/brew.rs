//! Homebrew as the server's installer of record.
//!
//! On macOS the server ships as the `byokey` formula under `brew services`.
//! Whatever brew installed, brew updates: replacing a keg behind its back
//! would leave brew believing an older version is still there.
//!
//! The app itself needs none of this. Its cask declares `auto_updates true`,
//! which tells brew the app replaces its own bundle and makes brew read the
//! installed version from that bundle, so the in-app updater stays in step.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

pub const FORMULA: &str = "byokey";

/// The brew prefix a formula's executable was installed under.
///
/// Takes a canonical path: kegs live at `<prefix>/Cellar/<formula>/<version>/…`
/// and are reached through `opt/` and `bin/` symlinks.
#[must_use]
pub fn formula_prefix(executable: &Path, formula: &str) -> Option<PathBuf> {
    let components: Vec<_> = executable.components().collect();
    components.windows(2).enumerate().find_map(|(i, pair)| {
        let is_keg = pair[0].as_os_str() == "Cellar" && pair[1].as_os_str() == formula;
        is_keg.then(|| components[..i].iter().collect())
    })
}

/// The brew that installed the server at `executable`, if brew did and
/// that server runs on this machine.
#[must_use]
pub fn server_brew(executable: &str) -> Option<PathBuf> {
    // A path that doesn't resolve here belongs to a server on another machine.
    let executable = std::fs::canonicalize(executable).ok()?;
    let brew = formula_prefix(&executable, FORMULA)?.join("bin/brew");
    brew.is_file().then_some(brew)
}

/// Upgrade the formula and restart its service. Blocking; run it off the UI
/// thread.
///
/// # Errors
///
/// The first brew step that fails, with its output.
pub fn upgrade_server(brew: &Path) -> Result<()> {
    run(brew, &["update", "--quiet"])?;
    run(brew, &["upgrade", FORMULA])?;
    // `brew upgrade` leaves the running service on the old keg.
    run(brew, &["services", "restart", FORMULA])
}

fn run(brew: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new(brew)
        .args(args)
        // `update` runs explicitly first; keep later steps from repeating it.
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .env("NONINTERACTIVE", "1")
        .output()
        .with_context(|| format!("run brew {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("brew {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keg_path_yields_its_prefix() {
        for (exe, prefix) in [
            (
                "/opt/homebrew/Cellar/byokey/3.0.0/bin/byokey",
                "/opt/homebrew",
            ),
            ("/usr/local/Cellar/byokey/3.0.0_1/bin/byokey", "/usr/local"),
        ] {
            assert_eq!(
                formula_prefix(Path::new(exe), FORMULA),
                Some(PathBuf::from(prefix)),
                "{exe}"
            );
        }
    }

    #[test]
    fn other_installs_are_not_kegs() {
        for exe in [
            "/Users/me/.cargo/bin/byokey",
            "/opt/homebrew/Cellar/other/1.0.0/bin/byokey",
            "/opt/homebrew/opt/byokey/bin/byokey",
            "/opt/homebrew/bin/byokey",
            "",
        ] {
            assert_eq!(formula_prefix(Path::new(exe), FORMULA), None, "{exe}");
        }
    }
}
